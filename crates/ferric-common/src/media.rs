use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    DirectFile,
    HlsManifest,
    DashManifest,
    MediaSegment,
    Unsupported,
    ProtectedOrExpired,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaVariant {
    pub id: String,
    pub label: String,
    pub url: String,
    pub bandwidth: Option<u64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub codecs: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaTrack {
    pub id: String,
    pub label: String,
    pub url: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaProbeResponse {
    pub url: String,
    pub kind: MediaKind,
    pub mime_type: Option<String>,
    pub supported: bool,
    pub variants: Vec<MediaVariant>,
    pub audio_tracks: Vec<MediaTrack>,
    pub subtitle_tracks: Vec<MediaTrack>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FfmpegStatus {
    pub available: bool,
    pub path: Option<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MediaCandidate {
    pub id: String,
    pub url: String,
    pub page_url: Option<String>,
    pub mime_type: Option<String>,
    pub kind: MediaKind,
    pub observed_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractedFormat {
    pub id: String,
    pub label: String,
    pub url: String,
    pub ext: String,
    pub protocol: Option<String>,
    pub manifest_url: Option<String>,
    pub format_note: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub tbr_kbps: Option<f64>,
    pub abr_kbps: Option<f64>,
    pub vbr_kbps: Option<f64>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub acodec: Option<String>,
    pub container: Option<String>,
    pub language: Option<String>,
    pub filesize: Option<u64>,
    pub has_drm: bool,
    pub headers: Vec<(String, String)>,
    pub is_audio_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtractResult {
    pub title: String,
    pub uploader: Option<String>,
    pub thumbnail: Option<String>,
    pub duration_secs: Option<f64>,
    pub formats: Vec<ExtractedFormat>,
    pub extractor: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub upload_date: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub subtitles: Vec<SubtitleTrack>,
    #[serde(default)]
    pub playlist_entries: Vec<PlaylistEntry>,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubtitleFormat {
    Vtt,
    Srt,
    Ttml,
    Json3,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleTrack {
    pub lang: String,
    pub name: Option<String>,
    pub url: String,
    pub format: SubtitleFormat,
    pub is_auto_generated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PlaylistEntry {
    pub url: String,
    pub title: Option<String>,
    pub index: u32,
    pub duration_secs: Option<f64>,
    pub thumbnail: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Chapter {
    pub title: String,
    pub start_secs: f64,
    pub end_secs: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MergeState {
    Queued,
    Merging,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergeJobRecord {
    pub id: String,
    pub video_path: String,
    pub audio_path: String,
    pub output_path: String,
    pub state: MergeState,
    pub message: Option<String>,
    pub created_at: u64,
    pub completed_at: Option<u64>,
}

#[cfg(test)]
mod extract_tests {
    use super::*;

    #[test]
    fn extract_result_roundtrips_json() {
        let result = ExtractResult {
            title: "Test Video".to_string(),
            uploader: Some("TestUser".to_string()),
            thumbnail: None,
            duration_secs: Some(120.0),
            formats: vec![ExtractedFormat {
                id: "hd".to_string(),
                label: "1080p".to_string(),
                url: "https://example.com/video.mp4".to_string(),
                ext: "mp4".to_string(),
                protocol: Some("https".to_string()),
                manifest_url: None,
                format_note: None,
                width: Some(1920),
                height: Some(1080),
                tbr_kbps: Some(4200.0),
                abr_kbps: None,
                vbr_kbps: None,
                fps: None,
                vcodec: Some("avc1".to_string()),
                acodec: Some("mp4a".to_string()),
                container: Some("mp4".to_string()),
                language: None,
                filesize: None,
                has_drm: false,
                headers: Vec::new(),
                is_audio_only: false,
            }],
            extractor: "test".to_string(),
            description: None,
            upload_date: None,
            tags: Vec::new(),
            subtitles: Vec::new(),
            playlist_entries: Vec::new(),
            chapters: Vec::new(),
        };

        let json = serde_json::to_string(&result).expect("serialize ExtractResult");
        let back: ExtractResult = serde_json::from_str(&json).expect("deserialize ExtractResult");

        assert_eq!(result, back);
        assert_eq!(back.formats[0].height, Some(1080));
    }
}
