use anyhow::{Context, Result};
use ferric_common::{ExtractResult, PlaylistEntry};

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    (u.contains("youtube.com/playlist") && u.contains("list="))
        || (u.contains("youtube.com/watch") && u.contains("list="))
        || (u.contains("soundcloud.com/") && u.contains("/sets/"))
}

/// Expands a playlist URL into an `ExtractResult` with `playlist_entries` populated.
/// `formats` is empty -- callers iterate `playlist_entries` and extract each individually.
#[tracing::instrument(skip(client))]
pub async fn expand(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    expand_range(url, client, None, None).await
}

/// Task 114: optional 1-based inclusive range (`start`..=`end`) and/or `newest_n`.
#[tracing::instrument(skip(client))]
pub async fn expand_range(
    url: &str,
    client: &reqwest::Client,
    start: Option<u32>,
    end: Option<u32>,
) -> Result<ExtractResult> {
    let mut result = if url.contains("youtube.com") || url.contains("music.youtube.com") {
        expand_youtube(url, client).await?
    } else {
        expand_soundcloud_set(url, client).await?
    };
    result.playlist_entries = slice_entries(result.playlist_entries, start, end);
    // reindex
    for (i, e) in result.playlist_entries.iter_mut().enumerate() {
        e.index = (i + 1) as u32;
    }
    Ok(result)
}

/// Keep entries in range. `start`/`end` are 1-based inclusive. If only `end` is
/// set with `start=None` and end small, treat as "newest N" (last N entries).
fn slice_entries(
    mut entries: Vec<PlaylistEntry>,
    start: Option<u32>,
    end: Option<u32>,
) -> Vec<PlaylistEntry> {
    if entries.is_empty() {
        return entries;
    }
    match (start, end) {
        (None, None) => entries,
        (Some(s), Some(e)) => {
            let s = s.max(1) as usize;
            let e = e as usize;
            entries
                .into_iter()
                .filter(|en| en.index as usize >= s && en.index as usize <= e)
                .collect()
        }
        (Some(s), None) => {
            let s = s.max(1) as usize;
            entries
                .into_iter()
                .filter(|en| en.index as usize >= s)
                .collect()
        }
        (None, Some(n)) if n > 0 && (n as usize) < entries.len() => {
            // newest N
            let skip = entries.len().saturating_sub(n as usize);
            entries.drain(0..skip);
            entries
        }
        (None, Some(_)) => entries,
    }
}

async fn expand_youtube(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("YouTube playlist page returned error")?
        .text()
        .await
        .context("read YouTube playlist page")?;

    let entries = extract_yt_videos(&html);
    let title = find_og(&html, "og:title").unwrap_or_else(|| "YouTube Playlist".to_string());

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail: find_og(&html, "og:image"),
        duration_secs: None,
        formats: Vec::new(),
        extractor: "youtube:playlist".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: entries,
        chapters: Vec::new(),
    })
}

/// Extracts video IDs and titles from YouTube's embedded ytInitialData JSON.
/// Pattern: `"videoId":"xxxxxxxxxxx"` followed nearby by title text.
fn extract_yt_videos(html: &str) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut search = html;
    let mut index = 1u32;

    while let Some(pos) = search.find("\"videoId\":\"") {
        let after = pos + 11;
        if after + 11 > search.len() {
            break;
        }
        let id_slice = &search[after..after + 11];
        // YouTube video IDs are 11 chars: alphanumeric + - _
        if id_slice
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
            && seen.insert(id_slice.to_string())
        {
            entries.push(PlaylistEntry {
                url: format!("https://www.youtube.com/watch?v={id_slice}"),
                title: None,
                index,
                duration_secs: None,
                thumbnail: Some(format!("https://i.ytimg.com/vi/{id_slice}/mqdefault.jpg")),
            });
            index += 1;
        }
        search = &search[after..];
    }

    entries
}

async fn expand_soundcloud_set(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let html = client
        .get(url)
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .context("SoundCloud returned error")?
        .text()
        .await
        .context("read SoundCloud page")?;

    let entries = soundcloud_track_urls(&html);
    let title = find_og(&html, "og:title").unwrap_or_else(|| "SoundCloud Set".to_string());

    Ok(ExtractResult {
        title,
        uploader: None,
        thumbnail: find_og(&html, "og:image"),
        duration_secs: None,
        formats: Vec::new(),
        extractor: "soundcloud:set".to_string(),
        description: None,
        upload_date: None,
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: entries,
        chapters: Vec::new(),
    })
}

/// Pulls individual track permalink_urls out of SoundCloud's hydration JSON blobs.
fn soundcloud_track_urls(html: &str) -> Vec<PlaylistEntry> {
    let mut entries = Vec::new();
    let mut search = html;
    let mut index = 1u32;

    while let Some(start) = search.find("\"permalink_url\":\"https://soundcloud.com/") {
        let after = start + 17;
        if after >= search.len() {
            break;
        }
        let rest = &search[after..];
        if let Some(end) = rest.find('"') {
            let url = rest[..end].replace("\\/", "/");
            if !url.contains("/sets/") && url.starts_with("https://soundcloud.com/") {
                entries.push(PlaylistEntry {
                    url,
                    title: None,
                    index,
                    duration_secs: None,
                    thumbnail: None,
                });
                index += 1;
            }
        }
        search = &search[after..];
    }

    entries
}

fn find_og(html: &str, property: &str) -> Option<String> {
    let needle = format!("property=\"{property}\" content=\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    let end = rest.find('"')?;
    let v = &rest[..end];
    if v.is_empty() {
        None
    } else {
        Some(v.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_youtube_playlist() {
        assert!(matches("https://www.youtube.com/playlist?list=PLabcdef123"));
        assert!(matches("https://www.youtube.com/watch?v=abc&list=PLabcdef"));
        assert!(!matches("https://www.youtube.com/watch?v=abc"));
    }

    #[test]
    fn matches_soundcloud_set() {
        assert!(matches("https://soundcloud.com/artist/sets/my-playlist"));
        assert!(!matches("https://soundcloud.com/artist/track-name"));
    }

    #[test]
    fn yt_video_extraction_deduplicates() {
        let html =
            r#"{"videoId":"dQw4w9WgXcQ"},{"videoId":"dQw4w9WgXcQ"},{"videoId":"abcdefghijk"}"#;
        let entries = extract_yt_videos(html);
        assert_eq!(entries.len(), 2);
        assert!(entries[0].url.contains("dQw4w9WgXcQ"));
        assert!(entries[1].url.contains("abcdefghijk"));
    }

    #[test]
    fn sc_tracks_skip_sets() {
        let html = r#"{"permalink_url":"https://soundcloud.com/artist/track1"},{"permalink_url":"https://soundcloud.com/artist/sets/mylist"}"#;
        let entries = soundcloud_track_urls(html);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].url.contains("track1"));
    }
}
