//! Poison-tolerant mutex locking.
//!
//! A `std::sync::Mutex` becomes *poisoned* if a thread panics while holding it,
//! after which every `.lock().unwrap()` panics in turn — one thread's crash
//! cascades into a dead application. For an unattended installation that means a
//! black gallery. [`LockExt::lock_unpoisoned`] recovers the guard instead.

use std::sync::{Mutex, MutexGuard};

pub trait LockExt<T> {
    /// Acquire the lock, recovering the guard even if a previous holder panicked.
    ///
    /// The poison flag is cleared on recovery so later plain `.lock()` callers
    /// succeed again (otherwise they would keep hitting the poison forever).
    ///
    /// Trade-off: data written by the panicking thread may be partial. For show
    /// control, continuing with possibly-stale state beats crashing the whole app.
    /// Recovery is logged at error level so the trade-off leaves a trace.
    #[track_caller]
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T>;
}

impl<T> LockExt<T> for Mutex<T> {
    fn lock_unpoisoned(&self) -> MutexGuard<'_, T> {
        match self.lock() {
            Ok(guard) => guard,
            Err(poisoned) => {
                // No once-guard, unlike zero_copy.rs: clear_poison() below hands
                // the next caller a healthy lock, so each panic is reported once,
                // by whichever call site reaches it first. A repeat means a
                // genuinely new panic, which is worth another line.
                log::error!(
                    "Recovered a poisoned mutex at {}: a thread panicked while holding it, so this state may be partially written",
                    std::panic::Location::caller()
                );
                self.clear_poison();
                poisoned.into_inner()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    static CAPTURED: Mutex<Vec<String>> = Mutex::new(Vec::new());

    struct CaptureLogger;

    impl log::Log for CaptureLogger {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            // Plain lock, not lock_unpoisoned: logging from inside the logger
            // would recurse.
            CAPTURED
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .push(record.args().to_string());
        }

        fn flush(&self) {}
    }

    /// Poison a fresh mutex the way a real panic does, and hand it back.
    fn poisoned() -> Arc<Mutex<i32>> {
        let m = Arc::new(Mutex::new(5));
        let m2 = Arc::clone(&m);
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());
        m
    }

    /// The log line has to name the *call site*, not this file's impl block, or
    /// it points every poison in the app at one useless location. That is what
    /// `#[track_caller]` buys, so pin it to an exact line number.
    #[test]
    fn recovery_logs_the_call_site() {
        log::set_logger(&CaptureLogger).ok();
        log::set_max_level(log::LevelFilter::Error);
        let m = poisoned();

        let expected_line = line!() + 1;
        let _ = *m.lock_unpoisoned();

        let wanted = format!("sync.rs:{expected_line}");
        let captured = CAPTURED.lock().unwrap_or_else(|e| e.into_inner()).clone();
        assert!(
            captured.iter().any(|line| line.contains(&wanted)),
            "no log line naming {wanted}; captured: {captured:?}"
        );
    }

    #[test]
    fn recovers_and_clears_poison() {
        let m = Arc::new(Mutex::new(5));
        let m2 = Arc::clone(&m);
        // Poison the mutex by panicking while it is held.
        let _ = std::thread::spawn(move || {
            let _g = m2.lock().unwrap();
            panic!("poison it");
        })
        .join();
        assert!(m.is_poisoned());

        // Recovers the value despite the poison...
        assert_eq!(*m.lock_unpoisoned(), 5);
        // ...and clears the flag so a plain lock() works again (no cascade).
        assert!(!m.is_poisoned());
        assert_eq!(*m.lock().unwrap(), 5);
    }
}
