use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use regex::Regex;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("vk.com") || u.contains("vk.ru") || u.contains("vkvideo.ru")
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("VK Video page returned error status")?
        .text()
        .await
        .context("read VK Video page body")?;

    let mut title = "VK Video".to_string();
    if let Some(og_title) = extract_og_title(&html) {
        title = og_title;
    }

    let thumbnail = extract_og_image(&html);

    // Regex to match qualities in the page config/JSON:
    // e.g. "url240": "https://..." or "url720": "https://..."
    let url_re = Regex::new(r#""url(240|360|480|720|1080)":\s*["']([^"']+)["']"#).unwrap();
    let mut formats = Vec::new();

    for cap in url_re.captures_iter(&html) {
        let height_str = &cap[1];
        let stream_url_escaped = &cap[2];
        let height = height_str.parse::<u32>().unwrap_or(360);
        let stream_url = stream_url_escaped.replace(r#"\/"#, "/");

        if formats
            .iter()
            .any(|f: &ExtractedFormat| f.url == stream_url)
        {
            continue;
        }

        formats.push(ExtractedFormat {
            id: format!("vk_{}", height_str),
            label: format!("{}p", height_str),
            url: stream_url,
            ext: "mp4".to_string(),
            protocol: Some("https".to_string()),
            manifest_url: None,
            format_note: Some(format!("{}p", height_str)),
            width: None,
            height: Some(height),
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
        bail!("No video streams found in the VK page HTML. Note: Video might be private or age-restricted.");
    }

    formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail,
        duration_secs: None,
        formats,
        extractor: "vk".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_og_title(html: &str) -> Option<String> {
    let pattern = r#"property="og:title"\s+content="([^"]+)""#;
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(html)?;
    Some(decode_html_entities(&caps[1]))
}

fn extract_og_image(html: &str) -> Option<String> {
    let pattern = r#"property="og:image"\s+content="([^"]+)""#;
    let re = Regex::new(pattern).ok()?;
    let caps = re.captures(html)?;
    Some(decode_html_entities(&caps[1]))
}

fn decode_html_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&#39;", "'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_vk_urls() {
        assert!(matches("https://vk.com/video-77521_162222515"));
        assert!(matches("https://vk.ru/video1234_5678"));
        assert!(matches("https://vkvideo.ru/video-93049196_456239755"));
        assert!(!matches("https://google.com"));
    }
}
