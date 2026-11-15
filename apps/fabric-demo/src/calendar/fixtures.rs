//! Fixture calendars (section 14): JSON files with UTC times and durations in minutes, so they
//! can be read and reviewed by hand. `demo/fixtures/` holds one per person.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use fabric_policy::time::parse_utc;
use serde::Deserialize;

use super::store::Event;

pub const FORMAT: &str = "fabric-demo-fixtures-v1";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureFile {
    format: String,
    person: String,
    events: Vec<FixtureEvent>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FixtureEvent {
    id: String,
    start: String,
    minutes: i64,
    title: String,
    busy: bool,
}

/// Reads `person`'s fixtures from `path`.
pub fn load(path: &Path, person: &str) -> Result<Vec<Event>, String> {
    let text = fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()))?;
    parse(&text, person).map_err(|error| format!("{}: {error}", path.display()))
}

/// Parses fixtures, checking that they belong to `person` and that every event is well formed.
pub fn parse(text: &str, person: &str) -> Result<Vec<Event>, String> {
    let file: FixtureFile = serde_json::from_str(text).map_err(|error| error.to_string())?;
    if file.format != FORMAT {
        return Err(format!("format {:?}; expected {FORMAT:?}", file.format));
    }
    if file.person != person {
        return Err(format!(
            "the fixtures are {}'s, not {person}'s",
            file.person
        ));
    }
    let mut ids = HashSet::new();
    file.events
        .into_iter()
        .map(|event| {
            if event.id.is_empty() || !ids.insert(event.id.clone()) {
                return Err(format!("event ID {:?} is empty or repeated", event.id));
            }
            if event.title.trim().is_empty() {
                return Err(format!("event {} has no title", event.id));
            }
            if !(1..=7 * 24 * 60).contains(&event.minutes) {
                return Err(format!(
                    "event {} lasts {} minutes; expected 1 to 10080",
                    event.id, event.minutes
                ));
            }
            let start_ms = parse_utc(&event.start)
                .ok_or_else(|| format!("event {} starts at {:?}", event.id, event.start))?;
            Ok(Event {
                id: event.id,
                start_ms,
                length_ms: event.minutes * 60 * 1_000,
                title: event.title,
                busy: event.busy,
                added_by: None,
            })
        })
        .collect()
}
