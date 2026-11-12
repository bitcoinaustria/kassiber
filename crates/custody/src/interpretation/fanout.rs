//! Recorded 1:N derivation. Raw rows establish candidates; the shared claim
//! compiler independently revalidates the entire group's native observations.
use super::{pairs, Context};
use crate::fields::{Fields, Row};
use crate::python_text::PythonText;
use crate::{codec, snapshot::Snapshot};
use kassiber_pydecimal::{Context as DecimalContext, Decimal, DecimalError};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::{collections::BTreeSet, str::FromStr};

pub(super) fn amount<R: Row + ?Sized>(row: &R, field: &str) -> Result<BigInt, String> {
    BigInt::from_str(&super::integer_or_zero(row.at(field))?.to_string()).map_err(|e| e.to_string())
}
pub(super) fn number(value: &BigInt) -> Value {
    serde_json::from_str(&value.to_string()).expect("converted integer")
}

fn decimal_error(error: DecimalError) -> String {
    match error {
        DecimalError::ConversionSyntax
        | DecimalError::NonFinite
        | DecimalError::DivisionUndefined => "InvalidOperation",
        _ => error.name(),
    }
    .to_owned()
}

fn unit_rate(value: &Value, full: &BigInt) -> Result<String, String> {
    let raw = codec::pystr(value);
    let raw = raw.py_strip();
    let decimal = |s: &str| Decimal::from_str(s).map_err(decimal_error);
    let btc = decimal(&full.to_string())?
        .div(&Decimal::from(100_000_000_000i64), &DecimalContext::RP2)
        .map_err(decimal_error)?;
    match Decimal::from_str(raw) {
        Ok(value) => value
            .div(&btc, &DecimalContext::RP2)
            .and_then(|v| v.to_fixed_string())
            .map_err(decimal_error),
        Err(kassiber_pydecimal::DecimalError::NonFinite) => {
            // Decimal's finite core intentionally excludes these. Python's
            // split-row pricing preserves quiet NaN payloads and infinities.
            let normalized: String = raw
                .py_lower()
                .chars()
                .filter(|c| *c != '_')
                .map(|c| {
                    crate::python_text::decimal_digit(c)
                        .map(|d| char::from(b'0' + d))
                        .unwrap_or(c)
                })
                .collect();
            let sign = if normalized.starts_with('-') { "-" } else { "" };
            let body = normalized.trim_start_matches(['+', '-']);
            if matches!(body, "inf" | "infinity") {
                return Ok(format!("{sign}Infinity"));
            }
            if let Some(payload) = body.strip_prefix("nan") {
                let start = payload
                    .len()
                    .saturating_sub(DecimalContext::RP2.prec as usize);
                let payload = payload[start..].trim_start_matches('0');
                return Ok(format!("{sign}NaN{payload}"));
            }
            Err("InvalidOperation".into())
        }
        Err(error) => Err(decimal_error(error)),
    }
}

/// Shared pricing and quantity portion of Python's `_clone_row`.
pub(super) fn clone_base<R: Row + ?Sized>(
    row: &R,
    amount_msat: &BigInt,
    fee: &BigInt,
) -> Result<Value, String> {
    let mut base = row.json().into_owned();
    let full = amount(row, "amount")?;
    let absent = |v: &Value| v.is_null() || v == "";
    if absent(row.at("fiat_rate_exact")) && absent(row.at("fiat_rate")) && full > BigInt::from(0) {
        let value = if codec::truth(row.at("fiat_value_exact")) {
            row.at("fiat_value_exact")
        } else {
            row.at("fiat_value")
        };
        if !absent(value) {
            let rate = unit_rate(value, &full)?;
            base["fiat_rate"] = json!(rate);
            base["fiat_rate_exact"] = base["fiat_rate"].clone();
        }
    }
    base["amount"] = number(amount_msat);
    base["fee"] = number(fee);
    base["fiat_value"] = Value::Null;
    base["fiat_value_exact"] = Value::Null;
    Ok(base)
}

fn clone_out<R: Row + ?Sized>(
    row: &R,
    amount_msat: &BigInt,
    fee: &BigInt,
    id: &str,
) -> Result<Value, String> {
    let mut base = clone_base(row, amount_msat, fee)?;
    base["id"] = json!(id);
    base["external_id"] = json!(id);
    base["kind"] = json!("self_transfer_out");
    base["journal_transaction_id"] = json!(codec::pystr(row.at("id")));
    Ok(base)
}

pub(super) fn derive<R: Row>(rows: &[R], handled: &BTreeSet<String>) -> Result<Value, String> {
    // A Vec retains Python's first-seen scope order, including interleaved groups.
    let mut groups: Vec<(_, Vec<&R>)> = Vec::new();
    let mut index = crate::fxhash::HashMap::default();
    for row in rows {
        if let Some(scope) = row.onchain_scope() {
            crate::codec::push_grouped(&mut groups, &mut index, scope, row);
        }
    }
    let mut derived_pairs = Vec::new();
    let mut synthetic_rows = Vec::new();
    let mut dropped = BTreeSet::new();
    for ((_, _, txid, _), group) in groups {
        // Count every positive source before applying handled reservations.
        let mut outs = Vec::new();
        for row in &group {
            if row.at("direction") == "outbound" && amount(row, "amount")? > BigInt::from(0) {
                outs.push(*row);
            }
        }
        if outs.len() != 1 {
            continue;
        }
        let out = outs[0];
        let source_id = codec::pystr(out.at("id"));
        if handled.contains(&source_id) {
            continue;
        }
        let source_wallet = codec::pystr(out.at("wallet_id"));
        let ins: Vec<_> = group
            .into_iter()
            .filter(|row| {
                row.at("direction") == "inbound"
                    && !handled.contains(&codec::pystr(row.at("id")))
                    && codec::pystr(row.at("wallet_id")) != source_wallet
            })
            .collect();
        if ins.len() < 2 {
            continue;
        }
        let wallets: BTreeSet<_> = ins
            .iter()
            .map(|row| codec::pystr(row.at("wallet_id")))
            .collect();
        if wallets.len() != ins.len() {
            continue;
        }
        let mut legs = ins
            .into_iter()
            .map(|row| Ok((amount(row, "amount")?, codec::pystr(row.at("id")), row)))
            .collect::<Result<Vec<_>, String>>()?;
        let total: BigInt = legs.iter().map(|(amount, _, _)| amount).sum();
        if total != amount(out, "amount")? {
            continue;
        }
        let fee = amount(out, "fee")?;
        legs.sort_by(|a, b| (&a.0, &a.1).cmp(&(&b.0, &b.1)));
        let mut leg_pairs = Vec::new();
        let mut leg_rows = Vec::new();
        for (position, (leg_amount, in_id, incoming)) in legs.into_iter().enumerate() {
            if leg_amount <= BigInt::from(0) {
                break;
            }
            let id = format!("recorded-fanout:{txid}:out:{in_id}");
            let leg = clone_out(
                out,
                &leg_amount,
                &if position == 0 {
                    fee.clone()
                } else {
                    BigInt::from(0)
                },
                &id,
            )?;
            leg_pairs.push(json!({"out":leg,"in":incoming.json(),"source":"recorded_fanout","group_id":format!("recorded-fanout:{txid}")}));
            leg_rows.push(leg);
        }
        // Sorted amounts put any nonpositive leg first, so no partial split leaks.
        if leg_pairs.is_empty() {
            continue;
        }
        derived_pairs.extend(leg_pairs);
        synthetic_rows.extend(leg_rows);
        dropped.insert(source_id);
    }
    Ok(
        json!({"derived_pairs":derived_pairs,"synthetic_rows":synthetic_rows,
        "out_row_overrides":{},"dropped_out_ids":dropped,"dropped_in_ids":[],"blocked_sources":[]}),
    )
}

pub(super) fn compile(snapshot: &Snapshot, context: &Context) -> Result<Value, String> {
    let rows: Vec<_> = snapshot.transactions.iter().map(|r| json!(r)).collect();
    // This isolated seam reserves complete recorded pairs; other interpreter
    // interactions remain deferred until full composition is ported.
    let (recorded, _) = super::recorded::detect(&rows)?;
    let handled = super::recorded::resolved_ids(&recorded, context, true);
    let derivation = derive(&rows, &handled)?;
    let candidates = super::array(&derivation["derived_pairs"]);
    let compiled = pairs::compile(candidates, context, snapshot)?;
    Ok(
        json!({"derivation":derivation,"handled_ids":handled,"claims":compiled["claims"],
        "blocked_transaction_ids":compiled["blocked_transaction_ids"],"quarantines":compiled["quarantines"],
        "native_audits":pairs::native_audits(candidates, context)?}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn check(case: &str) {
        let vectors: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_fanout_cases.json"
        ))
        .unwrap();
        let vector = vectors.iter().find(|v| v["case"] == case).unwrap();
        let handled = super::super::array(&vector["handled_ids"])
            .iter()
            .map(codec::text)
            .collect();
        let result = derive(super::super::array(&vector["rows"]), &handled);
        if vector.get("error").is_some() {
            assert_eq!(
                result.unwrap_err(),
                vector["error"],
                "{case}: exception family"
            );
        } else {
            assert_eq!(result.unwrap(), vector["expected"], "{case}");
        }
    }
    macro_rules! cases {
        ($($name:ident => $case:literal),* $(,)?) => { $(#[test] fn $name() { check($case); })* };
    }
    cases! {
        fanout_boundary_handled_source => "handled_source",
        fanout_boundary_single_destination => "single_destination",
        fanout_boundary_zero_target => "zero_target",
        fanout_boundary_negative_target => "negative_target",
        fanout_boundary_handled_target => "handled_target",
        fanout_boundary_handled_other_source => "handled_other_source",
        fanout_boundary_zero_other_source => "zero_other_source",
        fanout_boundary_negative_other_source => "negative_other_source",
        fanout_boundary_source_string => "source_string",
        fanout_boundary_source_unicode => "source_unicode",
        fanout_boundary_target_string => "target_string",
        fanout_boundary_fee_string => "fee_string",
        fanout_boundary_huge_integer => "huge_integer",
        fanout_boundary_sum_above_i64 => "sum_above_i64",
        fanout_boundary_bad_source => "bad_source",
        fanout_boundary_bad_target => "bad_target",
        fanout_boundary_bad_fee => "bad_fee",
        fanout_boundary_bad_type => "bad_type",
        fanout_boundary_bad_decimal => "bad_decimal",
        fanout_boundary_overflow_value => "overflow_value",
        fanout_boundary_snan_value => "snan_value",
        fanout_boundary_nan_payload_overflow => "nan_payload_overflow",
        fanout_boundary_false_value => "false_value",
        fanout_boundary_empty_value => "empty_value",
        fanout_boundary_empty_rate => "empty_rate",
        fanout_boundary_empty_exact_rate => "empty_exact_rate",
        fanout_boundary_numeric_value => "numeric_value",
        fanout_boundary_float_value => "float_value",
        fanout_boundary_negative_zero_value => "negative_zero_value",
        fanout_boundary_nan_payload_limit => "nan_payload_limit",
        fanout_boundary_positive_nan_value => "positive_nan_value",
        fanout_boundary_long_nan_zero_prefix => "long_nan_zero_prefix",
        fanout_boundary_unicode_wallet_ids => "unicode_wallet_ids",
        fanout_boundary_nonstring_ids => "nonstring_ids",
    }
}
