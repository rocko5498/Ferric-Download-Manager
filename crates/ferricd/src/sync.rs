//! IDM's "Periodic synchronization" queues: every N minutes the finished files of the queue are
//! checked against the server and the ones that changed are downloaded again.
use std::collections::HashMap;
use std::time::Duration;

use ferric_common::QueueRecord;
use sqlx::SqlitePool;
use tokio::sync::mpsc;
use uuid::Uuid;

use crate::manager::ManagerCommand;
use crate::{db, scheduler, security};

type Version = (Option<String>, Option<String>, Option<u64>);

/// Start rule: the weekday is selected and the time of day is
/// `sched_start + k * interval` (the start time itself counts; before it nothing runs), and the
/// stop time, if any, has not passed.
#[must_use]
pub fn is_due(queue: &QueueRecord, minute: u16, weekday: u8) -> bool {
    let Some(interval) = queue.sync_interval_min.filter(|i| *i > 0) else {
        return false;
    };
    if !queue.enabled || !scheduler::runs_on(queue, weekday) {
        return false;
    }
    let start = queue.sched_start.unwrap_or(0);
    if minute < start || queue.sched_end.is_some_and(|end| minute > end) {
        return false;
    }
    u32::from(minute - start) % interval == 0
}

/// Whether the server's copy differs from the saved one: ETag first, then Last-Modified, then size.
#[must_use]
pub fn remote_changed(saved: &Version, remote: &Version) -> bool {
    if let (Some(a), Some(b)) = (&saved.0, &remote.0) {
        return a != b;
    }
    if let (Some(a), Some(b)) = (&saved.1, &remote.1) {
        return a != b;
    }
    matches!((saved.2, remote.2), (Some(a), Some(b)) if a != b)
}

async fn probe(client: &reqwest::Client, url: &str) -> Option<Version> {
    security::validate_url(url).ok()?;
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let h = resp.headers();
    let text = |n: &str| h.get(n).and_then(|v| v.to_str().ok()).map(str::to_owned);
    let size = text("content-range")
        .and_then(|v| v.rsplit('/').next().and_then(|t| t.parse().ok()))
        .or_else(|| text("content-length").and_then(|v| v.parse().ok()));
    Some((text("etag"), text("last-modified"), size))
}

async fn run_queue(
    pool: &SqlitePool,
    tx: &mpsc::Sender<ManagerCommand>,
    client: &reqwest::Client,
    queue: &QueueRecord,
) {
    tracing::info!(queue = %queue.name, "synchronization run");
    let Ok(ids) = db::queues::download_ids(pool, queue.id).await else {
        return;
    };
    for (id, status) in ids {
        if status != "completed" {
            continue;
        }
        let Ok(Some(rec)) = db::downloads::get(pool, id).await else {
            continue;
        };
        let Some(remote) = probe(client, rec.final_url.as_deref().unwrap_or(&rec.url)).await else {
            tracing::warn!(%id, queue = %queue.name, "synchronization: could not check the file on the server");
            continue;
        };
        let saved = (
            rec.etag.clone(),
            rec.last_modified.clone(),
            rec.total_size.or(Some(rec.downloaded)),
        );
        if remote_changed(&saved, &remote) {
            tracing::info!(%id, queue = %queue.name, "synchronization: file changed on the server, downloading again");
            tx.send(ManagerCommand::Redownload(id)).await.ok();
        }
    }
}

pub fn spawn(pool: SqlitePool, tx: mpsc::Sender<ManagerCommand>) {
    tokio::spawn(async move {
        let client = reqwest::Client::builder()
            .user_agent(crate::chunk::default_user_agent())
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap_or_default();
        // Queue id -> (weekday, minute) of its last run, so a 10 s tick never runs a minute twice.
        let mut last_run: HashMap<Uuid, (u8, u16)> = HashMap::new();
        loop {
            tokio::time::sleep(Duration::from_secs(10)).await;
            let (minute, weekday) = (
                scheduler::current_minute_local(),
                scheduler::current_weekday_local(),
            );
            for queue in db::queues::list(&pool).await.unwrap_or_default() {
                if is_due(&queue, minute, weekday)
                    && last_run.get(&queue.id) != Some(&(weekday, minute))
                {
                    last_run.insert(queue.id, (weekday, minute));
                    run_queue(&pool, &tx, &client, &queue).await;
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn queue(interval: Option<u32>, start: Option<u16>, end: Option<u16>, days: u8) -> QueueRecord {
        QueueRecord {
            id: Uuid::nil(),
            name: "Sync".into(),
            max_concurrent: 2,
            max_bandwidth: None,
            sched_start: start,
            sched_end: end,
            sched_days: days,
            start_on_startup: false,
            sync_interval_min: interval,
            max_retries: None,
            open_file: None,
            when_done: None,
            enabled: true,
            priority: 0,
        }
    }

    fn v(etag: Option<&str>, date: Option<&str>, size: Option<u64>) -> Version {
        (etag.map(Into::into), date.map(Into::into), size)
    }

    #[test]
    fn due_every_interval_from_the_start_time() {
        let q = queue(Some(60), Some(9 * 60), None, 127);
        assert!(is_due(&q, 9 * 60, 3));
        assert!(is_due(&q, 11 * 60, 3));
        assert!(!is_due(&q, 9 * 60 + 30, 3));
        assert!(!is_due(&q, 8 * 60, 3), "nothing before the start time");
        assert!(
            !is_due(&queue(None, Some(0), None, 127), 0, 3),
            "not a synchronization queue"
        );
    }

    #[test]
    fn stop_time_and_weekdays_limit_runs() {
        let q = queue(Some(30), Some(0), Some(60), 0b000_0010); // Monday only
        assert!(is_due(&q, 30, 1));
        assert!(!is_due(&q, 90, 1), "past the stop time");
        assert!(!is_due(&q, 30, 2), "Tuesday is not selected");
    }

    #[test]
    fn change_detection_prefers_etag_then_date_then_size() {
        assert!(remote_changed(
            &v(Some("a"), Some("d"), Some(1)),
            &v(Some("b"), Some("d"), Some(1))
        ));
        assert!(!remote_changed(
            &v(Some("a"), Some("d1"), Some(1)),
            &v(Some("a"), Some("d2"), Some(2))
        ));
        assert!(remote_changed(
            &v(None, Some("d1"), Some(1)),
            &v(None, Some("d2"), Some(1))
        ));
        assert!(remote_changed(
            &v(None, None, Some(1)),
            &v(None, None, Some(2))
        ));
        assert!(!remote_changed(
            &v(None, None, Some(1)),
            &v(None, None, Some(1))
        ));
    }
}
