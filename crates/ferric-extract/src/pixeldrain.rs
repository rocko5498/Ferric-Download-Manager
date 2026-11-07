//! pixeldrain.com — pure Rust public API.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["pixeldrain.com/u/", "pixeldrain.com/l/"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let id = file_id(url).ok_or_else(|| anyhow::anyhow!("pixeldrain: no file id"))?;
    let api = format!("https://pixeldrain.com/api/file/{id}/info");
    let v: Value = client
        .get(&api)
        .header("User-Agent", UA)
        .send()
        .await
        .with_context(|| format!("GET {api}"))?
        .error_for_status()
        .context("pixeldrain info error")?
        .json()
        .await
        .context("parse pixeldrain JSON")?;

    let title = v
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("pixeldrain")
        .to_string();
    let size = v.get("size").and_then(|s| s.as_u64());
    let mime = v
        .get("mime_type")
        .and_then(|m| m.as_str())
        .unwrap_or("application/octet-stream");
    let ext = title
        .rsplit('.')
        .next()
        .filter(|e| e.len() <= 5)
        .unwrap_or("bin")
        .to_string();

    let dl = format!("https://pixeldrain.com/api/file/{id}");
    let mut f = direct_format("original", mime, dl, &ext);
    f.filesize = size;

    if f.url.is_empty() {
        bail!("pixeldrain: empty download url");
    }

    let mut result = empty_result(title, "pixeldrain");
    result.formats = vec![f];
    Ok(result)
}

fn file_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let mut segs = u.path_segments()?;
    // /u/ID or /l/ID
    let kind = segs.next()?;
    if kind != "u" && kind != "l" {
        return None;
    }
    segs.next().map(str::to_string).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id_parse() {
        assert_eq!(
            file_id("https://pixeldrain.com/u/abc123").as_deref(),
            Some("abc123")
        );
        assert!(matches("https://pixeldrain.com/u/xyz"));
    }
}
