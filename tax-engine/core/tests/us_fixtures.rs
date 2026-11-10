//! Checks the engine's United States country against RP2's US plugin.
//!
//! `data/us_fixtures.json` comes from `tax-engine/scripts/gen_us_fixtures.py`
//! (see its docstring). `universal` cases expect RP2's whole response under
//! the global pool scope, with each gain/loss's term by the IRS calendar
//! rule; a case where RP2's day count gives another term also drops the
//! yearly summary, which RP2 splits by its own term. `per_wallet` cases
//! expect the same lots and values per (event, original lot) under the
//! wallet scope, values equal at RP2's 13 places because RP2 rebases a
//! moved lot once more. Setting `KASSIBER_US_FIXTURES` to another generated
//! file checks that file instead.

use std::collections::BTreeMap;

use kassiber_tax_core::compute;
use kassiber_tax_core::decimal::{Context, Decimal};
use serde_json::Value;

const FIXTURES: &str = include_str!("data/us_fixtures.json");

/// Fewest cases of each kind, so a truncated file cannot pass.
const MIN_CASES: [(&str, usize); 6] = [
    ("universal:ok", 200),
    ("universal:error", 3),
    ("universal:term_differs", 10),
    ("per_wallet:ok", 200),
    ("per_wallet:term_differs", 25),
    ("per_wallet:scope_matters", 120),
];

fn diff(path: &str, expected: &Value, actual: &Value, out: &mut Vec<String>) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            for (key, value) in e {
                match a.get(key) {
                    Some(actual) => diff(&format!("{path}.{key}"), value, actual, out),
                    None => out.push(format!("{path}.{key}: missing")),
                }
            }
            for key in a.keys().filter(|key| !e.contains_key(*key)) {
                out.push(format!("{path}.{key}: unexpected field"));
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            if e.len() != a.len() {
                out.push(format!(
                    "{path}: expected {} items, got {}",
                    e.len(),
                    a.len()
                ));
            }
            for (index, (e, a)) in e.iter().zip(a).enumerate() {
                diff(&format!("{path}[{index}]"), e, a, out);
            }
        }
        _ if expected == actual => {}
        _ => out.push(format!("{path}: expected {expected}, got {actual}")),
    }
}

fn dec(value: &Value) -> Decimal {
    Decimal::from_canonical(value.as_str().expect("a canonical decimal")).unwrap()
}

/// Per (event kind, event row, lot): summed amount, basis, proceeds, gain,
/// and the terms seen.
type Grouped = BTreeMap<(String, i64, Option<i64>), ([Decimal; 4], Vec<bool>)>;

fn grouped(gain_losses: &Value) -> Grouped {
    let ctx = Context::RP2;
    let mut out = Grouped::new();
    for gl in gain_losses.as_array().unwrap() {
        let key = (
            gl["event"]["kind"].as_str().unwrap().to_owned(),
            gl["event"]["row"].as_i64().unwrap(),
            gl["lot"].as_i64(),
        );
        let values =
            ["crypto_amount", "fiat_cost_basis", "proceeds", "fiat_gain"].map(|k| dec(&gl[k]));
        let slot = out.entry(key).or_insert_with(|| {
            (
                [
                    Decimal::zero(),
                    Decimal::zero(),
                    Decimal::zero(),
                    Decimal::zero(),
                ],
                Vec::new(),
            )
        });
        for (sum, value) in slot.0.iter_mut().zip(values) {
            *sum = sum.add(&value, &ctx).unwrap();
        }
        slot.1.push(gl["long_term"].as_bool().unwrap());
    }
    out
}

fn check(case: &Value) -> Vec<String> {
    let name = case["name"].as_str().unwrap();
    let response: Value = serde_json::from_str(&compute(&case["request"].to_string())).unwrap();
    let expected = &case["expected"];
    let mut out = Vec::new();
    if expected["ok"] == false {
        diff(name, &expected["error"], &response["error"], &mut out);
        return out;
    }
    if response["ok"] != true {
        return vec![format!(
            "{name}: expected a result, got {}",
            response["error"]
        )];
    }
    let asset = &response["assets"][0];
    if case["application"] == "universal" {
        let mut want = expected["asset"].clone();
        for (gl, irs) in want["gain_losses"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .zip(expected["gain_losses"].as_array().unwrap())
        {
            gl["long_term"] = irs["long_term"].clone();
        }
        let mut got = asset.clone();
        if expected["yearly_differs"] == true {
            want.as_object_mut().unwrap().remove("yearly");
            got.as_object_mut().unwrap().remove("yearly");
        }
        // The engine's Austrian fields are absent from RP2's documents.
        for gl in got["gain_losses"].as_array_mut().unwrap() {
            let fields = gl.as_object_mut().unwrap();
            fields.retain(|key, value| !(key.starts_with("at_category") && value.is_null()));
        }
        diff(name, &want, &got, &mut out);
        return out;
    }
    let ctx = Context::RP2;
    let want = grouped(&expected["gain_losses"]);
    let got = grouped(&asset["gain_losses"]);
    if want.keys().collect::<Vec<_>>() != got.keys().collect::<Vec<_>>() {
        out.push(format!(
            "{name}: lots differ: expected {:?}, got {:?}",
            want.keys().collect::<Vec<_>>(),
            got.keys().collect::<Vec<_>>()
        ));
        return out;
    }
    for (key, (values, terms)) in &want {
        let (actual, actual_terms) = &got[key];
        for (field, (e, a)) in ["crypto_amount", "fiat_cost_basis", "proceeds", "fiat_gain"]
            .iter()
            .zip(values.iter().zip(actual))
        {
            if !e.rp2_eq(a, &ctx).unwrap() {
                out.push(format!("{name} {key:?}.{field}: expected {e}, got {a}"));
            }
        }
        let mut terms = terms.clone();
        let mut actual_terms = actual_terms.clone();
        terms.sort();
        actual_terms.sort();
        if terms != actual_terms {
            out.push(format!(
                "{name} {key:?}: terms {terms:?} != {actual_terms:?}"
            ));
        }
    }
    out
}

#[test]
fn engine_matches_rp2s_us_plugin() {
    let text = std::env::var("KASSIBER_US_FIXTURES")
        .map(|path| std::fs::read_to_string(path).unwrap())
        .unwrap_or_else(|_| FIXTURES.to_owned());
    let document: Value = serde_json::from_str(&text).unwrap();
    for (key, minimum) in MIN_CASES {
        let count = document["stats"][key].as_u64().unwrap_or(0) as usize;
        assert!(
            count >= minimum,
            "{key}: {count} cases, expected at least {minimum}"
        );
    }
    let mut failures = Vec::new();
    let cases = document["cases"].as_array().unwrap();
    for case in cases {
        failures.extend(check(case));
    }
    assert!(
        failures.is_empty(),
        "{} mismatches over {} cases:\n{}",
        failures.len(),
        cases.len(),
        failures
            .iter()
            .take(60)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
