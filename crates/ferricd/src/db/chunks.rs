use anyhow::{Context, Result};
use ferric_common::{ChunkRecord, ChunkStatus};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::chunk::ChunkRange;

pub async fn insert_chunks(
    pool: &SqlitePool,
    download_id: Uuid,
    chunks: &[ChunkRange],
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let download_id = download_id.to_string();

    for chunk in chunks {
        sqlx::query(
            r"
            INSERT INTO chunks (
                id, download_id, chunk_index, byte_start, byte_end, downloaded, status
            )
            VALUES (?1, ?2, ?3, ?4, ?5, 0, 'pending')
            ON CONFLICT(download_id, chunk_index) DO UPDATE SET
                byte_start = excluded.byte_start,
                byte_end = excluded.byte_end,
                downloaded = 0,
                status = 'pending',
                error_count = 0
            ",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&download_id)
        .bind(u32_to_i64(chunk.chunk_index))
        .bind(u64_to_i64(chunk.byte_start)?)
        .bind(u64_to_i64(chunk.byte_end)?)
        .execute(tx.as_mut())
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_chunks(pool: &SqlitePool, download_id: Uuid) -> Result<Vec<ChunkRecord>> {
    let rows = sqlx::query(
        r"
        SELECT * FROM chunks
        WHERE download_id = ?1
        ORDER BY chunk_index ASC
        ",
    )
    .bind(download_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(chunk_from_row).collect()
}

pub async fn get_incomplete_chunks(
    pool: &SqlitePool,
    download_id: Uuid,
) -> Result<Vec<ChunkRecord>> {
    let rows = sqlx::query(
        r"
        SELECT * FROM chunks
        WHERE download_id = ?1 AND status != 'completed'
        ORDER BY chunk_index ASC
        ",
    )
    .bind(download_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(chunk_from_row).collect()
}

pub async fn update_chunk_progress(
    pool: &SqlitePool,
    chunk_id: Uuid,
    downloaded: u64,
    status: ChunkStatus,
) -> Result<()> {
    sqlx::query(
        r"
        UPDATE chunks
        SET downloaded = ?1,
            status = ?2,
            error_count = CASE WHEN ?2 = 'error' THEN error_count + 1 ELSE error_count END
        WHERE id = ?3
          -- a late still-downloading write (an aborted worker, a slow flush) must never undo a finished piece
          AND NOT (?2 = 'downloading' AND status = 'completed')
        ",
    )
    .bind(u64_to_i64(downloaded)?)
    .bind(chunk_status_to_str(status))
    .bind(chunk_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Truncate `chunk.byte_end` to `midpoint - 1` and insert a new pending chunk
/// covering `[midpoint, chunk.byte_end]`. Returns the new chunk record.
///
/// Used by tail-splitting: when a worker pops the last queue item, it splits
/// the piece so that an idle worker can download the second half in parallel.
pub async fn split_chunk(
    pool: &SqlitePool,
    chunk: &ChunkRecord,
    midpoint: u64,
) -> Result<ChunkRecord> {
    let mut tx = pool.begin().await?;
    let download_id_str = chunk.download_id.to_string();

    sqlx::query("UPDATE chunks SET byte_end = ?1 WHERE id = ?2")
        .bind(u64_to_i64(midpoint - 1)?)
        .bind(chunk.id.to_string())
        .execute(tx.as_mut())
        .await?;

    let max_index: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(chunk_index), -1) FROM chunks WHERE download_id = ?1",
    )
    .bind(&download_id_str)
    .fetch_one(tx.as_mut())
    .await?;
    let new_index = u32::try_from(max_index + 1).context("chunk_index overflow")?;
    let new_id = Uuid::new_v4();

    sqlx::query(
        "INSERT INTO chunks \
         (id, download_id, chunk_index, byte_start, byte_end, downloaded, status) \
         VALUES (?1, ?2, ?3, ?4, ?5, 0, 'pending')",
    )
    .bind(new_id.to_string())
    .bind(&download_id_str)
    .bind(i64::from(new_index))
    .bind(u64_to_i64(midpoint)?)
    .bind(u64_to_i64(chunk.byte_end)?)
    .execute(tx.as_mut())
    .await?;

    tx.commit().await?;

    Ok(ChunkRecord {
        id: new_id,
        download_id: chunk.download_id,
        chunk_index: new_index,
        byte_start: midpoint,
        byte_end: chunk.byte_end,
        downloaded: 0,
        status: ferric_common::ChunkStatus::Pending,
        error_count: 0,
    })
}

/// Forget every piece of a download (the remote file changed, so old progress is worthless).
pub async fn delete_all(pool: &SqlitePool, download_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM chunks WHERE download_id = ?1")
        .bind(download_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn reset_chunks_for_resume(pool: &SqlitePool, download_id: Uuid) -> Result<()> {
    sqlx::query(
        r"
        UPDATE chunks
        SET status = 'pending'
        WHERE download_id = ?1 AND status = 'downloading'
        ",
    )
    .bind(download_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

/// Reset all non-completed chunks to pending so a URL-refreshed download
/// re-downloads only the pieces that failed or were interrupted.
pub async fn reset_for_url_refresh(pool: &SqlitePool, download_id: Uuid) -> Result<()> {
    sqlx::query(
        r"
        UPDATE chunks
        SET status = 'pending', error_count = 0
        WHERE download_id = ?1 AND status != 'completed'
        ",
    )
    .bind(download_id.to_string())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn all_chunks_complete(pool: &SqlitePool, download_id: Uuid) -> Result<bool> {
    let incomplete: i64 = sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM chunks
        WHERE download_id = ?1 AND status != 'completed'
        ",
    )
    .bind(download_id.to_string())
    .fetch_one(pool)
    .await?;

    Ok(incomplete == 0)
}

fn chunk_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ChunkRecord> {
    let id: String = row.try_get("id")?;
    let download_id: String = row.try_get("download_id")?;
    let status: String = row.try_get("status")?;
    let chunk_index: i64 = row.try_get("chunk_index")?;
    let byte_start: i64 = row.try_get("byte_start")?;
    let byte_end: i64 = row.try_get("byte_end")?;
    let downloaded: i64 = row.try_get("downloaded")?;
    let error_count: i64 = row.try_get("error_count")?;

    Ok(ChunkRecord {
        id: Uuid::parse_str(&id).with_context(|| format!("invalid chunk id: {id}"))?,
        download_id: Uuid::parse_str(&download_id)
            .with_context(|| format!("invalid chunk download id: {download_id}"))?,
        chunk_index: u32::try_from(chunk_index).context("chunk_index exceeds u32 range")?,
        byte_start: i64_to_u64(byte_start)?,
        byte_end: i64_to_u64(byte_end)?,
        downloaded: i64_to_u64(downloaded)?,
        status: chunk_status_from_str(&status)?,
        error_count: u32::try_from(error_count).context("error_count exceeds u32 range")?,
    })
}

fn chunk_status_from_str(value: &str) -> Result<ChunkStatus> {
    match value {
        "pending" => Ok(ChunkStatus::Pending),
        "downloading" => Ok(ChunkStatus::Downloading),
        "completed" => Ok(ChunkStatus::Completed),
        "error" => Ok(ChunkStatus::Error),
        other => Err(anyhow::anyhow!("invalid chunk status: {other}")),
    }
}

fn chunk_status_to_str(status: ChunkStatus) -> &'static str {
    match status {
        ChunkStatus::Pending => "pending",
        ChunkStatus::Downloading => "downloading",
        ChunkStatus::Completed => "completed",
        ChunkStatus::Error => "error",
    }
}

fn u32_to_i64(value: u32) -> i64 {
    i64::from(value)
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("value exceeds SQLite INTEGER range")
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).context("negative integer found in unsigned DB column")
}

/// Save how far a piece has got, but never move it backwards and never touch a finished piece.
/// (The job's checkpointer writes a snapshot taken before a flush; the piece itself may have finished since.)
pub async fn checkpoint_progress(pool: &SqlitePool, chunk_id: Uuid, downloaded: u64) -> Result<()> {
    sqlx::query("UPDATE chunks SET downloaded = ?1 WHERE id = ?2 AND status != 'completed' AND downloaded < ?1")
        .bind(i64::try_from(downloaded).unwrap_or(i64::MAX))
        .bind(chunk_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use ferric_common::messages::DownloadRequest;

    #[tokio::test]
    async fn a_late_downloading_write_does_not_undo_a_finished_chunk() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let id = Uuid::new_v4();
        let req = DownloadRequest {
            id,
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
        db::downloads::insert(&pool, &req, "a.bin", "Other").await?;
        insert_chunks(
            &pool,
            id,
            &[ChunkRange {
                chunk_index: 0,
                byte_start: 0,
                byte_end: 99,
            }],
        )
        .await?;
        let chunk = get_chunks(&pool, id).await?.remove(0);
        update_chunk_progress(&pool, chunk.id, 100, ChunkStatus::Completed).await?;
        update_chunk_progress(&pool, chunk.id, 0, ChunkStatus::Downloading).await?;
        assert!(all_chunks_complete(&pool, id).await?);
        // other transitions still work (a digest mismatch sends a finished chunk back)
        update_chunk_progress(&pool, chunk.id, 0, ChunkStatus::Error).await?;
        assert!(!all_chunks_complete(&pool, id).await?);
        Ok(())
    }
}
