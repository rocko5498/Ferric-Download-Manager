use sqlx::{sqlite::SqliteConnectOptions, SqlitePool};
use std::path::Path;
use std::path::PathBuf;

pub mod audit;
pub mod chunks;
pub mod cookies;
pub mod downloads;
pub mod extract_meta;
pub mod grabber_projects;
pub mod merge_jobs;
pub mod peer_bans;
pub mod proxies;
pub mod queues;
pub mod settings;
pub mod site_rules;
pub mod torrent_audit;
pub mod torrents;

pub fn default_db_path() -> anyhow::Result<PathBuf> {
    let data_dir = ferric_common::resolve_data_dir().map_err(|e| anyhow::anyhow!(e))?;
    Ok(data_dir.join("ferric.db"))
}

pub async fn init(db_path: &Path) -> anyhow::Result<SqlitePool> {
    if let Some(parent) = db_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let opts = SqliteConnectOptions::new()
        .filename(db_path)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(std::time::Duration::from_millis(5000))
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(opts).await?;

    // Run migrations from migrations/ directory
    sqlx::migrate!("../../migrations").run(&pool).await?;

    tracing::info!(path = %db_path.display(), "database ready");
    Ok(pool)
}

#[cfg(test)]
mod tests {
    use super::{chunks, cookies, downloads, settings};
    use crate::chunk::ChunkRange;
    use ferric_common::messages::DownloadRequest;
    use ferric_common::{ChunkStatus, DownloadStatus};
    use std::collections::HashMap;
    use uuid::Uuid;

    #[tokio::test]
    async fn sqlite_persistence_round_trip() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let db_path = dir.path().join("ferric.db");
        let pool = super::init(&db_path).await?;

        // Defaults are seeded by migrations; verify they are present.
        assert!(settings::get_all(&pool)
            .await?
            .contains_key("default_queue_id"));
        settings::set(&pool, "theme", "dark").await?;
        assert_eq!(
            settings::get(&pool, "theme").await?,
            Some("dark".to_string())
        );

        let req = DownloadRequest {
            id: Uuid::new_v4(),
            url: "https://example.com/releases/ferric.zip".to_string(),
            final_url: None,
            filename: None,
            referrer: Some("https://example.com/".to_string()),
            cookies: None,
            headers: Some(HashMap::new()),
            size_hint: Some(2048),
            mime_type: Some("application/zip".to_string()),
            method: "GET".to_string(),
            post_body_b64: None,
            queue_id: None,
            checksum: None,
            mirrors: vec![],
            timestamp: 1_234_567,
        };

        let record = downloads::insert(&pool, &req, "C:/Downloads/ferric.zip", "Archive").await?;
        assert_eq!(record.filename, "ferric.zip");
        downloads::update_response_metadata(
            &pool,
            req.id,
            Some("https://cdn.example/ferric.zip"),
            Some(r#"["https://example.com/releases/ferric.zip","https://cdn.example/ferric.zip"]"#),
            None,
            None,
            None,
        )
        .await?;
        let record = downloads::get(&pool, req.id).await?.unwrap();
        assert_eq!(
            record.final_url.as_deref(),
            Some("https://cdn.example/ferric.zip")
        );
        assert!(record
            .redirect_chain
            .as_deref()
            .is_some_and(|chain| chain.contains("cdn.example")));
        downloads::update_status(&pool, req.id, DownloadStatus::Connecting, None).await?;
        downloads::update_progress(&pool, req.id, 512).await?;
        downloads::set_file_info(&pool, req.id, 2048, Some("etag-1"), Some("today")).await?;

        let incomplete = downloads::find_incomplete(&pool).await?;
        assert_eq!(incomplete.len(), 1);
        assert_eq!(incomplete[0].id, req.id);
        assert_eq!(incomplete[0].downloaded, 512);

        chunks::insert_chunks(
            &pool,
            req.id,
            &[
                ChunkRange {
                    chunk_index: 0,
                    byte_start: 0,
                    byte_end: 1023,
                },
                ChunkRange {
                    chunk_index: 1,
                    byte_start: 1024,
                    byte_end: 2047,
                },
            ],
        )
        .await?;

        let chunk_records = chunks::get_chunks(&pool, req.id).await?;
        assert_eq!(chunk_records.len(), 2);
        chunks::update_chunk_progress(&pool, chunk_records[0].id, 1024, ChunkStatus::Completed)
            .await?;
        assert!(!chunks::all_chunks_complete(&pool, req.id).await?);
        chunks::update_chunk_progress(&pool, chunk_records[1].id, 1024, ChunkStatus::Completed)
            .await?;
        assert!(chunks::all_chunks_complete(&pool, req.id).await?);

        cookies::delete(&pool, req.id).await?;

        downloads::update_status(&pool, req.id, DownloadStatus::Completed, None).await?;
        assert!(downloads::find_incomplete(&pool).await?.is_empty());

        // description round-trip
        assert!(downloads::get(&pool, req.id)
            .await?
            .unwrap()
            .description
            .is_none());
        downloads::set_description(&pool, req.id, Some("test note")).await?;
        assert_eq!(
            downloads::get(&pool, req.id).await?.unwrap().description,
            Some("test note".to_string())
        );
        downloads::set_description(&pool, req.id, None).await?;
        assert!(downloads::get(&pool, req.id)
            .await?
            .unwrap()
            .description
            .is_none());

        Ok(())
    }
}
