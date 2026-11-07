//! yt-dlp-compatible `info.json` export from pure-Rust `ExtractResult`.
//!
//! Field names follow yt-dlp conventions so tools that consume `--write-info-json`
//! can interoperate without the Python binary.

use ferric_common::{ExtractResult, ExtractedFormat, SubtitleFormat, SubtitleTrack};
use serde_json::{json, Map, Value};

/// Serialize to a yt-dlp-shaped JSON object (not identical to every yt-dlp version,
/// but covers the common consumer fields).
pub fn to_ytdlp_info(result: &ExtractResult) -> Value {
    let mut formats = Vec::new();
    for f in &result.formats {
        formats.push(format_to_json(f));
    }

    let mut subtitles = Map::new();
    let mut automatic_captions = Map::new();
    for t in &result.subtitles {
        let entry = json!({
            "url": t.url,
            "ext": subtitle_ext(t),
            "name": t.name,
        });
        let map = if t.is_auto_generated {
            &mut automatic_captions
        } else {
            &mut subtitles
        };
        map.entry(t.lang.clone())
            .or_insert_with(|| Value::Array(Vec::new()))
            .as_array_mut()
            .map(|a| a.push(entry));
    }

    let chapters: Vec<Value> = result
        .chapters
        .iter()
        .map(|c| {
            json!({
                "title": c.title,
                "start_time": c.start_secs,
                "end_time": c.end_secs,
            })
        })
        .collect();

    let entries: Vec<Value> = result
        .playlist_entries
        .iter()
        .map(|e| {
            json!({
                "url": e.url,
                "title": e.title,
                "id": e.index.to_string(),
                "duration": e.duration_secs,
                "thumbnail": e.thumbnail,
                "_type": "url",
            })
        })
        .collect();

    let is_playlist = !result.playlist_entries.is_empty();
    let extractor_key = result
        .extractor
        .strip_prefix("yt-dlp:")
        .unwrap_or(&result.extractor);

    let mut root = json!({
        "id": result.formats.first().map(|f| f.id.clone()).unwrap_or_else(|| "unknown".into()),
        "title": result.title,
        "fulltitle": result.title,
        "uploader": result.uploader,
        "channel": result.uploader,
        "thumbnail": result.thumbnail,
        "duration": result.duration_secs,
        "description": result.description,
        "upload_date": result.upload_date,
        "tags": result.tags,
        "extractor": extractor_key,
        "extractor_key": extractor_key,
        "formats": formats,
        "subtitles": Value::Object(subtitles),
        "automatic_captions": Value::Object(automatic_captions),
        "chapters": chapters,
        "_filename": sanitize_filename(&result.title),
        "webpage_url_basename": null,
        "epoch": null,
        // Ferric provenance
        "_ferric": {
            "pure_rust": !result.extractor.starts_with("yt-dlp:"),
            "extractor": result.extractor,
        }
    });

    if is_playlist {
        root["_type"] = json!("playlist");
        root["entries"] = Value::Array(entries);
        root["playlist_count"] = json!(result.playlist_entries.len());
    } else {
        root["_type"] = json!("video");
    }

    root
}

pub fn to_ytdlp_info_string(result: &ExtractResult) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&to_ytdlp_info(result))
}

fn format_to_json(f: &ExtractedFormat) -> Value {
    let mut http_headers = Map::new();
    for (k, v) in &f.headers {
        http_headers.insert(k.clone(), json!(v));
    }
    json!({
        "format_id": f.id,
        "url": f.url,
        "ext": f.ext,
        "width": f.width,
        "height": f.height,
        "tbr": f.tbr_kbps,
        "abr": f.abr_kbps,
        "vbr": f.vbr_kbps,
        "fps": f.fps,
        "vcodec": f.vcodec.as_deref().unwrap_or("none"),
        "acodec": f.acodec.as_deref().unwrap_or("none"),
        "protocol": f.protocol,
        "filesize": f.filesize,
        "format_note": f.format_note,
        "format": f.label,
        "language": f.language,
        "container": f.container,
        "manifest_url": f.manifest_url,
        "http_headers": Value::Object(http_headers),
        "audio_ext": if f.is_audio_only { json!(f.ext) } else { json!("none") },
        "video_ext": if f.is_audio_only { json!("none") } else { json!(f.ext) },
    })
}

fn subtitle_ext(t: &SubtitleTrack) -> &'static str {
    match t.format {
        SubtitleFormat::Vtt => "vtt",
        SubtitleFormat::Srt => "srt",
        SubtitleFormat::Ttml => "ttml",
        SubtitleFormat::Json3 => "json3",
    }
}

fn sanitize_filename(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let s = s.trim().trim_matches('.');
    if s.is_empty() {
        "download".into()
    } else {
        s.chars().take(180).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferric_common::ExtractedFormat;

    #[test]
    fn exports_video_shape() {
        let r = ExtractResult {
            title: "Hello: World?".into(),
            uploader: Some("U".into()),
            thumbnail: None,
            duration_secs: Some(10.0),
            formats: vec![ExtractedFormat {
                id: "18".into(),
                label: "360p".into(),
                url: "https://x/v.mp4".into(),
                ext: "mp4".into(),
                protocol: Some("https".into()),
                manifest_url: None,
                format_note: None,
                width: Some(640),
                height: Some(360),
                tbr_kbps: Some(500.0),
                abr_kbps: None,
                vbr_kbps: None,
                fps: None,
                vcodec: Some("avc1".into()),
                acodec: Some("mp4a".into()),
                container: Some("mp4".into()),
                language: None,
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: false,
            }],
            extractor: "youtube".into(),
            description: Some("d".into()),
            upload_date: Some("20240101".into()),
            tags: vec!["a".into()],
            subtitles: Vec::new(),
            playlist_entries: Vec::new(),
            chapters: Vec::new(),
        };
        let v = to_ytdlp_info(&r);
        assert_eq!(v["_type"], "video");
        assert_eq!(v["title"], "Hello: World?");
        assert_eq!(v["_filename"], "Hello_ World_");
        assert_eq!(v["formats"][0]["height"], 360);
        assert_eq!(v["_ferric"]["pure_rust"], true);
    }
}
