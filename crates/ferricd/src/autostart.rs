use anyhow::Result;
use std::path::Path;

// ── Windows ───────────────────────────────────────────────────────────────────

#[cfg(windows)]
pub fn register(exe_path: &Path) -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run = hkcu.open_subkey_with_flags(
        r"Software\Microsoft\Windows\CurrentVersion\Run",
        KEY_READ | KEY_WRITE,
    )?;

    // Like other download managers: start the main program with a flag so it comes up hidden in the
    // tray, and let it start the daemon silently. A bare daemon at login has no tray icon and no way back.
    let ui_path = exe_path
        .parent()
        .map(|d| d.join("ferric-qt.exe"))
        .filter(|p| p.exists());
    let val: String = match ui_path {
        Some(ui) => format!("\"{}\" --autostart", ui.display()),
        None => format!("\"{}\"", exe_path.display()),
    };

    if cfg!(debug_assertions) {
        if matches!(run.get_value::<String, _>("Ferric"), Ok(current) if current == val) {
            run.delete_value("Ferric").ok();
        }
        return Ok(());
    }

    run.set_value("Ferric", &val)?;
    Ok(())
}

#[cfg(windows)]
pub fn unregister() -> Result<()> {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_WRITE};
    use winreg::RegKey;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let run =
        hkcu.open_subkey_with_flags(r"Software\Microsoft\Windows\CurrentVersion\Run", KEY_WRITE)?;
    match run.delete_value("Ferric") {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

// ── Linux ─────────────────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn desktop_path() -> Result<std::path::PathBuf> {
    use directories::BaseDirs;
    let base = BaseDirs::new().ok_or_else(|| anyhow::anyhow!("cannot determine base dirs"))?;
    Ok(base.config_dir().join("autostart").join("ferric.desktop"))
}

#[cfg(target_os = "linux")]
pub fn register(exe_path: &Path) -> Result<()> {
    let path = desktop_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=Ferric\nExec={}\nHidden=false\nX-GNOME-Autostart-enabled=true\n",
        exe_path.display()
    );
    std::fs::write(&path, content)?;
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn unregister() -> Result<()> {
    let path = desktop_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

// ── Other platforms (macOS, etc.) — no-op ────────────────────────────────────

#[cfg(not(any(windows, target_os = "linux")))]
pub fn register(_exe_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn unregister() -> Result<()> {
    Ok(())
}
