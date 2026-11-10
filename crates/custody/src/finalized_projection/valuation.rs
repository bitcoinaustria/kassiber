//! Valuation holds and retained price causes (Python rounds 5 and 8).
//! A transaction whose stored price is unusable, or whose reviewed payout is
//! split, is quarantined and blocked; the rest of the rebuild completes. A
//! hold never prices a row, books a residual, or establishes basis.
use super::row::RowRef;
use super::*;
use crate::python_text::PythonText;
use crate::quantity::DecisionRef;
use kassiber_pydecimal::{Context, Decimal};
use money::{empty, Money};
use std::collections::BTreeMap;

/// `msat_to_btc(int(value or 0))` in the RP2 decimal context.
fn btc(value: &Value) -> Result<Decimal, Error> {
    let msat = integer_big(value)?;
    let msat = Decimal::from_str(&msat.to_string()).map_err(|_| Error::from("ValueError"))?;
    msat.div(&Decimal::from_str("100000000000").unwrap(), &Context::RP2)
        .map_err(|_| Error::from("InvalidOperation"))
}

fn finite(value: &Value, field: &str) -> Result<Decimal, Error> {
    match Money::finite_field(value, field)? {
        Money::Finite(number) => Ok(number),
        Money::Special(_) => unreachable!("finite_field refuses specials"),
    }
}

/// `custody_pricing.validate_transaction_price`: every stored field, then
/// both derived valuations, without persisting either inference.
fn validate(row: RowRef<'_>) -> Result<(), Error> {
    let id = string(&row["id"]);
    for field in [
        "fiat_value",
        "fiat_value_exact",
        "fiat_rate",
        "fiat_rate_exact",
    ] {
        if !empty(&row[field]) {
            finite(&row[field], &format!("transaction {id}: {field}"))?;
        }
    }
    // `exact or legacy`: a falsy exact field falls through to the legacy one.
    let either = |exact: &str, legacy: &str| {
        if codec::truth(&row[exact]) {
            row.at(exact)
        } else {
            row.at(legacy)
        }
    };
    let rate = either("fiat_rate_exact", "fiat_rate");
    let value = either("fiat_value_exact", "fiat_value");
    let amount = btc(&row["amount"])?.copy_abs();
    if !empty(rate) && !amount.is_zero() {
        let derived = finite(rate, "fiat_rate")?
            .mul(&amount, &Context::RP2)
            .map_err(|_| Error::from("InvalidOperation"))?;
        Money::finite_decimal(derived, &format!("transaction {id}: derived fiat_value"))?;
    }
    if !empty(value) && !amount.is_zero() {
        let derived = finite(value, "fiat_value")?
            .div(&amount, &Context::RP2)
            .map_err(|_| Error::from("InvalidOperation"))?;
        Money::finite_decimal(derived, &format!("transaction {id}: derived fiat_rate"))?;
    }
    Ok(())
}

/// `_quarantine`: the profile's mandatory fields and sorted-key detail text.
pub(super) fn record(profile: &Value, id: &str, reason: &str, detail: Value) -> Result<Py, String> {
    let workspace = profile.get("workspace_id").ok_or("KeyError")?;
    let profile_id = profile.get("id").ok_or("KeyError")?;
    let detail = kassiber_pyjson::dumps(
        &codec::from_json(&detail)?,
        &kassiber_pyjson::Options::new().sort_keys(true),
    )
    .map_err(|_| "TypeError")?;
    codec::from_json(&json!({"transaction_id":id,"workspace_id":workspace,
        "profile_id":profile_id,"reason":reason,"detail_json":detail}))
}

/// The valuation holds Python adds before the tax eligibility it recomputes:
/// invalid stored prices in row order, then split payouts in id order.
/// Rows are taken by id as `rows_by_id` keeps them (last row, first place).
pub(super) fn blockers<'a>(
    profile: &Value,
    rows: impl IntoIterator<Item = RowRef<'a>>,
    observations: &composition::Observations<'_>,
    decisions: &[DecisionRef<'_>],
    payouts: &Value,
) -> Result<Vec<Py>, Error> {
    let mut order = Vec::new();
    let mut by_id: BTreeMap<String, RowRef<'a>> = BTreeMap::new();
    for row in rows {
        let id = text(&row["id"]);
        if by_id.insert(id.clone(), row).is_none() {
            order.push(id);
        }
    }
    let mut result = Vec::new();
    for id in order {
        let row = by_id[&id];
        let cause = match validate(row) {
            Ok(()) if row["pricing_method"] == "unsupported_price_range" => {
                "cached rate produces a value outside the supported decimal range".to_owned()
            }
            Ok(()) => continue,
            Err(error) if error.code == "validation" => error.message,
            Err(error) => return Err(error),
        };
        result.push(record(
            profile,
            &id,
            "invalid_transaction_price",
            json!({ "cause": cause }),
        )?);
    }
    let mut split = BTreeSet::new();
    for decision in decisions {
        let source = observations
            .get(&*string_ref(&decision.at("source")["observation_hash"]))
            .ok_or_else(|| Error::from("KeyError"))?;
        let anchor = string(&source["anchor_transaction_id"]);
        let Some(payout) = payouts.get(&anchor) else {
            continue;
        };
        let reviewed = if codec::truth(&payout["out_amount"]) {
            &payout["out_amount"]
        } else {
            &source["amount_msat"]
        };
        let selected = integer_big(&decision.at("source")["end_msat"])?
            - integer_big(&decision.at("source")["start_msat"])?;
        if decision.at("reason") == "reviewed_direct_payout_source"
            && *decision.at("component_id") == payout["component_id"]
            && selected != integer_big(reviewed)?
        {
            split.insert(anchor);
        }
    }
    for id in split {
        result.push(record(
            profile,
            &id,
            "split_payout_requires_review",
            json!({"cause": "split payout allocation requires review"}),
        )?);
    }
    Ok(result)
}

/// `ledger_request._requires_coarse_review`.
fn requires_coarse_review(profile: &Value) -> bool {
    match &profile["require_coarse_review"] {
        Value::String(s) => matches!(
            s.py_strip().py_lower().as_str(),
            "1" | "true" | "yes" | "on"
        ),
        value => codec::truth(value),
    }
}

/// `decimal_from_exact(exact, legacy)` as a positive-price check.
fn price(row: RowRef<'_>, exact: &str, legacy: &str) -> Result<Option<Money>, Error> {
    let value = [exact, legacy]
        .into_iter()
        .map(|field| row.at(field))
        .find(|value| !empty(value));
    value
        .map(|value| {
            Money::parse(value).map_err(|_| Error {
                code: "app_error".into(),
                message: format!("Invalid decimal value: {}", string(value)),
            })
        })
        .transpose()
}

/// `tax_events.transaction_price_quarantine`: a standalone event's own price
/// cause, kept even while a prior basis barrier holds the row. A coarse price
/// the owner accepted, unchanged since, needs no review (main #708).
pub(super) fn price_cause(
    row: RowRef<'_>,
    profile: &Value,
    wallet_label: &str,
) -> Result<Option<(&'static str, Value)>, Error> {
    let direction = &row["direction"];
    let inbound = direction == "inbound";
    if !inbound && direction != "outbound" {
        return Ok(None);
    }
    let amount = btc(&row["amount"])?;
    let fee = btc(&row["fee"])?;
    let total = amount
        .add(&fee, &Context::RP2)
        .map_err(|_| Error::from("InvalidOperation"))?;
    if !inbound && !total.is_positive() {
        return Ok(None);
    }
    let asset = string(row.get("asset").ok_or_else(|| Error::from("KeyError"))?);
    let coarse = requires_coarse_review(profile)
        && crate::coarse_review::needs_review(|key| row.get(key)).map_err(|(code, message)| {
            Error {
                code: code.into(),
                message,
            }
        })?;
    if coarse {
        let field = |key: &str| row.get(key).cloned().unwrap_or(Value::Null);
        return Ok(Some((
            "pricing_review_required",
            json!({"wallet":wallet_label,"asset":asset,"direction":direction,
                "required_for":"pricing_review","pricing_quality":field("pricing_quality"),
                "pricing_source_kind":field("pricing_source_kind"),
                "pricing_provider":field("pricing_provider"),"pricing_pair":field("pricing_pair"),
                "pricing_timestamp":field("pricing_timestamp"),
                "pricing_granularity":field("pricing_granularity"),
                "pricing_method":field("pricing_method")}),
        )));
    }
    let quantity = if amount.is_positive() { amount } else { fee };
    let priced = match price(row, "fiat_rate_exact", "fiat_rate")? {
        Some(rate) if rate.positive()? => true,
        _ => match price(row, "fiat_value_exact", "fiat_value")? {
            Some(value) => quantity.is_positive() && value.positive()?,
            None => false,
        },
    };
    if priced {
        return Ok(None);
    }
    Ok(Some((
        "missing_spot_price",
        json!({"wallet":wallet_label,"asset":asset,"direction":direction,
            "required_for":if inbound {"acquisition"} else {"disposal"}}),
    )))
}
