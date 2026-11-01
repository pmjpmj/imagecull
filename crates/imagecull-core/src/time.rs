//! What time it is, spelled the way a sidecar records it.
//!
//! RFC 3339 in UTC, hand-rolled: a date crate for one timestamp format is a
//! dependency this tool can do without, and `group` already carries the inverse
//! calendar arithmetic for capture times.

use std::time::{SystemTime, UNIX_EPOCH};
/// The current time, RFC 3339 in UTC.
pub fn now_rfc3339() -> String {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    timestamp(since_epoch.as_secs())
}

/// `seconds` since the Unix epoch as `YYYY-MM-DDTHH:MM:SSZ`.
fn timestamp(seconds: u64) -> String {
    let (year, month, day) = civil_from_days(seconds / 86_400);
    let rest = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

/// The calendar date of a day count from 1970-01-01: Howard Hinnant's
/// `civil_from_days`, which is the whole of the calendar arithmetic a timestamp
/// needs and one dependency cheaper than a date crate.
fn civil_from_days(days: u64) -> (i64, u64, u64) {
    let days = i64::try_from(days).unwrap_or(i64::MAX);
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    };
    let year = if month <= 2 { year + 1 } else { year };
    (
        year,
        u64::try_from(month).unwrap_or(0),
        u64::try_from(day).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_timestamp_is_rfc_3339_utc() {
        assert_eq!(timestamp(0), "1970-01-01T00:00:00Z");
        // Expected values checked against an independent calendar, not against
        // this function: a leap day, a plain date, and the last second of 2099.
        assert_eq!(timestamp(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(timestamp(1_762_593_242), "2025-11-08T09:14:02Z");
        assert_eq!(timestamp(4_102_444_799), "2099-12-31T23:59:59Z");
    }
}
