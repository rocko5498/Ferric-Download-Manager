use crate::{db, manager::ManagerCommand, security};
use ferric_common::messages::{DaemonMessage, DaemonReply, HostMessage};
use ferric_common::ProgressEvent;
use sqlx::SqlitePool;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};
const VERSION: &str = env!("CARGO_PKG_VERSION");
const MAX_FRAME: usize = 4_194_304; // 4 MB

const CAPTURE_SETTINGS_KEYS: &[&str] = &[
    "capture_enabled",
    "min_capture_size",
    "capture_extensions",
    "capture_mime_prefixes",
    "capture_exclude_urls",
    "capture_panel_exclude_hosts",
    "capture_manifests",
];

pub async fn serve(
    pool: SqlitePool,
    manager_tx: mpsc::Sender<ManagerCommand>,
    progress_tx: broadcast::Sender<ProgressEvent>,
) -> anyhow::Result<()> {
    platform::listen(pool, manager_tx, progress_tx).await
}

// â”€â”€ Async frame I/O â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

async fn read_frame(r: &mut (impl AsyncReadExt + Unpin)) -> anyhow::Result<Vec<u8>> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf) as usize;
    anyhow::ensure!(len <= MAX_FRAME, "IPC frame too large: {len} bytes");
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn write_frame(w: &mut (impl AsyncWriteExt + Unpin), data: &[u8]) -> anyhow::Result<()> {
    w.write_all(&(data.len() as u32).to_le_bytes()).await?;
    w.write_all(data).await?;
    w.flush().await?;
    Ok(())
}

// â”€â”€ Per-connection handler (platform-agnostic) â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

async fn handle_connection<S>(
    mut stream: S,
    manager_tx: mpsc::Sender<ManagerCommand>,
    pool: SqlitePool,
    progress_tx: broadcast::Sender<ProgressEvent>,
) -> anyhow::Result<()>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    loop {
        let bytes = match read_frame(&mut stream).await {
            Ok(b) => b,
            // Client disconnected cleanly
            Err(e) if is_eof(&e) => break,
            Err(e) => return Err(e),
        };

        let msg: HostMessage =
            postcard::from_bytes(&bytes).map_err(|e| anyhow::anyhow!("IPC deserialize: {e}"))?;

        let reply = dispatch(msg, &manager_tx, &pool, &progress_tx).await;

        let encoded =
            postcard::to_stdvec(&reply).map_err(|e| anyhow::anyhow!("IPC serialize: {e}"))?;
        write_frame(&mut stream, &encoded).await?;
    }
    Ok(())
}

async fn dispatch(
    msg: HostMessage,
    manager_tx: &mpsc::Sender<ManagerCommand>,
    pool: &SqlitePool,
    progress_tx: &broadcast::Sender<ProgressEvent>,
) -> DaemonMessage {
    match msg {
        HostMessage::Ping => DaemonMessage::Pong {
            version: VERSION.into(),
        },
        HostMessage::DownloadReq(req) => {
            if let Err(e) = security::validate_url(&req.url) {
                return DaemonMessage::Reply(DaemonReply::Error {
                    id: req.id,
                    message: e.to_string(),
                });
            }

            let (tx, rx) = tokio::sync::oneshot::channel();
            // "Show start download dialog" off = start straight away, like IDM.
            let hold = db::settings::get_flag(pool, "show_start_dialog", true).await;
            if manager_tx
                .send(ManagerCommand::Add {
                    req: req.clone(),
                    save_dir_override: None,
                    hold,
                    audio_url: None,
                    subtitle_url: None,
                    subtitle_lang: None,
                    proxy_url: None,
                    reply_tx: tx,
                })
                .await
                .is_err()
            {
                return DaemonMessage::Reply(DaemonReply::Error {
                    id: req.id,
                    message: "daemon shutting down".into(),
                });
            }
            let reply = rx.await.unwrap_or_else(|_| DaemonReply::Error {
                id: req.id,
                message: "manager dropped reply channel".into(),
            });
            DaemonMessage::Reply(reply)
        }
        HostMessage::ExtractFormats { url } => match crate::page_download::formats(url, pool).await
        {
            Ok(json) => DaemonMessage::Formats { json },
            Err(message) => DaemonMessage::Reply(DaemonReply::Error {
                id: uuid::Uuid::nil(),
                message,
            }),
        },
        HostMessage::PageDownload {
            url,
            format_id,
            cookies,
            referrer,
            title,
        } => DaemonMessage::Reply(
            crate::page_download::run(url, format_id, cookies, referrer, title, manager_tx, pool)
                .await,
        ),
        HostMessage::MediaCandidate(candidate) => {
            if let Err(e) = security::validate_url(&candidate.url) {
                return DaemonMessage::Reply(DaemonReply::Error {
                    id: uuid::Uuid::nil(),
                    message: e.to_string(),
                });
            }
            let _ = progress_tx.send(ProgressEvent::MediaCandidate { candidate });
            DaemonMessage::Ack
        }
        HostMessage::GetCapturePolicy => {
            let mut settings = std::collections::HashMap::new();
            for key in CAPTURE_SETTINGS_KEYS {
                match db::settings::get(pool, key).await {
                    Ok(Some(value)) => {
                        settings.insert(key.to_string(), value);
                    }
                    Ok(None) => {}
                    Err(e) => tracing::warn!("failed to read capture setting {key}: {e}"),
                }
            }
            DaemonMessage::CapturePolicy { settings }
        }
        HostMessage::SetCapturePolicy { settings } => {
            for (key, value) in &settings {
                if CAPTURE_SETTINGS_KEYS.contains(&key.as_str()) {
                    if let Err(e) = db::settings::set(pool, key, value).await {
                        tracing::warn!("failed to write capture setting {key}: {e}");
                    }
                }
            }
            DaemonMessage::Ack
        }
    }
}

fn is_eof(e: &anyhow::Error) -> bool {
    e.downcast_ref::<std::io::Error>()
        .is_some_and(|io| io.kind() == std::io::ErrorKind::UnexpectedEof)
}

// â”€â”€ Platform-specific listeners â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[cfg(target_os = "linux")]
mod platform {
    use super::handle_connection;
    use crate::manager::ManagerCommand;
    use ferric_common::ProgressEvent;
    use sqlx::SqlitePool;
    use tokio::net::UnixListener;
    use tokio::sync::{broadcast, mpsc};

    pub async fn listen(
        pool: SqlitePool,
        manager_tx: mpsc::Sender<ManagerCommand>,
        progress_tx: broadcast::Sender<ProgressEvent>,
    ) -> anyhow::Result<()> {
        let uid = unsafe { libc::getuid() };
        let socket_dir = format!("/run/user/{uid}/ferric");
        tokio::fs::create_dir_all(&socket_dir).await?;
        let socket_path = format!("{socket_dir}/daemon.sock");

        // Remove stale socket from a previous crash
        let _ = tokio::fs::remove_file(&socket_path).await;

        let listener = UnixListener::bind(&socket_path)?;
        tracing::info!(path = %socket_path, "IPC server listening");

        loop {
            let (stream, _addr) = listener.accept().await?;
            let tx = manager_tx.clone();
            let p = pool.clone();
            let progress = progress_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(stream, tx, p, progress).await {
                    tracing::warn!("IPC connection closed: {e}");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferric_common::messages::DownloadRequest;

    fn request_with_url(url: &str) -> DownloadRequest {
        DownloadRequest {
            id: uuid::Uuid::new_v4(),
            url: url.to_string(),
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
        }
    }

    #[tokio::test]
    async fn dispatch_rejects_invalid_download_url_before_manager() {
        let (tx, mut rx) = mpsc::channel(1);
        let pool = sqlx::SqlitePool::connect(":memory:").await.unwrap();
        let id = uuid::Uuid::new_v4();
        let mut req = request_with_url("javascript:alert(1)");
        req.id = id;

        let (progress_tx, _) = broadcast::channel(4);
        let reply = dispatch(HostMessage::DownloadReq(req), &tx, &pool, &progress_tx).await;

        match reply {
            DaemonMessage::Reply(DaemonReply::Error {
                id: reply_id,
                message,
            }) => {
                assert_eq!(reply_id, id);
                assert!(message.contains("scheme not allowed"));
            }
            other => panic!("unexpected reply: {other:?}"),
        }
        assert!(rx.try_recv().is_err());
    }
}

#[cfg(windows)]
mod platform {
    use super::handle_connection;
    use crate::manager::ManagerCommand;
    use ferric_common::ProgressEvent;
    use sqlx::SqlitePool;
    use tokio::net::windows::named_pipe::ServerOptions;
    use tokio::sync::{broadcast, mpsc};

    pub async fn listen(
        pool: SqlitePool,
        manager_tx: mpsc::Sender<ManagerCommand>,
        progress_tx: broadcast::Sender<ProgressEvent>,
    ) -> anyhow::Result<()> {
        let username = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        let pipe_name = format!(r"\\.\pipe\ferric-{username}");
        tracing::info!(pipe = %pipe_name, "IPC server listening");

        loop {
            // Each iteration creates a new pipe instance for the next client.
            let server = ServerOptions::new()
                .access_inbound(true)
                .access_outbound(true)
                .create(&pipe_name)?;

            server.connect().await?;

            let tx = manager_tx.clone();
            let p = pool.clone();
            let progress = progress_tx.clone();
            tokio::spawn(async move {
                if let Err(e) = handle_connection(server, tx, p, progress).await {
                    tracing::warn!("IPC connection closed: {e}");
                }
            });
        }
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    use crate::manager::ManagerCommand;
    use sqlx::SqlitePool;
    use tokio::sync::mpsc;

    pub async fn listen(
        _pool: SqlitePool,
        _manager_tx: mpsc::Sender<ManagerCommand>,
        _progress_tx: tokio::sync::broadcast::Sender<ferric_common::ProgressEvent>,
    ) -> anyhow::Result<()> {
        anyhow::bail!("IPC not supported on this platform");
    }
}
