use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use regex::Regex;
use scraper::{Html, Selector};
use serde_json::Value;
use std::collections::HashSet;

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("page returned error status")?
        .text()
        .await
        .context("read page body")?;

    let embed_url = {
        let document = Html::parse_document(&html);
        extract_known_embed_url(&document, &html, url)
    };
    if let Some(embed_url) = embed_url {
        tracing::debug!("generic extractor delegated embedded URL: {embed_url}");
        return extract_known_embed(&embed_url, client).await;
    }

    let document = Html::parse_document(&html);
    let title = extract_title(&document).unwrap_or_else(|| "Untitled".to_string());
    let thumbnail = extract_og_image(&document);
    let mut video_urls = extract_video_tags(&document);
    video_urls.extend(extract_og_video(&document));
    video_urls.extend(extract_json_ld_video(&document));
    video_urls.extend(extract_embedded_media_urls(&html));
    dedupe(&mut video_urls);

    if video_urls.is_empty() {
        bail!("generic extractor: no video tags, og:video, or embedded media URLs found in {url}");
    }

    let formats: Vec<ExtractedFormat> = video_urls
        .into_iter()
        .enumerate()
        .filter_map(|(index, src)| format_from_src(url, &src, index))
        .collect();

    if formats.is_empty() {
        bail!("generic extractor: all found URLs were invalid");
    }

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail,
        duration_secs: None,
        formats,
        extractor: "generic".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

async fn extract_known_embed(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    // Site IEs only — never extract_native (would recurse into generic).
    let name = crate::extractor_name(url);
    match crate::dispatch_site_ie(name, url, client).await {
        Ok(mut result) => {
            if let Some(n) = name {
                result.extractor = format!("generic:{n}");
            }
            crate::sort_formats(&mut result.formats);
            Ok(result)
        }
        Err(_) => bail!("generic extractor: unsupported embedded URL {url}"),
    }
}

fn format_from_src(page_url: &str, src: &str, index: usize) -> Option<ExtractedFormat> {
    let trimmed = src.trim();
    if trimmed.is_empty() || trimmed.to_ascii_lowercase().starts_with("javascript:") {
        return None;
    }
    let abs_url = url::Url::parse(page_url)
        .ok()
        .and_then(|base| base.join(trimmed).ok())
        .map_or_else(|| trimmed.to_string(), |url| url.to_string());
    let ext = abs_url
        .split('?')
        .next()
        .and_then(|path| path.rsplit('.').next())
        .filter(|ext| !ext.contains('/'))
        .unwrap_or("mp4")
        .to_ascii_lowercase();

    Some(ExtractedFormat {
        id: format!("generic-{index}"),
        label: format!("Video {}", index + 1),
        protocol: protocol_for_ext(&ext).map(str::to_string),
        manifest_url: manifest_for_ext(&ext, &abs_url),
        format_note: None,
        ext: ext.clone(),
        url: abs_url,
        width: None,
        height: None,
        tbr_kbps: None,
        abr_kbps: None,
        vbr_kbps: None,
        fps: None,
        vcodec: None,
        acodec: None,
        container: container_for_ext(&ext),
        language: None,
        filesize: None,
        has_drm: false,
        headers: Vec::new(),
        is_audio_only: false,
    })
}

fn protocol_for_ext(ext: &str) -> Option<&'static str> {
    match ext {
        "m3u8" => Some("m3u8"),
        "mpd" => Some("dash"),
        _ => Some("https"),
    }
}

fn manifest_for_ext(ext: &str, url: &str) -> Option<String> {
    matches!(ext, "m3u8" | "mpd").then(|| url.to_string())
}

fn container_for_ext(ext: &str) -> Option<String> {
    match ext {
        "m3u8" | "mpd" => None,
        "mp4" | "m4v" => Some("mp4".to_string()),
        "webm" => Some("webm".to_string()),
        "mov" => Some("mov".to_string()),
        _ => Some(ext.to_string()),
    }
}

fn extract_title(doc: &Html) -> Option<String> {
    let og_title = Selector::parse(r#"meta[property="og:title"]"#).ok()?;
    if let Some(content) = doc
        .select(&og_title)
        .next()
        .and_then(|element| element.value().attr("content"))
    {
        return Some(content.to_string());
    }

    let title_sel = Selector::parse("title").ok()?;
    doc.select(&title_sel)
        .next()
        .map(|element| element.text().collect::<String>().trim().to_string())
        .filter(|title| !title.is_empty())
}

fn extract_og_image(doc: &Html) -> Option<String> {
    let sel = Selector::parse(r#"meta[property="og:image"]"#).ok()?;
    doc.select(&sel)
        .next()
        .and_then(|element| element.value().attr("content"))
        .map(str::to_string)
}

fn extract_video_tags(doc: &Html) -> Vec<String> {
    let mut urls = Vec::new();
    for selector in ["video[src]", "video source[src]"] {
        if let Ok(sel) = Selector::parse(selector) {
            urls.extend(
                doc.select(&sel)
                    .filter_map(|element| element.value().attr("src"))
                    .map(str::to_string),
            );
        }
    }
    urls
}

fn extract_known_embed_url(doc: &Html, html: &str, page_url: &str) -> Option<String> {
    let mut urls = Vec::new();
    for selector in ["iframe[src]", "embed[src]", "object[data]", "a[href]"] {
        if let Ok(sel) = Selector::parse(selector) {
            urls.extend(
                doc.select(&sel)
                    .filter_map(|element| {
                        element
                            .value()
                            .attr("src")
                            .or_else(|| element.value().attr("data"))
                            .or_else(|| element.value().attr("href"))
                    })
                    .filter_map(|value| normalize_url(page_url, value)),
            );
        }
    }
    urls.extend(
        extract_embed_regex_urls(html)
            .into_iter()
            .filter_map(|value| normalize_url(page_url, &value)),
    );
    dedupe(&mut urls);
    urls.into_iter().find(|url| {
        crate::youtube::matches(url)
            || crate::vimeo::matches(url)
            || crate::reddit::matches(url) && !url.contains("v.redd.it/")
    })
}

fn normalize_url(page_url: &str, value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.to_ascii_lowercase().starts_with("javascript:") {
        return None;
    }
    if let Some(protocol_relative) = trimmed.strip_prefix("//") {
        return Some(format!("https://{protocol_relative}"));
    }
    url::Url::parse(page_url)
        .ok()
        .and_then(|base| base.join(trimmed).ok())
        .map(|url| url.to_string())
        .or_else(|| url::Url::parse(trimmed).ok().map(|url| url.to_string()))
}

fn extract_embed_regex_urls(html: &str) -> Vec<String> {
    const PATTERNS: &[&str] = &[
        r#"https?://(?:www\.)?youtube\.com/(?:watch\?v=|embed/|shorts/)[A-Za-z0-9_-]+"#,
        r#"https?://youtu\.be/[A-Za-z0-9_-]+"#,
        r#"https?://player\.vimeo\.com/video/[0-9]+"#,
        r#"https?://vimeo\.com/[0-9]+"#,
        r#"https?://(?:www\.)?reddit\.com/r/[^"'<>\s]+/comments/[^"'<>\s]+"#,
    ];
    let normalized = html.replace("\\/", "/").replace("&amp;", "&");
    let mut urls = Vec::new();
    for pattern in PATTERNS {
        if let Ok(regex) = Regex::new(pattern) {
            urls.extend(regex.find_iter(&normalized).map(|m| m.as_str().to_string()));
        }
    }
    urls
}

fn extract_og_video(doc: &Html) -> Vec<String> {
    let mut urls = Vec::new();
    for property in ["og:video", "og:video:url", "og:video:secure_url"] {
        let selector = format!(r#"meta[property="{property}"]"#);
        let parsed = Selector::parse(&selector);
        if let Ok(sel) = parsed {
            urls.extend(
                doc.select(&sel)
                    .filter_map(|element| element.value().attr("content"))
                    .map(str::to_string),
            );
        }
    }
    urls
}

fn extract_json_ld_video(doc: &Html) -> Vec<String> {
    let Ok(sel) = Selector::parse(r#"script[type="application/ld+json"]"#) else {
        return Vec::new();
    };
    let mut urls = Vec::new();
    for script in doc.select(&sel) {
        let json = script.text().collect::<String>();
        if let Ok(value) = serde_json::from_str::<Value>(&json) {
            collect_media_strings(&value, &mut urls);
        }
    }
    urls
}

fn collect_media_strings(value: &Value, urls: &mut Vec<String>) {
    match value {
        Value::String(s) if looks_like_media_url(s) => urls.push(s.clone()),
        Value::Array(items) => {
            for item in items {
                collect_media_strings(item, urls);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_media_strings(value, urls);
            }
        }
        _ => {}
    }
}

fn extract_embedded_media_urls(html: &str) -> Vec<String> {
    let normalized = html
        .replace("\\/", "/")
        .replace("\\u002F", "/")
        .replace("\\u002f", "/")
        .replace("&amp;", "&");
    let mut urls = Vec::new();
    let mut offset = 0;
    while let Some(pos) = normalized[offset..]
        .find("https://")
        .or_else(|| normalized[offset..].find("http://"))
    {
        let start = offset + pos;
        let tail = &normalized[start..];
        let end = tail
            .find(|c: char| {
                matches!(
                    c,
                    '"' | '\'' | '<' | '>' | ' ' | '\n' | '\r' | '\t' | ')' | ']' | '}'
                )
            })
            .unwrap_or(tail.len());
        let candidate = &tail[..end];
        if looks_like_media_url(candidate) {
            urls.push(candidate.trim_end_matches([',', ';']).to_string());
        }
        offset = start + end.max(1);
    }
    urls
}

fn looks_like_media_url(value: &str) -> bool {
    let lower = value
        .split('?')
        .next()
        .unwrap_or(value)
        .to_ascii_lowercase();
    [".mp4", ".webm", ".m4v", ".mov", ".m3u8", ".mpd"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

fn dedupe(urls: &mut Vec<String>) {
    let mut seen = HashSet::new();
    urls.retain(|url| seen.insert(url.clone()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_video_tag_src() {
        let html =
            r#"<html><body><video src="https://example.com/video.mp4"></video></body></html>"#;
        let doc = Html::parse_document(html);
        assert_eq!(
            extract_video_tags(&doc),
            vec!["https://example.com/video.mp4"]
        );
    }

    #[test]
    fn extracts_source_tag_inside_video() {
        let html = r#"<video>
            <source src="https://cdn.example.com/hi.mp4" type="video/mp4">
            <source src="https://cdn.example.com/lo.mp4" type="video/mp4">
        </video>"#;
        let doc = Html::parse_document(html);
        let urls = extract_video_tags(&doc);
        assert_eq!(urls.len(), 2);
        assert!(urls[0].contains("hi.mp4"));
    }

    #[test]
    fn extracts_og_video() {
        let html = r#"<meta property="og:video" content="https://example.com/stream.m3u8">"#;
        let doc = Html::parse_document(html);
        assert_eq!(
            extract_og_video(&doc),
            vec!["https://example.com/stream.m3u8"]
        );
    }

    #[test]
    fn extracts_og_title() {
        let html = r#"<meta property="og:title" content="My Video"><title>Fallback</title>"#;
        let doc = Html::parse_document(html);
        assert_eq!(extract_title(&doc), Some("My Video".to_string()));
    }

    #[test]
    fn resolves_relative_url() {
        let format = format_from_src("https://example.com/a/page.html", "../video.mp4", 0).unwrap();
        assert_eq!(format.url, "https://example.com/video.mp4");
        assert_eq!(format.ext, "mp4");
    }

    #[test]
    fn extracts_json_ld_video_url() {
        let html = r#"<script type="application/ld+json">
          {"@type":"VideoObject","contentUrl":"https://cdn.example.com/movie.mp4"}
        </script>"#;
        let doc = Html::parse_document(html);
        assert_eq!(
            extract_json_ld_video(&doc),
            vec!["https://cdn.example.com/movie.mp4"]
        );
    }

    #[test]
    fn extracts_embedded_escaped_media_url() {
        let html = r#"{"playbackUrl":"https:\/\/cdn.example.com\/master.m3u8?token=abc"}"#;
        let urls = extract_embedded_media_urls(html);
        assert_eq!(urls, vec!["https://cdn.example.com/master.m3u8?token=abc"]);
    }

    #[test]
    fn extracts_known_iframe_embed() {
        let html = r#"<iframe src="//www.youtube.com/embed/dQw4w9WgXcQ"></iframe>"#;
        let doc = Html::parse_document(html);
        assert_eq!(
            extract_known_embed_url(&doc, html, "https://example.com/page").as_deref(),
            Some("https://www.youtube.com/embed/dQw4w9WgXcQ")
        );
    }

    #[test]
    fn extracts_known_embed_from_script() {
        let html = r#"{"embed":"https:\/\/player.vimeo.com\/video\/123456789"}"#;
        let urls = extract_embed_regex_urls(html);
        assert_eq!(urls, vec!["https://player.vimeo.com/video/123456789"]);
    }
}
