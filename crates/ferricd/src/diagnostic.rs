use ferric_common::{
    ChunkRecord, ChunkStatus, DiagnosticSeverity, DownloadDiagnostic, DownloadRecord,
    DownloadStatus,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DiagnosticPolicy {
    pub max_connections: Option<usize>,
    pub max_speed_bps: Option<u64>,
}

#[cfg(test)]
pub fn classify_download_with_chunks(
    record: &DownloadRecord,
    chunks: &[ChunkRecord],
) -> DownloadDiagnostic {
    classify_download_with_policy(record, chunks, DiagnosticPolicy::default())
}

pub fn classify_download_with_policy(
    record: &DownloadRecord,
    chunks: &[ChunkRecord],
    policy: DiagnosticPolicy,
) -> DownloadDiagnostic {
    let error = record.error_msg.as_deref().unwrap_or_default();
    let lower = error.to_ascii_lowercase();

    match record.status {
        DownloadStatus::Completed => diagnostic(
            "completed",
            DiagnosticSeverity::Info,
            "Download completed",
            "The file is written to disk. Removing the row will not delete the file unless you choose that option.",
            None,
            95,
        ),
        DownloadStatus::Cancelled => diagnostic(
            "cancelled",
            DiagnosticSeverity::Warning,
            "Download cancelled",
            "The transfer was stopped. A partial file may still exist at the save path.",
            Some("Resume if the server still accepts ranges, or remove the row and choose whether to delete the file too."),
            90,
        ),
        DownloadStatus::Paused => {
            if record.referrer.is_some() {
                diagnostic(
                    "paused_browser_capture",
                    DiagnosticSeverity::Info,
                    "Paused browser capture",
                    "This browser-captured download is paused. Its live cookies and headers are intentionally not persisted.",
                    Some("Resume soon, or recapture from the browser if the signed URL or session expires."),
                    90,
                )
            } else {
                diagnostic(
                    "paused",
                    DiagnosticSeverity::Info,
                    "Paused",
                    "The row is paused and should not have active network connections.",
                    Some("Resume when queue policy and network conditions are ready."),
                    90,
                )
            }
        }
        DownloadStatus::Queued => diagnostic(
            "queued",
            DiagnosticSeverity::Info,
            "Waiting in queue",
            "The daemon accepted this download and will start it when queue limits allow.",
            Some("Raise priority or adjust queue limits if this should start first."),
            85,
        ),
        DownloadStatus::Connecting => diagnostic(
            "connecting",
            DiagnosticSeverity::Info,
            "Connecting to remote server",
            "Ferric is preparing the request and checking server download capabilities.",
            Some("If this stays here, the remote host, DNS, TLS, proxy, or browser token may be blocking the request."),
            70,
        ),
        DownloadStatus::Merging => diagnostic(
            "merging",
            DiagnosticSeverity::Info,
            "Merging audio and video",
            "ffmpeg is combining the separately downloaded video and audio streams into the final file.",
            Some("This usually takes a few seconds. If it stalls, check that ffmpeg is installed and on PATH."),
            80,
        ),
        DownloadStatus::Downloading => classify_live_download(record, chunks, policy),
        DownloadStatus::Error => classify_error(record, &lower, error),
    }
}

fn classify_live_download(
    record: &DownloadRecord,
    chunks: &[ChunkRecord],
    policy: DiagnosticPolicy,
) -> DownloadDiagnostic {
    if chunks.is_empty() {
        return diagnostic(
            "downloading_no_chunk_telemetry",
            DiagnosticSeverity::Info,
            "Transfer starting",
            "The download is active, but chunk telemetry is not available yet. This usually means the daemon is still planning or the server is using a single stream.",
            Some("If bytes do not move soon, check whether the URL requires fresh browser capture or the server is delaying the first response."),
            68,
        );
    }

    let active = chunks
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Downloading)
        .count();
    let errors = chunks
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Error)
        .count();
    let completed = chunks
        .iter()
        .filter(|chunk| chunk.status == ChunkStatus::Completed)
        .count();

    if errors > 0 && active == 0 {
        return diagnostic(
            "chunk_errors_no_active_workers",
            DiagnosticSeverity::Warning,
            "Chunk workers stopped",
            "One or more byte ranges failed and no range worker is currently active.",
            Some("Retry the download. If this repeats, reduce connections for this host or refresh the URL."),
            84,
        );
    }

    if active > 0 && record.downloaded == 0 {
        return diagnostic(
            "active_workers_waiting_for_bytes",
            DiagnosticSeverity::Warning,
            "Connections open, no bytes committed yet",
            "Range workers are active, but the daemon has not committed downloaded bytes to the file yet.",
            Some("This can be normal during first-byte latency. If it persists, the remote server or CDN path may be stalling."),
            78,
        );
    }

    if let Some(max_speed_bps) = policy.max_speed_bps {
        return diagnostic(
            "site_speed_limit_active",
            DiagnosticSeverity::Info,
            "Host speed policy active",
            &format!(
                "This host currently has a persisted speed cap of {}/s. Lower UI speeds can be intentional policy, not a network failure.",
                format_bps(max_speed_bps)
            ),
            Some("Adjust the host rule or global speed limiter if this download should use more bandwidth."),
            88,
        );
    }

    if policy.max_connections == Some(1) {
        return diagnostic(
            "site_connection_cap_active",
            DiagnosticSeverity::Warning,
            "Host connection cap active",
            "Ferric has a persisted rule limiting this host to one connection, usually learned after rate limits or range instability.",
            Some("This protects reliability but can reduce speed. If the host is stable now, clear or relax the site rule."),
            86,
        );
    }

    diagnostic(
        "downloading_with_chunk_telemetry",
        DiagnosticSeverity::Info,
        "Transfer is live",
        &format!(
            "{active} active chunk worker(s), {completed} completed chunk(s), {errors} failed chunk(s)."
        ),
        Some("Short dips are normal on remote/CDN paths. Sustained zero-byte samples should be treated as a stall."),
        80,
    )
}

fn classify_error(record: &DownloadRecord, lower: &str, raw: &str) -> DownloadDiagnostic {
    if contains_any(
        lower,
        &[
            "no pending capture",
            "browser capture expired",
            "capture expired",
            "daemon restarted",
        ],
    ) {
        return diagnostic(
            "browser_capture_expired",
            DiagnosticSeverity::Warning,
            "Browser capture expired",
            "The daemon no longer has the in-memory browser headers for this captured download. Ferric intentionally does not persist cookies or headers.",
            Some("Start the download again from Chrome so the extension can send a fresh URL and live browser headers."),
            95,
        );
    }

    if contains_any(
        lower,
        &[
            "native messaging",
            "ferric-host",
            "host disconnected",
            "daemon unavailable",
            "manager unavailable",
            "connection refused",
            "failed to connect",
        ],
    ) {
        return diagnostic(
            "browser_bridge_unavailable",
            DiagnosticSeverity::Error,
            "Browser bridge unavailable",
            "Chrome, the native messaging host, or the daemon bridge failed before the captured download could be handed off cleanly.",
            Some("Keep Ferric running, reload the extension if needed, and retry the browser download. If it repeats, inspect the native host registration and daemon logs."),
            88,
        );
    }

    if contains_any(
        lower,
        &["403", "forbidden", "access denied", "unauthorized", "401"],
    ) {
        if let Some(provider) = signed_url_provider(record) {
            return diagnostic(
                "signed_url_rejected",
                DiagnosticSeverity::Error,
                "Signed URL rejected",
                &format!(
                    "This looks like a temporary {provider} signed URL. The server rejected it, usually because the token expired or the browser session changed."
                ),
                Some("Recapture the download from the browser so Ferric receives a fresh signed URL and live headers."),
                94,
            );
        }
        return diagnostic(
            "auth_or_signed_url_rejected",
            DiagnosticSeverity::Error,
            "Server rejected the request",
            if record.referrer.is_some() {
                "The site rejected this browser-captured URL. The signed link, cookie, or session context may have expired."
            } else {
                "The server refused this URL without the required authorization or browser context."
            },
            Some("Recapture from the browser or use Refresh URL with a fresh signed link."),
            88,
        );
    }

    if contains_any(lower, &["404", "410", "not found", "gone"]) {
        if let Some(provider) = signed_url_provider(record) {
            return diagnostic(
                "signed_url_expired_or_gone",
                DiagnosticSeverity::Error,
                "Signed URL expired",
                &format!(
                    "This {provider} signed URL no longer resolves. Temporary download links can disappear after a time limit or a single browser session."
                ),
                Some("Return to the source page and let the browser extension capture a new link."),
                94,
            );
        }
        if let Some((from, to)) = redirected_host_pair(record) {
            return diagnostic(
                "redirect_target_missing",
                DiagnosticSeverity::Error,
                "Redirect target disappeared",
                &format!(
                    "The original host redirected from {from} to {to}, but the final target no longer serves this object."
                ),
                Some("Open the source page again and capture a fresh link. The redirect target may be temporary or single-use."),
                88,
            );
        }
        return diagnostic(
            "url_expired_or_missing",
            DiagnosticSeverity::Error,
            "URL expired or file disappeared",
            "The remote endpoint no longer serves this object.",
            Some("Open the source page again and capture a fresh URL before retrying."),
            88,
        );
    }

    if contains_any(lower, &["429", "rate limited", "too many requests"]) {
        return diagnostic(
            "remote_rate_limit",
            DiagnosticSeverity::Warning,
            "Remote rate limit",
            "The host is throttling or refusing too many parallel requests.",
            Some("Retry after a short wait. If it repeats, lower the connection count for this host."),
            90,
        );
    }

    if contains_any(lower, &["range", "206", "accept-ranges"]) {
        return diagnostic(
            "range_rejected",
            DiagnosticSeverity::Warning,
            "Server rejected range requests",
            "The server did not honor multipart byte-range downloading for this file.",
            Some(
                "Retry with fewer connections or allow Ferric to fall back to single-stream mode.",
            ),
            86,
        );
    }

    if contains_any(lower, &["etag", "last-modified", "file changed"]) {
        return diagnostic(
            "remote_file_changed",
            DiagnosticSeverity::Error,
            "Remote file changed",
            "The server metadata changed while resuming, so continuing could corrupt the file.",
            Some("Restart the download from a fresh URL."),
            92,
        );
    }

    if contains_any(lower, &["checksum", "hash", "mismatch"]) || record.checksum_ok == Some(false) {
        return diagnostic(
            "checksum_mismatch",
            DiagnosticSeverity::Error,
            "Integrity check failed",
            "The downloaded file did not match the expected checksum.",
            Some("Keep the partial file only if you trust the source; otherwise restart or repair the failed chunks when repair mode is available."),
            90,
        );
    }

    if contains_any(
        lower,
        &[
            "being used by another process",
            "sharing violation",
            "file is locked",
            "lock violation",
            "resource busy",
        ],
    ) {
        return diagnostic(
            "file_write_lock",
            DiagnosticSeverity::Error,
            "Target file is locked",
            "Another app or scanner appears to be holding the partial/target file open, so Ferric cannot safely write to it.",
            Some("Close previewers/extractors for this file, wait for the scanner to release it, then resume."),
            88,
        );
    }

    if contains_any(
        lower,
        &[
            "antivirus",
            "anti-virus",
            "defender",
            "quarantine",
            "malware",
            "virus",
            "threat",
        ],
    ) {
        return diagnostic(
            "security_scanner_blocked_file",
            DiagnosticSeverity::Error,
            "Security scanner blocked the file",
            "A local security tool appears to have blocked, quarantined, or delayed access to the download target.",
            Some("Check Windows Security or your antivirus history before retrying. Do not bypass a real malware warning."),
            88,
        );
    }

    if contains_any(
        lower,
        &[
            "i/o",
            "io error",
            "permission denied",
            "access is denied",
            "disk",
        ],
    ) {
        return diagnostic(
            "disk_or_permission_error",
            DiagnosticSeverity::Error,
            "Disk or permission error",
            "Ferric could not write or sync the target file.",
            Some("Check free space, folder permissions, antivirus locks, and whether the file is open in another app."),
            82,
        );
    }

    if let Some((from, to)) = redirected_host_pair(record) {
        let hop_note = redirect_chain_hop_count(record)
            .map(|count| format!(" across {count} observed redirect endpoint(s)"))
            .unwrap_or_default();
        return diagnostic(
            "redirect_target_failed",
            DiagnosticSeverity::Warning,
            "Redirect target failed",
            &format!(
                "The request left {from} and landed on {to}{hop_note}; the failure happened after that redirect."
            ),
            Some("Retry from the source page. If this repeats, the target CDN or file host may be rejecting resumed/browser-captured requests."),
            78,
        );
    }

    diagnostic(
        "unclassified_error",
        DiagnosticSeverity::Error,
        "Download failed",
        if raw.is_empty() {
            "The daemon reported a transfer failure but did not provide a specific classified cause."
        } else {
            raw
        },
        Some("Refresh the URL, check the source site, and retry. If it repeats, inspect connection telemetry."),
        45,
    )
}

fn contains_any(value: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| value.contains(needle))
}

fn format_bps(value: u64) -> String {
    if value < 1024 {
        return format!("{value} B");
    }
    let kib = value as f64 / 1024.0;
    if kib < 1024.0 {
        return format!("{kib:.1} KB");
    }
    let mib = kib / 1024.0;
    if mib < 1024.0 {
        return format!("{mib:.1} MB");
    }
    format!("{:.1} GB", mib / 1024.0)
}

fn redirected_host_pair(record: &DownloadRecord) -> Option<(String, String)> {
    let final_url = redirect_chain_end(record).or_else(|| record.final_url.clone())?;
    let original = url::Url::parse(&record.url).ok()?;
    let final_url = url::Url::parse(&final_url).ok()?;
    let original_host = original.host_str()?.to_ascii_lowercase();
    let final_host = final_url.host_str()?.to_ascii_lowercase();

    (original_host != final_host).then_some((original_host, final_host))
}

fn redirect_chain_end(record: &DownloadRecord) -> Option<String> {
    let chain = record.redirect_chain.as_deref()?;
    let values = serde_json::from_str::<Vec<String>>(chain).ok()?;
    values.last().cloned()
}

fn redirect_chain_hop_count(record: &DownloadRecord) -> Option<usize> {
    let chain = record.redirect_chain.as_deref()?;
    let values = serde_json::from_str::<Vec<String>>(chain).ok()?;
    (!values.is_empty()).then_some(values.len())
}

fn signed_url_provider(record: &DownloadRecord) -> Option<&'static str> {
    std::iter::once(record.url.as_str())
        .chain(record.final_url.as_deref())
        .find_map(classify_signed_url)
}

fn classify_signed_url(raw: &str) -> Option<&'static str> {
    let url = url::Url::parse(raw).ok()?;
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    let pairs = url
        .query_pairs()
        .map(|(key, _)| key.to_ascii_lowercase())
        .collect::<Vec<_>>();

    if has_all(&pairs, &["x-amz-algorithm", "x-amz-signature"])
        || has_any(&pairs, &["x-amz-expires", "x-amz-credential"])
    {
        return Some("S3-compatible");
    }

    if has_all(&pairs, &["x-goog-algorithm", "x-goog-signature"])
        || has_any(&pairs, &["x-goog-expires", "x-goog-credential"])
    {
        return Some("Google Cloud Storage");
    }

    if pairs.iter().any(|key| key == "x-ms-sig" || key == "sig")
        && pairs
            .iter()
            .any(|key| matches!(key.as_str(), "se" | "sp" | "sv" | "sr"))
    {
        return Some("Azure SAS");
    }

    if has_any(&pairs, &["expires", "signature", "key-pair-id"])
        && (host.contains("cloudfront.net") || has_all(&pairs, &["expires", "signature"]))
    {
        return Some("CloudFront-style");
    }

    if has_any(
        &pairs,
        &["token", "expires", "expire", "expiry", "auth", "signature"],
    ) && has_any(&pairs, &["download", "dl", "id", "file"])
    {
        return Some("temporary");
    }

    None
}

fn has_any(keys: &[String], needles: &[&str]) -> bool {
    keys.iter()
        .any(|key| needles.iter().any(|needle| key == needle))
}

fn has_all(keys: &[String], needles: &[&str]) -> bool {
    needles
        .iter()
        .all(|needle| keys.iter().any(|key| key == needle))
}

fn diagnostic(
    code: &str,
    severity: DiagnosticSeverity,
    title: &str,
    summary: &str,
    action: Option<&str>,
    confidence: u8,
) -> DownloadDiagnostic {
    DownloadDiagnostic {
        code: code.to_string(),
        severity,
        title: title.to_string(),
        summary: summary.to_string(),
        action: action.map(str::to_string),
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn record(status: DownloadStatus, error_msg: Option<&str>) -> DownloadRecord {
        DownloadRecord {
            id: Uuid::new_v4(),
            url: "https://example.com/file.bin".to_string(),
            final_url: None,
            redirect_chain: None,
            filename: "file.bin".to_string(),
            save_path: "file.bin".to_string(),
            status,
            total_size: Some(1024),
            downloaded: 0,
            created_at: 0,
            started_at: None,
            completed_at: None,
            error_msg: error_msg.map(str::to_string),
            error_count: 0,
            referrer: None,
            mime_type: None,
            etag: None,
            last_modified: None,
            category: "Other".to_string(),
            queue_id: None,
            priority: 0,
            checksum: None,
            checksum_ok: None,
            speed_limit_bps: 0,
            audio_url: None,
            subtitle_url: None,
            subtitle_lang: None,
            proxy_url: None,
            mirrors: vec![],
            description: None,
        }
    }

    #[test]
    fn classifies_rate_limit() {
        let rec = record(DownloadStatus::Error, Some("rate limited by server (429)"));
        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "remote_rate_limit");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn classifies_signed_url_or_auth_rejection() {
        let rec = record(DownloadStatus::Error, Some("access denied by server (403)"));
        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "auth_or_signed_url_rejected");
        assert_eq!(d.severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn classifies_s3_signed_url_rejection() {
        let mut rec = record(DownloadStatus::Error, Some("access denied by server (403)"));
        rec.url = "https://bucket.s3.amazonaws.com/file.bin?X-Amz-Algorithm=AWS4-HMAC-SHA256&X-Amz-Signature=abc&X-Amz-Expires=60".to_string();

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "signed_url_rejected");
        assert!(d.summary.contains("S3-compatible"));
    }

    #[test]
    fn classifies_azure_signed_url_gone() {
        let mut rec = record(
            DownloadStatus::Error,
            Some("file not found on server (404)"),
        );
        rec.final_url = Some("https://account.blob.core.windows.net/c/file.bin?sv=2024-01-01&se=2026-07-03T00%3A00%3A00Z&sp=r&sr=b&sig=abc".to_string());

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "signed_url_expired_or_gone");
        assert!(d.summary.contains("Azure SAS"));
    }

    #[test]
    fn classifies_range_rejection() {
        let rec = record(DownloadStatus::Error, Some("server rejected Range request"));
        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "range_rejected");
    }

    #[test]
    fn classifies_redirect_target_missing() {
        let mut rec = record(
            DownloadStatus::Error,
            Some("file not found on server (404)"),
        );
        rec.url = "https://source.example/download?id=42".to_string();
        rec.final_url = Some("https://cdn.example/object.bin".to_string());

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "redirect_target_missing");
        assert!(d.summary.contains("source.example"));
        assert!(d.summary.contains("cdn.example"));
    }

    #[test]
    fn classifies_redirect_target_failed() {
        let mut rec = record(
            DownloadStatus::Error,
            Some("download request failed with 502"),
        );
        rec.url = "https://source.example/download?id=42".to_string();
        rec.final_url = Some("https://cdn.example/object.bin".to_string());

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "redirect_target_failed");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn classifies_redirect_target_failed_from_persisted_chain() {
        let mut rec = record(
            DownloadStatus::Error,
            Some("download request failed with 502"),
        );
        rec.url = "https://source.example/download?id=42".to_string();
        rec.redirect_chain = Some(
            serde_json::to_string(&[
                "https://source.example/download?id=42",
                "https://cdn.example/object.bin",
            ])
            .unwrap(),
        );

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "redirect_target_failed");
        assert!(d.summary.contains("2 observed redirect"));
    }

    #[test]
    fn classifies_file_write_lock() {
        let rec = record(
            DownloadStatus::Error,
            Some("The process cannot access the file because it is being used by another process"),
        );

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "file_write_lock");
    }

    #[test]
    fn classifies_security_scanner_block() {
        let rec = record(
            DownloadStatus::Error,
            Some("Windows Defender quarantined the partial file as a threat"),
        );

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "security_scanner_blocked_file");
    }

    #[test]
    fn classifies_browser_capture_expired() {
        let rec = record(
            DownloadStatus::Error,
            Some("This browser capture expired after the daemon restarted"),
        );

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "browser_capture_expired");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
        assert!(d.action.unwrap().contains("Chrome"));
    }

    #[test]
    fn classifies_browser_bridge_unavailable() {
        let rec = record(
            DownloadStatus::Error,
            Some("native messaging host disconnected; connection refused"),
        );

        let d = classify_download_with_chunks(&rec, &[]);
        assert_eq!(d.code, "browser_bridge_unavailable");
        assert_eq!(d.severity, DiagnosticSeverity::Error);
    }

    #[test]
    fn classifies_active_workers_without_bytes() {
        let rec = record(DownloadStatus::Downloading, None);
        let chunks = vec![ChunkRecord {
            id: Uuid::new_v4(),
            download_id: rec.id,
            chunk_index: 0,
            byte_start: 0,
            byte_end: 1023,
            downloaded: 0,
            status: ChunkStatus::Downloading,
            error_count: 0,
        }];

        let d = classify_download_with_chunks(&rec, &chunks);
        assert_eq!(d.code, "active_workers_waiting_for_bytes");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn classifies_chunk_errors_with_no_active_workers() {
        let rec = record(DownloadStatus::Downloading, None);
        let chunks = vec![ChunkRecord {
            id: Uuid::new_v4(),
            download_id: rec.id,
            chunk_index: 0,
            byte_start: 0,
            byte_end: 1023,
            downloaded: 0,
            status: ChunkStatus::Error,
            error_count: 1,
        }];

        let d = classify_download_with_chunks(&rec, &chunks);
        assert_eq!(d.code, "chunk_errors_no_active_workers");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
    }

    #[test]
    fn classifies_site_speed_limit_active() {
        let mut rec = record(DownloadStatus::Downloading, None);
        rec.downloaded = 512;
        let chunks = vec![ChunkRecord {
            id: Uuid::new_v4(),
            download_id: rec.id,
            chunk_index: 0,
            byte_start: 0,
            byte_end: 1023,
            downloaded: 512,
            status: ChunkStatus::Downloading,
            error_count: 0,
        }];

        let d = classify_download_with_policy(
            &rec,
            &chunks,
            DiagnosticPolicy {
                max_connections: None,
                max_speed_bps: Some(1_048_576),
            },
        );
        assert_eq!(d.code, "site_speed_limit_active");
        assert!(d.summary.contains("1.0 MB/s"));
    }

    #[test]
    fn classifies_site_connection_cap_active() {
        let mut rec = record(DownloadStatus::Downloading, None);
        rec.downloaded = 512;
        let chunks = vec![ChunkRecord {
            id: Uuid::new_v4(),
            download_id: rec.id,
            chunk_index: 0,
            byte_start: 0,
            byte_end: 1023,
            downloaded: 512,
            status: ChunkStatus::Downloading,
            error_count: 0,
        }];

        let d = classify_download_with_policy(
            &rec,
            &chunks,
            DiagnosticPolicy {
                max_connections: Some(1),
                max_speed_bps: None,
            },
        );
        assert_eq!(d.code, "site_connection_cap_active");
        assert_eq!(d.severity, DiagnosticSeverity::Warning);
    }
}
