//! Pure functions deciding whether a UI action is currently enabled, given a
//! transfer's status. No cxx-qt/QML dependency, so these are unit-testable
//! without a Qt build — `CommandBus` (`commands.rs`) will call these once
//! real actions land (FQT-021 downloads, FQT-032 torrents).

use ferric_common::models::DownloadStatus;
use ferric_common::torrent::TorrentStatus;

pub fn can_pause_download(status: DownloadStatus) -> bool {
    matches!(
        status,
        DownloadStatus::Queued | DownloadStatus::Connecting | DownloadStatus::Downloading
    )
}

pub fn can_resume_download(status: DownloadStatus) -> bool {
    matches!(status, DownloadStatus::Paused | DownloadStatus::Error)
}

pub fn can_pause_torrent(status: TorrentStatus) -> bool {
    matches!(status, TorrentStatus::Downloading | TorrentStatus::Checking)
}

pub fn can_resume_torrent(status: TorrentStatus) -> bool {
    matches!(status, TorrentStatus::Paused | TorrentStatus::EngineGated)
}

pub fn can_stop_download(status: ferric_common::models::DownloadStatus) -> bool {
    use ferric_common::models::DownloadStatus;
    matches!(
        status,
        DownloadStatus::Queued
            | DownloadStatus::Connecting
            | DownloadStatus::Downloading
            | DownloadStatus::Paused
            | DownloadStatus::Error
    )
}

pub fn can_delete_download(_status: ferric_common::models::DownloadStatus) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FQT-120: Table-driven enablement matrix tests ──────────────────

    // Download states × commands
    #[test]
    fn download_enablement_matrix() {
        // (status, can_pause, can_resume, can_stop, can_delete)
        let matrix: &[(DownloadStatus, bool, bool, bool, bool)] = &[
            (DownloadStatus::Queued, true, false, true, true),
            (DownloadStatus::Connecting, true, false, true, true),
            (DownloadStatus::Downloading, true, false, true, true),
            (DownloadStatus::Merging, false, false, false, true),
            (DownloadStatus::Paused, false, true, true, true),
            (DownloadStatus::Completed, false, false, false, true),
            (DownloadStatus::Error, false, true, true, true),
            (DownloadStatus::Cancelled, false, false, false, true),
        ];

        for &(status, exp_pause, exp_resume, exp_stop, exp_delete) in matrix {
            assert_eq!(
                can_pause_download(status),
                exp_pause,
                "can_pause_download({status:?}) expected {exp_pause}"
            );
            assert_eq!(
                can_resume_download(status),
                exp_resume,
                "can_resume_download({status:?}) expected {exp_resume}"
            );
            assert_eq!(
                can_stop_download(status),
                exp_stop,
                "can_stop_download({status:?}) expected {exp_stop}"
            );
            assert_eq!(
                can_delete_download(status),
                exp_delete,
                "can_delete_download({status:?}) expected {exp_delete}"
            );
        }
    }

    // Torrent states × commands
    #[test]
    fn torrent_enablement_matrix() {
        // (status, can_pause, can_resume)
        let matrix: &[(TorrentStatus, bool, bool)] = &[
            (TorrentStatus::Staged, false, false),
            (TorrentStatus::MetadataReady, false, false),
            (TorrentStatus::EngineGated, false, true),
            (TorrentStatus::Checking, true, false),
            (TorrentStatus::Downloading, true, false),
            (TorrentStatus::Paused, false, true),
            (TorrentStatus::Completed, false, false),
            (TorrentStatus::Error, false, false),
        ];

        for &(ref status, exp_pause, exp_resume) in matrix {
            assert_eq!(
                can_pause_torrent(status.clone()),
                exp_pause,
                "can_pause_torrent({status:?}) expected {exp_pause}"
            );
            assert_eq!(
                can_resume_torrent(status.clone()),
                exp_resume,
                "can_resume_torrent({status:?}) expected {exp_resume}"
            );
        }
    }

    // Edge: every status is covered (compile-time exhaustive via match)
    #[test]
    fn all_download_statuses_covered() {
        for s in [
            DownloadStatus::Queued,
            DownloadStatus::Connecting,
            DownloadStatus::Downloading,
            DownloadStatus::Merging,
            DownloadStatus::Paused,
            DownloadStatus::Completed,
            DownloadStatus::Error,
            DownloadStatus::Cancelled,
        ] {
            let _ = can_pause_download(s);
            let _ = can_resume_download(s);
            let _ = can_stop_download(s);
            let _ = can_delete_download(s);
        }
    }

    #[test]
    fn all_torrent_statuses_covered() {
        for s in [
            TorrentStatus::Staged,
            TorrentStatus::MetadataReady,
            TorrentStatus::EngineGated,
            TorrentStatus::Checking,
            TorrentStatus::Downloading,
            TorrentStatus::Paused,
            TorrentStatus::Completed,
            TorrentStatus::Error,
        ] {
            let _ = can_pause_torrent(s.clone());
            let _ = can_resume_torrent(s);
        }
    }
}
