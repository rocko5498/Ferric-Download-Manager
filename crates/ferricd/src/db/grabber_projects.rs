//! Saved Site Grabber projects and templates (IDM's "Recent projects" and "Project template").
use anyhow::Result;
use serde::Serialize;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Serialize)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    pub is_template: bool,
    pub updated_at: i64,
    pub result_count: usize,
}

#[derive(Serialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    pub is_template: bool,
    pub updated_at: i64,
    /// The crawl settings exactly as the window sent them (url, extensions, depth, filters ...).
    pub config: serde_json::Value,
    pub results: Vec<String>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// Create the project `name`, or replace its settings and results when it exists.
pub async fn save(
    pool: &SqlitePool,
    name: &str,
    is_template: bool,
    config: &serde_json::Value,
    results: &[String],
) -> Result<String> {
    let name = name.trim();
    anyhow::ensure!(!name.is_empty(), "a project needs a name");
    sqlx::query(
        "INSERT INTO grabber_projects (id, name, is_template, config, results, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(name) DO UPDATE SET is_template = excluded.is_template, config = excluded.config,
             results = excluded.results, updated_at = excluded.updated_at",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(name)
    .bind(i64::from(is_template))
    .bind(config.to_string())
    .bind(serde_json::to_string(results)?)
    .bind(now_ms())
    .execute(pool)
    .await?;
    let saved: String = sqlx::query_scalar("SELECT id FROM grabber_projects WHERE name = ?1")
        .bind(name)
        .fetch_one(pool)
        .await?;
    Ok(saved)
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<ProjectSummary>> {
    let rows = sqlx::query(
        "SELECT id, name, is_template, updated_at, results FROM grabber_projects ORDER BY updated_at DESC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|r| {
            let results: String = r.try_get("results")?;
            Ok(ProjectSummary {
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                is_template: r.try_get::<i64, _>("is_template")? != 0,
                updated_at: r.try_get("updated_at")?,
                result_count: serde_json::from_str::<Vec<String>>(&results).map_or(0, |v| v.len()),
            })
        })
        .collect()
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<Project>> {
    let Some(r) = sqlx::query(
        "SELECT id, name, is_template, updated_at, config, results FROM grabber_projects WHERE id = ?1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(None);
    };
    let config: String = r.try_get("config")?;
    let results: String = r.try_get("results")?;
    Ok(Some(Project {
        id: r.try_get("id")?,
        name: r.try_get("name")?,
        is_template: r.try_get::<i64, _>("is_template")? != 0,
        updated_at: r.try_get("updated_at")?,
        config: serde_json::from_str(&config).unwrap_or(serde_json::Value::Null),
        results: serde_json::from_str(&results).unwrap_or_default(),
    }))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM grabber_projects WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;

    #[tokio::test]
    async fn projects_are_saved_replaced_listed_and_deleted() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = db::init(&dir.path().join("ferric.db")).await?;
        let cfg = serde_json::json!({ "url": "http://x.test/", "depth": 2 });
        let id = save(
            &pool,
            "  site  ",
            false,
            &cfg,
            &["http://x.test/a.zip".into()],
        )
        .await?;
        // the same name again replaces: one row, the same id
        let again = save(&pool, "site", false, &cfg, &["a".into(), "b".into()]).await?;
        assert_eq!(id, again);
        let all = list(&pool).await?;
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].name, "site");
        assert_eq!(all[0].result_count, 2);
        save(&pool, "tpl", true, &cfg, &[]).await?;
        assert!(list(&pool)
            .await?
            .iter()
            .any(|p| p.name == "tpl" && p.is_template));
        let got = get(&pool, &id).await?.expect("project");
        assert_eq!(got.config["depth"], 2);
        assert!(save(&pool, "  ", false, &cfg, &[]).await.is_err());
        assert!(delete(&pool, &id).await?);
        assert!(get(&pool, &id).await?.is_none());
        assert!(!delete(&pool, &id).await?);
        Ok(())
    }
}
