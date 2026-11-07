use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use regex::Regex;
use serde_json::Value;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("rumble.com")
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let mut video_id = None;
    if url.contains("/embed/") {
        if let Some(pos) = url.find("/embed/") {
            let id: String = url[pos + 7..]
                .chars()
                .take_while(|c| c.is_alphanumeric())
                .collect();
            if !id.is_empty() {
                video_id = Some(id);
            }
        }
    }

    if video_id.is_none() {
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
            .context("Rumble page returned error status")?
            .text()
            .await
            .context("read Rumble page body")?;

        let embed_re = Regex::new(r#"rumble\.com/embed/([a-zA-Z0-9]+)"#).unwrap();
        if let Some(caps) = embed_re.captures(&html) {
            video_id = Some(caps[1].to_string());
        }
    }

    let Some(id) = video_id else {
        bail!("Could not extract Rumble video ID from URL or page HTML.");
    };

    // Call internal embedJS endpoint to retrieve video stream JSON metadata
    let api_url = format!(
        "https://rumble.com/embedJS/u3/?request=video&ver=2&v={}",
        id
    );
    let api_res = client
        .get(&api_url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .with_context(|| format!("GET {api_url}"))?
        .error_for_status()
        .context("Rumble embedJS endpoint returned error status")?
        .text()
        .await
        .context("read Rumble embedJS body")?;

    let val: Value =
        serde_json::from_str(&api_res).context("failed to parse Rumble embedJS JSON response")?;

    let title = val["title"].as_str().unwrap_or("Rumble Video").to_string();
    let thumbnail = val["poster"]
        .as_str()
        .or_else(|| val["thumb"].as_str())
        .map(String::from);
    let duration_secs = val["duration"].as_f64();
    let uploader = val["author"]["name"].as_str().map(String::from);

    let mut stream_urls = Vec::new();
    if let Some(ua) = val.get("ua") {
        find_urls(ua, &mut stream_urls);
    } else {
        find_urls(&val, &mut stream_urls);
    }

    if stream_urls.is_empty() {
        bail!("No streaming URLs found in Rumble response JSON");
    }

    let mut formats = Vec::new();
    for (s_url, proto) in stream_urls {
        let ext = if s_url.contains(".m3u8") {
            "m3u8".to_string()
        } else {
            "mp4".to_string()
        };
        let height = parse_height_from_url(&s_url);
        let is_audio = s_url.contains("audio");

        formats.push(ExtractedFormat {
            id: format!("rumble_{}", height.unwrap_or(0)),
            label: format!("{}p", height.unwrap_or(360)),
            url: s_url.clone(),
            ext: ext.clone(),
            protocol: proto.or_else(|| Some("https".to_string())),
            manifest_url: if ext == "m3u8" { Some(s_url) } else { None },
            format_note: Some(format!("{}p", height.unwrap_or(360))),
            width: None,
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
            is_audio_only: is_audio,
        });
    }

    formats.sort_by(|a, b| b.height.unwrap_or(0).cmp(&a.height.unwrap_or(0)));

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats,
        extractor: "rumble".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn find_urls(val: &Value, urls: &mut Vec<(String, Option<String>)>) {
    if let Some(obj) = val.as_object() {
        if let Some(u) = obj.get("url").and_then(|u| u.as_str()) {
            if u.starts_with("http") {
                urls.push((u.to_string(), None));
            }
        }
        for (k, v) in obj {
            if k == "meta" {
                if let Some(u) = v.get("url").and_then(|u| u.as_str()) {
                    if u.starts_with("http") {
                        urls.push((u.to_string(), Some("hls".to_string())));
                    }
                }
            }
            find_urls(v, urls);
        }
    } else if let Some(arr) = val.as_array() {
        for v in arr {
            find_urls(v, urls);
        }
    }
}

fn parse_height_from_url(url: &str) -> Option<u32> {
    let re = Regex::new(r"[-_](\d+)p\.").unwrap();
    if let Some(caps) = re.captures(url) {
        return caps[1].parse().ok();
    }
    let re2 = Regex::new(r"(\d+)p/").unwrap();
    if let Some(caps) = re2.captures(url) {
        return caps[1].parse().ok();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_rumble_urls() {
        assert!(matches("https://rumble.com/v5pv5f-some-video.html"));
        assert!(matches("https://rumble.com/embed/v5pv5f"));
        assert!(!matches("https://google.com"));
    }
}
