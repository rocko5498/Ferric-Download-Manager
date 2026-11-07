pub mod error;
pub mod media;
pub mod messages;
pub mod metalink;
pub mod models;
pub mod torrent;

pub const APP_QUALIFIER: &str = "com";
pub const APP_ORGANIZATION: &str = "ferric";
pub const APP_NAME: &str = "ferric";
pub const API_LOCK_FILE: &str = "ferric.lock";

/// Resolve the per-user (or portable) data directory.
///
/// If a file named `portable` sits next to the current executable, use `./data`
/// beside that executable so daemon + UI share the same lockfile/DB.
pub fn resolve_data_dir() -> Result<std::path::PathBuf, String> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            if exe_dir.join("portable").exists() {
                return Ok(exe_dir.join("data"));
            }
        }
    }
    directories::ProjectDirs::from(APP_QUALIFIER, APP_ORGANIZATION, APP_NAME)
        .map(|d| d.data_local_dir().to_path_buf())
        .ok_or_else(|| "could not determine data directory".into())
}

pub use error::FerricError;
pub use media::{
    Chapter, ExtractResult, ExtractedFormat, FfmpegStatus, MediaCandidate, MediaKind,
    MediaProbeResponse, MediaTrack, MediaVariant, MergeJobRecord, MergeState, PlaylistEntry,
    SubtitleFormat, SubtitleTrack,
};
pub use messages::{DaemonMessage, DaemonReply, HostMessage, ProgressEvent};
pub use metalink::{is_metalink_url, parse as parse_metalink, MetalinkError, MetalinkFile};
pub use models::{
    ChunkRecord, ChunkStatus, DiagnosticSeverity, DownloadDiagnostic, DownloadRecord,
    DownloadStatus, QueueRecord, SiteRule, SiteRuleType,
};
pub use torrent::{
    LivePeerView, PrivacyCheckResult, ProxyConfigInput, ProxyConfigRecord, ProxyKind,
    TorrentAddRequest, TorrentDetail, TorrentEngineStatus, TorrentFileRecord, TorrentInputKind,
    TorrentPeerRecord, TorrentPieceRecord, TorrentPolicy, TorrentProbeFile, TorrentProbeResponse,
    TorrentRecord, TorrentStageRequest, TorrentStatus, TorrentTrackerRecord,
};
