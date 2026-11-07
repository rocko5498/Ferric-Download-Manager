//! Task 144 / 185 RSS — poll feeds for enclosure/magnet links.

use crate::manager::ManagerCommand;
use ferric_common::messages::{DaemonReply, DownloadRequest};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssFeed {
    pub id: String,
    pub url: String,
    pub poll_secs: i64,
    pub enabled: bool,
}

pub async fn list_feeds(pool: &SqlitePool) -> anyhow::Result<Vec<RssFeed>> {
    use sqlx::Row;
    let rows = sqlx::query("SELECT id, url, poll_secs, enabled FROM rss_feeds")
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(RssFeed {
                id: row.try_get("id").ok()?,
                url: row.try_get("url").ok()?,
                poll_secs: row.try_get("poll_secs").ok()?,
                enabled: row.try_get::<i64, _>("enabled").ok()? != 0,
            })
        })
        .collect())
}

pub async fn add_feed(pool: &SqlitePool, url: &str, poll_secs: i64) -> anyhow::Result<RssFeed> {
    crate::security::validate_url(url)?;
    let id = Uuid::new_v4().to_string();
    let poll_secs = poll_secs.clamp(300, 86_400);
    sqlx::query(
        "INSERT INTO rss_feeds (id, url, poll_secs, last_poll_at, enabled) VALUES (?1, ?2, ?3, NULL, 1)",
    )
    .bind(&id)
    .bind(url)
    .bind(poll_secs)
    .execute(pool)
    .await?;
    Ok(RssFeed {
        id,
        url: url.to_string(),
        poll_secs,
        enabled: true,
    })
}

/// Best-effort parse of enclosure urls / magnets from RSS/Atom XML text.
pub fn extract_links(xml: &str) -> Vec<String> {
    let mut out = Vec::new();
    for marker in ["url=\"", "url='", "href=\"", "href='"] {
        let mut rest = xml;
        while let Some(i) = rest.find(marker) {
            let after = &rest[i + marker.len()..];
            let end = after.find(['"', '\'']).unwrap_or(0);
            let u = &after[..end];
            if (u.starts_with("http://") || u.starts_with("https://") || u.starts_with("magnet:?"))
                && !out.iter().any(|x| x == u)
            {
                out.push(u.to_string());
            }
            rest = &after[end.saturating_add(1)..];
            if out.len() > 50 {
                return out;
            }
        }
    }
    out
}

// ── FQT-102: RSS auto-download rules ──────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RssRule {
    pub id: String,
    pub feed_id: String,
    pub name: String,
    pub match_pattern: String,
    pub action: String,
    pub save_dir: Option<String>,
    pub enabled: bool,
    pub created_at: i64,
}

pub async fn list_rules(pool: &SqlitePool, feed_id: Option<&str>) -> anyhow::Result<Vec<RssRule>> {
    use sqlx::Row;
    let rows = if let Some(fid) = feed_id {
        sqlx::query("SELECT id, feed_id, name, match_pattern, action, save_dir, enabled, created_at FROM rss_rules WHERE feed_id = ?1 ORDER BY created_at DESC")
            .bind(fid)
            .fetch_all(pool)
            .await
            .unwrap_or_default()
    } else {
        sqlx::query("SELECT id, feed_id, name, match_pattern, action, save_dir, enabled, created_at FROM rss_rules ORDER BY created_at DESC")
            .fetch_all(pool)
            .await
            .unwrap_or_default()
    };
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            Some(RssRule {
                id: row.try_get("id").ok()?,
                feed_id: row.try_get("feed_id").ok()?,
                name: row.try_get("name").ok()?,
                match_pattern: row.try_get("match_pattern").ok()?,
                action: row.try_get("action").ok()?,
                save_dir: row.try_get("save_dir").ok()?,
                enabled: row.try_get::<i64, _>("enabled").ok()? != 0,
                created_at: row.try_get("created_at").ok()?,
            })
        })
        .collect())
}

pub async fn add_rule(
    pool: &SqlitePool,
    feed_id: &str,
    name: &str,
    match_pattern: &str,
    action: &str,
    save_dir: Option<&str>,
) -> anyhow::Result<RssRule> {
    let id = Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64);
    sqlx::query(
        "INSERT INTO rss_rules (id, feed_id, name, match_pattern, action, save_dir, enabled, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 1, ?7)",
    )
    .bind(&id)
    .bind(feed_id)
    .bind(name)
    .bind(match_pattern)
    .bind(action)
    .bind(save_dir)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(RssRule {
        id,
        feed_id: feed_id.to_string(),
        name: name.to_string(),
        match_pattern: match_pattern.to_string(),
        action: action.to_string(),
        save_dir: save_dir.map(String::from),
        enabled: true,
        created_at: now,
    })
}

pub async fn delete_rule(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM rss_rules WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_feed(pool: &SqlitePool, id: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM rss_feeds WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Check if a URL matches any enabled rules for a feed. Returns the first matching rule.
pub async fn match_rules(
    pool: &SqlitePool,
    feed_id: &str,
    url: &str,
) -> anyhow::Result<Option<RssRule>> {
    let rules = list_rules(pool, Some(feed_id)).await?;
    for rule in rules {
        if !rule.enabled {
            continue;
        }
        if url.contains(&rule.match_pattern) {
            return Ok(Some(rule));
        }
    }
    Ok(None)
}

// ── FQT-102: Background RSS poll engine ────────────────────────────────

/// How often the scheduler looks for feeds that are due. Each feed has its own
/// `poll_secs`; polling every feed every 5 minutes (as this used to) ignored it
/// and a feed added just now waited the full 5 minutes for its first fetch.
const TICK: Duration = Duration::from_secs(30);

/// Spawn the background RSS poll loop. Follows `kill_switch::spawn` pattern.
pub fn spawn(pool: SqlitePool, manager_tx: mpsc::Sender<ManagerCommand>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(TICK).await;
            if let Err(e) = poll_due_feeds(&pool, &manager_tx, unix_now()).await {
                warn!(error = %e, "RSS poll cycle failed");
            }
        }
    });
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(0))
}

/// A feed is due when it was never polled or `poll_secs` have passed since.
fn is_due(last_poll_at: Option<i64>, poll_secs: i64, now: i64) -> bool {
    last_poll_at.is_none_or(|last| now.saturating_sub(last) >= poll_secs)
}

/// Poll every enabled feed that is due: fetch XML -> extract links -> match rules
/// -> trigger action, then stamp `last_poll_at` (also after a failure, so a dead
/// feed is retried at its own interval instead of every tick).
async fn poll_due_feeds(
    pool: &SqlitePool,
    manager_tx: &mpsc::Sender<ManagerCommand>,
    now: i64,
) -> anyhow::Result<()> {
    let rows = sqlx::query_as::<_, (String, String, i64, Option<i64>, i64)>(
        "SELECT id, url, poll_secs, last_poll_at, enabled FROM rss_feeds",
    )
    .fetch_all(pool)
    .await?;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .user_agent("Ferric/1.0")
        .build()?;

    for (id, url, poll_secs, last_poll_at, enabled) in rows {
        if enabled == 0 || !is_due(last_poll_at, poll_secs, now) {
            continue;
        }
        let feed = RssFeed {
            id,
            url,
            poll_secs,
            enabled: true,
        };
        poll_feed(pool, manager_tx, &client, &feed).await;
        sqlx::query("UPDATE rss_feeds SET last_poll_at = ?1 WHERE id = ?2")
            .bind(now)
            .bind(&feed.id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

async fn poll_feed(
    pool: &SqlitePool,
    manager_tx: &mpsc::Sender<ManagerCommand>,
    client: &reqwest::Client,
    feed: &RssFeed,
) {
    if let Err(e) = crate::security::validate_url(&feed.url) {
        warn!(feed = %feed.id, error = %e, "RSS feed URL rejected");
        return;
    }
    let xml = match client.get(&feed.url).send().await {
        Ok(resp) => match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                warn!(feed = %feed.id, error = %e, "failed to read RSS body");
                return;
            }
        },
        Err(e) => {
            warn!(feed = %feed.id, error = %e, "failed to fetch RSS feed");
            return;
        }
    };

    for url in &extract_links(&xml) {
        let rule = match match_rules(pool, &feed.id, url).await {
            Ok(Some(r)) => r,
            Ok(None) => continue,
            Err(e) => {
                warn!(feed = %feed.id, error = %e, "failed to match rules");
                continue;
            }
        };

        let save_dir = rule.save_dir.as_deref();
        match rule.action.as_str() {
            "download" => {
                if let Err(e) = trigger_download(pool, manager_tx, url, save_dir).await {
                    warn!(rule = %rule.id, url = %url, error = %e, "failed to trigger download");
                }
            }
            "torrent" | "magnet" => {
                // Torrent staging requires probe -> stage flow; log for now.
                info!(rule = %rule.id, url = %url, "RSS torrent match - staging not yet wired, skipping");
            }
            other => {
                warn!(rule = %rule.id, action = other, "unknown RSS rule action");
            }
        }
    }
}

/// Send a download command to the manager. Deduplicates by URL (checks DB first).
async fn trigger_download(
    pool: &SqlitePool,
    manager_tx: &mpsc::Sender<ManagerCommand>,
    url: &str,
    save_dir: Option<&str>,
) -> anyhow::Result<()> {
    // Deduplicate: skip if this URL already exists in the download table
    let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM downloads WHERE url = ?1")
        .bind(url)
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    if exists > 0 {
        return Ok(());
    }

    let req = DownloadRequest {
        id: Uuid::new_v4(),
        url: url.to_string(),
        final_url: None,
        filename: None,
        referrer: None,
        cookies: None,
        headers: None,
        size_hint: None,
        mime_type: None,
        method: "GET".into(),
        post_body_b64: None,
        queue_id: None,
        checksum: None,
        mirrors: vec![],
        timestamp: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64),
    };

    let (tx, rx) = tokio::sync::oneshot::channel();
    manager_tx
        .send(ManagerCommand::Add {
            req,
            save_dir_override: save_dir.map(std::path::PathBuf::from),
            hold: false,
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            reply_tx: tx,
        })
        .await
        .map_err(|_| anyhow::anyhow!("manager channel closed"))?;

    match rx.await {
        Ok(DaemonReply::Accepted { id }) => {
            info!(download_id = %id, url = %url, "RSS auto-download started");
            Ok(())
        }
        Ok(DaemonReply::Error { message, .. }) => {
            Err(anyhow::anyhow!("manager rejected: {message}"))
        }
        Err(_) => Err(anyhow::anyhow!("manager reply lost")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_is_due_when_never_polled_or_interval_elapsed() {
        assert!(is_due(None, 3600, 1_000));
        assert!(!is_due(Some(1_000), 3600, 1_000 + 3599));
        assert!(is_due(Some(1_000), 3600, 1_000 + 3600));
    }

    #[test]
    fn parses_enclosure() {
        let xml =
            r#"<item><enclosure url="https://cdn.example.com/a.mp4" type="video/mp4"/></item>"#;
        let links = extract_links(xml);
        assert_eq!(links.len(), 1);
    }

    #[tokio::test]
    async fn match_rules_finds_matching_pattern() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE rss_feeds (id TEXT PRIMARY KEY, url TEXT, poll_secs INTEGER, last_poll_at INTEGER, enabled INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE rss_rules (id TEXT PRIMARY KEY, feed_id TEXT, name TEXT, match_pattern TEXT, action TEXT, save_dir TEXT, enabled INTEGER, created_at INTEGER)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let feed = add_feed(&pool, "https://example.com/feed.xml", 3600)
            .await
            .unwrap();
        add_rule(&pool, &feed.id, "mp4 rule", ".mp4", "download", None)
            .await
            .unwrap();

        let hit = match_rules(&pool, &feed.id, "https://cdn.example.com/video.mp4")
            .await
            .unwrap();
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().action, "download");

        let miss = match_rules(&pool, &feed.id, "https://cdn.example.com/image.png")
            .await
            .unwrap();
        assert!(miss.is_none());
    }
}
