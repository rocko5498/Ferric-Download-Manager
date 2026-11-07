//! Page-URL downloads for the browser panel (YouTube and other extractable sites): list the
//! formats yt-dlp / the native extractors find, and queue the one the user picked. Formats flagged
//! as protected are never listed and never downloaded.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use ferric_common::media::{ExtractResult, ExtractedFormat};
use ferric_common::messages::{DaemonReply, DownloadRequest};
use serde_json::json;
use sqlx::SqlitePool;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;

use crate::{manager::ManagerCommand, security};

/// Extraction takes seconds (yt-dlp), and the panel asks twice: once to list the qualities and once
/// to download. Results are kept briefly per page, and concurrent requests for the same page share
/// one extraction. Stream URLs stay valid far longer than this.
const CACHE_TTL: Duration = Duration::from_secs(300);
const CACHE_MAX: usize = 32;

type Slot = Arc<tokio::sync::Mutex<Option<(Instant, Arc<ExtractResult>)>>>;
static CACHE: LazyLock<tokio::sync::Mutex<HashMap<String, Slot>>> =
    LazyLock::new(|| tokio::sync::Mutex::new(HashMap::new()));

async fn cached<F, Fut>(key: &str, fetch: F) -> Result<Arc<ExtractResult>, String>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<ExtractResult, String>>,
{
    let slot = {
        let mut map = CACHE.lock().await;
        if map.len() >= CACHE_MAX && !map.contains_key(key) {
            // Drop the entries nobody is waiting on that are already stale.
            map.retain(|_, s| {
                s.try_lock().map_or(true, |g| {
                    g.as_ref().is_some_and(|(t, _)| t.elapsed() < CACHE_TTL)
                })
            });
        }
        map.entry(key.to_string()).or_default().clone()
    };
    let mut guard = slot.lock().await;
    if let Some((at, result)) = guard.as_ref() {
        if at.elapsed() < CACHE_TTL {
            return Ok(result.clone());
        }
    }
    let fresh = Arc::new(fetch().await?);
    *guard = Some((Instant::now(), fresh.clone()));
    Ok(fresh)
}

async fn extract(url: &str, pool: &SqlitePool) -> Result<Arc<ExtractResult>, String> {
    security::validate_url(url).map_err(|e| e.to_string())?;
    cached(url, || extract_uncached(url, pool)).await
}

fn is_manifest_url(url: &str) -> bool {
    let path = url
        .split(['?', '#'])
        .next()
        .unwrap_or(url)
        .to_ascii_lowercase();
    path.ends_with(".m3u8") || path.ends_with(".mpd")
}

/// A bare playlist address needs no site extractor: its own qualities are listed (and DRM-protected
/// manifests are dropped by the parser).
async fn from_manifest(client: &reqwest::Client, url: &str) -> Option<ExtractResult> {
    let mut formats = ferric_extract::manifest::formats_from_manifest_url(client, url).await?;
    formats.retain(|f| !f.has_drm);
    if formats.is_empty() {
        return None;
    }
    let title = url
        .split(['?', '#'])
        .next()
        .and_then(|p| p.rsplit('/').find(|s| !s.is_empty()))
        .map(|s| s.rsplit_once('.').map_or(s, |(stem, _)| stem).to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "video".to_string());
    Some(ExtractResult {
        title,
        uploader: None,
        thumbnail: None,
        duration_secs: None,
        formats,
        extractor: "manifest".into(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

async fn extract_uncached(url: &str, pool: &SqlitePool) -> Result<ExtractResult, String> {
    // The same client the downloader uses: redirects and DNS answers are checked against the SSRF rules.
    let client = crate::manager::build_reqwest_client(None).map_err(|e| e.to_string())?;
    if is_manifest_url(url) {
        if let Some(result) = from_manifest(&client, url).await {
            return Ok(result);
        }
    }
    let cfg = crate::api::extract_config_from_settings(pool, false, None).await;
    let mut result = ferric_extract::extract_with_config(url, &client, cfg)
        .await
        .map_err(|e| format!("extraction failed: {e}"))?;
    result.formats.retain(|f| !f.has_drm);
    if result.formats.is_empty() {
        return Err("no downloadable formats found on this page".into());
    }
    Ok(result)
}

fn has_video(f: &ExtractedFormat) -> bool {
    !f.is_audio_only && f.vcodec.as_deref().is_none_or(|c| c != "none")
}

fn has_audio(f: &ExtractedFormat) -> bool {
    f.is_audio_only || f.acodec.as_deref().is_some_and(|c| c != "none")
}

fn best_audio(formats: &[ExtractedFormat]) -> Option<&ExtractedFormat> {
    formats.iter().filter(|f| f.is_audio_only).max_by(|a, b| {
        a.abr_kbps
            .partial_cmp(&b.abr_kbps)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

/// The audio for one video: when a playlist says which audio group the video plays with, only that
/// group's tracks qualify (so an AAC variant is not paired with an E-AC-3 track); otherwise the best.
fn audio_for<'a>(
    video: &ExtractedFormat,
    formats: &'a [ExtractedFormat],
) -> Option<&'a ExtractedFormat> {
    let group = video
        .format_note
        .as_deref()
        .filter(|n| n.starts_with("audio-group:"));
    let same_group: Vec<&ExtractedFormat> = formats
        .iter()
        .filter(|f| f.is_audio_only && group.is_some() && f.format_note.as_deref() == group)
        .collect();
    if same_group.is_empty() {
        return best_audio(formats);
    }
    same_group.into_iter().max_by(|a, b| {
        a.abr_kbps
            .partial_cmp(&b.abr_kbps)
            .unwrap_or(std::cmp::Ordering::Equal)
    })
}

fn is_direct(f: &ExtractedFormat) -> bool {
    matches!(f.protocol.as_deref(), None | Some("https" | "http"))
}

/// Default pick: the tallest direct HTTP stream (MP4 over WebM, then bitrate), paired with the
/// matching audio when it is video-only. Streams behind a manifest are only a fallback.
fn best_direct(formats: &[ExtractedFormat]) -> Option<(ExtractedFormat, Option<ExtractedFormat>)> {
    let direct: Vec<&ExtractedFormat> = formats.iter().filter(|f| is_direct(f)).collect();
    let video = direct
        .iter()
        .copied()
        .filter(|f| has_video(f))
        .max_by(|a, b| {
            let key = |f: &ExtractedFormat| {
                (
                    f.height.unwrap_or(0),
                    f.ext == "mp4",
                    f.tbr_kbps.unwrap_or(0.0) as u64,
                )
            };
            key(a).cmp(&key(b))
        })?;
    if has_audio(video) {
        return Some((video.clone(), None));
    }
    let audio = direct
        .iter()
        .copied()
        .filter(|f| f.is_audio_only)
        .max_by(|a, b| {
            let key = |f: &ExtractedFormat| {
                (
                    // m4a goes with mp4 video, anything else with webm.
                    (f.ext == "m4a") == (video.ext == "mp4"),
                    f.abr_kbps.unwrap_or(0.0) as u64,
                )
            };
            key(a).cmp(&key(b))
        })?;
    Some((video.clone(), Some(audio.clone())))
}

/// JSON the extension shows in its panel: one row per downloadable format, best first.
pub async fn formats(url: String, pool: &SqlitePool) -> Result<String, String> {
    let result = extract(&url, pool).await?;
    let audio_size = best_audio(&result.formats).and_then(|a| a.filesize);
    let mut rows: Vec<serde_json::Value> = result
        .formats
        .iter()
        .filter(|f| {
            matches!(
                f.protocol.as_deref(),
                None | Some("https" | "http" | "m3u8" | "m3u8_native" | "dash")
            )
        })
        .map(|f| {
            let merge = has_video(f) && !has_audio(f);
            let size = match (f.filesize, merge, audio_size) {
                (Some(v), true, Some(a)) => Some(v + a),
                (v, _, _) => v,
            };
            json!({
                "id": f.id, "label": f.label, "ext": f.ext, "height": f.height, "fps": f.fps,
                "size": size, "audio_only": f.is_audio_only, "merge": merge,
                "abr": f.abr_kbps, "tbr": f.tbr_kbps,
                "vcodec": f.vcodec, "acodec": f.acodec, "protocol": f.protocol,
            })
        })
        .collect();
    // Video first (tallest on top, then the higher bitrate), then audio-only.
    rows.sort_by_key(|v| {
        std::cmp::Reverse((
            !v["audio_only"].as_bool().unwrap_or(false),
            v["height"].as_u64().unwrap_or(0),
            v["tbr"].as_f64().unwrap_or(0.0) as u64,
        ))
    });
    Ok(json!({
        "title": result.title,
        "extractor": result.extractor,
        "thumbnail": result.thumbnail,
        "duration": result.duration_secs,
        "formats": rows,
        "subtitles": result
            .subtitles
            .iter()
            .map(|s| json!({"lang": s.lang, "auto": s.is_auto_generated}))
            .collect::<Vec<_>>(),
    })
    .to_string())
}

/// Queue the chosen format (best quality when `format_id` is None).
pub async fn run(
    url: String,
    format_id: Option<String>,
    cookies: Option<String>,
    referrer: Option<String>,
    title: Option<String>,
    manager_tx: &mpsc::Sender<ManagerCommand>,
    pool: &SqlitePool,
) -> DaemonReply {
    let id = Uuid::new_v4();
    let fail = |message: String| DaemonReply::Error { id, message };
    let result = match extract(&url, pool).await {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let (video, audio) = match format_id.as_deref() {
        Some(fid) => {
            let Some(f) = result.formats.iter().find(|f| f.id == fid) else {
                return fail(format!("format {fid} not available"));
            };
            let audio = (has_video(f) && !has_audio(f))
                .then(|| audio_for(f, &result.formats))
                .flatten();
            (f.clone(), audio.cloned())
        }
        None => match best_direct(&result.formats) {
            Some(pair) => pair,
            None => {
                match ferric_extract::selector::select(&result.formats, "bestvideo+bestaudio/best")
                {
                    Ok(ferric_extract::selector::FormatSelection::Single(v)) => (v.clone(), None),
                    Ok(ferric_extract::selector::FormatSelection::Merge { video, audio }) => {
                        (video.clone(), Some(audio.clone()))
                    }
                    Err(e) => return fail(format!("no format selected: {e}")),
                }
            }
        },
    };
    if let Err(e) = security::validate_url(&video.url) {
        return fail(e.to_string());
    }
    if let Some(a) = &audio {
        if let Err(e) = security::validate_url(&a.url) {
            return fail(e.to_string());
        }
    }
    let ext = if video.ext.is_empty() || video.ext == "m3u8" {
        "mp4"
    } else {
        video.ext.as_str()
    };
    // A bare playlist is named after the page it was found on, not after "master".
    let name = match (&title, result.extractor.as_str()) {
        (Some(t), "manifest") if !t.trim().is_empty() => t.trim().to_string(),
        _ => result.title.clone(),
    };
    let filename = security::safe_filename(&format!("{name}.{ext}"), std::path::Path::new("."))
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()));
    let req = DownloadRequest {
        id,
        url: video.url.clone(),
        final_url: None,
        filename,
        referrer: referrer.or(Some(url)),
        cookies,
        headers: None,
        size_hint: video.filesize,
        mime_type: None,
        method: "GET".into(),
        post_body_b64: None,
        queue_id: None,
        checksum: None,
        mirrors: Vec::new(),
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
    };
    let (tx, rx) = oneshot::channel();
    let cmd = ManagerCommand::Add {
        req,
        save_dir_override: None,
        hold: crate::db::settings::get_flag(pool, "show_start_dialog", true).await,
        audio_url: audio.map(|a| a.url),
        subtitle_url: None,
        subtitle_lang: None,
        proxy_url: None,
        reply_tx: tx,
    };
    if manager_tx.send(cmd).await.is_err() {
        return fail("daemon shutting down".into());
    }
    rx.await
        .unwrap_or_else(|_| fail("manager dropped reply channel".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fmt(
        id: &str,
        ext: &str,
        proto: &str,
        height: Option<u32>,
        audio_only: bool,
        tbr: f64,
    ) -> ExtractedFormat {
        ExtractedFormat {
            id: id.into(),
            label: id.into(),
            url: format!("https://cdn.example/{id}"),
            ext: ext.into(),
            protocol: Some(proto.into()),
            manifest_url: None,
            format_note: None,
            width: None,
            height,
            tbr_kbps: Some(tbr),
            abr_kbps: audio_only.then_some(tbr),
            vbr_kbps: None,
            fps: None,
            vcodec: Some(if audio_only { "none" } else { "avc1" }.into()),
            acodec: Some(if audio_only { "mp4a" } else { "none" }.into()),
            container: None,
            language: None,
            filesize: None,
            has_drm: false,
            headers: Vec::new(),
            is_audio_only: audio_only,
        }
    }

    #[test]
    fn default_pick_prefers_direct_streams_over_a_manifest_of_the_same_height() {
        let formats = vec![
            fmt("hls2160", "mp4", "m3u8_native", Some(2160), false, 20000.0),
            fmt("webm2160", "webm", "https", Some(2160), false, 15000.0),
            fmt("mp42160", "mp4", "https", Some(2160), false, 9000.0),
            fmt("mp41080", "mp4", "https", Some(1080), false, 4000.0),
            fmt("opus", "webm", "https", None, true, 130.0),
            fmt("m4a", "m4a", "https", None, true, 128.0),
        ];
        let (video, audio) = best_direct(&formats).unwrap();
        assert_eq!(
            video.id, "mp42160",
            "tallest direct, mp4 preferred over webm"
        );
        assert_eq!(
            audio.unwrap().id,
            "m4a",
            "m4a pairs with mp4 even at a lower bitrate"
        );
    }

    #[tokio::test]
    async fn concurrent_and_repeated_requests_share_one_extraction() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let calls = Arc::new(AtomicUsize::new(0));
        let key = "https://test.invalid/shared";
        let make = |calls: Arc<AtomicUsize>| async move {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::time::sleep(Duration::from_millis(50)).await;
            Ok(ExtractResult {
                title: "t".into(),
                uploader: None,
                thumbnail: None,
                duration_secs: None,
                formats: vec![],
                extractor: "test".into(),
                description: None,
                upload_date: None,
                tags: vec![],
                subtitles: vec![],
                playlist_entries: vec![],
                chapters: vec![],
            })
        };
        let (a, b) = tokio::join!(
            cached(key, || make(calls.clone())),
            cached(key, || make(calls.clone()))
        );
        assert!(a.is_ok() && b.is_ok());
        assert!(cached(key, || make(calls.clone())).await.is_ok());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "one extraction serves every request"
        );
    }

    #[test]
    fn a_variant_is_paired_with_the_audio_of_its_own_group() {
        let mut aac_video = fmt("v-aac", "m3u8", "m3u8", Some(720), false, 3000.0);
        aac_video.acodec = Some("none".into());
        aac_video.format_note = Some("audio-group:aud1".into());
        let mut aac = fmt("a-aac", "m3u8", "m3u8", None, true, 96.0);
        aac.format_note = Some("audio-group:aud1".into());
        let mut ac3 = fmt("a-ac3", "m3u8", "m3u8", None, true, 128.0);
        ac3.format_note = Some("audio-group:aud2".into());
        let all = vec![aac_video.clone(), aac.clone(), ac3];
        assert_eq!(
            audio_for(&aac_video, &all).unwrap().id,
            "a-aac",
            "own group wins even at a lower bitrate"
        );
        let plain = fmt("v", "mp4", "https", Some(720), false, 1000.0);
        assert_eq!(
            audio_for(&plain, &all).unwrap().id,
            "a-ac3",
            "no group tag: the best audio"
        );
    }

    #[test]
    fn default_pick_is_none_when_only_manifests_exist() {
        let formats = vec![fmt("hls", "mp4", "m3u8_native", Some(720), false, 3000.0)];
        assert!(best_direct(&formats).is_none());
    }
}
