use anyhow::{anyhow, Result};
use ferric_common::ExtractResult;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("youtube.com/watch")
        || u.contains("youtube.com/embed/")
        || u.contains("youtu.be/")
        || u.contains("youtube.com/shorts/")
        || u.contains("youtube.com/live/")
        || u.contains("music.youtube.com/watch")
        || u.contains("music.youtube.com/playlist")
        || (u.contains("youtube.com/") && u.contains("v="))
}

#[must_use]
pub fn normalize_watch_url(url: &str) -> String {
    if let Some(id) = extract_video_id(url)
        .or_else(|| extract_path_id(url, "/shorts/"))
        .or_else(|| extract_path_id(url, "/live/"))
    {
        let host = if is_music(url) {
            "music.youtube.com"
        } else {
            "www.youtube.com"
        };
        return format!("https://{host}/watch?v={id}");
    }
    url.to_string()
}

#[must_use]
pub fn is_shorts(url: &str) -> bool {
    url.to_ascii_lowercase().contains("/shorts/")
}

#[must_use]
pub fn is_music(url: &str) -> bool {
    url.to_ascii_lowercase().contains("music.youtube.com")
}

/// YouTube changes too frequently for the unmaintained embedded-JS extractor.
/// Returning an error makes `extract_with_config` use packaged official yt-dlp.
#[tracing::instrument(skip_all, fields(url))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let _ = (url, client);
    Err(anyhow!(
        "YouTube extraction requires the packaged yt-dlp engine"
    ))
}

fn extract_video_id(url: &str) -> Option<String> {
    if let Some(pos) = url.find("v=") {
        let id = valid_id(&url[pos + 2..]);
        if id.is_some() {
            return id;
        }
    }
    url.find("youtu.be/")
        .and_then(|pos| valid_id(&url[pos + 9..]))
}

fn extract_path_id(url: &str, marker: &str) -> Option<String> {
    let pos = url.to_ascii_lowercase().find(marker)?;
    valid_id(&url[pos + marker.len()..])
}

fn valid_id(value: &str) -> Option<String> {
    let id: String = value
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    (id.len() == 11).then_some(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_supported_youtube_urls() {
        assert!(matches("https://www.youtube.com/watch?v=dQw4w9WgXcQ"));
        assert!(matches("https://youtu.be/dQw4w9WgXcQ"));
        assert!(matches("https://youtube.com/shorts/abc123defgh"));
        assert!(matches("https://www.youtube.com/live/abcdefghijk"));
        assert!(!matches("https://vimeo.com/123456789"));
    }

    #[test]
    fn normalizes_short_and_music_urls() {
        assert_eq!(
            normalize_watch_url("https://www.youtube.com/shorts/abcdefghijk"),
            "https://www.youtube.com/watch?v=abcdefghijk"
        );
        assert_eq!(
            normalize_watch_url("https://music.youtube.com/watch?v=dQw4w9WgXcQ"),
            "https://music.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert!(is_shorts("https://www.youtube.com/shorts/dQw4w9WgXcQ"));
        assert!(is_music("https://music.youtube.com/watch?v=dQw4w9WgXcQ"));
    }
}
