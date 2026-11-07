//! Metalink v3 (XML) and v4 (XML / JSON) parsers.
//! Feeds multi-source downloads: primary URL + mirrors + optional checksum.

use serde::Deserialize;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetalinkFile {
    pub filename: Option<String>,
    pub size: Option<u64>,
    /// Ordered sources (first = preferred / primary).
    pub urls: Vec<String>,
    /// `sha256:<hex>`, `sha1:<hex>`, or `md5:<hex>` when present.
    pub checksum: Option<String>,
}

#[derive(Debug, Error)]
pub enum MetalinkError {
    #[error("empty metalink document")]
    Empty,
    #[error("metalink has no usable URLs")]
    NoUrls,
    #[error("invalid metalink JSON: {0}")]
    Json(String),
    #[error("invalid metalink XML: {0}")]
    Xml(String),
}

/// Parse metalink bytes (JSON meta4 or XML metalink 3/4).
///
/// # Errors
///
/// Returns [`MetalinkError`] when the document is empty, not valid metalink
/// JSON/XML, or yields no download URLs.
pub fn parse(bytes: &[u8]) -> Result<Vec<MetalinkFile>, MetalinkError> {
    let trimmed = trim_bom(bytes);
    if trimmed.is_empty() {
        return Err(MetalinkError::Empty);
    }
    let files = if looks_like_json(trimmed) {
        parse_json(trimmed)?
    } else {
        parse_xml(trimmed)?
    };
    let files: Vec<MetalinkFile> = files
        .into_iter()
        .map(normalize_file)
        .filter(|f| !f.urls.is_empty())
        .collect();
    if files.is_empty() {
        return Err(MetalinkError::NoUrls);
    }
    Ok(files)
}

/// True when a download URL points at a metalink document.
#[must_use]
pub fn is_metalink_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    std::path::Path::new(path).extension().is_some_and(|ext| {
        ext.eq_ignore_ascii_case("metalink") || ext.eq_ignore_ascii_case("meta4")
    })
}

fn normalize_file(mut file: MetalinkFile) -> MetalinkFile {
    let mut seen = std::collections::HashSet::new();
    file.urls.retain(|u| {
        let t = u.trim();
        !t.is_empty() && seen.insert(t.to_string())
    });
    file
}

fn looks_like_json(bytes: &[u8]) -> bool {
    bytes.iter().find(|b| !b.is_ascii_whitespace()).copied() == Some(b'{')
}

fn trim_bom(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    }
}

// ── JSON Metalink 4 ──────────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct JsonMetalink {
    #[serde(default)]
    files: Vec<JsonFile>,
}

#[derive(Debug, Deserialize)]
struct JsonFile {
    name: Option<String>,
    size: Option<u64>,
    #[serde(default)]
    urls: Vec<JsonUrl>,
    #[serde(default)]
    hashes: Vec<JsonHash>,
}

#[derive(Debug, Deserialize)]
struct JsonUrl {
    url: String,
}

#[derive(Debug, Deserialize)]
struct JsonHash {
    #[serde(rename = "type")]
    hash_type: String,
    hash: String,
}

fn parse_json(bytes: &[u8]) -> Result<Vec<MetalinkFile>, MetalinkError> {
    let doc: JsonMetalink =
        serde_json::from_slice(bytes).map_err(|e| MetalinkError::Json(e.to_string()))?;
    Ok(doc
        .files
        .into_iter()
        .map(|f| MetalinkFile {
            filename: f.name,
            size: f.size,
            urls: f.urls.into_iter().map(|u| u.url).collect(),
            checksum: pick_checksum(
                f.hashes
                    .iter()
                    .map(|h| (h.hash_type.as_str(), h.hash.as_str())),
            ),
        })
        .collect())
}

// ── XML Metalink 3 / 4 ───────────────────────────────────────────────────────

fn parse_xml(bytes: &[u8]) -> Result<Vec<MetalinkFile>, MetalinkError> {
    use quick_xml::events::Event;
    use quick_xml::reader::Reader;

    let mut reader = Reader::from_reader(bytes);
    reader.config_mut().trim_text(true);

    let mut files = Vec::new();
    let mut current: Option<MetalinkFile> = None;
    let mut buf = Vec::new();
    let mut text_target: Option<&'static str> = None;
    let mut hash_type: Option<String> = None;
    let mut hashes: Vec<(String, String)> = Vec::new();

    loop {
        match reader
            .read_event_into(&mut buf)
            .map_err(|e| MetalinkError::Xml(e.to_string()))?
        {
            Event::Start(e) => {
                let name = local_name(e.name().as_ref());
                match name.as_str() {
                    "file" => {
                        let filename = attr_value(&e, "name");
                        current = Some(MetalinkFile {
                            filename,
                            size: None,
                            urls: Vec::new(),
                            checksum: None,
                        });
                        hashes.clear();
                        text_target = None;
                    }
                    "url" => text_target = Some("url"),
                    "hash" => {
                        hash_type = attr_value(&e, "type");
                        text_target = Some("hash");
                    }
                    "size" => text_target = Some("size"),
                    "name" => text_target = Some("name"),
                    _ => text_target = None,
                }
            }
            Event::Text(t) => {
                let text = t
                    .decode()
                    .map_err(|e| MetalinkError::Xml(e.to_string()))?
                    .into_owned();
                let text = text.trim();
                if text.is_empty() {
                    // skip
                } else if let Some(target) = text_target {
                    match target {
                        "url" => {
                            if let Some(file) = current.as_mut() {
                                file.urls.push(text.to_string());
                            }
                        }
                        "hash" => {
                            if let Some(ht) = hash_type.take() {
                                hashes.push((ht, text.to_string()));
                            }
                        }
                        "size" => {
                            if let Some(file) = current.as_mut() {
                                file.size = text.parse().ok();
                            }
                        }
                        "name" => {
                            if let Some(file) = current.as_mut() {
                                if file.filename.is_none() {
                                    file.filename = Some(text.to_string());
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
            Event::End(e) => {
                let name = local_name(e.name().as_ref());
                if name == "file" {
                    if let Some(mut file) = current.take() {
                        file.checksum =
                            pick_checksum(hashes.iter().map(|(t, h)| (t.as_str(), h.as_str())));
                        files.push(file);
                    }
                    hashes.clear();
                }
                text_target = None;
                if name == "hash" {
                    hash_type = None;
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(files)
}

fn attr_value(e: &quick_xml::events::BytesStart<'_>, key: &str) -> Option<String> {
    e.attributes()
        .filter_map(Result::ok)
        .find(|a| local_name(a.key.as_ref()) == key)
        .and_then(|a| String::from_utf8(a.value.into_owned()).ok())
}

fn local_name(qname: &[u8]) -> String {
    let s = String::from_utf8_lossy(qname);
    s.rsplit(['}', ':']).next().unwrap_or(&s).to_string()
}

fn pick_checksum<'a>(hashes: impl Iterator<Item = (&'a str, &'a str)>) -> Option<String> {
    let mut sha256 = None;
    let mut sha1 = None;
    let mut md5 = None;
    for (ty, hex) in hashes {
        let t = ty.to_ascii_lowercase().replace('_', "-");
        let hex = hex.trim().to_ascii_lowercase();
        if hex.is_empty() {
            continue;
        }
        match t.as_str() {
            "sha-256" | "sha256" => sha256 = Some(format!("sha256:{hex}")),
            "sha-1" | "sha1" => sha1 = Some(format!("sha1:{hex}")),
            "md5" => md5 = Some(format!("md5:{hex}")),
            _ => {}
        }
    }
    sha256.or(sha1).or(md5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_metalink4_json() {
        let raw = br#"{
          "files": [{
            "name": "pkg.zip",
            "size": 42,
            "hashes": [{"type": "sha-256", "hash": "aabb"}],
            "urls": [
              {"url": "https://a.example/pkg.zip"},
              {"url": "https://b.example/pkg.zip"}
            ]
          }]
        }"#;
        let files = parse(raw).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].filename.as_deref(), Some("pkg.zip"));
        assert_eq!(files[0].size, Some(42));
        assert_eq!(files[0].checksum.as_deref(), Some("sha256:aabb"));
        assert_eq!(files[0].urls.len(), 2);
    }

    #[test]
    fn parses_metalink4_xml() {
        let raw = br#"<?xml version="1.0"?>
        <metalink xmlns="urn:ietf:params:xml:ns:metalink">
          <file name="demo.bin">
            <size>100</size>
            <hash type="sha-256">deadbeef</hash>
            <url>https://cdn1.example/demo.bin</url>
            <url>https://cdn2.example/demo.bin</url>
          </file>
        </metalink>"#;
        let files = parse(raw).unwrap();
        assert_eq!(files[0].filename.as_deref(), Some("demo.bin"));
        assert_eq!(files[0].size, Some(100));
        assert_eq!(files[0].checksum.as_deref(), Some("sha256:deadbeef"));
        assert_eq!(
            files[0].urls,
            vec![
                "https://cdn1.example/demo.bin".to_string(),
                "https://cdn2.example/demo.bin".to_string()
            ]
        );
    }

    #[test]
    fn parses_metalink3_xml() {
        let raw = br#"<?xml version="1.0"?>
        <metalink version="3.0" xmlns="http://www.metalinker.org/">
          <files>
            <file name="old.bin">
              <size>9</size>
              <verification>
                <hash type="sha1">abc123</hash>
              </verification>
              <resources>
                <url type="http">http://mirror.example/old.bin</url>
              </resources>
            </file>
          </files>
        </metalink>"#;
        let files = parse(raw).unwrap();
        assert_eq!(files[0].filename.as_deref(), Some("old.bin"));
        assert_eq!(files[0].checksum.as_deref(), Some("sha1:abc123"));
        assert_eq!(
            files[0].urls,
            vec!["http://mirror.example/old.bin".to_string()]
        );
    }

    #[test]
    fn detects_metalink_urls() {
        assert!(is_metalink_url("https://ex.com/f.meta4"));
        assert!(is_metalink_url("https://ex.com/f.metalink?x=1"));
        assert!(!is_metalink_url("https://ex.com/f.zip"));
    }
}
