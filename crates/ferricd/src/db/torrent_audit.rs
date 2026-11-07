//! Per-torrent audit log (`torrent_audit` table from migration 022).

use anyhow::{Context, Result};
use serde_json::Value;
use sqlx::SqlitePool;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

pub async fn append(
    pool: &SqlitePool,
    torrent_id: &str,
    event_type: &str,
    data: &Value,
) -> Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64);
    let json = serde_json::to_string(data).context("serialize torrent audit data")?;
    sqlx::query(
        r"
        INSERT INTO torrent_audit (id, torrent_id, event_type, data_json, created_at)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(torrent_id)
    .bind(event_type)
    .bind(json)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct TorrentAuditRow {
    pub id: String,
    pub torrent_id: String,
    pub event_type: String,
    pub data_json: String,
    pub created_at: i64,
}

pub async fn list_for_torrent(pool: &SqlitePool, torrent_id: &str) -> Result<Vec<TorrentAuditRow>> {
    use sqlx::Row;
    let rows = sqlx::query(
        r"
        SELECT id, torrent_id, event_type, data_json, created_at
        FROM torrent_audit
        WHERE torrent_id = ?1
        ORDER BY created_at ASC
        ",
    )
    .bind(torrent_id)
    .fetch_all(pool)
    .await?;

    rows.into_iter()
        .map(|row| {
            Ok(TorrentAuditRow {
                id: row.try_get("id")?,
                torrent_id: row.try_get("torrent_id")?,
                event_type: row.try_get("event_type")?,
                data_json: row.try_get("data_json")?,
                created_at: row.try_get("created_at")?,
            })
        })
        .collect()
}
