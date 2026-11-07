use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Sliding-window speed tracker. Keeps bytes recorded over the last
/// `window` seconds and reports bytes/second.
pub struct SpeedTracker {
    window: VecDeque<(Instant, u64)>,
    window_secs: u64,
}

impl SpeedTracker {
    pub fn new(window_secs: u64) -> Self {
        Self {
            window: VecDeque::new(),
            window_secs,
        }
    }

    pub fn record(&mut self, bytes: u64) {
        let now = Instant::now();
        self.window.push_back((now, bytes));
        let cutoff = now
            .checked_sub(Duration::from_secs(self.window_secs))
            .unwrap_or(now);
        while self.window.front().is_some_and(|(t, _)| *t < cutoff) {
            self.window.pop_front();
        }
    }

    /// Bytes per second (smoothed over the window).
    pub fn speed_bps(&self) -> u64 {
        if self.window.len() < 2 {
            return 0;
        }
        let total: u64 = self.window.iter().map(|(_, b)| b).sum();
        let sample_span = self
            .window
            .back()
            .unwrap()
            .0
            .duration_since(self.window.front().unwrap().0)
            .as_secs_f64();
        let average_interval = sample_span / (self.window.len().saturating_sub(1) as f64);
        let elapsed = sample_span + average_interval;
        if elapsed < 0.1 {
            return 0;
        }
        (total as f64 / elapsed) as u64
    }

    /// Estimated seconds until `remaining` bytes are downloaded.
    pub fn eta_secs(&self, remaining: u64) -> Option<u64> {
        let speed = self.speed_bps();
        if speed == 0 {
            return None;
        }
        Some((remaining as f64 / speed as f64).ceil() as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_before_records() {
        let tracker = SpeedTracker::new(5);
        assert_eq!(tracker.speed_bps(), 0);
    }

    #[test]
    fn eta_none_when_no_speed() {
        let tracker = SpeedTracker::new(5);
        assert!(tracker.eta_secs(1000).is_none());
    }

    #[test]
    fn eta_available_after_elapsed_samples() {
        let mut tracker = SpeedTracker::new(5);
        tracker.record(100);
        std::thread::sleep(Duration::from_millis(120));
        tracker.record(100);

        assert!(tracker.speed_bps() > 0);
        assert!(tracker.eta_secs(1000).is_some());
    }

    #[test]
    fn eta_rounds_up_instead_of_truncating_to_zero() {
        let mut tracker = SpeedTracker::new(5);
        tracker.record(1024);
        std::thread::sleep(Duration::from_millis(120));
        tracker.record(1024);

        assert_eq!(tracker.eta_secs(1), Some(1));
    }

    #[test]
    fn speed_accounts_for_first_sample_interval() {
        let mut tracker = SpeedTracker::new(5);
        tracker.record(120);
        std::thread::sleep(Duration::from_millis(120));
        tracker.record(120);

        assert!(
            tracker.speed_bps() < 1_500,
            "two 120-byte samples over roughly 240 ms should not be treated as a 120 ms window"
        );
    }
}
