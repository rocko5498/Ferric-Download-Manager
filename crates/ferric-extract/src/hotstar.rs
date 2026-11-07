//! Disney+ Hotstar — public/page scrape; DRM titles flagged.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["hotstar.com", "disneyplus.com"])
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header("User-Agent", UA)
        .header("Accept-Language", "en-IN,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("Hotstar page error")?
        .text()
        .await?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "Hotstar".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();
    let mut has_drm = false;

    if html.contains("widevine") || html.contains("\"drm\"") {
        has_drm = true;
    }

    for key in ["\"playbackUrl\":\"", "\"hlsMasterTagUrl\":\"", "\"url\":\""] {
        let mut s = html.as_str();
        while let Some(rest) = s.find(key).map(|i| &s[i + key.len()..]) {
            if let Some(end) = rest.find('"') {
                let u = rest[..end].replace("\\/", "/");
                if u.starts_with("http") && (u.contains("m3u8") || u.contains("mpd")) {
                    let ext = if u.contains("mpd") { "mpd" } else { "m3u8" };
                    let mut f = direct_format(&format!("hs_{}", formats.len()), "Stream", u, ext);
                    f.has_drm = has_drm;
                    formats.push(f);
                }
                s = &rest[end + 1..];
            } else {
                break;
            }
        }
    }

    if formats.is_empty() {
        bail!("Hotstar: no cleartext stream (most titles are DRM — cookies may not help)");
    }

    let mut result = empty_result(title, "hotstar");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_hs() {
        assert!(matches("https://www.hotstar.com/in/movies/x/1"));
        assert!(matches("https://www.disneyplus.com/video/abc"));
    }
}
