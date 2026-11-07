use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};

const HOST_NAME: &str = "com.ferric.host";
const DESCRIPTION: &str = "Ferric Download Manager native messaging host";
const MANIFEST_FILE: &str = "com.ferric.host.json";
const DEFAULT_FIREFOX_EXTENSION_ID: &str = "ferric@ferric.local";
/// The id of the packed (CRX) extension; its registry keys are removed again by `--uninstall`.
#[cfg(windows)]
const PACKED_EXTENSION_ID: &str = "gepkkfjfgdcgelcnejpikdeanggknbjo";

/// `ids` are the extension ids the host accepts; the first Chromium id is the packed extension
/// that the browsers are offered through the external-extension registry key.
pub fn install(ids: &[String], offer_extension: bool) -> anyhow::Result<()> {
    anyhow::ensure!(!ids.is_empty(), "no extension id given");
    for id in ids {
        validate_extension_id(id)?;
    }
    let manifest_path = stable_manifest_path()?;
    let host_path = std::env::current_exe()?;
    write_manifest(&manifest_path, &host_path, ids)?;
    install_platform(&manifest_path, &host_path, ids, offer_extension)?;
    Ok(())
}

/// True when the registered connector file points at `exe` (or there is nothing readable to compare).
/// The registration is shared by every copy of Ferric on the machine, so one copy must not remove another's.
fn registration_is_ours(manifest: &Path, exe: &Path) -> bool {
    let Ok(text) = fs::read_to_string(manifest) else {
        return true;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return true;
    };
    let Some(registered) = value.get("path").and_then(|p| p.as_str()) else {
        return true;
    };
    let norm = |s: &str| s.replace('/', "\\").to_ascii_lowercase();
    norm(registered) == norm(&exe.to_string_lossy())
}

pub fn uninstall() -> anyhow::Result<()> {
    if let (Ok(manifest), Ok(exe)) = (stable_manifest_path(), std::env::current_exe()) {
        if manifest.exists() && !registration_is_ours(&manifest, &exe) {
            eprintln!("ferric-host: the browser registration belongs to another Ferric install; left in place");
            return Ok(());
        }
    }
    uninstall_platform()?;
    let manifest_path = stable_manifest_path()?;
    if manifest_path.exists() {
        fs::remove_file(manifest_path)?;
    }
    Ok(())
}

pub fn is_registered() -> bool {
    stable_manifest_path().is_ok_and(|p| p.exists())
}

fn write_manifest(
    manifest_path: &Path,
    host_path: &Path,
    extension_ids: &[String],
) -> anyhow::Result<()> {
    if let Some(parent) = manifest_path.parent() {
        fs::create_dir_all(parent)?;
    }
    let manifest = manifest_json(host_path, extension_ids);
    fs::write(manifest_path, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn manifest_json(host_path: &Path, extension_ids: &[String]) -> serde_json::Value {
    let origins: Vec<String> = extension_ids
        .iter()
        .filter(|id| is_chromium_extension_id(id))
        .map(|id| format!("chrome-extension://{id}/"))
        .collect();
    json!({
        "name": HOST_NAME,
        "description": DESCRIPTION,
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_origins": origins,
    })
}

fn firefox_manifest_json(host_path: &Path, extension_id: &str) -> serde_json::Value {
    let firefox_id = firefox_extension_id(extension_id);
    json!({
        "name": HOST_NAME,
        "description": DESCRIPTION,
        "path": host_path.to_string_lossy(),
        "type": "stdio",
        "allowed_extensions": [firefox_id],
    })
}

fn write_firefox_manifest(
    target: &Path,
    host_path: &Path,
    extension_id: &str,
) -> anyhow::Result<()> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    let manifest = firefox_manifest_json(host_path, extension_id);
    fs::write(target, serde_json::to_vec_pretty(&manifest)?)?;
    Ok(())
}

fn firefox_extension_id(extension_id: &str) -> String {
    std::env::var("FERRIC_FIREFOX_EXTENSION_ID").unwrap_or_else(|_| {
        let trimmed = extension_id.trim();
        if trimmed.contains('@') {
            trimmed.to_string()
        } else {
            DEFAULT_FIREFOX_EXTENSION_ID.to_string()
        }
    })
}

fn stable_manifest_path() -> anyhow::Result<PathBuf> {
    Ok(stable_config_dir()?.join(MANIFEST_FILE))
}

#[cfg(windows)]
fn stable_config_dir() -> anyhow::Result<PathBuf> {
    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    Ok(PathBuf::from(appdata).join("Ferric"))
}

#[cfg(target_os = "linux")]
fn stable_config_dir() -> anyhow::Result<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("ferric"));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    Ok(PathBuf::from(home).join(".config").join("ferric"))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn stable_config_dir() -> anyhow::Result<PathBuf> {
    Err(anyhow::anyhow!(
        "native messaging install is not supported on this platform"
    ))
}

/// The packed extension that ships next to the host (`extension\Ferric.crx` and its `manifest.json`),
/// as (path, version). IDM offers its extension the same way: a registry key the browser reads.
#[cfg(windows)]
fn packed_extension(host_path: &Path) -> Option<(PathBuf, String)> {
    let dir = host_path.parent()?.join("extension");
    let crx = dir.join("Ferric.crx");
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.join("manifest.json")).ok()?).ok()?;
    let version = manifest.get("version")?.as_str()?.to_string();
    crx.exists().then_some((crx, version))
}

#[cfg(windows)]
fn install_platform(
    manifest_path: &Path,
    host_path: &Path,
    extension_ids: &[String],
    offer_extension: bool,
) -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for base in [
        r"Software\Google\Chrome\NativeMessagingHosts\com.ferric.host",
        r"Software\Microsoft\Edge\NativeMessagingHosts\com.ferric.host",
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.ferric.host",
    ] {
        let (key, _) = hkcu.create_subkey(base)?;
        key.set_value("", &manifest_path.to_string_lossy().to_string())?;
    }

    // Offer the packed extension to Chrome, Brave and Edge through the external-extension keys, the way
    // IDM does. Off by default: the browsers only let such an extension run when it is in their store
    // (an off-store one is listed but cannot be switched on), so use `--offer-extension` once Ferric is listed.
    if let (true, Some(id), Some((crx, version))) = (
        offer_extension,
        extension_ids.iter().find(|id| is_chromium_extension_id(id)),
        packed_extension(host_path),
    ) {
        for base in [
            r"Software\Google\Chrome\Extensions",
            r"Software\Microsoft\Edge\Extensions",
        ] {
            let (key, _) = hkcu.create_subkey(format!(r"{base}\{id}"))?;
            key.set_value("path", &crx.to_string_lossy().to_string())?;
            key.set_value("version", &version)?;
        }
    }

    let extension_id = extension_ids.first().map_or("", String::as_str);
    let firefox_manifest = firefox_manifest_path()?;
    write_firefox_manifest(&firefox_manifest, host_path, extension_id)?;
    let (key, _) = hkcu.create_subkey(r"Software\Mozilla\NativeMessagingHosts\com.ferric.host")?;
    key.set_value("", &firefox_manifest.to_string_lossy().to_string())?;
    Ok(())
}

#[cfg(windows)]
fn uninstall_platform() -> anyhow::Result<()> {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for base in [
        r"Software\Google\Chrome\NativeMessagingHosts\com.ferric.host",
        r"Software\Microsoft\Edge\NativeMessagingHosts\com.ferric.host",
        r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts\com.ferric.host",
        r"Software\Mozilla\NativeMessagingHosts\com.ferric.host",
    ] {
        match hkcu.delete_subkey(base) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    for base in [
        r"Software\Google\Chrome\Extensions",
        r"Software\Microsoft\Edge\Extensions",
    ] {
        match hkcu.delete_subkey_all(format!(r"{base}\{PACKED_EXTENSION_ID}")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    let firefox_manifest = firefox_manifest_path()?;
    if firefox_manifest.exists() {
        fs::remove_file(firefox_manifest)?;
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn install_platform(
    manifest_path: &Path,
    host_path: &Path,
    extension_ids: &[String],
    _offer_extension: bool,
) -> anyhow::Result<()> {
    let extension_id = extension_ids.first().map_or("", String::as_str);
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    for browser_dir in [
        ".config/google-chrome/NativeMessagingHosts",
        ".config/microsoft-edge/NativeMessagingHosts",
    ] {
        let target = PathBuf::from(&home).join(browser_dir).join(MANIFEST_FILE);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(manifest_path, target)?;
    }
    let firefox_manifest = PathBuf::from(&home)
        .join(".mozilla/native-messaging-hosts")
        .join(MANIFEST_FILE);
    write_firefox_manifest(&firefox_manifest, host_path, extension_id)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn uninstall_platform() -> anyhow::Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow::anyhow!("HOME is not set"))?;
    for browser_dir in [
        ".config/google-chrome/NativeMessagingHosts",
        ".config/microsoft-edge/NativeMessagingHosts",
        ".mozilla/native-messaging-hosts",
    ] {
        let target = PathBuf::from(&home).join(browser_dir).join(MANIFEST_FILE);
        if target.exists() {
            fs::remove_file(target)?;
        }
    }
    Ok(())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn install_platform(
    _manifest_path: &Path,
    _host_path: &Path,
    _extension_ids: &[String],
    _offer_extension: bool,
) -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "native messaging install is not supported on this platform"
    ))
}

#[cfg(not(any(windows, target_os = "linux")))]
fn uninstall_platform() -> anyhow::Result<()> {
    Err(anyhow::anyhow!(
        "native messaging uninstall is not supported on this platform"
    ))
}

#[cfg(windows)]
fn firefox_manifest_path() -> anyhow::Result<PathBuf> {
    let appdata =
        std::env::var_os("APPDATA").ok_or_else(|| anyhow::anyhow!("APPDATA is not set"))?;
    Ok(PathBuf::from(appdata)
        .join("Mozilla")
        .join("NativeMessagingHosts")
        .join(MANIFEST_FILE))
}

fn validate_extension_id(extension_id: &str) -> anyhow::Result<()> {
    let valid = is_chromium_extension_id(extension_id) || is_firefox_extension_id(extension_id);
    if valid {
        Ok(())
    } else {
        Err(anyhow::anyhow!("invalid extension id: {extension_id}"))
    }
}

fn is_chromium_extension_id(extension_id: &str) -> bool {
    extension_id.len() == 32 && extension_id.bytes().all(|byte| byte.is_ascii_lowercase())
}

fn is_firefox_extension_id(extension_id: &str) -> bool {
    let id = extension_id.trim();
    id.len() >= 3
        && id.len() <= 128
        && id.contains('@')
        && !id.contains('/')
        && !id.contains('\\')
        && !id.chars().any(char::is_whitespace)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_registration_is_removed_only_by_the_copy_it_points_at() {
        let dir = std::env::temp_dir().join(format!("ferric-host-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let manifest = dir.join("com.ferric.host.json");
        // JSON needs doubled backslashes: this registers F:\Kit\bin\ferric-host.exe
        std::fs::write(
            &manifest,
            r#"{"name":"com.ferric.host","path":"F:\\Kit\\bin\\ferric-host.exe"}"#,
        )
        .unwrap();
        // the copy the manifest names (any case, either slash) may remove it
        assert!(registration_is_ours(
            &manifest,
            Path::new("f:/kit/bin/Ferric-Host.exe")
        ));
        // a throwaway copy elsewhere must leave it alone
        assert!(!registration_is_ours(
            &manifest,
            Path::new(r"C:\Temp\test\ferric-host.exe")
        ));
        // nothing readable to compare: behave as before
        assert!(registration_is_ours(
            &dir.join("missing.json"),
            Path::new("x.exe")
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn manifest_contains_required_native_messaging_fields() {
        let manifest = manifest_json(
            Path::new(if cfg!(windows) {
                r"C:\Ferric\ferric-host.exe"
            } else {
                "/opt/ferric/ferric-host"
            }),
            &[
                "gepkkfjfgdcgelcnejpikdeanggknbjo".to_string(),
                "bngipfinnjpcnebfchfbeinfppepibkb".to_string(),
            ],
        );

        assert_eq!(manifest["name"], HOST_NAME);
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(
            manifest["allowed_origins"][0],
            "chrome-extension://gepkkfjfgdcgelcnejpikdeanggknbjo/"
        );
        assert_eq!(
            manifest["allowed_origins"][1],
            "chrome-extension://bngipfinnjpcnebfchfbeinfppepibkb/"
        );
        assert!(manifest["path"]
            .as_str()
            .is_some_and(|path| path.contains("ferric-host")));
    }

    #[test]
    fn firefox_manifest_uses_allowed_extensions() {
        let manifest = firefox_manifest_json(
            Path::new(if cfg!(windows) {
                r"C:\Ferric\ferric-host.exe"
            } else {
                "/opt/ferric/ferric-host"
            }),
            "ferric@example.org",
        );

        assert_eq!(manifest["name"], HOST_NAME);
        assert_eq!(manifest["type"], "stdio");
        assert_eq!(manifest["allowed_extensions"][0], "ferric@example.org");
        assert!(manifest.get("allowed_origins").is_none());
    }

    #[test]
    fn firefox_manifest_falls_back_to_packaged_id_for_chromium_id() {
        let manifest = firefox_manifest_json(
            Path::new(if cfg!(windows) {
                r"C:\Ferric\ferric-host.exe"
            } else {
                "/opt/ferric/ferric-host"
            }),
            "bngipfinnjpcnebfchfbeinfppepibkb",
        );

        assert_eq!(
            manifest["allowed_extensions"][0],
            DEFAULT_FIREFOX_EXTENSION_ID
        );
    }

    #[test]
    fn extension_id_validation_rejects_bad_values() {
        assert!(validate_extension_id("bngipfinnjpcnebfchfbeinfppepibkb").is_ok());
        assert!(validate_extension_id("ferric@example.org").is_ok());
        assert!(validate_extension_id("INVALID_EXTENSION_ID").is_err());
        assert!(validate_extension_id("short").is_err());
        assert!(validate_extension_id("BNGIPFINNJPCNEBFCHFBEINFPPepibkb").is_err());
        assert!(validate_extension_id("ferric example@example.org").is_err());
    }
}
