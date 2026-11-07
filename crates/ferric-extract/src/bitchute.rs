//! BitChute — page scrape for media URL.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["bitchute.com"])
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

    let title = find_og(&html, "og:title").unwrap_or_else(|| "BitChute".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    for key in [
        "id=\"player\"",
        "source src=\"",
        "\"media_url\":\"",
        "\"file\":\"",
    ] {
        if let Some(pos) = html.find(key) {
            let window = &html[pos..].get(..800).unwrap_or(&html[pos..]);
            for attr_key in ["src=\"", "content=\""] {
                if let Some(u) = find_between(window, attr_key, "\"") {
                    if u.contains("http") && (u.contains(".mp4") || u.contains("m3u8")) {
                        let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                        formats.push(direct_format("bc_0", "BitChute", u.to_string(), ext));
                    }
                }
            }
        }
    }

    // seed / seed126 style CDN
    let mut s = html.as_str();
    while let Some(pos) = s.find("https://") {
        let rest = &s[pos..];
        let end = rest
            .find('"')
            .or_else(|| rest.find('\''))
            .unwrap_or(200.min(rest.len()));
        let u = &rest[..end];
        if (u.contains("bitchute.com") || u.contains("seed"))
            && (u.contains(".mp4") || u.contains("m3u8"))
        {
            let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
            formats.push(direct_format(
                &format!("bc_{}", formats.len()),
                "Stream",
                u.to_string(),
                ext,
            ));
        }
        s = &rest[end.max(1)..];
    }

    formats.sort_by(|a, b| a.url.cmp(&b.url));
    formats.dedup_by(|a, b| a.url == b.url);

    if formats.is_empty() {
        bail!("bitchute: no media URL found");
    }

    let mut result = empty_result(title, "bitchute");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_bc() {
        assert!(matches("https://www.bitchute.com/video/abc/"));
    }
}
