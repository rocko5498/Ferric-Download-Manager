use anyhow::Result;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// Upsert a peer using `addr_hash` as the conflict key (not the generated UUID).
pub async fn upsert_peer_by_addr(
    pool: &SqlitePool,
    torrent_id: &str,
    addr_hash: &str,
    client: Option<&str>,
    downloaded: u64,
    uploaded: u64,
    state: &str,
) -> Result<()> {
    // Check if peer already exists for this torrent
    let existing: Option<SqliteRow> = sqlx::query(
        "SELECT id FROM torrent_peers WHERE torrent_id = ?1 AND addr_hash = ?2 LIMIT 1",
    )
    .bind(torrent_id)
    .bind(addr_hash)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        let id: String = row.try_get("id")?;
        sqlx::query(
            "UPDATE torrent_peers SET client = ?1, downloaded = ?2, uploaded = ?3, state = ?4
             WHERE id = ?5",
        )
        .bind(client)
        .bind(downloaded as i64)
        .bind(uploaded as i64)
        .bind(state)
        .bind(id)
        .execute(pool)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO torrent_peers (id, torrent_id, addr_hash, client, downloaded, uploaded, state)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(torrent_id)
        .bind(addr_hash)
        .bind(client)
        .bind(downloaded as i64)
        .bind(uploaded as i64)
        .bind(state)
        .execute(pool)
        .await?;
    }
    Ok(())
}

/// Check if an `addr_hash` is banned for a given torrent.
pub async fn is_banned(pool: &SqlitePool, torrent_id: &str, addr_hash: &str) -> Result<bool> {
    let now = now_ms();
    let row: Option<SqliteRow> = sqlx::query(
        "SELECT id FROM banned_peers
         WHERE torrent_id = ?1 AND addr_hash = ?2
           AND (expires_at IS NULL OR expires_at > ?3)
         LIMIT 1",
    )
    .bind(torrent_id)
    .bind(addr_hash)
    .bind(now as i64)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Ban a peer. `expires_at` = None means session-permanent ban.
pub async fn ban_peer(
    pool: &SqlitePool,
    torrent_id: &str,
    addr_hash: &str,
    reason: &str,
    expires_at: Option<u64>,
) -> Result<()> {
    let now = now_ms();
    sqlx::query(
        "INSERT INTO banned_peers (id, torrent_id, addr_hash, reason, banned_at, expires_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT DO NOTHING",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(torrent_id)
    .bind(addr_hash)
    .bind(reason)
    .bind(now as i64)
    .bind(expires_at.map(|v| v as i64))
    .execute(pool)
    .await?;
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn ban_and_check_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;

        // Stage a torrent so FK constraint is satisfied
        sqlx::query(
            "INSERT INTO torrents (id, info_hash, name, source, source_kind, status, is_private, total_size, downloaded, uploaded, ratio, peer_id_seed, created_at, dht_enabled, pex_enabled, lsd_enabled, seed_enabled, mse_required, proxy_required, blocklist_enabled, policy_locked_fields, engine)
             VALUES ('t1', 'hash1', 'test', 'magnet:test', 'magnet', 'staged', 0, 1024, 0, 0, 0.0, 'seed1', 0, 0, 0, 0, 1, 0, 0, 0, '[]', '')"
        )
        .execute(&pool)
        .await?;

        assert!(!is_banned(&pool, "t1", "addr_abc").await?);

        ban_peer(&pool, "t1", "addr_abc", "metadata honeypot", None).await?;
        assert!(is_banned(&pool, "t1", "addr_abc").await?);

        Ok(())
    }

    #[tokio::test]
    async fn expired_bans_are_not_active() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;

        sqlx::query(
            "INSERT INTO torrents (id, info_hash, name, source, source_kind, status, is_private, total_size, downloaded, uploaded, ratio, peer_id_seed, created_at, dht_enabled, pex_enabled, lsd_enabled, seed_enabled, mse_required, proxy_required, blocklist_enabled, policy_locked_fields, engine)
             VALUES ('t2', 'hash2', 'test2', 'magnet:test2', 'magnet', 'staged', 0, 1024, 0, 0, 0.0, 'seed2', 0, 0, 0, 0, 1, 0, 0, 0, '[]', '')"
        )
        .execute(&pool)
        .await?;

        // Ban with expiry in the past
        ban_peer(&pool, "t2", "addr_xyz", "test ban", Some(1)).await?;
        assert!(!is_banned(&pool, "t2", "addr_xyz").await?);
        Ok(())
    }

    #[tokio::test]
    async fn upsert_peer_by_addr_updates_existing() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;

        sqlx::query(
            "INSERT INTO torrents (id, info_hash, name, source, source_kind, status, is_private, total_size, downloaded, uploaded, ratio, peer_id_seed, created_at, dht_enabled, pex_enabled, lsd_enabled, seed_enabled, mse_required, proxy_required, blocklist_enabled, policy_locked_fields, engine)
             VALUES ('t3', 'hash3', 'test3', 'magnet:test3', 'magnet', 'staged', 0, 1024, 0, 0, 0.0, 'seed3', 0, 0, 0, 0, 1, 0, 0, 0, '[]', '')"
        )
        .execute(&pool)
        .await?;

        upsert_peer_by_addr(
            &pool,
            "t3",
            "addr_1",
            Some("qBittorrent"),
            100,
            50,
            "connected",
        )
        .await?;
        upsert_peer_by_addr(
            &pool,
            "t3",
            "addr_1",
            Some("qBittorrent"),
            200,
            100,
            "connected",
        )
        .await?;

        let rows = sqlx::query("SELECT COUNT(*) as count FROM torrent_peers WHERE torrent_id = 't3' AND addr_hash = 'addr_1'")
            .fetch_one(&pool)
            .await?;
        let count: i64 = rows.try_get("count")?;
        assert_eq!(count, 1, "should upsert, not duplicate");
        Ok(())
    }
}
