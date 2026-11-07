//! streamable.com — public API.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["streamable.com"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let id = shortcode(url).ok_or_else(|| anyhow::anyhow!("streamable: no shortcode"))?;
    let api = format!("https://api.streamable.com/videos/{id}");
    let v: Value = client
        .get(&api)
        .header("User-Agent", UA)
        .send()
        .await
        .with_context(|| format!("GET {api}"))?
        .error_for_status()?
        .json()
        .await
        .context("parse streamable JSON")?;

    let title = v
        .get("title")
        .and_then(|t| t.as_str())
        .unwrap_or("Streamable")
        .to_string();
    let thumbnail = v.get("thumbnail_url").and_then(|t| t.as_str()).map(|t| {
        if t.starts_with("//") {
            format!("https:{t}")
        } else {
            t.to_string()
        }
    });

    let mut formats = Vec::new();
    if let Some(files) = v.get("files").and_then(|f| f.as_object()) {
        for (quality, meta) in files {
            if let Some(u) = meta.get("url").and_then(|u| u.as_str()) {
                let url = if u.starts_with("//") {
                    format!("https:{u}")
                } else {
                    u.to_string()
                };
                let height = meta
                    .get("height")
                    .and_then(|h| h.as_u64())
                    .map(|h| h as u32);
                let mut f = direct_format(quality, quality, url, "mp4");
                f.height = height;
                formats.push(f);
            }
        }
    }

    if formats.is_empty() {
        bail!("streamable: no files in API response");
    }

    let mut result = empty_result(title, "streamable");
    result.thumbnail = thumbnail;
    result.duration_secs = v.get("duration").and_then(|d| d.as_f64());
    result.formats = formats;
    Ok(result)
}

fn shortcode(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    u.path_segments()?
        .find(|s| !s.is_empty() && *s != "e")
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_and_code() {
        assert!(matches("https://streamable.com/abc12"));
        assert_eq!(
            shortcode("https://streamable.com/abc12").as_deref(),
            Some("abc12")
        );
    }
}
