//! Tax years and the per-year tables a country's rules read: which tax
//! year a local date belongs to, and what a rate, threshold or allowance
//! is on that date. A date a table does not cover is `Unknown`, and a date
//! from which a country's regime is not settled is `Refused` with the
//! country's code: neither is ever filled in with a neighbouring year.

use crate::calendar::Date;

/// The tax year a local date belongs to, named by the calendar year of its
/// first day: with a 6 April start, 5 April 2025 is in 2024 and 6 April
/// 2025 in 2025.
pub fn tax_year(local: Date, start: (u8, u8)) -> i32 {
    if (local.month, local.day) >= (u32::from(start.0), u32::from(start.1)) {
        local.year
    } else {
        local.year - 1
    }
}

/// A tax year's label: `2024` for a calendar year, `2024-25` for one that
/// starts later in the year.
pub fn tax_year_label(year: i32, start: (u8, u8)) -> String {
    if start == (1, 1) {
        year.to_string()
    } else {
        format!("{year}-{:02}", (year + 1).rem_euclid(100))
    }
}

/// The first and last day of tax year `year`.
pub fn tax_year_bounds(year: i32, start: (u8, u8)) -> (Date, Date) {
    let first = Date::new(year, u32::from(start.0), u32::from(start.1))
        .expect("a tax year starts on a real date");
    let next = Date::new(year + 1, u32::from(start.0), u32::from(start.1))
        .expect("a tax year starts on a real date");
    (first, Date::from_days(next.to_days() - 1))
}

/// What a table holds from a date on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// A value, as decimal text.
    Value(&'static str),
    /// The country refuses dates from here on, with this code: the law is
    /// not modelled or not settled.
    Refused(&'static str),
}

/// A value that changes on local dates: a rate, a threshold, an
/// allowance, or a regime.
#[derive(Debug, PartialEq, Eq)]
pub struct Schedule {
    /// `<country>.<name>`.
    pub id: &'static str,
    /// From each local date (`YYYY-MM-DD`, ascending), what holds until the
    /// next. A date before the first is unknown.
    pub steps: &'static [(&'static str, Step)],
    /// The last local date the table is known for; later dates are unknown.
    /// `None` when the last step holds for good (a refusal).
    pub known_until: Option<&'static str>,
    /// Where the values come from.
    pub source: &'static str,
}

/// A table's answer for one date.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lookup {
    Value(&'static str),
    Refused(&'static str),
    /// Kassiber has no value for the date.
    Unknown,
}

fn date(text: &str) -> Date {
    Date::parse(text).expect("a table date")
}

impl Schedule {
    /// What holds on `local`.
    pub fn at(&self, local: Date) -> Lookup {
        let Some((_, step)) = self
            .steps
            .iter()
            .rev()
            .find(|(from, _)| date(from) <= local)
        else {
            return Lookup::Unknown;
        };
        match step {
            Step::Refused(code) => Lookup::Refused(code),
            Step::Value(_) if self.known_until.is_some_and(|until| local > date(until)) => {
                Lookup::Unknown
            }
            Step::Value(value) => Lookup::Value(value),
        }
    }

    /// The dates in `first..=last` on which the value changes: where a tax
    /// year splits (the United Kingdom's 30 October 2024, Ireland's 7
    /// October 2026).
    pub fn changes_within(&self, first: Date, last: Date) -> Vec<Date> {
        self.steps
            .iter()
            .map(|(from, _)| date(from))
            .filter(|from| *from > first && *from <= last)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tables;

    fn d(text: &str) -> Date {
        Date::parse(text).unwrap()
    }

    #[test]
    fn tax_years_follow_their_start_day() {
        // gb-09: 6 April 00:30 in London is 2025-26; 5 April is 2024-25.
        assert_eq!(tax_year(d("2025-04-06"), (4, 6)), 2025);
        assert_eq!(tax_year(d("2025-04-05"), (4, 6)), 2024);
        assert_eq!(tax_year(d("2025-01-01"), (1, 1)), 2025);
        assert_eq!(tax_year(d("2024-12-31"), (1, 1)), 2024);
        assert_eq!(tax_year_label(2024, (4, 6)), "2024-25");
        assert_eq!(tax_year_label(2099, (4, 6)), "2099-00");
        assert_eq!(tax_year_label(2024, (1, 1)), "2024");
        assert_eq!(
            tax_year_bounds(2024, (4, 6)),
            (d("2024-04-06"), d("2025-04-05"))
        );
        assert_eq!(
            tax_year_bounds(2024, (1, 1)),
            (d("2024-01-01"), d("2024-12-31"))
        );
    }

    #[test]
    fn schedules_answer_by_local_date_and_fail_closed() {
        let freigrenze = &tables::DE_P23_FREIGRENZE;
        assert_eq!(freigrenze.at(d("2023-12-31")), Lookup::Value("600.00"));
        assert_eq!(freigrenze.at(d("2024-01-01")), Lookup::Value("1000.00"));
        assert_eq!(freigrenze.at(d("2026-12-31")), Lookup::Value("1000.00"));
        // A year Kassiber has not checked is unknown, not last year's.
        assert_eq!(freigrenze.at(d("2027-01-01")), Lookup::Unknown);
        assert_eq!(freigrenze.at(d("2008-12-31")), Lookup::Unknown);
        // ie-11: the rate follows the Dublin date; before 6 December 2012 it
        // is not modelled.
        let rate = &tables::IE_CGT_RATE;
        assert_eq!(rate.at(d("2026-10-06")), Lookup::Value("0.33"));
        assert_eq!(rate.at(d("2026-10-07")), Lookup::Value("0.31"));
        assert_eq!(
            rate.at(d("2012-12-05")),
            Lookup::Refused("ie_rate_before_2012_12_06_not_modelled")
        );
        assert_eq!(
            rate.changes_within(d("2026-01-01"), d("2026-12-31")),
            [d("2026-10-07")]
        );
        assert!(rate
            .changes_within(d("2025-01-01"), d("2025-12-31"))
            .is_empty());
        // jp-17: from 2027 the regime is refused for good.
        let regime = &tables::JP_TAX_REGIME;
        assert_eq!(regime.at(d("2026-12-31")), Lookup::Value("zatsu_shotoku"));
        assert_eq!(
            regime.at(d("2099-01-01")),
            Lookup::Refused("jp_separate_taxation_regime_unconfirmed")
        );
        // gb-19: 2024-25 splits on 30 October.
        let (first, last) = tax_year_bounds(2024, (4, 6));
        assert_eq!(
            tables::GB_CGT_MAIN_RATES.changes_within(first, last),
            [d("2024-10-30")]
        );
    }

    #[test]
    fn every_table_is_ordered_and_dated() {
        for table in tables::TABLES {
            assert!(!table.steps.is_empty(), "{}", table.id);
            let dates: Vec<Date> = table.steps.iter().map(|(from, _)| d(from)).collect();
            assert!(dates.windows(2).all(|w| w[0] < w[1]), "{}", table.id);
            match (table.known_until, table.steps.last()) {
                (None, Some((_, Step::Refused(_)))) => {}
                (Some(until), _) => assert!(d(until) >= *dates.last().unwrap(), "{}", table.id),
                _ => panic!("{} has no end", table.id),
            }
            assert!(
                table.id.contains('.') && !table.source.is_empty(),
                "{}",
                table.id
            );
        }
    }
}
