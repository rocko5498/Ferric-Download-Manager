use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("tiktok.com/") && (u.contains("/video/") || u.contains("/@"))
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    // TikTok injects video data into __UNIVERSAL_DATA_FOR_REHYDRATION__ script tag
    let html = client
        .get(url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36")
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("TikTok page error")?
        .text()
        .await
        .context("read TikTok page")?;

    let json_blob = extract_json_blob(&html)
        .ok_or_else(|| anyhow::anyhow!("TikTok: could not find video data in page"))?;

    let root: serde_json::Value =
        serde_json::from_str(json_blob).context("parse TikTok JSON blob")?;

    // Path: __DEFAULT_SCOPE__ → webapp.video-detail → itemInfo → itemStruct
    let item = root
        .pointer("/__DEFAULT_SCOPE__/webapp.video-detail/itemInfo/itemStruct")
        .ok_or_else(|| anyhow::anyhow!("TikTok: itemStruct not found in JSON"))?;

    let title = item["desc"].as_str().unwrap_or("TikTok video").to_string();
    let author = item["author"]["nickname"].as_str().map(str::to_string);
    let duration_secs = item["video"]["duration"].as_f64();
    let thumbnail = item["video"]["cover"].as_str().map(str::to_string);
    let upload_date = item["createTime"].as_u64().map(|ts| {
        // Unix timestamp → YYYYMMDD
        use std::time::{Duration, UNIX_EPOCH};
        let d = UNIX_EPOCH + Duration::from_secs(ts);
        let secs = d.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
        // Simple conversion: days since epoch
        let days = secs / 86400;
        let (y, m, day) = days_to_ymd(days);
        format!("{y:04}{m:02}{day:02}")
    });

    let mut formats = Vec::new();

    // Primary play URL
    if let Some(play_url) = item["video"]["playAddr"].as_str().filter(|s| !s.is_empty()) {
        let height = item["video"]["height"].as_u64().map(|h| h as u32);
        let width = item["video"]["width"].as_u64().map(|w| w as u32);
        let bitrate = item["video"]["bitrate"].as_f64().map(|b| b / 1000.0);
        formats.push(ExtractedFormat {
            id: "play".to_string(),
            label: height.map_or_else(|| "TikTok".to_string(), |h| format!("{h}p")),
            url: play_url.to_string(),
            ext: "mp4".to_string(),
            protocol: Some("https".to_string()),
            manifest_url: None,
            format_note: Some("direct".to_string()),
            width,
            height,
            tbr_kbps: bitrate,
            abr_kbps: None,
            vbr_kbps: bitrate,
            fps: item["video"]["ratio"]
                .as_str()
                .and_then(|r| r.split('p').next())
                .and_then(|r| r.parse::<f64>().ok()),
            vcodec: item["video"]["videoQuality"].as_str().map(str::to_string),
            acodec: None,
            container: Some("mp4".to_string()),
            language: None,
            filesize: item["video"]["size"].as_u64(),
            has_drm: false,
            headers: vec![("Referer".to_string(), "https://www.tiktok.com/".to_string())],
            is_audio_only: false,
        });
    }

    // Download URL (no watermark, if present)
    if let Some(dl_url) = item["video"]["downloadAddr"]
        .as_str()
        .filter(|s| !s.is_empty())
    {
        if item["video"]["downloadAddr"] != item["video"]["playAddr"] {
            formats.push(ExtractedFormat {
                id: "download".to_string(),
                label: "No watermark".to_string(),
                url: dl_url.to_string(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: Some("no watermark".to_string()),
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
                headers: vec![("Referer".to_string(), "https://www.tiktok.com/".to_string())],
                is_audio_only: false,
            });
        }
    }

    if formats.is_empty() {
        bail!("TikTok: no video URLs found for {url}");
    }

    Ok(ExtractResult {
        title,
        uploader: author,
        thumbnail,
        duration_secs,
        formats,
        extractor: "tiktok".to_string(),
        description: item["desc"].as_str().map(str::to_string),
        upload_date,
        tags: item["textExtra"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t["hashtagName"].as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_json_blob(html: &str) -> Option<&str> {
    // <script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" ...>{...}</script>
    let marker = r#"id="__UNIVERSAL_DATA_FOR_REHYDRATION__""#;
    let start = html.find(marker)?;
    let after_tag = html[start..].find('>')?;
    let json_start = start + after_tag + 1;
    let json_end = html[json_start..].find("</script>")?;
    Some(&html[json_start..json_start + json_end])
}

// Minimal Gregorian calendar conversion (no chrono dep)
fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let days_in_year = if leap { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_tiktok_urls() {
        assert!(matches("https://www.tiktok.com/@user/video/1234567890"));
        assert!(matches("https://tiktok.com/@user/video/123"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
        assert!(!matches("https://twitter.com/user/status/123"));
    }

    #[test]
    fn extracts_json_blob() {
        let html = r#"<script id="__UNIVERSAL_DATA_FOR_REHYDRATION__" type="application/json">{"test":1}</script>"#;
        assert_eq!(extract_json_blob(html), Some(r#"{"test":1}"#));
    }

    #[test]
    fn days_to_ymd_epoch() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        assert_eq!(days_to_ymd(365), (1971, 1, 1));
    }
}
