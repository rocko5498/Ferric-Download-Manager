//! `WebDAV` downloads (`webdav://` / `webdavs://`).
//!
//! Rewrites to http(s), uses PROPFIND for size when HEAD lacks Content-Length,
//! then falls through to the normal HTTP range engine.

use anyhow::{anyhow, Context, Result};
use reqwest::header::{HeaderValue, CONTENT_LENGTH};
use reqwest::Client;
use std::collections::HashMap;

use crate::chunk::{self, ChunkPlan, ResponseMetadata};

/// Map webdav schemes to HTTP.
#[must_use]
pub fn to_http_url(url: &str) -> Option<String> {
    if let Some(rest) = url.strip_prefix("webdavs://") {
        Some(format!("https://{rest}"))
    } else {
        url.strip_prefix("webdav://")
            .map(|rest| format!("http://{rest}"))
    }
}

#[must_use]
pub fn is_webdav_url(url: &str) -> bool {
    url.starts_with("webdav://") || url.starts_with("webdavs://")
}

/// Effective download URL (webdav rewritten, else original).
#[must_use]
pub fn effective_http_url(url: &str) -> String {
    to_http_url(url).unwrap_or_else(|| url.to_string())
}

/// Plan a `WebDAV` download: try HTTP HEAD plan first; on missing size, PROPFIND.
pub async fn plan_webdav(
    url: &str,
    client: &Client,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
    max_connections: usize,
) -> Result<ChunkPlan> {
    let http_url = to_http_url(url).ok_or_else(|| anyhow!("not a webdav URL"))?;

    match chunk::plan(&http_url, client, cookies, extra_headers, max_connections).await {
        Ok(plan @ ChunkPlan::Segmented { .. }) => return Ok(plan),
        Ok(ChunkPlan::SingleStream {
            etag,
            last_modified,
            metadata,
        }) => {
            if let Some(size) = propfind_size(client, &http_url, cookies, extra_headers).await? {
                return Ok(segmented_from_size(
                    size,
                    max_connections,
                    etag,
                    last_modified,
                    metadata,
                ));
            }
            return Ok(ChunkPlan::SingleStream {
                etag,
                last_modified,
                metadata,
            });
        }
        Err(_) => {
            // HEAD may be forbidden on WebDAV; PROPFIND is the real probe.
            if let Some(size) = propfind_size(client, &http_url, cookies, extra_headers).await? {
                return Ok(segmented_from_size(
                    size,
                    max_connections,
                    None,
                    None,
                    ResponseMetadata {
                        final_url: Some(http_url),
                        ..ResponseMetadata::default()
                    },
                ));
            }
        }
    }

    // Last resort: single-stream GET without known size.
    Ok(ChunkPlan::SingleStream {
        etag: None,
        last_modified: None,
        metadata: ResponseMetadata {
            final_url: Some(http_url),
            ..ResponseMetadata::default()
        },
    })
}

fn segmented_from_size(
    file_size: u64,
    max_connections: usize,
    etag: Option<String>,
    last_modified: Option<String>,
    metadata: ResponseMetadata,
) -> ChunkPlan {
    if file_size == 0 {
        return ChunkPlan::SingleStream {
            etag,
            last_modified,
            metadata,
        };
    }
    let concurrency = chunk::planned_chunk_count(file_size, max_connections);
    let chunks = chunk::plan_pieces(file_size, concurrency);
    ChunkPlan::Segmented {
        file_size,
        chunks,
        concurrency,
        etag,
        last_modified,
        metadata,
    }
}

/// PROPFIND Depth: 0 → parse getcontentlength.
async fn propfind_size(
    client: &Client,
    http_url: &str,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
) -> Result<Option<u64>> {
    let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:getcontentlength/></d:prop>
</d:propfind>"#;

    let mut req = client
        .request(
            reqwest::Method::from_bytes(b"PROPFIND").unwrap_or(reqwest::Method::GET),
            http_url,
        )
        .header("Depth", "0")
        .header("Content-Type", "application/xml; charset=utf-8")
        .body(body);

    if let Some(c) = cookies {
        req = req.header(reqwest::header::COOKIE, c);
    }
    if let Some(extra) = extra_headers {
        for (k, v) in extra {
            if let (Ok(name), Ok(val)) = (
                reqwest::header::HeaderName::from_bytes(k.as_bytes()),
                HeaderValue::from_str(v),
            ) {
                req = req.header(name, val);
            }
        }
    }

    let response = req.send().await.context("webdav PROPFIND")?;
    // 207 Multi-Status is success for PROPFIND
    let status = response.status().as_u16();
    if status != 207 && !response.status().is_success() {
        // Some servers answer HEAD/GET only — treat as unknown size.
        return Ok(None);
    }

    // Prefer Content-Length on the PROPFIND response itself if present.
    if let Some(len) = response
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse().ok())
    {
        if len > 0 {
            return Ok(Some(len));
        }
    }

    let text = response.text().await.unwrap_or_default();
    Ok(parse_getcontentlength(&text))
}

fn parse_getcontentlength(xml: &str) -> Option<u64> {
    // Case-insensitive hunt for getcontentlength>...
    let lower = xml.to_ascii_lowercase();
    for tag in [
        "getcontentlength>",
        "d:getcontentlength>",
        "lp1:getcontentlength>",
    ] {
        if let Some(pos) = lower.find(tag) {
            let after = &xml[pos + tag.len()..];
            let end = after.find('<')?;
            let num = after[..end].trim();
            if let Ok(n) = num.parse::<u64>() {
                return Some(n);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_schemes() {
        assert_eq!(
            to_http_url("webdavs://cloud.example/remote.php/dav/files/u/f.bin").as_deref(),
            Some("https://cloud.example/remote.php/dav/files/u/f.bin")
        );
        assert_eq!(
            to_http_url("webdav://nas.local/share/a.zip").as_deref(),
            Some("http://nas.local/share/a.zip")
        );
        assert!(to_http_url("https://example.com/x").is_none());
    }

    #[test]
    fn parses_propfind_length() {
        let xml = r#"<?xml version="1.0"?>
        <d:multistatus xmlns:d="DAV:">
          <d:response>
            <d:propstat>
              <d:prop>
                <d:getcontentlength>12345</d:getcontentlength>
              </d:prop>
            </d:propstat>
          </d:response>
        </d:multistatus>"#;
        assert_eq!(parse_getcontentlength(xml), Some(12345));
    }
}
