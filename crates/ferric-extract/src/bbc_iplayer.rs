//! BBC iPlayer — geofenced; scrapes mediaselector / playlist JSON when available.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(
        url,
        &["bbc.co.uk/iplayer", "bbc.com/iplayer", "bbc.co.uk/sounds"],
    )
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", UA)
        .header("Accept-Language", "en-GB,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("BBC iPlayer page error")?
        .text()
        .await?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "BBC iPlayer".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    for key in ["\"href\":\"", "\"url\":\"", "\"media_file_url\":\""] {
        let mut s = html.as_str();
        while let Some(rest) = s.find(key).map(|i| &s[i + key.len()..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/");
                if u.contains(".m3u8") || u.contains("mediaselector") {
                    formats.push(direct_format(
                        &format!("bbc_{}", formats.len()),
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

    // vpid for mediaselector
    if formats.is_empty() {
        if let Some(vpid) = find_between(&html, "\"vpid\":\"", "\"")
            .or_else(|| find_between(&html, "data-vpid=\"", "\""))
        {
            let api = format!(
                "https://open.live.bbc.co.uk/mediaselector/6/select/version/2.0/mediaset/pc/vpid/{vpid}/format/json"
            );
            if let Ok(body) = client.get(&api).header("User-Agent", UA).send().await {
                if let Ok(text) = body.text().await {
                    let mut s = text.as_str();
                    while let Some(rest) = s.find("\"href\":\"").map(|i| &s[i + 8..]) {
                        if let Some(end) = rest.find('"') {
                            let u = rest[..end].replace("\\/", "/");
                            if u.contains("m3u8") {
                                formats.push(direct_format(
                                    &format!("bbc_ms_{}", formats.len()),
                                    "MediaSelector",
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
            }
        }
    }

    if formats.is_empty() {
        bail!("BBC iPlayer: no stream (UK geo + cookies usually required)");
    }

    let mut result = empty_result(title, "bbc_iplayer");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_bbc() {
        assert!(matches(
            "https://www.bbc.co.uk/iplayer/episode/b0xxxx/title"
        ));
    }
}
