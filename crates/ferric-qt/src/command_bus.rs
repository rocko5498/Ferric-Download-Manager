use crate::daemon_session::read_lock_file;
use core::pin::Pin;
use cxx_qt::Threading;
use cxx_qt_lib::QString;

fn post_action(id: &str, action: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    post_action_at(port, &token, id, action)
}

fn post_action_at(port: u16, token: &str, id: &str, action: &str) -> Result<(), String> {
    let url = format!("http://127.0.0.1:{port}/downloads/{id}/{action}");
    // The request future must be *created* inside the runtime: reqwest arms its
    // timeout timer when `send()` is called, which panics ("no reactor running")
    // outside a Tokio context and silently killed this worker thread.
    let resp = crate::http_client::RUNTIME
        .block_on(async {
            crate::http_client::HTTP_CLIENT
                .clone()
                .post(&url)
                .header("X-Ferric-Auth", token)
                .send()
                .await
        })
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if !resp.status().is_success() {
        Err(format!("daemon returned {}", resp.status()))
    } else {
        Ok(())
    }
}

fn delete_action(id: &str, with_file: bool) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/{id}?delete_file={with_file}");
    let resp = crate::http_client::RUNTIME
        .block_on(async {
            crate::http_client::HTTP_CLIENT
                .clone()
                .delete(&url)
                .header("X-Ferric-Auth", &token)
                .send()
                .await
        })
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if !resp.status().is_success() {
        Err(format!("daemon returned {}", resp.status()))
    } else {
        Ok(())
    }
}

fn to_qs(r: Result<(), String>) -> QString {
    match r {
        Ok(()) => QString::from(""),
        Err(e) => QString::from(e.as_str()),
    }
}

fn add_download_http(
    url: &str,
    filename: &str,
    save_dir: &str,
    cookies: &str,
    description: &str,
    hold: bool,
) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/downloads");
    let mut body = serde_json::json!({ "url": url });
    if hold {
        // Held for the Download File Info dialog (announced to the UI by a capture_held event).
        body["hold"] = serde_json::Value::Bool(true);
    }
    if !filename.is_empty() {
        body["filename"] = serde_json::Value::String(filename.to_owned());
    }
    if !save_dir.is_empty() {
        body["save_dir"] = serde_json::Value::String(save_dir.to_owned());
    }
    if !cookies.is_empty() {
        body["cookies"] = serde_json::Value::String(cookies.to_owned());
    }
    if !description.is_empty() {
        body["description"] = serde_json::Value::String(description.to_owned());
    }
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            return Ok(());
        }
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        Err(format!("daemon returned {status}: {text}"))
    })
}

fn pause_all_http() -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    crate::http_client::RUNTIME.block_on(async {
        use ferric_common::models::DownloadStatus;
        let client = crate::http_client::HTTP_CLIENT.clone();
        let records: Vec<ferric_common::models::DownloadRecord> = client
            .get(format!("http://127.0.0.1:{port}/downloads"))
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        for rec in &records {
            if matches!(
                rec.status,
                DownloadStatus::Queued | DownloadStatus::Connecting | DownloadStatus::Downloading
            ) {
                let _ = client
                    .post(format!(
                        "http://127.0.0.1:{port}/downloads/{}/pause",
                        rec.id
                    ))
                    .header("X-Ferric-Auth", &token)
                    .send()
                    .await;
            }
        }
        Ok(())
    })
}

fn resume_all_http() -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    crate::http_client::RUNTIME.block_on(async {
        use ferric_common::models::DownloadStatus;
        let client = crate::http_client::HTTP_CLIENT.clone();
        let records: Vec<ferric_common::models::DownloadRecord> = client
            .get(format!("http://127.0.0.1:{port}/downloads"))
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        for rec in &records {
            if matches!(rec.status, DownloadStatus::Paused | DownloadStatus::Error) {
                let _ = client
                    .post(format!(
                        "http://127.0.0.1:{port}/downloads/{}/resume",
                        rec.id
                    ))
                    .header("X-Ferric-Auth", &token)
                    .send()
                    .await;
            }
        }
        Ok(())
    })
}

fn clear_completed_http() -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    crate::http_client::RUNTIME.block_on(async {
        use ferric_common::models::DownloadStatus;
        let client = crate::http_client::HTTP_CLIENT.clone();
        let records: Vec<ferric_common::models::DownloadRecord> = client
            .get(format!("http://127.0.0.1:{port}/downloads"))
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?
            .json()
            .await
            .map_err(|e| e.to_string())?;
        for rec in &records {
            if matches!(rec.status, DownloadStatus::Completed) {
                let _ = client
                    .delete(format!(
                        "http://127.0.0.1:{port}/downloads/{}?delete_file=false",
                        rec.id
                    ))
                    .header("X-Ferric-Auth", &token)
                    .send()
                    .await;
            }
        }
        Ok(())
    })
}

fn get_json_http(path_suffix: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/{path_suffix}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .get(&url)
            .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("daemon returned {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

fn put_json_http(path_suffix: &str, body: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/{path_suffix}");
    let body = body.to_string();
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .put(&url)
            .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("daemon returned {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

fn post_json_http(path_suffix: &str, body: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/{path_suffix}");
    let body = body.to_string();
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("daemon returned {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

fn delete_json_http(path_suffix: &str, body: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/{path_suffix}");
    let body = body.to_string();
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .header("Content-Type", "application/json")
            .body(body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("daemon returned {}", resp.status()));
        }
        resp.text().await.map_err(|e| e.to_string())
    })
}

fn get_settings_http() -> Result<String, String> {
    get_json_http("settings")
}

fn update_torrent_policy_http(id: &str, policy_json: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/policy");
    let body: serde_json::Value =
        serde_json::from_str(policy_json).map_err(|e| format!("bad policy json: {e}"))?;
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .patch(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

/// Asks the running Ferric window (via the daemon) to come forward. Used by a second launch when the
/// first copy was started hidden at login and has no native window to find yet.
pub fn request_show_ui() -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/ui/show");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .timeout(crate::http_client::INTERACTIVE_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn extract_media_http(url: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/media/extract");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn download_media_http(url: &str, format_id: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/media/download");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "url": url, "format_id": format_id }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn playlist_download_http(url: &str, save_dir: &str, max_items: u32) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/media/playlist-download");
    let mut body = serde_json::json!({ "url": url, "max_items": max_items });
    if !save_dir.is_empty() {
        body["save_dir"] = serde_json::Value::String(save_dir.to_owned());
    }
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn parse_cookies_http(text: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/media/cookies/parse");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_site_rules_http() -> Result<String, String> {
    get_json_http("site-rules")
}

fn add_site_rule_http(pattern: &str, rule_type: &str, value: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/site-rules");
    let mut body = serde_json::json!({ "pattern": pattern, "rule_type": rule_type });
    if !value.is_empty() {
        body["value"] = serde_json::Value::String(value.to_owned());
    }
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn update_site_rule_http(
    id: &str,
    pattern: &str,
    rule_type: &str,
    value: &str,
) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/site-rules/{id}");
    let mut body = serde_json::json!({});
    if !pattern.is_empty() {
        body["pattern"] = serde_json::Value::String(pattern.to_owned());
    }
    if !rule_type.is_empty() {
        body["rule_type"] = serde_json::Value::String(rule_type.to_owned());
    }
    if !value.is_empty() {
        body["value"] = serde_json::Value::String(value.to_owned());
    }
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .put(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_download_audit_http(id: &str) -> Result<String, String> {
    get_json_http(&format!("downloads/{id}/audit"))
}

fn set_torrent_location_http(id: &str, location: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/location");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "location": location }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn list_rss_http() -> Result<String, String> {
    get_json_http("rss/feeds")
}

fn delete_rss_feed_http(id: &str) -> Result<(), String> {
    delete_json_http(&format!("rss/feeds/{id}"), "{}").map(|_| ())
}

fn set_torrent_queue_http(id: &str, position: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/queue");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "position": position }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_stats_http() -> Result<String, String> {
    get_json_http("stats")
}

fn batch_downloads_http(ids: &[String], action: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/batch");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "ids": ids, "action": action }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn batch_torrents_http(ids: &[String], action: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/batch");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "ids": ids, "action": action }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn add_rss_http(url: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/rss/feeds");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "url": url }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn set_torrent_trackers_http(id: &str, trackers: &[String]) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/trackers");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .put(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "trackers": trackers }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_download_diagnostic_http(id: &str) -> Result<String, String> {
    get_json_http(&format!("downloads/{id}/diagnostic"))
}

fn get_downloads_json_http() -> Result<String, String> {
    get_json_http("downloads")
}

fn delete_site_rule_http(id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/site-rules/{id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn export_cookies_http(text: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/media/cookies/export");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "text": text }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_merge_jobs_http() -> Result<String, String> {
    get_json_http("media/merge-jobs")
}

fn ffmpeg_status_http() -> Result<String, String> {
    get_json_http("media/ffmpeg")
}

fn ytdlp_status_http() -> Result<String, String> {
    get_json_http("media/ytdlp")
}

fn split_patterns(text: &str) -> Vec<String> {
    text.split([',', '\n', ' '])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn grab_links_http(
    url: &str,
    extensions: &[String],
    depth: u8,
    exclude: &[String],
    include: &[String],
    stay_below: bool,
    external_levels: u8,
) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/grab");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({
                "url": url,
                "extensions": extensions,
                "depth": depth,
                "exclude": exclude,
                "include": include,
                "stay_below": stay_below,
                "external_levels": external_levels,
            }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_queues_http() -> Result<String, String> {
    get_json_http("queues")
}

fn delete_site_login_http(id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/site-logins/{id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(resp
                .text()
                .await
                .unwrap_or_else(|_| "login could not be deleted".into()))
        }
    })
}

fn delete_queue_http(id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/queues/{id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(resp
                .text()
                .await
                .unwrap_or_else(|_| "queue could not be deleted".into()))
        }
    })
}

fn update_queue_http(id: &str, patch_json: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/queues/{id}");
    let body: serde_json::Value =
        serde_json::from_str(patch_json).map_err(|e| format!("bad patch json: {e}"))?;
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .patch(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn get_categories_http() -> Result<String, String> {
    get_json_http("categories")
}

fn add_category_http(name: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/categories");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "name": name }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn set_torrent_category_http(id: &str, category: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/category");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .patch(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "category": category }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            Ok(String::new())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn set_download_description_http(id: &str, description: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/{id}/description");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .patch(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "description": description }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        if status.is_success() {
            Ok(String::new())
        } else {
            let text = resp.text().await.unwrap_or_default();
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn privacy_check_torrent_http(id: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/privacy-check");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .get(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn set_setting_http(key: &str, value: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/settings");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ key: value }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

// ── FQT-081/082/084/085: Privacy, proxies, webhooks helpers ──

fn get_privacy_dashboard_http() -> Result<String, String> {
    get_json_http("privacy/dashboard")
}
fn get_anonymity_score_http() -> Result<String, String> {
    get_json_http("privacy/anonymity-score")
}
fn export_audit_http() -> Result<String, String> {
    get_json_http("audit/export")
}
fn get_proxies_json_http() -> Result<String, String> {
    get_json_http("proxies")
}
fn get_health_json_http() -> Result<String, String> {
    get_json_http("health")
}

fn add_proxy_http(json: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/proxies");
    let body: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("bad json: {e}"))?;
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn delete_proxy_http(id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/proxies/{id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn test_proxy_http(id: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/proxies/{id}/test");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn assign_torrent_proxy_http(torrent_id: &str, proxy_id: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{torrent_id}/proxy");
    let body =
        serde_json::json!({ "proxy_id": if proxy_id.is_empty() { None } else { Some(proxy_id) } });
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn get_webhooks_json_http() -> Result<String, String> {
    get_json_http("webhooks")
}

fn add_webhook_http(url: &str, events: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/webhooks");
    let body = serde_json::json!({ "url": url, "events": events });
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

// ── FQT-102: RSS rules helpers ────────────────────────────────────────

fn get_rss_rules_json_http() -> Result<String, String> {
    get_json_http("rss/rules")
}

fn add_rss_rule_http(
    feed_id: &str,
    name: &str,
    pattern: &str,
    action: &str,
) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/rss/rules");
    let body = serde_json::json!({ "feed_id": feed_id, "name": name, "match_pattern": pattern, "action": action });
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn delete_rss_rule_http(id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/rss/rules/{id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .delete(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn open_folder_os(path: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("open_folder: {e}"))
}

/// Console helpers must not flash a window (see the GUI-subsystem note in main.rs).
fn quiet(cmd: &mut std::process::Command) -> &mut std::process::Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    cmd
}

fn open_file_os(path: &str) -> Result<(), String> {
    quiet(std::process::Command::new("cmd").args(["/c", "start", "", path]))
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("open_file: {e}"))
}

fn ferric_host_status() -> Result<String, String> {
    let output = quiet(std::process::Command::new("ferric-host").arg("--status"))
        .output()
        .map_err(|e| format!("ferric-host not found: {e}"))?;
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    if stderr.contains("\"registered\"") {
        Ok(stderr.trim().to_string())
    } else {
        Err(format!("ferric-host status failed: {stderr}"))
    }
}

fn move_to_queue_http(id: &str, queue_id: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/{id}/queue");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "queue_id": queue_id }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn speed_limit_http(id: &str, bps: i64) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/{id}/speed-limit");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "bps": bps }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn global_speed_limit_http(bps: i64) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/settings");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "bandwidth_limit_bps": bps.to_string() }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn refresh_url_http(id: &str, new_url: &str) -> Result<(), String> {
    refresh_url_with_proxy_http(id, new_url, "")
}

fn refresh_url_with_proxy_http(id: &str, new_url: &str, proxy_url: &str) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let api_url = format!("http://127.0.0.1:{port}/downloads/{id}/refresh-url");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&api_url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "url": new_url, "proxy_url": proxy_url }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn set_priority_http(id: &str, priority: i32) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/downloads/{id}/priority");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "priority": priority }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(format!("daemon returned {}", resp.status()))
        }
    })
}

fn probe_torrent_http(input: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/probe");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "input": input }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn import_qbittorrent_http(directory: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/import/qbittorrent");
    let body = if directory.is_empty() {
        serde_json::json!({})
    } else {
        serde_json::json!({ "directory": directory })
    };
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            // Scans and parses an entire BT_backup directory server-side; a large
            // library outruns the client default.
            .timeout(crate::http_client::LONG_REQUEST_TIMEOUT)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| e.to_string())?;
        if !status.is_success() {
            return Err(format!("daemon returned {status}: {text}"));
        }
        Ok(text)
    })
}

fn stage_torrent_http(probe_json: &str, save_root: &str) -> Result<String, String> {
    let probe: serde_json::Value =
        serde_json::from_str(probe_json).map_err(|e| format!("bad probe result: {e}"))?;
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents");
    let mut body = serde_json::json!({
        "probe": probe,
        "policy": ferric_common::torrent::TorrentPolicy::default(),
    });
    if !save_root.is_empty() {
        body["save_root"] = serde_json::Value::String(save_root.to_owned());
    }
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

/// Torrent start/pause/resume all return the updated `TorrentDetail` on success,
/// or (critically for the privacy gate) the daemon's actual refusal reason on
/// failure — read the response body in both cases so a gate/kill-switch
/// rejection reaches the user's error dialog verbatim, not a generic status code.
fn torrent_action_http(id: &str, action: &str) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/{action}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .post(&url)
            .header("X-Ferric-Auth", &token)
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

fn delete_torrent_http(id: &str, delete_files: bool) -> Result<(), String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}?delete_files={delete_files}");
    let resp = crate::http_client::RUNTIME
        .block_on(async {
            crate::http_client::HTTP_CLIENT
                .clone()
                .delete(&url)
                .header("X-Ferric-Auth", &token)
                .send()
                .await
        })
        .map_err(|e| format!("daemon unreachable: {e}"))?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("daemon returned {}", resp.status()))
    }
}

fn update_torrent_file_http(
    id: &str,
    file_id: &str,
    selected: bool,
    priority: i32,
) -> Result<String, String> {
    let (port, token) = read_lock_file()?;
    let url = format!("http://127.0.0.1:{port}/torrents/{id}/files/{file_id}");
    crate::http_client::RUNTIME.block_on(async {
        let resp = crate::http_client::HTTP_CLIENT
            .clone()
            .patch(&url)
            .header("X-Ferric-Auth", &token)
            .json(&serde_json::json!({ "selected": selected, "priority": priority }))
            .send()
            .await
            .map_err(|e| format!("daemon unreachable: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if status.is_success() {
            Ok(text)
        } else {
            Err(format!("daemon returned {status}: {text}"))
        }
    })
}

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[qml_element]
        type CommandBus = super::CommandBusRust;

        #[qsignal]
        #[cxx_name = "commandFinished"]
        fn command_finished(self: Pin<&mut CommandBus>, request_id: QString, result: QString);
        #[qinvokable]
        fn request(
            self: Pin<&mut CommandBus>,
            request_id: &QString,
            operation: &QString,
            args_json: &QString,
        );

        #[qinvokable]
        fn pause_download(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn resume_download(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn play_completion_sound(self: &CommandBus);
        #[qinvokable]
        fn play_error_sound(self: &CommandBus);
        #[qinvokable]
        fn stop_download(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn delete_download(self: &CommandBus, id: &QString, with_file: bool) -> QString;
        #[qinvokable]
        fn add_download(
            self: &CommandBus,
            url: &QString,
            filename: &QString,
            save_dir: &QString,
            cookies: &QString,
            description: &QString,
        ) -> QString;
        #[qinvokable]
        fn pause_all_downloads(self: &CommandBus) -> QString;
        #[qinvokable]
        fn resume_all_downloads(self: &CommandBus) -> QString;
        #[qinvokable]
        fn clear_completed_downloads(self: &CommandBus) -> QString;
        #[qinvokable]
        fn get_download_json(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn get_chunks_json(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn get_diagnostic_json(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn open_folder(self: &CommandBus, path: &QString) -> QString;
        #[qinvokable]
        fn open_file(self: &CommandBus, path: &QString) -> QString;
        #[qinvokable]
        fn write_text_file(self: &CommandBus, path: &QString, content: &QString) -> QString;
        /// "shutdown" starts the OS shutdown with a 60 s delay; "abort" cancels it. Returns "" or an error.
        #[qinvokable]
        fn system_power(self: &CommandBus, action: &QString) -> QString;
        /// Export file content for `format` ("text" or "ef2"); `ids_json` = JSON array of ids, or empty for all.
        #[qinvokable]
        fn build_list_export(self: &CommandBus, format: &QString, ids_json: &QString) -> QString;
        /// URLs (one per line) found in an exported list file, or {"error": ...}.
        #[qinvokable]
        fn read_list_file(self: &CommandBus, path: &QString) -> QString;
        #[qinvokable]
        fn set_speed_limit(self: &CommandBus, id: &QString, bps: i64) -> QString;
        #[qinvokable]
        fn set_global_speed_limit(self: &CommandBus, bps: i64) -> QString;
        #[qinvokable]
        fn refresh_download_url(self: &CommandBus, id: &QString, url: &QString) -> QString;
        #[qinvokable]
        fn set_download_priority(self: &CommandBus, id: &QString, priority: i32) -> QString;

        /// Returns `TorrentProbeResponse` JSON, or `{"error": "..."}` on failure.
        #[qinvokable]
        fn probe_torrent(self: &CommandBus, input: &QString) -> QString;
        /// Scans a qBittorrent BT_backup directory (empty string = platform
        /// default) and imports every torrent found, hash-verifying against
        /// existing files rather than trusting qBittorrent's own resume state.
        /// Returns `{"found": N, "imported": N, "items": [...]}` JSON.
        #[qinvokable]
        fn import_qbittorrent(self: &CommandBus, directory: &QString) -> QString;
        /// `probe_json` is the exact string previously returned by `probe_torrent`.
        /// Returns `TorrentDetail` JSON (staged or, for a duplicate infohash, the
        /// existing record - same shape either way), or `{"error": "..."}`.
        #[qinvokable]
        fn stage_torrent(self: &CommandBus, probe_json: &QString, save_root: &QString) -> QString;
        /// Returns `TorrentDetail` JSON on success, or `{"error": "..."}` - a
        /// privacy-gate/kill-switch refusal's exact reason lands in `error`.
        #[qinvokable]
        fn start_torrent(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn pause_torrent(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn resume_torrent(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn delete_torrent(self: &CommandBus, id: &QString, delete_files: bool) -> QString;
        /// Returns `TorrentDetail` JSON, or `{"error": "..."}` on failure.
        #[qinvokable]
        fn get_torrent_json(self: &CommandBus, id: &QString) -> QString;
        /// Returns the updated `TorrentDetail` JSON, or `{"error": "..."}`.
        #[qinvokable]
        fn set_torrent_file_selection(
            self: &CommandBus,
            id: &QString,
            file_id: &QString,
            selected: bool,
            priority: i32,
        ) -> QString;
        /// Returns `Vec<LivePeerView>` JSON, or `{"error": "..."}`.
        #[qinvokable]
        fn get_torrent_peers_json(self: &CommandBus, id: &QString) -> QString;
        /// Returns the refreshed `TorrentDetail` JSON, or `{"error": "..."}`.
        #[qinvokable]
        fn sync_torrent(self: &CommandBus, id: &QString) -> QString;

        /// Returns all settings as a JSON object (`{key: value, ...}`).
        #[qinvokable]
        fn get_settings_json(self: &CommandBus) -> QString;
        /// Persists a single settings key-value pair. Returns error string or empty on success.
        #[qinvokable]
        fn set_setting(self: &CommandBus, key: &QString, value: &QString) -> QString;
        /// Updates a torrent's per-torrent policy. `policy_json` is a JSON object
        /// with TorrentPolicy fields. Returns updated TorrentDetail JSON, or {"error": "..."}.
        #[qinvokable]
        fn update_torrent_policy(self: &CommandBus, id: &QString, policy_json: &QString)
            -> QString;
        /// Runs privacy check for a torrent. Returns PrivacyCheckResult JSON,
        /// or {"error": "..."}.
        #[qinvokable]
        fn privacy_check_torrent(self: &CommandBus, id: &QString) -> QString;
        /// Returns all categories as a JSON array.
        #[qinvokable]
        fn get_categories_json(self: &CommandBus) -> QString;
        /// Creates a new category. Returns the created category JSON, or {"error": "..."}.
        #[qinvokable]
        fn add_category(self: &CommandBus, name: &QString) -> QString;
        /// Sets the description/note on a download. Returns {} on success, or {"error": "..."}.
        #[qinvokable]
        fn set_download_description(
            self: &CommandBus,
            id: &QString,
            description: &QString,
        ) -> QString;
        /// Returns all queues as a JSON array.
        #[qinvokable]
        fn get_queues_json(self: &CommandBus) -> QString;
        /// Updates a queue. `patch_json` is a JSON object with QueuePatchBody fields.
        /// Returns error string or empty on success.
        #[qinvokable]
        fn update_queue(self: &CommandBus, id: &QString, patch_json: &QString) -> QString;
        /// Grabs links from a URL. Returns JSON array of link strings, or {"error": "..."}.
        #[qinvokable]
        fn grab_links(
            self: &CommandBus,
            url: &QString,
            extensions: &QString,
            depth: i32,
        ) -> QString;
        /// Extracts media formats from a URL. Returns ExtractResult JSON, or {"error": "..."}.
        #[qinvokable]
        fn extract_media(self: &CommandBus, url: &QString) -> QString;
        /// Downloads media with a specific format. Returns MediaDownloadResponse JSON, or {"error": "..."}.
        #[qinvokable]
        fn download_media(self: &CommandBus, url: &QString, format_id: &QString) -> QString;
        /// Returns yt-dlp engine status JSON, or {"error": "..."}.
        #[qinvokable]
        fn ytdlp_status(self: &CommandBus) -> QString;
        /// Downloads a playlist. Returns { queued, ids, titles } JSON, or {"error": "..."}.
        #[qinvokable]
        fn playlist_download(
            self: &CommandBus,
            url: &QString,
            save_dir: &QString,
            max_items: i32,
        ) -> QString;
        /// Returns merge jobs as a JSON array.
        #[qinvokable]
        fn get_merge_jobs_json(self: &CommandBus) -> QString;
        /// Returns ffmpeg status JSON, or {"error": "..."}.
        #[qinvokable]
        fn ffmpeg_status(self: &CommandBus) -> QString;
        /// Parses Netscape cookies text. Returns { count, header_example }, or {"error": "..."}.
        #[qinvokable]
        fn parse_cookies(self: &CommandBus, text: &QString) -> QString;
        /// Exports cookies back to Netscape text. Returns { text, count }, or {"error": "..."}.
        #[qinvokable]
        fn export_cookies(self: &CommandBus, text: &QString) -> QString;
        /// Returns all site rules as a JSON array.
        #[qinvokable]
        fn get_site_rules_json(self: &CommandBus) -> QString;
        /// Adds a site rule. Returns created rule JSON, or {"error": "..."}.
        #[qinvokable]
        fn add_site_rule(
            self: &CommandBus,
            pattern: &QString,
            rule_type: &QString,
            value: &QString,
        ) -> QString;
        /// Updates a site rule. Returns updated rule JSON, or {"error": "..."}.
        #[qinvokable]
        fn update_site_rule(
            self: &CommandBus,
            id: &QString,
            pattern: &QString,
            rule_type: &QString,
            value: &QString,
        ) -> QString;
        /// Deletes a site rule. Returns error string or empty on success.
        #[qinvokable]
        fn delete_site_rule(self: &CommandBus, id: &QString) -> QString;
        /// Returns all downloads as a JSON array.
        #[qinvokable]
        fn get_downloads_json(self: &CommandBus) -> QString;
        /// Returns audit events for a download as a JSON array.
        #[qinvokable]
        fn get_download_audit(self: &CommandBus, id: &QString) -> QString;
        /// Returns diagnostic info for a download as JSON.
        #[qinvokable]
        fn get_download_diagnostic(self: &CommandBus, id: &QString) -> QString;
        /// Sets the save location for a torrent. Returns updated TorrentDetail JSON.
        #[qinvokable]
        fn set_torrent_location(self: &CommandBus, id: &QString, location: &QString) -> QString;
        /// Sets trackers for a torrent. Returns updated TorrentDetail JSON.
        #[qinvokable]
        fn set_torrent_trackers(self: &CommandBus, id: &QString, trackers: &QString) -> QString;
        /// Generic torrent action (recheck, reannounce, etc). Returns TorrentDetail JSON.
        #[qinvokable]
        fn torrent_action(self: &CommandBus, id: &QString, action: &QString) -> QString;
        /// Returns all RSS feeds as a JSON array.
        #[qinvokable]
        fn get_rss_feeds_json(self: &CommandBus) -> QString;
        /// Adds an RSS feed. Returns created feed JSON, or {"error": "..."}.
        #[qinvokable]
        fn add_rss_feed(self: &CommandBus, url: &QString) -> QString;
        /// Returns global transfer stats JSON.
        #[qinvokable]
        fn get_stats(self: &CommandBus) -> QString;
        #[qinvokable]
        fn get_health_json(self: &CommandBus) -> QString;
        #[qinvokable]
        fn get_ferric_host_status(self: &CommandBus) -> QString;
        /// Batch pause/resume/delete downloads. Returns { succeeded, failed } JSON.
        #[qinvokable]
        fn batch_downloads(self: &CommandBus, ids: &QString, action: &QString) -> QString;
        /// Batch pause/start torrents. Returns { succeeded, failed } JSON.
        #[qinvokable]
        fn batch_torrents(self: &CommandBus, ids: &QString, action: &QString) -> QString;
        /// Sets torrent queue position (up/down/top/bottom). Returns TorrentDetail JSON.
        #[qinvokable]
        fn set_torrent_queue_position(
            self: &CommandBus,
            id: &QString,
            position: &QString,
        ) -> QString;

        // ── Torrent tags ─────────────────────────────────────────────────
        #[qinvokable]
        fn get_torrent_tags(self: &CommandBus, id: &QString) -> QString;
        #[qinvokable]
        fn set_torrent_tags(self: &CommandBus, id: &QString, tags: &QString) -> QString;
        #[qinvokable]
        fn set_torrent_category(self: &CommandBus, id: &QString, category: &QString) -> QString;
        #[qinvokable]
        fn add_torrent_tag(self: &CommandBus, id: &QString, tag: &QString) -> QString;
        #[qinvokable]
        fn remove_torrent_tag(self: &CommandBus, id: &QString, tag: &QString) -> QString;

        // ── FQT-081: Privacy / audit ─────────────────────────────────────
        /// Returns privacy dashboard JSON.
        #[qinvokable]
        fn get_privacy_dashboard(self: &CommandBus) -> QString;
        /// Returns anonymity score JSON.
        #[qinvokable]
        fn get_anonymity_score(self: &CommandBus) -> QString;
        /// Returns audit export JSON.
        #[qinvokable]
        fn export_audit(self: &CommandBus) -> QString;

        // ── FQT-082: Proxy manager ───────────────────────────────────────
        /// Returns all proxy configs as JSON array.
        #[qinvokable]
        fn get_proxies_json(self: &CommandBus) -> QString;
        /// Adds/updates a proxy config. `json` is ProxyConfigInput. Returns created record JSON.
        #[qinvokable]
        fn add_proxy(self: &CommandBus, json: &QString) -> QString;
        /// Deletes a proxy config. Returns error string or empty.
        #[qinvokable]
        fn delete_proxy(self: &CommandBus, id: &QString) -> QString;
        /// Tests proxy connectivity. Returns { ok, latency_ms, error } JSON.
        #[qinvokable]
        fn test_proxy(self: &CommandBus, id: &QString) -> QString;
        /// Assigns a proxy to a torrent. Empty proxy_id clears assignment. Returns torrent detail JSON.
        #[qinvokable]
        fn assign_torrent_proxy(
            self: &CommandBus,
            torrent_id: &QString,
            proxy_id: &QString,
        ) -> QString;

        // ── FQT-085: Webhooks ────────────────────────────────────────────
        /// Returns all webhooks as JSON array.
        #[qinvokable]
        fn get_webhooks_json(self: &CommandBus) -> QString;
        /// Adds a webhook. Returns created webhook JSON.
        #[qinvokable]
        fn add_webhook(self: &CommandBus, url: &QString, events: &QString) -> QString;

        // ── FQT-102: RSS rules ───────────────────────────────────────────
        /// Returns all RSS rules as JSON array.
        #[qinvokable]
        fn get_rss_rules_json(self: &CommandBus) -> QString;
        /// Adds an RSS rule. Returns created rule JSON.
        #[qinvokable]
        fn add_rss_rule(
            self: &CommandBus,
            feed_id: &QString,
            name: &QString,
            pattern: &QString,
            action: &QString,
        ) -> QString;
        /// Deletes an RSS rule.
        #[qinvokable]
        fn delete_rss_rule(self: &CommandBus, id: &QString) -> QString;
    }

    impl cxx_qt::Threading for CommandBus {}
}

#[derive(Default)]
pub struct CommandBusRust {}

impl qobject::CommandBus {
    pub fn request(
        self: Pin<&mut Self>,
        request_id: &QString,
        operation: &QString,
        args_json: &QString,
    ) {
        let qt_thread = self.qt_thread();
        let request_id = request_id.to_string();
        let operation = operation.to_string();
        let args_json = args_json.to_string();
        std::thread::spawn(move || {
            let result = dispatch_request(&operation, &args_json);
            let _ = qt_thread.queue(move |mut bus| {
                bus.as_mut().command_finished(
                    QString::from(request_id.as_str()),
                    QString::from(result.as_str()),
                );
            });
        });
    }

    /// The Windows "asterisk" system sound, for the "play a sound when downloads finish" option.
    pub fn play_completion_sound(&self) {
        #[cfg(windows)]
        // SAFETY: MessageBeep takes a plain flag value and no pointers.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONASTERISK,
            );
        }
    }

    /// The Windows "hand" system sound, for a download that failed.
    pub fn play_error_sound(&self) {
        #[cfg(windows)]
        // SAFETY: MessageBeep takes a plain flag value and no pointers.
        unsafe {
            windows_sys::Win32::System::Diagnostics::Debug::MessageBeep(
                windows_sys::Win32::UI::WindowsAndMessaging::MB_ICONHAND,
            );
        }
    }

    pub fn pause_download(&self, id: &QString) -> QString {
        to_qs(post_action(&id.to_string(), "pause"))
    }

    pub fn resume_download(&self, id: &QString) -> QString {
        to_qs(post_action(&id.to_string(), "resume"))
    }

    pub fn stop_download(&self, id: &QString) -> QString {
        to_qs(post_action(&id.to_string(), "cancel"))
    }

    pub fn delete_download(&self, id: &QString, with_file: bool) -> QString {
        to_qs(delete_action(&id.to_string(), with_file))
    }

    pub fn add_download(
        &self,
        url: &QString,
        filename: &QString,
        save_dir: &QString,
        cookies: &QString,
        description: &QString,
    ) -> QString {
        to_qs(add_download_http(
            &url.to_string(),
            &filename.to_string(),
            &save_dir.to_string(),
            &cookies.to_string(),
            &description.to_string(),
            false,
        ))
    }

    pub fn pause_all_downloads(&self) -> QString {
        to_qs(pause_all_http())
    }

    pub fn resume_all_downloads(&self) -> QString {
        to_qs(resume_all_http())
    }

    pub fn clear_completed_downloads(&self) -> QString {
        to_qs(clear_completed_http())
    }

    pub fn get_download_json(&self, id: &QString) -> QString {
        match get_json_http(&format!("downloads/{}", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_chunks_json(&self, id: &QString) -> QString {
        match get_json_http(&format!("downloads/{}/chunks", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_diagnostic_json(&self, id: &QString) -> QString {
        match get_json_http(&format!("downloads/{}/diagnostic", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn open_folder(&self, path: &QString) -> QString {
        to_qs(open_folder_os(&path.to_string()))
    }

    pub fn open_file(&self, path: &QString) -> QString {
        to_qs(open_file_os(&path.to_string()))
    }

    pub fn system_power(&self, action: &QString) -> QString {
        let Some(args) = power_args(&action.to_string()) else {
            return QString::from("unknown power action");
        };
        #[cfg(windows)]
        {
            match quiet(std::process::Command::new("shutdown.exe").args(args)).spawn() {
                Ok(_) => QString::from(""),
                Err(e) => QString::from(&e.to_string()),
            }
        }
        #[cfg(not(windows))]
        {
            let _ = args;
            QString::from("shutting down the computer is only supported on Windows")
        }
    }

    pub fn build_list_export(&self, format: &QString, ids_json: &QString) -> QString {
        let result = (|| -> Result<String, String> {
            let rows: Vec<serde_json::Value> =
                serde_json::from_str(&get_json_http("downloads")?).map_err(|e| e.to_string())?;
            let only: Vec<String> = serde_json::from_str(&ids_json.to_string()).unwrap_or_default();
            let entries: Vec<crate::list_io::Entry> = rows
                .iter()
                .filter(|r| {
                    only.is_empty()
                        || r["id"]
                            .as_str()
                            .is_some_and(|id| only.iter().any(|o| o == id))
                })
                .filter_map(|r| {
                    Some(crate::list_io::Entry {
                        url: r["url"].as_str()?.to_string(),
                        referrer: r["referrer"].as_str().map(str::to_string),
                    })
                })
                .collect();
            Ok(if format.to_string() == "ef2" {
                crate::list_io::to_ef2(&entries)
            } else {
                crate::list_io::to_text(&entries)
            })
        })();
        match result {
            Ok(text) => QString::from(&text),
            Err(e) => QString::from(&serde_json::json!({ "error": e }).to_string()),
        }
    }

    pub fn read_list_file(&self, path: &QString) -> QString {
        match std::fs::read(path.to_string()) {
            Ok(bytes) => {
                let urls: Vec<String> = crate::list_io::parse(&String::from_utf8_lossy(&bytes))
                    .into_iter()
                    .map(|e| e.url)
                    .collect();
                QString::from(&urls.join(
                    "
",
                ))
            }
            Err(e) => QString::from(&serde_json::json!({ "error": e.to_string() }).to_string()),
        }
    }

    pub fn write_text_file(&self, path: &QString, content: &QString) -> QString {
        match std::fs::write(path.to_string(), content.to_string()) {
            Ok(()) => QString::from(""),
            Err(e) => QString::from(&e.to_string()),
        }
    }

    pub fn set_speed_limit(&self, id: &QString, bps: i64) -> QString {
        to_qs(speed_limit_http(&id.to_string(), bps))
    }

    pub fn set_global_speed_limit(&self, bps: i64) -> QString {
        to_qs(global_speed_limit_http(bps))
    }

    pub fn refresh_download_url(&self, id: &QString, url: &QString) -> QString {
        to_qs(refresh_url_http(&id.to_string(), &url.to_string()))
    }

    pub fn set_download_priority(&self, id: &QString, priority: i32) -> QString {
        to_qs(set_priority_http(&id.to_string(), priority))
    }

    pub fn probe_torrent(&self, input: &QString) -> QString {
        match probe_torrent_http(&input.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn import_qbittorrent(&self, directory: &QString) -> QString {
        match import_qbittorrent_http(&directory.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn stage_torrent(&self, probe_json: &QString, save_root: &QString) -> QString {
        match stage_torrent_http(&probe_json.to_string(), &save_root.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn start_torrent(&self, id: &QString) -> QString {
        match torrent_action_http(&id.to_string(), "start") {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn pause_torrent(&self, id: &QString) -> QString {
        match torrent_action_http(&id.to_string(), "pause") {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn resume_torrent(&self, id: &QString) -> QString {
        match torrent_action_http(&id.to_string(), "resume") {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn delete_torrent(&self, id: &QString, delete_files: bool) -> QString {
        to_qs(delete_torrent_http(&id.to_string(), delete_files))
    }

    pub fn get_torrent_json(&self, id: &QString) -> QString {
        match get_json_http(&format!("torrents/{}", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_file_selection(
        &self,
        id: &QString,
        file_id: &QString,
        selected: bool,
        priority: i32,
    ) -> QString {
        match update_torrent_file_http(&id.to_string(), &file_id.to_string(), selected, priority) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_torrent_peers_json(&self, id: &QString) -> QString {
        match get_json_http(&format!("torrents/{}/peers", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn sync_torrent(&self, id: &QString) -> QString {
        match torrent_action_http(&id.to_string(), "sync") {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_settings_json(&self) -> QString {
        match get_settings_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_setting(&self, key: &QString, value: &QString) -> QString {
        to_qs(set_setting_http(&key.to_string(), &value.to_string()))
    }

    pub fn update_torrent_policy(&self, id: &QString, policy_json: &QString) -> QString {
        match update_torrent_policy_http(&id.to_string(), &policy_json.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn privacy_check_torrent(&self, id: &QString) -> QString {
        match privacy_check_torrent_http(&id.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_categories_json(&self) -> QString {
        match get_categories_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_category(&self, name: &QString) -> QString {
        match add_category_http(&name.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_download_description(&self, id: &QString, description: &QString) -> QString {
        match set_download_description_http(&id.to_string(), &description.to_string()) {
            Ok(_) => QString::from("{}"),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_queues_json(&self) -> QString {
        match get_queues_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn update_queue(&self, id: &QString, patch_json: &QString) -> QString {
        to_qs(update_queue_http(&id.to_string(), &patch_json.to_string()))
    }

    pub fn grab_links(&self, url: &QString, extensions: &QString, depth: i32) -> QString {
        let exts: Vec<String> = extensions
            .to_string()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match grab_links_http(&url.to_string(), &exts, depth as u8, &[], &[], false, 0) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn extract_media(&self, url: &QString) -> QString {
        match extract_media_http(&url.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn download_media(&self, url: &QString, format_id: &QString) -> QString {
        match download_media_http(&url.to_string(), &format_id.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn ytdlp_status(&self) -> QString {
        match ytdlp_status_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn playlist_download(&self, url: &QString, save_dir: &QString, max_items: i32) -> QString {
        match playlist_download_http(&url.to_string(), &save_dir.to_string(), max_items as u32) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_merge_jobs_json(&self) -> QString {
        match get_merge_jobs_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn ffmpeg_status(&self) -> QString {
        match ffmpeg_status_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn parse_cookies(&self, text: &QString) -> QString {
        match parse_cookies_http(&text.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn export_cookies(&self, text: &QString) -> QString {
        match export_cookies_http(&text.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_site_rules_json(&self) -> QString {
        match get_site_rules_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_site_rule(
        &self,
        pattern: &QString,
        rule_type: &QString,
        value: &QString,
    ) -> QString {
        match add_site_rule_http(
            &pattern.to_string(),
            &rule_type.to_string(),
            &value.to_string(),
        ) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn update_site_rule(
        &self,
        id: &QString,
        pattern: &QString,
        rule_type: &QString,
        value: &QString,
    ) -> QString {
        match update_site_rule_http(
            &id.to_string(),
            &pattern.to_string(),
            &rule_type.to_string(),
            &value.to_string(),
        ) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn delete_site_rule(&self, id: &QString) -> QString {
        to_qs(delete_site_rule_http(&id.to_string()))
    }

    pub fn get_downloads_json(&self) -> QString {
        match get_downloads_json_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_download_audit(&self, id: &QString) -> QString {
        match get_download_audit_http(&id.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_download_diagnostic(&self, id: &QString) -> QString {
        match get_download_diagnostic_http(&id.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_location(&self, id: &QString, location: &QString) -> QString {
        match set_torrent_location_http(&id.to_string(), &location.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_trackers(&self, id: &QString, trackers: &QString) -> QString {
        let list: Vec<String> = trackers
            .to_string()
            .lines()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match set_torrent_trackers_http(&id.to_string(), &list) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn torrent_action(&self, id: &QString, action: &QString) -> QString {
        match torrent_action_http(&id.to_string(), &action.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_rss_feeds_json(&self) -> QString {
        match list_rss_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_rss_feed(&self, url: &QString) -> QString {
        match add_rss_http(&url.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn delete_rss_feed(&self, id: &QString) -> QString {
        to_qs(delete_rss_feed_http(&id.to_string()))
    }

    pub fn get_stats(&self) -> QString {
        match get_stats_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_health_json(&self) -> QString {
        match get_health_json_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_ferric_host_status(&self) -> QString {
        match ferric_host_status() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"registered\":false,\"error\":\"{e}\"}}")),
        }
    }

    pub fn batch_downloads(&self, ids: &QString, action: &QString) -> QString {
        let list: Vec<String> = ids
            .to_string()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match batch_downloads_http(&list, &action.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn batch_torrents(&self, ids: &QString, action: &QString) -> QString {
        let list: Vec<String> = ids
            .to_string()
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        match batch_torrents_http(&list, &action.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_queue_position(&self, id: &QString, position: &QString) -> QString {
        match set_torrent_queue_http(&id.to_string(), &position.to_string()) {
            Ok(json) => QString::from(&json),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_torrent_tags(&self, id: &QString) -> QString {
        match get_json_http(&format!("torrents/{}/tags", id)) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_tags(&self, id: &QString, tags: &QString) -> QString {
        let url = format!("torrents/{}/tags", id);
        match put_json_http(&url, &tags.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn set_torrent_category(&self, id: &QString, category: &QString) -> QString {
        match set_torrent_category_http(&id.to_string(), &category.to_string()) {
            Ok(_) => QString::from("{}"),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_torrent_tag(&self, id: &QString, tag: &QString) -> QString {
        let url = format!("torrents/{}/tags", id);
        let body = serde_json::json!({ "tag": tag.to_string() }).to_string();
        match post_json_http(&url, &body) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn remove_torrent_tag(&self, id: &QString, tag: &QString) -> QString {
        let url = format!("torrents/{}/tags", id);
        let body = serde_json::json!({ "tag": tag.to_string() }).to_string();
        match delete_json_http(&url, &body) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    // ── FQT-081: Privacy / audit ─────────────────────────────────────────

    pub fn get_privacy_dashboard(&self) -> QString {
        match get_privacy_dashboard_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn get_anonymity_score(&self) -> QString {
        match get_anonymity_score_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn export_audit(&self) -> QString {
        match export_audit_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    // ── FQT-082: Proxy manager ───────────────────────────────────────────

    pub fn get_proxies_json(&self) -> QString {
        match get_proxies_json_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_proxy(&self, json: &QString) -> QString {
        match add_proxy_http(&json.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn delete_proxy(&self, id: &QString) -> QString {
        match delete_proxy_http(&id.to_string()) {
            Ok(()) => QString::from(""),
            Err(e) => QString::from(&e),
        }
    }

    pub fn test_proxy(&self, id: &QString) -> QString {
        match test_proxy_http(&id.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn assign_torrent_proxy(&self, torrent_id: &QString, proxy_id: &QString) -> QString {
        match assign_torrent_proxy_http(&torrent_id.to_string(), &proxy_id.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    // ── FQT-085: Webhooks ────────────────────────────────────────────────

    pub fn get_webhooks_json(&self) -> QString {
        match get_webhooks_json_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_webhook(&self, url: &QString, events: &QString) -> QString {
        match add_webhook_http(&url.to_string(), &events.to_string()) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    // ── FQT-102: RSS rules ─────────────────────────────────────────────

    pub fn get_rss_rules_json(&self) -> QString {
        match get_rss_rules_json_http() {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn add_rss_rule(
        &self,
        feed_id: &QString,
        name: &QString,
        pattern: &QString,
        action: &QString,
    ) -> QString {
        match add_rss_rule_http(
            &feed_id.to_string(),
            &name.to_string(),
            &pattern.to_string(),
            &action.to_string(),
        ) {
            Ok(s) => QString::from(&s),
            Err(e) => QString::from(&format!("{{\"error\":\"{e}\"}}")),
        }
    }

    pub fn delete_rss_rule(&self, id: &QString) -> QString {
        match delete_rss_rule_http(&id.to_string()) {
            Ok(()) => QString::from(""),
            Err(e) => QString::from(&e),
        }
    }
}

fn dispatch_request(operation: &str, args_json: &str) -> String {
    let args: serde_json::Value =
        serde_json::from_str(args_json).unwrap_or_else(|_| serde_json::json!({}));
    match operation {
        "get_settings" => get_json_http("settings").unwrap_or_else(error_json),
        "get_categories" => get_json_http("categories").unwrap_or_else(error_json),
        "get_queues" => get_json_http("queues").unwrap_or_else(error_json),
        "get_stats" => get_json_http("stats").unwrap_or_else(error_json),
        "get_downloads" => get_json_http("downloads").unwrap_or_else(error_json),
        "grabber_projects" => get_json_http("grabber/projects").unwrap_or_else(error_json),
        "grabber_project_get" => {
            get_json_http(&format!("grabber/projects/{}", arg_str(&args, "id")))
                .unwrap_or_else(error_json)
        }
        "grabber_project_save" => post_json_http(
            "grabber/projects",
            &serde_json::json!({
                "name": arg_str(&args, "name"),
                "is_template": arg_bool(&args, "is_template"),
                "config": args.get("config").cloned().unwrap_or(serde_json::Value::Null),
                "results": args.get("results").cloned().unwrap_or_else(|| serde_json::json!([])),
            })
            .to_string(),
        )
        .unwrap_or_else(error_json),
        "grabber_project_update" => {
            post_json_http(&format!("grabber/projects/{}/update", arg_str(&args, "id")), "{}")
                .unwrap_or_else(error_json)
        }
        "grabber_project_delete" => {
            delete_json_http(&format!("grabber/projects/{}", arg_str(&args, "id")), "")
                .unwrap_or_else(error_json)
        }
        "get_rss_feeds" => get_json_http("rss/feeds").unwrap_or_else(error_json),
        "get_rss_rules" => get_json_http("rss/rules").unwrap_or_else(error_json),
        "get_anonymity_score" => {
            get_json_http("privacy/anonymity-score").unwrap_or_else(error_json)
        }
        "get_privacy_dashboard" => get_json_http("privacy/dashboard").unwrap_or_else(error_json),
        "grab_links" => {
            let extensions: Vec<String> = arg_str(&args, "extensions")
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            grab_links_http(
                arg_str(&args, "url"),
                &extensions,
                arg_u32(&args, "depth").min(u32::from(u8::MAX)) as u8,
                &split_patterns(arg_str(&args, "exclude")),
                &split_patterns(arg_str(&args, "include")),
                arg_bool(&args, "stay_below"),
                arg_u32(&args, "external_levels").min(5) as u8,
            )
            .unwrap_or_else(error_json)
        }
        "extract_media" => extract_media_http(arg_str(&args, "url")).unwrap_or_else(error_json),
        "download_media" => download_media_http(arg_str(&args, "url"), arg_str(&args, "format_id"))
            .unwrap_or_else(error_json),
        "playlist_download" => playlist_download_http(
            arg_str(&args, "url"),
            arg_str(&args, "save_dir"),
            arg_u32(&args, "max_items"),
        )
        .unwrap_or_else(error_json),
        "probe_torrent" => probe_torrent_http(arg_str(&args, "input")).unwrap_or_else(error_json),
        "stage_torrent" => {
            stage_torrent_http(arg_str(&args, "probe_json"), arg_str(&args, "save_root"))
                .unwrap_or_else(error_json)
        }
        "add_download" => unit_result(add_download_http(
            arg_str(&args, "url"),
            arg_str(&args, "filename"),
            arg_str(&args, "save_dir"),
            arg_str(&args, "cookies"),
            arg_str(&args, "description"),
            arg_bool(&args, "hold"),
        )),
        "get_pending_captures" => get_json_http("captures/pending").unwrap_or_else(error_json),
        "get_zip_preview" => get_json_http(&format!("downloads/{}/zip-preview", arg_str(&args, "id")))
            .unwrap_or_else(error_json),
        "get_category_folders" => get_json_http("category-folders").unwrap_or_else(error_json),
        "set_category_folder" => unit_result(
            put_json_http(
                "category-folders",
                &serde_json::json!({ "name": arg_str(&args, "name"), "dir": arg_str(&args, "dir") })
                    .to_string(),
            )
            .map(|_| ()),
        ),
        "confirm_download" => {
            // Everything but the id is the confirm body (filename, save_dir, url, start, description, category).
            let mut body = args.clone();
            let id = arg_str(&args, "id").to_owned();
            if let Some(o) = body.as_object_mut() {
                o.remove("id");
            }
            unit_result(
                post_json_http(&format!("downloads/{id}/confirm"), &body.to_string()).map(|_| ()),
            )
        }
        "download_action" => {
            unit_result(post_action(arg_str(&args, "id"), arg_str(&args, "action")))
        }
        "delete_download" => unit_result(delete_action(
            arg_str(&args, "id"),
            arg_bool(&args, "with_file"),
        )),
        "torrent_action" => torrent_action_http(arg_str(&args, "id"), arg_str(&args, "action"))
            .unwrap_or_else(error_json),
        "sync_torrent" => {
            torrent_action_http(arg_str(&args, "id"), "sync").unwrap_or_else(error_json)
        }
        "add_torrent_tag" => post_json_http(
            &format!("torrents/{}/tags", arg_str(&args, "id")),
            &serde_json::json!({ "tag": arg_str(&args, "tag") }).to_string(),
        )
        .unwrap_or_else(error_json),
        "remove_torrent_tag" => delete_json_http(
            &format!(
                "torrents/{}/tags/{}",
                arg_str(&args, "id"),
                arg_str(&args, "tag")
            ),
            "{}",
        )
        .map(|_| "{}".to_owned())
        .unwrap_or_else(error_json),
        "delete_torrent" => unit_result(delete_torrent_http(
            arg_str(&args, "id"),
            arg_bool(&args, "delete_files"),
        )),
        "get_download" => {
            get_json_http(&format!("downloads/{}", arg_str(&args, "id"))).unwrap_or_else(error_json)
        }
        "get_download_chunks" => {
            get_json_http(&format!("downloads/{}/chunks", arg_str(&args, "id")))
                .unwrap_or_else(error_json)
        }
        "get_download_diagnostic" => {
            get_json_http(&format!("downloads/{}/diagnostic", arg_str(&args, "id")))
                .unwrap_or_else(error_json)
        }
        "get_download_audit" => {
            get_download_audit_http(arg_str(&args, "id")).unwrap_or_else(error_json)
        }
        "get_torrent" => {
            get_json_http(&format!("torrents/{}", arg_str(&args, "id"))).unwrap_or_else(error_json)
        }
        "get_torrent_peers" => get_json_http(&format!("torrents/{}/peers", arg_str(&args, "id")))
            .unwrap_or_else(error_json),
        "set_torrent_file" => update_torrent_file_http(
            arg_str(&args, "id"),
            arg_str(&args, "file_id"),
            arg_bool(&args, "selected"),
            arg_i32(&args, "priority"),
        )
        .unwrap_or_else(error_json),
        "add_category" => add_category_http(arg_str(&args, "name")).unwrap_or_else(error_json),
        "add_rss_feed" => add_rss_http(arg_str(&args, "url")).unwrap_or_else(error_json),
        "add_rss_rule" => add_rss_rule_http(
            arg_str(&args, "feed_id"),
            arg_str(&args, "name"),
            arg_str(&args, "pattern"),
            arg_str(&args, "action"),
        )
        .unwrap_or_else(error_json),
        "delete_rss_feed" => unit_result(delete_rss_feed_http(arg_str(&args, "id"))),
        "delete_rss_rule" => unit_result(delete_rss_rule_http(arg_str(&args, "id"))),
        "get_site_rules" => get_json_http("site-rules").unwrap_or_else(error_json),
        "add_site_rule" => add_site_rule_http(
            arg_str(&args, "pattern"),
            arg_str(&args, "rule_type"),
            arg_str(&args, "value"),
        )
        .unwrap_or_else(error_json),
        "update_site_rule" => update_site_rule_http(
            arg_str(&args, "id"),
            arg_str(&args, "pattern"),
            arg_str(&args, "rule_type"),
            arg_str(&args, "value"),
        )
        .unwrap_or_else(error_json),
        "delete_site_rule" => unit_result(delete_site_rule_http(arg_str(&args, "id"))),
        "create_queue" => post_json_http("queues", arg_str(&args, "body_json")).unwrap_or_else(error_json),
        "get_site_logins" => get_json_http("site-logins").unwrap_or_else(error_json),
        "save_site_login" => post_json_http("site-logins", arg_str(&args, "body_json")).unwrap_or_else(error_json),
        "delete_site_login" => unit_result(delete_site_login_http(arg_str(&args, "id"))),
        "delete_queue" => unit_result(delete_queue_http(arg_str(&args, "id"))),
        "queue_action" => unit_result(
            post_json_http(
                &format!("queues/{}/{}", arg_str(&args, "id"), arg_str(&args, "action")),
                "{}",
            )
            .map(|_| ()),
        ),
        "update_queue" => unit_result(update_queue_http(
            arg_str(&args, "id"),
            arg_str(&args, "patch_json"),
        )),
        "get_merge_jobs" => get_json_http("media/merge-jobs").unwrap_or_else(error_json),
        "update_torrent_policy" => {
            update_torrent_policy_http(arg_str(&args, "id"), arg_str(&args, "policy_json"))
                .unwrap_or_else(error_json)
        }
        "privacy_check_torrent" => {
            privacy_check_torrent_http(arg_str(&args, "id")).unwrap_or_else(error_json)
        }
        "set_torrent_location" => {
            set_torrent_location_http(arg_str(&args, "id"), arg_str(&args, "location"))
                .unwrap_or_else(error_json)
        }
        "set_torrent_queue" => {
            set_torrent_queue_http(arg_str(&args, "id"), arg_str(&args, "position"))
                .unwrap_or_else(error_json)
        }
        "set_download_description" => {
            set_download_description_http(arg_str(&args, "id"), arg_str(&args, "description"))
                .unwrap_or_else(error_json)
        }
        "move_download_to_queue" => unit_result(move_to_queue_http(
            arg_str(&args, "id"),
            arg_str(&args, "queue_id"),
        )),
        "set_download_speed_limit" => unit_result(speed_limit_http(
            arg_str(&args, "id"),
            arg_i64(&args, "bps"),
        )),
        "set_global_speed_limit" => unit_result(global_speed_limit_http(arg_i64(&args, "bps"))),
        "refresh_download_url" => unit_result(refresh_url_with_proxy_http(
            arg_str(&args, "id"),
            arg_str(&args, "url"),
            arg_str(&args, "proxy_url"),
        )),
        "set_download_priority" => unit_result(set_priority_http(
            arg_str(&args, "id"),
            arg_i32(&args, "priority"),
        )),
        "pause_all_downloads" => unit_result(pause_all_http()),
        "resume_all_downloads" => unit_result(resume_all_http()),
        "clear_completed_downloads" => unit_result(clear_completed_http()),
        "get_ferric_host_status" => ferric_host_status()
            .unwrap_or_else(|e| serde_json::json!({ "registered": false, "error": e }).to_string()),
        "get_health" => get_health_json_http().unwrap_or_else(error_json),
        "ffmpeg_status" => ffmpeg_status_http().unwrap_or_else(error_json),
        "ytdlp_status" => ytdlp_status_http().unwrap_or_else(error_json),
        "get_proxies" => get_json_http("proxies").unwrap_or_else(error_json),
        "get_webhooks" => get_json_http("webhooks").unwrap_or_else(error_json),
        "parse_cookies" => parse_cookies_http(arg_str(&args, "text")).unwrap_or_else(error_json),
        "export_cookies" => export_cookies_http(arg_str(&args, "text")).unwrap_or_else(error_json),
        "import_qbittorrent" => {
            import_qbittorrent_http(arg_str(&args, "directory")).unwrap_or_else(error_json)
        }
        "export_audit" => export_audit_http().unwrap_or_else(error_json),
        "add_proxy" => add_proxy_http(arg_str(&args, "json")).unwrap_or_else(error_json),
        "delete_proxy" => unit_result(delete_proxy_http(arg_str(&args, "id"))),
        "test_proxy" => test_proxy_http(arg_str(&args, "id")).unwrap_or_else(error_json),
        "add_webhook" => add_webhook_http(arg_str(&args, "url"), arg_str(&args, "events"))
            .unwrap_or_else(error_json),
        "set_torrent_trackers" => {
            let trackers: Vec<String> = arg_str(&args, "trackers")
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned)
                .collect();
            set_torrent_trackers_http(arg_str(&args, "id"), &trackers).unwrap_or_else(error_json)
        }
        "set_setting" => {
            let key = args.get("key").and_then(|v| v.as_str()).unwrap_or_default();
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            set_setting_http(key, value)
                .map(|()| String::new())
                .unwrap_or_else(|e| e)
        }
        _ => format!("{{\"error\":\"unknown async operation: {operation}\"}}"),
    }
}

fn error_json(error: String) -> String {
    serde_json::json!({ "error": error }).to_string()
}

fn arg_str<'a>(args: &'a serde_json::Value, key: &str) -> &'a str {
    args.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

fn arg_u32(args: &serde_json::Value, key: &str) -> u32 {
    args.get(key)
        .and_then(|v| v.as_u64())
        .and_then(|v| u32::try_from(v).ok())
        .unwrap_or_default()
}

fn arg_i32(args: &serde_json::Value, key: &str) -> i32 {
    args.get(key)
        .and_then(|v| v.as_i64())
        .and_then(|v| i32::try_from(v).ok())
        .unwrap_or_default()
}

fn arg_bool(args: &serde_json::Value, key: &str) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(false)
}

fn arg_i64(args: &serde_json::Value, key: &str) -> i64 {
    args.get(key).and_then(|v| v.as_i64()).unwrap_or_default()
}

fn unit_result(result: Result<(), String>) -> String {
    result.map(|()| String::new()).unwrap_or_else(|e| e)
}

#[cfg(test)]
mod action_tests {
    use super::post_action_at;
    use std::io::{Read, Write};

    /// Regression: Stop/Pause/Resume from the toolbar never reached the daemon
    /// because the request future was built outside the Tokio runtime and the
    /// worker thread panicked ("there is no reactor running"). Calling the
    /// function from a plain thread, as `CommandBus::request` does, must work.
    #[test]
    fn post_action_works_from_a_plain_thread() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0_u8; 2048];
            let n = s.read(&mut buf).unwrap();
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            s.write_all(
                b"HTTP/1.1 204 No Content
Connection: close

",
            )
            .unwrap();
            req
        });
        let result = std::thread::spawn(move || post_action_at(port, "tok", "abc", "cancel"))
            .join()
            .expect("post_action_at must not panic");
        assert_eq!(result, Ok(()));
        let req = server.join().unwrap();
        assert!(req.starts_with("POST /downloads/abc/cancel "), "{req}");
        assert!(
            req.to_ascii_lowercase().contains("x-ferric-auth: tok"),
            "{req}"
        );
    }
}

/// Arguments for `shutdown.exe`: a 60-second delayed shutdown (so the user can cancel) or the abort.
fn power_args(action: &str) -> Option<Vec<&'static str>> {
    match action {
        "shutdown" => Some(vec![
            "/s",
            "/t",
            "60",
            "/c",
            "Ferric finished all downloads",
        ]),
        "abort" => Some(vec!["/a"]),
        _ => None,
    }
}

#[cfg(test)]
mod power_tests {
    use super::power_args;

    #[test]
    fn only_a_delayed_shutdown_and_its_abort_are_allowed() {
        assert_eq!(power_args("shutdown").unwrap()[..3], ["/s", "/t", "60"]);
        assert_eq!(power_args("abort").unwrap(), ["/a"]);
        for bad in ["", "restart", "/s /t 0", "shutdown /f"] {
            assert!(power_args(bad).is_none(), "{bad:?} must be refused");
        }
    }
}
