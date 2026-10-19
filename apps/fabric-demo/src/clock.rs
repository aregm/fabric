//! The demo clock (decision D22). Each run starts at a configured UTC time and then advances in
//! real time, so fixtures and denials are reproducible while tokens still expire. Every process
//! derives the time from the same start, so they agree to the millisecond.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use fabric_policy::time::parse_utc;
use serde::{Deserialize, Serialize};

pub trait Clock: Send + Sync {
    /// Demo time, in UTC milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// Demo time `start` at the wall-clock instant `wall_start_ms`, as the launcher records them in
/// the per-run configuration.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClockConfig {
    /// For example `2027-01-11T08:00:00Z`.
    pub start: String,
    pub wall_start_ms: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DemoClock {
    start_ms: i64,
    wall_start_ms: i64,
}

impl DemoClock {
    pub fn from_config(config: &ClockConfig) -> Result<Self, String> {
        let start_ms = parse_utc(&config.start)
            .ok_or_else(|| format!("clock start {:?} is not a UTC time", config.start))?;
        Ok(Self {
            start_ms,
            wall_start_ms: config.wall_start_ms,
        })
    }

    /// A clock whose run starts now, at demo time `start_ms`.
    #[must_use]
    pub fn starting_now(start_ms: i64) -> Self {
        Self {
            start_ms,
            wall_start_ms: wall_now_ms(),
        }
    }
}

impl Clock for DemoClock {
    fn now_ms(&self) -> i64 {
        self.start_ms
            .saturating_add(wall_now_ms().saturating_sub(self.wall_start_ms))
    }
}

/// The wall clock, in milliseconds since the Unix epoch.
#[must_use]
pub fn wall_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// A clock that moves only when told to, for tests.
#[derive(Debug, Default)]
pub struct ManualClock(AtomicI64);

impl ManualClock {
    #[must_use]
    pub fn new(now_ms: i64) -> Self {
        Self(AtomicI64::new(now_ms))
    }

    pub fn set(&self, now_ms: i64) {
        self.0.store(now_ms, Ordering::SeqCst);
    }

    pub fn advance(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn now_ms(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::{Clock, ClockConfig, DemoClock, wall_now_ms};

    #[test]
    fn the_demo_clock_starts_at_its_configured_time() {
        let config = ClockConfig {
            start: "2027-01-11T08:00:00Z".to_owned(),
            wall_start_ms: wall_now_ms() - 5_000,
        };
        let clock = DemoClock::from_config(&config).unwrap();
        let elapsed = clock.now_ms() - 1_799_654_400_000;
        assert!((5_000..60_000).contains(&elapsed), "{elapsed} ms");
        assert!(
            DemoClock::from_config(&ClockConfig {
                start: "11 January".to_owned(),
                wall_start_ms: 0,
            })
            .is_err()
        );
    }
}
