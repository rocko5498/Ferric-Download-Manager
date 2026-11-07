use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TorrentInputKind {
    Magnet,
    TorrentFile,
    Unsupported,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentProbeResponse {
    pub input: String,
    pub kind: TorrentInputKind,
    pub supported: bool,
    pub name: Option<String>,
    pub info_hash: Option<String>,
    pub trackers: Vec<String>,
    #[serde(default)]
    pub files: Vec<TorrentProbeFile>,
    pub file_count: Option<u64>,
    pub total_size: Option<u64>,
    pub is_private: bool,
    pub warnings: Vec<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentProbeFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentAddRequest {
    pub magnet_uri: Option<String>,
    pub torrent_path: Option<String>,
    pub save_dir: Option<String>,
    pub start_paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TorrentStatus {
    Staged,
    MetadataReady,
    EngineGated,
    /// rqbit `initializing` / piece check — not the same as Downloading.
    Checking,
    Downloading,
    Paused,
    Completed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TorrentPolicy {
    pub dht_enabled: bool,
    pub pex_enabled: bool,
    pub lsd_enabled: bool,
    pub seed_enabled: bool,
    pub seed_ratio_limit: Option<f64>,
    pub seed_time_limit_min: Option<u64>,
    pub upload_limit_bps: Option<u64>,
    pub proxy_required: bool,
    pub blocklist_enabled: bool,
    pub mse_required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProxyKind {
    Socks5,
    Http,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyConfigRecord {
    pub id: String,
    pub name: String,
    pub proxy_type: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub has_password: bool,
    pub created_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProxyConfigInput {
    pub name: String,
    pub proxy_type: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl Default for TorrentPolicy {
    fn default() -> Self {
        Self {
            dht_enabled: false,
            pex_enabled: false,
            lsd_enabled: false,
            seed_enabled: false,
            seed_ratio_limit: Some(0.0),
            seed_time_limit_min: Some(0),
            upload_limit_bps: None,
            proxy_required: false,
            blocklist_enabled: false,
            mse_required: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TorrentRecord {
    pub id: String,
    pub info_hash: String,
    pub name: String,
    pub source: String,
    pub source_kind: TorrentInputKind,
    pub save_root: Option<String>,
    pub status: TorrentStatus,
    pub total_size: u64,
    pub downloaded: u64,
    pub uploaded: u64,
    pub ratio: f64,
    /// Ephemeral live speed from last engine poll (not persisted to SQLite).
    #[serde(default)]
    pub download_speed: u64,
    /// Ephemeral live speed from last engine poll (not persisted to SQLite).
    #[serde(default)]
    pub upload_speed: u64,
    /// Ephemeral connected peer count from last engine poll (not persisted).
    #[serde(default)]
    pub peers: u64,
    #[serde(default)]
    pub policy: TorrentPolicy,
    pub proxy_id: Option<String>,
    #[serde(default)]
    pub policy_locked_fields: Vec<String>,
    pub is_private: bool,
    pub peer_id_seed: String,
    pub engine: String,
    pub created_at: u64,
    pub completed_at: Option<u64>,
    pub av_status: Option<String>,
    pub av_result: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Higher value = higher priority (list ORDER BY `queue_position` DESC). Set via queue API.
    #[serde(default)]
    pub queue_position: i32,
    /// Assigned category name (shared with Downloads' category list).
    #[serde(default)]
    pub category: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PrivacyCheckResult {
    pub passed: bool,
    pub violations: Vec<String>,
    /// Non-blocking exposure notices. Unlike `violations` these never gate
    /// engine start, but they must always be surfaced so a "passed" result is
    /// never read as "anonymous".
    #[serde(default)]
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentEngineStatus {
    pub state: String,
    pub endpoint: Option<String>,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_error: Option<String>,
    pub explanation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentFileRecord {
    pub id: String,
    pub torrent_id: String,
    pub path: String,
    pub size: u64,
    pub selected: bool,
    pub priority: i32,
    pub downloaded: u64,
    pub engine_index: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentTrackerRecord {
    pub id: String,
    pub torrent_id: String,
    pub url: String,
    pub status: String,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentPeerRecord {
    pub id: String,
    pub torrent_id: String,
    pub addr_hash: String,
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub state: String,
    /// Task 182 — ISO country when `GeoIP` available
    #[serde(default)]
    pub country_iso: Option<String>,
    #[serde(default)]
    pub country_name: Option<String>,
    #[serde(default)]
    pub flag_emoji: Option<String>,
    /// five | nine | fourteen | other | local | unknown
    #[serde(default)]
    pub eyes_tier: Option<String>,
}

/// Live peer snapshot with optional geo (Task 182).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LivePeerView {
    pub addr_hash: String,
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub interested: bool,
    pub choked: bool,
    pub country_iso: String,
    pub country_name: String,
    pub flag_emoji: String,
    pub eyes_tier: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TorrentPieceRecord {
    pub id: String,
    pub torrent_id: String,
    pub piece_index: u64,
    pub size: u64,
    pub status: String,
    pub availability: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TorrentDetail {
    pub record: TorrentRecord,
    pub files: Vec<TorrentFileRecord>,
    pub trackers: Vec<TorrentTrackerRecord>,
    pub peers: Vec<TorrentPeerRecord>,
    pub pieces: Vec<TorrentPieceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TorrentStageRequest {
    pub probe: TorrentProbeResponse,
    pub save_root: Option<String>,
    pub policy: TorrentPolicy,
}
