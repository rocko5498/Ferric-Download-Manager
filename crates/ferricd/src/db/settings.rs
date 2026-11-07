use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

// Defaults are seeded by the SQL migrations (001_initial.sql / 002_queues.sql)
// via `INSERT OR IGNORE`, which is the single source of truth.

pub async fn get(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    let value = sqlx::query_scalar("SELECT value FROM settings WHERE key = ?1")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(value)
}

/// A 0/1 style setting; anything unset or unrecognised falls back to `default`.
pub async fn get_flag(pool: &SqlitePool, key: &str, default: bool) -> bool {
    match get(pool, key).await {
        Ok(Some(v)) => match v.trim() {
            "0" | "false" | "off" => false,
            "1" | "true" | "on" => true,
            _ => default,
        },
        _ => default,
    }
}

pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        r"
        INSERT INTO settings (key, value)
        VALUES (?1, ?2)
        ON CONFLICT(key) DO UPDATE SET value = excluded.value
        ",
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_all(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let rows = sqlx::query("SELECT key, value FROM settings ORDER BY key ASC")
        .fetch_all(pool)
        .await?;

    rows.into_iter()
        .map(|row| {
            let key: String = row.try_get("key")?;
            let value: String = row.try_get("value")?;
            Ok((key, value))
        })
        .collect()
}
