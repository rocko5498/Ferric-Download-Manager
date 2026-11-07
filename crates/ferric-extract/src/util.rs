//! Shared helpers for site extractors.

use ferric_common::{ExtractResult, ExtractedFormat};

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";

#[must_use]
pub fn host_contains(url: &str, needles: &[&str]) -> bool {
    let u = url.to_ascii_lowercase();
    needles.iter().any(|n| u.contains(n))
}

#[must_use]
pub fn find_between<'a>(hay: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let i = hay.find(start)?;
    let rest = &hay[i + start.len()..];
    let j = rest.find(end)?;
    Some(&rest[..j])
}

#[must_use]
pub fn find_og(html: &str, prop: &str) -> Option<String> {
    // <meta property="og:title" content="...">
    let needle = format!("property=\"{prop}\"");
    let mut search = html;
    while let Some(pos) = search.find(&needle) {
        let window = &search[pos..].get(..400).unwrap_or(&search[pos..]);
        if let Some(c) = attr(window, "content") {
            return Some(c);
        }
        search = &search[pos + needle.len()..];
    }
    // property before content alternate order
    let needle2 = format!("content=\"");
    let _ = needle2;
    None
}

#[must_use]
pub fn attr(chunk: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = chunk.find(&key)?;
    let rest = &chunk[i + key.len()..];
    let j = rest.find('"')?;
    Some(rest[..j].to_string())
}

#[must_use]
pub fn direct_format(id: &str, label: &str, url: String, ext: &str) -> ExtractedFormat {
    let height = parse_height(&url).or_else(|| parse_height(label));
    ExtractedFormat {
        id: id.to_string(),
        label: label.to_string(),
        url,
        ext: ext.to_string(),
        protocol: Some(if ext == "m3u8" {
            "m3u8".into()
        } else if ext == "mpd" {
            "dash".into()
        } else {
            "https".into()
        }),
        manifest_url: None,
        format_note: Some(label.to_string()),
        width: None,
        height,
        tbr_kbps: None,
        abr_kbps: None,
        vbr_kbps: None,
        fps: None,
        vcodec: None,
        acodec: None,
        container: Some(ext.to_string()),
        language: None,
        filesize: None,
        has_drm: false,
        headers: Vec::new(),
        is_audio_only: false,
    }
}

#[must_use]
pub fn parse_height(s: &str) -> Option<u32> {
    // match 1080p, /1080/, height=720
    for token in s.split(|c: char| !c.is_ascii_digit()) {
        if token.is_empty() {
            continue;
        }
        if let Ok(n) = token.parse::<u32>() {
            if (144..=4320).contains(&n) {
                return Some(n);
            }
        }
    }
    None
}

#[must_use]
pub fn empty_result(title: String, extractor: &str) -> ExtractResult {
    ExtractResult {
        title,
        uploader: None,
        thumbnail: None,
        duration_secs: None,
        formats: Vec::new(),
        extractor: extractor.to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_height_from_label() {
        assert_eq!(parse_height("1080p"), Some(1080));
        assert_eq!(parse_height("https://cdn/video/720/index.m3u8"), Some(720));
    }
}
