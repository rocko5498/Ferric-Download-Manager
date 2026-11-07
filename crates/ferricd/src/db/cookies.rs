// Per-download cookie rows. Task 130 purges them on complete.

use anyhow::Result;
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn delete(pool: &SqlitePool, download_id: Uuid) -> Result<()> {
    sqlx::query("DELETE FROM cookies WHERE download_id = ?1")
        .bind(download_id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}
