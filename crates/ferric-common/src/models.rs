use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

// ── Download status ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadStatus {
    Queued,
    Connecting,
    Downloading,
    Merging,
    Paused,
    Completed,
    Error,
    Cancelled,
}

impl fmt::Display for DownloadStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Queued => "queued",
            Self::Connecting => "connecting",
            Self::Downloading => "downloading",
            Self::Merging => "merging",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Error => "error",
            Self::Cancelled => "cancelled",
        };
        f.write_str(s)
    }
}

impl std::str::FromStr for DownloadStatus {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "queued" => Ok(Self::Queued),
            "connecting" => Ok(Self::Connecting),
            "downloading" => Ok(Self::Downloading),
            "merging" => Ok(Self::Merging),
            "paused" => Ok(Self::Paused),
            "completed" => Ok(Self::Completed),
            "error" => Ok(Self::Error),
            "cancelled" => Ok(Self::Cancelled),
            other => Err(format!("unknown status: {other}")),
        }
    }
}

// ── Chunk status ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    Pending,
    Downloading,
    Completed,
    Error,
}

impl fmt::Display for ChunkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Self::Pending => "pending",
            Self::Downloading => "downloading",
            Self::Completed => "completed",
            Self::Error => "error",
        };
        f.write_str(s)
    }
}

// ── Download record ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadRecord {
    pub id: Uuid,
    pub url: String,
    pub final_url: Option<String>,
    pub redirect_chain: Option<String>,
    pub filename: String,
    pub save_path: String,
    pub status: DownloadStatus,
    /// `None` if Content-Length was not provided
    pub total_size: Option<u64>,
    pub downloaded: u64,
    /// Unix milliseconds
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub completed_at: Option<i64>,
    pub error_msg: Option<String>,
    pub error_count: u32,
    pub referrer: Option<String>,
    pub mime_type: Option<String>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub category: String,
    pub queue_id: Option<Uuid>,
    pub priority: i32,
    /// e.g. "sha256:abc123..." if server provided a checksum
    pub checksum: Option<String>,
    /// `None` = not checked, `true` = ok, `false` = mismatch
    pub checksum_ok: Option<bool>,
    /// Per-download speed cap in bytes/sec; 0 = unlimited
    pub speed_limit_bps: u64,
    pub audio_url: Option<String>,
    pub subtitle_url: Option<String>,
    pub subtitle_lang: Option<String>,
    pub proxy_url: Option<String>,
    /// Alternate source URLs (same file); JSON in DB, empty = primary only.
    #[serde(default)]
    pub mirrors: Vec<String>,
    /// User-supplied free-text note, set via the Add URL flow or Properties dialog.
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DownloadDiagnostic {
    pub code: String,
    pub severity: DiagnosticSeverity,
    pub title: String,
    pub summary: String,
    pub action: Option<String>,
    pub confidence: u8,
}

// ── Chunk record ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkRecord {
    pub id: Uuid,
    pub download_id: Uuid,
    pub chunk_index: u32,
    pub byte_start: u64,
    pub byte_end: u64,
    pub downloaded: u64,
    pub status: ChunkStatus,
    pub error_count: u32,
}

impl ChunkRecord {
    #[must_use]
    pub fn resume_offset(&self) -> u64 {
        self.byte_start + self.downloaded
    }

    #[must_use]
    pub fn remaining(&self) -> u64 {
        (self.byte_end - self.byte_start + 1).saturating_sub(self.downloaded)
    }

    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.downloaded > self.byte_end - self.byte_start
    }
}

// ── Queue record ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueRecord {
    pub id: Uuid,
    pub name: String,
    pub max_concurrent: u32,
    /// Bytes per second, `None` = unlimited
    pub max_bandwidth: Option<u64>,
    /// Minutes from midnight (0–1439), `None` = always active
    pub sched_start: Option<u16>,
    pub sched_end: Option<u16>,
    /// Days the queue may run: bit 0 = Sunday ... bit 6 = Saturday (127 = every day).
    #[serde(default = "all_days")]
    pub sched_days: u8,
    /// IDM's "Start download on IDM startup": the daemon starts this queue when it boots.
    #[serde(default)]
    pub start_on_startup: bool,
    /// IDM's "Periodic synchronization": re-check the queue's finished files every N minutes.
    #[serde(default)]
    pub sync_interval_min: Option<u32>,
    /// Retries per file for this queue; `None` = the global `max_retries` preference.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// File to open when the queue has finished.
    #[serde(default)]
    pub open_file: Option<String>,
    /// "exit" or "shutdown" once the queue has finished; `None` = nothing.
    #[serde(default)]
    pub when_done: Option<String>,
    pub enabled: bool,
    pub priority: i32,
}

fn all_days() -> u8 {
    127
}

// ── Site rule ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteRule {
    pub id: Uuid,
    /// Glob pattern, e.g. "*.example.com/*"
    pub pattern: String,
    pub rule_type: SiteRuleType,
    pub value: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SiteRuleType {
    Capture,
    Skip,
    MaxConnections,
    MaxSpeedBps,
    /// Alias for `MaxSpeedBps` (roadmap Task 100 naming).
    BandwidthCapBytes,
    AlwaysAsk,
}

// ── Settings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub max_connections_global: u32,
    pub max_connections_per_host: u32,
    /// 0 = unlimited
    pub bandwidth_limit_bps: u64,
    pub default_save_dir: String,
    /// Bytes — files smaller than this are NOT captured
    pub min_capture_size: u64,
    pub auto_start: bool,
    pub theme: String,
    pub default_queue_id: Option<Uuid>,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
}

fn default_true() -> bool {
    true
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_connections_global: 128,
            max_connections_per_host: 8,
            bandwidth_limit_bps: 0,
            default_save_dir: String::new(),
            min_capture_size: 1_048_576,
            auto_start: true,
            theme: "system".into(),
            default_queue_id: None,
            notifications_enabled: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TorrentRecord;

    /// Invariant #5: old-shape JSON (missing fields added later) must deserialize
    /// via #[serde(default)] without crashing.
    #[test]
    fn download_record_deserializes_minimal_json() {
        // Required fields only; fields with #[serde(default)] omitted to test defaults
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440000",
            "url": "https://example.com/file.zip",
            "filename": "file.zip",
            "save_path": "/downloads/file.zip",
            "status": "completed",
            "downloaded": 1024,
            "created_at": 1000,
            "error_count": 0,
            "category": "",
            "priority": 0,
            "speed_limit_bps": 0
        }"#;
        let record: DownloadRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.filename, "file.zip");
        // Fields with serde(default) are absent — verify they get defaults
        assert!(record.mirrors.is_empty());
        assert!(record.description.is_none());
    }

    #[test]
    fn torrent_record_deserializes_minimal_json() {
        // Only original schema fields — newer fields use #[serde(default)]
        let json = r#"{
            "id": "550e8400-e29b-41d4-a716-446655440001",
            "info_hash": "abc123def456",
            "name": "Test Torrent",
            "source": "magnet:?xt=urn:btih:abc123def456",
            "source_kind": "magnet",
            "status": "engine_gated",
            "total_size": 2048,
            "downloaded": 0,
            "uploaded": 0,
            "ratio": 0.0,
            "is_private": false,
            "peer_id_seed": "seed123",
            "engine": "rqbit",
            "created_at": 1000
        }"#;
        let record: TorrentRecord = serde_json::from_str(json).unwrap();
        assert_eq!(record.name, "Test Torrent");
        assert!(record.tags.is_empty());
        assert!(record.save_root.is_none());
    }

    #[test]
    fn download_record_roundtrip() {
        let record = DownloadRecord {
            id: Uuid::new_v4(),
            url: "https://example.com/test.bin".into(),
            final_url: None,
            redirect_chain: None,
            filename: "test.bin".into(),
            save_path: "/dl/test.bin".into(),
            status: DownloadStatus::Downloading,
            total_size: Some(4096),
            downloaded: 1024,
            created_at: 2000,
            started_at: None,
            completed_at: None,
            error_msg: None,
            error_count: 0,
            referrer: None,
            mime_type: None,
            etag: None,
            last_modified: None,
            category: String::new(),
            queue_id: None,
            priority: 0,
            checksum: None,
            checksum_ok: None,
            speed_limit_bps: 0,
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            mirrors: Vec::new(),
            description: None,
        };
        let json = serde_json::to_string(&record).unwrap();
        let roundtripped: DownloadRecord = serde_json::from_str(&json).unwrap();
        assert_eq!(roundtripped.id, record.id);
        assert_eq!(roundtripped.status, record.status);
    }
}
