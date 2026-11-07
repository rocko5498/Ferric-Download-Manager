use anyhow::{bail, Context, Result};
use ferric_common::{ExtractResult, ExtractedFormat};
use serde::Deserialize;

pub fn matches(url: &str) -> bool {
    let u = url.to_ascii_lowercase();
    u.contains("bilibili.com/video/") || u.contains("b23.tv/")
}

#[tracing::instrument(skip(client))]
pub async fn extract(url: &str, client: &reqwest::Client) -> Result<ExtractResult> {
    let bvid = extract_bvid(url)
        .ok_or_else(|| anyhow::anyhow!("Bilibili: cannot extract BVID from {url}"))?;

    // Step 1: video info (title, cid, owner)
    let info_url = format!("https://api.bilibili.com/x/web-interface/view?bvid={bvid}");
    let info_resp = client
        .get(&info_url)
        .header("Referer", "https://www.bilibili.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .with_context(|| format!("GET {info_url}"))?
        .error_for_status()
        .context("Bilibili info API error")?;

    let info: BiliResponse<BiliVideoInfo> =
        info_resp.json().await.context("parse Bilibili info")?;
    if info.code != 0 {
        bail!(
            "Bilibili API error {}: {}",
            info.code,
            info.message.unwrap_or_default()
        );
    }
    let data = info
        .data
        .ok_or_else(|| anyhow::anyhow!("Bilibili: no data in info response"))?;

    let title = data.title.clone();
    let uploader = Some(data.owner.name.clone());
    let thumbnail = Some(data.pic.clone());
    let duration_secs = Some(data.duration as f64);
    let cid = data.cid;
    let aid = data.aid;

    // Step 2: playurl (qn=80=1080p, 64=720p, 32=480p, 16=360p)
    let play_url = format!(
        "https://api.bilibili.com/x/player/playurl?avid={aid}&cid={cid}&qn=80&type=mp4&fnval=1&fnver=0&fourk=1"
    );
    let play_resp = client
        .get(&play_url)
        .header("Referer", "https://www.bilibili.com/")
        .header(
            "User-Agent",
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
        )
        .send()
        .await
        .with_context(|| format!("GET {play_url}"))?
        .error_for_status()
        .context("Bilibili playurl API error")?;

    let play: BiliResponse<BiliPlayUrl> =
        play_resp.json().await.context("parse Bilibili playurl")?;

    let formats: Vec<ExtractedFormat> = play
        .data
        .as_ref()
        .map(|d| d.durl.as_slice())
        .unwrap_or(&[])
        .iter()
        .enumerate()
        .filter_map(|(i, durl)| {
            let url_str = durl.url.split('?').next().unwrap_or(&durl.url);
            let ext = url_str
                .rsplit('.')
                .next()
                .unwrap_or("mp4")
                .to_ascii_lowercase();
            Some(ExtractedFormat {
                id: format!("bili-{i}"),
                label: format!("Bilibili stream {}", i + 1),
                url: durl.url.clone(),
                ext,
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: None,
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
                filesize: durl.size,
                has_drm: false,
                headers: vec![(
                    "Referer".to_string(),
                    "https://www.bilibili.com/".to_string(),
                )],
                is_audio_only: false,
            })
        })
        .collect();

    if formats.is_empty() {
        bail!("Bilibili: no formats found for {bvid}");
    }

    Ok(ExtractResult {
        title,
        uploader,
        thumbnail,
        duration_secs,
        formats,
        extractor: "bilibili".to_string(),
        description: data.desc.filter(|s| !s.is_empty()),
        upload_date: Some(unix_to_ymd(data.pubdate)),
        tags: data
            .tags
            .unwrap_or_default()
            .into_iter()
            .map(|t| t.tag_name)
            .collect(),
        subtitles: Vec::new(),
        playlist_entries: Vec::new(),
        chapters: Vec::new(),
    })
}

fn extract_bvid(url: &str) -> Option<String> {
    // https://www.bilibili.com/video/BV1xx411c7XE
    url.split("/video/")
        .nth(1)
        .map(|s| s.split(['?', '/']).next().unwrap_or(s).to_string())
        .filter(|s| s.starts_with("BV") || s.starts_with("av"))
}

fn unix_to_ymd(ts: u64) -> String {
    let days = ts / 86400;
    let (y, m, d) = days_to_ymd(days);
    format!("{y:04}{m:02}{d:02}")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let diy = if leap { 366 } else { 365 };
        if days < diy {
            break;
        }
        days -= diy;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let months = [
        31u64,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1u64;
    for &dm in &months {
        if days < dm {
            break;
        }
        days -= dm;
        month += 1;
    }
    (year, month, days + 1)
}

// ── Deserialization ──────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct BiliResponse<T> {
    code: i64,
    message: Option<String>,
    data: Option<T>,
}

#[derive(Deserialize)]
struct BiliVideoInfo {
    title: String,
    desc: Option<String>,
    pic: String,
    duration: u64,
    pubdate: u64,
    cid: u64,
    aid: u64,
    owner: BiliOwner,
    tags: Option<Vec<BiliTag>>,
}

#[derive(Deserialize)]
struct BiliOwner {
    name: String,
}

#[derive(Deserialize)]
struct BiliTag {
    tag_name: String,
}

#[derive(Deserialize)]
struct BiliPlayUrl {
    durl: Vec<BiliDurl>,
}

#[derive(Deserialize)]
struct BiliDurl {
    url: String,
    size: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_bilibili() {
        assert!(matches("https://www.bilibili.com/video/BV1xx411c7XE"));
        assert!(matches("https://bilibili.com/video/av170001"));
        assert!(!matches("https://youtube.com/watch?v=abc"));
    }

    #[test]
    fn extracts_bvid() {
        assert_eq!(
            extract_bvid("https://www.bilibili.com/video/BV1xx411c7XE"),
            Some("BV1xx411c7XE".to_string())
        );
        assert_eq!(
            extract_bvid("https://bilibili.com/video/BV1xx411c7XE?p=1"),
            Some("BV1xx411c7XE".to_string())
        );
    }
}
