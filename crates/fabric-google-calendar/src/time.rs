//! Times between Google's RFC 3339 strings and the contract's types (6.9, ADR-111).
//!
//! The adapter does no zone arithmetic. It parses RFC 3339 to an instant only to compare
//! instants (6.12, section 7) and to cut busy periods to a window (6.3); it formats an instant
//! as the contract's `Timestamp`; and it turns a Google `dateTime` into the contract's
//! `OffsetDateTime` by rewriting `Z` as `+00:00` and dropping a fraction of a second.
//!
//! The contract crate gives the target types and their checks ([`Timestamp`],
//! [`OffsetDateTime`]); its day count is private, so the civil-date arithmetic is written here
//! (H. Hinnant's algorithms, as the contract and the store use).

use std::fmt;

use fabric_desktop_contracts::scalar::{OffsetDateTime, Timestamp};

/// The earliest instant a [`Timestamp`] can hold: 0000-01-01T00:00:00.000Z.
pub const MIN_TIMESTAMP_MS: i64 = -62_167_219_200_000;

/// The latest instant a [`Timestamp`] can hold: 9999-12-31T23:59:59.999Z.
pub const MAX_TIMESTAMP_MS: i64 = 253_402_300_799_999;

/// Why a time was refused. Fixed labels; the text is not kept (PD27).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimeError {
    /// Not `YYYY-MM-DDTHH:MM:SS[.fraction](Z|±HH:MM)`, or a field out of its range, or a day
    /// that its month does not have.
    NotRfc3339,
    /// The instant is outside what a contract `Timestamp` can hold (years 0000 to 9999).
    OutOfRange,
}

impl TimeError {
    /// The fixed label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            TimeError::NotRfc3339 => "the time is not an RFC 3339 date-time",
            TimeError::OutOfRange => "the time is outside years 0000 to 9999",
        }
    }
}

impl fmt::Display for TimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for TimeError {}

/// Days from 1970-01-01 to a proleptic Gregorian date (H. Hinnant, `days_from_civil`).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The proleptic Gregorian date of a day count from 1970-01-01 (H. Hinnant,
/// `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// The parts of an RFC 3339 date-time, checked.
struct Parsed {
    /// Seconds of the local date and time from 1970-01-01T00:00:00, before the offset.
    local_seconds: i64,
    /// Milliseconds of the fraction, truncated.
    millis: i64,
    /// The offset east of UTC in seconds.
    offset_seconds: i64,
    /// Where the fraction or the zone starts (byte 19).
    zone_at: usize,
}

/// Reads `n` ASCII digits at `b[at..]`.
fn number(b: &[u8], at: usize, n: usize) -> Option<i64> {
    let digits = b.get(at..at + n)?;
    digits
        .iter()
        .all(u8::is_ascii_digit)
        .then(|| digits.iter().fold(0, |v, &c| v * 10 + i64::from(c - b'0')))
}

/// Parses `YYYY-MM-DDTHH:MM:SS[.d{1,9}](Z|±HH:MM)`. `T` and `Z` must be upper case (Google
/// writes them so). A leap second (`:60`) is refused.
fn parse(text: &str) -> Option<Parsed> {
    let b = text.as_bytes();
    let year = number(b, 0, 4)?;
    let month = number(b, 5, 2)?;
    let day = number(b, 8, 2)?;
    let hour = number(b, 11, 2)?;
    let minute = number(b, 14, 2)?;
    let second = number(b, 17, 2)?;
    let separators = [(4, b'-'), (7, b'-'), (10, b'T'), (13, b':'), (16, b':')];
    if separators.iter().any(|&(at, c)| b[at] != c) {
        return None;
    }
    if !(1..=12).contains(&month)
        || !(1..=days_in_month(year, month)).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let mut at = 19;
    let mut millis = 0;
    if b.get(at) == Some(&b'.') {
        at += 1;
        let start = at;
        while b.get(at).is_some_and(u8::is_ascii_digit) {
            at += 1;
        }
        let count = at - start;
        if !(1..=9).contains(&count) {
            return None;
        }
        // Truncated to milliseconds: the first three digits, padded.
        for i in 0..3 {
            let digit = if i < count {
                i64::from(b[start + i] - b'0')
            } else {
                0
            };
            millis = millis * 10 + digit;
        }
    }
    let offset_seconds = match b.get(at) {
        Some(b'Z') if b.len() == at + 1 => 0,
        Some(&sign @ (b'+' | b'-')) if b.len() == at + 6 && b[at + 3] == b':' => {
            let oh = number(b, at + 1, 2)?;
            let om = number(b, at + 4, 2)?;
            if oh > 23 || om > 59 {
                return None;
            }
            let magnitude = oh * 3600 + om * 60;
            if sign == b'+' { magnitude } else { -magnitude }
        }
        _ => return None,
    };
    let local_seconds =
        days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second;
    Some(Parsed {
        local_seconds,
        millis,
        offset_seconds,
        zone_at: 19,
    })
}

/// Parses an RFC 3339 date-time (`Z` or an offset, an optional fraction of a second, kept to
/// the millisecond by truncation) to Unix milliseconds (6.9).
pub fn parse_rfc3339_ms(text: &str) -> Result<i64, TimeError> {
    let parsed = parse(text).ok_or(TimeError::NotRfc3339)?;
    Ok((parsed.local_seconds - parsed.offset_seconds) * 1000 + parsed.millis)
}

/// Formats Unix milliseconds as the contract's `Timestamp`, `YYYY-MM-DDTHH:MM:SS.mmmZ` (6.9:
/// busy periods in UTC with milliseconds).
pub fn format_timestamp(unix_ms: i64) -> Result<Timestamp, TimeError> {
    if !(MIN_TIMESTAMP_MS..=MAX_TIMESTAMP_MS).contains(&unix_ms) {
        return Err(TimeError::OutOfRange);
    }
    let millis = unix_ms.rem_euclid(1000);
    let seconds = unix_ms.div_euclid(1000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let of_day = seconds.rem_euclid(86_400);
    let text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{millis:03}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    );
    Timestamp::new(text).map_err(|_| TimeError::OutOfRange)
}

/// Turns a Google `dateTime` into the contract's `OffsetDateTime`, `YYYY-MM-DDTHH:MM:SS±HH:MM`
/// (6.9): `Z` becomes `+00:00` and a fraction of a second is dropped. The date and time are
/// kept as written; no zone arithmetic is done.
pub fn to_offset_date_time(google: &str) -> Result<OffsetDateTime, TimeError> {
    let parsed = parse(google).ok_or(TimeError::NotRfc3339)?;
    let local = &google[..parsed.zone_at];
    let zone = &google[google.len() - 1..];
    let offset = if zone == "Z" {
        "+00:00"
    } else {
        &google[google.len() - 6..]
    };
    OffsetDateTime::new(format!("{local}{offset}")).map_err(|_| TimeError::NotRfc3339)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants_parse_to_their_unix_milliseconds() {
        let cases = [
            ("1970-01-01T00:00:00Z", 0),
            ("1970-01-01T00:00:00.001Z", 1),
            ("2026-11-17T09:30:00Z", 1_794_907_800_000),
            ("2026-11-17T10:30:00+01:00", 1_794_907_800_000),
            ("2026-11-17T04:30:00-05:00", 1_794_907_800_000),
            ("2026-11-17T15:00:00+05:30", 1_794_907_800_000),
            ("2026-11-17T09:30:00.123456789Z", 1_794_907_800_123),
            ("2026-11-17T09:30:00.5Z", 1_794_907_800_500),
            ("2026-11-17T09:30:00.99Z", 1_794_907_800_990),
            ("1969-12-31T23:59:59.999Z", -1),
            ("2000-02-29T00:00:00Z", 951_782_400_000),
        ];
        for (text, ms) in cases {
            assert_eq!(parse_rfc3339_ms(text), Ok(ms), "{text}");
        }
    }

    #[test]
    fn offsets_on_both_sides_and_at_their_limits_parse() {
        let base = parse_rfc3339_ms("2026-03-01T12:00:00Z").expect("valid");
        assert_eq!(parse_rfc3339_ms("2026-03-01T23:59:00+11:59"), Ok(base));
        assert_eq!(parse_rfc3339_ms("2026-03-01T00:01:00-11:59"), Ok(base));
        assert_eq!(parse_rfc3339_ms("2026-03-02T11:00:00+23:00"), Ok(base));
        assert_eq!(parse_rfc3339_ms("2026-02-28T13:00:00-23:00"), Ok(base));
        assert_eq!(parse_rfc3339_ms("2026-03-01T12:00:00-00:00"), Ok(base));
    }

    #[test]
    fn leap_years_follow_the_gregorian_rule() {
        assert!(parse_rfc3339_ms("2024-02-29T00:00:00Z").is_ok());
        assert!(parse_rfc3339_ms("2000-02-29T00:00:00Z").is_ok());
        assert!(parse_rfc3339_ms("1900-02-29T00:00:00Z").is_err());
        assert!(parse_rfc3339_ms("2026-02-29T00:00:00Z").is_err());
        assert!(parse_rfc3339_ms("2026-04-31T00:00:00Z").is_err());
        assert_eq!(
            format_timestamp(parse_rfc3339_ms("2024-02-29T23:59:59.999Z").expect("valid"))
                .expect("in range")
                .as_str(),
            "2024-02-29T23:59:59.999Z"
        );
        assert_eq!(
            format_timestamp(parse_rfc3339_ms("2100-03-01T00:00:00Z").expect("valid"))
                .expect("in range")
                .as_str(),
            "2100-03-01T00:00:00.000Z"
        );
    }

    #[test]
    fn garbage_is_refused() {
        for bad in [
            "",
            "2026-11-17",
            "2026-11-17T09:30Z",
            "2026-11-17T09:30:00",
            "2026-11-17 09:30:00Z",
            "2026-11-17t09:30:00Z",
            "2026-11-17T09:30:00z",
            "2026-13-01T00:00:00Z",
            "2026-00-01T00:00:00Z",
            "2026-11-00T00:00:00Z",
            "2026-11-17T24:00:00Z",
            "2026-11-17T23:60:00Z",
            "2026-11-17T23:59:60Z",
            "2026-11-17T09:30:00.Z",
            "2026-11-17T09:30:00.1234567890Z",
            "2026-11-17T09:30:00+0100",
            "2026-11-17T09:30:00+24:00",
            "2026-11-17T09:30:00+01:60",
            "2026-11-17T09:30:00+01:00 ",
            "2026-11-17T09:30:00ZZ",
            "+2026-11-17T09:30:00Z",
            "2026-11-17T09:30:00.12a4Z",
            "\u{661}026-11-17T09:30:00Z",
            "20x6-11-17T09:30:00Z",
        ] {
            assert_eq!(parse_rfc3339_ms(bad), Err(TimeError::NotRfc3339), "{bad:?}");
            assert_eq!(
                to_offset_date_time(bad).err(),
                Some(TimeError::NotRfc3339),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn timestamps_format_known_instants_and_agree_with_the_contract() {
        for (ms, text) in [
            (0, "1970-01-01T00:00:00.000Z"),
            (1_794_907_800_123, "2026-11-17T09:30:00.123Z"),
            (-1, "1969-12-31T23:59:59.999Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
        ] {
            let ts = format_timestamp(ms).expect("in range");
            assert_eq!(ts.as_str(), text);
            assert_eq!(ts.unix_millis(), Some(ms));
        }
    }

    #[test]
    fn timestamps_hold_years_0000_to_9999_and_nothing_outside() {
        let min = format_timestamp(MIN_TIMESTAMP_MS).expect("the minimum");
        assert_eq!(min.as_str(), "0000-01-01T00:00:00.000Z");
        assert_eq!(min.unix_millis(), Some(MIN_TIMESTAMP_MS));
        let max = format_timestamp(MAX_TIMESTAMP_MS).expect("the maximum");
        assert_eq!(max.as_str(), "9999-12-31T23:59:59.999Z");
        assert_eq!(max.unix_millis(), Some(MAX_TIMESTAMP_MS));
        assert_eq!(
            format_timestamp(MIN_TIMESTAMP_MS - 1),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(
            format_timestamp(MAX_TIMESTAMP_MS + 1),
            Err(TimeError::OutOfRange)
        );
        assert_eq!(format_timestamp(i64::MIN), Err(TimeError::OutOfRange));
        assert_eq!(format_timestamp(i64::MAX), Err(TimeError::OutOfRange));
        assert_eq!(
            parse_rfc3339_ms("0000-01-01T00:00:00Z"),
            Ok(MIN_TIMESTAMP_MS)
        );
        assert_eq!(
            parse_rfc3339_ms("9999-12-31T23:59:59.999Z"),
            Ok(MAX_TIMESTAMP_MS)
        );
        // An offset can carry a valid text past the bounds; parsing works, formatting refuses.
        let early = parse_rfc3339_ms("0000-01-01T00:00:00+01:00").expect("parses");
        assert_eq!(format_timestamp(early), Err(TimeError::OutOfRange));
    }

    #[test]
    fn every_day_of_four_centuries_round_trips() {
        let start = days_from_civil(1900, 1, 1);
        let end = days_from_civil(2300, 1, 1);
        for days in start..end {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
            assert!((1..=days_in_month(y, m)).contains(&d));
        }
    }

    #[test]
    fn google_date_times_become_offset_date_times_without_zone_arithmetic() {
        for (google, contract) in [
            ("2026-11-17T09:30:00Z", "2026-11-17T09:30:00+00:00"),
            ("2026-11-17T09:30:00.000Z", "2026-11-17T09:30:00+00:00"),
            ("2026-11-17T09:30:00+01:00", "2026-11-17T09:30:00+01:00"),
            ("2026-11-17T09:30:00.25-05:00", "2026-11-17T09:30:00-05:00"),
            ("2026-03-29T02:30:00+02:00", "2026-03-29T02:30:00+02:00"),
            ("2026-10-25T02:30:00+01:00", "2026-10-25T02:30:00+01:00"),
            ("2026-10-25T02:30:00+02:00", "2026-10-25T02:30:00+02:00"),
            ("0000-01-01T00:00:00-23:59", "0000-01-01T00:00:00-23:59"),
        ] {
            assert_eq!(
                to_offset_date_time(google).expect("valid").as_str(),
                contract,
                "{google}"
            );
        }
    }
}
