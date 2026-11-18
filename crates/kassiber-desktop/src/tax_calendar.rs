//! The tax countries' zone calendars, compiled from the same source as the
//! `kassiber-tax-countries` crate (`crates/tax-countries/src/calendar.rs`). That file uses only
//! the standard library, so the desktop finds a tax year's bounds in the book's zone with the
//! core's own rules (no time zone database of its own) without depending on the backend
//! workspace. The crate is edition 2021 and formatted there; this crate's edition 2024 lints and
//! rustfmt must not rewrite it.

#[path = "../../tax-countries/src/calendar.rs"]
#[allow(dead_code)]
#[rustfmt::skip]
pub mod calendar;

use calendar::Date;

/// `YYYY-MM-DDTHH:MM:SSZ` of a UTC instant in µs, as the core's transaction reads write bounds.
fn utc_text(us: i64) -> String {
    let secs = us.div_euclid(1_000_000);
    let day = Date::from_days(secs.div_euclid(86_400));
    let clock = secs.rem_euclid(86_400);
    format!(
        "{}T{:02}:{:02}:{:02}Z",
        day.iso(),
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

/// The tax year `year` that starts on (month, day) in `zone`, as the transaction list's
/// inclusive `since`/`until` bounds: its first local midnight, and the last whole second before
/// the next year's (the core's dashboard years bound the same way). None for a zone the
/// registry doesn't know or a date before its calendar starts.
pub fn tax_year_window(
    zone: &str,
    (month, day): (u32, u32),
    year: i32,
) -> Option<(String, String)> {
    let zone = calendar::zone(zone)?;
    let first = Date::new(year, month, day)?;
    let next = Date::new(year.checked_add(1)?, month, day)?;
    let start = zone.local_midnight(first).ok()?;
    let end = zone.local_midnight(next).ok()?;
    Some((utc_text(start), utc_text(end - 1_000_000)))
}

/// The local date (`YYYY-MM-DD`) in `zone` of a `YYYY-MM-DDTHH:MM:SSZ` instant.
pub fn local_day(zone: &str, instant: &str) -> Option<String> {
    let zone = calendar::zone(zone)?;
    let date = Date::parse(instant.get(..10)?)?;
    let clock = instant.get(11..19)?;
    let (h, m, s): (i64, i64, i64) = (
        clock.get(0..2)?.parse().ok()?,
        clock.get(3..5)?.parse().ok()?,
        clock.get(6..8)?.parse().ok()?,
    );
    let us = (date.to_days() * 86_400 + h * 3600 + m * 60 + s) * 1_000_000;
    Some(zone.local_date(us).ok()?.iso())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The UK's 2024-25 tax year runs 6 April to 5 April in London time (summer time on both
    /// ends); New Zealand's calendar year starts at local midnight, the previous day in UTC.
    #[test]
    fn tax_year_bounds_follow_the_zone() {
        assert_eq!(
            tax_year_window("Europe/London", (4, 6), 2024),
            Some(("2024-04-05T23:00:00Z".into(), "2025-04-05T22:59:59Z".into()))
        );
        assert_eq!(
            tax_year_window("Pacific/Auckland", (1, 1), 2025),
            Some(("2024-12-31T11:00:00Z".into(), "2025-12-31T10:59:59Z".into()))
        );
        assert_eq!(
            tax_year_window("UTC", (1, 1), 2025),
            Some(("2025-01-01T00:00:00Z".into(), "2025-12-31T23:59:59Z".into()))
        );
        assert_eq!(tax_year_window("Mars/Olympus", (1, 1), 2025), None);
        assert_eq!(
            local_day("Europe/London", "2025-04-05T22:59:59Z").as_deref(),
            Some("2025-04-05")
        );
    }
}
