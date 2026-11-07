#[cfg(test)]
use anyhow::Context as _;
use anyhow::Result;
use globset::Glob;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

pub const LEARNED_SUCCESS_PRIORITY: i32 = 50;
pub const LEARNED_RATE_LIMIT_PRIORITY: i32 = 100;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SitePolicy {
    pub max_connections: Option<usize>,
    pub max_speed_bps: Option<u64>,
}

pub async fn policy_for_url(pool: &SqlitePool, url: &str) -> Result<SitePolicy> {
    let rows = sqlx::query(
        r"
        SELECT pattern, rule_type, value
        FROM site_rules
        ORDER BY priority DESC
        ",
    )
    .fetch_all(pool)
    .await?;

    let mut policy = SitePolicy::default();
    for row in rows {
        let pattern: String = row.try_get("pattern")?;
        if !pattern_matches_url(&pattern, url) {
            continue;
        }

        let rule_type: String = row.try_get("rule_type")?;
        let value: Option<String> = row.try_get("value")?;
        match rule_type.as_str() {
            "max_connections" if policy.max_connections.is_none() => {
                policy.max_connections = value
                    .as_deref()
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .filter(|value| *value > 0);
            }
            // max_speed_bps (legacy) and bandwidth_cap_bytes (Task 100 name) are aliases.
            "max_speed_bps" | "bandwidth_cap_bytes" if policy.max_speed_bps.is_none() => {
                policy.max_speed_bps = value
                    .as_deref()
                    .and_then(|value| value.trim().parse::<u64>().ok())
                    .filter(|value| *value > 0);
            }
            _ => {}
        }
    }

    Ok(policy)
}

pub async fn learn_max_connections(
    pool: &SqlitePool,
    url: &str,
    max_connections: usize,
    priority: i32,
) -> Result<()> {
    let pattern = host_pattern(url);
    let value = max_connections.max(1).to_string();

    sqlx::query(
        r"
        DELETE FROM site_rules
        WHERE pattern = ?1 AND rule_type = 'max_connections'
          AND priority <= ?2
        ",
    )
    .bind(&pattern)
    .bind(priority)
    .execute(pool)
    .await?;

    let higher_priority_exists: bool = sqlx::query_scalar(
        r"
        SELECT EXISTS(
            SELECT 1
            FROM site_rules
            WHERE pattern = ?1 AND rule_type = 'max_connections'
              AND priority > ?2
        )
        ",
    )
    .bind(&pattern)
    .bind(priority)
    .fetch_one(pool)
    .await?;
    if higher_priority_exists {
        return Ok(());
    }

    insert_rule(pool, &pattern, "max_connections", Some(&value), priority).await
}

pub async fn insert_rule(
    pool: &SqlitePool,
    pattern: &str,
    rule_type: &str,
    value: Option<&str>,
    priority: i32,
) -> Result<()> {
    sqlx::query(
        r"
        INSERT INTO site_rules (id, pattern, rule_type, value, priority)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(pattern)
    .bind(rule_type)
    .bind(value)
    .bind(priority)
    .execute(pool)
    .await?;
    Ok(())
}

/// Row shape for CRUD API (W1).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SiteRuleRow {
    pub id: String,
    pub pattern: String,
    pub rule_type: String,
    pub value: Option<String>,
    pub priority: i32,
}

pub async fn list_rules(pool: &SqlitePool) -> Result<Vec<SiteRuleRow>> {
    let rows = sqlx::query(
        r"
        SELECT id, pattern, rule_type, value, priority
        FROM site_rules
        ORDER BY priority DESC, pattern ASC
        ",
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(SiteRuleRow {
            id: row.try_get("id")?,
            pattern: row.try_get("pattern")?,
            rule_type: row.try_get("rule_type")?,
            value: row.try_get("value")?,
            priority: row.try_get("priority")?,
        });
    }
    Ok(out)
}

pub async fn insert_rule_row(
    pool: &SqlitePool,
    pattern: &str,
    rule_type: &str,
    value: Option<&str>,
    priority: i32,
) -> Result<SiteRuleRow> {
    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r"
        INSERT INTO site_rules (id, pattern, rule_type, value, priority)
        VALUES (?1, ?2, ?3, ?4, ?5)
        ",
    )
    .bind(&id)
    .bind(pattern)
    .bind(rule_type)
    .bind(value)
    .bind(priority)
    .execute(pool)
    .await?;
    Ok(SiteRuleRow {
        id,
        pattern: pattern.to_string(),
        rule_type: rule_type.to_string(),
        value: value.map(str::to_string),
        priority,
    })
}

pub async fn update_rule(
    pool: &SqlitePool,
    id: &str,
    pattern: Option<&str>,
    rule_type: Option<&str>,
    value: Option<Option<&str>>,
    priority: Option<i32>,
) -> Result<Option<SiteRuleRow>> {
    let existing = sqlx::query(
        r"
        SELECT id, pattern, rule_type, value, priority
        FROM site_rules
        WHERE id = ?1
        ",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    let Some(row) = existing else {
        return Ok(None);
    };

    let pattern = pattern.map_or_else(
        || row.try_get::<String, _>("pattern").unwrap_or_default(),
        str::to_string,
    );
    let rule_type = rule_type.map_or_else(
        || row.try_get::<String, _>("rule_type").unwrap_or_default(),
        str::to_string,
    );
    let value = match value {
        Some(v) => v.map(str::to_string),
        None => row.try_get::<Option<String>, _>("value").unwrap_or(None),
    };
    let priority = priority.unwrap_or_else(|| row.try_get::<i32, _>("priority").unwrap_or(0));

    sqlx::query(
        r"
        UPDATE site_rules
        SET pattern = ?2, rule_type = ?3, value = ?4, priority = ?5
        WHERE id = ?1
        ",
    )
    .bind(id)
    .bind(&pattern)
    .bind(&rule_type)
    .bind(&value)
    .bind(priority)
    .execute(pool)
    .await?;

    Ok(Some(SiteRuleRow {
        id: id.to_string(),
        pattern,
        rule_type,
        value,
        priority,
    }))
}

pub async fn delete_rule(pool: &SqlitePool, id: &str) -> Result<bool> {
    let res = sqlx::query("DELETE FROM site_rules WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

fn pattern_matches_url(pattern: &str, url: &str) -> bool {
    if let Some(expected_host) = pattern.strip_prefix("host:") {
        return url::Url::parse(url)
            .ok()
            .and_then(|url| url.host_str().map(ToOwned::to_owned))
            .is_some_and(|host| host.eq_ignore_ascii_case(expected_host));
    }

    Glob::new(pattern).is_ok_and(|glob| glob.compile_matcher().is_match(url))
}

fn host_pattern(url: &str) -> String {
    url::Url::parse(url)
        .ok()
        .and_then(|url| url.host_str().map(ToOwned::to_owned))
        .map_or_else(|| url.to_string(), |host| format!("host:{host}"))
}

#[cfg(test)]
async fn count_for_rule(pool: &SqlitePool, pattern: &str, rule_type: &str) -> Result<i64> {
    sqlx::query_scalar(
        r"
        SELECT COUNT(*)
        FROM site_rules
        WHERE pattern = ?1 AND rule_type = ?2
        ",
    )
    .bind(pattern)
    .bind(rule_type)
    .fetch_one(pool)
    .await
    .context("counting site rules")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn policy_uses_highest_priority_matching_rule() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        insert_rule(&pool, "*://example.com/*", "max_connections", Some("8"), 10).await?;
        insert_rule(&pool, "*://example.com/*", "max_connections", Some("2"), 20).await?;
        insert_rule(&pool, "*://other.test/*", "max_connections", Some("1"), 100).await?;

        let policy = policy_for_url(&pool, "https://example.com/file.bin").await?;

        assert_eq!(policy.max_connections, Some(2));
        assert_eq!(
            policy_for_url(&pool, "https://missing.test/file.bin")
                .await?
                .max_connections,
            None
        );
        Ok(())
    }

    #[tokio::test]
    async fn learned_max_connections_rule_is_upserted_per_host() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        let url = "http://127.0.0.1:12345/payload.bin";
        let pattern = host_pattern(url);

        learn_max_connections(&pool, url, 1, LEARNED_RATE_LIMIT_PRIORITY).await?;
        learn_max_connections(&pool, url, 1, LEARNED_RATE_LIMIT_PRIORITY).await?;

        assert_eq!(count_for_rule(&pool, &pattern, "max_connections").await?, 1);
        assert_eq!(
            policy_for_url(&pool, "http://127.0.0.1:54321/other.bin")
                .await?
                .max_connections,
            Some(1)
        );
        Ok(())
    }

    #[tokio::test]
    async fn success_priority_does_not_replace_rate_limit_priority() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        let url = "http://127.0.0.1:12345/payload.bin";
        let pattern = host_pattern(url);

        learn_max_connections(&pool, url, 1, LEARNED_RATE_LIMIT_PRIORITY).await?;
        learn_max_connections(&pool, url, 4, LEARNED_SUCCESS_PRIORITY).await?;

        assert_eq!(count_for_rule(&pool, &pattern, "max_connections").await?, 1);
        assert_eq!(
            policy_for_url(&pool, "http://127.0.0.1:54321/other.bin")
                .await?
                .max_connections,
            Some(1)
        );
        let priority: i64 = sqlx::query_scalar(
            "SELECT priority FROM site_rules WHERE pattern = ?1 AND rule_type = 'max_connections'",
        )
        .bind(pattern)
        .fetch_one(&pool)
        .await?;
        assert_eq!(priority, i64::from(LEARNED_RATE_LIMIT_PRIORITY));
        Ok(())
    }

    #[tokio::test]
    async fn site_rule_max_speed_bps_is_parsed_into_policy() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        insert_rule(
            &pool,
            "host:example.com",
            "max_speed_bps",
            Some("1024"),
            100,
        )
        .await?;

        let policy = policy_for_url(&pool, "https://example.com/file.bin").await?;
        assert_eq!(
            policy.max_speed_bps,
            Some(1024),
            "max_speed_bps rule must be parsed into SitePolicy"
        );
        Ok(())
    }

    #[tokio::test]
    async fn bandwidth_cap_bytes_alias_is_parsed() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        insert_rule(
            &pool,
            "host:cdn.example",
            "bandwidth_cap_bytes",
            Some("4096"),
            50,
        )
        .await?;
        let policy = policy_for_url(&pool, "https://cdn.example/v.bin").await?;
        assert_eq!(policy.max_speed_bps, Some(4096));
        Ok(())
    }

    #[tokio::test]
    async fn learned_host_rule_is_exact_but_port_insensitive() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;

        learn_max_connections(
            &pool,
            "https://example.com:8443/payload.bin",
            1,
            LEARNED_RATE_LIMIT_PRIORITY,
        )
        .await?;

        assert_eq!(
            policy_for_url(&pool, "https://example.com/file.bin")
                .await?
                .max_connections,
            Some(1)
        );
        assert_eq!(
            policy_for_url(&pool, "https://example.com:9443/file.bin")
                .await?
                .max_connections,
            Some(1)
        );
        assert_eq!(
            policy_for_url(&pool, "https://example.com.evil/file.bin")
                .await?
                .max_connections,
            None
        );
        Ok(())
    }
}
