//! What country layers share about a tax year: which year an entry's
//! instant belongs to in the country's calendar, the tables that apply on
//! a date, and the two ways a yearly limit works (`docs/reference/tax-countries.md`).
//! Each layer decides what its forms do with them.

use super::*;
use kassiber_tax_countries::calendar::{Date, OutOfRange};
use kassiber_tax_countries::years::{self, Lookup, Schedule};
use kassiber_tax_countries::Country;

/// The tax year of an instant (µs since the epoch) in the country's zone
/// and tax-year start, with its label (`2024`, `2024-25`).
pub fn tax_year(country: &Country, us: i64) -> std::result::Result<(i32, String), OutOfRange> {
    let zone = kassiber_tax_countries::calendar::zone(country.timezone)
        .expect("every registry zone has a calendar");
    let year = years::tax_year(zone.local_date(us)?, country.tax_year_start);
    Ok((year, years::tax_year_label(year, country.tax_year_start)))
}

/// A table's value on `local` as a decimal; a refused or unknown date is
/// returned as is, so the layer reports it instead of a number.
pub fn table_value(table: &Schedule, local: Date) -> Result<std::result::Result<Decimal, Lookup>> {
    match table.at(local) {
        Lookup::Value(text) => text
            .parse()
            .map(Ok)
            .map_err(|_| AppError::new("internal_error", format!("{} is not a decimal", table.id))),
        other => Ok(Err(other)),
    }
}

/// A year's figure measured against an exemption limit (Freigrenze): the
/// whole figure is taxable once it reaches the limit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Limit {
    /// Nothing to report.
    None,
    /// A net loss.
    Loss,
    /// Below the limit: exempt.
    Exempt,
    /// At or above the limit: all of it is taxable.
    Taxable,
}

/// `total` against the exemption limit `limit`, and the figure that
/// counts: all of it once taxable, the loss itself (a loss only offsets
/// the same kind of gains), nothing below the limit.
pub fn exemption_limit(total: &Decimal, limit: &Decimal) -> (Limit, Decimal) {
    if total.is_zero() {
        (Limit::None, Decimal::zero())
    } else if total.is_negative() {
        (Limit::Loss, total.clone())
    } else if total < limit {
        (Limit::Exempt, Decimal::zero())
    } else {
        (Limit::Taxable, total.clone())
    }
}

/// An allowance deducted from gains taxed at several rates, the highest
/// rate's first (Ireland's s.601(3)): what remains taxable per rate, in the
/// order given, and the allowance used.
pub fn allowance_highest_rate_first(
    gains: &[(Decimal, Decimal)],
    allowance: &Decimal,
) -> Result<(Vec<(Decimal, Decimal)>, Decimal)> {
    let mut order: Vec<usize> = (0..gains.len()).collect();
    order.sort_by(|&a, &b| gains[b].0.partial_cmp(&gains[a].0).expect("finite rates"));
    let mut left = allowance.clone();
    let mut taxable: Vec<(Decimal, Decimal)> = gains.to_vec();
    for index in order {
        let gain = &gains[index].1;
        if !gain.is_positive() {
            continue;
        }
        let used = if &left < gain {
            left.clone()
        } else {
            gain.clone()
        };
        taxable[index].1 = gain.sub(&used, &DECIMAL).map_err(arithmetic)?;
        left = left.sub(&used, &DECIMAL).map_err(arithmetic)?;
    }
    let used = allowance.sub(&left, &DECIMAL).map_err(arithmetic)?;
    Ok((taxable, used))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dec(text: &str) -> Decimal {
        text.parse().unwrap()
    }

    #[test]
    fn instants_fall_in_the_countrys_tax_year() {
        let us = |days: i64, seconds: i64| (days * 86_400 + seconds) * 1_000_000;
        let gb = Country {
            timezone: "Europe/London",
            tax_year_start: (4, 6),
            ..*kassiber_tax_countries::lookup("us").unwrap()
        };
        // gb-09: 2025-04-05T23:30Z is 6 April 00:30 BST.
        let day = Date::parse("2025-04-05").unwrap().to_days();
        assert_eq!(
            tax_year(&gb, us(day, 23 * 3600 + 1800)),
            Ok((2025, "2025-26".into()))
        );
        assert_eq!(
            tax_year(&gb, us(day, 22 * 3600)),
            Ok((2024, "2024-25".into()))
        );
        let utc = kassiber_tax_countries::lookup("us").unwrap();
        assert_eq!(
            tax_year(utc, us(day, 23 * 3600 + 1800)),
            Ok((2025, "2025".into()))
        );
        assert!(tax_year(&gb, -1).is_err());
    }

    #[test]
    fn exemption_limits_tax_all_once_reached() {
        let limit = dec("1000.00");
        // de-08, de-09: 999.99 is exempt, 1,000.00 taxable in full.
        assert_eq!(
            exemption_limit(&dec("999.99"), &limit),
            (Limit::Exempt, Decimal::zero())
        );
        assert_eq!(
            exemption_limit(&dec("1000.00"), &limit),
            (Limit::Taxable, dec("1000.00"))
        );
        // de-12: a net loss is reported as it is.
        assert_eq!(
            exemption_limit(&dec("-300"), &limit),
            (Limit::Loss, dec("-300"))
        );
        assert_eq!(exemption_limit(&dec("0.00"), &limit).0, Limit::None);
    }

    #[test]
    fn an_allowance_goes_to_the_highest_rate_first() {
        // ie-11: 2,000 at 33 % and 3,000 at 31 %; the 1,270 goes to the 33 %
        // gains.
        let gains = [(dec("0.31"), dec("3000")), (dec("0.33"), dec("2000"))];
        let (taxable, used) = allowance_highest_rate_first(&gains, &dec("1270")).unwrap();
        assert_eq!(
            taxable,
            [(dec("0.31"), dec("3000")), (dec("0.33"), dec("730"))]
        );
        assert_eq!(used, dec("1270"));
        // ie-01: a 500 gain uses 500.
        let (taxable, used) =
            allowance_highest_rate_first(&[(dec("0.33"), dec("500"))], &dec("1270")).unwrap();
        assert_eq!((taxable[0].1.is_zero(), used), (true, dec("500")));
        // It spills to the next rate.
        let gains = [(dec("0.33"), dec("1000")), (dec("0.31"), dec("1000"))];
        let (taxable, _) = allowance_highest_rate_first(&gains, &dec("1270")).unwrap();
        assert_eq!(
            taxable,
            [(dec("0.33"), dec("0")), (dec("0.31"), dec("730"))]
        );
    }

    #[test]
    fn tables_read_as_decimals_or_say_why_not() {
        let date = |text| Date::parse(text).unwrap();
        let table = &kassiber_tax_countries::tables::IE_CGT_RATE;
        assert_eq!(
            table_value(table, date("2025-01-01")).unwrap(),
            Ok(dec("0.33"))
        );
        assert_eq!(
            table_value(table, date("2027-01-01")).unwrap(),
            Err(Lookup::Unknown)
        );
    }
}
