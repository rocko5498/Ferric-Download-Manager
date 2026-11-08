//! FTP and FTPS (explicit + implicit) via suppaftp.
//!
//! - `ftp://`            plain FTP port 21
//! - `ftps://`           explicit TLS (AUTH TLS) port 21
//! - `ftps-implicit://`  implicit TLS port 990

use anyhow::{anyhow, Context, Result};
use ferric_common::models::ChunkStatus;
use std::sync::Arc;
use suppaftp::tokio::{AsyncFtpStream, AsyncRustlsConnector, AsyncRustlsFtpStream};
use tokio::io::AsyncReadExt;
use url::Url;

use crate::chunk::{ChunkArgs, ChunkPlan, ResponseMetadata};

enum FtpSession {
    Plain(AsyncFtpStream),
    Tls(AsyncRustlsFtpStream),
}

fn rustls_connector() -> Result<AsyncRustlsConnector> {
    // Install ring provider once (shared with hyper client).
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut roots = rustls::RootCertStore::empty();
    let loaded = rustls_native_certs::load_native_certs();
    for err in &loaded.errors {
        tracing::debug!(error = %err, "native cert load warning");
    }
    for cert in loaded.certs {
        let _ = roots.add(cert);
    }
    if roots.is_empty() {
        return Err(anyhow!("no native root certificates available for FTPS"));
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(AsyncRustlsConnector::from(
        tokio_rustls::TlsConnector::from(Arc::new(config)),
    ))
}

async fn connect_ftp(url_str: &str) -> Result<FtpSession> {
    let parsed = Url::parse(url_str).context("parse ftp URL")?;
    let scheme = parsed.scheme();
    if !matches!(scheme, "ftp" | "ftps" | "ftps-implicit") {
        return Err(anyhow!("not an FTP/FTPS URL"));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("FTP URL missing host"))?
        .to_string();
    let default_port = match scheme {
        "ftps-implicit" => 990,
        _ => 21,
    };
    let port = parsed.port().unwrap_or(default_port);
    // URL userinfo is percent-encoded (`a%40b.c`); the FTP server needs the real text.
    let decode = |raw: &str| crate::chunk::percent_decode(raw).unwrap_or_else(|| raw.to_string());
    let username = if parsed.username().is_empty() {
        "anonymous".to_string()
    } else {
        decode(parsed.username())
    };
    let password = parsed
        .password()
        .map_or_else(|| "anonymous@example.com".to_string(), decode);

    let mut session = match scheme {
        "ftp" => {
            let stream = AsyncFtpStream::connect((host.as_str(), port))
                .await
                .with_context(|| format!("ftp connect {host}:{port}"))?;
            FtpSession::Plain(stream)
        }
        "ftps" => {
            let connector = rustls_connector()?;
            let stream = AsyncRustlsFtpStream::connect((host.as_str(), port))
                .await
                .with_context(|| format!("ftps connect {host}:{port}"))?
                .into_secure(connector, &host)
                .await
                .context("ftps AUTH TLS")?;
            FtpSession::Tls(stream)
        }
        "ftps-implicit" => {
            let connector = rustls_connector()?;
            let stream = AsyncRustlsFtpStream::connect_secure_implicit(
                (host.as_str(), port),
                connector,
                &host,
            )
            .await
            .with_context(|| format!("ftps-implicit connect {host}:{port}"))?;
            FtpSession::Tls(stream)
        }
        _ => unreachable!(),
    };

    session.login(&username, &password).await?;
    session.set_binary().await?;
    Ok(session)
}

impl FtpSession {
    async fn login(&mut self, user: &str, pass: &str) -> Result<()> {
        match self {
            Self::Plain(s) => s
                .login(user, pass)
                .await
                .map_err(|e| anyhow!("ftp login: {e}")),
            Self::Tls(s) => s
                .login(user, pass)
                .await
                .map_err(|e| anyhow!("ftps login: {e}")),
        }
    }

    async fn set_binary(&mut self) -> Result<()> {
        match self {
            Self::Plain(s) => s
                .transfer_type(suppaftp::types::FileType::Binary)
                .await
                .map_err(|e| anyhow!("ftp TYPE I: {e}")),
            Self::Tls(s) => s
                .transfer_type(suppaftp::types::FileType::Binary)
                .await
                .map_err(|e| anyhow!("ftps TYPE I: {e}")),
        }
    }

    async fn size(&mut self, path: &str) -> Result<usize> {
        match self {
            Self::Plain(s) => s.size(path).await.map_err(|e| anyhow!("ftp SIZE: {e}")),
            Self::Tls(s) => s.size(path).await.map_err(|e| anyhow!("ftps SIZE: {e}")),
        }
    }

    async fn quit(self) -> Result<()> {
        match self {
            Self::Plain(mut s) => {
                let _ = s.quit().await;
            }
            Self::Tls(mut s) => {
                let _ = s.quit().await;
            }
        }
        Ok(())
    }

    async fn resume_transfer(&mut self, offset: usize) -> Result<()> {
        match self {
            Self::Plain(s) => s
                .resume_transfer(offset)
                .await
                .map_err(|e| anyhow!("ftp REST: {e}")),
            Self::Tls(s) => s
                .resume_transfer(offset)
                .await
                .map_err(|e| anyhow!("ftps REST: {e}")),
        }
    }

    async fn download_range(
        &mut self,
        path: &str,
        byte_start: u64,
        byte_end: u64,
        already: u64,
        args: &ChunkArgs,
    ) -> Result<u64> {
        let start = byte_start + already;
        // The piece may be shortened while we download (another worker took its
        // upper half); always read the current end from the shared live view.
        let end_now = || args.live.as_ref().map_or(byte_end, |l| l.end());
        if start > end_now() {
            return Ok(already);
        }
        self.resume_transfer(start as usize).await?;

        let mut offset = start;
        let mut buffer = vec![0_u8; 32 * 1024];
        let mut last_flush = std::time::Instant::now();
        let mut unflushed = 0_u64;
        let mut stopped_early = false;

        macro_rules! pump {
            ($stream:expr, $label:literal) => {{
                let mut data_stream = $stream
                    .retr_as_stream(path)
                    .await
                    .map_err(|e| anyhow!(concat!($label, " RETR: {}"), e))?;
                loop {
                    if *args.cancel_rx.borrow() {
                        return Err(anyhow!("Cancelled by user"));
                    }
                    let end = end_now();
                    if offset > end {
                        stopped_early = true; // piece was split: stop at the new end
                        break;
                    }
                    let want = ((end - offset + 1).min(buffer.len() as u64)) as usize;
                    let n = data_stream.read(&mut buffer[..want]).await?;
                    if n == 0 {
                        break;
                    }
                    args.disk.write_at(offset, &buffer[..n]).await?;
                    offset += n as u64;
                    args.bandwidth.acquire(n).await;
                    let downloaded = already + (offset - start);
                    if let Some(live) = &args.live {
                        live.set_done(downloaded);
                    }
                    unflushed += n as u64;
                    if unflushed >= 1_048_576
                        || last_flush.elapsed() >= std::time::Duration::from_secs(1)
                    {
                        args.disk.sync().await?;
                        crate::db::chunks::update_chunk_progress(
                            &args.pool,
                            args.chunk.id,
                            downloaded,
                            ChunkStatus::Downloading,
                        )
                        .await?;
                        unflushed = 0;
                        last_flush = std::time::Instant::now();
                    }
                }
                // Closing the data connection early makes the server answer 426;
                // that is expected after a deliberate stop.
                let finished = data_stream.finish().await;
                if !stopped_early {
                    finished.map_err(|e| anyhow!(concat!($label, " finalize RETR: {}"), e))?;
                }
            }};
        }

        match self {
            Self::Plain(stream) => pump!(stream, "ftp"),
            Self::Tls(stream) => pump!(stream, "ftps"),
        }

        if offset <= end_now() {
            return Err(anyhow!(
                "FTP connection closed prematurely (missing {} bytes)",
                end_now() + 1 - offset
            ));
        }
        args.disk.sync().await?;
        Ok(already + (offset - start))
    }
}

/// Plan an FTP/FTPS download.
/// Remote path from the URL, percent-decoded (`a%20b` -> `a b`).
fn ftp_path(url: &str) -> Result<String> {
    let parsed = Url::parse(url)?;
    let raw = parsed.path();
    Ok(crate::chunk::percent_decode(raw).unwrap_or_else(|| raw.to_string()))
}

pub async fn plan_ftp(url: &str, max_connections: usize) -> Result<ChunkPlan> {
    let mut stream = connect_ftp(url).await?;
    let path = ftp_path(url)?;
    let size = stream.size(&path).await?;
    stream.quit().await?;

    let file_size = size as u64;
    if file_size == 0 {
        return Ok(ChunkPlan::SingleStream {
            etag: None,
            last_modified: None,
            metadata: ResponseMetadata::default(),
        });
    }

    // Same layout as HTTP: 4 pieces to start, then dynamic halving (one control
    // connection per piece, like IDM's 4 simultaneous FTP logins).
    let concurrency = crate::chunk::planned_chunk_count(file_size, max_connections);
    let chunks = crate::chunk::plan_pieces(file_size, concurrency);

    Ok(ChunkPlan::Segmented {
        file_size,
        chunks,
        concurrency,
        etag: None,
        last_modified: None,
        metadata: ResponseMetadata::default(),
    })
}

/// Download a specific chunk via FTP/FTPS.
pub async fn download_ftp_chunk(args: ChunkArgs) -> Result<u64> {
    let path = ftp_path(&args.url)?;
    let mut stream = connect_ftp(&args.url).await?;
    let written = stream
        .download_range(
            &path,
            args.chunk.byte_start,
            args.chunk.byte_end,
            args.chunk.downloaded,
            &args,
        )
        .await?;
    stream.quit().await?;
    crate::db::chunks::update_chunk_progress(
        &args.pool,
        args.chunk.id,
        written,
        ChunkStatus::Completed,
    )
    .await?;
    Ok(written)
}

#[cfg(test)]
mod tests {
    #[test]
    fn scheme_defaults() {
        assert_eq!(
            url::Url::parse("ftp://h/a")
                .unwrap()
                .port_or_known_default(),
            Some(21)
        );
        // custom scheme has no known default
        assert!(url::Url::parse("ftps-implicit://h/a")
            .unwrap()
            .port()
            .is_none());
    }
}
