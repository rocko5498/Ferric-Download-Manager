//! ferric-host — Chrome native messaging relay
//!
//! Chrome spawns this binary when the extension calls `connectNative()`.
//! It reads JSON messages from stdin, forwards them to ferricd over IPC,
//! and writes responses back to stdout.
//!
//! Deliberately sync-only (no tokio) to keep the binary small.

mod install;

use ferric_common::messages::{DaemonMessage, DaemonReply, HostMessage};
use std::io::{self, Read, Write};
use std::process;

/// The packed extension (Ferric.crx) is signed with a key that gives this id.
const DEV_EXTENSION_ID: &str = "gepkkfjfgdcgelcnejpikdeanggknbjo";
/// The id of the earlier unpacked builds; still accepted so an extension loaded from a folder keeps working.
const LEGACY_EXTENSION_ID: &str = "bngipfinnjpcnebfchfbeinfppepibkb";
const DEV_FIREFOX_EXTENSION_ID: &str = "ferric@ferric.local";
/// Release builds must set these at compile time. Unset → no production allowlist entry.
/// FERRIC_EXTENSION_ID may list several ids separated by commas.
const CONFIGURED_EXTENSION_ID: Option<&str> = option_env!("FERRIC_EXTENSION_ID");
const CONFIGURED_FIREFOX_EXTENSION_ID: Option<&str> = option_env!("FERRIC_FIREFOX_EXTENSION_ID");

fn main() {
    match handle_install_flags() {
        Ok(true) => return,
        Ok(false) => {}
        Err(e) => {
            eprintln!("ferric-host: {e}");
            process::exit(1);
        }
    }

    if let Err(e) = run() {
        // Write an error reply to Chrome stdout before exiting
        let reply = serde_json::json!({
            "type": "error",
            "message": format!("{e}")
        });
        let _ = write_chrome_message(&mut io::stdout(), reply.to_string().as_bytes());
        process::exit(1);
    }
}

fn handle_install_flags() -> anyhow::Result<bool> {
    let mut args = std::env::args().skip(1);
    let Some(flag) = args.next() else {
        return Ok(false);
    };

    match flag.as_str() {
        "--install" => {
            let rest: Vec<String> = args.collect();
            let offer_extension = rest.iter().any(|a| a == "--offer-extension");
            let ids: Vec<String> = match rest.into_iter().find(|a| !a.starts_with("--")) {
                Some(one) => vec![one],
                None => {
                    let configured: Vec<String> =
                        configured_extension_ids().map(str::to_string).collect();
                    if !configured.is_empty() {
                        configured
                    } else if cfg!(debug_assertions) {
                        vec![
                            DEV_EXTENSION_ID.to_string(),
                            LEGACY_EXTENSION_ID.to_string(),
                        ]
                    } else {
                        anyhow::bail!(
                            "extension id required: pass --install <id> or set FERRIC_EXTENSION_ID at build time"
                        )
                    }
                }
            };
            install::install(&ids, offer_extension)?;
            eprintln!(
                "ferric-host: installed native messaging host for {}",
                ids.join(", ")
            );
            Ok(true)
        }
        "--uninstall" => {
            install::uninstall()?;
            eprintln!("ferric-host: uninstalled native messaging host");
            Ok(true)
        }
        "--status" => {
            let registered = install::is_registered();
            eprintln!("{}", serde_json::json!({ "registered": registered }));
            Ok(true)
        }
        "--help" | "-h" => {
            eprintln!(
                "Usage:\n  ferric-host --install [extension-id]\n  ferric-host --uninstall\n  ferric-host --status\n  ferric-host"
            );
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    validate_caller()?;

    let mut stdin = io::stdin();
    let mut stdout = io::stdout();

    // Ensure daemon is running before we try to connect
    ensure_daemon_running()?;

    let mut ipc = connect_to_daemon()?;

    loop {
        // Read one Chrome native message
        let chrome_json = match read_chrome_message(&mut stdin) {
            Ok(msg) => msg,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => break, // Chrome closed
            Err(e) => return Err(e.into()),
        };

        // Deserialize Chrome message → HostMessage
        let host_msg: HostMessage = serde_json::from_slice(&chrome_json)?;
        if matches!(
            host_msg,
            HostMessage::DownloadReq(_)
                | HostMessage::MediaCandidate(_)
                | HostMessage::PageDownload { .. }
        ) {
            let _ = ensure_ui_running();
        }

        // Forward to daemon as postcard frame
        let encoded = postcard::to_stdvec(&host_msg)?;
        write_ipc_frame(&mut ipc, &encoded)?;

        // Read daemon response
        let response_bytes = read_ipc_frame(&mut ipc)?;
        let daemon_msg: DaemonMessage = postcard::from_bytes(&response_bytes)?;

        // Convert daemon response to JSON for Chrome
        let chrome_reply = daemon_message_to_json(&daemon_msg);
        write_chrome_message(&mut stdout, chrome_reply.as_bytes())?;
    }

    Ok(())
}

// ── Extension ID validation ───────────────────────────────────────────────────

fn validate_caller() -> Result<(), String> {
    // Chromium passes "chrome-extension://EXTENSION_ID/". Firefox passes
    // the manifest path followed by the add-on ID, so scan all arguments.
    let args: Vec<String> = std::env::args().collect();
    let id = args
        .iter()
        .find_map(|arg| chromium_extension_id(arg).or_else(|| firefox_extension_id(arg)))
        .ok_or("no extension caller argument")?;

    if !is_extension_allowed(&id) {
        return Err(format!("extension not authorized: {id}"));
    }
    Ok(())
}

fn configured_extension_ids() -> impl Iterator<Item = &'static str> {
    CONFIGURED_EXTENSION_ID
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

fn is_extension_allowed(id: &str) -> bool {
    if configured_extension_ids().any(|cfg| cfg == id)
        || CONFIGURED_FIREFOX_EXTENSION_ID.is_some_and(|cfg| cfg == id)
    {
        return true;
    }
    // Dev IDs only in debug builds — never permanently authorize unpacked IDs in release.
    cfg!(debug_assertions)
        && (id == DEV_EXTENSION_ID || id == LEGACY_EXTENSION_ID || id == DEV_FIREFOX_EXTENSION_ID)
}

fn chromium_extension_id(arg: &str) -> Option<String> {
    arg.strip_prefix("chrome-extension://")
        .and_then(|s| s.strip_suffix('/'))
        .map(ToString::to_string)
}

fn firefox_extension_id(arg: &str) -> Option<String> {
    let id = arg.trim();
    let looks_like_addon_id = id.contains('@')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.chars().any(char::is_whitespace);
    looks_like_addon_id.then(|| id.to_string())
}

// ── Chrome native messaging framing ─────────────────────────────────────────

fn read_chrome_message(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_ne_bytes(len_buf) as usize;
    if len > 1_048_576 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "chrome message too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

fn write_chrome_message(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    let len = (data.len() as u32).to_ne_bytes();
    w.write_all(&len)?;
    w.write_all(data)?;
    w.flush()
}

// ── IPC framing (4-byte LE length prefix + postcard payload) ─────────────────

fn write_ipc_frame(w: &mut impl Write, data: &[u8]) -> io::Result<()> {
    w.write_all(&(data.len() as u32).to_le_bytes())?;
    w.write_all(data)?;
    w.flush()
}

fn read_ipc_frame(r: &mut impl Read) -> io::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > 4_194_304 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "IPC frame too large",
        ));
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf)?;
    Ok(buf)
}

// ── Daemon connection ─────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn connect_to_daemon() -> io::Result<impl Read + Write> {
    use std::os::unix::net::UnixStream;
    let uid = unsafe { libc::getuid() };
    let path = format!("/run/user/{uid}/ferric/daemon.sock");
    for attempt in 0..5u32 {
        match UnixStream::connect(&path) {
            Ok(stream) => return Ok(stream),
            Err(_) if attempt < 4 => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

#[cfg(windows)]
fn connect_to_daemon() -> io::Result<impl Read + Write> {
    use std::fs::OpenOptions;
    let username = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    let pipe_path = format!(r"\\.\pipe\ferric-{username}");
    for attempt in 0..5u32 {
        match OpenOptions::new().read(true).write(true).open(&pipe_path) {
            Ok(f) => return Ok(f),
            Err(_) if attempt < 4 => std::thread::sleep(std::time::Duration::from_millis(200)),
            Err(e) => return Err(e),
        }
    }
    unreachable!()
}

// ── Daemon autostart ──────────────────────────────────────────────────────────

/// In-process guard so two near-simultaneous host calls don't both spawn ferricd.
/// Cross-process double-spawn is handled by ferricd's single-instance guard.
static SPAWN_IN_FLIGHT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn ensure_daemon_running() -> io::Result<()> {
    // Fast path: IPC already open (pipe / unix socket).
    if is_daemon_reachable() {
        return Ok(());
    }

    // Another call in this process is already spawning — wait for it.
    if SPAWN_IN_FLIGHT.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return wait_for_daemon_reachable();
    }

    let spawn_result = (|| -> io::Result<()> {
        let exe = std::env::current_exe()?;
        let ferricd = exe
            .parent()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not find parent dir"))?
            .join(if cfg!(windows) {
                "ferricd.exe"
            } else {
                "ferricd"
            });

        if !ferricd.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("ferricd not found at {}", ferricd.display()),
            ));
        }

        let mut command = std::process::Command::new(&ferricd);
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

        command
            .spawn()
            .map_err(|e| io::Error::new(e.kind(), format!("failed to start ferricd: {e}")))?;
        Ok(())
    })();

    if let Err(e) = spawn_result {
        SPAWN_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
        return Err(e);
    }

    let wait_result = wait_for_daemon_reachable();
    SPAWN_IN_FLIGHT.store(false, std::sync::atomic::Ordering::SeqCst);
    wait_result
}

/// Poll IPC reachability for up to ~10s (matches ferric-qt `ensure_daemon` margin).
fn wait_for_daemon_reachable() -> io::Result<()> {
    for _ in 0..50 {
        if is_daemon_reachable() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    Err(io::Error::new(
        io::ErrorKind::TimedOut,
        "daemon did not become reachable within 10s of starting",
    ))
}

fn ensure_ui_running() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let parent = exe
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "could not find parent dir"))?;

    // FQT-113: ferric-qt is the shipped UI; ferric-ui is deprecated legacy fallback.
    let candidates: &[&str] = if cfg!(windows) {
        &["ferric-qt.exe", "ferric-ui.exe"]
    } else {
        &["ferric-qt", "ferric-ui"]
    };

    let Some(ui) = candidates
        .iter()
        .map(|name| parent.join(name))
        .find(|path| path.exists())
    else {
        return Ok(());
    };

    let mut command = std::process::Command::new(&ui);
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

    command.spawn().map(|_| ()).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!("failed to start UI {}: {e}", ui.display()),
        )
    })
}

#[cfg(target_os = "linux")]
fn is_daemon_reachable() -> bool {
    use std::os::unix::net::UnixStream;
    let uid = unsafe { libc::getuid() };
    UnixStream::connect(format!("/run/user/{uid}/ferric/daemon.sock")).is_ok()
}

#[cfg(windows)]
fn is_daemon_reachable() -> bool {
    let username = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(format!(r"\\.\pipe\ferric-{username}"))
        .is_ok()
}

// ── Response conversion ───────────────────────────────────────────────────────

fn daemon_message_to_json(msg: &DaemonMessage) -> String {
    match msg {
        DaemonMessage::Reply(DaemonReply::Accepted { id }) => {
            serde_json::json!({ "type": "accepted", "id": id }).to_string()
        }
        DaemonMessage::Reply(DaemonReply::Error { id, message }) => {
            serde_json::json!({ "type": "error", "id": id, "message": message }).to_string()
        }
        DaemonMessage::Pong { version } => {
            serde_json::json!({ "type": "pong", "version": version }).to_string()
        }
        DaemonMessage::SiteRules(rules) => {
            serde_json::json!({ "type": "site_rules", "rules": rules }).to_string()
        }
        DaemonMessage::CapturePolicy { settings } => {
            serde_json::json!({ "type": "capture_policy", "settings": settings }).to_string()
        }
        DaemonMessage::Ack => serde_json::json!({ "type": "ok" }).to_string(),
        DaemonMessage::Formats { json } => serde_json::json!({
            "type": "formats",
            "data": serde_json::from_str::<serde_json::Value>(json).unwrap_or_default()
        })
        .to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_chromium_native_message_origin() {
        assert_eq!(
            chromium_extension_id("chrome-extension://bngipfinnjpcnebfchfbeinfppepibkb/"),
            Some("bngipfinnjpcnebfchfbeinfppepibkb".to_string())
        );
        assert_eq!(chromium_extension_id("--parent-window=123"), None);
    }

    #[test]
    fn parses_firefox_native_message_addon_id() {
        assert_eq!(
            firefox_extension_id("ferric@ferric.local"),
            Some("ferric@ferric.local".to_string())
        );
        assert_eq!(
            firefox_extension_id(
                r"C:\Users\someone\AppData\Roaming\Mozilla\NativeMessagingHosts\com.ferric.host.json"
            ),
            None
        );
    }

    #[test]
    fn allows_configured_development_callers() {
        assert!(is_extension_allowed("bngipfinnjpcnebfchfbeinfppepibkb"));
        assert!(is_extension_allowed("gepkkfjfgdcgelcnejpikdeanggknbjo"));
        assert!(is_extension_allowed("ferric@ferric.local"));
        assert!(!is_extension_allowed("other@example.org"));
    }
}
