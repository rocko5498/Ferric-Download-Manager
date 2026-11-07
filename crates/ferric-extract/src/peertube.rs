//! PeerTube instances — public API `/api/v1/videos/{id}`.

use anyhow::{bail, Context, Result};
use ferric_common::ExtractResult;
use serde_json::Value;

use crate::util::{direct_format, empty_result, UA};

pub fn matches(url: &str) -> bool {
    // /w/ or /videos/watch/ is PeerTube's path convention
    let u = url.to_ascii_lowercase();
    (u.contains("/w/") || u.contains("/videos/watch/"))
        && (u.starts_with("http://") || u.starts_with("https://"))
        && !u.contains("youtube.com")
        && !u.contains("vimeo.com")
}

#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let parsed = url::Url::parse(url).context("parse peertube URL")?;
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("no host"))?;
    let id = video_id(&parsed).ok_or_else(|| anyhow::anyhow!("peertube: no video id"))?;
    let api = format!("https://{host}/api/v1/videos/{id}");
    let v: Value = client
        .get(&api)
        .header("User-Agent", UA)
        .send()
        .await
        .with_context(|| format!("GET {api}"))?
        .error_for_status()?
        .json()
        .await
        .context("parse peertube JSON")?;

    let title = v
        .get("name")
        .and_then(|n| n.as_str())
        .unwrap_or("PeerTube")
        .to_string();
    let thumbnail = v.get("thumbnailPath").and_then(|t| t.as_str()).map(|p| {
        if p.starts_with("http") {
            p.to_string()
        } else {
            format!("https://{host}{p}")
        }
    });
    let description = v
        .get("description")
        .and_then(|d| d.as_str())
        .map(str::to_string);
    let duration_secs = v.get("duration").and_then(|d| d.as_f64());
    let uploader = v
        .pointer("/account/displayName")
        .or_else(|| v.pointer("/channel/displayName"))
        .and_then(|n| n.as_str())
        .map(str::to_string);

    let mut formats = Vec::new();
    if let Some(files) = v.get("files").and_then(|f| f.as_array()) {
        for (i, f) in files.iter().enumerate() {
            if let Some(u) = f
                .get("fileUrl")
                .or_else(|| f.pointer("/fileDownloadUrl"))
                .and_then(|u| u.as_str())
            {
                let height = f
                    .pointer("/resolution/id")
                    .and_then(|h| h.as_u64())
                    .map(|h| h as u32);
                let label = f
                    .pointer("/resolution/label")
                    .and_then(|l| l.as_str())
                    .unwrap_or("video");
                let mut fmt = direct_format(&format!("pt_{i}"), label, u.to_string(), "mp4");
                fmt.height = height;
                formats.push(fmt);
            }
        }
    }
    // streaming playlists (HLS)
    if let Some(pls) = v.get("streamingPlaylists").and_then(|p| p.as_array()) {
        for (i, p) in pls.iter().enumerate() {
            if let Some(u) = p.get("playlistUrl").and_then(|u| u.as_str()) {
                formats.push(direct_format(
                    &format!("pt_hls_{i}"),
                    "HLS",
                    u.to_string(),
                    "m3u8",
                ));
            }
        }
    }

    if formats.is_empty() {
        bail!("peertube: no files or playlists");
    }

    let mut result = empty_result(title, "peertube");
    result.thumbnail = thumbnail;
    result.description = description;
    result.duration_secs = duration_secs;
    result.uploader = uploader;
    result.formats = formats;
    Ok(result)
}

fn video_id(u: &url::Url) -> Option<String> {
    let segs: Vec<_> = u.path_segments()?.collect();
    // /w/UUID or /videos/watch/UUID
    if let Some(i) = segs.iter().position(|s| *s == "w" || *s == "watch") {
        return segs.get(i + 1).map(|s| (*s).to_string());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn matches_pt() {
        assert!(matches("https://peertube.example/w/aaabbbccc"));
        assert!(!matches("https://youtube.com/watch?v=1"));
    }
}
