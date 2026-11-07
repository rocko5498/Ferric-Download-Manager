use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::{Quota, RateLimiter};
use std::collections::HashMap;
use std::num::NonZeroU32;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

type Limiter = RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

/// Token-bucket bandwidth limiter. `bytes_per_sec = 0` means unlimited.
/// Optional `parent` is acquired first (e.g. shared per-domain site cap).
pub struct BandwidthLimiter {
    /// The bucket and its size; swapped as a whole when the limit changes while the job runs.
    inner: RwLock<Bucket>,
    parent: Option<Arc<BandwidthLimiter>>,
    /// Every byte that went through `acquire`, limited or not (lets the job measure its own speed).
    seen: AtomicU64,
}

/// A rate limiter plus the largest number of tokens it can hold at once (bigger requests are paid
/// for in slices of this size). `limiter: None` = unlimited.
#[derive(Clone)]
struct Bucket {
    limiter: Option<Arc<Limiter>>,
    burst: u32,
}

fn make_bucket(bytes_per_sec: u64) -> Bucket {
    let bytes_per_sec = bytes_per_sec.min(u64::from(u32::MAX));
    Bucket {
        limiter: build_limiter(bytes_per_sec),
        burst: burst_for(bytes_per_sec),
    }
}

impl BandwidthLimiter {
    pub fn new(bytes_per_sec: u64) -> Self {
        Self {
            inner: RwLock::new(make_bucket(bytes_per_sec)),
            parent: None,
            seen: AtomicU64::new(0),
        }
    }

    /// Job/queue/global layer with optional shared parent (per-domain site cap).
    pub fn with_parent(bytes_per_sec: u64, parent: Option<Arc<BandwidthLimiter>>) -> Self {
        let mut lim = Self::new(bytes_per_sec);
        lim.parent = parent;
        lim
    }

    /// Change this job's limit while it runs (0 = unlimited). Connections pick it up with their next frame.
    pub fn set_rate(&self, bytes_per_sec: u64) {
        let bucket = make_bucket(bytes_per_sec);
        *self
            .inner
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = bucket;
    }

    /// Bytes this job has received so far.
    pub fn bytes_seen(&self) -> u64 {
        self.seen.load(Ordering::Relaxed)
    }

    /// Wait until `bytes` tokens are available (parent first, then self).
    pub async fn acquire(&self, bytes: usize) {
        self.seen.fetch_add(bytes as u64, Ordering::Relaxed);
        // Parent chain first (no async recursion — boxed futures would be wasteful).
        let mut chain: Vec<&BandwidthLimiter> = Vec::with_capacity(2);
        let mut cur = Some(self);
        while let Some(node) = cur {
            chain.push(node);
            cur = node.parent.as_deref();
        }
        for node in chain.iter().rev() {
            node.acquire_local(bytes).await;
        }
    }

    async fn acquire_local(&self, bytes: usize) {
        let bucket = self
            .inner
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let Some(limiter) = &bucket.limiter else {
            return;
        };
        // `until_n_ready` refuses requests larger than the bucket, which used to
        // fall back to a single token and let big frames bypass the limit. Pay
        // for every byte, one bucket-sized slice at a time.
        let mut remaining = u32::try_from(bytes.max(1)).unwrap_or(u32::MAX);
        while remaining > 0 {
            let take = remaining.min(bucket.burst).max(1);
            let tokens = NonZeroU32::new(take).unwrap_or(NonZeroU32::MIN);
            if limiter.until_n_ready(tokens).await.is_err() {
                limiter.until_ready().await;
            }
            remaining -= take;
        }
    }
}

/// Shared per-domain limiters so concurrent downloads to one host share one bucket.
#[derive(Default)]
pub struct DomainBandwidthPool {
    // host → (cap_bps, limiter)
    map: Mutex<HashMap<String, (u64, Arc<BandwidthLimiter>)>>,
}

impl DomainBandwidthPool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Shared limiter for `host` at `cap_bps` (0 = unlimited stub, not shared).
    pub fn get(&self, host: &str, cap_bps: u64) -> Option<Arc<BandwidthLimiter>> {
        if cap_bps == 0 || host.is_empty() {
            return None;
        }
        let key = host.to_ascii_lowercase();
        let mut map = self
            .map
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((existing, lim)) = map.get(&key) {
            if *existing == cap_bps {
                return Some(Arc::clone(lim));
            }
        }
        let lim = Arc::new(BandwidthLimiter::new(cap_bps));
        map.insert(key, (cap_bps, Arc::clone(&lim)));
        Some(lim)
    }
}

fn burst_for(bytes_per_sec: u64) -> u32 {
    u32::try_from((bytes_per_sec / 10).max(1)).unwrap_or(u32::MAX)
}

fn build_limiter(bytes_per_sec: u64) -> Option<Arc<Limiter>> {
    if bytes_per_sec == 0 {
        return None;
    }
    let burst = NonZeroU32::new(burst_for(bytes_per_sec))?;
    let rate = NonZeroU32::new(bytes_per_sec as u32)?;
    let quota = Quota::per_second(rate).allow_burst(burst);
    Some(Arc::new(RateLimiter::direct(quota)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn frames_larger_than_the_bucket_are_still_limited() {
        // 100 KB/s => 10 KB bucket. A 100 KB frame must take about a second, not ~0.
        let limiter = BandwidthLimiter::new(100_000);
        let started = std::time::Instant::now();
        limiter.acquire(100_000).await;
        limiter.acquire(100_000).await;
        assert!(
            started.elapsed() >= std::time::Duration::from_millis(1500),
            "200 KB at 100 KB/s took only {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn domain_pool_reuses_same_limiter() {
        let pool = DomainBandwidthPool::new();
        let a = pool.get("cdn.example.com", 1_000_000).unwrap();
        let b = pool.get("CDN.example.com", 1_000_000).unwrap();
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn domain_pool_replaces_on_cap_change() {
        let pool = DomainBandwidthPool::new();
        let a = pool.get("x.com", 1000).unwrap();
        let b = pool.get("x.com", 2000).unwrap();
        assert!(!Arc::ptr_eq(&a, &b));
    }
}
