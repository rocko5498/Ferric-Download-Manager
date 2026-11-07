use core::pin::Pin;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::Duration;

use cxx_qt::{CxxQtThread, Threading};
use futures::StreamExt;

use crate::daemon_session::{check_health, read_lock_file};

const POLL_INTERVAL: Duration = Duration::from_millis(1000);
const RETRY_BACKOFF: Duration = Duration::from_secs(2);
const NOTIFY_THROTTLE: Duration = Duration::from_millis(300);

/// Per-download live speed/ETA from SSE progress events (not on GET /downloads).
#[derive(Debug, Clone, Copy, Default)]
pub struct LiveSpeed {
    pub speed_bps: i64,
    pub eta_secs: Option<u64>,
}

static SPEED_CACHE: LazyLock<Mutex<HashMap<String, LiveSpeed>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Snapshot of live speed/ETA for a download (or torrent id sharing the cache).
pub fn live_speed_for(id: &str) -> Option<LiveSpeed> {
    SPEED_CACHE.lock().ok()?.get(id).copied()
}

fn cache_upsert(id: &str, speed_bps: i64, eta_secs: Option<u64>) {
    if let Ok(mut map) = SPEED_CACHE.lock() {
        map.insert(
            id.to_owned(),
            LiveSpeed {
                speed_bps,
                eta_secs,
            },
        );
    }
}

fn cache_remove(id: &str) {
    if let Ok(mut map) = SPEED_CACHE.lock() {
        map.remove(id);
    }
}

fn cache_clear() {
    if let Ok(mut map) = SPEED_CACHE.lock() {
        map.clear();
    }
}

fn cache_totals() -> (i64, i32) {
    match SPEED_CACHE.lock() {
        Ok(map) => {
            let total_bps: i64 = map.values().map(|e| e.speed_bps).sum();
            let active = map.len() as i32;
            (total_bps, active)
        }
        Err(_) => (0, 0),
    }
}

/// Drop all live speeds and push zeros to QML so active_downloads/bps stay honest
/// after SSE disconnect or before poll fallback / retry.
fn clear_speed_cache_and_notify(qt_thread: &CxxQtThread<qobject::ProgressHub>) {
    cache_clear();
    notify_speeds(qt_thread, 0, 0);
    notify_updated(qt_thread);
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        #[qproperty(bool, live)]
        #[qproperty(i64, download_bps)]
        #[qproperty(i32, active_downloads)]
        type ProgressHub = super::ProgressHubRust;

        #[qsignal]
        fn updated(self: Pin<&mut ProgressHub>);

        /// A browser capture is waiting for the Download File Info dialog.
        #[qsignal]
        fn capture_held(self: Pin<&mut ProgressHub>, id: QString);

        /// A download failed with HTTP 401: ask for a user name and password (like IDM).
        #[qsignal]
        fn login_required(self: Pin<&mut ProgressHub>, id: QString);

        /// A download failed with HTTP 407: the proxy wants a user name and password.
        #[qsignal]
        fn proxy_login_required(self: Pin<&mut ProgressHub>, id: QString);

        /// The probe of a held capture finished (name, size, type, category filled in).
        #[qsignal]
        fn capture_updated(self: Pin<&mut ProgressHub>, id: QString);

        /// One download finished.
        #[qsignal]
        fn download_done(self: Pin<&mut ProgressHub>, id: QString);

        /// One download failed for good (not a login prompt).
        #[qsignal]
        fn download_failed(self: Pin<&mut ProgressHub>, id: QString);

        /// Another Ferric launch asked this window to come forward.
        #[qsignal]
        fn show_ui(self: Pin<&mut ProgressHub>);

        #[qinvokable]
        fn start(self: Pin<&mut ProgressHub>);
    }

    impl cxx_qt::Threading for ProgressHub {}
}

#[derive(Default)]
pub struct ProgressHubRust {
    live: bool,
    download_bps: i64,
    active_downloads: i32,
}

static STARTED: AtomicBool = AtomicBool::new(false);

impl qobject::ProgressHub {
    pub fn start(self: Pin<&mut Self>) {
        if STARTED.swap(true, Ordering::SeqCst) {
            return;
        }

        let qt_thread = self.qt_thread();

        std::thread::spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::warn!("ProgressHub: failed to start tokio runtime: {e}");
                    return;
                }
            };
            rt.block_on(run_hub(qt_thread));
        });
    }
}

async fn run_hub(qt_thread: CxxQtThread<qobject::ProgressHub>) {
    loop {
        let (port, token) = match read_lock_file() {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("ProgressHub: {e}");
                set_live(&qt_thread, false);
                tokio::time::sleep(RETRY_BACKOFF).await;
                continue;
            }
        };

        match run_sse(port, &token, &qt_thread).await {
            Ok(()) => {
                // Stream ended cleanly (daemon closed it) — fall through and retry.
                tracing::info!("ProgressHub: SSE stream ended, retrying");
                clear_speed_cache_and_notify(&qt_thread);
                set_live(&qt_thread, false);
            }
            Err(e) => {
                tracing::warn!("ProgressHub: SSE unavailable ({e}), falling back to poll");
                clear_speed_cache_and_notify(&qt_thread);
                set_live(&qt_thread, false);
                run_poll_until_daemon_recovers(port, &token, &qt_thread).await;
            }
        }
        tokio::time::sleep(RETRY_BACKOFF).await;
    }
}

fn set_live(qt_thread: &CxxQtThread<qobject::ProgressHub>, live: bool) {
    let _ = qt_thread.queue(move |mut qobj: Pin<&mut qobject::ProgressHub>| {
        if *qobj.as_ref().live() != live {
            qobj.as_mut().set_live(live);
        }
    });
}

fn notify_updated(qt_thread: &CxxQtThread<qobject::ProgressHub>) {
    let _ = qt_thread.queue(|qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.updated();
    });
}

fn notify_capture_held(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.capture_held(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_login_required(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.login_required(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_proxy_login_required(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.proxy_login_required(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_show_ui(qt_thread: &CxxQtThread<qobject::ProgressHub>) {
    let _ = qt_thread.queue(|qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.show_ui();
    });
}

fn notify_download_done(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.download_done(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_download_failed(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.download_failed(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_capture_updated(qt_thread: &CxxQtThread<qobject::ProgressHub>, id: String) {
    let _ = qt_thread.queue(move |qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.capture_updated(cxx_qt_lib::QString::from(&id));
    });
}

fn notify_speeds(qt_thread: &CxxQtThread<qobject::ProgressHub>, bps: i64, active: i32) {
    let _ = qt_thread.queue(move |mut qobj: Pin<&mut qobject::ProgressHub>| {
        qobj.as_mut().set_download_bps(bps);
        qobj.as_mut().set_active_downloads(active);
    });
}

async fn run_sse(
    port: u16,
    token: &str,
    qt_thread: &CxxQtThread<qobject::ProgressHub>,
) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/events");
    let resp = crate::http_client::STREAM_CLIENT
        .clone()
        .get(&url)
        .header("X-Ferric-Auth", token)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !resp.status().is_success() {
        return Err(format!("daemon returned {}", resp.status()));
    }

    set_live(qt_thread, true);
    notify_updated(qt_thread);

    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut last_notify = std::time::Instant::now() - NOTIFY_THROTTLE;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| e.to_string())?;
        buf.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(idx) = buf.find("\n\n") {
            let event_block = buf[..idx].to_string();
            buf.drain(..=idx + 1);
            for line in event_block.lines() {
                if let Some(data) = line.strip_prefix("data: ") {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(data) {
                        let event = val.get("event").and_then(|v| v.as_str()).unwrap_or("");
                        if event == "progress" {
                            if let Some(id) = val["id"].as_str() {
                                let bps = val["speed_bps"].as_i64().unwrap_or(0);
                                let eta_secs = val["eta_secs"].as_u64();
                                let status = val["status"].as_str().unwrap_or("");
                                // paused: remove so 0-speed rows don't inflate active_downloads
                                let drop_from_cache = matches!(
                                    status,
                                    "completed" | "cancelled" | "error" | "paused"
                                );
                                if drop_from_cache {
                                    cache_remove(id);
                                    notify_updated(qt_thread);
                                    last_notify = std::time::Instant::now();
                                } else {
                                    cache_upsert(id, bps, eta_secs);
                                }
                            }
                            let (total_bps, active) = cache_totals();
                            notify_speeds(qt_thread, total_bps, active);
                        } else if event == "show_ui" {
                            notify_show_ui(qt_thread);
                        } else if event == "capture_updated" {
                            if let Some(id) = val["record"]["id"].as_str() {
                                notify_capture_updated(qt_thread, id.to_owned());
                            }
                        } else if event == "capture_held" {
                            if let Some(id) = val["record"]["id"].as_str() {
                                notify_capture_held(qt_thread, id.to_owned());
                            }
                            notify_updated(qt_thread);
                            last_notify = std::time::Instant::now();
                        } else if matches!(event, "done" | "failed" | "removed") {
                            if let Some(id) = val["id"].as_str() {
                                if event == "failed"
                                    && val["message"].as_str().is_some_and(|m| m.contains("401"))
                                {
                                    notify_login_required(qt_thread, id.to_owned());
                                }
                                if event == "failed"
                                    && val["message"].as_str().is_some_and(|m| m.contains("407"))
                                {
                                    notify_proxy_login_required(qt_thread, id.to_owned());
                                }
                                if event == "done" {
                                    notify_download_done(qt_thread, id.to_owned());
                                }
                                if event == "failed"
                                    && !val["message"]
                                        .as_str()
                                        .is_some_and(|m| m.contains("401") || m.contains("407"))
                                {
                                    notify_download_failed(qt_thread, id.to_owned());
                                }
                                cache_remove(id);
                                let (total_bps, active) = cache_totals();
                                notify_speeds(qt_thread, total_bps, active);
                                notify_updated(qt_thread);
                                last_notify = std::time::Instant::now();
                            }
                        } else if event == "torrent_updated" {
                            // F28: torrent speeds live on record after engine poll (F26 fields).
                            if let Some(rec) = val.get("record") {
                                if let Some(id) = rec.get("id").and_then(|v| v.as_str()) {
                                    let bps = rec
                                        .get("download_speed")
                                        .and_then(|v| v.as_u64())
                                        .unwrap_or(0)
                                        as i64;
                                    let status =
                                        rec.get("status").and_then(|v| v.as_str()).unwrap_or("");
                                    let is_terminal = matches!(
                                        status,
                                        "completed"
                                            | "paused"
                                            | "error"
                                            | "engine_gated"
                                            | "staged"
                                    );
                                    if is_terminal {
                                        cache_remove(id);
                                    } else {
                                        // Torrents: speed only; ETA not tracked on this path.
                                        cache_upsert(id, bps, None);
                                    }
                                    let (total_bps, active) = cache_totals();
                                    notify_speeds(qt_thread, total_bps, active);
                                }
                            }
                        }
                    }
                    if last_notify.elapsed() >= NOTIFY_THROTTLE {
                        notify_updated(qt_thread);
                        last_notify = std::time::Instant::now();
                    }
                }
            }
        }
    }

    Ok(())
}

/// Poll `/downloads` and `/torrents` every `POLL_INTERVAL` until the daemon's
/// `/health` responds again, then hand control back to `run_hub`'s retry loop
/// so it re-attempts SSE.
async fn run_poll_until_daemon_recovers(
    port: u16,
    token: &str,
    qt_thread: &CxxQtThread<qobject::ProgressHub>,
) {
    // Timed client, not STREAM_CLIENT: this is a recovery poll, and an untimed
    // request against a half-alive daemon would park the loop forever, so SSE
    // would never be re-attempted.
    let client = crate::http_client::HTTP_CLIENT.clone();
    loop {
        let downloads_ok = client
            .get(format!("http://127.0.0.1:{port}/downloads"))
            .header("X-Ferric-Auth", token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());
        let torrents_ok = client
            .get(format!("http://127.0.0.1:{port}/torrents"))
            .header("X-Ferric-Auth", token)
            .send()
            .await
            .is_ok_and(|r| r.status().is_success());

        if downloads_ok || torrents_ok {
            notify_updated(qt_thread);
        }

        tokio::time::sleep(POLL_INTERVAL).await;

        if check_health(port, token).await.is_ok() {
            return;
        }

        // F-1: a restarted daemon comes back on a NEW port with a NEW token, so the
        // health check above can never succeed against the captured (dead) one and this
        // loop would poll forever. Hand control back to run_hub as soon as the lock file
        // names a different endpoint — it re-reads the lock and re-attempts SSE.
        if let Ok((current_port, current_token)) = read_lock_file() {
            if current_port != port || current_token != token {
                tracing::info!(
                    dead_port = port,
                    new_port = current_port,
                    "daemon reappeared on a new port; leaving recovery poll"
                );
                return;
            }
        }
    }
}
