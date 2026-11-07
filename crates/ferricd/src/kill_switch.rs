//! Task 188 — app-level torrent kill switch on proxy drop.

use crate::db;
use crate::torrent_manager::TorrentManager;
use ferric_common::TorrentStatus;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::time::Duration;
use tracing::{info, warn};

const INTERVAL: Duration = Duration::from_secs(30);
const PAUSED_IDS_KEY: &str = "kill_switch_paused_ids";

/// Background loop for torrent proxy fail-closed behavior:
/// - If `torrent_proxy_required=1` and `torrent_proxy_url` empty → kill switch + pause downloads
/// - If proxy URL set and health probe fails → kill switch + pause downloads
/// - On recovery: clear flag and resume only torrents this loop paused (not user-paused)
pub fn spawn(pool: SqlitePool, torrents: TorrentManager) {
    tokio::spawn(async move {
        let mut was_down = setting_truthy(
            db::settings::get(&pool, "torrent_kill_switch_active")
                .await
                .ok()
                .flatten(),
        );
        // IDs paused by this kill-switch loop — never resume user-paused torrents.
        // Restore from settings when KS was already active across a daemon restart.
        let mut paused_by_ks: HashSet<String> = if was_down {
            load_paused_ids(&pool).await
        } else {
            HashSet::new()
        };
        loop {
            tokio::time::sleep(INTERVAL).await;
            let required = setting_truthy(
                db::settings::get(&pool, "torrent_proxy_required")
                    .await
                    .ok()
                    .flatten(),
            );
            let proxy = db::settings::get(&pool, "torrent_proxy_url")
                .await
                .ok()
                .flatten()
                .filter(|p| !p.trim().is_empty());

            let (should_kill, reason) = match (required, proxy.as_deref()) {
                (true, None) => (
                    true,
                    "torrent_proxy_required=1 but torrent_proxy_url is empty".to_string(),
                ),
                (_, Some(url)) => {
                    if probe_socks5(url).await {
                        (false, String::new())
                    } else {
                        (true, format!("proxy health failed: {}", redact_proxy(url)))
                    }
                }
                (false, None) => {
                    // Optional proxy not configured — no kill-switch pressure.
                    if was_down {
                        deactivate_kill_switch(&pool, &torrents, &mut paused_by_ks).await;
                        was_down = false;
                    }
                    continue;
                }
            };

            if should_kill {
                if !was_down {
                    warn!(%reason, "activating torrent kill switch");
                    let _ = db::settings::set(&pool, "torrent_kill_switch_active", "1").await;
                    let _ = db::torrent_audit::append(
                        &pool,
                        "*",
                        "kill_switch_activated",
                        &serde_json::json!({ "reason": reason }),
                    )
                    .await;
                }
                // Pause any currently-Downloading torrents (already-Paused skipped).
                pause_active_torrents(&pool, &torrents, &mut paused_by_ks).await;
                was_down = true;
            } else if was_down {
                deactivate_kill_switch(&pool, &torrents, &mut paused_by_ks).await;
                was_down = false;
            }
        }
    });
}

async fn deactivate_kill_switch(
    pool: &SqlitePool,
    torrents: &TorrentManager,
    paused_by_ks: &mut HashSet<String>,
) {
    info!("torrent proxy recovered — clearing kill switch and auto-resuming tracked torrents");
    // Clear flag before resume so privacy gate allows start().
    let _ = db::settings::set(pool, "torrent_kill_switch_active", "0").await;
    resume_tracked_torrents(torrents, paused_by_ks).await;
    clear_paused_ids(pool).await;
    let _ = db::torrent_audit::append(pool, "*", "kill_switch_deactivated", &serde_json::json!({}))
        .await;
}

async fn pause_active_torrents(
    pool: &SqlitePool,
    torrents: &TorrentManager,
    paused_by_ks: &mut HashSet<String>,
) {
    let mut changed = false;
    if let Ok(list) = torrents.list().await {
        for t in list {
            if !matches!(
                t.status,
                TorrentStatus::Downloading | TorrentStatus::Checking
            ) {
                continue;
            }
            match torrents.pause(&t.id).await {
                Ok(_) => {
                    if paused_by_ks.insert(t.id) {
                        changed = true;
                    }
                }
                Err(e) => {
                    tracing::debug!(id = %t.id, error = %e, "kill-switch pause");
                }
            }
        }
    }
    if changed {
        persist_paused_ids(pool, paused_by_ks).await;
    }
}

async fn resume_tracked_torrents(torrents: &TorrentManager, paused_by_ks: &mut HashSet<String>) {
    let ids: Vec<String> = paused_by_ks.drain().collect();
    for id in ids {
        let status = match torrents.detail(&id).await {
            Ok(Some(d)) => d.record.status,
            Ok(None) => continue,
            Err(e) => {
                tracing::debug!(%id, error = %e, "kill-switch resume detail");
                continue;
            }
        };
        // Only resume what we paused and is still paused (skip user state changes).
        if status != TorrentStatus::Paused {
            continue;
        }
        if let Err(e) = torrents.start(&id).await {
            tracing::debug!(%id, error = %e, "kill-switch resume");
        }
    }
}

async fn load_paused_ids(pool: &SqlitePool) -> HashSet<String> {
    let raw = match db::settings::get(pool, PAUSED_IDS_KEY).await {
        Ok(Some(v)) if !v.trim().is_empty() => v,
        _ => return HashSet::new(),
    };
    match serde_json::from_str::<Vec<String>>(&raw) {
        Ok(ids) => ids.into_iter().collect(),
        Err(e) => {
            warn!(error = %e, "invalid kill_switch_paused_ids; starting empty");
            HashSet::new()
        }
    }
}

async fn persist_paused_ids(pool: &SqlitePool, paused_by_ks: &HashSet<String>) {
    let ids: Vec<&String> = paused_by_ks.iter().collect();
    let json = match serde_json::to_string(&ids) {
        Ok(j) => j,
        Err(e) => {
            warn!(error = %e, "failed to serialize kill_switch_paused_ids");
            return;
        }
    };
    if let Err(e) = db::settings::set(pool, PAUSED_IDS_KEY, &json).await {
        warn!(error = %e, "failed to persist kill_switch_paused_ids");
    }
}

async fn clear_paused_ids(pool: &SqlitePool) {
    if let Err(e) = db::settings::set(pool, PAUSED_IDS_KEY, "[]").await {
        warn!(error = %e, "failed to clear kill_switch_paused_ids");
    }
}

fn setting_truthy(value: Option<String>) -> bool {
    matches!(
        value.as_deref().map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

pub(crate) async fn probe_socks5(proxy_url: &str) -> bool {
    let Ok(url) = url::Url::parse(proxy_url) else {
        return false;
    };
    if url.scheme() != "socks5" && url.scheme() != "socks5h" && url.scheme() != "socks4" {
        // http proxy: try TCP connect only
        return tcp_connect_host(&url).await;
    }
    let host = match url.host_str() {
        Some(h) => h.to_string(),
        None => return false,
    };
    let port = url.port().unwrap_or(1080);
    let Ok(stream) = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((host.as_str(), port)),
    )
    .await
    else {
        return false;
    };
    let Ok(stream) = stream else {
        return false;
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    // SOCKS5 greeting: ver=5, nmethods=1, method=0 (no auth)
    let mut s = stream;
    if s.write_all(&[0x05, 0x01, 0x00]).await.is_err() {
        return false;
    }
    let mut buf = [0u8; 2];
    match tokio::time::timeout(Duration::from_secs(3), s.read_exact(&mut buf)).await {
        Ok(Ok(_)) => buf[0] == 0x05,
        _ => false,
    }
}

async fn tcp_connect_host(url: &url::Url) -> bool {
    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    let port = url.port_or_known_default().unwrap_or(1080);
    tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::TcpStream::connect((host, port)),
    )
    .await
    .is_ok_and(|r| r.is_ok())
}

fn redact_proxy(raw: &str) -> String {
    // strip userinfo
    if let Ok(mut u) = url::Url::parse(raw) {
        let _ = u.set_username("");
        let _ = u.set_password(None);
        return u.to_string();
    }
    "socks5://[redacted]".into()
}
