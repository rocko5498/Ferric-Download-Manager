use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("reddit.com/r/") || u.contains("redd.it/") || u.contains("v.redd.it/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let json_url = to_json_url(url)?;
    let resp = client
        .get(&json_url)
        .header("User-Agent", "ferric-extract/0.1")
        .send()
        .await
        .with_context(|| format!("GET {json_url}"))?
        .error_for_status()
        .context("Reddit API returned error status")?;

    let root: Vec<RedditListing> = resp.json().await.context("parse Reddit JSON")?;
    let post = root
        .first()
        .and_then(|listing| listing.data.children.first())
        .map(|child| &child.data)
        .ok_or_else(|| anyhow::anyhow!("Reddit: no post data in response"))?;
    let reddit_video = post
        .media
        .as_ref()
        .and_then(|media| media.reddit_video.as_ref())
        .ok_or_else(|| anyhow::anyhow!("Reddit: post has no reddit_video media"))?;

    let mut formats = Vec::new();
    if let Some(dash_url) = &reddit_video.dash_url {
        formats.push(ExtractedFormat {
            id: "dash".to_string(),
            label: "DASH (adaptive)".to_string(),
            url: dash_url.clone(),
            ext: "mpd".to_string(),
            protocol: Some("dash".to_string()),
            manifest_url: Some(dash_url.clone()),
            format_note: Some("adaptive".to_string()),
            width: reddit_video.width,
            height: reddit_video.height,
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
    if let Some(hls_url) = &reddit_video.hls_url {
        formats.push(ExtractedFormat {
            id: "hls".to_string(),
            label: "HLS (adaptive)".to_string(),
            url: hls_url.clone(),
            ext: "m3u8".to_string(),
            protocol: Some("m3u8".to_string()),
            manifest_url: Some(hls_url.clone()),
            format_note: Some("adaptive".to_string()),
            width: reddit_video.width,
            height: reddit_video.height,
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
    if let Some(fallback_url) = &reddit_video.fallback_url {
        let clean_url = fallback_url.split('?').next().unwrap_or(fallback_url);
        let height_label = reddit_video
            .height
            .map_or_else(String::new, |h| format!("{h}p "));
        formats.push(ExtractedFormat {
            id: "fallback".to_string(),
            label: format!("{height_label}MP4 (video only, no audio)"),
            url: clean_url.to_string(),
            ext: "mp4".to_string(),
            protocol: Some("https".to_string()),
            manifest_url: None,
            format_note: Some("video only".to_string()),
            width: reddit_video.width,
            height: reddit_video.height,
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
    }

    if formats.is_empty() {
        bail!("Reddit: no video formats found for {url}");
    }

    Ok(ExtractResult {
        title: post.title.clone(),
        uploader: post.author.as_ref().map(|author| format!("u/{author}")),
        thumbnail: post.thumbnail.clone().filter(|t| t.starts_with("http")),
        duration_secs: reddit_video.duration.map(|d| d as f64),
        formats,
        extractor: "reddit".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn to_json_url(url: &str) -> Result<String> {
    if url.to_ascii_lowercase().contains("v.redd.it/") {
        bail!("Reddit: v.redd.it URLs require full reddit.com post URL");
    }
    let base = url.split('?').next().unwrap_or(url).trim_end_matches('/');
    Ok(format!("{base}.json"))
}

#[derive(Deserialize)]
struct RedditListing {
    data: RedditListingData,
}

#[derive(Deserialize)]
struct RedditListingData {
    children: Vec<RedditChild>,
}

#[derive(Deserialize)]
struct RedditChild {
    data: RedditPost,
}

#[derive(Deserialize)]
struct RedditPost {
    title: String,
    author: Option<String>,
    thumbnail: Option<String>,
    media: Option<RedditMedia>,
}

#[derive(Deserialize)]
struct RedditMedia {
    reddit_video: Option<RedditVideo>,
}

#[derive(Deserialize)]
struct RedditVideo {
    fallback_url: Option<String>,
    dash_url: Option<String>,
    hls_url: Option<String>,
    duration: Option<u64>,
    width: Option<u32>,
    height: Option<u32>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reddit_urls() {
        assert!(matches(
            "https://www.reddit.com/r/videos/comments/abc/title/"
        ));
        assert!(matches("https://redd.it/abc123"));
        assert!(matches("https://v.redd.it/abc123"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn to_json_url_appends_suffix() {
        let url = to_json_url("https://www.reddit.com/r/videos/comments/abc/title/?x=1").unwrap();
        assert!(url.ends_with(".json"));
        assert!(!url.contains('?'));
    }

    #[test]
    fn to_json_url_rejects_bare_video_host() {
        assert!(to_json_url("https://v.redd.it/abc123").is_err());
    }
}
