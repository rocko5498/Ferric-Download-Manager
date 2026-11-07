use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;
use std::collections::HashMap;

pub fn matches(url: &str) -> bool {
    url.to_ascii_lowercase().contains("vimeo.com/")
}

/// Fetches public player config JSON. Auth is intentionally not supported.
#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let video_id = extract_id(url)
        .ok_or_else(|| anyhow::anyhow!("cannot extract Vimeo video ID from {url}"))?;

    let config_url = format!("https://player.vimeo.com/video/{video_id}/config");
    let resp = client
        .get(&config_url)
        .header("Referer", url)
        .send()
        .await
        .with_context(|| format!("GET {config_url}"))?
        .error_for_status()
        .context("Vimeo config API returned error status")?;

    let config: VimeoConfig = resp.json().await.context("parse Vimeo config JSON")?;
    let mut formats: Vec<ExtractedFormat> = config
        .request
        .files
        .progressive
        .into_iter()
        .map(|p| {
            let kbps = p.bitrate.map(|b| b as f64 / 1000.0);
            ExtractedFormat {
                id: p.quality.clone(),
                label: format_label(&p.quality, p.width, p.height, kbps),
                url: p.url,
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: Some(p.quality.clone()),
                width: p.width,
                height: p.height,
                tbr_kbps: kbps,
                abr_kbps: None,
                vbr_kbps: kbps,
                fps: None,
                vcodec: None,
                acodec: None,
                container: Some("mp4".to_string()),
                language: None,
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: false,
            }
        })
        .collect();

    if let Some(hls) = config.request.files.hls {
        if let Some(cdn_info) = hls
            .default_cdn
            .as_deref()
            .and_then(|cdn| hls.cdns.get(cdn))
            .or_else(|| hls.cdns.values().next())
        {
            formats.push(ExtractedFormat {
                id: "hls-master".to_string(),
                label: "HLS (adaptive quality)".to_string(),
                url: cdn_info.url.clone(),
                ext: "m3u8".to_string(),
                protocol: Some("m3u8".to_string()),
                manifest_url: Some(cdn_info.url.clone()),
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
        }
    }

    if formats.is_empty() {
        bail!("Vimeo: no downloadable formats found for video {video_id}");
    }

    formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

    Ok(ExtractResult {
        title: config.video.title,
        uploader: Some(config.video.owner.name).filter(|s| !s.is_empty()),
        thumbnail: config.video.thumbs.values().next().cloned(),
        duration_secs: Some(config.video.duration as f64),
        formats,
        extractor: "vimeo".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn format_label(
    quality: &str,
    width: Option<u32>,
    height: Option<u32>,
    kbps: Option<f64>,
) -> String {
    match (width, height, kbps) {
        (Some(w), Some(h), Some(b)) => format!("{quality} ({w}x{h}, {b:.0} kbps)"),
        (Some(w), Some(h), None) => format!("{quality} ({w}x{h})"),
        _ => quality.to_string(),
    }
}

fn extract_id(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    parsed
        .path_segments()?
        .find(|segment| segment.chars().all(|c| c.is_ascii_digit()))
        .map(ToString::to_string)
}

#[derive(Deserialize)]
struct VimeoConfig {
    video: VimeoVideo,
    request: VimeoRequest,
}

#[derive(Deserialize)]
struct VimeoVideo {
    title: String,
    duration: u64,
    thumbs: HashMap<String, String>,
    owner: VimeoOwner,
}

#[derive(Deserialize)]
struct VimeoOwner {
    name: String,
}

#[derive(Deserialize)]
struct VimeoRequest {
    files: VimeoFiles,
}

#[derive(Deserialize)]
struct VimeoFiles {
    #[serde(default)]
    progressive: Vec<VimeoProgressive>,
    hls: Option<VimeoHls>,
}

#[derive(Deserialize)]
struct VimeoProgressive {
    url: String,
    quality: String,
    width: Option<u32>,
    height: Option<u32>,
    bitrate: Option<u64>,
}

#[derive(Deserialize)]
struct VimeoHls {
    cdns: HashMap<String, VimeoCdn>,
    default_cdn: Option<String>,
}

#[derive(Deserialize)]
struct VimeoCdn {
    url: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_vimeo_urls() {
        assert!(matches("https://vimeo.com/123456789"));
        assert!(matches("https://player.vimeo.com/video/123456789"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_vimeo_id() {
        assert_eq!(
            extract_id("https://vimeo.com/123456789").as_deref(),
            Some("123456789")
        );
        assert_eq!(
            extract_id("https://player.vimeo.com/video/987654321").as_deref(),
            Some("987654321")
        );
        assert_eq!(
            extract_id("https://vimeo.com/channels/staff/123").as_deref(),
            Some("123")
        );
        assert_eq!(extract_id("https://vimeo.com/notavideo"), None);
    }

    #[test]
    fn formats_sorted_best_first() {
        let mut formats = vec![
            ExtractedFormat {
                id: "sd".to_string(),
                label: "480p".to_string(),
                url: "a".to_string(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: None,
                width: Some(854),
                height: Some(480),
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
            },
            ExtractedFormat {
                id: "hd".to_string(),
                label: "1080p".to_string(),
                url: "b".to_string(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: None,
                width: Some(1920),
                height: Some(1080),
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
            },
        ];

        formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

        assert_eq!(formats[0].id, "hd");
    }
}
