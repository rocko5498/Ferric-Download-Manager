use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("twitch.tv/")
        && (u.contains("/videos/") || u.contains("/clip/") || u.contains("clips.twitch.tv"))
}

// Public Twitch web-app client_id (stable for years)
const CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";
const GQL_URL: &str = "https://gql.twitch.tv/gql";

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let u = url.to_ascii_lowercase();
    if u.contains("/clip/") || u.contains("clips.twitch.tv") {
        extract_clip(url, client).await
    } else {
        extract_vod(url, client).await
    }
}

async fn extract_vod(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let vod_id = extract_vod_id(url)
        .ok_or_else(|| anyhow::anyhow!("Twitch: cannot extract VOD id from {url}"))?;

    // Step 1: get access token via GQL
    let token_query = serde_json::json!([{
        "operationName": "VideoAccessToken_Vod",
        "variables": { "id": vod_id, "params": "platform=web&playerType=picture-by-picture" },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": "0828119ded1c13477966434e15800ff57ddacf13ba1911c129dc2200705b0712"
            }
        }
    }]);

    let token_resp: serde_json::Value = client
        .post(GQL_URL)
        .header("Client-ID", CLIENT_ID)
        .json(&token_query)
        .send()
        .await
        .context("Twitch GQL token request")?
        .error_for_status()
        .context("Twitch GQL error")?
        .json()
        .await
        .context("parse Twitch GQL token response")?;

    let token = token_resp[0]["data"]["videoPlaybackAccessToken"]["value"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Twitch: no playback token in GQL response"))?;
    let sig = token_resp[0]["data"]["videoPlaybackAccessToken"]["signature"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("Twitch: no signature in GQL response"))?;

    // Step 2: get HLS playlist from Usher
    let m3u8_url = format!(
        "https://usher.twitchapps.com/vod/{vod_id}?nauth={token}&nauthsig={sig}&allow_source=true&allow_spectre=true"
    );

    // Step 3: get VOD metadata
    let meta_query = serde_json::json!([{
        "operationName": "VideoMetadata",
        "variables": { "channelLogin": "", "videoID": vod_id },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": "49b5b8f268cdeb259d75b58dcb0c1a748e3b575003448a2333dc5cdafd49adad"
            }
        }
    }]);

    let meta_resp: serde_json::Value = client
        .post(GQL_URL)
        .header("Client-ID", CLIENT_ID)
        .json(&meta_query)
        .send()
        .await
        .context("Twitch metadata GQL")?
        .json()
        .await
        .context("parse Twitch metadata")?;

    let video = &meta_resp[0]["data"]["video"];
    let title = video["title"].as_str().unwrap_or("Twitch VOD").to_string();
    let uploader = video["owner"]["login"].as_str().map(str::to_string);
    let thumbnail = video["previewThumbnailURL"].as_str().map(str::to_string);
    let duration_secs = video["lengthSeconds"].as_f64();

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats: vec![ExtractedFormat {
            id: "hls-source".to_string(),
            label: "HLS (source quality)".to_string(),
            url: m3u8_url.clone(),
            ext: "m3u8".to_string(),
            protocol: Some("m3u8".to_string()),
            manifest_url: Some(m3u8_url),
            format_note: Some("adaptive".to_string()),
            width: None,
            height: None,
            tbr_kbps: None,
            abr_kbps: None,
            vbr_kbps: None,
            fps: None,
            vcodec: None,
            acodec: None,
            container: None,
            language: None,
            filesize: None,
            has_drm: false,
            headers: Vec::new(),
            is_audio_only: false,
        }],
        extractor: "twitch".to_string(),
        description: video["description"].as_str().map(str::to_string),
        upload_date: video["publishedAt"]
            .as_str()
            .map(|s| s.split('T').next().unwrap_or("").replace('-', "")),
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

async fn extract_clip(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let slug = extract_clip_slug(url)
        .ok_or_else(|| anyhow::anyhow!("Twitch: cannot extract clip slug from {url}"))?;

    let query = serde_json::json!([{
        "operationName": "VideoAccessToken_Clip",
        "variables": { "slug": slug },
        "extensions": {
            "persistedQuery": {
                "version": 1,
                "sha256Hash": "36b89d2507fce29e5ca551df756d27c1cfe079e2609642b4390aa4c35796eb11"
            }
        }
    }]);

    let resp: serde_json::Value = client
        .post(GQL_URL)
        .header("Client-ID", CLIENT_ID)
        .json(&query)
        .send()
        .await
        .context("Twitch clip GQL")?
        .error_for_status()
        .context("Twitch GQL error")?
        .json()
        .await
        .context("parse Twitch clip GQL")?;

    let clip = &resp[0]["data"]["clip"];
    let title = clip["title"].as_str().unwrap_or("Twitch Clip").to_string();
    let uploader = clip["broadcaster"]["login"].as_str().map(str::to_string);
    let thumbnail = clip["thumbnailURL"].as_str().map(str::to_string);
    let duration_secs = clip["durationSeconds"].as_f64();

    let formats: Vec<ExtractedFormat> = clip["videoQualities"]
        .as_array()
        .unwrap_or(&vec![])
        .iter()
        .enumerate()
        .filter_map(|(i, q)| {
            let source_url = q["sourceURL"].as_str()?;
            let quality = q["quality"].as_str().unwrap_or("unknown");
            let fps = q["frameRate"].as_f64();
            let height = quality.trim_end_matches('p').parse::<u32>().ok();
            Some(ExtractedFormat {
                id: format!("clip-{i}"),
                label: format!("{quality}p"),
                url: source_url.to_string(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: Some(quality.to_string()),
                width: None,
                height,
                tbr_kbps: None,
                abr_kbps: None,
                vbr_kbps: None,
                fps,
                vcodec: None,
                acodec: None,
                container: Some("mp4".to_string()),
                language: None,
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: false,
            })
        })
        .collect();

    if formats.is_empty() {
        bail!("Twitch: no clip formats found for slug {slug}");
    }

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats,
        extractor: "twitch".to_string(),
        description: None,
        upload_date: clip["createdAt"]
            .as_str()
            .map(|s| s.split('T').next().unwrap_or("").replace('-', "")),
        tags: Vec::new(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_vod_id(url: &str) -> Option<String> {
    url.split("/videos/")
        .nth(1)
        .map(|s| s.split(['?', '/']).next().unwrap_or(s).to_string())
        .filter(|s| !s.is_empty())
}

fn extract_clip_slug(url: &str) -> Option<String> {
    // https://clips.twitch.tv/SlugHere or https://twitch.tv/channel/clip/SlugHere
    let s = if url.contains("/clip/") {
        url.split("/clip/").nth(1)?
    } else {
        url.split("clips.twitch.tv/").nth(1)?
    };
    Some(s.split(['?', '/']).next().unwrap_or(s).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_twitch_vod() {
        assert!(matches("https://www.twitch.tv/videos/2420786073"));
        assert!(matches("https://twitch.tv/channel/clip/FunnyMomentABC"));
        assert!(matches("https://clips.twitch.tv/FunnyMomentABC"));
        assert!(!matches("https://twitch.tv/channel"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_vod_id() {
        assert_eq!(
            extract_vod_id("https://www.twitch.tv/videos/2420786073"),
            Some("2420786073".to_string())
        );
        assert_eq!(
            extract_vod_id("https://twitch.tv/videos/123?t=1h"),
            Some("123".to_string())
        );
    }

    #[test]
    fn extracts_clip_slug() {
        assert_eq!(
            extract_clip_slug("https://clips.twitch.tv/FunnySlugABC"),
            Some("FunnySlugABC".to_string())
        );
        assert_eq!(
            extract_clip_slug("https://twitch.tv/ch/clip/NiceClip123"),
            Some("NiceClip123".to_string())
        );
    }
}
