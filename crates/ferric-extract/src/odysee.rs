use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde_json::Value;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("odysee.com") || u.contains("lbry.tv")
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    // 1. Resolve canonical URI from page URL
    let payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "resolve",
        "params": {
            "urls": [url]
        },
        "id": 1
    });

    let res = client
        .post("https://api.odysee.com/api/v1/proxy")
        .json(&payload)
        .header("Content-Type", "application/json")
        .send()
        .await
        .with_context(|| format!("POST resolve to Odysee API for {url}"))?
        .error_for_status()
        .context("Odysee API resolve returned error status")?
        .json::<Value>()
        .await
        .context("parse Odysee resolve JSON response")?;

    let result = &res["result"][url];
    if result.is_null() {
        bail!("Could not resolve Odysee/LBRY URL: {}", url);
    }

    let canonical_url = result["canonical_url"]
        .as_str()
        .context("no canonical_url in resolved details")?;
    let title = result["value"]["title"]
        .as_str()
        .unwrap_or("Odysee Video")
        .to_string();
    let thumbnail = result["value"]["thumbnail"]["url"]
        .as_str()
        .map(String::from);
    let uploader = result["signing_channel"]["value"]["title"]
        .as_str()
        .or_else(|| result["signing_channel"]["name"].as_str())
        .map(String::from);
    let duration_secs = result["value"]["video"]["duration"]
        .as_f64()
        .or_else(|| result["value"]["audio"]["duration"].as_f64());

    // 2. Fetch the streaming URL using 'get' method
    let get_payload = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "get",
        "params": {
            "uri": canonical_url
        },
        "id": 1
    });

    let get_res = client
        .post("https://api.odysee.com/api/v1/proxy")
        .json(&get_payload)
        .header("Content-Type", "application/json")
        .send()
        .await
        .with_context(|| format!("POST get to Odysee API for {canonical_url}"))?
        .error_for_status()
        .context("Odysee API get returned error status")?
        .json::<Value>()
        .await
        .context("parse Odysee get JSON response")?;

    let streaming_url = get_res["result"]["streaming_url"]
        .as_str()
        .or_else(|| get_res["result"]["download_url"].as_str())
        .context("no streaming_url or download_url in get response")?;

    let is_audio = !result["value"]["audio"].is_null();
    let ext = "mp4".to_string();

    let format = ExtractedFormat {
        id: result["claim_id"]
            .as_str()
            .unwrap_or("odysee_stream")
            .to_string(),
        label: "Direct Stream".to_string(),
        url: streaming_url.to_string(),
        ext: ext.clone(),
        protocol: Some("https".to_string()),
        manifest_url: None,
        format_note: Some("Direct".to_string()),
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
        is_audio_only: is_audio,
    };

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats: vec![format],
        extractor: "odysee".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_odysee_urls() {
        assert!(matches("https://odysee.com/@SomeCreator/SomeVideo"));
        assert!(matches("https://lbry.tv/@SomeCreator/SomeVideo"));
        assert!(!matches("https://google.com"));
    }
}
