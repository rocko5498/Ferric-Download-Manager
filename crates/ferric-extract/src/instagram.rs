use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

use crate::cookies;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("instagram.com/p/")
        || u.contains("instagram.com/reel/")
        || u.contains("instagram.com/reels/")
        || u.contains("instagram.com/tv/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let shortcode = extract_shortcode(url)
        .ok_or_else(|| anyhow::anyhow!("cannot extract Instagram shortcode from {url}"))?;

    let browser_cookies = cookies::read_cookies("instagram.com").await;
    let cookie_header = cookies::to_header(&browser_cookies);

    let mut builder = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header(
            "Accept",
            "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .header("Referer", "https://www.instagram.com/");

    if !cookie_header.is_empty() {
        builder = builder.header("Cookie", &cookie_header);
    }

    let html = builder
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("Instagram returned error status")?
        .text()
        .await
        .context("read Instagram page")?;

    let video_url = find_video_url(&html)
        .ok_or_else(|| anyhow::anyhow!("Instagram: no video URL in page (login required?)"))?;

    if video_url.is_empty() {
        bail!("Instagram: empty video URL extracted");
    }

    let title = find_og(&html, "og:title").unwrap_or_else(|| format!("Instagram - {shortcode}"));
    let thumbnail = find_og(&html, "og:image");
    let description = find_og(&html, "og:description");

    let headers: Vec<(String, String)> = if cookie_header.is_empty() {
        Vec::new()
    } else {
        vec![("Cookie".to_string(), cookie_header)]
    };

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail,
        duration_secs: None,
        formats: vec![ExtractedFormat {
            id: "video".to_string(),
            label: "Video".to_string(),
            url: video_url,
            ext: "mp4".to_string(),
            protocol: Some("https".to_string()),
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
            container: Some("mp4".to_string()),
            language: None,
            filesize: None,
            has_drm: false,
            headers,
            is_audio_only: false,
        }],
        extractor: "instagram".to_string(),
        description,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

/// Tries og:video first, then scans page JSON blobs for "video_url" / "playable_url".
fn find_video_url(html: &str) -> Option<String> {
    if let Some(url) = find_og(html, "og:video") {
        if url.contains("mp4") || url.contains("video") {
            return Some(url);
        }
    }
    for key in &["video_url", "playable_url"] {
        if let Some(url) = find_json_str(html, key) {
            if url.starts_with("http") {
                return Some(url);
            }
        }
    }
    None
}

fn find_og(html: &str, property: &str) -> Option<String> {
    let needle = format!("property=\"{property}\" content=\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let v = &rest[..end];
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

fn find_json_str(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let url = rest[..end].replace("\\u0026", "&").replace("\\/", "/");
    if url.is_empty() {
        None
    } else {
        Some(url)
    }
}

fn extract_shortcode(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let segments: Vec<&str> = parsed.path_segments()?.collect();
    let idx = segments
        .iter()
        .position(|s| matches!(*s, "p" | "reel" | "reels" | "tv"))?;
    segments
        .get(idx + 1)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_instagram_urls() {
        assert!(matches("https://www.instagram.com/p/CxYZabc123/"));
        assert!(matches("https://www.instagram.com/reel/CxYZabc123/"));
        assert!(matches("https://www.instagram.com/tv/CxYZabc123/"));
        assert!(!matches("https://www.youtube.com/watch?v=abc"));
        assert!(!matches("https://www.facebook.com/watch/?v=123"));
    }

    #[test]
    fn extracts_shortcode() {
        assert_eq!(
            extract_shortcode("https://www.instagram.com/p/CxYZabc123/"),
            Some("CxYZabc123".to_string())
        );
        assert_eq!(
            extract_shortcode("https://www.instagram.com/reel/Def456/"),
            Some("Def456".to_string())
        );
    }

    #[test]
    fn finds_og_video() {
        let html = r#"<meta property="og:video" content="https://example.com/video.mp4">"#;
        assert_eq!(
            find_video_url(html),
            Some("https://example.com/video.mp4".to_string())
        );
    }

    #[test]
    fn finds_video_url_in_json() {
        let html = r#"{"video_url":"https:\/\/cdn.example.com\/video.mp4","other":"x"}"#;
        let found = find_video_url(html);
        assert!(found.is_some());
        assert!(found.unwrap().contains("video.mp4"));
    }
}
