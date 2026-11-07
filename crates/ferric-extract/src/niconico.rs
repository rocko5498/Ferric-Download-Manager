//! Niconico (nicovideo.jp) — needs cookies for many videos; scrapes smile/hls when public.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["nicovideo.jp", "nico.ms"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", UA)
        .header("Accept-Language", "ja,en;q=0.8")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("Niconico page error")?
        .text()
        .await?;

    let title = find_og(&html, "og:title")
        .or_else(|| find_between(&html, "<title>", "</title>").map(|s| html_unescape(s.trim())))
        .unwrap_or_else(|| "Niconico Video".into());
    let thumbnail = find_og(&html, "og:image");

    let mut formats = Vec::new();
    // data-api-data JSON often has media.delivery
    if let Some(json) = find_between(&html, "data-api-data=\"", "\"")
        .map(|s| html_unescape(s))
        .or_else(|| {
            find_between(&html, "id=\"js-initial-watch-data\" data-api-data=\"", "\"")
                .map(html_unescape)
        })
    {
        for key in ["\"url\":\"", "\"contentUrl\":\""] {
            let mut s = json.as_str();
            while let Some(rest) = s.find(key).map(|i| &s[i + key.len()..]) {
                if let Some(end) = rest.find('"') {
                    let u = rest[..end].replace("\\/", "/").replace("\\u0026", "&");
                    if u.starts_with("http")
                        && (u.contains("m3u8") || u.contains("mp4") || u.contains("smile"))
                    {
                        let ext = if u.contains("m3u8") { "m3u8" } else { "mp4" };
                        formats.push(direct_format(
                            &format!("nico_{}", formats.len()),
                            "Niconico",
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
    }

    if formats.is_empty() {
        bail!("Niconico: no stream (login cookies usually required)");
    }

    let mut result = empty_result(title, "niconico");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn html_unescape(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&amp;", "&")
        .replace("&#039;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_nico() {
        assert!(matches("https://www.nicovideo.jp/watch/sm9"));
        assert!(matches("https://nico.ms/sm9"));
    }
}
