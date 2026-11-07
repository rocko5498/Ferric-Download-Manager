use anyhow::{Context, Result};
use ferric_common::messages::DownloadRequest;
use ferric_common::{DownloadRecord, DownloadStatus};
use sqlx::{Row, SqlitePool};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub async fn insert(
    pool: &SqlitePool,
    req: &DownloadRequest,
    save_path: &str,
    category: &str,
) -> Result<DownloadRecord> {
    let filename = req
        .filename
        .clone()
        .or_else(|| filename_from_url(&req.url))
        .unwrap_or_else(|| req.id.to_string());
    let created_at = unix_millis()?;
    let total_size = req.size_hint.map(u64_to_i64).transpose()?;
    let id = req.id.to_string();
    let status = DownloadStatus::Queued.to_string();

    sqlx::query(
        r"
        INSERT INTO downloads (
            id, url, final_url, filename, save_path, status, total_size, downloaded,
            created_at, referrer, mime_type, category, queue_id, checksum, mirrors
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0, ?8, ?9, ?10, ?11, ?12, ?13, ?14)
        ",
    )
    .bind(&id)
    .bind(&req.url)
    .bind(&req.final_url)
    .bind(&filename)
    .bind(save_path)
    .bind(&status)
    .bind(total_size)
    .bind(created_at)
    .bind(&req.referrer)
    .bind(&req.mime_type)
    .bind(category)
    .bind(req.queue_id.map(|id| id.to_string()))
    .bind(&req.checksum)
    .bind(mirrors_to_db(&req.mirrors)?)
    .execute(pool)
    .await?;

    get(pool, req.id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("inserted download was not found: {}", req.id))
}

pub async fn get(pool: &SqlitePool, id: Uuid) -> Result<Option<DownloadRecord>> {
    let row = sqlx::query("SELECT * FROM downloads WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;

    row.map(download_from_row).transpose()
}

pub async fn list(
    pool: &SqlitePool,
    status_filter: Option<DownloadStatus>,
) -> Result<Vec<DownloadRecord>> {
    let rows = if let Some(status) = status_filter {
        sqlx::query("SELECT * FROM downloads WHERE status = ?1 ORDER BY created_at DESC")
            .bind(status.to_string())
            .fetch_all(pool)
            .await?
    } else {
        sqlx::query("SELECT * FROM downloads ORDER BY created_at DESC")
            .fetch_all(pool)
            .await?
    };

    rows.into_iter().map(download_from_row).collect()
}

/// All `queued` downloads in admission order: highest priority first, then oldest.
pub async fn list_queued_prioritized(pool: &SqlitePool) -> Result<Vec<DownloadRecord>> {
    let rows = sqlx::query(
        "SELECT * FROM downloads WHERE status = 'queued' ORDER BY priority DESC, created_at ASC",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(download_from_row).collect()
}

/// Put a failed download back in the queue *behind* everything already waiting
/// (queue order is `created_at`), like IDM moving failed files to the end.
pub async fn requeue_at_end(pool: &SqlitePool, id: Uuid) -> Result<bool> {
    let now = unix_millis()?;
    let done = sqlx::query(
        "UPDATE downloads SET status = 'queued', error_msg = NULL, created_at = ?1 \
         WHERE id = ?2 AND status = 'error'",
    )
    .bind(now)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn update_status(
    pool: &SqlitePool,
    id: Uuid,
    status: DownloadStatus,
    error_msg: Option<&str>,
) -> Result<()> {
    let now = unix_millis()?;
    sqlx::query(
        r"
        UPDATE downloads
        SET status = ?1,
            error_msg = ?2,
            error_count = CASE WHEN ?1 = 'error' THEN error_count + 1 ELSE error_count END,
            started_at = CASE
                WHEN ?1 IN ('connecting', 'downloading') AND started_at IS NULL THEN ?3
                ELSE started_at
            END,
            completed_at = CASE WHEN ?1 = 'completed' THEN ?3 ELSE completed_at END
        WHERE id = ?4
        ",
    )
    .bind(status.to_string())
    .bind(error_msg)
    .bind(now)
    .bind(id.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

/// Update the user-chosen filename and target path (used when a held capture
/// is confirmed with edited filename/folder).
pub async fn update_url(pool: &SqlitePool, id: Uuid, new_url: &str) -> Result<()> {
    sqlx::query(
        "UPDATE downloads SET url = ?1, final_url = NULL, redirect_chain = NULL WHERE id = ?2",
    )
    .bind(new_url)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_target(
    pool: &SqlitePool,
    id: Uuid,
    filename: &str,
    save_path: &str,
    audio_url: Option<&str>,
    subtitle_url: Option<&str>,
    subtitle_lang: Option<&str>,
    proxy_url: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE downloads SET filename = ?1, save_path = ?2, audio_url = ?3, subtitle_url = ?4, subtitle_lang = ?5, proxy_url = ?6 WHERE id = ?7")
        .bind(filename)
        .bind(save_path)
        .bind(audio_url)
        .bind(subtitle_url)
        .bind(subtitle_lang)
        .bind(proxy_url)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_response_metadata(
    pool: &SqlitePool,
    id: Uuid,
    final_url: Option<&str>,
    redirect_chain: Option<&str>,
    mime_type: Option<&str>,
    filename: Option<&str>,
    save_path: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r"
        UPDATE downloads
        SET final_url = COALESCE(?1, final_url),
            redirect_chain = COALESCE(?2, redirect_chain),
            mime_type = COALESCE(?3, mime_type),
            filename = COALESCE(?4, filename),
            save_path = COALESCE(?5, save_path)
        WHERE id = ?6
        ",
    )
    .bind(final_url)
    .bind(redirect_chain)
    .bind(mime_type)
    .bind(filename)
    .bind(save_path)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_priority(pool: &SqlitePool, id: Uuid, priority: i32) -> Result<bool> {
    let result = sqlx::query("UPDATE downloads SET priority = ?1 WHERE id = ?2")
        .bind(priority)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Per-download proxy (used when the proxy asked for a login and the user typed one).
pub async fn set_proxy(pool: &SqlitePool, id: Uuid, proxy_url: Option<&str>) -> Result<bool> {
    let result = sqlx::query("UPDATE downloads SET proxy_url = ?1 WHERE id = ?2")
        .bind(proxy_url)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// IDM's "Move to queue": the download now belongs to `queue_id` (None = the default queue).
pub async fn set_queue(pool: &SqlitePool, id: Uuid, queue_id: Option<Uuid>) -> Result<bool> {
    let result = sqlx::query("UPDATE downloads SET queue_id = ?1 WHERE id = ?2")
        .bind(queue_id.map(|q| q.to_string()))
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_speed_limit(pool: &SqlitePool, id: Uuid, bps: u64) -> Result<bool> {
    let bps = u64_to_i64(bps)?;
    let result = sqlx::query("UPDATE downloads SET speed_limit_bps = ?1 WHERE id = ?2")
        .bind(bps)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn update_progress(pool: &SqlitePool, id: Uuid, downloaded: u64) -> Result<()> {
    sqlx::query("UPDATE downloads SET downloaded = ?1 WHERE id = ?2")
        .bind(u64_to_i64(downloaded)?)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_checksum_ok(pool: &SqlitePool, id: Uuid, ok: bool) -> Result<()> {
    sqlx::query("UPDATE downloads SET checksum_ok = ?1 WHERE id = ?2")
        .bind(i64::from(ok))
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_description(pool: &SqlitePool, id: Uuid, description: Option<&str>) -> Result<()> {
    sqlx::query("UPDATE downloads SET description = ?1 WHERE id = ?2")
        .bind(description)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// A finished download whose size the server never announced: the bytes received are its size.
pub async fn set_total_if_missing(pool: &SqlitePool, id: Uuid) -> Result<()> {
    sqlx::query("UPDATE downloads SET total_size = downloaded WHERE id = ?1 AND total_size IS NULL AND downloaded > 0")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Remember the server's ETag / Last-Modified for a download that has no known size (single stream).
pub async fn set_validators(
    pool: &SqlitePool,
    id: Uuid,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    sqlx::query("UPDATE downloads SET etag = ?1, last_modified = ?2 WHERE id = ?3")
        .bind(etag)
        .bind(last_modified)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_file_info(
    pool: &SqlitePool,
    id: Uuid,
    total: u64,
    etag: Option<&str>,
    last_modified: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r"
        UPDATE downloads
        SET total_size = ?1, etag = ?2, last_modified = ?3
        WHERE id = ?4
        ",
    )
    .bind(u64_to_i64(total)?)
    .bind(etag)
    .bind(last_modified)
    .bind(id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM downloads WHERE id = ?1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn set_category(pool: &SqlitePool, id: Uuid, category: &str) -> Result<()> {
    sqlx::query("UPDATE downloads SET category = ?1 WHERE id = ?2")
        .bind(category)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// A capture is waiting for the Download File Info dialog.
pub async fn held_add(pool: &SqlitePool, id: Uuid) -> Result<()> {
    sqlx::query("INSERT OR IGNORE INTO held_captures (id) VALUES (?1)")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn held_remove(pool: &SqlitePool, id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM held_captures WHERE id = ?1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

/// Drop captures left over from a previous run (their credentials are gone, so they can never start).
pub async fn purge_held(pool: &SqlitePool) -> Result<u64> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM held_captures")
        .fetch_all(pool)
        .await?;
    for id in &ids {
        // A capture that was started while its dialog was open leaves a partial file behind: remove it
        // (a finished file stays, the user may want it).
        let row: Option<(String, String)> =
            sqlx::query_as("SELECT save_path, status FROM downloads WHERE id = ?1")
                .bind(id)
                .fetch_optional(pool)
                .await?;
        if let Some((path, status)) = row {
            if status != "completed" {
                let _ = tokio::fs::remove_file(&path).await;
            }
        }
        sqlx::query("DELETE FROM downloads WHERE id = ?1")
            .bind(id)
            .execute(pool)
            .await?;
    }
    sqlx::query("DELETE FROM held_captures")
        .execute(pool)
        .await?;
    Ok(ids.len() as u64)
}

/// Remember where a still-downloading file must go when the transfer ends (see migration 042).
pub async fn pending_move_set(pool: &SqlitePool, id: Uuid, target: &str, name: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO pending_moves (id, target, name) VALUES (?1, ?2, ?3)
         ON CONFLICT(id) DO UPDATE SET target = excluded.target, name = excluded.name",
    )
    .bind(id.to_string())
    .bind(target)
    .bind(name)
    .execute(pool)
    .await?;
    Ok(())
}

/// The pending move of `id`, removed from the table (None when there is none).
pub async fn pending_move_take(pool: &SqlitePool, id: Uuid) -> Result<Option<(String, String)>> {
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT target, name FROM pending_moves WHERE id = ?1")
            .bind(id.to_string())
            .fetch_optional(pool)
            .await?;
    sqlx::query("DELETE FROM pending_moves WHERE id = ?1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(row)
}

/// Ids that still have a pending move.
pub async fn pending_move_ids(pool: &SqlitePool) -> Result<Vec<Uuid>> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM pending_moves")
        .fetch_all(pool)
        .await?;
    Ok(ids.iter().filter_map(|s| Uuid::parse_str(s).ok()).collect())
}

pub async fn find_incomplete(pool: &SqlitePool) -> Result<Vec<DownloadRecord>> {
    let rows = sqlx::query(
        r"
        SELECT * FROM downloads
        WHERE status IN ('downloading', 'connecting')
        ORDER BY created_at ASC
        ",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(download_from_row).collect()
}

fn download_from_row(row: sqlx::sqlite::SqliteRow) -> Result<DownloadRecord> {
    let id: String = row.try_get("id")?;
    let status: String = row.try_get("status")?;
    let total_size: Option<i64> = row.try_get("total_size")?;
    let downloaded: i64 = row.try_get("downloaded")?;
    let error_count: i64 = row.try_get("error_count")?;
    let queue_id: Option<String> = row.try_get("queue_id")?;
    let checksum_ok: Option<i64> = row.try_get("checksum_ok")?;

    Ok(DownloadRecord {
        id: Uuid::parse_str(&id).with_context(|| format!("invalid download id: {id}"))?,
        url: row.try_get("url")?,
        final_url: row.try_get("final_url")?,
        redirect_chain: row.try_get("redirect_chain")?,
        filename: row.try_get("filename")?,
        save_path: row.try_get("save_path")?,
        status: DownloadStatus::from_str(&status)
            .map_err(|e| anyhow::anyhow!("invalid download status {status}: {e}"))?,
        total_size: total_size.map(i64_to_u64).transpose()?,
        downloaded: i64_to_u64(downloaded)?,
        created_at: row.try_get("created_at")?,
        started_at: row.try_get("started_at")?,
        completed_at: row.try_get("completed_at")?,
        error_msg: row.try_get("error_msg")?,
        error_count: i64_to_u32(error_count)?,
        referrer: row.try_get("referrer")?,
        mime_type: row.try_get("mime_type")?,
        etag: row.try_get("etag")?,
        last_modified: row.try_get("last_modified")?,
        category: row.try_get("category")?,
        queue_id: queue_id
            .map(|value| {
                Uuid::parse_str(&value).with_context(|| format!("invalid queue id: {value}"))
            })
            .transpose()?,
        priority: row.try_get("priority")?,
        checksum: row.try_get("checksum")?,
        checksum_ok: checksum_ok.map(|value| value != 0),
        speed_limit_bps: {
            let v: i64 = row.try_get("speed_limit_bps").unwrap_or(0);
            i64_to_u64(v)?
        },
        audio_url: row.try_get("audio_url")?,
        subtitle_url: row.try_get("subtitle_url")?,
        subtitle_lang: row.try_get("subtitle_lang")?,
        proxy_url: row.try_get("proxy_url").unwrap_or(None),
        mirrors: mirrors_from_db(row.try_get("mirrors").unwrap_or(None))?,
        description: row.try_get("description")?,
    })
}

fn mirrors_to_db(mirrors: &[String]) -> Result<Option<String>> {
    if mirrors.is_empty() {
        return Ok(None);
    }
    Ok(Some(serde_json::to_string(mirrors)?))
}

fn mirrors_from_db(raw: Option<String>) -> Result<Vec<String>> {
    let Some(raw) = raw.filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };
    let parsed: Vec<String> =
        serde_json::from_str(&raw).with_context(|| format!("invalid mirrors JSON: {raw}"))?;
    Ok(parsed)
}

fn filename_from_url(raw: &str) -> Option<String> {
    url::Url::parse(raw)
        .ok()
        .and_then(|url| url.path_segments()?.next_back().map(str::to_owned))
        .filter(|value| !value.is_empty())
}

fn unix_millis() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?;
    i64::try_from(elapsed.as_millis()).context("current timestamp exceeds i64 range")
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("value exceeds SQLite INTEGER range")
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).context("negative integer found in unsigned DB column")
}

fn i64_to_u32(value: i64) -> Result<u32> {
    u32::try_from(value).context("integer exceeds u32 range")
}
