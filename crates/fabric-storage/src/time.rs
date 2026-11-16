//! UTC timestamps of the contract (`2026-11-20T16:00:00.000Z`) formatted from Unix milliseconds.
//!
//! No time crate: the date comes from the day count by H. Hinnant's `civil_from_days`, the
//! inverse of the `days_from_civil` the contract crate uses to read a timestamp back.

use fabric_desktop_contracts::scalar::Timestamp;

/// The last instant the contract's timestamp pattern can write: `9999-12-31T23:59:59.999Z`.
pub const MAX_TIMESTAMP_MS: u64 = 253_402_300_799_999;

const MS_PER_DAY: u64 = 86_400_000;

/// The contract timestamp of `unix_ms` milliseconds after 1970-01-01T00:00:00.000Z, in the
/// proleptic Gregorian calendar. A later instant than [`MAX_TIMESTAMP_MS`] is written as that
/// instant, because the pattern has four year digits.
#[must_use]
pub fn format_timestamp(unix_ms: u64) -> Timestamp {
    let ms = unix_ms.min(MAX_TIMESTAMP_MS);
    let days = ms / MS_PER_DAY;
    let in_day = ms % MS_PER_DAY;
    let (year, month, day) = civil_from_days(days);
    let text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.{:03}Z",
        in_day / 3_600_000,
        in_day / 60_000 % 60,
        in_day / 1000 % 60,
        in_day % 1000,
    );
    Timestamp::new(text).expect("a date from 1970 to 9999 matches the timestamp pattern")
}

/// The proleptic Gregorian date of a count of days since 1970-01-01 (H. Hinnant,
/// `civil_from_days`, restricted to days on or after the epoch).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    // Shift the epoch to 0000-03-01, so that a leap day is the last day of its year.
    let z = days + 719_468;
    let era = z / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_instants() {
        let cases = [
            (0, "1970-01-01T00:00:00.000Z"),
            (94_694_399_999, "1972-12-31T23:59:59.999Z"),
            (951_782_400_000, "2000-02-29T00:00:00.000Z"),
            (1_709_251_199_999, "2024-02-29T23:59:59.999Z"),
            (1_795_190_400_000, "2026-11-20T16:00:00.000Z"),
            (4_107_542_399_999, "2100-02-28T23:59:59.999Z"),
            (4_107_542_400_000, "2100-03-01T00:00:00.000Z"),
            (MAX_TIMESTAMP_MS, "9999-12-31T23:59:59.999Z"),
        ];
        for (ms, text) in cases {
            assert_eq!(format_timestamp(ms).as_str(), text, "{ms}");
        }
    }

    #[test]
    fn a_later_instant_is_written_as_the_last_one() {
        assert_eq!(
            format_timestamp(u64::MAX).as_str(),
            "9999-12-31T23:59:59.999Z"
        );
    }

    #[test]
    fn the_contract_crate_reads_every_day_back() {
        // Every day from 1970 to 2200 at a time with every field non-zero, read back by the
        // contract crate's own `days_from_civil`.
        let offset = 13 * 3_600_000 + 7 * 60_000 + 9 * 1000 + 42;
        for day in 0..84_000_u64 {
            let ms = day * MS_PER_DAY + offset;
            let stamp = format_timestamp(ms);
            assert_eq!(
                stamp.unix_millis(),
                Some(i64::try_from(ms).unwrap()),
                "{stamp}"
            );
        }
    }
}
