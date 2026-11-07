use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat, PlaylistEntry};
use regex::Regex;
use serde_json::Value;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("pinterest.com") || u.contains("pinterest.cl") || u.contains("pinterest.fr")
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
        .context("Pinterest page returned error status")?
        .text()
        .await
        .context("read Pinterest page body")?;

    let json_str = extract_pws_json(&html)
        .context("could not find __PWS_INITIAL_STORE__ in Pinterest page")?;
    let json_val: Value =
        serde_json::from_str(json_str).context("failed to parse __PWS_INITIAL_STORE__ JSON")?;

    let is_pin_page = url.contains("/pin/");

    if is_pin_page {
        let mut title = "Pinterest Pin".to_string();
        if let Some(og_title) = extract_og_title(&html) {
            title = og_title;
        }

        // 1. Try video_list
        if let Some(video_list) = find_key_in_value(&json_val, "video_list") {
            let mut formats = Vec::new();
            if let Some(obj) = video_list.as_object() {
                for (name, format_val) in obj {
                    if let Some(v_url) = format_val.get("url").and_then(|u| u.as_str()) {
                        let height = format_val
                            .get("height")
                            .and_then(|h| h.as_u64())
                            .map(|h| h as u32);
                        let width = format_val
                            .get("width")
                            .and_then(|w| w.as_u64())
                            .map(|w| w as u32);
                        let ext = if v_url.contains(".m3u8") {
                            "m3u8".to_string()
                        } else {
                            "mp4".to_string()
                        };

                        formats.push(ExtractedFormat {
                            id: name.clone(),
                            label: format!("{}x{}", width.unwrap_or(0), height.unwrap_or(0)),
                            url: v_url.to_string(),
                            ext: ext.clone(),
                            protocol: Some("https".to_string()),
                            manifest_url: if ext == "m3u8" {
                                Some(v_url.to_string())
                            } else {
                                None
                            },
                            format_note: Some(name.clone()),
                            width,
                            height,
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
                        });
                    }
                }
            }

            if !formats.is_empty() {
                formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));
                return Ok(ExtractResult {
                    title,
                    uploader: None,
                    thumbnail: None,
                    duration_secs: None,
                    formats,
                    extractor: "pinterest".to_string(),
                    description: None,
                    upload_date: None,
                    tags: Vec::new(),
                    subtitles: Vec::new(),
                    playlist_entries: Vec::new(),
                    chapters: Vec::new(),
                });
            }
        }

        // 2. Fallback to closeup images
        if let Some(closeup) = find_key_in_value(&json_val, "images") {
            if let Some(orig) = closeup
                .get("orig")
                .and_then(|o| o.get("url"))
                .and_then(|u| u.as_str())
            {
                let format = ExtractedFormat {
                    id: "original_image".to_string(),
                    label: "Original Image".to_string(),
                    url: orig.to_string(),
                    ext: "jpg".to_string(),
                    protocol: Some("https".to_string()),
                    manifest_url: None,
                    format_note: Some("Image".to_string()),
                    width: None,
                    height: None,
                    tbr_kbps: None,
                    abr_kbps: None,
                    vbr_kbps: None,
                    fps: None,
                    vcodec: None,
                    acodec: None,
                    container: Some("jpg".to_string()),
                    language: None,
                    filesize: None,
                    has_drm: false,
                    headers: Vec::new(),
                    is_audio_only: false,
                };
                return Ok(ExtractResult {
                    title,
                    uploader: None,
                    thumbnail: Some(orig.to_string()),
                    duration_secs: None,
                    formats: vec![format],
                    extractor: "pinterest".to_string(),
                    description: None,
                    upload_date: None,
                    tags: Vec::new(),
                    subtitles: Vec::new(),
                    playlist_entries: Vec::new(),
                    chapters: Vec::new(),
                });
            }
        }

        bail!("No video or image formats found on this Pinterest pin.");
    } else {
        // Collect Pin IDs from board/profile HTML
        let pin_regex = Regex::new(r#"/pin/(\d+)/"#).unwrap();
        let mut pin_ids = std::collections::HashSet::new();
        for cap in pin_regex.captures_iter(&html) {
            pin_ids.insert(cap[1].to_string());
        }

        let mut playlist_entries = Vec::new();
        let mut index = 1;
        let mut pins: Vec<String> = pin_ids.into_iter().collect();
        pins.sort();

        for pin_id in pins {
            let pin_url = format!("https://www.pinterest.com/pin/{}/", pin_id);
            playlist_entries.push(PlaylistEntry {
                url: pin_url,
                title: Some(format!("Pinterest Pin {}", pin_id)),
                index,
                duration_secs: None,
                thumbnail: None,
            });
            index += 1;
        }

        if playlist_entries.is_empty() {
            bail!("No Pins found on this Pinterest page.");
        }

        Ok(ExtractResult {
            title: "Pinterest Board".to_string(),
            uploader: None,
            thumbnail: None,
            duration_secs: None,
            formats: Vec::new(),
            extractor: "pinterest".to_string(),
            description: None,
            upload_date: None,
            tags: Vec::new(),
            subtitles: Vec::new(),
            playlist_entries,
            chapters: Vec::new(),
        })
    }
}

fn extract_pws_json(html: &str) -> Option<&str> {
    let marker = r#"<script id="__PWS_INITIAL_STORE__" type="application/json">"#;
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find("</script>").map(|i| start + i)?;
    Some(&html[start..end])
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

fn find_key_in_value<'a>(val: &'a Value, key_to_find: &str) -> Option<&'a Value> {
    if let Some(obj) = val.as_object() {
        if let Some(target) = obj.get(key_to_find) {
            return Some(target);
        }
        for sub_val in obj.values() {
            if let Some(found) = find_key_in_value(sub_val, key_to_find) {
                return Some(found);
            }
        }
    } else if let Some(arr) = val.as_array() {
        for sub_val in arr {
            if let Some(found) = find_key_in_value(sub_val, key_to_find) {
                return Some(found);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_pinterest_urls() {
        assert!(matches("https://www.pinterest.com/pin/12345/"));
        assert!(matches("https://pinterest.cl/pin/12345/"));
        assert!(matches("https://www.pinterest.fr/pin/12345"));
    }
}
