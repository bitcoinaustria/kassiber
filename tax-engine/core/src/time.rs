//! Timestamps as RP2 sees them.
//!
//! Python parses each timestamp once and sends its fields; the engine checks
//! they agree with each other. Ordering, eligibility, and holding periods use
//! the absolute instant; the calendar date and year use the timestamp's own
//! offset, as `datetime.date()` and `datetime.year` do.

use crate::error::{EngineError, EngineResult, ErrorClass};
use crate::model::TsInput;

const US_PER_SECOND: i64 = 1_000_000;
const US_PER_DAY: i64 = 86_400 * US_PER_SECOND;

/// A proleptic Gregorian calendar date.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CivilDate {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

/// RP2's `MIN_DATE`: earlier entries are invisible to every iteration.
pub const MIN_VISIBLE_DATE: CivilDate = CivilDate {
    year: 1970,
    month: 1,
    day: 1,
};

impl CivilDate {
    /// Parses `date.isoformat()` output (`YYYY-MM-DD`).
    pub fn parse(text: &str) -> Option<Self> {
        let bytes = text.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let number = |range: std::ops::Range<usize>| -> Option<u32> {
            let part = text.get(range)?;
            if !part.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            part.parse().ok()
        };
        let year = i32::try_from(number(0..4)?).ok()?;
        let month = number(5..7)?;
        let day = number(8..10)?;
        if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return None;
        }
        let date = CivilDate { year, month, day };
        // Out-of-range days (February 30) do not survive the round trip.
        (CivilDate::from_days(date.to_days()) == date).then_some(date)
    }

    /// Days since 1970-01-01 (negative before).
    pub fn to_days(self) -> i64 {
        let year = i64::from(self.year) - i64::from(self.month <= 2);
        let era = year.div_euclid(400);
        let year_of_era = year - era * 400;
        let month = i64::from(self.month);
        let day_of_year = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5
            + i64::from(self.day)
            - 1;
        let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
        era * 146_097 + day_of_era - 719_468
    }

    /// The date `days` after 1970-01-01.
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let mp = (5 * day_of_year + 2) / 153;
        let day = day_of_year - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        CivilDate {
            year: year as i32,
            month: month as u32,
            day: day as u32,
        }
    }
}

/// A parsed, timezone-aware timestamp.
#[derive(Clone, Debug)]
pub struct Timestamp {
    /// `str()` of the constructor argument.
    pub raw: String,
    /// Microseconds since the Unix epoch.
    pub us: i64,
    /// UTC offset in seconds.
    pub offset_s: i32,
    /// The date in the timestamp's own offset.
    pub date: CivilDate,
    /// The Europe/Vienna date, for Austrian rules.
    pub vienna_date: Option<CivilDate>,
    /// `str(datetime)`, as RP2's messages render it.
    pub display: String,
    /// `datetime.isoformat()`.
    pub iso: String,
}

impl Timestamp {
    /// Reads the fields Python sent, or raises the parse error RP2 would.
    pub fn from_input(input: &TsInput) -> EngineResult<Timestamp> {
        if let Some(message) = &input.error {
            let class = input.error_class.unwrap_or(ErrorClass::ValueError);
            return Err(EngineError::new(class, message.clone()));
        }
        let missing =
            |field: &str| EngineError::request(format!("timestamp field '{field}' is missing"));
        let us = input.us.ok_or_else(|| missing("us"))?;
        let offset_s = input.offset_s.ok_or_else(|| missing("offset_s"))?;
        let date_text = input.date.as_deref().ok_or_else(|| missing("date"))?;
        let year = input.year.ok_or_else(|| missing("year"))?;
        let display = input.display.clone().ok_or_else(|| missing("display"))?;
        let iso = input.iso.clone().ok_or_else(|| missing("iso"))?;
        let date = CivilDate::parse(date_text)
            .ok_or_else(|| EngineError::request(format!("invalid timestamp date {date_text:?}")))?;
        let vienna_date = match input.vienna_date.as_deref() {
            None => None,
            Some(text) => Some(CivilDate::parse(text).ok_or_else(|| {
                EngineError::request(format!("invalid timestamp vienna_date {text:?}"))
            })?),
        };
        let local_us = i128::from(us) + i128::from(offset_s) * i128::from(US_PER_SECOND);
        let local_days = local_us.div_euclid(i128::from(US_PER_DAY));
        let consistent = i64::try_from(local_days)
            .map(|days| CivilDate::from_days(days) == date)
            .unwrap_or(false);
        if !consistent || year != date.year {
            return Err(EngineError::request(format!(
                "timestamp {:?}: date {date_text} and year {year} do not match us {us} at offset {offset_s}",
                input.raw
            )));
        }
        Ok(Timestamp {
            raw: input.raw.clone(),
            us,
            offset_s,
            date,
            vienna_date,
            display,
            iso,
        })
    }

    /// `datetime.year` in the timestamp's own offset.
    pub fn year(&self) -> i32 {
        self.date.year
    }

    /// Whether RP2's set iteration yields the entry (own-offset date on or
    /// after 1970-01-01).
    pub fn is_visible(&self) -> bool {
        self.date >= MIN_VISIBLE_DATE
    }

    /// `datetime.timestamp()`: the microsecond count divided by 10^6,
    /// correctly rounded to a float as Python's integer division is.
    pub fn posix_seconds(&self) -> EngineResult<f64> {
        let magnitude = self.us.unsigned_abs();
        let sign = if self.us < 0 { "-" } else { "" };
        let text = format!(
            "{sign}{}.{:06}",
            magnitude / US_PER_SECOND as u64,
            magnitude % US_PER_SECOND as u64
        );
        text.parse::<f64>().map_err(|_| {
            EngineError::request(format!("timestamp {:?} has no float form", self.raw))
        })
    }
}

/// Whole elapsed days from `earlier` to `later`, floored like
/// `timedelta.days`.
pub fn elapsed_days(later: &Timestamp, earlier: &Timestamp) -> i128 {
    (i128::from(later.us) - i128::from(earlier.us)).div_euclid(i128::from(US_PER_DAY))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn civil_dates_round_trip() {
        for days in [-719_468_i64, -1, 0, 1, 59, 60, 365, 10_957, 2_932_896] {
            let date = CivilDate::from_days(days);
            assert_eq!(date.to_days(), days, "{date:?}");
        }
        assert_eq!(
            CivilDate::from_days(0),
            CivilDate {
                year: 1970,
                month: 1,
                day: 1
            }
        );
        assert_eq!(
            CivilDate::parse("2024-02-29"),
            Some(CivilDate {
                year: 2024,
                month: 2,
                day: 29
            })
        );
        assert_eq!(CivilDate::parse("2023-02-29"), None);
        assert_eq!(CivilDate::parse("2023-2-01"), None);
    }
}
