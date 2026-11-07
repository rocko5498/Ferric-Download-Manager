//! OK.ru (odnoklassniki) video — metadata JSON scrape.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["ok.ru", "odnoklassniki.ru"])
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

    let title = find_og(&html, "og:title").unwrap_or_else(|| "OK.ru Video".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    // data-options HTML-escaped JSON with videos[].url
    if let Some(opts) = find_between(&html, "data-options=\"", "\"") {
        let decoded = opts
            .replace("&quot;", "\"")
            .replace("&amp;", "&")
            .replace("&#039;", "'");
        let mut s = decoded.as_str();
        while let Some(rest) = s.find("\"url\":\"").map(|i| &s[i + 7..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/").replace("\\u0026", "&");
                if u.starts_with("http") {
                    let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(
                        &format!("ok_{}", formats.len()),
                        "OK",
                        u,
                        ext,
                    ));
                }
                s = &rest[end + 1..];
            } else {
                break;
            }
        }
        // quality labels nearby
        let _ = decoded.contains("hd");
    }

    if formats.is_empty() {
        bail!("ok.ru: no video URLs (may need cookies)");
    }

    let mut result = empty_result(title, "okru");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_ok() {
        assert!(matches("https://ok.ru/video/123456"));
    }
}
