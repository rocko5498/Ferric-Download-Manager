use anyhow::{Context as _, Result};
use ferric_common::{MediaKind, MediaProbeResponse, MediaTrack, MediaVariant};
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::path::{Path, PathBuf};

const HLS_MIME_TYPES: &[&str] = &["application/vnd.apple.mpegurl", "application/x-mpegurl"];
const DASH_MIME_TYPES: &[&str] = &["application/dash+xml"];
const MEDIA_SEGMENT_EXTENSIONS: &[&str] = &["ts", "m4s", "cmfv", "cmfa"];
const DIRECT_MEDIA_EXTENSIONS: &[&str] = &[
    "mp4", "mkv", "webm", "mov", "mp3", "m4a", "aac", "flac", "wav",
];

pub async fn probe_url(url: &str, client: &reqwest::Client) -> Result<MediaProbeResponse> {
    crate::security::validate_url(url)?;
    let extension_kind = classify_by_url(url);

    let response = client.head(url).send().await;
    let (status, mime_type) = match response {
        Ok(response) => {
            let status = response.status();
            let mime_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(|value| {
                    value
                        .split(';')
                        .next()
                        .unwrap_or(value)
                        .trim()
                        .to_ascii_lowercase()
                });
            (Some(status), mime_type)
        }
        Err(err) => {
            return Ok(MediaProbeResponse {
                url: url.to_string(),
                kind: MediaKind::Unsupported,
                mime_type: None,
                supported: false,
                variants: Vec::new(),
                audio_tracks: Vec::new(),
                subtitle_tracks: Vec::new(),
                explanation: format!("Could not probe media URL: {err}"),
            });
        }
    };

    if status.is_some_and(|status| {
        status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN
    }) {
        return Ok(explain(url, MediaKind::ProtectedOrExpired, mime_type, false, Vec::new(), Vec::new(), Vec::new(), "The server rejected the media probe. The URL may require fresh browser credentials, may be expired, or may be protected."));
    }
    if status.is_some_and(|status| {
        status == reqwest::StatusCode::NOT_FOUND || status == reqwest::StatusCode::GONE
    }) {
        return Ok(explain(
            url,
            MediaKind::ProtectedOrExpired,
            mime_type,
            false,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            "The media URL is no longer available. Signed or temporary links may have expired.",
        ));
    }

    let kind = classify_by_mime(mime_type.as_deref()).unwrap_or(extension_kind);
    match kind {
        MediaKind::HlsManifest => {
            let text = client.get(url).send().await?.error_for_status()?.text().await?;
            let variants = parse_hls_variants(url, text.as_bytes())?;
            let (audio, subtitles) = parse_hls_alternatives(url, text.as_bytes())?;
            Ok(explain(url, kind, mime_type, true, variants, audio, subtitles, "HLS manifest detected. Ferric can download legal non-DRM variants from this playlist."))
        }
        MediaKind::DashManifest => {
            let text = client.get(url).send().await?.error_for_status()?.text().await?;
            let (variants, audio, subtitles) = parse_dash_all(url, &text)?;
            Ok(explain(url, kind, mime_type, true, variants, audio, subtitles, "DASH manifest detected. Ferric can download supported non-DRM representations from this MPD."))
        }
        MediaKind::DirectFile => Ok(explain(url, kind, mime_type, true, Vec::new(), Vec::new(), Vec::new(), "Direct media file detected.")),
        MediaKind::MediaSegment => Ok(explain(url, kind, mime_type, false, Vec::new(), Vec::new(), Vec::new(), "This looks like an individual media segment. Capture the parent HLS/DASH manifest instead.")),
        MediaKind::ProtectedOrExpired | MediaKind::Unsupported => Ok(explain(url, MediaKind::Unsupported, mime_type, false, Vec::new(), Vec::new(), Vec::new(), "This URL is not a supported direct media file, HLS playlist, or DASH manifest.")),
    }
}

pub fn classify_by_url(url: &str) -> MediaKind {
    let extension = url::Url::parse(url)
        .ok()
        .and_then(|url| {
            Path::new(url.path())
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_ascii_lowercase)
        })
        .unwrap_or_default();

    match extension.as_str() {
        "m3u8" => MediaKind::HlsManifest,
        "mpd" => MediaKind::DashManifest,
        ext if MEDIA_SEGMENT_EXTENSIONS.contains(&ext) => MediaKind::MediaSegment,
        ext if DIRECT_MEDIA_EXTENSIONS.contains(&ext) => MediaKind::DirectFile,
        _ => MediaKind::Unsupported,
    }
}

fn classify_by_mime(mime_type: Option<&str>) -> Option<MediaKind> {
    let mime_type = mime_type?.to_ascii_lowercase();
    if HLS_MIME_TYPES.contains(&mime_type.as_str()) {
        Some(MediaKind::HlsManifest)
    } else if DASH_MIME_TYPES.contains(&mime_type.as_str()) {
        Some(MediaKind::DashManifest)
    } else if mime_type.starts_with("video/") || mime_type.starts_with("audio/") {
        Some(MediaKind::DirectFile)
    } else {
        None
    }
}

pub fn parse_hls_variants(manifest_url: &str, bytes: &[u8]) -> Result<Vec<MediaVariant>> {
    let playlist = m3u8_rs::parse_playlist_res(bytes)
        .map_err(|err| anyhow::anyhow!("parse HLS manifest: {err:?}"))?;
    let m3u8_rs::Playlist::MasterPlaylist(master) = playlist else {
        return Ok(Vec::new());
    };

    master
        .variants
        .iter()
        .enumerate()
        .map(|(index, variant)| {
            let width = variant
                .resolution
                .as_ref()
                .and_then(|resolution| u32::try_from(resolution.width).ok());
            let height = variant
                .resolution
                .as_ref()
                .and_then(|resolution| u32::try_from(resolution.height).ok());
            let label = variant_label(
                "HLS",
                variant.bandwidth.into(),
                width,
                height,
                variant.codecs.as_deref(),
            );
            Ok(MediaVariant {
                id: format!("hls-{index}"),
                label,
                url: crate::hls::resolve_url(manifest_url, &variant.uri)?,
                bandwidth: Some(variant.bandwidth),
                width,
                height,
                codecs: variant.codecs.clone(),
            })
        })
        .collect()
}

pub fn parse_hls_alternatives(
    manifest_url: &str,
    bytes: &[u8],
) -> Result<(Vec<MediaTrack>, Vec<MediaTrack>)> {
    let playlist = m3u8_rs::parse_playlist_res(bytes)
        .map_err(|err| anyhow::anyhow!("parse HLS manifest: {err:?}"))?;
    let m3u8_rs::Playlist::MasterPlaylist(master) = playlist else {
        return Ok((Vec::new(), Vec::new()));
    };

    let mut audio_tracks = Vec::new();
    let mut subtitle_tracks = Vec::new();

    for (index, alt) in master.alternatives.iter().enumerate() {
        if let Some(uri) = &alt.uri {
            let url = crate::hls::resolve_url(manifest_url, uri)?;
            let track = MediaTrack {
                id: format!("hls-alt-{index}"),
                label: alt.name.clone(),
                url,
                language: alt.language.clone(),
            };
            match alt.media_type {
                m3u8_rs::AlternativeMediaType::Audio => {
                    audio_tracks.push(track);
                }
                m3u8_rs::AlternativeMediaType::Subtitles => {
                    subtitle_tracks.push(track);
                }
                _ => {}
            }
        }
    }

    Ok((audio_tracks, subtitle_tracks))
}

struct AdaptationSetInfo {
    content_type: Option<String>,
    mime_type: Option<String>,
    lang: Option<String>,
}

pub fn parse_dash_all(
    manifest_url: &str,
    manifest: &str,
) -> Result<(Vec<MediaVariant>, Vec<MediaTrack>, Vec<MediaTrack>)> {
    let mut reader = Reader::from_str(manifest);
    reader.config_mut().trim_text(true);
    let mut variants = Vec::new();
    let mut audio_tracks = Vec::new();
    let mut subtitle_tracks = Vec::new();

    let mut base_stack: Vec<String> = Vec::new();
    let mut in_base_url = false;
    let mut base_text = String::new();
    let mut current_set: Option<AdaptationSetInfo> = None;

    loop {
        match reader.read_event()? {
            Event::Start(event) if event.name().as_ref() == b"AdaptationSet" => {
                let content_type = attr(&event, b"contentType");
                let mime_type = attr(&event, b"mimeType");
                let lang = attr(&event, b"lang");
                current_set = Some(AdaptationSetInfo {
                    content_type,
                    mime_type,
                    lang,
                });
            }
            Event::End(event) if event.name().as_ref() == b"AdaptationSet" => {
                current_set = None;
            }
            Event::Start(event) if event.name().as_ref() == b"BaseURL" => {
                in_base_url = true;
                base_text.clear();
            }
            Event::End(event) if event.name().as_ref() == b"BaseURL" => {
                if in_base_url {
                    base_stack.push(base_text.trim().to_string());
                    in_base_url = false;
                }
            }
            Event::Text(text) if in_base_url => {
                base_text.push_str(&text.decode()?);
            }
            Event::Start(event) | Event::Empty(event)
                if event.name().as_ref() == b"Representation" =>
            {
                let id = attr(&event, b"id").unwrap_or_else(|| {
                    format!(
                        "dash-{}",
                        variants.len() + audio_tracks.len() + subtitle_tracks.len()
                    )
                });
                let bandwidth =
                    attr(&event, b"bandwidth").and_then(|value| value.parse::<u64>().ok());
                let width = attr(&event, b"width").and_then(|value| value.parse::<u32>().ok());
                let height = attr(&event, b"height").and_then(|value| value.parse::<u32>().ok());
                let codecs = attr(&event, b"codecs");
                let base = base_stack.last().map_or("", String::as_str);
                let url = resolve_url(manifest_url, base)?;

                // Determine type
                let mut is_audio = false;
                let mut is_sub = false;
                let mut lang = None;

                if let Some(set) = &current_set {
                    lang = set.lang.clone();
                    if let Some(ct) = &set.content_type {
                        match ct.to_ascii_lowercase().as_str() {
                            "video" => {
                                is_audio = false;
                                is_sub = false;
                            }
                            "audio" => {
                                is_audio = true;
                                is_sub = false;
                            }
                            "text" | "subtitle" | "subtitles" => {
                                is_audio = false;
                                is_sub = true;
                            }
                            _ => {}
                        }
                    } else if let Some(mt) = &set.mime_type {
                        let mt = mt.to_ascii_lowercase();
                        if mt.starts_with("video/") {
                            is_audio = false;
                            is_sub = false;
                        } else if mt.starts_with("audio/") {
                            is_audio = true;
                            is_sub = false;
                        } else if mt.starts_with("text/") || mt.contains("ttml") {
                            is_audio = false;
                            is_sub = true;
                        }
                    }
                } else {
                    // Fallback heuristics
                    if width.is_none() && height.is_none() {
                        if let Some(c) = &codecs {
                            if c.to_ascii_lowercase().starts_with("mp4a") {
                                is_audio = true;
                            }
                        }
                    }
                }

                if is_sub {
                    let label = if let Some(ref l) = lang {
                        format!("Subtitle track: {l}")
                    } else {
                        format!("Subtitle track: {id}")
                    };
                    subtitle_tracks.push(MediaTrack {
                        id,
                        label,
                        url,
                        language: lang,
                    });
                } else if is_audio {
                    let rate_str =
                        bandwidth.map_or(String::new(), |b| format!(" ({} kbps)", b / 1000));
                    let label = if let Some(ref l) = lang {
                        format!("Audio track: {l}{rate_str}")
                    } else {
                        format!("Audio track: {id}{rate_str}")
                    };
                    audio_tracks.push(MediaTrack {
                        id,
                        label,
                        url,
                        language: lang,
                    });
                } else {
                    variants.push(MediaVariant {
                        id,
                        label: variant_label("DASH", bandwidth, width, height, codecs.as_deref()),
                        url,
                        bandwidth,
                        width,
                        height,
                        codecs,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    Ok((variants, audio_tracks, subtitle_tracks))
}

pub fn build_ffmpeg_merge_args(video: &Path, audio: &Path, output: &Path) -> Vec<String> {
    vec![
        "-hide_banner".to_string(),
        "-loglevel".to_string(),
        "error".to_string(),
        "-y".to_string(),
        "-i".to_string(),
        video.to_string_lossy().to_string(),
        "-i".to_string(),
        audio.to_string_lossy().to_string(),
        "-c".to_string(),
        "copy".to_string(),
        output.to_string_lossy().to_string(),
    ]
}

pub fn ffmpeg_path() -> Option<PathBuf> {
    which::which("ffmpeg").ok()
}

pub fn validate_merge_paths(
    video: &Path,
    audio: &Path,
    output: &Path,
    save_dir: &Path,
) -> Result<()> {
    let video = video
        .canonicalize()
        .with_context(|| format!("canonicalize video stream {}", video.display()))?;
    let audio = audio
        .canonicalize()
        .with_context(|| format!("canonicalize audio stream {}", audio.display()))?;
    let save_dir = save_dir
        .canonicalize()
        .with_context(|| format!("canonicalize save directory {}", save_dir.display()))?;
    let output_parent = output
        .parent()
        .ok_or_else(|| anyhow::anyhow!("merge output has no parent directory"))?
        .canonicalize()
        .with_context(|| format!("canonicalize merge output parent {}", output.display()))?;
    let output_name = output
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("merge output has no file name"))?;
    let output_path = output_parent.join(output_name);

    if !output_parent.starts_with(&save_dir) {
        anyhow::bail!("merge output must stay inside the selected save directory");
    }
    if !video.starts_with(&save_dir) || !audio.starts_with(&save_dir) {
        anyhow::bail!("merge inputs must stay inside the selected save directory");
    }
    if output_path == video || output_path == audio {
        anyhow::bail!("merge output must not overwrite source streams");
    }
    Ok(())
}

pub async fn run_ffmpeg_merge(
    video: &Path,
    audio: &Path,
    output: &Path,
    save_dir: &Path,
) -> Result<()> {
    validate_merge_paths(video, audio, output, save_dir)?;
    let ffmpeg = ffmpeg_path()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg is not installed or is not available on PATH"))?;
    let args = build_ffmpeg_merge_args(video, audio, output);
    // Task 127 — sandboxed spawn (no window, stripped proxy env, kill_on_drop)
    let work = save_dir;
    let output_result = crate::ffmpeg_sandbox::run_sandboxed(&ffmpeg, &args, Some(work)).await?;

    if output_result.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output_result.stderr);
    let message = stderr.trim();
    if message.is_empty() {
        anyhow::bail!("ffmpeg merge failed with status {}", output_result.status);
    }
    anyhow::bail!("ffmpeg merge failed: {message}");
}

fn explain(
    url: &str,
    kind: MediaKind,
    mime_type: Option<String>,
    supported: bool,
    variants: Vec<MediaVariant>,
    audio_tracks: Vec<MediaTrack>,
    subtitle_tracks: Vec<MediaTrack>,
    explanation: &str,
) -> MediaProbeResponse {
    MediaProbeResponse {
        url: url.to_string(),
        kind,
        mime_type,
        supported,
        variants,
        audio_tracks,
        subtitle_tracks,
        explanation: explanation.to_string(),
    }
}

fn variant_label(
    prefix: &str,
    bandwidth: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
    codecs: Option<&str>,
) -> String {
    let mut parts = vec![prefix.to_string()];
    if let (Some(width), Some(height)) = (width, height) {
        parts.push(format!("{width}x{height}"));
    }
    if let Some(bandwidth) = bandwidth {
        parts.push(format!("{} kbps", bandwidth / 1000));
    }
    if let Some(codecs) = codecs.filter(|value| !value.is_empty()) {
        parts.push(codecs.to_string());
    }
    parts.join(" ")
}

fn resolve_url(manifest_url: &str, value: &str) -> Result<String> {
    let base =
        url::Url::parse(manifest_url).with_context(|| format!("parse media URL {manifest_url}"))?;
    Ok(base.join(value)?.to_string())
}

fn attr(event: &BytesStart<'_>, name: &[u8]) -> Option<String> {
    event.attributes().flatten().find_map(|attr| {
        (attr.key.as_ref() == name)
            .then(|| {
                attr.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                    .ok()
                    .map(std::borrow::Cow::into_owned)
            })
            .flatten()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_media_urls() {
        assert_eq!(
            classify_by_url("https://example.com/master.m3u8"),
            MediaKind::HlsManifest
        );
        assert_eq!(
            classify_by_url("https://example.com/manifest.mpd"),
            MediaKind::DashManifest
        );
        assert_eq!(
            classify_by_url("https://example.com/seg-1.m4s"),
            MediaKind::MediaSegment
        );
        assert_eq!(
            classify_by_url("https://example.com/movie.mp4"),
            MediaKind::DirectFile
        );
    }

    #[test]
    fn parses_hls_master_variants() -> Result<()> {
        let manifest = br#"#EXTM3U
#EXT-X-STREAM-INF:BANDWIDTH=800000,RESOLUTION=640x360,CODECS="avc1.4d401e,mp4a.40.2"
low/index.m3u8
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080
hi/index.m3u8
"#;
        let variants = parse_hls_variants("https://example.com/master.m3u8", manifest)?;
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0].height, Some(360));
        assert_eq!(variants[1].url, "https://example.com/hi/index.m3u8");
        Ok(())
    }

    #[test]
    fn parses_dash_representations_as_variants() -> Result<()> {
        let manifest = r#"
            <MPD>
              <Period>
                <AdaptationSet>
                  <BaseURL>video/</BaseURL>
                  <Representation id="v360" bandwidth="800000" width="640" height="360" codecs="avc1" />
                  <Representation id="v1080" bandwidth="2500000" width="1920" height="1080" codecs="avc1" />
                </AdaptationSet>
              </Period>
            </MPD>
        "#;
        let (variants, _, _) = parse_dash_all("https://example.com/manifest.mpd", manifest)?;
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[1].id, "v1080");
        assert_eq!(variants[1].height, Some(1080));
        assert_eq!(variants[1].url, "https://example.com/video/");
        Ok(())
    }

    #[test]
    fn parses_hls_alternatives_audio_subtitles() -> Result<()> {
        let manifest = br#"#EXTM3U
#EXT-X-MEDIA:TYPE=AUDIO,GROUP-ID="audio",NAME="English",DEFAULT=YES,AUTOSELECT=YES,LANGUAGE="en",URI="eng/prog_index.m3u8"
#EXT-X-MEDIA:TYPE=SUBTITLES,GROUP-ID="subs",NAME="French",DEFAULT=NO,AUTOSELECT=YES,LANGUAGE="fr",URI="sub/fra.m3u8"
#EXT-X-STREAM-INF:BANDWIDTH=2500000,RESOLUTION=1920x1080
hi/index.m3u8
"#;
        let (audio, subtitles) =
            parse_hls_alternatives("https://example.com/master.m3u8", manifest)?;
        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].label, "English");
        assert_eq!(audio[0].url, "https://example.com/eng/prog_index.m3u8");
        assert_eq!(audio[0].language.as_deref(), Some("en"));

        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].label, "French");
        assert_eq!(subtitles[0].url, "https://example.com/sub/fra.m3u8");
        assert_eq!(subtitles[0].language.as_deref(), Some("fr"));
        Ok(())
    }

    #[test]
    fn parses_dash_all_tracks() -> Result<()> {
        let manifest = r#"
            <MPD>
              <Period>
                <AdaptationSet contentType="video">
                  <BaseURL>video/</BaseURL>
                  <Representation id="v1080" bandwidth="2500000" width="1920" height="1080" codecs="avc1" />
                </AdaptationSet>
                <AdaptationSet contentType="audio" lang="en">
                  <BaseURL>audio/</BaseURL>
                  <Representation id="a1" bandwidth="128000" codecs="mp4a" />
                </AdaptationSet>
                <AdaptationSet contentType="text" lang="fr">
                  <BaseURL>sub/</BaseURL>
                  <Representation id="s1" />
                </AdaptationSet>
              </Period>
            </MPD>
        "#;
        let (variants, audio, subtitles) =
            parse_dash_all("https://example.com/manifest.mpd", manifest)?;
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0].height, Some(1080));

        assert_eq!(audio.len(), 1);
        assert_eq!(audio[0].language.as_deref(), Some("en"));
        assert_eq!(audio[0].url, "https://example.com/audio/");

        assert_eq!(subtitles.len(), 1);
        assert_eq!(subtitles[0].language.as_deref(), Some("fr"));
        assert_eq!(subtitles[0].url, "https://example.com/sub/");
        Ok(())
    }

    #[test]
    fn ffmpeg_merge_args_are_shell_free() {
        let args = build_ffmpeg_merge_args(
            Path::new("video track.mp4"),
            Path::new("audio & track.m4a"),
            Path::new("out file.mp4"),
        );
        assert!(args.contains(&"-y".to_string()));
        assert!(args.contains(&"-loglevel".to_string()));
        assert!(args.contains(&"audio & track.m4a".to_string()));
        assert!(!args.join(" ").contains('"'));
    }

    #[test]
    fn merge_paths_must_stay_inside_save_dir() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let video = dir.path().join("video.mp4");
        let audio = dir.path().join("audio.m4a");
        let outside = tempfile::tempdir()?;
        let output = outside.path().join("merged.mp4");
        std::fs::write(&video, b"video")?;
        std::fs::write(&audio, b"audio")?;

        let err = validate_merge_paths(&video, &audio, &output, dir.path())
            .expect_err("merge output outside save dir must fail");
        assert!(err
            .to_string()
            .contains("merge output must stay inside the selected save directory"));
        Ok(())
    }

    #[test]
    fn merge_paths_refuse_overwriting_source_stream() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let video = dir.path().join("video.mp4");
        let audio = dir.path().join("audio.m4a");
        std::fs::write(&video, b"video")?;
        std::fs::write(&audio, b"audio")?;

        let err = validate_merge_paths(&video, &audio, &video, dir.path())
            .expect_err("merge output must not overwrite input");
        assert!(err
            .to_string()
            .contains("merge output must not overwrite source streams"));
        Ok(())
    }
}
