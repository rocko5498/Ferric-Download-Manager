use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

/// The scheduler option "number of retries for each file if downloading failed" (setting `max_retries`).
static MAX_ATTEMPTS: AtomicU32 = AtomicU32::new(4);
const MAX_ATTEMPTS_LIMIT: u32 = 50;

pub fn set_max_attempts(n: u32) {
    MAX_ATTEMPTS.store(n.min(MAX_ATTEMPTS_LIMIT), Ordering::Relaxed);
}

pub fn max_attempts() -> u32 {
    MAX_ATTEMPTS.load(Ordering::Relaxed)
}

/// Retry delay for the chunk worker. The first retry is almost immediate (up to 300 ms): a dropped
/// connection normally just needs a new one, and the first retry reconnects at once (a server that cut
/// connections). Later retries back off exponentially (base, 2x base, ...) up to `max_secs`,
/// with up to 1 s jitter, so a struggling server is not hammered.
pub fn backoff_delay(attempt: u32, base_secs: u64, max_secs: u64) -> Duration {
    if attempt == 0 {
        return Duration::from_millis(rand::random::<u64>() % 300);
    }
    let exp = 2u64.saturating_pow(attempt - 1).saturating_mul(base_secs);
    let capped = exp.min(max_secs);
    let jitter_ms = rand::random::<u64>() % 1000;
    Duration::from_secs(capped) + Duration::from_millis(jitter_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_retry_is_immediate_and_later_ones_back_off() {
        for _ in 0..20 {
            assert!(
                backoff_delay(0, 2, 30) < Duration::from_millis(300),
                "first retry waits under 300 ms"
            );
        }
        assert!(backoff_delay(1, 2, 30) >= Duration::from_secs(2));
        assert!(backoff_delay(2, 2, 30) >= Duration::from_secs(4));
        assert!(
            backoff_delay(20, 2, 30) < Duration::from_secs(32),
            "capped at max plus jitter"
        );
    }

    #[test]
    fn retry_limit_is_settable_and_capped() {
        let before = max_attempts();
        set_max_attempts(7);
        assert_eq!(max_attempts(), 7);
        set_max_attempts(10_000);
        assert_eq!(max_attempts(), MAX_ATTEMPTS_LIMIT);
        set_max_attempts(before);
    }
}
