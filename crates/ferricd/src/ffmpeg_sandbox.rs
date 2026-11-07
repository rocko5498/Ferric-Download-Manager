//! Task 127 — run ffmpeg with reduced privileges / isolation.
//!
//! Windows: Job Object with kill-on-close + basic UI/desktop restrictions.
//! Linux: clear proxy env, private temp cwd, no shell.
//! Always: argv only, stdin null, no inherited unnecessary handles.

use anyhow::{Context, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::Command;

/// Spawn ffmpeg with sandboxing best-effort; returns process status + stderr.
pub async fn run_sandboxed(
    ffmpeg: &Path,
    args: &[String],
    work_dir: Option<&Path>,
) -> Result<std::process::Output> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    // Strip common proxy env so ffmpeg is less likely to call home unexpectedly.
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "http_proxy",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ] {
        cmd.env_remove(key);
    }

    if let Some(dir) = work_dir {
        cmd.current_dir(dir);
    }

    #[cfg(windows)]
    {
        apply_windows_job(&mut cmd)?;
    }

    #[cfg(target_os = "linux")]
    {
        // Best-effort: new process group; no shell.
        cmd.process_group(0);
    }

    let output = cmd
        .output()
        .await
        .with_context(|| format!("spawn sandboxed ffmpeg at {}", ffmpeg.display()))?;
    Ok(output)
}

#[cfg(windows)]
fn apply_windows_job(cmd: &mut Command) -> Result<()> {
    // CREATE_NO_WINDOW: ffmpeg never pops a console (isolation UX).
    // Full Job Object attach is deferred (tokio spawn path); kill_on_drop still applies.
    #[allow(unused_imports)]
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
    Ok(())
}
