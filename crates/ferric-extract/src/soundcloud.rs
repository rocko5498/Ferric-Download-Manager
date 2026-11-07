use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;

pub fn matches(url: &str) -> bool {
    url.to_ascii_lowercase().contains("soundcloud.com/")
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
        .context("SoundCloud page error")?
        .text()
        .await
        .context("read SoundCloud page")?;

    let client_id = extract_client_id(&html)
        .ok_or_else(|| anyhow::anyhow!("SoundCloud: client_id not found in page"))?;

    // Hydration JSON contains track data
    let hydration = extract_hydration(&html)
        .ok_or_else(|| anyhow::anyhow!("SoundCloud: hydration data not found"))?;

    let root: serde_json::Value =
        serde_json::from_str(hydration).context("parse SoundCloud hydration JSON")?;

    // Find the track entity (first Sound item in hydration array)
    let track = root
        .as_array()
        .and_then(|arr| arr.iter().find(|item| item["hydratable"] == "sound"))
        .and_then(|item| item.get("data"))
        .ok_or_else(|| anyhow::anyhow!("SoundCloud: no track data in hydration"))?;

    let track_id = track["id"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("SoundCloud: no track id"))?;

    let title = track["title"]
        .as_str()
        .unwrap_or("SoundCloud track")
        .to_string();
    let uploader = track["user"]["username"].as_str().map(str::to_string);
    let thumbnail = track["artwork_url"]
        .as_str()
        .map(|u| u.replace("-large", "-t500x500"));
    let duration_secs = track["duration"].as_f64().map(|ms| ms / 1000.0);
    let description = track["description"].as_str().map(str::to_string);
    let upload_date = track["created_at"].as_str().map(|s| {
        // "2024-01-15T12:00:00Z" → "20240115"
        s.split('T').next().unwrap_or("").replace('-', "")
    });

    // Resolve stream URL
    let streams_url =
        format!("https://api-v2.soundcloud.com/tracks/{track_id}/streams?client_id={client_id}");
    let streams_resp = client
        .get(&streams_url)
        .send()
        .await
        .with_context(|| format!("GET {streams_url}"))?
        .error_for_status()
        .context("SoundCloud streams API error")?;

    let streams: SoundCloudStreams = streams_resp.json().await.context("parse streams JSON")?;

    let mut formats = Vec::new();

    if let Some(mp3_url) = &streams.http_mp3_128_url {
        formats.push(make_format(
            "mp3-128",
            "MP3 128kbps",
            mp3_url,
            "mp3",
            Some(128.0),
            true,
        ));
    }
    if let Some(hls_url) = &streams.hls_mp3_128_url {
        formats.push(make_format(
            "hls-mp3-128",
            "HLS MP3 128kbps",
            hls_url,
            "mp3",
            Some(128.0),
            true,
        ));
    }
    if let Some(opus_url) = &streams.hls_opus_64_url {
        formats.push(make_format(
            "hls-opus-64",
            "HLS Opus 64kbps",
            opus_url,
            "opus",
            Some(64.0),
            true,
        ));
    }

    if formats.is_empty() {
        bail!("SoundCloud: no stream URLs found for track {track_id}");
    }

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats,
        extractor: "soundcloud".to_string(),
        description,
        upload_date,
        tags: track["tag_list"]
            .as_str()
            .map(|t| t.split_whitespace().map(str::to_string).collect())
            .unwrap_or_default(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn make_format(
    id: &str,
    label: &str,
    url: &str,
    ext: &str,
    abr: Option<f64>,
    audio_only: bool,
) -> ExtractedFormat {
    ExtractedFormat {
        id: id.to_string(),
        label: label.to_string(),
        url: url.to_string(),
        ext: ext.to_string(),
        protocol: Some(if url.contains(".m3u8") {
            "m3u8".to_string()
        } else {
            "https".to_string()
        }),
        manifest_url: if url.contains(".m3u8") {
            Some(url.to_string())
        } else {
            None
        },
        format_note: Some(label.to_string()),
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
        is_audio_only: audio_only,
    }
}

fn extract_client_id(html: &str) -> Option<String> {
    // All known serialization forms of client_id in SoundCloud page JS
    let needles = [
        r#"client_id":""#,
        r#"client_id:""#,
        "client_id=",
        "client_id%3D",
        "client_id%22%3A%22",
    ];
    for needle in &needles {
        for (i, chunk) in html.split(needle).enumerate() {
            if i == 0 {
                continue;
            } // skip content before first needle
            let id: String = chunk
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if id.len() >= 16 {
                return Some(id);
            }
        }
    }
    None
}

fn extract_hydration(html: &str) -> Option<&str> {
    let marker = "window.__sc_hydration = ";
    let start = html.find(marker)? + marker.len();
    let end = html[start..].find(";</script>").map(|i| start + i)?;
    Some(&html[start..end])
}

#[derive(Deserialize)]
struct SoundCloudStreams {
    http_mp3_128_url: Option<String>,
    hls_mp3_128_url: Option<String>,
    hls_opus_64_url: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_soundcloud_urls() {
        assert!(matches("https://soundcloud.com/artist/track-name"));
        assert!(matches("https://soundcloud.com/artist/sets/playlist"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_client_id_from_html() {
        let html = r#"<script>client_id:"abc123def456ghi7"</script>"#;
        assert_eq!(
            extract_client_id(html),
            Some("abc123def456ghi7".to_string())
        );
    }

    #[test]
    fn upload_date_format() {
        let s = "2024-01-15T12:00:00Z";
        let result = s.split('T').next().unwrap_or("").replace('-', "");
        assert_eq!(result, "20240115");
    }
}
