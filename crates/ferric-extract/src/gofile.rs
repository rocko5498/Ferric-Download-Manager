//! gofile.io — pure Rust (content API, public files).

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["gofile.io/d/"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let content_id = content_id(url).ok_or_else(|| anyhow::anyhow!("gofile: no content id"))?;

    // Guest account token (public API pattern)
    let token_resp: Value = client
        .post("https://api.gofile.io/accounts")
        .header("User-Agent", UA)
        .send()
        .await
        .context("gofile create guest")?
        .error_for_status()
        .context("gofile accounts status")?
        .json()
        .await
        .context("gofile accounts json")?;

    let token = token_resp
        .pointer("/data/token")
        .and_then(|t| t.as_str())
        .ok_or_else(|| anyhow::anyhow!("gofile: no guest token"))?
        .to_string();

    let content_url =
        format!("https://api.gofile.io/contents/{content_id}?wt=4fd6sg89d7s6&cache=true");
    let v: Value = client
        .get(&content_url)
        .header("User-Agent", UA)
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .context("gofile contents")?
        .error_for_status()
        .context("gofile contents status")?
        .json()
        .await
        .context("gofile contents json")?;

    let data = v
        .get("data")
        .ok_or_else(|| anyhow::anyhow!("gofile: no data"))?;
    let title = data
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("gofile")
        .to_string();

    let mut formats = Vec::new();
    if let Some(children) = data.get("children").and_then(|c| c.as_object()) {
        for (id, child) in children {
            let name = child
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or(id)
                .to_string();
            let link = child
                .get("link")
                .and_then(|l| l.as_str())
                .map(str::to_string);
            let size = child.get("size").and_then(|s| s.as_u64());
            if let Some(link) = link {
                let ext = name
                    .rsplit('.')
                    .next()
                    .filter(|e| e.len() <= 5)
                    .unwrap_or("bin");
                let mut f = direct_format(id, &name, link, ext);
                f.filesize = size;
                formats.push(f);
            }
        }
    }

    if formats.is_empty() {
        bail!("gofile: no children/links (folder empty or passworded)");
    }

    let mut result = empty_result(title, "gofile");
    result.formats = formats;
    Ok(result)
}

fn content_id(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let mut segs = u.path_segments()?;
    if segs.next()? != "d" {
        return None;
    }
    segs.next().map(str::to_string).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn id() {
        assert_eq!(
            content_id("https://gofile.io/d/AbCdEf").as_deref(),
            Some("AbCdEf")
        );
        assert!(matches("https://gofile.io/d/xyz"));
    }
}
