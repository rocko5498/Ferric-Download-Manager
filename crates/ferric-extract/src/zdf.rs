//! ZDF Mediathek — public ptmd/tmd stream JSON.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["zdf.de"])
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

    let title = find_og(&html, "og:title").unwrap_or_else(|| "ZDF".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    // data-zdfplayer-jsb or content URL
    let content_url = find_between(&html, "\"content\":\"", "\"")
        .or_else(|| find_between(&html, "data-zdfplayer-id=\"", "\""))
        .map(|s| s.replace("\\/", "/"));

    if let Some(api) = content_url {
        let api = if api.starts_with("http") {
            api
        } else {
            format!("https://api.zdf.de{api}")
        };
        if let Ok(resp) = client
            .get(&api)
            .header("User-Agent", UA)
            .header(
                "Api-Auth",
                "Bearer d2726b6c8c655e42b68b0db26131b15b22bd1a32",
            )
            .send()
            .await
        {
            if let Ok(v) = resp.json::<Value>().await {
                walk_urls(&v, &mut formats);
            }
        }
    }

    if formats.is_empty() {
        if let Some(u) = find_between(&html, "https://", ".m3u8") {
            formats.push(direct_format(
                "zdf_page",
                "HLS",
                format!("https://{u}.m3u8"),
                "m3u8",
            ));
        }
    }

    if formats.is_empty() {
        bail!("ZDF: no stream found");
    }

    let mut result = empty_result(title, "zdf");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn walk_urls(v: &Value, formats: &mut Vec<ferric_common::ExtractedFormat>) {
    match v {
        Value::Object(m) => {
            if let Some(u) = m.get("url").and_then(|u| u.as_str()) {
                if u.contains("m3u8") || u.ends_with(".mp4") {
                    let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(
                        &format!("zdf_{}", formats.len()),
                        "Stream",
                        u.to_string(),
                        ext,
                    ));
                }
            }
            for child in m.values() {
                walk_urls(child, formats);
            }
        }
        Value::Array(a) => {
            for child in a {
                walk_urls(child, formats);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_zdf() {
        assert!(matches("https://www.zdf.de/filme/x/video.html"));
    }
}
