use anyhow::{Context, Result};
use ferric_common::{ProxyConfigInput, ProxyConfigRecord, ProxyKind};
use ring::aead::{self, Aad, LessSafeKey, Nonce, UnboundKey};
use ring::rand::{SecureRandom, SystemRandom};
use sqlx::{Row, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const NONCE_LEN: usize = 12;
const AES_256_KEY_LEN: usize = 32;
const KEY_SETTING: &str = "proxy_secret_key_hex";

#[derive(Clone, PartialEq, Debug)]
pub struct ProxyRuntimeConfig {
    pub id: String,
    pub proxy_type: ProxyKind,
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<ProxyConfigRecord>> {
    let rows = sqlx::query("SELECT * FROM proxy_configs ORDER BY created_at DESC")
        .fetch_all(pool)
        .await?;
    rows.into_iter().map(record_from_row).collect()
}

pub async fn get(pool: &SqlitePool, id: &str) -> Result<Option<ProxyConfigRecord>> {
    let row = sqlx::query("SELECT * FROM proxy_configs WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?;
    row.map(record_from_row).transpose()
}

pub async fn get_runtime(pool: &SqlitePool, id: &str) -> Result<Option<ProxyRuntimeConfig>> {
    let Some(row) = sqlx::query("SELECT * FROM proxy_configs WHERE id = ?1")
        .bind(id)
        .fetch_optional(pool)
        .await?
    else {
        return Ok(None);
    };
    runtime_from_row(pool, row).await.map(Some)
}

/// Parse a settings `torrent_proxy_url` (socks5://host:port or http://…) into engine config.
/// Best-effort SOCKS for managed rqbit — not a packet-verified privacy claim.
pub fn runtime_from_proxy_url(raw: &str) -> Result<ProxyRuntimeConfig> {
    let url = url::Url::parse(raw.trim()).context("invalid torrent_proxy_url")?;
    let scheme = url.scheme().to_ascii_lowercase();
    let proxy_type = match scheme.as_str() {
        "socks5" | "socks5h" | "socks" => ProxyKind::Socks5,
        "http" | "https" => ProxyKind::Http,
        other => anyhow::bail!("unsupported torrent_proxy_url scheme: {other}"),
    };
    let host = url
        .host_str()
        .filter(|h| !h.is_empty())
        .ok_or_else(|| anyhow::anyhow!("torrent_proxy_url missing host"))?
        .to_string();
    let port = url
        .port_or_known_default()
        .ok_or_else(|| anyhow::anyhow!("torrent_proxy_url missing port"))?;
    let username = {
        let u = url.username();
        if u.is_empty() {
            None
        } else {
            Some(u.to_string())
        }
    };
    let password = url.password().map(str::to_string);
    Ok(ProxyRuntimeConfig {
        id: "settings:torrent_proxy_url".to_string(),
        proxy_type,
        host,
        port,
        username,
        password,
    })
}

pub async fn upsert(pool: &SqlitePool, input: &ProxyConfigInput) -> Result<ProxyConfigRecord> {
    validate_input(input)?;
    let id = Uuid::new_v4().to_string();
    let (password_enc, nonce) = match input.password.as_deref().filter(|value| !value.is_empty()) {
        Some(password) => {
            let key = load_or_create_key(pool).await?;
            let nonce = random_nonce()?;
            let mut data = password.as_bytes().to_vec();
            key.seal_in_place_append_tag(
                Nonce::assume_unique_for_key(nonce),
                Aad::empty(),
                &mut data,
            )
            .map_err(|_| anyhow::anyhow!("proxy password encryption failed"))?;
            (Some(data), Some(nonce.to_vec()))
        }
        None => (None, None),
    };

    sqlx::query(
        r"
        INSERT INTO proxy_configs (
            id, name, proxy_type, host, port, username, password_enc, nonce, created_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        ",
    )
    .bind(&id)
    .bind(input.name.trim())
    .bind(kind_to_str(&input.proxy_type))
    .bind(input.host.trim())
    .bind(i64::from(input.port))
    .bind(
        input
            .username
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty()),
    )
    .bind(password_enc)
    .bind(nonce)
    .bind(unix_millis()?)
    .execute(pool)
    .await?;

    get(pool, &id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("proxy config missing after insert"))
}

pub async fn delete(pool: &SqlitePool, id: &str) -> Result<()> {
    sqlx::query("DELETE FROM proxy_configs WHERE id = ?1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

fn validate_input(input: &ProxyConfigInput) -> Result<()> {
    if input.name.trim().is_empty() {
        anyhow::bail!("proxy name is required");
    }
    if input.host.trim().is_empty() {
        anyhow::bail!("proxy host is required");
    }
    if input.port == 0 {
        anyhow::bail!("proxy port is required");
    }
    Ok(())
}

fn record_from_row(row: sqlx::sqlite::SqliteRow) -> Result<ProxyConfigRecord> {
    let proxy_type: String = row.try_get("proxy_type")?;
    let port: i64 = row.try_get("port")?;
    let created_at: i64 = row.try_get("created_at")?;
    let password_enc: Option<Vec<u8>> = row.try_get("password_enc")?;
    Ok(ProxyConfigRecord {
        id: row.try_get("id")?,
        name: row.try_get("name")?,
        proxy_type: kind_from_str(&proxy_type)?,
        host: row.try_get("host")?,
        port: u16::try_from(port).context("proxy port out of range")?,
        username: row.try_get("username")?,
        has_password: password_enc.is_some_and(|value| !value.is_empty()),
        created_at: u64::try_from(created_at).context("proxy created_at out of range")?,
    })
}

async fn runtime_from_row(
    pool: &SqlitePool,
    row: sqlx::sqlite::SqliteRow,
) -> Result<ProxyRuntimeConfig> {
    let proxy_type: String = row.try_get("proxy_type")?;
    let port: i64 = row.try_get("port")?;
    let password_enc: Option<Vec<u8>> = row.try_get("password_enc")?;
    let nonce: Option<Vec<u8>> = row.try_get("nonce")?;
    let password = match (password_enc, nonce) {
        (Some(mut encrypted), Some(nonce)) if !encrypted.is_empty() => {
            Some(decrypt_password(pool, &mut encrypted, &nonce).await?)
        }
        _ => None,
    };

    Ok(ProxyRuntimeConfig {
        id: row.try_get("id")?,
        proxy_type: kind_from_str(&proxy_type)?,
        host: row.try_get("host")?,
        port: u16::try_from(port).context("proxy port out of range")?,
        username: row.try_get("username")?,
        password,
    })
}

async fn decrypt_password(
    pool: &SqlitePool,
    encrypted: &mut Vec<u8>,
    nonce: &[u8],
) -> Result<String> {
    if nonce.len() != NONCE_LEN {
        anyhow::bail!("invalid proxy password nonce length");
    }
    let key_hex = super::settings::get(pool, KEY_SETTING)
        .await?
        .ok_or_else(|| anyhow::anyhow!("proxy encryption key is missing"))?;
    let key = encryption_key(&hex_to_bytes(&key_hex)?)?;
    let nonce = Nonce::try_assume_unique_for_key(nonce)
        .map_err(|_| anyhow::anyhow!("invalid proxy password nonce"))?;
    let plaintext = key
        .open_in_place(nonce, Aad::empty(), encrypted)
        .map_err(|_| anyhow::anyhow!("proxy password decryption failed"))?;
    String::from_utf8(plaintext.to_vec()).context("proxy password is not valid UTF-8")
}

fn kind_to_str(kind: &ProxyKind) -> &'static str {
    match kind {
        ProxyKind::Socks5 => "socks5",
        ProxyKind::Http => "http",
    }
}

fn kind_from_str(value: &str) -> Result<ProxyKind> {
    match value {
        "socks5" => Ok(ProxyKind::Socks5),
        "http" => Ok(ProxyKind::Http),
        other => anyhow::bail!("unknown proxy type {other}"),
    }
}

async fn load_or_create_key(pool: &SqlitePool) -> Result<LessSafeKey> {
    if let Some(hex) = super::settings::get(pool, KEY_SETTING).await? {
        return encryption_key(&hex_to_bytes(&hex)?);
    }
    let mut key = [0_u8; AES_256_KEY_LEN];
    SystemRandom::new()
        .fill(&mut key)
        .map_err(|_| anyhow::anyhow!("failed to generate proxy encryption key"))?;
    super::settings::set(pool, KEY_SETTING, &bytes_to_hex(&key)).await?;
    encryption_key(&key)
}

fn encryption_key(key: &[u8]) -> Result<LessSafeKey> {
    if key.len() != AES_256_KEY_LEN {
        anyhow::bail!("AES-256-GCM proxy key must be {AES_256_KEY_LEN} bytes");
    }
    let unbound = UnboundKey::new(&aead::AES_256_GCM, key)
        .map_err(|_| anyhow::anyhow!("invalid AES-256-GCM proxy key"))?;
    Ok(LessSafeKey::new(unbound))
}

fn random_nonce() -> Result<[u8; NONCE_LEN]> {
    let mut nonce = [0_u8; NONCE_LEN];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| anyhow::anyhow!("failed to generate proxy nonce"))?;
    Ok(nonce)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn hex_to_bytes(hex: &str) -> Result<Vec<u8>> {
    if !hex.len().is_multiple_of(2) {
        anyhow::bail!("invalid proxy key hex length");
    }
    (0..hex.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&hex[index..index + 2], 16).context("invalid proxy key hex")
        })
        .collect()
}

fn unix_millis() -> Result<i64> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("system clock is before Unix epoch")?;
    i64::try_from(elapsed.as_millis()).context("current timestamp exceeds i64 range")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn stores_proxy_password_encrypted_and_redacted() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        let record = upsert(
            &pool,
            &ProxyConfigInput {
                name: "vpn socks".to_string(),
                proxy_type: ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: 1080,
                username: Some("user".to_string()),
                password: Some("secret".to_string()),
            },
        )
        .await?;

        assert!(record.has_password);
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT password_enc FROM proxy_configs WHERE id = ?1")
                .bind(&record.id)
                .fetch_one(&pool)
                .await?;
        assert_ne!(stored, b"secret");
        assert_eq!(list(&pool).await?[0].username.as_deref(), Some("user"));
        Ok(())
    }

    #[tokio::test]
    async fn runtime_proxy_decrypts_password_only_for_engine_path() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let pool = crate::db::init(&dir.path().join("ferric.db")).await?;
        let record = upsert(
            &pool,
            &ProxyConfigInput {
                name: "vpn socks".to_string(),
                proxy_type: ProxyKind::Socks5,
                host: "127.0.0.1".to_string(),
                port: 1080,
                username: Some("user".to_string()),
                password: Some("secret".to_string()),
            },
        )
        .await?;

        let runtime = get_runtime(&pool, &record.id)
            .await?
            .expect("runtime proxy should exist");

        assert_eq!(runtime.username.as_deref(), Some("user"));
        assert_eq!(runtime.password.as_deref(), Some("secret"));
        assert!(list(&pool).await?[0].has_password);
        Ok(())
    }
}
