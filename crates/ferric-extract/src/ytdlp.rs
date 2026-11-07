//! yt-dlp subprocess fallback (Task 101).
//!
//! Safety invariants:
//! - Never invoke a shell (`cmd` / `sh -c`). Args are always argv vectors.
//! - URL must be `http`/`https`, no control characters / NULs.
//! - Optional binary override must be an absolute existing file named like yt-dlp.
//! - Cookies path (if any) must be an absolute regular file under the user's data dir
//!   or explicitly passed absolute path (no `..` components).
//! - Process is killed on timeout; stdout is capped.
//! - stderr is logged truncated; never treat stderr as a shell command.

use anyhow::{anyhow, bail, Context, Result};
use ferric_common::{
    Chapter, ExtractResult, ExtractedFormat, PlaylistEntry, SubtitleFormat, SubtitleTrack,
};
use serde::Deserialize;
use serde_json::Value;
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Hard caps — defend against runaway subprocess output / hang.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_STDOUT_BYTES: usize = 32 * 1024 * 1024; // 32 MiB
const MAX_STDERR_LOG: usize = 4 * 1024;

static DETECTED: OnceLock<Option<PathBuf>> = OnceLock::new();

/// Snapshot of yt-dlp availability (for API / UI).
#[derive(Debug, Clone, serde::Serialize)]
pub struct YtdlpStatus {
    pub available: bool,
    pub path: Option<String>,
    pub version: Option<String>,
    pub explanation: String,
}

/// Options for a single yt-dlp extraction.
#[derive(Debug, Clone, Default)]
pub struct YtdlpOptions {
    /// Absolute path to cookies.txt (Netscape). Optional.
    pub cookies_file: Option<PathBuf>,
    /// Override binary (absolute). If None, use discovered PATH binary.
    pub binary: Option<PathBuf>,
    pub timeout: Option<Duration>,
}

/// Deno is yt-dlp's JS runtime for YouTube's challenge; without it only a few formats come back.
/// PATH of a GUI-launched daemon often lacks winget's deno, so also probe its install dirs.
fn detect_deno() -> Option<PathBuf> {
    if let Ok(p) = which::which("deno") {
        return Some(p);
    }
    let local = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let winget = local.join("Microsoft").join("WinGet");
    let link = winget.join("Links").join("deno.exe");
    if link.is_file() {
        return Some(link);
    }
    std::fs::read_dir(winget.join("Packages"))
        .ok()?
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("DenoLand.Deno"))
        .map(|e| e.path().join("deno.exe"))
        .find(|p| p.is_file())
}

/// Probe PATH (and common names) once; cache result.
pub fn detect() -> Option<PathBuf> {
    DETECTED
        .get_or_init(|| {
            if let Ok(override_path) = std::env::var("FERRIC_YTDLP") {
                match validate_binary_path(Path::new(&override_path)) {
                    Ok(p) => {
                        tracing::info!(path = %p.display(), "yt-dlp from FERRIC_YTDLP");
                        return Some(p);
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "FERRIC_YTDLP invalid; falling back to PATH");
                    }
                }
            }
            for name in ["yt-dlp", "yt-dlp.exe", "youtube-dl", "youtube-dl.exe"] {
                if let Ok(p) = which::which(name) {
                    if validate_binary_path(&p).is_ok() {
                        tracing::info!(path = %p.display(), "yt-dlp detected on PATH");
                        return Some(p);
                    }
                }
            }
            tracing::info!("yt-dlp not found on PATH");
            None
        })
        .clone()
}

pub async fn status() -> YtdlpStatus {
    status_with_binary(None).await
}

/// Status using optional settings override (absolute path), else PATH / FERRIC_YTDLP.
pub async fn status_with_binary(binary: Option<PathBuf>) -> YtdlpStatus {
    let path = match binary {
        Some(ref p) => match validate_binary_path(p) {
            Ok(p) => Some(p),
            Err(e) => {
                return YtdlpStatus {
                    available: false,
                    path: Some(p.display().to_string()),
                    version: None,
                    explanation: format!("configured yt-dlp path invalid: {e}"),
                };
            }
        },
        None => detect(),
    };
    let Some(path) = path else {
        return YtdlpStatus {
            available: false,
            path: None,
            version: None,
            explanation: "yt-dlp not found. Install from https://github.com/yt-dlp/yt-dlp, \
                 set Settings → Media → yt-dlp path, or set FERRIC_YTDLP."
                .into(),
        };
    };
    let version = run_version(&path).await.ok();
    YtdlpStatus {
        available: true,
        path: Some(path.display().to_string()),
        version: version.clone(),
        explanation: match version {
            Some(v) => format!("yt-dlp available ({v})"),
            None => "yt-dlp binary found but --version failed".into(),
        },
    }
}

async fn run_version(bin: &Path) -> Result<String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // yt-dlp is a console program: without this Windows flashes a console window each time it runs.
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("yt-dlp stdout missing"))?;
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(10), stdout.read_to_end(&mut buf));
    let status = tokio::time::timeout(Duration::from_secs(10), child.wait());

    let (read_res, status_res) = tokio::join!(read, status);
    let _ = read_res;
    let status = status_res
        .map_err(|_| anyhow!("yt-dlp --version timed out"))?
        .context("wait yt-dlp --version")?;
    if !status.success() {
        bail!("yt-dlp --version exit {status}");
    }
    let s = String::from_utf8_lossy(&buf).trim().to_string();
    if s.is_empty() {
        bail!("empty version");
    }
    // First line only, strip control chars
    Ok(s.lines()
        .next()
        .unwrap_or(&s)
        .chars()
        .filter(|c| !c.is_control())
        .collect())
}

/// Run yt-dlp `--dump-single-json` and map into `ExtractResult`.
pub async fn extract(url: &str, opts: YtdlpOptions) -> Result<ExtractResult> {
    let url = sanitize_extract_url(url)?;
    let bin = match opts.binary {
        Some(ref p) => validate_binary_path(p)?,
        None => detect().ok_or_else(|| anyhow!("yt-dlp not available"))?,
    };
    if let Some(ref cookies) = opts.cookies_file {
        validate_cookies_path(cookies)?;
    }

    let timeout = opts.timeout.unwrap_or(DEFAULT_TIMEOUT);
    let mut args: Vec<String> = vec![
        "--dump-single-json".into(),
        "--no-download".into(),
        "--no-warnings".into(),
        "--no-call-home".into(),
        "--no-playlist".into(), // single video first; playlists use flat mode below
        "--socket-timeout".into(),
        "30".into(),
        "--retries".into(),
        "2".into(),
        // Quiet progress on stderr
        "--quiet".into(),
    ];
    if let Some(deno) = detect_deno() {
        args.push("--js-runtimes".into());
        args.push(format!("deno:{}", deno.display()));
    }
    if let Some(ref cookies) = opts.cookies_file {
        args.push("--cookies".into());
        args.push(cookies.to_string_lossy().into_owned());
    }
    // URL last — never prefixed with flags that could be misparsed if URL starts with -
    args.push("--".into());
    args.push(url.clone());

    let (stdout, stderr, code) = run_capped(&bin, &args, timeout).await?;
    if code != 0 {
        // Retry as playlist / flat entries if single failed
        let mut plist_args = vec![
            "--dump-single-json".into(),
            "--no-download".into(),
            "--flat-playlist".into(),
            "--no-warnings".into(),
            "--no-call-home".into(),
            "--socket-timeout".into(),
            "30".into(),
            "--quiet".into(),
        ];
        if let Some(ref cookies) = opts.cookies_file {
            plist_args.push("--cookies".into());
            plist_args.push(cookies.to_string_lossy().into_owned());
        }
        plist_args.push("--".into());
        plist_args.push(url.clone());
        let (stdout2, stderr2, code2) = run_capped(&bin, &plist_args, timeout).await?;
        if code2 != 0 {
            let err = truncate_log(&stderr).or_else(|| truncate_log(&stderr2));
            bail!(
                "yt-dlp failed (exit {code}/{code2}){}",
                err.map(|e| format!(": {e}")).unwrap_or_default()
            );
        }
        return parse_dump_json(&stdout2);
    }

    parse_dump_json(&stdout)
}

async fn run_capped(
    bin: &Path,
    args: &[String],
    timeout: Duration,
) -> Result<(Vec<u8>, Vec<u8>, i32)> {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
                                     // Do not inherit unnecessary env secrets; keep PATH for yt-dlp deps.
                                     // Explicitly drop known proxy override confusion only if we want isolation —
                                     // leave env as-is for geo/proxy users, but never inject shell.

    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawn yt-dlp at {}", bin.display()))?;

    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("missing stdout pipe"))?;
    let mut stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow!("missing stderr pipe"))?;

    let stdout_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            let n = stdout.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            if buf.len().saturating_add(n) > MAX_STDOUT_BYTES {
                anyhow::bail!("yt-dlp stdout exceeded {MAX_STDOUT_BYTES} bytes");
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Ok::<_, anyhow::Error>(buf)
    });
    let stderr_task = tokio::spawn(async move {
        let mut buf = Vec::new();
        let mut chunk = [0_u8; 2048];
        loop {
            let n = stderr.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            // Cap stderr kept in memory
            if buf.len() < MAX_STDERR_LOG * 2 {
                let take = n.min(MAX_STDERR_LOG * 2 - buf.len());
                buf.extend_from_slice(&chunk[..take]);
            }
        }
        Ok::<_, anyhow::Error>(buf)
    });

    let wait = tokio::time::timeout(timeout, child.wait());
    let status = match wait.await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            let _ = child.start_kill();
            return Err(e).context("wait yt-dlp");
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            let _ = stdout_task.await;
            let _ = stderr_task.await;
            bail!("yt-dlp timed out after {}s", timeout.as_secs());
        }
    };

    let stdout_buf = stdout_task
        .await
        .map_err(|e| anyhow!("stdout join: {e}"))??;
    let stderr_buf = stderr_task
        .await
        .map_err(|e| anyhow!("stderr join: {e}"))??;

    if !stderr_buf.is_empty() {
        if let Some(msg) = truncate_log(&stderr_buf) {
            tracing::debug!(stderr = %msg, "yt-dlp stderr");
        }
    }

    let code = status.code().unwrap_or(-1);
    Ok((stdout_buf, stderr_buf, code))
}

// ── Path / URL guards ────────────────────────────────────────────────────────

fn sanitize_extract_url(raw: &str) -> Result<String> {
    if raw.is_empty() {
        bail!("empty URL");
    }
    if raw.len() > 4096 {
        bail!("URL too long");
    }
    if raw.chars().any(|c| c.is_control() || c == '\0') {
        bail!("URL contains control characters");
    }
    let parsed = url::Url::parse(raw).map_err(|e| anyhow!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        other => bail!("yt-dlp only allowed for http(s), got {other}"),
    }
    if parsed.host_str().is_none() {
        bail!("URL missing host");
    }
    // Re-serialize to a clean string (drops odd whitespace etc.)
    Ok(parsed.to_string())
}

fn validate_binary_path(path: &Path) -> Result<PathBuf> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        // which() returns absolute; relative only allowed if we resolve
        std::fs::canonicalize(path).with_context(|| format!("canonicalize {}", path.display()))?
    };
    if !path.is_file() {
        bail!("yt-dlp path is not a file: {}", path.display());
    }
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // Allow yt-dlp, yt-dlp.exe, youtube-dl, yt-dlp_x86.exe, etc.
    let ok = name.starts_with("yt-dlp") || name.starts_with("youtube-dl");
    if !ok {
        bail!("refusing binary name {name:?} (must look like yt-dlp)");
    }
    Ok(path)
}

fn validate_cookies_path(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        bail!("cookies path must be absolute");
    }
    if path.components().any(|c| matches!(c, Component::ParentDir)) {
        bail!("cookies path must not contain ..");
    }
    if !path.is_file() {
        bail!("cookies file not found: {}", path.display());
    }
    // Size guard — cookies.txt should be small
    let meta = std::fs::metadata(path).context("cookies metadata")?;
    if meta.len() > 8 * 1024 * 1024 {
        bail!("cookies file too large");
    }
    Ok(())
}

fn truncate_log(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    let s = String::from_utf8_lossy(bytes);
    let s = s
        .chars()
        .filter(|c| !c.is_control() || *c == '\n')
        .collect::<String>();
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    Some(if s.len() > MAX_STDERR_LOG {
        format!("{}…", &s[..MAX_STDERR_LOG])
    } else {
        s.to_string()
    })
}

// ── JSON mapping ─────────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct Dump {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    fulltitle: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    thumbnail: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    upload_date: Option<String>,
    #[serde(default)]
    tags: Option<Vec<String>>,
    #[serde(default)]
    extractor: Option<String>,
    #[serde(default)]
    extractor_key: Option<String>,
    #[serde(default)]
    formats: Option<Vec<DumpFormat>>,
    #[serde(default)]
    requested_formats: Option<Vec<DumpFormat>>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    subtitles: Option<Value>,
    #[serde(default)]
    automatic_captions: Option<Value>,
    #[serde(default)]
    chapters: Option<Vec<DumpChapter>>,
    #[serde(default)]
    entries: Option<Vec<Value>>,
    #[serde(default)]
    _type: Option<String>,
    #[serde(default)]
    http_headers: Option<Value>,
}

#[derive(Debug, Deserialize)]
struct DumpFormat {
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    tbr: Option<f64>,
    #[serde(default)]
    abr: Option<f64>,
    #[serde(default)]
    vbr: Option<f64>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
    #[serde(default)]
    filesize: Option<u64>,
    #[serde(default)]
    filesize_approx: Option<u64>,
    #[serde(default)]
    format_note: Option<String>,
    #[serde(default)]
    format: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    http_headers: Option<Value>,
    #[serde(default)]
    manifest_url: Option<String>,
    #[serde(default)]
    container: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DumpChapter {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    start_time: Option<f64>,
    #[serde(default)]
    end_time: Option<f64>,
}

pub fn parse_dump_json(bytes: &[u8]) -> Result<ExtractResult> {
    // yt-dlp may emit multiple JSON objects for playlists without -J; take first object.
    let text = std::str::from_utf8(bytes).context("yt-dlp stdout not utf-8")?;
    let text = text.trim();
    if text.is_empty() {
        bail!("yt-dlp returned empty JSON");
    }
    // Prefer first complete JSON value
    let dump: Dump = serde_json::from_str(text)
        .or_else(|_| {
            // Multi-line NDJSON: first line
            let first = text.lines().next().unwrap_or(text);
            serde_json::from_str(first)
        })
        .context("parse yt-dlp JSON")?;

    // Playlist container
    if dump._type.as_deref() == Some("playlist")
        || (dump.entries.is_some() && dump.formats.is_none() && dump.url.is_none())
    {
        return Ok(map_playlist(dump));
    }

    Ok(map_video(dump))
}

fn map_playlist(dump: Dump) -> ExtractResult {
    let mut entries = Vec::new();
    if let Some(list) = dump.entries {
        for (i, ent) in list.into_iter().enumerate() {
            let url = if let Some(u) = ent
                .get("url")
                .or_else(|| ent.get("webpage_url"))
                .and_then(|v| v.as_str())
            {
                u.to_string()
            } else if let Some(id) = ent.get("id").and_then(|id| id.as_str()) {
                format!("https://www.youtube.com/watch?v={id}")
            } else {
                continue;
            };
            entries.push(PlaylistEntry {
                url,
                title: ent
                    .get("title")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                index: (i + 1) as u32,
                duration_secs: ent.get("duration").and_then(|v| v.as_f64()),
                thumbnail: ent
                    .get("thumbnail")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
            });
        }
    }
    ExtractResult {
        title: dump
            .title
            .or(dump.fulltitle)
            .unwrap_or_else(|| "Playlist".into()),
        uploader: dump.uploader.or(dump.channel),
        thumbnail: dump.thumbnail,
        duration_secs: None,
        formats: Vec::new(),
        extractor: format!(
            "yt-dlp:{}",
            dump.extractor_key
                .or(dump.extractor)
                .unwrap_or_else(|| "playlist".into())
        ),
        description: dump.description,
        upload_date: dump.upload_date,
        tags: dump.tags.unwrap_or_default(),
        subtitles: Vec::new(),
        playlist_entries: entries,
        chapters: Vec::new(),
    }
}

fn map_video(dump: Dump) -> ExtractResult {
    let mut formats = Vec::new();

    let mut raw_formats = Vec::new();
    if let Some(list) = dump.formats {
        raw_formats.extend(list);
    }
    if raw_formats.is_empty() {
        if let Some(list) = dump.requested_formats {
            raw_formats.extend(list);
        }
    }
    for f in raw_formats {
        if let Some(fmt) = dump_format_to_extracted(f, formats.len()) {
            formats.push(fmt);
        }
    }
    // Progressive single-url result
    if formats.is_empty() {
        if let Some(url) = dump.url.filter(|u| u.starts_with("http")) {
            let ext = dump.ext.unwrap_or_else(|| "mp4".into());
            let headers = http_headers_from_value(dump.http_headers.as_ref());
            formats.push(ExtractedFormat {
                id: dump.id.clone().unwrap_or_else(|| "0".into()),
                label: "direct".into(),
                url,
                ext: ext.clone(),
                protocol: Some("https".into()),
                manifest_url: None,
                format_note: None,
                width: None,
                height: None,
                tbr_kbps: None,
                abr_kbps: None,
                vbr_kbps: None,
                fps: None,
                vcodec: None,
                acodec: None,
                container: Some(ext),
                language: None,
                filesize: None,
                has_drm: false,
                headers,
                is_audio_only: false,
            });
        }
    }

    let subtitles = merge_subs(dump.subtitles.as_ref(), false);
    let mut auto = merge_subs(dump.automatic_captions.as_ref(), true);
    let mut all_subs = subtitles;
    all_subs.append(&mut auto);

    let duration = dump.duration;
    let chapters = dump
        .chapters
        .unwrap_or_default()
        .into_iter()
        .map(|c| {
            let start = c.start_time.unwrap_or(0.0);
            let end = c.end_time.unwrap_or(duration.unwrap_or(start));
            Chapter {
                title: c.title.unwrap_or_else(|| "Chapter".into()),
                start_secs: start,
                end_secs: end,
            }
        })
        .collect();

    let extractor = format!(
        "yt-dlp:{}",
        dump.extractor_key
            .or(dump.extractor)
            .unwrap_or_else(|| "generic".into())
    );

    ExtractResult {
        title: dump
            .title
            .or(dump.fulltitle)
            .or(dump.id)
            .unwrap_or_else(|| "Video".into()),
        uploader: dump.uploader.or(dump.channel),
        thumbnail: dump.thumbnail,
        duration_secs: duration,
        formats,
        extractor,
        description: dump.description,
        upload_date: dump.upload_date,
        tags: dump.tags.unwrap_or_default(),
        subtitles: all_subs,
        playlist_entries: Vec::new(),
        chapters,
    }
}

fn merge_subs(v: Option<&Value>, auto: bool) -> Vec<SubtitleTrack> {
    let Some(Value::Object(map)) = v else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (lang, arr) in map {
        let Some(list) = arr.as_array() else {
            continue;
        };
        for item in list {
            let url = item
                .get("url")
                .and_then(|u| u.as_str())
                .unwrap_or("")
                .to_string();
            if !url.starts_with("http") {
                continue;
            }
            let ext = item
                .get("ext")
                .and_then(|e| e.as_str())
                .unwrap_or("vtt")
                .to_ascii_lowercase();
            let format = match ext.as_str() {
                "srt" => SubtitleFormat::Srt,
                "ttml" | "xml" => SubtitleFormat::Ttml,
                "json3" | "srv1" | "srv2" | "srv3" => SubtitleFormat::Json3,
                _ => SubtitleFormat::Vtt,
            };
            let name = item
                .get("name")
                .and_then(|n| n.as_str())
                .map(str::to_string);
            out.push(SubtitleTrack {
                lang: lang.clone(),
                name,
                url,
                format,
                is_auto_generated: auto,
            });
        }
    }
    out
}

fn dump_format_to_extracted(f: DumpFormat, idx: usize) -> Option<ExtractedFormat> {
    let url = f.url.filter(|u| u.starts_with("http"))?;
    let ext = f
        .ext
        .clone()
        .unwrap_or_else(|| guess_ext(&url).unwrap_or_else(|| "bin".into()));
    let vcodec = f.vcodec.filter(|c| c != "none");
    let acodec = f.acodec.filter(|c| c != "none");
    let is_audio_only = vcodec.is_none() && acodec.is_some();
    let height = f.height;
    let label = f
        .format_note
        .clone()
        .or(f.format.clone())
        .or_else(|| height.map(|h| format!("{h}p")))
        .unwrap_or_else(|| f.format_id.clone().unwrap_or_else(|| "fmt".into()));
    let headers = http_headers_from_value(f.http_headers.as_ref());
    Some(ExtractedFormat {
        id: f.format_id.unwrap_or_else(|| idx.to_string()),
        label,
        url,
        ext: ext.clone(),
        protocol: f.protocol.or_else(|| {
            if ext == "m3u8" {
                Some("m3u8".into())
            } else {
                Some("https".into())
            }
        }),
        manifest_url: f.manifest_url,
        format_note: f.format_note.or(f.format),
        width: f.width,
        height,
        tbr_kbps: f.tbr,
        abr_kbps: f.abr,
        vbr_kbps: f.vbr,
        fps: f.fps,
        vcodec,
        acodec,
        container: f.container.or(Some(ext)),
        language: f.language,
        filesize: f.filesize.or(f.filesize_approx),
        has_drm: false,
        headers,
        is_audio_only,
    })
}

fn http_headers_from_value(v: Option<&Value>) -> Vec<(String, String)> {
    let Some(Value::Object(map)) = v else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(k, val)| {
            let v = val.as_str()?;
            // Never surface authorization-like headers into generic logs via structure —
            // still pass through for download requests (caller must not log).
            Some((k.clone(), v.to_string()))
        })
        .collect()
}

fn guess_ext(url: &str) -> Option<String> {
    let path = url::Url::parse(url).ok()?.path().to_ascii_lowercase();
    if path.contains(".m3u8") {
        return Some("m3u8".into());
    }
    if path.contains(".mpd") {
        return Some("mpd".into());
    }
    Path::new(path.split('?').next()?)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_urls() {
        assert!(sanitize_extract_url("file:///etc/passwd").is_err());
        assert!(sanitize_extract_url("javascript:alert(1)").is_err());
        assert!(sanitize_extract_url("https://example.com/a\nb").is_err());
        assert!(sanitize_extract_url("https://example.com/watch?v=1").is_ok());
    }

    #[test]
    fn rejects_bad_binary_names() {
        // Use a temp file with wrong name
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("malware.exe");
        std::fs::write(&p, b"x").unwrap();
        let abs = std::fs::canonicalize(&p).unwrap();
        assert!(validate_binary_path(&abs).is_err());
    }

    #[test]
    fn accepts_ytdlp_named_binary() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("yt-dlp.exe");
        std::fs::write(&p, b"x").unwrap();
        let abs = std::fs::canonicalize(&p).unwrap();
        assert!(validate_binary_path(&abs).is_ok());
    }

    #[test]
    fn parses_minimal_video_json() {
        let raw = br#"{
          "id": "abc",
          "title": "Test",
          "uploader": "Ch",
          "duration": 12.5,
          "extractor_key": "Youtube",
          "formats": [
            {
              "format_id": "18",
              "url": "https://cdn.example/v.mp4",
              "ext": "mp4",
              "height": 360,
              "vcodec": "avc1",
              "acodec": "mp4a",
              "tbr": 500
            }
          ],
          "subtitles": {
            "en": [{"url": "https://cdn.example/en.vtt", "ext": "vtt"}]
          },
          "chapters": [
            {"title": "Intro", "start_time": 0, "end_time": 5}
          ]
        }"#;
        let r = parse_dump_json(raw).unwrap();
        assert_eq!(r.title, "Test");
        assert_eq!(r.extractor, "yt-dlp:Youtube");
        assert_eq!(r.formats.len(), 1);
        assert_eq!(r.formats[0].height, Some(360));
        assert_eq!(r.subtitles.len(), 1);
        assert_eq!(r.chapters.len(), 1);
        assert_eq!(r.chapters[0].title, "Intro");
    }

    #[test]
    fn parses_playlist_json() {
        let raw = br#"{
          "_type": "playlist",
          "title": "My PL",
          "extractor_key": "YoutubeTab",
          "entries": [
            {"id": "aaaaaaaaaaa", "title": "One", "url": "https://www.youtube.com/watch?v=aaaaaaaaaaa"},
            {"id": "bbbbbbbbbbb", "title": "Two"}
          ]
        }"#;
        let r = parse_dump_json(raw).unwrap();
        assert!(r.formats.is_empty());
        assert_eq!(r.playlist_entries.len(), 2);
        assert_eq!(r.playlist_entries[0].index, 1);
        assert!(r.playlist_entries[1].url.contains("bbbbbbbbbbb"));
    }

    #[test]
    fn cookies_path_rejects_parent_dir() {
        #[cfg(windows)]
        let p = PathBuf::from(r"C:\Users\test\..\cookies.txt");
        #[cfg(not(windows))]
        let p = PathBuf::from("/tmp/foo/../cookies.txt");
        assert!(validate_cookies_path(&p).is_err());
    }
}
