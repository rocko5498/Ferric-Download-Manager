//! Task 123 — per-download network audit log (no cookies).

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub async fn append(
    pool: &SqlitePool,
    download_id: Uuid,
    event_type: &str,
    detail: &Value,
) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let json = serde_json::to_string(detail).context("serialize audit detail")?;
    sqlx::query(
        r"
        INSERT INTO download_audit (id, download_id, event_type, detail_json, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(download_id.to_string())
    .bind(event_type)
    .bind(json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AuditRow {
    pub id: String,
    pub download_id: String,
    pub event_type: String,
    pub detail_json: String,
    pub created_at: i64,
}

pub async fn list_for_download(pool: &SqlitePool, download_id: Uuid) -> Result<Vec<AuditRow>> {
    use sqlx::Row;
    let rows = sqlx::query(
        r"
        SELECT id, download_id, event_type, detail_json, created_at
        FROM download_audit
        WHERE download_id = ?1
        ORDER BY created_at ASC
        ",
    )
    .bind(download_id.to_string())
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(AuditRow {
                id: row.try_get("id")?,
                download_id: row.try_get("download_id")?,
                event_type: row.try_get("event_type")?,
                detail_json: row.try_get("detail_json")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect()
}
