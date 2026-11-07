use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

pub fn matches(url: &str) -> bool {
    url.to_ascii_lowercase().contains(".bandcamp.com/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
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
        .context("Bandcamp page error")?
        .text()
        .await
        .context("read Bandcamp page")?;

    let tralbum = extract_tralbum(&html)
        .ok_or_else(|| anyhow::anyhow!("Bandcamp: data-tralbum not found in page"))?;

    let root: serde_json::Value =
        serde_json::from_str(tralbum).context("parse Bandcamp tralbum JSON")?;

    let title = root["current"]["title"]
        .as_str()
        .unwrap_or_else(|| {
            root["trackinfo"][0]["title"]
                .as_str()
                .unwrap_or("Bandcamp track")
        })
        .to_string();

    let uploader = extract_artist(&html, &root);
    let thumbnail = extract_thumbnail(&html);

    let mut formats = Vec::new();

    // Single track page
    if let Some(tracks) = root["trackinfo"].as_array() {
        for (i, track) in tracks.iter().enumerate() {
            let track_title = track["title"].as_str().unwrap_or(&title);
            for quality in &[
                "mp3-v0",
                "mp3-128",
                "flac",
                "aiff-lossless",
                "vorbis",
                "alac",
                "wav",
                "aac-hi",
            ] {
                if let Some(file_url) = track["file"][quality].as_str() {
                    let ext = quality_to_ext(quality);
                    let abr = quality_to_abr(quality);
                    formats.push(ExtractedFormat {
                        id: format!("track-{i}-{quality}"),
                        label: format!("{track_title} ({quality})"),
                        url: file_url.to_string(),
                        ext: ext.to_string(),
                        protocol: Some("https".to_string()),
                        manifest_url: None,
                        format_note: Some((*quality).to_string()),
                        width: None,
                        height: None,
                        tbr_kbps: abr,
                        abr_kbps: abr,
                        vbr_kbps: None,
                        fps: None,
                        vcodec: None,
                        acodec: Some(ext.to_string()),
                        container: None,
                        language: None,
                        filesize: None,
                        has_drm: false,
                        headers: Vec::new(),
                        is_audio_only: true,
                    });
                    break; // one format per track is enough; best quality first
                }
            }
        }
    }

    if formats.is_empty() {
        bail!("Bandcamp: no audio formats found at {url}");
    }

    let duration_secs = root["trackinfo"][0]["duration"].as_f64();
    let upload_date = root["current"]["publish_date"].as_str().map(|s| {
        // "12 Jan 2024 00:00:00 GMT" or ISO-like
        parse_bandcamp_date(s)
    });

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats,
        extractor: "bandcamp".to_string(),
        description: root["current"]["about"].as_str().map(str::to_string),
        upload_date,
        tags: root["tags"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| t["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_tralbum(html: &str) -> Option<&str> {
    let marker = "data-tralbum=\"";
    let start = html.find(marker)? + marker.len();
    // The JSON is HTML-entity-escaped — find the closing unescaped quote
    // Bandcamp uses &quot; for " inside the attribute value
    let end = html[start..].find('"').map(|i| start + i)?;
    Some(&html[start..end])
}

fn extract_artist(html: &str, root: &serde_json::Value) -> Option<String> {
    root["artist"]
        .as_str()
        .or_else(|| root["current"]["band_name"].as_str())
        .map(str::to_string)
        .or_else(|| {
            // Fall back to <meta property="og:site_name">
            let marker = r#"og:site_name" content=""#;
            html.find(marker).map(|pos| {
                let after = &html[pos + marker.len()..];
                after.split('"').next().unwrap_or("").to_string()
            })
        })
}

fn extract_thumbnail(html: &str) -> Option<String> {
    let marker = r#"og:image" content=""#;
    html.find(marker).map(|pos| {
        let after = &html[pos + marker.len()..];
        after.split('"').next().unwrap_or("").to_string()
    })
}

fn quality_to_ext(q: &str) -> &'static str {
    match q {
        "mp3-128" | "mp3-v0" => "mp3",
        "flac" | "alac" => "flac",
        "aiff-lossless" => "aiff",
        "vorbis" => "ogg",
        "wav" => "wav",
        "aac-hi" => "m4a",
        _ => "mp3",
    }
}

fn quality_to_abr(q: &str) -> Option<f64> {
    match q {
        "mp3-128" => Some(128.0),
        "mp3-v0" => Some(225.0),
        "aac-hi" => Some(256.0),
        _ => None,
    }
}

fn parse_bandcamp_date(s: &str) -> String {
    // Try ISO first: "2024-01-15"
    if s.len() >= 10 && s.as_bytes()[4] == b'-' {
        return s[..10].replace('-', "");
    }
    s.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_bandcamp() {
        assert!(matches("https://artist.bandcamp.com/track/song-name"));
        assert!(matches("https://artist.bandcamp.com/album/album-name"));
        assert!(!matches("https://soundcloud.com/artist/track"));
    }

    #[test]
    fn quality_mappings() {
        assert_eq!(quality_to_ext("mp3-128"), "mp3");
        assert_eq!(quality_to_ext("flac"), "flac");
        assert_eq!(quality_to_abr("mp3-128"), Some(128.0));
        assert_eq!(quality_to_abr("flac"), None);
    }

    #[test]
    fn date_parse() {
        assert_eq!(parse_bandcamp_date("2024-01-15"), "20240115");
        assert_eq!(parse_bandcamp_date("2024-01-15T00:00:00Z"), "20240115");
    }
}
