use ferric_common::FerricError;
use sanitize_filename::sanitize;
use std::collections::HashSet;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use url::Url;

const WIN_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const DANGEROUS_EXTENSIONS: &[&str] = &[
    "exe", "msi", "bat", "cmd", "ps1", "vbs", "js", "jse", "wsf", "reg", "scr", "pif", "com",
    "hta", "cpl", "dll", "sys", "sh", "deb", "rpm", "dmg", "pkg",
];

/// File-transfer schemes that may legitimately target LAN / NAS hosts.
const LAN_OK_SCHEMES: &[&str] = &["ftp", "ftps", "ftps-implicit", "sftp", "webdav", "webdavs"];

/// Validate a URL before making any network request.
/// Blocks private IPs / loopback for http(s); allows LAN for FTP/SFTP/WebDAV.
pub fn validate_url(raw: &str) -> Result<Url, FerricError> {
    if raw.chars().any(char::is_control) {
        return Err(FerricError::InvalidUrl(
            "URL contains non-printable characters".into(),
        ));
    }

    let url = Url::parse(raw)
        .map_err(|e| FerricError::InvalidUrl(format!("{e}: {}", redact_url(raw))))?;

    let scheme = url.scheme();
    // Task 128: never allow file/data/javascript schemes into the download engine.
    if matches!(scheme, "file" | "data" | "javascript" | "vbscript") {
        return Err(FerricError::SchemeNotAllowed(scheme.into()));
    }
    let lan_ok = LAN_OK_SCHEMES.contains(&scheme);
    match scheme {
        "http" | "https" => {}
        _ if lan_ok => {}
        s => return Err(FerricError::SchemeNotAllowed(s.into())),
    }

    let host = url
        .host_str()
        .ok_or_else(|| FerricError::InvalidUrl("URL has no host".into()))?;

    // Loopback / mDNS blocked for browser-captured http(s); LAN protocols may use them.
    if !lan_ok
        && (host.eq_ignore_ascii_case("localhost") || host.to_ascii_lowercase().ends_with(".local"))
    {
        return Err(FerricError::SsrfBlocked(host.into()));
    }

    // Cloud metadata / SSRF classics (Task 128)
    if !lan_ok && is_metadata_host(host) {
        return Err(FerricError::SsrfBlocked(host.into()));
    }

    // Private IP block — http(s) only (SSRF via extension). FTP/SFTP/WebDAV to NAS is intentional.
    if !lan_ok {
        match url.host() {
            Some(url::Host::Ipv4(addr)) => {
                if is_private_ip(IpAddr::V4(addr)) {
                    return Err(FerricError::SsrfBlocked(host.into()));
                }
            }
            Some(url::Host::Ipv6(addr)) => {
                if is_private_ip(IpAddr::V6(addr)) {
                    return Err(FerricError::SsrfBlocked(host.into()));
                }
            }
            Some(url::Host::Domain(domain)) => {
                let port = url
                    .port()
                    .unwrap_or(if url.scheme() == "https" { 443 } else { 80 });
                use std::net::ToSocketAddrs as _;
                // Fail closed on empty answers; on DNS *errors* still fail closed outside tests
                // (tests may run offline / with flaky resolvers).
                match (domain, port).to_socket_addrs() {
                    Ok(addrs) => {
                        let mut saw_any = false;
                        for addr in addrs {
                            saw_any = true;
                            if is_private_ip(addr.ip()) {
                                return Err(FerricError::SsrfBlocked(domain.to_string()));
                            }
                        }
                        if !saw_any {
                            return Err(FerricError::SsrfBlocked(format!(
                                "{domain} (DNS returned no addresses)"
                            )));
                        }
                    }
                    Err(_) if cfg!(test) => {}
                    Err(_) => {
                        return Err(FerricError::SsrfBlocked(format!(
                            "{domain} (DNS resolution failed)"
                        )));
                    }
                }
            }
            None => return Err(FerricError::InvalidUrl("URL has no host".into())),
        }
    } else if url.host().is_none() {
        return Err(FerricError::InvalidUrl("URL has no host".into()));
    }

    Ok(url)
}

/// Return a URL suitable for diagnostics. It preserves scheme/host/path but
/// removes query strings, fragments, and embedded credentials.
pub fn redact_url(raw: &str) -> String {
    if let Ok(mut url) = Url::parse(raw) {
        if !url.username().is_empty() {
            let _ = url.set_username("redacted");
        }
        if url.password().is_some() {
            let _ = url.set_password(Some("redacted"));
        }
        if url.query().is_some() {
            url.set_query(Some("redacted"));
        }
        if url.fragment().is_some() {
            url.set_fragment(Some("redacted"));
        }
        return url.to_string();
    }

    let visible = raw.split(['?', '#']).next().unwrap_or(raw);
    if visible == raw {
        visible.to_string()
    } else {
        format!("{visible}?redacted")
    }
}

/// Validate a save-path before accepting it from the UI or API.
/// Non-empty paths must be absolute; reject relative paths, URIs, and garbage.  (S12)
pub fn validate_save_path(raw: &str) -> Result<(), FerricError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(()); // empty → caller falls back to default
    }
    let p = Path::new(trimmed);
    if p.is_absolute() {
        Ok(())
    } else {
        Err(FerricError::InvalidSavePath(format!(
            "save path must be absolute, got: {trimmed}"
        )))
    }
}

/// Cut `name` to at most `max` bytes, keeping its extension (and never splitting a character).
fn truncate_keeping_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let ext = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if ext.is_empty() || ext.len() + 1 >= max {
        cut_at_char_boundary(name, max).to_string()
    } else {
        format!("{}.{ext}", cut_at_char_boundary(name, max - ext.len() - 1))
    }
}

/// `s` cut to at most `max` bytes without splitting a multi-byte character
/// (slicing mid-character panics, and file names come from the server).
fn cut_at_char_boundary(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Sanitize a raw filename from a server and join it safely with `save_dir`.
pub fn safe_filename(raw: &str, save_dir: &Path) -> Result<PathBuf, FerricError> {
    if has_path_component(raw) {
        return Err(FerricError::PathTraversal { raw: raw.into() });
    }

    // 1. Take only the basename (strip any path component)
    let name = raw.rsplit(['/', '\\']).next().unwrap_or("download");

    // Strip query string if the filename came from a URL path segment
    let name = name.split('?').next().unwrap_or(name);
    // Strip Unicode directional-override chars before any further processing
    let name_stripped = strip_rtl_overrides(name);
    let name = name_stripped.as_str();
    let name = if is_windows_reserved_stem(name) {
        format!("_{name}")
    } else {
        name.to_string()
    };

    // 2. Sanitize: remove null bytes, control chars, path separators. Truncate first:
    // `sanitize` cuts at 255 bytes and would drop the extension of a long name.
    let name = sanitize(&truncate_keeping_extension(&name, 200));

    // 3. Fallback if empty
    let name = if name.is_empty() || name == "." || name == ".." {
        "download".to_string()
    } else {
        name
    };

    // 4. Strip leading dots (hidden files on Unix, also a common traversal trick)
    let name = name
        .trim_start_matches('.')
        .trim_end_matches(['.', ' '])
        .to_string();
    let name = if name.is_empty() {
        "download".to_string()
    } else {
        name
    };

    // 5. Windows reserved name check (case-insensitive, ignore extension)
    let name = if is_windows_reserved_stem(&name) {
        format!("_{name}")
    } else {
        name
    };

    // 6. Backstop: a reserved-name prefix or replacement could have grown the name.
    let name = truncate_keeping_extension(&name, 200);

    // 7. Build candidate path
    let candidate = save_dir.join(&name);

    // 8. Path traversal check — ensure parent stays within save_dir
    // We canonicalize the save_dir itself; candidate may not exist yet so we
    // only canonicalize its parent.
    let save_dir_canon = save_dir
        .canonicalize()
        .map_err(|e| FerricError::Io(e.to_string()))?;

    let candidate_parent = candidate.parent().unwrap_or(&candidate);
    if candidate_parent.exists() {
        let parent_canon = candidate_parent
            .canonicalize()
            .map_err(|e| FerricError::Io(e.to_string()))?;
        if !parent_canon.starts_with(&save_dir_canon) {
            return Err(FerricError::PathTraversal { raw: raw.into() });
        }
    } else if !candidate.starts_with(&save_dir_canon) {
        // Parent doesn't exist — do a lexical check
        return Err(FerricError::PathTraversal { raw: raw.into() });
    }

    Ok(candidate)
}

fn has_path_component(raw: &str) -> bool {
    raw.contains('/') || raw.contains('\\')
}

fn is_windows_reserved_stem(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or(name)
        .trim_matches(|c: char| c == ' ' || c == '.')
        .to_uppercase();
    WIN_RESERVED.contains(&stem.as_str())
}

/// Returns `true` if the file extension is potentially dangerous.
/// Advisory only — callers should warn the user, not block.
pub fn is_dangerous_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| DANGEROUS_EXTENSIONS.contains(&e.to_lowercase().as_str()))
}

/// Strip Unicode directional-override / zero-width chars that can be used to
/// disguise a filename extension in the UI (e.g. U+202E right-to-left override).
pub fn strip_rtl_overrides(name: &str) -> String {
    name.chars()
        .filter(|&c| {
            !matches!(c,
                '\u{200B}' | '\u{200E}' | '\u{200F}' | // zero-width, LRM, RLM
                '\u{202A}'..='\u{202E}' |               // directional embeddings/overrides
                '\u{2066}'..='\u{2069}' |               // directional isolates
                '\u{FEFF}'                               // BOM
            ) && !c.is_control()
        })
        .collect()
}

/// Detect S3 / `CloudFront` / Azure SAS / GCS pre-signed URLs by query parameters.
/// These URLs are time-limited; a 403 on one means expiry, not a policy block.
pub fn is_signed_url_str(raw: &str) -> bool {
    let Ok(url) = Url::parse(raw) else {
        return false;
    };
    let keys: HashSet<String> = url
        .query_pairs()
        .map(|(k, _)| k.to_ascii_lowercase())
        .collect();
    // S3 pre-signed (AWS Signature V4)
    keys.contains("x-amz-signature")
    // S3 / CloudFront (legacy V2 + CloudFront signed URLs)
    || (keys.contains("signature") && (keys.contains("awsaccesskeyid") || keys.contains("key-pair-id")))
    // Azure SAS
    || (keys.contains("sig") && keys.contains("sv"))
    // GCS signed
    || keys.contains("x-goog-signature")
}

/// Hostnames that must never be contacted from download/extract paths (Task 128).
fn is_metadata_host(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "metadata.google.internal"
        || h == "metadata"
        || h.ends_with(".metadata.google.internal")
        || h == "169.254.169.254"
        || h == "metadata.azure.com"
        || h == "instance-data"
}

/// User preference "Allow downloads from my local network" (off by default).
static ALLOW_LAN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_allow_lan(on: bool) {
    ALLOW_LAN.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// The ranges a home NAS or router lives in. Loopback, link-local, CGNAT and the cloud metadata
/// address are never part of this, so the preference cannot reach the daemon's own API or metadata.
fn is_lan_range(addr: IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            o[0] == 10 || (o[0] == 172 && (16..=31).contains(&o[1])) || (o[0] == 192 && o[1] == 168)
        }
        IpAddr::V6(v6) => {
            (v6.segments()[0] & 0xfe00) == 0xfc00
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_lan_range(IpAddr::V4(v4)))
        }
    }
}

pub fn is_private_ip(addr: IpAddr) -> bool {
    is_private_ip_with(addr, ALLOW_LAN.load(std::sync::atomic::Ordering::Relaxed))
}

fn is_private_ip_with(addr: IpAddr, allow_lan: bool) -> bool {
    if allow_lan && is_lan_range(addr) {
        return false;
    }
    // Dev-only escape hatch — never honor in release builds.
    if cfg!(debug_assertions)
        && std::env::var("FERRIC_ALLOW_PRIVATE_IPS").ok().as_deref() == Some("1")
    {
        return false;
    }
    match addr {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            // 10.0.0.0/8
            o[0] == 10
            // 172.16.0.0/12
            || (o[0] == 172 && (16..=31).contains(&o[1]))
            // 192.168.0.0/16
            || (o[0] == 192 && o[1] == 168)
            // 127.0.0.0/8 — loopback
            || o[0] == 127
            // 169.254.0.0/16 — link-local
            || (o[0] == 169 && o[1] == 254)
            // 0.0.0.0/8
            || o[0] == 0
            // 100.64.0.0/10 — CGNAT
            || (o[0] == 100 && (64..=127).contains(&o[1]))
            || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
            || o[0] >= 240
        }
        IpAddr::V6(v6) => {
            // ::1 loopback
            v6.is_loopback()
            // fc00::/7 — unique local
            || (v6.segments()[0] & 0xfe00) == 0xfc00
            // fe80::/10 — link-local
            || (v6.segments()[0] & 0xffc0) == 0xfe80
            || v6
                .to_ipv4_mapped()
                .is_some_and(|v4| is_private_ip_with(IpAddr::V4(v4), allow_lan))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn tmpdir() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn lan_preference_opens_only_the_home_network_ranges() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        for lan in ["192.168.1.20", "10.0.0.5", "172.20.0.1", "fd12::1"] {
            assert!(
                is_private_ip_with(ip(lan), false),
                "{lan} blocked by default"
            );
            assert!(
                !is_private_ip_with(ip(lan), true),
                "{lan} allowed once the user opts in"
            );
        }
        for never in [
            "127.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
        ] {
            assert!(is_private_ip_with(ip(never), true), "{never} stays blocked");
        }
    }

    // ── validate_url ─────────────────────────────────────────────────────────

    #[test]
    fn rejects_javascript_scheme() {
        assert!(validate_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn rejects_file_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn rejects_cloud_metadata_hosts() {
        assert!(validate_url("http://metadata.google.internal/").is_err());
        assert!(validate_url("http://169.254.169.254/latest/meta-data/").is_err());
    }

    #[test]
    fn rejects_private_ipv4() {
        assert!(validate_url("http://192.168.1.1/file.zip").is_err());
        assert!(validate_url("http://10.0.0.1/file.zip").is_err());
        assert!(validate_url("http://172.20.0.1/file.zip").is_err());
    }

    #[test]
    fn rejects_loopback() {
        assert!(validate_url("http://127.0.0.1/file.zip").is_err());
        assert!(validate_url("http://localhost/file.zip").is_err());
    }

    #[test]
    fn rejects_local_hostname_case_insensitive() {
        assert!(validate_url("http://Printer.LOCAL/file.zip").is_err());
    }

    #[test]
    fn rejects_control_chars_in_url() {
        assert!(validate_url("https://example.com/file\n.zip").is_err());
    }

    #[test]
    fn accepts_public_https() {
        assert!(validate_url("https://example.com/file.zip").is_ok());
        assert!(validate_url("http://download.example.org/big.tar.gz").is_ok());
    }

    #[test]
    fn accepts_file_transfer_schemes_including_lan() {
        assert!(validate_url("sftp://user@nas.example/path/file.bin").is_ok());
        assert!(validate_url("ftp://ftp.example.com/pub/a.zip").is_ok());
        assert!(validate_url("ftps://ftp.example.com/pub/a.zip").is_ok());
        assert!(validate_url("ftps-implicit://ftp.example.com/pub/a.zip").is_ok());
        assert!(validate_url("webdavs://cloud.example/remote.php/dav/f").is_ok());
        // LAN / private OK for transfer schemes (NAS)
        assert!(validate_url("sftp://user@192.168.1.10/data.bin").is_ok());
        assert!(validate_url("ftp://192.168.1.10/file.bin").is_ok());
    }

    #[test]
    fn redacts_url_query_fragment_and_credentials() {
        let redacted = redact_url("https://user:secret@example.com/file.zip?token=abc#frag");
        assert_eq!(
            redacted,
            "https://redacted:redacted@example.com/file.zip?redacted#redacted"
        );
    }

    #[test]
    fn invalid_url_error_redacts_query_tokens() {
        let err = validate_url("https://exa mple.com/file.zip?token=secret")
            .expect_err("invalid URL should be rejected")
            .to_string();
        assert!(!err.contains("token=secret"));
        assert!(err.contains("?redacted"));
    }

    // ── safe_filename ─────────────────────────────────────────────────────────

    #[test]
    fn strips_path_traversal() {
        let dir = tmpdir();
        let result = safe_filename("../../etc/passwd", dir.path());
        assert!(matches!(result, Err(FerricError::PathTraversal { .. })));
    }

    #[test]
    fn rejects_deep_path_traversal() {
        let dir = tmpdir();
        let result = safe_filename("../../../evil", dir.path());
        assert!(matches!(result, Err(FerricError::PathTraversal { .. })));
    }

    #[test]
    fn rejects_null_bytes() {
        let dir = tmpdir();
        let result = safe_filename("file\x00name.zip", dir.path());
        if let Ok(p) = result {
            let name = p.file_name().unwrap().to_string_lossy();
            assert!(!name.contains('\x00'));
        }
    }

    #[test]
    fn long_multibyte_file_names_are_cut_on_a_character_boundary() {
        let dir = tempfile::tempdir().unwrap();
        // 3-byte characters put byte 200 inside a character; this used to panic.
        let name = format!("{}.zip", "€".repeat(100));
        let p = safe_filename(&name, dir.path()).unwrap();
        let file = p.file_name().unwrap().to_str().unwrap().to_string();
        assert!(file.len() <= 200 && file.ends_with(".zip"), "{file}");
        let no_ext = "€".repeat(100);
        assert!(
            safe_filename(&no_ext, dir.path())
                .unwrap()
                .file_name()
                .unwrap()
                .len()
                <= 200
        );
    }

    #[test]
    fn prefixes_windows_reserved_con() {
        let dir = tmpdir();
        let p = safe_filename("CON.exe", dir.path()).unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert_eq!(name, "_CON.exe");
    }

    #[test]
    fn prefixes_windows_reserved_nul() {
        let dir = tmpdir();
        let p = safe_filename("NUL", dir.path()).unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert_eq!(name, "_NUL");
    }

    #[test]
    fn handles_empty_filename() {
        let dir = tmpdir();
        let p = safe_filename("", dir.path()).unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "download");
    }

    #[test]
    fn strips_leading_dots() {
        let dir = tmpdir();
        let p = safe_filename(".hidden.zip", dir.path()).unwrap();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        assert!(!name.starts_with('.'));
    }

    #[test]
    fn strips_trailing_dots_and_spaces() {
        let dir = tmpdir();
        let p = safe_filename("report.txt.  ", dir.path()).unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "report.txt");
    }

    #[test]
    fn truncates_long_filename() {
        let dir = tmpdir();
        let long = "a".repeat(300) + ".zip";
        let p = safe_filename(&long, dir.path()).unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(name.len() <= 210, "expected ≤210 chars, got {}", name.len());
    }

    #[test]
    fn normal_filename_passes_through() {
        let dir = tmpdir();
        let p = safe_filename("ubuntu-24.04.iso", dir.path()).unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "ubuntu-24.04.iso");
    }

    #[test]
    fn unicode_filename_passes_through_safely() {
        let dir = tmpdir();
        let p = safe_filename("résumé-東京.txt", dir.path()).unwrap();
        assert_eq!(p.file_name().unwrap().to_string_lossy(), "résumé-東京.txt");
    }

    #[test]
    fn dangerous_extension_is_advisory() {
        assert!(is_dangerous_extension(Path::new("installer.exe")));
        assert!(is_dangerous_extension(Path::new("script.PS1")));
        assert!(!is_dangerous_extension(Path::new("archive.zip")));
    }

    // ── is_private_ip ─────────────────────────────────────────────────────────

    #[test]
    fn private_ranges() {
        use std::net::Ipv4Addr;
        assert!(is_private_ip("10.0.0.1".parse().unwrap()));
        assert!(is_private_ip("192.168.1.100".parse().unwrap()));
        assert!(is_private_ip("172.16.0.1".parse().unwrap()));
        assert!(is_private_ip("127.0.0.1".parse().unwrap()));
        assert!(is_private_ip("100.64.1.1".parse().unwrap()));
        assert!(is_private_ip("198.18.0.1".parse().unwrap()));
        assert!(is_private_ip("240.0.0.1".parse().unwrap()));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
        assert!(!is_private_ip(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))));
    }

    #[test]
    fn private_ipv6_ranges() {
        assert!(is_private_ip("::1".parse().unwrap()));
        assert!(is_private_ip("fc00::1".parse().unwrap()));
        assert!(is_private_ip("fe80::1".parse().unwrap()));
        assert!(is_private_ip("::ffff:192.168.1.1".parse().unwrap()));
        assert!(is_private_ip("::ffff:127.0.0.1".parse().unwrap()));
        assert!(validate_url("http://[::ffff:192.168.1.1]/").is_err());
        assert!(!is_private_ip("::ffff:8.8.8.8".parse().unwrap()));
        assert!(!is_private_ip("2606:4700:4700::1111".parse().unwrap()));
    }

    // ── strip_rtl_overrides ───────────────────────────────────────────────────

    #[test]
    fn rtl_override_stripped_from_filename() {
        // U+202E disguises a filename extension in some UI renderers
        let tricky = "safe.fdp\u{202E}exe.bat";
        let clean = strip_rtl_overrides(tricky);
        assert!(!clean.contains('\u{202E}'));
        assert_eq!(clean, "safe.fdpexe.bat");
    }

    #[test]
    fn rtl_strip_removes_all_directional_chars() {
        let s = "\u{200B}\u{200E}\u{200F}\u{202A}\u{202B}\u{202C}\u{202D}\u{202E}\u{2066}\u{2067}\u{2068}\u{2069}\u{FEFF}clean";
        assert_eq!(strip_rtl_overrides(s), "clean");
    }

    #[test]
    fn safe_filename_strips_rtl_override() {
        let dir = tmpdir();
        let p = safe_filename("file\u{202E}exe.txt", dir.path()).unwrap();
        let name = p.file_name().unwrap().to_string_lossy();
        assert!(!name.contains('\u{202E}'));
    }

    // ── is_signed_url_str ─────────────────────────────────────────────────────

    #[test]
    fn detects_s3_v4_presigned() {
        assert!(is_signed_url_str(
            "https://bucket.s3.amazonaws.com/file.zip?X-Amz-Signature=abc&X-Amz-Expires=300"
        ));
    }

    #[test]
    fn detects_cloudfront_signed() {
        assert!(is_signed_url_str(
            "https://cdn.example.com/file.zip?Signature=abc&Key-Pair-Id=KXYZ&Policy=base64"
        ));
    }

    #[test]
    fn detects_azure_sas() {
        assert!(is_signed_url_str(
            "https://storage.blob.core.windows.net/c/f.zip?sv=2023-01-03&sr=b&sig=xyz"
        ));
    }

    #[test]
    fn plain_url_not_signed() {
        assert!(!is_signed_url_str("https://example.com/file.zip"));
        assert!(!is_signed_url_str("https://example.com/file.zip?token=abc"));
    }

    #[test]
    fn private_ip_env_override_only_applies_to_debug_builds() {
        std::env::set_var("FERRIC_ALLOW_PRIVATE_IPS", "1");
        // The override is a dev-only escape hatch: honoured in debug, ignored in release.
        assert_eq!(
            is_private_ip("192.168.1.1".parse().unwrap()),
            !cfg!(debug_assertions)
        );
        std::env::remove_var("FERRIC_ALLOW_PRIVATE_IPS");
    }

    #[test]
    fn rejects_domain_resolving_to_private_ip() {
        use std::net::ToSocketAddrs as _;
        if let Ok(addrs) = ("127.0.0.1.nip.io", 80).to_socket_addrs() {
            if addrs.into_iter().any(|a| is_private_ip(a.ip())) {
                assert!(validate_url("http://127.0.0.1.nip.io/file.zip").is_err());
            }
        }
    }

    // ── validate_save_path (S12) ───────────────────────────────────────────

    #[test]
    fn save_path_empty_is_ok() {
        assert!(validate_save_path("").is_ok());
        assert!(validate_save_path("  ").is_ok());
    }

    #[test]
    #[cfg(windows)]
    fn save_path_absolute_is_ok() {
        assert!(validate_save_path("C:\\Downloads").is_ok());
        assert!(validate_save_path("D:/Torrents").is_ok());
        assert!(validate_save_path("\\\\server\\share").is_ok()); // UNC
    }

    #[test]
    #[cfg(not(windows))]
    fn save_path_absolute_is_ok() {
        assert!(validate_save_path("/downloads").is_ok());
        assert!(validate_save_path("/mnt/data/torrents").is_ok());
    }

    #[test]
    fn save_path_relative_rejected() {
        assert!(validate_save_path("relative/path").is_err());
        assert!(validate_save_path("Downloads").is_err());
    }

    #[test]
    fn save_path_magnet_uri_rejected() {
        assert!(validate_save_path("magnet:?xt=urn:btih:abc123").is_err());
    }

    #[test]
    fn save_path_url_rejected() {
        assert!(validate_save_path("https://example.com/file.zip").is_err());
    }
}
