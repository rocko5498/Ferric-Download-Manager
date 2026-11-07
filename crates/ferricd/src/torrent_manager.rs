use crate::db;
use crate::torrent::validate_torrent_relative_path;
use crate::torrent_engine::{
    format_engine_ref, parse_engine_ref, proxy_privacy_boundary, resolve_save_root, EngineFile,
    EnginePeerStats, RqbitEngine,
};
use anyhow::{bail, Result};
use ferric_common::{
    PrivacyCheckResult, TorrentDetail, TorrentEngineStatus, TorrentFileRecord, TorrentInputKind,
    TorrentPolicy, TorrentProbeResponse, TorrentRecord, TorrentStageRequest, TorrentStatus,
    TorrentTrackerRecord,
};
use rand::RngCore;
use sha1::{Digest, Sha1};
use sqlx::SqlitePool;
use uuid::Uuid;

/// Resolve rqbit API identity for a stored torrent.
/// Prefer `info_hash` over the numeric id embedded in `engine` — rqbit renumbers
/// ids on restart (live 2026-07-17: same ubuntu torrent moved |123 → id=0).
fn rqbit_endpoint_and_id(record: &TorrentRecord) -> Option<(String, String)> {
    let (endpoint, external_id) = parse_engine_ref(&record.engine)?;
    let id = if record.info_hash.is_empty() {
        external_id.to_string()
    } else {
        record.info_hash.clone()
    };
    Some((endpoint.to_string(), id))
}

#[derive(Clone)]
pub struct TorrentManager {
    pool: SqlitePool,
    engine: RqbitEngine,
}

impl TorrentManager {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            engine: RqbitEngine::new(),
        }
    }

    pub async fn list(&self) -> Result<Vec<TorrentRecord>> {
        db::torrents::list(&self.pool).await
    }

    pub async fn detail(&self, id: &str) -> Result<Option<TorrentDetail>> {
        db::torrents::get_detail(&self.pool, id).await
    }

    pub async fn stage(&self, request: TorrentStageRequest) -> Result<TorrentDetail> {
        let info_hash = request
            .probe
            .info_hash
            .clone()
            .ok_or_else(|| anyhow::anyhow!("torrent probe has no info hash"))?;
        if request.probe.kind == TorrentInputKind::Unsupported {
            bail!("unsupported torrent input cannot be staged");
        }

        // S12: validate save_root is absolute (or empty) before persisting.
        if let Some(ref root) = request.save_root {
            crate::security::validate_save_path(root)?;
        }

        // Same magnet/.torrent twice → return existing row (UNIQUE info_hash), not a SQL error.
        if let Some(existing) = db::torrents::get_by_info_hash(&self.pool, &info_hash).await? {
            return self
                .detail(&existing.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("existing torrent missing after lookup"));
        }

        let id = Uuid::new_v4().to_string();
        let mut policy = request.policy;
        enforce_safe_policy_defaults(&mut policy);

        let is_private = request.probe.is_private;
        let mut policy_locked_fields = Vec::new();
        // Global setting wins: never stage a torrent that can start bare on the public net
        // while torrent_proxy_required is on.
        if setting_truthy(db::settings::get(&self.pool, "torrent_proxy_required").await?) {
            policy.proxy_required = true;
            if !policy_locked_fields.iter().any(|f| f == "proxy_required") {
                policy_locked_fields.push("proxy_required".to_string());
            }
        }
        // Global discovery settings are the master switch for *public* torrents.
        // Bug (live 2026-07-17): we only forced OFF when setting was "0", never ON
        // when setting was "1" — so stage kept TorrentPolicy::default() (all false)
        // and public ubuntu saw DHT/PEX/LSD off even with torrent_dht_enabled=1 →
        // known peers stuck at ~2, download crawled.
        if is_private {
            policy.dht_enabled = false;
            policy.pex_enabled = false;
            policy.lsd_enabled = false;
            policy_locked_fields.push("dht_enabled".to_string());
            policy_locked_fields.push("pex_enabled".to_string());
            policy_locked_fields.push("lsd_enabled".to_string());
        } else {
            policy.dht_enabled =
                setting_truthy(db::settings::get(&self.pool, "torrent_dht_enabled").await?);
            policy.pex_enabled =
                setting_truthy(db::settings::get(&self.pool, "torrent_pex_enabled").await?);
            policy.lsd_enabled =
                setting_truthy(db::settings::get(&self.pool, "torrent_lsd_enabled").await?);
        }

        let peer_id_seed = generate_peer_id_seed();

        let record = TorrentRecord {
            id: id.clone(),
            info_hash,
            name: request
                .probe
                .name
                .clone()
                .unwrap_or_else(|| "Unnamed torrent".to_string()),
            source: request.probe.input.clone(),
            source_kind: request.probe.kind.clone(),
            save_root: request.save_root,
            status: TorrentStatus::EngineGated,
            total_size: request.probe.total_size.unwrap_or(0),
            downloaded: 0,
            uploaded: 0,
            ratio: 0.0,
            download_speed: 0,
            upload_speed: 0,
            peers: 0,
            policy,
            proxy_id: None,
            policy_locked_fields,
            is_private,
            peer_id_seed,
            engine: "librqbit-candidate/gated".to_string(),
            created_at: now_ms(),
            completed_at: None,
            av_status: None,
            av_result: None,
            tags: Vec::new(),
            queue_position: 0,
            category: None,
        };

        let files = staged_files(&id, &request.probe);
        let trackers = request
            .probe
            .trackers
            .iter()
            .map(|url| TorrentTrackerRecord {
                id: Uuid::new_v4().to_string(),
                torrent_id: id.clone(),
                url: url.clone(),
                status: "staged".to_string(),
                last_error: None,
            })
            .collect::<Vec<_>>();

        db::torrents::upsert_record(&self.pool, &record).await?;
        db::torrents::replace_files(&self.pool, &id, &files).await?;
        db::torrents::replace_trackers(&self.pool, &id, &trackers).await?;
        self.detail(&id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("staged torrent missing after insert"))
    }

    pub async fn update_policy(
        &self,
        id: &str,
        mut policy: TorrentPolicy,
    ) -> Result<TorrentDetail> {
        enforce_safe_policy_defaults(&mut policy);
        db::torrents::update_policy(&self.pool, id, &policy).await?;
        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))
    }

    pub async fn update_file_selection(
        &self,
        id: &str,
        file_id: &str,
        selected: bool,
        priority: i32,
    ) -> Result<TorrentDetail> {
        if !(-1..=1).contains(&priority) {
            bail!("torrent file priority must be -1, 0, or 1");
        }
        if !db::torrents::update_file_selection(&self.pool, id, file_id, selected, priority).await?
        {
            bail!("torrent file not found");
        }
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        self.apply_engine_file_selection(&detail).await?;
        Ok(detail)
    }

    pub async fn assign_proxy(&self, id: &str, proxy_id: Option<String>) -> Result<TorrentDetail> {
        if let Some(proxy_id) = proxy_id.as_deref() {
            if db::proxies::get(&self.pool, proxy_id).await?.is_none() {
                bail!("proxy config not found");
            }
        }
        db::torrents::assign_proxy(&self.pool, id, proxy_id.as_deref()).await?;
        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))
    }

    pub async fn run_privacy_check(&self, id: &str) -> Result<PrivacyCheckResult> {
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let record = &detail.record;
        let policy = &record.policy;

        let mut violations: Vec<String> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        let global_proxy_required =
            setting_truthy(db::settings::get(&self.pool, "torrent_proxy_required").await?);
        let proxy_required = policy.proxy_required || global_proxy_required;
        let global_proxy_url = db::settings::get(&self.pool, "torrent_proxy_url")
            .await?
            .filter(|s| !s.trim().is_empty());
        let kill_active =
            setting_truthy(db::settings::get(&self.pool, "torrent_kill_switch_active").await?);

        if kill_active {
            violations.push(
                "Kill switch is active (torrent proxy unreachable or missing while required). Fix proxy/VPN, then clear kill switch / Resume after health recovers.".to_string(),
            );
        }

        let assigned_proxy = match record.proxy_id.as_deref() {
            Some(proxy_id) => db::proxies::get(&self.pool, proxy_id).await?,
            None => None,
        };
        // A settings URL only counts if it parses the same way engine start parses
        // it (`start()` -> runtime_from_proxy_url). A typo that the engine will
        // reject must not read here as "you are proxied".
        let global_proxy_usable = global_proxy_url
            .as_deref()
            .is_some_and(|url| db::proxies::runtime_from_proxy_url(url).is_ok());
        let proxy_configured = assigned_proxy.is_some() || global_proxy_usable;
        // Whether the daemon can honestly claim traffic egresses through that proxy.
        // Evaluated regardless of `proxy_required`, because the exposure is the same
        // either way — only the gating differs.
        let boundary =
            proxy_privacy_boundary(assigned_proxy.as_ref().map(|p| (p.host.as_str(), p.port)));
        let proxy_enforced = proxy_configured && boundary.allows_proxy_assigned_start;

        if record.proxy_id.is_some() && assigned_proxy.is_none() {
            let prefix = if proxy_required {
                "Proxy required but"
            } else {
                "Proxy assigned but"
            };
            violations.push(format!(
                "{prefix} the assigned proxy config no longer exists. Assign a valid proxy before engine start."
            ));
        } else if proxy_required && !proxy_enforced {
            // Strict path only when privacy is required (qB-style: optional SOCKS is best-effort).
            if !proxy_configured {
                violations.push(
                    "Proxy required but no proxy is assigned. Set torrent_proxy_url (socks5://…) or assign a proxy in Proxies before engine start. Bare public-IP BitTorrent is refused while torrent_proxy_required=1 or policy.proxy_required.".to_string(),
                );
            } else if assigned_proxy.is_some() {
                violations.push(format!(
                    "Proxy is assigned and privacy is required, but managed rqbit cannot claim leak-proof routing without a packet-verified boundary. {}",
                    boundary.explanation
                ));
            } else {
                violations.push(format!(
                    "Proxy required and torrent_proxy_url is set, but privacy-required mode needs a packet-verified boundary (SOCKS alone is not leak-proof). {}",
                    boundary.explanation
                ));
            }
        }

        if global_proxy_url.is_some() && !global_proxy_usable {
            warnings.push(
                "torrent_proxy_url is set but is not a usable proxy URL (expected scheme://host:port, e.g. socks5://127.0.0.1:9050). It is being ignored, so it is protecting nothing."
                    .to_string(),
            );
        }

        if !proxy_configured {
            warnings.push(
                "No proxy/VPN configured — your real IP will be visible to the tracker and all peers in this swarm. Assign a proxy in Proxies or set torrent_proxy_url (socks5://…) to avoid exposing it."
                    .to_string(),
            );
        } else if !proxy_enforced && !proxy_required {
            // A configured proxy is not a routed proxy. Saying nothing here would
            // reproduce the false green this warning list exists to prevent, one
            // step further along. When proxy_required is on, the violation above
            // already says this and blocks the start.
            warnings.push(format!(
                "A proxy is configured, but Ferric cannot verify that torrent traffic actually leaves through it, so your real IP may still be visible to the tracker and peers. {}",
                boundary.explanation
            ));
        }

        if record.is_private {
            if policy.dht_enabled {
                violations.push("Private torrent: DHT must be off. DHT leaks swarm membership to the global network, violating BEP-27 and private tracker rules.".to_string());
            }
            if policy.pex_enabled {
                violations.push("Private torrent: PEX must be off. Peer exchange distributes your IP to peers outside the tracker's knowledge, violating private tracker rules.".to_string());
            }
            if policy.lsd_enabled {
                violations.push("Private torrent: LSD must be off. Local service discovery broadcasts swarm membership on the LAN.".to_string());
            }
        }

        if policy.seed_enabled
            && policy.seed_ratio_limit == Some(0.0)
            && policy.seed_time_limit_min == Some(0)
        {
            violations.push("Seeding is enabled with no ratio or time limit. This means unlimited background upload — set at least one bound.".to_string());
        }

        if policy.mse_required {
            violations.push("MSE/PE encryption is required by policy, but the current rqbit sidecar adapter does not expose a packet-verified force-encryption mode. Engine start is refused instead of silently allowing plaintext peer connections.".to_string());
        }

        if record.peer_id_seed.is_empty() {
            violations.push("No peer identity seed on record. The engine would use a deterministic default that could be correlated across sessions.".to_string());
        }

        Ok(PrivacyCheckResult {
            passed: violations.is_empty(),
            violations,
            warnings,
        })
    }

    /// Re-attach librqbit engines for torrents that were actively downloading
    /// when the daemon last stopped. Mirrors `DownloadManager::recover_incomplete()`
    /// (main.rs calls this at boot) -- only resumes torrents already in
    /// Downloading status (i.e. already explicitly started by the user before);
    /// never auto-starts a torrent that was merely staged/gated/paused, since
    /// that would start peer activity without a fresh explicit user action.
    pub async fn resume_active(&self) {
        let all = match db::torrents::list(&self.pool).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("resume_active: failed to list torrents: {e}");
                return;
            }
        };
        let active: Vec<_> = all
            .into_iter()
            .filter(|t| {
                // Never auto-resume completed (even if engine string still looks live).
                if t.status == TorrentStatus::Completed {
                    return false;
                }
                matches!(
                    t.status,
                    TorrentStatus::Downloading | TorrentStatus::Checking
                )
            })
            .collect();
        if active.is_empty() {
            return;
        }
        tracing::info!(count = active.len(), "resuming active torrent engines");
        for record in active {
            if let Err(e) = self.start(&record.id).await {
                tracing::warn!(id = %record.id, name = %record.name, "failed to resume torrent engine: {e}");
            }
        }
    }

    pub async fn start(&self, id: &str) -> Result<TorrentDetail> {
        // Explicit master switch: set torrent_engine_enabled=0 to disable all peer activity.
        // Default when unset = enabled (subject to privacy gates). Only "0"/"false" disables.
        if let Ok(Some(v)) = db::settings::get(&self.pool, "torrent_engine_enabled").await {
            if matches!(v.trim(), "0" | "false" | "FALSE" | "no" | "off") {
                bail!(
                    "torrent engine disabled (torrent_engine_enabled=0). \
                     Set torrent_engine_enabled=1 only if you accept BitTorrent network exposure."
                );
            }
        }

        let check = self.run_privacy_check(id).await?;
        if !check.passed {
            bail!(
                "torrent privacy gate refused engine start: {}",
                check.violations.join("; ")
            );
        }

        let mut detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;

        // Re-apply global discovery for public torrents at start (fixes torrents
        // staged before policy-from-settings was applied correctly).
        if !detail.record.is_private {
            let dht = setting_truthy(db::settings::get(&self.pool, "torrent_dht_enabled").await?);
            let pex = setting_truthy(db::settings::get(&self.pool, "torrent_pex_enabled").await?);
            let lsd = setting_truthy(db::settings::get(&self.pool, "torrent_lsd_enabled").await?);
            if detail.record.policy.dht_enabled != dht
                || detail.record.policy.pex_enabled != pex
                || detail.record.policy.lsd_enabled != lsd
            {
                detail.record.policy.dht_enabled = dht;
                detail.record.policy.pex_enabled = pex;
                detail.record.policy.lsd_enabled = lsd;
                db::torrents::upsert_record(&self.pool, &detail.record).await?;
            }
        }

        // ── Never re-add a torrent already in the rqbit session ────────────
        // Live logs 2026-07-17 showed repeated `add_torrent` for the same ubuntu
        // ISO → double `initialize_and_start`, checksum % > 100, and
        // `Error setting length … file is None`. Fall back to re-POST only when
        // the hash is absent from the session.
        if let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) {
            let hash = detail.record.info_hash.clone();
            let already = !hash.is_empty()
                && self
                    .engine
                    .session_has_info_hash(&endpoint, &hash)
                    .await
                    .unwrap_or(false);
            if already {
                let key = if hash.is_empty() {
                    external_id.as_str()
                } else {
                    hash.as_str()
                };
                // Only unpause when paused; leave live/initializing alone.
                match self.engine.stats(&endpoint, key).await {
                    Ok(st) if st.state == "live" || st.state == "initializing" => {
                        tracing::debug!(
                            torrent_id = id,
                            state = %st.state,
                            "rqbit already has torrent; skip re-add"
                        );
                    }
                    Ok(st) if st.state == "paused" || st.state.is_empty() => {
                        if let Err(e) = self.engine.start_existing(&endpoint, key).await {
                            tracing::warn!(
                                torrent_id = id,
                                error = %e,
                                "start_existing failed for session torrent"
                            );
                        }
                    }
                    Ok(st) => {
                        tracing::debug!(torrent_id = id, state = %st.state, "start_existing for other state");
                        let _ = self.engine.start_existing(&endpoint, key).await;
                    }
                    Err(_) => {
                        let _ = self.engine.start_existing(&endpoint, key).await;
                    }
                }
                detail.record.status = TorrentStatus::Downloading;
                detail.record.engine = format_engine_ref(&endpoint, key);
                db::torrents::upsert_record(&self.pool, &detail.record).await?;
                let _ = db::torrent_audit::append(
                    &self.pool,
                    id,
                    "resumed",
                    &serde_json::json!({ "engine": detail.record.engine, "via": "session_reuse" }),
                )
                .await;
                return self
                    .detail(id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("torrent not found after engine start"));
            }

            // Hash not in session yet — try unpause of stored id once, then add.
            match self.engine.start_existing(&endpoint, &external_id).await {
                Ok(()) => {
                    detail.record.status = TorrentStatus::Downloading;
                    detail.record.engine = format_engine_ref(
                        &endpoint,
                        if hash.is_empty() {
                            external_id.as_str()
                        } else {
                            hash.as_str()
                        },
                    );
                    db::torrents::upsert_record(&self.pool, &detail.record).await?;
                    let _ = db::torrent_audit::append(
                        &self.pool,
                        id,
                        "resumed",
                        &serde_json::json!({ "engine": detail.record.engine, "via": "start_existing" }),
                    )
                    .await;
                    return self
                        .detail(id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("torrent not found after engine start"));
                }
                Err(e) => {
                    tracing::debug!(
                        torrent_id = id,
                        error = %e,
                        "start_existing failed; will add only if hash still absent"
                    );
                }
            }
        }

        let save_root = resolve_save_root(
            detail.record.save_root.as_deref(),
            db::settings::get(&self.pool, "default_save_dir").await?,
        )?;
        if let Err(e) = tokio::fs::create_dir_all(&save_root).await {
            bail!(
                "torrent save folder unavailable ({}): {e}. Set Location or Preferences → default save dir.",
                save_root.display()
            );
        }
        // Persist default when user never configured one so Path column and next starts stay stable.
        if db::settings::get(&self.pool, "default_save_dir")
            .await?
            .as_ref()
            .is_none_or(|v| v.trim().is_empty())
        {
            let _ = db::settings::set(&self.pool, "default_save_dir", &save_root.to_string_lossy())
                .await;
        }
        let global_proxy_required =
            setting_truthy(db::settings::get(&self.pool, "torrent_proxy_required").await?);
        let privacy_required = detail.record.policy.proxy_required || global_proxy_required;

        // Prefer assigned proxy; else settings torrent_proxy_url (SOCKS/HTTP best-effort).
        let runtime_proxy = match detail.record.proxy_id.as_deref() {
            Some(proxy_id) => db::proxies::get_runtime(&self.pool, proxy_id).await?,
            None => match db::settings::get(&self.pool, "torrent_proxy_url").await? {
                Some(url) if !url.trim().is_empty() => {
                    Some(db::proxies::runtime_from_proxy_url(&url)?)
                }
                _ => None,
            },
        };
        let selected_paths = selected_file_paths(&detail.files)?;

        let blocklist_url = if detail.record.policy.blocklist_enabled {
            db::settings::get(&self.pool, "blocklist_url")
                .await?
                .or(db::settings::get(&self.pool, "torrent_blocklist_url").await?)
        } else {
            None
        };

        // Inbound peer listen off by default (outbound-only). Opt in via setting.
        let tcp_listen_enabled =
            setting_truthy(db::settings::get(&self.pool, "torrent_tcp_listen_enabled").await?);

        // Resuming into files this torrent already partially wrote (either a manual
        // re-start after pause, or TorrentManager::resume_active() on daemon boot)
        // must tell rqbit it's OK to write into the existing partial file.
        // NOTE: detail.record.downloaded is NOT a reliable signal here -- it only
        // reflects whatever the last stats-sync happened to write, which can stay
        // stale at 0 even after the engine has genuinely written real bytes to disk
        // (confirmed live: a torrent with a real partial .mp4 on disk still showed
        // downloaded=0 in the DB). The reliable signal is status: anything other
        // than the pre-engine-start states means this torrent's engine has been
        // started at least once before, so files may already exist on disk.
        //
        // That status-based signal alone is NOT enough, though (confirmed live,
        // 2026-07-17, via the qBittorrent import feature): a torrent imported
        // from a third party is always freshly EngineGated in *our* DB even
        // though its files already exist on disk from the other client's prior
        // download. So also check disk directly for any selected file already
        // present under save_root -- that's the actually-correct signal for
        // "rqbit may write into an existing file here", independent of why.
        let status_implies_resume = !matches!(
            detail.record.status,
            TorrentStatus::Staged | TorrentStatus::MetadataReady | TorrentStatus::EngineGated
        );
        let files_already_on_disk = selected_paths.iter().any(|p| save_root.join(p).exists());
        let resume = status_implies_resume || files_already_on_disk;
        // Last chance: hash may have appeared mid-flight (parallel Start / resume_active).
        if !detail.record.info_hash.is_empty() {
            // Probe any known endpoint from a previous engine ref, or skip.
            if let Some((endpoint, _)) = rqbit_endpoint_and_id(&detail.record) {
                if self
                    .engine
                    .session_has_info_hash(&endpoint, &detail.record.info_hash)
                    .await
                    .unwrap_or(false)
                {
                    let key = detail.record.info_hash.as_str();
                    let _ = self.engine.start_existing(&endpoint, key).await;
                    detail.record.status = TorrentStatus::Downloading;
                    detail.record.engine = format_engine_ref(&endpoint, key);
                    detail.record.save_root = Some(save_root.to_string_lossy().to_string());
                    db::torrents::upsert_record(&self.pool, &detail.record).await?;
                    return self
                        .detail(id)
                        .await?
                        .ok_or_else(|| anyhow::anyhow!("torrent not found after engine start"));
                }
            }
        }

        let started = self
            .engine
            .start_torrent(
                &detail.record.source,
                &save_root,
                &detail.record.peer_id_seed,
                runtime_proxy.as_ref(),
                detail.record.policy.dht_enabled,
                blocklist_url.as_deref(),
                &selected_paths,
                tcp_listen_enabled,
                privacy_required,
                detail.record.policy.seed_enabled,
                resume,
            )
            .await?;
        // Prefer hash in engine ref (stable); fall back to whatever start returned.
        let stable_id = if detail.record.info_hash.is_empty() {
            started.external_id.clone()
        } else {
            detail.record.info_hash.clone()
        };
        // Add already starts the torrent — do NOT call start_existing again
        // (that caused duplicate initialize_and_start in rqbit logs).
        detail.record.status = TorrentStatus::Downloading;
        detail.record.save_root = Some(save_root.to_string_lossy().to_string());
        detail.record.engine = format_engine_ref(&started.endpoint, &stable_id);
        db::torrents::upsert_record(&self.pool, &detail.record).await?;
        self.refresh_engine_files(&detail.record.id, &started.endpoint, &stable_id, &[])
            .await?;
        let event = if resume { "resumed" } else { "started" };
        let _ = db::torrent_audit::append(
            &self.pool,
            id,
            event,
            &serde_json::json!({ "engine": detail.record.engine }),
        )
        .await;
        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after engine start"))
    }

    pub async fn pause(&self, id: &str) -> Result<TorrentDetail> {
        let mut detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            bail!("torrent is not attached to rqbit engine");
        };

        // rqbit returns 400 while state=initializing (no pause yet). Still mark
        // Ferric paused so UI/poller stop treating it as active; resume will unpause.
        if let Err(e) = self.engine.pause_torrent(&endpoint, &external_id).await {
            tracing::warn!(
                torrent_id = id,
                error = %e,
                "rqbit pause failed; recording paused in DB anyway"
            );
        }
        // If already fully downloaded, pause → Completed (not a lying "Paused" at 100%).
        let fully_done =
            detail.record.total_size > 0 && detail.record.downloaded >= detail.record.total_size;
        detail.record.status = if fully_done {
            if detail.record.completed_at.is_none() {
                detail.record.completed_at = Some(now_ms());
            }
            TorrentStatus::Completed
        } else {
            TorrentStatus::Paused
        };
        // Keep engine ref on stable hash form.
        detail.record.engine = format_engine_ref(&endpoint, &external_id);
        db::torrents::upsert_record(&self.pool, &detail.record).await?;
        let _ = db::torrent_audit::append(
            &self.pool,
            id,
            if fully_done { "completed" } else { "paused" },
            &serde_json::json!({}),
        )
        .await;

        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after pause"))
    }

    pub async fn force_recheck(&self, id: &str) -> Result<TorrentDetail> {
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            bail!("torrent is not attached to rqbit engine");
        };

        self.engine.force_recheck(&endpoint, &external_id).await?;

        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after recheck"))
    }

    pub async fn force_announce(&self, id: &str) -> Result<TorrentDetail> {
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            bail!("torrent is not attached to rqbit engine");
        };

        if let Err(e) = self.engine.force_announce(&endpoint, &external_id).await {
            let _ = db::torrent_audit::append(
                &self.pool,
                id,
                "announce_failed",
                &serde_json::json!({ "error": e.to_string() }),
            )
            .await;
            return Err(e);
        }

        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after announce"))
    }

    pub async fn set_location(&self, id: &str, new_path: &str) -> Result<TorrentDetail> {
        let mut detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;

        // Update DB first
        detail.record.save_root = Some(new_path.to_string());
        db::torrents::upsert_record(&self.pool, &detail.record).await?;

        // Try to move in engine if attached
        if let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) {
            if let Err(e) = self
                .engine
                .set_location(&endpoint, &external_id, new_path)
                .await
            {
                tracing::warn!(torrent_id = id, error = %e, "engine move failed, DB updated");
            }
        }

        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after location set"))
    }

    pub async fn set_trackers(&self, id: &str, trackers: Vec<String>) -> Result<TorrentDetail> {
        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;

        let records: Vec<TorrentTrackerRecord> = trackers
            .into_iter()
            .map(|url| TorrentTrackerRecord {
                id: Uuid::new_v4().to_string(),
                torrent_id: id.to_string(),
                url,
                status: "staged".to_string(),
                last_error: None,
            })
            .collect();
        db::torrents::replace_trackers(&self.pool, id, &records).await?;

        self.detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after trackers set"))
    }

    pub async fn sync_stats(&self, id: &str) -> Result<TorrentDetail> {
        let mut detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            bail!("torrent is not attached to rqbit engine");
        };

        let mut stats = self.engine.stats(&endpoint, &external_id).await?;
        // Self-heal: Ferric thinks Downloading but rqbit left the torrent paused
        // (common after daemon restart / failed resume). Unpause then re-read.
        if matches!(
            detail.record.status,
            TorrentStatus::Downloading | TorrentStatus::Checking
        ) && stats.state == "paused"
        {
            if let Err(e) = self.engine.start_existing(&endpoint, &external_id).await {
                tracing::debug!(
                    torrent_id = id,
                    error = %e,
                    "engine paused while status=downloading; start_existing failed"
                );
            } else if let Ok(fresh) = self.engine.stats(&endpoint, &external_id).await {
                stats = fresh;
            }
        }
        // Persist hash-based engine ref when still on a numeric id.
        if detail.record.engine != format_engine_ref(&endpoint, &external_id) {
            detail.record.engine = format_engine_ref(&endpoint, &external_id);
        }
        tracing::debug!(
            torrent_id = id,
            download_speed = stats.download_speed,
            upload_speed = stats.upload_speed,
            peers = stats.peers,
            engine_state = %stats.state,
            "synced rqbit torrent stats"
        );

        // T0: during rqbit "initializing" (checksum), stats.downloaded can temporarily
        // under-report vs prior live values. Never write progress backwards — that made
        // the ubuntu ISO jump from ~5.6 GiB down to ~2.1 GiB in SQLite (live 2026-07-17).
        let checking = stats.state == "initializing";
        if checking {
            detail.record.downloaded = detail.record.downloaded.max(stats.downloaded);
            detail.record.uploaded = detail.record.uploaded.max(stats.uploaded);
            detail.record.download_speed = 0;
            detail.record.upload_speed = 0;
            // Peers may still be useful during check; take max so we don't wipe.
            detail.record.peers = detail.record.peers.max(stats.peers);
        } else {
            detail.record.downloaded = stats.downloaded;
            detail.record.uploaded = stats.uploaded;
            detail.record.download_speed = stats.download_speed;
            detail.record.upload_speed = stats.upload_speed;
            detail.record.peers = stats.peers;
        }
        detail.record.ratio = if detail.record.downloaded > 0 {
            detail.record.uploaded as f64 / detail.record.downloaded as f64
        } else {
            0.0
        };

        let is_complete = detail.record.total_size > 0
            && detail.record.downloaded >= detail.record.total_size
            && !checking;
        if is_complete {
            if detail.record.status != TorrentStatus::Completed {
                detail.record.status = TorrentStatus::Completed;
                detail.record.completed_at = Some(now_ms());

                // Trigger AV scan hook
                let pool = self.pool.clone();
                let torrent_id = id.to_string();
                let save_root_res = resolve_save_root(
                    detail.record.save_root.as_deref(),
                    db::settings::get(&self.pool, "default_save_dir").await?,
                );
                let torrent_name = detail.record.name.clone();
                tokio::spawn(async move {
                    match save_root_res {
                        Ok(save_root) => {
                            if let Err(e) = crate::av::scan_torrent(
                                &pool,
                                &torrent_id,
                                &save_root,
                                &torrent_name,
                            )
                            .await
                            {
                                tracing::error!(torrent_id, error = %e, "torrent AV scan failed");
                            }
                        }
                        Err(e) => {
                            tracing::error!(torrent_id, error = %e, "failed to resolve save root for AV scan");
                        }
                    }
                });
            }
        } else if detail.record.status != TorrentStatus::Paused {
            // Honest engine map: rqbit "initializing" is piece-check, not download.
            detail.record.status = if checking {
                TorrentStatus::Checking
            } else {
                TorrentStatus::Downloading
            };
        }

        db::torrents::upsert_record(&self.pool, &detail.record).await?;
        self.refresh_engine_files(id, &endpoint, &external_id, &stats.file_progress)
            .await?;

        // Sync peers and run anomaly detection/bans in the background/inline
        if let Err(e) = self.sync_peers(id).await {
            tracing::debug!(torrent_id = id, error = %e, "failed to sync torrent peers during stats sync");
        }

        // Enforce seeding limits and auto-pause if limit exceeded
        let should_pause = should_auto_pause(
            is_complete,
            &detail.record.status,
            &detail.record.policy,
            detail.record.ratio,
            detail.record.completed_at,
            now_ms(),
        );

        if should_pause {
            if let Err(e) = self.pause(id).await {
                tracing::error!(torrent_id = id, error = %e, "failed to auto-pause torrent after completion limits exceeded");
            }
        }

        // DB does not persist ephemeral speeds/peers (record_from_row zeros them).
        // Re-apply after final detail reload so poller/SSE keep real values.
        let mut out = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found after stats sync"))?;
        out.record.download_speed = stats.download_speed;
        out.record.upload_speed = stats.upload_speed;
        out.record.peers = stats.peers;
        Ok(out)
    }

    pub async fn engine_status(&self) -> TorrentEngineStatus {
        self.engine.status().await
    }

    async fn refresh_engine_files(
        &self,
        id: &str,
        endpoint: &str,
        external_id: &str,
        file_progress: &[u64],
    ) -> Result<()> {
        let engine_files = self.engine.files(endpoint, external_id).await?;
        if engine_files.is_empty() {
            return Ok(());
        }
        let Some(mut detail) = db::torrents::get_detail(&self.pool, id).await? else {
            return Ok(());
        };
        let mut files = merge_engine_files(id, &engine_files, &detail.files)?;
        if file_progress.is_empty() {
            apply_single_file_progress(&mut files, detail.record.downloaded);
        } else {
            apply_file_progress(&mut files, file_progress);
        }
        let total_size = files.iter().map(|file| file.size).sum();
        if total_size > 0 {
            detail.record.total_size = total_size;
            db::torrents::upsert_record(&self.pool, &detail.record).await?;
        }
        db::torrents::replace_files(&self.pool, id, &files).await
    }

    async fn apply_engine_file_selection(&self, detail: &TorrentDetail) -> Result<()> {
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            return Ok(());
        };
        let Some(indices) = selected_engine_indices(&detail.files) else {
            return Ok(());
        };
        self.engine
            .update_only_files(&endpoint, &external_id, &indices)
            .await
    }

    /// Poll rqbit `peer_stats` for a torrent and sync into our DB.
    /// Detects anomalous peers (metadata honeypots) and bans them.
    pub async fn sync_peers(&self, id: &str) -> Result<()> {
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            bail!("torrent is not attached to rqbit engine");
        };

        let engine_peers = self.engine.peer_stats(&endpoint, &external_id).await?;
        for peer in &engine_peers {
            let addr_hash = peer_addr_hash(peer);

            // Skip already-banned peers
            if db::peer_bans::is_banned(&self.pool, id, &addr_hash).await? {
                continue;
            }

            // Upsert peer record
            let state = if peer.interested {
                "interested"
            } else {
                "connected"
            };
            db::peer_bans::upsert_peer_by_addr(
                &self.pool,
                id,
                &addr_hash,
                peer.client.as_deref(),
                peer.downloaded,
                peer.uploaded,
                state,
            )
            .await?;

            // Detect anomalies
            if let Some(reason) = detect_peer_anomaly(peer) {
                tracing::warn!(
                    torrent_id = id,
                    addr_hash = %addr_hash,
                    reason = %reason,
                    "banning anomalous peer"
                );
                db::peer_bans::ban_peer(&self.pool, id, &addr_hash, &reason, None).await?;
            }
        }

        Ok(())
    }

    /// Task 182 — live peers from engine + `GeoIP` eyes-tier (never logs raw IPs).
    pub async fn live_peers_geo(&self, id: &str) -> Result<Vec<ferric_common::LivePeerView>> {
        let detail = self
            .detail(id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("torrent not found"))?;
        let Some((endpoint, external_id)) = rqbit_endpoint_and_id(&detail.record) else {
            return Ok(Vec::new());
        };
        let engine_peers = self.engine.peer_stats(&endpoint, &external_id).await?;
        let mut out = Vec::with_capacity(engine_peers.len());
        for peer in engine_peers {
            let addr_hash = peer_addr_hash(&peer);
            let geo = peer
                .addr
                .as_deref()
                .map_or_else(|| crate::geoip::lookup_addr(""), crate::geoip::lookup_addr);
            let tier = match geo.eyes_tier {
                crate::geoip::EyesTier::Five => "five",
                crate::geoip::EyesTier::Nine => "nine",
                crate::geoip::EyesTier::Fourteen => "fourteen",
                crate::geoip::EyesTier::Other => "other",
                crate::geoip::EyesTier::Local => "local",
                crate::geoip::EyesTier::Unknown => "unknown",
            };
            out.push(ferric_common::LivePeerView {
                addr_hash,
                client: peer.client,
                downloaded: peer.downloaded,
                uploaded: peer.uploaded,
                interested: peer.interested,
                choked: peer.choked,
                country_iso: geo.country_iso,
                country_name: geo.country_name,
                flag_emoji: geo.flag_emoji,
                eyes_tier: tier.into(),
            });
        }
        Ok(out)
    }
}

fn generate_peer_id_seed() -> String {
    let mut seed = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut seed);
    seed.iter().map(|b| format!("{b:02x}")).collect()
}

fn staged_files(torrent_id: &str, probe: &TorrentProbeResponse) -> Vec<TorrentFileRecord> {
    if !probe.files.is_empty() {
        return probe
            .files
            .iter()
            .map(|file| TorrentFileRecord {
                id: Uuid::new_v4().to_string(),
                torrent_id: torrent_id.to_string(),
                path: file.path.clone(),
                size: file.size,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: None,
            })
            .collect();
    }
    let count = probe.file_count.unwrap_or(0);
    if count == 0 {
        return Vec::new();
    }
    if count == 1 {
        return vec![TorrentFileRecord {
            id: Uuid::new_v4().to_string(),
            torrent_id: torrent_id.to_string(),
            path: probe
                .name
                .clone()
                .unwrap_or_else(|| "metadata-pending".to_string()),
            size: probe.total_size.unwrap_or(0),
            selected: true,
            priority: 0,
            downloaded: 0,
            engine_index: None,
        }];
    }
    vec![TorrentFileRecord {
        id: Uuid::new_v4().to_string(),
        torrent_id: torrent_id.to_string(),
        path: format!("{count} files - detailed tree pending engine metadata handoff"),
        size: probe.total_size.unwrap_or(0),
        selected: true,
        priority: 0,
        downloaded: 0,
        engine_index: None,
    }]
}

fn selected_file_paths(files: &[TorrentFileRecord]) -> Result<Vec<String>> {
    if files.is_empty() {
        return Ok(Vec::new());
    }
    let selected = files
        .iter()
        .filter(|file| file.selected)
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    if selected.is_empty() {
        bail!("select at least one torrent file before starting the engine");
    }
    Ok(selected)
}

fn merge_engine_files(
    torrent_id: &str,
    engine_files: &[EngineFile],
    existing: &[TorrentFileRecord],
) -> Result<Vec<TorrentFileRecord>> {
    engine_files
        .iter()
        .map(|file| {
            validate_torrent_relative_path(&file.path)?;
            let previous = existing.iter().find(|old| old.path == file.path);
            Ok(TorrentFileRecord {
                id: previous.map_or_else(|| Uuid::new_v4().to_string(), |old| old.id.clone()),
                torrent_id: torrent_id.to_string(),
                path: file.path.clone(),
                size: file.size,
                selected: previous.is_none_or(|old| old.selected),
                priority: previous.map_or(0, |old| old.priority),
                downloaded: file.downloaded,
                engine_index: Some(file.index),
            })
        })
        .collect()
}

fn selected_engine_indices(files: &[TorrentFileRecord]) -> Option<Vec<u64>> {
    files
        .iter()
        .filter(|file| file.selected)
        .map(|file| file.engine_index)
        .collect()
}

fn apply_single_file_progress(files: &mut [TorrentFileRecord], total_downloaded: u64) {
    if let [file] = files {
        file.downloaded = total_downloaded.min(file.size);
    }
}

/// Apply rqbit `/stats/v1` `file_progress` (index-aligned) onto merged file rows.
fn apply_file_progress(files: &mut [TorrentFileRecord], file_progress: &[u64]) {
    for file in files.iter_mut() {
        let Some(idx) = file.engine_index else {
            continue;
        };
        if let Some(&bytes) = file_progress.get(idx as usize) {
            file.downloaded = bytes.min(file.size);
        }
    }
}

#[cfg(test)]
fn jitter_announce_interval(base_secs: u64, jitter_bps: i32, sample_bps: i32) -> u64 {
    let jitter_bps = jitter_bps.clamp(1_000, 3_000);
    let sample_bps = sample_bps.clamp(-jitter_bps, jitter_bps);
    let multiplier_bps = 10_000 + sample_bps;
    ((base_secs.saturating_mul(multiplier_bps as u64)) / 10_000).max(1)
}

/// Hash a peer address for storage. We never store raw IPs.
fn peer_addr_hash(peer: &EnginePeerStats) -> String {
    let input = peer
        .addr
        .as_deref()
        .or(peer.peer_id.as_deref())
        .unwrap_or("unknown");
    let mut hasher = Sha1::new();
    hasher.update(b"ferric-peer-addr-v1");
    hasher.update(input.as_bytes());
    let hash = hasher.finalize();
    hash.iter().map(|b| format!("{b:02x}")).collect()
}

/// Detect metadata honeypot patterns in a peer's stats.
/// Returns a reason string if the peer is anomalous, None otherwise.
///
/// Honeypot indicators:
/// - Peer has no pieces (hasn't downloaded anything from the swarm)
/// - Peer shows interest but never uploads
/// - Peer is not choked but still has zero upload
fn detect_peer_anomaly(peer: &EnginePeerStats) -> Option<String> {
    // Pattern: peer connected, interested, not choked, has no pieces, never uploads.
    // This matches monitoring nodes that join swarms to collect participant IPs
    // without contributing any data.
    if !peer.has_pieces && peer.interested && !peer.choked && peer.uploaded == 0 {
        return Some(
            "Metadata honeypot pattern: interested, unchoked, no pieces, zero upload".to_string(),
        );
    }

    // Pattern: peer has been transferring but ratio is extremely lopsided
    // (downloaded a lot from us, uploaded nothing). Threshold: >64 KiB downloaded,
    // zero uploaded.
    if peer.downloaded > 65_536 && peer.uploaded == 0 && !peer.has_pieces {
        return Some(format!(
            "Leeching monitor pattern: downloaded {} bytes, uploaded 0, no pieces",
            peer.downloaded
        ));
    }

    None
}

fn should_auto_pause(
    is_complete: bool,
    status: &TorrentStatus,
    policy: &TorrentPolicy,
    ratio: f64,
    completed_at: Option<u64>,
    now: u64,
) -> bool {
    if !is_complete || *status == TorrentStatus::Paused {
        return false;
    }
    if !policy.seed_enabled {
        return true;
    }
    if let Some(ratio_limit) = policy.seed_ratio_limit {
        if ratio_limit > 0.0 && ratio >= ratio_limit {
            return true;
        }
    }
    if let Some(time_limit_mins) = policy.seed_time_limit_min {
        if time_limit_mins > 0 {
            if let Some(completed_time) = completed_at {
                let limit_ms = time_limit_mins * 60 * 1000;
                if now >= completed_time.saturating_add(limit_ms) {
                    return true;
                }
            }
        }
    }
    false
}

fn enforce_safe_policy_defaults(policy: &mut TorrentPolicy) {
    if !policy.seed_enabled {
        policy.seed_ratio_limit = Some(0.0);
        policy.seed_time_limit_min = Some(0);
    }
}

fn setting_truthy(value: Option<String>) -> bool {
    matches!(
        value.as_deref().map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "on")
    )
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_defaults_disable_silent_seeding() {
        let mut policy = TorrentPolicy {
            seed_enabled: false,
            seed_ratio_limit: Some(2.0),
            seed_time_limit_min: Some(120),
            ..TorrentPolicy::default()
        };

        enforce_safe_policy_defaults(&mut policy);

        assert!(!policy.seed_enabled);
        assert_eq!(policy.seed_ratio_limit, Some(0.0));
        assert_eq!(policy.seed_time_limit_min, Some(0));
        assert!(!policy.dht_enabled);
        assert!(!policy.pex_enabled);
        assert!(!policy.lsd_enabled);
    }

    #[test]
    fn peer_id_seed_is_40_hex_chars() {
        let seed = generate_peer_id_seed();
        assert_eq!(seed.len(), 40, "20 bytes → 40 hex chars");
        assert!(seed.chars().all(|c| c.is_ascii_hexdigit()), "must be hex");
    }

    #[test]
    fn peer_id_seeds_are_unique() {
        let a = generate_peer_id_seed();
        let b = generate_peer_id_seed();
        assert_ne!(a, b, "CSPRNG seeds must not repeat");
    }

    #[test]
    fn staged_files_use_probe_file_list_when_available() {
        let files = staged_files(
            "torrent-1",
            &TorrentProbeResponse {
                input: "test.torrent".to_string(),
                kind: TorrentInputKind::TorrentFile,
                supported: true,
                name: Some("bundle".to_string()),
                info_hash: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                trackers: Vec::new(),
                files: vec![
                    ferric_common::TorrentProbeFile {
                        path: "dir/file.txt".to_string(),
                        size: 10,
                    },
                    ferric_common::TorrentProbeFile {
                        path: "other/data.bin".to_string(),
                        size: 20,
                    },
                ],
                file_count: Some(2),
                total_size: Some(30),
                is_private: false,
                warnings: Vec::new(),
                explanation: "test".to_string(),
            },
        );

        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "dir/file.txt");
        assert_eq!(files[0].size, 10);
        assert!(files[0].selected);
        assert_eq!(files[1].path, "other/data.bin");
    }

    #[test]
    fn selected_file_paths_refuses_empty_selection() {
        let files = vec![TorrentFileRecord {
            id: "file-1".to_string(),
            torrent_id: "torrent-1".to_string(),
            path: "dir/file.txt".to_string(),
            size: 10,
            selected: false,
            priority: 0,
            downloaded: 0,
            engine_index: None,
        }];

        let err = selected_file_paths(&files).expect_err("empty selection must fail");

        assert!(err.to_string().contains("select at least one"));
    }

    #[test]
    fn merge_engine_files_preserves_selection_and_rejects_unsafe_paths() -> Result<()> {
        let existing = vec![TorrentFileRecord {
            id: "existing-id".to_string(),
            torrent_id: "torrent-1".to_string(),
            path: "dir/file.txt".to_string(),
            size: 10,
            selected: false,
            priority: -1,
            downloaded: 0,
            engine_index: Some(3),
        }];
        let files = merge_engine_files(
            "torrent-1",
            &[
                EngineFile {
                    index: 3,
                    path: "dir/file.txt".to_string(),
                    size: 20,
                    downloaded: 5,
                },
                EngineFile {
                    index: 4,
                    path: "new.bin".to_string(),
                    size: 30,
                    downloaded: 0,
                },
            ],
            &existing,
        )?;

        assert_eq!(files[0].id, "existing-id");
        assert_eq!(files[0].engine_index, Some(3));
        assert!(!files[0].selected);
        assert_eq!(files[0].priority, -1);
        assert!(files[1].selected);
        assert_eq!(files[1].engine_index, Some(4));

        let err = merge_engine_files(
            "torrent-1",
            &[EngineFile {
                index: 0,
                path: "../escape.bin".to_string(),
                size: 1,
                downloaded: 0,
            }],
            &[],
        )
        .expect_err("unsafe engine path must be rejected");
        assert!(err.to_string().contains("unsafe component"));
        Ok(())
    }

    #[test]
    fn selected_engine_indices_require_synced_metadata() {
        let files = vec![
            TorrentFileRecord {
                id: "file-1".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "a.bin".to_string(),
                size: 1,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: Some(7),
            },
            TorrentFileRecord {
                id: "file-2".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "b.bin".to_string(),
                size: 1,
                selected: false,
                priority: 0,
                downloaded: 0,
                engine_index: Some(8),
            },
        ];

        assert_eq!(selected_engine_indices(&files), Some(vec![7]));

        let mut missing = files;
        missing[0].engine_index = None;
        assert_eq!(selected_engine_indices(&missing), None);
    }

    #[test]
    fn single_file_progress_uses_total_downloaded_without_faking_multi_file() {
        let mut single = vec![TorrentFileRecord {
            id: "file-1".to_string(),
            torrent_id: "torrent-1".to_string(),
            path: "a.bin".to_string(),
            size: 10,
            selected: true,
            priority: 0,
            downloaded: 0,
            engine_index: Some(0),
        }];
        apply_single_file_progress(&mut single, 12);
        assert_eq!(single[0].downloaded, 10);

        let mut multi = vec![
            TorrentFileRecord {
                id: "file-1".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "a.bin".to_string(),
                size: 10,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: Some(0),
            },
            TorrentFileRecord {
                id: "file-2".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "b.bin".to_string(),
                size: 10,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: Some(1),
            },
        ];
        apply_single_file_progress(&mut multi, 12);
        assert_eq!(multi[0].downloaded, 0);
        assert_eq!(multi[1].downloaded, 0);
    }

    #[test]
    fn apply_file_progress_maps_rqbit_index_aligned_array() {
        // Live shape: stats/v1.file_progress[i] matches files[i] by engine index.
        let mut multi = vec![
            TorrentFileRecord {
                id: "file-1".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "a.bin".to_string(),
                size: 10,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: Some(0),
            },
            TorrentFileRecord {
                id: "file-2".to_string(),
                torrent_id: "torrent-1".to_string(),
                path: "b.bin".to_string(),
                size: 20,
                selected: true,
                priority: 0,
                downloaded: 0,
                engine_index: Some(1),
            },
        ];
        apply_file_progress(&mut multi, &[7, 20]);
        assert_eq!(multi[0].downloaded, 7);
        assert_eq!(multi[1].downloaded, 20);

        // Cap at size; ignore unknown indices.
        apply_file_progress(&mut multi, &[99, 5]);
        assert_eq!(multi[0].downloaded, 10);
        assert_eq!(multi[1].downloaded, 5);
    }

    #[test]
    fn announce_jitter_stays_inside_privacy_window() {
        let base = 1_800;

        assert_eq!(jitter_announce_interval(base, 2_000, -2_000), 1_440);
        assert_eq!(jitter_announce_interval(base, 2_000, 2_000), 2_160);
        assert_eq!(jitter_announce_interval(base, 500, -10_000), 1_620);
        assert_eq!(jitter_announce_interval(base, 9_000, 10_000), 2_340);
    }

    #[tokio::test]
    async fn update_file_selection_persists_selected_and_priority() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let manager = TorrentManager::new(pool);
        let detail = manager
            .stage(TorrentStageRequest {
                probe: TorrentProbeResponse {
                    input: "test.torrent".to_string(),
                    kind: TorrentInputKind::TorrentFile,
                    supported: true,
                    name: Some("bundle".to_string()),
                    info_hash: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                    trackers: Vec::new(),
                    files: vec![ferric_common::TorrentProbeFile {
                        path: "dir/file.txt".to_string(),
                        size: 10,
                    }],
                    file_count: Some(1),
                    total_size: Some(10),
                    is_private: false,
                    warnings: Vec::new(),
                    explanation: "test".to_string(),
                },
                save_root: Some(dir.path().to_string_lossy().to_string()),
                policy: TorrentPolicy::default(),
            })
            .await?;

        let updated = manager
            .update_file_selection(&detail.record.id, &detail.files[0].id, false, -1)
            .await?;

        assert!(!updated.files[0].selected);
        assert_eq!(updated.files[0].priority, -1);
        Ok(())
    }

    #[tokio::test]
    async fn start_refuses_when_privacy_gate_fails() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let manager = TorrentManager::new(pool);
        let mut policy = TorrentPolicy {
            seed_enabled: true,
            seed_ratio_limit: Some(0.0),
            seed_time_limit_min: Some(0),
            ..TorrentPolicy::default()
        };
        enforce_safe_policy_defaults(&mut policy);
        policy.seed_enabled = true;

        let detail = manager
            .stage(TorrentStageRequest {
                probe: TorrentProbeResponse {
                    input: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"
                        .to_string(),
                    kind: TorrentInputKind::Magnet,
                    supported: true,
                    name: Some("privacy-test".to_string()),
                    info_hash: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                    trackers: Vec::new(),
                    files: Vec::new(),
                    file_count: Some(1),
                    total_size: Some(1024),
                    is_private: false,
                    warnings: Vec::new(),
                    explanation: "test".to_string(),
                },
                save_root: Some(dir.path().to_string_lossy().to_string()),
                policy,
            })
            .await?;

        let err = manager
            .start(&detail.record.id)
            .await
            .expect_err("privacy violations must refuse engine start");

        assert!(err.to_string().contains("privacy gate refused"));
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_requires_assigned_proxy_when_policy_requires_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                proxy_required: true,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(!check.passed);
        assert!(check
            .violations
            .iter()
            .any(|violation| violation.contains("no proxy is assigned")));
        Ok(())
    }

    #[tokio::test]
    async fn assign_proxy_rejects_unknown_proxy_id() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy::default(),
            dir.path().to_string_lossy().to_string(),
        )
        .await?;

        let err = manager
            .assign_proxy(&detail.record.id, Some("missing-proxy".to_string()))
            .await
            .expect_err("unknown proxy id must be rejected");

        assert!(err.to_string().contains("proxy config not found"));
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_refuses_assigned_proxy_until_engine_enforces_it() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let proxy = db::proxies::upsert(
            &pool,
            &ferric_common::ProxyConfigInput {
                name: "local socks".to_string(),
                proxy_type: ferric_common::ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: 1080,
                username: None,
                password: None,
            },
        )
        .await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                proxy_required: true,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;
        manager
            .assign_proxy(&detail.record.id, Some(proxy.id.clone()))
            .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(!check.passed);
        assert!(
            check.violations.iter().any(|violation| {
                violation.contains("leak-proof")
                    || violation.contains("packet-verified")
                    || violation.contains("does not yet enforce proxy routing")
            }),
            "violations={:?}",
            check.violations
        );
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_allows_best_effort_socks_when_not_required() -> Result<()> {
        // Research-aligned: optional SOCKS is best-effort (safer than bare IP), not fail-closed.
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        // Migrations may seed torrent_proxy_required=1; clear for this case.
        db::settings::set(&pool, "torrent_proxy_required", "0").await?;
        let proxy = db::proxies::upsert(
            &pool,
            &ferric_common::ProxyConfigInput {
                name: "local socks".to_string(),
                proxy_type: ferric_common::ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: 1080,
                username: None,
                password: None,
            },
        )
        .await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                proxy_required: false,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;
        manager
            .assign_proxy(&detail.record.id, Some(proxy.id.clone()))
            .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(
            check.passed,
            "optional SOCKS should pass privacy check; violations={:?}",
            check.violations
        );
        assert!(
            !check
                .warnings
                .iter()
                .any(|w| w.contains("No proxy/VPN configured")),
            "an assigned proxy must not warn as though nothing were configured; warnings={:?}",
            check.warnings
        );
        // Best-effort is not verified: passing the gate must still say so, or this
        // is the same false green one step along.
        assert!(
            check
                .warnings
                .iter()
                .any(|w| w.contains("cannot verify that torrent traffic actually leaves")),
            "unverified proxy must still warn; warnings={:?}",
            check.warnings
        );
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_warns_when_global_proxy_url_is_malformed() -> Result<()> {
        // A typo'd proxy URL is rejected by runtime_from_proxy_url at engine start,
        // so it must not read as "you are proxied" here either.
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::settings::set(&pool, "torrent_proxy_required", "0").await?;
        db::settings::set(&pool, "torrent_proxy_url", "definitely not a url").await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                proxy_required: false,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(
            check
                .warnings
                .iter()
                .any(|w| w.contains("not a usable proxy URL")),
            "warnings={:?}",
            check.warnings
        );
        assert!(
            check
                .warnings
                .iter()
                .any(|w| w.contains("No proxy/VPN configured")),
            "a malformed URL protects nothing, so the bare-IP warning must still fire; warnings={:?}",
            check.warnings
        );
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_warns_when_no_proxy_configured() -> Result<()> {
        // Default config (proxy not required, no proxy assigned) still passes the
        // gate, but must never report a bare "passed" while the real IP is exposed.
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        db::settings::set(&pool, "torrent_proxy_required", "0").await?;
        db::settings::set(&pool, "torrent_proxy_url", "").await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                proxy_required: false,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(
            check.passed,
            "missing proxy must stay a warning, not a gate; violations={:?}",
            check.violations
        );
        assert!(
            check
                .warnings
                .iter()
                .any(|w| w.contains("No proxy/VPN configured")
                    && w.contains("real IP will be visible")),
            "warnings={:?}",
            check.warnings
        );
        Ok(())
    }

    #[tokio::test]
    async fn privacy_check_surfaces_missing_mse_enforcement() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let manager = TorrentManager::new(pool);
        let detail = stage_test_torrent(
            &manager,
            TorrentPolicy {
                mse_required: true,
                ..TorrentPolicy::default()
            },
            dir.path().to_string_lossy().to_string(),
        )
        .await?;

        let check = manager.run_privacy_check(&detail.record.id).await?;

        assert!(!check.passed);
        assert!(check
            .violations
            .iter()
            .any(|violation| violation.contains("MSE/PE encryption is required")));
        Ok(())
    }

    async fn stage_test_torrent(
        manager: &TorrentManager,
        policy: TorrentPolicy,
        save_root: String,
    ) -> Result<TorrentDetail> {
        manager
            .stage(TorrentStageRequest {
                probe: TorrentProbeResponse {
                    input: "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567"
                        .to_string(),
                    kind: TorrentInputKind::Magnet,
                    supported: true,
                    name: Some("privacy-test".to_string()),
                    info_hash: Some("0123456789abcdef0123456789abcdef01234567".to_string()),
                    trackers: Vec::new(),
                    files: Vec::new(),
                    file_count: Some(1),
                    total_size: Some(1024),
                    is_private: false,
                    warnings: Vec::new(),
                    explanation: "test".to_string(),
                },
                save_root: Some(save_root),
                policy,
            })
            .await
    }

    #[test]
    fn test_detect_peer_anomaly_honeypot() {
        let normal_peer = EnginePeerStats {
            addr: Some("127.0.0.1:5000".to_string()),
            peer_id: Some("normal_peer_id".to_string()),
            client: Some("Transmission".to_string()),
            downloaded: 1024,
            uploaded: 512,
            interested: true,
            choked: false,
            has_pieces: true,
        };
        assert!(detect_peer_anomaly(&normal_peer).is_none());

        let honeypot_peer = EnginePeerStats {
            addr: Some("1.2.3.4:5000".to_string()),
            peer_id: Some("honeypot_peer_id".to_string()),
            client: Some("Unknown".to_string()),
            downloaded: 0,
            uploaded: 0,
            interested: true,
            choked: false,
            has_pieces: false,
        };
        let reason = detect_peer_anomaly(&honeypot_peer);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("Metadata honeypot"));
    }

    #[test]
    fn test_detect_peer_anomaly_leeching_monitor() {
        let leech_peer = EnginePeerStats {
            addr: Some("5.6.7.8:5000".to_string()),
            peer_id: Some("leech_peer_id".to_string()),
            client: Some("Unknown".to_string()),
            downloaded: 100_000,
            uploaded: 0,
            interested: true,
            choked: true,
            has_pieces: false,
        };
        let reason = detect_peer_anomaly(&leech_peer);
        assert!(reason.is_some());
        assert!(reason.unwrap().contains("Leeching monitor"));
    }

    #[test]
    fn test_should_auto_pause_scenarios() {
        let mut policy = TorrentPolicy {
            dht_enabled: false,
            pex_enabled: false,
            lsd_enabled: false,
            seed_enabled: false, // Seeding is disabled
            seed_ratio_limit: None,
            seed_time_limit_min: None,
            upload_limit_bps: None,
            proxy_required: false,
            blocklist_enabled: false,
            mse_required: false,
        };

        // 1. Incomplete torrent should not pause
        assert!(!should_auto_pause(
            false,
            &TorrentStatus::Downloading,
            &policy,
            0.0,
            None,
            1000
        ));

        // 2. Already paused torrent should not pause again
        assert!(!should_auto_pause(
            true,
            &TorrentStatus::Paused,
            &policy,
            0.0,
            None,
            1000
        ));

        // 3. Seeding disabled: complete torrent should auto-pause
        assert!(should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            0.0,
            None,
            1000
        ));

        // 4. Seeding enabled, no limits: complete torrent should not pause
        policy.seed_enabled = true;
        assert!(!should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            10.0,
            None,
            1000
        ));

        // 5. Seeding ratio limit reached: auto-pause
        policy.seed_ratio_limit = Some(2.0);
        assert!(!should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            1.5,
            None,
            1000
        ));
        assert!(should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            2.0,
            None,
            1000
        ));
        assert!(should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            2.5,
            None,
            1000
        ));

        // 6. Seeding time limit reached: auto-pause
        policy.seed_ratio_limit = None;
        policy.seed_time_limit_min = Some(10); // 10 minutes (600,000 ms)

        let completed_at = 100_000;
        // Under time limit
        assert!(!should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            0.0,
            Some(completed_at),
            699_999
        ));
        // Hit time limit
        assert!(should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            0.0,
            Some(completed_at),
            700_000
        ));
        // Past time limit
        assert!(should_auto_pause(
            true,
            &TorrentStatus::Downloading,
            &policy,
            0.0,
            Some(completed_at),
            800_000
        ));
    }
}
