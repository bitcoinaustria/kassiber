//! `core/tax_estimate.py`: the indicative Austrian tax for one tax year of
//! one book. It owns no tax math beyond multiplying the reported base. The
//! Reports screen and the review's per-year tax figures both read it.
use super::*;
use kassiber_pyjson::BigInt;

const GAINS: [i64; 2] = [172, 174];
const LOSSES: [i64; 1] = [176];
const PROGRESSIVE: [i64; 1] = [801];
pub const RATE: &str = "0.275";
pub const OLD_RULES_BLOCKER: &str = "crypto_rows_before_2022_03";
const BASIS: &str = "E 1kv KZ 172 + KZ 174 - KZ 176, this book only";
const CAVEATS: [&str; 3] = [
    "Indicative only, not tax advice; a Steuerberater confirms the figure.",
    "Covers this book only. Capital income and losses outside it (securities, other books) net against the same 27.5% pool on the real return.",
    "Kassiber files every amount under the foreign-income Kennzahlen; tax an Austrian provider already withheld (KESt) is not deducted.",
];
const OLD_RULES_NOTE: &str = "Rows from before 1 March 2022 followed the old rules unless the taxpayer opted for the new ones; they have no field here, so the figure is withheld until a Steuerberater states them.";
const NOT_APPLICABLE_NOTE: &str = "The crypto Kennzahlen (KZ 172, 174, 176) and their 27.5% rate apply from tax year 2022 (realisations from 1 March 2022). Earlier years followed the old rules (Spekulationsgeschäfte on the E 1); Kassiber does not estimate them, so a Steuerberater states that year.";
const PROGRESSIVE_NOTE: &str = "KZ 801 (Altbestand Spekulationsgeschäfte) is taxed at the progressive income-tax rate, which depends on income outside this book; it is not estimated.";

/// `austrian_indicative_tax`: `kennzahl_rows` are `(kennzahl, amount_eur_cents)`
/// pairs from the Kennzahl summary, whose KZ 176 is already a positive loss.
/// Amounts are Python ints, so they stay unbounded here.
pub fn austrian_indicative_tax(
    tax_year: i64,
    kennzahl_rows: &[(i64, BigInt)],
    blocker_ids: &[String],
    rows_before_crypto_kennzahlen: i64,
) -> Value {
    let mut by_code = std::collections::BTreeMap::<i64, BigInt>::new();
    for (code, cents) in kennzahl_rows {
        *by_code.entry(*code).or_default() += cents;
    }
    let sum = |codes: &[i64]| -> BigInt {
        codes
            .iter()
            .map(|c| by_code.get(c).cloned().unwrap_or_default())
            .sum()
    };
    let gains = sum(&GAINS);
    let losses = sum(&LOSSES);
    let base = &gains - &losses;
    let applies = austrian::e1kv_applies(tax_year);
    // January-February 2022 rows have no field, so a base without them would
    // understate the year without saying so.
    let mut blockers: Vec<String> = blocker_ids
        .iter()
        .filter(|b| !b.is_empty())
        .cloned()
        .collect();
    if applies && rows_before_crypto_kennzahlen != 0 {
        blockers.push(OLD_RULES_BLOCKER.into());
    }
    blockers.sort();
    blockers.dedup();
    // 27.5 % of a non-negative base, half up to the cent.
    let estimate = (applies && blockers.is_empty())
        .then(|| (base.clone().max(BigInt::default()) * 275 + 500) / 1000);
    let big = |n: BigInt| Value::Int(n);
    let progressive: Vec<_> = PROGRESSIVE
        .iter()
        .filter_map(|code| {
            by_code
                .get(code)
                .filter(|cents| **cents != BigInt::default())
                .map(|cents| (code.to_string(), big(cents.clone())))
        })
        .collect();
    let mut notes: Vec<Value> = if applies {
        CAVEATS.iter().map(|note| s(*note)).collect()
    } else {
        vec![s(NOT_APPLICABLE_NOTE)]
    };
    if blockers.iter().any(|b| b == OLD_RULES_BLOCKER) {
        notes.push(s(OLD_RULES_NOTE));
    }
    if !progressive.is_empty() {
        notes.push(s(PROGRESSIVE_NOTE));
    }
    let when = |v: Value| if applies { v } else { Value::Null };
    dict([
        ("jurisdiction", s("AT")),
        ("tax_year", Value::Int(tax_year.into())),
        (
            "status",
            s(if !applies {
                "not_applicable"
            } else if blockers.is_empty() {
                "indicative"
            } else {
                "withheld"
            }),
        ),
        (
            "withheld_by",
            Value::List(blockers.into_iter().map(s).collect()),
        ),
        // Before the crypto Kennzahlen there is no rate or base to quote.
        ("rate", when(s(RATE))),
        ("basis", when(s(BASIS))),
        ("special_rate_gains_eur_cents", when(big(gains))),
        ("special_rate_losses_eur_cents", when(big(losses))),
        ("special_rate_base_eur_cents", when(big(base))),
        ("estimated_tax_eur_cents", estimate.map_or(Value::Null, big)),
        ("progressive_rate_amounts_eur_cents", dict(progressive)),
        ("notes", Value::List(notes)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(pairs: &[(i64, i64)]) -> Vec<(i64, BigInt)> {
        pairs
            .iter()
            .map(|(code, cents)| (*code, (*cents).into()))
            .collect()
    }
    fn cents(v: &Value, key: &str) -> Option<i64> {
        match value(v, key) {
            Value::Int(n) => Some(i64::try_from(n.clone()).unwrap()),
            Value::Null => None,
            other => panic!("{key}: {other:?}"),
        }
    }
    fn notes(v: &Value) -> String {
        let Value::List(notes) = value(v, "notes") else {
            unreachable!()
        };
        notes.iter().map(py_str).collect::<Vec<_>>().join(" ")
    }

    // tests/test_tax_estimate.py AustrianIndicativeTaxTest, ported.
    #[test]
    fn income_and_gains_net_against_losses_at_the_special_rate() {
        let r = austrian_indicative_tax(
            2025,
            &rows(&[(172, 10_000), (174, 150_000), (176, 60_000)]),
            &[],
            0,
        );
        assert_eq!(text(&r, "status"), "indicative");
        assert_eq!(text(&r, "rate"), "0.275");
        assert_eq!(cents(&r, "special_rate_base_eur_cents"), Some(100_000));
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), Some(27_500));
        assert_eq!(
            value(&r, "progressive_rate_amounts_eur_cents"),
            &dict::<&str>([])
        );
    }

    #[test]
    fn a_zero_kz_801_is_not_listed() {
        // The Kennzahl summary always carries KZ 801, usually at zero.
        let r = austrian_indicative_tax(2025, &rows(&[(174, 100), (801, 0)]), &[], 0);
        assert_eq!(
            value(&r, "progressive_rate_amounts_eur_cents"),
            &dict::<&str>([])
        );
        assert!(!notes(&r).contains("progressive"));
    }

    #[test]
    fn rounds_half_up_to_the_cent() {
        let r = austrian_indicative_tax(2025, &rows(&[(174, 2)]), &[], 0);
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), Some(1));
        // Exact halves round up (5.5 -> 6, 16.5 -> 17), not to even.
        for (base, tax) in [(1, 0), (7, 2), (9, 2), (10, 3), (20, 6), (60, 17)] {
            let r = austrian_indicative_tax(2025, &rows(&[(174, base)]), &[], 0);
            assert_eq!(cents(&r, "estimated_tax_eur_cents"), Some(tax), "{base}");
        }
    }

    #[test]
    fn a_net_loss_estimates_zero() {
        let r = austrian_indicative_tax(2025, &rows(&[(174, 1_000), (176, 5_000)]), &[], 0);
        assert_eq!(cents(&r, "special_rate_base_eur_cents"), Some(-4_000));
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), Some(0));
    }

    #[test]
    fn report_blockers_withhold_the_figure() {
        let blockers = ["quarantine", "journals_stale", "quarantine"].map(String::from);
        let r = austrian_indicative_tax(2025, &rows(&[(174, 150_000)]), &blockers, 0);
        assert_eq!(text(&r, "status"), "withheld");
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), None);
        assert_eq!(
            value(&r, "withheld_by"),
            &Value::List(vec![s("journals_stale"), s("quarantine")])
        );
        assert_eq!(cents(&r, "special_rate_base_eur_cents"), Some(150_000));
    }

    #[test]
    fn altbestand_speculation_is_reported_but_never_estimated() {
        let r = austrian_indicative_tax(2025, &rows(&[(174, 0), (801, 40_000)]), &[], 0);
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), Some(0));
        assert_eq!(
            value(&r, "progressive_rate_amounts_eur_cents"),
            &dict([("801", Value::Int(40_000.into()))])
        );
        assert!(notes(&r).contains("progressive"));
    }

    #[test]
    fn a_year_before_the_e1kv_gets_no_estimate() {
        let r = austrian_indicative_tax(2021, &rows(&[(174, 150_000), (801, 40_000)]), &[], 0);
        assert_eq!(text(&r, "status"), "not_applicable");
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), None);
        assert_eq!(value(&r, "rate"), &Value::Null);
        assert_eq!(value(&r, "basis"), &Value::Null);
        assert_eq!(cents(&r, "special_rate_base_eur_cents"), None);
        assert!(notes(&r).contains("from tax year 2022"));
        assert!(!notes(&r).contains("KESt"));
        assert_eq!(
            value(&r, "progressive_rate_amounts_eur_cents"),
            &dict([("801", Value::Int(40_000.into()))])
        );
        // Old-rule rows do not withhold a year that has no figure at all.
        let r = austrian_indicative_tax(2021, &rows(&[(174, 100)]), &[], 3);
        assert_eq!(value(&r, "withheld_by"), &Value::List(vec![]));
        let r = austrian_indicative_tax(2022, &rows(&[(174, 100)]), &[], 0);
        assert_eq!(text(&r, "status"), "indicative");
    }

    #[test]
    fn rows_before_march_2022_withhold_that_years_figure() {
        let r = austrian_indicative_tax(2022, &rows(&[(174, 100_000)]), &[], 2);
        assert_eq!(text(&r, "status"), "withheld");
        assert_eq!(
            value(&r, "withheld_by"),
            &Value::List(vec![s(OLD_RULES_BLOCKER)])
        );
        assert_eq!(cents(&r, "estimated_tax_eur_cents"), None);
        assert!(notes(&r).contains("1 March 2022"));
    }

    #[test]
    fn notes_say_what_the_figure_leaves_out() {
        let notes = notes(&austrian_indicative_tax(2025, &[], &[], 0));
        assert!(notes.contains("not tax advice"));
        assert!(notes.contains("this book only"));
        assert!(notes.contains("KESt"));
        assert!(!notes.contains("1 March 2022"));
    }
}
