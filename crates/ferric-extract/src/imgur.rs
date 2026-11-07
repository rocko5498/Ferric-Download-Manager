use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat, PlaylistEntry};
use regex::Regex;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("imgur.com")
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    // 1. Direct gifv/mp4/image URL handling
    if url.ends_with(".gifv") {
        let mp4_url = url.replace(".gifv", ".mp4");
        return single_media_result("imgur_gifv", "mp4", &mp4_url);
    }

    for ext in &["mp4", "jpg", "jpeg", "png", "gif"] {
        if url.ends_with(&format!(".{}", ext)) {
            return single_media_result("imgur_direct", ext, url);
        }
    }

    // 2. Fetch page HTML
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
        .context("Imgur page returned error status")?
        .text()
        .await
        .context("read Imgur page body")?;

    let mut title = "Imgur Post".to_string();
    if let Some(og_title) = extract_og_title(&html) {
        title = og_title;
    }

    // 3. Scan page HTML for CDN URLs
    let media_regex =
        Regex::new(r#"i\.imgur\.com/([a-zA-Z0-9]+)\.(mp4|jpg|jpeg|png|gif)"#).unwrap();
    let mut media_items = std::collections::HashMap::new();

    for cap in media_regex.captures_iter(&html) {
        let id = cap[1].to_string();
        let ext = cap[2].to_string();
        // Skip common layouts or avatars if any
        if id == "logo" || id == "favicon" {
            continue;
        }
        let full_url = format!("https://i.imgur.com/{}.{}", id, ext);

        // Prefer mp4 over static formats for the same media ID
        if ext == "mp4" {
            media_items.insert(id, (ext, full_url));
        } else {
            media_items.entry(id).or_insert((ext, full_url));
        }
    }

    if media_items.is_empty() {
        bail!("No Imgur media elements found on this page.");
    }

    if media_items.len() == 1 {
        let (id, (ext, item_url)) = media_items.into_iter().next().unwrap();
        let format = ExtractedFormat {
            id: id.clone(),
            label: format!("Imgur {}", ext.to_uppercase()),
            url: item_url.clone(),
            ext: ext.clone(),
            protocol: Some("https".to_string()),
            manifest_url: None,
            format_note: Some(ext.to_uppercase()),
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
            headers: Vec::new(),
            is_audio_only: false,
        };
        return Ok(ExtractResult {
            title,
            uploader: None,
            thumbnail: Some(item_url),
            duration_secs: None,
            formats: vec![format],
            extractor: "imgur".to_string(),
            description: None,
            upload_date: None,
            tags: Vec::new(),
            subtitles: Vec::new(),
            playlist_entries: Vec::new(),
            chapters: Vec::new(),
        });
    }

    // Multiple elements -> treat as album/playlist
    let mut playlist_entries = Vec::new();
    let mut index = 1;
    let mut sorted_keys: Vec<String> = media_items.keys().cloned().collect();
    sorted_keys.sort();

    for key in sorted_keys {
        let (ext, item_url) = &media_items[&key];
        playlist_entries.push(PlaylistEntry {
            url: item_url.clone(),
            title: Some(format!("Imgur Item {} ({})", key, ext)),
            index,
            duration_secs: None,
            thumbnail: Some(item_url.clone()),
        });
        index += 1;
    }

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail: playlist_entries.first().and_then(|e| e.thumbnail.clone()),
        duration_secs: None,
        formats: Vec::new(),
        extractor: "imgur".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries,
        chapters: Vec::new(),
    })
}

fn single_media_result(id: &str, ext: &str, url: &str) -> Result<ExtractResult> {
    let format = ExtractedFormat {
        id: id.to_string(),
        label: format!("Imgur {}", ext.to_uppercase()),
        url: url.to_string(),
        ext: ext.to_string(),
        protocol: Some("https".to_string()),
        manifest_url: None,
        format_note: Some(ext.to_uppercase()),
        width: None,
        height: None,
        tbr_kbps: None,
        abr_kbps: None,
        vbr_kbps: None,
        fps: None,
        vcodec: None,
        acodec: None,
        container: Some(ext.to_string()),
        language: None,
        filesize: None,
        has_drm: false,
        headers: Vec::new(),
        is_audio_only: false,
    };
    Ok(ExtractResult {
        title: format!("Imgur Media ({})", id),
        uploader: None,
        thumbnail: Some(url.to_string()),
        duration_secs: None,
        formats: vec![format],
        extractor: "imgur".to_string(),
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
    fn matches_imgur_urls() {
        assert!(matches("https://imgur.com/gallery/12345"));
        assert!(matches("https://i.imgur.com/12345.gifv"));
        assert!(!matches("https://google.com"));
    }
}
