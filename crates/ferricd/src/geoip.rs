//! Task 182 — peer `GeoIP` + Five/Nine/Fourteen-eyes tiers.
//!
//! Optional MaxMind/dbip MMDB at `{data_dir}/geoip.mmdb` or settings `geoip_mmdb_path`.
//! Without a DB file, country is unknown but private/loopback are classified.

use serde::Serialize;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EyesTier {
    Five,
    Nine,
    Fourteen,
    Other,
    Local,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerGeo {
    pub country_iso: String,
    pub country_name: String,
    pub flag_emoji: String,
    pub eyes_tier: EyesTier,
}

static MMDB_PATH: RwLock<Option<PathBuf>> = RwLock::new(None);

pub fn set_mmdb_path(path: Option<PathBuf>) {
    if let Ok(mut g) = MMDB_PATH.write() {
        *g = path;
    }
}

pub fn configure_from_data_dir(data_dir: &Path) {
    ensure_geoip_placeholder(data_dir);
    let candidate = data_dir.join("geoip.mmdb");
    if candidate.is_file() {
        set_mmdb_path(Some(candidate));
    }
}

/// Log MMDB readiness after default path + settings override.
/// Never auto-downloads commercial/free DBs (license key required for `GeoLite2`).
pub fn log_status(data_dir: &Path) {
    let expected = data_dir.join("geoip.mmdb");
    let configured = MMDB_PATH.read().ok().and_then(|g| g.clone());
    match configured {
        Some(p) if p.is_file() => {
            tracing::info!(path = %p.display(), "geoip: MMDB ready");
        }
        Some(p) => {
            tracing::info!(
                path = %p.display(),
                expected = %expected.display(),
                "geoip: MMDB path set but file missing; peer country/eyes_tier stay unknown (no auto-download)"
            );
        }
        None => {
            tracing::info!(
                expected = %expected.display(),
                setting = "geoip_mmdb_path",
                "geoip: no MMDB at data_dir; place GeoLite2/db-ip Country or City MMDB there or set geoip_mmdb_path (free MaxMind signup for GeoLite2; ferricd does not download DB)"
            );
        }
    }
}

fn ensure_geoip_placeholder(data_dir: &Path) {
    let dir = data_dir.join("geoip");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::debug!(error = %e, "geoip: could not create geoip/ dir");
        return;
    }
    let readme = dir.join("README.md");
    if readme.is_file() {
        return;
    }
    const README: &str = "# Peer GeoIP MMDB\n\
\n\
Place a MaxMind GeoLite2 or db-ip Country/City `.mmdb` as:\n\
\n\
```\n\
../geoip.mmdb\n\
```\n\
\n\
(i.e. next to this folder: `{data_dir}/geoip.mmdb`).\n\
\n\
Or set settings key `geoip_mmdb_path` to an absolute path.\n\
\n\
**ferricd never downloads GeoIP databases.** GeoLite2 needs a free MaxMind\n\
account: https://dev.maxmind.com/geoip/geolite2-free-geolocation-data\n\
\n\
db-ip Lite (free, attribution required): https://db-ip.com/db/lite.php\n\
\n\
Used only for peer country + Five/Nine/Fourteen-eyes tiers. Raw peer IPs\n\
are never logged.\n";
    if let Err(e) = std::fs::write(&readme, README) {
        tracing::debug!(error = %e, "geoip: could not write geoip/README.md");
    }
}

/// Lookup geo for an IP string (`1.2.3.4` or `1.2.3.4:6881` or `[::1]:1`).
pub fn lookup_addr(addr: &str) -> PeerGeo {
    let ip = parse_ip(addr);
    match ip {
        Some(ip) if is_local(ip) => PeerGeo {
            country_iso: "LO".into(),
            country_name: "Local/Private".into(),
            flag_emoji: "🏠".into(),
            eyes_tier: EyesTier::Local,
        },
        Some(ip) => lookup_ip(ip),
        None => unknown(),
    }
}

fn lookup_ip(ip: IpAddr) -> PeerGeo {
    // Optional MMDB via maxminddb if file configured — pure best-effort without hard dep:
    // We decode ISO from a tiny built-in eyes-country map only when country is known from mmdb path.
    // Without mmdb crate at runtime file parse, use heuristic: no country, but still classify unknown.
    if let Some(iso) = try_read_mmdb_iso(ip) {
        return from_iso(&iso);
    }
    unknown()
}

fn try_read_mmdb_iso(ip: IpAddr) -> Option<String> {
    let path = MMDB_PATH.read().ok()?.clone()?;
    let reader = maxminddb::Reader::open_readfile(&path).ok()?;
    // maxminddb 0.27+: lookup → LookupResult; decode via path!
    let result = reader.lookup(ip).ok()?;
    result
        .decode_path(&maxminddb::path!["country", "iso_code"])
        .ok()
        .flatten()
        .or_else(|| {
            result
                .decode_path(&maxminddb::path!["registered_country", "iso_code"])
                .ok()
                .flatten()
        })
}

fn from_iso(iso: &str) -> PeerGeo {
    let iso = iso.to_ascii_uppercase();
    let eyes = eyes_for_iso(&iso);
    let name = country_name(&iso).to_string();
    let flag = flag_emoji(&iso);
    PeerGeo {
        country_iso: iso,
        country_name: name,
        flag_emoji: flag,
        eyes_tier: eyes,
    }
}

fn eyes_for_iso(iso: &str) -> EyesTier {
    match iso {
        "US" | "GB" | "CA" | "AU" | "NZ" => EyesTier::Five,
        "FR" | "DK" | "NL" | "NO" => EyesTier::Nine,
        "DE" | "BE" | "IT" | "SE" | "ES" => EyesTier::Fourteen,
        _ => EyesTier::Other,
    }
}

fn country_name(iso: &str) -> &'static str {
    match iso {
        "US" => "United States",
        "GB" => "United Kingdom",
        "CA" => "Canada",
        "AU" => "Australia",
        "NZ" => "New Zealand",
        "FR" => "France",
        "DE" => "Germany",
        "NL" => "Netherlands",
        "SE" => "Sweden",
        "NO" => "Norway",
        "DK" => "Denmark",
        "BE" => "Belgium",
        "IT" => "Italy",
        "ES" => "Spain",
        _ => "Unknown",
    }
}

fn flag_emoji(iso: &str) -> String {
    let b = iso.as_bytes();
    if b.len() != 2 || !b[0].is_ascii_alphabetic() || !b[1].is_ascii_alphabetic() {
        return "🏳️".into();
    }
    let c1 = 0x1F1E6 + u32::from(b[0].to_ascii_uppercase() - b'A');
    let c2 = 0x1F1E6 + u32::from(b[1].to_ascii_uppercase() - b'A');
    format!(
        "{}{}",
        char::from_u32(c1).unwrap_or('🏳'),
        char::from_u32(c2).unwrap_or('️')
    )
}

fn unknown() -> PeerGeo {
    PeerGeo {
        country_iso: "ZZ".into(),
        country_name: "Unknown (install geoip.mmdb)".into(),
        flag_emoji: "🌐".into(),
        eyes_tier: EyesTier::Unknown,
    }
}

fn parse_ip(addr: &str) -> Option<IpAddr> {
    let s = addr.trim();
    // strip [ipv6]:port
    if let Some(inner) = s.strip_prefix('[') {
        let end = inner.find(']')?;
        return inner[..end].parse().ok();
    }
    // host:port
    if let Some((host, _port)) = s.rsplit_once(':') {
        if host.parse::<IpAddr>().is_ok() {
            return host.parse().ok();
        }
        // might be ipv6 without brackets — try full
    }
    s.parse().ok()
}

fn is_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => v6.is_loopback() || v6.is_unique_local() || v6.is_unicast_link_local(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_and_eyes() {
        let g = lookup_addr("127.0.0.1:1");
        assert_eq!(g.eyes_tier, EyesTier::Local);
        assert_eq!(eyes_for_iso("US"), EyesTier::Five);
        assert_eq!(eyes_for_iso("DE"), EyesTier::Fourteen);
    }
}
