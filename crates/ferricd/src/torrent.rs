use anyhow::{anyhow, bail, Result};
use ferric_common::{TorrentInputKind, TorrentProbeFile, TorrentProbeResponse};
use sha1::{Digest, Sha1};
use std::collections::HashSet;
use std::path::Path;

const MAX_TRACKERS: usize = 64;
const MAX_TORRENT_METADATA_BYTES: u64 = 4 * 1024 * 1024;

pub fn probe_input(input: &str) -> Result<TorrentProbeResponse> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        bail!("torrent input is empty");
    }

    if trimmed.to_ascii_lowercase().starts_with("magnet:?") {
        return probe_magnet(trimmed);
    }

    if trimmed.to_ascii_lowercase().ends_with(".torrent") {
        return probe_torrent_file(trimmed);
    }

    Ok(TorrentProbeResponse {
        input: trimmed.to_string(),
        kind: TorrentInputKind::Unsupported,
        supported: false,
        name: None,
        info_hash: None,
        trackers: Vec::new(),
        files: Vec::new(),
        file_count: None,
        total_size: None,
        is_private: false,
        warnings: Vec::new(),
        explanation: "Paste a magnet URI or select a .torrent file. HTTP URLs still belong in the normal download flow.".to_string(),
    })
}

fn probe_torrent_file(input: &str) -> Result<TorrentProbeResponse> {
    let path = Path::new(input);
    if !path.exists() {
        return Ok(TorrentProbeResponse {
            input: input.to_string(),
            kind: TorrentInputKind::TorrentFile,
            supported: false,
            name: file_stem(input),
            info_hash: None,
            trackers: Vec::new(),
            files: Vec::new(),
            file_count: None,
            total_size: None,
            is_private: false,
            warnings: base_warnings(),
            explanation: ".torrent path recognized, but the file was not found. Choose an existing metadata file before Ferric can inspect file paths safely.".to_string(),
        });
    }

    let metadata = std::fs::metadata(path)?;
    if metadata.len() > MAX_TORRENT_METADATA_BYTES {
        bail!("torrent metadata is too large to inspect safely");
    }

    let bytes = std::fs::read(path)?;
    let parsed = parse_torrent_metadata(&bytes)?;
    let is_private = parsed.is_private;
    let mut warnings = base_warnings();
    if is_private {
        warnings.push("Private torrent detected (BEP-27). DHT, PEX, and LSD will be permanently locked off — sharing this torrent's swarm membership would violate the tracker's rules.".to_string());
    }
    Ok(TorrentProbeResponse {
        input: input.to_string(),
        kind: TorrentInputKind::TorrentFile,
        supported: true,
        name: parsed.name.or_else(|| file_stem(input)),
        info_hash: Some(parsed.info_hash),
        trackers: parsed.trackers,
        files: parsed.files,
        file_count: Some(parsed.file_count),
        total_size: Some(parsed.total_size),
        is_private,
        warnings,
        explanation: ".torrent metadata inspected successfully. File paths are safe to stage, but peer networking remains disabled until the engine and privacy controls are wired.".to_string(),
    })
}

fn probe_magnet(input: &str) -> Result<TorrentProbeResponse> {
    let query = input
        .strip_prefix("magnet:?")
        .or_else(|| input.strip_prefix("MAGNET:?"))
        .ok_or_else(|| anyhow!("invalid magnet URI"))?;

    let mut name = None;
    let mut info_hash = None;
    let mut trackers = Vec::new();
    let mut seen_trackers = HashSet::new();
    let mut warnings = base_warnings();

    for (key, value) in form_pairs(query) {
        match key.as_str() {
            "dn" if name.is_none() && !value.trim().is_empty() => {
                name = Some(value.trim().to_string());
            }
            "xt" if info_hash.is_none() => {
                if let Some(hash) = value.strip_prefix("urn:btih:") {
                    info_hash = Some(validate_btih(hash)?);
                } else if let Some(hash) = value.strip_prefix("urn:btmh:1220") {
                    info_hash = Some(validate_btih(hash)?);
                }
            }
            "tr" if trackers.len() < MAX_TRACKERS
                && is_allowed_tracker(&value)
                && seen_trackers.insert(value.clone()) =>
            {
                trackers.push(value);
            }
            _ => {}
        }
    }

    if info_hash.is_none() {
        bail!("magnet URI is missing xt=urn:btih:<info-hash>");
    }

    if trackers.is_empty() {
        warnings.push(
            "No trackers were found; discovery would depend on DHT if the user enables it."
                .to_string(),
        );
    }

    Ok(TorrentProbeResponse {
        input: input.to_string(),
        kind: TorrentInputKind::Magnet,
        supported: true,
        name,
        info_hash,
        trackers,
        files: Vec::new(),
        file_count: None,
        total_size: None,
        is_private: false,
        warnings,
        explanation: "Magnet recognized. Ferric can now show torrent intent safely, but the peer engine is intentionally not enabled until the sidecar/library ADR and security tests are complete.".to_string(),
    })
}

#[derive(Debug)]
struct TorrentMetadata {
    name: Option<String>,
    info_hash: String,
    trackers: Vec<String>,
    files: Vec<TorrentProbeFile>,
    file_count: u64,
    total_size: u64,
    is_private: bool,
}

fn parse_torrent_metadata(bytes: &[u8]) -> Result<TorrentMetadata> {
    let info_slice = find_top_level_value_slice(bytes, b"info")?;
    let mut parser = BencodeParser::new(bytes);
    let root = parser.parse_value()?;
    parser.ensure_finished()?;

    let root = root.as_dict()?;
    let info = root.get_bytes(b"info")?.as_dict()?;

    let info_hash = if info.get_bytes_opt(b"file tree").is_some() {
        sha256_hex(info_slice)
    } else {
        sha1_hex(info_slice)
    };

    let name = info
        .get_bytes_opt(b"name")
        .and_then(|value| value.as_utf8().ok())
        .map(ToString::to_string);
    let is_private = info
        .get_bytes_opt(b"private")
        .and_then(|v| {
            if let BencodeValue::Int(n) = v {
                Some(*n != 0)
            } else {
                None
            }
        })
        .unwrap_or(false);

    let mut trackers = Vec::new();
    if let Some(announce) = root
        .get_bytes_opt(b"announce")
        .and_then(|value| value.as_utf8().ok())
    {
        if is_allowed_tracker(announce) {
            trackers.push(announce.to_string());
        }
    }
    if let Some(list) = root
        .get_bytes_opt(b"announce-list")
        .and_then(|v| v.as_list().ok())
    {
        for tier in list {
            if let Ok(urls) = tier.as_list() {
                for url in urls {
                    if let Ok(url) = url.as_utf8() {
                        if trackers.len() < MAX_TRACKERS
                            && is_allowed_tracker(url)
                            && !trackers.iter().any(|existing| existing == url)
                        {
                            trackers.push(url.to_string());
                        }
                    }
                }
            }
        }
    }

    if let Some(file_tree) = info.get_bytes_opt(b"file tree") {
        let mut parsed_files = Vec::new();
        let mut current_path = Vec::new();
        walk_file_tree(file_tree, &mut current_path, &mut parsed_files)?;
        let total_size = parsed_files.iter().map(|f| f.size).sum();
        let file_count = parsed_files.len() as u64;
        if parsed_files.is_empty() {
            bail!("torrent metadata has no files in file tree");
        }
        Ok(TorrentMetadata {
            name,
            info_hash,
            trackers,
            files: parsed_files,
            file_count,
            total_size,
            is_private,
        })
    } else if let Some(files) = info
        .get_bytes_opt(b"files")
        .and_then(|value| value.as_list().ok())
    {
        let mut total_size = 0u64;
        let mut parsed_files = Vec::with_capacity(files.len());
        for file in files {
            let file = file.as_dict()?;
            let length = file.get_bytes(b"length")?.as_nonnegative_u64()?;
            let path = file.get_bytes(b"path")?.as_list()?;
            validate_torrent_path(path)?;
            parsed_files.push(TorrentProbeFile {
                path: torrent_path_string(path)?,
                size: length,
            });
            total_size = total_size
                .checked_add(length)
                .ok_or_else(|| anyhow!("torrent total size overflow"))?;
        }
        if files.is_empty() {
            bail!("torrent metadata has no files");
        }
        Ok(TorrentMetadata {
            name,
            info_hash,
            trackers,
            files: parsed_files,
            file_count: files.len() as u64,
            total_size,
            is_private,
        })
    } else {
        let length = info.get_bytes(b"length")?.as_nonnegative_u64()?;
        let file_name = info
            .get_bytes(b"name")?
            .as_utf8()
            .map_err(|_| anyhow!("torrent single-file name is not valid utf-8"))?;
        validate_torrent_path_component(file_name)?;
        Ok(TorrentMetadata {
            name,
            info_hash,
            trackers,
            files: vec![TorrentProbeFile {
                path: file_name.to_string(),
                size: length,
            }],
            file_count: 1,
            total_size: length,
            is_private,
        })
    }
}

fn find_top_level_value_slice<'a>(bytes: &'a [u8], key: &[u8]) -> Result<&'a [u8]> {
    let mut parser = BencodeParser::new(bytes);
    parser.expect(b'd')?;
    while parser.peek().is_some_and(|byte| byte != b'e') {
        let candidate = match parser.parse_bytes()? {
            BencodeValue::Bytes(bytes) => bytes,
            _ => unreachable!("parse_bytes always returns bytes"),
        };
        let value_start = parser.offset;
        parser.skip_value()?;
        let value_end = parser.offset;
        if candidate == key {
            return Ok(&bytes[value_start..value_end]);
        }
    }
    parser.expect(b'e')?;
    parser.ensure_finished()?;
    bail!("torrent metadata missing top-level info dictionary")
}

fn sha1_hex(bytes: &[u8]) -> String {
    let digest = Sha1::digest(bytes);
    let mut out = String::with_capacity(40);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest as _, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn walk_file_tree(
    tree: &BencodeValue<'_>,
    current_path: &mut Vec<String>,
    files: &mut Vec<TorrentProbeFile>,
) -> Result<()> {
    if current_path.len() > 32 {
        bail!("torrent file tree directory structure is too deep (max 32)");
    }
    if files.len() > 50000 {
        bail!("torrent file tree contains too many files (max 50000)");
    }
    match tree {
        BencodeValue::Dict(dict) => {
            if let Some(length_val) = dict.get_bytes_opt(b"length") {
                let length = length_val.as_nonnegative_u64()?;
                let path = current_path.join("/");
                files.push(TorrentProbeFile { path, size: length });
            } else {
                for (name_bytes, child) in dict {
                    let name = std::str::from_utf8(name_bytes)
                        .map_err(|_| anyhow!("invalid UTF-8 in file tree name"))?
                        .to_string();
                    validate_torrent_path_component(&name)?;
                    current_path.push(name);
                    walk_file_tree(child, current_path, files)?;
                    current_path.pop();
                }
            }
        }
        _ => bail!("invalid file tree structure"),
    }
    Ok(())
}

fn validate_torrent_path(path: &[BencodeValue<'_>]) -> Result<()> {
    if path.is_empty() {
        bail!("torrent file path is empty");
    }
    for component in path {
        validate_torrent_path_component(component.as_utf8()?)?;
    }
    Ok(())
}

pub(crate) fn validate_torrent_relative_path(path: &str) -> Result<()> {
    if path.trim().is_empty() {
        bail!("torrent file path is empty");
    }
    for component in path.split(['/', '\\']) {
        validate_torrent_path_component(component)?;
    }
    Ok(())
}

fn torrent_path_string(path: &[BencodeValue<'_>]) -> Result<String> {
    path.iter()
        .map(|component| component.as_utf8().map(ToString::to_string))
        .collect::<Result<Vec<_>>>()
        .map(|parts| parts.join("/"))
}

fn validate_torrent_path_component(component: &str) -> Result<()> {
    let trimmed = component.trim();
    if trimmed.is_empty() || trimmed == "." || trimmed == ".." {
        bail!("torrent file path contains an unsafe component");
    }
    if trimmed.contains('/') || trimmed.contains('\\') || trimmed.contains(':') {
        bail!("torrent file path contains separators or an absolute path marker");
    }
    let stem = trimmed
        .split('.')
        .next()
        .unwrap_or(trimmed)
        .to_ascii_uppercase();
    let reserved = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
        "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    if reserved.contains(&stem.as_str()) {
        bail!("torrent file path uses a Windows reserved device name");
    }
    Ok(())
}

#[derive(Debug)]
enum BencodeValue<'a> {
    Int(i64),
    Bytes(&'a [u8]),
    List(Vec<BencodeValue<'a>>),
    Dict(Vec<(&'a [u8], BencodeValue<'a>)>),
}

impl<'a> BencodeValue<'a> {
    fn as_dict(&'a self) -> Result<&'a [(&'a [u8], BencodeValue<'a>)]> {
        match self {
            BencodeValue::Dict(values) => Ok(values),
            _ => bail!("expected bencode dictionary"),
        }
    }

    fn as_list(&'a self) -> Result<&'a [BencodeValue<'a>]> {
        match self {
            BencodeValue::List(values) => Ok(values),
            _ => bail!("expected bencode list"),
        }
    }

    fn as_utf8(&self) -> Result<&str> {
        match self {
            BencodeValue::Bytes(bytes) => std::str::from_utf8(bytes)
                .map_err(|_| anyhow!("torrent metadata contains non-utf8 path/name text")),
            _ => bail!("expected bencode byte string"),
        }
    }

    fn as_nonnegative_u64(&self) -> Result<u64> {
        match self {
            BencodeValue::Int(value) if *value >= 0 => Ok(*value as u64),
            BencodeValue::Int(_) => bail!("torrent file length cannot be negative"),
            _ => bail!("expected bencode integer"),
        }
    }
}

trait BencodeDictExt<'a> {
    fn get_bytes(&'a self, key: &[u8]) -> Result<&'a BencodeValue<'a>>;
    fn get_bytes_opt(&'a self, key: &[u8]) -> Option<&'a BencodeValue<'a>>;
}

impl<'a> BencodeDictExt<'a> for [(&'a [u8], BencodeValue<'a>)] {
    fn get_bytes(&'a self, key: &[u8]) -> Result<&'a BencodeValue<'a>> {
        self.get_bytes_opt(key)
            .ok_or_else(|| anyhow!("torrent metadata missing required key"))
    }

    fn get_bytes_opt(&'a self, key: &[u8]) -> Option<&'a BencodeValue<'a>> {
        self.iter()
            .find_map(|(candidate, value)| (*candidate == key).then_some(value))
    }
}

struct BencodeParser<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> BencodeParser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn parse_value(&mut self) -> Result<BencodeValue<'a>> {
        match self.peek() {
            Some(b'i') => self.parse_int(),
            Some(b'l') => self.parse_list(),
            Some(b'd') => self.parse_dict(),
            Some(b'0'..=b'9') => self.parse_bytes(),
            _ => bail!("invalid bencode value"),
        }
    }

    fn parse_int(&mut self) -> Result<BencodeValue<'a>> {
        self.expect(b'i')?;
        let start = self.offset;
        while self.peek().is_some_and(|byte| byte != b'e') {
            self.offset += 1;
        }
        self.expect(b'e')?;
        let text = std::str::from_utf8(&self.bytes[start..self.offset - 1])?;
        let value = text.parse::<i64>()?;
        Ok(BencodeValue::Int(value))
    }

    fn parse_bytes(&mut self) -> Result<BencodeValue<'a>> {
        let start = self.offset;
        while self.peek().is_some_and(|byte| byte.is_ascii_digit()) {
            self.offset += 1;
        }
        self.expect(b':')?;
        let len_text = std::str::from_utf8(&self.bytes[start..self.offset - 1])?;
        let len = len_text.parse::<usize>()?;
        let end = self
            .offset
            .checked_add(len)
            .ok_or_else(|| anyhow!("bencode byte string length overflow"))?;
        if end > self.bytes.len() {
            bail!("bencode byte string exceeds input");
        }
        let value = &self.bytes[self.offset..end];
        self.offset = end;
        Ok(BencodeValue::Bytes(value))
    }

    fn parse_list(&mut self) -> Result<BencodeValue<'a>> {
        self.expect(b'l')?;
        let mut values = Vec::new();
        while self.peek().is_some_and(|byte| byte != b'e') {
            values.push(self.parse_value()?);
        }
        self.expect(b'e')?;
        Ok(BencodeValue::List(values))
    }

    fn parse_dict(&mut self) -> Result<BencodeValue<'a>> {
        self.expect(b'd')?;
        let mut values = Vec::new();
        while self.peek().is_some_and(|byte| byte != b'e') {
            let key = match self.parse_bytes()? {
                BencodeValue::Bytes(bytes) => bytes,
                _ => unreachable!("parse_bytes always returns bytes"),
            };
            let value = self.parse_value()?;
            values.push((key, value));
        }
        self.expect(b'e')?;
        Ok(BencodeValue::Dict(values))
    }

    fn skip_value(&mut self) -> Result<()> {
        self.parse_value().map(|_| ())
    }

    fn ensure_finished(&self) -> Result<()> {
        if self.offset == self.bytes.len() {
            Ok(())
        } else {
            bail!("trailing bytes after bencode document")
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.offset).copied()
    }

    fn expect(&mut self, expected: u8) -> Result<()> {
        if self.peek() == Some(expected) {
            self.offset += 1;
            Ok(())
        } else {
            bail!("invalid bencode delimiter")
        }
    }
}

fn form_pairs(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter_map(|part| {
            let (key, value) = part.split_once('=').unwrap_or((part, ""));
            Some((percent_decode(key)?, percent_decode(value)?))
        })
        .collect()
}

fn percent_decode(value: &str) -> Option<String> {
    let mut out = Vec::with_capacity(value.len());
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            b'%' if index + 2 < bytes.len() => {
                let hi = hex_value(bytes[index + 1])?;
                let lo = hex_value(bytes[index + 2])?;
                out.push((hi << 4) | lo);
                index += 3;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

fn hex_value(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn validate_btih(hash: &str) -> Result<String> {
    let normalized = hash.trim();
    let is_hex_v1 = normalized.len() == 40 && normalized.bytes().all(|b| b.is_ascii_hexdigit());
    let is_hex_v2 = normalized.len() == 64 && normalized.bytes().all(|b| b.is_ascii_hexdigit());
    let is_base32_v1 = normalized.len() == 32
        && normalized
            .bytes()
            .all(|b| b.is_ascii_uppercase() || matches!(b, b'2'..=b'7') || b.is_ascii_lowercase());
    if is_hex_v1 || is_hex_v2 || is_base32_v1 {
        Ok(normalized.to_ascii_lowercase())
    } else {
        bail!("unsupported or malformed btih info-hash")
    }
}

fn is_allowed_tracker(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.starts_with("udp://") || lower.starts_with("http://") || lower.starts_with("https://")
}

fn file_stem(input: &str) -> Option<String> {
    std::path::Path::new(input)
        .file_stem()
        .and_then(|value| value.to_str())
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn base_warnings() -> Vec<String> {
    vec![
        "Torrent peers and trackers can expose your public IP unless proxy/VPN binding is configured.".to_string(),
        "Seeding must stay opt-in with ratio/time limits; no silent default seeding.".to_string(),
        "Every torrent file path must be validated before writing to disk.".to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torrent_parser_never_panics_on_malformed_input() {
        let seeds: [&[u8]; 2] = [
            b"d4:infod6:lengthi5e4:name1:a12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaaee",
            b"d4:infod9:file treed3:dird8:file.txtd6:lengthi9eeee4:name1:bee",
        ];
        for data in crate::fuzz_tests::inputs(&seeds, 4000) {
            let _ = parse_torrent_metadata(&data);
        }
    }

    #[test]
    fn parses_magnet_name_hash_and_trackers() {
        let result = probe_input("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Ubuntu+ISO&tr=https%3A%2F%2Ftracker.example%2Fannounce&tr=udp%3A%2F%2Ftracker.example%3A80")
            .expect("valid magnet");

        assert_eq!(result.kind, TorrentInputKind::Magnet);
        assert!(result.supported);
        assert_eq!(result.name.as_deref(), Some("Ubuntu ISO"));
        assert_eq!(
            result.info_hash.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        assert_eq!(result.trackers.len(), 2);
    }

    #[test]
    fn rejects_magnet_without_info_hash() {
        let err = probe_input("magnet:?dn=NoHash").expect_err("hash required");
        assert!(err.to_string().contains("missing xt=urn:btih"));
    }

    #[test]
    fn blocks_unsafe_tracker_schemes() {
        let result = probe_input("magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&tr=file%3A%2F%2FC%3A%2Fsecret&tr=https%3A%2F%2Ftracker.example%2Fannounce")
            .expect("valid magnet");

        assert_eq!(result.trackers, vec!["https://tracker.example/announce"]);
    }

    #[test]
    fn torrent_file_is_design_only_for_now() {
        #[cfg(windows)]
        let input = "C:\\Downloads\\sample.torrent";
        #[cfg(not(windows))]
        let input = "/downloads/sample.torrent";
        let result = probe_input(input).expect("torrent file probe");
        assert_eq!(result.kind, TorrentInputKind::TorrentFile);
        assert!(!result.supported);
        assert_eq!(result.name.as_deref(), Some("sample"));
    }

    #[test]
    fn parses_single_file_torrent_metadata() {
        let metadata =
            parse_torrent_metadata(b"d8:announce9:https://t4:infod6:lengthi123e4:name8:file.txtee")
                .expect("single-file metadata");

        assert_eq!(metadata.name.as_deref(), Some("file.txt"));
        assert_eq!(
            metadata.info_hash,
            "abd3ba80e40d26785bef279a79b6b57a52ec4a48"
        );
        assert_eq!(metadata.trackers, vec!["https://t"]);
        assert_eq!(metadata.file_count, 1);
        assert_eq!(metadata.total_size, 123);
        assert_eq!(metadata.files[0].path, "file.txt");
        assert_eq!(metadata.files[0].size, 123);
    }

    #[test]
    fn parses_multi_file_torrent_metadata() {
        let metadata = parse_torrent_metadata(
            b"d4:infod5:filesld6:lengthi10e4:pathl3:dir8:file.txteed6:lengthi20e4:pathl5:other8:data.bineee4:name6:bundleee",
        )
        .expect("multi-file metadata");

        assert_eq!(metadata.name.as_deref(), Some("bundle"));
        assert_eq!(metadata.file_count, 2);
        assert_eq!(metadata.total_size, 30);
        assert_eq!(metadata.files[0].path, "dir/file.txt");
        assert_eq!(metadata.files[0].size, 10);
        assert_eq!(metadata.files[1].path, "other/data.bin");
        assert_eq!(metadata.files[1].size, 20);
    }

    #[test]
    fn info_hash_uses_exact_raw_info_slice() {
        let first =
            parse_torrent_metadata(b"d8:announce9:https://t4:infod6:lengthi123e4:name8:file.txtee")
                .expect("first metadata");
        let second =
            parse_torrent_metadata(b"d8:announce9:https://t4:infod6:lengthi124e4:name8:file.txtee")
                .expect("second metadata");

        assert_eq!(first.info_hash, "abd3ba80e40d26785bef279a79b6b57a52ec4a48");
        assert_eq!(second.info_hash, "1a3e1bb86c1cf4bda97cf742a117d7a8a2e0b744");
        assert_ne!(first.info_hash, second.info_hash);
    }

    #[test]
    fn rejects_torrent_without_top_level_info() {
        let err = parse_torrent_metadata(b"d8:announce9:https://te").expect_err("info required");

        assert!(err.to_string().contains("missing top-level info"));
    }

    #[test]
    fn rejects_torrent_path_traversal() {
        let err = parse_torrent_metadata(
            b"d4:infod5:filesld6:lengthi1e4:pathl2:..8:file.txteee4:name6:bundleee",
        )
        .expect_err("traversal rejected");

        assert!(err.to_string().contains("unsafe component"));
    }

    #[test]
    fn rejects_torrent_absolute_path_marker() {
        let err = parse_torrent_metadata(
            b"d4:infod5:filesld6:lengthi1e4:pathl2:C:8:file.txteee4:name6:bundleee",
        )
        .expect_err("absolute marker rejected");

        assert!(err.to_string().contains("absolute path marker"));
    }

    #[test]
    fn rejects_torrent_empty_path_component() {
        let err = parse_torrent_metadata(
            b"d4:infod5:filesld6:lengthi1e4:pathl0:8:file.txteee4:name6:bundleee",
        )
        .expect_err("empty component rejected");

        assert!(err.to_string().contains("unsafe component"));
    }

    #[test]
    fn rejects_torrent_windows_device_name() {
        let err = parse_torrent_metadata(
            b"d4:infod5:filesld6:lengthi1e4:pathl3:CON8:file.txteee4:name6:bundleee",
        )
        .expect_err("device name rejected");

        assert!(err.to_string().contains("reserved device"));
    }

    #[test]
    fn detects_private_flag_set_to_1() {
        // Single-file torrent with private:1 in the info dict ("4:test" = 4-byte name)
        let metadata = parse_torrent_metadata(b"d4:infod6:lengthi500e4:name4:test7:privatei1eee")
            .expect("valid private torrent");

        assert!(metadata.is_private, "private:1 must be detected");
    }

    #[test]
    fn private_flag_absent_means_public() {
        let metadata =
            parse_torrent_metadata(b"d8:announce9:https://t4:infod6:lengthi123e4:name8:file.txtee")
                .expect("valid torrent");

        assert!(!metadata.is_private, "no private key → public torrent");
    }

    #[test]
    fn private_flag_0_means_public() {
        let metadata = parse_torrent_metadata(b"d4:infod6:lengthi500e4:name4:test7:privatei0eee")
            .expect("valid torrent");

        assert!(!metadata.is_private, "private:0 must not trigger the lock");
    }

    #[test]
    fn parses_v2_file_tree_metadata() {
        let bencode = b"d4:infod9:file treed3:dird8:file.txtd6:lengthi999eeee4:name6:bundleee";
        let metadata = parse_torrent_metadata(bencode).expect("V2 file tree metadata");

        assert_eq!(metadata.name.as_deref(), Some("bundle"));
        assert_eq!(metadata.file_count, 1);
        assert_eq!(metadata.total_size, 999);
        assert_eq!(metadata.files[0].path, "dir/file.txt");
        assert_eq!(metadata.files[0].size, 999);
        // V2 info hash is 64 hex characters (SHA-256)
        assert_eq!(metadata.info_hash.len(), 64);
    }
}
