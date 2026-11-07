//! mediafire.com — pure Rust (page scrape for direct download).

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["mediafire.com/"])
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
        .context("mediafire page error")?
        .text()
        .await
        .context("read mediafire page")?;

    let title = find_og(&html, "og:title").unwrap_or_else(|| "MediaFire file".into());

    let mut formats = Vec::new();

    if let Some(href) = find_between(&html, "id=\"downloadButton\" href=\"", "\"")
        .or_else(|| find_between(&html, "id='downloadButton' href='", "'"))
        .or_else(|| find_between(&html, "\"downloadUrl\":\"", "\""))
    {
        let u = href.replace("\\/", "/");
        if u.starts_with("http") {
            let ext = ext_guess(&u, &title);
            formats.push(direct_format("direct", "Direct download", u, &ext));
        }
    }

    // Fallback: first mediafire download CDN link in page
    if formats.is_empty() {
        if let Some(i) = html.find("https://download") {
            if let Some(u) = take_https(&html[i..]) {
                if u.contains("mediafire.com") {
                    let ext = ext_guess(&u, &title);
                    formats.push(direct_format("cdn", "CDN download", u, &ext));
                }
            }
        }
    }

    if formats.is_empty() {
        bail!("mediafire: no download URL (password/folder pages unsupported)");
    }

    let mut result = empty_result(title, "mediafire");
    result.formats = formats;
    Ok(result)
}

fn take_https(s: &str) -> Option<String> {
    if !s.starts_with("https://") && !s.starts_with("http://") {
        return None;
    }
    let end = s
        .find(|c: char| c == '"' || c == '\'' || c == ' ' || c == '>')
        .unwrap_or(s.len().min(2048));
    Some(s[..end].to_string())
}

fn ext_guess(url: &str, title: &str) -> String {
    let path = url.split('?').next().unwrap_or(url);
    path.rsplit('.')
        .next()
        .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .or_else(|| {
            title
                .rsplit('.')
                .next()
                .filter(|e| e.len() <= 5 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        })
        .unwrap_or("bin")
        .to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_file() {
        assert!(matches("https://www.mediafire.com/file/abc/file.zip"));
        assert!(!matches("https://example.com/"));
    }
}
