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
}
