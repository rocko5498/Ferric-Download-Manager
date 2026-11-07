use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

use crate::cookies;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    !u.contains("messenger.com")
        && (u.contains("facebook.com/watch")
            || u.contains("facebook.com/videos/")
            || u.contains("facebook.com/video/")
            || (u.contains("facebook.com/") && u.contains("/videos/")))
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let browser_cookies = cookies::read_cookies("facebook.com").await;
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
        .header("Accept-Language", "en-US,en;q=0.9");

    if !cookie_header.is_empty() {
        builder = builder.header("Cookie", &cookie_header);
    }

    let html = builder
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("Facebook returned error status")?
        .text()
        .await
        .context("read Facebook page")?;

    let formats = extract_formats(&html, &cookie_header);
    if formats.is_empty() {
        bail!("Facebook: no video URL found (login required or unsupported URL)");
    }

    let title = find_og(&html, "og:title").unwrap_or_else(|| "Facebook Video".to_string());

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail: find_og(&html, "og:image"),
        duration_secs: None,
        formats,
        extractor: "facebook".to_string(),
        description: find_og(&html, "og:description"),
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_formats(html: &str, cookie_header: &str) -> Vec<ExtractedFormat> {
    let mut formats = Vec::new();
    let headers: Vec<(String, String)> = if cookie_header.is_empty() {
        Vec::new()
    } else {
        vec![("Cookie".to_string(), cookie_header.to_string())]
    };

    // Facebook embeds HD and SD URLs as "playable_url_quality_hd" and "playable_url"
    if let Some(url) = find_json_str(html, "playable_url_quality_hd") {
        formats.push(make_format("hd", "HD", url, headers.clone()));
    }
    if let Some(url) = find_json_str(html, "playable_url") {
        if !formats.iter().any(|f: &ExtractedFormat| f.url == url) {
            formats.push(make_format("sd", "SD", url, headers));
        }
    }

    formats
}

fn make_format(
    id: &str,
    label: &str,
    url: String,
    headers: Vec<(String, String)>,
) -> ExtractedFormat {
    ExtractedFormat {
        id: id.to_string(),
        label: label.to_string(),
        url,
        ext: "mp4".to_string(),
        protocol: Some("https".to_string()),
        manifest_url: None,
        format_note: Some(label.to_string()),
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
    }
}

fn find_json_str(html: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\":\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let url = rest[..end].replace("\\u0026", "&").replace("\\/", "/");
    if url.starts_with("http") {
        Some(url)
    } else {
        None
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_facebook_video_urls() {
        assert!(matches("https://www.facebook.com/watch/?v=123456789"));
        assert!(matches("https://www.facebook.com/videos/123456789/"));
        assert!(matches(
            "https://www.facebook.com/username/videos/123456789/"
        ));
        assert!(!matches("https://messenger.com/watch/?v=123"));
        assert!(!matches("https://www.youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_hd_and_sd() {
        let html = r#"{"playable_url_quality_hd":"https:\/\/video.fbcdn.net\/hd.mp4","playable_url":"https:\/\/video.fbcdn.net\/sd.mp4"}"#;
        let formats = extract_formats(html, "");
        assert_eq!(formats.len(), 2);
        assert_eq!(formats[0].id, "hd");
        assert_eq!(formats[1].id, "sd");
    }

    #[test]
    fn deduplicates_same_url() {
        let html = r#"{"playable_url_quality_hd":"https:\/\/video.fbcdn.net\/same.mp4","playable_url":"https:\/\/video.fbcdn.net\/same.mp4"}"#;
        let formats = extract_formats(html, "");
        assert_eq!(formats.len(), 1);
    }

    #[test]
    fn sd_only_when_no_hd() {
        let html = r#"{"playable_url":"https:\/\/video.fbcdn.net\/sd.mp4"}"#;
        let formats = extract_formats(html, "");
        assert_eq!(formats.len(), 1);
        assert_eq!(formats[0].id, "sd");
    }
}
