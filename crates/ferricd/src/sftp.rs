//! SFTP downloads via pure-Rust `russh` + `russh-sftp`.
//!
//! URL: `sftp://user:pass@host:port/path`
//! Auth: password from URL, else key files from `~/.ssh/` (`id_ed25519`, `id_rsa`).

use anyhow::{anyhow, Context, Result};
use ferric_common::models::ChunkStatus;
use russh::client;
use russh::keys::ssh_key::PrivateKey;
use russh_sftp::client::SftpSession;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use url::Url;

use crate::chunk::{ChunkArgs, ChunkPlan, ChunkRange, ResponseMetadata};

/// What the server key store says about a host (like SSH's known_hosts, trust on first use).
#[derive(Debug, PartialEq, Eq)]
enum HostKey {
    Known,
    New,
    Changed,
}

/// `store` has one `host:port fingerprint` pair per line.
fn classify_host_key(store: &str, host_id: &str, fingerprint: &str) -> HostKey {
    for line in store.lines() {
        let mut parts = line.split_whitespace();
        if parts.next() == Some(host_id) {
            return if parts.next() == Some(fingerprint) {
                HostKey::Known
            } else {
                HostKey::Changed
            };
        }
    }
    HostKey::New
}

fn known_hosts_path() -> PathBuf {
    ferric_common::resolve_data_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join("sftp_known_hosts")
}

/// Trust on first use: remember a server's key the first time, refuse the connection if it later changes.
struct TofuHandler {
    host_id: String,
    store: PathBuf,
    changed: Arc<std::sync::atomic::AtomicBool>,
}

impl client::Handler for TofuHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_key: &russh::keys::PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = server_key else {
            return Ok(false); // host certificates are not supported
        };
        let fingerprint = key.fingerprint(russh::keys::HashAlg::Sha256).to_string();
        let store = std::fs::read_to_string(&self.store).unwrap_or_default();
        match classify_host_key(&store, &self.host_id, &fingerprint) {
            HostKey::Known => Ok(true),
            HostKey::Changed => {
                self.changed
                    .store(true, std::sync::atomic::Ordering::Relaxed);
                Ok(false)
            }
            HostKey::New => {
                if let Some(dir) = self.store.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let line = format!(
                    "{} {}
",
                    self.host_id, fingerprint
                );
                if let Ok(mut f) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.store)
                {
                    use std::io::Write;
                    let _ = f.write_all(line.as_bytes());
                }
                Ok(true)
            }
        }
    }
}

#[derive(Clone)]
struct SftpTarget {
    host: String,
    port: u16,
    username: String,
    password: Option<String>,
    path: String,
}

fn parse_target(url_str: &str) -> Result<SftpTarget> {
    let parsed = Url::parse(url_str).context("parse sftp URL")?;
    if parsed.scheme() != "sftp" {
        return Err(anyhow!("not an sftp:// URL"));
    }
    let host = parsed
        .host_str()
        .ok_or_else(|| anyhow!("sftp URL missing host"))?
        .to_string();
    let port = parsed.port().unwrap_or(22);
    let username = if parsed.username().is_empty() {
        whoami_username()
    } else {
        percent_decode(parsed.username())
    };
    let password = parsed.password().map(percent_decode);
    let path = {
        let p = parsed.path();
        if p.is_empty() {
            "/".to_string()
        } else {
            p.to_string()
        }
    };
    Ok(SftpTarget {
        host,
        port,
        username,
        password,
        path,
    })
}

fn percent_decode(s: &str) -> String {
    let mut out = Vec::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push((h << 4) | l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

fn whoami_username() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_else(|_| "anonymous".into())
}

fn default_key_paths() -> Vec<PathBuf> {
    let mut keys = Vec::new();
    if let Some(home) = dirs_home() {
        let ssh = home.join(".ssh");
        for name in ["id_ed25519", "id_rsa", "id_ecdsa"] {
            let p = ssh.join(name);
            if p.is_file() {
                keys.push(p);
            }
        }
    }
    keys
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

async fn open_sftp(target: &SftpTarget) -> Result<(client::Handle<TofuHandler>, SftpSession)> {
    let config = client::Config::default();
    let changed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let store = known_hosts_path();
    let handler = TofuHandler {
        host_id: format!("{}:{}", target.host, target.port),
        store: store.clone(),
        changed: changed.clone(),
    };
    let mut handle = match client::connect(
        Arc::new(config),
        (target.host.as_str(), target.port),
        handler,
    )
    .await
    {
        Ok(h) => h,
        Err(e) if changed.load(std::sync::atomic::Ordering::Relaxed) => {
            return Err(anyhow!(
                "the SSH host key of {}:{} changed since it was first seen (possible man-in-the-middle);                  remove its line from {} only if you trust the change",
                target.host,
                target.port,
                store.display()
            ))
            .context(e.to_string());
        }
        Err(e) => {
            return Err(e).with_context(|| format!("sftp connect {}:{}", target.host, target.port))
        }
    };

    let mut ok = false;
    if let Some(ref pass) = target.password {
        match handle.authenticate_password(&target.username, pass).await {
            Ok(r) if r.success() => ok = true,
            Ok(_) | Err(_) => {}
        }
    }
    if !ok {
        for key_path in default_key_paths() {
            let key_data = match tokio::fs::read_to_string(&key_path).await {
                Ok(s) => s,
                Err(_) => continue,
            };
            let key = match PrivateKey::from_openssh(&key_data) {
                Ok(k) => k,
                Err(e) => {
                    tracing::debug!(path = %key_path.display(), error = %e, "skip sftp key");
                    continue;
                }
            };
            // None → russh picks legacy/default RSA hash when needed.
            let key_with_hash = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);
            match handle
                .authenticate_publickey(&target.username, key_with_hash)
                .await
            {
                Ok(r) if r.success() => {
                    ok = true;
                    break;
                }
                _ => continue,
            }
        }
    }
    if !ok {
        return Err(anyhow!(
            "sftp auth failed for {}@{}",
            target.username,
            target.host
        ));
    }

    let channel = handle
        .channel_open_session()
        .await
        .context("sftp channel")?;
    channel
        .request_subsystem(true, "sftp")
        .await
        .context("sftp subsystem")?;
    let sftp = SftpSession::new(channel.into_stream())
        .await
        .context("sftp session")?;
    Ok((handle, sftp))
}

/// Plan an SFTP download (stat size → range pieces).
pub async fn plan_sftp(url: &str, max_connections: usize) -> Result<ChunkPlan> {
    let target = parse_target(url)?;
    let (_handle, sftp) = open_sftp(&target).await?;
    let meta = sftp
        .metadata(&target.path)
        .await
        .with_context(|| format!("sftp stat {}", target.path))?;
    let file_size = meta.size.unwrap_or(0);
    let _ = sftp.close().await;
    Ok(segment_plan(file_size, max_connections))
}

fn segment_plan(file_size: u64, max_connections: usize) -> ChunkPlan {
    if file_size == 0 {
        return ChunkPlan::SingleStream {
            etag: None,
            last_modified: None,
            metadata: ResponseMetadata::default(),
        };
    }
    let concurrency = max_connections.clamp(1, 8);
    let piece_size = (file_size / concurrency as u64).max(1024 * 1024);
    let mut chunks = Vec::new();
    let mut offset = 0_u64;
    let mut index = 0_u32;
    while offset < file_size {
        let end = (offset + piece_size - 1).min(file_size - 1);
        chunks.push(ChunkRange {
            chunk_index: index,
            byte_start: offset,
            byte_end: end,
        });
        offset = end + 1;
        index += 1;
    }
    let concurrency = concurrency.min(chunks.len().max(1));
    ChunkPlan::Segmented {
        file_size,
        chunks,
        concurrency,
        etag: None,
        last_modified: None,
        metadata: ResponseMetadata::default(),
    }
}

/// Download one byte range over SFTP.
pub async fn download_sftp_chunk(args: ChunkArgs) -> Result<u64> {
    let target = parse_target(&args.url)?;
    let start = args.chunk.resume_offset();
    // The piece can be shortened while we run (another worker took its upper half).
    let end_now = || args.live.as_ref().map_or(args.chunk.byte_end, |l| l.end());
    let end = end_now();

    let (_handle, sftp) = open_sftp(&target).await?;
    let mut remote = sftp
        .open(target.path.clone())
        .await
        .with_context(|| format!("sftp open {}", target.path))?;
    remote
        .seek(SeekFrom::Start(start))
        .await
        .context("sftp seek")?;

    let mut buf = vec![0_u8; 64 * 1024];
    let mut offset = start;
    let mut total_written = args.chunk.downloaded;
    let mut last_flush = std::time::Instant::now();
    let mut unflushed = 0_u64;

    loop {
        if *args.cancel_rx.borrow() {
            return Err(ferric_common::FerricError::Cancelled.into());
        }
        let end = end_now();
        if offset > end {
            break;
        }
        let to_read = (buf.len() as u64).min(end - offset + 1) as usize;
        let n = remote
            .read(&mut buf[..to_read])
            .await
            .context("sftp read")?;
        if n == 0 {
            break;
        }
        args.disk.write_at(offset, &buf[..n]).await?;
        args.bandwidth.acquire(n).await;
        offset += n as u64;
        total_written = args.chunk.downloaded + (offset - start);
        if let Some(live) = &args.live {
            live.set_done(total_written);
        }
        unflushed += n as u64;
        if unflushed >= 1_048_576 || last_flush.elapsed() >= std::time::Duration::from_secs(1) {
            args.disk.sync().await?;
            crate::db::chunks::update_chunk_progress(
                &args.pool,
                args.chunk.id,
                total_written,
                ChunkStatus::Downloading,
            )
            .await?;
            unflushed = 0;
            last_flush = std::time::Instant::now();
        }
    }

    if offset <= end_now() {
        return Err(anyhow!(
            "sftp EOF early, missing {} bytes of range {start}-{end}",
            end_now() + 1 - offset
        ));
    }
    args.disk.sync().await?;

    crate::db::chunks::update_chunk_progress(
        &args.pool,
        args.chunk.id,
        total_written,
        ChunkStatus::Completed,
    )
    .await?;
    Ok(total_written)
}

#[cfg(test)]
mod tests {
    #[test]
    fn host_keys_are_trusted_on_first_use_and_refused_when_they_change() {
        use super::{classify_host_key, HostKey};
        let store = "a.example:22 SHA256:aaa
b.example:2222 SHA256:bbb
";
        assert_eq!(
            classify_host_key(store, "a.example:22", "SHA256:aaa"),
            HostKey::Known
        );
        assert_eq!(
            classify_host_key(store, "a.example:22", "SHA256:evil"),
            HostKey::Changed
        );
        assert_eq!(
            classify_host_key(store, "c.example:22", "SHA256:ccc"),
            HostKey::New
        );
        assert_eq!(
            classify_host_key(store, "b.example:22", "SHA256:bbb"),
            HostKey::New,
            "the port is part of the identity"
        );
    }

    use super::*;

    #[test]
    fn parse_sftp_url() {
        let t = parse_target("sftp://alice:s3cret@files.example.com:2222/data/a.bin").unwrap();
        assert_eq!(t.host, "files.example.com");
        assert_eq!(t.port, 2222);
        assert_eq!(t.username, "alice");
        assert_eq!(t.password.as_deref(), Some("s3cret"));
        assert_eq!(t.path, "/data/a.bin");
    }

    #[test]
    fn segment_plan_splits() {
        let plan = segment_plan(10_000_000, 4);
        match plan {
            ChunkPlan::Segmented {
                file_size,
                chunks,
                concurrency,
                ..
            } => {
                assert_eq!(file_size, 10_000_000);
                assert!(!chunks.is_empty());
                assert!(concurrency >= 1);
            }
            _ => panic!("expected segmented"),
        }
    }
}
