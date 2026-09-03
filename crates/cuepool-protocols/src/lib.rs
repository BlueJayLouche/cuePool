//! CuePool Protocols — OSC, MSC, and MIDI.

pub mod ltc;
pub mod midi;
pub mod msc;
pub mod osc;
pub mod timecode;

/// Lock a `Mutex` while tolerating poisoning, so a panicking protocol handler
/// can't kill the receive thread (or stop dispatch) on its next lock. Mirrors
/// `cuepool_core::LockExt` — duplicated to keep this crate dependency-free.
pub(crate) trait LockExt<T> {
    #[track_caller]
    fn lock_unpoisoned(&self) -> std::sync::MutexGuard<'_, T>;
}

impl<T> LockExt<T> for std::sync::Mutex<T> {
    fn lock_unpoisoned(&self) -> std::sync::MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                log::error!(
                    "Recovered a poisoned mutex at {}: a protocol handler panicked while holding it, so this state may be partially written",
                    std::panic::Location::caller()
                );
                self.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

/// Rate limit for warnings driven by network input. The first event logs
/// immediately; after that at most one summary line per `interval`, carrying
/// the count suppressed since the last line. Without this a device spraying
/// bad datagrams turns every packet into a synchronous persistent-log write
/// on the receive thread and evicts real diagnostics from the log ring.
pub(crate) struct WarnThrottle {
    interval: std::time::Duration,
    last_logged: Option<std::time::Instant>,
    suppressed: u64,
}

impl WarnThrottle {
    pub(crate) fn new(interval: std::time::Duration) -> Self {
        Self {
            interval,
            last_logged: None,
            suppressed: 0,
        }
    }

    /// Record one event at `now`. Returns `Some(suppressed_since_last_line)`
    /// when the caller should log now, `None` when it should stay quiet.
    pub(crate) fn tick(&mut self, now: std::time::Instant) -> Option<u64> {
        match self.last_logged {
            Some(last) if now.duration_since(last) < self.interval => {
                self.suppressed += 1;
                None
            }
            _ => {
                self.last_logged = Some(now);
                Some(std::mem::take(&mut self.suppressed))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn first_tick_logs_immediately_with_nothing_suppressed() {
        let mut throttle = WarnThrottle::new(Duration::from_secs(10));
        assert_eq!(throttle.tick(Instant::now()), Some(0));
    }

    #[test]
    fn a_tick_inside_the_interval_stays_quiet() {
        let mut throttle = WarnThrottle::new(Duration::from_secs(10));
        let t0 = Instant::now();
        assert_eq!(throttle.tick(t0), Some(0));
        assert_eq!(throttle.tick(t0 + Duration::from_secs(1)), None);
    }

    #[test]
    fn a_tick_at_the_interval_boundary_logs_the_suppressed_count() {
        let mut throttle = WarnThrottle::new(Duration::from_secs(10));
        let t0 = Instant::now();
        assert_eq!(throttle.tick(t0), Some(0));
        assert_eq!(throttle.tick(t0 + Duration::from_secs(1)), None);
        assert_eq!(throttle.tick(t0 + Duration::from_secs(10)), Some(1));
    }

    #[test]
    fn a_tick_right_after_a_logged_one_stays_quiet() {
        let mut throttle = WarnThrottle::new(Duration::from_secs(10));
        let t0 = Instant::now();
        assert_eq!(throttle.tick(t0), Some(0));
        assert_eq!(throttle.tick(t0 + Duration::from_secs(10)), Some(0));
        assert_eq!(throttle.tick(t0 + Duration::from_secs(10)), None);
    }
}
