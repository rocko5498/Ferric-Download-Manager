//! Crunchyroll free/public stream scrape (login-walled titles need cookies).

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["crunchyroll.com"])
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
        .context("Crunchyroll page error")?
        .text()
        .await?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "Crunchyroll".into());
    let thumbnail = find_og(&html, "og:image");
    let description = find_og(&html, "og:description");

    let mut formats = Vec::new();
    for key in ["\"url\":\"", "\"href\":\"", "\"stream_url\":\""] {
        let mut s = html.as_str();
        while let Some(rest) = s.find(key).map(|i| &s[i + key.len()..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/");
                if u.contains(".m3u8") || u.contains("playlist.m3u8") {
                    formats.push(direct_format(
                        &format!("cr_{}", formats.len()),
                        "HLS",
                        u,
                        "m3u8",
                    ));
                }
                s = &rest[end + 1..];
            } else {
                break;
            }
        }
    }

    // __NEXT_DATA__ blob
    if formats.is_empty() {
        if let Some(blob) = find_between(
            &html,
            "id=\"__NEXT_DATA__\" type=\"application/json\">",
            "</script>",
        ) {
            if let Some(u) = find_between(blob, "\"url\":\"https://", ".m3u8") {
                let full = format!("https://{u}.m3u8").replace("\\/", "/");
                formats.push(direct_format("cr_next", "HLS", full, "m3u8"));
            }
        }
    }

    if formats.is_empty() {
        bail!("Crunchyroll: no free stream (premium/login or geo block)");
    }

    let mut result = empty_result(title, "crunchyroll");
    result.thumbnail = thumbnail;
    result.description = description;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_cr() {
        assert!(matches("https://www.crunchyroll.com/watch/GXk123/ep"));
    }
}
