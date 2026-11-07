use anyhow::Result;
use ferric_common::{MergeJobRecord, MergeState};
use sqlx::{Row, SqlitePool};

pub async fn upsert(pool: &SqlitePool, job: &MergeJobRecord) -> Result<()> {
    sqlx::query(
        r"
        INSERT INTO merge_jobs (
            id, video_path, audio_path, output_path, state, message, created_at, completed_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(id) DO UPDATE SET
            video_path = excluded.video_path,
            audio_path = excluded.audio_path,
            output_path = excluded.output_path,
            state = excluded.state,
            message = excluded.message,
            completed_at = excluded.completed_at
        ",
    )
    .bind(&job.id)
    .bind(&job.video_path)
    .bind(&job.audio_path)
    .bind(&job.output_path)
    .bind(state_to_str(&job.state))
    .bind(&job.message)
    .bind(u64_to_i64(job.created_at)?)
    .bind(job.completed_at.map(u64_to_i64).transpose()?)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<MergeJobRecord>> {
    let rows = sqlx::query("SELECT * FROM merge_jobs ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(record_from_row).collect()
}

fn record_from_row(row: sqlx::sqlite::SqliteRow) -> Result<MergeJobRecord> {
    let state: String = row.try_get("state")?;
    Ok(MergeJobRecord {
        id: row.try_get("id")?,
        video_path: row.try_get("video_path")?,
        audio_path: row.try_get("audio_path")?,
        output_path: row.try_get("output_path")?,
        state: state_from_str(&state)?,
        message: row.try_get("message")?,
        created_at: i64_to_u64(row.try_get("created_at")?)?,
        completed_at: row
            .try_get::<Option<i64>, _>("completed_at")?
            .map(i64_to_u64)
            .transpose()?,
    })
}

fn state_from_str(value: &str) -> Result<MergeState> {
    match value {
        "queued" => Ok(MergeState::Queued),
        "merging" => Ok(MergeState::Merging),
        "completed" => Ok(MergeState::Completed),
        "failed" => Ok(MergeState::Failed),
        other => Err(anyhow::anyhow!("invalid merge state: {other}")),
    }
}

fn state_to_str(state: &MergeState) -> &'static str {
    match state {
        MergeState::Queued => "queued",
        MergeState::Merging => "merging",
        MergeState::Completed => "completed",
        MergeState::Failed => "failed",
    }
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| anyhow::anyhow!("value too large for sqlite integer: {value}"))
}

fn i64_to_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| anyhow::anyhow!("negative sqlite integer: {value}"))
}
