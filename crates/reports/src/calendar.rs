//! Python's ISO parser plus the pinned Europe/Vienna timezone transitions.
use super::*;
use kassiber_rates::time;

fn instant(raw: &str) -> Result<i64> {
    time::parse(raw)
        .filter(|_| raw == kassiber_pytext::strip(raw))
        .ok_or_else(|| {
            AppError::new(
                "internal_error",
                format!(
                    "Invalid isoformat string: {}",
                    kassiber_pytext::repr_str(raw)
                ),
            )
        })
}

/// Whether `raw` precedes 1 March 2022 in Europe/Vienna (CET, so
/// 2022-02-28T23:00:00Z), when the E 1kv's crypto Kennzahlen begin.
pub fn before_crypto_kennzahlen(raw: &str) -> Result<bool> {
    Ok(instant(raw)? < time::parse("2022-02-28T23:00:00Z").unwrap())
}

pub fn vienna(raw: &str) -> Result<(i64, String)> {
    let us = instant(raw)?;
    let table = value(super::austrian::metadata(), "vienna_transitions");
    let Value::List(transitions) = table else {
        unreachable!()
    };
    let mut offset = 3921;
    for transition in transitions {
        let Value::List(pair) = transition else {
            unreachable!()
        };
        let at = integer(&pair[0]);
        if at > us {
            break;
        }
        offset = integer(&pair[1]);
    }
    let utc = time::format(us)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
    let year: i64 = utc[..4].parse().unwrap();
    // TZif's POSIX tail: CET-1CEST,M3.5.0,M10.5.0. EU transitions
    // occur on the last Sunday of March/October at 01:00 UTC.
    if year >= 2038 {
        let transition = |month: &str| {
            let end = time::parse(&format!("{year:04}-{month}-31T01:00:00Z")).unwrap();
            let weekday = (end / 86_400_000_000) % 7;
            end - ((weekday + 1) % 7) * 86_400_000_000
        };
        offset = if us >= transition("03") && us < transition("10") {
            7200
        } else {
            3600
        };
    }
    let local = time::format(us + offset * 1_000_000)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
    Ok((local[..4].parse().unwrap(), local[..10].into()))
}
/// The local year and date of `raw` in a book's `zone`: Vienna reads its
/// pinned table (`vienna`), every other registry zone its calendar from
/// `kassiber_tax_countries::calendar`.
pub fn local(raw: &str, zone: &str) -> Result<(i64, String)> {
    if zone == kassiber_tax_countries::VIENNA {
        return vienna(raw);
    }
    let us = instant(raw)?;
    let calendar = kassiber_tax_countries::calendar::zone(zone)
        .ok_or_else(|| AppError::new("internal_error", format!("No calendar for {zone}")))?;
    let epoch = kassiber_rates::time::UNIX;
    let date = calendar
        .local_date(us - epoch)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    Ok((i64::from(date.year), date.iso()))
}

/// The UTC instant (`YYYY-MM-DDTHH:MM:SSZ`) at which `year` begins in
/// `zone`: local midnight of 1 January.
pub fn year_start(year: i64, zone: &str) -> Result<String> {
    let calendar = kassiber_tax_countries::calendar::zone(zone)
        .ok_or_else(|| AppError::new("internal_error", format!("No calendar for {zone}")))?;
    let day = kassiber_tax_countries::calendar::Date::new(year as i32, 1, 1)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))?;
    let midnight_us = day.to_days() * 86_400_000_000;
    // No zone here changes its clocks around New Year.
    let offset = calendar
        .offset_s(midnight_us)
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let start = midnight_us - i64::from(offset) * 1_000_000;
    time::format(start + kassiber_rates::time::UNIX)
        .ok_or_else(|| AppError::new("internal_error", "date value out of range"))
}
fn integer(value: &Value) -> i64 {
    let Value::Int(n) = value else { unreachable!() };
    i64::try_from(n.clone()).unwrap()
}

pub fn window(year: i64) -> (String, String) {
    (
        format!("{}-12-31T00:00:00Z", year - 1),
        format!("{}-01-02T00:00:00Z", year + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn vienna_calendar_vectors() {
        let Value::List(cases) = value(austrian::metadata(), "calendar_vectors") else {
            unreachable!()
        };
        for case in cases {
            let Value::List(case) = case else {
                unreachable!()
            };
            assert_eq!(
                vienna(&py_str(&case[0])).unwrap(),
                (integer(&case[1]), py_str(&case[2])),
                "{:?}",
                case[0]
            );
        }
        assert!(vienna(" 2024-01-01 ").is_err());
        assert!(vienna("bad").is_err());
        // 23:59:59 on 28 February in Vienna is still the old rules.
        assert!(before_crypto_kennzahlen("2022-02-28T22:59:59Z").unwrap());
        assert!(!before_crypto_kennzahlen("2022-02-28T23:00:00Z").unwrap());
        assert!(before_crypto_kennzahlen("2022-03-01T00:59:59+02:00").unwrap());
        assert!(!before_crypto_kennzahlen("2022-03-01T00:00:00+01:00").unwrap());
        // A naive timestamp is UTC.
        assert!(before_crypto_kennzahlen("2022-02-28T22:59:59").unwrap());
        assert!(!before_crypto_kennzahlen("2022-03-01").unwrap());
        assert!(before_crypto_kennzahlen("bad").is_err());
        assert_eq!(
            window(2025),
            ("2024-12-31T00:00:00Z".into(), "2026-01-02T00:00:00Z".into())
        );
    }

    #[test]
    fn every_supported_zone_has_local_dates_and_new_years() {
        // ch-09: 23:30 UTC on 31 December is 1 January in Zurich and Berlin.
        for zone in ["Europe/Berlin", "Europe/Zurich", "Europe/Vienna"] {
            assert_eq!(
                local("2024-12-31T23:30:00Z", zone).unwrap(),
                (2025, "2025-01-01".into()),
                "{zone}"
            );
            assert_eq!(
                local("2024-12-31T22:59:59Z", zone).unwrap(),
                (2024, "2024-12-31".into())
            );
            assert_eq!(
                year_start(2025, zone).unwrap(),
                "2024-12-31T23:00:00Z",
                "{zone}"
            );
        }
        // Summer time: 22:30 UTC on 10 June is 11 June in Berlin (de-06).
        assert_eq!(
            local("2023-06-10T22:30:00Z", "Europe/Berlin").unwrap().1,
            "2023-06-11"
        );
        // gb-09: 23:30 UTC on 5 April 2025 is 6 April in London; ie-11:
        // 23:30 UTC on 6 October 2026 is 7 October in Dublin. Both start
        // their year at UTC midnight.
        assert_eq!(
            local("2025-04-05T23:30:00Z", "Europe/London").unwrap(),
            (2025, "2025-04-06".into())
        );
        assert_eq!(
            local("2026-10-06T23:30:00Z", "Europe/Dublin").unwrap(),
            (2026, "2026-10-07".into())
        );
        for zone in ["Europe/London", "Europe/Dublin"] {
            assert_eq!(year_start(2025, zone).unwrap(), "2025-01-01T00:00:00Z");
        }
        // Before the registry's calendars start, and outside them.
        assert!(local("1995-06-01T00:00:00Z", "Europe/Berlin").is_err());
        assert!(local("2024-01-01T00:00:00Z", "Mars/Olympus").is_err());
        assert!(local("bad", "Europe/Zurich").is_err());
        for zone in kassiber_tax_countries::SUPPORTED_ZONES {
            if *zone != kassiber_tax_countries::UTC {
                assert!(local("2024-01-01T00:00:00Z", zone).is_ok(), "{zone}");
            }
        }
    }
}
