//! Tasks 124, 180, 190 — privacy dashboard + anonymity score.

use serde::Serialize;
use sqlx::SqlitePool;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct DomainContact {
    pub host: String,
    pub count: u64,
    pub is_tracker_like: bool,
    pub risk: u8,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrivacyDashboard {
    pub unique_hosts: usize,
    pub total_events: u64,
    pub domains: Vec<DomainContact>,
    pub summary: String,
    pub recommendations: Vec<String>,
}

/// Aggregate download audit + global network audit for the last `days` days.
pub async fn dashboard(pool: &SqlitePool, days: i64) -> anyhow::Result<PrivacyDashboard> {
    let since = now_ms() - days.saturating_mul(86_400_000);
    let mut host_counts: HashMap<String, u64> = HashMap::new();

    use sqlx::Row;
    let rows = sqlx::query("SELECT detail_json FROM download_audit WHERE created_at >= ?1")
        .bind(since)
        .fetch_all(pool)
        .await
        .unwrap_or_default();

    for row in rows {
        let Ok(json) = row.try_get::<String, _>("detail_json") else {
            continue;
        };
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
                if let Some(h) = host_of(url) {
                    *host_counts.entry(h).or_default() += 1;
                }
            }
        }
    }

    let global = sqlx::query(
        "SELECT host FROM network_audit_global WHERE created_at >= ?1 AND host IS NOT NULL",
    )
    .bind(since)
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    for row in global {
        if let Ok(h) = row.try_get::<String, _>("host") {
            *host_counts.entry(h).or_default() += 1;
        }
    }

    let dls = sqlx::query("SELECT url FROM downloads WHERE created_at >= ?1")
        .bind(since)
        .fetch_all(pool)
        .await
        .unwrap_or_default();
    for row in dls {
        if let Ok(url) = row.try_get::<String, _>("url") {
            if let Some(h) = host_of(&url) {
                *host_counts.entry(h).or_default() += 1;
            }
        }
    }

    let total_events: u64 = host_counts.values().sum();
    let mut domains: Vec<DomainContact> = host_counts
        .into_iter()
        .map(|(host, count)| {
            let is_tracker_like = looks_tracker(&host);
            let risk = if is_tracker_like {
                70
            } else if is_cdn(&host) {
                15
            } else {
                35
            };
            DomainContact {
                host,
                count,
                is_tracker_like,
                risk,
            }
        })
        .collect();
    domains.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.host.cmp(&b.host)));
    domains.truncate(50);

    let unique_hosts = domains.len();
    let mut recommendations = Vec::new();
    if domains.iter().any(|d| d.is_tracker_like) {
        recommendations
            .push("Tracker-like hosts seen — prefer torrent proxy + disable DHT.".into());
    }
    if unique_hosts > 20 {
        recommendations
            .push("High host diversity this week — review extension capture filter.".into());
    }
    if recommendations.is_empty() {
        recommendations
            .push("Activity looks quiet. Keep proxy defaults and SSRF protections on.".into());
    }

    let summary = format!(
        "Your download activity contacted {unique_hosts} unique domain(s) ({total_events} events) in the last {days} day(s)."
    );

    Ok(PrivacyDashboard {
        unique_hosts,
        total_events,
        domains,
        summary,
        recommendations,
    })
}

#[derive(Debug, Clone, Serialize)]
pub struct AnonymityScore {
    /// `false` when torrents are not in use: every component below is a torrent
    /// setting, so a number would only mislead (shown as N/A).
    pub applicable: bool,
    pub score: u8,
    pub grade: char,
    pub breakdown: Vec<ScoreComponent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScoreComponent {
    pub name: String,
    pub points_possible: u8,
    pub points_earned: u8,
    pub fix_action: Option<String>,
}

pub async fn anonymity_score(pool: &SqlitePool) -> anyhow::Result<AnonymityScore> {
    let get = |k: &'static str| async move {
        crate::db::settings::get(pool, k)
            .await
            .ok()
            .flatten()
            .unwrap_or_default()
    };

    let dht_off = get("torrent_dht_enabled").await != "1";
    let proxy = get("torrent_proxy_url").await;
    let proxy_on = !proxy.trim().is_empty();
    let seeding_off = get("torrent_seeding_default").await != "1";
    let blocklist = !get("torrent_blocklist_url").await.trim().is_empty();
    let kill = get("torrent_kill_switch_active").await;

    let mut breakdown = Vec::new();
    let mut score = 0u8;

    let p = if proxy_on { 50 } else { 0 };
    score = score.saturating_add(p);
    breakdown.push(ScoreComponent {
        name: "Torrent proxy".into(),
        points_possible: 50,
        points_earned: p,
        fix_action: if proxy_on {
            None
        } else {
            Some("Set torrent_proxy_url (socks5://…)".into())
        },
    });

    let p = if dht_off { 15 } else { 0 };
    score = score.saturating_add(p);
    breakdown.push(ScoreComponent {
        name: "DHT disabled".into(),
        points_possible: 15,
        points_earned: p,
        fix_action: if dht_off {
            None
        } else {
            Some("Set torrent_dht_enabled=0".into())
        },
    });

    let p = if seeding_off { 10 } else { 0 };
    score = score.saturating_add(p);
    breakdown.push(ScoreComponent {
        name: "Seeding default off".into(),
        points_possible: 10,
        points_earned: p,
        fix_action: if seeding_off {
            None
        } else {
            Some("Set torrent_seeding_default=0".into())
        },
    });

    let p = if blocklist { 15 } else { 0 };
    score = score.saturating_add(p);
    breakdown.push(ScoreComponent {
        name: "IP blocklist".into(),
        points_possible: 15,
        points_earned: p,
        fix_action: if blocklist {
            None
        } else {
            Some("Configure torrent_blocklist_url".into())
        },
    });

    let p = if kill == "1" { 5 } else { 10 }; // kill active means currently exposed recovery
    score = score.saturating_add(p);
    breakdown.push(ScoreComponent {
        name: "Kill-switch state".into(),
        points_possible: 10,
        points_earned: p,
        fix_action: None,
    });

    let grade = match score {
        90..=255 => 'A',
        75..=89 => 'B',
        60..=74 => 'C',
        40..=59 => 'D',
        _ => 'F',
    };

    let torrents: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM torrents")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    Ok(AnonymityScore {
        applicable: torrents > 0 || proxy_on,
        score: score.min(100),
        grade,
        breakdown,
    })
}

fn host_of(url: &str) -> Option<String> {
    url::Url::parse(url).ok().and_then(|u| {
        u.host_str()
            .map(|h| h.trim_start_matches("www.").to_ascii_lowercase())
    })
}

fn looks_tracker(host: &str) -> bool {
    host.contains("tracker") || host.contains("announce") || host.ends_with(".onion")
}

fn is_cdn(host: &str) -> bool {
    host.contains("cloudfront")
        || host.contains("akamai")
        || host.contains("fastly")
        || host.contains("cdn.")
        || host.ends_with(".googleapis.com")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn score_is_not_applicable_without_torrents() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        assert!(!anonymity_score(&pool).await?.applicable);
        crate::db::settings::set(&pool, "torrent_proxy_url", "socks5://127.0.0.1:1080").await?;
        assert!(anonymity_score(&pool).await?.applicable);
        Ok(())
    }
}
