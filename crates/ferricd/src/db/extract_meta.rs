//! `ExtractResult` JSON stored per download for pure-Rust post-download sidecars.

use anyhow::{Context, Result};
use ferric_common::ExtractResult;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub async fn put(pool: &SqlitePool, download_id: Uuid, result: &ExtractResult) -> Result<()> {
    let json = serde_json::to_string(result).context("serialize ExtractResult")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    sqlx::query(
        r"
        INSERT INTO download_extract_meta (download_id, meta_json, created_at)
        VALUES (?1, ?2, ?3)
        ON CONFLICT(download_id) DO UPDATE SET meta_json = excluded.meta_json, created_at = excluded.created_at
        ",
    )
    .bind(download_id.to_string())
    .bind(json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &SqlitePool, download_id: Uuid) -> Result<Option<ExtractResult>> {
    let row: Option<String> =
        sqlx::query_scalar("SELECT meta_json FROM download_extract_meta WHERE download_id = ?1")
            .bind(download_id.to_string())
            .fetch_optional(pool)
            .await?;
    match row {
        Some(json) => {
            let r: ExtractResult =
                serde_json::from_str(&json).context("parse stored ExtractResult")?;
            Ok(Some(r))
        }
        None => Ok(None),
    }
}

pub async fn delete(pool: &SqlitePool, download_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM download_extract_meta WHERE download_id = ?1")
        .bind(download_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}
