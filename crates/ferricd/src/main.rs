#![cfg_attr(all(not(debug_assertions), windows), windows_subsystem = "windows")]
#![cfg_attr(windows, windows_subsystem = "windows")]

mod api;
mod auth;
mod autostart;
mod av;
mod categories;
mod chunk;
mod dash;
mod db;
mod diagnostic;
mod diagnostics;
mod disk;
mod dns;
mod ffmpeg_sandbox;
mod ftp;
#[cfg(test)]
mod fuzz_tests;
mod geoip;
mod grabber;
mod hls;
mod import_qbt;
mod ipc;
mod job;
mod kill_switch;
mod manager;
mod media;
mod pac;
mod page_download;
#[cfg(test)]
mod phase1_tests;
mod postdownload;
mod privacy;
mod product_mode;
mod quota;
mod retry;
mod rss_watch;
mod scheduler;
mod security;
mod sftp;
mod site_logins;
mod socks4;
mod speed;
mod sync;
mod throttle;
mod torrent;
mod torrent_engine;
mod torrent_manager;
mod webdav;
mod webhooks;
mod zip_preview;

use anyhow::Result;
use ferric_common::API_LOCK_FILE;
use single_instance::SingleInstance;
use tokio::signal;
use tracing::info;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[tokio::main]
async fn main() -> Result<()> {
    // Resolve data dir before tracing so logs go to a file (no console window)
    let data_dir = ferric_common::resolve_data_dir().map_err(|e| anyhow::anyhow!(e))?;
    std::fs::create_dir_all(&data_dir)?;

    init_tracing(&data_dir);

    // Prevent multiple daemon instances
    let instance = SingleInstance::new("ferric-daemon")
        .map_err(|e| anyhow::anyhow!("single-instance check failed: {e}"))?;
    if !instance.is_single() {
        tracing::warn!("ferricd is already running — exiting");
        return Ok(());
    }

    info!(version = VERSION, data_dir = %data_dir.display(), "ferricd starting");

    // Product: native IEs + yt-dlp as site-coverage engine (see docs/YTDLP-INTEGRATION.md).
    {
        let ies = ferric_extract::REGISTRY.len();
        #[cfg(feature = "ytdlp-subprocess")]
        {
            let st = ferric_extract::ytdlp::status().await;
            if st.available {
                info!(
                    native_ies = ies,
                    path = st.path.as_deref().unwrap_or("?"),
                    version = st.version.as_deref().unwrap_or("?"),
                    "extract: native fast-path + yt-dlp engine ready"
                );
            } else {
                info!(
                    native_ies = ies,
                    "extract: native IEs only — install yt-dlp or set FERRIC_YTDLP for full site coverage"
                );
            }
        }
        #[cfg(not(feature = "ytdlp-subprocess"))]
        {
            info!(
                native_ies = ies,
                "extract: native IEs only (yt-dlp engine not compiled in)"
            );
        }
    }

    // Initialise database
    let db_path = data_dir.join("ferric.db");
    let pool = db::init(&db_path).await?;

    if let Some(n) = db::settings::get(&pool, "max_retries")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<u32>().ok())
    {
        retry::set_max_attempts(n);
    }
    security::set_allow_lan(
        db::settings::get(&pool, "allow_lan_downloads")
            .await
            .ok()
            .flatten()
            .as_deref()
            == Some("1"),
    );

    // Task 182 — optional GeoIP MMDB (no auto-download)
    geoip::configure_from_data_dir(&data_dir);
    if let Ok(Some(p)) = db::settings::get(&pool, "geoip_mmdb_path").await {
        if !p.trim().is_empty() {
            geoip::set_mmdb_path(Some(std::path::PathBuf::from(p)));
        }
    }
    geoip::log_status(&data_dir);

    // Sync OS autostart state with the persisted setting
    // Only act on an explicit choice: a second instance (portable, dev, test data
    // dir) with no `auto_start` row used to delete the autostart entry that the
    // user's real installation had registered.
    if let Ok(val) = db::settings::get(&pool, "auto_start").await {
        if let Ok(exe) = std::env::current_exe() {
            match val.as_deref() {
                Some("1") => {
                    autostart::register(&exe).ok();
                }
                Some("0") => {
                    autostart::unregister().ok();
                }
                _ => {}
            }
        }
    }

    // Build manager
    let manager = manager::DownloadManager::new(pool.clone()).await?;
    let manager_tx = manager.command_tx();
    let progress_tx = manager.progress_tx();

    // Recover incomplete downloads from previous session
    manager.recover_incomplete().await?;

    // Task 188 — torrent proxy kill switch
    let torrent_mgr = torrent_manager::TorrentManager::new(pool.clone());
    // Resume engines for torrents that were actively downloading when the
    // daemon last stopped (mirrors manager.recover_incomplete() above) --
    // must run before torrent_mgr moves into kill_switch::spawn below.
    torrent_mgr.resume_active().await;
    kill_switch::spawn(pool.clone(), torrent_mgr);

    // FQT-102 — RSS auto-download poll loop
    rss_watch::spawn(pool.clone(), manager_tx.clone());

    // Start IPC server (receives connections from ferric-host)
    let ipc_handle = tokio::spawn(ipc::serve(
        pool.clone(),
        manager_tx.clone(),
        progress_tx.clone(),
    ));

    // Start axum HTTP API (ferric-qt is the shipped UI; ferric-ui/Tauri is deprecated, FQT-112/113)
    let api_handle = tokio::spawn(api::serve(
        pool.clone(),
        manager_tx.clone(),
        progress_tx,
        data_dir.clone(),
    ));

    // Run manager event loop
    let manager_handle = tokio::spawn(manager.run());

    sync::spawn(pool.clone(), manager_tx.clone());

    // IDM's "Start download on IDM startup", per queue.
    for queue in db::queues::list(&pool).await.unwrap_or_default() {
        if queue.start_on_startup {
            if let Err(e) = api::start_queue_downloads(&pool, &manager_tx, queue.id).await {
                tracing::warn!(error = %e, queue = %queue.name, "startup queue start failed");
            }
        }
    }

    info!("ferricd ready");

    tokio::select! {
        r = ipc_handle     => { tracing::error!("IPC server exited: {r:?}"); }
        r = api_handle     => { tracing::error!("API server exited: {r:?}"); }
        r = manager_handle => { tracing::error!("Manager exited: {r:?}"); }
        _ = signal::ctrl_c() => { tracing::info!("received Ctrl-C, shutting down"); }
        () = wait_for_sigterm() => { tracing::info!("received SIGTERM, shutting down"); }
    }

    tokio::fs::remove_file(data_dir.join(API_LOCK_FILE))
        .await
        .ok();
    tracing::info!("ferricd stopped");
    Ok(())
}

async fn wait_for_sigterm() {
    #[cfg(unix)]
    {
        if let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) {
            sig.recv().await;
        }
    }

    #[cfg(not(unix))]
    {
        std::future::pending::<()>().await;
    }
}

fn init_tracing(data_dir: &std::path::Path) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::Layer;

    let log_path = data_dir.join("ferricd.log");
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "ferricd=info,ferric_common=info".into());

    let fmt_layer = match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
    {
        Ok(file) => tracing_subscriber::fmt::layer()
            .with_writer(std::sync::Mutex::new(file))
            .with_ansi(false)
            .with_target(false)
            .boxed(),
        Err(_) => {
            // Unwritable data dir — still log so daemon can surface the failure.
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(false)
                .with_target(false)
                .boxed()
        }
    };

    #[cfg(feature = "tokio-console")]
    {
        tracing_subscriber::registry()
            .with(fmt_layer.with_filter(env_filter))
            .with(console_subscriber::spawn())
            .init();
    }
    #[cfg(not(feature = "tokio-console"))]
    {
        tracing_subscriber::registry()
            .with(fmt_layer.with_filter(env_filter))
            .init();
    }
}
