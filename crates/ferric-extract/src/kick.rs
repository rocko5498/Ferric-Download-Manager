//! Kick.com VOD/live extractor — page JSON + HLS.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["kick.com"])
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
        .context("Kick page error")?
        .text()
        .await
        .context("read Kick page")?;

    let title = find_og(&html, "og:title")
        .or_else(|| find_between(&html, "<title>", "</title>").map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "Kick Video".into());
    let thumbnail = find_og(&html, "og:image");

    let mut formats = Vec::new();
    // Playback URL often embedded as "playback_url":"https://...m3u8"
    for key in ["\"playback_url\":\"", "\"source\":\"", "\"src\":\""] {
        let mut search = html.as_str();
        while let Some(rest) = search.find(key).map(|i| &search[i + key.len()..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/").replace("\\u0026", "&");
                if u.starts_with("http") && (u.contains(".m3u8") || u.contains(".mp4")) {
                    let ext = if u.contains(".m3u8") { "m3u8" } else { "mp4" };
                    formats.push(direct_format(
                        &format!("kick_{}", formats.len()),
                        if ext == "m3u8" { "HLS" } else { "MP4" },
                        u,
                        ext,
                    ));
                }
                search = &rest[end + 1..];
            } else {
                break;
            }
        }
    }

    if formats.is_empty() {
        // Channel live slug: /api/v2/channels/{slug}/livestream
        if let Some(slug) = channel_slug(url) {
            if let Ok(api) = client
                .get(format!(
                    "https://kick.com/api/v2/channels/{slug}/livestream"
                ))
                .header("User-Agent", UA)
                .header("Accept", "application/json")
                .send()
                .await
            {
                if let Ok(text) = api.text().await {
                    if let Some(u) = find_between(&text, "\"playback_url\":\"", "\"") {
                        let u = u.replace("\\/", "/");
                        formats.push(direct_format("kick_live", "Live HLS", u, "m3u8"));
                    }
                }
            }
        }
    }

    if formats.is_empty() {
        bail!("Kick: no playback URL found (may need cookies for private VODs)");
    }

    let mut result = empty_result(title, "kick");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn channel_slug(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let mut segs = u.path_segments()?;
    let first = segs.next()?;
    if first.is_empty() || first == "video" {
        return None;
    }
    Some(first.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_kick() {
        assert!(matches("https://kick.com/xqc"));
        assert!(matches("https://kick.com/video/abc"));
        assert!(!matches("https://twitch.tv/x"));
    }
}
