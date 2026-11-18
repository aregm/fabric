//! Times as the fake reads and writes them: RFC 3339 strings and Unix milliseconds.
//!
//! The fake does no zone arithmetic (14.1). It turns an RFC 3339 string with an offset into an
//! instant only to compare instants (the list window, free/busy) and reads an all-day `date` as
//! midnight UTC of that day.

/// The fake clock's start: 2026-11-17T09:00:00Z, in Unix milliseconds.
pub const DEFAULT_NOW_MS: i64 = 1_794_906_000_000;

/// Days since 1970-01-01 of a proleptic Gregorian date.
pub(crate) fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m = i64::from(month);
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + i64::from(day) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The date of a day number of [`days_from_civil`].
pub(crate) fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 => 29,
        _ => 28,
    }
}

fn number(s: &str) -> Option<u32> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Reads `YYYY-MM-DD` as midnight UTC of that day.
pub fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let year = i64::from(number(&s[0..4])?);
    let month = number(&s[5..7])?;
    let day = number(&s[8..10])?;
    if !(1..=12).contains(&month) || day == 0 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400_000)
}

/// Reads an RFC 3339 date-time with an offset (`Z` or `+hh:mm`), a fraction of a second
/// allowed, as Unix milliseconds. A fraction beyond milliseconds is cut.
pub fn parse_rfc3339(s: &str) -> Option<i64> {
    if s.len() < 20 || !s.is_ascii() {
        return None;
    }
    let date = parse_date(&s[0..10])?;
    let b = s.as_bytes();
    if !matches!(b[10], b'T' | b't') || b[13] != b':' || b[16] != b':' {
        return None;
    }
    let hour = number(&s[11..13])?;
    let minute = number(&s[14..16])?;
    let second = number(&s[17..19])?;
    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0i64;
    if let Some(fraction) = rest.strip_prefix('.') {
        let digits = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let mut padded: String = fraction[..digits].chars().take(3).collect();
        while padded.len() < 3 {
            padded.push('0');
        }
        millis = i64::from(number(&padded)?);
        rest = &fraction[digits..];
    }
    let offset_minutes: i64 = match rest {
        "Z" | "z" => 0,
        _ => {
            let ob = rest.as_bytes();
            if ob.len() != 6 || ob[3] != b':' || !matches!(ob[0], b'+' | b'-') {
                return None;
            }
            let oh = i64::from(number(&rest[1..3])?);
            let om = i64::from(number(&rest[4..6])?);
            if oh > 23 || om > 59 {
                return None;
            }
            let total = oh * 60 + om;
            if ob[0] == b'-' { -total } else { total }
        }
    };
    let local = date
        + i64::from(hour) * 3_600_000
        + i64::from(minute) * 60_000
        + i64::from(second.min(59)) * 1000
        + millis;
    Some(local - offset_minutes * 60_000)
}

/// Writes Unix milliseconds as `YYYY-MM-DDTHH:MM:SSZ`, with `.mmm` only when the milliseconds
/// are not zero.
pub fn format_utc(ms: i64) -> String {
    let base = format_seconds(ms);
    let millis = ms.rem_euclid(1000);
    if millis == 0 {
        format!("{base}Z")
    } else {
        format!("{base}.{millis:03}Z")
    }
}

/// Writes Unix milliseconds as `YYYY-MM-DDTHH:MM:SS.mmmZ`, the form of Google's `updated`.
pub fn format_utc_millis(ms: i64) -> String {
    format!("{}.{:03}Z", format_seconds(ms), ms.rem_euclid(1000))
}

fn format_seconds(ms: i64) -> String {
    let seconds = ms.div_euclid(1000);
    let days = seconds.div_euclid(86_400);
    let in_day = seconds.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        in_day / 3600,
        (in_day / 60) % 60,
        in_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_clock_is_the_seventeenth_of_november_2026() {
        assert_eq!(format_utc(DEFAULT_NOW_MS), "2026-11-17T09:00:00Z");
    }

    #[test]
    fn offsets_and_fractions_read_as_one_instant() {
        let a = parse_rfc3339("2026-11-17T10:00:00Z").unwrap();
        assert_eq!(parse_rfc3339("2026-11-17T11:00:00+01:00"), Some(a));
        assert_eq!(parse_rfc3339("2026-11-17T05:00:00.000-05:00"), Some(a));
        assert_eq!(parse_rfc3339("2026-11-17T10:00:00.1234Z"), Some(a + 123));
        assert_eq!(parse_rfc3339("2026-11-17 10:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-02-30T10:00:00Z"), None);
        assert_eq!(parse_rfc3339("2026-11-17T10:00:00"), None);
    }

    #[test]
    fn dates_round_trip() {
        for days in [-1000, 0, 365, 20_000, 20_774] {
            let (y, m, d) = civil_from_days(days);
            assert_eq!(days_from_civil(y, m, d), days);
        }
        assert_eq!(
            parse_date("2026-11-17"),
            Some(DEFAULT_NOW_MS - 9 * 3_600_000)
        );
    }
}
