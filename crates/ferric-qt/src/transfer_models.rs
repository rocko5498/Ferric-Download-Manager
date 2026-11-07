//! `DownloadModel` (full §6.3 P0 columns + stable `id` role for selection)
//! and `TorrentModel` (name+status only — full columns are FQT-030).
//! Populated from `GET /downloads` and `GET /torrents`.

use crate::daemon_session::read_lock_file;

fn format_bps(n: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    let n = n as f64;
    if n >= MIB {
        format!("{:.1} MB/s", n / MIB)
    } else if n >= KIB {
        format!("{:.0} KB/s", n / KIB)
    } else {
        format!("{n:.0} B/s")
    }
}

fn format_eta(secs: u64) -> String {
    if secs == 0 {
        return "-".to_string();
    }
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}h {m:02}m")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

fn format_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut val = n as f64;
    let mut unit = 0usize;
    while val >= 1024.0 && unit < UNITS.len() - 1 {
        val /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{val:.1} {}", UNITS[unit])
    }
}

fn format_epoch_ms(ms: i64) -> String {
    if ms <= 0 {
        return String::new();
    }
    // Local wall-clock time (was UTC, 5.5 h off in India).
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| {
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        })
        .unwrap_or_default()
}

fn url_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .unwrap_or(url)
        .to_string()
}

struct DownloadRow {
    id: String,
    name: String,
    status: String,
    size: String,
    done: String,
    progress: String,
    queue: String,
    category: String,
    added: String,
    completed: String,
    save_path: String,
    url_host: String,
    error: String,
    /// Live from ProgressHub SSE cache (not on GET /downloads).
    speed: String,
    eta: String,
    description: String,
}

struct TorrentRow {
    id: String,
    name: String,
    size: String,
    done: String,
    progress: String,
    status: String,
    speed: String,
    uploaded: String,
    ratio: String,
    save_path: String,
    info_hash: String,
    added: String,
    completed: String,
    is_private: String,
    tags: String,
    /// Numeric queue rank from daemon (`queue_position`); higher = higher priority.
    queue_position: String,
    up_speed: String,
    seeds: String,
    peers: String,
    eta: String,
    /// 0.0–1.0 fraction for progress bar fill (string for QVariant role).
    progress_pct: String,
    availability: String,
    category: String,
}

fn download_row(r: ferric_common::models::DownloadRecord) -> DownloadRow {
    let progress = match r.total_size {
        Some(total) if total > 0 => {
            format!(
                "{:.0}%",
                (r.downloaded as f64 / total as f64 * 100.0).min(100.0)
            )
        }
        _ => "-".to_string(),
    };
    DownloadRow {
        id: r.id.to_string(),
        name: r.filename,
        status: r.status.to_string(),
        size: r
            .total_size
            .map(format_bytes)
            .unwrap_or_else(|| "-".to_string()),
        done: format_bytes(r.downloaded),
        progress,
        queue: r
            .queue_id
            .map(|u| u.to_string()[..8].to_string())
            .unwrap_or_default(),
        category: r.category,
        added: format_epoch_ms(r.created_at),
        completed: r.completed_at.map(format_epoch_ms).unwrap_or_default(),
        save_path: r.save_path,
        url_host: url_host(&r.url),
        error: r.error_msg.unwrap_or_default(),
        speed: "-".to_string(),
        eta: "-".to_string(),
        description: r.description.unwrap_or_default(),
    }
}

fn overlay_live_speed(mut row: DownloadRow) -> DownloadRow {
    match crate::progress_hub::live_speed_for(&row.id) {
        Some(live) => {
            row.speed = if live.speed_bps > 0 {
                format_bps(live.speed_bps as u64)
            } else {
                "-".to_string()
            };
            row.eta = match live.eta_secs {
                Some(s) if s > 0 => format_eta(s),
                _ => "-".to_string(),
            };
        }
        None => {
            row.speed = "-".to_string();
            row.eta = "-".to_string();
        }
    }
    row
}

/// IDM's View > Arrange files: 0 = order of addition (as the daemon lists them), then the other keys
/// in IDM's menu order.
static DOWNLOAD_SORT: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

fn sort_records(records: &mut [ferric_common::models::DownloadRecord], key: i32) {
    let live = |r: &ferric_common::models::DownloadRecord| {
        crate::progress_hub::live_speed_for(&r.id.to_string())
    };
    match key {
        1 => records.sort_by_cached_key(|r| r.filename.to_lowercase()),
        2 => records.sort_by_key(|r| r.total_size.unwrap_or(0)),
        3 => records.sort_by_cached_key(|r| r.status.to_string()),
        4 => records.sort_by_key(|r| live(r).and_then(|l| l.eta_secs).unwrap_or(u64::MAX)),
        5 => records.sort_by_key(|r| std::cmp::Reverse(live(r).map_or(0, |l| l.speed_bps))),
        6 => records.sort_by_key(|r| r.started_at.unwrap_or(r.created_at)),
        7 => {
            records.sort_by_cached_key(|r| r.description.clone().unwrap_or_default().to_lowercase())
        }
        8 => records.sort_by_cached_key(|r| r.save_path.to_lowercase()),
        // Ferric keeps one referrer per download, so "Referer" and "Parent web page" sort alike.
        9 | 10 => {
            records.sort_by_cached_key(|r| r.referrer.clone().unwrap_or_default().to_lowercase())
        }
        // Default: order of addition, oldest first.
        _ => records.sort_by_key(|r| r.created_at),
    }
}

async fn fetch_download_rows() -> Result<Vec<DownloadRow>, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads");
    let resp = crate::http_client::HTTP_CLIENT
        .clone()
        .get(&url)
        .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
        .header("X-Ferric-Auth", token)
        .send()
        .await
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("daemon returned {}", resp.status()));
    }
    let mut records: Vec<ferric_common::models::DownloadRecord> =
        resp.json().await.map_err(|e| e.to_string())?;
    sort_records(
        &mut records,
        DOWNLOAD_SORT.load(std::sync::atomic::Ordering::Relaxed),
    );
    Ok(records
        .into_iter()
        .map(download_row)
        .map(overlay_live_speed)
        .collect())
}

fn torrent_status_str(status: ferric_common::torrent::TorrentStatus) -> String {
    use ferric_common::torrent::TorrentStatus as S;
    match status {
        S::Staged => "Staged",
        S::MetadataReady => "Metadata ready",
        // EngineGated is the generic "staged, engine never started" state set
        // for every new torrent (see TorrentManager::stage()) -- it does not
        // mean the torrent is specifically blocked by a proxy requirement.
        // The old label ("Gated: proxy req.") falsely implied that for every
        // torrent, not just ones actually gated by a missing proxy.
        S::EngineGated => "Ready to start",
        S::Checking => "Checking",
        S::Downloading => "Downloading",
        S::Paused => "Paused",
        S::Completed => "Completed",
        S::Error => "Error",
    }
    .to_string()
}

async fn fetch_torrent_rows() -> Result<Vec<TorrentRow>, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents");
    let resp = crate::http_client::HTTP_CLIENT
        .clone()
        .get(&url)
        .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
        .header("X-Ferric-Auth", token)
        .send()
        .await
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("daemon returned {}", resp.status()));
    }
    let records: Vec<ferric_common::torrent::TorrentRecord> =
        resp.json().await.map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(torrent_row).collect())
}

fn torrent_row(r: ferric_common::torrent::TorrentRecord) -> TorrentRow {
    let frac = if r.total_size > 0 {
        (r.downloaded as f64 / r.total_size as f64).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let progress = if r.total_size > 0 {
        format!("{:.1}%", frac * 100.0)
    } else {
        "-".to_string()
    };
    let remaining = r.total_size.saturating_sub(r.downloaded);
    let eta = if r.download_speed > 0 && remaining > 0 {
        format_eta(remaining / r.download_speed)
    } else {
        "-".to_string()
    };
    TorrentRow {
        id: r.id,
        name: r.name,
        size: format_bytes(r.total_size),
        done: format_bytes(r.downloaded),
        progress,
        status: torrent_status_str(r.status),
        // F28: ephemeral download_speed from last poll (0 when idle / not yet polled).
        speed: if r.download_speed > 0 {
            format_bps(r.download_speed)
        } else {
            "-".to_string()
        },
        uploaded: format_bytes(r.uploaded),
        ratio: format!("{:.2}", r.ratio),
        save_path: r.save_root.unwrap_or_default(),
        info_hash: r.info_hash.chars().take(8).collect(),
        added: format_epoch_ms(r.created_at as i64),
        completed: r
            .completed_at
            .map(|t| format_epoch_ms(t as i64))
            .unwrap_or_else(|| "-".to_string()),
        is_private: if r.is_private {
            "Private".to_string()
        } else {
            String::new()
        },
        tags: r.tags.join(","),
        queue_position: r.queue_position.to_string(),
        up_speed: if r.upload_speed > 0 {
            format_bps(r.upload_speed)
        } else {
            "-".to_string()
        },
        // rqbit exposes connected peers only — not seed/leecher split.
        seeds: "-".to_string(),
        peers: if r.peers > 0 {
            r.peers.to_string()
        } else {
            "0".to_string()
        },
        eta,
        progress_pct: format!("{frac:.4}"),
        availability: "-".to_string(),
        category: r.category.unwrap_or_default(),
    }
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++Qt" {
        include!(<QtCore/QAbstractListModel>);
        #[qobject]
        type QAbstractListModel;
    }

    unsafe extern "C++" {
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;

        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;

        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;

        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;

        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    #[qenum(DownloadModel)]
    /// Roles exposed to QML delegates for `DownloadModel` — matches the
    /// index order in `download_role_names`/`download_role_data` below.
    enum DownloadRoles {
        Id,
        Name,
        Status,
        Size,
        Done,
        Progress,
        Queue,
        Category,
        Added,
        Completed,
        SavePath,
        UrlHost,
        Error,
        Speed,
        Eta,
    }

    #[qenum(TorrentModel)]
    /// Roles exposed to QML delegates for `TorrentModel` — matches torrent_role_names/data.
    enum TorrentRoles {
        Id,
        Name,
        Size,
        Done,
        Progress,
        Status,
        Speed,
        Uploaded,
        Ratio,
        SavePath,
        InfoHash,
        Added,
        Completed,
        IsPrivate,
        Tags,
        QueuePosition,
        UpSpeed,
        Seeds,
        Peers,
        Eta,
        ProgressPct,
        Availability,
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        /// last_ok / last_error: T0 list-truth — never silently wipe on fetch failure.
        #[qproperty(bool, last_ok)]
        #[qproperty(QString, last_error)]
        type DownloadModel = super::DownloadModelRust;

        #[qobject]
        #[base = QAbstractListModel]
        #[qml_element]
        #[qproperty(bool, last_ok)]
        #[qproperty(QString, last_error)]
        type TorrentModel = super::TorrentModelRust;
    }

    unsafe extern "RustQt" {
        /// # Safety
        /// Must be paired with `end_reset_model` — enforced by `refresh()` below.
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut DownloadModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut DownloadModel>);

        /// # Safety
        /// Must be paired with `end_reset_model` — enforced by `refresh()` below.
        #[inherit]
        #[cxx_name = "beginResetModel"]
        unsafe fn begin_reset_model(self: Pin<&mut TorrentModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        unsafe fn end_reset_model(self: Pin<&mut TorrentModel>);
    }

    extern "RustQt" {
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &DownloadModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &DownloadModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        fn data(self: &DownloadModel, index: &QModelIndex, role: i32) -> QVariant;
        /// Fetch `GET /downloads` from the daemon and reset the model with the result.
        #[qinvokable]
        fn refresh(self: Pin<&mut DownloadModel>);
        /// IDM's View > Arrange files: 0 order of addition, 1 name, 2 size, 3 status, 4 time left,
        /// 5 transfer rate, 6 last try date, 7 description, 8 save path, 9 referer, 10 parent page.
        #[qinvokable]
        fn set_sort(self: Pin<&mut DownloadModel>, key: i32);
        #[qinvokable]
        fn sort_key(self: &DownloadModel) -> i32;

        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &TorrentModel) -> QHash_i32_QByteArray;
        #[qinvokable]
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &TorrentModel, _parent: &QModelIndex) -> i32;
        #[qinvokable]
        #[cxx_override]
        fn data(self: &TorrentModel, index: &QModelIndex, role: i32) -> QVariant;
        /// Fetch `GET /torrents` from the daemon and reset the model with the result.
        #[qinvokable]
        fn refresh(self: Pin<&mut TorrentModel>);
        /// Return distinct tags across all torrent rows (comma-joined per row).
        #[qinvokable]
        fn distinct_tags(self: &TorrentModel) -> QStringList;
    }

    impl cxx_qt::Threading for DownloadModel {}
    impl cxx_qt::Threading for TorrentModel {}
}

use core::pin::Pin;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QStringList, QVariant,
};

pub struct DownloadModelRust {
    rows: Vec<DownloadRow>,
    last_ok: bool,
    last_error: QString,
}

impl Default for DownloadModelRust {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            last_ok: false,
            last_error: QString::from(""),
        }
    }
}

pub struct TorrentModelRust {
    rows: Vec<TorrentRow>,
    last_ok: bool,
    last_error: QString,
}

impl Default for TorrentModelRust {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            last_ok: false,
            last_error: QString::from(""),
        }
    }
}

fn qv(s: &str) -> QVariant {
    QVariant::from(&QString::from(s))
}

fn download_role_names() -> QHash<QHashPair_i32_QByteArray> {
    let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
    for (i, name) in [
        "id",
        "name",
        "status",
        "size",
        "done",
        "progress",
        "queue",
        "category",
        "added",
        "completed",
        "savePath",
        "urlHost",
        "error",
        "speed",
        "eta",
        "description",
    ]
    .iter()
    .enumerate()
    {
        roles.insert(i as i32, QByteArray::from(*name));
    }
    roles
}

fn download_role_data(row: Option<&DownloadRow>, role: i32) -> QVariant {
    let Some(row) = row else {
        return QVariant::default();
    };
    match role {
        0 => qv(&row.id),
        1 => qv(&row.name),
        2 => qv(&row.status),
        3 => qv(&row.size),
        4 => qv(&row.done),
        5 => qv(&row.progress),
        6 => qv(&row.queue),
        7 => qv(&row.category),
        8 => qv(&row.added),
        9 => qv(&row.completed),
        10 => qv(&row.save_path),
        11 => qv(&row.url_host),
        12 => qv(&row.error),
        13 => qv(&row.speed),
        14 => qv(&row.eta),
        15 => qv(&row.description),
        _ => QVariant::default(),
    }
}

fn torrent_role_names() -> QHash<QHashPair_i32_QByteArray> {
    let mut roles = QHash::<QHashPair_i32_QByteArray>::default();
    for (i, name) in [
        "id",
        "name",
        "size",
        "done",
        "progress",
        "status",
        "speed",
        "uploaded",
        "ratio",
        "savePath",
        "infoHash",
        "added",
        "completed",
        "isPrivate",
        "tags",
        "queuePosition",
        "upSpeed",
        "seeds",
        "peers",
        "eta",
        "progressPct",
        "availability",
        "category",
    ]
    .iter()
    .enumerate()
    {
        roles.insert(i as i32, QByteArray::from(*name));
    }
    roles
}

fn torrent_role_data(row: Option<&TorrentRow>, role: i32) -> QVariant {
    let Some(r) = row else {
        return QVariant::default();
    };
    match role {
        0 => qv(&r.id),
        1 => qv(&r.name),
        2 => qv(&r.size),
        3 => qv(&r.done),
        4 => qv(&r.progress),
        5 => qv(&r.status),
        6 => qv(&r.speed),
        7 => qv(&r.uploaded),
        8 => qv(&r.ratio),
        9 => qv(&r.save_path),
        10 => qv(&r.info_hash),
        11 => qv(&r.added),
        12 => qv(&r.completed),
        13 => qv(&r.is_private),
        14 => qv(&r.tags),
        15 => qv(&r.queue_position),
        16 => qv(&r.up_speed),
        17 => qv(&r.seeds),
        18 => qv(&r.peers),
        19 => qv(&r.eta),
        20 => qv(&r.progress_pct),
        21 => qv(&r.availability),
        22 => qv(&r.category),
        _ => QVariant::default(),
    }
}

impl qobject::DownloadModel {
    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        download_role_names()
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        download_role_data(self.rows.get(index.row() as usize), role)
    }

    pub fn set_sort(self: Pin<&mut Self>, key: i32) {
        DOWNLOAD_SORT.store(key, std::sync::atomic::Ordering::Relaxed);
        self.refresh();
    }

    pub fn sort_key(&self) -> i32 {
        DOWNLOAD_SORT.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        // T0 F-ARCH-2: never map Err → empty list. Keep last good rows and surface error.
        let qt_thread = self.as_mut().qt_thread();
        crate::http_client::RUNTIME.spawn(async move {
            let result = fetch_download_rows().await;
            let _ = qt_thread.queue(move |mut model| match result {
                Ok(rows) => {
                    unsafe {
                        model.as_mut().begin_reset_model();
                        model.as_mut().rust_mut().rows = rows;
                        model.as_mut().end_reset_model();
                    }
                    model.as_mut().set_last_ok(true);
                    model.as_mut().set_last_error(QString::from(""));
                }
                Err(e) => {
                    tracing::warn!("DownloadModel refresh failed (rows retained): {e}");
                    model.as_mut().set_last_ok(false);
                    model.as_mut().set_last_error(QString::from(e.as_str()));
                }
            });
        });
    }
}

impl qobject::TorrentModel {
    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        torrent_role_names()
    }

    pub fn row_count(&self, _parent: &QModelIndex) -> i32 {
        self.rows.len() as i32
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        torrent_role_data(self.rows.get(index.row() as usize), role)
    }

    pub fn refresh(mut self: Pin<&mut Self>) {
        let qt_thread = self.as_mut().qt_thread();
        crate::http_client::RUNTIME.spawn(async move {
            let result = fetch_torrent_rows().await;
            let _ = qt_thread.queue(move |mut model| match result {
                Ok(rows) => {
                    unsafe {
                        model.as_mut().begin_reset_model();
                        model.as_mut().rust_mut().rows = rows;
                        model.as_mut().end_reset_model();
                    }
                    model.as_mut().set_last_ok(true);
                    model.as_mut().set_last_error(QString::from(""));
                }
                Err(e) => {
                    tracing::warn!("TorrentModel refresh failed (rows retained): {e}");
                    model.as_mut().set_last_ok(false);
                    model.as_mut().set_last_error(QString::from(e.as_str()));
                }
            });
        });
    }

    pub fn distinct_tags(&self) -> QStringList {
        let mut seen = std::collections::HashSet::new();
        let mut out = QStringList::default();
        for row in &self.rows {
            for tag in row.tags.split(',').filter(|s| !s.is_empty()) {
                if seen.insert(tag) {
                    out.append(QString::from(tag));
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024 * 3), "3.0 MB");
    }

    #[test]
    fn url_host_strips_scheme_and_path() {
        assert_eq!(url_host("https://example.com/a/b?c=1"), "example.com");
        assert_eq!(url_host("http://x.y:8080/"), "x.y:8080");
        assert_eq!(url_host("not-a-url"), "not-a-url");
    }

    #[test]
    fn format_epoch_ms_empty_for_non_positive() {
        assert_eq!(format_epoch_ms(0), "");
        assert_eq!(format_epoch_ms(-1), "");
    }
}
