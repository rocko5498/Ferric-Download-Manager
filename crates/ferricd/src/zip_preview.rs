//! IDM's "Preview" in the Download File Info dialog: list what is inside a zip archive by reading its
//! central directory from the END of the file with ranged requests, without downloading the archive.
//!

use serde::Serialize;

/// End-of-central-directory record (22 bytes) plus the longest possible archive comment.
const MAX_TAIL: u64 = 22 + 65_535;
const MAX_DIRECTORY: u64 = 4 * 1024 * 1024;
const MAX_ENTRIES: usize = 2000;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct ZipEntry {
    pub name: String,
    pub size: u64,
}

fn u16_at(b: &[u8], at: usize) -> usize {
    usize::from(u16::from_le_bytes([b[at], b[at + 1]]))
}

fn u32_at(b: &[u8], at: usize) -> u64 {
    u64::from(u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]]))
}

/// Offset of the end-of-central-directory record in the tail of the file.
fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    (0..=tail.len() - 22).rev().find(|&i| {
        tail[i..i + 4] == [0x50, 0x4b, 0x05, 0x06] && i + 22 + u16_at(tail, i + 20) == tail.len()
    })
}

/// (entry count, directory size, directory offset)
fn parse_eocd(rec: &[u8]) -> Result<(usize, u64, u64), String> {
    let entries = u16_at(rec, 10);
    let size = u32_at(rec, 12);
    let offset = u32_at(rec, 16);
    if entries == 0xFFFF || size == 0xFFFF_FFFF || offset == 0xFFFF_FFFF {
        return Err("ZIP64 archives cannot be previewed yet".into());
    }
    Ok((entries, size, offset))
}

fn parse_directory(dir: &[u8], count: usize) -> Result<Vec<ZipEntry>, String> {
    let mut out = Vec::new();
    let mut pos = 0usize;
    while out.len() < count.min(MAX_ENTRIES) {
        if pos + 46 > dir.len() || dir[pos..pos + 4] != [0x50, 0x4b, 0x01, 0x02] {
            break;
        }
        let size = u32_at(dir, pos + 24);
        let (name_len, extra_len, comment_len) = (
            u16_at(dir, pos + 28),
            u16_at(dir, pos + 30),
            u16_at(dir, pos + 32),
        );
        let name_end = pos + 46 + name_len;
        if name_end > dir.len() {
            return Err("damaged zip directory".into());
        }
        out.push(ZipEntry {
            name: String::from_utf8_lossy(&dir[pos + 46..name_end]).into_owned(),
            size,
        });
        pos = name_end + extra_len + comment_len;
    }
    if out.is_empty() && count > 0 {
        return Err("could not read the zip directory".into());
    }
    Ok(out)
}

async fn fetch_range(
    client: &reqwest::Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, String> {
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={start}-{end}"))
        .timeout(std::time::Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    // A server that ignores Range would send the whole archive: refuse before reading the body.
    if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
        return Err(
            "this server does not support partial downloads, so the archive cannot be previewed"
                .into(),
        );
    }
    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("read failed: {e}"))?;
    Ok(bytes.to_vec())
}

pub async fn preview(
    client: &reqwest::Client,
    url: &str,
    total: u64,
) -> Result<Vec<ZipEntry>, String> {
    if total < 22 {
        return Err("not a zip archive".into());
    }
    let tail_start = total.saturating_sub(MAX_TAIL);
    let tail = fetch_range(client, url, tail_start, total - 1).await?;
    let at = find_eocd(&tail).ok_or("this does not look like a zip archive")?;
    let (count, cd_size, cd_off) = parse_eocd(&tail[at..])?;
    if cd_size > MAX_DIRECTORY || cd_off.checked_add(cd_size).is_none_or(|e| e > total) {
        return Err("zip directory is too large or damaged".into());
    }
    let dir = if cd_off >= tail_start {
        let from = (cd_off - tail_start) as usize;
        tail.get(from..from + cd_size as usize)
            .ok_or("damaged zip directory")?
            .to_vec()
    } else if cd_size == 0 {
        Vec::new()
    } else {
        fetch_range(client, url, cd_off, cd_off + cd_size - 1).await?
    };
    parse_directory(&dir, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real (stored, no compression) zip built by hand: local headers, central directory, EOCD.
    fn make_zip(files: &[(&str, u32)], comment: &str) -> Vec<u8> {
        let mut body = Vec::new();
        let mut central = Vec::new();
        for (name, size) in files {
            let offset = body.len() as u32;
            body.extend([0x50, 0x4b, 0x03, 0x04, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            body.extend(0u32.to_le_bytes()); // crc
            body.extend(size.to_le_bytes());
            body.extend(size.to_le_bytes());
            body.extend((name.len() as u16).to_le_bytes());
            body.extend(0u16.to_le_bytes());
            body.extend(name.as_bytes());
            body.extend(std::iter::repeat_n(b'x', *size as usize));

            central.extend([0x50, 0x4b, 0x01, 0x02, 20, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend(0u32.to_le_bytes());
            central.extend(size.to_le_bytes());
            central.extend(size.to_le_bytes());
            central.extend((name.len() as u16).to_le_bytes());
            central.extend([0u8; 8]); // extra len, comment len, disk number, internal attrs
            central.extend(0u32.to_le_bytes()); // external attrs
            central.extend(offset.to_le_bytes());
            central.extend(name.as_bytes());
        }
        let cd_offset = body.len() as u32;
        let mut zip = body;
        zip.extend(&central);
        zip.extend([0x50, 0x4b, 0x05, 0x06, 0, 0, 0, 0]);
        zip.extend((files.len() as u16).to_le_bytes());
        zip.extend((files.len() as u16).to_le_bytes());
        zip.extend((central.len() as u32).to_le_bytes());
        zip.extend(cd_offset.to_le_bytes());
        zip.extend((comment.len() as u16).to_le_bytes());
        zip.extend(comment.as_bytes());
        zip
    }

    fn list(zip: &[u8]) -> Result<Vec<ZipEntry>, String> {
        let at = find_eocd(zip).ok_or("no eocd")?;
        let (count, size, off) = parse_eocd(&zip[at..])?;
        parse_directory(&zip[off as usize..(off + size) as usize], count)
    }

    #[test]
    fn lists_names_and_sizes_from_the_central_directory() {
        let zip = make_zip(
            &[
                ("a.txt", 5),
                ("docs/readme.md", 120),
                ("img/logo.png", 3000),
            ],
            "",
        );
        let entries = list(&zip).unwrap();
        assert_eq!(
            entries,
            vec![
                ZipEntry {
                    name: "a.txt".into(),
                    size: 5
                },
                ZipEntry {
                    name: "docs/readme.md".into(),
                    size: 120
                },
                ZipEntry {
                    name: "img/logo.png".into(),
                    size: 3000
                },
            ]
        );
    }

    #[test]
    fn an_archive_comment_does_not_hide_the_directory() {
        let zip = make_zip(&[("only.bin", 10)], "made by a tool, with a comment");
        assert_eq!(list(&zip).unwrap()[0].name, "only.bin");
    }

    #[test]
    fn junk_and_zip64_are_refused_not_misread() {
        assert!(find_eocd(&[0u8; 100]).is_none());
        let mut zip64 = make_zip(&[("x", 1)], "");
        let at = find_eocd(&zip64).unwrap();
        zip64[at + 16..at + 20].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
        assert!(parse_eocd(&zip64[at..]).unwrap_err().contains("ZIP64"));
    }

    #[test]
    fn a_truncated_directory_is_an_error() {
        let zip = make_zip(&[("name-that-gets-cut.txt", 4)], "");
        let at = find_eocd(&zip).unwrap();
        let (count, _, off) = parse_eocd(&zip[at..]).unwrap();
        assert!(parse_directory(&zip[off as usize..off as usize + 60], count).is_err());
    }
}
