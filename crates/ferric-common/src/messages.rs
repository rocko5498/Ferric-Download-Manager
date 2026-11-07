use crate::media::MediaCandidate;
use crate::models::{DownloadRecord, DownloadStatus, SiteRule};
use crate::torrent::TorrentRecord;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

// ── Extension → Host → Daemon ─────────────────────────────────────────────────

/// Sent by Chrome extension when it captures a download.
/// JSON-encoded over Chrome native messaging (extension↔host).
/// Postcard-encoded inside `HostMessage::DownloadReq` over IPC (host↔daemon).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRequest {
    pub id: Uuid,
    pub url: String,
    /// Final URL after Chrome followed redirects (may equal `url`)
    pub final_url: Option<String>,
    pub filename: Option<String>,
    pub referrer: Option<String>,
    /// Cookie string: "name=value; name2=value2"
    pub cookies: Option<String>,
    /// Extra HTTP headers to forward (e.g., Authorization)
    pub headers: Option<HashMap<String, String>>,
    /// Chrome's reported file size (-1 if unknown)
    pub size_hint: Option<u64>,
    pub mime_type: Option<String>,
    /// "GET" or "POST"
    pub method: String,
    /// Base64-encoded POST body (Power Mode only, usually None)
    pub post_body_b64: Option<String>,
    pub queue_id: Option<Uuid>,
    /// Optional expected hash: "sha256:<hex>" or "md5:<hex>".
    #[serde(default)]
    pub checksum: Option<String>,
    /// Alternate source URLs for the same file (multi-source / mirrors).
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// Unix milliseconds — used to detect expired signed URLs
    pub timestamp: u64,
}

// ── Host → Daemon (IPC, postcard-encoded) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum HostMessage {
    DownloadReq(DownloadRequest),
    MediaCandidate(MediaCandidate),
    Ping,
    /// Request the daemon's current capture policy settings.
    GetCapturePolicy,
    /// Push updated capture policy settings to the daemon.
    SetCapturePolicy {
        settings: std::collections::HashMap<String, String>,
    },
    /// List downloadable formats for a page URL (YouTube etc.); protected formats are never listed.
    ExtractFormats {
        url: String,
    },
    /// Download the video on a page: best quality, or the chosen `format_id`.
    PageDownload {
        url: String,
        format_id: Option<String>,
        cookies: Option<String>,
        referrer: Option<String>,
        /// The page's title; names the file when the address is just a playlist.
        #[serde(default)]
        title: Option<String>,
    },
}

// ── Daemon → Host (IPC, postcard-encoded) ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonMessage {
    Reply(DaemonReply),
    /// Sent once on connect so host can cache site rules for `shouldCapture()`
    SiteRules(Vec<SiteRule>),
    Pong {
        version: String,
    },
    /// Response to `GetCapturePolicy`: flat key→value settings map for all
    /// capture-related keys (`capture_*` and `min_capture_size`).
    CapturePolicy {
        settings: std::collections::HashMap<String, String>,
    },
    /// Generic success acknowledgement (e.g. for `SetCapturePolicy`).
    Ack,
    /// Response to `ExtractFormats`: JSON text describing the page's formats.
    Formats {
        json: String,
    },
}

/// Response to a `DownloadRequest`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonReply {
    Accepted { id: Uuid },
    Error { id: Uuid, message: String },
}

// ── UI → Daemon (HTTP POST bodies, JSON) ─────────────────────────────────────

// ── Daemon → UI (SSE stream, JSON) ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum ProgressEvent {
    /// Emitted when a download is first added to the list
    Added { record: DownloadRecord },

    /// Emitted when a browser capture is held, waiting for the Download File Info dialog.
    CaptureHeld { record: DownloadRecord },

    /// A second copy of Ferric was started (or the user asked): bring the UI window forward.
    ShowUi,

    /// The probe of a held capture finished: name, size, type, category and folder were filled in.
    CaptureUpdated { record: DownloadRecord },

    /// Emitted periodically during download (~4Hz)
    Progress {
        id: Uuid,
        downloaded: u64,
        total: Option<u64>,
        speed_bps: u64,
        eta_secs: Option<u64>,
        status: DownloadStatus,
    },

    /// Emitted when a download completes successfully
    Done {
        id: Uuid,
        path: String,
        checksum_ok: Option<bool>,
    },

    /// Emitted when a download fails permanently (no more retries)
    Failed { id: Uuid, message: String },

    /// Emitted after a download is removed from the list
    Removed { id: Uuid },

    /// Emitted when the browser observes a media URL worth showing in the UI
    MediaCandidate { candidate: MediaCandidate },

    /// Emitted after torrent engine stats are synced.
    TorrentUpdated { record: TorrentRecord },
}

// ── Wire helpers ──────────────────────────────────────────────────────────────

/// Encode a message with a 4-byte LE length prefix (for Chrome native messaging
/// and for the postcard IPC frames).
#[must_use]
pub fn length_prefix_encode(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let mut out = Vec::with_capacity(4 + data.len());
    out.extend_from_slice(&len.to_le_bytes());
    out.extend_from_slice(data);
    out
}

/// Decode one framed message from a byte slice, returning (payload, rest).
#[must_use]
pub fn length_prefix_decode(buf: &[u8]) -> Option<(&[u8], &[u8])> {
    if buf.len() < 4 {
        return None;
    }
    let len = u32::from_le_bytes(buf[..4].try_into().ok()?) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    Some((&buf[4..4 + len], &buf[4 + len..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_host_message() {
        let req = DownloadRequest {
            id: Uuid::new_v4(),
            url: "https://example.com/file.zip".into(),
            final_url: None,
            filename: Some("file.zip".into()),
            referrer: None,
            cookies: Some("session=abc".into()),
            headers: None,
            size_hint: Some(1024),
            mime_type: Some("application/zip".into()),
            method: "GET".into(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1_000_000,
        };
        let msg = HostMessage::DownloadReq(req);
        let encoded = postcard::to_stdvec(&msg).unwrap();
        let decoded: HostMessage = postcard::from_bytes(&encoded).unwrap();
        matches!(decoded, HostMessage::DownloadReq(_));
    }

    #[test]
    fn page_messages_parse_extension_json_and_round_trip() {
        let json = r#"{"PageDownload":{"url":"https://www.youtube.com/watch?v=x","format_id":null,"cookies":"a=b","referrer":null}}"#;
        let msg: HostMessage = serde_json::from_str(json).unwrap();
        let back: HostMessage = postcard::from_bytes(&postcard::to_stdvec(&msg).unwrap()).unwrap();
        assert!(matches!(
            back,
            HostMessage::PageDownload { ref url, format_id: None, .. } if url.contains("youtube")
        ));
        let list: HostMessage =
            serde_json::from_str(r#"{"ExtractFormats":{"url":"https://example.com/v"}}"#).unwrap();
        assert!(matches!(list, HostMessage::ExtractFormats { .. }));
    }

    #[test]
    fn length_prefix_round_trip() {
        let data = b"hello world";
        let framed = length_prefix_encode(data);
        let (payload, rest) = length_prefix_decode(&framed).unwrap();
        assert_eq!(payload, data);
        assert!(rest.is_empty());
    }

    #[test]
    fn show_ui_event_serializes_under_the_name_the_ui_matches_on() {
        let json = serde_json::to_string(&ProgressEvent::ShowUi).unwrap();
        assert_eq!(json, r#"{"event":"show_ui"}"#);
    }
}
