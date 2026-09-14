//! Minimal, model-free calendar primitives.
//!
//! The first executable slice uses synthetic busy intervals. Those intervals stay
//! inside [`LocalCalendar`]; only candidate-specific Boolean answers leave a
//! [`SchedulingAgent`](crate::rendezvous::SchedulingAgent).

use std::error::Error;
use std::fmt;

/// A checked, half-open UTC interval `[start, end)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct UtcInterval {
    start_utc_ms: i64,
    duration_seconds: u32,
}

impl UtcInterval {
    /// Constructs an interval and rejects zero duration or millisecond overflow.
    pub fn new(start_utc_ms: i64, duration_seconds: u32) -> Result<Self, CalendarError> {
        if duration_seconds == 0 {
            return Err(CalendarError::ZeroDuration);
        }

        let duration_ms = i64::from(duration_seconds)
            .checked_mul(1_000)
            .ok_or(CalendarError::IntervalOverflow)?;
        start_utc_ms
            .checked_add(duration_ms)
            .ok_or(CalendarError::IntervalOverflow)?;

        Ok(Self {
            start_utc_ms,
            duration_seconds,
        })
    }

    /// Inclusive UTC start in Unix epoch milliseconds.
    #[must_use]
    pub const fn start_utc_ms(self) -> i64 {
        self.start_utc_ms
    }

    /// Duration in seconds.
    #[must_use]
    pub const fn duration_seconds(self) -> u32 {
        self.duration_seconds
    }

    /// Exclusive UTC end in Unix epoch milliseconds.
    #[must_use]
    pub fn end_utc_ms(self) -> i64 {
        // Construction proves this addition cannot overflow.
        self.start_utc_ms + i64::from(self.duration_seconds) * 1_000
    }

    /// Returns true when the two half-open intervals overlap.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        self.start_utc_ms < other.end_utc_ms() && other.start_utc_ms < self.end_utc_ms()
    }

    /// Returns true when `other` is wholly inside this interval.
    #[must_use]
    pub fn contains(self, other: Self) -> bool {
        self.start_utc_ms <= other.start_utc_ms && other.end_utc_ms() <= self.end_utc_ms()
    }
}

/// Validation error for a UTC interval.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CalendarError {
    /// An interval must occupy some time.
    ZeroDuration,
    /// Converting the duration to milliseconds or computing the end overflowed.
    IntervalOverflow,
}

impl fmt::Display for CalendarError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroDuration => formatter.write_str("an interval must have non-zero duration"),
            Self::IntervalOverflow => {
                formatter.write_str("the interval end overflows UTC milliseconds")
            }
        }
    }
}

impl Error for CalendarError {}

/// Private local availability input owned by one scheduling agent.
///
/// There is deliberately no public busy-interval getter. The semantic simulator
/// can ask only whether a disclosed candidate is available.
#[derive(Clone, Default)]
pub struct LocalCalendar {
    busy: Vec<UtcInterval>,
}

impl LocalCalendar {
    /// Creates a synthetic local calendar from already validated busy intervals.
    #[must_use]
    pub fn from_busy(busy: Vec<UtcInterval>) -> Self {
        Self { busy }
    }

    pub(crate) fn is_available(&self, candidate: UtcInterval) -> bool {
        !self.busy.iter().any(|busy| busy.overlaps(candidate))
    }
}

#[cfg(test)]
mod tests {
    use super::{LocalCalendar, UtcInterval};

    #[test]
    fn touching_half_open_intervals_do_not_overlap() {
        let first = UtcInterval::new(1_000, 30).expect("valid interval");
        let second = UtcInterval::new(31_000, 30).expect("valid interval");

        assert!(!first.overlaps(second));
        assert!(LocalCalendar::from_busy(vec![first]).is_available(second));
    }

    #[test]
    fn overlapping_interval_is_not_available() {
        let busy = UtcInterval::new(10_000, 60).expect("valid interval");
        let candidate = UtcInterval::new(69_000, 60).expect("valid interval");

        assert!(!LocalCalendar::from_busy(vec![busy]).is_available(candidate));
    }
}
