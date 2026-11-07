use chrono::Timelike as _;
use ferric_common::QueueRecord;

const MINUTES_PER_DAY: u16 = 24 * 60;

/// Minutes since local midnight. Queue windows are entered as wall-clock times
/// (IDM compares them with the local clock), so UTC would shift every schedule
/// by the user's UTC offset.
#[must_use]
pub fn current_minute_local() -> u16 {
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

/// Day of the week as a `sched_days` bit: 0 = Sunday ... 6 = Saturday.
#[must_use]
pub fn current_weekday_local() -> u8 {
    use chrono::Datelike;
    chrono::Local::now().weekday().num_days_from_sunday() as u8
}

/// Whether the queue's weekday checkboxes include `weekday` (0 = Sunday).
#[must_use]
pub fn runs_on(queue: &QueueRecord, weekday: u8) -> bool {
    queue.sched_days & (1 << (weekday % 7)) != 0
}

#[must_use]
pub fn is_queue_active(queue: &QueueRecord, minute: u16) -> bool {
    if !queue.enabled {
        return false;
    }

    match (queue.sched_start, queue.sched_end) {
        (Some(start), Some(end)) => is_minute_in_window(minute, start, end),
        _ => true,
    }
}

#[must_use]
pub fn is_minute_in_window(minute: u16, start: u16, end: u16) -> bool {
    let minute = minute % MINUTES_PER_DAY;
    let start = start % MINUTES_PER_DAY;
    let end = end % MINUTES_PER_DAY;

    if start == end {
        return true;
    }

    if start < end {
        minute >= start && minute < end
    } else {
        minute >= start || minute < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn queue(enabled: bool, start: Option<u16>, end: Option<u16>) -> QueueRecord {
        QueueRecord {
            id: Uuid::new_v4(),
            name: "q".into(),
            max_concurrent: 2,
            max_bandwidth: None,
            sched_start: start,
            sched_end: end,
            sched_days: 127,
            start_on_startup: false,
            sync_interval_min: None,
            max_retries: None,
            open_file: None,
            when_done: None,
            enabled,
            priority: 0,
        }
    }

    #[test]
    fn weekday_bits_pick_the_days_a_queue_runs() {
        let mut q = queue(true, None, None);
        q.sched_days = 0b0100010; // Monday and Friday
        assert!(runs_on(&q, 1) && runs_on(&q, 5));
        assert!(!runs_on(&q, 0) && !runs_on(&q, 6));
        assert!(current_weekday_local() < 7);
    }

    #[test]
    fn current_minute_is_a_valid_minute_of_day() {
        assert!(current_minute_local() < MINUTES_PER_DAY);
    }

    #[test]
    fn queue_without_window_is_active_when_enabled() {
        assert!(is_queue_active(&queue(true, None, None), 12 * 60));
        assert!(!is_queue_active(&queue(false, None, None), 12 * 60));
    }

    #[test]
    fn same_day_window_is_start_inclusive_end_exclusive() {
        assert!(!is_minute_in_window(8 * 60 + 59, 9 * 60, 17 * 60));
        assert!(is_minute_in_window(9 * 60, 9 * 60, 17 * 60));
        assert!(is_minute_in_window(16 * 60 + 59, 9 * 60, 17 * 60));
        assert!(!is_minute_in_window(17 * 60, 9 * 60, 17 * 60));
    }

    #[test]
    fn midnight_wrapping_window_spans_two_days() {
        assert!(is_minute_in_window(23 * 60, 22 * 60, 2 * 60));
        assert!(is_minute_in_window(60, 22 * 60, 2 * 60));
        assert!(!is_minute_in_window(12 * 60, 22 * 60, 2 * 60));
    }

    #[test]
    fn equal_start_end_means_always_active() {
        assert!(is_minute_in_window(0, 8 * 60, 8 * 60));
        assert!(is_minute_in_window(23 * 60 + 59, 8 * 60, 8 * 60));
    }
}
