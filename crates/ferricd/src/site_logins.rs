//! IDM's "Sites Logins": credentials kept per server (optionally `server/path`) and used for any
//! download from there that does not carry its own `user:pass@` in the URL.
use anyhow::{Context, Result};
use base64::Engine as _;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SiteLogin {
    pub id: String,
    pub server: String,
    pub username: String,
}

#[cfg(windows)]
fn protect(plain: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: plain.len() as u32,
        pbData: plain.as_ptr().cast_mut(),
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: `input` points at `plain` for the call; `out` is filled by the API and freed below.
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut out,
        )
    };
    anyhow::ensure!(ok != 0, "CryptProtectData failed");
    // SAFETY: the API returned `cbData` bytes at `pbData`, which we copy and then release.
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData.cast()) };
    Ok(bytes)
}

#[cfg(windows)]
fn unprotect(blob: &[u8]) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};
    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr().cast_mut(),
    };
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // SAFETY: as in `protect`.
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut out,
        )
    };
    anyhow::ensure!(ok != 0, "CryptUnprotectData failed");
    let bytes = unsafe { std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec() };
    unsafe { LocalFree(out.pbData.cast()) };
    Ok(bytes)
}

// shortcut: no OS key store off Windows, so site logins are refused there; add libsecret/Keychain if a port needs them.
#[cfg(not(windows))]
fn protect(_: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!("stored site logins need the Windows credential key")
}
#[cfg(not(windows))]
fn unprotect(_: &[u8]) -> Result<Vec<u8>> {
    anyhow::bail!("stored site logins need the Windows credential key")
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<SiteLogin>> {
    let rows = sqlx::query("SELECT id, server, username FROM site_logins ORDER BY server")
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|r| {
            Ok(SiteLogin {
                id: r.try_get("id")?,
                server: r.try_get("server")?,
                username: r.try_get("username")?,
            })
        })
        .collect()
}

/// Add or replace the login of `server`.
pub async fn upsert(pool: &SqlitePool, server: &str, username: &str, password: &str) -> Result<()> {
    let server = normalize_server(server);
    anyhow::ensure!(
        !server.is_empty() && !username.is_empty(),
        "server and user name are required"
    );
    let enc = base64::engine::general_purpose::STANDARD.encode(protect(password.as_bytes())?);
    sqlx::query(
        "INSERT INTO site_logins (id, server, username, password_enc) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(server) DO UPDATE SET username = excluded.username, password_enc = excluded.password_enc",
    )
    .bind(Uuid::new_v4().to_string())
    .bind(server)
    .bind(username)
    .bind(enc)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<bool> {
    Ok(sqlx::query("DELETE FROM site_logins WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?
        .rows_affected()
        > 0)
}

fn normalize_server(s: &str) -> String {
    let s = s
        .trim()
        .trim_start_matches("http://")
        .trim_start_matches("https://");
    s.trim_end_matches('/').to_ascii_lowercase()
}

/// `*` matches any run of characters; everything else is literal.
pub(crate) fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == text;
    }
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if i == 0 {
            let Some(r) = rest.strip_prefix(part) else {
                return false;
            };
            rest = r;
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            let Some(at) = rest.find(part) else {
                return false;
            };
            rest = &rest[at + part.len()..];
        }
    }
    true
}

/// Does the stored `server` (host, `host:port`, or `host/path`, `*` allowed) cover this URL?
pub fn covers(server: &str, url: &url::Url) -> bool {
    let host = url.host_str().unwrap_or("").to_ascii_lowercase();
    let with_port = url
        .port()
        .map_or_else(|| host.clone(), |p| format!("{host}:{p}"));
    let path = url.path().to_ascii_lowercase();
    if server.contains('/') {
        let target = format!("{with_port}{path}");
        let target_default = format!("{host}{path}");
        let pat = format!("{server}*");
        let pat = if server.contains('*') {
            server.to_string()
        } else {
            pat
        };
        glob(&pat, &target) || glob(&pat, &target_default)
    } else {
        glob(server, &host) || glob(server, &with_port)
    }
}

/// `url` with the stored credentials of a matching server inserted, if there is one. The result is
/// for requests only; it must not be saved back to the database.
pub async fn apply(pool: &SqlitePool, url: &str) -> Option<String> {
    let mut parsed = url::Url::parse(url).ok()?;
    if !parsed.username().is_empty() {
        return None;
    }
    let rows = sqlx::query("SELECT server, username, password_enc FROM site_logins")
        .fetch_all(pool)
        .await
        .ok()?;
    // Longest (most specific) server pattern wins.
    let best = rows
        .iter()
        .filter_map(|r| {
            let server: String = r.try_get("server").ok()?;
            covers(&server, &parsed).then_some((server.len(), r))
        })
        .max_by_key(|(len, _)| *len)?
        .1;
    let username: String = best.try_get("username").ok()?;
    let enc: String = best.try_get("password_enc").ok()?;
    let password = base64::engine::general_purpose::STANDARD
        .decode(enc)
        .context("bad stored login")
        .and_then(|b| unprotect(&b))
        .ok()
        .and_then(|b| String::from_utf8(b).ok())?;
    parsed.set_username(&username).ok()?;
    parsed.set_password(Some(&password)).ok()?;
    Some(parsed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> url::Url {
        url::Url::parse(s).unwrap()
    }

    #[test]
    fn server_patterns_cover_the_right_urls() {
        assert!(covers("example.com", &u("http://example.com/a/b")));
        assert!(covers("*.example.com", &u("http://files.example.com/x")));
        assert!(covers("example.com:8080", &u("http://example.com:8080/x")));
        assert!(!covers("example.com:8080", &u("http://example.com/x")));
        assert!(covers(
            "example.com/private",
            &u("http://example.com/private/file.zip")
        ));
        assert!(!covers(
            "example.com/private",
            &u("http://example.com/public/file.zip")
        ));
        assert!(!covers("example.com", &u("http://evil-example.com/x")));
    }

    #[cfg(windows)]
    #[tokio::test]
    async fn login_is_stored_encrypted_and_applied() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        upsert(&pool, "http://Files.Example.com/", "alice", "s3cret").await?;
        let stored: String = sqlx::query_scalar("SELECT password_enc FROM site_logins")
            .fetch_one(&pool)
            .await?;
        assert!(!stored.contains("s3cret"));
        assert_eq!(list(&pool).await?[0].server, "files.example.com");
        let with = apply(&pool, "http://files.example.com/a.bin")
            .await
            .expect("applied");
        assert_eq!(with, "http://alice:s3cret@files.example.com/a.bin");
        assert!(apply(&pool, "http://other.example.com/a.bin")
            .await
            .is_none());
        assert!(
            apply(&pool, "http://bob:x@files.example.com/a.bin")
                .await
                .is_none(),
            "URL credentials win"
        );
        Ok(())
    }
}
