use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    (u.contains("twitter.com/") || u.contains("x.com/")) && u.contains("/status/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let tweet_id =
        extract_id(url).ok_or_else(|| anyhow::anyhow!("cannot extract tweet ID from {url}"))?;

    // Twitter syndication API — public, no auth required for public tweets
    let api_url =
        format!("https://cdn.syndication.twimg.com/tweet-result?id={tweet_id}&lang=en&token=0");
    let resp = client
        .get(&api_url)
        .header("Origin", "https://platform.twitter.com")
        .header("Referer", "https://platform.twitter.com/")
        .send()
        .await
        .with_context(|| format!("GET {api_url}"))?
        .error_for_status()
        .context("Twitter syndication API error")?;

    let tweet: TweetResult = resp.json().await.context("parse tweet JSON")?;

    let mut formats: Vec<ExtractedFormat> = tweet
        .media_details
        .iter()
        .flat_map(|m| m.iter())
        .flat_map(|media| {
            media
                .video_info
                .as_ref()
                .map(|vi| vi.variants.as_slice())
                .unwrap_or(&[])
        })
        .filter(|v| v.content_type == "video/mp4")
        .enumerate()
        .map(|(i, v)| {
            let height = v
                .url
                .split('/')
                .find(|s| s.contains('x'))
                .and_then(|dim| dim.split('x').nth(1))
                .and_then(|h| h.parse::<u32>().ok());
            let tbr = v.bitrate.map(|b| b as f64 / 1000.0);
            ExtractedFormat {
                id: format!("twitter-{i}"),
                label: height.map_or_else(|| "unknown".to_string(), |h| format!("{h}p")),
                url: v.url.clone(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: None,
                width: None,
                height,
                tbr_kbps: tbr,
                abr_kbps: None,
                vbr_kbps: tbr,
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

    if formats.is_empty() {
        bail!("Twitter: no video formats found in tweet {tweet_id}");
    }

    formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

    Ok(ExtractResult {
        title: tweet
            .text
            .clone()
            .unwrap_or_else(|| format!("Tweet {tweet_id}")),
        uploader: tweet.user.as_ref().map(|u| u.name.clone()),
        thumbnail: None,
        duration_secs: None,
        formats,
        extractor: "twitter".to_string(),
        description: tweet.text,
        upload_date: tweet.created_at.as_deref().map(to_upload_date),
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_id(url: &str) -> Option<String> {
    // https://twitter.com/user/status/1234567890
    // https://x.com/user/status/1234567890
    url.split("/status/")
        .nth(1)
        .map(|s| s.split(['?', '/']).next().unwrap_or(s).to_string())
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
}

fn to_upload_date(s: &str) -> String {
    // "Mon Jan 01 00:00:00 +0000 2024" → "20240101"
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() >= 6 {
        let months = [
            "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
        ];
        let year = parts[5];
        let month = months.iter().position(|&m| m == parts[1]).map(|i| i + 1);
        let day = parts[2].parse::<u32>().ok();
        if let (Some(m), Some(d)) = (month, day) {
            return format!("{year}{m:02}{d:02}");
        }
    }
    s.to_string()
}

// ── Deserialization ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct TweetResult {
    text: Option<String>,
    #[serde(rename = "created_at")]
    created_at: Option<String>,
    user: Option<TweetUser>,
    #[serde(rename = "mediaDetails")]
    media_details: Option<Vec<TweetMedia>>,
}

#[derive(Deserialize)]
struct TweetUser {
    name: String,
}

#[derive(Deserialize)]
struct TweetMedia {
    #[serde(rename = "video_info")]
    video_info: Option<VideoInfo>,
}

#[derive(Deserialize)]
struct VideoInfo {
    variants: Vec<VideoVariant>,
}

#[derive(Deserialize)]
struct VideoVariant {
    #[serde(rename = "content_type")]
    content_type: String,
    url: String,
    bitrate: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_twitter_status() {
        assert!(matches("https://twitter.com/user/status/1234567890"));
        assert!(matches("https://x.com/user/status/1234567890"));
        assert!(!matches("https://twitter.com/user"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_tweet_id() {
        assert_eq!(
            extract_id("https://twitter.com/user/status/1234567890"),
            Some("1234567890".to_string())
        );
        assert_eq!(
            extract_id("https://x.com/user/status/9876543210?s=20"),
            Some("9876543210".to_string())
        );
        assert_eq!(extract_id("https://twitter.com/user"), None);
    }

    #[test]
    fn upload_date_parses() {
        assert_eq!(to_upload_date("Mon Jan 01 00:00:00 +0000 2024"), "20240101");
        assert_eq!(to_upload_date("Fri Dec 31 23:59:59 +0000 2023"), "20231231");
    }
}
