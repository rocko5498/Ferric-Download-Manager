//! Phase 1 engine tests: idle timeout, piece re-queue, resume-overlap check,
//! HEAD-less probing and servers that ignore `Range`. Each test runs a real
//! `job::run` against a small fault-injecting HTTP server.

use crate::job::{run, JobContext};
use crate::throttle::BandwidthLimiter;
use crate::{chunk, db};
use anyhow::Result;
use ferric_common::messages::DownloadRequest;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{broadcast, watch, Mutex};
use uuid::Uuid;

const SIZE: usize = 6 * 1024 * 1024;

fn payload() -> Arc<Vec<u8>> {
    Arc::new(
        (0..SIZE)
            .map(|i| ((i * 31 + (i >> 8) * 17) & 0xff) as u8)
            .collect(),
    )
}

#[derive(Default)]
struct Faults {
    /// Status for HEAD requests (0 = normal 200).
    head_status: u16,
    /// Number of initial HEAD requests answered 503 before behaving normally.
    head_fail: AtomicU32,
    /// HEAD requests received.
    heads: AtomicU32,
    /// Plain (range-less, non-HEAD) probe requests received.
    probes: AtomicU32,
    /// Answer ranged GETs with a full 200 body.
    ignore_range: bool,
    /// Plain GET answers without `Accept-Ranges` (a server that cannot do ranges at all).
    no_ranges_header: bool,
    /// Entity tag advertised on plain GETs.
    etag: Mutex<Option<String>>,
    /// Switch the ETag right after answering the planning GET (file replaced meanwhile).
    flip_etag_after_probe: bool,
    /// `If-Range` values received on ranged requests.
    if_ranges: Mutex<Vec<String>>,
    /// Ranged requests with start > 0 that stall mid-body (socket held open).
    stall: AtomicU32,
    /// Ranged requests with start > 0 that are cut mid-body.
    cut: AtomicU32,
    /// Ranged requests with start > 0 that crawl (4 KiB per 40 ms).
    slow: AtomicU32,
    /// Per-connection throughput in bytes/s for normal ranged responses (0 = unthrottled).
    rate: u64,
    /// `Some((user, pass, digest))`: every request must carry valid credentials.
    auth: Option<(String, String, bool)>,
    /// Digest `nc` values seen on accepted requests (must be unique).
    accepted_nc: Mutex<Vec<String>>,
    /// Requests answered 401.
    challenges: AtomicU32,
    ranges: Mutex<Vec<(usize, usize)>>,
    starts: Mutex<Vec<std::time::Instant>>,
    inflight: AtomicUsize,
    peak: AtomicUsize,
}

struct Server {
    url: String,
    faults: Arc<Faults>,
}

fn parse_range(request: &str) -> Option<(usize, Option<usize>)> {
    let line = request
        .lines()
        .find(|l| l.to_ascii_lowercase().starts_with("range:"))?;
    let spec = line.split("bytes=").nth(1)?.trim();
    let (a, b) = spec.split_once('-')?;
    Some((a.parse().ok()?, b.parse().ok()))
}

async fn spawn_server(data: Arc<Vec<u8>>, faults: Faults) -> Result<Server> {
    spawn_listener(data, faults, None).await
}

async fn spawn_tls_server(
    data: Arc<Vec<u8>>,
    faults: Faults,
    acceptor: tokio_rustls::TlsAcceptor,
) -> Result<Server> {
    spawn_listener(data, faults, Some(acceptor)).await
}

async fn spawn_listener(
    data: Arc<Vec<u8>>,
    faults: Faults,
    acceptor: Option<tokio_rustls::TlsAcceptor>,
) -> Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let faults = Arc::new(faults);
    let server_faults = faults.clone();
    let scheme = if acceptor.is_some() { "https" } else { "http" };
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let data = data.clone();
            let faults = server_faults.clone();
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                match acceptor {
                    Some(acceptor) => {
                        if let Ok(tls) = acceptor.accept(stream).await {
                            serve(tls, data, faults).await;
                        }
                    }
                    None => serve(stream, data, faults).await,
                }
            });
        }
    });
    Ok(Server {
        url: format!("{scheme}://{addr}/payload.bin"),
        faults,
    })
}

/// Serve one request, then flush and close the stream. (Dropping a TLS stream
/// without flushing can lose the last records, which looked like a stalled
/// download in the tests.)
async fn serve<S>(mut stream: S, data: Arc<Vec<u8>>, faults: Arc<Faults>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    serve_inner(&mut stream, data, faults).await;
    let _ = stream.flush().await;
    let _ = stream.shutdown().await;
}

async fn serve_inner<S>(stream: &mut S, data: Arc<Vec<u8>>, faults: Arc<Faults>)
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut buf = vec![0_u8; 4096];
    let Ok(n) = stream.read(&mut buf).await else {
        return;
    };
    let request = String::from_utf8_lossy(&buf[..n]).to_string();
    if let Some((user, pass, digest)) = &faults.auth {
        if !authorized(&request, user, pass, *digest, &faults).await {
            faults.challenges.fetch_add(1, Ordering::SeqCst);
            let challenge = if *digest {
                "Digest realm=\"t\", nonce=\"abcdef0123456789\", qop=\"auth\", opaque=\"op1\", algorithm=MD5"
            } else {
                "Basic realm=\"t\""
            };
            let resp = format!(
                    "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: {challenge}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                );
            let _ = stream.write_all(resp.as_bytes()).await;
            return;
        }
    }
    // The initial probe is now a plain GET (IDM never sends HEAD): fail it like a HEAD.
    let plain_get = !request.starts_with("HEAD ") && parse_range(&request).is_none();
    if plain_get {
        faults.probes.fetch_add(1, Ordering::SeqCst);
    }
    if plain_get && take_one(&faults.head_fail) {
        let _ = stream
                .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .await;
        return;
    }
    if request.starts_with("HEAD ") {
        faults.heads.fetch_add(1, Ordering::SeqCst);
        if take_one(&faults.head_fail) {
            let _ = stream
                    .write_all(b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .await;
            return;
        }
        let resp = if faults.head_status == 0 {
            format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                    data.len()
                )
        } else {
            format!(
                "HTTP/1.1 {} X\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                faults.head_status
            )
        };
        let _ = stream.write_all(resp.as_bytes()).await;
        return;
    }
    let Some((start, end)) = parse_range(&request).filter(|_| !faults.ignore_range) else {
        // Like real servers, advertise ranges on the plain GET the job now plans with.
        let ranges_header = if faults.no_ranges_header {
            ""
        } else {
            "Accept-Ranges: bytes\r\n"
        };
        let etag_header = faults
            .etag
            .lock()
            .await
            .as_ref()
            .map_or_else(String::new, |e| format!("ETag: {e}\r\n"));
        let head = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\n{ranges_header}{etag_header}Connection: close\r\n\r\n",
            data.len()
        );
        let _ = stream.write_all(head.as_bytes()).await;
        if faults.flip_etag_after_probe && plain_get {
            *faults.etag.lock().await = Some("\"v2\"".to_string());
        }
        let _ = stream.write_all(&data).await;
        return;
    };
    // A conditional range whose validator no longer matches is answered with the whole file.
    if let Some(wanted) = header_value(&request, "if-range") {
        faults.if_ranges.lock().await.push(wanted.to_string());
        if faults.etag.lock().await.as_deref() != Some(wanted) {
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                data.len()
            );
            let _ = stream.write_all(head.as_bytes()).await;
            let _ = stream.write_all(&data).await;
            return;
        }
    }
    let end = end.unwrap_or(data.len() - 1).min(data.len() - 1);
    faults.ranges.lock().await.push((start, end));
    faults.starts.lock().await.push(std::time::Instant::now());
    let now_inflight = faults.inflight.fetch_add(1, Ordering::SeqCst) + 1;
    faults.peak.fetch_max(now_inflight, Ordering::SeqCst);
    let _guard = InflightGuard(faults.clone());
    let body = &data[start..=end];
    let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
            body.len(),
            start,
            end,
            data.len()
        );
    let _ = stream.write_all(head.as_bytes()).await;
    // A request that is only the 1-byte probe has start 0; faults hit later pieces.
    if start > 0 && take_one(&faults.stall) {
        let _ = stream.write_all(&body[..20_000.min(body.len())]).await;
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        return;
    }
    if start > 0 && take_one(&faults.cut) {
        let _ = stream.write_all(&body[..1_000.min(body.len())]).await;
        return; // closes the socket mid-body
    }
    let slow = start > 0 && take_one(&faults.slow);
    let (step, pause) = if slow {
        (4_096, 40)
    } else if faults.rate > 0 {
        (
            usize::try_from(faults.rate / 50).unwrap_or(1024).max(1024),
            20,
        )
    } else {
        (body.len().max(1), 0)
    };
    for piece in body.chunks(step) {
        if stream.write_all(piece).await.is_err() {
            return;
        }
        if pause > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(pause)).await;
        }
    }
}

fn header_value<'a>(request: &'a str, name: &str) -> Option<&'a str> {
    request.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn digest_param<'a>(header: &'a str, key: &str) -> Option<&'a str> {
    let body = header.strip_prefix("Digest ")?;
    body.split(',').find_map(|p| {
        let (k, v) = p.trim().split_once('=')?;
        (k.trim() == key).then(|| v.trim().trim_matches('"'))
    })
}

async fn authorized(request: &str, user: &str, pass: &str, digest: bool, faults: &Faults) -> bool {
    use base64::Engine as _;
    let Some(auth) = header_value(request, "authorization") else {
        return false;
    };
    if !digest {
        let want = format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
        );
        return auth == want;
    }
    let method = request.split_whitespace().next().unwrap_or("");
    let uri = request.split_whitespace().nth(1).unwrap_or("");
    let (Some(nonce), Some(nc), Some(cnonce), Some(qop), Some(resp), Some(got_uri)) = (
        digest_param(auth, "nonce"),
        digest_param(auth, "nc"),
        digest_param(auth, "cnonce"),
        digest_param(auth, "qop"),
        digest_param(auth, "response"),
        digest_param(auth, "uri"),
    ) else {
        return false;
    };
    if qop != "auth" || got_uri != uri || digest_param(auth, "username") != Some(user) {
        return false;
    }
    let ha1 = format!("{:x}", md5::compute(format!("{user}:t:{pass}")));
    let ha2 = format!("{:x}", md5::compute(format!("{method}:{uri}")));
    let want = format!(
        "{:x}",
        md5::compute(format!("{ha1}:{nonce}:{nc}:{cnonce}:{qop}:{ha2}"))
    );
    if resp != want {
        return false;
    }
    let mut seen = faults.accepted_nc.lock().await;
    if seen.iter().any(|n| n == nc) {
        return false; // replayed nonce count
    }
    seen.push(nc.to_string());
    true
}

struct InflightGuard(Arc<Faults>);

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.0.inflight.fetch_sub(1, Ordering::SeqCst);
    }
}

fn take_one(counter: &AtomicU32) -> bool {
    loop {
        let v = counter.load(Ordering::SeqCst);
        if v == 0 {
            return false;
        }
        if counter
            .compare_exchange(v, v - 1, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return true;
        }
    }
}

struct Job {
    ctx: JobContext,
    pool: sqlx::SqlitePool,
    id: Uuid,
    save_path: std::path::PathBuf,
    _dir: tempfile::TempDir,
}

async fn make_job(url: &str) -> Result<Job> {
    let dir = tempfile::tempdir()?;
    let pool = db::init(&dir.path().join("ferric.db")).await?;
    let id = Uuid::new_v4();
    let save_path = dir.path().join("payload.bin");
    let req = DownloadRequest {
        id,
        url: url.to_string(),
        final_url: None,
        filename: Some("payload.bin".to_string()),
        referrer: None,
        cookies: None,
        headers: None,
        size_hint: Some(SIZE as u64),
        mime_type: Some("application/octet-stream".to_string()),
        method: "GET".to_string(),
        post_body_b64: None,
        queue_id: None,
        checksum: None,
        mirrors: vec![],
        timestamp: 1,
    };
    let record = db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
    let (progress_tx, _) = broadcast::channel(16);
    let (cancel_tx, cancel_rx) = watch::channel(false);
    std::mem::forget(cancel_tx); // keep the channel open for the whole test
    let ctx = JobContext {
        pool: pool.clone(),
        record,
        client: reqwest::Client::new(),
        progress_tx,
        bandwidth: Arc::new(BandwidthLimiter::new(0)),
        cancel_rx,
        cookies: None,
        extra_headers: None,
        method: "GET".to_string(),
        proxy_client: None,
        post_body: None,
    };
    Ok(Job {
        ctx,
        pool,
        id,
        save_path,
        _dir: dir,
    })
}

async fn run_to_end(job: Job, secs: u64) -> Result<(Vec<u8>, Job2)> {
    let Job {
        ctx,
        pool,
        id,
        save_path,
        _dir,
    } = job;
    tokio::time::timeout(std::time::Duration::from_secs(secs), run(ctx))
        .await
        .map_err(|_| anyhow::anyhow!("job did not finish within {secs}s"))??;
    let bytes = tokio::fs::read(&save_path).await?;
    Ok((
        bytes,
        Job2 {
            _pool: pool,
            _id: id,
            _dir,
        },
    ))
}

#[allow(dead_code)]
struct Job2 {
    _pool: sqlx::SqlitePool,
    _id: Uuid,
    _dir: tempfile::TempDir,
}

#[tokio::test]
async fn stalled_connection_is_retried_after_idle_timeout() -> Result<()> {
    chunk::set_idle_timeout_ms(700);
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            stall: AtomicU32::new(1),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let started = std::time::Instant::now();
    let (bytes, _keep) = run_to_end(job, 25).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(20),
        "stalled socket held the job for {:?}",
        started.elapsed()
    );
    assert_eq!(
        server.faults.stall.load(Ordering::SeqCst),
        0,
        "stall fault was never hit"
    );
    Ok(())
}

#[tokio::test]
async fn failed_piece_is_requeued_instead_of_aborting_job() -> Result<()> {
    // 7 cuts > the 5 attempts of one retry round, so the piece must be re-queued.
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            cut: AtomicU32::new(7),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 60).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    assert_eq!(
        server.faults.cut.load(Ordering::SeqCst),
        0,
        "cut fault was never hit"
    );
    Ok(())
}

#[tokio::test]
async fn resume_with_wrong_bytes_on_disk_restarts_the_piece() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let job = make_job(&server.url).await?;

    // Seed a "resumable" first piece: DB says 100_000 bytes are done, disk holds garbage.
    let planned = chunk::plan_pieces(SIZE as u64, 4);
    db::chunks::insert_chunks(&job.pool, job.id, &planned).await?;
    let disk = crate::disk::DiskWriter::create(&job.save_path, Some(SIZE as u64)).await?;
    disk.write_at(0, &vec![0xAB_u8; 100_000]).await?;
    disk.sync().await?;
    let first = db::chunks::get_chunks(&job.pool, job.id).await?.remove(0);
    db::chunks::update_chunk_progress(
        &job.pool,
        first.id,
        100_000,
        ferric_common::ChunkStatus::Downloading,
    )
    .await?;

    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert!(bytes == *data, "garbage resume data was not repaired");
    let ranges = server.faults.ranges.lock().await.clone();
    assert!(
        ranges.iter().any(|r| r.0 == 100_000 - 64),
        "resume request must rewind 64 bytes, saw {ranges:?}"
    );
    assert!(
        ranges.iter().filter(|r| r.0 == 0).count() >= 1,
        "after the mismatch the piece must restart at its first byte, saw {ranges:?}"
    );
    Ok(())
}

#[tokio::test]
async fn resume_with_correct_bytes_continues_after_overlap() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let job = make_job(&server.url).await?;

    let planned = chunk::plan_pieces(SIZE as u64, 4);
    db::chunks::insert_chunks(&job.pool, job.id, &planned).await?;
    let disk = crate::disk::DiskWriter::create(&job.save_path, Some(SIZE as u64)).await?;
    disk.write_at(0, &data[..100_000]).await?;
    disk.sync().await?;
    let first = db::chunks::get_chunks(&job.pool, job.id).await?.remove(0);
    db::chunks::update_chunk_progress(
        &job.pool,
        first.id,
        100_000,
        ferric_common::ChunkStatus::Downloading,
    )
    .await?;

    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let ranges = server.faults.ranges.lock().await.clone();
    assert!(ranges.iter().any(|r| r.0 == 100_000 - 64), "saw {ranges:?}");
    assert!(
        !ranges.iter().any(|r| r.0 == 0),
        "good resume data must not be re-downloaded, saw {ranges:?}"
    );
    Ok(())
}

#[tokio::test]
async fn job_probes_with_a_plain_get_and_never_sends_head() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            head_status: 405,
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    // IDM never sends HEAD: a server that refuses HEAD is simply never asked.
    assert_eq!(server.faults.heads.load(Ordering::SeqCst), 0);
    assert_eq!(server.faults.probes.load(Ordering::SeqCst), 1);
    let ranges = server.faults.ranges.lock().await.clone();
    assert!(
        ranges.len() > 2,
        "download should still be segmented, saw {ranges:?}"
    );
    // Connection 0 is the planning GET itself, so no ranged request starts at byte 0.
    assert!(
        ranges.iter().all(|r| r.0 > 0),
        "piece 0 must reuse the first GET: {ranges:?}"
    );
    Ok(())
}

#[tokio::test]
async fn idle_worker_steals_from_a_slow_in_flight_piece() -> Result<()> {
    // One piece crawls at ~100 KB/s. Without stealing the job needs ~10 s for
    // that 1 MiB piece; with IDM-style stealing the other workers take its
    // upper halves and the job ends once only a small remainder is left.
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            slow: AtomicU32::new(1),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let started = std::time::Instant::now();
    let (bytes, _keep) = run_to_end(job, 40).await?;
    let took = started.elapsed();
    assert!(bytes == *data, "downloaded bytes differ from source");
    assert!(
        took < std::time::Duration::from_secs(7),
        "slow piece was not helped (took {took:?})"
    );
    // The slow piece started at some multiple of the piece size; a stolen
    // second half starts strictly inside a planned piece.
    let planned = chunk::plan_pieces(SIZE as u64, 8);
    let ranges = server.faults.ranges.lock().await.clone();
    let inside = ranges.iter().any(|r| {
        planned
            .iter()
            .any(|p| r.0 as u64 > p.byte_start && (r.0 as u64) <= p.byte_end)
    });
    assert!(
        inside,
        "expected a range starting inside a planned piece, saw {ranges:?}"
    );
    Ok(())
}

#[tokio::test]
async fn connections_ramp_four_at_once_then_up_to_eight() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            rate: 1_000_000, // 1 MB/s per connection keeps the job running for seconds
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 60).await?;
    assert_eq!(bytes, *data);
    let starts = server.faults.starts.lock().await.clone();
    let t0 = starts.iter().min().copied().expect("requests were made");
    // The 1-byte probe is not a ranged piece request here (HEAD works), so every
    // entry is a piece request.
    let early = starts
        .iter()
        .filter(|t| t.duration_since(t0) < std::time::Duration::from_millis(500))
        .count();
    assert_eq!(
        early,
        4,
        "first burst must be 4 connections, starts: {}",
        starts.len()
    );
    assert!(
        server.faults.peak.load(Ordering::SeqCst) >= 6,
        "ramp should reach 6+ concurrent connections, peak {}",
        server.faults.peak.load(Ordering::SeqCst)
    );
    Ok(())
}

fn url_with_credentials(url: &str) -> String {
    url.replacen("http://", "http://user:pass@", 1)
}

#[tokio::test]
async fn basic_credentials_in_url_apply_to_every_range_request() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            auth: Some(("user".into(), "pass".into(), false)),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&url_with_credentials(&server.url)).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let ranges = server.faults.ranges.lock().await.clone();
    assert!(
        ranges.len() > 2,
        "download must stay segmented, saw {ranges:?}"
    );
    // Pre-emptive Basic: no 401 round trips (IDM sends it on the first request).
    assert_eq!(server.faults.challenges.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn digest_auth_uses_qop_and_unique_nc_on_every_range_request() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            auth: Some(("user".into(), "pass".into(), true)),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&url_with_credentials(&server.url)).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let ranges = server.faults.ranges.lock().await.clone();
    assert!(
        ranges.len() > 2,
        "download must stay segmented, saw {ranges:?}"
    );
    let ncs = server.faults.accepted_nc.lock().await.clone();
    assert!(
        ncs.len() >= ranges.len(),
        "every request needs its own nc: {ncs:?}"
    );
    // One challenge is expected (the first request), not one per connection.
    assert!(
        server.faults.challenges.load(Ordering::SeqCst) <= 2,
        "too many 401 round trips: {}",
        server.faults.challenges.load(Ordering::SeqCst)
    );
    Ok(())
}

#[tokio::test]
async fn first_request_5xx_is_retried_once_immediately() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            head_fail: AtomicU32::new(1),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    assert_eq!(server.faults.probes.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn two_failed_first_requests_fail_the_job_without_more_retries() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            head_fail: AtomicU32::new(5),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let result = run_to_end(job, 40).await;
    assert!(
        result.is_err(),
        "job must fail when the first request keeps failing"
    );
    // One try + one immediate retry, nothing more at this layer.
    assert_eq!(server.faults.probes.load(Ordering::SeqCst), 2);
    Ok(())
}

#[tokio::test]
async fn server_ignoring_range_finishes_as_single_stream() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            ignore_range: true,
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    Ok(())
}

// ───────────────────────────── proxy conformance ─────────────────────────────

#[derive(Default)]
struct ProxyLog {
    /// Request lines the proxy saw (first line of each request).
    lines: Mutex<Vec<String>>,
    /// Number of 407 answers sent.
    denied: AtomicU32,
    /// SOCKS targets (host, port) requested.
    targets: Mutex<Vec<(String, u16)>>,
}

async fn read_head(stream: &mut tokio::net::TcpStream) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0_u8; 1];
    while !buf.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte).await.ok()? == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.len() > 16_384 {
            return None;
        }
    }
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// HTTP proxy: absolute-URI requests are forwarded to `origin`. When
/// `require_auth` is set, requests without that exact `Proxy-Authorization`
/// get a 407.
async fn spawn_http_proxy(
    origin: std::net::SocketAddr,
    require_auth: Option<String>,
) -> Result<(std::net::SocketAddr, Arc<ProxyLog>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log = Arc::new(ProxyLog::default());
    let task_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let log = task_log.clone();
            let require_auth = require_auth.clone();
            tokio::spawn(async move {
                let Some(head) = read_head(&mut client).await else {
                    return;
                };
                let first = head.lines().next().unwrap_or("").to_string();
                log.lines.lock().await.push(first.clone());
                if let Some(want) = &require_auth {
                    if header_value(&head, "proxy-authorization") != Some(want.as_str()) {
                        log.denied.fetch_add(1, Ordering::SeqCst);
                        let _ = client
                            .write_all(b"HTTP/1.1 407 Proxy Authentication Required\r\nProxy-Authenticate: Basic realm=\"px\"\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                            .await;
                        return;
                    }
                }
                let mut parts = first.split_whitespace();
                let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
                    return;
                };
                if method == "CONNECT" {
                    // Open the tunnel and hang up: the TLS handshake then fails, which
                    // is enough to observe that HTTPS goes through CONNECT.
                    let _ = client
                        .write_all(
                            b"HTTP/1.1 200 Connection established

",
                        )
                        .await;
                    return;
                }
                let path = target
                    .strip_prefix("http://")
                    .and_then(|rest| rest.find('/').map(|i| &rest[i..]))
                    .unwrap_or(target);
                let mut forward = format!("{method} {path} HTTP/1.1\r\n");
                for line in head.lines().skip(1).filter(|l| !l.is_empty()) {
                    if !line.to_ascii_lowercase().starts_with("proxy-") {
                        forward.push_str(line);
                        forward.push_str("\r\n");
                    }
                }
                forward.push_str("\r\n");
                let Ok(mut upstream) = tokio::net::TcpStream::connect(origin).await else {
                    return;
                };
                let _ = upstream.write_all(forward.as_bytes()).await;
                let _ = tokio::io::copy(&mut upstream, &mut client).await;
            });
        }
    });
    Ok((addr, log))
}

/// SOCKS5 proxy (no-auth, or RFC 1929 user/pass when `creds` is set) that
/// connects every request to `origin`, recording the target it was asked for.
async fn spawn_socks5(
    origin: std::net::SocketAddr,
    creds: Option<(String, String)>,
) -> Result<(std::net::SocketAddr, Arc<ProxyLog>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log = Arc::new(ProxyLog::default());
    let task_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let log = task_log.clone();
            let creds = creds.clone();
            tokio::spawn(async move {
                let mut hello = [0_u8; 2];
                if client.read_exact(&mut hello).await.is_err() {
                    return;
                }
                let mut methods = vec![0_u8; hello[1] as usize];
                if client.read_exact(&mut methods).await.is_err() {
                    return;
                }
                if let Some((user, pass)) = &creds {
                    if !methods.contains(&2) {
                        let _ = client.write_all(&[5, 0xff]).await;
                        return;
                    }
                    let _ = client.write_all(&[5, 2]).await;
                    let mut ver = [0_u8; 2];
                    if client.read_exact(&mut ver).await.is_err() {
                        return;
                    }
                    let mut u = vec![0_u8; ver[1] as usize];
                    let _ = client.read_exact(&mut u).await;
                    let mut plen = [0_u8; 1];
                    let _ = client.read_exact(&mut plen).await;
                    let mut p = vec![0_u8; plen[0] as usize];
                    let _ = client.read_exact(&mut p).await;
                    if u != user.as_bytes() || p != pass.as_bytes() {
                        let _ = client.write_all(&[1, 1]).await;
                        return;
                    }
                    let _ = client.write_all(&[1, 0]).await;
                } else {
                    let _ = client.write_all(&[5, 0]).await;
                }
                let mut req = [0_u8; 4];
                if client.read_exact(&mut req).await.is_err() {
                    return;
                }
                let host = match req[3] {
                    3 => {
                        let mut len = [0_u8; 1];
                        let _ = client.read_exact(&mut len).await;
                        let mut name = vec![0_u8; len[0] as usize];
                        let _ = client.read_exact(&mut name).await;
                        String::from_utf8_lossy(&name).to_string()
                    }
                    1 => {
                        let mut ip = [0_u8; 4];
                        let _ = client.read_exact(&mut ip).await;
                        std::net::Ipv4Addr::from(ip).to_string()
                    }
                    _ => return,
                };
                let mut port = [0_u8; 2];
                let _ = client.read_exact(&mut port).await;
                log.targets
                    .lock()
                    .await
                    .push((host, u16::from_be_bytes(port)));
                let Ok(mut upstream) = tokio::net::TcpStream::connect(origin).await else {
                    return;
                };
                let _ = client.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).await;
                let (mut cr, mut cw) = client.split();
                let (mut ur, mut uw) = upstream.split();
                let _ = tokio::join!(
                    tokio::io::copy(&mut cr, &mut uw),
                    tokio::io::copy(&mut ur, &mut cw)
                );
            });
        }
    });
    Ok((addr, log))
}

fn origin_addr(url: &str) -> std::net::SocketAddr {
    let parsed = url::Url::parse(url).expect("test url");
    format!("127.0.0.1:{}", parsed.port().expect("port"))
        .parse()
        .expect("addr")
}

/// Same server URL but with an unresolvable host name: only a proxy can reach it.
fn unreachable_name(url: &str) -> String {
    url.replacen("127.0.0.1", "origin.invalid", 1)
}

async fn proxied_job(url: &str, proxy_url: &str) -> Result<Job> {
    let mut job = make_job(url).await?;
    job.ctx.proxy_client = Some(crate::manager::build_reqwest_client(Some(proxy_url))?);
    Ok(job)
}

#[tokio::test]
async fn http_proxy_carries_planning_and_every_range_request() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let (proxy, log) = spawn_http_proxy(origin_addr(&server.url), None).await?;
    let job = proxied_job(&unreachable_name(&server.url), &format!("http://{proxy}")).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let lines = log.lines.lock().await.clone();
    assert!(
        lines.iter().all(|l| l.contains("http://origin.invalid:")),
        "proxy requests must use absolute-URI form: {lines:?}"
    );
    assert!(
        lines.iter().all(|l| !l.starts_with("HEAD ")),
        "IDM never sends HEAD: {lines:?}"
    );
    assert!(
        lines.iter().filter(|l| l.starts_with("GET ")).count() > 2,
        "planning GET and ranged pieces must all go through the proxy: {lines:?}"
    );
    Ok(())
}

#[tokio::test]
async fn proxy_407_is_answered_with_configured_credentials() -> Result<()> {
    use base64::Engine as _;
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let want = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode("pu:pp")
    );
    let (proxy, log) = spawn_http_proxy(origin_addr(&server.url), Some(want)).await?;
    let job = proxied_job(
        &unreachable_name(&server.url),
        &format!("http://pu:pp@{proxy}"),
    )
    .await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    // IDM sends Proxy-Authorization up front on every connection; so do we.
    assert_eq!(
        log.denied.load(Ordering::SeqCst),
        0,
        "no 407 round trips expected"
    );
    Ok(())
}

#[tokio::test]
async fn socks5_resolves_the_host_on_the_proxy() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let (proxy, log) = spawn_socks5(origin_addr(&server.url), None).await?;
    let job = proxied_job(
        &unreachable_name(&server.url),
        &format!("socks5h://{proxy}"),
    )
    .await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let targets = log.targets.lock().await.clone();
    assert!(
        !targets.is_empty() && targets.iter().all(|t| t.0 == "origin.invalid"),
        "SOCKS5 must receive the domain name (ATYP 3), saw {targets:?}"
    );
    Ok(())
}

#[tokio::test]
async fn socks5_username_password_authentication() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let (proxy, log) =
        spawn_socks5(origin_addr(&server.url), Some(("su".into(), "sp".into()))).await?;
    let job = proxied_job(
        &unreachable_name(&server.url),
        &format!("socks5h://su:sp@{proxy}"),
    )
    .await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    assert!(!log.targets.lock().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn https_through_http_proxy_uses_connect_with_host_and_port() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let (proxy, log) = spawn_http_proxy(origin_addr(&server.url), None).await?;
    let job = proxied_job(
        "https://origin.invalid/payload.bin",
        &format!("http://{proxy}"),
    )
    .await?;
    let result = run_to_end(job, 40).await;
    assert!(
        result.is_err(),
        "the stub tunnel has no TLS server behind it"
    );
    let lines = log.lines.lock().await.clone();
    assert!(
        lines
            .iter()
            .any(|l| l.starts_with("CONNECT origin.invalid:443 ")),
        "HTTPS must be tunnelled with CONNECT host:443, saw {lines:?}"
    );
    assert!(lines.iter().all(|l| l.starts_with("CONNECT ")), "{lines:?}");
    Ok(())
}

/// Prints the request timeline for a 20 MB file served at 300 KB/s per
/// connection (a fixed file and rate, for comparing runs). Run with
/// `cargo test -p ferricd trace_20mb -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "prints a timeline for manual comparison between runs"]
async fn trace_20mb_reference_run() -> Result<()> {
    let size = 20 * 1024 * 1024;
    let data: Arc<Vec<u8>> = Arc::new(
        (0..size)
            .map(|i| ((i * 31 + (i >> 8) * 17) & 0xff) as u8)
            .collect(),
    );
    let server = spawn_server(
        data.clone(),
        Faults {
            rate: 300_000,
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let started = std::time::Instant::now();
    let (bytes, _keep) = run_to_end(job, 120).await?;
    assert_eq!(bytes, *data);
    let starts = server.faults.starts.lock().await.clone();
    let ranges = server.faults.ranges.lock().await.clone();
    let t0 = starts.iter().min().copied().unwrap();
    println!(
        "total time {:?}, requests {}, peak concurrency {}",
        started.elapsed(),
        starts.len(),
        server.faults.peak.load(Ordering::SeqCst)
    );
    let mut rows: Vec<_> = starts.iter().zip(ranges.iter()).collect();
    rows.sort_by_key(|(t, _)| **t);
    for (t, r) in rows {
        println!(
            "  +{:>6.0} ms  bytes={}-{}",
            t.duration_since(t0).as_secs_f64() * 1000.0,
            r.0,
            r.1
        );
    }
    Ok(())
}

#[tokio::test]
async fn speed_limit_is_shared_by_all_connections_of_a_download() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let mut job = make_job(&server.url).await?;
    // 2 MB/s for the whole download: 6 MiB cannot finish in under ~2 s however
    // many connections are open (a per-connection limit would allow 8x faster).
    job.ctx.bandwidth = Arc::new(BandwidthLimiter::new(2_000_000));
    let started = std::time::Instant::now();
    let (bytes, _keep) = run_to_end(job, 60).await?;
    let took = started.elapsed();
    assert_eq!(bytes, *data);
    assert!(
        took >= std::time::Duration::from_millis(1800),
        "limit not shared across connections (took {took:?})"
    );
    // Several pieces/connections were in play (the server cannot see concurrency
    // here because it writes into socket buffers faster than the client reads).
    assert!(server.faults.ranges.lock().await.len() >= 4);
    Ok(())
}

// ───────────────────────────── TLS (certificate checking, CONNECT) ─────────────────────────────

struct TestPki {
    ca_pem: String,
    acceptor: tokio_rustls::TlsAcceptor,
}

/// A throw-away CA plus a server certificate for `name`.
fn test_pki(name: &str) -> Result<TestPki> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
    let ca_key = rcgen::KeyPair::generate()?;
    let mut ca_params = rcgen::CertificateParams::new(Vec::<String>::new())?;
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "phase1 test CA");
    let ca_cert = ca_params.self_signed(&ca_key)?;
    let server_key = rcgen::KeyPair::generate()?;
    let server_cert = rcgen::CertificateParams::new(vec![name.to_string()])?.signed_by(
        &server_key,
        &ca_cert,
        &ca_key,
    )?;
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ServerConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()?
        .with_no_client_auth()
        .with_single_cert(
            vec![CertificateDer::from(server_cert.der().to_vec())],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(server_key.serialize_der())),
        )?;
    Ok(TestPki {
        ca_pem: ca_cert.pem(),
        acceptor: tokio_rustls::TlsAcceptor::from(Arc::new(config)),
    })
}

/// CONNECT proxy that really tunnels to `target`.
async fn spawn_connect_tunnel(
    target: std::net::SocketAddr,
) -> Result<(std::net::SocketAddr, Arc<ProxyLog>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log = Arc::new(ProxyLog::default());
    let task_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let log = task_log.clone();
            tokio::spawn(async move {
                let Some(head) = read_head(&mut client).await else {
                    return;
                };
                let first = head.lines().next().unwrap_or("").to_string();
                log.lines.lock().await.push(first.clone());
                if !first.starts_with("CONNECT ") {
                    return;
                }
                let Ok(mut upstream) = tokio::net::TcpStream::connect(target).await else {
                    return;
                };
                let _ = client
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await;
                let (mut cr, mut cw) = client.split();
                let (mut ur, mut uw) = upstream.split();
                let _ = tokio::join!(
                    tokio::io::copy(&mut cr, &mut uw),
                    tokio::io::copy(&mut ur, &mut cw)
                );
            });
        }
    });
    Ok((addr, log))
}

#[tokio::test]
async fn https_download_through_connect_tunnel_with_trusted_ca() -> Result<()> {
    let data = payload();
    let pki = test_pki("origin.invalid")?;
    let server = spawn_tls_server(data.clone(), Faults::default(), pki.acceptor.clone()).await?;
    let (proxy, log) = spawn_connect_tunnel(origin_addr(&server.url)).await?;
    let mut job = make_job(&unreachable_name(&server.url)).await?;
    let client = reqwest::Client::builder()
        .add_root_certificate(reqwest::Certificate::from_pem(pki.ca_pem.as_bytes())?)
        .proxy(reqwest::Proxy::all(format!("http://{proxy}"))?)
        .build()?;
    job.ctx.proxy_client = Some(client);
    let (bytes, _keep) = run_to_end(job, 60).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    let lines = log.lines.lock().await.clone();
    let port = origin_addr(&server.url).port();
    assert!(
        lines
            .iter()
            .all(|l| l.starts_with(&format!("CONNECT origin.invalid:{port} "))),
        "every connection must be a CONNECT to host:port, saw {lines:?}"
    );
    assert!(
        lines.len() >= 3,
        "planning and several ranged connections expected: {lines:?}"
    );
    Ok(())
}

#[tokio::test]
async fn untrusted_server_certificate_is_rejected() -> Result<()> {
    let data = payload();
    let pki = test_pki("localhost")?;
    let server = spawn_tls_server(data.clone(), Faults::default(), pki.acceptor).await?;
    // `127.0.0.1` is not in the certificate and the CA is not trusted: IDM would
    // download anyway (no certificate checks); we must refuse.
    let job = make_job(&server.url).await?;
    let result = run_to_end(job, 30).await;
    assert!(
        result.is_err(),
        "download over an untrusted certificate must fail"
    );
    assert!(
        server.faults.ranges.lock().await.is_empty(),
        "no data may be fetched"
    );
    Ok(())
}

/// `SOCKS4a` proxy: records the host name it was asked to resolve.
async fn spawn_socks4a(
    origin: std::net::SocketAddr,
    want_user: &'static str,
) -> Result<(std::net::SocketAddr, Arc<ProxyLog>)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log = Arc::new(ProxyLog::default());
    let task_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut client, _)) = listener.accept().await else {
                break;
            };
            let log = task_log.clone();
            tokio::spawn(async move {
                let mut head = [0_u8; 8];
                if client.read_exact(&mut head).await.is_err() || head[0] != 4 || head[1] != 1 {
                    return;
                }
                let port = u16::from_be_bytes([head[2], head[3]]);
                let read_cstr = async |c: &mut tokio::net::TcpStream| {
                    let mut out = Vec::new();
                    let mut b = [0_u8; 1];
                    while c.read_exact(&mut b).await.is_ok() && b[0] != 0 {
                        out.push(b[0]);
                    }
                    String::from_utf8_lossy(&out).to_string()
                };
                let user = read_cstr(&mut client).await;
                let host = if head[4..7] == [0, 0, 0] && head[7] != 0 {
                    read_cstr(&mut client).await // SOCKS4a: resolve this name
                } else {
                    std::net::Ipv4Addr::new(head[4], head[5], head[6], head[7]).to_string()
                };
                log.targets
                    .lock()
                    .await
                    .push((format!("{user}@{host}"), port));
                let Ok(mut upstream) = tokio::net::TcpStream::connect(origin).await else {
                    return;
                };
                let status = if user == want_user { 0x5a } else { 0x5b };
                let _ = client.write_all(&[0, status, 0, 0, 0, 0, 0, 0]).await;
                if status == 0x5a {
                    let _ = tokio::io::copy_bidirectional(&mut client, &mut upstream).await;
                }
            });
        }
    });
    Ok((addr, log))
}

#[tokio::test]
async fn socks4a_proxy_gets_the_host_name_and_user_id() -> Result<()> {
    let data = payload();
    let server = spawn_server(data.clone(), Faults::default()).await?;
    let (proxy, log) = spawn_socks4a(origin_addr(&server.url), "bob").await?;
    let job = proxied_job(
        &unreachable_name(&server.url),
        &format!("socks4a://bob@{proxy}"),
    )
    .await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert_eq!(bytes, *data);
    let targets = log.targets.lock().await.clone();
    assert!(
        !targets.is_empty() && targets.iter().all(|t| t.0 == "bob@origin.invalid"),
        "SOCKS4a must carry user id and host name, saw {targets:?}"
    );
    Ok(())
}

// ───────────────────────────── FTP ─────────────────────────────

#[derive(Default)]
struct FtpLog {
    logins: Mutex<Vec<(String, String)>>,
    /// (path, REST offset) of every RETR.
    retrs: Mutex<Vec<(String, u64)>>,
    controls: AtomicU32,
}

/// Minimal passive-mode FTP server (USER/PASS/TYPE/SIZE/PASV/REST/RETR/QUIT).
/// `rate` = bytes per second per data connection (0 = unthrottled).
async fn spawn_ftp(data: Arc<Vec<u8>>, rate: u64) -> Result<(std::net::SocketAddr, Arc<FtpLog>)> {
    use tokio::io::{AsyncBufReadExt, BufReader};
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let log = Arc::new(FtpLog::default());
    let task_log = log.clone();
    tokio::spawn(async move {
        loop {
            let Ok((control, _)) = listener.accept().await else {
                break;
            };
            let data = data.clone();
            let log = task_log.clone();
            log.controls.fetch_add(1, Ordering::SeqCst);
            tokio::spawn(async move {
                let (read, mut write) = tokio::io::split(control);
                let mut lines = BufReader::new(read).lines();
                let _ = write.write_all(b"220 test ftp\r\n").await;
                let mut user = String::new();
                let mut rest = 0_u64;
                let mut pasv: Option<TcpListener> = None;
                while let Ok(Some(line)) = lines.next_line().await {
                    let (cmd, arg) = line.split_once(' ').unwrap_or((line.as_str(), ""));
                    match cmd.to_ascii_uppercase().as_str() {
                        "USER" => {
                            user = arg.to_string();
                            let _ = write.write_all(b"331 password please\r\n").await;
                        }
                        "PASS" => {
                            log.logins
                                .lock()
                                .await
                                .push((user.clone(), arg.to_string()));
                            let _ = write.write_all(b"230 logged in\r\n").await;
                        }
                        "TYPE" => {
                            let _ = write.write_all(b"200 ok\r\n").await;
                        }
                        "SIZE" => {
                            let _ = write
                                .write_all(format!("213 {}\r\n", data.len()).as_bytes())
                                .await;
                        }
                        "REST" => {
                            rest = arg.parse().unwrap_or(0);
                            let _ = write.write_all(b"350 restarting\r\n").await;
                        }
                        "PASV" => {
                            let l = TcpListener::bind("127.0.0.1:0").await.expect("pasv");
                            let p = l.local_addr().expect("addr").port();
                            pasv = Some(l);
                            let _ = write
                                .write_all(
                                    format!(
                                        "227 Entering Passive Mode (127,0,0,1,{},{})\r\n",
                                        p >> 8,
                                        p & 255
                                    )
                                    .as_bytes(),
                                )
                                .await;
                        }
                        "RETR" => {
                            log.retrs.lock().await.push((arg.to_string(), rest));
                            let Some(l) = pasv.take() else {
                                let _ = write.write_all(b"425 no data connection\r\n").await;
                                continue;
                            };
                            let _ = write.write_all(b"150 opening\r\n").await;
                            let Ok((mut dc, _)) = l.accept().await else {
                                continue;
                            };
                            let body = &data[(rest as usize).min(data.len())..];
                            let (step, pause) = if rate > 0 {
                                (usize::try_from(rate / 50).unwrap_or(1024).max(1024), 20)
                            } else {
                                (64 * 1024, 0)
                            };
                            let mut ok = true;
                            for piece in body.chunks(step) {
                                if dc.write_all(piece).await.is_err() {
                                    ok = false;
                                    break;
                                }
                                if pause > 0 {
                                    tokio::time::sleep(std::time::Duration::from_millis(pause))
                                        .await;
                                }
                            }
                            let _ = dc.shutdown().await;
                            drop(dc);
                            rest = 0;
                            let reply: &[u8] = if ok {
                                b"226 transfer complete\r\n"
                            } else {
                                b"426 transfer aborted\r\n"
                            };
                            let _ = write.write_all(reply).await;
                        }
                        "QUIT" => {
                            let _ = write.write_all(b"221 bye\r\n").await;
                            break;
                        }
                        _ => {
                            let _ = write.write_all(b"502 not implemented\r\n").await;
                        }
                    }
                }
            });
        }
    });
    Ok((addr, log))
}

#[tokio::test]
async fn ftp_is_segmented_and_decodes_credentials_and_path() -> Result<()> {
    let data = payload();
    let (addr, log) = spawn_ftp(data.clone(), 0).await?;
    // `%40` in the user name / password and `%20` in the path: IDM mis-parsed these.
    let url = format!("ftp://us%40er:p%40ss@{addr}/dir%20x/file.bin");
    let job = make_job(&url).await?;
    let (bytes, _keep) = run_to_end(job, 60).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    let logins = log.logins.lock().await.clone();
    assert_ne!(
        logins,
        [] as [(std::string::String, std::string::String); 0]
    );
    assert!(
        logins.iter().all(|l| l.0 == "us@er" && l.1 == "p@ss"),
        "credentials must be percent-decoded, saw {logins:?}"
    );
    let retrs = log.retrs.lock().await.clone();
    assert!(
        retrs.iter().all(|r| r.0 == "/dir x/file.bin"),
        "path must be percent-decoded, saw {retrs:?}"
    );
    assert!(
        retrs.len() >= 2,
        "several control connections expected: {retrs:?}"
    );
    Ok(())
}

#[tokio::test]
async fn ftp_pieces_are_halved_while_downloading() -> Result<()> {
    let data = payload();
    // 1 MB/s per data connection keeps pieces in flight long enough to be split.
    let (addr, log) = spawn_ftp(data.clone(), 1_000_000).await?;
    let job = make_job(&format!("ftp://{addr}/file.bin")).await?;
    let (bytes, _keep) = run_to_end(job, 90).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    let mut offsets: Vec<u64> = log.retrs.lock().await.iter().map(|r| r.1).collect();
    offsets.sort_unstable();
    offsets.dedup();
    assert!(
        offsets.len() >= 5,
        "4 initial pieces plus at least one halving expected, REST offsets: {offsets:?}"
    );
    Ok(())
}

/// Keep-alive NTLM server: Type 3 is accepted only when it follows a Type 1 on
/// the *same* connection. Returns the URL and the number of accepted Type 3s.
async fn spawn_ntlm_server(data: Arc<Vec<u8>>) -> Result<(String, Arc<AtomicU32>)> {
    use base64::Engine as _;
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let accepted = Arc::new(AtomicU32::new(0));
    let accepted_srv = accepted.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let data = data.clone();
            let accepted = accepted_srv.clone();
            tokio::spawn(async move {
                let b64 = base64::engine::general_purpose::STANDARD;
                let mut saw_type1 = false;
                let mut buf = vec![0_u8; 8192];
                loop {
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    if n == 0 {
                        return;
                    }
                    let req = String::from_utf8_lossy(&buf[..n]).to_string();
                    let ntlm = header_value(&req, "authorization")
                        .and_then(|a| a.strip_prefix("NTLM "))
                        .and_then(|m| b64.decode(m.trim()).ok());
                    let kind = ntlm.as_ref().filter(|m| m.len() > 12).map(|m| m[8]);
                    let resp: Vec<u8> = match kind {
                        Some(3) if saw_type1 => {
                            accepted.fetch_add(1, Ordering::SeqCst);
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\n\r\n",
                                data.len()
                            );
                            let mut out = head.into_bytes();
                            if !req.starts_with("HEAD ") {
                                out.extend_from_slice(&data);
                            }
                            out
                        }
                        Some(1) => {
                            saw_type1 = true;
                            let mut t2 = b"NTLMSSP\0".to_vec();
                            t2.extend_from_slice(&2u32.to_le_bytes());
                            t2.extend_from_slice(&[0, 0, 0, 0]);
                            t2.extend_from_slice(&48u32.to_le_bytes());
                            t2.extend_from_slice(&0xa289_8205u32.to_le_bytes());
                            t2.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
                            t2.extend_from_slice(&[0u8; 8]);
                            t2.extend_from_slice(&[0, 0, 0, 0]);
                            t2.extend_from_slice(&48u32.to_le_bytes());
                            format!(
                                "HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: NTLM {}\r\nContent-Length: 0\r\n\r\n",
                                b64.encode(t2)
                            )
                            .into_bytes()
                        }
                        _ => b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: NTLM\r\nContent-Length: 0\r\n\r\n"
                            .to_vec(),
                    };
                    if stream.write_all(&resp).await.is_err() {
                        return;
                    }
                }
            });
        }
    });
    Ok((format!("http://{addr}/payload.bin"), accepted))
}

#[tokio::test]
async fn ntlm_download_authenticates_on_one_connection_and_runs_single_stream() -> Result<()> {
    let data = payload();
    let (url, accepted) = spawn_ntlm_server(data.clone()).await?;
    let job = make_job(&url_with_credentials(&url)).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert!(bytes == *data, "downloaded bytes differ from source");
    assert!(
        accepted.load(Ordering::SeqCst) >= 1,
        "server never accepted a Type 3 message"
    );
    Ok(())
}

#[tokio::test]
async fn single_stream_download_reuses_the_planning_get() -> Result<()> {
    // A server without ranges, whose URL could be single-use: it must be hit exactly once.
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            no_ranges_header: true,
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert!(bytes == *data);
    assert_eq!(server.faults.probes.load(Ordering::SeqCst), 1);
    assert!(server.faults.ranges.lock().await.is_empty());
    Ok(())
}

#[tokio::test]
async fn ranged_requests_carry_the_etag_as_if_range() -> Result<()> {
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            etag: Mutex::new(Some("\"v1\"".to_string())),
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert!(bytes == *data);
    let sent = server.faults.if_ranges.lock().await.clone();
    assert!(
        sent.len() >= 3,
        "every ranged request needs If-Range, saw {sent:?}"
    );
    assert!(sent.iter().all(|v| v == "\"v1\""), "{sent:?}");
    Ok(())
}

#[tokio::test]
async fn file_replaced_mid_download_falls_back_to_one_consistent_stream() -> Result<()> {
    // The ETag changes right after planning, so every conditional range gets a full 200.
    let data = payload();
    let server = spawn_server(
        data.clone(),
        Faults {
            etag: Mutex::new(Some("\"v1\"".to_string())),
            flip_etag_after_probe: true,
            ..Faults::default()
        },
    )
    .await?;
    let job = make_job(&server.url).await?;
    let (bytes, _keep) = run_to_end(job, 40).await?;
    assert!(
        bytes == *data,
        "output must come from one coherent version of the file"
    );
    Ok(())
}
