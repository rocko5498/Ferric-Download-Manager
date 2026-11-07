//! Post-download sidecar files + embedding (Tasks 107–109, 113).

use anyhow::{Context, Result};
use ferric_common::{Chapter, ExtractResult};
use std::path::{Path, PathBuf};

/// Write yt-dlp-compatible `.info.json` next to the media file (pure-Rust export).
pub async fn write_info_json(media_path: &Path, result: &ExtractResult) -> Result<PathBuf> {
    let path = sidecar(media_path, "info.json");
    let json =
        ferric_extract::info_json::to_ytdlp_info_string(result).context("serialize info.json")?;
    tokio::fs::write(&path, json)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Write plain-text description.
pub async fn write_description(media_path: &Path, description: &str) -> Result<PathBuf> {
    let path = sidecar(media_path, "description.txt");
    tokio::fs::write(&path, description)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Write simple chapter text file (ffmpeg/mkv friendly style).
pub async fn write_chapters_txt(media_path: &Path, chapters: &[Chapter]) -> Result<PathBuf> {
    let path = sidecar(media_path, "chapters.txt");
    let mut body = String::from(";FFMETADATA1\n");
    for ch in chapters {
        let start_ms = (ch.start_secs * 1000.0) as u64;
        let end_ms = (ch.end_secs * 1000.0) as u64;
        body.push_str(&format!(
            "[CHAPTER]\nTIMEBASE=1/1000\nSTART={start_ms}\nEND={end_ms}\ntitle={}\n",
            ch.title.replace('\n', " ")
        ));
    }
    tokio::fs::write(&path, body)
        .await
        .with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

/// Fetch thumbnail URL and embed into MP4/MKV via ffmpeg (Task 107).
pub async fn embed_thumbnail(
    media_path: &Path,
    thumbnail_url: &str,
    client: &reqwest::Client,
) -> Result<()> {
    crate::security::validate_url(thumbnail_url)
        .map_err(|e| anyhow::anyhow!("thumbnail URL rejected: {e}"))?;
    let ffmpeg = crate::media::ffmpeg_path()
        .ok_or_else(|| anyhow::anyhow!("ffmpeg not available for thumbnail embed"))?;
    let bytes = client
        .get(thumbnail_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let thumb_path = media_path.with_extension("thumb.jpg");
    tokio::fs::write(&thumb_path, &bytes).await?;
    let out_path = media_path.with_extension("thumb-out.mp4");
    // Prefer stream copy + attach for mkv; for mp4 use -i thumb map.
    let mut ffmpeg_cmd = tokio::process::Command::new(&ffmpeg);
    #[cfg(windows)]
    ffmpeg_cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flash
    let status = ffmpeg_cmd
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-y",
            "-i",
            &media_path.to_string_lossy(),
            "-i",
            &thumb_path.to_string_lossy(),
            "-map",
            "0",
            "-map",
            "1",
            "-c",
            "copy",
            "-c:v:1",
            "mjpeg",
            "-disposition:v:1",
            "attached_pic",
            &out_path.to_string_lossy(),
        ])
        .status()
        .await
        .context("ffmpeg thumbnail embed")?;
    let _ = tokio::fs::remove_file(&thumb_path).await;
    if status.success() {
        if tokio::fs::rename(&out_path, media_path).await.is_err() {
            tokio::fs::copy(&out_path, media_path).await?;
            let _ = tokio::fs::remove_file(&out_path).await;
        }
        Ok(())
    } else {
        let _ = tokio::fs::remove_file(&out_path).await;
        anyhow::bail!("ffmpeg thumbnail embed failed")
    }
}

/// Run configured sidecars after a successful extract-based download.
pub async fn apply_extract_sidecars(
    media_path: &Path,
    result: &ExtractResult,
    client: &reqwest::Client,
    write_info: bool,
    write_desc: bool,
    write_chapters: bool,
    embed_thumb: bool,
) -> Result<()> {
    if write_info {
        let _ = write_info_json(media_path, result).await?;
    }
    if write_desc {
        if let Some(ref d) = result.description {
            if !d.is_empty() {
                let _ = write_description(media_path, d).await?;
            }
        }
    }
    if write_chapters && !result.chapters.is_empty() {
        let _ = write_chapters_txt(media_path, &result.chapters).await?;
    }
    if embed_thumb {
        if let Some(ref t) = result.thumbnail {
            if let Err(e) = embed_thumbnail(media_path, t, client).await {
                tracing::warn!(error = %e, "thumbnail embed skipped");
            }
        }
    }
    // Task 108 — auto-download preferred subtitles next to media
    if let Err(e) = download_subtitles_sidecar(media_path, result, client).await {
        tracing::debug!(error = %e, "subtitle sidecar skipped");
    }
    Ok(())
}

/// Download preferred-language subtitles; convert VTT→SRT when useful.
pub async fn download_subtitles_sidecar(
    media_path: &Path,
    result: &ExtractResult,
    client: &reqwest::Client,
) -> Result<()> {
    if result.subtitles.is_empty() {
        return Ok(());
    }
    let preferred = ["en", "en-US", "en-GB"];
    let tracks = ferric_extract::subtitles::filter_langs(&result.subtitles, &preferred);
    for (i, track) in tracks.iter().take(3).enumerate() {
        let resp = client.get(&track.url).send().await?;
        if !resp.status().is_success() {
            continue;
        }
        let body = resp.text().await?;
        let (ext, body) = if body.contains("WEBVTT") || body.trim_start().starts_with("WEBVTT") {
            ("srt", ferric_extract::subtitles::vtt_to_srt(&body))
        } else {
            ("vtt", body)
        };
        let suffix = if i == 0 {
            format!("{}.{ext}", track.lang)
        } else {
            format!("{}-{i}.{ext}", track.lang)
        };
        let path = sidecar(media_path, &suffix);
        tokio::fs::write(&path, body)
            .await
            .with_context(|| format!("write {}", path.display()))?;
    }
    Ok(())
}

fn sidecar(media: &Path, suffix: &str) -> PathBuf {
    let stem = media
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("download");
    media
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("{stem}.{suffix}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferric_common::Chapter;

    #[tokio::test]
    async fn writes_chapters_and_info() {
        let dir = tempfile::tempdir().unwrap();
        let media = dir.path().join("video.mp4");
        tokio::fs::write(&media, b"fake").await.unwrap();
        let chapters = vec![Chapter {
            title: "Intro".into(),
            start_secs: 0.0,
            end_secs: 10.0,
        }];
        let path = write_chapters_txt(&media, &chapters).await.unwrap();
        let text = tokio::fs::read_to_string(path).await.unwrap();
        assert!(text.contains("Intro"));
        assert!(text.contains("START=0"));
    }
}
