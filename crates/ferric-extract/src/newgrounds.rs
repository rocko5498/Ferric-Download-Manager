//! newgrounds.com video/audio — pure Rust HTML + JSON scrape.

use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["newgrounds.com/"])
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
        .context("newgrounds page error")?
        .text()
        .await
        .context("read newgrounds page")?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "Newgrounds".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats: Vec<ExtractedFormat> = Vec::new();

    // Player embeds often expose sources in "sources" JSON or data attributes
    if let Some(sources_json) =
        find_between(&html, "\"sources\":{", "}").or_else(|| find_between(&html, "sources:{", "}"))
    {
        // crude: pull https URLs from the sources blob
        let blob = format!("{{{sources_json}}}");
        for piece in blob.split("https://") {
            if piece.is_empty() {
                continue;
            }
            let end = piece
                .find(|c: char| c == '"' || c == '\'' || c == '\\' || c == ' ')
                .unwrap_or(piece.len().min(512));
            let u = format!("https://{}", &piece[..end]);
            if looks_media(&u) && !formats.iter().any(|f| f.url == u) {
                let ext = ext_of(&u);
                formats.push(direct_format(&ext, &ext, u, &ext));
            }
        }
    }

    if let Some(v) = find_og(&html, "og:video").or_else(|| find_og(&html, "og:video:secure_url")) {
        if !formats.iter().any(|f| f.url == v) {
            formats.push(direct_format("og-video", "Video", v, "mp4"));
        }
    }

    // data-src / source tags style
    for marker in ["src=\"https://", "src=\\\"https://", "\"url\":\"https://"] {
        let mut search = html.as_str();
        while let Some(i) = search.find(marker) {
            let rest = &search[i + marker.len() - "https://".len()..];
            if let Some(u) = take_url(rest) {
                if looks_media(&u) && !formats.iter().any(|f| f.url == u) {
                    let ext = ext_of(&u);
                    formats.push(direct_format(
                        &format!("src-{}", formats.len()),
                        &ext,
                        u,
                        &ext,
                    ));
                }
            }
            search = &search[i + marker.len()..];
            if formats.len() > 12 {
                break;
            }
        }
    }

    if formats.is_empty() {
        bail!("newgrounds: no media URL found (age gate / login possible)");
    }

    let mut result = empty_result(title, "newgrounds");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn take_url(s: &str) -> Option<String> {
    if !s.starts_with("https://") {
        return None;
    }
    let end = s
        .find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '\\' || c == '>')
        .unwrap_or(s.len().min(2048));
    Some(s[..end].replace("\\/", "/"))
}

fn looks_media(u: &str) -> bool {
    let l = u.to_ascii_lowercase();
    l.contains(".mp4")
        || l.contains(".webm")
        || l.contains(".m3u8")
        || l.contains(".mp3")
        || l.contains("ngfiles.com")
        || l.contains("uploads.ungrounded.net")
}

fn ext_of(u: &str) -> String {
    let path = u.split('?').next().unwrap_or(u);
    path.rsplit('.')
        .next()
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("mp4")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_portal() {
        assert!(matches("https://www.newgrounds.com/portal/view/12345"));
        assert!(!matches("https://example.com/"));
    }
}
