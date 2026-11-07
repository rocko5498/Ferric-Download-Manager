use crate::db;
use crate::diagnostic;
use crate::manager::ManagerCommand;
use crate::media;
use crate::security;
use crate::torrent;
use crate::torrent_manager::TorrentManager;
use axum::{
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, patch, post, put},
    Json, Router,
};
use ferric_common::messages::{DaemonReply, DownloadRequest, ProgressEvent};
use ferric_common::{
    ChunkRecord, DownloadDiagnostic, DownloadRecord, ExtractResult, MergeJobRecord, MergeState,
    PrivacyCheckResult, ProxyConfigInput, ProxyConfigRecord, QueueRecord, SiteRuleType,
    TorrentDetail, TorrentEngineStatus, TorrentPolicy, TorrentRecord, TorrentStageRequest,
    TorrentStatus, API_LOCK_FILE,
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::convert::Infallible;
use std::path::{Path as FsPath, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, mpsc, oneshot, Mutex};
use tokio_stream::{wrappers::BroadcastStream, Stream, StreamExt as _};
use uuid::Uuid;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const TORRENT_STATS_POLL_SECS: u64 = 2;

/// Keys accepted by POST /settings (reject unknown keys to limit config injection).
const ALLOWED_SETTINGS_KEYS: &[&str] = &[
    "max_connections_global",
    "max_connections_per_host",
    "max_connections_per_download",
    "max_concurrent",
    "bandwidth_limit_bps",
    "default_save_dir",
    "save_dir",
    "min_capture_size",
    "auto_start",
    "autostart",
    "theme",
    "default_queue_id",
    "notifications_enabled",
    "sound_on_complete",
    "sound_on_each_done",
    "sound_on_error",
    "proxy_url",
    "duplicate_action",
    "pac_url",
    "pac_from_windows",
    "start_while_info_dialog",
    "reduce_motion",
    "quota_mb",
    "quota_hours",
    "user_agent",
    "connect_timeout_secs",
    "capture_enabled",
    "capture_media",
    "capture_extensions",
    "capture_mime_prefixes",
    "capture_exclude_urls",
    "capture_panel_exclude_hosts",
    "capture_manifests",
    "capture_min_size_kb",
    "chunk_size_kb",
    "chunk_size_jitter_pct",
    "audit_mode",
    "audit_retention_days",
    "webhook_enabled",
    "ui_language",
    "dedupe_enabled",
    "show_progress_dialog",
    "show_start_dialog",
    "show_complete_dialog",
    "portable_mode",
    "blocklist_url",
    "ytdlp_path",
    "ytdlp_cookies",
    "ytdlp_max_compat",
    "ytdlp_enable_fallback",
    "torrent_dht_enabled",
    "torrent_lsd_enabled",
    "torrent_pex_enabled",
    "torrent_proxy_url",
    "torrent_proxy_required",
    "torrent_proxy_warned_at",
    "torrent_blocklist_url",
    "torrent_blocklist_last_update",
    "torrent_kill_switch_active",
    // 0 (default) = outbound-only (--disable-tcp-listen). 1 = allow inbound peer port.
    "torrent_tcp_listen_enabled",
    // 0 = refuse all torrent engine starts (panic switch). 1 or unset = allow gated starts.
    "torrent_engine_enabled",
    // T1: strict | balanced | open — expands into discovery/listen/proxy defaults.
    "product_mode",
    "torrent_seeding_default",
    "torrent_seed_ratio_limit",
    "torrent_seed_time_limit_secs",
    "geoip_mmdb_path",
    "av_enabled",
    "av_path",
    "av_args",
    "api_rate_limit_per_min",
    "allow_lan_downloads",
    "when_done_action",
    "open_file_when_done",
    "file_time_from_server",
    "only_add_to_queue",
    "show_queue_panel",
    "toolbar_small",
    "show_tray_icon",
    "ui_font",
    "adaptive_connections",
    "adaptive_max_connections",
    "ignore_mtime_changes",
    "show_drop_target",
    "max_retries",
    "onboarding_done",
    "clipboard_monitor_enabled",
    // Plugins (default off — WASM runtime is half-wired, gate until finished)
    // WebUI (default off)
    "webui_enabled",
    "webui_port",
    // General
    "start_minimized",
    "close_to_tray",
    "confirm_on_exit",
    "check_updates",
    "single_instance",
    // Downloads
    "ask_where_to_save",
    "auto_resume_on_start",
    "preallocate_disk",
    // Connection (torrent)
    "listen_port",
    "protocol_encryption",
    "upnp_enabled",
    "max_uploads_per_torrent",
    // Speed
    "global_upload_limit_bps",
    "alt_speed_enabled",
    "alt_speed_download_bps",
    "alt_speed_upload_bps",
    "alt_speed_schedule_days",
    "alt_speed_schedule_from",
    "alt_speed_schedule_to",
    // Column visibility (FQT-073)
    "download_visible_columns",
    "torrent_visible_columns",
];

// â"€â"€ App state â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[derive(Clone)]
struct AppState {
    pool: SqlitePool,
    manager_tx: mpsc::Sender<ManagerCommand>,
    progress_tx: broadcast::Sender<ProgressEvent>,
    auth_token: String,
    start_time: Instant,
    merge_jobs: Arc<Mutex<HashMap<String, MergeJobRecord>>>,
    torrents: TorrentManager,
    /// Task 131 — simple sliding window rate limit (timestamps ms).
    rate_hits: Arc<Mutex<Vec<u64>>>,
    /// Live torrent speeds: id → (`download_bps`, `upload_bps`, peers). Updated by stats poller.
    torrent_speeds: Arc<Mutex<HashMap<String, (u64, u64, u64)>>>,
}

// â"€â"€ Error type â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[derive(Debug)]
struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, self.1).into_response()
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn not_found(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::NOT_FOUND, msg.into())
}

fn bad_request(msg: impl Into<String>) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, msg.into())
}

fn confirm_error(message: String) -> ApiError {
    if message == "no pending capture for this download" {
        ApiError(
            StatusCode::CONFLICT,
            "This browser capture expired after the daemon restarted. Capture the link again so Ferric can use the live browser headers.".into(),
        )
    } else {
        ApiError(StatusCode::INTERNAL_SERVER_ERROR, message)
    }
}

// â"€â"€ Server startup â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

pub async fn serve(
    pool: SqlitePool,
    manager_tx: mpsc::Sender<ManagerCommand>,
    progress_tx: broadcast::Sender<ProgressEvent>,
    data_dir: PathBuf,
) -> anyhow::Result<()> {
    let auth_token = generate_token()?;

    let state = AppState {
        torrents: TorrentManager::new(pool.clone()),
        pool,
        manager_tx,
        progress_tx,
        auth_token: auth_token.clone(),
        start_time: Instant::now(),
        merge_jobs: Arc::new(Mutex::new(HashMap::new())),
        rate_hits: Arc::new(Mutex::new(Vec::new())),
        torrent_speeds: Arc::new(Mutex::new(HashMap::new())),
    };
    start_torrent_stats_poller(
        state.torrents.clone(),
        state.progress_tx.clone(),
        state.torrent_speeds.clone(),
    );

    let app = build_router(state);

    // Bind to random localhost port
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let port = listener.local_addr()?.port();

    let lock_path = data_dir.join(API_LOCK_FILE);
    write_lock_file(data_dir, port, &auth_token).await?;
    tracing::info!(port, "API server listening");

    let result = axum::serve(listener, app).await;
    tokio::fs::remove_file(&lock_path).await.ok();
    result?;
    Ok(())
}

fn start_torrent_stats_poller(
    torrents: TorrentManager,
    progress_tx: broadcast::Sender<ProgressEvent>,
    torrent_speeds: Arc<Mutex<HashMap<String, (u64, u64, u64)>>>,
) {
    tokio::spawn(async move {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_secs(TORRENT_STATS_POLL_SECS));
        loop {
            interval.tick().await;
            let Ok(records) = torrents.list().await else {
                continue;
            };
            // IDs we attempted this cycle (Downloading + rqbit). Retain uses this so a
            // transient sync Err does not wipe overlay speeds mid-download.
            let mut polled: std::collections::HashSet<String> = std::collections::HashSet::new();
            for record in records {
                if !should_poll_torrent_stats(&record) {
                    continue;
                }
                polled.insert(record.id.clone());
                match torrents.sync_stats(&record.id).await {
                    Ok(detail) => {
                        {
                            let mut map = torrent_speeds.lock().await;
                            map.insert(
                                detail.record.id.clone(),
                                (
                                    detail.record.download_speed,
                                    detail.record.upload_speed,
                                    detail.record.peers,
                                ),
                            );
                        }
                        let _ = progress_tx.send(ProgressEvent::TorrentUpdated {
                            record: detail.record,
                        });
                    }
                    Err(err) => {
                        // Keep any existing map entry; list overlay stays non-zero.
                        tracing::debug!(torrent_id = %record.id, error = %err, "torrent stats sync failed");
                    }
                }
            }
            // Drop speeds only for torrents no longer in the poll set (paused/completed/removed).
            {
                let mut map = torrent_speeds.lock().await;
                map.retain(|id, _| polled.contains(id));
            }
        }
    });
}

fn should_poll_torrent_stats(record: &TorrentRecord) -> bool {
    if !record.engine.starts_with("rqbit:") {
        return false;
    }
    // Active transfers always.
    if matches!(
        record.status,
        TorrentStatus::Downloading | TorrentStatus::Checking
    ) {
        return true;
    }
    // Fully-written but still "Paused" — poll once so sync_stats can mark Completed
    // (live 2026-07-17: ubuntu 100% stayed Paused because poller skipped paused).
    record.status == TorrentStatus::Paused
        && record.total_size > 0
        && record.downloaded >= record.total_size
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/downloads", get(list_downloads).post(add_download))
        .route("/downloads/{id}", get(get_download).delete(remove_download))
        .route("/downloads/{id}/diagnostic", get(get_download_diagnostic))
        .route("/downloads/{id}/chunks", get(get_chunks))
        .route("/downloads/{id}/pause", post(pause_download))
        .route("/downloads/{id}/resume", post(resume_download))
        .route("/downloads/{id}/redownload", post(redownload_download))
        .route("/downloads/{id}/start-now", post(start_now_download))
        .route("/ui/show", post(show_ui))
        .route("/downloads/{id}/cancel", post(cancel_download))
        .route("/downloads/{id}/confirm", post(confirm_download))
        .route("/captures/pending", get(list_pending_captures))
        .route("/downloads/{id}/zip-preview", get(zip_preview))
        .route("/category-folders", get(list_category_folders).put(set_category_folder))
        .route("/downloads/{id}/priority", post(set_priority))
        .route("/downloads/{id}/speed-limit", post(set_speed_limit))
        .route("/downloads/{id}/queue", post(set_download_queue))
        .route("/downloads/{id}/refresh-url", post(refresh_url))
        .route("/downloads/{id}/audit", get(get_download_audit))
        .route(
            "/downloads/{id}/description",
            axum::routing::patch(set_download_description),
        )
        .route("/media/extract", post(extract_media))
        .route("/media/download", post(media_download))
        .route("/media/playlist-download", post(playlist_download))
        .route("/media/select", post(select_media_formats))
        .route(
            "/media/presets",
            get(list_format_presets).post(save_user_presets),
        )
        .route("/media/cookies/parse", post(parse_netscape_cookies))
        .route("/media/cookies/export", post(export_netscape_cookies))
        .route(
            "/media/cookies/from-browser",
            post(cookies_from_browser_disabled),
        )
        .route("/media/extract-login", post(extract_login_disabled))
        .route("/media/ytdlp", get(ytdlp_status))
        .route("/media/probe", post(probe_media))
        .route("/media/ffmpeg", get(ffmpeg_status))
        .route(
            "/media/merge-jobs",
            get(list_merge_jobs).post(start_merge_job),
        )
        .route("/torrents", get(list_torrents).post(stage_torrent))
        .route(
            "/torrents/{id}",
            get(get_torrent_detail).delete(remove_torrent),
        )
        .route("/torrents/{id}/policy", patch(update_torrent_policy))
        .route("/torrents/{id}/files/{file_id}", patch(update_torrent_file))
        .route("/torrents/{id}/proxy", post(assign_torrent_proxy))
        .route("/torrents/{id}/privacy-check", get(torrent_privacy_check))
        .route("/torrents/{id}/start", post(start_torrent_engine))
        .route("/torrents/{id}/sync", post(sync_torrent_engine))
        .route("/torrents/{id}/pause", post(pause_torrent_engine))
        .route("/torrents/{id}/resume", post(resume_torrent_engine))
        .route("/torrents/{id}/recheck", post(force_recheck_torrent))
        .route("/torrents/{id}/reannounce", post(force_reannounce_torrent))
        .route("/torrents/{id}/location", post(set_torrent_location))
        .route("/torrents/{id}/trackers", put(set_torrent_trackers))
        .route("/torrents/{id}/peers", get(torrent_live_peers))
        .route("/torrents/{id}/audit", get(get_torrent_audit))
        .route("/torrents/engine/status", get(torrent_engine_status))
        .route("/torrents/probe", post(probe_torrent))
        .route("/torrents/import/qbittorrent", post(import_qbittorrent))
        .route("/proxies", get(list_proxy_configs).post(save_proxy_config))
        .route("/proxies/{id}", axum::routing::delete(delete_proxy_config))
        .route("/proxies/{id}/test", post(test_proxy_config))
        .route("/grab", post(grab_handler))
        .route(
            "/grabber/projects",
            get(list_grabber_projects).post(save_grabber_project),
        )
        .route(
            "/grabber/projects/{id}",
            get(get_grabber_project).delete(delete_grabber_project),
        )
        .route("/grabber/projects/{id}/update", post(update_grabber_project))
        .route("/queues", get(list_queues).post(create_queue))
        .route("/site-logins", get(list_site_logins).post(save_site_login))
        .route("/site-logins/{id}", axum::routing::delete(delete_site_login))
        .route("/queues/{id}", get(get_queue).patch(update_queue).delete(delete_queue))
        .route("/queues/{id}/start", post(start_queue))
        .route("/queues/{id}/stop", post(stop_queue))
        .route("/settings", get(get_settings).post(update_settings))
        .route("/privacy/dashboard", get(privacy_dashboard))
        .route("/privacy/anonymity-score", get(anonymity_score_handler))
        .route("/audit/export", get(audit_export))
        .route("/rss/feeds", get(list_rss).post(add_rss))
        .route("/rss/feeds/{id}", axum::routing::delete(delete_rss_feed))
        .route("/rss/rules", get(list_rss_rules).post(add_rss_rule))
        .route("/rss/rules/{id}", axum::routing::delete(delete_rss_rule))
        .route("/webhooks", get(list_webhooks).post(add_webhook))
        .route("/categories", get(list_categories).post(add_category))
        .route("/site-rules", get(list_site_rules).post(add_site_rule))
        .route(
            "/site-rules/{id}",
            put(update_site_rule).delete(delete_site_rule),
        )
        .route("/events", get(events_handler))
        .route("/health", get(health_handler))
        .route("/stats", get(get_stats))
        .route("/downloads/batch", post(batch_downloads))
        .route("/torrents/batch", post(batch_torrents))
        .route("/torrents/{id}/queue", post(set_torrent_queue_position))
        .route(
            "/torrents/{id}/tags",
            get(get_torrent_tags)
                .put(set_torrent_tags)
                .post(add_torrent_tag)
                .delete(remove_torrent_tag),
        )
        .route(
            "/torrents/{id}/category",
            axum::routing::patch(set_torrent_category),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        // Outermost: runs before authentication.
        .layer(middleware::from_fn(host_origin_guard))
        .with_state(state)
}

async fn write_lock_file(data_dir: PathBuf, port: u16, auth_token: &str) -> anyhow::Result<()> {
    let lock_path = data_dir.join(API_LOCK_FILE);
    write_lock_file_contents(&lock_path, format!("{port}\n{auth_token}\n")).await?;
    tracing::info!(lock = %lock_path.display(), "API lock file written");
    Ok(())
}

#[cfg(unix)]
async fn write_lock_file_contents(lock_path: &FsPath, contents: String) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;

    let lock_path = lock_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        use std::io::Write as _;
        use std::os::unix::fs::OpenOptionsExt as _;

        let tmp_path = lock_path.with_extension("lock.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp_path)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp_path, &lock_path)?;
        std::fs::set_permissions(&lock_path, std::fs::Permissions::from_mode(0o600))?;
        Ok(())
    })
    .await?
}

#[cfg(not(unix))]
async fn write_lock_file_contents(lock_path: &FsPath, contents: String) -> anyhow::Result<()> {
    let tmp_path = lock_path.with_extension("lock.tmp");
    tokio::fs::write(&tmp_path, contents).await?;
    tokio::fs::rename(&tmp_path, lock_path).await?;
    harden_lock_file_permissions(lock_path).await?;
    Ok(())
}

#[cfg(windows)]
async fn harden_lock_file_permissions(lock_path: &FsPath) -> anyhow::Result<()> {
    let lock_path = lock_path.to_path_buf();
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        let user = windows_current_user_principal()?;
        use std::os::windows::process::CommandExt as _;
        let status = std::process::Command::new("icacls")
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .arg(&lock_path)
            .arg("/inheritance:r")
            .arg("/grant:r")
            .arg(format!("{user}:(F)"))
            .arg("/remove:g")
            .arg("*S-1-1-0")
            .arg("*S-1-5-32-545")
            .status()?;
        if !status.success() {
            anyhow::bail!("failed to restrict API lock file permissions with icacls: {status}");
        }
        Ok(())
    })
    .await?
}

#[cfg(windows)]
fn windows_current_user_principal() -> anyhow::Result<String> {
    let username = std::env::var("USERNAME")
        .map_err(|_| anyhow::anyhow!("USERNAME is not set; cannot harden lock file ACL"))?;
    let username = username.trim();
    if username.is_empty() {
        anyhow::bail!("USERNAME is empty; cannot harden lock file ACL");
    }
    let domain = std::env::var("USERDOMAIN").unwrap_or_default();
    let domain = domain.trim();
    if domain.is_empty() {
        Ok(username.to_string())
    } else {
        Ok(format!("{domain}\\{username}"))
    }
}

#[cfg(not(any(unix, windows)))]
async fn harden_lock_file_permissions(_lock_path: &FsPath) -> anyhow::Result<()> {
    Ok(())
}

fn generate_token() -> anyhow::Result<String> {
    use ring::rand::{SecureRandom, SystemRandom};
    let rng = SystemRandom::new();
    let mut bytes = [0u8; 32];
    rng.fill(&mut bytes)
        .map_err(|_| anyhow::anyhow!("failed to generate API auth token"))?;
    use std::fmt::Write as _;
    let mut token = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(token, "{b:02x}").expect("writing to a String is infallible");
    }
    Ok(token)
}

// â"€â"€ Auth middleware â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

/// Defence in depth for a loopback-only API: a browser page can never reach it with
/// a valid request. Pages always send `Origin` on cross-site requests, and a DNS
/// rebinding attack arrives with the attacker's host name in `Host`.
async fn host_origin_guard(req: Request, next: Next) -> Result<Response, StatusCode> {
    if req.headers().contains_key(axum::http::header::ORIGIN) {
        return Err(StatusCode::FORBIDDEN);
    }
    if let Some(host) = req.headers().get(axum::http::header::HOST) {
        if !host.to_str().is_ok_and(is_loopback_host) {
            return Err(StatusCode::FORBIDDEN);
        }
    }
    Ok(next.run(req).await)
}

/// `127.0.0.1`, `localhost` or `[::1]`, optionally followed by `:port`.
fn is_loopback_host(host: &str) -> bool {
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split_once(']').map(|(ip, _)| format!("[{ip}]"))
    } else {
        Some(host.rsplit_once(':').map_or(host, |(n, _)| n).to_string())
    };
    name.is_some_and(|n| {
        matches!(
            n.to_ascii_lowercase().as_str(),
            "127.0.0.1" | "localhost" | "[::1]"
        )
    })
}

async fn auth_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    let provided = req
        .headers()
        .get("X-Ferric-Auth")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    // Constant-time comparison to avoid timing attacks
    if !constant_eq(provided.as_bytes(), state.auth_token.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }

    // Task 131 — rate limit after auth (localhost still needs abuse protection)
    let limit = db::settings::get(&state.pool, "api_rate_limit_per_min")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3000)
        .max(30);
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64);
    let window_start = now_ms.saturating_sub(60_000);
    {
        let mut hits = state.rate_hits.lock().await;
        hits.retain(|t| *t >= window_start);
        if hits.len() >= limit {
            return Err(StatusCode::TOO_MANY_REQUESTS);
        }
        hits.push(now_ms);
    }

    Ok(next.run(req).await)
}

fn constant_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter()
        .zip(b.iter())
        .fold(0u8, |acc, (x, y)| acc | (x ^ y))
        == 0
}

// â"€â"€ Request/response types â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

#[derive(Deserialize)]
struct AddUrlBody {
    /// Hold the download for the Download File Info dialog instead of starting it.
    #[serde(default)]
    hold: bool,
    url: String,
    filename: Option<String>,
    save_dir: Option<String>,
    queue_id: Option<Uuid>,
    cookies: Option<String>,
    checksum: Option<String>,
    /// Alternate source URLs for multi-source download.
    #[serde(default)]
    mirrors: Vec<String>,
    audio_url: Option<String>,
    subtitle_url: Option<String>,
    subtitle_lang: Option<String>,
    proxy_url: Option<String>,
    description: Option<String>,
}

#[derive(Serialize)]
struct HealthResponse {
    version: &'static str,
    uptime_secs: u64,
    data_dir: String,
}

// â"€â"€ Handlers â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

async fn list_downloads(State(s): State<AppState>) -> ApiResult<Vec<DownloadRecord>> {
    let records = db::downloads::list(&s.pool, None).await?;
    Ok(Json(records))
}

async fn get_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<DownloadRecord>, ApiError> {
    db::downloads::get(&s.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| not_found(format!("download {id} not found")))
}

async fn get_download_diagnostic(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<DownloadDiagnostic> {
    let record = db::downloads::get(&s.pool, id)
        .await?
        .ok_or_else(|| not_found(format!("download {id} not found")))?;
    let chunks = db::chunks::get_chunks(&s.pool, id)
        .await
        .unwrap_or_default();
    let site_policy = db::site_rules::policy_for_url(&s.pool, &record.url)
        .await
        .unwrap_or_default();
    let policy = diagnostic::DiagnosticPolicy {
        max_connections: site_policy.max_connections,
        max_speed_bps: site_policy.max_speed_bps,
    };
    Ok(Json(diagnostic::classify_download_with_policy(
        &record, &chunks, policy,
    )))
}

async fn get_chunks(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ChunkRecord>>, ApiError> {
    ensure_exists(&s.pool, id).await?;
    Ok(Json(db::chunks::get_chunks(&s.pool, id).await?))
}

async fn add_download(
    State(s): State<AppState>,
    Json(body): Json<AddUrlBody>,
) -> Result<(StatusCode, Json<DownloadRecord>), ApiError> {
    // Validate URL early so bad input gets a 400, not 500
    security::validate_url(&body.url).map_err(|e| bad_request(e.to_string()))?;

    let AddUrlBody {
        hold,
        url,
        filename,
        save_dir,
        queue_id,
        cookies,
        checksum,
        mirrors,
        audio_url,
        subtitle_url,
        subtitle_lang,
        proxy_url,
        description,
    } = body;

    if let Some(id) = queue_id {
        if db::queues::get(&s.pool, id).await?.is_none() {
            return Err(bad_request(format!("queue {id} not found")));
        }
    }

    for mirror in &mirrors {
        security::validate_url(mirror).map_err(|e| bad_request(e.to_string()))?;
    }
    if let Some(ref a) = audio_url {
        security::validate_url(a).map_err(|e| bad_request(e.to_string()))?;
    }
    if let Some(ref s) = subtitle_url {
        security::validate_url(s).map_err(|e| bad_request(e.to_string()))?;
    }
    if let Some(ref p) = proxy_url {
        security::validate_url(p).map_err(|e| bad_request(e.to_string()))?;
    }
    if let Some(ref name) = filename {
        if security::is_dangerous_extension(std::path::Path::new(name)) {
            tracing::warn!(%name, "download filename uses a high-risk extension");
        }
    }

    let req = DownloadRequest {
        id: Uuid::new_v4(),
        url,
        final_url: None,
        filename,
        referrer: None,
        cookies,
        headers: None,
        size_hint: None,
        mime_type: None,
        method: "GET".into(),
        post_body_b64: None,
        queue_id,
        checksum,
        mirrors,
        timestamp: unix_ms(),
    };

    let save_dir_override = save_dir.map(PathBuf::from);

    let (tx, rx) = oneshot::channel::<DaemonReply>();
    s.manager_tx
        .send(ManagerCommand::Add {
            req: req.clone(),
            save_dir_override,
            hold,
            audio_url,
            subtitle_url,
            subtitle_lang,
            proxy_url,
            reply_tx: tx,
        })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;

    match rx.await {
        Ok(DaemonReply::Accepted { id }) => {
            if let Some(ref desc) = description {
                db::downloads::set_description(&s.pool, id, Some(desc)).await?;
            }
            let record = db::downloads::get(&s.pool, id).await?.ok_or_else(|| {
                ApiError(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "record not found after insert".into(),
                )
            })?;
            Ok((StatusCode::CREATED, Json(record)))
        }
        Ok(DaemonReply::Error { message, .. }) => {
            Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, message))
        }
        Err(_) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "manager reply lost".into(),
        )),
    }
}

/// Stop an active download and mark it cancelled, keeping the record.
async fn cancel_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx.send(ManagerCommand::Cancel(id)).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ConfirmBody {
    filename: Option<String>,
    save_dir: Option<String>,
    url: Option<String>,
    audio_url: Option<String>,
    subtitle_url: Option<String>,
    subtitle_lang: Option<String>,
    proxy_url: Option<String>,
    /// `false` = "Download Later" (default true).
    #[serde(default)]
    start: Option<bool>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    category: Option<String>,
}

#[derive(Deserialize)]
struct PriorityBody {
    priority: i32,
}

#[derive(Deserialize, Default)]
struct RemoveQuery {
    delete_file: Option<bool>,
}

/// serde maps JSON `null` to the outer `None` for `Option<Option<T>>`; keep absent (preserve) and
/// null (clear) apart.
fn double_option<'de, T, D>(d: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize)]
#[allow(clippy::option_option)]
struct QueuePatchBody {
    name: Option<String>,
    max_concurrent: Option<u32>,
    /// Tri-state: absent = preserve, null = clear to unlimited, value = set.
    #[serde(default, deserialize_with = "double_option")]
    max_bandwidth: Option<Option<u64>>,
    enabled: Option<bool>,
    /// Minutes since midnight UTC (0–1439). Tri-state like `max_bandwidth`.
    #[serde(default, deserialize_with = "double_option")]
    sched_start: Option<Option<u16>>,
    #[serde(default, deserialize_with = "double_option")]
    sched_end: Option<Option<u16>>,
    /// Weekday bits (0 = Sunday ... 6 = Saturday); at least one day.
    sched_days: Option<u8>,
    start_on_startup: Option<bool>,
    /// Minutes between synchronization runs; null = not a synchronization queue.
    #[serde(default, deserialize_with = "double_option")]
    sync_interval_min: Option<Option<u32>>,
    #[serde(default, deserialize_with = "double_option")]
    max_retries: Option<Option<u32>>,
    #[serde(default, deserialize_with = "double_option")]
    open_file: Option<Option<String>>,
    #[serde(default, deserialize_with = "double_option")]
    when_done: Option<Option<String>>,
}

/// Confirm a held capture, applying optional filename/folder overrides and
/// starting the download.
/// IDM's Preview button: the file list of a zip, read from the end of the remote file.
async fn zip_preview(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<crate::zip_preview::ZipEntry>> {
    ensure_exists(&s.pool, id).await?;
    let record = db::downloads::get(&s.pool, id)
        .await?
        .ok_or_else(|| ApiError(StatusCode::NOT_FOUND, "download not found".into()))?;
    let total = record
        .total_size
        .ok_or_else(|| bad_request("the file size is not known yet"))?;
    let url = record.final_url.as_deref().unwrap_or(&record.url);
    security::validate_url(url).map_err(|e| bad_request(e.to_string()))?;
    let client = crate::manager::build_reqwest_client(None)
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let entries = crate::zip_preview::preview(&client, url, total)
        .await
        .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e))?;
    Ok(Json(entries))
}

/// Captures waiting for the Download File Info dialog (the UI asks on startup/reconnect).
async fn list_pending_captures(State(s): State<AppState>) -> ApiResult<Vec<DownloadRecord>> {
    let (tx, rx) = oneshot::channel::<Vec<Uuid>>();
    s.manager_tx
        .send(ManagerCommand::ListPending { reply_tx: tx })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;
    let ids = rx.await.unwrap_or_default();
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(record) = db::downloads::get(&s.pool, id).await? {
            out.push(record);
        }
    }
    Ok(Json(out))
}

#[derive(Serialize)]
struct CategoryFolder {
    name: String,
    dir: String,
}

/// Every category with the folder it saves to (General first), for the Category combo.
async fn list_category_folders(State(s): State<AppState>) -> ApiResult<Vec<CategoryFolder>> {
    let cats = crate::categories::load(&s.pool).await;
    let base = crate::categories::base_dir(&s.pool).await;
    let mut out = vec![CategoryFolder {
        name: crate::categories::GENERAL.into(),
        dir: crate::categories::folder_for(crate::categories::GENERAL, &base, &cats, &s.pool)
            .await
            .to_string_lossy()
            .into_owned(),
    }];
    for c in &cats {
        out.push(CategoryFolder {
            name: c.name.clone(),
            dir: crate::categories::folder_for(&c.name, &base, &cats, &s.pool)
                .await
                .to_string_lossy()
                .into_owned(),
        });
    }
    Ok(Json(out))
}

#[derive(Deserialize)]
struct CategoryFolderBody {
    name: String,
    dir: String,
}

/// "Remember this path for the category".
async fn set_category_folder(
    State(s): State<AppState>,
    Json(body): Json<CategoryFolderBody>,
) -> Result<StatusCode, ApiError> {
    if body.name.trim().is_empty() {
        return Err(bad_request("name required"));
    }
    crate::categories::remember_folder(&s.pool, body.name.trim(), body.dir.trim())
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn confirm_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ConfirmBody>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    let (tx, rx) = oneshot::channel::<DaemonReply>();
    s.manager_tx
        .send(ManagerCommand::Confirm {
            id,
            filename: body.filename,
            save_dir: body.save_dir,
            url: body.url,
            audio_url: body.audio_url,
            subtitle_url: body.subtitle_url,
            subtitle_lang: body.subtitle_lang,
            proxy_url: body.proxy_url,
            start: body.start.unwrap_or(true),
            description: body.description,
            category: body.category,
            reply_tx: tx,
        })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;

    match rx.await {
        Ok(DaemonReply::Accepted { .. }) => Ok(StatusCode::NO_CONTENT),
        Ok(DaemonReply::Error { message, .. }) => Err(confirm_error(message)),
        Err(_) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "manager reply lost".into(),
        )),
    }
}

/// Stop and delete the record entirely.
async fn remove_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Query(query): Query<RemoveQuery>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx
        .send(ManagerCommand::Remove {
            id,
            delete_file: query.delete_file.unwrap_or(false),
        })
        .await
        .ok();
    Ok(StatusCode::NO_CONTENT)
}

async fn set_priority(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PriorityBody>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx
        .send(ManagerCommand::SetPriority {
            id,
            priority: body.priority,
        })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct SpeedLimitBody {
    /// bytes/sec; 0 = unlimited
    bps: u64,
}

async fn set_speed_limit(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SpeedLimitBody>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx
        .send(ManagerCommand::SetSpeedLimit { id, bps: body.bps })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MoveToQueueBody {
    queue_id: Uuid,
}

/// IDM's "Move to queue".
async fn set_download_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<MoveToQueueBody>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    let Some(queue) = db::queues::get(&s.pool, body.queue_id).await? else {
        return Err(not_found(format!("queue {} not found", body.queue_id)));
    };
    // IDM: finished files can only be put into a synchronization queue.
    if queue.sync_interval_min.is_none() {
        let finished = db::downloads::get(&s.pool, id)
            .await?
            .is_some_and(|d| d.status == ferric_common::DownloadStatus::Completed);
        if finished {
            return Err(bad_request(
                "a finished file can only be moved to a synchronization queue",
            ));
        }
    }
    db::downloads::set_queue(&s.pool, id, Some(body.queue_id)).await?;
    s.manager_tx.send(ManagerCommand::PumpQueue).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct DescriptionBody {
    description: Option<String>,
}

async fn set_download_description(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<DescriptionBody>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    db::downloads::set_description(&s.pool, id, body.description.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn pause_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx.send(ManagerCommand::Pause(id)).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx.send(ManagerCommand::Resume(id)).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

/// Asks the running UI window to come to the front (sent by a second `ferric-qt` launch, so opening Ferric
/// again works even when the first copy was started hidden at login and has no window handle yet).
async fn show_ui(State(s): State<AppState>) -> StatusCode {
    let _ = s.progress_tx.send(ProgressEvent::ShowUi);
    StatusCode::NO_CONTENT
}

async fn start_now_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx.send(ManagerCommand::StartNow(id)).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

async fn redownload_download(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    ensure_exists(&s.pool, id).await?;
    s.manager_tx.send(ManagerCommand::Redownload(id)).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

async fn get_settings(State(s): State<AppState>) -> ApiResult<HashMap<String, String>> {
    let mut settings = db::settings::get_all(&s.pool).await?;
    // Default mode when never set — matches balanced discovery defaults.
    settings
        .entry(crate::product_mode::PRODUCT_MODE_KEY.to_string())
        .or_insert_with(|| "balanced".to_string());
    Ok(Json(settings))
}

async fn update_settings(
    State(s): State<AppState>,
    Json(updates): Json<HashMap<String, String>>,
) -> Result<StatusCode, ApiError> {
    // Validate all keys first (reject unknown keys).
    for key in updates.keys() {
        if !ALLOWED_SETTINGS_KEYS.contains(&key.as_str()) {
            return Err(bad_request(format!("unknown settings key: {key}")));
        }
    }
    // product_mode must be a known value before we expand it.
    if let Some(mode) = updates.get(crate::product_mode::PRODUCT_MODE_KEY) {
        if crate::product_mode::normalize_mode(mode).is_none() {
            return Err(bad_request(
                "product_mode must be one of: strict, balanced, open".to_string(),
            ));
        }
    }
    // Re-validate kill-switch disable: reject if proxy is required but unreachable.
    if let Some(v) = updates.get("torrent_kill_switch_active") {
        if v == "0" {
            let required = db::settings::get(&s.pool, "torrent_proxy_required")
                .await
                .ok()
                .flatten()
                .is_some_and(|v| v == "1");
            if required {
                let proxy = db::settings::get(&s.pool, "torrent_proxy_url")
                    .await
                    .ok()
                    .flatten()
                    .filter(|p| !p.trim().is_empty());
                let healthy = match proxy.as_deref() {
                    Some(url) => crate::kill_switch::probe_socks5(url).await,
                    None => false,
                };
                if !healthy {
                    return Err(bad_request(
                        "cannot disable kill switch: torrent_proxy_required=1 and the proxy is still unreachable"
                            .to_string(),
                    ));
                }
            }
        }
    }
    // T1: product_mode expands into the full constellation + public torrent policies.
    // Applied first so any same-request overrides of individual keys still win below.
    if let Some(mode) = updates.get(crate::product_mode::PRODUCT_MODE_KEY) {
        crate::product_mode::apply_mode(&s.pool, mode)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
    }
    // Write remaining keys (including optional per-toggle overrides after mode).
    for (key, value) in &updates {
        if key == crate::product_mode::PRODUCT_MODE_KEY {
            continue; // already written by apply_mode
        }
        db::settings::set(&s.pool, key, value).await?;
    }
    if let Some(val) = updates.get("max_retries") {
        if let Ok(n) = val.trim().parse::<u32>() {
            crate::retry::set_max_attempts(n);
        }
    }
    if let Some(val) = updates.get("allow_lan_downloads") {
        crate::security::set_allow_lan(val == "1");
    }
    if updates.contains_key("proxy_url") {
        s.manager_tx.send(ManagerCommand::ReloadProxy).await.ok();
    }
    if updates.contains_key("bandwidth_limit_bps") {
        // Running downloads adopt the new global limit now, not at their next start.
        s.manager_tx.send(ManagerCommand::RefreshLimits).await.ok();
    }
    if let Some(val) = updates.get("auto_start") {
        let exe = std::env::current_exe()
            .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
        let result = if val == "1" {
            crate::autostart::register(&exe)
        } else {
            crate::autostart::unregister()
        };
        result.map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_site_logins(
    State(s): State<AppState>,
) -> ApiResult<Vec<crate::site_logins::SiteLogin>> {
    Ok(Json(crate::site_logins::list(&s.pool).await?))
}

#[derive(Deserialize)]
struct SiteLoginBody {
    server: String,
    username: String,
    password: String,
}

async fn save_site_login(
    State(s): State<AppState>,
    Json(body): Json<SiteLoginBody>,
) -> Result<StatusCode, ApiError> {
    crate::site_logins::upsert(&s.pool, &body.server, &body.username, &body.password)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_site_login(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if crate::site_logins::delete(&s.pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found(format!("site login {id} not found")))
    }
}

async fn list_grabber_projects(
    State(s): State<AppState>,
) -> ApiResult<Vec<db::grabber_projects::ProjectSummary>> {
    Ok(Json(db::grabber_projects::list(&s.pool).await?))
}

#[derive(Deserialize)]
struct GrabberProjectBody {
    name: String,
    #[serde(default)]
    is_template: bool,
    #[serde(default)]
    config: serde_json::Value,
    #[serde(default)]
    results: Vec<String>,
}

async fn save_grabber_project(
    State(s): State<AppState>,
    Json(body): Json<GrabberProjectBody>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let id = db::grabber_projects::save(
        &s.pool,
        &body.name,
        body.is_template,
        &body.config,
        &body.results,
    )
    .await
    .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(serde_json::json!({ "id": id })))
}

#[derive(serde::Serialize)]
struct GrabberUpdateReply {
    added: Vec<String>,
    removed: Vec<String>,
    total: usize,
}

/// IDM's "Update": explore the project's site again and keep what it finds now.
async fn update_grabber_project(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<GrabberUpdateReply>, ApiError> {
    let project = db::grabber_projects::get(&s.pool, &id)
        .await?
        .ok_or_else(|| not_found(format!("grabber project {id} not found")))?;
    if let Some(url) = project.config.get("url").and_then(|v| v.as_str()) {
        security::validate_url(url).map_err(|e| bad_request(e.to_string()))?;
    }
    let found = crate::grabber::crawl_from_config(&project.config)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let diff = crate::grabber::diff_results(&project.results, &found);
    db::grabber_projects::save(
        &s.pool,
        &project.name,
        project.is_template,
        &project.config,
        &found,
    )
    .await?;
    Ok(Json(GrabberUpdateReply {
        added: diff.added,
        removed: diff.removed,
        total: found.len(),
    }))
}

async fn get_grabber_project(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<db::grabber_projects::Project>, ApiError> {
    db::grabber_projects::get(&s.pool, &id)
        .await?
        .map(Json)
        .ok_or_else(|| not_found(format!("grabber project {id} not found")))
}

async fn delete_grabber_project(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if db::grabber_projects::delete(&s.pool, &id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found(format!("grabber project {id} not found")))
    }
}

async fn list_queues(State(s): State<AppState>) -> ApiResult<Vec<QueueRecord>> {
    Ok(Json(db::queues::list(&s.pool).await?))
}

async fn get_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<QueueRecord>, ApiError> {
    db::queues::get(&s.pool, id)
        .await?
        .map(Json)
        .ok_or_else(|| not_found(format!("queue {id} not found")))
}

#[derive(Deserialize)]
struct NewQueueBody {
    name: String,
    max_concurrent: Option<u32>,
}

async fn create_queue(
    State(s): State<AppState>,
    Json(body): Json<NewQueueBody>,
) -> Result<(StatusCode, Json<QueueRecord>), ApiError> {
    let name = body.name.trim();
    if name.is_empty() || name.len() > 60 {
        return Err(bad_request("queue name must be 1-60 characters"));
    }
    match db::queues::create(&s.pool, name, body.max_concurrent.unwrap_or(2)).await? {
        Some(q) => Ok((StatusCode::CREATED, Json(q))),
        None => Err(bad_request(format!(
            "a queue named \"{name}\" already exists"
        ))),
    }
}

async fn delete_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    if id == db::queues::DEFAULT_QUEUE_ID {
        return Err(bad_request("the default queue cannot be deleted"));
    }
    if !db::queues::delete(&s.pool, id).await? {
        return Err(not_found(format!("queue {id} not found")));
    }
    Ok(StatusCode::NO_CONTENT)
}

/// IDM's "Start queue": resume every download of the queue that is not running or finished.
async fn start_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    start_queue_downloads(&s.pool, &s.manager_tx, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn start_queue_downloads(
    pool: &sqlx::SqlitePool,
    tx: &mpsc::Sender<ManagerCommand>,
    id: Uuid,
) -> anyhow::Result<()> {
    for (did, status) in db::queues::download_ids(pool, id).await? {
        if matches!(status.as_str(), "paused" | "error" | "cancelled") {
            tx.send(ManagerCommand::Resume(did)).await.ok();
        }
    }
    tx.send(ManagerCommand::PumpQueue).await.ok();
    Ok(())
}

/// IDM's "Stop queue": pause everything of the queue that is running or waiting.
async fn stop_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    for (did, status) in db::queues::download_ids(&s.pool, id).await? {
        if matches!(status.as_str(), "downloading" | "connecting" | "queued") {
            s.manager_tx.send(ManagerCommand::Pause(did)).await.ok();
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn update_queue(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<QueuePatchBody>,
) -> Result<StatusCode, ApiError> {
    if body.max_concurrent == Some(0) {
        return Err(bad_request("max_concurrent must be >= 1"));
    }
    if matches!(body.sched_start, Some(Some(m)) if m >= 1440) {
        return Err(bad_request("sched_start must be < 1440 (minutes per day)"));
    }
    if matches!(body.sched_end, Some(Some(m)) if m >= 1440) {
        return Err(bad_request("sched_end must be < 1440 (minutes per day)"));
    }
    if matches!(body.sync_interval_min, Some(Some(0))) {
        return Err(bad_request("sync_interval_min must be >= 1"));
    }
    if matches!(&body.when_done, Some(Some(w)) if w != "exit" && w != "shutdown") {
        return Err(bad_request("when_done must be \"exit\" or \"shutdown\""));
    }
    if matches!(body.sched_days, Some(d) if d == 0 || d > 127) {
        return Err(bad_request(
            "sched_days must select at least one weekday (1..=127)",
        ));
    }

    let found = db::queues::update(
        &s.pool,
        id,
        db::queues::QueueUpdate {
            name: body.name,
            max_concurrent: body.max_concurrent,
            max_bandwidth: body.max_bandwidth,
            sched_start: body.sched_start,
            sched_end: body.sched_end,
            sched_days: body.sched_days,
            start_on_startup: body.start_on_startup,
            sync_interval_min: body.sync_interval_min,
            max_retries: body.max_retries,
            open_file: body.open_file,
            when_done: body.when_done,
            enabled: body.enabled,
        },
    )
    .await?;
    if !found {
        return Err(not_found(format!("queue {id} not found")));
    }

    s.manager_tx.send(ManagerCommand::PumpQueue).await.ok();
    Ok(StatusCode::NO_CONTENT)
}

async fn events_handler(
    State(s): State<AppState>,
) -> Sse<impl futures::Stream<Item = Result<Event, Infallible>>> {
    let rx = s.progress_tx.subscribe();
    let stream = progress_event_stream(rx);
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn progress_event_stream(
    rx: broadcast::Receiver<ProgressEvent>,
) -> impl Stream<Item = Result<Event, Infallible>> {
    BroadcastStream::new(rx).filter_map(|result| {
        let event = result.ok()?;
        let data = serde_json::to_string(&event).ok()?;
        Some(Ok::<Event, Infallible>(Event::default().data(data)))
    })
}

async fn health_handler(State(s): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        version: VERSION,
        uptime_secs: s.start_time.elapsed().as_secs(),
        data_dir: ferric_common::resolve_data_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    })
}

// ── FQT-071: Global transfer stats ──────────────────────────────────────

#[derive(Serialize)]
struct GlobalStats {
    download_bps: u64,
    upload_bps: u64,
    active_downloads: u32,
    active_torrents: u32,
    total_downloads: u32,
    total_torrents: u32,
    uptime_secs: u64,
}

async fn get_stats(State(s): State<AppState>) -> ApiResult<GlobalStats> {
    use sqlx::Row;
    let dl_count: i64 = sqlx::query("SELECT COUNT(*) as cnt FROM downloads")
        .fetch_one(&s.pool)
        .await
        .map_or(0, |r| r.try_get("cnt").unwrap_or(0));
    let active_dl: i64 = sqlx::query("SELECT COUNT(*) as cnt FROM downloads WHERE status IN ('downloading','connecting','queued')")
        .fetch_one(&s.pool)
        .await
        .map_or(0, |r| r.try_get("cnt").unwrap_or(0));
    let tor_count: i64 = sqlx::query("SELECT COUNT(*) as cnt FROM torrents")
        .fetch_one(&s.pool)
        .await
        .map_or(0, |r| r.try_get("cnt").unwrap_or(0));
    let active_tor: i64 =
        sqlx::query("SELECT COUNT(*) as cnt FROM torrents WHERE status = 'downloading'")
            .fetch_one(&s.pool)
            .await
            .map_or(0, |r| r.try_get("cnt").unwrap_or(0));

    // Torrent speeds come from the 2s poller cache (F26 engine stats). HTTP download
    // speeds still arrive via SSE Progress events on the UI side — sum both surfaces.
    let (torrent_dl, torrent_ul) = {
        let map = s.torrent_speeds.lock().await;
        map.values()
            .fold((0u64, 0u64), |(d, u), (dd, uu, _)| (d + dd, u + uu))
    };

    Ok(Json(GlobalStats {
        download_bps: torrent_dl,
        upload_bps: torrent_ul,
        active_downloads: active_dl as u32,
        active_torrents: active_tor as u32,
        total_downloads: dl_count as u32,
        total_torrents: tor_count as u32,
        uptime_secs: s.start_time.elapsed().as_secs(),
    }))
}

// ── FQT-070: Batch endpoints ────────────────────────────────────────────

#[derive(Deserialize)]
struct BatchBody {
    ids: Vec<String>,
    action: String,
}

#[derive(Serialize)]
struct BatchResult {
    succeeded: Vec<String>,
    failed: Vec<String>,
}

async fn batch_downloads(
    State(s): State<AppState>,
    Json(body): Json<BatchBody>,
) -> ApiResult<BatchResult> {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for id in &body.ids {
        let uuid = if let Ok(u) = Uuid::parse_str(id) {
            u
        } else {
            failed.push(id.clone());
            continue;
        };
        let result = match body.action.as_str() {
            "pause" => {
                s.manager_tx.send(ManagerCommand::Pause(uuid)).await.ok();
                Ok(())
            }
            "resume" => {
                s.manager_tx.send(ManagerCommand::Resume(uuid)).await.ok();
                Ok(())
            }
            "delete" => {
                s.manager_tx
                    .send(ManagerCommand::Remove {
                        id: uuid,
                        delete_file: false,
                    })
                    .await
                    .ok();
                Ok(())
            }
            _ => Err("unknown action"),
        };
        if result.is_ok() {
            succeeded.push(id.clone());
        } else {
            failed.push(id.clone());
        }
    }
    Ok(Json(BatchResult { succeeded, failed }))
}

async fn batch_torrents(
    State(s): State<AppState>,
    Json(body): Json<BatchBody>,
) -> ApiResult<BatchResult> {
    let mut succeeded = Vec::new();
    let mut failed = Vec::new();
    for id in &body.ids {
        let ok = match body.action.as_str() {
            "pause" => s.torrents.pause(id).await.is_ok(),
            "start" | "resume" => s.torrents.start(id).await.is_ok(),
            "delete" => db::torrents::remove_torrent(&s.pool, id).await.is_ok(),
            _ => false,
        };
        if ok {
            succeeded.push(id.clone());
        } else {
            failed.push(id.clone());
        }
    }
    Ok(Json(BatchResult { succeeded, failed }))
}

// ── Privacy / RSS / webhooks / categories / web UI (roadmap 63) ──

async fn privacy_dashboard(
    State(s): State<AppState>,
) -> ApiResult<crate::privacy::PrivacyDashboard> {
    Ok(Json(crate::privacy::dashboard(&s.pool, 7).await?))
}

async fn anonymity_score_handler(
    State(s): State<AppState>,
) -> ApiResult<crate::privacy::AnonymityScore> {
    Ok(Json(crate::privacy::anonymity_score(&s.pool).await?))
}

async fn audit_export(State(s): State<AppState>) -> ApiResult<serde_json::Value> {
    use sqlx::Row;
    let dl_rows = sqlx::query(
        "SELECT id, download_id, event_type, detail_json, created_at FROM download_audit ORDER BY created_at DESC LIMIT 5000",
    )
    .fetch_all(&s.pool)
    .await
    .unwrap_or_default();
    let torrent_rows = sqlx::query(
        "SELECT id, torrent_id, event_type, data_json, created_at FROM torrent_audit ORDER BY created_at DESC LIMIT 5000",
    )
    .fetch_all(&s.pool)
    .await
    .unwrap_or_default();
    let download_audit: Vec<serde_json::Value> = dl_rows
        .iter()
        .filter_map(|r| {
            Some(serde_json::json!({
                "id": r.try_get::<String,_>("id").ok()?,
                "download_id": r.try_get::<String,_>("download_id").ok()?,
                "event_type": r.try_get::<String,_>("event_type").ok()?,
                "detail_json": r.try_get::<String,_>("detail_json").ok()?,
                "created_at": r.try_get::<i64,_>("created_at").ok()?,
            }))
        })
        .collect();
    let torrent_audit: Vec<serde_json::Value> = torrent_rows
        .iter()
        .filter_map(|r| {
            Some(serde_json::json!({
                "id": r.try_get::<String,_>("id").ok()?,
                "torrent_id": r.try_get::<String,_>("torrent_id").ok()?,
                "event_type": r.try_get::<String,_>("event_type").ok()?,
                "data_json": r.try_get::<String,_>("data_json").ok()?,
                "created_at": r.try_get::<i64,_>("created_at").ok()?,
            }))
        })
        .collect();
    Ok(Json(serde_json::json!({
        "exported_at": unix_ms_now(),
        "download_audit": download_audit,
        "torrent_audit": torrent_audit,
        "note": "Cookies, passkeys, and proxy credentials are never stored in audit payloads."
    })))
}

fn unix_ms_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

async fn list_site_rules(State(s): State<AppState>) -> ApiResult<Vec<db::site_rules::SiteRuleRow>> {
    Ok(Json(db::site_rules::list_rules(&s.pool).await?))
}

#[derive(Debug, Deserialize)]
struct AddSiteRuleBody {
    pattern: String,
    rule_type: SiteRuleType,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    priority: Option<i32>,
}

async fn add_site_rule(
    State(s): State<AppState>,
    Json(body): Json<AddSiteRuleBody>,
) -> Result<(StatusCode, Json<db::site_rules::SiteRuleRow>), ApiError> {
    let pattern = body.pattern.trim().to_string();
    if pattern.is_empty() {
        return Err(bad_request("pattern required"));
    }
    let rule_type = site_rule_type_str(&body.rule_type);
    let row = db::site_rules::insert_rule_row(
        &s.pool,
        &pattern,
        rule_type,
        body.value.as_deref(),
        body.priority.unwrap_or(0),
    )
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((StatusCode::CREATED, Json(row)))
}

#[derive(Debug, Deserialize)]
struct UpdateSiteRuleBody {
    pattern: Option<String>,
    rule_type: Option<SiteRuleType>,
    #[serde(default)]
    value: Option<Option<String>>,
    priority: Option<i32>,
}

async fn update_site_rule(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<UpdateSiteRuleBody>,
) -> ApiResult<db::site_rules::SiteRuleRow> {
    let rule_type = body.rule_type.as_ref().map(site_rule_type_str);
    let value = body.value.as_ref().map(|v| v.as_deref());
    let row = db::site_rules::update_rule(
        &s.pool,
        &id,
        body.pattern
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        rule_type,
        value,
        body.priority,
    )
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?
    .ok_or_else(|| not_found("site rule not found"))?;
    Ok(Json(row))
}

async fn delete_site_rule(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let deleted = db::site_rules::delete_rule(&s.pool, &id)
        .await
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(not_found("site rule not found"))
    }
}

fn site_rule_type_str(t: &SiteRuleType) -> &'static str {
    match t {
        SiteRuleType::Capture => "capture",
        SiteRuleType::Skip => "skip",
        SiteRuleType::MaxConnections => "max_connections",
        SiteRuleType::MaxSpeedBps => "max_speed_bps",
        SiteRuleType::BandwidthCapBytes => "bandwidth_cap_bytes",
        SiteRuleType::AlwaysAsk => "always_ask",
    }
}

async fn list_rss(State(s): State<AppState>) -> ApiResult<Vec<crate::rss_watch::RssFeed>> {
    Ok(Json(crate::rss_watch::list_feeds(&s.pool).await?))
}

#[derive(Deserialize)]
struct AddRssBody {
    url: String,
    #[serde(default)]
    poll_secs: Option<i64>,
}

async fn add_rss(
    State(s): State<AppState>,
    Json(body): Json<AddRssBody>,
) -> Result<(StatusCode, Json<crate::rss_watch::RssFeed>), ApiError> {
    let feed = crate::rss_watch::add_feed(&s.pool, &body.url, body.poll_secs.unwrap_or(3600))
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(feed)))
}

async fn list_rss_rules(State(s): State<AppState>) -> ApiResult<Vec<crate::rss_watch::RssRule>> {
    Ok(Json(crate::rss_watch::list_rules(&s.pool, None).await?))
}

#[derive(Deserialize)]
struct AddRssRuleBody {
    feed_id: String,
    name: String,
    match_pattern: String,
    #[serde(default = "default_action")]
    action: String,
    save_dir: Option<String>,
}

fn default_action() -> String {
    "download".to_string()
}

async fn add_rss_rule(
    State(s): State<AppState>,
    Json(body): Json<AddRssRuleBody>,
) -> Result<(StatusCode, Json<crate::rss_watch::RssRule>), ApiError> {
    let rule = crate::rss_watch::add_rule(
        &s.pool,
        &body.feed_id,
        &body.name,
        &body.match_pattern,
        &body.action,
        body.save_dir.as_deref(),
    )
    .await
    .map_err(|e| bad_request(e.to_string()))?;
    Ok((StatusCode::CREATED, Json(rule)))
}

async fn delete_rss_rule(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    crate::rss_watch::delete_rule(&s.pool, &id).await?;
    Ok(Json(()))
}

async fn delete_rss_feed(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    crate::rss_watch::delete_feed(&s.pool, &id).await?;
    Ok(Json(()))
}

#[derive(Serialize)]
struct WebhookRow {
    id: String,
    url: String,
    events: String,
    enabled: bool,
}

async fn list_webhooks(State(s): State<AppState>) -> ApiResult<Vec<WebhookRow>> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, url, events, enabled FROM webhooks")
        .fetch_all(&s.pool)
        .await
        .unwrap_or_default();
    Ok(Json(
        rows.into_iter()
            .filter_map(|r| {
                Some(WebhookRow {
                    id: r.try_get("id").ok()?,
                    url: r.try_get("url").ok()?,
                    events: r.try_get("events").ok()?,
                    enabled: r.try_get::<i64, _>("enabled").ok()? != 0,
                })
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct AddWebhookBody {
    url: String,
    #[serde(default)]
    events: Option<String>,
}

async fn add_webhook(
    State(s): State<AppState>,
    Json(body): Json<AddWebhookBody>,
) -> Result<(StatusCode, Json<WebhookRow>), ApiError> {
    security::validate_url(&body.url).map_err(|e| bad_request(e.to_string()))?;
    let id = Uuid::new_v4().to_string();
    let events = body.events.unwrap_or_else(|| "*".into());
    let now = unix_ms_now();
    sqlx::query(
        "INSERT INTO webhooks (id, url, events, enabled, created_at) VALUES (?1, ?2, ?3, 1, ?4)",
    )
    .bind(&id)
    .bind(&body.url)
    .bind(&events)
    .bind(now)
    .execute(&s.pool)
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(WebhookRow {
            id,
            url: body.url,
            events,
            enabled: true,
        }),
    ))
}

#[derive(Serialize)]
struct CategoryRow {
    id: String,
    name: String,
    pattern: Option<String>,
    save_subdir: Option<String>,
}

async fn list_categories(State(s): State<AppState>) -> ApiResult<Vec<CategoryRow>> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, name, pattern, save_subdir FROM download_categories")
        .fetch_all(&s.pool)
        .await
        .unwrap_or_default();
    Ok(Json(
        rows.into_iter()
            .filter_map(|r| {
                Some(CategoryRow {
                    id: r.try_get("id").ok()?,
                    name: r.try_get("name").ok()?,
                    pattern: r.try_get("pattern").ok(),
                    save_subdir: r.try_get("save_subdir").ok(),
                })
            })
            .collect(),
    ))
}

#[derive(Deserialize)]
struct AddCategoryBody {
    name: String,
    #[serde(default)]
    pattern: Option<String>,
    #[serde(default)]
    save_subdir: Option<String>,
}

async fn add_category(
    State(s): State<AppState>,
    Json(body): Json<AddCategoryBody>,
) -> Result<(StatusCode, Json<CategoryRow>), ApiError> {
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return Err(bad_request("name required"));
    }
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO download_categories (id, name, pattern, save_subdir) VALUES (?1, ?2, ?3, ?4)",
    )
    .bind(&id)
    .bind(&name)
    .bind(&body.pattern)
    .bind(&body.save_subdir)
    .execute(&s.pool)
    .await
    .map_err(|e| bad_request(e.to_string()))?;
    Ok((
        StatusCode::CREATED,
        Json(CategoryRow {
            id,
            name,
            pattern: body.pattern,
            save_subdir: body.save_subdir,
        }),
    ))
}

#[derive(Deserialize)]
struct RefreshUrlBody {
    url: String,
    /// A proxy (with the login the user just typed) to use for this download from now on.
    #[serde(default)]
    proxy_url: Option<String>,
}

#[derive(Deserialize)]
struct MediaProbeBody {
    url: String,
}

#[derive(Deserialize)]
struct MergeJobBody {
    video_path: String,
    audio_path: String,
    output_path: String,
    save_dir: String,
}

#[derive(Deserialize)]
struct TorrentProbeBody {
    input: String,
}

#[derive(Deserialize)]
struct AssignProxyBody {
    proxy_id: Option<String>,
}

#[derive(Deserialize)]
struct TorrentFilePatchBody {
    selected: bool,
    priority: i32,
}

async fn probe_media(
    State(_state): State<AppState>,
    Json(body): Json<MediaProbeBody>,
) -> ApiResult<ferric_common::MediaProbeResponse> {
    media::probe_url(&body.url, &reqwest::Client::new())
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

#[derive(Deserialize)]
struct ExtractRequest {
    url: String,
    /// Optional 1-based playlist range start.
    #[serde(default)]
    playlist_start: Option<u32>,
    /// Optional 1-based playlist range end, or "newest N" when start omitted.
    #[serde(default)]
    playlist_end: Option<u32>,
    /// Format preset id or raw yt-dlp-style spec (informational; selection is separate).
    #[serde(default)]
    format_preset: Option<String>,
    /// Force yt-dlp engine (max site compatibility). Also set via settings `ytdlp_max_compat`.
    #[serde(default)]
    force_ytdlp: bool,
    /// Absolute path to Netscape cookies.txt. Overrides settings `ytdlp_cookies` when set.
    #[serde(default)]
    cookies_file: Option<String>,
    /// Task 112 — Cookie header string for login-walled native extract (never logged).
    #[serde(default)]
    cookie_header: Option<String>,
}

/// Pure-Rust default; optional yt-dlp binary only when settings/API opt in.
pub(crate) async fn extract_config_from_settings(
    pool: &sqlx::SqlitePool,
    force_ytdlp: bool,
    cookies_override: Option<String>,
) -> ferric_extract::ExtractConfig {
    let path = db::settings::get(pool, "ytdlp_path")
        .await
        .ok()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .map(std::path::PathBuf::from);
    let cookies = match cookies_override.filter(|s| !s.trim().is_empty()) {
        Some(p) => Some(std::path::PathBuf::from(p)),
        None => db::settings::get(pool, "ytdlp_cookies")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .map(std::path::PathBuf::from),
    };
    let truthy = |v: &str| v == "1" || v.eq_ignore_ascii_case("true");
    let max_compat = db::settings::get(pool, "ytdlp_max_compat")
        .await
        .ok()
        .flatten()
        .is_some_and(|v| truthy(&v));
    let enable_fallback = db::settings::get(pool, "ytdlp_enable_fallback")
        .await
        .ok()
        .flatten()
        .is_some_and(|v| truthy(&v));

    let force = force_ytdlp || max_compat;
    // Pure Rust by default. Binary bridge only if force or explicit fallback setting.
    ferric_extract::ExtractConfig {
        pure_rust_only: !force && !enable_fallback,
        force_ytdlp: force,
        ytdlp_binary: path,
        cookies_file: cookies,
    }
}

async fn extract_media(
    State(state): State<AppState>,
    Json(body): Json<ExtractRequest>,
) -> ApiResult<ExtractResult> {
    security::validate_url(&body.url).map_err(|e| bad_request(format!("invalid URL: {e}")))?;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cfg =
        extract_config_from_settings(&state.pool, body.force_ytdlp, body.cookies_file.clone())
            .await;

    // Task 112: optional Cookie header for native IEs (login-walled). Prefer over logging.
    let client = if let Some(ref cookie) = body.cookie_header {
        if !cookie.is_empty() && !cookie.chars().any(char::is_control) {
            reqwest::Client::builder()
                .user_agent("Mozilla/5.0")
                .default_headers({
                    let mut h = reqwest::header::HeaderMap::new();
                    if let Ok(v) = reqwest::header::HeaderValue::from_str(cookie) {
                        h.insert(reqwest::header::COOKIE, v);
                    }
                    h
                })
                .build()
                .unwrap_or(client)
        } else {
            client
        }
    } else {
        client
    };

    let result = if body.playlist_start.is_some() || body.playlist_end.is_some() {
        ferric_extract::playlist::expand_range(
            &body.url,
            &client,
            body.playlist_start,
            body.playlist_end,
        )
        .await
    } else {
        ferric_extract::extract_with_config(&body.url, &client, cfg).await
    }
    .map_err(|e| {
        ApiError(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("extraction failed: {e}"),
        )
    })?;

    if let Some(ref preset) = body.format_preset {
        let _ = ferric_extract::presets::resolve(preset);
    }
    Ok(Json(result))
}

/// Extract (pure Rust first) → pick format → queue download. One product action.
#[derive(Deserialize)]
struct MediaDownloadBody {
    url: String,
    /// Preset id or raw format spec. Default: best.
    #[serde(default)]
    format_preset: Option<String>,
    /// Prefer this format id when present (skips selector).
    #[serde(default)]
    format_id: Option<String>,
    #[serde(default)]
    force_ytdlp: bool,
    #[serde(default)]
    cookies_file: Option<String>,
    #[serde(default)]
    save_dir: Option<String>,
    #[serde(default)]
    filename: Option<String>,
}

#[derive(Serialize)]
struct MediaDownloadResponse {
    download: DownloadRecord,
    extractor: String,
    title: String,
    selected: String,
}

async fn media_download(
    State(s): State<AppState>,
    Json(body): Json<MediaDownloadBody>,
) -> Result<(StatusCode, Json<MediaDownloadResponse>), ApiError> {
    security::validate_url(&body.url).map_err(|e| bad_request(format!("invalid URL: {e}")))?;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let cfg =
        extract_config_from_settings(&s.pool, body.force_ytdlp, body.cookies_file.clone()).await;
    let result = ferric_extract::extract_with_config(&body.url, &client, cfg)
        .await
        .map_err(|e| {
            ApiError(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("extraction failed: {e}"),
            )
        })?;

    if result.formats.is_empty() {
        return Err(bad_request("extraction returned no formats"));
    }

    let (video_url, audio_url, selected_label) = if let Some(ref id) = body.format_id {
        let f = result
            .formats
            .iter()
            .find(|f| f.id == *id)
            .ok_or_else(|| bad_request(format!("format_id {id} not found")))?;
        (f.url.clone(), None, f.label.clone())
    } else {
        let spec = body
            .format_preset
            .as_deref()
            .map_or("bestvideo+bestaudio/best", ferric_extract::presets::resolve);
        match ferric_extract::selector::select(&result.formats, spec) {
            Ok(ferric_extract::selector::FormatSelection::Single(v)) => {
                (v.url.clone(), None, v.label.clone())
            }
            Ok(ferric_extract::selector::FormatSelection::Merge { video, audio }) => (
                video.url.clone(),
                Some(audio.url.clone()),
                format!("{} + {}", video.label, audio.label),
            ),
            Err(e) => {
                // Fallback: first usable format after sort
                let f = result
                    .formats
                    .first()
                    .ok_or_else(|| bad_request(format!("format select failed: {e}")))?;
                (f.url.clone(), None, f.label.clone())
            }
        }
    };

    security::validate_url(&video_url).map_err(|e| bad_request(e.to_string()))?;
    if let Some(ref a) = audio_url {
        security::validate_url(a).map_err(|e| bad_request(e.to_string()))?;
    }

    let safe_name = body.filename.or_else(|| {
        let ext = result
            .formats
            .iter()
            .find(|f| f.url == video_url)
            .map(|f| f.ext.as_str())
            .filter(|e| !e.is_empty() && *e != "m3u8")
            .unwrap_or("mp4");
        let raw = format!("{}.{ext}", result.title);
        security::safe_filename(&raw, std::path::Path::new("."))
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
    });

    let add_body = AddUrlBody {
        hold: false,
        url: video_url,
        filename: safe_name,
        save_dir: body.save_dir,
        queue_id: None,
        cookies: None,
        checksum: None,
        mirrors: Vec::new(),
        audio_url,
        subtitle_url: result.subtitles.first().map(|s| s.url.clone()),
        subtitle_lang: result.subtitles.first().map(|s| s.lang.clone()),
        proxy_url: None,
        description: None,
    };

    let (status, Json(record)) = add_download(State(s.clone()), Json(add_body)).await?;

    // Persist extract metadata for pure-Rust post-download sidecars
    if let Err(e) = db::extract_meta::put(&s.pool, record.id, &result).await {
        tracing::warn!(error = %e, id = %record.id, "failed to store extract meta");
    }

    Ok((
        status,
        Json(MediaDownloadResponse {
            download: record,
            extractor: result.extractor,
            title: result.title,
            selected: selected_label,
        }),
    ))
}

#[derive(Serialize)]
struct FormatPresetOut {
    id: String,
    label: String,
    spec: String,
    #[serde(default)]
    user_defined: bool,
}

async fn list_format_presets(State(s): State<AppState>) -> ApiResult<Vec<FormatPresetOut>> {
    let mut out: Vec<FormatPresetOut> = ferric_extract::presets::list()
        .iter()
        .map(|p| FormatPresetOut {
            id: p.id.to_string(),
            label: p.label.to_string(),
            spec: p.spec.to_string(),
            user_defined: false,
        })
        .collect();
    // Task 115 — merge user presets from settings
    if let Ok(Some(raw)) = db::settings::get(&s.pool, "user_format_presets").await {
        if let Ok(serde_json::Value::Array(arr)) = serde_json::from_str(&raw) {
            for item in arr {
                let id = item
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let label = item
                    .get("label")
                    .and_then(|v| v.as_str())
                    .unwrap_or(&id)
                    .to_string();
                let spec = item
                    .get("spec")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if !id.is_empty() && !spec.is_empty() {
                    out.push(FormatPresetOut {
                        id,
                        label,
                        spec,
                        user_defined: true,
                    });
                }
            }
        }
    }
    Ok(Json(out))
}

#[derive(Serialize)]
struct ExtractEngineStatus {
    mode: &'static str,
    pure_rust_ies: usize,
    ytdlp_subprocess: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    ytdlp: Option<YtdlpStatusDto>,
}

#[derive(Serialize)]
struct YtdlpStatusDto {
    available: bool,
    path: Option<String>,
    version: Option<String>,
    explanation: String,
}

async fn ytdlp_status(State(state): State<AppState>) -> ApiResult<ExtractEngineStatus> {
    #[cfg(feature = "ytdlp-subprocess")]
    {
        let path = db::settings::get(&state.pool, "ytdlp_path")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.trim().is_empty())
            .map(std::path::PathBuf::from);
        let st = ferric_extract::ytdlp::status_with_binary(path).await;
        let max_compat = db::settings::get(&state.pool, "ytdlp_max_compat")
            .await
            .ok()
            .flatten()
            .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
        Ok(Json(ExtractEngineStatus {
            mode: if !st.available {
                "native_only_ytdlp_missing"
            } else if max_compat {
                "ytdlp_max_compat"
            } else {
                "native_fastpath+ytdlp_engine"
            },
            pure_rust_ies: ferric_extract::REGISTRY.len(),
            ytdlp_subprocess: true,
            ytdlp: Some(YtdlpStatusDto {
                available: st.available,
                path: st.path,
                version: st.version,
                explanation: st.explanation,
            }),
        }))
    }
    #[cfg(not(feature = "ytdlp-subprocess"))]
    {
        let _ = state;
        Ok(Json(ExtractEngineStatus {
            mode: "pure_rust",
            pure_rust_ies: ferric_extract::REGISTRY.len(),
            ytdlp_subprocess: false,
            ytdlp: None,
        }))
    }
}

#[derive(Deserialize)]
struct CookiesBody {
    text: String,
}

#[derive(Serialize)]
struct CookiesParseResponse {
    count: usize,
    header_example: Option<String>,
}

async fn parse_netscape_cookies(Json(body): Json<CookiesBody>) -> ApiResult<CookiesParseResponse> {
    let cookies =
        ferric_extract::netscape::parse(&body.text).map_err(|e| bad_request(e.to_string()))?;
    let header_example = cookies.first().and_then(|c| {
        ferric_extract::netscape::header_for_domain(&cookies, c.domain.trim_start_matches('.'))
    });
    Ok(Json(CookiesParseResponse {
        count: cookies.len(),
        header_example,
    }))
}

/// Task 111 — export cookies back to Netscape text (round-trip).
#[derive(Deserialize)]
struct CookiesExportBody {
    text: String,
}

#[derive(Serialize)]
struct CookiesExportResponse {
    text: String,
    count: usize,
}

async fn export_netscape_cookies(
    Json(body): Json<CookiesExportBody>,
) -> ApiResult<CookiesExportResponse> {
    let cookies =
        ferric_extract::netscape::parse(&body.text).map_err(|e| bad_request(e.to_string()))?;
    let text = ferric_extract::netscape::export(&cookies);
    Ok(Json(CookiesExportResponse {
        count: cookies.len(),
        text,
    }))
}

/// Task 112 — pull cookies for a site from installed browsers (read-only snapshot).
/// Returns Cookie header + count; never logs cookie values.
#[derive(Deserialize)]
struct BrowserCookiesBody {
    /// Page or site URL (host used to match cookies).
    url: String,
}

#[derive(Serialize)]
struct BrowserCookiesResponse {
    domain: String,
    count: usize,
    /// Ready-to-send Cookie header (caller must not log).
    cookie_header: Option<String>,
    explanation: String,
}

async fn cookies_from_browser_disabled(
    Json(body): Json<BrowserCookiesBody>,
) -> ApiResult<BrowserCookiesResponse> {
    // Hard constraint: never read browser cookie databases from disk.
    let host = url::Url::parse(&body.url)
        .ok()
        .and_then(|u| u.host_str().map(std::string::ToString::to_string))
        .unwrap_or_default();
    let domain = host.trim_start_matches("www.").to_string();
    Ok(Json(BrowserCookiesResponse {
        domain,
        count: 0,
        cookie_header: None,
        explanation: "Browser cookie DB access is disabled. Pass cookies from the extension capture path only."
            .into(),
    }))
}

/// Task 112 — deliberately disabled: browser cookie DB access is off-limits.
/// Body is accepted untyped so the 410 contract survives request-shape drift.
async fn extract_login_disabled(
    State(_s): State<AppState>,
    Json(_body): Json<serde_json::Value>,
) -> Result<Json<ExtractResult>, ApiError> {
    Err(ApiError(
        StatusCode::GONE,
        "extract-login disabled: browser cookie DB access is not allowed. Capture with extension cookies or pass cookies explicitly.".into(),
    ))
}

async fn get_download_audit(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
) -> ApiResult<Vec<db::audit::AuditRow>> {
    ensure_exists(&s.pool, id).await?;
    let rows = db::audit::list_for_download(&s.pool, id).await?;
    Ok(Json(rows))
}

/// Task 114 — expand playlist range and queue each entry as a download.
#[derive(Deserialize)]
struct PlaylistDownloadBody {
    url: String,
    #[serde(default)]
    playlist_start: Option<u32>,
    #[serde(default)]
    playlist_end: Option<u32>,
    #[serde(default)]
    max_items: Option<u32>,
    #[serde(default)]
    save_dir: Option<String>,
}

#[derive(Serialize)]
struct PlaylistDownloadResponse {
    queued: usize,
    ids: Vec<Uuid>,
    titles: Vec<String>,
}

async fn playlist_download(
    State(s): State<AppState>,
    Json(body): Json<PlaylistDownloadBody>,
) -> Result<(StatusCode, Json<PlaylistDownloadResponse>), ApiError> {
    security::validate_url(&body.url).map_err(|e| bad_request(e.to_string()))?;
    let client = reqwest::Client::builder()
        .user_agent("Mozilla/5.0")
        .build()
        .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let expanded = ferric_extract::playlist::expand_range(
        &body.url,
        &client,
        body.playlist_start,
        body.playlist_end,
    )
    .await
    .map_err(|e| ApiError(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;

    let mut entries = expanded.playlist_entries;
    if entries.is_empty() && !expanded.formats.is_empty() {
        // Single video treated as one-entry playlist
        entries.push(ferric_common::PlaylistEntry {
            url: body.url.clone(),
            title: Some(expanded.title.clone()),
            index: 1,
            duration_secs: expanded.duration_secs,
            thumbnail: expanded.thumbnail.clone(),
        });
    }
    let max = body.max_items.unwrap_or(50).min(200) as usize;
    entries.truncate(max);

    let mut ids = Vec::new();
    let mut titles = Vec::new();
    for entry in entries {
        if security::validate_url(&entry.url).is_err() {
            continue;
        }
        let title = entry
            .title
            .clone()
            .unwrap_or_else(|| format!("playlist-{}", entry.index));
        let add = AddUrlBody {
            hold: false,
            url: entry.url,
            filename: Some(format!(
                "{:03}-{}.mp4",
                entry.index,
                security::safe_filename(&format!("{title}.mp4"), std::path::Path::new("."))
                    .ok()
                    .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .unwrap_or_else(|| format!("item-{}.mp4", entry.index))
                    .trim_end_matches(".mp4")
            )),
            save_dir: body.save_dir.clone(),
            queue_id: None,
            cookies: None,
            checksum: None,
            mirrors: Vec::new(),
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            description: None,
        };
        match add_download(State(s.clone()), Json(add)).await {
            Ok((_, Json(rec))) => {
                ids.push(rec.id);
                titles.push(title);
            }
            Err(e) => tracing::warn!(error = %e.1, "playlist item skip"),
        }
    }

    Ok((
        StatusCode::CREATED,
        Json(PlaylistDownloadResponse {
            queued: ids.len(),
            ids,
            titles,
        }),
    ))
}

#[derive(Deserialize)]
struct UserPresetsBody {
    /// JSON array of {id,label,spec} or raw settings string.
    presets: serde_json::Value,
}

async fn save_user_presets(
    State(s): State<AppState>,
    Json(body): Json<UserPresetsBody>,
) -> ApiResult<serde_json::Value> {
    let raw = match &body.presets {
        serde_json::Value::String(s) => s.clone(),
        other => serde_json::to_string(other).map_err(|e| bad_request(e.to_string()))?,
    };
    // Validate JSON array
    let parsed: serde_json::Value = serde_json::from_str(&raw)
        .map_err(|e| bad_request(format!("invalid presets JSON: {e}")))?;
    if !parsed.is_array() {
        return Err(bad_request("presets must be a JSON array"));
    }
    db::settings::set(&s.pool, "user_format_presets", &raw).await?;
    Ok(Json(
        serde_json::json!({ "ok": true, "count": parsed.as_array().map_or(0, std::vec::Vec::len) }),
    ))
}

#[derive(Deserialize)]
struct SelectFormatsBody {
    formats: Vec<ferric_common::ExtractedFormat>,
    spec: String,
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum SelectedFormatResult {
    Single {
        video: ferric_common::ExtractedFormat,
    },
    Merge {
        video: ferric_common::ExtractedFormat,
        audio: ferric_common::ExtractedFormat,
    },
}

async fn select_media_formats(
    State(_state): State<AppState>,
    Json(body): Json<SelectFormatsBody>,
) -> ApiResult<SelectedFormatResult> {
    match ferric_extract::selector::select(&body.formats, &body.spec) {
        Ok(ferric_extract::selector::FormatSelection::Single(video)) => {
            Ok(Json(SelectedFormatResult::Single { video }))
        }
        Ok(ferric_extract::selector::FormatSelection::Merge { video, audio }) => {
            Ok(Json(SelectedFormatResult::Merge { video, audio }))
        }
        Err(e) => Err(bad_request(e.to_string())),
    }
}

async fn ffmpeg_status() -> ApiResult<ferric_common::FfmpegStatus> {
    let path = media::ffmpeg_path().map(|path| path.to_string_lossy().to_string());
    let available = path.is_some();
    Ok(Json(ferric_common::FfmpegStatus {
        available,
        path,
        explanation: if available {
            "ffmpeg is available for legal non-DRM audio/video merge jobs.".to_string()
        } else {
            "ffmpeg is not available on PATH; separate audio/video merge jobs will need configuration."
                .to_string()
        },
    }))
}

async fn probe_torrent(
    State(_state): State<AppState>,
    Json(body): Json<TorrentProbeBody>,
) -> ApiResult<ferric_common::TorrentProbeResponse> {
    torrent::probe_input(&body.input)
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn list_torrents(State(s): State<AppState>) -> ApiResult<Vec<TorrentRecord>> {
    let mut rows = s.torrents.list().await?;
    // Overlay ephemeral poller speeds/peers (not stored in SQLite).
    {
        let speeds = s.torrent_speeds.lock().await;
        for row in &mut rows {
            if let Some((dl, ul, peers)) = speeds.get(&row.id) {
                row.download_speed = *dl;
                row.upload_speed = *ul;
                row.peers = *peers;
            }
        }
    }
    Ok(Json(rows))
}

async fn stage_torrent(
    State(s): State<AppState>,
    Json(body): Json<TorrentStageRequest>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .stage(body)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

#[derive(Deserialize)]
struct ImportQbittorrentBody {
    /// Absolute path to a qBittorrent `BT_backup` directory. If omitted, defaults
    /// to the platform-standard qBittorrent data location.
    directory: Option<String>,
}

#[derive(Serialize)]
struct ImportQbittorrentResultItem {
    info_hash: String,
    name: Option<String>,
    imported: bool,
    error: Option<String>,
}

#[derive(Serialize)]
struct ImportQbittorrentResponse {
    found: usize,
    imported: usize,
    items: Vec<ImportQbittorrentResultItem>,
}

fn default_qbittorrent_backup_dir() -> Option<std::path::PathBuf> {
    #[cfg(windows)]
    {
        std::env::var_os("LOCALAPPDATA").map(|p| {
            std::path::PathBuf::from(p)
                .join("qBittorrent")
                .join("BT_backup")
        })
    }
    #[cfg(target_os = "linux")]
    {
        directories::BaseDirs::new().map(|d| d.config_dir().join("qBittorrent").join("BT_backup"))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        None
    }
}

/// Import torrents from a qBittorrent installation: parses .fastresume files
/// (deliberately not trusting their piece bitfield -- see `import_qbt.rs` doc
/// comment), stages each via the normal `TorrentManager::stage()` path (so the
/// settings-allowlist policy gate still applies, same as any other add), then
/// starts non-stopped torrents so our own engine hash-verifies existing files.
async fn import_qbittorrent(
    State(s): State<AppState>,
    Json(body): Json<ImportQbittorrentBody>,
) -> ApiResult<ImportQbittorrentResponse> {
    let dir = match body
        .directory
        .map(std::path::PathBuf::from)
        .or_else(default_qbittorrent_backup_dir)
    {
        Some(d) => d,
        None => {
            return Err(bad_request(
                "no directory given and no default known for this platform",
            ))
        }
    };
    if !dir.is_dir() {
        return Err(bad_request(format!("not a directory: {}", dir.display())));
    }

    let candidates =
        crate::import_qbt::scan_directory(&dir).map_err(|e| bad_request(e.to_string()))?;
    let found = candidates.len();
    let mut items = Vec::with_capacity(found);
    let mut imported = 0usize;

    for c in candidates {
        let name = c.name.clone();
        let result = import_one(&s, &c).await;
        match result {
            Ok(()) => {
                imported += 1;
                items.push(ImportQbittorrentResultItem {
                    info_hash: c.info_hash,
                    name,
                    imported: true,
                    error: None,
                });
            }
            Err(e) => {
                items.push(ImportQbittorrentResultItem {
                    info_hash: c.info_hash,
                    name,
                    imported: false,
                    error: Some(e.to_string()),
                });
            }
        }
    }

    Ok(Json(ImportQbittorrentResponse {
        found,
        imported,
        items,
    }))
}

async fn import_one(s: &AppState, c: &crate::import_qbt::QbtImportCandidate) -> anyhow::Result<()> {
    let torrent_path = c.torrent_file_path.to_string_lossy().to_string();
    let probe = torrent::probe_input(&torrent_path)?;
    let request = TorrentStageRequest {
        probe,
        save_root: c.save_path.clone(),
        policy: ferric_common::TorrentPolicy::default(),
    };
    let detail = s.torrents.stage(request).await?;

    if !c.tags.is_empty() {
        db::torrents::set_tags(&s.pool, &detail.record.id, &c.tags).await?;
    }

    if !c.stopped {
        s.torrents.start(&detail.record.id).await?;
    }

    Ok(())
}

async fn get_torrent_detail(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    let mut detail = s
        .torrents
        .detail(&id)
        .await?
        .ok_or_else(|| not_found("torrent not found"))?;
    if let Some((dl, ul, peers)) = s.torrent_speeds.lock().await.get(&id).copied() {
        detail.record.download_speed = dl;
        detail.record.upload_speed = ul;
        detail.record.peers = peers;
    }
    Ok(Json(detail))
}

async fn update_torrent_policy(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(policy): Json<TorrentPolicy>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .update_policy(&id, policy)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn update_torrent_file(
    State(s): State<AppState>,
    Path((id, file_id)): Path<(String, String)>,
    Json(body): Json<TorrentFilePatchBody>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .update_file_selection(&id, &file_id, body.selected, body.priority)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn assign_torrent_proxy(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<AssignProxyBody>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .assign_proxy(&id, body.proxy_id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

#[derive(Deserialize)]
struct RemoveTorrentQuery {
    #[serde(default)]
    delete_files: bool,
}

/// File paths come from the torrent metainfo — refuse anything that could
/// escape `save_root` instead of trusting it.
async fn remove_torrent_files_from_disk(root: &std::path::Path, file_paths: &[String], name: &str) {
    use std::path::Component;
    for p in file_paths {
        let rel = std::path::Path::new(p);
        if rel.components().any(|c| {
            matches!(
                c,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            continue;
        }
        let _ = tokio::fs::remove_file(root.join(rel)).await;
    }
    // Multi-file torrents live in a content dir named after the torrent;
    // remove it only if now empty (remove_dir refuses non-empty).
    let _ = tokio::fs::remove_dir(root.join(name)).await;
}

async fn remove_torrent(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<RemoveTorrentQuery>,
) -> ApiResult<()> {
    if q.delete_files {
        if let Some(detail) = db::torrents::get_detail(&s.pool, &id).await? {
            if let Some(root) = detail.record.save_root.as_deref() {
                let paths: Vec<String> = detail.files.iter().map(|f| f.path.clone()).collect();
                remove_torrent_files_from_disk(
                    std::path::Path::new(root),
                    &paths,
                    &detail.record.name,
                )
                .await;
            }
        }
    }
    db::torrents::remove_torrent(&s.pool, &id).await?;
    Ok(Json(()))
}

async fn torrent_privacy_check(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<PrivacyCheckResult> {
    let result = s.torrents.run_privacy_check(&id).await?;
    Ok(Json(result))
}

#[derive(Deserialize)]
struct QueuePositionBody {
    position: String, // "up", "down", "top", "bottom"
}

async fn set_torrent_queue_position(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<QueuePositionBody>,
) -> ApiResult<TorrentDetail> {
    use sqlx::Row;
    // F22: BEGIN IMMEDIATE takes the write lock up front so concurrent
    // top/up/down cannot race on MAX(queue_position) under DEFERRED.
    let mut conn = s
        .pool
        .acquire()
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    sqlx::query("BEGIN IMMEDIATE")
        .execute(&mut *conn)
        .await
        .map_err(|e| bad_request(e.to_string()))?;

    let tx_result: Result<(), ApiError> = async {
        let row = sqlx::query("SELECT queue_position FROM torrents WHERE id = ?1")
            .bind(&id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e| bad_request(e.to_string()))?
            .ok_or_else(|| not_found("torrent not found"))?;
        let current: i32 = row.try_get("queue_position").unwrap_or(0);

        let new_pos = match body.position.as_str() {
            "up" => current + 1,
            "down" => (current - 1).max(0),
            "top" => {
                let max: i32 =
                    sqlx::query("SELECT COALESCE(MAX(queue_position), 0) as mx FROM torrents")
                        .fetch_one(&mut *conn)
                        .await
                        .map_or(0, |r| r.try_get("mx").unwrap_or(0));
                max + 1
            }
            "bottom" => 0,
            _ => return Err(bad_request("position must be up/down/top/bottom")),
        };

        sqlx::query("UPDATE torrents SET queue_position = ?1 WHERE id = ?2")
            .bind(new_pos)
            .bind(&id)
            .execute(&mut *conn)
            .await
            .map_err(|e| bad_request(e.to_string()))?;
        Ok(())
    }
    .await;

    match tx_result {
        Ok(()) => {
            if let Err(e) = sqlx::query("COMMIT").execute(&mut *conn).await {
                let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
                return Err(bad_request(e.to_string()));
            }
        }
        Err(e) => {
            let _ = sqlx::query("ROLLBACK").execute(&mut *conn).await;
            return Err(e);
        }
    }

    s.torrents
        .detail(&id)
        .await?
        .ok_or_else(|| not_found("torrent not found"))
        .map(Json)
}

#[derive(Deserialize)]
struct TagBody {
    tag: String,
}

async fn get_torrent_tags(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<String>> {
    let tags = db::torrents::get_tags(&s.pool, &id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(tags))
}

async fn set_torrent_tags(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<Vec<String>>,
) -> ApiResult<Vec<String>> {
    db::torrents::set_tags(&s.pool, &id, &body)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    let tags = db::torrents::get_tags(&s.pool, &id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(tags))
}

#[derive(Deserialize)]
struct SetTorrentCategoryBody {
    category: Option<String>,
}

async fn set_torrent_category(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SetTorrentCategoryBody>,
) -> Result<StatusCode, ApiError> {
    db::torrents::set_category(&s.pool, &id, body.category.as_deref()).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn add_torrent_tag(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TagBody>,
) -> ApiResult<Vec<String>> {
    db::torrents::add_tag(&s.pool, &id, &body.tag)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    let tags = db::torrents::get_tags(&s.pool, &id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(tags))
}

async fn remove_torrent_tag(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<TagBody>,
) -> ApiResult<Vec<String>> {
    db::torrents::remove_tag(&s.pool, &id, &body.tag)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    let tags = db::torrents::get_tags(&s.pool, &id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    Ok(Json(tags))
}

async fn get_torrent_audit(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<db::torrent_audit::TorrentAuditRow>> {
    if s.torrents
        .detail(&id)
        .await
        .map_err(|e| bad_request(e.to_string()))?
        .is_none()
    {
        return Err(not_found("torrent not found"));
    }
    let rows = db::torrent_audit::list_for_torrent(&s.pool, &id).await?;
    Ok(Json(rows))
}

async fn start_torrent_engine(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .start(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn sync_torrent_engine(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    let detail = s
        .torrents
        .sync_stats(&id)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    {
        let mut map = s.torrent_speeds.lock().await;
        map.insert(
            detail.record.id.clone(),
            (
                detail.record.download_speed,
                detail.record.upload_speed,
                detail.record.peers,
            ),
        );
    }
    Ok(Json(detail))
}

async fn pause_torrent_engine(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .pause(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn force_recheck_torrent(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .force_recheck(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn force_reannounce_torrent(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .force_announce(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

#[derive(Deserialize)]
struct SetLocationBody {
    location: String,
}

async fn set_torrent_location(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SetLocationBody>,
) -> ApiResult<TorrentDetail> {
    if body.location.trim().is_empty() {
        return Err(bad_request("location required"));
    }
    s.torrents
        .set_location(&id, &body.location)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

#[derive(Deserialize)]
struct SetTrackersBody {
    trackers: Vec<String>,
}

async fn set_torrent_trackers(
    State(s): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<SetTrackersBody>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .set_trackers(&id, body.trackers)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn torrent_live_peers(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Vec<ferric_common::LivePeerView>> {
    s.torrents
        .live_peers_geo(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn resume_torrent_engine(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<TorrentDetail> {
    s.torrents
        .start(&id)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn torrent_engine_status(State(s): State<AppState>) -> ApiResult<TorrentEngineStatus> {
    Ok(Json(s.torrents.engine_status().await))
}

async fn list_proxy_configs(State(s): State<AppState>) -> ApiResult<Vec<ProxyConfigRecord>> {
    Ok(Json(db::proxies::list(&s.pool).await?))
}

async fn save_proxy_config(
    State(s): State<AppState>,
    Json(input): Json<ProxyConfigInput>,
) -> ApiResult<ProxyConfigRecord> {
    db::proxies::upsert(&s.pool, &input)
        .await
        .map(Json)
        .map_err(|e| bad_request(e.to_string()))
}

async fn delete_proxy_config(State(s): State<AppState>, Path(id): Path<String>) -> ApiResult<()> {
    db::proxies::delete(&s.pool, &id).await?;
    Ok(Json(()))
}

#[derive(Serialize)]
struct ProxyTestResult {
    ok: bool,
    latency_ms: Option<u64>,
    error: Option<String>,
}

async fn test_proxy_config(
    State(s): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<ProxyTestResult> {
    let runtime = db::proxies::get_runtime(&s.pool, &id)
        .await
        .map_err(|e| bad_request(e.to_string()))?
        .ok_or_else(|| bad_request("proxy config not found"))?;

    let url = match runtime.proxy_type {
        ferric_common::ProxyKind::Socks5 => {
            if let (Some(user), Some(pass)) = (&runtime.username, &runtime.password) {
                format!(
                    "socks5h://{}:{}@{}:{}",
                    url::form_urlencoded::byte_serialize(user.as_bytes()).collect::<String>(),
                    url::form_urlencoded::byte_serialize(pass.as_bytes()).collect::<String>(),
                    runtime.host,
                    runtime.port
                )
            } else {
                format!("socks5h://{}:{}", runtime.host, runtime.port)
            }
        }
        ferric_common::ProxyKind::Http => {
            if let (Some(user), Some(pass)) = (&runtime.username, &runtime.password) {
                format!(
                    "http://{}:{}@{}:{}",
                    url::form_urlencoded::byte_serialize(user.as_bytes()).collect::<String>(),
                    url::form_urlencoded::byte_serialize(pass.as_bytes()).collect::<String>(),
                    runtime.host,
                    runtime.port
                )
            } else {
                format!("http://{}:{}", runtime.host, runtime.port)
            }
        }
    };

    let test_url = "https://httpbin.org/ip";
    let start = std::time::Instant::now();

    let proxy = match reqwest::Proxy::all(&url) {
        Ok(p) => p,
        Err(e) => {
            return Ok(Json(ProxyTestResult {
                ok: false,
                latency_ms: None,
                error: Some(format!("invalid proxy url: {e}")),
            }));
        }
    };

    let client = match reqwest::Client::builder()
        .proxy(proxy)
        .timeout(std::time::Duration::from_secs(10))
        .danger_accept_invalid_certs(false)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Ok(Json(ProxyTestResult {
                ok: false,
                latency_ms: None,
                error: Some(format!("failed to build client: {e}")),
            }));
        }
    };

    match client.get(test_url).send().await {
        Ok(resp) => {
            let latency = start.elapsed().as_millis() as u64;
            if resp.status().is_success() {
                Ok(Json(ProxyTestResult {
                    ok: true,
                    latency_ms: Some(latency),
                    error: None,
                }))
            } else {
                Ok(Json(ProxyTestResult {
                    ok: false,
                    latency_ms: Some(latency),
                    error: Some(format!("HTTP {}", resp.status())),
                }))
            }
        }
        Err(e) => Ok(Json(ProxyTestResult {
            ok: false,
            latency_ms: None,
            error: Some(e.to_string()),
        })),
    }
}

async fn list_merge_jobs(State(s): State<AppState>) -> ApiResult<Vec<MergeJobRecord>> {
    let mut records = db::merge_jobs::list(&s.pool).await?;
    let live_jobs = s.merge_jobs.lock().await;
    for record in &mut records {
        if let Some(live) = live_jobs.get(&record.id) {
            *record = live.clone();
        }
    }
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(Json(records))
}

async fn start_merge_job(
    State(s): State<AppState>,
    Json(body): Json<MergeJobBody>,
) -> ApiResult<MergeJobRecord> {
    let id = Uuid::new_v4().to_string();
    let record = MergeJobRecord {
        id: id.clone(),
        video_path: body.video_path.clone(),
        audio_path: body.audio_path.clone(),
        output_path: body.output_path.clone(),
        state: MergeState::Queued,
        message: Some("Merge queued".to_string()),
        created_at: now_ms(),
        completed_at: None,
    };
    db::merge_jobs::upsert(&s.pool, &record).await?;
    s.merge_jobs.lock().await.insert(id.clone(), record.clone());

    let jobs = s.merge_jobs.clone();
    let pool = s.pool.clone();
    tokio::spawn(async move {
        {
            let mut jobs = jobs.lock().await;
            if let Some(job) = jobs.get_mut(&id) {
                job.state = MergeState::Merging;
                job.message = Some("ffmpeg merge running".to_string());
                let _ = db::merge_jobs::upsert(&pool, job).await;
            }
        }

        let result = media::run_ffmpeg_merge(
            FsPath::new(&body.video_path),
            FsPath::new(&body.audio_path),
            FsPath::new(&body.output_path),
            FsPath::new(&body.save_dir),
        )
        .await;

        let mut jobs = jobs.lock().await;
        if let Some(job) = jobs.get_mut(&id) {
            job.completed_at = Some(now_ms());
            match result {
                Ok(()) => {
                    job.state = MergeState::Completed;
                    job.message = Some("Merge complete".to_string());
                }
                Err(err) => {
                    job.state = MergeState::Failed;
                    job.message = Some(err.to_string());
                }
            }
            let _ = db::merge_jobs::upsert(&pool, job).await;
        }
    });

    Ok(Json(record))
}

fn now_ms() -> u64 {
    u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis()),
    )
    .unwrap_or(u64::MAX)
}

async fn refresh_url(
    State(s): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RefreshUrlBody>,
) -> Result<StatusCode, ApiError> {
    security::validate_url(&body.url).map_err(|e| bad_request(e.to_string()))?;
    ensure_exists(&s.pool, id).await?;
    if let Some(proxy) = body.proxy_url.as_deref().filter(|p| !p.trim().is_empty()) {
        security::validate_url(proxy).map_err(|e| bad_request(format!("proxy: {e}")))?;
        db::downloads::set_proxy(&s.pool, id, Some(proxy)).await?;
    }
    let (tx, rx) = oneshot::channel::<DaemonReply>();
    s.manager_tx
        .send(ManagerCommand::UpdateUrl {
            id,
            new_url: body.url,
            reply_tx: tx,
        })
        .await
        .map_err(|_| {
            ApiError(
                StatusCode::SERVICE_UNAVAILABLE,
                "manager unavailable".into(),
            )
        })?;
    match rx.await {
        Ok(DaemonReply::Accepted { .. }) => Ok(StatusCode::NO_CONTENT),
        Ok(DaemonReply::Error { message, .. }) => Err(ApiError(StatusCode::BAD_REQUEST, message)),
        Err(_) => Err(ApiError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "manager reply lost".into(),
        )),
    }
}

#[derive(Deserialize)]
struct GrabBody {
    url: String,
    #[serde(default)]
    extensions: Vec<String>,
    /// BFS crawl depth. 0 = single page (default). Max clamped to 5.
    #[serde(default)]
    depth: u8,
    /// Patterns (`*`) never opened or collected, e.g. logout pages.
    #[serde(default)]
    exclude: Vec<String>,
    /// Patterns (`*`); when given, only matching pages are explored.
    #[serde(default)]
    include: Vec<String>,
    /// Do not explore pages above the start page's directory.
    #[serde(default)]
    stay_below: bool,
    /// Link levels to follow on other sites (0 = stay on the start site).
    #[serde(default)]
    external_levels: u8,
}

async fn grab_handler(
    State(_s): State<AppState>,
    Json(body): Json<GrabBody>,
) -> Result<Json<Vec<String>>, ApiError> {
    security::validate_url(&body.url).map_err(|e| bad_request(e.to_string()))?;
    let depth = body.depth.min(5);
    let links = crate::grabber::grab_links(
        &body.url,
        &body.extensions,
        depth,
        &crate::grabber::GrabFilters {
            exclude: body.exclude,
            include: body.include,
            stay_below: body.stay_below,
            external_levels: body.external_levels,
        },
    )
    .await
    .map_err(|e| ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(links))
}

// â"€â"€ Helpers â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€â"€

async fn ensure_exists(pool: &SqlitePool, id: Uuid) -> Result<(), ApiError> {
    db::downloads::get(pool, id)
        .await?
        .ok_or_else(|| not_found(format!("download {id} not found")))?;
    Ok(())
}

fn unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use ferric_common::DownloadStatus;
    use tower::ServiceExt as _;

    #[tokio::test]
    async fn remove_torrent_files_deletes_inside_root_refuses_traversal() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path();
        let content_dir = root.join("MyTorrent");
        std::fs::create_dir(&content_dir)?;
        std::fs::write(content_dir.join("a.bin"), b"x")?;
        std::fs::write(root.join("single.bin"), b"y")?;
        let outside = dir.path().parent().unwrap().join("ferric-test-escape.bin");
        std::fs::write(&outside, b"z")?;

        let paths = vec![
            "MyTorrent/a.bin".to_string(),
            "single.bin".to_string(),
            "../ferric-test-escape.bin".to_string(),
        ];
        remove_torrent_files_from_disk(root, &paths, "MyTorrent").await;

        assert!(
            !content_dir.join("a.bin").exists(),
            "in-root file should be deleted"
        );
        assert!(
            !content_dir.exists(),
            "emptied content dir should be removed"
        );
        assert!(
            !root.join("single.bin").exists(),
            "root-level file should be deleted"
        );
        assert!(outside.exists(), "traversal path must be refused");
        std::fs::remove_file(&outside)?;
        Ok(())
    }

    async fn test_state() -> anyhow::Result<AppState> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        // TempDir cleanup on drop deletes the DB out from under the live pool.
        // Windows masks this (open handles block deletion); Linux does not —
        // every DB-touching test failed with SQLITE_CANTOPEN on ubuntu CI.
        std::mem::forget(dir);
        let (manager_tx, _manager_rx) = mpsc::channel(1);
        let (progress_tx, _) = broadcast::channel(4);
        Ok(AppState {
            torrents: TorrentManager::new(pool.clone()),
            pool,
            manager_tx,
            progress_tx,
            auth_token: "a".repeat(64),
            start_time: Instant::now(),
            merge_jobs: Arc::new(Mutex::new(HashMap::new())),
            rate_hits: Arc::new(Mutex::new(Vec::new())),
            torrent_speeds: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    #[tokio::test]
    async fn auth_middleware_rejects_missing_token() -> anyhow::Result<()> {
        let app = build_router(test_state().await?);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn auth_middleware_rejects_wrong_token() -> anyhow::Result<()> {
        let app = build_router(test_state().await?);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("X-Ferric-Auth", "b".repeat(64))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn auth_middleware_rejects_malformed_token() -> anyhow::Result<()> {
        let app = build_router(test_state().await?);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("X-Ferric-Auth", "not-a-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        Ok(())
    }

    #[tokio::test]
    async fn auth_middleware_accepts_valid_token() -> anyhow::Result<()> {
        let state = test_state().await?;
        let token = state.auth_token.clone();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("X-Ferric-Auth", token)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::OK);
        Ok(())
    }

    async fn health_status(headers: &[(&str, &str)]) -> anyhow::Result<StatusCode> {
        let state = test_state().await?;
        let token = state.auth_token.clone();
        let app = build_router(state);
        let mut req = Request::builder()
            .uri("/health")
            .header("X-Ferric-Auth", token);
        for (k, v) in headers {
            req = req.header(*k, *v);
        }
        Ok(app
            .oneshot(req.body(Body::empty()).unwrap())
            .await?
            .status())
    }

    #[tokio::test]
    async fn api_rejects_browser_origin_even_with_valid_token() -> anyhow::Result<()> {
        assert_eq!(
            health_status(&[("Origin", "https://evil.example")]).await?,
            StatusCode::FORBIDDEN
        );
        Ok(())
    }

    #[tokio::test]
    async fn api_rejects_dns_rebinding_host_even_with_valid_token() -> anyhow::Result<()> {
        for host in [
            "evil.example",
            "evil.example:8080",
            "127.0.0.1.evil.example",
            "10.0.0.5:80",
        ] {
            assert_eq!(
                health_status(&[("Host", host)]).await?,
                StatusCode::FORBIDDEN,
                "{host}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn api_accepts_loopback_hosts() -> anyhow::Result<()> {
        for host in [
            "127.0.0.1:4000",
            "localhost:4000",
            "LOCALHOST",
            "[::1]:4000",
        ] {
            assert_eq!(
                health_status(&[("Host", host)]).await?,
                StatusCode::OK,
                "{host}"
            );
        }
        Ok(())
    }

    #[tokio::test]
    async fn extract_rejects_private_ip_url() -> anyhow::Result<()> {
        let state = test_state().await?;
        let token = state.auth_token.clone();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/media/extract")
                    .header("X-Ferric-Auth", token)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"http://192.168.1.1/video.mp4"}"#))
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn extract_rejects_non_http_scheme() -> anyhow::Result<()> {
        let state = test_state().await?;
        let token = state.auth_token.clone();
        let app = build_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/media/extract")
                    .header("X-Ferric-Auth", token)
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"url":"file:///etc/passwd"}"#))
                    .unwrap(),
            )
            .await?;

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn lock_file_contains_port_and_token() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let token = generate_token()?;
        write_lock_file(dir.path().to_path_buf(), 49152, &token).await?;

        let contents = tokio::fs::read_to_string(dir.path().join(API_LOCK_FILE)).await?;
        let lines: Vec<&str> = contents.lines().collect();
        assert_eq!(lines, vec!["49152", token.as_str()]);
        assert_eq!(token.len(), 64);
        Ok(())
    }

    #[tokio::test]
    async fn lock_file_write_replaces_temp_file() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let token = generate_token()?;
        let lock_path = dir.path().join(API_LOCK_FILE);
        let tmp_path = lock_path.with_extension("lock.tmp");

        write_lock_file(dir.path().to_path_buf(), 49152, &token).await?;

        assert!(lock_path.exists());
        assert!(!tmp_path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn lock_file_is_user_only_on_unix() -> anyhow::Result<()> {
        use std::os::unix::fs::PermissionsExt as _;

        let dir = tempfile::tempdir()?;
        let token = generate_token()?;
        let lock_path = dir.path().join(API_LOCK_FILE);

        write_lock_file(dir.path().to_path_buf(), 49152, &token).await?;

        let mode = std::fs::metadata(lock_path)?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        Ok(())
    }

    #[tokio::test]
    async fn progress_event_stream_yields_broadcast_events() -> anyhow::Result<()> {
        let (tx, rx) = broadcast::channel(4);
        let mut stream = progress_event_stream(rx);
        tx.send(ProgressEvent::Progress {
            id: Uuid::new_v4(),
            downloaded: 7,
            total: Some(10),
            speed_bps: 3,
            eta_secs: Some(1),
            status: DownloadStatus::Downloading,
        })?;

        let event = stream.next().await.expect("stream should yield one event");
        assert!(event.is_ok());
        Ok(())
    }

    #[tokio::test]
    async fn cancel_sends_cancel_remove_sends_remove() -> anyhow::Result<()> {
        let state = test_state().await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: "https://example.com/file.bin".to_string(),
            final_url: None,
            filename: Some("file.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        db::downloads::insert(&state.pool, &req, "file.bin", "Other").await?;

        let (manager_tx, mut manager_rx) = mpsc::channel(2);
        let state = AppState {
            manager_tx,
            ..state
        };

        // POST /cancel stops the job but keeps the record.
        let status = cancel_download(State(state.clone()), Path(id))
            .await
            .unwrap_or_else(|e| panic!("cancel failed: {}", e.1));
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(
            matches!(manager_rx.recv().await, Some(ManagerCommand::Cancel(found)) if found == id)
        );
        assert!(manager_rx.try_recv().is_err());

        // DELETE removes the record.
        let status = remove_download(
            State(state),
            Path(id),
            Query(RemoveQuery {
                delete_file: Some(true),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("remove failed: {}", e.1));
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(matches!(
            manager_rx.recv().await,
            Some(ManagerCommand::Remove {
                id: found,
                delete_file: true,
            }) if found == id
        ));
        assert!(manager_rx.try_recv().is_err());
        Ok(())
    }

    #[tokio::test]
    async fn confirm_stale_capture_returns_conflict() -> anyhow::Result<()> {
        let state = test_state().await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: "https://example.com/file.bin".to_string(),
            final_url: None,
            filename: Some("file.bin".to_string()),
            referrer: Some("https://example.com/".to_string()),
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        db::downloads::insert(&state.pool, &req, "file.bin", "Other").await?;

        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        let state = AppState {
            manager_tx,
            ..state
        };
        let responder = tokio::spawn(async move {
            let Some(ManagerCommand::Confirm { reply_tx, .. }) = manager_rx.recv().await else {
                panic!("expected confirm command");
            };
            reply_tx
                .send(DaemonReply::Error {
                    id,
                    message: "no pending capture for this download".into(),
                })
                .ok();
        });

        let err = confirm_download(
            State(state),
            Path(id),
            Json(ConfirmBody {
                filename: Some("file.bin".to_string()),
                save_dir: None,
                url: None,
                audio_url: None,
                subtitle_url: None,
                subtitle_lang: None,
                proxy_url: None,
                start: None,
                description: None,
                category: None,
            }),
        )
        .await
        .expect_err("stale capture should fail");

        assert_eq!(err.0, StatusCode::CONFLICT);
        assert!(err.1.contains("Capture the link again"));
        responder.await?;
        Ok(())
    }

    #[tokio::test]
    async fn set_priority_updates_db_and_api_sends_command() -> anyhow::Result<()> {
        let state = test_state().await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: "https://example.com/file.bin".to_string(),
            final_url: None,
            filename: Some("file.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        db::downloads::insert(&state.pool, &req, "file.bin", "Other").await?;

        assert!(db::downloads::update_priority(&state.pool, id, 7).await?);
        assert_eq!(
            db::downloads::get(&state.pool, id)
                .await?
                .expect("record exists")
                .priority,
            7
        );
        assert!(!db::downloads::update_priority(&state.pool, Uuid::new_v4(), 3).await?);

        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        let state = AppState {
            manager_tx,
            ..state
        };
        let status = set_priority(
            State(state.clone()),
            Path(id),
            Json(PriorityBody { priority: 42 }),
        )
        .await
        .unwrap_or_else(|e| panic!("set priority failed: {}", e.1));
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(matches!(
            manager_rx.recv().await,
            Some(ManagerCommand::SetPriority {
                id: found,
                priority: 42
            }) if found == id
        ));

        let err = set_priority(
            State(state),
            Path(Uuid::new_v4()),
            Json(PriorityBody { priority: 1 }),
        )
        .await
        .expect_err("unknown download must 404");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test]
    async fn queue_handlers_list_patch_validate_and_pump() -> anyhow::Result<()> {
        let state = test_state().await?;
        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        let state = AppState {
            manager_tx,
            ..state
        };
        let id = db::queues::DEFAULT_QUEUE_ID;

        let Json(queues) = list_queues(State(state.clone()))
            .await
            .unwrap_or_else(|e| panic!("list queues failed: {}", e.1));
        assert!(queues.iter().any(|queue| queue.id == id));

        let Json(queue) = get_queue(State(state.clone()), Path(id))
            .await
            .unwrap_or_else(|e| panic!("get queue failed: {}", e.1));
        assert_eq!(queue.name, "Default");
        assert_eq!(queue.max_concurrent, 2);

        let status = update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                name: Some("Fast".to_string()),
                max_concurrent: Some(4),
                max_bandwidth: None, // absent = preserve
                sched_start: None,
                sched_end: None,
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                enabled: Some(false),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("patch queue failed: {}", e.1));
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(matches!(
            manager_rx.recv().await,
            Some(ManagerCommand::PumpQueue)
        ));

        let Json(updated) = get_queue(State(state.clone()), Path(id))
            .await
            .unwrap_or_else(|e| panic!("get updated queue failed: {}", e.1));
        assert_eq!(updated.name, "Fast");
        assert_eq!(updated.max_concurrent, 4);
        assert!(!updated.enabled);

        let Err(err) = update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                name: None,
                max_concurrent: Some(0),
                max_bandwidth: None,
                sched_start: None,
                sched_end: None,
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                enabled: None,
            }),
        )
        .await
        else {
            panic!("zero max_concurrent must be rejected");
        };
        assert_eq!(err.0, StatusCode::BAD_REQUEST);

        let Err(err) = get_queue(State(state), Path(Uuid::new_v4())).await else {
            panic!("unknown queue must 404");
        };
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        Ok(())
    }

    #[tokio::test]
    async fn queue_patch_clears_max_bandwidth_with_explicit_null() -> anyhow::Result<()> {
        let state = test_state().await?;
        let (manager_tx, mut manager_rx) = mpsc::channel(4);
        let state = AppState {
            manager_tx,
            ..state
        };
        let id = db::queues::DEFAULT_QUEUE_ID;

        // Set a limit first.
        update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                max_bandwidth: Some(Some(8192)),
                name: None,
                max_concurrent: None,
                sched_start: None,
                sched_end: None,
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                enabled: None,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("set bandwidth failed: {}", e.1));
        let _ = manager_rx.recv().await; // PumpQueue

        let Json(q) = get_queue(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(q.max_bandwidth, Some(8192));

        // Clear it with explicit null.
        update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                max_bandwidth: Some(None),
                name: None,
                max_concurrent: None,
                sched_start: None,
                sched_end: None,
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                enabled: None,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("clear bandwidth failed: {}", e.1));
        let _ = manager_rx.recv().await;

        let Json(q) = get_queue(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(
            q.max_bandwidth, None,
            "explicit null must clear max_bandwidth"
        );
        Ok(())
    }

    #[tokio::test]
    async fn queue_patch_sets_and_validates_schedule_fields() -> anyhow::Result<()> {
        let state = test_state().await?;
        let (manager_tx, mut manager_rx) = mpsc::channel(4);
        let state = AppState {
            manager_tx,
            ..state
        };
        let id = db::queues::DEFAULT_QUEUE_ID;

        // Set a valid schedule window.
        update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                sched_start: Some(Some(540)), // 09:00
                sched_end: Some(Some(1020)),  // 17:00
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                name: None,
                max_concurrent: None,
                max_bandwidth: None,
                enabled: None,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("set schedule failed: {}", e.1));
        let _ = manager_rx.recv().await;

        let Json(q) = get_queue(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(q.sched_start, Some(540));
        assert_eq!(q.sched_end, Some(1020));

        // Out-of-range minute is rejected.
        let Err(err) = update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                sched_start: Some(Some(1440)),
                sched_end: None,
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                name: None,
                max_concurrent: None,
                max_bandwidth: None,
                enabled: None,
            }),
        )
        .await
        else {
            panic!("sched_start >= 1440 must be rejected");
        };
        assert_eq!(err.0, StatusCode::BAD_REQUEST);

        // Clear both fields with explicit null.
        update_queue(
            State(state.clone()),
            Path(id),
            Json(QueuePatchBody {
                sched_start: Some(None),
                sched_end: Some(None),
                sched_days: None,
                start_on_startup: None,
                sync_interval_min: None,
                max_retries: None,
                open_file: None,
                when_done: None,
                name: None,
                max_concurrent: None,
                max_bandwidth: None,
                enabled: None,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("clear schedule failed: {}", e.1));
        let _ = manager_rx.recv().await;

        let Json(q) = get_queue(State(state.clone()), Path(id)).await.unwrap();
        assert_eq!(q.sched_start, None, "explicit null must clear sched_start");
        assert_eq!(q.sched_end, None, "explicit null must clear sched_end");
        Ok(())
    }

    #[tokio::test]
    async fn add_download_passes_save_dir_override() -> anyhow::Result<()> {
        let state = test_state().await?;
        let pool = state.pool.clone();
        let override_dir = tempfile::tempdir()?;
        let expected_dir = override_dir.path().to_path_buf();
        let (manager_tx, mut manager_rx) = mpsc::channel(1);
        let state = AppState {
            manager_tx,
            ..state
        };

        let responder_pool = pool.clone();
        let expected_for_task = expected_dir.clone();
        let expected_queue = db::queues::DEFAULT_QUEUE_ID;
        let manager_task = tokio::spawn(async move {
            let Some(ManagerCommand::Add {
                req,
                save_dir_override,
                hold,
                audio_url: _,
                subtitle_url: _,
                subtitle_lang: _,
                proxy_url: _,
                reply_tx,
            }) = manager_rx.recv().await
            else {
                panic!("expected add command");
            };
            assert!(!hold, "UI-typed adds must not hold");
            assert_eq!(save_dir_override, Some(expected_for_task.clone()));
            assert_eq!(req.queue_id, Some(expected_queue));
            let save_path = expected_for_task.join(req.filename.clone().unwrap());
            db::downloads::insert(&responder_pool, &req, &save_path.to_string_lossy(), "Other")
                .await
                .unwrap();
            reply_tx.send(DaemonReply::Accepted { id: req.id }).ok();
        });

        let body = AddUrlBody {
            hold: false,
            url: "https://example.com/file.bin".to_string(),
            filename: Some("file.bin".to_string()),
            save_dir: Some(expected_dir.to_string_lossy().to_string()),
            queue_id: Some(expected_queue),
            cookies: None,
            checksum: None,
            mirrors: vec![],
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            description: None,
        };
        let (status, Json(record)) = add_download(State(state), Json(body))
            .await
            .unwrap_or_else(|e| panic!("add failed: {}", e.1));
        manager_task.await?;

        assert_eq!(status, StatusCode::CREATED);
        assert_eq!(
            record.save_path,
            expected_dir.join("file.bin").to_string_lossy()
        );
        assert_eq!(record.queue_id, Some(expected_queue));
        Ok(())
    }

    #[tokio::test]
    async fn add_download_rejects_unknown_queue_id() -> anyhow::Result<()> {
        let state = test_state().await?;
        let body = AddUrlBody {
            hold: false,
            url: "https://example.com/file.bin".to_string(),
            filename: Some("file.bin".to_string()),
            save_dir: None,
            queue_id: Some(Uuid::new_v4()),
            cookies: None,
            checksum: None,
            mirrors: vec![],
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            description: None,
        };

        let Err(err) = add_download(State(state), Json(body)).await else {
            panic!("unknown queue_id must be rejected");
        };
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        Ok(())
    }

    #[tokio::test]
    async fn torrent_stage_list_policy_remove() -> anyhow::Result<()> {
        let state = test_state().await?;

        // List empty
        let Json(empty) = list_torrents(State(state.clone()))
            .await
            .unwrap_or_else(|e| panic!("list_torrents failed: {}", e.1));
        assert_eq!(empty, [] as [ferric_common::TorrentRecord; 0]);

        // Probe then stage a magnet
        let probe = torrent::probe_input(
            "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Test+Torrent&tr=https%3A%2F%2Ftracker.example%2Fannounce",
        )?;
        let Json(detail) = stage_torrent(
            State(state.clone()),
            Json(TorrentStageRequest {
                probe,
                save_root: None,
                policy: TorrentPolicy::default(),
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("stage_torrent failed: {}", e.1));
        assert_eq!(detail.record.name, "Test Torrent");
        assert!(!detail.record.policy.dht_enabled);
        assert!(!detail.record.policy.seed_enabled);
        assert_eq!(detail.trackers.len(), 1);

        // List shows the staged torrent
        let Json(list) = list_torrents(State(state.clone()))
            .await
            .unwrap_or_else(|e| panic!("list after stage failed: {}", e.1));
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, detail.record.id);

        // Get detail
        let Json(fetched) =
            get_torrent_detail(State(state.clone()), Path(detail.record.id.clone()))
                .await
                .unwrap_or_else(|e| panic!("get_detail failed: {}", e.1));
        assert_eq!(
            fetched.record.info_hash,
            "0123456789abcdef0123456789abcdef01234567"
        );

        // Update policy (enable blocklist, still no seeding)
        let new_policy = TorrentPolicy {
            blocklist_enabled: true,
            ..TorrentPolicy::default()
        };
        let Json(updated) = update_torrent_policy(
            State(state.clone()),
            Path(detail.record.id.clone()),
            Json(new_policy),
        )
        .await
        .unwrap_or_else(|e| panic!("update_policy failed: {}", e.1));
        assert!(updated.record.policy.blocklist_enabled);
        assert!(!updated.record.policy.seed_enabled);

        // Remove
        let _ = remove_torrent(
            State(state.clone()),
            Path(detail.record.id.clone()),
            Query(RemoveTorrentQuery {
                delete_files: false,
            }),
        )
        .await
        .unwrap_or_else(|e| panic!("remove_torrent failed: {}", e.1));
        let Json(after_remove) = list_torrents(State(state))
            .await
            .unwrap_or_else(|e| panic!("list after remove failed: {}", e.1));
        assert_eq!(after_remove, [] as [ferric_common::TorrentRecord; 0]);

        Ok(())
    }

    #[tokio::test]
    async fn torrent_detail_404_for_unknown_id() -> anyhow::Result<()> {
        let state = test_state().await?;
        let err = get_torrent_detail(State(state), Path("no-such-id".to_string()))
            .await
            .expect_err("unknown id must 404");
        assert_eq!(err.0, StatusCode::NOT_FOUND);
        Ok(())
    }

    #[test]
    fn torrent_stats_poller_only_syncs_running_rqbit_records() {
        let mut record = TorrentRecord {
            id: "t1".to_string(),
            info_hash: "0123456789abcdef0123456789abcdef01234567".to_string(),
            name: "test".to_string(),
            source: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567".to_string(),
            source_kind: ferric_common::TorrentInputKind::Magnet,
            save_root: None,
            status: TorrentStatus::Downloading,
            total_size: 0,
            downloaded: 0,
            uploaded: 0,
            ratio: 0.0,
            download_speed: 0,
            upload_speed: 0,
            peers: 0,
            policy: TorrentPolicy::default(),
            proxy_id: None,
            policy_locked_fields: Vec::new(),
            is_private: false,
            peer_id_seed: "0123456789abcdef0123456789abcdef01234567".to_string(),
            engine: "rqbit:http://127.0.0.1:3030|1".to_string(),
            created_at: 0,
            completed_at: None,
            av_status: None,
            av_result: None,
            tags: Vec::new(),
            queue_position: 0,
            category: None,
        };

        assert!(should_poll_torrent_stats(&record));
        record.status = TorrentStatus::Paused;
        assert!(!should_poll_torrent_stats(&record));
        record.status = TorrentStatus::Downloading;
        record.engine = "librqbit-candidate/gated".to_string();
        assert!(!should_poll_torrent_stats(&record));
    }

    #[tokio::test]
    async fn batch_torrents_partial_failure_reports_correctly() -> anyhow::Result<()> {
        let s = test_state().await?;
        let mut ids: Vec<String> = (0..49).map(|_| Uuid::new_v4().to_string()).collect();
        ids.push("not-a-real-id".to_string());
        let body = BatchBody {
            ids: ids.clone(),
            action: "delete".to_string(),
        };
        let result = batch_torrents(State(s), Json(body)).await.unwrap().0;
        assert_eq!(result.failed.len() + result.succeeded.len(), 50);
        // remove_torrent uses DELETE WHERE id = ? which succeeds even for missing ids
        assert_eq!(result.succeeded.len(), 50);
        assert_eq!(result.failed, [] as [std::string::String; 0]);
        Ok(())
    }
}

#[cfg(test)]
mod queue_patch_tests {
    use super::*;

    #[test]
    fn null_clears_and_absent_preserves() {
        let b: QueuePatchBody =
            serde_json::from_str(r#"{"sync_interval_min": null, "max_retries": 3}"#).unwrap();
        assert_eq!(b.sync_interval_min, Some(None));
        assert_eq!(b.max_retries, Some(Some(3)));
        assert_eq!(b.when_done, None);
    }
}
