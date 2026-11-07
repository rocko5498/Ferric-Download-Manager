//! Import torrents from a qBittorrent installation's `BT_backup` directory.
//!
//! qBittorrent stores one <info-hash-hex>.torrent + <info-hash-hex>.fastresume
//! pair per torrent. The .fastresume is a bencode dict of libtorrent `resume_data`
//! plus qBt-* vendor extensions (savePath, name, tags, ...). We deliberately do
//! NOT trust the foreign piece bitfield in the .fastresume -- the caller re-stages
//! via `TorrentManager::stage()/start()` and lets our own engine hash-verify
//! whatever files already exist on disk (the same code path F24's resume fix
//! uses), rather than importing a possibly-stale or tampered "these pieces are
//! done" claim from a third-party file.

use anyhow::{Context, Result};
use serde_bencode::value::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub struct QbtImportCandidate {
    pub info_hash: String,
    pub name: Option<String>,
    pub save_path: Option<String>,
    pub tags: Vec<String>,
    pub torrent_file_path: PathBuf,
    pub stopped: bool,
}

fn get_bytes<'a>(dict: &'a HashMap<Vec<u8>, Value>, key: &str) -> Option<&'a [u8]> {
    match dict.get(key.as_bytes()) {
        Some(Value::Bytes(b)) => Some(b.as_slice()),
        _ => None,
    }
}

fn get_str(dict: &HashMap<Vec<u8>, Value>, key: &str) -> Option<String> {
    get_bytes(dict, key).map(|b| String::from_utf8_lossy(b).into_owned())
}

fn get_int(dict: &HashMap<Vec<u8>, Value>, key: &str) -> Option<i64> {
    match dict.get(key.as_bytes()) {
        Some(Value::Int(i)) => Some(*i),
        _ => None,
    }
}

fn get_list_str(dict: &HashMap<Vec<u8>, Value>, key: &str) -> Vec<String> {
    match dict.get(key.as_bytes()) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Scan a BT_backup-style directory for importable torrents. Individually
/// unparsable files are logged and skipped, not fatal to the whole scan.
pub fn scan_directory(dir: &Path) -> Result<Vec<QbtImportCandidate>> {
    let mut out = Vec::new();
    let entries = std::fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("fastresume") {
            continue;
        }
        match parse_fastresume(&path) {
            Ok(Some(candidate)) => out.push(candidate),
            Ok(None) => {}
            Err(e) => tracing::warn!(file = %path.display(), "failed to parse .fastresume: {e}"),
        }
    }
    Ok(out)
}

fn parse_fastresume(path: &Path) -> Result<Option<QbtImportCandidate>> {
    let bytes = std::fs::read(path)?;
    let value: Value = serde_bencode::from_bytes(&bytes)
        .with_context(|| format!("bencode decode {}", path.display()))?;
    let Value::Dict(dict) = value else {
        anyhow::bail!("not a bencode dict: {}", path.display());
    };

    // qBittorrent names .fastresume files by the torrent's hex info-hash --
    // more robust to read from the filename than to depend on a specific
    // raw "info-hash" bencode key being present across libtorrent versions.
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let info_hash = stem.to_lowercase();
    if info_hash.len() != 40 || !info_hash.chars().all(|c| c.is_ascii_hexdigit()) {
        return Ok(None);
    }

    let torrent_file_path = path.with_extension("torrent");
    if !torrent_file_path.exists() {
        tracing::warn!(hash = %info_hash, "fastresume has no paired .torrent file, skipping");
        return Ok(None);
    }

    let name = get_str(&dict, "qBt-name").or_else(|| get_str(&dict, "name"));
    let save_path = get_str(&dict, "qBt-savePath").or_else(|| get_str(&dict, "save_path"));
    let tags = get_list_str(&dict, "qBt-tags");
    let stopped = get_int(&dict, "qBt-stopped").is_some_and(|v| v != 0);

    Ok(Some(QbtImportCandidate {
        info_hash,
        name,
        save_path,
        tags,
        torrent_file_path,
        stopped,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dict(pairs: Vec<(&str, Value)>) -> Value {
        let map: HashMap<Vec<u8>, Value> = pairs
            .into_iter()
            .map(|(k, v)| (k.as_bytes().to_vec(), v))
            .collect();
        Value::Dict(map)
    }

    fn bstr(s: &str) -> Value {
        Value::Bytes(s.as_bytes().to_vec())
    }

    #[test]
    fn skips_non_hash_named_fastresume_files() {
        let dir = tempfile::tempdir().unwrap();
        let body = serde_bencode::to_bytes(&dict(vec![("qBt-name", bstr("hello"))])).unwrap();
        std::fs::write(dir.path().join("not-a-hash.fastresume"), body).unwrap();
        let found = scan_directory(dir.path()).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn skips_fastresume_without_paired_torrent_file() {
        let dir = tempfile::tempdir().unwrap();
        let hash = "a".repeat(40);
        let body = serde_bencode::to_bytes(&dict(vec![("qBt-name", bstr("hello"))])).unwrap();
        std::fs::write(dir.path().join(format!("{hash}.fastresume")), body).unwrap();
        let found = scan_directory(dir.path()).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn parses_a_minimal_valid_pair() {
        let dir = tempfile::tempdir().unwrap();
        let hash = "b".repeat(40);
        let torrent_body =
            serde_bencode::to_bytes(&dict(vec![("info", dict(vec![("name", bstr("xxx"))]))]))
                .unwrap();
        std::fs::write(dir.path().join(format!("{hash}.torrent")), torrent_body).unwrap();

        let resume_body = serde_bencode::to_bytes(&dict(vec![
            ("qBt-savePath", bstr("E:\\Films\\")),
            ("qBt-name", bstr("Movie")),
            ("qBt-tags", Value::List(vec![bstr("foo"), bstr("bar")])),
        ]))
        .unwrap();
        std::fs::write(dir.path().join(format!("{hash}.fastresume")), resume_body).unwrap();

        let found = scan_directory(dir.path()).unwrap();
        assert_eq!(found.len(), 1);
        let c = &found[0];
        assert_eq!(c.info_hash, hash);
        assert_eq!(c.name.as_deref(), Some("Movie"));
        assert_eq!(c.save_path.as_deref(), Some("E:\\Films\\"));
        assert_eq!(c.tags, vec!["foo".to_string(), "bar".to_string()]);
        assert!(!c.stopped);
    }
}
