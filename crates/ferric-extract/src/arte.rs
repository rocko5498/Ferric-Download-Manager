//! Arte.tv — public API config JSON.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["arte.tv"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", UA)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()?
        .text()
        .await?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "ARTE".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    // program id from URL /videos/{lang}/{id}/...
    let program_id = extract_program_id(url)
        .or_else(|| find_between(&html, "\"programId\":\"", "\"").map(str::to_string));

    if let Some(pid) = program_id {
        for lang in ["en", "fr", "de", "es", "pl", "it"] {
            let api = format!("https://api.arte.tv/api/player/v2/config/{lang}/{pid}");
            if let Ok(resp) = client.get(&api).header("User-Agent", UA).send().await {
                if let Ok(v) = resp.json::<Value>().await {
                    collect_arte_streams(&v, &mut formats);
                    if !formats.is_empty() {
                        break;
                    }
                }
            }
        }
    }

    // fallback scrape
    if formats.is_empty() {
        if let Some(u) = find_between(&html, "\"https://", ".m3u8") {
            formats.push(direct_format(
                "arte_page",
                "HLS",
                format!("https://{u}.m3u8"),
                "m3u8",
            ));
        }
    }

    if formats.is_empty() {
        bail!("ARTE: no stream found");
    }

    let mut result = empty_result(title, "arte");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn extract_program_id(url: &str) -> Option<String> {
    // .../videos/en/123456-000-A/...
    let u = url::Url::parse(url).ok()?;
    let segs: Vec<_> = u.path_segments()?.collect();
    segs.iter()
        .find(|s| s.chars().next().is_some_and(|c| c.is_ascii_digit()) && s.contains('-'))
        .map(|s| (*s).to_string())
}

fn collect_arte_streams(v: &Value, formats: &mut Vec<ferric_common::ExtractedFormat>) {
    fn walk(val: &Value, out: &mut Vec<(String, Option<u32>)>) {
        match val {
            Value::Object(map) => {
                if let Some(url) = map.get("url").and_then(|u| u.as_str()) {
                    if url.contains("m3u8") || url.contains("mp4") {
                        let h = map.get("height").and_then(|h| h.as_u64()).map(|h| h as u32);
                        out.push((url.to_string(), h));
                    }
                }
                for v in map.values() {
                    walk(v, out);
                }
            }
            Value::Array(arr) => {
                for v in arr {
                    walk(v, out);
                }
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    walk(v, &mut found);
    for (i, (url, h)) in found.into_iter().enumerate() {
        let ext = if url.contains("m3u8") { "m3u8" } else { "mp4" };
        let mut f = direct_format(
            &format!("arte_{i}"),
            &h.map(|x| format!("{x}p"))
                .unwrap_or_else(|| "Stream".into()),
            url,
            ext,
        );
        f.height = h;
        formats.push(f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_arte() {
        assert!(matches("https://www.arte.tv/en/videos/123456-000-A/title/"));
    }
    #[test]
    fn program_id() {
        assert_eq!(
            extract_program_id("https://www.arte.tv/en/videos/123456-000-A/foo/").as_deref(),
            Some("123456-000-A")
        );
    }
}
