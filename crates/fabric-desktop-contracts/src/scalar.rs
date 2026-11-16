//! Scalars and the generic building blocks of the transport types.
//!
//! Every scalar is a newtype that can only hold a value its schema accepts. Each pattern of
//! `schemas/common.schema.json`, `owner-channel.schema.json` and `agent-channel.schema.json`
//! is implemented by hand below, next to the pattern it mirrors. Lengths count Unicode code
//! points (README section 6).

use std::collections::BTreeSet;
use std::fmt;
use std::marker::PhantomData;
use std::ops::Deref;
use std::str::FromStr;

use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::json::{MAX_SAFE_INTEGER, canonical};

/// Why a value is not a valid instance of a scalar or a bounded list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScalarError {
    /// The schema definition the value was checked against.
    pub kind: &'static str,
    pub reason: String,
}

impl fmt::Display for ScalarError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid {}: {}", self.kind, self.reason)
    }
}

impl std::error::Error for ScalarError {}

/// Deserializes a nullable member that must be present: a missing member is an error, `null`
/// is `None` (README section 3: every property is required). Use it with
/// `#[serde(deserialize_with = "nullable")]` on every `Option` field.
pub(crate) fn nullable<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)
}

macro_rules! string_scalar {
    ($(#[$doc:meta])* $name:ident, $schema:literal, $check:path) => {
        $(#[$doc])*
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(String);

        impl $name {
            /// The schema definition this type mirrors.
            pub const SCHEMA: &'static str = $schema;

            /// Checks `value` against the definition and wraps it.
            pub fn new(value: impl Into<String>) -> Result<Self, ScalarError> {
                let value = value.into();
                if $check(&value) {
                    Ok(Self(value))
                } else {
                    Err(ScalarError {
                        kind: $schema,
                        reason: format!("{value:?} does not match"),
                    })
                }
            }

            /// Whether `value` is an instance of the definition.
            #[must_use]
            pub fn is_valid(value: &str) -> bool {
                $check(value)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }

            #[must_use]
            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                &self.0
            }
        }

        impl FromStr for $name {
            type Err = ScalarError;
            fn from_str(value: &str) -> Result<Self, ScalarError> {
                Self::new(value)
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_str(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                let value = String::deserialize(deserializer)?;
                Self::new(value).map_err(de::Error::custom)
            }
        }
    };
}

string_scalar!(
    /// `common#/$defs/id`: `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`, an opaque id of at most 128
    /// ASCII characters (README section 14).
    Id,
    "common.schema.json#/$defs/id",
    is_id
);
string_scalar!(
    /// `common#/$defs/timestamp`: a UTC instant with milliseconds, `YYYY-MM-DDTHH:MM:SS.mmmZ`
    /// (README section 14). The pattern checks ranges per field, not that the day
    /// exists in its month.
    Timestamp,
    "common.schema.json#/$defs/timestamp",
    is_timestamp
);
string_scalar!(
    /// `common#/$defs/digest`: `^sha256:[0-9a-f]{64}$` (README section 4).
    Digest,
    "common.schema.json#/$defs/digest",
    is_digest
);
string_scalar!(
    /// `common#/$defs/name_token`: `^[a-z][a-z0-9_]{0,63}$`, a rule or capability name.
    NameToken,
    "common.schema.json#/$defs/name_token",
    is_name_token
);
string_scalar!(
    /// `common#/$defs/email`: at most 320 characters; an ASCII local part of 1 to 64
    /// characters from ``[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]``, `@`, and a lowercase domain.
    Email,
    "common.schema.json#/$defs/email",
    is_email
);
string_scalar!(
    /// `common#/$defs/domain`: at most 253 characters; two or more dot-separated labels of 1
    /// to 63 characters from `[a-z0-9-]` that neither begin nor end with `-`.
    DomainName,
    "common.schema.json#/$defs/domain",
    is_domain
);
string_scalar!(
    /// `common#/$defs/iana_zone`: at most 64 characters,
    /// `^[A-Za-z][A-Za-z0-9_+-]*(/[A-Za-z0-9_+-]+)*$`. The broker checks that the zone exists.
    IanaZone,
    "common.schema.json#/$defs/iana_zone",
    is_iana_zone
);
string_scalar!(
    /// `common#/$defs/date`: `YYYY-MM-DD` with month 01–12 and day 01–31.
    Date,
    "common.schema.json#/$defs/date",
    is_date
);
string_scalar!(
    /// `common#/$defs/local_time`: `HH:MM`, 00:00 to 23:59.
    LocalTime,
    "common.schema.json#/$defs/local_time",
    is_local_time
);
string_scalar!(
    /// `common#/$defs/offset_date_time`: `YYYY-MM-DDTHH:MM:SS±HH:MM`, offset hours 00–23.
    /// `Z` is not accepted.
    OffsetDateTime,
    "common.schema.json#/$defs/offset_date_time",
    is_offset_date_time
);
string_scalar!(
    /// `common#/$defs/provider_ref`: `^[!-~]{1,1024}$`, printable ASCII without spaces.
    ProviderRef,
    "common.schema.json#/$defs/provider_ref",
    is_provider_ref
);
string_scalar!(
    /// `owner#/$defs/custom_profile_id`: `^custom-[A-Za-z0-9._-]{1,120}$` (README section 14).
    CustomProfileId,
    "owner-channel.schema.json#/$defs/custom_profile_id",
    is_custom_profile_id
);

impl Timestamp {
    /// Milliseconds since 1970-01-01T00:00:00.000Z in the proleptic Gregorian calendar, or
    /// `None` when the day does not exist in its month (the pattern allows `02-30`).
    #[must_use]
    pub fn unix_millis(&self) -> Option<i64> {
        let b = self.0.as_bytes();
        let num = |r: std::ops::Range<usize>| -> i64 {
            b[r].iter().fold(0, |n, &c| n * 10 + i64::from(c - b'0'))
        };
        let (year, month, day) = (num(0..4), num(5..7), num(8..10));
        let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
        let month_days = [
            31,
            if leap { 29 } else { 28 },
            31,
            30,
            31,
            30,
            31,
            31,
            30,
            31,
            30,
            31,
        ];
        if day > month_days[(month - 1) as usize] {
            return None;
        }
        let days = days_from_civil(year, month, day);
        let seconds = days * 86_400 + num(11..13) * 3600 + num(14..16) * 60 + num(17..19);
        Some(seconds * 1000 + num(20..23))
    }
}

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

fn is_alnum(b: u8) -> bool {
    b.is_ascii_alphanumeric()
}

fn is_id_tail(b: u8) -> bool {
    is_alnum(b) || matches!(b, b'.' | b'_' | b'-')
}

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$`
pub fn is_id(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=128).contains(&b.len()) && is_alnum(b[0]) && b[1..].iter().all(|&c| is_id_tail(c))
}

fn digits(b: &[u8]) -> bool {
    b.iter().all(u8::is_ascii_digit)
}

fn two(b: &[u8]) -> u8 {
    (b[0] - b'0') * 10 + (b[1] - b'0')
}

/// `[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])` on exactly 10 bytes.
fn date_bytes(b: &[u8]) -> bool {
    b.len() == 10
        && digits(&b[0..4])
        && b[4] == b'-'
        && digits(&b[5..7])
        && (1..=12).contains(&two(&b[5..7]))
        && b[7] == b'-'
        && digits(&b[8..10])
        && (1..=31).contains(&two(&b[8..10]))
}

/// `([01][0-9]|2[0-3]):[0-5][0-9]` on exactly 5 bytes.
fn hour_minute(b: &[u8]) -> bool {
    b.len() == 5
        && digits(&b[0..2])
        && two(&b[0..2]) <= 23
        && b[2] == b':'
        && digits(&b[3..5])
        && two(&b[3..5]) <= 59
}

/// `([01][0-9]|2[0-3]):[0-5][0-9]:[0-5][0-9]` on exactly 8 bytes.
fn hour_minute_second(b: &[u8]) -> bool {
    b.len() == 8 && hour_minute(&b[0..5]) && b[5] == b':' && digits(&b[6..8]) && two(&b[6..8]) <= 59
}

/// `^DATE T HH:MM:SS \. [0-9]{3} Z$`
pub fn is_timestamp(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 24
        && date_bytes(&b[0..10])
        && b[10] == b'T'
        && hour_minute_second(&b[11..19])
        && b[19] == b'.'
        && digits(&b[20..23])
        && b[23] == b'Z'
}

/// `^sha256:[0-9a-f]{64}$`
pub fn is_digest(s: &str) -> bool {
    s.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    })
}

/// `^[a-z][a-z0-9_]{0,63}$`
pub fn is_name_token(s: &str) -> bool {
    let b = s.as_bytes();
    (1..=64).contains(&b.len())
        && b[0].is_ascii_lowercase()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
}

/// `[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?`
fn domain_label(b: &[u8]) -> bool {
    let end_ok = |c: u8| c.is_ascii_lowercase() || c.is_ascii_digit();
    (1..=63).contains(&b.len())
        && end_ok(b[0])
        && end_ok(b[b.len() - 1])
        && b.iter().all(|&c| end_ok(c) || c == b'-')
}

/// `LABEL(\.LABEL)+`, the domain pattern without its length bound.
fn domain_labels(s: &str) -> bool {
    let labels: Vec<&str> = s.split('.').collect();
    labels.len() >= 2 && labels.iter().all(|l| domain_label(l.as_bytes()))
}

/// maxLength 253 and `^LABEL(\.LABEL)+$`
pub fn is_domain(s: &str) -> bool {
    s.chars().count() <= 253 && domain_labels(s)
}

/// maxLength 320 and ``^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]{1,64}@LABEL(\.LABEL)+$``. The local
/// part cannot hold `@`, so the first `@` is the separator.
pub fn is_email(s: &str) -> bool {
    if s.chars().count() > 320 {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = |c: u8| is_alnum(c) || b".!#$%&'*+/=?^_`{|}~-".contains(&c);
    (1..=64).contains(&local.len()) && local.bytes().all(local_ok) && domain_labels(domain)
}

/// maxLength 64 and `^[A-Za-z][A-Za-z0-9_+-]*(/[A-Za-z0-9_+-]+)*$`
pub fn is_iana_zone(s: &str) -> bool {
    let part = |c: u8| is_alnum(c) || matches!(c, b'_' | b'+' | b'-');
    if s.chars().count() > 64 {
        return false;
    }
    let mut segments = s.split('/');
    let first = segments.next().unwrap_or_default().as_bytes();
    !first.is_empty()
        && first[0].is_ascii_alphabetic()
        && first.iter().all(|&c| part(c))
        && segments.all(|seg| !seg.is_empty() && seg.bytes().all(part))
}

/// `^[0-9]{4}-(0[1-9]|1[0-2])-(0[1-9]|[12][0-9]|3[01])$`
pub fn is_date(s: &str) -> bool {
    date_bytes(s.as_bytes())
}

/// `^([01][0-9]|2[0-3]):[0-5][0-9]$`
pub fn is_local_time(s: &str) -> bool {
    hour_minute(s.as_bytes())
}

/// `^DATE T HH:MM:SS [+-] ([01][0-9]|2[0-3]):[0-5][0-9]$`
pub fn is_offset_date_time(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 25
        && date_bytes(&b[0..10])
        && b[10] == b'T'
        && hour_minute_second(&b[11..19])
        && matches!(b[19], b'+' | b'-')
        && hour_minute(&b[20..25])
}

/// `^[!-~]{1,1024}$`
pub fn is_provider_ref(s: &str) -> bool {
    (1..=1024).contains(&s.len()) && s.bytes().all(|c| (b'!'..=b'~').contains(&c))
}

/// `^custom-[A-Za-z0-9._-]{1,120}$`
pub fn is_custom_profile_id(s: &str) -> bool {
    s.strip_prefix("custom-")
        .is_some_and(|rest| (1..=120).contains(&rest.len()) && rest.bytes().all(is_id_tail))
}

/// A string of `MIN` to `MAX` Unicode code points with no pattern: labels, explanations,
/// tasks and event text.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Text<const MIN: usize, const MAX: usize>(String);

/// `common#/$defs/label`: 1 to 200 code points, rendered as plain text.
pub type Label = Text<1, 200>;
/// `common#/$defs/explanation`: 1 to 1000 code points written by the broker.
pub type Explanation = Text<1, 1000>;
/// `agent#/$defs/task`: at most 1000 code points of the agent's own words (README section 14).
pub type Task = Text<0, 1000>;
/// `agent#/$defs/event_title`: 1 to 1024 code points.
pub type EventTitle = Text<1, 1024>;
/// `agent#/$defs/event_location`: at most 1024 code points.
pub type EventLocation = Text<0, 1024>;
/// `agent#/$defs/event_description`: at most 8192 code points.
pub type EventDescription = Text<0, 8192>;
/// A title as a read discloses it: at most 1024 code points, possibly empty (`event_view.title`).
pub type ReadTitle = Text<0, 1024>;
/// `common#/$defs/nullable_method` when not null: 1 to 100 code points.
pub type MethodText = Text<1, 100>;

impl<const MIN: usize, const MAX: usize> Text<MIN, MAX> {
    pub fn new(value: impl Into<String>) -> Result<Self, ScalarError> {
        let value = value.into();
        let n = value.chars().count();
        if (MIN..=MAX).contains(&n) {
            Ok(Self(value))
        } else {
            Err(ScalarError {
                kind: "text",
                reason: format!("{n} code points, expected {MIN} to {MAX}"),
            })
        }
    }

    #[must_use]
    pub fn is_valid(value: &str) -> bool {
        (MIN..=MAX).contains(&value.chars().count())
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    #[must_use]
    pub fn into_string(self) -> String {
        self.0
    }
}

impl<const MIN: usize, const MAX: usize> fmt::Display for Text<MIN, MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const MIN: usize, const MAX: usize> Serialize for Text<MIN, MAX> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const MIN: usize, const MAX: usize> Deserialize<'de> for Text<MIN, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(String::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A JSON integer from `MIN` to `MAX`. Booleans and strings are refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Integer<const MIN: u64, const MAX: u64>(u64);

/// `common#/$defs/count`: 0 to 2^53 − 1.
pub type Count = Integer<0, { MAX_SAFE_INTEGER as u64 }>;
/// `common#/$defs/positive_count`: 1 to 2^53 − 1.
pub type PositiveCount = Integer<1, { MAX_SAFE_INTEGER as u64 }>;

impl<const MIN: u64, const MAX: u64> Integer<MIN, MAX> {
    pub fn new(value: u64) -> Result<Self, ScalarError> {
        if MIN <= value && value <= MAX {
            Ok(Self(value))
        } else {
            Err(ScalarError {
                kind: "integer",
                reason: format!("{value} is outside {MIN} to {MAX}"),
            })
        }
    }

    /// For constants: fails to compile, or panics, when `value` is out of range.
    #[must_use]
    pub const fn of(value: u64) -> Self {
        assert!(MIN <= value && value <= MAX, "integer out of range");
        Self(value)
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl<const MIN: u64, const MAX: u64> fmt::Display for Integer<MIN, MAX> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl<const MIN: u64, const MAX: u64> Serialize for Integer<MIN, MAX> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(self.0)
    }
}

impl<'de, const MIN: u64, const MAX: u64> Deserialize<'de> for Integer<MIN, MAX> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(u64::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// An array of `MIN` to `MAX` items; when `UNIQUE`, no two items have the same canonical form
/// (README section 6). The order is kept and counts (README section 3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct List<T, const MIN: usize, const MAX: usize, const UNIQUE: bool>(Vec<T>);

/// An array with no bounds and no uniqueness.
pub type AnyList<T> = List<T, 0, { usize::MAX }, false>;

impl<T: Serialize, const MIN: usize, const MAX: usize, const UNIQUE: bool>
    List<T, MIN, MAX, UNIQUE>
{
    pub fn new(items: Vec<T>) -> Result<Self, ScalarError> {
        if !(MIN..=MAX).contains(&items.len()) {
            return Err(ScalarError {
                kind: "list",
                reason: format!("{} items, expected {MIN} to {MAX}", items.len()),
            });
        }
        if UNIQUE {
            let mut seen = BTreeSet::new();
            for item in &items {
                let value = serde_json::to_value(item).map_err(|e| ScalarError {
                    kind: "list",
                    reason: e.to_string(),
                })?;
                if !seen.insert(canonical(&value)) {
                    return Err(ScalarError {
                        kind: "list",
                        reason: "two items are equal".to_owned(),
                    });
                }
            }
        }
        Ok(Self(items))
    }
}

impl<T, const MIN: usize, const MAX: usize, const UNIQUE: bool> List<T, MIN, MAX, UNIQUE> {
    #[must_use]
    pub fn into_vec(self) -> Vec<T> {
        self.0
    }
}

impl<T, const MIN: usize, const MAX: usize, const UNIQUE: bool> Deref
    for List<T, MIN, MAX, UNIQUE>
{
    type Target = [T];
    fn deref(&self) -> &[T] {
        &self.0
    }
}

impl<T: Serialize, const MIN: usize, const MAX: usize, const UNIQUE: bool> Serialize
    for List<T, MIN, MAX, UNIQUE>
{
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de, T, const MIN: usize, const MAX: usize, const UNIQUE: bool> Deserialize<'de>
    for List<T, MIN, MAX, UNIQUE>
where
    T: Deserialize<'de> + Serialize,
{
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::new(Vec::<T>::deserialize(deserializer)?).map_err(de::Error::custom)
    }
}

/// A member whose schema is `{"type": "null"}`: it is always present and always `null`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Null;

impl Serialize for Null {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_unit()
    }
}

impl<'de> Deserialize<'de> for Null {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <()>::deserialize(deserializer).map(|()| Null)
    }
}

/// A value fixed by a schema `const`: one JSON string, written by the type itself.
pub trait ConstText {
    const VALUE: &'static str;
}

/// A member whose schema is `{"const": <string>}`. It holds nothing; it reads and writes `C::VALUE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixed<C>(PhantomData<C>);

impl<C> Default for Fixed<C> {
    fn default() -> Self {
        Self(PhantomData)
    }
}

impl<C> Fixed<C> {
    #[must_use]
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<C: ConstText> Serialize for Fixed<C> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(C::VALUE)
    }
}

impl<'de, C: ConstText> Deserialize<'de> for Fixed<C> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == C::VALUE {
            Ok(Self::new())
        } else {
            Err(de::Error::custom(format!("expected {:?}", C::VALUE)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn id_allows_128_characters() {
        assert!(is_id(&"a".repeat(128)));
        assert!(!is_id(&"a".repeat(129)));
    }

    #[test]
    fn id_starts_with_a_letter_or_digit() {
        assert!(is_id("0a"));
        assert!(!is_id("-a"));
        assert!(!is_id(""));
    }

    #[test]
    fn timestamp_needs_milliseconds_and_z() {
        assert!(is_timestamp("2026-11-15T22:12:00.000Z"));
        assert!(!is_timestamp("2026-11-15T22:12:00Z"));
        assert!(!is_timestamp("2026-11-15T24:00:00.000Z"));
    }

    #[test]
    fn digest_is_lowercase_hex() {
        assert!(is_digest(&format!("sha256:{}", "a".repeat(64))));
        assert!(!is_digest(&format!("sha256:{}", "A".repeat(64))));
    }

    #[test]
    fn email_domain_is_lowercase() {
        assert!(is_email("Bob@example.com"));
        assert!(!is_email("bob@Example.com"));
        assert!(!is_email("bob@example"));
    }

    #[test]
    fn zone_segments_are_not_empty() {
        assert!(is_iana_zone("America/Argentina/Buenos_Aires"));
        assert!(!is_iana_zone("America//Chicago"));
        assert!(!is_iana_zone("America/"));
    }

    #[test]
    fn offset_date_time_refuses_z() {
        assert!(is_offset_date_time("2026-11-20T10:00:00-06:00"));
        assert!(!is_offset_date_time("2026-11-20T10:00:00Z"));
    }

    #[test]
    fn custom_profile_id_needs_a_suffix() {
        assert!(is_custom_profile_id("custom-a"));
        assert!(!is_custom_profile_id("custom-"));
    }

    #[test]
    fn text_counts_code_points() {
        assert!(Label::is_valid(&"\u{1F600}".repeat(200)));
        assert!(!Label::is_valid(&"\u{1F600}".repeat(201)));
    }

    #[test]
    fn unique_list_compares_canonical_forms() {
        type Ids = List<Id, 1, 3, true>;
        let a = Id::new("a").unwrap();
        assert!(Ids::new(vec![a.clone(), a.clone()]).is_err());
        assert!(Ids::new(vec![]).is_err());
        assert!(Ids::new(vec![a]).is_ok());
    }

    #[test]
    fn integer_refuses_booleans() {
        assert!(crate::typed::from_value::<Count>(&serde_json::json!(true)).is_err());
        assert!(crate::typed::from_value::<Count>(&serde_json::json!(-1)).is_err());
    }
}
