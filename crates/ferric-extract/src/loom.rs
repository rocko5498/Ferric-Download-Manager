//! loom.com share pages — pure Rust (og:video + page JSON).

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["loom.com/"])
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
        .context("loom page error")?
        .text()
        .await
        .context("read loom page")?;

    let title = find_og(&html, "og:title")
        .or_else(|| find_og(&html, "twitter:title"))
        .unwrap_or_else(|| "Loom video".into());
    let thumbnail = find_og(&html, "og:image");

    let mut formats = Vec::new();

    if let Some(v) = find_og(&html, "og:video")
        .or_else(|| find_og(&html, "og:video:secure_url"))
        .or_else(|| find_og(&html, "twitter:player:stream"))
    {
        let ext = if v.contains(".m3u8") { "m3u8" } else { "mp4" };
        formats.push(direct_format("og-video", "Loom video", v, ext));
    }

    // Common Loom hydration keys
    for (key, label) in [
        ("\"url\":\"https://cdn.loom.com", "cdn"),
        ("\"source\":\"https://", "source"),
        ("\"transcodedUrl\":\"", "transcoded"),
        ("\"raw_url\":\"", "raw"),
        ("\"playableUrl\":\"", "playable"),
    ] {
        if let Some(rest) = html.find(key).map(|i| &html[i..]) {
            if let Some(u) = extract_quoted_https(rest) {
                if looks_like_media(&u) && !formats.iter().any(|f| f.url == u) {
                    let ext = if u.contains(".m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(label, label, u, ext));
                }
            }
        }
    }

    if let Some(m3u8) = find_between(&html, "https://", ".m3u8") {
        let u = format!("https://{m3u8}.m3u8");
        if !formats.iter().any(|f| f.url == u) {
            formats.push(direct_format("hls", "HLS", u, "m3u8"));
        }
    }

    if formats.is_empty() {
        bail!("loom: no playable URL found (may require login)");
    }

    let mut result = empty_result(title, "loom");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn extract_quoted_https(s: &str) -> Option<String> {
    let i = s.find("https://")?;
    let rest = &s[i..];
    let end = rest
        .find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '\\')
        .unwrap_or(rest.len().min(2048));
    let u = rest[..end].replace("\\u002F", "/").replace("\\/", "/");
    if u.starts_with("https://") {
        Some(u)
    } else {
        None
    }
}

fn looks_like_media(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.contains(".mp4")
        || l.contains(".m3u8")
        || l.contains(".webm")
        || l.contains("cdn.loom.com")
        || l.contains("/transcoded/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_share() {
        assert!(matches("https://www.loom.com/share/abc123"));
        assert!(!matches("https://example.com/loom"));
    }
}
