use crate::diagnostics;
use anyhow::{Context, Result};
use std::fs::File as StdFile;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};

/// Shared random-access writer for chunk workers.
///
/// Writes and reads are positioned (`seek_write`/`write_at`) on one shared
/// handle, so connections never wait on a file lock or disturb a shared file
/// cursor. The syscalls run on the blocking pool.
#[derive(Clone)]
pub struct DiskWriter {
    file: Arc<StdFile>,
    flush: Arc<FlushState>,
}

/// Lets many connections ask for "my data is on disk" without each forcing its own full-file flush.
/// One flush covers every write made before it started, so callers that arrive while a flush runs
/// wait for it (or for the next one) instead of queueing a flush each.
#[derive(Default)]
struct FlushState {
    /// Number of completed writes.
    writes: AtomicU64,
    /// Writes known to be flushed.
    flushed: AtomicU64,
    lock: tokio::sync::Mutex<()>,
}

#[cfg(windows)]
fn write_all_at(file: &StdFile, mut offset: u64, mut buf: &[u8]) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    while !buf.is_empty() {
        let n = file.seek_write(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &StdFile, mut offset: u64, mut buf: &mut [u8]) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt as _;
    while !buf.is_empty() {
        let n = file.seek_read(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        let rest = buf;
        buf = &mut rest[n..];
        offset += n as u64;
    }
    Ok(())
}

#[cfg(unix)]
fn write_all_at(file: &StdFile, offset: u64, buf: &[u8]) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, buf, offset)
}

#[cfg(unix)]
fn read_exact_at(file: &StdFile, offset: u64, buf: &mut [u8]) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

impl DiskWriter {
    pub async fn create(path: &Path, size: Option<u64>) -> Result<Self> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }

        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(path)
            .await
            .with_context(|| format!("open download file {}", path.display()))?;

        if let Some(size) = size {
            try_preallocate(&file, size).await?;
        }

        Ok(Self {
            file: Arc::new(file.into_std().await),
            flush: Arc::new(FlushState::default()),
        })
    }

    pub async fn write_at(&self, offset: u64, data: &[u8]) -> Result<()> {
        let file = self.file.clone();
        let buf = data.to_vec();
        tokio::task::spawn_blocking(move || write_all_at(&file, offset, &buf)).await??;
        self.flush.writes.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    /// Read `len` bytes at `offset` (used to verify the resume overlap).
    pub async fn read_at(&self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let file = self.file.clone();
        let buf = tokio::task::spawn_blocking(move || {
            let mut buf = vec![0_u8; len];
            read_exact_at(&file, offset, &mut buf).map(|()| buf)
        })
        .await??;
        Ok(buf)
    }

    /// Returns once everything written before this call is on disk. Concurrent callers share flushes.
    pub async fn sync(&self) -> Result<()> {
        let wanted = self.flush.writes.load(Ordering::SeqCst);
        if self.flush.flushed.load(Ordering::SeqCst) >= wanted {
            return Ok(());
        }
        let _turn = self.flush.lock.lock().await;
        if self.flush.flushed.load(Ordering::SeqCst) >= wanted {
            return Ok(()); // the flush ahead of us already covered our data
        }
        let covered = self.flush.writes.load(Ordering::SeqCst);
        let file = self.file.clone();
        tokio::task::spawn_blocking(move || file.sync_all()).await??;
        self.flush.flushed.fetch_max(covered, Ordering::SeqCst);
        Ok(())
    }

    /// One flush per call, no sharing (what every connection used to do); kept to measure the difference.
    #[cfg(test)]
    async fn sync_each(&self) -> Result<()> {
        let file = self.file.clone();
        tokio::task::spawn_blocking(move || file.sync_all()).await??;
        Ok(())
    }
}

async fn try_preallocate(file: &File, size: u64) -> Result<()> {
    mark_sparse(file).await;
    file.set_len(size).await?;
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
async fn mark_sparse(file: &File) {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::System::Ioctl::FSCTL_SET_SPARSE;
    use windows_sys::Win32::System::IO::DeviceIoControl;

    let mut bytes_returned = 0_u32;
    let ok = unsafe {
        DeviceIoControl(
            file.as_raw_handle(),
            FSCTL_SET_SPARSE,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            &raw mut bytes_returned,
            std::ptr::null_mut(),
        )
    };

    diagnostics::event(
        "disk_sparse_mark",
        serde_json::json!({
            "ok": ok != 0,
        }),
    )
    .await;
}

#[cfg(not(windows))]
async fn mark_sparse(_file: &File) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_at_offsets() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("file.bin");
        let writer = DiskWriter::create(&path, Some(8)).await?;

        writer.write_at(4, b"tail").await?;
        writer.write_at(0, b"head").await?;
        writer.sync().await?;

        let data = tokio::fs::read(&path).await?;
        assert_eq!(&data, b"headtail");
        Ok(())
    }

    /// 16 connections each writing 24 MB in 32 KB frames and asking for a flush every MB, the way the
    /// piece workers do. Run by hand: `cargo test -p ferricd sync_sharing -- --ignored --nocapture`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    #[ignore = "timing measurement, run by hand"]
    async fn sync_sharing_costs_less_than_a_flush_per_connection() -> Result<()> {
        for shared in [false, true, false] {
            let no_sync = !shared && std::env::var("FERRIC_BENCH_NOSYNC").is_ok();
            let dir = tempfile::tempdir_in(
                std::env::var("FERRIC_BENCH_DIR").unwrap_or_else(|_| ".".into()),
            )?;
            let writer =
                DiskWriter::create(&dir.path().join("f.bin"), Some(16 * 24 * 1024 * 1024)).await?;
            let started = std::time::Instant::now();
            let mut tasks = Vec::new();
            for c in 0..16_u64 {
                let w = writer.clone();
                tasks.push(tokio::spawn(async move {
                    let frame = vec![7_u8; 32 * 1024];
                    let base = c * 24 * 1024 * 1024;
                    for i in 0..(24 * 32) {
                        w.write_at(base + i * 32 * 1024, &frame).await.unwrap();
                        if (i + 1) % 32 == 0 {
                            if no_sync {
                            } else if shared {
                                w.sync().await.unwrap()
                            } else {
                                w.sync_each().await.unwrap()
                            }
                        }
                    }
                }));
            }
            for t in tasks {
                t.await.unwrap();
            }
            let secs = started.elapsed().as_secs_f64();
            println!(
                "{}: {:.2} s for 384 MB = {:.0} MB/s",
                if shared {
                    "shared flushes"
                } else if no_sync {
                    "no flush at all"
                } else {
                    "flush per connection"
                },
                secs,
                384.0 / secs
            );
        }
        Ok(())
    }
}
