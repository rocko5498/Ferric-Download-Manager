//! NHK World English — public stream scrape.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["nhk.or.jp", "nhk.jp", "www3.nhk.or.jp"])
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

    let title = find_og(&html, "og:title").unwrap_or_else(|| "NHK".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    for key in ["\"url\":\"", "\"src\":\"", "\"file\":\""] {
        let mut s = html.as_str();
        while let Some(rest) = s.find(key).map(|i| &s[i + key.len()..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/");
                if u.starts_with("http") && (u.contains("m3u8") || u.contains(".mp4")) {
                    let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(
                        &format!("nhk_{}", formats.len()),
                        "NHK",
                        u,
                        ext,
                    ));
                }
                s = &rest[end + 1..];
            } else {
                break;
            }
        }
    }

    if formats.is_empty() {
        if let Some(u) = find_between(&html, "https://", ".m3u8") {
            formats.push(direct_format(
                "nhk_hls",
                "HLS",
                format!("https://{u}.m3u8"),
                "m3u8",
            ));
        }
    }

    if formats.is_empty() {
        bail!("NHK: no stream found");
    }

    let mut result = empty_result(title, "nhk");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_nhk() {
        assert!(matches(
            "https://www3.nhk.or.jp/nhkworld/en/ondemand/video/123/"
        ));
    }
}
