use anyhow::{Context, Result};
use ferric_common::QueueRecord;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

/// The seeded "Default" queue (`migrations/002_queues.sql`):
/// `00000000-0000-0000-0000-000000000001`.
pub const DEFAULT_QUEUE_ID: Uuid = Uuid::from_u128(1);

/// Fallback when a queue row is missing or unreadable.
const DEFAULT_MAX_CONCURRENT: u32 = 2;

/// Maximum simultaneous downloads allowed for a queue.
pub async fn max_concurrent(pool: &SqlitePool, id: Uuid) -> u32 {
    let value: Option<i64> = sqlx::query_scalar("SELECT max_concurrent FROM queues WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

    value
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(DEFAULT_MAX_CONCURRENT)
}

/// Tri-state patch for optional queue fields.
/// `None` = absent (preserve current), `Some(None)` = explicit null (clear),
/// `Some(Some(v))` = set to v.
#[derive(Debug, Default)]
#[allow(clippy::option_option)]
pub struct QueueUpdate {
    pub name: Option<String>,
    pub max_concurrent: Option<u32>,
    pub max_bandwidth: Option<Option<u64>>,
    pub sched_start: Option<Option<u16>>,
    pub sched_end: Option<Option<u16>>,
    pub sched_days: Option<u8>,
    pub start_on_startup: Option<bool>,
    pub sync_interval_min: Option<Option<u32>>,
    pub max_retries: Option<Option<u32>>,
    pub open_file: Option<Option<String>>,
    pub when_done: Option<Option<String>>,
    pub enabled: Option<bool>,
}

pub async fn get(pool: &SqlitePool, id: Uuid) -> Result<Option<QueueRecord>> {
    let row = sqlx::query("SELECT * FROM queues WHERE id = ?1")
        .bind(id.to_string())
        .fetch_optional(pool)
        .await?;
    row.map(queue_from_row).transpose()
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<QueueRecord>> {
    let rows = sqlx::query("SELECT * FROM queues ORDER BY priority DESC, name ASC")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(queue_from_row).collect()
}

/// A new queue (IDM's "New queue"). Names are unique; `Ok(None)` when the name is taken.
pub async fn create(
    pool: &SqlitePool,
    name: &str,
    max_concurrent: u32,
) -> Result<Option<QueueRecord>> {
    let id = Uuid::new_v4();
    let inserted = sqlx::query("INSERT OR IGNORE INTO queues (id, name, max_concurrent, enabled, priority) VALUES (?1, ?2, ?3, 1, 0)")
        .bind(id.to_string())
        .bind(name)
        .bind(i64::from(max_concurrent.max(1)))
        .execute(pool)
        .await?;
    if inserted.rows_affected() == 0 {
        return Ok(None);
    }
    get(pool, id).await
}

/// Delete a queue; its downloads fall back to the default queue. The default queue itself stays.
pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<bool> {
    if id == DEFAULT_QUEUE_ID {
        return Ok(false);
    }
    sqlx::query("UPDATE downloads SET queue_id = NULL WHERE queue_id = ?1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    let gone = sqlx::query("DELETE FROM queues WHERE id = ?1")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(gone.rows_affected() > 0)
}

/// Ids and statuses of the downloads in a queue (the default queue also holds downloads with no queue).
pub async fn download_ids(pool: &SqlitePool, id: Uuid) -> Result<Vec<(Uuid, String)>> {
    let rows = sqlx::query("SELECT id, status FROM downloads WHERE COALESCE(queue_id, ?2) = ?1")
        .bind(id.to_string())
        .bind(DEFAULT_QUEUE_ID.to_string())
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|r| {
            let did: String = r.try_get("id")?;
            Ok((Uuid::parse_str(&did)?, r.try_get("status")?))
        })
        .collect()
}

pub async fn update(pool: &SqlitePool, id: Uuid, update: QueueUpdate) -> Result<bool> {
    let Some(current) = get(pool, id).await? else {
        return Ok(false);
    };

    let name = update.name.unwrap_or(current.name);
    let max_concurrent = update.max_concurrent.unwrap_or(current.max_concurrent);
    // Tri-state: None = preserve, Some(None) = clear, Some(Some(v)) = set.
    let max_bandwidth = match update.max_bandwidth {
        Some(v) => v,
        None => current.max_bandwidth,
    };
    let sched_start = match update.sched_start {
        Some(v) => v,
        None => current.sched_start,
    };
    let sched_end = match update.sched_end {
        Some(v) => v,
        None => current.sched_end,
    };
    let enabled = update.enabled.unwrap_or(current.enabled);
    let sched_days = update.sched_days.unwrap_or(current.sched_days);
    let start_on_startup = update.start_on_startup.unwrap_or(current.start_on_startup);
    let sync_interval_min = match update.sync_interval_min {
        Some(v) => v,
        None => current.sync_interval_min,
    };
    let max_retries = update.max_retries.unwrap_or(current.max_retries);
    let open_file = update.open_file.unwrap_or(current.open_file);
    let when_done = update.when_done.unwrap_or(current.when_done);

    let result = sqlx::query(
        r"
        UPDATE queues
        SET name = ?1,
            max_concurrent = ?2,
            max_bandwidth = ?3,
            enabled = ?4,
            sched_start = ?5,
            sched_end = ?6,
            sched_days = ?8,
            start_on_startup = ?9,
            sync_interval_min = ?10,
            max_retries = ?11,
            open_file = ?12,
            when_done = ?13
        WHERE id = ?7
        ",
    )
    .bind(name)
    .bind(i64::from(max_concurrent))
    .bind(max_bandwidth.map(u64_to_i64).transpose()?)
    .bind(i64::from(enabled))
    .bind(sched_start.map(i64::from))
    .bind(sched_end.map(i64::from))
    .bind(id.to_string())
    .bind(i64::from(sched_days))
    .bind(i64::from(start_on_startup))
    .bind(sync_interval_min.map(i64::from))
    .bind(max_retries.map(i64::from))
    .bind(open_file)
    .bind(when_done)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

fn queue_from_row(row: sqlx::sqlite::SqliteRow) -> Result<QueueRecord> {
    let id: String = row.try_get("id")?;
    let max_concurrent: i64 = row.try_get("max_concurrent")?;
    let max_bandwidth: Option<i64> = row.try_get("max_bandwidth")?;
    let sched_start: Option<i64> = row.try_get("sched_start")?;
    let sched_end: Option<i64> = row.try_get("sched_end")?;
    let enabled: i64 = row.try_get("enabled")?;

    Ok(QueueRecord {
        id: Uuid::parse_str(&id).with_context(|| format!("invalid queue id: {id}"))?,
        name: row.try_get("name")?,
        max_concurrent: u32::try_from(max_concurrent)
            .context("queue max_concurrent is out of range")?,
        max_bandwidth: max_bandwidth.map(i64_to_u64).transpose()?,
        sched_start: sched_start.map(i64_to_u16).transpose()?,
        sched_end: sched_end.map(i64_to_u16).transpose()?,
        sched_days: u8::try_from(row.try_get::<i64, _>("sched_days")?).unwrap_or(127),
        start_on_startup: row.try_get::<i64, _>("start_on_startup")? != 0,
        sync_interval_min: row
            .try_get::<Option<i64>, _>("sync_interval_min")?
            .and_then(|v| u32::try_from(v).ok()),
        max_retries: row
            .try_get::<Option<i64>, _>("max_retries")?
            .and_then(|v| u32::try_from(v).ok()),
        open_file: row.try_get("open_file")?,
        when_done: row.try_get("when_done")?,
        enabled: enabled != 0,
        priority: row.try_get("priority")?,
    })
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).context("value exceeds SQLite INTEGER range")
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).context("negative integer found in unsigned DB column")
}

fn i64_to_u16(value: i64) -> Result<u16> {
    u16::try_from(value).context("integer exceeds u16 range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queues_can_be_created_renamed_uniquely_and_deleted() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        let night = create(&pool, "Night", 3).await?.expect("created");
        assert_eq!(
            (night.name.as_str(), night.max_concurrent, night.sched_days),
            ("Night", 3, 127)
        );
        assert!(
            create(&pool, "Night", 1).await?.is_none(),
            "names are unique"
        );
        assert!(!night.start_on_startup);
        update(
            &pool,
            night.id,
            QueueUpdate {
                start_on_startup: Some(true),
                ..Default::default()
            },
        )
        .await?;
        assert!(get(&pool, night.id).await?.expect("queue").start_on_startup);
        assert_eq!(max_retries_for_download(&pool, Uuid::new_v4()).await, None);
        update(
            &pool,
            night.id,
            QueueUpdate {
                max_retries: Some(Some(2)),
                open_file: Some(Some("/tmp/done.txt".into())),
                when_done: Some(Some("exit".into())),
                ..Default::default()
            },
        )
        .await?;
        let got = get(&pool, night.id).await?.expect("queue");
        assert_eq!(
            (
                got.max_retries,
                got.open_file.as_deref(),
                got.when_done.as_deref()
            ),
            (Some(2), Some("/tmp/done.txt"), Some("exit"))
        );
        update(
            &pool,
            night.id,
            QueueUpdate {
                when_done: Some(None),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(get(&pool, night.id).await?.expect("queue").when_done, None);

        assert!(
            !delete(&pool, DEFAULT_QUEUE_ID).await?,
            "the default queue cannot be deleted"
        );
        assert!(delete(&pool, night.id).await?);
        assert!(get(&pool, night.id).await?.is_none());
        assert!(!delete(&pool, night.id).await?);
        Ok(())
    }

    #[tokio::test]
    async fn queue_get_list_update_round_trip() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        let queues = list(&pool).await?;
        assert!(queues.iter().any(|q| q.id == DEFAULT_QUEUE_ID));

        let default = get(&pool, DEFAULT_QUEUE_ID)
            .await?
            .expect("default queue exists");
        assert_eq!(default.name, "Default");
        assert_eq!(default.max_concurrent, 2);

        assert!(
            update(
                &pool,
                DEFAULT_QUEUE_ID,
                QueueUpdate {
                    name: Some("Fast".to_string()),
                    max_concurrent: Some(4),
                    max_bandwidth: Some(Some(1024)),
                    enabled: Some(false),
                    ..Default::default()
                },
            )
            .await?
        );

        let updated = get(&pool, DEFAULT_QUEUE_ID)
            .await?
            .expect("default queue exists");
        assert_eq!(updated.name, "Fast");
        assert_eq!(updated.max_concurrent, 4);
        assert_eq!(updated.max_bandwidth, Some(1024));
        assert!(!updated.enabled);

        assert!(!update(&pool, Uuid::new_v4(), QueueUpdate::default()).await?);
        Ok(())
    }

    #[tokio::test]
    async fn queue_max_bandwidth_can_be_cleared_to_none() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        // Set max_bandwidth to a non-zero value.
        update(
            &pool,
            DEFAULT_QUEUE_ID,
            QueueUpdate {
                max_bandwidth: Some(Some(4096)),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(
            get(&pool, DEFAULT_QUEUE_ID).await?.unwrap().max_bandwidth,
            Some(4096)
        );

        // Passing None preserves the current value.
        update(
            &pool,
            DEFAULT_QUEUE_ID,
            QueueUpdate {
                max_bandwidth: None,
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(
            get(&pool, DEFAULT_QUEUE_ID).await?.unwrap().max_bandwidth,
            Some(4096),
            "None must preserve the existing value"
        );

        // Passing Some(None) clears to NULL / unlimited.
        update(
            &pool,
            DEFAULT_QUEUE_ID,
            QueueUpdate {
                max_bandwidth: Some(None),
                ..Default::default()
            },
        )
        .await?;
        assert_eq!(
            get(&pool, DEFAULT_QUEUE_ID).await?.unwrap().max_bandwidth,
            None,
            "Some(None) must clear max_bandwidth to unlimited"
        );
        Ok(())
    }

    #[tokio::test]
    async fn queue_schedule_fields_can_be_set_and_cleared() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        // Set a time window.
        update(
            &pool,
            DEFAULT_QUEUE_ID,
            QueueUpdate {
                sched_start: Some(Some(540)), // 09:00
                sched_end: Some(Some(1020)),  // 17:00
                ..Default::default()
            },
        )
        .await?;
        let q = get(&pool, DEFAULT_QUEUE_ID).await?.unwrap();
        assert_eq!(q.sched_start, Some(540));
        assert_eq!(q.sched_end, Some(1020));

        // None preserves both values.
        update(&pool, DEFAULT_QUEUE_ID, QueueUpdate::default()).await?;
        let q = get(&pool, DEFAULT_QUEUE_ID).await?.unwrap();
        assert_eq!(q.sched_start, Some(540), "None must not clear sched_start");
        assert_eq!(q.sched_end, Some(1020), "None must not clear sched_end");

        // Some(None) clears the fields.
        update(
            &pool,
            DEFAULT_QUEUE_ID,
            QueueUpdate {
                sched_start: Some(None),
                sched_end: Some(None),
                ..Default::default()
            },
        )
        .await?;
        let q = get(&pool, DEFAULT_QUEUE_ID).await?.unwrap();
        assert_eq!(q.sched_start, None, "Some(None) must clear sched_start");
        assert_eq!(q.sched_end, None, "Some(None) must clear sched_end");
        Ok(())
    }
}

/// Retries per file configured on the queue a download belongs to (`None` = use the global setting).
pub async fn max_retries_for_download(pool: &SqlitePool, download_id: Uuid) -> Option<u32> {
    let v: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT q.max_retries FROM downloads d JOIN queues q ON q.id = COALESCE(d.queue_id, ?2) WHERE d.id = ?1",
    )
    .bind(download_id.to_string())
    .bind(DEFAULT_QUEUE_ID.to_string())
    .fetch_optional(pool)
    .await
    .ok()?;
    v.flatten().and_then(|n| u32::try_from(n).ok())
}
