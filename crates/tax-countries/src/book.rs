//! A book's tax calendar: the zone whose local dates cut its tax years and
//! report periods, and its tax year's first day.
//!
//! Every book has its country's ([`Country::timezone`],
//! [`Country::tax_year_start`]). A country that lets a book choose
//! ([`Country::tax_year_editable`], [`Country::zones`]) accepts a book's own
//! start or zone; every other country's are locked to its law's, and a book
//! asking for another fails closed. This is the one place that says which
//! tax year an instant belongs to and where a tax year begins and ends:
//! the engine request, the stored tax summary and every report reader ask
//! it (`docs/reference/tax.md#tax-years`).

use crate::calendar::{self, Date, OutOfRange, Zone};
use crate::{years, Country, TAX_YEAR_PRESETS};

/// A book's tax calendar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BookCalendar {
    /// The IANA zone, one of the registry's calendars.
    pub timezone: &'static str,
    /// The tax year's first day (month, day).
    pub tax_year_start: (u8, u8),
}

/// Why a book cannot have the calendar it asked for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CalendarError {
    /// The text is neither a preset name nor a `MM-DD` day of every year.
    InvalidStart(String),
    /// The zone has no calendar in the registry.
    UnknownZone(String),
    /// The country's tax year is its law's.
    StartLocked {
        country: &'static str,
        start: (u8, u8),
    },
    /// The country's zone is its law's, or the zone is not one of the
    /// country's.
    ZoneLocked {
        country: &'static str,
        zones: Vec<&'static str>,
    },
}

impl CalendarError {
    /// The machine code callers return.
    pub fn code(&self) -> &'static str {
        match self {
            CalendarError::InvalidStart(_) => "tax_year_start_invalid",
            CalendarError::UnknownZone(_) => "timezone_unknown",
            CalendarError::StartLocked { .. } => "tax_year_start_locked",
            CalendarError::ZoneLocked { .. } => "timezone_locked",
        }
    }

    /// A hint naming what the book may choose.
    pub fn hint(&self) -> String {
        match self {
            CalendarError::InvalidStart(_) => format!(
                "Use MM-DD (not 02-29) or one of: {}",
                TAX_YEAR_PRESETS
                    .iter()
                    .map(|preset| preset.id)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            CalendarError::UnknownZone(_) => format!(
                "Use one of: {}",
                calendar::ZONES
                    .iter()
                    .map(|zone| zone.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            CalendarError::StartLocked { start, .. } => {
                format!("Its tax year starts on {}.", format_start(*start))
            }
            CalendarError::ZoneLocked { zones, .. } => format!("Use one of: {}", zones.join(", ")),
        }
    }
}

impl std::fmt::Display for CalendarError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CalendarError::InvalidStart(text) => {
                write!(f, "Unsupported tax year start '{text}'")
            }
            CalendarError::UnknownZone(zone) => write!(f, "Unsupported time zone '{zone}'"),
            CalendarError::StartLocked { country, .. } => write!(
                f,
                "The tax year of tax_country='{country}' is fixed by its law and cannot be changed"
            ),
            CalendarError::ZoneLocked { country, .. } => write!(
                f,
                "The time zone of tax_country='{country}' cannot be changed to that zone"
            ),
        }
    }
}

/// `MM-DD`.
pub fn format_start(start: (u8, u8)) -> String {
    format!("{:02}-{:02}", start.0, start.1)
}

/// Reads a tax year's first day: a preset name (`calendar`, `uk`, `april`,
/// `july`, `march`) or `MM-DD` of a day every year has (not `02-29`).
pub fn parse_start(text: &str) -> Option<(u8, u8)> {
    let text = text.trim();
    if let Some(preset) = TAX_YEAR_PRESETS
        .iter()
        .find(|preset| preset.id.eq_ignore_ascii_case(text))
    {
        return Some(preset.start);
    }
    let bytes = text.as_bytes();
    if bytes.len() != 5 || bytes[2] != b'-' {
        return None;
    }
    // 2001 is a common year, so 29 February is refused.
    Date::parse(&format!("2001-{text}")).map(|date| (date.month as u8, date.day as u8))
}

impl Country {
    /// The zones a book of this country may use, its own first.
    pub fn book_zones(&self) -> Vec<&'static str> {
        let mut zones = vec![self.timezone];
        zones.extend(
            self.zones
                .iter()
                .copied()
                .filter(|zone| *zone != self.timezone),
        );
        zones
    }

    /// The calendar a book of this country has without choices of its own.
    pub fn default_calendar(&self) -> BookCalendar {
        BookCalendar {
            timezone: self.timezone,
            tax_year_start: self.tax_year_start,
        }
    }

    /// The calendar of a book that stores `tax_year_start` and `timezone`
    /// (each `None` for the country's). A value equal to the country's is
    /// accepted everywhere; another one only where the country lets a book
    /// choose.
    pub fn book_calendar(
        &self,
        tax_year_start: Option<&str>,
        timezone: Option<&str>,
    ) -> Result<BookCalendar, CalendarError> {
        let mut book = self.default_calendar();
        if let Some(text) = tax_year_start {
            let start =
                parse_start(text).ok_or_else(|| CalendarError::InvalidStart(text.to_owned()))?;
            if start != self.tax_year_start && !self.tax_year_editable {
                return Err(CalendarError::StartLocked {
                    country: self.code,
                    start: self.tax_year_start,
                });
            }
            book.tax_year_start = start;
        }
        if let Some(name) = timezone {
            let zone = calendar::zone(name.trim())
                .ok_or_else(|| CalendarError::UnknownZone(name.to_owned()))?;
            if !self.book_zones().contains(&zone.name) {
                return Err(CalendarError::ZoneLocked {
                    country: self.code,
                    zones: self.book_zones(),
                });
            }
            book.timezone = zone.name;
        }
        Ok(book)
    }
}

impl BookCalendar {
    /// Whether this is the country's own calendar.
    pub fn is_default(&self, country: &Country) -> bool {
        *self == country.default_calendar()
    }

    /// The zone's calendar.
    pub fn zone(&self) -> Zone {
        calendar::zone(self.timezone).expect("a book calendar names a registry zone")
    }

    /// The local date of the instant `us` (µs since 1970-01-01 UTC).
    pub fn local_date(&self, us: i64) -> Result<Date, OutOfRange> {
        self.zone().local_date(us)
    }

    /// The tax year the instant `us` belongs to, named by its first day's
    /// year.
    pub fn tax_year(&self, us: i64) -> Result<i32, OutOfRange> {
        Ok(years::tax_year(self.local_date(us)?, self.tax_year_start))
    }

    /// The tax year a local date belongs to.
    pub fn tax_year_of_date(&self, local: Date) -> i32 {
        years::tax_year(local, self.tax_year_start)
    }

    /// A tax year's label: `2024`, or `2024-25` for a year that does not
    /// start on 1 January.
    pub fn label(&self, year: i32) -> String {
        years::tax_year_label(year, self.tax_year_start)
    }

    /// The tax year's first and last local day.
    pub fn days(&self, year: i32) -> (Date, Date) {
        years::tax_year_bounds(year, self.tax_year_start)
    }

    /// The tax year's bounds as instants (µs since 1970-01-01 UTC): local
    /// midnight of its first day, inclusive, and of the next year's first
    /// day, exclusive.
    pub fn bounds(&self, year: i32) -> Result<(i64, i64), OutOfRange> {
        let zone = self.zone();
        let (first, last) = self.days(year);
        Ok((
            zone.local_midnight(first)?,
            zone.local_midnight(Date::from_days(last.to_days() + 1))?,
        ))
    }

    /// The `MM-DD` of its first day.
    pub fn start_text(&self) -> String {
        format_start(self.tax_year_start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lookup;

    fn us_of(text: &str) -> i64 {
        let date = Date::parse(&text[..10]).unwrap();
        (date.to_days() * 86_400 + text[11..13].parse::<i64>().unwrap() * 3600) * 1_000_000
    }

    #[test]
    fn starts_parse_presets_and_days_of_every_year() {
        assert_eq!(parse_start("calendar"), Some((1, 1)));
        assert_eq!(parse_start("UK"), Some((4, 6)));
        assert_eq!(parse_start("july"), Some((7, 1)));
        assert_eq!(parse_start("march"), Some((3, 1)));
        assert_eq!(parse_start("april"), Some((4, 1)));
        assert_eq!(parse_start("10-15"), Some((10, 15)));
        assert_eq!(parse_start("02-28"), Some((2, 28)));
        for refused in ["02-29", "13-01", "4-6", "04/06", "", "06-31"] {
            assert_eq!(parse_start(refused), None, "{refused}");
        }
    }

    #[test]
    fn only_a_country_that_lets_a_book_choose_accepts_its_choice() {
        let generic = lookup("generic").unwrap();
        let book = generic
            .book_calendar(Some("07-01"), Some("Europe/Vienna"))
            .unwrap();
        assert_eq!(
            (book.timezone, book.tax_year_start),
            ("Europe/Vienna", (7, 1))
        );
        assert!(!book.is_default(generic));
        assert!(generic
            .book_calendar(None, None)
            .unwrap()
            .is_default(generic));
        // Austria, the United Kingdom and every reviewed or preview country
        // with a legal year keep it; asking for the same is fine.
        let at = lookup("at").unwrap();
        assert_eq!(
            at.book_calendar(Some("07-01"), None).unwrap_err().code(),
            "tax_year_start_locked"
        );
        assert!(at
            .book_calendar(Some("01-01"), Some("Europe/Vienna"))
            .is_ok());
        assert_eq!(
            at.book_calendar(None, Some("UTC")).unwrap_err().code(),
            "timezone_locked"
        );
        let gb = lookup("gb").unwrap();
        assert_eq!(
            gb.book_calendar(Some("01-01"), None).unwrap_err().code(),
            "tax_year_start_locked"
        );
        // Australia keeps 1 July but lets a book choose its state's zone.
        let au = lookup("au").unwrap();
        let perth = au.book_calendar(None, Some("Australia/Perth")).unwrap();
        assert_eq!(
            (perth.timezone, perth.tax_year_start),
            ("Australia/Perth", (7, 1))
        );
        assert_eq!(
            au.book_calendar(None, Some("Europe/Vienna"))
                .unwrap_err()
                .code(),
            "timezone_locked"
        );
        assert_eq!(
            generic
                .book_calendar(None, Some("Mars/Olympus"))
                .unwrap_err()
                .code(),
            "timezone_unknown"
        );
        assert_eq!(
            generic
                .book_calendar(Some("02-29"), None)
                .unwrap_err()
                .code(),
            "tax_year_start_invalid"
        );
        for country in crate::COUNTRIES {
            assert_eq!(
                country.tax_year_editable,
                country.code == "generic",
                "{}",
                country.code
            );
        }
    }

    #[test]
    fn a_calendar_names_years_and_bounds_them_in_its_zone() {
        let au = lookup("au").unwrap().default_calendar();
        // 2024-06-30T15:00Z is 1 July 01:00 in Sydney.
        assert_eq!(au.tax_year(us_of("2024-06-30T15")), Ok(2024));
        assert_eq!(au.tax_year(us_of("2024-06-30T13")), Ok(2023));
        assert_eq!(au.label(2024), "2024-25");
        assert_eq!(
            au.bounds(2024),
            Ok((us_of("2024-06-30T14"), us_of("2025-06-30T14")))
        );
        let gb = lookup("gb").unwrap().default_calendar();
        // 6 April 2025 is summer time in London.
        assert_eq!(
            gb.bounds(2025),
            Ok((us_of("2025-04-05T23"), us_of("2026-04-05T23")))
        );
        assert_eq!(gb.days(2024).1.iso(), "2025-04-05");
        let generic = lookup("generic").unwrap().default_calendar();
        assert_eq!(generic.label(2024), "2024");
        assert_eq!(
            generic.bounds(2024),
            Ok((us_of("2024-01-01T00"), us_of("2025-01-01T00")))
        );
        let za = lookup("za").unwrap().default_calendar();
        // The year from 1 March 2024 ends on 28 February 2025.
        assert_eq!(za.days(2024).1.iso(), "2025-02-28");
        assert_eq!(za.days(2023).1.iso(), "2024-02-29");
    }
}
