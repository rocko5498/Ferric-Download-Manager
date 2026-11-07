//! T1 product modes — one setting expands into discovery/listen defaults.
//!
//! Modes (setting key `product_mode`):
//! - `strict`  — no DHT/PEX/LSD, no inbound listen, no default seeding, proxy required
//! - `balanced`— DHT+PEX on, LSD off, outbound-only, seeding off, proxy optional (default)
//! - `open`    — full discovery + inbound listen, default seeding on, proxy optional
//!
//! Individual toggles remain writable; setting `product_mode` overwrites the constellation
//! so "mode on UI" never lies about discovery/listen.

use std::collections::HashMap;

use anyhow::Result;
use sqlx::SqlitePool;

pub const PRODUCT_MODE_KEY: &str = "product_mode";

/// Keys written when a mode is applied (also used to document UI consequences).
pub fn mode_settings(mode: &str) -> Option<HashMap<&'static str, &'static str>> {
    let mut m = HashMap::new();
    match mode.trim().to_ascii_lowercase().as_str() {
        "strict" => {
            m.insert("product_mode", "strict");
            m.insert("torrent_dht_enabled", "0");
            m.insert("torrent_pex_enabled", "0");
            m.insert("torrent_lsd_enabled", "0");
            m.insert("torrent_tcp_listen_enabled", "0");
            m.insert("torrent_proxy_required", "1");
            m.insert("torrent_seeding_default", "0");
        }
        "balanced" => {
            m.insert("product_mode", "balanced");
            m.insert("torrent_dht_enabled", "1");
            m.insert("torrent_pex_enabled", "1");
            m.insert("torrent_lsd_enabled", "0");
            m.insert("torrent_tcp_listen_enabled", "0");
            m.insert("torrent_proxy_required", "0");
            m.insert("torrent_seeding_default", "0");
        }
        "open" => {
            m.insert("product_mode", "open");
            m.insert("torrent_dht_enabled", "1");
            m.insert("torrent_pex_enabled", "1");
            m.insert("torrent_lsd_enabled", "1");
            m.insert("torrent_tcp_listen_enabled", "1");
            m.insert("torrent_proxy_required", "0");
            m.insert("torrent_seeding_default", "1");
        }
        _ => return None,
    }
    Some(m)
}

pub fn normalize_mode(raw: &str) -> Option<&'static str> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "strict" => Some("strict"),
        "balanced" => Some("balanced"),
        "open" => Some("open"),
        _ => None,
    }
}

/// Persist mode constellation. Returns number of public torrents whose policy was updated.
pub async fn apply_mode(pool: &SqlitePool, mode: &str) -> Result<usize> {
    let mode = normalize_mode(mode)
        .ok_or_else(|| anyhow::anyhow!("unknown product_mode (use strict|balanced|open)"))?;
    let settings = mode_settings(mode).expect("normalized mode always has settings");

    for (k, v) in &settings {
        crate::db::settings::set(pool, k, v).await?;
    }

    let dht = *settings.get("torrent_dht_enabled").unwrap_or(&"0") == "1";
    let pex = *settings.get("torrent_pex_enabled").unwrap_or(&"0") == "1";
    let lsd = *settings.get("torrent_lsd_enabled").unwrap_or(&"0") == "1";
    let proxy_required = *settings.get("torrent_proxy_required").unwrap_or(&"0") == "1";
    let seed_default = *settings.get("torrent_seeding_default").unwrap_or(&"0") == "1";

    let all = crate::db::torrents::list(pool).await?;
    let mut updated = 0usize;
    for mut rec in all {
        if rec.is_private {
            // Private torrents never get DHT/PEX/LSD; still honor proxy_required + seed default.
            let mut changed = false;
            if rec.policy.proxy_required != proxy_required {
                rec.policy.proxy_required = proxy_required;
                changed = true;
            }
            if rec.policy.seed_enabled != seed_default {
                rec.policy.seed_enabled = seed_default;
                changed = true;
            }
            if changed {
                crate::db::torrents::upsert_record(pool, &rec).await?;
                updated += 1;
            }
            continue;
        }
        let mut changed = false;
        if rec.policy.dht_enabled != dht {
            rec.policy.dht_enabled = dht;
            changed = true;
        }
        if rec.policy.pex_enabled != pex {
            rec.policy.pex_enabled = pex;
            changed = true;
        }
        if rec.policy.lsd_enabled != lsd {
            rec.policy.lsd_enabled = lsd;
            changed = true;
        }
        if rec.policy.proxy_required != proxy_required {
            rec.policy.proxy_required = proxy_required;
            changed = true;
        }
        if rec.policy.seed_enabled != seed_default {
            rec.policy.seed_enabled = seed_default;
            changed = true;
        }
        if changed {
            crate::db::torrents::upsert_record(pool, &rec).await?;
            updated += 1;
        }
    }

    tracing::info!(
        mode,
        torrents_updated = updated,
        "applied product_mode constellation"
    );
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_disables_discovery_and_listen() {
        let s = mode_settings("strict").unwrap();
        assert_eq!(s["torrent_dht_enabled"], "0");
        assert_eq!(s["torrent_pex_enabled"], "0");
        assert_eq!(s["torrent_lsd_enabled"], "0");
        assert_eq!(s["torrent_tcp_listen_enabled"], "0");
        assert_eq!(s["torrent_proxy_required"], "1");
    }

    #[test]
    fn balanced_is_default_shape() {
        let s = mode_settings("Balanced").unwrap();
        assert_eq!(s["product_mode"], "balanced");
        assert_eq!(s["torrent_dht_enabled"], "1");
        assert_eq!(s["torrent_tcp_listen_enabled"], "0");
        assert_eq!(s["torrent_proxy_required"], "0");
    }

    #[test]
    fn open_enables_listen_and_lsd() {
        let s = mode_settings("open").unwrap();
        assert_eq!(s["torrent_lsd_enabled"], "1");
        assert_eq!(s["torrent_tcp_listen_enabled"], "1");
        assert_eq!(s["torrent_seeding_default"], "1");
    }

    #[test]
    fn unknown_mode_rejected() {
        assert!(mode_settings("turbo").is_none());
        assert!(normalize_mode("nope").is_none());
    }
}
