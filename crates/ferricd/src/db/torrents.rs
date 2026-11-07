use anyhow::Result;
use ferric_common::{
    TorrentDetail, TorrentFileRecord, TorrentInputKind, TorrentPeerRecord, TorrentPieceRecord,
    TorrentPolicy, TorrentRecord, TorrentStatus, TorrentTrackerRecord,
};
use sqlx::{Row, SqlitePool};

pub async fn upsert_record(pool: &SqlitePool, record: &TorrentRecord) -> Result<()> {
    let locked_json = serde_json::to_string(&record.policy_locked_fields)?;
    sqlx::query(
        r"
        INSERT INTO torrents (
            id, info_hash, name, source, source_kind, save_root, status, total_size, downloaded,
            uploaded, ratio, proxy_id, dht_enabled, pex_enabled, lsd_enabled, seed_enabled,
            seed_ratio_limit, seed_time_limit_min, upload_limit_bps, proxy_required,
            blocklist_enabled, mse_required, is_private, policy_locked_fields, peer_id_seed,
            engine, created_at, completed_at, av_status, av_result
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27, ?28, ?29, ?30)
        ON CONFLICT(id) DO UPDATE SET
            name = excluded.name,
            save_root = excluded.save_root,
            status = excluded.status,
            total_size = excluded.total_size,
            downloaded = excluded.downloaded,
            uploaded = excluded.uploaded,
            ratio = excluded.ratio,
            proxy_id = excluded.proxy_id,
            dht_enabled = excluded.dht_enabled,
            pex_enabled = excluded.pex_enabled,
            lsd_enabled = excluded.lsd_enabled,
            seed_enabled = excluded.seed_enabled,
            seed_ratio_limit = excluded.seed_ratio_limit,
            seed_time_limit_min = excluded.seed_time_limit_min,
            upload_limit_bps = excluded.upload_limit_bps,
            proxy_required = excluded.proxy_required,
            blocklist_enabled = excluded.blocklist_enabled,
            mse_required = excluded.mse_required,
            is_private = excluded.is_private,
            policy_locked_fields = excluded.policy_locked_fields,
            peer_id_seed = excluded.peer_id_seed,
            engine = excluded.engine,
            completed_at = excluded.completed_at,
            av_status = excluded.av_status,
            av_result = excluded.av_result
        ",
    )
    .bind(&record.id)
    .bind(&record.info_hash)
    .bind(&record.name)
    .bind(&record.source)
    .bind(kind_to_str(&record.source_kind))
    .bind(&record.save_root)
    .bind(status_to_str(&record.status))
    .bind(u64_to_i64(record.total_size)?)
    .bind(u64_to_i64(record.downloaded)?)
    .bind(u64_to_i64(record.uploaded)?)
    .bind(record.ratio)
    .bind(&record.proxy_id)
    .bind(bool_to_i64(record.policy.dht_enabled))
    .bind(bool_to_i64(record.policy.pex_enabled))
    .bind(bool_to_i64(record.policy.lsd_enabled))
    .bind(bool_to_i64(record.policy.seed_enabled))
    .bind(record.policy.seed_ratio_limit)
    .bind(record.policy.seed_time_limit_min.map(u64_to_i64).transpose()?)
    .bind(record.policy.upload_limit_bps.map(u64_to_i64).transpose()?)
    .bind(bool_to_i64(record.policy.proxy_required))
    .bind(bool_to_i64(record.policy.blocklist_enabled))
    .bind(bool_to_i64(record.policy.mse_required))
    .bind(bool_to_i64(record.is_private))
    .bind(&locked_json)
    .bind(&record.peer_id_seed)
    .bind(&record.engine)
    .bind(u64_to_i64(record.created_at)?)
    .bind(record.completed_at.map(u64_to_i64).transpose()?)
    .bind(&record.av_status)
    .bind(&record.av_result)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn replace_files(
    pool: &SqlitePool,
    torrent_id: &str,
    files: &[TorrentFileRecord],
) -> Result<()> {
    sqlx::query("DELETE FROM torrent_files WHERE torrent_id = ?1")
        .bind(torrent_id)
        .execute(pool)
        .await?;
    for file in files {
        sqlx::query(
            r"
            INSERT INTO torrent_files (id, torrent_id, path, size, selected, priority, downloaded, engine_index)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ",
        )
        .bind(&file.id)
        .bind(&file.torrent_id)
        .bind(&file.path)
        .bind(u64_to_i64(file.size)?)
        .bind(bool_to_i64(file.selected))
        .bind(file.priority)
        .bind(u64_to_i64(file.downloaded)?)
        .bind(file.engine_index.map(u64_to_i64).transpose()?)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn update_file_selection(
    pool: &SqlitePool,
    torrent_id: &str,
    file_id: &str,
    selected: bool,
    priority: i32,
) -> Result<bool> {
    let result = sqlx::query(
        r"
        UPDATE torrent_files
        SET selected = ?1, priority = ?2
        WHERE torrent_id = ?3 AND id = ?4
        ",
    )
    .bind(bool_to_i64(selected))
    .bind(priority)
    .bind(torrent_id)
    .bind(file_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn replace_trackers(
    pool: &SqlitePool,
    torrent_id: &str,
    trackers: &[TorrentTrackerRecord],
) -> Result<()> {
    sqlx::query("DELETE FROM torrent_trackers WHERE torrent_id = ?1")
        .bind(torrent_id)
        .execute(pool)
        .await?;
    for tracker in trackers {
        sqlx::query(
            r"
            INSERT INTO torrent_trackers (id, torrent_id, url, status, last_error)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ",
        )
        .bind(&tracker.id)
        .bind(&tracker.torrent_id)
        .bind(&tracker.url)
        .bind(&tracker.status)
        .bind(&tracker.last_error)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<TorrentRecord>> {
    let rows = sqlx::query("SELECT * FROM torrents ORDER BY queue_position DESC, created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(record_from_row).collect()
}

pub async fn get_by_info_hash(pool: &SqlitePool, info_hash: &str) -> Result<Option<TorrentRecord>> {
    let row = sqlx::query("SELECT * FROM torrents WHERE info_hash = ?1 COLLATE NOCASE")
        .bind(info_hash)
        .fetch_optional(pool)
        .await?;
    row.map(record_from_row).transpose()
}

pub async fn get_detail(pool: &SqlitePool, id: &str) -> Result<Option<TorrentDetail>> {
    let Some(row) = sqlx::query("SELECT * FROM torrents WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    Ok(Some(TorrentDetail {
        record: record_from_row(row)?,
        files: list_files(pool, id).await?,
        trackers: list_trackers(pool, id).await?,
        peers: list_peers(pool, id).await?,
        pieces: list_pieces(pool, id).await?,
    }))
}

pub async fn remove_torrent(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM torrents WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_category(pool: &SqlitePool, id: &str, category: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE torrents SET category = ?1 WHERE id = ?2")
        .bind(category)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_tags(pool: &SqlitePool, id: &str, tags: &[String]) -> Result<()> {
    let json = serde_json::to_string(tags)?;
    sqlx::query("UPDATE torrents SET tags = ?1 WHERE id = ?2")
        .bind(&json)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_tag(pool: &SqlitePool, id: &str, tag: &str) -> Result<()> {
    let current = get_tags(pool, id).await?;
    if !current.contains(&tag.to_string()) {
        let mut updated = current;
        updated.push(tag.to_string());
        set_tags(pool, id, &updated).await?;
    }
    Ok(())
}

pub async fn remove_tag(pool: &SqlitePool, id: &str, tag: &str) -> Result<()> {
    let current = get_tags(pool, id).await?;
    let updated: Vec<String> = current.into_iter().filter(|t| t != tag).collect();
    set_tags(pool, id, &updated).await?;
    Ok(())
}

pub async fn get_tags(pool: &SqlitePool, id: &str) -> Result<Vec<String>> {
    let row = sqlx::query("SELECT tags FROM torrents WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    match row {
        Some(r) => {
            let json: String = r.try_get("tags")?;
            Ok(serde_json::from_str(&json).unwrap_or_default())
        }
        None => Ok(Vec::new()),
    }
}

pub async fn update_policy(pool: &SqlitePool, id: &str, policy: &TorrentPolicy) -> Result<()> {
    let locked_row = sqlx::query("SELECT policy_locked_fields FROM torrents WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    let locked: Vec<String> = locked_row
        .and_then(|row| row.try_get::<String, _>("policy_locked_fields").ok())
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();

    let dht = if locked.iter().any(|f| f == "dht_enabled") {
        false
    } else {
        policy.dht_enabled
    };
    let pex = if locked.iter().any(|f| f == "pex_enabled") {
        false
    } else {
        policy.pex_enabled
    };
    let lsd = if locked.iter().any(|f| f == "lsd_enabled") {
        false
    } else {
        policy.lsd_enabled
    };

    sqlx::query(
        r"
        UPDATE torrents
        SET dht_enabled = ?2,
            pex_enabled = ?3,
            lsd_enabled = ?4,
            seed_enabled = ?5,
            seed_ratio_limit = ?6,
            seed_time_limit_min = ?7,
            upload_limit_bps = ?8,
            proxy_required = ?9,
            blocklist_enabled = ?10,
            mse_required = ?11
        WHERE id = ?1
        ",
    )
    .bind(id)
    .bind(bool_to_i64(dht))
    .bind(bool_to_i64(pex))
    .bind(bool_to_i64(lsd))
    .bind(bool_to_i64(policy.seed_enabled))
    .bind(policy.seed_ratio_limit)
    .bind(policy.seed_time_limit_min.map(u64_to_i64).transpose()?)
    .bind(policy.upload_limit_bps.map(u64_to_i64).transpose()?)
    .bind(bool_to_i64(policy.proxy_required))
    .bind(bool_to_i64(policy.blocklist_enabled))
    .bind(bool_to_i64(policy.mse_required))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn assign_proxy(pool: &SqlitePool, id: &str, proxy_id: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE torrents SET proxy_id = ?2 WHERE id = ?1")
        .bind(id)
        .bind(proxy_id)
        .execute(pool)
        .await?;
    Ok(())
}

async fn list_files(pool: &SqlitePool, id: &str) -> Result<Vec<TorrentFileRecord>> {
    let rows = sqlx::query("SELECT * FROM torrent_files WHERE torrent_id = ?1 ORDER BY path")
        .bind(id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(file_from_row).collect()
}

async fn list_trackers(pool: &SqlitePool, id: &str) -> Result<Vec<TorrentTrackerRecord>> {
    let rows = sqlx::query("SELECT * FROM torrent_trackers WHERE torrent_id = ?1 ORDER BY url")
        .bind(id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(tracker_from_row).collect()
}

async fn list_peers(pool: &SqlitePool, id: &str) -> Result<Vec<TorrentPeerRecord>> {
    let rows = sqlx::query("SELECT * FROM torrent_peers WHERE torrent_id = ?1 ORDER BY state, id")
        .bind(id)
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(peer_from_row).collect()
}

async fn list_pieces(pool: &SqlitePool, id: &str) -> Result<Vec<TorrentPieceRecord>> {
    let rows =
        sqlx::query("SELECT * FROM torrent_pieces WHERE torrent_id = ?1 ORDER BY piece_index")
            .bind(id)
            .fetch_all(pool)
            .await?;
    rows.into_iter().map(piece_from_row).collect()
}

fn record_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TorrentRecord> {
    let source_kind: String = row.try_get("source_kind")?;
    let status: String = row.try_get("status")?;
    let locked_json: String = row.try_get("policy_locked_fields").unwrap_or_default();
    let policy_locked_fields: Vec<String> = serde_json::from_str(&locked_json).unwrap_or_default();
    Ok(TorrentRecord {
        id: row.try_get("id")?,
        info_hash: row.try_get("info_hash")?,
        name: row.try_get("name")?,
        source: row.try_get("source")?,
        source_kind: kind_from_str(&source_kind)?,
        save_root: row.try_get("save_root")?,
        status: status_from_str(&status)?,
        total_size: i64_to_u64(row.try_get("total_size")?)?,
        downloaded: i64_to_u64(row.try_get("downloaded")?)?,
        uploaded: i64_to_u64(row.try_get("uploaded")?)?,
        ratio: row.try_get("ratio")?,
        download_speed: 0,
        upload_speed: 0,
        peers: 0,
        proxy_id: row.try_get::<Option<String>, _>("proxy_id").unwrap_or(None),
        policy: TorrentPolicy {
            dht_enabled: i64_to_bool(row.try_get("dht_enabled")?),
            pex_enabled: i64_to_bool(row.try_get("pex_enabled")?),
            lsd_enabled: i64_to_bool(row.try_get("lsd_enabled")?),
            seed_enabled: i64_to_bool(row.try_get("seed_enabled")?),
            seed_ratio_limit: row.try_get("seed_ratio_limit")?,
            seed_time_limit_min: row
                .try_get::<Option<i64>, _>("seed_time_limit_min")?
                .map(i64_to_u64)
                .transpose()?,
            upload_limit_bps: row
                .try_get::<Option<i64>, _>("upload_limit_bps")?
                .map(i64_to_u64)
                .transpose()?,
            proxy_required: i64_to_bool(row.try_get("proxy_required")?),
            blocklist_enabled: i64_to_bool(row.try_get("blocklist_enabled")?),
            mse_required: i64_to_bool(row.try_get("mse_required").unwrap_or(0)),
        },
        policy_locked_fields,
        is_private: i64_to_bool(row.try_get("is_private").unwrap_or(0)),
        peer_id_seed: row.try_get("peer_id_seed").unwrap_or_default(),
        engine: row.try_get("engine")?,
        created_at: i64_to_u64(row.try_get("created_at")?)?,
        completed_at: row
            .try_get::<Option<i64>, _>("completed_at")?
            .map(i64_to_u64)
            .transpose()?,
        av_status: row
            .try_get::<Option<String>, _>("av_status")
            .unwrap_or(None),
        av_result: row
            .try_get::<Option<String>, _>("av_result")
            .unwrap_or(None),
        tags: {
            let tags_json: String = row.try_get("tags").unwrap_or_default();
            serde_json::from_str(&tags_json).unwrap_or_default()
        },
        // Missing column on pre-027 DBs → 0; set_torrent_queue_position owns writes.
        queue_position: row.try_get::<i32, _>("queue_position").unwrap_or(0),
        category: row.try_get("category").ok(),
    })
}

fn file_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TorrentFileRecord> {
    Ok(TorrentFileRecord {
        id: row.try_get("id")?,
        torrent_id: row.try_get("torrent_id")?,
        path: row.try_get("path")?,
        size: i64_to_u64(row.try_get("size")?)?,
        selected: i64_to_bool(row.try_get("selected")?),
        priority: row.try_get("priority")?,
        downloaded: i64_to_u64(row.try_get("downloaded")?)?,
        engine_index: row
            .try_get::<Option<i64>, _>("engine_index")
            .unwrap_or(None)
            .map(i64_to_u64)
            .transpose()?,
    })
}

fn tracker_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TorrentTrackerRecord> {
    Ok(TorrentTrackerRecord {
        id: row.try_get("id")?,
        torrent_id: row.try_get("torrent_id")?,
        url: row.try_get("url")?,
        status: row.try_get("status")?,
        last_error: row.try_get("last_error")?,
    })
}

fn peer_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TorrentPeerRecord> {
    Ok(TorrentPeerRecord {
        id: row.try_get("id")?,
        torrent_id: row.try_get("torrent_id")?,
        addr_hash: row.try_get("addr_hash")?,
        client: row.try_get("client")?,
        downloaded: i64_to_u64(row.try_get("downloaded")?)?,
        uploaded: i64_to_u64(row.try_get("uploaded")?)?,
        state: row.try_get("state")?,
        country_iso: None,
        country_name: None,
        flag_emoji: None,
        eyes_tier: None,
    })
}

fn piece_from_row(row: sqlx::sqlite::SqliteRow) -> Result<TorrentPieceRecord> {
    Ok(TorrentPieceRecord {
        id: row.try_get("id")?,
        torrent_id: row.try_get("torrent_id")?,
        piece_index: i64_to_u64(row.try_get("piece_index")?)?,
        size: i64_to_u64(row.try_get("size")?)?,
        status: row.try_get("status")?,
        availability: i64_to_u64(row.try_get("availability")?)?,
    })
}

pub fn status_to_str(status: &TorrentStatus) -> &'static str {
    match status {
        TorrentStatus::Staged => "staged",
        TorrentStatus::MetadataReady => "metadata_ready",
        TorrentStatus::EngineGated => "engine_gated",
        TorrentStatus::Checking => "checking",
        TorrentStatus::Downloading => "downloading",
        TorrentStatus::Paused => "paused",
        TorrentStatus::Completed => "completed",
        TorrentStatus::Error => "error",
    }
}

fn status_from_str(value: &str) -> Result<TorrentStatus> {
    match value {
        "staged" => Ok(TorrentStatus::Staged),
        "metadata_ready" => Ok(TorrentStatus::MetadataReady),
        "engine_gated" => Ok(TorrentStatus::EngineGated),
        "checking" => Ok(TorrentStatus::Checking),
        "downloading" => Ok(TorrentStatus::Downloading),
        "paused" => Ok(TorrentStatus::Paused),
        "completed" => Ok(TorrentStatus::Completed),
        "error" => Ok(TorrentStatus::Error),
        other => Err(anyhow::anyhow!("invalid torrent status: {other}")),
    }
}

fn kind_to_str(kind: &TorrentInputKind) -> &'static str {
    match kind {
        TorrentInputKind::Magnet => "magnet",
        TorrentInputKind::TorrentFile => "torrent_file",
        TorrentInputKind::Unsupported => "unsupported",
    }
}

fn kind_from_str(value: &str) -> Result<TorrentInputKind> {
    match value {
        "magnet" => Ok(TorrentInputKind::Magnet),
        "torrent_file" => Ok(TorrentInputKind::TorrentFile),
        "unsupported" => Ok(TorrentInputKind::Unsupported),
        other => Err(anyhow::anyhow!("invalid torrent source kind: {other}")),
    }
}

fn bool_to_i64(value: bool) -> i64 {
    i64::from(value)
}

fn i64_to_bool(value: i64) -> bool {
    value != 0
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow::anyhow!("value too large for sqlite integer: {value}"))
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("negative sqlite integer: {value}"))
}

pub async fn update_av_status(
    pool: &SqlitePool,
    id: &str,
    av_status: &str,
    av_result: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r"
        UPDATE torrents
        SET av_status = ?2, av_result = ?3
        WHERE id = ?1
        ",
    )
    .bind(id)
    .bind(av_status)
    .bind(av_result)
    .execute(pool)
    .await?;
    Ok(())
}
