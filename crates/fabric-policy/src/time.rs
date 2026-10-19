//! UTC ISO 8601 conversion for denial messages and fixtures, without a date crate.

/// Parses `YYYY-MM-DDTHH:MM[:SS]Z` into Unix milliseconds.
#[must_use]
pub fn parse_utc(text: &str) -> Option<i64> {
    let body = text.trim().strip_suffix('Z')?;
    let (date, clock) = body.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut clock_parts = clock.split(':');
    let hour: i64 = clock_parts.next()?.parse().ok()?;
    let minute: i64 = clock_parts.next()?.parse().ok()?;
    let second: i64 = clock_parts.next().map_or(Some(0), |s| s.parse().ok())?;
    if date_parts.next().is_some()
        || clock_parts.next().is_some()
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    Some((days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second) * 1_000)
}

/// Formats Unix milliseconds as `YYYY-MM-DDTHH:MMZ`, or with seconds when they are not zero.
#[must_use]
pub fn format_utc(ms: i64) -> String {
    let seconds = ms.div_euclid(1_000);
    let (year, month, day) = civil_from_days(seconds.div_euclid(86_400));
    let rest = seconds.rem_euclid(86_400);
    let (hour, minute, second) = (rest / 3_600, rest % 3_600 / 60, rest % 60);
    if second == 0 {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}Z")
    } else {
        format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
    }
}

// Howard Hinnant's civil-calendar algorithms.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let year_of_era = year - era * 400;
    let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = (if days >= 0 { days } else { days - 146_096 }) / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::{format_utc, parse_utc};

    #[test]
    fn round_trips_the_rendezvous_epoch() {
        assert_eq!(parse_utc("2027-01-15T08:00:00Z"), Some(1_800_000_000_000));
        assert_eq!(parse_utc("2027-01-15T08:00Z"), Some(1_800_000_000_000));
        assert_eq!(format_utc(1_800_000_000_000), "2027-01-15T08:00Z");
        assert_eq!(format_utc(1_800_000_001_000), "2027-01-15T08:00:01Z");
        assert_eq!(parse_utc("2027-01-15T08:00:00+01:00"), None);
    }
}
