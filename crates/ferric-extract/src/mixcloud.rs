//! mixcloud.com — pure Rust (page hydration + cloudcast stream).

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["mixcloud.com/"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", UA)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("mixcloud page error")?
        .text()
        .await
        .context("read mixcloud page")?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "Mixcloud".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    // Look for stream URL in embedded JSON
    if let Some(stream) = find_between(&html, "\"stream_url\":\"", "\"")
        .or_else(|| find_between(&html, "\"streamUrl\":\"", "\""))
        .or_else(|| find_between(&html, "stream_url\":\"", "\""))
    {
        let stream = stream.replace("\\u002F", "/").replace("\\/", "/");
        if stream.starts_with("http") {
            formats.push(direct_format("stream", "Stream", stream, "m4a"));
        }
    }

    // window.__INITIAL_STATE__ style blobs
    if formats.is_empty() {
        if let Some(blob) = find_between(&html, "window.__INITIAL_STATE__=", ";</script>")
            .or_else(|| find_between(&html, "__INITIAL_STATE__=", "</script>"))
        {
            if let Ok(v) = serde_json::from_str::<Value>(blob.trim().trim_start_matches('=')) {
                collect_stream_urls(&v, &mut formats);
            }
        }
    }

    if formats.is_empty() {
        if let Some(audio) =
            find_og(&html, "og:audio").or_else(|| find_og(&html, "og:audio:secure_url"))
        {
            formats.push(direct_format("og-audio", "Audio", audio, "m4a"));
        }
    }

    if formats.is_empty() {
        bail!("mixcloud: no stream URL (may need login or API token)");
    }

    for f in &mut formats {
        f.is_audio_only = true;
        f.acodec = Some("aac".into());
        f.vcodec = Some("none".into());
    }

    let mut result = empty_result(title, "mixcloud");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn collect_stream_urls(v: &Value, out: &mut Vec<ferric_common::ExtractedFormat>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let kl = k.to_ascii_lowercase();
                if (kl.contains("stream") || kl == "url") && val.is_string() {
                    if let Some(u) = val.as_str() {
                        if u.starts_with("http")
                            && (u.contains("m4a")
                                || u.contains("mp3")
                                || u.contains("stream")
                                || u.contains(".m3u8"))
                            && !out.iter().any(|f| f.url == u)
                        {
                            let ext = if u.contains("mp3") { "mp3" } else { "m4a" };
                            out.push(direct_format(k, k, u.to_string(), ext));
                        }
                    }
                }
                collect_stream_urls(val, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                collect_stream_urls(item, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_track() {
        assert!(matches("https://www.mixcloud.com/user/show-name/"));
        assert!(!matches("https://soundcloud.com/x"));
    }
}
