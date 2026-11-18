//! `Clock`: the one source of time of the crate (section 2.2).
//!
//! Everything that needs the time takes a clock: deadlines (5.2), access-token ends (3.8), the
//! identity token's times (3.5 item 7), the listener's lifetime (3.3). Only [`SystemClock`]
//! reads the system clock. Tests use `ManualClock` and move it themselves; it exists only in a
//! debug build (PD2: no test seam in release).

use std::sync::Arc;
#[cfg(debug_assertions)]
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A source of the current time in Unix milliseconds.
pub trait Clock: Send + Sync {
    /// Milliseconds since 1970-01-01T00:00:00Z.
    fn now_ms(&self) -> u64;
}

/// A clock shared by the parts of the adapter (the transport, the token cache, the sign-in).
pub type SharedClock = Arc<dyn Clock>;

/// The system's wall clock. A time before 1970 reads as 0.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
    }
}

/// A test double: a clock that moves only when told. Clones share one time, so a test can hold
/// one handle and give another to the code under test, across threads. Only in a debug build
/// (PD2).
#[cfg(debug_assertions)]
#[derive(Clone, Debug, Default)]
pub struct ManualClock(Arc<AtomicU64>);

#[cfg(debug_assertions)]
impl ManualClock {
    /// A clock that reads `now_ms` until it is set or advanced.
    #[must_use]
    pub fn new(now_ms: u64) -> ManualClock {
        ManualClock(Arc::new(AtomicU64::new(now_ms)))
    }

    /// Sets the time for every handle.
    pub fn set(&self, now_ms: u64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }

    /// Moves the time forward by `ms` for every handle (saturating).
    pub fn advance(&self, ms: u64) {
        let _ = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |t| {
                Some(t.saturating_add(ms))
            });
    }
}

#[cfg(debug_assertions)]
impl Clock for ManualClock {
    fn now_ms(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_after_2020() {
        // 2020-01-01T00:00:00Z.
        assert!(SystemClock.now_ms() > 1_577_836_800_000);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn a_manual_clock_moves_only_when_told_and_its_clones_share_the_time() {
        let clock = ManualClock::new(1_000);
        let other = clock.clone();
        assert_eq!(other.now_ms(), 1_000);
        clock.advance(500);
        assert_eq!(other.now_ms(), 1_500);
        other.set(7);
        assert_eq!(clock.now_ms(), 7);
        clock.set(u64::MAX - 1);
        clock.advance(10);
        assert_eq!(clock.now_ms(), u64::MAX);
    }

    #[test]
    #[cfg(debug_assertions)]
    fn a_manual_clock_is_shared_across_threads() {
        let clock = ManualClock::new(0);
        let shared: SharedClock = Arc::new(clock.clone());
        std::thread::spawn(move || clock.advance(42))
            .join()
            .expect("the thread ends");
        assert_eq!(shared.now_ms(), 42);
    }
}
