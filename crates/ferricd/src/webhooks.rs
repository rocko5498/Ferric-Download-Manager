//! Task 154 — fire-and-forget webhooks on download events.

use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

pub async fn emit(pool: &SqlitePool, event: &str, payload: serde_json::Value) {
    let enabled = crate::db::settings::get(pool, "webhook_enabled")
        .await
        .ok()
        .flatten()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    if !enabled {
        return;
    }

    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT id, url, events FROM webhooks WHERE enabled = 1",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };

    for (id, url, events) in rows {
        let ok = events == "*" || events.split(',').any(|e| e.trim() == event);
        if !ok {
            continue;
        }
        // SSRF: only allow http(s) public hosts
        if crate::security::validate_url(&url).is_err() {
            tracing::warn!(%id, "webhook URL rejected by validate_url");
            continue;
        }
        let body = json!({
            "event": event,
            "webhook_id": id,
            "data": payload,
            "ts": chrono_like_ms(),
        });
        let client = client.clone();
        let url = url.clone();
        tokio::spawn(async move {
            let _ = client.post(&url).json(&body).send().await;
        });
    }
}

pub async fn emit_download_done(pool: &SqlitePool, download_id: Uuid, path: &str) {
    emit(
        pool,
        "download.completed",
        json!({ "id": download_id, "path": path }),
    )
    .await;
}

fn chrono_like_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}
