use crate::chunk::{self, ChunkArgs, ChunkPlan, ResponseMetadata};
use crate::db;
use crate::diagnostics;
use crate::disk::DiskWriter;
use crate::retry;
use crate::security;
use crate::speed::SpeedTracker;
use crate::throttle::BandwidthLimiter;
use anyhow::{Context, Result};
use ferric_common::messages::ProgressEvent;
use ferric_common::{ChunkRecord, DownloadRecord, DownloadStatus, FerricError};
use reqwest::header::{HeaderName, HeaderValue, CONTENT_DISPOSITION, CONTENT_TYPE, COOKIE};
use sqlx::SqlitePool;
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinSet;
use uuid::Uuid;

#[derive(Clone)]
pub struct JobHandle {
    pub id: Uuid,
    /// Effective queue this job occupies a slot in (used for concurrency limits).
    pub queue_id: Uuid,
    cancel_tx: watch::Sender<bool>,
    /// The job's own limiter: the manager changes its rate when a limit is edited while it runs.
    pub bandwidth: Arc<BandwidthLimiter>,
}

impl JobHandle {
    pub fn new(
        id: Uuid,
        queue_id: Uuid,
        cancel_tx: watch::Sender<bool>,
        bandwidth: Arc<BandwidthLimiter>,
    ) -> Self {
        Self {
            id,
            queue_id,
            cancel_tx,
            bandwidth,
        }
    }

    pub fn cancel(&self) {
        let _ = self.cancel_tx.send(true);
    }
}

pub struct JobContext {
    pub pool: SqlitePool,
    pub record: DownloadRecord,
    pub client: reqwest::Client,
    pub proxy_client: Option<reqwest::Client>,
    pub progress_tx: broadcast::Sender<ProgressEvent>,
    pub bandwidth: Arc<BandwidthLimiter>,
    pub cancel_rx: watch::Receiver<bool>,
    pub cookies: Option<String>,
    pub extra_headers: Option<HashMap<String, String>>,
    pub method: String,
    pub post_body: Option<Vec<u8>>,
}

pub async fn run(mut ctx: JobContext) -> Result<()> {
    // Planning, single-stream, HLS/DASH and merge requests all use `ctx.client`;
    // when a proxy is configured they must go through it too (only the ranged
    // chunk requests used `proxy_client` before, so the rest leaked direct).
    if let Some(proxied) = &ctx.proxy_client {
        ctx.client = proxied.clone();
    }
    diagnostics::event(
        "job_start",
        serde_json::json!({
            "id": ctx.record.id,
            "url": security::redact_url(&ctx.record.url),
            "filename": ctx.record.filename,
            "save_path": ctx.record.save_path,
            "method": ctx.method,
        }),
    )
    .await;

    if *ctx.cancel_rx.borrow() {
        return Err(FerricError::Cancelled.into());
    }

    // IDM's Sites Logins: credentials for this server, used for the requests only (never saved).
    if let Some(with_login) = crate::site_logins::apply(&ctx.pool, &ctx.record.url).await {
        ctx.record.url = with_login;
    }

    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Connecting, None)
        .await?;
    if is_hls(&ctx.record.url, ctx.record.mime_type.as_deref()) {
        return run_streaming_manifest(ctx, ManifestKind::Hls).await;
    }
    if is_dash(&ctx.record.url, ctx.record.mime_type.as_deref()) {
        return run_streaming_manifest(ctx, ManifestKind::Dash).await;
    }
    // Non-streaming merge: separate video + audio URLs from format selection.
    if ctx.record.audio_url.is_some()
        && !is_hls(&ctx.record.url, ctx.record.mime_type.as_deref())
        && !is_dash(&ctx.record.url, ctx.record.mime_type.as_deref())
    {
        return run_merge(ctx).await;
    }
    if ctx.method.eq_ignore_ascii_case("POST") {
        return run_single_stream(ctx, None).await;
    }

    if crate::webdav::is_webdav_url(&ctx.record.url) {
        // Explicit LAN protocol: its host may legitimately resolve to a private address.
        if let Some(host) = crate::dns::host_from_url(&ctx.record.url) {
            crate::dns::CachingResolver::global().allow_private(&host);
        }
    }
    let max_connections = effective_max_connections_for_url(&ctx.pool, &ctx.record.url).await?;
    let mut first_response: Option<reqwest::Response> = None;
    let plan = if is_ftp_family(&ctx.record.url) {
        crate::ftp::plan_ftp(&ctx.record.url, max_connections).await?
    } else if ctx.record.url.starts_with("sftp://") {
        crate::sftp::plan_sftp(&ctx.record.url, max_connections).await?
    } else if crate::webdav::is_webdav_url(&ctx.record.url) {
        crate::webdav::plan_webdav(
            &ctx.record.url,
            &ctx.client,
            ctx.cookies.as_deref(),
            ctx.extra_headers.as_ref(),
            max_connections,
        )
        .await?
    } else {
        match chunk::plan_first_get(
            &ctx.record.url,
            &ctx.client,
            ctx.cookies.as_deref(),
            ctx.extra_headers.as_ref(),
            max_connections,
        )
        .await
        {
            Ok((p, first)) => {
                first_response = first;
                p
            }
            Err(e) if is_rate_limited(&e) => {
                save_rate_limit_rule(&ctx.pool, &ctx.record.url).await?;
                return Err(e);
            }
            Err(e) => return Err(e),
        }
    };

    if *ctx.cancel_rx.borrow() {
        return Err(FerricError::Cancelled.into());
    }

    // A web page where a file was expected (a verification page, a login page, an expired link) is not a download.
    {
        let (mime, name, size) = match &plan {
            ChunkPlan::Segmented {
                file_size,
                metadata,
                ..
            } => (
                metadata.mime_type.as_deref(),
                metadata.filename.as_deref(),
                Some(*file_size),
            ),
            ChunkPlan::SingleStream { metadata, .. } => (
                metadata.mime_type.as_deref(),
                metadata.filename.as_deref(),
                first_response
                    .as_ref()
                    .and_then(reqwest::Response::content_length),
            ),
        };
        if looks_like_web_page_instead_of_file(mime, name.unwrap_or(&ctx.record.filename), size) {
            return Err(FerricError::WebPageInsteadOfFile.into());
        }
    }

    match plan {
        ChunkPlan::Segmented {
            file_size,
            chunks,
            concurrency,
            etag,
            last_modified,
            metadata,
        } => {
            diagnostics::event(
                "job_plan_segmented",
                serde_json::json!({
                    "id": ctx.record.id,
                    "file_size": file_size,
                    "chunk_count": chunks.len(),
                    "concurrency": concurrency,
                }),
            )
            .await;
            if let Some(ref chain) = metadata.redirect_chain {
                diagnostics::event(
                    "download_redirected",
                    serde_json::json!({ "id": ctx.record.id, "chain": chain }),
                )
                .await;
            }
            // Progress from an earlier run is only valid for the same bytes: a changed
            // size/ETag/Last-Modified means the remote file was replaced, start over.
            // IDM's "Ignore file modification time changes when resuming a download".
            let ignore_mtime =
                db::settings::get_flag(&ctx.pool, "ignore_mtime_changes", false).await;
            if let Some(why) = remote_file_change(
                &ctx.record,
                file_size,
                etag.as_deref(),
                if ignore_mtime {
                    None
                } else {
                    last_modified.as_deref()
                },
            ) {
                tracing::warn!(id = %ctx.record.id, file = %ctx.record.filename, "{why} Restarting the download from the beginning.");
                db::chunks::delete_all(&ctx.pool, ctx.record.id).await?;
            }
            db::downloads::set_file_info(
                &ctx.pool,
                ctx.record.id,
                file_size,
                etag.as_deref(),
                last_modified.as_deref(),
            )
            .await?;
            let validator = chunk::range_validator(etag.as_deref(), last_modified.as_deref());
            let challenge = metadata.www_authenticate.clone();
            let ctx = apply_response_metadata(ctx, &metadata).await?;
            run_segmented(
                ctx,
                file_size,
                chunks,
                concurrency,
                challenge,
                first_response,
                validator,
            )
            .await
        }
        ChunkPlan::SingleStream {
            etag,
            last_modified,
            metadata,
        } => {
            db::downloads::set_validators(
                &ctx.pool,
                ctx.record.id,
                etag.as_deref(),
                last_modified.as_deref(),
            )
            .await?;
            if let Some(ref chain) = metadata.redirect_chain {
                diagnostics::event(
                    "download_redirected",
                    serde_json::json!({ "id": ctx.record.id, "chain": chain }),
                )
                .await;
            }
            let ctx = apply_response_metadata(ctx, &metadata).await?;
            run_single_stream(ctx, first_response).await
        }
    }
}

/// Shared, cloneable context every piece worker needs (everything in
/// [`ChunkArgs`] except the per-piece `chunk`).
#[derive(Clone)]
struct PieceWorkerCtx {
    hyper_client: chunk::HyperClient,
    pool: SqlitePool,
    disk: DiskWriter,
    download_id: Uuid,
    /// Live multi-source pool (primary + mirrors).
    mirrors: Arc<chunk::MirrorSet>,
    bandwidth: Arc<BandwidthLimiter>,
    cancel_rx: watch::Receiver<bool>,
    cookies: Option<String>,
    extra_headers: Option<HashMap<String, String>>,
    proxy_client: Option<reqwest::Client>,
    /// Pieces re-queued after a failure, across all workers of this job.
    restarts: Arc<std::sync::atomic::AtomicU32>,
    /// Pieces being downloaded right now (candidates for work-stealing).
    live: Arc<std::sync::Mutex<HashMap<Uuid, Arc<chunk::LiveChunk>>>>,
    /// Serialises steals so two idle workers never split the same piece at once.
    steal_lock: Arc<Mutex<()>>,
    /// Smallest unfetched part of a piece that may still be split (see [`min_steal`]).
    min_steal: u64,
    auth: Option<Arc<crate::auth::AuthState>>,
    /// Planning-GET response waiting for piece 0 (connection 0).
    first: chunk::FirstResponse,
    /// `If-Range` validator for every ranged request of this run.
    validator: Option<String>,
    /// An extra connection opened by the adaptive controller: if the server refuses it the piece goes
    /// back to the queue and the job carries on with the connections it has.
    soft: bool,
    /// Set when a soft connection was refused or failed: the controller stops adding connections.
    extra_rejected: Arc<AtomicBool>,
}

/// A piece is only worth splitting when this much of it is still unfetched: at least the grain
/// (2 x 116,800), and on a big file more, because each new request waits for its first byte
/// (about a second on a busy host) and a 100 KB piece is gone sooner than that. A 95 GB download
/// ended in requests as small as 127 KB; the floor grows with the file, capped at 8 MiB.
fn min_steal(file_size: u64) -> u64 {
    (file_size / 2048).clamp(chunk::PIECE_GRAIN, 8 << 20)
}

impl PieceWorkerCtx {
    fn live_lock(&self) -> std::sync::MutexGuard<'_, HashMap<Uuid, Arc<chunk::LiveChunk>>> {
        self.live
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// `true` when some in-flight piece could be split by an extra worker.
    fn has_stealable(&self) -> bool {
        self.live_lock()
            .values()
            .any(|l| l.remaining() >= self.min_steal)
    }
}

/// IDM-style dynamic segmentation: take the upper half of the in-flight piece
/// with the most bytes left. The owner is told its new end through
/// [`chunk::LiveChunk`] and stops there; the DB row is split in one transaction.
async fn try_steal(wc: &PieceWorkerCtx) -> Option<ChunkRecord> {
    let _guard = wc.steal_lock.lock().await;
    let (victim_id, victim) = {
        let map = wc.live_lock();
        let (id, live) = map.iter().max_by_key(|(_, l)| l.remaining())?;
        if live.remaining() < wc.min_steal {
            return None;
        }
        (*id, live.clone())
    };
    let position = victim.byte_start + victim.done();
    let midpoint = position + victim.remaining() / 2;
    if midpoint <= position || midpoint > victim.end() {
        return None;
    }
    let mut row = db::chunks::get_chunks(&wc.pool, wc.download_id)
        .await
        .ok()?
        .into_iter()
        .find(|c| c.id == victim_id)?;
    row.byte_end = victim.end();
    match db::chunks::split_chunk(&wc.pool, &row, midpoint).await {
        Ok(second_half) => {
            victim.set_end(midpoint - 1);
            tracing::debug!(
                victim = %victim_id,
                midpoint,
                "work-stealing: split in-flight piece"
            );
            Some(second_half)
        }
        Err(e) => {
            tracing::warn!("work-stealing split failed: {e}");
            None
        }
    }
}

/// More than this many failed-and-requeued pieces means the server cannot
/// serve parallel ranges reliably; fall back to one stream (IDM: >20 restarts).
const MAX_PIECE_RESTARTS: u32 = 20;

#[derive(Debug)]
struct TooManyRestarts;

impl std::fmt::Display for TooManyRestarts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("too many piece restarts")
    }
}

impl std::error::Error for TooManyRestarts {}

/// A job-level failure worth trying again later (network trouble, 5xx, rate
/// limit); permanent answers (404, 403, invalid URL, ...) are not.
pub(crate) fn is_transient_failure(error: &anyhow::Error) -> bool {
    match error.downcast_ref::<FerricError>() {
        Some(FerricError::RateLimited) => true,
        Some(e) => e.is_retryable(),
        None => true,
    }
}

/// Errors that must end the job (or switch engine) instead of re-queueing the piece.
fn is_piece_terminal(error: &anyhow::Error) -> bool {
    if is_rate_limited(error) || is_range_rejected(error) {
        return true;
    }
    error
        .downcast_ref::<FerricError>()
        .is_some_and(|e| !e.is_retryable())
}

/// Outcome that requires abandoning the parallel piece queue and switching
/// engines, decided once an error surfaces from a worker.
enum SegmentFallback {
    /// Server returned 429 mid-flight; finish the rest sequentially under a
    /// learned rate-limit rule.
    Sequential,
    /// Server rejected a Range request; the whole download must be one stream.
    SingleStream,
}

/// Unique bytes of the file that are in by now: each piece counts what it has received, so bytes that
/// were fetched twice (a stolen piece's overlap, a retry) are not counted twice.
async fn live_progress(wc: &PieceWorkerCtx) -> Result<u64> {
    let live: HashMap<Uuid, u64> = wc
        .live_lock()
        .iter()
        .map(|(id, l)| (*id, l.done()))
        .collect();
    let rows = db::chunks::get_chunks(&wc.pool, wc.download_id).await?;
    Ok(rows
        .iter()
        .map(|c| live.get(&c.id).copied().unwrap_or(c.downloaded))
        .sum())
}

/// Flush the file, then save the progress the pieces had *before* the flush: a saved offset is never
/// ahead of what is on disk, yet the connections never wait for the disk.
async fn checkpoint(wc: &PieceWorkerCtx, save_total: bool) -> Result<()> {
    let snapshot: Vec<(Uuid, u64)> = wc
        .live_lock()
        .iter()
        .map(|(id, live)| (*id, live.done()))
        .collect();
    if snapshot.is_empty() {
        return Ok(());
    }
    wc.disk.sync().await?;
    for (id, done) in snapshot {
        db::chunks::checkpoint_progress(&wc.pool, id, done).await?;
    }
    if save_total {
        let total = chunk::total_downloaded(&wc.pool, wc.download_id).await?;
        db::downloads::update_progress(&wc.pool, wc.download_id, total).await?;
    }
    Ok(())
}

/// State of the adaptive connection controller (see `adaptive_plan`).
struct AdaptivePlan {
    ceiling: usize,
    next_step_at: tokio::time::Instant,
    /// (evaluate at, speed before the batch, connections before, connections added, re-checks left)
    pending: Option<(tokio::time::Instant, f64, usize, usize, u8)>,
    done: bool,
    /// When a controller that stopped because the speed stopped growing may try once more: a host
    /// or a line can free up during a download that lasts hours. `None` after a refused connection.
    recheck_at: Option<tokio::time::Instant>,
}

/// How long after "speed stopped growing" the controller tries again.
// shortcut: fixed 10 minutes, no setting; make it one if a host needs a different pace.
const ADAPTIVE_RECHECK: std::time::Duration = std::time::Duration::from_secs(600);

impl AdaptivePlan {
    /// A stopped controller starts growing again when its pause is over and there is room left.
    fn recheck_due(&self, now: tokio::time::Instant, connections: usize) -> bool {
        self.done && connections < self.ceiling && self.recheck_at.is_some_and(|at| now >= at)
    }
}

/// Adaptive mode is for big files on servers nobody asked us to be careful with: the user never set a
/// per-host cap, no site rule or learned rate limit lowers it, and there is enough left to share out.
/// It then grows past a fixed 8 connections for as long as the measured speed keeps growing.
async fn adaptive_plan(ctx: &JobContext, file_size: u64, base: usize) -> Option<AdaptivePlan> {
    const MIN_FILE: u64 = 32 * 1024 * 1024;
    if file_size < MIN_FILE || base < 8 {
        return None;
    }
    if !db::settings::get_flag(&ctx.pool, "adaptive_connections", true).await {
        return None;
    }
    // The shipped default is 32 ("no cap"); anything lower is a cap somebody chose on purpose.
    let configured = db::settings::get(&ctx.pool, "max_connections_per_host")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<usize>().ok());
    if configured.is_some_and(|c| c < 32) {
        return None;
    }
    let policy = db::site_rules::policy_for_url(&ctx.pool, &ctx.record.url)
        .await
        .ok()?;
    if policy.max_connections.is_some() {
        return None;
    }
    let ceiling = db::settings::get(&ctx.pool, "adaptive_max_connections")
        .await
        .ok()
        .flatten()
        .and_then(|v| v.trim().parse::<usize>().ok())
        .unwrap_or(24)
        .clamp(base, 32);
    (ceiling > base).then(|| AdaptivePlan {
        ceiling,
        next_step_at: tokio::time::Instant::now() + std::time::Duration::from_secs(60),
        pending: None,
        done: false,
        recheck_at: None,
    })
}

/// Work-stealing segmented download. `planned` is a fine-grained piece list;
/// `concurrency` workers each pull the next pending piece until the queue
/// drains, so a fast connection takes more pieces and none idle early.
#[allow(clippy::too_many_lines)]
async fn run_segmented(
    ctx: JobContext,
    file_size: u64,
    planned: Vec<chunk::ChunkRange>,
    concurrency: usize,
    challenge: Option<String>,
    first_response: Option<reqwest::Response>,
    validator: Option<String>,
) -> Result<()> {
    let auth = crate::auth::AuthState::from_url(&ctx.record.url);
    if let (Some(a), Some(c)) = (&auth, &challenge) {
        a.adopt_plan_challenge(c);
    }
    let path = PathBuf::from(&ctx.record.save_path);
    diagnostics::event(
        "disk_preallocate_start",
        serde_json::json!({
            "id": ctx.record.id,
            "path": path,
            "file_size": file_size,
        }),
    )
    .await;
    let disk = DiskWriter::create(&path, Some(file_size)).await?;
    diagnostics::event(
        "disk_preallocate_done",
        serde_json::json!({
            "id": ctx.record.id,
            "file_size": file_size,
        }),
    )
    .await;
    let existing = db::chunks::get_chunks(&ctx.pool, ctx.record.id).await?;
    if existing.is_empty() {
        db::chunks::insert_chunks(&ctx.pool, ctx.record.id, &planned).await?;
    }
    db::chunks::reset_chunks_for_resume(&ctx.pool, ctx.record.id).await?;

    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Downloading, None)
        .await?;

    let mut chunks = db::chunks::get_incomplete_chunks(&ctx.pool, ctx.record.id).await?;
    chunks.sort_by_key(|chunk| chunk.chunk_index);
    // The planning GET only helps a piece 0 that starts from byte 0 (not a resume).
    let first_response = first_response.filter(|_| {
        chunks
            .iter()
            .any(|c| c.chunk_index == 0 && c.downloaded == 0)
    });

    let worker_count = concurrency.max(1).min(chunks.len().max(1)).min(4);
    diagnostics::event(
        "segmented_workers_start",
        serde_json::json!({
            "id": ctx.record.id,
            "worker_count": worker_count,
        }),
    )
    .await;
    let queue: Arc<Mutex<VecDeque<ChunkRecord>>> =
        Arc::new(Mutex::new(chunks.into_iter().collect()));

    let hyper_client = chunk::make_hyper_client()?;
    let primary_owned = ctx
        .record
        .final_url
        .as_deref()
        .filter(|u| !u.is_empty())
        .unwrap_or(ctx.record.url.as_str())
        .to_string();
    // WebDAV scheme → http(s) so Range GETs use the normal HTTP engine.
    let primary = crate::webdav::effective_http_url(&primary_owned);
    let sources = chunk::build_sources(&primary, &ctx.record.mirrors);
    let mirrors = Arc::new(chunk::MirrorSet::new(sources));
    if mirrors.source_count() > 1 {
        diagnostics::event(
            "multi_source_start",
            serde_json::json!({
                "id": ctx.record.id,
                "source_count": mirrors.source_count(),
            }),
        )
        .await;
    }
    let worker_ctx = PieceWorkerCtx {
        hyper_client: hyper_client.clone(),
        pool: ctx.pool.clone(),
        disk: disk.clone(),
        download_id: ctx.record.id,
        mirrors,
        bandwidth: ctx.bandwidth.clone(),
        cancel_rx: ctx.cancel_rx.clone(),
        cookies: ctx.cookies.clone(),
        extra_headers: ctx.extra_headers.clone(),
        proxy_client: ctx.proxy_client.clone(),
        restarts: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        live: Arc::new(std::sync::Mutex::new(HashMap::new())),
        steal_lock: Arc::new(Mutex::new(())),
        min_steal: min_steal(file_size),
        auth,
        first: Arc::new(std::sync::Mutex::new(first_response)),
        validator,
        soft: false,
        extra_rejected: Arc::new(AtomicBool::new(false)),
    };

    let mut join_set = JoinSet::new();
    for _ in 0..worker_count {
        join_set.spawn(download_piece_worker(queue.clone(), worker_ctx.clone()));
    }

    let stop_reporter = Arc::new(AtomicBool::new(false));
    let start_total = chunk::total_downloaded(&ctx.pool, ctx.record.id)
        .await
        .unwrap_or(0);
    let shown_total = Arc::new(std::sync::atomic::AtomicU64::new(start_total));
    let reporter = spawn_progress_reporter(
        ctx.pool.clone(),
        ctx.record.id,
        ctx.progress_tx.clone(),
        file_size,
        stop_reporter.clone(),
        shown_total.clone(),
        start_total,
    );
    let checkpointer = {
        let wc = worker_ctx.clone();
        let stop = stop_reporter.clone();
        let shown_total = shown_total.clone();
        tokio::spawn(async move {
            let mut round = 0_u32;
            let mut sampled = start_total;
            loop {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                if stop.load(Ordering::Relaxed) {
                    break;
                }
                round += 1;
                // What the lists and `GET /downloads` show: cheap, twice a second, no flush involved.
                if let Ok(now) = live_progress(&wc).await {
                    let now = now.min(file_size);
                    shown_total.store(now, Ordering::Relaxed);
                    let _ = db::downloads::update_progress(&wc.pool, wc.download_id, now).await;
                    // A speed sample every 30 s, so a long download can be analysed afterwards.
                    if round % 60 == 0 {
                        diagnostics::event(
                            "progress_sample",
                            serde_json::json!({
                                "id": wc.download_id,
                                "downloaded": now,
                                "mbps": now.saturating_sub(sampled) as f64 / 30.0 / 1_048_576.0,
                                "pieces_in_flight": wc.live_lock().len(),
                            }),
                        )
                        .await;
                        sampled = now;
                    }
                }
                // What a restart may rely on: flush first, then save the offsets, every 2 s.
                if round % 4 == 0 {
                    if let Err(e) = checkpoint(&wc, false).await {
                        tracing::warn!(error = %e, "checkpoint failed");
                    }
                }
            }
        })
    };

    let mut fallback: Option<SegmentFallback> = None;
    let mut worker_error: Option<anyhow::Error> = None;

    let mut adaptive = adaptive_plan(&ctx, file_size, concurrency).await;
    // First check after 3 s (every second in adaptive mode); the 4 + ramp-to-8 start-up is handled by `ramp`.
    let tick_secs: f64 = if adaptive.is_some() { 1.0 } else { 3.0 };
    let mut check_interval = tokio::time::interval_at(
        tokio::time::Instant::now()
            + std::time::Duration::from_millis(if adaptive.is_some() { 1500 } else { 3000 }),
        std::time::Duration::from_secs_f64(tick_secs),
    );
    let seen_at_job_start = ctx.bandwidth.bytes_seen();
    let mut last_seen = ctx.bandwidth.bytes_seen();
    let mut last_tick = tokio::time::Instant::now();
    let mut ewma = 0.0_f64;
    // The last two one-second speeds: decisions use their average, and only when they agree.
    let mut recent: [f64; 2] = [0.0, 0.0];
    let ramp = tokio::time::sleep(std::time::Duration::from_secs(1));
    tokio::pin!(ramp);
    let mut ramp_done = false;
    let mut last_downloaded = ctx.bandwidth.bytes_seen();
    let mut last_speed = 0_u64;
    let mut current_concurrency = worker_count;
    let max_concurrency = concurrency.clamp(1, 32);

    loop {
        if *ctx.cancel_rx.borrow() {
            join_set.abort_all();
            stop_reporter.store(true, Ordering::Relaxed);
            reporter.abort();
            checkpointer.abort();
            let _ = reporter.await;
            let _ = checkpointer.await;
            let _ = checkpoint(&worker_ctx, true).await;
            return Err(FerricError::Cancelled.into());
        }

        tokio::select! {
            result = join_set.join_next() => {
                let Some(res) = result else {
                    break;
                };
                match res.context("chunk worker panicked")? {
                    Ok(()) => {
                        current_concurrency = current_concurrency.saturating_sub(1);
                    }
                    Err(e) if is_rate_limited(&e) => {
                        join_set.abort_all();
                        fallback = Some(SegmentFallback::Sequential);
                        break;
                    }
                    Err(e) if is_range_rejected(&e) || e.downcast_ref::<TooManyRestarts>().is_some() => {
                        join_set.abort_all();
                        fallback = Some(SegmentFallback::SingleStream);
                        break;
                    }
                    Err(e) => {
                        join_set.abort_all();
                        worker_error = Some(e);
                        break;
                    }
                }
            }
            () = &mut ramp, if !ramp_done => {
                ramp_done = true;
                if let Some(a) = adaptive.as_mut() {
                    // Let the base connections settle before judging them.
                    a.next_step_at = tokio::time::Instant::now() + std::time::Duration::from_millis(1000);
                }
                let target = concurrency.max(1).min(max_concurrency);
                let queue_len = queue.lock().await.len();
                if current_concurrency < target && (queue_len > 0 || worker_ctx.has_stealable()) {
                    let to_spawn = (target - current_concurrency).min(4);
                    for _ in 0..to_spawn {
                        join_set.spawn(download_piece_worker(queue.clone(), worker_ctx.clone()));
                    }
                    current_concurrency += to_spawn;
                }
            }
            _ = check_interval.tick() => {
                let queue_len = queue.lock().await.len();
                let has_work = queue_len > 0 || worker_ctx.has_stealable();
                if adaptive.is_some() {
                    // Adaptive mode: measure the job's own speed every tick and keep adding connections
                    // for as long as each batch still pays for itself.
                    let seen = ctx.bandwidth.bytes_seen();
                    let now = tokio::time::Instant::now();
                    let elapsed = now.duration_since(last_tick).as_secs_f64().max(0.05);
                    last_tick = now;
                    let tick_speed = seen.saturating_sub(last_seen) as f64 / elapsed;
                    last_seen = seen;
                    ewma = if ewma == 0.0 { tick_speed } else { 0.5 * ewma + 0.5 * tick_speed };
                    recent = [recent[1], tick_speed];
                    let steady = recent[0] > 0.0 && (recent[0] - recent[1]).abs() <= 0.15 * recent[0].max(recent[1]);
                    let avg2 = (recent[0] + recent[1]) / 2.0;
                    tracing::debug!(id = %ctx.record.id, tick_mbps = tick_speed / 1_048_576.0, ewma_mbps = ewma / 1_048_576.0, connections = current_concurrency, "adaptive tick");
                    if let Some(a) = adaptive.as_mut() {
                        if ramp_done && has_work {
                            let now = tokio::time::Instant::now();
                            let rejected = worker_ctx.extra_rejected.load(Ordering::SeqCst);
                            if rejected && !a.done {
                                a.done = true;
                                a.pending = None;
                                a.recheck_at = None;
                                tracing::info!(id = %ctx.record.id, connections = current_concurrency, "adaptive connections: server refused an extra connection, stopping");
                            }
                            if a.recheck_due(now, current_concurrency) {
                                a.done = false;
                                a.recheck_at = None;
                                a.next_step_at = now;
                                tracing::info!(id = %ctx.record.id, connections = current_concurrency, "adaptive connections: trying to grow again");
                            }
                            if let Some((due, before, count_before, added, rechecks)) = a.pending {
                                if now >= due {
                                    // Each added connection should bring at least half of what an average
                                    // connection brought before; otherwise the server (or the line) is full.
                                    let per_conn = before / count_before.max(1) as f64;
                                    let gain = avg2 - before;
                                    if gain >= 0.5 * per_conn * added as f64 {
                                        a.pending = None;
                                    } else if rechecks > 0 {
                                        // New connections need a moment to speed up (TCP slow start): look once more.
                                        a.pending = Some((now + std::time::Duration::from_millis(1500), before, count_before, added, rechecks - 1));
                                    } else {
                                        a.pending = None;
                                        a.done = true;
                                        a.recheck_at = Some(now + ADAPTIVE_RECHECK);
                                        tracing::info!(
                                            id = %ctx.record.id,
                                            connections = current_concurrency,
                                            speed_mbps = avg2 / 1_048_576.0,
                                            before_mbps = before / 1_048_576.0,
                                            added,
                                            "adaptive connections: speed stopped growing, keeping the current count"
                                        );
                                    }
                                }
                            }
                            if a.pending.is_none() && !a.done && steady && current_concurrency < a.ceiling && now >= a.next_step_at {
                                // Slow start: half again as many connections each round, so 8 -> 12 -> 18 -> 24.
                                let add = (current_concurrency / 2).max(4).min(a.ceiling - current_concurrency);
                                let mut spawned = 0;
                                for _ in 0..add {
                                    let mut extra = worker_ctx.clone();
                                    extra.soft = true;
                                    join_set.spawn(download_piece_worker(queue.clone(), extra));
                                    spawned += 1;
                                }
                                a.pending = Some((
                                    now + std::time::Duration::from_millis(1000),
                                    avg2,
                                    current_concurrency,
                                    spawned,
                                    1,
                                ));
                                current_concurrency += spawned;
                                tracing::debug!(id = %ctx.record.id, connections = current_concurrency, "adaptive connections: added a batch");
                            }
                        }
                    }
                } else if has_work && current_concurrency < max_concurrency {
                    let downloaded = ctx.bandwidth.bytes_seen();
                    let delta = downloaded.saturating_sub(last_downloaded);
                    last_downloaded = downloaded;
                    let current_speed = delta / 3;

                    let is_unsaturated = current_speed == 0 || current_speed > last_speed * 105 / 100;
                    last_speed = current_speed;

                    if is_unsaturated {
                        let to_spawn = 2.min(max_concurrency - current_concurrency).min(queue_len.max(1));
                        if to_spawn > 0 {
                            tracing::debug!(
                                id = %ctx.record.id,
                                current = current_concurrency,
                                new = current_concurrency + to_spawn,
                                "dynamic connection scaling: spawning more workers"
                            );
                            for _ in 0..to_spawn {
                                join_set.spawn(download_piece_worker(queue.clone(), worker_ctx.clone()));
                            }
                            current_concurrency += to_spawn;
                        }
                    }
                }
            }
        }
    }

    stop_reporter.store(true, Ordering::Relaxed);
    // Both only sleep and save numbers: stop them now instead of waiting out their half-second naps.
    reporter.abort();
    checkpointer.abort();
    let _ = reporter.await;
    let _ = checkpointer.await;
    // Whatever the pieces had reached when the workers stopped (a failure or a fallback is next).
    let _ = checkpoint(&worker_ctx, true).await;
    tracing::info!(
        id = %ctx.record.id,
        received_bytes = ctx.bandwidth.bytes_seen().saturating_sub(seen_at_job_start),
        file_bytes = file_size,
        "segmented transfer ended"
    );

    if let Some(e) = worker_error {
        return Err(e);
    }

    let mut completed_without_fallback = true;
    match fallback {
        Some(SegmentFallback::SingleStream) => {
            tracing::warn!(
                id = %ctx.record.id,
                "server rejected Range; falling back to single-stream"
            );
            return run_single_stream(ctx, None).await;
        }
        Some(SegmentFallback::Sequential) => {
            completed_without_fallback = false;
            save_rate_limit_rule(&ctx.pool, &ctx.record.url).await?;
            db::chunks::reset_chunks_for_resume(&ctx.pool, ctx.record.id).await?;
            let mut tracker = SpeedTracker::new(5);
            download_remaining_sequential(&ctx, &disk, file_size, &mut tracker, &hyper_client)
                .await?;
        }
        None => {}
    }

    if !db::chunks::all_chunks_complete(&ctx.pool, ctx.record.id).await? {
        let left = db::chunks::get_incomplete_chunks(&ctx.pool, ctx.record.id).await?;
        return Err(anyhow::anyhow!(
            "download finished with incomplete chunks: {}",
            left.iter()
                .map(|c| format!(
                    "#{} {:?} {}/{}..{}",
                    c.chunk_index, c.status, c.downloaded, c.byte_start, c.byte_end
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }

    disk.sync().await?;
    db::downloads::update_progress(&ctx.pool, ctx.record.id, file_size).await?;
    let checksum_ok = verify_completion_checksum(&ctx, &path).await?;
    if checksum_ok == Some(false) {
        spawn_av_scan(path);
        return Ok(());
    }
    // A host is capped only by what it actually refused (a rate limit, see `save_rate_limit_rule`) or by a
    // rule the user set. "It worked with N connections" is not a limit: remembering it as one kept every
    // later download from that host at the starting 4 and stopped the ramp to 8.
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Completed, None).await?;
    emit_done(&ctx, checksum_ok);
    spawn_av_scan(path);
    Ok(())
}

/// One concurrent connection: pull pending pieces from the shared queue and
/// download each until the queue is empty (`Ok`) or a piece fails (`Err`).
async fn download_piece_worker(
    queue: Arc<Mutex<VecDeque<ChunkRecord>>>,
    wc: PieceWorkerCtx,
) -> Result<()> {
    loop {
        if *wc.cancel_rx.borrow() {
            return Err(FerricError::Cancelled.into());
        }
        let next = queue.lock().await.pop_front();
        let chunk = if let Some(chunk) = next {
            chunk
        } else if let Some(stolen) = try_steal(&wc).await {
            stolen
        } else {
            return Ok(());
        };

        let chunk_id = chunk.id;
        let live = {
            let l = Arc::new(chunk::LiveChunk::new(
                chunk.byte_start,
                chunk.byte_end,
                chunk.downloaded,
            ));
            wc.live_lock().insert(chunk_id, l.clone());
            Some(l)
        };
        let reached_live = live.clone();
        let result = download_piece_multi_source(&wc, chunk, live).await;
        wc.live_lock().remove(&chunk_id);
        // Progress is only saved to the database on the checkpointer's clock, so a re-queued piece
        // takes whichever is further: the saved offset or what this attempt actually received.
        let reached = reached_live.map_or(0, |l| l.done());
        if let Err(e) = result {
            let cancelled = matches!(
                e.downcast_ref::<FerricError>(),
                Some(FerricError::Cancelled)
            );
            if wc.soft && !cancelled {
                // An extra connection the server did not like: give its piece back and stop it.
                tracing::info!(error = %e, "extra connection refused; the job continues without it");
                wc.extra_rejected.store(true, Ordering::SeqCst);
                let fresh = db::chunks::get_chunks(&wc.pool, wc.download_id)
                    .await?
                    .into_iter()
                    .find(|c| c.id == chunk_id);
                if let Some(fresh) = fresh {
                    if !fresh.is_complete() {
                        queue.lock().await.push_back(fresh);
                    }
                }
                return Ok(());
            }
            if is_piece_terminal(&e) {
                return Err(e);
            }
            let restarts = wc
                .restarts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if restarts > MAX_PIECE_RESTARTS {
                return Err(TooManyRestarts.into());
            }
            tracing::warn!(restarts, error = %e, "piece failed; re-queueing");
            // Re-read the row so the retry resumes from the last saved progress.
            let fresh = db::chunks::get_chunks(&wc.pool, wc.download_id)
                .await?
                .into_iter()
                .find(|c| c.id == chunk_id);
            if let Some(mut fresh) = fresh {
                fresh.downloaded = fresh.downloaded.max(reached);
                if !fresh.is_complete() {
                    queue.lock().await.push_back(fresh);
                }
            }
            let pause = if cfg!(test) { 20 } else { 1000 };
            tokio::time::sleep(std::time::Duration::from_millis(pause)).await;
        }
    }
}

/// Download one piece, trying live mirrors round-robin until one succeeds or
/// every survivor has failed (terminal protocol errors abort immediately).
async fn download_piece_multi_source(
    wc: &PieceWorkerCtx,
    chunk: ChunkRecord,
    live: Option<Arc<chunk::LiveChunk>>,
) -> Result<()> {
    let chunk_index = chunk.chunk_index;
    let mut tried = std::collections::HashSet::new();
    let mut last_error: Option<anyhow::Error> = None;

    loop {
        let Some((src_idx, url)) = wc.mirrors.pick(chunk_index) else {
            return Err(last_error.unwrap_or_else(|| {
                anyhow::anyhow!("no live download sources remaining for chunk {chunk_index}")
            }));
        };
        if !tried.insert(src_idx) {
            // Already tried every currently-live source for this chunk.
            return Err(last_error.unwrap_or_else(|| {
                anyhow::anyhow!("all live sources failed for chunk {chunk_index}")
            }));
        }

        let args = ChunkArgs {
            hyper_client: wc.hyper_client.clone(),
            pool: wc.pool.clone(),
            disk: wc.disk.clone(),
            download_id: wc.download_id,
            chunk: chunk.clone(),
            url: url.clone(),
            bandwidth: wc.bandwidth.clone(),
            cancel_rx: wc.cancel_rx.clone(),
            cookies: wc.cookies.clone(),
            extra_headers: wc.extra_headers.clone(),
            proxy_client: wc.proxy_client.clone(),
            live: live.clone(),
            auth: wc.auth.clone(),
            first: Some(wc.first.clone()),
            validator: wc.validator.clone(),
        };

        match download_chunk_with_retry(args).await {
            Ok(_) => return Ok(()),
            Err(e) if is_rate_limited(&e) || is_range_rejected(&e) => return Err(e),
            Err(e)
                if matches!(
                    e.downcast_ref::<FerricError>(),
                    Some(FerricError::Cancelled)
                ) =>
            {
                return Err(e);
            }
            Err(e) => {
                tracing::warn!(
                    chunk = chunk_index,
                    source = %security::redact_url(&url),
                    error = %e,
                    "mirror failed; redistributing chunk to survivors"
                );
                diagnostics::event(
                    "mirror_failed",
                    serde_json::json!({
                        "download_id": wc.download_id,
                        "chunk": chunk_index,
                        "source_index": src_idx,
                        "live_remaining": wc.mirrors.live_count().saturating_sub(1),
                        "error": e.to_string(),
                    }),
                )
                .await;
                // Never retire the last live source: it is retried by the caller.
                if wc.mirrors.live_count() > 1 {
                    wc.mirrors.mark_dead(src_idx);
                }
                last_error = Some(e);
            }
        }
    }
}

/// Periodically emit a smooth progress event independent of piece boundaries,
/// reading the aggregate downloaded bytes from the chunk table.
fn spawn_progress_reporter(
    pool: SqlitePool,
    download_id: Uuid,
    progress_tx: broadcast::Sender<ProgressEvent>,
    file_size: u64,
    stop: Arc<AtomicBool>,
    shown: Arc<std::sync::atomic::AtomicU64>,
    start_total: u64,
) -> tokio::task::JoinHandle<()> {
    let _ = pool; // progress comes from the checkpointer's live view (see `live_progress`)
    tokio::spawn(async move {
        let mut tracker = SpeedTracker::new(5);
        let mut last = start_total;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if stop.load(Ordering::Relaxed) {
                break;
            }
            let downloaded = shown
                .load(Ordering::Relaxed)
                .max(start_total)
                .min(file_size);
            let delta = downloaded.saturating_sub(last);
            if delta > 0 {
                tracker.record(delta);
            }
            last = downloaded;
            let remaining = file_size.saturating_sub(downloaded);
            let _ = progress_tx.send(ProgressEvent::Progress {
                id: download_id,
                downloaded,
                total: Some(file_size),
                speed_bps: tracker.speed_bps(),
                eta_secs: tracker.eta_secs(remaining),
                status: DownloadStatus::Downloading,
            });
        }
    })
}

async fn download_remaining_sequential(
    ctx: &JobContext,
    disk: &DiskWriter,
    file_size: u64,
    tracker: &mut SpeedTracker,
    hyper_client: &chunk::HyperClient,
) -> Result<()> {
    let mut chunks = db::chunks::get_incomplete_chunks(&ctx.pool, ctx.record.id).await?;
    chunks.sort_by_key(|chunk| chunk.chunk_index);

    for chunk in chunks {
        if *ctx.cancel_rx.borrow() {
            return Err(FerricError::Cancelled.into());
        }

        let url = crate::webdav::effective_http_url(&ctx.record.url);
        let written = if is_ftp_family(&url) {
            crate::ftp::download_ftp_chunk(ChunkArgs {
                hyper_client: hyper_client.clone(),
                pool: ctx.pool.clone(),
                disk: disk.clone(),
                download_id: ctx.record.id,
                chunk,
                url,
                bandwidth: ctx.bandwidth.clone(),
                cancel_rx: ctx.cancel_rx.clone(),
                cookies: ctx.cookies.clone(),
                extra_headers: ctx.extra_headers.clone(),
                proxy_client: ctx.proxy_client.clone(),
                live: None,
                auth: crate::auth::AuthState::from_url(&ctx.record.url),
                first: None,
                validator: None,
            })
            .await?
        } else if url.starts_with("sftp://") {
            crate::sftp::download_sftp_chunk(ChunkArgs {
                hyper_client: hyper_client.clone(),
                pool: ctx.pool.clone(),
                disk: disk.clone(),
                download_id: ctx.record.id,
                chunk,
                url,
                bandwidth: ctx.bandwidth.clone(),
                cancel_rx: ctx.cancel_rx.clone(),
                cookies: ctx.cookies.clone(),
                extra_headers: ctx.extra_headers.clone(),
                proxy_client: ctx.proxy_client.clone(),
                live: None,
                auth: crate::auth::AuthState::from_url(&ctx.record.url),
                first: None,
                validator: None,
            })
            .await?
        } else {
            chunk::download_chunk(ChunkArgs {
                hyper_client: hyper_client.clone(),
                pool: ctx.pool.clone(),
                disk: disk.clone(),
                download_id: ctx.record.id,
                chunk,
                url,
                bandwidth: ctx.bandwidth.clone(),
                cancel_rx: ctx.cancel_rx.clone(),
                cookies: ctx.cookies.clone(),
                extra_headers: ctx.extra_headers.clone(),
                proxy_client: ctx.proxy_client.clone(),
                live: None,
                auth: crate::auth::AuthState::from_url(&ctx.record.url),
                first: None,
                validator: None,
            })
            .await?
        };
        tracker.record(written);
        let downloaded = chunk::total_downloaded(&ctx.pool, ctx.record.id).await?;
        db::downloads::update_progress(&ctx.pool, ctx.record.id, downloaded).await?;
        emit_progress(ctx, downloaded, Some(file_size), tracker);
    }

    Ok(())
}

async fn apply_response_metadata(
    mut ctx: JobContext,
    metadata: &ResponseMetadata,
) -> Result<JobContext> {
    let mut filename = None;
    let mut save_path = None;

    if let Some(server_name) = metadata.filename.as_deref() {
        if should_replace_filename(&ctx.record, server_name) {
            let current_path = PathBuf::from(&ctx.record.save_path);
            let save_dir = current_path.parent().unwrap_or_else(|| Path::new("."));
            let target = security::safe_filename(server_name, save_dir)?;
            if let Some(name) = target.file_name().and_then(|value| value.to_str()) {
                filename = Some(name.to_string());
                save_path = Some(target.to_string_lossy().to_string());
            }
        }
    }

    db::downloads::update_response_metadata(
        &ctx.pool,
        ctx.record.id,
        metadata.final_url.as_deref(),
        metadata.redirect_chain.as_deref(),
        metadata.mime_type.as_deref(),
        filename.as_deref(),
        save_path.as_deref(),
    )
    .await?;

    if let Some(value) = metadata.final_url.clone() {
        ctx.record.final_url = Some(value);
    }
    if let Some(value) = metadata.mime_type.clone() {
        ctx.record.mime_type = Some(value);
    }
    if let Some(value) = filename {
        ctx.record.filename = value;
    }
    if let Some(value) = save_path {
        ctx.record.save_path = value;
    }

    Ok(ctx)
}

fn should_replace_filename(record: &DownloadRecord, server_name: &str) -> bool {
    let server_name = server_name.trim();
    if server_name.is_empty() || server_name == record.filename {
        return false;
    }
    let current_has_extension = Path::new(&record.filename).extension().is_some();
    if !current_has_extension {
        return true;
    }
    url::Url::parse(&record.url)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_string))
        .is_some_and(|fallback| fallback == record.filename)
}

#[derive(Debug, Clone, Copy)]
enum ManifestKind {
    Hls,
    Dash,
}

async fn run_streaming_manifest(ctx: JobContext, kind: ManifestKind) -> Result<()> {
    // HLS carries optional AES-128 keys; DASH is plain segment URLs.
    let video_hls = match kind {
        ManifestKind::Hls => {
            Some(crate::hls::fetch_hls_segments(&ctx.record.url, &ctx.client).await?)
        }
        ManifestKind::Dash => None,
    };
    let video_urls: Vec<String> = match kind {
        ManifestKind::Hls => video_hls
            .as_ref()
            .map(|s| s.iter().map(|x| x.url.clone()).collect())
            .unwrap_or_default(),
        ManifestKind::Dash => crate::dash::fetch_segments(&ctx.record.url, &ctx.client).await?,
    };

    let mut audio_urls = Vec::new();
    let mut audio_hls = None;
    if let Some(ref audio_url) = ctx.record.audio_url {
        crate::security::validate_url(audio_url)?;
        if is_hls(audio_url, None) {
            let segs = crate::hls::fetch_hls_segments(audio_url, &ctx.client).await?;
            audio_urls = segs.iter().map(|s| s.url.clone()).collect();
            audio_hls = Some(segs);
        } else if is_dash(audio_url, None) {
            audio_urls = crate::dash::fetch_segments(audio_url, &ctx.client).await?;
        } else {
            audio_urls = vec![audio_url.clone()];
        }
    }

    let mut subtitle_urls = Vec::new();
    if let Some(ref subtitle_url) = ctx.record.subtitle_url {
        crate::security::validate_url(subtitle_url)?;
        subtitle_urls = if is_hls(subtitle_url, None) {
            crate::hls::fetch_segments(subtitle_url, &ctx.client).await?
        } else if is_dash(subtitle_url, None) {
            crate::dash::fetch_segments(subtitle_url, &ctx.client).await?
        } else {
            vec![subtitle_url.clone()]
        };
    }

    if video_urls.is_empty() {
        return Err(anyhow::anyhow!("video stream has no media segments"));
    }

    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Downloading, None)
        .await?;

    let final_path = PathBuf::from(&ctx.record.save_path);
    let wants_mp4 = final_path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("mp4"));
    // Concat target may be remuxed to final_path for single-stream HLS/DASH.
    let video_concat = if audio_urls.is_empty() && wants_mp4 {
        PathBuf::from(format!("{}.tmp_concat", ctx.record.save_path))
    } else if audio_urls.is_empty() {
        final_path.clone()
    } else {
        PathBuf::from(format!("{}.tmp_video", ctx.record.save_path))
    };
    let audio_target = PathBuf::from(format!("{}.tmp_audio", ctx.record.save_path));
    let subtitle_target = PathBuf::from(format!("{}.tmp_subtitle", ctx.record.save_path));

    let total_segments = video_urls.len() + audio_urls.len() + subtitle_urls.len();
    let mut downloaded = 0_u64;
    let mut segments_downloaded = 0;
    let mut tracker = SpeedTracker::new(5);

    // 1. Video — parallel segments + AES-128 when HLS
    let n = assemble_segments_parallel(
        &ctx,
        &video_urls,
        video_hls.as_deref(),
        &video_concat,
        &mut downloaded,
        &mut segments_downloaded,
        total_segments,
        &mut tracker,
    )
    .await?;
    let _ = n;

    // 2. Audio
    if !audio_urls.is_empty() {
        assemble_segments_parallel(
            &ctx,
            &audio_urls,
            audio_hls.as_deref(),
            &audio_target,
            &mut downloaded,
            &mut segments_downloaded,
            total_segments,
            &mut tracker,
        )
        .await?;
    }

    // 3. Subtitles (usually few / sequential is fine — still use parallel helper)
    if !subtitle_urls.is_empty() {
        assemble_segments_parallel(
            &ctx,
            &subtitle_urls,
            None,
            &subtitle_target,
            &mut downloaded,
            &mut segments_downloaded,
            total_segments,
            &mut tracker,
        )
        .await?;
    }

    // 4. Mux / remux
    if !audio_urls.is_empty() {
        let save_dir = final_path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("save path has no parent"))?;
        crate::media::run_ffmpeg_merge(&video_concat, &audio_target, &final_path, save_dir).await?;
        let _ = tokio::fs::remove_file(&video_concat).await;
        let _ = tokio::fs::remove_file(&audio_target).await;
    } else if video_concat != final_path {
        // Single stream: remux TS/fMP4 concat → mp4 when ffmpeg present
        match crate::hls::remux_to_mp4(&video_concat, &final_path).await {
            Ok(()) => {
                let _ = tokio::fs::remove_file(&video_concat).await;
            }
            Err(e) => {
                tracing::warn!(error = %e, "remux failed; keeping concatenated stream as output");
                if tokio::fs::rename(&video_concat, &final_path).await.is_err() {
                    tokio::fs::copy(&video_concat, &final_path).await?;
                    let _ = tokio::fs::remove_file(&video_concat).await;
                }
            }
        }
    }

    // 5. Subtitle placement
    if !subtitle_urls.is_empty() {
        let subtitle_path = get_subtitle_path(
            &final_path,
            ctx.record.subtitle_lang.as_deref(),
            ctx.record.subtitle_url.as_deref().unwrap_or(""),
        );
        if let Err(_e) = tokio::fs::rename(&subtitle_target, &subtitle_path).await {
            tokio::fs::copy(&subtitle_target, &subtitle_path).await?;
            let _ = tokio::fs::remove_file(&subtitle_target).await;
        }
    }

    let checksum_ok = verify_completion_checksum(&ctx, &final_path).await?;
    if checksum_ok == Some(false) {
        spawn_av_scan(final_path);
        return Ok(());
    }
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Completed, None).await?;
    emit_done(&ctx, checksum_ok);
    spawn_av_scan(final_path);
    Ok(())
}

/// Download segments in parallel (capped concurrency), decrypt AES-128 HLS when
/// keys are provided, then concat in playlist order onto `target`.
#[allow(clippy::too_many_arguments)]
async fn assemble_segments_parallel(
    ctx: &JobContext,
    urls: &[String],
    hls: Option<&[crate::hls::HlsSegment]>,
    target: &Path,
    downloaded: &mut u64,
    segments_downloaded: &mut usize,
    total_segments: usize,
    tracker: &mut SpeedTracker,
) -> Result<u64> {
    if urls.is_empty() {
        return Ok(0);
    }
    const MAX_PARALLEL: usize = 8;
    let parallel = MAX_PARALLEL.min(urls.len()).max(1);

    // Prefetch AES keys once per unique URI.
    let mut key_cache: HashMap<String, [u8; 16]> = HashMap::new();
    if let Some(segs) = hls {
        for seg in segs {
            if let Some(ref k) = seg.key {
                if !key_cache.contains_key(&k.uri) {
                    let key = crate::hls::fetch_aes_key(&ctx.client, &k.uri).await?;
                    key_cache.insert(k.uri.clone(), key);
                }
            }
        }
    }

    // Segments go to disk in playlist order as they arrive; only out-of-order ones wait in memory
    // (never more than `parallel`), so a long stream never has to fit in RAM.
    let mut output = tokio::fs::File::create(target)
        .await
        .with_context(|| format!("create {}", target.display()))?;
    let mut waiting: std::collections::BTreeMap<usize, Vec<u8>> = std::collections::BTreeMap::new();
    let mut next_to_write = 0_usize;
    let mut next_to_spawn = 0_usize;
    let mut join_set = JoinSet::new();
    let mut in_flight = 0_usize;

    while next_to_spawn < urls.len() || in_flight > 0 {
        if *ctx.cancel_rx.borrow() {
            join_set.abort_all();
            return Err(FerricError::Cancelled.into());
        }
        while in_flight < parallel && next_to_spawn < urls.len() {
            let idx = next_to_spawn;
            next_to_spawn += 1;
            in_flight += 1;
            let url = urls[idx].clone();
            // EXT-X-BYTERANGE: this segment is only a slice of the resource.
            let range = hls.and_then(|s| s.get(idx)).and_then(|s| s.range);
            let client = ctx.client.clone();
            let cookies = ctx.cookies.clone();
            let headers = ctx.extra_headers.clone();
            let method = ctx.method.clone();
            let post_body = ctx.post_body.clone();
            // Lightweight GET via shared client (no need full JobContext in spawn)
            join_set.spawn(async move {
                let builder = client.get(&url);
                let mut builder =
                    apply_request_headers(builder, cookies.as_deref(), headers.as_ref());
                if let Some((start, end)) = range {
                    builder =
                        builder.header(reqwest::header::RANGE, format!("bytes={start}-{end}"));
                }
                let _ = (method, post_body); // reserved for POST-manifest edge cases
                let response = builder.send().await?;
                let status = response.status().as_u16();
                if status == 403 {
                    return Err(FerricError::Forbidden.into());
                }
                if status == 404 {
                    return Err(FerricError::NotFound.into());
                }
                if status == 429 {
                    return Err(FerricError::RateLimited.into());
                }
                if !response.status().is_success() {
                    return Err(
                        FerricError::from_status(status, "stream segment request failed").into(),
                    );
                }
                let mut data = response.bytes().await?.to_vec();
                // A server that ignores Range answers 200 with the whole file: cut out the slice we asked for.
                if let (Some((start, end)), 200) = (range, status) {
                    let (s, e) = (usize::try_from(start)?, usize::try_from(end)?);
                    data = data
                        .get(s..=e)
                        .ok_or_else(|| {
                            anyhow::anyhow!("segment byte range lies outside the resource")
                        })?
                        .to_vec();
                }
                Ok::<_, anyhow::Error>((idx, data))
            });
        }

        let Some(joined) = join_set.join_next().await else {
            break;
        };
        in_flight = in_flight.saturating_sub(1);
        let (idx, mut data) = joined.context("segment worker panicked")??;

        // Decrypt HLS AES-128 if needed
        if let Some(segs) = hls {
            if let Some(key_info) = segs.get(idx).and_then(|s| s.key.as_ref()) {
                let key = key_cache
                    .get(&key_info.uri)
                    .ok_or_else(|| anyhow::anyhow!("missing AES key cache entry"))?;
                let iv = key_info
                    .iv
                    .ok_or_else(|| anyhow::anyhow!("AES-128 segment missing IV"))?;
                data = crate::hls::decrypt_aes128(&data, key, &iv)?;
            }
        }

        ctx.bandwidth.acquire(data.len()).await;
        let len = u64::try_from(data.len()).context("segment length exceeds u64")?;
        waiting.insert(idx, data);
        while let Some(part) = waiting.remove(&next_to_write) {
            output.write_all(&part).await?;
            next_to_write += 1;
        }
        *downloaded = downloaded.saturating_add(len);
        tracker.record(len);
        *segments_downloaded += 1;
        db::downloads::update_progress(&ctx.pool, ctx.record.id, *downloaded).await?;
        let hint = streaming_total_hint(*downloaded, *segments_downloaded, total_segments);
        // The size recorded when the address was probed is the playlist's own length; replace it with
        // the running estimate of the whole stream so the list shows a sensible percentage.
        if let Some(total) = hint.filter(|_| *segments_downloaded % 5 == 1) {
            let _ = db::downloads::set_file_info(&ctx.pool, ctx.record.id, total, None, None).await;
        }
        emit_progress(ctx, *downloaded, hint, tracker);
    }

    if next_to_write != urls.len() {
        return Err(anyhow::anyhow!("missing segment {next_to_write}"));
    }
    output.flush().await?;
    output.sync_all().await?;
    Ok(*downloaded)
}

pub(crate) fn is_manifest_url(url: &str) -> bool {
    is_hls(url, None) || is_dash(url, None)
}

fn get_subtitle_path(video_path: &Path, language: Option<&str>, subtitle_url: &str) -> PathBuf {
    let ext = if subtitle_url.to_ascii_lowercase().contains("srt") {
        "srt"
    } else {
        "vtt"
    };
    let parent = video_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = video_path
        .file_stem()
        .map_or("download", |s| s.to_str().unwrap_or("download"));
    if let Some(lang) = language.filter(|l| !l.is_empty()) {
        parent.join(format!("{stem}.{lang}.{ext}"))
    } else {
        parent.join(format!("{stem}.{ext}"))
    }
}

fn streaming_total_hint(
    downloaded: u64,
    completed_segments: usize,
    total_segments: usize,
) -> Option<u64> {
    if completed_segments == 0 {
        return None;
    }
    if completed_segments >= total_segments {
        return Some(downloaded);
    }

    Some(downloaded.saturating_mul(total_segments as u64) / completed_segments as u64)
}

#[allow(clippy::too_many_lines)]
async fn run_single_stream(mut ctx: JobContext, first: Option<reqwest::Response>) -> Result<()> {
    // WebDAV schemes must hit http(s) endpoints.
    if crate::webdav::is_webdav_url(&ctx.record.url) {
        ctx.record.url = crate::webdav::effective_http_url(&ctx.record.url);
    }
    // The planning GET already is this request: reuse its stream (one-time URLs, no double hit).
    let response = match first {
        Some(first) => first,
        None => {
            let response = apply_request_headers(
                replay_request_builder(&ctx, &ctx.record.url),
                ctx.cookies.as_deref(),
                ctx.extra_headers.as_ref(),
            )
            .send()
            .await?;

            // Auth retry: if the server challenges us and the URL has embedded credentials, retry once.
            let response = if response.status().as_u16() == 401 {
                if let Some((user, pass)) = chunk::credentials_from_url(&ctx.record.url) {
                    let www_auth = response
                        .headers()
                        .get("WWW-Authenticate")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("Basic realm=\"\"")
                        .to_string();
                    let method = if ctx.method.eq_ignore_ascii_case("POST") {
                        "POST"
                    } else {
                        "GET"
                    };
                    if chunk::ntlm_auth::is_ntlm(&www_auth) && method == "GET" {
                        // NTLM needs its own Type 1/2/3 exchange on one connection.
                        chunk::auth_retry(
                            &ctx.client,
                            method,
                            &chunk::strip_userinfo(&ctx.record.url),
                            ctx.cookies.as_deref(),
                            ctx.extra_headers.as_ref(),
                            None,
                            &www_auth,
                            &user,
                            &pass,
                        )
                        .await?
                    } else {
                        let auth_value = chunk::build_auth_header(
                            &www_auth,
                            method,
                            &ctx.record.url,
                            &user,
                            &pass,
                        );
                        apply_request_headers(
                            replay_request_builder(&ctx, &ctx.record.url),
                            ctx.cookies.as_deref(),
                            ctx.extra_headers.as_ref(),
                        )
                        .header("Authorization", auth_value)
                        .send()
                        .await?
                    }
                } else {
                    response
                }
            } else {
                response
            };
            response
        }
    };

    let status = response.status().as_u16();
    if status == 403 || status == 410 {
        if security::is_signed_url_str(&ctx.record.url) {
            return Err(FerricError::SignedUrlExpired.into());
        }
        return Err(FerricError::Forbidden.into());
    }
    if status == 404 {
        return Err(FerricError::NotFound.into());
    }
    if status == 429 {
        save_rate_limit_rule(&ctx.pool, &ctx.record.url).await?;
        return Err(FerricError::RateLimited.into());
    }
    if !response.status().is_success() {
        return Err(FerricError::from_status(
            response.status().as_u16(),
            "download request failed",
        )
        .into());
    }

    let headers = response.headers();
    let final_url = response.url().to_string();
    let metadata = ResponseMetadata {
        www_authenticate: None,
        redirect_chain: chunk::redirect_chain_json(&ctx.record.url, &final_url),
        final_url: Some(final_url),
        filename: chunk::filename_from_content_disposition(
            headers
                .get(CONTENT_DISPOSITION)
                .and_then(|value| value.to_str().ok()),
        ),
        mime_type: headers
            .get(CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(|value| value.split(';').next().unwrap_or(value).trim().to_string())
            .filter(|value| !value.is_empty()),
    };
    if let Some(ref chain) = metadata.redirect_chain {
        diagnostics::event(
            "download_redirected",
            serde_json::json!({ "id": ctx.record.id, "chain": chain }),
        )
        .await;
    }
    let ctx = apply_response_metadata(ctx, &metadata).await?;

    let path = PathBuf::from(&ctx.record.save_path);
    // Single-stream always receives the full body from byte 0; truncate any
    // partial file so a resume cannot produce prefix+full-payload corruption.
    tokio::fs::File::create(&path)
        .await
        .with_context(|| format!("truncate {}", path.display()))?;
    let disk = DiskWriter::create(&path, None).await?;
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Downloading, None)
        .await?;

    let total = response.content_length();
    let mut stream = response.bytes_stream();
    let mut downloaded = 0u64; // always from 0; file truncated above
    let mut tracker = SpeedTracker::new(5);

    use futures::StreamExt as _;
    while let Some(next) = stream.next().await {
        if *ctx.cancel_rx.borrow() {
            return Err(FerricError::Cancelled.into());
        }

        let bytes = next?;
        ctx.bandwidth.acquire(bytes.len()).await;
        disk.write_at(downloaded, &bytes).await?;
        let len = u64::try_from(bytes.len()).context("body chunk length exceeds u64")?;
        downloaded = downloaded.saturating_add(len);
        tracker.record(len);
        db::downloads::update_progress(&ctx.pool, ctx.record.id, downloaded).await?;
        emit_progress(&ctx, downloaded, total, &tracker);
    }

    disk.sync().await?;
    let checksum_ok = verify_completion_checksum(&ctx, &path).await?;
    if checksum_ok == Some(false) {
        spawn_av_scan(path);
        return Ok(());
    }
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Completed, None).await?;
    emit_done(&ctx, checksum_ok);
    spawn_av_scan(path);
    Ok(())
}

/// Download a single URL to a file path, streaming all bytes through the
/// bandwidth limiter. Used to fetch the separate audio stream during a merge
/// download. Returns the number of bytes written.
/// Piece size for the parallel stream fetch. Google's video servers throttle a long single request
/// to a few MB/s but serve short ranged ones at full speed (yt-dlp's `http_chunk_size` is 10 MB).
const STREAM_PIECE: u64 = 4 * 1024 * 1024;

/// Ask the server for byte 0 only and read the full size from `Content-Range`: `Some(total)` when the
/// server really does ranged requests, `None` otherwise.
async fn probe_ranged_size(ctx: &JobContext, url: &str, post: bool) -> Option<u64> {
    let builder = if post {
        replay_request_builder(ctx, url)
    } else {
        ctx.client.get(url)
    };
    let response =
        apply_request_headers(builder, ctx.cookies.as_deref(), ctx.extra_headers.as_ref())
            .header(reqwest::header::RANGE, "bytes=0-0")
            .send()
            .await
            .ok()?;
    if response.status().as_u16() != 206 {
        return None;
    }
    response
        .headers()
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .rsplit('/')
        .next()?
        .parse::<u64>()
        .ok()
        .filter(|total| *total > 0)
}

/// One piece (`start..=end`) over a fresh ranged request, resumed from the last good byte after a
/// failure (expired links, resets, 5xx), up to a few tries.
async fn fetch_piece(
    ctx: &JobContext,
    url: &str,
    post: bool,
    disk: &DiskWriter,
    start: u64,
    end: u64,
    received: &std::sync::atomic::AtomicU64,
) -> Result<()> {
    use futures::StreamExt as _;
    let mut pos = start;
    let mut attempt = 0_u32;
    while pos <= end {
        if *ctx.cancel_rx.borrow() {
            return Err(FerricError::Cancelled.into());
        }
        let builder = if post {
            replay_request_builder(ctx, url)
        } else {
            ctx.client.get(url)
        };
        let outcome: Result<()> = async {
            let response =
                apply_request_headers(builder, ctx.cookies.as_deref(), ctx.extra_headers.as_ref())
                    .header(reqwest::header::RANGE, format!("bytes={pos}-{end}"))
                    .send()
                    .await?;
            let status = response.status().as_u16();
            if status != 206 {
                return Err(FerricError::from_status(status, "ranged request refused").into());
            }
            let mut body = response.bytes_stream();
            loop {
                let next = tokio::time::timeout(std::time::Duration::from_secs(30), body.next())
                    .await
                    .map_err(|_| anyhow::anyhow!("stream idle for 30 s"))?;
                let Some(frame) = next else { break };
                let mut bytes = frame?;
                let room = usize::try_from(end - pos + 1).unwrap_or(usize::MAX);
                if bytes.len() > room {
                    bytes.truncate(room);
                }
                ctx.bandwidth.acquire(bytes.len()).await;
                disk.write_at(pos, &bytes).await?;
                pos += bytes.len() as u64;
                received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
                if pos > end {
                    break;
                }
                if *ctx.cancel_rx.borrow() {
                    return Err(FerricError::Cancelled.into());
                }
            }
            Ok(())
        }
        .await;
        match outcome {
            Ok(()) if pos > end => return Ok(()),
            Ok(()) => attempt += 1, // the server closed early: ask for the rest
            Err(e)
                if matches!(
                    e.downcast_ref::<FerricError>(),
                    Some(FerricError::Cancelled)
                ) =>
            {
                return Err(e)
            }
            Err(e) => {
                attempt += 1;
                if attempt > 5 {
                    return Err(e);
                }
            }
        }
        if attempt > 5 {
            anyhow::bail!("stream ended early too many times");
        }
        tokio::time::sleep(std::time::Duration::from_millis(
            400 * u64::from(attempt.max(1)),
        ))
        .await;
    }
    Ok(())
}

/// Download one stream into `dest` with `connections` parallel ranged requests. Falls back to a plain
/// single request when the server does not do ranges. `received` counts the bytes as they arrive.
async fn fetch_stream(
    ctx: &JobContext,
    url: &str,
    post: bool,
    dest: &Path,
    total: Option<u64>,
    connections: usize,
    received: Arc<std::sync::atomic::AtomicU64>,
) -> Result<()> {
    let Some(total) = total.filter(|t| *t >= STREAM_PIECE) else {
        return single_stream_to_file(ctx, url, post, dest, &received).await;
    };
    let disk = DiskWriter::create(dest, Some(total)).await?;
    let pieces = total.div_ceil(STREAM_PIECE);
    let next = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let work = |_: usize| async {
        loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            let start = i * STREAM_PIECE;
            if start >= total {
                return Ok::<(), anyhow::Error>(());
            }
            let end = (start + STREAM_PIECE - 1).min(total - 1);
            fetch_piece(ctx, url, post, &disk, start, end, &received).await?;
        }
    };
    let workers = connections
        .min(usize::try_from(pieces).unwrap_or(usize::MAX))
        .max(1);
    // All workers run side by side on this task (they only wait on the network and the disk); the first
    // failure ends the others.
    futures::future::try_join_all((0..workers).map(work)).await?;
    disk.sync().await?;
    Ok(())
}

async fn single_stream_to_file(
    ctx: &JobContext,
    url: &str,
    post: bool,
    dest: &Path,
    received: &std::sync::atomic::AtomicU64,
) -> Result<()> {
    use futures::StreamExt as _;
    let builder = if post {
        replay_request_builder(ctx, url)
    } else {
        ctx.client.get(url)
    };
    let response =
        apply_request_headers(builder, ctx.cookies.as_deref(), ctx.extra_headers.as_ref())
            .send()
            .await
            .with_context(|| format!("GET {}", security::redact_url(url)))?;
    if !response.status().is_success() {
        return Err(
            FerricError::from_status(response.status().as_u16(), "stream request failed").into(),
        );
    }
    let mut file = tokio::fs::File::create(dest)
        .await
        .with_context(|| format!("create {}", dest.display()))?;
    let mut stream = response.bytes_stream();
    while let Some(next) = stream.next().await {
        if *ctx.cancel_rx.borrow() {
            return Err(FerricError::Cancelled.into());
        }
        let bytes = next?;
        ctx.bandwidth.acquire(bytes.len()).await;
        file.write_all(&bytes).await?;
        received.fetch_add(bytes.len() as u64, Ordering::Relaxed);
    }
    file.flush().await?;
    file.sync_all().await?;
    Ok(())
}

/// The video and audio halves of a merge download, fetched side by side over parallel ranged requests,
/// with one combined progress. Returns the total number of bytes when the sizes were known.
async fn fetch_merge_streams(
    ctx: &JobContext,
    audio_url: &str,
    video_tmp: &Path,
    audio_tmp: &Path,
) -> Result<Option<u64>> {
    let post = ctx.method.eq_ignore_ascii_case("POST");
    let (video_total, audio_total) = tokio::join!(
        probe_ranged_size(ctx, &ctx.record.url, post),
        probe_ranged_size(ctx, audio_url, false)
    );
    let grand_total = video_total.zip(audio_total).map(|(v, a)| v + a);
    let received = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));
    let reporter = {
        let (received, stop) = (received.clone(), stop.clone());
        let (pool, id, tx) = (ctx.pool.clone(), ctx.record.id, ctx.progress_tx.clone());
        tokio::spawn(async move {
            let mut tracker = SpeedTracker::new(5);
            let mut last = 0_u64;
            while !stop.load(Ordering::Relaxed) {
                tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                let now = received.load(Ordering::Relaxed);
                tracker.record(now.saturating_sub(last));
                last = now;
                let shown = grand_total.map_or(now, |t| now.min(t));
                let _ = db::downloads::update_progress(&pool, id, shown).await;
                let remaining = grand_total.map_or(0, |t| t.saturating_sub(shown));
                let _ = tx.send(ProgressEvent::Progress {
                    id,
                    downloaded: shown,
                    total: grand_total,
                    speed_bps: tracker.speed_bps(),
                    eta_secs: grand_total.and_then(|_| tracker.eta_secs(remaining)),
                    status: DownloadStatus::Downloading,
                });
            }
        })
    };
    let (video, audio) = tokio::join!(
        fetch_stream(
            ctx,
            &ctx.record.url,
            post,
            video_tmp,
            video_total,
            12,
            received.clone()
        ),
        fetch_stream(
            ctx,
            audio_url,
            false,
            audio_tmp,
            audio_total,
            4,
            received.clone()
        )
    );
    stop.store(true, Ordering::Relaxed);
    reporter.abort();
    let _ = reporter.await;
    video?;
    audio?;
    Ok(grand_total)
}

/// Merge download flow: download video and audio as separate temp files, then
/// combine them with ffmpeg. This handles the `FormatSelection::Merge` case
/// where a media extractor selected separate video-only and audio-only streams
/// (e.g. `YouTube` adaptive formats).
async fn run_merge(ctx: JobContext) -> Result<()> {
    let audio_url = ctx
        .record
        .audio_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("run_merge called without audio_url"))?
        .to_string();
    security::validate_url(&audio_url)?;

    let final_path = PathBuf::from(&ctx.record.save_path);
    let save_dir = final_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("save path has no parent directory"))?;

    let video_tmp = final_path.with_extension("tmp_video");
    let audio_tmp = final_path.with_extension("tmp_audio");

    diagnostics::event(
        "merge_start",
        serde_json::json!({
            "id": ctx.record.id,
            "video_url": security::redact_url(&ctx.record.url),
            "audio_url": security::redact_url(&audio_url),
        }),
    )
    .await;

    // ── 1. Download video stream ────────────────────────────────────────────
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Downloading, None)
        .await?;

    // Both streams at once, each over several ranged connections (was: video, then audio, one request each).
    let video_total = fetch_merge_streams(&ctx, &audio_url, &video_tmp, &audio_tmp)
        .await
        .inspect_err(|_| {
            let (v, a) = (video_tmp.clone(), audio_tmp.clone());
            tokio::spawn(async move {
                let _ = tokio::fs::remove_file(v).await;
                let _ = tokio::fs::remove_file(a).await;
            });
        })?;
    let downloaded = video_total.unwrap_or(0);

    if *ctx.cancel_rx.borrow() {
        let _ = tokio::fs::remove_file(&video_tmp).await;
        let _ = tokio::fs::remove_file(&audio_tmp).await;
        return Err(FerricError::Cancelled.into());
    }

    // ── 3. Merge with ffmpeg ────────────────────────────────────────────────
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Merging, None).await?;
    let _ = ctx.progress_tx.send(ProgressEvent::Progress {
        id: ctx.record.id,
        downloaded,
        total: video_total,
        speed_bps: 0,
        eta_secs: None,
        status: DownloadStatus::Merging,
    });

    let merge_result =
        crate::media::run_ffmpeg_merge(&video_tmp, &audio_tmp, &final_path, save_dir).await;

    // Clean up temp files regardless of outcome.
    let _ = tokio::fs::remove_file(&video_tmp).await;
    let _ = tokio::fs::remove_file(&audio_tmp).await;

    merge_result?;

    // ── 4. Done ─────────────────────────────────────────────────────────────
    let checksum_ok = verify_completion_checksum(&ctx, &final_path).await?;
    if checksum_ok == Some(false) {
        spawn_av_scan(final_path);
        return Ok(());
    }
    db::downloads::update_status(&ctx.pool, ctx.record.id, DownloadStatus::Completed, None).await?;
    emit_done(&ctx, checksum_ok);
    spawn_av_scan(final_path);
    Ok(())
}

async fn verify_completion_checksum(ctx: &JobContext, path: &Path) -> Result<Option<bool>> {
    let Some(expected) = ctx.record.checksum.as_deref() else {
        return Ok(None);
    };

    let ok = verify_checksum(path, expected).await?;
    db::downloads::set_checksum_ok(&ctx.pool, ctx.record.id, ok).await?;
    if !ok {
        let message = "checksum mismatch";
        db::downloads::update_status(
            &ctx.pool,
            ctx.record.id,
            DownloadStatus::Error,
            Some(message),
        )
        .await?;
        emit_failed(ctx, message);
    }
    Ok(Some(ok))
}

async fn verify_checksum(path: &Path, expected: &str) -> Result<bool> {
    use sha2::{Digest as _, Sha256};
    use tokio::io::AsyncReadExt as _;

    let (algorithm, expected_hex) = expected
        .split_once(':')
        .ok_or_else(|| anyhow::anyhow!("checksum must be prefixed with sha256:, sha1:, or md5:"))?;
    let expected_hex = expected_hex.trim().to_ascii_lowercase();

    let mut file = tokio::fs::File::open(path)
        .await
        .with_context(|| format!("open {} for checksum", path.display()))?;
    let mut buf = vec![0_u8; 64 * 1024];

    let actual_hex = match algorithm.to_ascii_lowercase().as_str() {
        "sha256" => {
            let mut hasher = Sha256::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            crate::chunk::hex_lower(hasher.finalize())
        }
        "sha1" => {
            use sha1::{Digest as _, Sha1};
            let mut hasher = Sha1::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            crate::chunk::hex_lower(hasher.finalize())
        }
        "md5" => {
            let mut hasher = md5::Context::new();
            loop {
                let n = file.read(&mut buf).await?;
                if n == 0 {
                    break;
                }
                hasher.consume(&buf[..n]);
            }
            format!("{:x}", hasher.compute())
        }
        other => {
            return Err(anyhow::anyhow!(
                "unsupported checksum algorithm: {other} (want sha256, sha1, or md5)"
            ))
        }
    };

    Ok(actual_hex == expected_hex)
}

fn emit_progress(ctx: &JobContext, downloaded: u64, total: Option<u64>, tracker: &SpeedTracker) {
    let remaining = total.map_or(0, |total| total.saturating_sub(downloaded));
    let _ = ctx.progress_tx.send(ProgressEvent::Progress {
        id: ctx.record.id,
        downloaded,
        total,
        speed_bps: tracker.speed_bps(),
        eta_secs: total.and_then(|_| tracker.eta_secs(remaining)),
        status: DownloadStatus::Downloading,
    });
}

/// IDM's Save-to option "Set file creation date as provided by the server" (off by default: IDM
/// stamps the download time, measured): give the file the server's Last-Modified time.
fn spawn_apply_server_file_time(pool: SqlitePool, id: Uuid, path: String) {
    tokio::spawn(async move {
        db::downloads::set_total_if_missing(&pool, id).await.ok();
        if !db::settings::get_flag(&pool, "file_time_from_server", false).await {
            return;
        }
        let Ok(Some(rec)) = db::downloads::get(&pool, id).await else {
            return;
        };
        let Some(modified) = rec.last_modified.as_deref().and_then(parse_http_date) else {
            return;
        };
        let applied = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .and_then(|f| f.set_modified(modified));
        if let Err(e) = applied {
            tracing::warn!(%id, error = %e, "could not set the file time from the server");
        }
    });
}

fn parse_http_date(text: &str) -> Option<std::time::SystemTime> {
    let t = chrono::DateTime::parse_from_rfc2822(text.trim()).ok()?;
    u64::try_from(t.timestamp())
        .ok()
        .map(|s| std::time::UNIX_EPOCH + std::time::Duration::from_secs(s))
}

fn emit_done(ctx: &JobContext, checksum_ok: Option<bool>) {
    let _ = ctx.progress_tx.send(ProgressEvent::Done {
        id: ctx.record.id,
        path: ctx.record.save_path.clone(),
        checksum_ok,
    });
    spawn_extract_sidecars(
        ctx.pool.clone(),
        ctx.record.id,
        ctx.record.save_path.clone(),
    );
    spawn_apply_server_file_time(
        ctx.pool.clone(),
        ctx.record.id,
        ctx.record.save_path.clone(),
    );
    // Task 123 — completion audit (no cookies)
    let pool_a = ctx.pool.clone();
    let id_a = ctx.record.id;
    let path_a = ctx.record.save_path.clone();
    tokio::spawn(async move {
        let _ = db::audit::append(
            &pool_a,
            id_a,
            "completed",
            &serde_json::json!({
                "path": path_a,
                "checksum_ok": checksum_ok,
            }),
        )
        .await;
        crate::webhooks::emit_download_done(&pool_a, id_a, &path_a).await;
    });
    // Task 130 — purge encrypted cookies on complete when enabled
    let pool_c = ctx.pool.clone();
    let id_c = ctx.record.id;
    tokio::spawn(async move {
        let purge = db::settings::get(&pool_c, "cookie_purge_on_complete")
            .await
            .ok()
            .flatten()
            .is_none_or(|v| v != "0" && !v.eq_ignore_ascii_case("false"));
        if purge {
            if let Err(e) = db::cookies::delete(&pool_c, id_c).await {
                tracing::debug!(error = %e, id = %id_c, "cookie purge skipped");
            }
        }
    });
}

/// Pure-Rust post-download: write info.json / description / chapters when meta was stored.
fn spawn_extract_sidecars(pool: sqlx::SqlitePool, id: uuid::Uuid, save_path: String) {
    tokio::spawn(async move {
        let meta = match db::extract_meta::get(&pool, id).await {
            Ok(Some(m)) => m,
            Ok(None) => return,
            Err(e) => {
                tracing::debug!(error = %e, %id, "extract meta load skipped");
                return;
            }
        };
        let path = std::path::PathBuf::from(&save_path);
        if !path.is_file() {
            return;
        }
        async fn setting_on(pool: &sqlx::SqlitePool, key: &str) -> bool {
            db::settings::get(pool, key)
                .await
                .ok()
                .flatten()
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        }
        let write_info = setting_on(&pool, "sidecar_info_json").await;
        let write_desc = setting_on(&pool, "sidecar_description").await;
        let write_chapters = setting_on(&pool, "sidecar_chapters").await;
        let embed_thumb = setting_on(&pool, "sidecar_embed_thumb").await;
        if !write_info && !write_desc && !write_chapters && !embed_thumb {
            return;
        }
        let client = match reqwest::Client::builder().user_agent("Mozilla/5.0").build() {
            Ok(c) => c,
            Err(_) => return,
        };
        if let Err(e) = crate::postdownload::apply_extract_sidecars(
            &path,
            &meta,
            &client,
            write_info,
            write_desc,
            write_chapters,
            embed_thumb,
        )
        .await
        {
            tracing::warn!(error = %e, %id, "extract sidecars failed");
        } else {
            tracing::info!(%id, "wrote extract sidecars");
            let _ = db::extract_meta::delete(&pool, id).await;
        }
    });
}

fn emit_failed(ctx: &JobContext, message: &str) {
    let _ = ctx.progress_tx.send(ProgressEvent::Failed {
        id: ctx.record.id,
        message: message.to_string(),
    });
}

async fn download_chunk_with_retry(mut args: ChunkArgs) -> Result<u64> {
    let mut attempt = 0_u32;
    let mut max_attempts = retry::max_attempts();

    loop {
        let result = if is_ftp_family(&args.url) {
            crate::ftp::download_ftp_chunk(args.clone()).await
        } else if args.url.starts_with("sftp://") {
            crate::sftp::download_sftp_chunk(args.clone()).await
        } else {
            // webdav:// already rewritten to http(s) in run_segmented sources
            chunk::download_chunk(args.clone()).await
        };

        match result {
            Ok(written) => break Ok(written),
            Err(error) => {
                let retryable = error
                    .downcast_ref::<FerricError>()
                    .is_some_and(FerricError::is_retryable)
                    || is_body_read_retryable(&error);
                if !retryable || is_rate_limited(&error) || is_range_rejected(&error) {
                    diagnostics::event(
                        "chunk_error_terminal",
                        serde_json::json!({
                            "download_id": args.download_id,
                            "chunk": args.chunk.chunk_index,
                            "attempt": attempt,
                            "error": error.to_string(),
                        }),
                    )
                    .await;
                    break Err(error);
                }
                // IDM keeps "number of retries for each file" per queue.
                if let Some(n) =
                    db::queues::max_retries_for_download(&args.pool, args.download_id).await
                {
                    max_attempts = n;
                }
                if attempt >= max_attempts {
                    diagnostics::event(
                        "chunk_retry_exhausted",
                        serde_json::json!({
                            "download_id": args.download_id,
                            "chunk": args.chunk.chunk_index,
                            "attempt": attempt,
                            "error": error.to_string(),
                        }),
                    )
                    .await;
                    break Err(error);
                }
                let delay = if cfg!(test) {
                    std::time::Duration::from_millis(30)
                } else {
                    retry::backoff_delay(attempt, 2, 30)
                };
                diagnostics::event(
                    "chunk_retry",
                    serde_json::json!({
                        "download_id": args.download_id,
                        "chunk": args.chunk.chunk_index,
                        "attempt": attempt,
                        "delay_ms": delay.as_millis(),
                        "error": error.to_string(),
                    }),
                )
                .await;
                tracing::warn!(
                    attempt,
                    chunk = args.chunk.chunk_index,
                    delay_ms = delay.as_millis(),
                    "chunk error, retrying"
                );
                // Keep what this attempt already wrote: the retry resumes from there instead of
                // fetching the whole piece again (IDM reconnects from the byte where it broke).
                if let Some(live) = &args.live {
                    let done = live.done();
                    if done > args.chunk.downloaded {
                        args.chunk.downloaded = done;
                        db::chunks::update_chunk_progress(
                            &args.pool,
                            args.chunk.id,
                            done,
                            ferric_common::ChunkStatus::Downloading,
                        )
                        .await
                        .ok();
                    }
                }
                tokio::time::sleep(delay).await;
                attempt += 1;
            }
        }
    }
}

fn is_body_read_retryable(error: &anyhow::Error) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("error reading a body")
        || message.contains("connection reset")
        || message.contains("connection closed")
        || message.contains("unexpected eof")
        || message.contains("incomplete message")
}

fn apply_request_headers(
    mut builder: reqwest::RequestBuilder,
    cookies: Option<&str>,
    extra_headers: Option<&HashMap<String, String>>,
) -> reqwest::RequestBuilder {
    if let Some(cookie) = cookies {
        builder = builder.header(COOKIE, cookie);
    }
    // Browser-like UA when the caller didn't set one (the same stable value the segment downloader uses)
    let has_ua =
        extra_headers.is_some_and(|h| h.keys().any(|k| k.eq_ignore_ascii_case("user-agent")));
    if !has_ua {
        builder = builder.header(
            reqwest::header::USER_AGENT,
            crate::chunk::default_user_agent(),
        );
    }
    if let Some(headers) = extra_headers {
        for (name, value) in headers {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(value),
            ) {
                builder = builder.header(name, value);
            }
        }
    }
    builder
}

fn replay_request_builder(ctx: &JobContext, url: &str) -> reqwest::RequestBuilder {
    if ctx.method.eq_ignore_ascii_case("POST") {
        ctx.client
            .post(url)
            .body(ctx.post_body.clone().unwrap_or_default())
    } else {
        ctx.client.get(url)
    }
}

#[cfg(not(test))]
fn spawn_av_scan(path: PathBuf) {
    use crate::av;

    tokio::spawn(async move {
        av::scan(&path).await;
    });
}

#[cfg(test)]
fn spawn_av_scan(_path: PathBuf) {}

fn is_ftp_family(url: &str) -> bool {
    url.starts_with("ftp://") || url.starts_with("ftps://") || url.starts_with("ftps-implicit://")
}

fn is_rate_limited(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<FerricError>(),
        Some(FerricError::RateLimited)
    )
}

/// An HTML page (a verification or login page, an expired-link notice) is not the file the link was meant to
/// give. Judged from the type the server declared, the name the file is saved under, and the size when known:
/// a real archive, video or installer is never a few hundred bytes of HTML.
fn looks_like_web_page_instead_of_file(
    mime: Option<&str>,
    filename: &str,
    size: Option<u64>,
) -> bool {
    let is_html = mime.is_some_and(|m| {
        let m = m
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        m == "text/html" || m == "application/xhtml+xml"
    });
    if !is_html {
        return false;
    }
    let ext = Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    // names that really are pages, scripts, or have no extension at all
    if matches!(
        ext.as_str(),
        "" | "html" | "htm" | "xhtml" | "shtml" | "mhtml" | "php" | "asp" | "aspx" | "jsp" | "cgi"
    ) {
        return false;
    }
    size.is_some_and(|s| s < 1024 * 1024)
}

/// Why a previous run's bytes cannot belong to the file the server serves now (worded for the user), or `None`.
fn remote_file_change(
    record: &ferric_common::DownloadRecord,
    size: u64,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Option<&'static str> {
    if record.total_size.is_some_and(|old| old != size) {
        return Some("The file size has been changed on the server.");
    }
    match (record.etag.as_deref(), etag) {
        (Some(old), Some(new)) if old != new => Some("The file has been changed on the server."),
        (Some(_), Some(_)) => None,
        _ => match (record.last_modified.as_deref(), last_modified) {
            (Some(old), Some(new)) if old != new => {
                Some("The file last modified date has been changed on the server.")
            }
            _ => None,
        },
    }
}

fn is_range_rejected(error: &anyhow::Error) -> bool {
    matches!(
        error.downcast_ref::<FerricError>(),
        Some(FerricError::RangeRejected)
    )
}

async fn save_rate_limit_rule(pool: &SqlitePool, url: &str) -> Result<()> {
    db::site_rules::learn_max_connections(pool, url, 1, db::site_rules::LEARNED_RATE_LIMIT_PRIORITY)
        .await
}

async fn effective_max_connections_for_url(pool: &SqlitePool, url: &str) -> Result<usize> {
    let configured = db::settings::get(pool, "max_connections_per_host")
        .await?
        .and_then(|value| value.trim().parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(8);
    let policy = db::site_rules::policy_for_url(pool, url).await?;
    Ok(effective_connection_cap(configured, policy.max_connections))
}

fn effective_connection_cap(configured: usize, rule_limit: Option<usize>) -> usize {
    let configured = configured.max(1);
    rule_limit
        .filter(|limit| *limit > 0)
        .map_or(configured, |limit| configured.min(limit))
}

fn is_hls(url: &str, mime: Option<&str>) -> bool {
    let path = url::Url::parse(url).ok().map_or_else(
        || url.to_ascii_lowercase(),
        |url| url.path().to_ascii_lowercase(),
    );
    let mime = mime.unwrap_or_default().to_ascii_lowercase();

    has_extension(&path, "m3u8")
        || matches!(
            mime.as_str(),
            "application/vnd.apple.mpegurl"
                | "application/x-mpegurl"
                | "audio/mpegurl"
                | "audio/x-mpegurl"
        )
}

fn is_dash(url: &str, mime: Option<&str>) -> bool {
    let path = url::Url::parse(url).ok().map_or_else(
        || url.to_ascii_lowercase(),
        |url| url.path().to_ascii_lowercase(),
    );
    let mime = mime.unwrap_or_default().to_ascii_lowercase();

    has_extension(&path, "mpd") || mime == "application/dash+xml"
}

fn has_extension(path: &str, expected: &str) -> bool {
    Path::new(path)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case(expected))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::split_into_chunks;

    #[test]
    fn a_stopped_controller_tries_again_only_after_its_pause_and_never_after_a_refusal() {
        let t0 = tokio::time::Instant::now();
        let mut plan = AdaptivePlan {
            ceiling: 24,
            next_step_at: t0,
            pending: None,
            done: true,
            recheck_at: Some(t0 + ADAPTIVE_RECHECK),
        };
        assert!(
            !plan.recheck_due(t0 + ADAPTIVE_RECHECK / 2, 18),
            "too early"
        );
        assert!(plan.recheck_due(t0 + ADAPTIVE_RECHECK, 18), "pause is over");
        assert!(
            !plan.recheck_due(t0 + ADAPTIVE_RECHECK, 24),
            "already at the ceiling"
        );
        plan.recheck_at = None; // a refused connection clears it
        assert!(!plan.recheck_due(t0 + ADAPTIVE_RECHECK * 2, 18));
    }

    #[test]
    fn the_smallest_splittable_piece_grows_with_the_file() {
        assert_eq!(min_steal(1 << 20), chunk::PIECE_GRAIN); // small files keep the fine grain
        assert_eq!(min_steal(1 << 30), 512 * 1024);
        assert_eq!(min_steal(95_426_807_152), 8 << 20); // the 95 GB game: capped at 8 MiB
    }

    #[test]
    fn a_small_html_reply_for_a_file_name_is_a_web_page_not_the_file() {
        let page = Some("text/html; charset=utf-8");
        // a verification page saved as an ISO: flagged
        assert!(looks_like_web_page_instead_of_file(
            page,
            "fedora.iso",
            Some(365)
        ));
        // real pages, extensionless names, big bodies and non-HTML types: left alone
        assert!(!looks_like_web_page_instead_of_file(
            page,
            "index.html",
            Some(365)
        ));
        assert!(!looks_like_web_page_instead_of_file(
            page,
            "download",
            Some(365)
        ));
        assert!(!looks_like_web_page_instead_of_file(
            page,
            "fedora.iso",
            Some(5 << 20)
        ));
        assert!(!looks_like_web_page_instead_of_file(
            Some("application/octet-stream"),
            "fedora.iso",
            Some(365)
        ));
        assert!(!looks_like_web_page_instead_of_file(
            None,
            "fedora.iso",
            Some(365)
        ));
    }
    use ferric_common::messages::DownloadRequest;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use uuid::Uuid;

    #[tokio::test]
    async fn segmented_job_downloads_file_to_completion() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("payload.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let save_path = dir.path().join("payload.bin");
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        let written = tokio::fs::read(&save_path).await?;
        assert_eq!(written, *data);
        let record = db::downloads::get(&pool, id).await?.expect("record exists");
        assert_eq!(record.status, DownloadStatus::Completed);
        assert_eq!(record.downloaded, data.len() as u64);
        assert!(db::chunks::all_chunks_complete(&pool, id).await?);

        Ok(())
    }

    #[tokio::test]
    async fn merge_streams_are_fetched_in_pieces_and_match_the_source() -> Result<()> {
        let video = Arc::new((0..9_500_000).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let audio = Arc::new((0..4_300_000).map(|i| (i % 241) as u8).collect::<Vec<_>>());
        let (vs, au) = (
            spawn_range_server(video.clone()).await?,
            spawn_range_server(audio.clone()).await?,
        );
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let req = DownloadRequest {
            id: Uuid::new_v4(),
            url: vs.url.clone(),
            final_url: None,
            filename: Some("v.mp4".to_string()),
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
        };
        let record = db::downloads::insert(
            &pool,
            &req,
            &dir.path().join("v.mp4").to_string_lossy(),
            "Other",
        )
        .await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let ctx = JobContext {
            pool,
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
        let (vt, at) = (dir.path().join("v.tmp"), dir.path().join("a.tmp"));
        let total = fetch_merge_streams(&ctx, &au.url, &vt, &at).await?;
        assert_eq!(total, Some((video.len() + audio.len()) as u64));
        assert_eq!(tokio::fs::read(&vt).await?, *video);
        assert_eq!(tokio::fs::read(&at).await?, *audio);
        Ok(())
    }

    /// A SOCKS5 proxy that refuses the login: the failure text must say why, not just "error sending request".
    #[tokio::test]
    async fn refused_socks_login_is_named_in_the_error() -> Result<()> {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let port = listener.local_addr()?.port();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = listener.accept().await {
                let mut buf = [0_u8; 262];
                let _ = c.read(&mut buf).await;
                let _ = c.write_all(&[5, 2]).await; // user/password method
                let _ = c.read(&mut buf).await;
                let _ = c.write_all(&[1, 1]).await; // login refused
            }
        });
        let client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(format!(
                "socks5h://u:wrong@127.0.0.1:{port}"
            ))?)
            .build()?;
        let err = client
            .get("http://example.test/f")
            .send()
            .await
            .unwrap_err();
        let text = format!("{:#}", anyhow::Error::from(err));
        assert!(
            text.len() > "error sending request for url".len() + 40,
            "{text}"
        );
        println!("{text}");
        Ok(())
    }

    #[tokio::test]
    async fn checksum_sha256_passes_on_correct_hash() -> Result<()> {
        use sha2::{Digest as _, Sha256};

        let data = Arc::new(b"known sha256 payload".to_vec());
        let checksum = format!("sha256:{}", crate::chunk::hex_lower(Sha256::digest(data.as_slice())));
        let (record, events, _dir) = run_no_range_checksum_job(data, Some(checksum)).await?;

        assert_eq!(record.status, DownloadStatus::Completed);
        assert_eq!(record.checksum_ok, Some(true));
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::Done {
                checksum_ok: Some(true),
                ..
            }
        )));
        Ok(())
    }

    #[tokio::test]
    async fn checksum_sha256_fails_on_wrong_hash() -> Result<()> {
        let data = Arc::new(b"known wrong-check payload".to_vec());
        let checksum = format!("sha256:{}", "0".repeat(64));
        let (record, events, _dir) = run_no_range_checksum_job(data, Some(checksum)).await?;

        assert_eq!(record.status, DownloadStatus::Error);
        assert_eq!(record.checksum_ok, Some(false));
        assert_eq!(record.error_msg.as_deref(), Some("checksum mismatch"));
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::Failed { message, .. } if message == "checksum mismatch"
        )));
        Ok(())
    }

    #[tokio::test]
    async fn checksum_md5_passes_on_correct_hash() -> Result<()> {
        let data = Arc::new(b"known md5 payload".to_vec());
        let checksum = format!("md5:{:x}", md5::compute(data.as_slice()));
        let (record, _events, _dir) = run_no_range_checksum_job(data, Some(checksum)).await?;

        assert_eq!(record.status, DownloadStatus::Completed);
        assert_eq!(record.checksum_ok, Some(true));
        Ok(())
    }

    #[tokio::test]
    async fn checksum_unknown_prefix_errors() -> Result<()> {
        let data = Arc::new(b"known unsupported checksum payload".to_vec());
        let server = spawn_no_range_server(data).await?;
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("unsupported.bin");
        let req = checksum_req(
            id,
            &server.url,
            "unsupported.bin",
            Some("crc32:abc".to_string()),
        );
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let err = run(JobContext {
            pool,
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
        })
        .await
        .expect_err("unsupported checksum prefix must error");

        assert!(err
            .to_string()
            .contains("unsupported checksum algorithm: crc32"));
        Ok(())
    }

    #[tokio::test]
    async fn checksum_skipped_when_none() -> Result<()> {
        let data = Arc::new(b"known unchecked payload".to_vec());
        let (record, events, _dir) = run_no_range_checksum_job(data, None).await?;

        assert_eq!(record.status, DownloadStatus::Completed);
        assert_eq!(record.checksum_ok, None);
        assert!(events.iter().any(|event| matches!(
            event,
            ProgressEvent::Done {
                checksum_ok: None,
                ..
            }
        )));
        Ok(())
    }

    #[tokio::test]
    async fn hls_segments_downloaded_in_order() -> Result<()> {
        let server = spawn_hls_server(
            "/playlist.m3u8",
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:1.0,\n/s1.ts\n#EXTINF:1.0,\n/s2.ts\n#EXTINF:1.0,\n/s3.ts\n#EXT-X-ENDLIST\n",
            vec![
                ("/s1.ts", b"first-".to_vec()),
                ("/s2.ts", b"second-".to_vec()),
                ("/s3.ts", b"third".to_vec()),
            ],
        )
        .await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("stream.ts");
        let req = checksum_req(id, &server.manifest_url, "stream.ts", None);
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, b"first-second-third");
        let record = db::downloads::get(&pool, id).await?.expect("record exists");
        assert_eq!(record.status, DownloadStatus::Completed);
        let requests = server.requests.lock().await.clone();
        assert_eq!(
            requests,
            vec!["/playlist.m3u8", "/s1.ts", "/s2.ts", "/s3.ts"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn hls_relative_urls_resolved_correctly() -> Result<()> {
        let server = spawn_hls_server(
            "/streams/live/playlist.m3u8",
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXTINF:1.0,\nseg-1.ts\n#EXTINF:1.0,\n../shared/seg-2.ts\n#EXT-X-ENDLIST\n",
            vec![
                ("/streams/live/seg-1.ts", b"alpha".to_vec()),
                ("/streams/shared/seg-2.ts", b"beta".to_vec()),
            ],
        )
        .await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("relative.ts");
        let req = checksum_req(id, &server.manifest_url, "relative.ts", None);
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, b"alphabeta");
        let requests = server.requests.lock().await.clone();
        assert_eq!(
            requests,
            vec![
                "/streams/live/playlist.m3u8",
                "/streams/live/seg-1.ts",
                "/streams/shared/seg-2.ts",
            ]
        );
        Ok(())
    }

    #[tokio::test]
    async fn max_connections_setting_limits_segmented_workers() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 251) as u8).collect::<Vec<_>>());
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::settings::set(&pool, "max_connections_per_host", "1").await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("capped.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let save_path = dir.path().join("capped.bin");
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        assert!(
            server
                .peak_concurrency
                .load(std::sync::atomic::Ordering::SeqCst)
                <= 1,
            "max_connections=1 must serialize range requests"
        );
        Ok(())
    }

    #[tokio::test]
    async fn workers_steal_more_pieces_than_their_count() -> Result<()> {
        // 5 MiB capped at 2 connections: 2 initial pieces, the rest is dynamic halving.
        let data = Arc::new(
            (0..5 * 1_048_576)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::settings::set(&pool, "max_connections_per_host", "2").await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("steal.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("steal.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        // Piece 0 rides on the planning GET; piece 1 needs one ranged request, halving may add more.
        assert!(!server.ranges.lock().await.is_empty());
        assert!(
            server
                .peak_concurrency
                .load(std::sync::atomic::Ordering::SeqCst)
                <= 2,
            "concurrency must stay within the 2-connection cap while stealing pieces"
        );
        Ok(())
    }

    #[tokio::test]
    async fn site_rule_limits_future_segmented_workers() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 241) as u8).collect::<Vec<_>>());
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::site_rules::learn_max_connections(
            &pool,
            &server.url,
            1,
            db::site_rules::LEARNED_RATE_LIMIT_PRIORITY,
        )
        .await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: server.url,
            final_url: None,
            filename: Some("site-rule.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let save_path = dir.path().join("site-rule.bin");
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        assert!(
            server
                .peak_concurrency
                .load(std::sync::atomic::Ordering::SeqCst)
                <= 1,
            "site rule max_connections=1 must serialize range requests"
        );
        Ok(())
    }

    #[tokio::test]
    async fn successful_segmented_download_does_not_cap_the_host() -> Result<()> {
        let data = Arc::new(
            (0..6 * 1_048_576)
                .map(|i| (i % 239) as u8)
                .collect::<Vec<_>>(),
        );
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("learn-success.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("learn-success.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        // 6 MiB starts as 4 pieces; piece 0 rides on the planning GET, so at least 3 ranged
        // requests follow (halving of in-flight pieces may add more).
        assert!(server.ranges.lock().await.len() >= 3);
        // "It worked with 4" is not a limit: remembering it capped every later download from the host at
        // the starting 4 and stopped the ramp to 8.
        let policy = db::site_rules::policy_for_url(&pool, &server.url).await?;
        assert_eq!(
            policy.max_connections, None,
            "a successful download must not cap the host"
        );
        Ok(())
    }

    #[tokio::test]
    async fn success_rule_does_not_override_stricter_429_rule() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 233) as u8).collect::<Vec<_>>());
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::site_rules::learn_max_connections(
            &pool,
            &server.url,
            1,
            db::site_rules::LEARNED_RATE_LIMIT_PRIORITY,
        )
        .await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("strict-rule.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("strict-rule.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        assert!(
            server
                .peak_concurrency
                .load(std::sync::atomic::Ordering::SeqCst)
                <= 1,
            "priority-100 cap should limit future downloads before success learning runs"
        );
        let policy = db::site_rules::policy_for_url(&pool, &server.url).await?;
        assert_eq!(policy.max_connections, Some(1));
        let low_priority_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM site_rules WHERE rule_type = 'max_connections' AND priority = ?1",
        )
        .bind(db::site_rules::LEARNED_SUCCESS_PRIORITY)
        .fetch_one(&pool)
        .await?;
        assert_eq!(low_priority_count, 0);
        Ok(())
    }

    #[tokio::test]
    async fn cancelled_job_does_not_overwrite_manager_status() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
            url: "http://127.0.0.1:9/payload.bin".to_string(),
            final_url: None,
            filename: Some("payload.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let save_path = dir.path().join("payload.bin");
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        db::downloads::update_status(&pool, id, DownloadStatus::Cancelled, None).await?;

        let (progress_tx, _) = broadcast::channel(16);
        let (cancel_tx, cancel_rx) = watch::channel(false);
        cancel_tx.send(true)?;

        let result = run(JobContext {
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
        })
        .await;

        assert!(matches!(
            result
                .expect_err("cancelled job should exit with Cancelled")
                .downcast_ref::<FerricError>(),
            Some(FerricError::Cancelled)
        ));
        let record = db::downloads::get(&pool, id).await?.expect("record exists");
        assert_eq!(record.status, DownloadStatus::Cancelled);

        Ok(())
    }

    #[tokio::test]
    async fn segmented_job_resumes_only_incomplete_chunks() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 199) as u8).collect::<Vec<_>>());
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("resume.bin");
        let req = DownloadRequest {
            id,
            url: server.url,
            final_url: None,
            filename: Some("resume.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let ranges = split_into_chunks(data.len() as u64, 4);
        db::chunks::insert_chunks(&pool, id, &ranges).await?;
        let disk = DiskWriter::create(&save_path, Some(data.len() as u64)).await?;
        for range in ranges.iter().take(3) {
            let body = &data[range.byte_start as usize..=range.byte_end as usize];
            disk.write_at(range.byte_start, body).await?;
        }
        disk.sync().await?;
        let records = db::chunks::get_chunks(&pool, id).await?;
        for chunk in records.iter().take(3) {
            db::chunks::update_chunk_progress(
                &pool,
                chunk.id,
                chunk.byte_end - chunk.byte_start + 1,
                ferric_common::ChunkStatus::Completed,
            )
            .await?;
        }

        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        run(JobContext {
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
        })
        .await?;

        // Only the incomplete piece may be fetched; work-stealing is allowed to
        // split it, so check that the requests tile exactly that piece.
        let mut requested = server.ranges.lock().await.clone();
        requested.sort_unstable();
        assert_eq!(
            requested.first().map(|r| r.0),
            Some(ranges[3].byte_start as usize)
        );
        assert_eq!(
            requested.last().map(|r| r.1),
            Some(ranges[3].byte_end as usize)
        );
        assert!(
            requested.windows(2).all(|w| w[0].1 + 1 == w[1].0),
            "requests must tile the incomplete piece without gaps or overlap: {requested:?}"
        );
        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        Ok(())
    }

    #[tokio::test]
    async fn rate_limit_saves_site_rule_and_finishes_sequentially() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 173) as u8).collect::<Vec<_>>());
        let server = spawn_one_rate_limit_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("rate.bin");
        let req = DownloadRequest {
            id,
            url: server.url,
            final_url: None,
            filename: Some("rate.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM site_rules WHERE rule_type = 'max_connections' AND value = '1'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(count, 1);
        Ok(())
    }

    #[tokio::test]
    async fn cookies_are_forwarded_to_head_and_chunk_requests() -> Result<()> {
        let data = Arc::new((0..1_100_000).map(|i| (i % 157) as u8).collect::<Vec<_>>());
        let server = spawn_cookie_required_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("cookie.bin");
        let req = DownloadRequest {
            id,
            url: server.url,
            final_url: None,
            filename: Some("cookie.bin".to_string()),
            referrer: None,
            cookies: Some("session=abc".to_string()),
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
            record,
            client: reqwest::Client::new(),
            progress_tx,
            bandwidth: Arc::new(BandwidthLimiter::new(0)),
            cancel_rx,
            cookies: req.cookies,
            extra_headers: None,
            method: "GET".to_string(),
            proxy_client: None,
            post_body: None,
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        Ok(())
    }

    #[tokio::test]
    async fn remote_changes_are_named_like_idm() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let req = DownloadRequest {
            id: Uuid::new_v4(),
            url: "http://example.test/a".into(),
            final_url: None,
            filename: Some("a.bin".into()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".into(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let mut record = db::downloads::insert(&pool, &req, "a.bin", "Other").await?;
        record.total_size = Some(100);
        record.etag = Some("\"a\"".into());
        record.last_modified = Some("Mon".into());
        assert_eq!(
            remote_file_change(&record, 100, Some("\"a\""), Some("Mon")),
            None
        );
        assert_eq!(
            remote_file_change(&record, 101, Some("\"a\""), Some("Mon")),
            Some("The file size has been changed on the server.")
        );
        assert_eq!(
            remote_file_change(&record, 100, Some("\"b\""), Some("Mon")),
            Some("The file has been changed on the server.")
        );
        record.etag = None;
        assert_eq!(
            remote_file_change(&record, 100, None, Some("Tue")),
            Some("The file last modified date has been changed on the server.")
        );
        assert_eq!(remote_file_change(&record, 100, None, None), None);
        Ok(())
    }

    #[tokio::test]
    async fn single_stream_resume_does_not_duplicate_bytes() -> Result<()> {
        let data = Arc::new((0..50_000).map(|i| (i % 211) as u8).collect::<Vec<_>>());
        let server = spawn_no_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("resume-single.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("resume-single.bin".to_string()),
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
        };
        // Simulate a previous partial download: write first half and mark downloaded.
        tokio::fs::write(&save_path, &data[..data.len() / 2]).await?;
        let mut record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        record.downloaded = (data.len() / 2) as u64;
        db::downloads::update_progress(&pool, id, record.downloaded).await?;

        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        run(JobContext {
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
        })
        .await?;

        let written = tokio::fs::read(&save_path).await?;
        assert_eq!(
            written, *data,
            "single-stream resume must produce exactly the source payload, not prefix+full"
        );
        let rec = db::downloads::get(&pool, id).await?.unwrap();
        assert_eq!(rec.status, DownloadStatus::Completed);
        Ok(())
    }

    #[tokio::test]
    async fn post_body_is_replayed_for_single_stream_download() -> Result<()> {
        let server = spawn_post_echo_server().await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("post.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("post.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: Some(HashMap::from([(
                "content-type".to_string(),
                "application/x-www-form-urlencoded".to_string(),
            )])),
            size_hint: None,
            mime_type: Some("application/octet-stream".to_string()),
            method: "POST".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
            pool,
            record,
            client: reqwest::Client::new(),
            progress_tx,
            bandwidth: Arc::new(BandwidthLimiter::new(0)),
            cancel_rx,
            cookies: None,
            extra_headers: req.headers,
            method: "POST".to_string(),
            proxy_client: None,
            post_body: Some(b"token=abc&download=1".to_vec()),
        })
        .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, b"post-ok");
        let received = server.body.lock().await.clone();
        assert_eq!(received, b"token=abc&download=1");
        Ok(())
    }

    #[tokio::test]
    async fn single_stream_429_learns_site_rule() -> Result<()> {
        let server = spawn_single_stream_429_server().await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("ss429.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("ss429.bin".to_string()),
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
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let result = run(JobContext {
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
        })
        .await;

        assert!(
            matches!(
                result
                    .expect_err("429 must fail the job")
                    .downcast_ref::<FerricError>(),
                Some(FerricError::RateLimited)
            ),
            "expected RateLimited error"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM site_rules WHERE rule_type = 'max_connections' AND value = '1'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(count, 1, "single-stream 429 must save a learned site rule");
        Ok(())
    }

    #[tokio::test]
    async fn head_429_learns_site_rule() -> Result<()> {
        let server = spawn_head_429_server().await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("head429.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("head429.bin".to_string()),
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
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);
        let result = run(JobContext {
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
        })
        .await;

        assert!(
            matches!(
                result
                    .expect_err("429 on the first request must fail the job")
                    .downcast_ref::<FerricError>(),
                Some(FerricError::RateLimited)
            ),
            "expected RateLimited error"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM site_rules WHERE rule_type = 'max_connections' AND value = '1'",
        )
        .fetch_one(&pool)
        .await?;
        assert_eq!(
            count, 1,
            "429 on the first request must save a learned site rule"
        );
        Ok(())
    }

    async fn run_no_range_checksum_job(
        data: Arc<Vec<u8>>,
        checksum: Option<String>,
    ) -> Result<(DownloadRecord, Vec<ProgressEvent>, tempfile::TempDir)> {
        let server = spawn_no_range_server(data).await?;
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("checksum.bin");
        let req = checksum_req(id, &server.url, "checksum.bin", checksum);
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, mut progress_rx) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        let mut events = Vec::new();
        while let Ok(event) = progress_rx.try_recv() {
            events.push(event);
        }

        let record = db::downloads::get(&pool, id).await?.expect("record exists");
        Ok((record, events, dir))
    }

    fn checksum_req(
        id: Uuid,
        url: &str,
        filename: &str,
        checksum: Option<String>,
    ) -> DownloadRequest {
        DownloadRequest {
            id,
            url: url.to_string(),
            final_url: None,
            filename: Some(filename.to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum,
            mirrors: vec![],
            timestamp: 1,
        }
    }

    struct TestServer {
        url: String,
        ranges: Arc<Mutex<Vec<(usize, usize)>>>,
        /// Highest number of range requests served concurrently — a direct
        /// measure of the effective connection count (independent of how many
        /// work-stealing pieces the file was split into).
        peak_concurrency: Arc<std::sync::atomic::AtomicUsize>,
    }

    struct HlsTestServer {
        manifest_url: String,
        requests: Arc<Mutex<Vec<String>>>,
    }

    struct PostEchoServer {
        url: String,
        body: Arc<Mutex<Vec<u8>>>,
    }

    async fn spawn_post_echo_server() -> Result<PostEchoServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let body = Arc::new(Mutex::new(Vec::new()));
        let server_body = body.clone();

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _addr)) = listener.accept().await else {
                    break;
                };
                let body = server_body.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = &buf[..n];
                    if let Some(pos) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                        *body.lock().await = request[pos + 4..].to_vec();
                    }
                    let response =
                        b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\npost-ok";
                    let _ = stream.write_all(response).await;
                });
            }
        });

        Ok(PostEchoServer {
            url: format!("http://{addr}/post-download"),
            body,
        })
    }

    async fn spawn_hls_server(
        manifest_path: &'static str,
        manifest: &'static str,
        segments: Vec<(&'static str, Vec<u8>)>,
    ) -> Result<HlsTestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server_requests = requests.clone();
        let segment_map = Arc::new(
            segments
                .into_iter()
                .map(|(path, body)| (path.to_string(), body))
                .collect::<HashMap<_, _>>(),
        );

        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _addr)) = listener.accept().await else {
                    break;
                };
                let requests = server_requests.clone();
                let segment_map = segment_map.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/")
                        .to_string();
                    requests.lock().await.push(path.clone());

                    let body = if path == manifest_path {
                        Some(manifest.as_bytes().to_vec())
                    } else {
                        segment_map.get(&path).cloned()
                    };

                    let response = if let Some(body) = body {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&body);
                        response
                    } else {
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                            .to_vec()
                    };
                    let _ = stream.write_all(&response).await;
                });
            }
        });

        Ok(HlsTestServer {
            manifest_url: format!("http://{addr}{manifest_path}"),
            requests,
        })
    }

    async fn spawn_range_server(data: Arc<Vec<u8>>) -> Result<TestServer> {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let server_ranges = ranges.clone();
        let inflight = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let server_inflight = inflight.clone();
        let server_peak = peak.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _addr)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                let ranges = server_ranges.clone();
                let inflight = server_inflight.clone();
                let peak = server_peak.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    let response = if request.starts_with("HEAD ") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes()
                    } else if let Some((start, end)) = parse_range(&request) {
                        let current = inflight.fetch_add(1, Ordering::SeqCst) + 1;
                        peak.fetch_max(current, Ordering::SeqCst);
                        // Hold the slot briefly so concurrent workers actually overlap.
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        let end = end.min(data.len().saturating_sub(1));
                        ranges.lock().await.push((start, end));
                        let body = &data[start..=end];
                        let mut response = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len(),
                            start,
                            end,
                            data.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(body);
                        inflight.fetch_sub(1, Ordering::SeqCst);
                        response
                    } else {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&data);
                        response
                    };
                    let _ = stream.write_all(&response).await;
                });
            }
        });

        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: peak,
        })
    }

    async fn spawn_one_rate_limit_server(data: Arc<Vec<u8>>) -> Result<TestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let server_ranges = ranges.clone();
        let tripped = Arc::new(AtomicBool::new(false));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _addr)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                let ranges = server_ranges.clone();
                let tripped = tripped.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if request.starts_with("HEAD ") || parse_range(&request).is_none() {
                        // Planning probe (plain GET): headers, plus the body for a GET.
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes();
                        if !request.starts_with("HEAD ") {
                            response.extend_from_slice(&data);
                        }
                        let _ = stream.write_all(&response).await;
                        return;
                    }

                    if !tripped.swap(true, Ordering::SeqCst) {
                        let response =
                            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }

                    let Some((start, end)) = parse_range(&request) else {
                        return;
                    };
                    let end = end.min(data.len().saturating_sub(1));
                    ranges.lock().await.push((start, end));
                    let body = &data[start..=end];
                    let mut response = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                        body.len(),
                        start,
                        end,
                        data.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(body);
                    let _ = stream.write_all(&response).await;
                });
            }
        });

        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    async fn spawn_cookie_required_range_server(data: Arc<Vec<u8>>) -> Result<TestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let server_ranges = ranges.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _addr)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                let ranges = server_ranges.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if !request.to_ascii_lowercase().contains("cookie: session=abc") {
                        let response =
                            "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }

                    let response = if request.starts_with("HEAD ") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes()
                    } else if let Some((start, end)) = parse_range(&request) {
                        let end = end.min(data.len().saturating_sub(1));
                        ranges.lock().await.push((start, end));
                        let body = &data[start..=end];
                        let mut response = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len(),
                            start,
                            end,
                            data.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(body);
                        response
                    } else {
                        let mut response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes();
                        response.extend_from_slice(&data);
                        response
                    };
                    let _ = stream.write_all(&response).await;
                });
            }
        });

        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    /// Server that never sends `Accept-Ranges` and always returns the full body.
    async fn spawn_no_range_server(data: Arc<Vec<u8>>) -> Result<TestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if request.starts_with("HEAD ") {
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            data.len()
                        );
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                    let mut response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    )
                    .into_bytes();
                    response.extend_from_slice(&data);
                    let _ = stream.write_all(&response).await;
                });
            }
        });
        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    /// Server without ranges that returns 429 to GET requests.
    async fn spawn_single_stream_429_server() -> Result<TestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if request.starts_with("HEAD ") {
                        let response =
                            "HTTP/1.1 200 OK\r\nContent-Length: 1000\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                        return;
                    }
                    let response = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                    let _ = stream.write_all(response.as_bytes()).await;
                });
            }
        });
        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    /// Server that returns 429 to HEAD requests.
    async fn spawn_head_429_server() -> Result<TestServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let ranges = Arc::new(Mutex::new(Vec::new()));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut buf = vec![0_u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&buf[..n]);
                    if request.starts_with("HEAD ") || parse_range(&request).is_none() {
                        let response = "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                        let _ = stream.write_all(response.as_bytes()).await;
                    }
                    // Ranged GETs are never reached: a 429 on the planning request aborts job::run.
                });
            }
        });
        Ok(TestServer {
            url: format!("http://{addr}/payload.bin"),
            ranges,
            peak_concurrency: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        })
    }

    #[tokio::test]
    async fn dynamic_halving_splits_in_flight_pieces() -> Result<()> {
        // 16 MiB with 2 connections starts as 2 pieces; whenever a worker is idle it
        // halves the in-flight piece with most bytes left, so there must be more
        // Range requests than the 2 initial pieces. The file must stay byte-perfect.
        let data = Arc::new(
            (0..16 * 1_048_576)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let server = spawn_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::settings::set(&pool, "max_connections_per_host", "2").await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("tail-split.bin");
        let req = DownloadRequest {
            id,
            url: server.url.clone(),
            final_url: None,
            filename: Some("tail-split.bin".to_string()),
            referrer: None,
            cookies: None,
            headers: None,
            size_hint: Some(data.len() as u64),
            mime_type: Some("application/octet-stream".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1,
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        run(JobContext {
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
        })
        .await?;

        let written = tokio::fs::read(&save_path).await?;
        assert_eq!(
            written, *data,
            "file content must be byte-perfect after split"
        );

        let record = db::downloads::get(&pool, id).await?.expect("record exists");
        assert_eq!(record.status, DownloadStatus::Completed);
        assert!(db::chunks::all_chunks_complete(&pool, id).await?);

        // Halving needs an idle worker next to a slow piece; the deterministic check
        // of that lives in phase1_tests (idle_worker_steals_from_a_slow_in_flight_piece).
        // Piece 0 rides on the planning GET, so only piece 1 is guaranteed a ranged request.
        assert!(!server.ranges.lock().await.is_empty());

        Ok(())
    }

    fn parse_range(request: &str) -> Option<(usize, usize)> {
        let marker = "Range: bytes=";
        let line = request
            .lines()
            .find(|line| line.to_ascii_lowercase().starts_with("range: bytes="))?;
        let range = line.get(marker.len()..)?;
        let (start, end) = range.split_once('-')?;
        Some((start.trim().parse().ok()?, end.trim().parse().ok()?))
    }

    #[tokio::test]
    async fn redirect_to_private_ip_is_blocked() -> Result<()> {
        // Spawn a server that redirects to a private IP.
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server_url = format!("http://{addr}/file.bin");

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let response = "HTTP/1.1 302 Found\r\nLocation: http://192.168.1.1/evil\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("file.bin");
        let req = DownloadRequest {
            id,
            url: server_url.clone(),
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
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        // Use the production client that has the SSRF redirect policy.
        let client = crate::manager::build_reqwest_client(None).unwrap();
        let err = run(JobContext {
            pool,
            record,
            client,
            progress_tx,
            bandwidth: Arc::new(BandwidthLimiter::new(0)),
            cancel_rx,
            cookies: None,
            extra_headers: None,
            method: "GET".to_string(),
            proxy_client: None,
            post_body: None,
        })
        .await
        .expect_err("redirect to private IP must fail");

        // The full error chain (Debug format) contains the redirect block message.
        let msg = format!("{err:?}").to_ascii_lowercase();
        assert!(
            msg.contains("blocked") || msg.contains("ssrf") || msg.contains("192.168"),
            "expected SSRF block in error chain, got: {msg}"
        );
        Ok(())
    }

    #[tokio::test]
    async fn signed_url_403_surfaces_expiry_error() -> Result<()> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let server_url = format!("http://{addr}/file.zip?X-Amz-Signature=abc&X-Amz-Expires=300");

        tokio::spawn(async move {
            if let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = tokio::io::AsyncReadExt::read(&mut stream, &mut buf).await;
                let response =
                    "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("file.zip");
        let req = DownloadRequest {
            id,
            url: server_url.clone(),
            final_url: None,
            filename: Some("file.zip".to_string()),
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
        };
        let record =
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        let (progress_tx, _) = broadcast::channel(16);
        let (_cancel_tx, cancel_rx) = watch::channel(false);

        let err = run(JobContext {
            pool,
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
        })
        .await
        .expect_err("signed URL 403 must return SignedUrlExpired");

        assert!(
            matches!(
                err.downcast_ref::<FerricError>(),
                Some(FerricError::SignedUrlExpired)
            ),
            "expected SignedUrlExpired, got: {err}"
        );
        Ok(())
    }
}

#[cfg(test)]
mod file_time_tests {
    use super::parse_http_date;

    #[test]
    fn http_dates_parse_to_the_server_time() {
        let t = parse_http_date("Tue, 15 Mar 2016 10:20:30 GMT").expect("parsed");
        assert_eq!(
            t.duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(),
            1_458_037_230
        );
        assert!(parse_http_date("not a date").is_none());
    }
}
