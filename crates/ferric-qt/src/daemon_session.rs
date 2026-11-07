use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;

/// Parse `ferric.lock` contents: line 1 is the daemon's port, line 2 is the
/// auth token. Format is written by `write_lock_file_contents` in
/// `crates/ferricd/src/api.rs` and read the same way by `crates/ferric-ui/src/main.rs`
/// — kept in sync with both by hand since this is a plain two-line text format,
/// not a shared parser crate.
pub fn parse_lock_contents(contents: &str) -> Result<(u16, String), String> {
    let mut lines = contents.lines();
    let port: u16 = lines
        .next()
        .ok_or_else(|| "lock file: missing port line".to_string())?
        .trim()
        .parse()
        .map_err(|e| format!("lock file: invalid port: {e}"))?;
    let token = lines
        .next()
        .ok_or_else(|| "lock file: missing auth token".to_string())?
        .trim()
        .to_string();
    Ok((port, token))
}

pub(crate) fn read_lock_file() -> Result<(u16, String), String> {
    let data_dir = ferric_common::resolve_data_dir()?;
    let lock_path = data_dir.join(ferric_common::API_LOCK_FILE);
    let contents = std::fs::read_to_string(&lock_path)
        .map_err(|e| format!("cannot read lock file {}: {e}", lock_path.display()))?;
    parse_lock_contents(&contents)
}

pub(crate) async fn check_health(port: u16, token: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/health");
    let resp = crate::http_client::HTTP_CLIENT
        .clone()
        .get(&url)
        .header("X-Ferric-Auth", token)
        .send()
        .await
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("daemon returned {}", resp.status()))
    }
}

/// In-process guard so two near-simultaneous `ensure_daemon()` calls (e.g. two
/// quick `refresh()` invokes) don't both spawn a `ferricd` process. Cross-process
/// double-spawn is already handled by `ferricd`'s own `single-instance` guard
/// (see `crates/ferricd/src/main.rs`) — this only covers the basic in-process case.
static SPAWN_IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Spawn `ferricd` from next to the running executable. Mirrors
/// `spawn_daemon_if_needed` in `crates/ferric-ui/src/main.rs` exactly (same
/// executable-resolution and `CREATE_NO_WINDOW` spawn flags) so both front
/// ends behave identically. Never elevates — plain `Command::new`, no runas/UAC.
fn spawn_ferricd() -> Result<(), String> {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
        .ok_or_else(|| "cannot determine executable directory".to_string())?;

    let daemon = exe_dir.join(if cfg!(windows) {
        "ferricd.exe"
    } else {
        "ferricd"
    });
    if !daemon.exists() {
        let msg = format!("daemon executable not found at {}", daemon.display());
        tracing::warn!("{msg}");
        return Err(msg);
    }

    let mut command = std::process::Command::new(&daemon);
    command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    match command.spawn() {
        Ok(_) => {
            tracing::info!("started daemon from {}", daemon.display());
            Ok(())
        }
        Err(e) => {
            let msg = format!("failed to start daemon {}: {e}", daemon.display());
            tracing::warn!("{msg}");
            Err(msg)
        }
    }
}

/// Passive check first; if the daemon isn't reachable, spawn it and poll for
/// up to ~10s for the lock file + health check to succeed.
async fn ensure_daemon() -> Result<(u16, String), String> {
    if let Ok((port, token)) = read_lock_file() {
        if check_health(port, &token).await.is_ok() {
            return Ok((port, token));
        }
        // T0: stale ferric.lock after crash points at a dead port (live 2026-07-17:
        // lock 57605, connection refused). Remove so a new ferricd can write a fresh lock.
        if let Ok(data_dir) = ferric_common::resolve_data_dir() {
            let lock_path = data_dir.join(ferric_common::API_LOCK_FILE);
            if let Err(e) = std::fs::remove_file(&lock_path) {
                tracing::debug!(
                    path = %lock_path.display(),
                    error = %e,
                    "could not remove stale lock before spawn"
                );
            } else {
                tracing::info!(
                    path = %lock_path.display(),
                    dead_port = port,
                    "removed stale lock (daemon not reachable)"
                );
            }
        }
    }

    if SPAWN_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return Err("daemon spawn already in progress".to_string());
    }
    let spawn_result = spawn_ferricd();
    SPAWN_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    spawn_result?;

    for _ in 0..50 {
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        if let Ok((port, token)) = read_lock_file() {
            if check_health(port, &token).await.is_ok() {
                return Ok((port, token));
            }
        }
    }
    let msg = "daemon did not become reachable within 10s of starting".to_string();
    tracing::warn!("{msg}");
    Err(msg)
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
        #[qproperty(bool, connected)]
        #[qproperty(i32, port)]
        #[qproperty(QString, last_error)]
        type DaemonSession = super::DaemonSessionRust;

        /// Read the lock file and hit `GET /health`, updating the properties above.
        /// Runs a fresh single-threaded Tokio runtime synchronously — this blocks
        /// the calling (QML/UI) thread for the duration of one local HTTP request.
        /// Acceptable for this scaffolding task; a non-blocking version (signal on
        /// completion, run off the UI thread) is follow-up work if this ever shows
        /// as janky in practice.
        #[qinvokable]
        fn refresh(self: Pin<&mut Self>);

        /// Health-check only — never spawns ferricd. Use on periodic list refresh so a
        /// dead daemon does not block the UI for ~10s of spawn polling every tick.
        #[qinvokable]
        fn ping(self: Pin<&mut Self>);
    }

    impl cxx_qt::Threading for DaemonSession {}
}

#[derive(Default)]
pub struct DaemonSessionRust {
    connected: bool,
    port: i32,
    last_error: QString,
}

impl qobject::DaemonSession {
    fn apply_connected(mut self: Pin<&mut Self>, port: u16) {
        self.as_mut().set_port(i32::from(port));
        self.as_mut().set_connected(true);
        self.as_mut().set_last_error(QString::from(""));
    }

    fn apply_disconnected(mut self: Pin<&mut Self>, e: &str) {
        self.as_mut().set_connected(false);
        self.as_mut().set_last_error(QString::from(e));
    }

    /// Checks the daemon; if it's not reachable, spawns `ferricd` and waits
    /// (up to ~10s) for it to come up before giving up. Covers cold start —
    /// launching the UI with no daemon running brings it up automatically.
    pub fn refresh(mut self: Pin<&mut Self>) {
        let qt_thread = self.as_mut().qt_thread();
        crate::http_client::RUNTIME.spawn(async move {
            let result = ensure_daemon().await.map(|(port, _token)| port);
            let _ = qt_thread.queue(move |mut session| match result {
                Ok(port) => session.as_mut().apply_connected(port),
                Err(e) => session.as_mut().apply_disconnected(&e),
            });
        });
    }

    pub fn ping(mut self: Pin<&mut Self>) {
        let qt_thread = self.as_mut().qt_thread();
        crate::http_client::RUNTIME.spawn(async move {
            let result = async {
                let (port, token) = read_lock_file()?;
                check_health(port, &token).await?;
                Ok::<u16, String>(port)
            }
            .await;
            let _ = qt_thread.queue(move |mut session| match result {
                Ok(port) => session.as_mut().apply_connected(port),
                Err(e) => session.as_mut().apply_disconnected(&e),
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_lock_contents() {
        let (port, token) = parse_lock_contents("50655\nabc123token\n").unwrap();
        assert_eq!(port, 50655);
        assert_eq!(token, "abc123token");
    }

    #[test]
    fn trims_whitespace_on_both_lines() {
        let (port, token) = parse_lock_contents("  50655  \n  abc123token  \n").unwrap();
        assert_eq!(port, 50655);
        assert_eq!(token, "abc123token");
    }

    #[test]
    fn rejects_empty_contents() {
        assert!(parse_lock_contents("").is_err());
    }

    #[test]
    fn rejects_non_numeric_port() {
        let err = parse_lock_contents("notaport\ntoken\n").unwrap_err();
        assert!(err.contains("invalid port"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_missing_token_line() {
        let err = parse_lock_contents("50655\n").unwrap_err();
        assert!(err.contains("auth token"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_port_out_of_u16_range() {
        assert!(parse_lock_contents("99999999\ntoken\n").is_err());
    }
}
