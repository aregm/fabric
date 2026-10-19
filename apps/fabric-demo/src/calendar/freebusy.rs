//! Free/busy: the only view of a calendar that an agent ever gets (section 5.1).

use serde::{Deserialize, Serialize};

use super::store::Event;

/// A busy interval, `[start_ms, end_ms)`.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Busy {
    pub start_ms: i64,
    pub end_ms: i64,
}

/// The busy events' intervals within `[start_ms, end_ms)`: clipped to the window, sorted, and
/// merged where they overlap or touch, so nothing reveals where one event ends and the next
/// begins.
#[must_use]
pub fn free_busy(events: &[Event], start_ms: i64, end_ms: i64) -> Vec<Busy> {
    let mut clipped: Vec<Busy> = events
        .iter()
        .filter(|event| event.busy)
        .map(|event| Busy {
            start_ms: event.start_ms.max(start_ms),
            end_ms: event.end_ms().min(end_ms),
        })
        .filter(|busy| busy.start_ms < busy.end_ms)
        .collect();
    clipped.sort_by_key(|busy| (busy.start_ms, busy.end_ms));

    let mut merged: Vec<Busy> = Vec::with_capacity(clipped.len());
    for busy in clipped {
        match merged.last_mut() {
            Some(last) if busy.start_ms <= last.end_ms => {
                last.end_ms = last.end_ms.max(busy.end_ms);
            }
            _ => merged.push(busy),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::{Busy, free_busy};
    use crate::calendar::store::Event;

    fn event(id: &str, start_ms: i64, length_ms: i64, busy: bool) -> Event {
        Event {
            id: id.to_owned(),
            start_ms,
            length_ms,
            title: format!("title {id}"),
            busy,
            added_by: None,
        }
    }

    fn busy(start_ms: i64, end_ms: i64) -> Busy {
        Busy { start_ms, end_ms }
    }

    #[test]
    fn clips_sorts_and_merges() {
        let events = [
            event("late", 90, 20, true),     // runs past the window
            event("a", 10, 10, true),        // [10, 20)
            event("touching", 20, 5, true),  // touches a: merged
            event("inside", 12, 3, true),    // inside a: merged
            event("free", 40, 10, false),    // not busy
            event("early", -5, 8, true),     // starts before the window
            event("outside", 200, 10, true), // after the window
            event("overlap", 50, 10, true),
            event("overlap2", 55, 10, true),
        ];
        assert_eq!(
            free_busy(&events, 0, 100),
            [busy(0, 3), busy(10, 25), busy(50, 65), busy(90, 100)]
        );
        assert!(free_busy(&events, 30, 40).is_empty());
    }
}
