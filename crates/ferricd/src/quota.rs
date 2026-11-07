//! IDM's "Download limits": once more than `quota_mb` MB came down within the last `quota_hours` hours
//! (1-23), every download is stopped and nothing new starts until the total falls under the limit again
//! (the window slides) or the user raises the limit.
use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Bytes received, kept per sample so the window can slide.
#[derive(Default)]
pub struct QuotaWindow {
    samples: VecDeque<(Instant, u64)>,
}

impl QuotaWindow {
    pub fn add(&mut self, now: Instant, bytes: u64) {
        if bytes > 0 {
            self.samples.push_back((now, bytes));
        }
    }

    /// Bytes received in the last `window`, forgetting older samples.
    pub fn total(&mut self, now: Instant, window: Duration) -> u64 {
        while self
            .samples
            .front()
            .is_some_and(|(at, _)| now.duration_since(*at) > window)
        {
            self.samples.pop_front();
        }
        self.samples.iter().map(|(_, b)| *b).sum()
    }
}

/// `(limit in bytes, window)` from the settings, or `None` when the feature is off or the hours are out of
/// IDM's 1..=23 range.
pub fn limits(quota_mb: Option<&str>, quota_hours: Option<&str>) -> Option<(u64, Duration)> {
    let mb: u64 = quota_mb?.trim().parse().ok().filter(|v| *v > 0)?;
    let hours: u64 = quota_hours?
        .trim()
        .parse()
        .ok()
        .filter(|h| (1..=23).contains(h))?;
    Some((
        mb.saturating_mul(1024 * 1024),
        Duration::from_secs(hours * 3600),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_slides_and_sums() {
        let t0 = Instant::now();
        let hour = Duration::from_secs(3600);
        let mut w = QuotaWindow::default();
        w.add(t0, 100);
        w.add(t0 + Duration::from_secs(1800), 50);
        assert_eq!(w.total(t0 + Duration::from_secs(1900), hour), 150);
        assert_eq!(w.total(t0 + Duration::from_secs(3700), hour), 50);
        assert_eq!(w.total(t0 + Duration::from_secs(6000), hour), 0);
    }

    #[test]
    fn limits_follow_idms_range() {
        assert_eq!(
            limits(Some("500"), Some("2")),
            Some((500 * 1024 * 1024, Duration::from_secs(7200)))
        );
        assert_eq!(limits(Some("0"), Some("2")), None);
        assert_eq!(limits(Some("500"), Some("24")), None);
        assert_eq!(limits(Some("500"), Some("0")), None);
        assert_eq!(limits(None, Some("2")), None);
    }
}
