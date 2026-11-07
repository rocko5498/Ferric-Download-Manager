use crate::chunk::{self, ChunkPlan};
use crate::job::{self, JobContext, JobHandle};
use crate::throttle::{BandwidthLimiter, DomainBandwidthPool};
use crate::{db, diagnostics, quota, scheduler, security};
use anyhow::Context as _;
use base64::Engine as _;
use directories::UserDirs;
use ferric_common::{
    messages::{DaemonReply, DownloadRequest, ProgressEvent},
    DownloadRecord, DownloadStatus, FerricError,
};
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot, watch, Mutex};
use uuid::Uuid;

pub enum ManagerCommand {
    /// Add a download. When `hold` is true the record is inserted `Paused` and
    /// NOT started â€” it waits for a `Confirm` from the capture dialog. Extension
    /// captures hold; UI/API adds auto-start.
    Add {
        req: DownloadRequest,
        save_dir_override: Option<PathBuf>,
        hold: bool,
        audio_url: Option<String>,
        subtitle_url: Option<String>,
        subtitle_lang: Option<String>,
        proxy_url: Option<String>,
        reply_tx: oneshot::Sender<DaemonReply>,
    },
    /// Confirm a held capture: apply filename/folder overrides, then start it (or, with
    /// `start: false`, keep it in the list for later like IDM's "Download Later").
    Confirm {
        id: Uuid,
        filename: Option<String>,
        save_dir: Option<String>,
        url: Option<String>,
        audio_url: Option<String>,
        subtitle_url: Option<String>,
        subtitle_lang: Option<String>,
        proxy_url: Option<String>,
        start: bool,
        description: Option<String>,
        category: Option<String>,
        reply_tx: oneshot::Sender<DaemonReply>,
    },
    /// Ids of captures waiting for the Download File Info dialog.
    ListPending {
        reply_tx: oneshot::Sender<Vec<Uuid>>,
    },
    Pause(Uuid),
    Resume(Uuid),
    /// IDM's "Download Now" (File menu): start this download at once, ignoring the queue's
    /// concurrency limit (it starts the selected, not-running items outside the queue gate).
    StartNow(Uuid),
    /// IDM's "Redownload": throw away the saved file and chunk state, then start from byte 0.
    Redownload(Uuid),
    /// Stop an active download and mark it `Cancelled`, keeping the record.
    Cancel(Uuid),
    /// Stop and delete the record entirely, optionally deleting the saved file.
    Remove {
        id: Uuid,
        delete_file: bool,
    },
    SetPriority {
        id: Uuid,
        priority: i32,
    },
    /// A limit changed (global setting, queue, per-download): running jobs adopt the new rate now.
    RefreshLimits,
    /// The global proxy setting changed.
    ReloadProxy,
    SetSpeedLimit {
        id: Uuid,
        /// bytes/sec; 0 = unlimited
        bps: u64,
    },
    /// Replace the URL for a download. If the download is in Error state it is
    /// automatically re-queued so it resumes with the fresh address.
    UpdateUrl {
        id: Uuid,
        new_url: String,
        reply_tx: oneshot::Sender<DaemonReply>,
    },
    /// Internal: a job slot freed (or new work arrived) — start queued downloads
    /// up to each queue's concurrency limit.
    PumpQueue,
    /// Internal: a held capture was probed; start it while its info dialog is open.
    EarlyStart(Uuid),
    /// Internal, every few seconds: count the bytes received and enforce the download limits.
    QuotaTick,
}

/// What the Download File Info dialog can add to a plain confirm.
pub struct ConfirmExtras {
    /// `false` = "Download Later": keep the download in the list without starting it.
    pub start: bool,
    pub description: Option<String>,
    pub category: Option<String>,
}

impl Default for ConfirmExtras {
    fn default() -> Self {
        Self {
            start: true,
            description: None,
            category: None,
        }
    }
}

/// Cookies/headers for a captured download held pending user confirmation.
/// Kept in memory only â€” never persisted (security rule), so an unconfirmed
/// capture loses its credentials across a daemon restart.
#[derive(Clone)]
struct PendingCapture {
    req: DownloadRequest,
    save_dir: PathBuf,
    /// The caller chose the folder, so a probe must not re-route it to a category folder.
    dir_overridden: bool,
    cookies: Option<String>,
    headers: Option<HashMap<String, String>>,
    method: String,
    post_body: Option<Vec<u8>>,
}

/// Credentials for a download that is `Queued` but not yet started (waiting for a
/// concurrency slot). In-memory only, consumed when the job spawns.
#[derive(Clone, Default)]
struct StartCreds {
    cookies: Option<String>,
    headers: Option<HashMap<String, String>>,
    method: String,
    post_body: Option<Vec<u8>>,
}

pub struct DownloadManager {
    pool: SqlitePool,
    jobs: Arc<Mutex<HashMap<Uuid, JobHandle>>>,
    pending_captures: Arc<Mutex<HashMap<Uuid, PendingCapture>>>,
    pending_creds: Arc<Mutex<HashMap<Uuid, StartCreds>>>,
    cmd_tx: mpsc::Sender<ManagerCommand>,
    cmd_rx: Option<mpsc::Receiver<ManagerCommand>>,
    progress_tx: broadcast::Sender<ProgressEvent>,
    http_client: reqwest::Client,
    /// The global proxy (setting `proxy_url`); swapped when the setting changes while the daemon runs.
    proxy_client: Arc<std::sync::RwLock<Option<reqwest::Client>>>,
    /// Shared per-host site bandwidth caps (Task 100).
    domain_bandwidth: Arc<DomainBandwidthPool>,
    /// When the last queued download was started (queue start pacing).
    last_queue_start: Arc<Mutex<Option<tokio::time::Instant>>>,
    /// A delayed `PumpQueue` is already scheduled.
    pump_scheduled: Arc<std::sync::atomic::AtomicBool>,
    /// The proxy auto-config script last fetched: when, from where, and its text.
    pac_cache: Arc<Mutex<Option<(std::time::Instant, String, String)>>>,
    /// Download limits (quota): the bytes of the last hours, what each job had counted, and whether the limit is hit.
    quota: Arc<Mutex<(quota::QuotaWindow, HashMap<Uuid, u64>)>>,
    quota_hit: Arc<std::sync::atomic::AtomicBool>,
    /// Held captures that were started early, with the path their job writes to.
    early_started: Arc<Mutex<HashMap<Uuid, PathBuf>>>,
}

impl DownloadManager {
    pub async fn new(pool: SqlitePool) -> anyhow::Result<Self> {
        let (cmd_tx, cmd_rx) = mpsc::channel(256);
        let (progress_tx, _) = broadcast::channel(1024);
        let proxy_url = db::settings::get(&pool, "proxy_url")
            .await
            .ok()
            .flatten()
            .filter(|s| !s.is_empty());
        let http_client = build_reqwest_client(None)?;
        let proxy_client = proxy_url
            .as_deref()
            .map(|url| build_reqwest_client(Some(url)))
            .transpose()?;
        // Held captures lost their credentials with the old process; drop them instead of leaving paused orphans.
        match db::downloads::purge_held(&pool).await {
            Ok(0) => {}
            Ok(n) => tracing::info!(
                count = n,
                "dropped unconfirmed captures from the previous run"
            ),
            Err(e) => tracing::warn!(error = %e, "could not purge held captures"),
        }

        // A rename answered while a download was still running, then the daemon stopped: finish the move now.
        let (startup_tx, _) = broadcast::channel(4);
        for id in db::downloads::pending_move_ids(&pool)
            .await
            .unwrap_or_default()
        {
            Self::apply_pending_move(&pool, &startup_tx, id).await;
        }

        Ok(Self {
            pool,
            jobs: Arc::new(Mutex::new(HashMap::new())),
            pending_captures: Arc::new(Mutex::new(HashMap::new())),
            quota: Arc::default(),
            pac_cache: Arc::default(),
            quota_hit: Arc::default(),
            early_started: Arc::default(),
            pending_creds: Arc::new(Mutex::new(HashMap::new())),
            cmd_tx,
            cmd_rx: Some(cmd_rx),
            progress_tx,
            http_client,
            proxy_client: Arc::new(std::sync::RwLock::new(proxy_client)),
            domain_bandwidth: Arc::new(DomainBandwidthPool::new()),
            last_queue_start: Arc::new(Mutex::new(None)),
            pump_scheduled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        })
    }

    pub fn command_tx(&self) -> mpsc::Sender<ManagerCommand> {
        self.cmd_tx.clone()
    }

    pub fn progress_tx(&self) -> broadcast::Sender<ProgressEvent> {
        self.progress_tx.clone()
    }

    pub async fn recover_incomplete(&self) -> anyhow::Result<()> {
        let incomplete = db::downloads::find_incomplete(&self.pool).await?;
        if incomplete.is_empty() {
            self.pump_queue().await?;
            return Ok(());
        }

        tracing::info!(count = incomplete.len(), "recovering incomplete downloads");
        for record in incomplete {
            db::chunks::reset_chunks_for_resume(&self.pool, record.id).await?;
            db::downloads::update_status(&self.pool, record.id, DownloadStatus::Queued, None)
                .await?;
        }
        // Cookies and extension-only headers are intentionally not persisted.
        // Authenticated downloads must be re-triggered by the extension after a restart.
        // The queue gate decides how many recovered downloads start at once.
        self.pump_queue().await?;
        Ok(())
    }

    pub async fn run(mut self) -> anyhow::Result<()> {
        let mut rx = self
            .cmd_rx
            .take()
            .ok_or_else(|| anyhow::anyhow!("run() called twice"))?;

        // Periodically re-check schedule windows so downloads start automatically
        // when a queue's time window opens (without needing an unrelated event).
        let pump_tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_mins(1));
            loop {
                interval.tick().await;
                if pump_tx.send(ManagerCommand::PumpQueue).await.is_err() {
                    break;
                }
            }
        });

        let quota_tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
            loop {
                interval.tick().await;
                if quota_tx.send(ManagerCommand::QuotaTick).await.is_err() {
                    break;
                }
            }
        });

        while let Some(cmd) = rx.recv().await {
            if let Err(e) = self.handle(cmd).await {
                tracing::warn!("manager command error: {e}");
            }
        }
        Ok(())
    }

    async fn handle(&self, cmd: ManagerCommand) -> anyhow::Result<()> {
        match cmd {
            ManagerCommand::Add {
                req,
                save_dir_override,
                hold,
                audio_url,
                subtitle_url,
                subtitle_lang,
                proxy_url,
                reply_tx,
            } => {
                self.handle_add(
                    req,
                    save_dir_override,
                    hold,
                    audio_url,
                    subtitle_url,
                    subtitle_lang,
                    proxy_url,
                    reply_tx,
                )
                .await?;
            }
            ManagerCommand::Confirm {
                id,
                filename,
                save_dir,
                url,
                audio_url,
                subtitle_url,
                subtitle_lang,
                proxy_url,
                start,
                description,
                category,
                reply_tx,
            } => {
                self.handle_confirm_ext(
                    id,
                    filename,
                    save_dir,
                    url,
                    audio_url,
                    subtitle_url,
                    subtitle_lang,
                    proxy_url,
                    ConfirmExtras {
                        start,
                        description,
                        category,
                    },
                    reply_tx,
                )
                .await?;
            }
            ManagerCommand::ListPending { reply_tx } => {
                let ids: Vec<Uuid> = self.pending_captures.lock().await.keys().copied().collect();
                let _ = reply_tx.send(ids);
            }
            ManagerCommand::Pause(id) => {
                self.cancel_job(id).await;
                db::downloads::update_status(&self.pool, id, DownloadStatus::Paused, None).await?;
                let _ = self.progress_tx.send(ProgressEvent::Progress {
                    id,
                    downloaded: 0,
                    total: None,
                    speed_bps: 0,
                    eta_secs: None,
                    status: DownloadStatus::Paused,
                });
            }
            ManagerCommand::Resume(id) => {
                db::downloads::update_status(&self.pool, id, DownloadStatus::Queued, None).await?;
                // Goes through the gate: starts now if a slot is free, else waits.
                self.pump_queue().await?;
            }
            ManagerCommand::Redownload(id) => {
                self.cancel_job(id).await;
                self.delete_saved_file(id).await?;
                db::chunks::delete_all(&self.pool, id).await?;
                db::downloads::update_progress(&self.pool, id, 0).await?;
                db::downloads::update_status(&self.pool, id, DownloadStatus::Queued, None).await?;
                let _ = self.progress_tx.send(ProgressEvent::Progress {
                    id,
                    downloaded: 0,
                    total: None,
                    speed_bps: 0,
                    eta_secs: None,
                    status: DownloadStatus::Queued,
                });
                self.pump_queue().await?;
            }
            ManagerCommand::StartNow(id) => {
                if self.jobs.lock().await.contains_key(&id) {
                    return Ok(());
                }
                let Some(record) = db::downloads::get(&self.pool, id).await? else {
                    return Ok(());
                };
                if !matches!(
                    record.status,
                    DownloadStatus::Paused
                        | DownloadStatus::Queued
                        | DownloadStatus::Error
                        | DownloadStatus::Cancelled
                ) {
                    return Ok(());
                }
                db::downloads::update_status(&self.pool, id, DownloadStatus::Queued, None).await?;
                let record = db::downloads::get(&self.pool, id).await?.unwrap_or(record);
                let creds = self
                    .pending_creds
                    .lock()
                    .await
                    .remove(&id)
                    .unwrap_or_default();
                self.spawn_job(record, creds).await?;
            }
            ManagerCommand::Cancel(id) => {
                diagnostics::event(
                    "download_cancel_requested",
                    serde_json::json!({
                        "id": id,
                    }),
                )
                .await;
                self.cancel_job(id).await;
                self.clear_transient_secrets(id).await;
                db::downloads::update_status(&self.pool, id, DownloadStatus::Cancelled, None)
                    .await?;
                // Keep the row, but reflect the new state. Send the current
                // downloaded/total so the progress bar isn't reset.
                let (downloaded, total) = db::downloads::get(&self.pool, id)
                    .await?
                    .map_or((0, None), |r| (r.downloaded, r.total_size));
                let _ = self.progress_tx.send(ProgressEvent::Progress {
                    id,
                    downloaded,
                    total,
                    speed_bps: 0,
                    eta_secs: None,
                    status: DownloadStatus::Cancelled,
                });
            }
            ManagerCommand::Remove { id, delete_file } => {
                let existing = db::downloads::get(&self.pool, id).await?;
                diagnostics::event(
                    "download_remove_requested",
                    serde_json::json!({
                        "id": id,
                        "delete_file": delete_file,
                        "save_path": existing.as_ref().map(|record| record.save_path.as_str()),
                        "status": existing.as_ref().map(|record| format!("{:?}", record.status)),
                    }),
                )
                .await;
                self.cancel_job(id).await;
                self.clear_transient_secrets(id).await;
                // Cancel in the info dialog of a download that already started: its partial file goes too.
                let was_early = self.early_started.lock().await.remove(&id).is_some();
                let _ = db::downloads::pending_move_take(&self.pool, id).await;
                if delete_file || was_early {
                    self.delete_saved_file(id).await?;
                }
                db::downloads::delete(&self.pool, id).await?;
                diagnostics::event(
                    "download_removed",
                    serde_json::json!({
                        "id": id,
                        "delete_file": delete_file,
                    }),
                )
                .await;
                let _ = self.progress_tx.send(ProgressEvent::Removed { id });
            }
            ManagerCommand::SetPriority { id, priority } => {
                db::downloads::update_priority(&self.pool, id, priority).await?;
                self.pump_queue().await?;
            }
            ManagerCommand::SetSpeedLimit { id, bps } => {
                db::downloads::update_speed_limit(&self.pool, id, bps).await?;
                self.refresh_limits().await?;
            }
            ManagerCommand::RefreshLimits => {
                self.refresh_limits().await?;
            }
            ManagerCommand::QuotaTick => self.quota_tick().await?,
            ManagerCommand::EarlyStart(id) => {
                if let Err(e) = self.early_start(id).await {
                    tracing::warn!(id = %id, error = %e, "could not start the capture early");
                }
            }
            ManagerCommand::ReloadProxy => {
                if let Err(e) = self.reload_proxy().await {
                    tracing::warn!(error = %e, "could not apply the new proxy setting");
                }
            }
            ManagerCommand::UpdateUrl {
                id,
                new_url,
                reply_tx,
            } => {
                let result = self.handle_update_url(id, new_url).await;
                let reply = match result {
                    Ok(()) => DaemonReply::Accepted { id },
                    Err(e) => DaemonReply::Error {
                        id,
                        message: e.to_string(),
                    },
                };
                let _ = reply_tx.send(reply);
            }
            ManagerCommand::PumpQueue => {
                self.pump_queue().await?;
            }
        }
        Ok(())
    }

    async fn clear_transient_secrets(&self, id: Uuid) {
        self.pending_captures.lock().await.remove(&id);
        let _ = db::downloads::held_remove(&self.pool, id).await;
        self.pending_creds.lock().await.remove(&id);
    }

    fn proxy(&self) -> Option<reqwest::Client> {
        self.proxy_client
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// The `proxy_url` setting changed: new downloads use the new proxy from now on.
    async fn reload_proxy(&self) -> anyhow::Result<()> {
        let url = db::settings::get(&self.pool, "proxy_url")
            .await?
            .filter(|s| !s.trim().is_empty());
        let client = url
            .as_deref()
            .map(|u| build_reqwest_client(Some(u)))
            .transpose()?;
        *self
            .proxy_client
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = client;
        Ok(())
    }

    /// Recompute the effective limit of every running job and hand it to the job's limiter.
    async fn refresh_limits(&self) -> anyhow::Result<()> {
        let global = load_global_bandwidth_limit_bps(&self.pool).await?;
        let handles: Vec<JobHandle> = self.jobs.lock().await.values().cloned().collect();
        for handle in handles {
            let Some(record) = db::downloads::get(&self.pool, handle.id).await? else {
                continue;
            };
            let queue_limit = db::queues::get(&self.pool, handle.queue_id)
                .await?
                .and_then(|queue| queue.max_bandwidth);
            handle.bandwidth.set_rate(effective_bandwidth_limit(
                global,
                queue_limit,
                nonzero_limit(record.speed_limit_bps),
            ));
        }
        Ok(())
    }

    async fn handle_update_url(&self, id: Uuid, new_url: String) -> anyhow::Result<()> {
        security::validate_url(&new_url)?;
        db::downloads::update_url(&self.pool, id, &new_url).await?;
        let Some(record) = db::downloads::get(&self.pool, id).await? else {
            return Ok(());
        };
        if record.status == DownloadStatus::Error {
            db::chunks::reset_for_url_refresh(&self.pool, id).await?;
            db::downloads::update_status(&self.pool, id, DownloadStatus::Queued, None).await?;
            self.pump_queue().await?;
        }
        Ok(())
    }

    async fn delete_saved_file(&self, id: Uuid) -> anyhow::Result<()> {
        let Some(record) = db::downloads::get(&self.pool, id).await? else {
            return Ok(());
        };
        let path = PathBuf::from(&record.save_path);
        match tokio::fs::symlink_metadata(&path).await {
            Ok(metadata) if metadata.file_type().is_file() => {
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("failed to delete {}", path.display()))?;
            }
            Ok(metadata) if metadata.file_type().is_symlink() => {
                anyhow::bail!("refusing to delete symlink {}", path.display());
            }
            Ok(_) => {
                anyhow::bail!("refusing to delete non-file path {}", path.display());
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e).with_context(|| format!("cannot inspect {}", path.display())),
        }
        Ok(())
    }

    async fn handle_add(
        &self,
        req: DownloadRequest,
        save_dir_override: Option<PathBuf>,
        hold: bool,
        audio_url: Option<String>,
        subtitle_url: Option<String>,
        subtitle_lang: Option<String>,
        proxy_url: Option<String>,
        reply_tx: oneshot::Sender<DaemonReply>,
    ) -> anyhow::Result<()> {
        if let Err(e) = security::validate_url(&req.url) {
            let _ = reply_tx.send(DaemonReply::Error {
                id: req.id,
                message: e.to_string(),
            });
            return Ok(());
        }

        let req_id = req.id;
        let req = match self.expand_metalink(req).await {
            Ok(r) => r,
            Err(e) => {
                let _ = reply_tx.send(DaemonReply::Error {
                    id: req_id,
                    message: format!("metalink: {e}"),
                });
                return Ok(());
            }
        };

        let cookies = req.cookies.clone();
        let extra_headers = req.headers.clone();
        let method = normalized_method(&req.method);
        let post_body = decode_post_body(req.post_body_b64.as_deref())?;
        // Browser captures already carry the size; a typed address is probed in the background
        // while the dialog is open (see `probe_held_capture`), so the dialog never waits on it.
        let req = if hold && method == "GET" && req.size_hint.is_some() {
            self.enrich_capture_metadata(req, cookies.as_deref(), extra_headers.as_ref())
                .await
        } else {
            req
        };
        let dir_overridden = save_dir_override.is_some();
        let cats = crate::categories::load(&self.pool).await;
        let category = crate::categories::classify(
            req.filename.as_deref().unwrap_or(&req.url),
            req.mime_type.as_deref(),
            &cats,
        );
        let save_dir = match save_dir_override {
            Some(dir) => {
                // S12: validate save_dir is absolute before using it.
                crate::security::validate_save_path(dir.to_string_lossy().as_ref())?;
                dir
            }
            None => {
                crate::categories::folder_for(
                    &category,
                    &self.resolve_save_dir().await,
                    &cats,
                    &self.pool,
                )
                .await
            }
        };
        tokio::fs::create_dir_all(&save_dir).await?;

        let save_path = match build_save_path(req.filename.as_deref(), naming_url(&req), &save_dir)
        {
            Ok(path) => path,
            Err(e) => {
                let _ = reply_tx.send(DaemonReply::Error {
                    id: req.id,
                    message: e.to_string(),
                });
                return Ok(());
            }
        };

        let overwrite = db::settings::get(&self.pool, "duplicate_action")
            .await?
            .as_deref()
            == Some("overwrite");
        let save_path = free_save_path(&self.pool, save_path, None, overwrite).await;
        let save_path_str = save_path.to_string_lossy().to_string();

        match db::downloads::insert(&self.pool, &req, &save_path_str, &category).await {
            Ok(mut record) => {
                if audio_url.is_some() || subtitle_url.is_some() || proxy_url.is_some() {
                    db::downloads::update_target(
                        &self.pool,
                        record.id,
                        &record.filename,
                        &save_path_str,
                        audio_url.as_deref(),
                        subtitle_url.as_deref(),
                        subtitle_lang.as_deref(),
                        proxy_url.as_deref(),
                    )
                    .await?;
                    record.audio_url = audio_url.clone();
                    record.subtitle_url = subtitle_url.clone();
                    record.subtitle_lang = subtitle_lang.clone();
                    record.proxy_url = proxy_url.clone();
                }
                if hold {
                    // Held capture: persist Paused, stash credentials in memory,
                    // and wait for the dialog to Confirm before starting.
                    db::downloads::update_status(
                        &self.pool,
                        record.id,
                        DownloadStatus::Paused,
                        None,
                    )
                    .await?;
                    record.status = DownloadStatus::Paused;
                    self.pending_captures.lock().await.insert(
                        record.id,
                        PendingCapture {
                            req: req.clone(),
                            save_dir,
                            dir_overridden,
                            cookies,
                            headers: extra_headers,
                            method,
                            post_body,
                        },
                    );
                    let _ = db::downloads::held_add(&self.pool, record.id).await;
                    let _ = self.progress_tx.send(ProgressEvent::Added {
                        record: record.clone(),
                    });
                    let _ = self.progress_tx.send(ProgressEvent::CaptureHeld {
                        record: record.clone(),
                    });
                    let early_tx = self.cmd_tx.clone();
                    let held_id = record.id;
                    if req.size_hint.is_none() {
                        let client = self.proxy().unwrap_or_else(|| self.http_client.clone());
                        let probe = probe_held_capture(
                            self.pool.clone(),
                            client,
                            self.pending_captures.clone(),
                            self.progress_tx.clone(),
                            record.id,
                            self.resolve_save_dir().await,
                        );
                        tokio::spawn(async move {
                            probe.await;
                            let _ = early_tx.send(ManagerCommand::EarlyStart(held_id)).await;
                        });
                    } else {
                        let _ = early_tx.try_send(ManagerCommand::EarlyStart(held_id));
                    }
                    let _ = reply_tx.send(DaemonReply::Accepted { id: req.id });
                    tracing::info!(
                        id = %req.id,
                        url = %security::redact_url(&req.url),
                        "capture held for confirmation"
                    );
                } else {
                    // Auto-start path: stash creds, mark Queued, and let the gate
                    // decide whether to start now or hold for a free slot.
                    self.pending_creds.lock().await.insert(
                        record.id,
                        StartCreds {
                            cookies,
                            headers: extra_headers,
                            method,
                            post_body,
                        },
                    );
                    // IDM's "Do not start downloading, only add files to the queue": the file waits
                    // (paused, so "Start queue" picks it up) until the user starts it.
                    let only_queue =
                        db::settings::get_flag(&self.pool, "only_add_to_queue", false).await;
                    if only_queue {
                        db::downloads::update_status(
                            &self.pool,
                            record.id,
                            DownloadStatus::Paused,
                            None,
                        )
                        .await?;
                        record.status = DownloadStatus::Paused;
                    }
                    let _ = self.progress_tx.send(ProgressEvent::Added {
                        record: record.clone(),
                    });
                    let _ = reply_tx.send(DaemonReply::Accepted { id: req.id });
                    if !only_queue {
                        self.pump_queue().await?;
                    }
                    tracing::info!(
                        id = %req.id,
                        url = %security::redact_url(&req.url),
                        "download enqueued"
                    );
                }
            }
            Err(e) => {
                let _ = reply_tx.send(DaemonReply::Error {
                    id: req.id,
                    message: e.to_string(),
                });
            }
        }
        Ok(())
    }

    #[cfg(test)]
    async fn handle_confirm(
        &self,
        id: Uuid,
        filename: Option<String>,
        save_dir: Option<String>,
        url: Option<String>,
        audio_url: Option<String>,
        subtitle_url: Option<String>,
        subtitle_lang: Option<String>,
        proxy_url: Option<String>,
        reply_tx: oneshot::Sender<DaemonReply>,
    ) -> anyhow::Result<()> {
        self.handle_confirm_ext(
            id,
            filename,
            save_dir,
            url,
            audio_url,
            subtitle_url,
            subtitle_lang,
            proxy_url,
            ConfirmExtras::default(),
            reply_tx,
        )
        .await
    }

    async fn handle_confirm_ext(
        &self,
        id: Uuid,
        filename: Option<String>,
        save_dir: Option<String>,
        url: Option<String>,
        audio_url: Option<String>,
        subtitle_url: Option<String>,
        subtitle_lang: Option<String>,
        proxy_url: Option<String>,
        extras: ConfirmExtras,
        reply_tx: oneshot::Sender<DaemonReply>,
    ) -> anyhow::Result<()> {
        let Some(mut pending) = self.pending_captures.lock().await.get(&id).cloned() else {
            let _ = reply_tx.send(DaemonReply::Error {
                id,
                message: "no pending capture for this download".into(),
            });
            return Ok(());
        };

        let original_url = pending.req.url.clone();
        if let Some(url) = url.filter(|value| !value.trim().is_empty()) {
            if let Err(e) = security::validate_url(&url) {
                let _ = reply_tx.send(DaemonReply::Error {
                    id,
                    message: e.to_string(),
                });
                return Ok(());
            }
            pending.req.url = url.clone();
            pending.req.final_url = None;
            db::downloads::update_url(&self.pool, id, &url).await?;
        }

        let mut early = self.early_started.lock().await.get(&id).cloned();
        // A proxy chosen in the dialog also means a different route: the running job must start over.
        let proxy_changed = match (&proxy_url, early.is_some()) {
            (Some(p), true) => {
                let current = db::downloads::get(&self.pool, id)
                    .await?
                    .and_then(|r| r.proxy_url);
                !p.trim().is_empty() && current.as_deref() != Some(p.as_str())
            }
            _ => false,
        };
        if early.is_some() && (pending.req.url != original_url || proxy_changed) {
            // The dialog changed the address or the proxy: what was fetched may belong to another file. Start over.
            self.cancel_job(id).await;
            self.delete_saved_file(id).await?;
            db::chunks::delete_all(&self.pool, id).await?;
            self.early_started.lock().await.remove(&id);
            early = None;
        }

        let dir = save_dir.map_or_else(|| pending.save_dir.clone(), PathBuf::from);
        tokio::fs::create_dir_all(&dir).await?;

        let chosen_name = filename
            .filter(|s| !s.trim().is_empty())
            .or_else(|| pending.req.filename.clone());
        let save_path =
            match build_save_path(chosen_name.as_deref(), naming_url(&pending.req), &dir) {
                Ok(path) => path,
                Err(e) => {
                    let _ = reply_tx.send(DaemonReply::Error {
                        id,
                        message: e.to_string(),
                    });
                    return Ok(());
                }
            };

        let overwrite = db::settings::get(&self.pool, "duplicate_action")
            .await?
            .as_deref()
            == Some("overwrite");
        let wanted_path = save_path.clone();
        let save_path = match &early {
            // the name and folder did not change: the file being written already sits there
            Some(started)
                if started
                    .to_string_lossy()
                    .eq_ignore_ascii_case(&wanted_path.to_string_lossy()) =>
            {
                started.clone()
            }
            _ => free_save_path(&self.pool, save_path, Some(id), overwrite).await,
        };
        let job_running = self.jobs.lock().await.contains_key(&id);
        // Where the file is saved now, and where the dialog wants it. While the job writes, the record keeps
        // the real path and the move happens when the job ends.
        let (save_path, moved_to) = match (&early, job_running) {
            (Some(started), true) if *started != save_path => (started.clone(), Some(save_path)),
            (Some(started), false) if *started != save_path => {
                if tokio::fs::metadata(started).await.is_ok() {
                    let _ = tokio::fs::rename(started, &save_path).await;
                    if tokio::fs::metadata(&save_path).await.is_ok() {
                        (save_path, None)
                    } else {
                        (started.clone(), None)
                    }
                } else {
                    (save_path, None)
                }
            }
            _ => (save_path, None),
        };
        let final_name = save_path
            .file_name()
            .map_or_else(|| "download".into(), |n| n.to_string_lossy().to_string());
        let save_path_str = save_path.to_string_lossy().to_string();

        // The dialog does not know about a paired audio track (YouTube, HLS audio groups): keep what the
        // capture was queued with unless the confirm names something else.
        let queued = db::downloads::get(&self.pool, id).await?;
        let audio_url = audio_url.or_else(|| queued.as_ref().and_then(|r| r.audio_url.clone()));
        let subtitle_url =
            subtitle_url.or_else(|| queued.as_ref().and_then(|r| r.subtitle_url.clone()));
        let subtitle_lang =
            subtitle_lang.or_else(|| queued.as_ref().and_then(|r| r.subtitle_lang.clone()));
        let proxy_url = proxy_url.or_else(|| queued.as_ref().and_then(|r| r.proxy_url.clone()));
        db::downloads::update_target(
            &self.pool,
            id,
            &final_name,
            &save_path_str,
            audio_url.as_deref(),
            subtitle_url.as_deref(),
            subtitle_lang.as_deref(),
            proxy_url.as_deref(),
        )
        .await?;
        if let Some(d) = extras
            .description
            .as_deref()
            .filter(|d| !d.trim().is_empty())
        {
            db::downloads::set_description(&self.pool, id, Some(d)).await?;
        }
        if let Some(c) = extras.category.as_deref().filter(|c| !c.trim().is_empty()) {
            db::downloads::set_category(&self.pool, id, c).await?;
        }
        if let Some(target) = moved_to {
            let name = target
                .file_name()
                .map_or_else(|| "download".into(), |n| n.to_string_lossy().to_string());
            db::downloads::pending_move_set(&self.pool, id, &target.to_string_lossy(), &name)
                .await?;
        }
        let was_early = self.early_started.lock().await.remove(&id).is_some();
        if !extras.start {
            if was_early && job_running {
                // "Download Later" for a download that already started: stop it, keep what was fetched.
                self.cancel_job(id).await;
                db::downloads::update_status(&self.pool, id, DownloadStatus::Paused, None).await?;
            }
            // "Download Later": stays in the list, paused; credentials kept so Resume still works.
            self.pending_creds.lock().await.insert(
                id,
                StartCreds {
                    cookies: pending.cookies.clone(),
                    headers: pending.headers.clone(),
                    method: pending.method.clone(),
                    post_body: pending.post_body.clone(),
                },
            );
            self.pending_captures.lock().await.remove(&id);
            let _ = db::downloads::held_remove(&self.pool, id).await;
            let _ = reply_tx.send(DaemonReply::Accepted { id });
            tracing::info!(id = %id, "capture saved for later");
            return Ok(());
        }
        if was_early {
            // Already started (or finished) while the dialog was open: nothing left to queue.
            self.pending_captures.lock().await.remove(&id);
            let _ = db::downloads::held_remove(&self.pool, id).await;
            let _ = reply_tx.send(DaemonReply::Accepted { id });
            tracing::info!(id = %id, "capture confirmed; it was already running");
            return Ok(());
        }
        db::downloads::update_status(&self.pool, id, DownloadStatus::Queued, None).await?;

        // Hand the stashed creds to the start path and drop the capture hold.
        self.pending_creds.lock().await.insert(
            id,
            StartCreds {
                cookies: pending.cookies,
                headers: pending.headers,
                method: pending.method,
                post_body: pending.post_body,
            },
        );
        self.pending_captures.lock().await.remove(&id);
        let _ = db::downloads::held_remove(&self.pool, id).await;

        // Confirmed downloads respect the same concurrency gate as everything else.
        self.pump_queue().await?;
        let _ = reply_tx.send(DaemonReply::Accepted { id });
        tracing::info!(id = %id, "capture confirmed; enqueued");
        Ok(())
    }

    /// IDM starts the transfer while its Download File Info dialog is still open. The held capture starts
    /// at once, at a free path (never on top of a file that exists), and the dialog's answer only decides
    /// afterwards: a changed name or folder moves the file when the job ends, Cancel deletes it.
    async fn early_start(&self, id: Uuid) -> anyhow::Result<()> {
        if !db::settings::get_flag(&self.pool, "start_while_info_dialog", true).await
            || db::settings::get_flag(&self.pool, "only_add_to_queue", false).await
        {
            return Ok(());
        }
        let Some(pending) = self.pending_captures.lock().await.get(&id).cloned() else {
            return Ok(());
        };
        let Some(mut record) = db::downloads::get(&self.pool, id).await? else {
            return Ok(());
        };
        // Merge downloads (separate audio/subtitle) and manifests are planned elsewhere: leave them to the dialog.
        if record.audio_url.is_some()
            || record.subtitle_url.is_some()
            || crate::job::is_manifest_url(&record.url)
            || self.jobs.lock().await.contains_key(&id)
            || self.early_started.lock().await.contains_key(&id)
        {
            return Ok(());
        }
        let overwrite = db::settings::get(&self.pool, "duplicate_action")
            .await?
            .as_deref()
            == Some("overwrite");
        let path = free_save_path(
            &self.pool,
            PathBuf::from(&record.save_path),
            Some(id),
            overwrite,
        )
        .await;
        let name = path.file_name().map_or_else(
            || record.filename.clone(),
            |n| n.to_string_lossy().to_string(),
        );
        let path_str = path.to_string_lossy().to_string();
        db::downloads::update_target(
            &self.pool,
            id,
            &name,
            &path_str,
            record.audio_url.as_deref(),
            record.subtitle_url.as_deref(),
            record.subtitle_lang.as_deref(),
            record.proxy_url.as_deref(),
        )
        .await?;
        record.filename = name;
        record.save_path = path_str;
        self.early_started.lock().await.insert(id, path);
        let creds = StartCreds {
            cookies: pending.cookies,
            headers: pending.headers,
            method: pending.method,
            post_body: pending.post_body,
        };
        self.spawn_job(record, creds).await?;
        tracing::info!(id = %id, "download started while the info dialog is open");
        Ok(())
    }

    /// Put a finished or stopped download's file where the dialog said (a different name or folder).
    async fn apply_pending_move(
        pool: &SqlitePool,
        tx: &broadcast::Sender<ProgressEvent>,
        id: Uuid,
    ) {
        let Ok(Some((target, name))) = db::downloads::pending_move_take(pool, id).await else {
            return;
        };
        let target = PathBuf::from(target);
        let Ok(Some(record)) = db::downloads::get(pool, id).await else {
            return;
        };
        let from = PathBuf::from(&record.save_path);
        if tokio::fs::metadata(&from).await.is_ok() {
            if let Some(parent) = target.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            if tokio::fs::rename(&from, &target).await.is_err() {
                // another drive: copy, then remove the original
                if tokio::fs::copy(&from, &target).await.is_err() {
                    tracing::warn!(id = %id, "could not move the file to the chosen place; it stays where it was saved");
                    return;
                }
                let _ = tokio::fs::remove_file(&from).await;
            }
        }
        let target_str = target.to_string_lossy().to_string();
        let _ = db::downloads::update_response_metadata(
            pool,
            id,
            None,
            None,
            None,
            Some(&name),
            Some(&target_str),
        )
        .await;
        if let Ok(Some(record)) = db::downloads::get(pool, id).await {
            let _ = tx.send(ProgressEvent::CaptureUpdated { record });
        }
    }

    /// The proxy IDM's "automatic configuration script" (PAC, setting `pac_url`) names for `target`, or
    /// `None` for a direct connection. The script is fetched directly and kept for ten minutes.
    async fn pac_proxy_for(&self, target: &str) -> Option<String> {
        let configured = db::settings::get(&self.pool, "pac_url")
            .await
            .ok()
            .flatten()
            .map(|s| s.trim().to_string())
            .filter(|s| s.starts_with("http://") || s.starts_with("https://"));
        // IDM offers the script Internet Explorer is set to use; here that is the opt-in "pac_from_windows".
        let pac_url = match configured {
            Some(url) => url,
            None if db::settings::get_flag(&self.pool, "pac_from_windows", false).await => {
                crate::pac::windows_pac_url()?
            }
            None => return None,
        };
        let cached = {
            let guard = self.pac_cache.lock().await;
            guard
                .as_ref()
                .filter(|(at, url, _)| url == &pac_url && at.elapsed() < Duration::from_secs(600))
                .map(|(_, _, script)| script.clone())
        };
        let script = if let Some(script) = cached {
            script
        } else {
            let fetched = async {
                let client = reqwest::Client::builder()
                    .timeout(Duration::from_secs(10))
                    .no_proxy()
                    .build()?;
                anyhow::Ok(
                    client
                        .get(&pac_url)
                        .send()
                        .await?
                        .error_for_status()?
                        .text()
                        .await?,
                )
            }
            .await;
            match fetched {
                Ok(script) => {
                    *self.pac_cache.lock().await =
                        Some((std::time::Instant::now(), pac_url.clone(), script.clone()));
                    script
                }
                Err(e) => {
                    tracing::warn!(error = %e, "could not fetch the proxy auto-config script; connecting directly");
                    return None;
                }
            }
        };
        let target = target.to_string();
        let choices = tokio::task::spawn_blocking(move || crate::pac::find_proxy(&script, &target))
            .await
            .ok()?
            .map_err(|e| tracing::warn!(error = %e, "proxy auto-config script failed; connecting directly"))
            .ok()?;
        match choices.into_iter().next()? {
            crate::pac::PacChoice::Proxy(url) => Some(url),
            crate::pac::PacChoice::Direct => None,
        }
    }

    /// Count what the running jobs received since the last tick; at the limit stop everything.
    async fn quota_tick(&self) -> anyhow::Result<()> {
        use std::sync::atomic::Ordering;
        let limits = quota::limits(
            db::settings::get(&self.pool, "quota_mb").await?.as_deref(),
            db::settings::get(&self.pool, "quota_hours")
                .await?
                .as_deref(),
        );
        let handles: Vec<JobHandle> = self.jobs.lock().await.values().cloned().collect();
        let now = std::time::Instant::now();
        let total = {
            let mut guard = self.quota.lock().await;
            let (window, seen) = &mut *guard;
            let mut live = HashMap::new();
            for handle in &handles {
                let now_seen = handle.bandwidth.bytes_seen();
                let before = seen.get(&handle.id).copied().unwrap_or(0);
                window.add(now, now_seen.saturating_sub(before));
                live.insert(handle.id, now_seen);
            }
            *seen = live;
            window.total(
                now,
                limits.map_or(std::time::Duration::from_secs(3600), |l| l.1),
            )
        };
        let Some((limit, _)) = limits else {
            self.quota_hit.store(false, Ordering::Relaxed);
            return Ok(());
        };
        if total < limit {
            self.quota_hit.store(false, Ordering::Relaxed);
            return Ok(());
        }
        if !self.quota_hit.swap(true, Ordering::Relaxed) || !handles.is_empty() {
            tracing::warn!(
                received_mb = total / 1_048_576,
                "download limit reached, stopping all downloads"
            );
            for handle in handles {
                self.cancel_job(handle.id).await;
                db::downloads::update_status(
                    &self.pool,
                    handle.id,
                    DownloadStatus::Paused,
                    Some("Download limit reached (Options > Connections)"),
                )
                .await?;
                let _ = self.progress_tx.send(ProgressEvent::Progress {
                    id: handle.id,
                    downloaded: 0,
                    total: None,
                    speed_bps: 0,
                    eta_secs: None,
                    status: DownloadStatus::Paused,
                });
            }
        }
        Ok(())
    }

    /// Start `Queued` downloads up to each queue's concurrency limit, in
    /// priority/age order, pulling any stashed credentials.
    async fn pump_queue(&self) -> anyhow::Result<()> {
        if self.quota_hit.load(std::sync::atomic::Ordering::Relaxed) {
            return Ok(());
        }
        self.pump_queue_at(scheduler::current_minute_local()).await
    }

    /// Inner pump with an explicit current minute, so tests can control the
    /// perceived time without touching the system clock.
    async fn pump_queue_at(&self, now_minute: u16) -> anyhow::Result<()> {
        let queued = db::downloads::list_queued_prioritized(&self.pool).await?;
        if queued.is_empty() {
            return Ok(());
        }

        // Snapshot which queues the currently-running jobs occupy.
        let running: HashMap<Uuid, Uuid> = self
            .jobs
            .lock()
            .await
            .values()
            .map(|h| (h.id, h.queue_id))
            .collect();

        // Resolve each distinct queue's active state and concurrency cap once.
        let mut caps: HashMap<Uuid, u32> = HashMap::new();
        for rec in &queued {
            let q = rec.queue_id.unwrap_or(db::queues::DEFAULT_QUEUE_ID);
            if let std::collections::hash_map::Entry::Vacant(e) = caps.entry(q) {
                let cap = match db::queues::get(&self.pool, q).await? {
                    Some(queue)
                        if scheduler::is_queue_active(&queue, now_minute)
                            && scheduler::runs_on(&queue, scheduler::current_weekday_local()) =>
                    {
                        queue.max_concurrent
                    }
                    Some(_) => 0,
                    None => db::queues::max_concurrent(&self.pool, q).await,
                };
                e.insert(cap);
            }
        }

        let spacing = self.queue_start_spacing().await;
        let to_start = select_admissions(&queued, db::queues::DEFAULT_QUEUE_ID, &running, &caps);
        for id in to_start {
            let Some(record) = queued.iter().find(|r| r.id == id).cloned() else {
                continue;
            };
            // Queued files start one at a time, ~3.3 s apart (as seen with six
            // 3 MB files: starts at +0, 2.3, 5.6, 8.9, 12.2, 15.4 s while each file
            // took 16 s, i.e. no waiting for the previous one to finish).
            if !spacing.is_zero() {
                let mut last = self.last_queue_start.lock().await;
                if let Some(at) = *last {
                    let waited = at.elapsed();
                    if waited < spacing {
                        self.schedule_pump_after(spacing.checked_sub(waited).unwrap());
                        break;
                    }
                }
                *last = Some(tokio::time::Instant::now());
            }
            let creds = self
                .pending_creds
                .lock()
                .await
                .remove(&id)
                .unwrap_or_default();
            self.spawn_job(record, creds).await?;
        }
        Ok(())
    }

    /// Minimum gap between two queued downloads starting (`queue_start_spacing_ms`
    /// setting; default 3300 ms like IDM, 0 in unit tests, 0 = off).
    async fn queue_start_spacing(&self) -> std::time::Duration {
        let default_ms: u64 = if cfg!(test) { 0 } else { 3_300 };
        let ms = db::settings::get(&self.pool, "queue_start_spacing_ms")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<u64>().ok())
            .unwrap_or(default_ms);
        std::time::Duration::from_millis(ms)
    }

    /// Run `PumpQueue` again after `delay` (at most one pending timer).
    fn schedule_pump_after(&self, delay: std::time::Duration) {
        use std::sync::atomic::Ordering;
        if self.pump_scheduled.swap(true, Ordering::SeqCst) {
            return;
        }
        let flag = self.pump_scheduled.clone();
        let tx = self.cmd_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            flag.store(false, Ordering::SeqCst);
            let _ = tx.send(ManagerCommand::PumpQueue).await;
        });
    }

    async fn spawn_job(&self, record: DownloadRecord, creds: StartCreds) -> anyhow::Result<()> {
        let id = record.id;
        let queue_id = record.queue_id.unwrap_or(db::queues::DEFAULT_QUEUE_ID);

        // Resolve bandwidth before inserting the handle so a DB failure here
        // cannot leave a stale handle that permanently occupies a queue slot.
        // Site cap is a *shared* per-domain parent bucket (Task 100); job layer
        // is min(global, queue, per-download) without double-counting site.
        let queue_limit = db::queues::get(&self.pool, queue_id)
            .await?
            .and_then(|queue| queue.max_bandwidth);
        let site_policy = db::site_rules::policy_for_url(&self.pool, &record.url).await?;
        // Read again for every job: the Speed Limiter dialog saves the setting while the daemon runs.
        let global_limit = load_global_bandwidth_limit_bps(&self.pool).await?;
        let job_bps = effective_bandwidth_limit(
            global_limit,
            queue_limit,
            nonzero_limit(record.speed_limit_bps),
        );
        let host = url::Url::parse(&record.url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_default();
        let domain_parent = site_policy
            .max_speed_bps
            .and_then(|bps| self.domain_bandwidth.get(&host, bps));

        let bandwidth = Arc::new(BandwidthLimiter::with_parent(job_bps, domain_parent));
        let cancel_rx = {
            let mut jobs = self.jobs.lock().await;
            if jobs.contains_key(&id) {
                return Ok(());
            }
            let (cancel_tx, cancel_rx) = watch::channel(false);
            jobs.insert(
                id,
                JobHandle::new(id, queue_id, cancel_tx, bandwidth.clone()),
            );
            cancel_rx
        };

        // A proxy chosen for this download wins, then the manual global proxy, then the auto-config script.
        let proxy_client = if let Some(ref url) = record.proxy_url {
            Some(build_reqwest_client(Some(url))?)
        } else if let Some(client) = self.proxy() {
            Some(client)
        } else if let Some(url) = self.pac_proxy_for(&record.url).await {
            Some(build_reqwest_client(Some(&url))?)
        } else {
            None
        };

        let ctx = JobContext {
            pool: self.pool.clone(),
            record,
            client: self.http_client.clone(),
            proxy_client,
            progress_tx: self.progress_tx.clone(),
            bandwidth,
            cancel_rx,
            cookies: creds.cookies,
            extra_headers: creds.headers,
            method: creds.method,
            post_body: creds.post_body,
        };
        let jobs = self.jobs.clone();
        let progress_tx = self.progress_tx.clone();
        let pool = self.pool.clone();
        let cmd_tx = self.cmd_tx.clone();

        tokio::spawn(async move {
            let result = job::run(ctx).await;
            jobs.lock().await.remove(&id);
            Self::apply_pending_move(&pool, &progress_tx, id).await;

            if let Err(e) = result {
                if !matches!(
                    e.downcast_ref::<FerricError>(),
                    Some(FerricError::Cancelled)
                ) {
                    // the whole cause chain: bare "error sending request" hides a refused proxy login
                    let message = format!("{e:#}");
                    diagnostics::event(
                        "job_error",
                        serde_json::json!({
                            "id": id,
                            "message": message.clone(),
                        }),
                    )
                    .await;
                    let _ = db::downloads::update_status(
                        &pool,
                        id,
                        DownloadStatus::Error,
                        Some(&message),
                    )
                    .await;
                    let _ = progress_tx.send(ProgressEvent::Failed { id, message });
                    if job::is_transient_failure(&e) {
                        schedule_auto_retry(pool.clone(), id, cmd_tx.clone());
                    }
                }
            }

            // This slot is now free â€” let the next queued download start.
            let _ = cmd_tx.send(ManagerCommand::PumpQueue).await;
        });
        Ok(())
    }

    /// Test-only wrapper: production resolves limits inline in `start_job`; the
    /// tests use this to cover the queue/site/global composition end-to-end.
    #[cfg(test)]
    async fn effective_bandwidth_limit_for_queue(
        &self,
        queue_id: Uuid,
        url: &str,
    ) -> anyhow::Result<u64> {
        let queue_limit = db::queues::get(&self.pool, queue_id)
            .await?
            .and_then(|queue| queue.max_bandwidth);
        let site_policy = db::site_rules::policy_for_url(&self.pool, url).await?;
        Ok(effective_bandwidth_limit(
            load_global_bandwidth_limit_bps(&self.pool).await?,
            queue_limit,
            site_policy.max_speed_bps,
        ))
    }

    async fn cancel_job(&self, id: Uuid) {
        if let Some(handle) = self.jobs.lock().await.remove(&id) {
            debug_assert_eq!(handle.id, id);
            handle.cancel();
        }
    }

    async fn resolve_save_dir(&self) -> PathBuf {
        if let Ok(Some(dir)) = db::settings::get(&self.pool, "default_save_dir").await {
            if !dir.is_empty() {
                return PathBuf::from(dir);
            }
        }
        if let Some(dl) = UserDirs::new().and_then(|d| d.download_dir().map(ToOwned::to_owned)) {
            return dl;
        }
        if let Ok(path) = db::default_db_path() {
            if let Some(parent) = path.parent() {
                return parent.join("Downloads");
            }
        }
        PathBuf::from("Downloads")
    }

    /// If `req.url` is a metalink document, fetch/parse it and rewrite the request
    /// to the first file's primary URL + mirrors + checksum (Task 094).
    async fn expand_metalink(&self, mut req: DownloadRequest) -> anyhow::Result<DownloadRequest> {
        if !ferric_common::is_metalink_url(&req.url) {
            return Ok(req);
        }
        let proxy = self.proxy();
        let client = proxy.as_ref().unwrap_or(&self.http_client);
        let response = client
            .get(&req.url)
            .send()
            .await
            .with_context(|| format!("fetch metalink {}", security::redact_url(&req.url)))?;
        if !response.status().is_success() {
            anyhow::bail!("metalink HTTP {}", response.status().as_u16());
        }
        let bytes = response.bytes().await.context("read metalink body")?;
        let files = ferric_common::parse_metalink(&bytes).map_err(|e| anyhow::anyhow!(e))?;
        let file = files
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("metalink contained no files"))?;

        let mut urls = file.urls;
        for url in &urls {
            security::validate_url(url)?;
        }
        let primary = urls.remove(0);
        if req.filename.is_none() {
            req.filename = file.filename;
        }
        if req.size_hint.is_none() {
            req.size_hint = file.size;
        }
        if req.checksum.is_none() {
            req.checksum = file.checksum;
        }
        let mut mirrors = urls;
        for m in req.mirrors.drain(..) {
            if m != primary && !mirrors.iter().any(|u| u == &m) {
                mirrors.push(m);
            }
        }
        req.url = primary;
        req.mirrors = mirrors;
        tracing::info!(
            id = %req.id,
            url = %security::redact_url(&req.url),
            mirrors = req.mirrors.len(),
            "expanded metalink into multi-source download"
        );
        Ok(req)
    }

    async fn enrich_capture_metadata(
        &self,
        mut req: DownloadRequest,
        cookies: Option<&str>,
        extra_headers: Option<&HashMap<String, String>>,
    ) -> DownloadRequest {
        let proxy = self.proxy();
        let client = proxy.as_ref().unwrap_or(&self.http_client);
        let metadata_result = tokio::time::timeout(
            Duration::from_secs(3),
            chunk::plan(&req.url, client, cookies, extra_headers, 1),
        )
        .await;

        let Ok(Ok(plan)) = metadata_result else {
            return with_mime_extension(req);
        };

        match plan {
            ChunkPlan::Segmented {
                file_size,
                metadata,
                ..
            } => {
                if !crate::job::is_manifest_url(&req.url) {
                    req.size_hint = Some(file_size);
                }
                apply_response_metadata(&mut req, metadata);
            }
            ChunkPlan::SingleStream { metadata, .. } => {
                apply_response_metadata(&mut req, metadata);
            }
        }

        with_mime_extension(req)
    }
}

/// IDM probes the address while its Download File Info dialog is open and then fills the dialog in.
/// Same here: the held record gets its real name, size, type, category and folder, and the UI is
/// told with `CaptureUpdated`. Does nothing if the capture was confirmed or cancelled meanwhile.
async fn probe_held_capture(
    pool: SqlitePool,
    client: reqwest::Client,
    pending: Arc<Mutex<HashMap<Uuid, PendingCapture>>>,
    tx: broadcast::Sender<ProgressEvent>,
    id: Uuid,
    base_dir: PathBuf,
) {
    let Some((url, cookies, headers)) = pending
        .lock()
        .await
        .get(&id)
        .map(|p| (p.req.url.clone(), p.cookies.clone(), p.headers.clone()))
    else {
        return;
    };
    let planned = tokio::time::timeout(
        Duration::from_secs(20),
        chunk::plan(&url, &client, cookies.as_deref(), headers.as_ref(), 1),
    )
    .await;
    let Ok(Ok(plan)) = planned else { return };
    let (size, metadata) = match plan {
        ChunkPlan::Segmented {
            file_size,
            metadata,
            ..
        } => (Some(file_size), metadata),
        ChunkPlan::SingleStream { metadata, .. } => (None, metadata),
    };

    // Hold the map while writing so a confirm cannot slip in between the check and the update.
    let mut map = pending.lock().await;
    let Some(entry) = map.get_mut(&id) else {
        return;
    };
    let mut req = entry.req.clone();
    if let Some(s) = size.filter(|_| !crate::job::is_manifest_url(&req.url)) {
        req.size_hint = Some(s);
    }
    apply_response_metadata(&mut req, metadata);
    let req = with_mime_extension(req);

    let cats = crate::categories::load(&pool).await;
    let category = crate::categories::classify(
        req.filename.as_deref().unwrap_or(&req.url),
        req.mime_type.as_deref(),
        &cats,
    );
    let dir = if entry.dir_overridden {
        entry.save_dir.clone()
    } else {
        crate::categories::folder_for(&category, &base_dir, &cats, &pool).await
    };
    let Ok(save_path) = build_save_path(req.filename.as_deref(), naming_url(&req), &dir) else {
        return;
    };
    let save_path_str = save_path.to_string_lossy().to_string();
    let name = save_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string());

    let written = async {
        db::downloads::update_response_metadata(
            &pool,
            id,
            req.final_url.as_deref(),
            None,
            req.mime_type.as_deref(),
            name.as_deref(),
            Some(&save_path_str),
        )
        .await?;
        if let Some(s) = size.filter(|_| !crate::job::is_manifest_url(&req.url)) {
            db::downloads::set_file_info(&pool, id, s, None, None).await?;
        }
        db::downloads::set_category(&pool, id, &category).await
    }
    .await;
    if written.is_err() {
        return;
    }
    entry.req = req;
    entry.save_dir = dir;
    if let Ok(Some(record)) = db::downloads::get(&pool, id).await {
        let _ = tx.send(ProgressEvent::CaptureUpdated { record });
    }
}

fn apply_response_metadata(req: &mut DownloadRequest, metadata: chunk::ResponseMetadata) {
    if let Some(final_url) = metadata.final_url {
        req.final_url = Some(final_url);
    }
    if let Some(filename) = metadata.filename {
        req.filename = Some(filename);
    }
    if let Some(mime_type) = metadata.mime_type {
        req.mime_type = Some(mime_type);
    }
}

fn with_mime_extension(mut req: DownloadRequest) -> DownloadRequest {
    // A name taken from the address gets its extension from the content type too (`/download` +
    // application/zip -> `download.zip`), as IDM does.
    let Some(filename) = req
        .filename
        .clone()
        .or_else(|| name_from_url(naming_url(&req)))
    else {
        return req;
    };
    if Path::new(&filename).extension().is_some() {
        return req;
    }
    let Some(extension) = extension_for_mime(req.mime_type.as_deref()) else {
        return req;
    };
    req.filename = Some(format!("{filename}.{extension}"));
    req
}

fn extension_for_mime(mime: Option<&str>) -> Option<&'static str> {
    let mime = mime?
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match mime.as_str() {
        "application/vnd.rar" | "application/x-rar-compressed" => Some("rar"),
        "application/zip" | "application/x-zip-compressed" => Some("zip"),
        "application/x-7z-compressed" => Some("7z"),
        "application/pdf" => Some("pdf"),
        "application/x-msdownload" | "application/vnd.microsoft.portable-executable" => Some("exe"),
        "application/vnd.apple.mpegurl" | "application/x-mpegurl" => Some("m3u8"),
        "application/dash+xml" => Some("mpd"),
        "video/mp4" => Some("mp4"),
        "video/x-matroska" => Some("mkv"),
        "audio/mpeg" => Some("mp3"),
        _ => None,
    }
}

pub(crate) fn build_reqwest_client(proxy_url: Option<&str>) -> anyhow::Result<reqwest::Client> {
    let redirect_policy = reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= 10 {
            return attempt.error("too many redirects");
        }
        let target = attempt.url().as_str().to_string();
        if security::validate_url(&target).is_err() {
            return attempt.error(format!(
                "redirect to blocked URL: {}",
                security::redact_url(&target)
            ));
        }
        attempt.follow()
    });
    let mut builder = reqwest::Client::builder().redirect(redirect_policy);
    // reqwest has no SOCKS4; those URLs go through a loopback SOCKS5 bridge.
    let bridged = match proxy_url {
        Some(url) if crate::socks4::parse(url).is_some() => Some(crate::socks4::start_bridge(url)?),
        _ => None,
    };
    let proxy_url = bridged.as_deref().or(proxy_url);
    // `socks5://` makes reqwest resolve the host itself and send the proxy an address; IDM sends the name
    // (SOCKS5 with ATYP 3), and a name lookup on our side would also leak where the download goes.
    let socks5h;
    let proxy_url = match proxy_url {
        Some(url) if url.starts_with("socks5://") => {
            socks5h = format!("socks5h://{}", &url["socks5://".len()..]);
            Some(socks5h.as_str())
        }
        other => other,
    };
    if proxy_url.is_none() {
        // Direct connections resolve names here (rebinding-safe); with a proxy, the proxy resolves.
        builder = builder.dns_resolver(std::sync::Arc::new(crate::dns::CachingResolver::global()));
    }
    if let Some(url) = proxy_url {
        builder = builder.proxy(
            reqwest::Proxy::all(url)
                .map_err(|e| anyhow::anyhow!("invalid proxy URL {url}: {e}"))?,
        );
    }
    Ok(builder.build()?)
}

/// Failed queue items are retried later (~20 s). Attempts are capped by
/// the `queue_auto_retry_max` setting (default 3, 0 = never); the failed item
/// goes to the back of the queue.
fn schedule_auto_retry(pool: SqlitePool, id: Uuid, cmd_tx: mpsc::Sender<ManagerCommand>) {
    tokio::spawn(async move {
        let max = db::settings::get(&pool, "queue_auto_retry_max")
            .await
            .ok()
            .flatten()
            .and_then(|v| v.trim().parse::<i64>().ok())
            .unwrap_or(3);
        let Ok(Some(record)) = db::downloads::get(&pool, id).await else {
            return;
        };
        if i64::from(record.error_count) > max {
            return;
        }
        let delay = if cfg!(test) { 100 } else { 20_000 };
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
        if db::downloads::requeue_at_end(&pool, id)
            .await
            .unwrap_or(false)
        {
            tracing::info!(%id, attempt = record.error_count, "auto-retrying failed download");
            let _ = cmd_tx.send(ManagerCommand::PumpQueue).await;
        }
    });
}

async fn load_global_bandwidth_limit_bps(pool: &SqlitePool) -> anyhow::Result<u64> {
    Ok(db::settings::get(pool, "bandwidth_limit_bps")
        .await?
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0))
}

fn effective_bandwidth_limit(
    global_limit: u64,
    queue_limit: Option<u64>,
    site_limit: Option<u64>,
) -> u64 {
    [
        nonzero_limit(global_limit),
        queue_limit.and_then(nonzero_limit),
        site_limit.and_then(nonzero_limit),
    ]
    .into_iter()
    .flatten()
    .min()
    .unwrap_or(0)
}

fn nonzero_limit(limit: u64) -> Option<u64> {
    (limit > 0).then_some(limit)
}

/// Decide which `Queued` downloads may start without exceeding any queue's
/// concurrency cap. Pure (no I/O) so the admission policy is unit-testable.
///
/// `queued` must already be in admission order (priority desc, age asc).
/// `running` maps active download id â†’ its queue. `caps` maps queue â†’ max
/// concurrent (missing â‡’ unlimited). Returns ids to start, in order.
fn select_admissions(
    queued: &[DownloadRecord],
    default_queue: Uuid,
    running: &HashMap<Uuid, Uuid>,
    caps: &HashMap<Uuid, u32>,
) -> Vec<Uuid> {
    let mut active: HashMap<Uuid, u32> = HashMap::new();
    for q in running.values() {
        *active.entry(*q).or_default() += 1;
    }

    let mut to_start = Vec::new();
    for rec in queued {
        if running.contains_key(&rec.id) {
            continue; // already running
        }
        let q = rec.queue_id.unwrap_or(default_queue);
        let cap = caps.get(&q).copied().unwrap_or(u32::MAX);
        let cur = active.entry(q).or_default();
        if *cur < cap {
            to_start.push(rec.id);
            *cur += 1;
        }
    }
    to_start
}

/// Pick a safe target path from an optional explicit filename, falling back to
/// the URL's last path segment, then a generic name. Shared by add and confirm.
/// The address a file is named after: where the redirects ended, like a browser and IDM.
fn naming_url(req: &DownloadRequest) -> &str {
    req.final_url
        .as_deref()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or(&req.url)
}

/// Last path segment of a URL, percent-decoded (`my%20file.zip` -> `my file.zip`).
fn name_from_url(url: &str) -> Option<String> {
    let segment = url::Url::parse(url)
        .ok()?
        .path_segments()?
        .next_back()?
        .to_string();
    if segment.is_empty() {
        return None;
    }
    Some(chunk::percent_decode(&segment).unwrap_or(segment))
}

/// A new download never takes a path that a file on disk or another download in the list already uses
/// (IDM asks or numbers the duplicate; the daemon cannot ask, so it numbers: `name_1.ext`, `name_2.ext`).
async fn free_save_path(
    pool: &SqlitePool,
    wanted: PathBuf,
    own_id: Option<Uuid>,
    overwrite: bool,
) -> PathBuf {
    let taken = |path: PathBuf| async move {
        // IDM's "add the duplicate and overwrite the existing file": a file on disk may be replaced, but never
        // one that another listed download is still writing.
        if !overwrite && tokio::fs::symlink_metadata(&path).await.is_ok() {
            return true;
        }
        let other: Option<String> = sqlx::query_scalar(
            "SELECT id FROM downloads WHERE lower(save_path) = lower(?1) AND id != ?2
               AND (?3 = 0 OR status NOT IN ('completed', 'cancelled')) LIMIT 1",
        )
        .bind(path.to_string_lossy().to_string())
        .bind(own_id.map(|id| id.to_string()).unwrap_or_default())
        .bind(i64::from(overwrite))
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();
        other.is_some()
    };
    if !taken(wanted.clone()).await {
        return wanted;
    }
    let stem = wanted.file_stem().map_or_else(
        || "download".to_string(),
        |s| s.to_string_lossy().to_string(),
    );
    let ext = wanted
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));
    for n in 1..10_000 {
        let candidate =
            wanted.with_file_name(format!("{stem}_{n}{}", ext.as_deref().unwrap_or("")));
        if !taken(candidate.clone()).await {
            return candidate;
        }
    }
    wanted
}

fn build_save_path(
    filename: Option<&str>,
    url: &str,
    save_dir: &Path,
) -> Result<PathBuf, FerricError> {
    let from_url = name_from_url(url);

    let raw_name = filename
        .map(str::to_string)
        .filter(|s| !s.trim().is_empty())
        .or(from_url)
        .unwrap_or_else(|| "download".into());

    security::safe_filename(&raw_name, save_dir)
}

fn normalized_method(method: &str) -> String {
    match method.to_ascii_uppercase().as_str() {
        "POST" => "POST".to_string(),
        _ => "GET".to_string(),
    }
}

fn decode_post_body(value: Option<&str>) -> anyhow::Result<Option<Vec<u8>>> {
    const MAX_POST_CAPTURE_BYTES: usize = 8 * 1024 * 1024;
    let Some(value) = value else {
        return Ok(None);
    };

    let bytes = base64::engine::general_purpose::STANDARD
        .decode(value)
        .context("decode POST body")?;
    if bytes.len() > MAX_POST_CAPTURE_BYTES {
        return Err(anyhow::anyhow!(
            "POST body capture exceeds {MAX_POST_CAPTURE_BYTES} bytes"
        ));
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capture_req(url: &str) -> DownloadRequest {
        DownloadRequest {
            id: Uuid::new_v4(),
            url: url.into(),
            final_url: None,
            filename: Some("file.bin".into()),
            referrer: None,
            cookies: Some("session=abc".into()),
            headers: None,
            size_hint: None,
            mime_type: None,
            method: "GET".into(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 0,
        }
    }

    async fn test_manager() -> (DownloadManager, SqlitePool, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init(&dir.path().join("ferric.db")).await.unwrap();
        let manager = DownloadManager::new(pool.clone()).await.unwrap();
        (manager, pool, dir)
    }

    #[test]
    fn effective_bandwidth_limit_uses_strictest_nonzero_limit() {
        assert_eq!(effective_bandwidth_limit(0, None, None), 0);
        assert_eq!(effective_bandwidth_limit(8_192, None, None), 8_192);
        assert_eq!(effective_bandwidth_limit(0, Some(2_048), None), 2_048);
        assert_eq!(effective_bandwidth_limit(8_192, Some(2_048), None), 2_048);
        assert_eq!(effective_bandwidth_limit(1_024, Some(2_048), None), 1_024);
        assert_eq!(effective_bandwidth_limit(1_024, Some(0), None), 1_024);
        // site rule is the strictest
        assert_eq!(
            effective_bandwidth_limit(8_192, Some(4_096), Some(512)),
            512
        );
        // global is strictest; site rule is ignored
        assert_eq!(
            effective_bandwidth_limit(256, Some(4_096), Some(1_024)),
            256
        );
        // site rule zero is treated as unlimited
        assert_eq!(effective_bandwidth_limit(8_192, None, Some(0)), 8_192);
    }

    #[test]
    fn held_capture_filename_gets_extension_from_mime() {
        let mut req = capture_req("https://example.com/d/qj0yjkadzh51");
        req.filename = Some("qj0yjkadzh51".into());
        req.mime_type = Some("application/vnd.rar".into());

        let req = with_mime_extension(req);

        assert_eq!(req.filename.as_deref(), Some("qj0yjkadzh51.rar"));
    }

    #[test]
    fn held_capture_filename_keeps_existing_extension() {
        let mut req = capture_req("https://example.com/d/archive.zip");
        req.filename = Some("archive.zip".into());
        req.mime_type = Some("application/vnd.rar".into());

        let req = with_mime_extension(req);

        assert_eq!(req.filename.as_deref(), Some("archive.zip"));
    }

    #[tokio::test]
    async fn manager_loads_global_bandwidth_limit_setting() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init(&dir.path().join("ferric.db")).await.unwrap();
        db::settings::set(&pool, "bandwidth_limit_bps", "8192")
            .await
            .unwrap();

        let manager = DownloadManager::new(pool.clone()).await.unwrap();

        assert_eq!(load_global_bandwidth_limit_bps(&pool).await.unwrap(), 8_192);
        assert_eq!(
            manager
                .effective_bandwidth_limit_for_queue(
                    db::queues::DEFAULT_QUEUE_ID,
                    "https://example.com/file.bin",
                )
                .await
                .unwrap(),
            8_192
        );
    }

    #[tokio::test]
    async fn queue_bandwidth_limit_can_override_global_limit() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init(&dir.path().join("ferric.db")).await.unwrap();
        db::settings::set(&pool, "bandwidth_limit_bps", "8192")
            .await
            .unwrap();
        db::queues::update(
            &pool,
            db::queues::DEFAULT_QUEUE_ID,
            db::queues::QueueUpdate {
                max_bandwidth: Some(Some(2_048)),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let manager = DownloadManager::new(pool.clone()).await.unwrap();

        assert_eq!(
            manager
                .effective_bandwidth_limit_for_queue(
                    db::queues::DEFAULT_QUEUE_ID,
                    "https://example.com/file.bin",
                )
                .await
                .unwrap(),
            2_048
        );
    }

    #[tokio::test]
    async fn site_rule_max_speed_bps_limits_job_bandwidth() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init(&dir.path().join("ferric.db")).await.unwrap();
        // global = 8 KB/s, no queue cap
        db::settings::set(&pool, "bandwidth_limit_bps", "8192")
            .await
            .unwrap();
        // site rule: 1 KB/s — strictest
        db::site_rules::insert_rule(
            &pool,
            "host:example.com",
            "max_speed_bps",
            Some("1024"),
            100,
        )
        .await
        .unwrap();

        let manager = DownloadManager::new(pool.clone()).await.unwrap();

        assert_eq!(
            manager
                .effective_bandwidth_limit_for_queue(
                    db::queues::DEFAULT_QUEUE_ID,
                    "https://example.com/file.bin",
                )
                .await
                .unwrap(),
            1_024,
            "site rule must win when it is the strictest limit"
        );
    }

    #[tokio::test]
    async fn global_limit_beats_site_rule_when_stricter() {
        let dir = tempfile::tempdir().unwrap();
        let pool = db::init(&dir.path().join("ferric.db")).await.unwrap();
        // global = 512 B/s — stricter than site rule
        db::settings::set(&pool, "bandwidth_limit_bps", "512")
            .await
            .unwrap();
        db::site_rules::insert_rule(
            &pool,
            "host:example.com",
            "max_speed_bps",
            Some("4096"),
            100,
        )
        .await
        .unwrap();

        let manager = DownloadManager::new(pool.clone()).await.unwrap();

        assert_eq!(
            manager
                .effective_bandwidth_limit_for_queue(
                    db::queues::DEFAULT_QUEUE_ID,
                    "https://example.com/file.bin",
                )
                .await
                .unwrap(),
            512,
            "global limit must win when it is the strictest"
        );
    }

    #[tokio::test]
    async fn per_download_proxy_setting_is_saved_and_overrides_global() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                Some("socks5://127.0.0.1:9050".to_string()),
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));

        let record = db::downloads::get(&pool, id).await?.unwrap();
        assert_eq!(
            record.proxy_url,
            Some("socks5://127.0.0.1:9050".to_string())
        );
        Ok(())
    }

    /// A held capture whose job is server-backed: the record and pending entry the dialog flow creates.
    async fn held_capture_at(
        manager: &DownloadManager,
        pool: &SqlitePool,
        dir: &Path,
        url: &str,
        name: &str,
    ) -> anyhow::Result<Uuid> {
        let id = Uuid::new_v4();
        let mut req = basic_req(id, url);
        req.filename = Some(name.to_string());
        db::downloads::insert(pool, &req, &dir.join(name).to_string_lossy(), "Other").await?;
        db::downloads::update_status(pool, id, DownloadStatus::Paused, None).await?;
        db::downloads::held_add(pool, id).await?;
        manager.pending_captures.lock().await.insert(
            id,
            PendingCapture {
                req,
                save_dir: dir.to_path_buf(),
                dir_overridden: true,
                cookies: None,
                headers: None,
                method: "GET".into(),
                post_body: None,
            },
        );
        Ok(id)
    }

    #[tokio::test]
    async fn capture_starts_while_the_dialog_is_open_and_a_rename_moves_the_file(
    ) -> anyhow::Result<()> {
        let data = Arc::new((0..90_000u64).map(|i| (i % 181) as u8).collect::<Vec<_>>());
        let server = spawn_recovery_range_server(data.clone()).await?;
        let (manager, pool, dir) = test_manager().await;
        let id = held_capture_at(&manager, &pool, dir.path(), &server.url, "file.bin").await?;

        manager.early_start(id).await?;
        wait_for_completion(&pool, id).await?;
        assert_eq!(tokio::fs::read(dir.path().join("file.bin")).await?, *data);
        assert!(
            manager.pending_captures.lock().await.contains_key(&id),
            "still held: the dialog has not answered"
        );

        // the dialog answers with a new name: the finished file is renamed
        let (tx, rx) = oneshot::channel();
        manager
            .handle_confirm(
                id,
                Some("renamed.bin".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));
        assert_eq!(
            tokio::fs::read(dir.path().join("renamed.bin")).await?,
            *data
        );
        assert!(tokio::fs::metadata(dir.path().join("file.bin"))
            .await
            .is_err());
        let record = db::downloads::get(&pool, id).await?.expect("record");
        assert_eq!(record.status, DownloadStatus::Completed);
        assert!(
            record.save_path.ends_with("renamed.bin"),
            "{}",
            record.save_path
        );
        assert!(!manager.pending_captures.lock().await.contains_key(&id));
        Ok(())
    }

    #[tokio::test]
    async fn a_rename_answered_while_the_transfer_runs_is_applied_when_it_ends(
    ) -> anyhow::Result<()> {
        let data = Arc::new((0..100_000u64).map(|i| (i % 173) as u8).collect::<Vec<_>>());
        let server = spawn_recovery_range_server(data.clone()).await?;
        let (manager, pool, dir) = test_manager().await;
        // about 50 KB/s: the transfer is still running when the dialog answers
        db::settings::set(&pool, "bandwidth_limit_bps", "50000").await?;
        let id = held_capture_at(&manager, &pool, dir.path(), &server.url, "slow.bin").await?;

        manager.early_start(id).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            manager.jobs.lock().await.contains_key(&id),
            "the job must still be running"
        );

        let (tx, rx) = oneshot::channel();
        manager
            .handle_confirm(
                id,
                Some("later.bin".into()),
                None,
                None,
                None,
                None,
                None,
                None,
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            let rec = db::downloads::get(&pool, id).await?.expect("record");
            if rec.status == DownloadStatus::Completed && rec.save_path.ends_with("later.bin") {
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "never moved; {:?} {}",
                rec.status,
                rec.save_path
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(tokio::fs::read(dir.path().join("later.bin")).await?, *data);
        assert!(tokio::fs::metadata(dir.path().join("slow.bin"))
            .await
            .is_err());
        Ok(())
    }

    #[tokio::test]
    async fn a_pending_move_survives_a_daemon_restart() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("ferric.db");
        let pool = db::init(&db_path).await?;
        let from = dir.path().join("old.bin");
        tokio::fs::write(&from, b"kept").await?;
        let id = Uuid::new_v4();
        db::downloads::insert(
            &pool,
            &basic_req(id, "http://example.test/a"),
            &from.to_string_lossy(),
            "Other",
        )
        .await?;
        let to = dir.path().join("new.bin");
        db::downloads::pending_move_set(&pool, id, &to.to_string_lossy(), "new.bin").await?;
        // a new manager over the same database = the daemon started again
        let _manager = DownloadManager::new(pool.clone()).await?;
        assert_eq!(tokio::fs::read(&to).await?, b"kept");
        assert!(tokio::fs::metadata(&from).await.is_err());
        assert!(db::downloads::pending_move_ids(&pool).await?.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn a_proxy_chosen_in_the_dialog_restarts_the_running_download() -> anyhow::Result<()> {
        let data = Arc::new(vec![3_u8; 100_000]);
        let server = spawn_recovery_range_server(data.clone()).await?;
        let (manager, pool, dir) = test_manager().await;
        db::settings::set(&pool, "bandwidth_limit_bps", "50000").await?;
        let id = held_capture_at(&manager, &pool, dir.path(), &server.url, "px.bin").await?;
        manager.early_start(id).await?;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(manager.jobs.lock().await.contains_key(&id));

        let (tx, rx) = oneshot::channel();
        manager
            .handle_confirm(
                id,
                None,
                None,
                None,
                None,
                None,
                None,
                Some("socks5://127.0.0.1:9".into()),
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));
        assert!(
            !manager.early_started.lock().await.contains_key(&id),
            "the early job was dropped and the download went through the normal start"
        );
        let record = db::downloads::get(&pool, id).await?.expect("record");
        assert_eq!(record.proxy_url.as_deref(), Some("socks5://127.0.0.1:9"));
        Ok(())
    }

    #[tokio::test]
    async fn early_start_never_writes_into_an_existing_file() -> anyhow::Result<()> {
        let data = Arc::new(vec![9_u8; 60_000]);
        let server = spawn_recovery_range_server(data.clone()).await?;
        let (manager, pool, dir) = test_manager().await;
        tokio::fs::write(dir.path().join("file.bin"), b"precious").await?;
        let id = held_capture_at(&manager, &pool, dir.path(), &server.url, "file.bin").await?;

        manager.early_start(id).await?;
        wait_for_completion(&pool, id).await?;
        assert_eq!(
            tokio::fs::read(dir.path().join("file.bin")).await?,
            b"precious"
        );
        assert_eq!(tokio::fs::read(dir.path().join("file_1.bin")).await?, *data);
        Ok(())
    }

    #[tokio::test]
    async fn cancelling_the_dialog_removes_what_was_already_downloaded() -> anyhow::Result<()> {
        let data = Arc::new(vec![5_u8; 50_000]);
        let server = spawn_recovery_range_server(data.clone()).await?;
        let (manager, pool, dir) = test_manager().await;
        let id = held_capture_at(&manager, &pool, dir.path(), &server.url, "gone.bin").await?;

        manager.early_start(id).await?;
        wait_for_completion(&pool, id).await?;
        assert!(tokio::fs::metadata(dir.path().join("gone.bin"))
            .await
            .is_ok());

        // the dialog's Cancel sends a plain remove (no file deletion requested)
        manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: false,
            })
            .await?;
        assert!(tokio::fs::metadata(dir.path().join("gone.bin"))
            .await
            .is_err());
        assert!(db::downloads::get(&pool, id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn a_move_chosen_while_the_job_runs_happens_when_it_ends() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let from = dir.path().join("a.bin");
        tokio::fs::write(&from, b"data").await?;
        let id = Uuid::new_v4();
        db::downloads::insert(
            &pool,
            &basic_req(id, "http://example.test/a"),
            &from.to_string_lossy(),
            "Other",
        )
        .await?;
        let to = dir.path().join("sub").join("b.bin");
        db::downloads::pending_move_set(&pool, id, &to.to_string_lossy(), "b.bin").await?;
        DownloadManager::apply_pending_move(&pool, &manager.progress_tx, id).await;
        assert_eq!(tokio::fs::read(&to).await?, b"data");
        assert!(tokio::fs::metadata(&from).await.is_err());
        let record = db::downloads::get(&pool, id).await?.expect("record");
        assert_eq!(record.filename, "b.bin");
        assert!(record.save_path.ends_with("b.bin"));
        Ok(())
    }

    #[tokio::test]
    async fn held_add_inserts_paused_and_does_not_spawn() {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                tx,
            )
            .await
            .unwrap();

        assert!(matches!(rx.await.unwrap(), DaemonReply::Accepted { .. }));
        let record = db::downloads::get(&pool, id).await.unwrap().unwrap();
        assert_eq!(record.status, DownloadStatus::Paused);
        assert!(manager.pending_captures.lock().await.contains_key(&id));
        assert!(
            manager.jobs.lock().await.is_empty(),
            "held add must not spawn"
        );
    }

    #[tokio::test]
    async fn held_capture_is_announced_listed_and_download_later_keeps_it_paused(
    ) -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let mut events = manager.progress_tx.subscribe();
        let req = capture_req("https://example.com/report.pdf");
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));

        let mut announced = false;
        while let Ok(event) = events.try_recv() {
            if matches!(event, ProgressEvent::CaptureHeld { ref record } if record.id == id) {
                announced = true;
            }
        }
        assert!(
            announced,
            "the UI needs a CaptureHeld event to open the dialog"
        );
        assert!(manager.pending_captures.lock().await.contains_key(&id));

        // "Download Later": stays in the list, paused, with the dialog's description and category.
        let (tx, rx) = oneshot::channel();
        manager
            .handle_confirm_ext(
                id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                ConfirmExtras {
                    start: false,
                    description: Some("for later".into()),
                    category: Some("Documents".into()),
                },
                tx,
            )
            .await?;
        assert!(matches!(rx.await?, DaemonReply::Accepted { .. }));
        let record = db::downloads::get(&pool, id).await?.unwrap();
        assert_eq!(record.status, DownloadStatus::Paused);
        assert_eq!(record.description.as_deref(), Some("for later"));
        assert_eq!(record.category, "Documents");
        assert!(
            manager.pending_captures.lock().await.is_empty(),
            "no longer waiting"
        );
        assert!(
            manager.jobs.lock().await.is_empty(),
            "later must not start it"
        );
        Ok(())
    }

    #[tokio::test]
    async fn captures_left_over_from_a_previous_run_are_dropped() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/orphan.bin");
        let id = req.id;
        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                tx,
            )
            .await?;
        rx.await?;
        assert!(db::downloads::get(&pool, id).await?.is_some());

        // A restart loses the in-memory credentials, so the unconfirmed row can never start.
        assert_eq!(db::downloads::purge_held(&pool).await?, 1);
        assert!(db::downloads::get(&pool, id).await?.is_none());
        assert_eq!(db::downloads::purge_held(&pool).await?, 0);
        Ok(())
    }

    #[tokio::test]
    async fn confirm_capture_can_switch_to_selected_media_variant() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        db::queues::update(
            &pool,
            db::queues::DEFAULT_QUEUE_ID,
            db::queues::QueueUpdate {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await?;

        let mut req = capture_req("https://cdn.example.com/master.m3u8");
        req.mime_type = Some("application/vnd.apple.mpegurl".into());
        let id = req.id;
        let variant_url = "https://cdn.example.com/720p/index.m3u8".to_string();

        let (add_tx, add_rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                add_tx,
            )
            .await?;
        assert!(matches!(add_rx.await?, DaemonReply::Accepted { .. }));

        let (confirm_tx, confirm_rx) = oneshot::channel();
        manager
            .handle_confirm(
                id,
                Some("ferric-720p.m3u8".into()),
                None,
                Some(variant_url.clone()),
                None,
                None,
                None,
                None,
                confirm_tx,
            )
            .await?;
        assert!(matches!(
            confirm_rx.await?,
            DaemonReply::Accepted { id: accepted } if accepted == id
        ));

        let record = db::downloads::get(&pool, id).await?.unwrap();
        assert_eq!(record.url, variant_url);
        assert_eq!(record.final_url, None);
        assert_eq!(record.filename, "ferric-720p.m3u8");
        assert_eq!(record.status, DownloadStatus::Queued);
        assert!(manager.pending_captures.lock().await.get(&id).is_none());
        assert!(manager.pending_creds.lock().await.get(&id).is_some());
        assert!(manager.jobs.lock().await.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn remove_clears_pending_capture() {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                tx,
            )
            .await
            .unwrap();
        rx.await.unwrap();

        manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: false,
            })
            .await
            .unwrap();

        assert!(!manager.pending_captures.lock().await.contains_key(&id));
        assert!(db::downloads::get(&pool, id).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn cancel_clears_transient_capture_secrets() {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;

        let (tx, rx) = oneshot::channel();
        manager
            .handle_add(
                req,
                Some(dir.path().to_path_buf()),
                true,
                None,
                None,
                None,
                None,
                tx,
            )
            .await
            .unwrap();
        rx.await.unwrap();
        manager.pending_creds.lock().await.insert(
            id,
            StartCreds {
                cookies: Some("sid=secret".to_string()),
                headers: Some(HashMap::from([(
                    "authorization".to_string(),
                    "Bearer secret".to_string(),
                )])),
                method: "POST".to_string(),
                post_body: Some(b"token=secret".to_vec()),
            },
        );

        manager.handle(ManagerCommand::Cancel(id)).await.unwrap();

        assert!(!manager.pending_captures.lock().await.contains_key(&id));
        assert!(!manager.pending_creds.lock().await.contains_key(&id));
        let record = db::downloads::get(&pool, id).await.unwrap().unwrap();
        assert_eq!(record.status, DownloadStatus::Cancelled);
    }

    #[tokio::test]
    async fn remove_with_delete_file_removes_saved_file() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;
        let save_path = dir.path().join("file.bin");
        tokio::fs::write(&save_path, b"partial or complete bytes").await?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: true,
            })
            .await?;

        assert!(
            tokio::fs::metadata(&save_path).await.is_err(),
            "delete_file=true must remove the saved file"
        );
        assert!(db::downloads::get(&pool, id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn redownload_discards_old_bytes_and_progress() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("http://127.0.0.1:9/file.bin");
        let id = req.id;
        let save_path = dir.path().join("file.bin");
        tokio::fs::write(&save_path, b"old complete bytes").await?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        db::downloads::update_progress(&pool, id, 18).await?;
        db::downloads::update_status(&pool, id, DownloadStatus::Completed, None).await?;

        manager.handle(ManagerCommand::Redownload(id)).await?;

        let record = db::downloads::get(&pool, id).await?.expect("row is kept");
        assert_eq!(record.downloaded, 0);
        assert_ne!(record.status, DownloadStatus::Completed);
        let left = tokio::fs::read(&save_path).await.unwrap_or_default();
        assert_ne!(left, b"old complete bytes", "the old file must not survive");
        Ok(())
    }

    #[tokio::test]
    async fn remove_without_delete_file_keeps_saved_file() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;
        let save_path = dir.path().join("file.bin");
        tokio::fs::write(&save_path, b"keep me").await?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: false,
            })
            .await?;

        assert_eq!(tokio::fs::read(&save_path).await?, b"keep me");
        assert!(db::downloads::get(&pool, id).await?.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn remove_with_delete_file_refuses_directory() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;
        let save_path = dir.path().join("not-a-file");
        tokio::fs::create_dir(&save_path).await?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        let err = manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: true,
            })
            .await
            .expect_err("delete_file=true must refuse directories");

        assert!(err.to_string().contains("refusing to delete non-file path"));
        assert!(tokio::fs::metadata(&save_path).await?.is_dir());
        assert!(
            db::downloads::get(&pool, id).await?.is_some(),
            "record should remain when file deletion is refused"
        );
        Ok(())
    }

    #[tokio::test]
    async fn remove_with_delete_file_allows_missing_saved_file() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;
        let save_path = dir.path().join("already-gone.bin");
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: true,
            })
            .await?;

        assert!(db::downloads::get(&pool, id).await?.is_none());
        Ok(())
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remove_with_delete_file_refuses_symlink() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        let req = capture_req("https://example.com/file.bin");
        let id = req.id;
        let target = dir.path().join("target.bin");
        let save_path = dir.path().join("link.bin");
        tokio::fs::write(&target, b"must survive").await?;
        std::os::unix::fs::symlink(&target, &save_path)?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        let err = manager
            .handle(ManagerCommand::Remove {
                id,
                delete_file: true,
            })
            .await
            .expect_err("delete_file=true must refuse symlinks");

        assert!(err.to_string().contains("refusing to delete symlink"));
        assert_eq!(tokio::fs::read(&target).await?, b"must survive");
        assert!(db::downloads::get(&pool, id).await?.is_some());
        Ok(())
    }

    #[tokio::test]
    async fn new_download_never_shares_a_path_with_a_file_or_another_download() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let wanted = dir.path().join("a.bin");
        assert_eq!(
            free_save_path(&pool, wanted.clone(), None, false).await,
            wanted
        );
        tokio::fs::write(&wanted, b"x").await?;
        assert_eq!(
            free_save_path(&pool, wanted.clone(), None, false).await,
            dir.path().join("a_1.bin")
        );
        // a listed download that has not written its file yet also holds its name
        let req = DownloadRequest {
            id: Uuid::new_v4(),
            url: "http://example.test/a".into(),
            final_url: None,
            filename: Some("a_1.bin".into()),
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
        db::downloads::insert(
            &pool,
            &req,
            &dir.path().join("a_1.bin").to_string_lossy(),
            "Other",
        )
        .await?;
        assert_eq!(
            free_save_path(&pool, wanted.clone(), None, false).await,
            dir.path().join("a_2.bin")
        );
        // overwrite: the file on disk may be replaced, but a download still in the list keeps its name
        assert_eq!(
            free_save_path(&pool, wanted.clone(), None, true).await,
            wanted
        );
        assert_eq!(
            free_save_path(&pool, dir.path().join("a_1.bin"), None, true).await,
            dir.path().join("a_1_1.bin")
        );
        // the download itself is not its own duplicate
        assert_eq!(
            free_save_path(&pool, dir.path().join("a_1.bin"), Some(req.id), false).await,
            dir.path().join("a_1.bin")
        );
        Ok(())
    }

    #[test]
    fn build_save_path_rejects_filename_path_component() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let err = build_save_path(
            Some("..\\outside\\payload.exe"),
            "https://example.com/payload.exe",
            dir.path(),
        )
        .expect_err("filename path components must be rejected");

        assert!(matches!(err, FerricError::PathTraversal { .. }));
        Ok(())
    }

    #[test]
    fn names_come_from_the_final_address_decoded_with_an_extension_from_the_type() {
        // (redirects name the file after the final URL; %20 is decoded;
        // `/download` served as application/zip becomes download.zip)
        let mut req = capture_req("http://h.test/redir/chain");
        req.filename = None;
        req.final_url = Some("http://h.test/plain/my%20report.pdf?x=1".into());
        assert_eq!(naming_url(&req), "http://h.test/plain/my%20report.pdf?x=1");
        assert_eq!(
            name_from_url(naming_url(&req)).as_deref(),
            Some("my report.pdf")
        );

        let mut req = capture_req("http://h.test/noext/download");
        req.filename = None;
        req.mime_type = Some("application/zip".into());
        assert_eq!(
            with_mime_extension(req).filename.as_deref(),
            Some("download.zip")
        );

        let mut req = capture_req("http://h.test/a/file.bin");
        req.filename = None;
        req.mime_type = Some("application/zip".into());
        assert_eq!(
            with_mime_extension(req).filename,
            None,
            "a name that already has an extension is left alone"
        );
    }

    #[tokio::test]
    async fn confirm_without_pending_capture_errors() {
        let (manager, _pool, _dir) = test_manager().await;
        let (tx, rx) = oneshot::channel();
        manager
            .handle_confirm(Uuid::new_v4(), None, None, None, None, None, None, None, tx)
            .await
            .unwrap();
        assert!(matches!(rx.await.unwrap(), DaemonReply::Error { .. }));
    }

    // ── Queue admission (select_admissions) ──────────────────────────────────

    fn queued_rec(queue_id: Option<Uuid>, priority: i32) -> DownloadRecord {
        DownloadRecord {
            id: Uuid::new_v4(),
            url: "https://example.com/f.bin".into(),
            final_url: None,
            redirect_chain: None,
            filename: "f.bin".into(),
            save_path: "f.bin".into(),
            status: DownloadStatus::Queued,
            total_size: None,
            downloaded: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
            error_msg: None,
            error_count: 0,
            referrer: None,
            mime_type: None,
            etag: None,
            last_modified: None,
            category: "Other".into(),
            queue_id,
            priority,
            checksum: None,
            checksum_ok: None,
            speed_limit_bps: 0,
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            mirrors: vec![],
            description: None,
        }
    }

    #[test]
    fn admissions_respect_cap_and_input_order() {
        let dq = db::queues::DEFAULT_QUEUE_ID;
        // Caller passes them already ordered (priority desc); we preserve order.
        let queued = vec![
            queued_rec(None, 10),
            queued_rec(None, 5),
            queued_rec(None, 1),
        ];
        let caps = HashMap::from([(dq, 2)]);
        let running = HashMap::new();

        let started = select_admissions(&queued, dq, &running, &caps);
        assert_eq!(started, vec![queued[0].id, queued[1].id], "cap=2, top two");
    }

    #[test]
    fn admissions_start_higher_priority_first() {
        let dq = db::queues::DEFAULT_QUEUE_ID;
        let high = queued_rec(None, 10);
        let low = queued_rec(None, 1);
        let queued = vec![high.clone(), low];
        let caps = HashMap::from([(dq, 1)]);
        let running = HashMap::new();

        let started = select_admissions(&queued, dq, &running, &caps);
        assert_eq!(started, vec![high.id]);
    }

    #[test]
    fn admissions_account_for_running_and_skip_already_running() {
        let dq = db::queues::DEFAULT_QUEUE_ID;
        let queued = vec![queued_rec(None, 1), queued_rec(None, 1)];
        let caps = HashMap::from([(dq, 2)]);
        // One default-queue slot already taken â†’ only one more may start.
        let running = HashMap::from([(Uuid::new_v4(), dq)]);

        let started = select_admissions(&queued, dq, &running, &caps);
        assert_eq!(started, vec![queued[0].id]);

        // A record that is itself already running is never re-admitted.
        let mut queued2 = queued.clone();
        let running2 = HashMap::from([(queued2[0].id, dq)]);
        let started2 = select_admissions(&queued2, dq, &running2, &caps);
        assert_eq!(started2, vec![queued2.remove(1).id]);
    }

    #[test]
    fn admissions_are_per_queue() {
        let dq = db::queues::DEFAULT_QUEUE_ID;
        let other = Uuid::from_u128(2);
        let queued = vec![
            queued_rec(Some(dq), 1),
            queued_rec(Some(dq), 1),
            queued_rec(Some(other), 1),
        ];
        let caps = HashMap::from([(dq, 1), (other, 1)]);
        let running = HashMap::new();

        let started = select_admissions(&queued, dq, &running, &caps);
        // One from each queue (caps are independent), not two from the default.
        assert_eq!(started, vec![queued[0].id, queued[2].id]);
    }

    #[tokio::test]
    async fn download_now_starts_even_when_every_queue_slot_is_busy() {
        let (manager, pool, dir) = test_manager().await;
        let dq = db::queues::DEFAULT_QUEUE_ID; // default cap = 2
        {
            let mut jobs = manager.jobs.lock().await;
            for _ in 0..2 {
                let fake = Uuid::new_v4();
                let (tx, _rx) = watch::channel(false);
                jobs.insert(
                    fake,
                    JobHandle::new(fake, dq, tx, Arc::new(BandwidthLimiter::new(0))),
                );
            }
        }
        let req = capture_req("http://127.0.0.1:9/f.bin");
        let id = req.id;
        let path = dir.path().join("f.bin");
        db::downloads::insert(&pool, &req, &path.to_string_lossy(), "Other")
            .await
            .unwrap();
        db::downloads::update_status(&pool, id, DownloadStatus::Paused, None)
            .await
            .unwrap();

        manager.pump_queue().await.unwrap();
        assert!(
            !manager.jobs.lock().await.contains_key(&id),
            "a plain resume would wait for a slot"
        );

        manager.handle(ManagerCommand::StartNow(id)).await.unwrap();
        assert!(
            manager.jobs.lock().await.contains_key(&id),
            "Download Now bypasses the cap"
        );
        assert_eq!(manager.jobs.lock().await.len(), 3);
    }

    #[tokio::test]
    async fn pump_queue_holds_when_all_slots_busy() {
        let (manager, pool, dir) = test_manager().await;
        let dq = db::queues::DEFAULT_QUEUE_ID; // default cap = 2

        // Occupy both default-queue slots with placeholder handles.
        {
            let mut jobs = manager.jobs.lock().await;
            for _ in 0..2 {
                let fake = Uuid::new_v4();
                let (tx, _rx) = watch::channel(false);
                jobs.insert(
                    fake,
                    JobHandle::new(fake, dq, tx, Arc::new(BandwidthLimiter::new(0))),
                );
            }
        }

        // Enqueue three real downloads.
        let mut ids = Vec::new();
        for _ in 0..3 {
            let req = capture_req("https://example.com/f.bin");
            ids.push(req.id);
            let path = dir.path().join(format!("{}.bin", req.id));
            db::downloads::insert(&pool, &req, &path.to_string_lossy(), "Other")
                .await
                .unwrap();
        }

        manager.pump_queue().await.unwrap();

        // Cap is full, so nothing new spawned and all three stay queued.
        assert_eq!(manager.jobs.lock().await.len(), 2, "no new jobs admitted");
        for id in ids {
            let rec = db::downloads::get(&pool, id).await.unwrap().unwrap();
            assert_eq!(rec.status, DownloadStatus::Queued);
        }
    }

    #[tokio::test]
    async fn spawn_job_leaves_no_stale_handle_when_bandwidth_lookup_fails() {
        let (manager, pool, _dir) = test_manager().await;
        let req = capture_req("https://example.com/f.bin");
        let record = queued_rec(None, 0);

        // Close the pool so effective_bandwidth_limit_for_queue → db::queues::get fails.
        pool.close().await;

        let result = manager.spawn_job(record, StartCreds::default()).await;
        assert!(result.is_err(), "closed pool must cause spawn_job to fail");
        assert!(
            manager.jobs.lock().await.is_empty(),
            "bandwidth failure must not leave a stale job handle"
        );
        // Suppress unused-variable warning for req.
        drop(req);
    }

    #[tokio::test]
    async fn pump_queue_at_respects_schedule_window() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;

        // Narrow window: active only from minute 100 to 200.
        db::queues::update(
            &pool,
            db::queues::DEFAULT_QUEUE_ID,
            db::queues::QueueUpdate {
                sched_start: Some(Some(100)),
                sched_end: Some(Some(200)),
                ..Default::default()
            },
        )
        .await?;

        let req = capture_req("https://example.com/f.bin");
        let id = req.id;
        let path = dir.path().join("sched.bin");
        db::downloads::insert(&pool, &req, &path.to_string_lossy(), "Other").await?;

        // Minute 50 is outside [100, 200) → cap = 0 → nothing admitted.
        manager.pump_queue_at(50).await?;
        assert!(
            manager.jobs.lock().await.is_empty(),
            "closed window (minute 50): download must stay queued"
        );
        let rec = db::downloads::get(&pool, id).await?.unwrap();
        assert_eq!(rec.status, DownloadStatus::Queued);

        // Minute 150 is inside [100, 200) → cap = 2 → download admitted.
        manager.pump_queue_at(150).await?;
        assert!(
            !manager.jobs.lock().await.is_empty(),
            "open window (minute 150): download must be admitted"
        );

        // Tidy up.
        manager.cancel_job(id).await;
        Ok(())
    }

    #[tokio::test]
    async fn pump_queue_holds_when_queue_disabled() {
        let (manager, pool, dir) = test_manager().await;
        db::queues::update(
            &pool,
            db::queues::DEFAULT_QUEUE_ID,
            db::queues::QueueUpdate {
                enabled: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        let req = capture_req("https://example.com/f.bin");
        let id = req.id;
        let path = dir.path().join("disabled.bin");
        db::downloads::insert(&pool, &req, &path.to_string_lossy(), "Other")
            .await
            .unwrap();

        manager.pump_queue().await.unwrap();

        assert!(manager.jobs.lock().await.is_empty());
        let rec = db::downloads::get(&pool, id).await.unwrap().unwrap();
        assert_eq!(rec.status, DownloadStatus::Queued);
    }

    // ── Crash-recovery tests ──────────────────────────────────────────────────

    use crate::chunk::split_into_chunks;
    use crate::disk::DiskWriter;
    use ferric_common::ChunkStatus;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    struct RecoveryServer {
        url: String,
    }

    fn parse_range_header(req: &str) -> Option<(usize, usize)> {
        let line = req
            .lines()
            .find(|l| l.to_ascii_lowercase().starts_with("range:"))?;
        let val = line.split_once(':')?.1.trim().strip_prefix("bytes=")?;
        let mut parts = val.splitn(2, '-');
        let start: usize = parts.next()?.trim().parse().ok()?;
        let end: usize = parts.next()?.split_whitespace().next()?.parse().ok()?;
        Some((start, end))
    }

    async fn spawn_recovery_range_server(data: Arc<Vec<u8>>) -> anyhow::Result<RecoveryServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let Ok(n) = stream.read(&mut buf).await else {
                        return;
                    };
                    let req = String::from_utf8_lossy(&buf[..n]);
                    let resp = if req.starts_with("HEAD ") {
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes()
                    } else if let Some((start, end)) = parse_range_header(&req) {
                        let end = end.min(data.len().saturating_sub(1));
                        let body = &data[start..=end];
                        let mut r = format!(
                            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\nConnection: close\r\n\r\n",
                            body.len(),
                            start,
                            end,
                            data.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(body);
                        r
                    } else {
                        let mut r = format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nAccept-Ranges: bytes\r\nConnection: close\r\n\r\n",
                            data.len()
                        )
                        .into_bytes();
                        r.extend_from_slice(&data);
                        r
                    };
                    let _ = stream.write_all(&resp).await;
                });
            }
        });
        Ok(RecoveryServer {
            url: format!("http://{addr}/file.bin"),
        })
    }

    async fn spawn_recovery_no_range_server(data: Arc<Vec<u8>>) -> anyhow::Result<RecoveryServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let mut resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    )
                    .into_bytes();
                    resp.extend_from_slice(&data);
                    let _ = stream.write_all(&resp).await;
                });
            }
        });
        Ok(RecoveryServer {
            url: format!("http://{addr}/file.bin"),
        })
    }

    fn basic_req(id: Uuid, url: &str) -> DownloadRequest {
        DownloadRequest {
            id,
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
            timestamp: 0,
        }
    }

    async fn wait_for_completion(pool: &SqlitePool, id: Uuid) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let rec = db::downloads::get(pool, id)
                .await?
                .expect("record must exist");
            if rec.status == DownloadStatus::Completed {
                return Ok(());
            }
            if matches!(
                rec.status,
                DownloadStatus::Error | DownloadStatus::Cancelled
            ) {
                anyhow::bail!("download reached {:?}: {:?}", rec.status, rec.error_msg);
            }
            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timeout: download still {:?} after 10 s", rec.status);
            }
            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        }
    }

    /// Accepts connections but drops the first `fail_first` of them unanswered,
    /// then serves `data` (plain 200, no ranges) to everyone.
    async fn spawn_flaky_server(
        data: Arc<Vec<u8>>,
        fail_first: usize,
    ) -> anyhow::Result<RecoveryServer> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;
        let seen = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let data = data.clone();
                let n = seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    if n < fail_first {
                        return; // drop the socket without a response
                    }
                    let mut resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        data.len()
                    )
                    .into_bytes();
                    resp.extend_from_slice(&data);
                    let _ = stream.write_all(&resp).await;
                });
            }
        });
        Ok(RecoveryServer {
            url: format!("http://{addr}/file.bin"),
        })
    }

    /// A transient failure is retried automatically and the item ends up
    /// completed without any user action (IDM re-queues failed files).
    #[tokio::test]
    async fn failed_download_is_retried_automatically() -> anyhow::Result<()> {
        let data = Arc::new((0..40_000u64).map(|i| (i % 199) as u8).collect::<Vec<_>>());
        // HEAD + its immediate retry both fail (attempt 1); the next attempt succeeds.
        let server = spawn_flaky_server(data.clone(), 3).await?;
        let (manager, pool, dir) = test_manager().await;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("retry.bin");
        db::downloads::insert(
            &pool,
            &basic_req(id, &server.url),
            &save_path.to_string_lossy(),
            "Other",
        )
        .await?;
        manager.pump_queue().await?;
        let runner = tokio::spawn(manager.run());

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            let rec = db::downloads::get(&pool, id).await?.expect("record");
            if rec.status == DownloadStatus::Completed {
                assert!(rec.error_count >= 1, "it must have failed at least once");
                break;
            }
            anyhow::ensure!(
                tokio::time::Instant::now() < deadline,
                "never completed; last status {:?}",
                rec.status
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert_eq!(tokio::fs::read(&save_path).await?, *data);
        runner.abort();
        Ok(())
    }

    /// Queued files start one at a time, `queue_start_spacing_ms` apart.
    #[tokio::test]
    async fn queued_downloads_start_with_spacing() -> anyhow::Result<()> {
        let data = Arc::new(vec![7_u8; 20_000]);
        let server = spawn_flaky_server(data.clone(), 0).await?;
        let (manager, pool, dir) = test_manager().await;
        db::settings::set(&pool, "queue_start_spacing_ms", "700").await?;
        let mut ids = Vec::new();
        for n in 0..3 {
            let id = Uuid::new_v4();
            ids.push(id);
            db::downloads::insert(
                &pool,
                &basic_req(id, &server.url),
                &dir.path().join(format!("s{n}.bin")).to_string_lossy(),
                "Other",
            )
            .await?;
            tokio::time::sleep(std::time::Duration::from_millis(5)).await; // distinct created_at
        }
        manager.pump_queue().await?;
        let runner = tokio::spawn(manager.run());
        let begun = tokio::time::Instant::now();
        let mut started_at: Vec<Option<std::time::Duration>> = vec![None; 3];
        let deadline = begun + std::time::Duration::from_secs(10);
        while started_at.iter().any(Option::is_none) && tokio::time::Instant::now() < deadline {
            for (i, id) in ids.iter().enumerate() {
                let rec = db::downloads::get(&pool, *id).await?.expect("record");
                if started_at[i].is_none() && rec.status != DownloadStatus::Queued {
                    started_at[i] = Some(begun.elapsed());
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        runner.abort();
        let t: Vec<_> = started_at
            .into_iter()
            .map(|d| d.expect("started"))
            .collect();
        assert!(
            t[0] < std::time::Duration::from_millis(500),
            "first start is immediate: {t:?}"
        );
        assert!(
            t[1] >= std::time::Duration::from_millis(600),
            "second start must wait: {t:?}"
        );
        assert!(
            t[2] >= std::time::Duration::from_millis(1300),
            "third start must wait: {t:?}"
        );
        Ok(())
    }

    /// With `queue_auto_retry_max = 0` a failure stays failed.
    #[tokio::test]
    async fn auto_retry_can_be_disabled() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;
        db::settings::set(&pool, "queue_auto_retry_max", "0").await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("off.bin");
        // Connection refused on every attempt.
        db::downloads::insert(
            &pool,
            &basic_req(id, "http://127.0.0.1:1/f.bin"),
            &save_path.to_string_lossy(),
            "Other",
        )
        .await?;
        manager.pump_queue().await?;
        let runner = tokio::spawn(manager.run());
        // Windows takes a few seconds to report a refused connection.
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(15);
        let rec = loop {
            let rec = db::downloads::get(&pool, id).await?.expect("record");
            if rec.status == DownloadStatus::Error || tokio::time::Instant::now() >= deadline {
                break rec;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        // Leave time for a (wrongly) scheduled retry to show up.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        let rec = db::downloads::get(&pool, id).await?.unwrap_or(rec);
        assert_eq!(rec.status, DownloadStatus::Error);
        assert_eq!(rec.error_count, 1, "no automatic second attempt expected");
        runner.abort();
        Ok(())
    }

    /// Verifies that `recover_incomplete` resets in-flight chunk state before
    /// re-queuing, so the resumed job can calculate the correct byte offset.
    /// Uses no HTTP server — only checks the synchronous DB mutations that happen
    /// before `pump_queue` is called.
    #[tokio::test]
    async fn recover_incomplete_resets_chunks_and_re_queues() -> anyhow::Result<()> {
        let (manager, pool, dir) = test_manager().await;

        // Two Downloading records (port 1 — connection refused; jobs will error
        // asynchronously, but chunk state is set before pump_queue is called).
        let req1 = basic_req(Uuid::new_v4(), "http://127.0.0.1:1/f1.bin");
        let req2 = basic_req(Uuid::new_v4(), "http://127.0.0.1:1/f2.bin");
        let (id1, id2) = (req1.id, req2.id);
        db::downloads::insert(
            &pool,
            &req1,
            &dir.path().join("f1").to_string_lossy(),
            "Other",
        )
        .await?;
        db::downloads::insert(
            &pool,
            &req2,
            &dir.path().join("f2").to_string_lossy(),
            "Other",
        )
        .await?;
        db::downloads::update_status(&pool, id1, DownloadStatus::Downloading, None).await?;
        db::downloads::update_status(&pool, id2, DownloadStatus::Downloading, None).await?;

        // Record 1: chunk 0 Completed, chunk 1 Downloading with 5 KB written.
        let ranges = split_into_chunks(100_000, 2);
        db::chunks::insert_chunks(&pool, id1, &ranges).await?;
        let chunks = db::chunks::get_chunks(&pool, id1).await?;
        db::chunks::update_chunk_progress(
            &pool,
            chunks[0].id,
            ranges[0].byte_end - ranges[0].byte_start + 1,
            ChunkStatus::Completed,
        )
        .await?;
        db::chunks::update_chunk_progress(&pool, chunks[1].id, 5_000, ChunkStatus::Downloading)
            .await?;

        assert_eq!(
            db::downloads::find_incomplete(&pool).await?.len(),
            2,
            "both records must appear as incomplete before recovery"
        );

        // recover_incomplete resets chunks synchronously, before spawning jobs.
        manager.recover_incomplete().await?;

        let chunks_after = db::chunks::get_chunks(&pool, id1).await?;
        assert_eq!(
            chunks_after[0].status,
            ChunkStatus::Completed,
            "completed chunk must not be touched by recovery"
        );
        assert_eq!(
            chunks_after[1].status,
            ChunkStatus::Pending,
            "downloading chunk must be reset to pending so the job can resume it"
        );
        assert_eq!(
            chunks_after[1].downloaded, 5_000,
            "partial downloaded byte count must be preserved for byte-accurate resume"
        );

        drop((id2, dir));
        Ok(())
    }

    /// Kills a segmented download mid-flight by forcing DB state to `Downloading`
    /// with partial chunks and a partial file on disk. A fresh manager must resume
    /// from the correct byte offset and produce a file identical to the source.
    #[tokio::test]
    async fn segmented_download_completes_after_simulated_crash() -> anyhow::Result<()> {
        let data = Arc::new(
            (0..1_048_576u64)
                .map(|i| (i % 251) as u8)
                .collect::<Vec<_>>(),
        );
        let server = spawn_recovery_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("recovery-seg.bin");
        let req = basic_req(id, &server.url);
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;

        // Set up 4 chunks: 0+1 Completed, 2 Downloading (partial), 3 Pending.
        let ranges = split_into_chunks(data.len() as u64, 4);
        db::chunks::insert_chunks(&pool, id, &ranges).await?;
        let chunks = db::chunks::get_chunks(&pool, id).await?;

        for chunk in chunks.iter().take(2) {
            db::chunks::update_chunk_progress(
                &pool,
                chunk.id,
                chunk.byte_end - chunk.byte_start + 1,
                ChunkStatus::Completed,
            )
            .await?;
        }
        let partial: u64 = 100_000;
        db::chunks::update_chunk_progress(&pool, chunks[2].id, partial, ChunkStatus::Downloading)
            .await?;

        // Write partial file: chunks 0+1 complete, chunk 2 first 100 KB only.
        let disk = DiskWriter::create(&save_path, Some(data.len() as u64)).await?;
        for chunk in chunks.iter().take(2) {
            disk.write_at(
                chunk.byte_start,
                &data[chunk.byte_start as usize..=chunk.byte_end as usize],
            )
            .await?;
        }
        disk.write_at(
            chunks[2].byte_start,
            &data[chunks[2].byte_start as usize..(chunks[2].byte_start + partial) as usize],
        )
        .await?;
        disk.sync().await?;

        // Simulate crash: leave status as Downloading.
        db::downloads::update_status(&pool, id, DownloadStatus::Downloading, None).await?;

        // Fresh manager recovers and resumes from chunk 2's partial offset.
        let manager = DownloadManager::new(pool.clone()).await?;
        manager.recover_incomplete().await?;

        wait_for_completion(&pool, id).await?;

        let written = tokio::fs::read(&save_path).await?;
        assert_eq!(
            written, *data,
            "recovered segmented download must exactly match source data"
        );
        assert!(db::chunks::all_chunks_complete(&pool, id).await?);

        Ok(())
    }

    /// Simulates a crash mid single-stream download. The file on disk contains a
    /// partial write; recovery must truncate and re-download from byte 0 (since
    /// single-stream servers do not support Range).
    #[tokio::test]
    async fn single_stream_download_completes_after_simulated_crash() -> anyhow::Result<()> {
        let data = Arc::new((0..50_000u64).map(|i| (i % 211) as u8).collect::<Vec<_>>());
        let server = spawn_recovery_no_range_server(data.clone()).await?;

        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let save_path = dir.path().join("recovery-single.bin");
        let req = basic_req(id, &server.url);

        // Simulate crash with a partial file (first half) already on disk.
        let partial_len = data.len() / 2;
        tokio::fs::write(&save_path, &data[..partial_len]).await?;
        db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
        db::downloads::update_progress(&pool, id, partial_len as u64).await?;
        db::downloads::update_status(&pool, id, DownloadStatus::Downloading, None).await?;

        let manager = DownloadManager::new(pool.clone()).await?;
        manager.recover_incomplete().await?;

        wait_for_completion(&pool, id).await?;

        let written = tokio::fs::read(&save_path).await?;
        assert_eq!(
            written, *data,
            "single-stream crash recovery must produce the complete file, not partial+full"
        );

        Ok(())
    }

    /// Three downloads in Downloading state with queue cap raised to 3. All must
    /// start on the initial pump from `recover_incomplete` and all complete.
    #[tokio::test]
    async fn multiple_downloads_all_recover() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;

        db::queues::update(
            &pool,
            db::queues::DEFAULT_QUEUE_ID,
            db::queues::QueueUpdate {
                max_concurrent: Some(3),
                ..Default::default()
            },
        )
        .await?;

        let mut ids = Vec::new();
        for i in 0..3u8 {
            let payload = Arc::new(vec![i; 10_000]);
            let server = spawn_recovery_no_range_server(payload).await?;
            let id = Uuid::new_v4();
            let save_path = dir.path().join(format!("multi-{i}.bin"));
            let req = basic_req(id, &server.url);
            db::downloads::insert(&pool, &req, &save_path.to_string_lossy(), "Other").await?;
            db::downloads::update_status(&pool, id, DownloadStatus::Downloading, None).await?;
            ids.push(id);
        }

        let manager = DownloadManager::new(pool.clone()).await?;
        manager.recover_incomplete().await?;

        for id in &ids {
            wait_for_completion(&pool, *id).await?;
        }

        Ok(())
    }
}
