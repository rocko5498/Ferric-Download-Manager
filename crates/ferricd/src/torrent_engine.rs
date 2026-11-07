use crate::db::proxies::ProxyRuntimeConfig;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use ferric_common::ProxyKind;
use ferric_common::TorrentEngineStatus;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration as StdDuration, Instant};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{sleep, Duration};
use url::Url;

const RESTART_BACKOFF: StdDuration = StdDuration::from_secs(10);
const MIN_PRIVACY_VERIFY_SECONDS: u64 = 30;
const MAX_PRIVACY_VERIFY_AGE_MINUTES: i64 = 15;
const PRIVACY_BOUNDARY_ENV: &str = "FERRIC_RQBIT_PRIVACY_BOUNDARY";
const EXTERNAL_ENDPOINT_ENV: &str = "FERRIC_RQBIT_ENDPOINT";
const PRIVACY_RESULT_ENV: &str = "FERRIC_RQBIT_PRIVACY_RESULT";

#[derive(Clone)]
pub struct RqbitEngine {
    client: reqwest::Client,
    state: Arc<Mutex<EngineState>>,
}

#[derive(Default)]
struct EngineState {
    endpoint: Option<String>,
    child: Option<Child>,
    restart_count: u32,
    last_error: Option<String>,
    last_start_attempt: Option<Instant>,
    state: EngineLifecycle,
    current_proxy: Option<ProxyRuntimeConfig>,
    dht_enabled: bool,
    current_blocklist_url: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum EngineLifecycle {
    #[default]
    NotStarted,
    External,
    Starting,
    Running,
    Error,
}

#[derive(Debug, Clone)]
pub struct EngineStart {
    pub endpoint: String,
    pub external_id: String,
}

#[derive(Debug, Clone, Default)]
pub struct EngineStats {
    pub downloaded: u64,
    pub uploaded: u64,
    pub download_speed: u64,
    pub upload_speed: u64,
    pub peers: u64,
    /// rqbit `/stats/v1` top-level `state` (`live`, `paused`, …). Empty if absent.
    pub state: String,
    /// Per-file downloaded bytes, index-aligned with rqbit's `/torrents/{id}` files list.
    /// Source: `/stats/v1` top-level `file_progress` array (not present on file objects).
    pub file_progress: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineFile {
    pub index: u64,
    pub path: String,
    pub size: u64,
    pub downloaded: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnginePeerStats {
    pub addr: Option<String>,
    pub peer_id: Option<String>,
    pub client: Option<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    pub interested: bool,
    pub choked: bool,
    pub has_pieces: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyPrivacyBoundary {
    pub mode: String,
    pub endpoint: Option<String>,
    pub allows_proxy_assigned_start: bool,
    pub explanation: String,
}

#[derive(Debug, Deserialize)]
struct AddTorrentResponse {
    #[serde(default)]
    id: Option<serde_json::Value>,
    #[serde(default)]
    info_hash: Option<String>,
}

impl RqbitEngine {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            state: Arc::new(Mutex::new(EngineState::default())),
        }
    }

    pub async fn start_torrent(
        &self,
        source: &str,
        save_root: &Path,
        peer_id_seed: &str,
        proxy: Option<&ProxyRuntimeConfig>,
        dht_enabled: bool,
        blocklist_url: Option<&str>,
        selected_paths: &[String],
        tcp_listen_enabled: bool,
        // privacy_required: external-verified boundary for SOCKS; else best-effort SOCKS.
        privacy_required: bool,
        seed_enabled: bool,
        // resume: true when this torrent already has partial data on disk from a
        // prior session (manual resume, or TorrentManager::resume_active() on daemon
        // boot) -- rqbit's "overwrite" flag must be true in that case or it refuses
        // to start with "file exists" (this was a real bug: resume always failed).
        resume: bool,
    ) -> Result<EngineStart> {
        if let Some(proxy) = proxy {
            if privacy_required {
                let boundary = proxy_privacy_boundary(Some((&proxy.host, proxy.port)));
                if !boundary.allows_proxy_assigned_start {
                    bail!("{}", proxy_enforcement_refusal(proxy, &boundary));
                }
            } else {
                tracing::warn!(
                    proxy = %format!("{}:{}", proxy.host, proxy.port),
                    "applying best-effort SOCKS to managed rqbit (not packet-verified; privacy_required=false)"
                );
            }
        }

        let endpoint = self
            .ensure_running(
                save_root,
                proxy,
                dht_enabled,
                blocklist_url,
                tcp_listen_enabled,
                seed_enabled,
            )
            .await?;
        let url = format!("{endpoint}/torrents");
        let output_folder = save_root.to_string_lossy().to_string();
        let mut query = vec![
            ("output_folder".to_string(), output_folder),
            ("overwrite".to_string(), resume.to_string()),
        ];
        if let Some(regex) = selected_files_regex(selected_paths) {
            query.push(("only_files_regex".to_string(), regex));
        }

        let response = self
            .client
            .post(url)
            .query(&query)
            .header("X-Ferric-Peer-Seed", peer_id_seed)
            .body(torrent_post_body(source).await?)
            .send()
            .await
            .context("posting torrent to rqbit")?;

        if !response.status().is_success() {
            bail!(
                "rqbit add torrent returned {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }

        let parsed = response.json::<AddTorrentResponse>().await.ok();
        // Prefer info_hash over numeric id — rqbit renumbers ids across restarts
        // (live 2026-07-17: ubuntu went from |123 to id=0 while hash stayed stable).
        let external_id = parsed
            .and_then(|value| {
                value
                    .info_hash
                    .or_else(|| value.id.and_then(json_value_to_string))
            })
            .unwrap_or_else(|| source.to_string());

        Ok(EngineStart {
            endpoint,
            external_id,
        })
    }

    pub async fn status(&self) -> TorrentEngineStatus {
        let state = self.state.lock().await;
        let pid = state.child.as_ref().and_then(Child::id);
        let state_name = match state.state {
            EngineLifecycle::NotStarted => "not_started",
            EngineLifecycle::External => "external",
            EngineLifecycle::Starting => "starting",
            EngineLifecycle::Running => "running",
            EngineLifecycle::Error => "error",
        }
        .to_string();
        TorrentEngineStatus {
            state: state_name.clone(),
            endpoint: state.endpoint.clone(),
            pid,
            restart_count: state.restart_count,
            last_error: state.last_error.clone(),
            explanation: status_explanation(&state_name, state.endpoint.as_deref()),
        }
    }

    pub async fn stats(&self, endpoint: &str, id_or_hash: &str) -> Result<EngineStats> {
        let response = self
            .client
            .get(format!("{endpoint}/torrents/{id_or_hash}/stats/v1"))
            .send()
            .await
            .context("fetching rqbit torrent stats")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!("rqbit stats returned {}", response.status());
        }
        let value = response.json::<serde_json::Value>().await?;
        // rqbit's real /stats/v1 shape (confirmed live, 2026-07-17, against an
        // actual in-progress torrent -- the previous field names here never
        // matched anything real and always fell back to 0, even while rqbit
        // was genuinely downloading at multiple MiB/s with a connected peer):
        //   progress_bytes / uploaded_bytes / total_bytes  -- top-level
        //   live.download_speed.mbps / live.upload_speed.mbps -- nested, and
        //     actually MiB/s despite the field name ("human_readable" confirms
        //     it, e.g. "1.19 MiB/s")
        //   live.snapshot.peer_stats.live -- nested, currently-connected count
        //     (there's ALSO a misleading top-level "live" key, but that's the
        //     torrent's *state* string like "live"/"paused", not a peer count)
        // The "live" sub-object is only present while the engine is actively
        // running the torrent, so all of these fall back to 0 gracefully when
        // it's absent (paused/not-yet-started/etc).
        const MIB: f64 = 1_048_576.0;
        let mbps_to_bytes = |v: &serde_json::Value, path: &[&str]| -> u64 {
            let mut cur = v;
            for key in path {
                match cur.get(key) {
                    Some(next) => cur = next,
                    None => return 0,
                }
            }
            cur.as_f64().map_or(0, |mbps| (mbps * MIB).round() as u64)
        };
        Ok(EngineStats {
            downloaded: value
                .get("progress_bytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            uploaded: value
                .get("uploaded_bytes")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            download_speed: mbps_to_bytes(&value, &["live", "download_speed", "mbps"]),
            upload_speed: mbps_to_bytes(&value, &["live", "upload_speed", "mbps"]),
            peers: value
                .get("live")
                .and_then(|l| l.get("snapshot"))
                .and_then(|s| s.get("peer_stats"))
                .and_then(|p| p.get("live"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
            state: value
                .get("state")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("")
                .to_string(),
            // Live-verified 2026-07-17 against rqbit: per-file bytes live ONLY here.
            // `/torrents/{id}` file objects have name/length/included — never downloaded.
            file_progress: value
                .get("file_progress")
                .and_then(serde_json::Value::as_array)
                .map(|arr| arr.iter().map(|v| v.as_u64().unwrap_or(0)).collect())
                .unwrap_or_default(),
        })
    }

    pub async fn files(&self, endpoint: &str, id_or_hash: &str) -> Result<Vec<EngineFile>> {
        let response = self
            .client
            .get(format!("{endpoint}/torrents/{id_or_hash}"))
            .send()
            .await
            .context("fetching rqbit torrent details")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!("rqbit details returned {}", response.status());
        }
        let value = response.json::<serde_json::Value>().await?;
        Ok(extract_engine_files(&value))
    }

    pub async fn peer_stats(
        &self,
        endpoint: &str,
        id_or_hash: &str,
    ) -> Result<Vec<EnginePeerStats>> {
        let response = self
            .client
            .get(format!("{endpoint}/torrents/{id_or_hash}/peer_stats"))
            .send()
            .await
            .context("fetching rqbit peer stats")?;
        // Live-verified 2026-07-17: this rqbit build returns 400 on /peer_stats
        // (and 404 on /peers). Treat as "no per-peer list available" so the UI
        // can show an empty peers table instead of a hard API 400.
        if response.status() == StatusCode::NOT_FOUND
            || response.status() == StatusCode::BAD_REQUEST
        {
            return Ok(Vec::new());
        }
        if !response.status().is_success() {
            tracing::debug!(
                status = %response.status(),
                "rqbit peer_stats unavailable; returning empty list"
            );
            return Ok(Vec::new());
        }
        let value = response.json::<serde_json::Value>().await?;
        Ok(parse_peer_stats_response(&value))
    }

    pub async fn pause_torrent(&self, endpoint: &str, id_or_hash: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{endpoint}/torrents/{id_or_hash}/pause"))
            .send()
            .await
            .context("pausing rqbit torrent")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!("rqbit pause returned {}", response.status());
        }
        Ok(())
    }

    /// True if this info-hash is already in the rqbit session (any numeric id).
    /// Used to **never re-POST** the same torrent (double init + "file is None").
    pub async fn session_has_info_hash(&self, endpoint: &str, info_hash: &str) -> Result<bool> {
        if info_hash.is_empty() {
            return Ok(false);
        }
        let want = info_hash.to_ascii_lowercase();
        let response = self
            .client
            .get(format!("{endpoint}/torrents"))
            .send()
            .await
            .context("listing rqbit torrents")?;
        if !response.status().is_success() {
            return Ok(false);
        }
        let value = response
            .json::<serde_json::Value>()
            .await
            .context("parsing rqbit torrent list")?;
        let Some(arr) = value
            .get("torrents")
            .and_then(|t| t.as_array())
            .or_else(|| value.as_array())
        else {
            return Ok(false);
        };
        Ok(arr.iter().any(|t| {
            t.get("info_hash")
                .and_then(|h| h.as_str())
                .is_some_and(|h| h.eq_ignore_ascii_case(&want))
        }))
    }

    /// Unpause / start an already-added rqbit torrent.
    /// Live-verified 2026-07-17: `POST /torrents/{id}/start` returns 200 and flips
    /// stats `state` from `paused` → `live`. Re-POSTing the magnet via
    /// [`Self::start_torrent`] does **not** unpause an existing id — that left
    /// Ferric status=Downloading while rqbit stayed paused (0 B/s forever).
    pub async fn start_existing(&self, endpoint: &str, id_or_hash: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{endpoint}/torrents/{id_or_hash}/start"))
            .send()
            .await
            .context("starting existing rqbit torrent")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!(
                "rqbit start returned {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    pub async fn force_recheck(&self, endpoint: &str, id_or_hash: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{endpoint}/torrents/{id_or_hash}/force_recheck"))
            .send()
            .await
            .context("force recheck rqbit torrent")?;
        // Live 2026-07-17: this rqbit build has no force_recheck route (404) —
        // soft-ok so UI Recheck does not toast generic 400.
        if matches!(
            response.status(),
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::BAD_REQUEST
        ) {
            tracing::warn!(
                status = %response.status(),
                "force_recheck not supported by this rqbit build; ignoring"
            );
            return Ok(());
        }
        if !response.status().is_success() {
            bail!(
                "rqbit force_recheck returned {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    pub async fn force_announce(&self, endpoint: &str, id_or_hash: &str) -> Result<()> {
        let response = self
            .client
            .post(format!("{endpoint}/torrents/{id_or_hash}/force_announce"))
            .send()
            .await
            .context("force announce rqbit torrent")?;
        // Some rqbit builds omit force_announce (404/405/400) — soft-ok.
        if matches!(
            response.status(),
            StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED | StatusCode::BAD_REQUEST
        ) {
            tracing::warn!(
                status = %response.status(),
                "force_announce not supported by this rqbit build; ignoring"
            );
            return Ok(());
        }
        if !response.status().is_success() {
            bail!(
                "rqbit force_announce returned {}: {}",
                response.status(),
                response.text().await.unwrap_or_default()
            );
        }
        Ok(())
    }

    pub async fn set_location(
        &self,
        endpoint: &str,
        id_or_hash: &str,
        new_path: &str,
    ) -> Result<()> {
        let response = self
            .client
            .post(format!("{endpoint}/torrents/{id_or_hash}/move"))
            .query(&[("new_output_folder", new_path)])
            .send()
            .await
            .context("moving rqbit torrent storage")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!("rqbit move returned {}", response.status());
        }
        Ok(())
    }

    pub async fn update_only_files(
        &self,
        endpoint: &str,
        id_or_hash: &str,
        selected_indices: &[u64],
    ) -> Result<()> {
        let response = self
            .client
            .post(format!(
                "{endpoint}/torrents/{id_or_hash}/update_only_files"
            ))
            .json(&serde_json::json!({ "only_files": selected_indices }))
            .send()
            .await
            .context("updating rqbit selected files")?;
        if response.status() == StatusCode::NOT_FOUND {
            bail!("rqbit torrent not found");
        }
        if !response.status().is_success() {
            bail!("rqbit update_only_files returned {}", response.status());
        }
        Ok(())
    }

    async fn ensure_running(
        &self,
        save_root: &Path,
        proxy: Option<&ProxyRuntimeConfig>,
        dht_enabled: bool,
        blocklist_url: Option<&str>,
        tcp_listen_enabled: bool,
        seed_enabled: bool,
    ) -> Result<String> {
        if let Ok(endpoint) = std::env::var(EXTERNAL_ENDPOINT_ENV) {
            let endpoint = endpoint.trim_end_matches('/').to_string();
            if !endpoint_is_loopback_http(&endpoint) {
                bail!("{EXTERNAL_ENDPOINT_ENV} must be an http:// loopback endpoint");
            }
            if let Err(err) = self.wait_until_ready(&endpoint).await {
                self.record_error(err.to_string()).await;
                return Err(err);
            }
            let mut state = self.state.lock().await;
            state.endpoint = Some(endpoint.clone());
            state.child = None;
            state.state = EngineLifecycle::External;
            state.last_error = None;
            state.current_proxy = proxy.cloned();
            state.dht_enabled = dht_enabled;
            state.current_blocklist_url = blocklist_url.map(String::from);
            return Ok(endpoint);
        }

        let requested_proxy = proxy.cloned();
        let mut state = self.state.lock().await;
        if let Some(endpoint) = state.endpoint.clone() {
            let blocklist_matches = match (&state.current_blocklist_url, blocklist_url) {
                (Some(a), Some(b)) => a == b,
                (None, None) => true,
                _ => false,
            };
            if state.current_proxy == requested_proxy
                && state.dht_enabled == dht_enabled
                && blocklist_matches
                && self.wait_until_ready(&endpoint).await.is_ok()
            {
                state.state = EngineLifecycle::Running;
                state.last_error = None;
                return Ok(endpoint);
            }
            state.endpoint = None;
            state.state = EngineLifecycle::Error;
            state.current_proxy = None;
            state.dht_enabled = false;
            state.current_blocklist_url = None;
            if let Some(mut child) = state.child.take() {
                let _ = child.kill().await;
            }
        }

        if let Some(last_attempt) = state.last_start_attempt {
            if let Some(wait) = restart_backoff_wait(last_attempt) {
                let message = format!("rqbit restart backoff active; retry in about {wait}s");
                state.state = EngineLifecycle::Error;
                state.last_error = Some(message.clone());
                bail!(message);
            }
        }

        let bin = std::env::var("FERRIC_RQBIT_BIN").unwrap_or_else(|_| "rqbit".to_string());
        let port = std::env::var("FERRIC_RQBIT_PORT")
            .ok()
            .and_then(|value| value.parse::<u16>().ok())
            .unwrap_or(3030);
        // Env overrides settings: FERRIC_RQBIT_TCP_LISTEN=1 forces inbound listen.
        let tcp_listen = tcp_listen_enabled
            || matches!(
                std::env::var("FERRIC_RQBIT_TCP_LISTEN").ok().as_deref(),
                Some("1" | "true" | "TRUE")
            );
        let endpoint = format!("http://127.0.0.1:{port}");
        state.state = EngineLifecycle::Starting;
        state.restart_count = state.restart_count.saturating_add(1);
        state.last_start_attempt = Some(Instant::now());

        let mut command = Command::new(&bin);
        command.args(build_rqbit_args(
            port,
            &save_root.to_string_lossy(),
            requested_proxy.as_ref(),
            dht_enabled,
            blocklist_url,
            tcp_listen,
            seed_enabled,
        ));
        command.kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: the rqbit sidecar is a console program

        let child = match command.spawn() {
            Ok(child) => child,
            Err(err) => {
                state.state = EngineLifecycle::Error;
                state.last_error = Some(format!("starting rqbit sidecar from {bin}: {err}"));
                return Err(err).with_context(|| format!("starting rqbit sidecar from {bin}"));
            }
        };
        state.child = Some(child);
        state.endpoint = Some(endpoint.clone());
        drop(state);

        if let Err(err) = self.wait_until_ready(&endpoint).await {
            self.record_error(err.to_string()).await;
            return Err(err);
        }
        let mut state = self.state.lock().await;
        state.state = EngineLifecycle::Running;
        state.last_error = None;
        state.current_proxy = requested_proxy;
        state.dht_enabled = dht_enabled;
        state.current_blocklist_url = blocklist_url.map(String::from);
        Ok(endpoint)
    }

    async fn wait_until_ready(&self, endpoint: &str) -> Result<()> {
        let url = format!("{endpoint}/");
        let mut last_error = None;
        for _ in 0..25 {
            match self.client.get(&url).send().await {
                Ok(response) if response.status().is_success() => return Ok(()),
                Ok(response) => {
                    last_error = Some(format!("rqbit readiness returned {}", response.status()));
                }
                Err(err) => last_error = Some(err.to_string()),
            }
            sleep(Duration::from_millis(200)).await;
        }
        bail!(
            "rqbit sidecar did not become ready: {}",
            last_error.unwrap_or_else(|| "unknown error".to_string())
        );
    }

    async fn record_error(&self, message: String) {
        let mut state = self.state.lock().await;
        state.state = EngineLifecycle::Error;
        state.last_error = Some(message);
    }
}

impl Drop for EngineState {
    fn drop(&mut self) {
        if let Some(child) = &mut self.child {
            let _ = child.start_kill();
        }
    }
}

pub fn proxy_privacy_boundary(expected_proxy: Option<(&str, u16)>) -> ProxyPrivacyBoundary {
    let result_path = std::env::var(PRIVACY_RESULT_ENV).ok();
    let result_json = result_path
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok());
    proxy_privacy_boundary_from_parts(
        std::env::var(PRIVACY_BOUNDARY_ENV).ok().as_deref(),
        std::env::var(EXTERNAL_ENDPOINT_ENV).ok().as_deref(),
        result_path.as_deref(),
        result_json.as_deref(),
        expected_proxy,
    )
}

fn proxy_privacy_boundary_from_parts(
    mode: Option<&str>,
    endpoint: Option<&str>,
    result_path: Option<&str>,
    result_json: Option<&str>,
    expected_proxy: Option<(&str, u16)>,
) -> ProxyPrivacyBoundary {
    let mode = mode.unwrap_or("").trim();
    let endpoint = endpoint.map(str::trim).filter(|value| !value.is_empty());
    let result_path = result_path.map(str::trim).filter(|value| !value.is_empty());
    let endpoint_owned = endpoint.map(ToOwned::to_owned);
    let endpoint_allowed = endpoint.is_some_and(endpoint_is_loopback_http);
    let result_passes = result_json
        .and_then(|json| verification_result_passes(json, expected_proxy, endpoint))
        .unwrap_or(false);
    let allows = mode.eq_ignore_ascii_case("external-verified")
        && endpoint_allowed
        && result_path.is_some()
        && result_passes;
    let explanation = if allows {
        format!(
            "Proxy-assigned torrents may start because {PRIVACY_BOUNDARY_ENV}=external-verified, {EXTERNAL_ENDPOINT_ENV} points at an externally constrained rqbit API, and {PRIVACY_RESULT_ENV} contains a passing verification artifact."
        )
    } else if mode.eq_ignore_ascii_case("external-verified") && endpoint_allowed {
        format!(
            "{PRIVACY_BOUNDARY_ENV}=external-verified also requires {PRIVACY_RESULT_ENV} to point at a passing Verify-TorrentPrivacy.ps1 JSON result for the assigned proxy."
        )
    } else if mode.eq_ignore_ascii_case("external-verified") && endpoint.is_some() {
        format!("{EXTERNAL_ENDPOINT_ENV} must be an http:// loopback endpoint.")
    } else if mode.eq_ignore_ascii_case("external-verified") {
        format!(
            "{PRIVACY_BOUNDARY_ENV}=external-verified requires {EXTERNAL_ENDPOINT_ENV}; managed rqbit sidecar proxy mode remains blocked."
        )
    } else {
        format!(
            "Set {PRIVACY_BOUNDARY_ENV}=external-verified with {EXTERNAL_ENDPOINT_ENV} only after an external firewall/container/VPN boundary has been packet-verified."
        )
    };
    ProxyPrivacyBoundary {
        mode: mode.to_string(),
        endpoint: endpoint_owned,
        allows_proxy_assigned_start: allows,
        explanation,
    }
}

fn endpoint_is_loopback_http(endpoint: &str) -> bool {
    let Ok(url) = Url::parse(endpoint) else {
        return false;
    };
    if url.scheme() != "http" {
        return false;
    }
    match url.host_str() {
        Some("localhost") => true,
        Some(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        None => false,
    }
}

fn verification_result_passes(
    json: &str,
    expected_proxy: Option<(&str, u16)>,
    endpoint: Option<&str>,
) -> Option<bool> {
    verification_result_passes_at(json, expected_proxy, endpoint, Utc::now())
}

fn verification_result_passes_at(
    json: &str,
    expected_proxy: Option<(&str, u16)>,
    endpoint: Option<&str>,
    now: DateTime<Utc>,
) -> Option<bool> {
    let value: serde_json::Value = serde_json::from_str(json).ok()?;
    if !value.get("passed")?.as_bool()? {
        return Some(false);
    }
    if value.get("schema_version")?.as_u64()? != 1 {
        return Some(false);
    }
    if value.get("duration_seconds")?.as_u64()? < MIN_PRIVACY_VERIFY_SECONDS {
        return Some(false);
    }
    let checked_at = DateTime::parse_from_rfc3339(value.get("checked_at")?.as_str()?)
        .ok()?
        .with_timezone(&Utc);
    if now.signed_duration_since(checked_at)
        > ChronoDuration::minutes(MAX_PRIVACY_VERIFY_AGE_MINUTES)
        || checked_at > now + ChronoDuration::minutes(1)
    {
        return Some(false);
    }
    if value
        .get("leaks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|leaks| !leaks.is_empty())
    {
        return Some(false);
    }
    if !value.get("packet_verified")?.as_bool()? {
        return Some(false);
    }
    if value
        .get("packet_leaks")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|leaks| !leaks.is_empty())
    {
        return Some(false);
    }
    if let Some((host, port)) = expected_proxy {
        let result_host = value.get("proxy_host")?.as_str()?;
        let result_port = value.get("proxy_port")?.as_u64()?;
        if result_host != host || result_port != u64::from(port) {
            return Some(false);
        }
    }
    if let Some(endpoint) = endpoint {
        if value.get("rqbit_endpoint")?.as_str()? != endpoint {
            return Some(false);
        }
    }
    Some(true)
}

pub fn proxy_enforcement_refusal(
    proxy: &ProxyRuntimeConfig,
    boundary: &ProxyPrivacyBoundary,
) -> String {
    let kind = match proxy.proxy_type {
        ProxyKind::Socks5 => "SOCKS5",
        ProxyKind::Http => "HTTP",
    };
    let auth = if proxy.username.is_some() || proxy.password.is_some() {
        " with authentication"
    } else {
        ""
    };
    format!(
        "proxy '{}' ({kind} {}:{}) is assigned{auth}, but rqbit sidecar proxy enforcement is not enabled because UDP tracker, DNS-over-proxy, and IPv6 leak controls are not packet-verified yet. {}",
        proxy.id, proxy.host, proxy.port, boundary.explanation
    )
}

fn json_value_to_string(value: serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::String(value) => Some(value),
        serde_json::Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn first_u64(value: &serde_json::Value, keys: &[&str]) -> u64 {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
        .unwrap_or(0)
}

fn parse_peer_stats_response(value: &serde_json::Value) -> Vec<EnginePeerStats> {
    let peers_array = value
        .as_array()
        .or_else(|| value.get("peers").and_then(|v| v.as_array()));
    let Some(peers) = peers_array else {
        return Vec::new();
    };
    peers
        .iter()
        .filter_map(|peer| {
            let addr = peer
                .get("addr")
                .or_else(|| peer.get("address"))
                .or_else(|| peer.get("ip"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let peer_id = peer
                .get("peer_id")
                .or_else(|| peer.get("id"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let client = peer
                .get("client")
                .or_else(|| peer.get("client_name"))
                .and_then(|v| v.as_str())
                .map(String::from);
            let downloaded = peer
                .get("downloaded")
                .or_else(|| peer.get("bytes_downloaded"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let uploaded = peer
                .get("uploaded")
                .or_else(|| peer.get("bytes_uploaded"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0);
            let interested = peer
                .get("interested")
                .or_else(|| peer.get("peer_interested"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let choked = peer
                .get("choked")
                .or_else(|| peer.get("peer_choking"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true);
            let has_pieces = peer
                .get("have_pieces")
                .or_else(|| peer.get("progress"))
                .and_then(serde_json::Value::as_f64)
                .map(|v| v > 0.0)
                .or_else(|| {
                    peer.get("pieces")
                        .and_then(serde_json::Value::as_u64)
                        .map(|v| v > 0)
                })
                .unwrap_or(false);
            Some(EnginePeerStats {
                addr,
                peer_id,
                client,
                downloaded,
                uploaded,
                interested,
                choked,
                has_pieces,
            })
        })
        .collect()
}

fn extract_engine_files(value: &serde_json::Value) -> Vec<EngineFile> {
    let mut files = Vec::new();
    collect_file_arrays(value, &mut files);
    files
}

fn collect_file_arrays(value: &serde_json::Value, files: &mut Vec<EngineFile>) {
    match value {
        serde_json::Value::Array(items) => {
            let before = files.len();
            for (index, item) in items.iter().enumerate() {
                if let Some(file) = engine_file_from_value(index as u64, item) {
                    files.push(file);
                }
            }
            if files.len() == before {
                for item in items {
                    collect_file_arrays(item, files);
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                if key.contains("file") {
                    collect_file_arrays(child, files);
                }
            }
        }
        _ => {}
    }
}

fn engine_file_from_value(fallback_index: u64, value: &serde_json::Value) -> Option<EngineFile> {
    let path = first_string(
        value,
        &[
            "path",
            "name",
            "filename",
            "relative_filename",
            "relative_path",
        ],
    )?;
    let size = first_u64(value, &["size", "length", "len", "total_size"]);
    Some(EngineFile {
        index: first_optional_u64(value, &["index", "idx", "file_index"]).unwrap_or(fallback_index),
        path,
        size,
        downloaded: first_u64(value, &["downloaded", "bytes_downloaded", "progress_bytes"]),
    })
}

fn first_string(value: &serde_json::Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_str))
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
}

fn first_optional_u64(value: &serde_json::Value, keys: &[&str]) -> Option<u64> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(serde_json::Value::as_u64))
}

fn selected_files_regex(paths: &[String]) -> Option<String> {
    if paths.is_empty() {
        return None;
    }
    let escaped = paths
        .iter()
        .map(|path| regex_escape(path))
        .collect::<Vec<_>>()
        .join("|");
    Some(format!("^(?:{escaped})$"))
}

async fn torrent_post_body(source: &str) -> Result<Vec<u8>> {
    let path = Path::new(source);
    if path.is_file() {
        return tokio::fs::read(path)
            .await
            .with_context(|| format!("reading torrent file {}", path.display()));
    }
    Ok(source.as_bytes().to_vec())
}

fn regex_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        if matches!(
            ch,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '|' | '[' | ']' | '{' | '}' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

fn status_explanation(state: &str, endpoint: Option<&str>) -> String {
    match (state, endpoint) {
        ("not_started", _) => "Torrent engine has not been started. No peer traffic is active.".to_string(),
        ("external", Some(endpoint)) => format!("Using external rqbit API at {endpoint}."),
        ("starting", _) => "Managed rqbit sidecar is starting on localhost.".to_string(),
        ("running", Some(endpoint)) => format!("Managed rqbit sidecar is reachable at {endpoint}."),
        ("error", _) => "Torrent engine failed to start or respond. Peer traffic is not considered active from Ferric.".to_string(),
        _ => "Torrent engine status is partially known.".to_string(),
    }
}

fn restart_backoff_wait(last_attempt: Instant) -> Option<u64> {
    let elapsed = last_attempt.elapsed();
    (elapsed < RESTART_BACKOFF).then(|| RESTART_BACKOFF.saturating_sub(elapsed).as_secs().max(1))
}

pub fn parse_engine_ref(engine: &str) -> Option<(&str, &str)> {
    let rest = engine.strip_prefix("rqbit:")?;
    rest.split_once('|')
}

pub fn format_engine_ref(endpoint: &str, external_id: &str) -> String {
    format!("rqbit:{}|{}", endpoint.trim_end_matches('/'), external_id)
}

fn build_rqbit_args(
    port: u16,
    save_root: &str,
    proxy: Option<&ProxyRuntimeConfig>,
    dht_enabled: bool,
    blocklist_url: Option<&str>,
    tcp_listen_enabled: bool,
    seed_enabled: bool,
) -> Vec<String> {
    let mut args = vec![
        "--http-api-listen-addr".to_string(),
        format!("127.0.0.1:{port}"),
        "--disable-upnp-port-forward".to_string(),
    ];

    // qBittorrent-style safer defaults: no inbound listen, no UPnP (research: anonymous mode).
    // Set FERRIC_RQBIT_TCP_LISTEN=1 or torrent_tcp_listen_enabled=1 to allow inbound.
    if !tcp_listen_enabled {
        args.push("--disable-tcp-listen".to_string());
    }

    if !dht_enabled {
        args.push("--disable-dht".to_string());
        args.push("--disable-dht-persistence".to_string());
    }

    // Cap upload when seeding is disabled (reduces accidental seeding surface).
    if !seed_enabled {
        args.push("--ratelimit-upload".to_string());
        args.push("1".to_string());
    }

    if let Some(p) = proxy {
        let scheme = match p.proxy_type {
            ProxyKind::Socks5 => "socks5h",
            ProxyKind::Http => "http",
        };
        let socks_url = if let (Some(u), Some(pw)) = (&p.username, &p.password) {
            format!("{}://{}:{}@{}:{}", scheme, u, pw, p.host, p.port)
        } else {
            format!("{}://{}:{}", scheme, p.host, p.port)
        };
        args.push("--socks-url".to_string());
        args.push(socks_url);
    }

    if let Some(url) = blocklist_url {
        if !url.is_empty() {
            args.push("--blocklist-url".to_string());
            args.push(url.to_string());
        }
    }

    args.push("server".to_string());
    args.push("start".to_string());
    args.push("--disable-persistence".to_string());
    args.push(save_root.to_string());
    args
}

pub fn resolve_save_root(
    record_save_root: Option<&str>,
    default_save_dir: Option<String>,
) -> Result<PathBuf> {
    // Prefer per-torrent path, then settings default_save_dir, then OS Downloads.
    // Live 2026-07-17: engine_gated imports often have empty save_root AND empty
    // default_save_dir — 400 "save folder is required" on Start. Fail-soft to
    // the user's Downloads folder instead of a dead-end error.
    //
    // S12: validate non-empty paths are absolute — reject URIs, relative paths, garbage.
    if let Some(raw) = record_save_root {
        crate::security::validate_save_path(raw)?;
    }
    let root = record_save_root
        .filter(|value| !value.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            default_save_dir
                .filter(|value| !value.trim().is_empty())
                .map(PathBuf::from)
        })
        .unwrap_or_else(platform_downloads_dir);
    Ok(root)
}

/// Best-effort user Downloads directory (Windows USERPROFILE\\Downloads, else HOME/Downloads).
pub fn platform_downloads_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("Downloads");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join("Downloads");
        }
    }
    std::env::temp_dir().join("ferric-downloads")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_ref_round_trips() {
        let value = format_engine_ref("http://127.0.0.1:3030/", "7");
        assert_eq!(
            parse_engine_ref(&value),
            Some(("http://127.0.0.1:3030", "7"))
        );
    }

    #[test]
    fn selected_files_regex_escapes_literal_paths() {
        let regex =
            selected_files_regex(&["dir/file.name[1].mkv".to_string(), "plain.txt".to_string()])
                .expect("selected paths produce regex");

        assert_eq!(regex, r"^(?:dir/file\.name\[1\]\.mkv|plain\.txt)$");
    }

    #[test]
    fn extract_engine_files_keeps_indices() {
        let value = serde_json::json!({
            "files": [
                { "path": "a.bin", "size": 10 },
                { "path": "dir/b.bin", "size": 20, "index": 7, "downloaded": 3 }
            ]
        });

        let files = extract_engine_files(&value);

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].index, 0);
        assert_eq!(files[1].index, 7);
        assert_eq!(files[1].downloaded, 3);
    }

    #[test]
    fn stats_v1_file_progress_parses_live_rqbit_shape() {
        // Captured from live rqbit /torrents/2/stats/v1 on 2026-07-17:
        // finished multi-file torrent; file objects themselves have no downloaded field.
        let value = serde_json::json!({
            "state": "paused",
            "file_progress": [1700762866u64, 1742437162u64, 1304252790u64, 1489766613u64, 34u64],
            "error": null,
            "progress_bytes": 6237219465u64,
            "uploaded_bytes": 0,
            "total_bytes": 6237219465u64,
            "finished": true,
            "live": null
        });
        let progress: Vec<u64> = value
            .get("file_progress")
            .and_then(serde_json::Value::as_array)
            .map(|arr| arr.iter().map(|v| v.as_u64().unwrap_or(0)).collect())
            .unwrap_or_default();
        assert_eq!(progress.len(), 5);
        assert_eq!(progress[0], 1700762866);
        assert_eq!(progress[4], 34);
        assert_eq!(progress.iter().sum::<u64>(), 6237219465);
    }

    #[test]
    fn save_root_prefers_record_value() {
        #[cfg(windows)]
        let (record, default) = ("D:/Torrent", "C:/Downloads");
        #[cfg(not(windows))]
        let (record, default) = ("/torrent", "/downloads");
        let root = resolve_save_root(Some(record), Some(default.to_string())).unwrap();
        assert_eq!(root, PathBuf::from(record));
    }

    #[test]
    fn save_root_uses_default_when_record_blank() {
        let root = resolve_save_root(Some(""), Some("C:/Downloads".to_string())).unwrap();
        assert_eq!(root, PathBuf::from("C:/Downloads"));
    }

    #[test]
    fn save_root_rejects_garbage_path_s12() {
        // S12: magnet URI as save_root must be rejected.
        let err = resolve_save_root(
            Some("magnet:?xt=urn:btih:abc123"),
            Some("C:/Downloads".to_string()),
        );
        assert!(err.is_err());
        let msg = err.unwrap_err().to_string();
        assert!(
            msg.contains("absolute"),
            "expected absolute-path error, got: {msg}"
        );
    }

    #[test]
    fn save_root_rejects_relative_path_s12() {
        let err = resolve_save_root(Some("relative/path"), Some("C:/Downloads".to_string()));
        assert!(err.is_err());
    }

    #[test]
    fn status_explains_not_started_without_peer_traffic() {
        let explanation = status_explanation("not_started", None);
        assert!(explanation.contains("No peer traffic"));
    }

    #[tokio::test]
    async fn fresh_engine_reports_not_started() {
        let status = RqbitEngine::new().status().await;
        assert_eq!(status.state, "not_started");
        assert_eq!(status.restart_count, 0);
        assert!(status.endpoint.is_none());
    }

    #[test]
    fn restart_backoff_reports_remaining_wait() {
        let wait = restart_backoff_wait(Instant::now()).expect("fresh attempt must be in backoff");
        assert!((1..=10).contains(&wait));
    }

    #[test]
    fn proxy_refusal_does_not_include_password() {
        let proxy = ProxyRuntimeConfig {
            id: "proxy-1".to_string(),
            proxy_type: ProxyKind::Socks5,
            host: "127.0.0.1".to_string(),
            port: 1080,
            username: Some("user".to_string()),
            password: Some("super-secret".to_string()),
        };
        let boundary = proxy_privacy_boundary_from_parts(None, None, None, None, None);

        let message = proxy_enforcement_refusal(&proxy, &boundary);

        assert!(message.contains("proxy-1"));
        assert!(message.contains("SOCKS5"));
        assert!(message.contains("with authentication"));
        assert!(!message.contains("super-secret"));
    }

    #[test]
    fn proxy_privacy_boundary_requires_external_verified_mode_and_endpoint() {
        let checked_at = Utc::now().to_rfc3339();
        let good_result = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}}"#
        );
        let missing_mode = proxy_privacy_boundary_from_parts(
            None,
            Some("http://127.0.0.1:3030"),
            Some("privacy.json"),
            Some(&good_result),
            Some(("127.0.0.1", 1080)),
        );
        assert!(!missing_mode.allows_proxy_assigned_start);

        let missing_endpoint = proxy_privacy_boundary_from_parts(
            Some("external-verified"),
            None,
            Some("privacy.json"),
            Some(&good_result),
            Some(("127.0.0.1", 1080)),
        );
        assert!(!missing_endpoint.allows_proxy_assigned_start);

        let verified = proxy_privacy_boundary_from_parts(
            Some("external-verified"),
            Some("http://127.0.0.1:3030"),
            Some("privacy.json"),
            Some(&good_result),
            Some(("127.0.0.1", 1080)),
        );
        assert!(verified.allows_proxy_assigned_start);
        assert_eq!(verified.endpoint.as_deref(), Some("http://127.0.0.1:3030"));
    }

    #[test]
    fn proxy_privacy_boundary_rejects_non_loopback_endpoint() {
        let checked_at = Utc::now().to_rfc3339();
        let result = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://192.168.1.10:3030"}}"#
        );
        let boundary = proxy_privacy_boundary_from_parts(
            Some("external-verified"),
            Some("http://192.168.1.10:3030"),
            Some("privacy.json"),
            Some(&result),
            Some(("127.0.0.1", 1080)),
        );

        assert!(!boundary.allows_proxy_assigned_start);
        assert!(boundary.explanation.contains("loopback"));
    }

    #[test]
    fn proxy_privacy_boundary_rejects_failed_or_mismatched_result() {
        let checked_at = Utc::now().to_rfc3339();
        let failed = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":false,"leaks":["TCP leak"],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}}"#
        );
        let wrong_proxy = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":9999,"rqbit_endpoint":"http://127.0.0.1:3030"}}"#
        );
        let no_packet_check = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}}"#
        );
        let packet_leak = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":["UDP 53"],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}}"#
        );
        let wrong_endpoint = format!(
            r#"{{"schema_version":1,"checked_at":"{checked_at}","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:4040"}}"#
        );

        for result in [
            &failed,
            &wrong_proxy,
            &no_packet_check,
            &packet_leak,
            &wrong_endpoint,
        ] {
            let boundary = proxy_privacy_boundary_from_parts(
                Some("external-verified"),
                Some("http://127.0.0.1:3030"),
                Some("privacy.json"),
                Some(result),
                Some(("127.0.0.1", 1080)),
            );
            assert!(!boundary.allows_proxy_assigned_start);
        }
    }

    #[test]
    fn verification_result_rejects_short_or_stale_artifacts() {
        let now = DateTime::parse_from_rfc3339("2026-07-04T12:20:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let short = r#"{"schema_version":1,"checked_at":"2026-07-04T12:20:00Z","duration_seconds":1,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}"#;
        let stale = r#"{"schema_version":1,"checked_at":"2026-07-04T11:00:00Z","duration_seconds":60,"passed":true,"leaks":[],"packet_verified":true,"packet_leaks":[],"proxy_host":"127.0.0.1","proxy_port":1080,"rqbit_endpoint":"http://127.0.0.1:3030"}"#;

        assert_eq!(
            verification_result_passes_at(
                short,
                Some(("127.0.0.1", 1080)),
                Some("http://127.0.0.1:3030"),
                now
            ),
            Some(false)
        );
        assert_eq!(
            verification_result_passes_at(
                stale,
                Some(("127.0.0.1", 1080)),
                Some("http://127.0.0.1:3030"),
                now
            ),
            Some(false)
        );
    }

    #[test]
    fn test_build_rqbit_args() {
        let save_root = "C:\\downloads";

        // 1. No proxy, DHT disabled, no blocklist, outbound-only, seed off
        let args_no_proxy = build_rqbit_args(3030, save_root, None, false, None, false, false);
        assert!(!args_no_proxy.contains(&"--socks-url".to_string()));
        assert!(!args_no_proxy.contains(&"--blocklist-url".to_string()));
        assert_eq!(args_no_proxy[1], "127.0.0.1:3030");
        assert!(args_no_proxy.contains(&"--disable-dht".to_string()));
        assert!(args_no_proxy.contains(&"--disable-dht-persistence".to_string()));
        assert!(args_no_proxy.contains(&"--disable-tcp-listen".to_string()));
        assert!(args_no_proxy.contains(&"--ratelimit-upload".to_string()));
        assert_eq!(args_no_proxy.last().unwrap(), save_root);

        // 1b. No proxy, DHT enabled, with blocklist, listen enabled, seed on
        let block_url = "https://example.com/block.txt";
        let args_no_proxy_dht =
            build_rqbit_args(3030, save_root, None, true, Some(block_url), true, true);
        assert!(!args_no_proxy_dht.contains(&"--disable-dht".to_string()));
        assert!(!args_no_proxy_dht.contains(&"--disable-dht-persistence".to_string()));
        assert!(!args_no_proxy_dht.contains(&"--disable-tcp-listen".to_string()));
        assert!(!args_no_proxy_dht.contains(&"--ratelimit-upload".to_string()));
        assert!(args_no_proxy_dht.contains(&"--blocklist-url".to_string()));
        let block_idx = args_no_proxy_dht
            .iter()
            .position(|r| r == "--blocklist-url")
            .unwrap();
        assert_eq!(args_no_proxy_dht[block_idx + 1], block_url);

        // 2. SOCKS5 proxy without credentials
        let proxy_no_auth = ProxyRuntimeConfig {
            id: "p1".to_string(),
            proxy_type: ProxyKind::Socks5,
            host: "1.2.3.4".to_string(),
            port: 1080,
            username: None,
            password: None,
        };
        let args_socks = build_rqbit_args(
            3030,
            save_root,
            Some(&proxy_no_auth),
            false,
            None,
            false,
            false,
        );
        assert!(args_socks.contains(&"--socks-url".to_string()));
        assert!(args_socks.contains(&"--disable-tcp-listen".to_string()));
        let socks_idx = args_socks.iter().position(|r| r == "--socks-url").unwrap();
        assert_eq!(args_socks[socks_idx + 1], "socks5h://1.2.3.4:1080");

        // 3. SOCKS5 proxy with credentials
        let proxy_auth = ProxyRuntimeConfig {
            id: "p2".to_string(),
            proxy_type: ProxyKind::Socks5,
            host: "5.6.7.8".to_string(),
            port: 1088,
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
        };
        let args_socks_auth = build_rqbit_args(
            3030,
            save_root,
            Some(&proxy_auth),
            false,
            None,
            false,
            false,
        );
        assert!(args_socks_auth.contains(&"--socks-url".to_string()));
        let socks_idx_auth = args_socks_auth
            .iter()
            .position(|r| r == "--socks-url")
            .unwrap();
        assert_eq!(
            args_socks_auth[socks_idx_auth + 1],
            "socks5h://user:pass@5.6.7.8:1088"
        );
    }
}
