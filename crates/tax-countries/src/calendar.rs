//! The countries' calendars: local dates in a registry time zone, and the
//! date arithmetic their rules count in.
//!
//! Each zone is a rule, not a copy of the tz database: a standard offset and,
//! for the European zones, the EU summer time in force since 1996 (one hour
//! ahead from 01:00 UTC on the last Sunday of March to 01:00 UTC on the last
//! Sunday of October). That is what tzdata 2026e says for every supported
//! zone from 1996 on, and what it projects for later years. Bitcoin is
//! younger than that, so an earlier instant has no calendar here and a
//! caller fails closed on it instead of guessing an offset.

/// A proleptic Gregorian calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

const US_PER_SECOND: i64 = 1_000_000;
const SECONDS_PER_DAY: i64 = 86_400;
const US_PER_DAY: i64 = SECONDS_PER_DAY * US_PER_SECOND;

fn is_leap(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn month_length(year: i32, month: u32) -> u32 {
    match month {
        2 if is_leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

impl Date {
    /// The date, if it exists.
    pub fn new(year: i32, month: u32, day: u32) -> Option<Date> {
        ((1..=12).contains(&month) && (1..=month_length(year, month)).contains(&day))
            .then_some(Date { year, month, day })
    }

    /// Reads `YYYY-MM-DD`.
    pub fn parse(text: &str) -> Option<Date> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| {
            let part = &text[range];
            part.bytes()
                .all(|b| b.is_ascii_digit())
                .then(|| part.parse::<u32>().ok())
                .flatten()
        };
        Date::new(
            i32::try_from(digits(0..4)?).ok()?,
            digits(5..7)?,
            digits(8..10)?,
        )
    }

    /// Days since 1970-01-01 (negative before).
    pub fn to_days(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let day_of_year = (153 * ((month + 9) % 12) + 2) / 5 + i64::from(self.day) - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The date `days` after 1970-01-01.
    pub fn from_days(days: i64) -> Date {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let mp = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        Date {
            year: (year_of_era + era * 400 + i64::from(month <= 2)) as i32,
            month: month as u32,
            day: day as u32,
        }
    }

    /// The same day `months` later, on the month's last day when it is
    /// shorter (31 August + 6 months is 28 February; 29 February + 12 months
    /// is 28 February in a common year).
    pub fn add_months(self, months: u32) -> Date {
        let index = self.year * 12 + (self.month as i32 - 1) + months as i32;
        let (year, month) = (index.div_euclid(12), index.rem_euclid(12) as u32 + 1);
        Date {
            year,
            month,
            day: self.day.min(month_length(year, month)),
        }
    }

    /// The anniversary `years` later, 29 February's being 28 February in a
    /// common year (§ 188 Abs. 3 BGB; the reading the US specification
    /// chose).
    pub fn anniversary(self, years: u32) -> Date {
        self.add_months(years * 12)
    }

    /// `YYYY-MM-DD`.
    pub fn iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// Held more than one year: the period starts the day after `acquired` and
/// includes `disposed`, so it exceeds a year exactly when `disposed` is after
/// the anniversary. Germany's § 23 period ends with the anniversary too (a
/// disposal on it is still within the year), so "taxable within the year"
/// is the negation (`docs/reference/tax-country-de.md`, DE-HOLD-CAL).
pub fn held_more_than_one_year(acquired: Date, disposed: Date) -> bool {
    disposed > acquired.anniversary(1)
}

/// How a zone moves its clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Summer {
    /// Never.
    None,
    /// The EU rule: one hour ahead from 01:00 UTC on the last Sunday of
    /// March until 01:00 UTC on the last Sunday of October.
    Eu,
}

/// An IANA time zone the registry supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zone {
    pub name: &'static str,
    /// Seconds east of UTC outside summer time.
    standard: i32,
    summer: Summer,
    /// The first instant (µs since the epoch) the rule describes.
    since_us: i64,
}

/// 1996-01-01T00:00:00Z, when every European zone here follows the rule.
const EU_RULE_SINCE_US: i64 = 820_454_400 * US_PER_SECOND;

/// UTC: every instant has a date.
pub const UTC: Zone = Zone {
    name: "UTC",
    standard: 0,
    summer: Summer::None,
    since_us: i64::MIN,
};

const fn eu(name: &'static str, standard: i32) -> Zone {
    Zone {
        name,
        standard,
        summer: Summer::Eu,
        since_us: EU_RULE_SINCE_US,
    }
}

/// Every zone a registry country may name.
pub const ZONES: &[Zone] = &[
    UTC,
    eu("Europe/Berlin", 3600),
    eu("Europe/Dublin", 0),
    eu("Europe/London", 0),
    eu("Europe/Madrid", 3600),
    eu("Europe/Vienna", 3600),
    eu("Europe/Zurich", 3600),
    // No summer time since 1951; the same validity as the others keeps one
    // rule for "Kassiber has a calendar here".
    Zone {
        name: "Asia/Tokyo",
        standard: 9 * 3600,
        summer: Summer::None,
        since_us: EU_RULE_SINCE_US,
    },
];

/// The zone named `name`.
pub fn zone(name: &str) -> Option<Zone> {
    ZONES.iter().copied().find(|zone| zone.name == name)
}

/// An instant before a zone's calendar starts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutOfRange {
    pub zone: &'static str,
    pub us: i64,
}

impl std::fmt::Display for OutOfRange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Kassiber has no {} calendar for the instant {} µs after 1970-01-01 UTC (it starts on 1996-01-01)",
            self.zone, self.us
        )
    }
}

/// The instant (µs) of 01:00 UTC on the last Sunday of `month`.
fn last_sunday_one_utc(year: i32, month: u32) -> i64 {
    let last = Date {
        year,
        month,
        day: month_length(year, month),
    }
    .to_days();
    // 1970-01-01 was a Thursday: (days + 4) mod 7 is 0 on Sundays.
    let sunday = last - (last + 4).rem_euclid(7);
    sunday * US_PER_DAY + 3600 * US_PER_SECOND
}

impl Zone {
    /// Seconds east of UTC outside summer time.
    pub fn standard_offset_s(&self) -> i32 {
        self.standard
    }

    /// Whether the zone keeps the EU's summer time (one hour ahead from
    /// the last Sunday of March to the last Sunday of October).
    pub fn eu_summer_time(&self) -> bool {
        self.summer == Summer::Eu
    }

    /// The UTC offset in seconds at the instant `us`.
    pub fn offset_s(&self, us: i64) -> Result<i32, OutOfRange> {
        if us < self.since_us {
            return Err(OutOfRange {
                zone: self.name,
                us,
            });
        }
        Ok(match self.summer {
            Summer::None => self.standard,
            Summer::Eu => {
                let year = Date::from_days(us.div_euclid(US_PER_DAY)).year;
                let summer =
                    last_sunday_one_utc(year, 3) <= us && us < last_sunday_one_utc(year, 10);
                self.standard + if summer { 3600 } else { 0 }
            }
        })
    }

    /// The local calendar date at the instant `us`.
    pub fn local_date(&self, us: i64) -> Result<Date, OutOfRange> {
        let offset = i64::from(self.offset_s(us)?);
        let local = i128::from(us) + i128::from(offset) * i128::from(US_PER_SECOND);
        let days = local.div_euclid(i128::from(US_PER_DAY));
        Ok(Date::from_days(days as i64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    /// Offsets Python's zoneinfo gives with tzdata 2026e
    /// (`scripts/gen_zone_vectors.py`): a second before and at each change
    /// of 1996, 2009, 2024, 2025, 2038 and 2099, and four instants around
    /// midnight, per zone.
    const ZONEINFO: &str = include_str!("../tests/data/zoneinfo.txt");

    #[test]
    fn offsets_match_zoneinfo() {
        let mut checked = 0;
        for line in ZONEINFO.lines().filter(|line| !line.starts_with('#')) {
            let fields: Vec<&str> = line.split(' ').collect();
            let (name, us, offset) = (fields[0], fields[1].parse().unwrap(), fields[2].parse());
            assert_eq!(
                zone(name).unwrap().offset_s(us),
                Ok(offset.unwrap()),
                "{line}"
            );
            checked += 1;
        }
        assert_eq!(checked, 196);
    }

    #[test]
    fn local_dates_follow_the_zone() {
        // de-06: 2023-06-10T22:30Z is 11 June in Berlin.
        let berlin = zone("Europe/Berlin").unwrap();
        let us = |text: &str| {
            (date(&text[..10]).to_days() * SECONDS_PER_DAY
                + text[11..13].parse::<i64>().unwrap() * 3600
                + text[14..16].parse::<i64>().unwrap() * 60)
                * US_PER_SECOND
        };
        assert_eq!(
            berlin.local_date(us("2023-06-10T22:30")),
            Ok(date("2023-06-11"))
        );
        assert_eq!(
            berlin.local_date(us("2023-06-10T21:59")),
            Ok(date("2023-06-10"))
        );
        // gb-09: 2025-04-05T23:30Z is 6 April in London.
        let london = zone("Europe/London").unwrap();
        assert_eq!(
            london.local_date(us("2025-04-05T23:30")),
            Ok(date("2025-04-06"))
        );
        assert_eq!(
            london.local_date(us("2024-12-31T23:30")),
            Ok(date("2024-12-31"))
        );
        // jp-16: 2025-12-31T15:30Z is 1 January in Tokyo.
        let tokyo = zone("Asia/Tokyo").unwrap();
        assert_eq!(
            tokyo.local_date(us("2025-12-31T15:30")),
            Ok(date("2026-01-01"))
        );
        assert_eq!(
            tokyo.local_date(us("2025-12-31T14:59")),
            Ok(date("2025-12-31"))
        );
        assert_eq!(UTC.local_date(-1), Ok(date("1969-12-31")));
        // No calendar before 1996 outside UTC.
        assert_eq!(
            berlin.local_date(EU_RULE_SINCE_US - 1),
            Err(OutOfRange {
                zone: "Europe/Berlin",
                us: EU_RULE_SINCE_US - 1
            })
        );
        assert!(berlin.local_date(EU_RULE_SINCE_US).is_ok());
        assert_eq!(zone("Europe/Paris"), None);
        assert_eq!(zone("utc"), None);
    }

    #[test]
    fn more_than_one_year_counts_calendar_dates() {
        // IRS Publication 544: 2024-06-17 to 2025-06-17 is not more than a
        // year; 2025-06-18 is.
        assert!(!held_more_than_one_year(
            date("2024-06-17"),
            date("2025-06-17")
        ));
        assert!(held_more_than_one_year(
            date("2024-06-17"),
            date("2025-06-18")
        ));
        // de-04: 29 February's anniversary is 28 February.
        assert!(!held_more_than_one_year(
            date("2024-02-29"),
            date("2025-02-28")
        ));
        assert!(held_more_than_one_year(
            date("2024-02-29"),
            date("2025-03-01")
        ));
        // de-05: 365 days across 29 February are still within the year.
        assert!(!held_more_than_one_year(
            date("2023-03-01"),
            date("2024-02-29")
        ));
        assert!(held_more_than_one_year(
            date("2023-03-01"),
            date("2024-03-02")
        ));
        assert_eq!(date("2024-02-29").anniversary(4), date("2028-02-29"));
    }

    #[test]
    fn months_clamp_to_the_months_end() {
        // ch-11: 31 August + 6 months is 28 February.
        assert_eq!(date("2024-08-31").add_months(6), date("2025-02-28"));
        assert_eq!(date("2023-08-31").add_months(6), date("2024-02-29"));
        assert_eq!(date("2024-01-31").add_months(6), date("2024-07-31"));
        assert_eq!(date("2024-11-15").add_months(2), date("2025-01-15"));
        assert_eq!(date("2024-01-31").add_months(0), date("2024-01-31"));
    }

    #[test]
    fn dates_round_trip_and_parse_strictly() {
        for days in [-719_468_i64, -1, 0, 1, 59, 60, 365, 10_957, 2_932_896] {
            assert_eq!(Date::from_days(days).to_days(), days);
        }
        assert_eq!(Date::from_days(0), date("1970-01-01"));
        assert_eq!(Date::parse("2023-02-29"), None);
        assert_eq!(Date::parse("2023-2-01"), None);
        assert_eq!(Date::parse("2023-02-+1"), None);
        assert_eq!(date("2024-02-29").iso(), "2024-02-29");
        assert_eq!(Date::new(2024, 13, 1), None);
    }
}
