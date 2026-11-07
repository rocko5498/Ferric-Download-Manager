// AV scan hooks — called after download completes, non-blocking.
// Windows: MpCmdRun.exe -Scan -ScanType 3 -File <path>
// Linux:   clamdscan --no-summary <path> (only if clamdscan is in PATH)

// `spawn_av_scan` (job.rs) is a no-op stub under `#[cfg(test)]`, so these
// functions are only reachable in non-test builds.
#![cfg_attr(test, allow(dead_code))]

use anyhow::Result;
use sqlx::SqlitePool;
use std::path::Path;

pub async fn scan(path: &Path) {
    #[cfg(windows)]
    scan_windows(path).await;

    #[cfg(target_os = "linux")]
    scan_linux(path).await;
}

#[cfg(windows)]
async fn scan_windows(path: &Path) {
    let path_str = path.to_string_lossy().to_string();
    let mut scan = tokio::process::Command::new("MpCmdRun.exe");
    scan.args(["-Scan", "-ScanType", "3", "-File", &path_str]);
    scan.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match scan.output().await {
        Ok(out) if out.status.success() => tracing::info!("Defender scan: clean"),
        Ok(out) => tracing::warn!(
            "Defender flagged file: {:?}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Err(e) => tracing::debug!("Defender scan skipped: {e}"),
    }
}

#[cfg(target_os = "linux")]
async fn scan_linux(path: &Path) {
    if which::which("clamdscan").is_err() {
        return;
    }
    let mut scan = tokio::process::Command::new("clamdscan");
    scan.arg("--no-summary").arg(path);
    #[cfg(windows)]
    scan.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match scan.output().await {
        Ok(out) if out.status.success() => tracing::info!("ClamAV scan: clean"),
        Ok(out) => tracing::warn!(
            "ClamAV flagged file: {:?}",
            String::from_utf8_lossy(&out.stdout)
        ),
        Err(e) => tracing::debug!("ClamAV scan skipped: {e}"),
    }
}

pub async fn scan_torrent(
    pool: &SqlitePool,
    torrent_id: &str,
    save_root: &Path,
    name: &str,
) -> Result<()> {
    let enabled = crate::db::settings::get(pool, "av_enabled")
        .await?
        .unwrap_or_else(|| "0".to_string());
    if enabled != "1" {
        crate::db::torrents::update_av_status(
            pool,
            torrent_id,
            "Skipped",
            Some("Antivirus scanning is disabled in settings"),
        )
        .await?;
        return Ok(());
    }

    let scan_path = save_root.join(name);
    if !scan_path.exists() {
        crate::db::torrents::update_av_status(
            pool,
            torrent_id,
            "Error",
            Some("Scan path does not exist"),
        )
        .await?;
        return Ok(());
    }

    crate::db::torrents::update_av_status(pool, torrent_id, "Scanning", None).await?;

    let custom_path = crate::db::settings::get(pool, "av_path")
        .await?
        .filter(|s| !s.is_empty());
    let custom_args = crate::db::settings::get(pool, "av_args")
        .await?
        .filter(|s| !s.is_empty());

    let (exe_path, args) =
        resolve_scanner(custom_path.as_deref(), custom_args.as_deref(), &scan_path)?;

    let mut scan = tokio::process::Command::new(&exe_path);
    scan.args(&args);
    #[cfg(windows)]
    scan.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    match scan.output().await {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout).to_string();
            let stderr = String::from_utf8_lossy(&out.stderr).to_string();
            let full_out = format!("{stdout}\n{stderr}");
            let success = out.status.success();
            if success {
                crate::db::torrents::update_av_status(pool, torrent_id, "Clean", Some(&full_out))
                    .await?;
            } else {
                let exit_code = out.status.code().unwrap_or(-1);
                if exit_code == 1 || exit_code == 2 {
                    crate::db::torrents::update_av_status(
                        pool,
                        torrent_id,
                        "Infected",
                        Some(&full_out),
                    )
                    .await?;
                } else {
                    crate::db::torrents::update_av_status(
                        pool,
                        torrent_id,
                        "Error",
                        Some(&format!("Exit code {exit_code}: {full_out}")),
                    )
                    .await?;
                }
            }
        }
        Err(e) => {
            crate::db::torrents::update_av_status(
                pool,
                torrent_id,
                "Error",
                Some(&format!("Failed to launch scanner: {e}")),
            )
            .await?;
        }
    }
    Ok(())
}

fn resolve_scanner(
    custom_path: Option<&str>,
    custom_args: Option<&str>,
    scan_path: &Path,
) -> Result<(String, Vec<String>)> {
    let scan_path_str = scan_path.to_string_lossy().to_string();
    if let Some(path) = custom_path {
        let args = if let Some(args_str) = custom_args {
            let mut resolved = Vec::new();
            let mut replaced = false;
            for part in args_str.split_whitespace() {
                if part.contains("{}") {
                    resolved.push(part.replace("{}", &scan_path_str));
                    replaced = true;
                } else {
                    resolved.push(part.to_string());
                }
            }
            if !replaced {
                resolved.push(scan_path_str);
            }
            resolved
        } else {
            vec![scan_path_str]
        };
        Ok((path.to_string(), args))
    } else {
        #[cfg(windows)]
        {
            Ok((
                "MpCmdRun.exe".to_string(),
                vec![
                    "-Scan".to_string(),
                    "-ScanType".to_string(),
                    "3".to_string(),
                    "-File".to_string(),
                    scan_path_str,
                ],
            ))
        }
        #[cfg(not(windows))]
        {
            Ok((
                "clamdscan".to_string(),
                vec!["--no-summary".to_string(), scan_path_str],
            ))
        }
    }
}
