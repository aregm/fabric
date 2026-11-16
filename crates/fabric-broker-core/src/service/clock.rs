//! The core clock (D34): it never goes back, across a run and across restarts.

/// The step clock: the raw wall clock plus an offset that only grows.
pub(crate) struct Clock {
    offset_ms: i64,
    last_ms: i64,
}

/// The raw clock as a signed count; a value past `i64::MAX` is held there.
fn signed(raw_now_ms: u64) -> i64 {
    i64::try_from(raw_now_ms).unwrap_or(i64::MAX)
}

impl Clock {
    /// `last = max(raw, last_committed)`, `offset = last - raw`.
    pub(crate) fn start(last_committed_ms: Option<i64>, raw_now_ms: u64) -> Clock {
        let raw = signed(raw_now_ms);
        let last = last_committed_ms.map_or(raw, |committed| committed.max(raw));
        Clock {
            offset_ms: last.saturating_sub(raw),
            last_ms: last,
        }
    }

    /// `raw + offset`, raised to the last value by growing the offset; never decreases. The bool is true when the
    /// offset grew in this call.
    pub(crate) fn now(&mut self, raw_now_ms: u64) -> (i64, bool) {
        let mut now = signed(raw_now_ms).saturating_add(self.offset_ms);
        let grew = now < self.last_ms;
        if grew {
            self.offset_ms = self.offset_ms.saturating_add(self.last_ms - now);
            now = self.last_ms;
        }
        self.last_ms = now;
        (now, grew)
    }

    /// What the clock adds to the raw wall clock.
    pub(crate) fn offset_ms(&self) -> i64 {
        self.offset_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_795_190_400_000;

    #[test]
    fn a_clock_starts_at_the_wall_clock_when_it_is_ahead() {
        let mut clock = Clock::start(Some(T0 as i64 - 5), T0);
        assert_eq!(clock.offset_ms(), 0);
        assert_eq!(clock.now(T0), (T0 as i64, false));
        let mut fresh = Clock::start(None, T0);
        assert_eq!(fresh.now(T0 + 1), (T0 as i64 + 1, false));
    }

    #[test]
    fn a_clock_starts_at_the_last_commit_when_the_wall_clock_is_behind() {
        let mut clock = Clock::start(Some(T0 as i64 + 10_000), T0);
        assert_eq!(clock.offset_ms(), 10_000);
        assert_eq!(clock.now(T0), (T0 as i64 + 10_000, false));
        assert_eq!(clock.now(T0 + 1), (T0 as i64 + 10_001, false));
    }

    #[test]
    fn a_backward_step_keeps_now_and_grows_the_offset() {
        let mut clock = Clock::start(None, T0);
        assert_eq!(clock.now(T0 + 1_000), (T0 as i64 + 1_000, false));
        assert_eq!(clock.now(T0 + 400), (T0 as i64 + 1_000, true));
        assert_eq!(clock.offset_ms(), 600);
        // Time then runs at wall rate from the floored value.
        assert_eq!(clock.now(T0 + 500), (T0 as i64 + 1_100, false));
        assert_eq!(clock.offset_ms(), 600);
    }

    #[test]
    fn a_forward_step_moves_now_forward_and_keeps_the_offset() {
        let mut clock = Clock::start(None, T0);
        assert_eq!(clock.now(T0 + 3_600_000), (T0 as i64 + 3_600_000, false));
        assert_eq!(clock.offset_ms(), 0);
    }

    #[test]
    fn now_never_decreases() {
        let mut clock = Clock::start(Some(T0 as i64), T0 - 50);
        let mut last = i64::MIN;
        for raw in [T0, T0 - 100, T0 + 5, T0 - 1_000, T0 + 2_000, 0, u64::MAX] {
            let (now, _) = clock.now(raw);
            assert!(now >= last, "{now} < {last}");
            last = now;
        }
    }
}
