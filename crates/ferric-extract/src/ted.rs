//! TED Talks — public talk page JSON.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["ted.com"])
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

    let title = find_og(&html, "og:title").unwrap_or_else(|| "TED Talk".into());
    let thumbnail = find_og(&html, "og:image");
    let description = find_og(&html, "og:description");
    let mut formats = Vec::new();

    // __NEXT_DATA__ or talk player JSON
    if let Some(blob) = find_between(
        &html,
        "id=\"__NEXT_DATA__\" type=\"application/json\">",
        "</script>",
    )
    .or_else(|| find_between(&html, "\"playerData\":\"", "\""))
    {
        let json_text = if blob.trim_start().starts_with('{') {
            blob.to_string()
        } else {
            // escaped JSON string
            blob.replace("\\\"", "\"").replace("\\\\", "\\")
        };
        if let Ok(v) = serde_json::from_str::<Value>(&json_text) {
            walk_resources(&v, &mut formats);
        }
        // also raw url scrape
        let mut s = blob;
        while let Some(rest) = s.find("https://").map(|i| &s[i..]) {
            let end = rest
                .find(|c: char| c == '"' || c == '\\' || c == ' ' || c == '\'')
                .unwrap_or(rest.len().min(500));
            let u = rest[..end].replace("\\u0026", "&");
            if u.contains(".mp4") || u.contains("m3u8") {
                let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                formats.push(direct_format(
                    &format!("ted_{}", formats.len()),
                    "TED",
                    u,
                    ext,
                ));
            }
            s = &rest[end.max(1)..];
            if s.is_empty() {
                break;
            }
        }
    }

    if formats.is_empty() {
        if let Some(v) = find_og(&html, "og:video") {
            formats.push(direct_format("ted_og", "Video", v, "mp4"));
        }
    }

    if formats.is_empty() {
        bail!("TED: no downloadable stream found");
    }

    // dedupe by url
    formats.sort_by(|a, b| a.url.cmp(&b.url));
    formats.dedup_by(|a, b| a.url == b.url);

    let mut result = empty_result(title, "ted");
    result.thumbnail = thumbnail;
    result.description = description;
    result.formats = formats;
    Ok(result)
}

fn walk_resources(v: &Value, formats: &mut Vec<ferric_common::ExtractedFormat>) {
    match v {
        Value::Object(m) => {
            if let (Some(url), h) = (
                m.get("file")
                    .or_else(|| m.get("uri"))
                    .or_else(|| m.get("url"))
                    .and_then(|u| u.as_str()),
                m.get("height").and_then(|h| h.as_u64()).map(|h| h as u32),
            ) {
                if url.contains("http") && (url.contains("mp4") || url.contains("m3u8")) {
                    let ext = if url.contains("m3u8") { "m3u8" } else { "mp4" };
                    let mut f = direct_format(
                        &format!("ted_{}", formats.len()),
                        &h.map(|x| format!("{x}p"))
                            .unwrap_or_else(|| "Stream".into()),
                        url.to_string(),
                        ext,
                    );
                    f.height = h;
                    formats.push(f);
                }
            }
            for c in m.values() {
                walk_resources(c, formats);
            }
        }
        Value::Array(a) => {
            for c in a {
                walk_resources(c, formats);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_ted() {
        assert!(matches("https://www.ted.com/talks/foo_bar"));
    }
}
