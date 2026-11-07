use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;

pub fn matches(url: &str) -> bool {
    url.to_ascii_lowercase().contains("dailymotion.com/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let video_id = extract_id(url)
        .ok_or_else(|| anyhow::anyhow!("Dailymotion: cannot extract video ID from {url}"))?;

    let meta_url = format!(
        "https://www.dailymotion.com/player/metadata/video/{video_id}?embedder=https://www.dailymotion.com&locale=en_US&dmV3=1"
    );

    let resp = client
        .get(&meta_url)
        .header("Referer", "https://www.dailymotion.com/")
        .send()
        .await
        .with_context(|| format!("GET {meta_url}"))?
        .error_for_status()
        .context("Dailymotion metadata API error")?;

    let meta: DailymotionMeta = resp.json().await.context("parse Dailymotion metadata")?;

    let mut formats = Vec::new();

    // Progressive qualities: 1080, 720, 480, 380, 240
    for (quality, streams) in &meta.qualities {
        if quality == "auto" {
            continue;
        }
        let height = quality.trim_end_matches('p').parse::<u32>().ok();
        for stream in streams {
            if stream.mime_type.contains("video/mp4") || stream.mime_type.is_empty() {
                formats.push(ExtractedFormat {
                    id: format!("dm-{quality}"),
                    label: format!("{quality}p"),
                    url: stream.url.clone(),
                    ext: "mp4".to_string(),
                    protocol: Some("https".to_string()),
                    manifest_url: None,
                    format_note: Some(quality.clone()),
                    width: None,
                    height,
                    tbr_kbps: None,
                    abr_kbps: None,
                    vbr_kbps: None,
                    fps: None,
                    vcodec: None,
                    acodec: None,
                    container: Some("mp4".to_string()),
                    language: None,
                    filesize: None,
                    has_drm: false,
                    headers: Vec::new(),
                    is_audio_only: false,
                });
                break;
            }
        }
    }

    // HLS auto-quality stream
    if let Some(auto) = meta.qualities.get("auto") {
        for stream in auto {
            if stream.url.contains(".m3u8") || stream.mime_type.contains("mpegurl") {
                formats.push(ExtractedFormat {
                    id: "hls-auto".to_string(),
                    label: "HLS (adaptive)".to_string(),
                    url: stream.url.clone(),
                    ext: "m3u8".to_string(),
                    protocol: Some("m3u8".to_string()),
                    manifest_url: Some(stream.url.clone()),
                    format_note: Some("adaptive".to_string()),
                    width: None,
                    height: None,
                    tbr_kbps: None,
                    abr_kbps: None,
                    vbr_kbps: None,
                    fps: None,
                    vcodec: None,
                    acodec: None,
                    container: None,
                    language: None,
                    filesize: None,
                    has_drm: false,
                    headers: Vec::new(),
                    is_audio_only: false,
                });
                break;
            }
        }
    }

    if formats.is_empty() {
        bail!("Dailymotion: no formats found for video {video_id}");
    }

    formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

    Ok(ExtractResult {
        title: meta
            .title
            .unwrap_or_else(|| format!("Dailymotion video {video_id}")),
        uploader: meta.owner.as_ref().map(|o| o.screenname.clone()),
        thumbnail: meta.thumbnail_url,
        duration_secs: meta.duration.map(|d| d as f64),
        formats,
        extractor: "dailymotion".to_string(),
        description: meta.description,
        upload_date: meta.created_time.map(|ts| unix_to_ymd(ts)),
        tags: meta.tags.unwrap_or_default(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_id(url: &str) -> Option<String> {
    // https://www.dailymotion.com/video/x7abc12
    // https://www.dailymotion.com/embed/video/x7abc12
    url.split("/video/")
        .nth(1)
        .map(|s| s.split(['?', '_', '/']).next().unwrap_or(s).to_string())
        .filter(|s| !s.is_empty())
}

fn unix_to_ymd(ts: u64) -> String {
    // Same minimal calendar as tiktok.rs
    let days = ts / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}{m:02}{d:02}")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let diy = if leap { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

#[derive(Deserialize)]
struct DailymotionMeta {
    title: Option<String>,
    description: Option<String>,
    duration: Option<u64>,
    #[serde(rename = "thumbnailURL")]
    thumbnail_url: Option<String>,
    owner: Option<DailymotionOwner>,
    #[serde(rename = "createdTime")]
    created_time: Option<u64>,
    tags: Option<Vec<String>>,
    qualities: std::collections::HashMap<String, Vec<DailymotionStream>>,
}

#[derive(Deserialize)]
struct DailymotionOwner {
    screenname: String,
}

#[derive(Deserialize)]
struct DailymotionStream {
    url: String,
    #[serde(rename = "type", default)]
    mime_type: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_dailymotion() {
        assert!(matches("https://www.dailymotion.com/video/x7abc12"));
        assert!(matches("https://dailymotion.com/embed/video/x7abc12"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_video_id() {
        assert_eq!(
            extract_id("https://www.dailymotion.com/video/x7abc12"),
            Some("x7abc12".to_string())
        );
        assert_eq!(
            extract_id("https://www.dailymotion.com/video/x7abc12_title"),
            Some("x7abc12".to_string())
        );
    }

    #[test]
    fn unix_ymd_epoch() {
        assert_eq!(unix_to_ymd(0), "19700101");
        assert_eq!(unix_to_ymd(86400 * 365), "19710101");
    }
}
