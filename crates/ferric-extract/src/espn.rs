//! ESPN video page extractor — og:video / m3u8 scrape.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["espn.com", "espn.go.com", "watchespn.com"])
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
        .context("ESPN page error")?
        .text()
        .await?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "ESPN Video".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    if let Some(v) = find_og(&html, "og:video")
        .or_else(|| find_og(&html, "og:video:url"))
        .or_else(|| find_og(&html, "og:video:secure_url"))
    {
        let ext = if v.contains("m3u8") { "m3u8" } else { "mp4" };
        formats.push(direct_format("espn_og", "ESPN", v, ext));
    }

    // JSON blob paths
    for key in ["\"hls\"", "\"m3u8\"", "\"stream\"", "\"source\""] {
        let mut s = html.as_str();
        while let Some(pos) = s.find(key) {
            let window = &s[pos..].get(..300).unwrap_or(&s[pos..]);
            if let Some(u) = find_between(window, "\"http", "\"") {
                let full = format!("http{u}").replace("\\/", "/");
                if full.contains(".m3u8") || full.contains(".mp4") {
                    let ext = if full.contains("m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(
                        &format!("espn_{}", formats.len()),
                        "Stream",
                        full,
                        ext,
                    ));
                }
            }
            s = &s[pos + key.len()..];
        }
    }

    if formats.is_empty() {
        bail!("ESPN: no stream found (often geo/login gated)");
    }

    let mut result = empty_result(title, "espn");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_espn() {
        assert!(matches("https://www.espn.com/video/clip?id=1"));
        assert!(!matches("https://cnn.com"));
    }
}
