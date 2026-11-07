//! Smart DNS cache + failover for outbound download connections.
//!
//! Resolves hosts via `getaddrinfo` (blocking pool), caches results, and
//! rotates failed IPs to the end of the list so the next connection tries
//! the next A/AAAA record (IPv6 kept when the OS returns it).

use hyper_util::client::legacy::connect::dns::Name;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::io;
use std::net::{IpAddr, SocketAddr, ToSocketAddrs};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};
use tower::Service;

const CACHE_TTL: Duration = Duration::from_secs(300);

#[derive(Clone, Default)]
pub struct CachingResolver {
    state: Arc<ResolverState>,
}

#[derive(Default)]
struct ResolverState {
    cache: Mutex<HashMap<String, CacheEntry>>,
    /// IPs that recently failed connect — deprioritized on next resolve.
    failed: Mutex<HashMap<String, HashSet<IpAddr>>>,
    /// Hosts of LAN-only protocols (WebDAV on a NAS) that may resolve to private addresses.
    lan_hosts: Mutex<HashSet<String>>,
    /// How many connections each host has asked an address for: the next one starts at the next record.
    turn: Mutex<HashMap<String, usize>>,
}

struct CacheEntry {
    addrs: Vec<SocketAddr>,
    at: Instant,
}

impl CachingResolver {
    #[must_use]
    pub fn global() -> Self {
        static GLOBAL: std::sync::OnceLock<CachingResolver> = std::sync::OnceLock::new();
        GLOBAL.get_or_init(Self::default).clone()
    }

    /// Mark an IP bad for `host` so the next resolve prefers other records.
    /// Test-only until job.rs wires connection failures into the resolver.
    #[cfg(test)]
    pub fn mark_failed(&self, host: &str, ip: IpAddr) {
        if let Ok(mut map) = self.state.failed.lock() {
            map.entry(host.to_ascii_lowercase()).or_default().insert(ip);
        }
    }

    /// Let `host` resolve to private/loopback addresses (explicit LAN protocols only).
    pub fn allow_private(&self, host: &str) {
        if let Ok(mut set) = self.state.lan_hosts.lock() {
            set.insert(host.to_ascii_lowercase());
        }
    }

    fn private_allowed(&self, host: &str) -> bool {
        self.state
            .lan_hosts
            .lock()
            .is_ok_and(|set| set.contains(&host.to_ascii_lowercase()))
    }

    /// Resolve `host`, cache the result and order it for connecting. Private,
    /// loopback and link-local answers are dropped here, at connect time, so a
    /// DNS server cannot pass the URL check with a public address and then
    /// rebind the name to an internal one (SSRF via DNS rebinding).
    async fn lookup(&self, host: String) -> io::Result<Vec<SocketAddr>> {
        {
            let cache = self
                .state
                .cache
                .lock()
                .map_err(|_| io::Error::other("dns cache lock poisoned"))?;
            if let Some(entry) = cache.get(&host) {
                if entry.at.elapsed() < CACHE_TTL && !entry.addrs.is_empty() {
                    return Ok(self.order_addrs(&host, &entry.addrs));
                }
            }
        }

        let host_for_lookup = host.clone();
        let found: Vec<SocketAddr> = tokio::task::spawn_blocking(move || {
            (host_for_lookup.as_str(), 0_u16)
                .to_socket_addrs()
                .map(Iterator::collect)
        })
        .await
        .map_err(io::Error::other)??;
        if found.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("no addresses for host {host}"),
            ));
        }
        let addrs = if self.private_allowed(&host) {
            found
        } else {
            keep_public(found, crate::security::is_private_ip)
        };
        if addrs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{host} resolves only to private or loopback addresses (blocked)"),
            ));
        }
        if let Ok(mut cache) = self.state.cache.lock() {
            cache.insert(
                host.clone(),
                CacheEntry {
                    addrs: addrs.clone(),
                    at: Instant::now(),
                },
            );
        }
        Ok(self.order_addrs(&host, &addrs))
    }

    /// Clear failure marks after a successful connection (optional).
    pub fn mark_ok(&self, host: &str) {
        if let Ok(mut map) = self.state.failed.lock() {
            map.remove(&host.to_ascii_lowercase());
        }
    }

    /// Drop cache entry so the next lookup re-resolves A/AAAA records.
    pub fn invalidate(&self, host: &str) {
        let key = host.to_ascii_lowercase();
        if let Ok(mut cache) = self.state.cache.lock() {
            cache.remove(&key);
        }
    }

    fn order_addrs(&self, host: &str, addrs: &[SocketAddr]) -> Vec<SocketAddr> {
        let key = host.to_ascii_lowercase();
        let failed = self.state.failed.lock().ok();
        let bad = failed.as_ref().and_then(|m| m.get(&key));
        let mut good = Vec::new();
        let mut demoted = Vec::new();
        for addr in addrs {
            if bad.is_some_and(|s| s.contains(&addr.ip())) {
                demoted.push(*addr);
            } else {
                good.push(*addr);
            }
        }
        // Spread connections over the host's addresses: CDNs often limit each address separately, and
        // the OS hands every connection the same first one. Only addresses of the first one's family
        // take turns, so the family preference (happy eyeballs) stays as the OS gave it.
        let turn = self.state.turn.lock().ok().map_or(0, |mut m| {
            let n = m.entry(key).or_insert(0);
            let t = *n;
            *n = n.wrapping_add(1);
            t
        });
        rotate_same_family(&mut good, turn);
        good.extend(demoted);
        good
    }
}

impl Service<Name> for CachingResolver {
    type Response = std::vec::IntoIter<SocketAddr>;
    type Error = io::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, name: Name) -> Self::Future {
        let resolver = self.clone();
        let host = name.as_str().to_string();
        Box::pin(async move { resolver.lookup(host).await.map(Vec::into_iter) })
    }
}

/// reqwest uses the same resolver (and therefore the same rebinding protection).
impl reqwest::dns::Resolve for CachingResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.clone();
        let host = name.as_str().to_string();
        Box::pin(async move {
            let addrs = resolver.lookup(host).await?;
            Ok(Box::new(addrs.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Rotate the addresses of the first address's family by `turn` places, leaving the others where they are.
fn rotate_same_family(addrs: &mut [SocketAddr], turn: usize) {
    let Some(first) = addrs.first() else { return };
    let v4 = first.is_ipv4();
    let slots: Vec<usize> = (0..addrs.len())
        .filter(|&i| addrs[i].is_ipv4() == v4)
        .collect();
    if slots.len() < 2 {
        return;
    }
    let mut same: Vec<SocketAddr> = slots.iter().map(|&i| addrs[i]).collect();
    same.rotate_left(turn % slots.len());
    for (slot, addr) in slots.into_iter().zip(same) {
        addrs[slot] = addr;
    }
}

/// Drop addresses a public download must never connect to.
fn keep_public(addrs: Vec<SocketAddr>, is_private: impl Fn(IpAddr) -> bool) -> Vec<SocketAddr> {
    addrs.into_iter().filter(|a| !is_private(a.ip())).collect()
}

/// Extract host from an http(s) URL for failure reporting.
#[must_use]
pub fn host_from_url(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, SocketAddrV4};

    #[test]
    fn order_demotes_failed_ips() {
        let r = CachingResolver::default();
        let a = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(1, 1, 1, 1), 0));
        let b = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(1, 0, 0, 1), 0));
        r.mark_failed("example.com", a.ip());
        let ordered = r.order_addrs("example.com", &[a, b]);
        assert_eq!(ordered[0], b);
        assert_eq!(ordered[1], a);
    }

    #[test]
    fn connections_take_turns_over_a_hosts_addresses() {
        let r = CachingResolver::default();
        let ips: Vec<SocketAddr> = (1..=4)
            .map(|n| SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(203, 0, 113, n), 0)))
            .collect();
        let firsts: Vec<SocketAddr> = (0..8)
            .map(|_| r.order_addrs("cdn.example", &ips)[0])
            .collect();
        assert_eq!(
            &firsts[..4],
            &ips[..],
            "the first four connections start at four different addresses"
        );
        assert_eq!(firsts[4], ips[0], "then it wraps around");
        // A failed address still goes last, whatever the turn.
        r.mark_failed("cdn.example", ips[2].ip());
        for _ in 0..8 {
            assert_eq!(*r.order_addrs("cdn.example", &ips).last().unwrap(), ips[2]);
        }
    }

    #[test]
    fn private_answers_are_dropped_before_connecting() {
        let public = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(93, 184, 216, 34), 0));
        let internal = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::new(10, 0, 0, 7), 0));
        let kept = keep_public(vec![internal, public], |ip| ip == internal.ip());
        assert_eq!(kept, vec![public]);
        assert!(keep_public(vec![internal], |ip| ip == internal.ip()).is_empty());
    }

    #[test]
    fn lan_hosts_are_opt_in() {
        let r = CachingResolver::default();
        assert!(!r.private_allowed("NAS.example"));
        r.allow_private("nas.EXAMPLE");
        assert!(r.private_allowed("NAS.example"));
    }

    #[test]
    fn host_from_url_extracts() {
        assert_eq!(
            host_from_url("https://cdn.example.com:443/x").as_deref(),
            Some("cdn.example.com")
        );
    }
}
