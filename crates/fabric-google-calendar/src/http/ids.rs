//! The checks of ids before they reach a URL or a header (5.3, PD32).
//!
//! An agent writes event ids and revisions; Google writes calendar ids. Each is turned into a
//! typed value here, and the request path and the `If-Match` header of [`super`] accept only
//! these types. A value of the wrong form makes no request: its constructor fails with a fixed
//! label and the value is not kept (PD27).
//!
//! `Debug` of each type prints its kind and length, never the value: a calendar id holds an
//! address, and g16 searches every formatted value for one.

use std::fmt;

/// Longest event id, reserved id and calendar id (5.3; the contract's `provider_ref`).
pub const MAX_ID_LEN: usize = 1024;

/// Longest text between the quotes of a revision (5.3).
pub const MAX_REVISION_TAG_LEN: usize = 256;

/// Which check a value failed. A fixed label; the value itself is not kept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IdError {
    /// Not `^[A-Za-z0-9_]{1,1024}$`.
    EventId,
    /// Not `^[0-9a-v]{5,1024}$`.
    ReservedEventId,
    /// Not exactly one strong entity tag.
    Revision,
    /// Empty, longer than 1024, not printable ASCII, holds `/`, or is `.` or `..`.
    CalendarId,
}

impl IdError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            IdError::EventId => "the event id is not of the form Google uses",
            IdError::ReservedEventId => "the reserved event id is not lower-case base32hex",
            IdError::Revision => "the revision is not one strong entity tag",
            IdError::CalendarId => "the calendar id is not usable",
        }
    }
}

impl fmt::Display for IdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for IdError {}

macro_rules! redacted_debug {
    ($name:ident) => {
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, concat!(stringify!($name), "(len {})"), self.0.len())
            }
        }
    };
}

/// An event id or instance id that may be put into a path: `^[A-Za-z0-9_]{1,1024}$` (5.3).
/// `.`, `..`, `%2e%2e`, `/`, `?`, `#` and spaces can therefore never reach the URL library.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct EventId(String);

impl EventId {
    /// Checks `value`. An id of another form is Rejected `invalid_arguments` by the caller and
    /// no request is made (5.3).
    pub fn new(value: &str) -> Result<EventId, IdError> {
        let ok = (1..=MAX_ID_LEN).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if ok {
            Ok(EventId(value.to_owned()))
        } else {
            Err(IdError::EventId)
        }
    }

    /// The id, verbatim (6.9).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(EventId);

/// The id the core reserved for a create: `^[0-9a-v]{5,1024}$`, lower-case base32hex (5.3,
/// PD16). Every reserved id is also an [`EventId`].
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ReservedEventId(String);

impl ReservedEventId {
    /// Checks `value`.
    pub fn new(value: &str) -> Result<ReservedEventId, IdError> {
        let ok = (5..=MAX_ID_LEN).contains(&value.len())
            && value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'v').contains(&b));
        if ok {
            Ok(ReservedEventId(value.to_owned()))
        } else {
            Err(IdError::ReservedEventId)
        }
    }

    /// The id, verbatim.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The same id as an [`EventId`], to read the event back (section 7).
    #[must_use]
    pub fn as_event_id(&self) -> EventId {
        // `[0-9a-v]` is inside `[A-Za-z0-9_]` and the lengths agree, so no check can fail.
        EventId(self.0.clone())
    }
}

redacted_debug!(ReservedEventId);

/// A revision that may be sent as `If-Match`: exactly one strong entity tag, a double quote, 1
/// to 256 printable ASCII characters other than the double quote, a double quote (5.3, PD14).
/// `*`, a `W/` prefix and a list are refused, so a change can never be forced through.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Revision(String);

impl Revision {
    /// Checks `value`, quotes included (6.9: the revision is the etag verbatim).
    pub fn new(value: &str) -> Result<Revision, IdError> {
        let b = value.as_bytes();
        let ok = b.len() >= 3
            && b[0] == b'"'
            && b[b.len() - 1] == b'"'
            && b.len() - 2 <= MAX_REVISION_TAG_LEN
            && b[1..b.len() - 1]
                .iter()
                .all(|&c| (b'!'..=b'~').contains(&c) && c != b'"');
        if ok {
            Ok(Revision(value.to_owned()))
        } else {
            Err(IdError::Revision)
        }
    }

    /// The revision, quotes included, as it goes into `If-Match`.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(Revision);

/// A Google calendar id taken from a calendar list (5.3, 6.2, PD21, PD22): non-empty, at most
/// 1024 characters, printable ASCII, no `/`, and neither `.` nor `..`. It may hold `@`, `#`
/// and other characters; it always goes into the path as one percent-encoded segment.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct CalendarId(String);

impl CalendarId {
    /// Checks an id that a calendar list returned, or that the store kept from one (8). An
    /// entry whose id fails is left out of the list (6.2). Never call it with `primary` from
    /// code: no request names Google's keyword (PD22).
    pub fn new(value: &str) -> Result<CalendarId, IdError> {
        let ok = (1..=MAX_ID_LEN).contains(&value.len())
            && value.bytes().all(|b| (b' '..=b'~').contains(&b))
            && !value.contains('/')
            && value != "."
            && value != "..";
        if ok {
            Ok(CalendarId(value.to_owned()))
        } else {
            Err(IdError::CalendarId)
        }
    }

    /// The id, verbatim, for a request body (6.3) and for the store.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(CalendarId);

#[cfg(test)]
mod tests {
    use super::*;

    const HOSTILE: [&str; 13] = [
        "", ".", "..", "%2e%2e", "%2E%2E", "a/b", "a?b", "a#b", "a b", "a.b", "a-b", "*", "a\u{e9}",
    ];

    #[test]
    fn event_ids_accept_letters_digits_and_underscores_only() {
        for good in [
            "a",
            "Z9_",
            "_20261117T090000Z",
            "abcdefghijklmnopqrstuv0123456789",
        ] {
            assert_eq!(EventId::new(good).expect("valid").as_str(), good);
        }
        assert!(EventId::new(&"a".repeat(1024)).is_ok());
        assert_eq!(EventId::new(&"a".repeat(1025)), Err(IdError::EventId));
        for bad in HOSTILE {
            assert_eq!(EventId::new(bad), Err(IdError::EventId), "{bad:?}");
        }
        assert!(EventId::new("a\0").is_err());
        assert!(EventId::new("a\n").is_err());
    }

    #[test]
    fn reserved_ids_are_lower_case_base32hex_of_5_to_1024() {
        let id = ReservedEventId::new("0123456789abcdefghijklmnopqrstuv").expect("valid");
        assert_eq!(id.as_event_id().as_str(), id.as_str());
        assert!(ReservedEventId::new("abcde").is_ok());
        assert!(ReservedEventId::new(&"v".repeat(1024)).is_ok());
        for bad in [
            "abcd",
            "abcdw",
            "ABCDE",
            "abcd_",
            "abcde ",
            &"v".repeat(1025),
        ] {
            assert_eq!(
                ReservedEventId::new(bad),
                Err(IdError::ReservedEventId),
                "{bad:?}"
            );
        }
        for bad in HOSTILE {
            assert!(ReservedEventId::new(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_revision_is_exactly_one_strong_entity_tag() {
        for good in ["\"x\"", "\"3381234567890000\"", "\"MTc2MzM5\"", "\"a'b!~\""] {
            assert_eq!(Revision::new(good).expect("valid").as_str(), good);
        }
        let longest = format!("\"{}\"", "a".repeat(256));
        assert!(Revision::new(&longest).is_ok());
        let too_long = format!("\"{}\"", "a".repeat(257));
        for bad in [
            "*",
            "W/\"x\"",
            "\"a\", \"b\"",
            "\"a\",\"b\"",
            "\"\"",
            "\"",
            "x",
            "\"x",
            "x\"",
            "\"a b\"",
            "\"a\tb\"",
            "\"\u{e9}\"",
            " \"x\"",
            "\"x\" ",
            "",
            &too_long,
        ] {
            assert_eq!(Revision::new(bad), Err(IdError::Revision), "{bad:?}");
        }
    }

    #[test]
    fn calendar_ids_from_a_list_refuse_slash_dots_empty_long_and_non_ascii() {
        for good in [
            "owner@example.com",
            "en.usa#holiday@group.v.calendar.google.com",
            "a b",
            "...",
            "%2e%2e",
            "a?b",
            "c_0123@group.calendar.google.com",
        ] {
            assert_eq!(CalendarId::new(good).expect("valid").as_str(), good);
        }
        assert!(CalendarId::new(&"c".repeat(1024)).is_ok());
        for bad in [
            "",
            ".",
            "..",
            "a/b",
            "/",
            "a\u{e9}",
            "a\tb",
            "a\nb",
            "a\u{7f}",
            &"c".repeat(1025),
        ] {
            assert_eq!(CalendarId::new(bad), Err(IdError::CalendarId), "{bad:?}");
        }
    }

    #[test]
    fn debug_prints_the_kind_and_length_never_the_value() {
        let calendar = CalendarId::new("owner@example.com").expect("valid");
        let event = EventId::new("secretevent").expect("valid");
        let revision = Revision::new("\"secretrev\"").expect("valid");
        let reserved = ReservedEventId::new("secretabc").expect("valid");
        let text = format!("{calendar:?} {event:?} {revision:?} {reserved:?}");
        assert_eq!(
            text,
            "CalendarId(len 17) EventId(len 11) Revision(len 11) ReservedEventId(len 9)"
        );
        assert!(!format!("{:?}", IdError::CalendarId).contains("owner"));
    }
}
