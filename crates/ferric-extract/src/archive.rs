//! archive.org / Internet Archive — pure Rust IE.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;

use crate::util::{direct_format, empty_result, find_between, find_og, host_contains, UA};

pub fn matches(url: &str) -> bool {
    host_contains(url, &["archive.org"])
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

    let title = find_og(&html, "og:title")
        .or_else(|| find_between(&html, "<title>", "</title>").map(|s| s.trim().to_string()))
        .unwrap_or_else(|| "Internet Archive".into());
    let thumbnail = find_og(&html, "og:image");
    let mut formats = Vec::new();

    // File list links: /download/IDENTIFIER/file.mp4
    let mut s = html.as_str();
    while let Some(pos) = s.find("/download/") {
        let rest = &s[pos..];
        let end = rest
            .find('"')
            .or_else(|| rest.find('\''))
            .or_else(|| rest.find(' '))
            .unwrap_or(rest.len().min(300));
        let path = &rest[..end];
        if path.contains(".mp4")
            || path.contains(".mp3")
            || path.contains(".mkv")
            || path.contains(".webm")
            || path.contains(".ogv")
            || path.contains(".m3u8")
        {
            let full = if path.starts_with("http") {
                path.to_string()
            } else {
                format!("https://archive.org{path}")
            };
            let ext = path
                .rsplit('.')
                .next()
                .unwrap_or("bin")
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .unwrap_or("bin");
            formats.push(direct_format(
                &format!("ia_{}", formats.len()),
                ext,
                full,
                ext,
            ));
        }
        s = &rest[end.max(1)..];
    }

    // metadata API if identifier present
    if formats.is_empty() {
        if let Some(id) = identifier_from_url(url) {
            let api = format!("https://archive.org/metadata/{id}");
            if let Ok(resp) = client.get(&api).header("User-Agent", UA).send().await {
                if let Ok(v) = resp.json::<serde_json::Value>().await {
                    if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
                        for f in files {
                            let name = f.get("name").and_then(|n| n.as_str()).unwrap_or("");
                            let format = f.get("format").and_then(|n| n.as_str()).unwrap_or("");
                            if format.contains("MPEG")
                                || format.contains("h.264")
                                || name.ends_with(".mp4")
                                || name.ends_with(".mp3")
                            {
                                let full = format!("https://archive.org/download/{id}/{name}");
                                let ext = name.rsplit('.').next().unwrap_or("mp4");
                                formats.push(direct_format(
                                    &format!("ia_{}", formats.len()),
                                    format,
                                    full,
                                    ext,
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    if formats.is_empty() {
        bail!("archive.org: no media files found");
    }

    // dedupe
    formats.sort_by(|a, b| a.url.cmp(&b.url));
    formats.dedup_by(|a, b| a.url == b.url);

    let mut result = empty_result(title, "archive.org");
    result.thumbnail = thumbnail;
    result.formats = formats;
    Ok(result)
}

fn identifier_from_url(url: &str) -> Option<String> {
    let u = url::Url::parse(url).ok()?;
    let mut segs = u.path_segments()?;
    // /details/IDENTIFIER or /download/IDENTIFIER/
    let first = segs.next()?;
    if first == "details" || first == "download" {
        return segs.next().map(str::to_string);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_ia() {
        assert!(matches("https://archive.org/details/something"));
        assert!(matches("https://archive.org/download/foo/bar.mp4"));
    }
    #[test]
    fn id_parse() {
        assert_eq!(
            identifier_from_url("https://archive.org/details/foo_bar").as_deref(),
            Some("foo_bar")
        );
    }
}
