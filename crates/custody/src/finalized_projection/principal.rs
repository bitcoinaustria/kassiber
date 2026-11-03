use super::*;
use crate::evidence::canonical::LegRef;

/// Construct one principal slice, retaining the raw unit rate when supplied.
pub fn slice_row(
    observation: &Value,
    quantity: &Value,
    rows: &Value,
    side: &str,
    fee: &BigInt,
) -> Result<Value, Error> {
    slice_row_with_rows(LegRef::Json(observation), quantity, rows, side, fee)
        .map(row::FinalizedRow::into_json)
}

pub(super) fn slice_row_with_rows<'a>(
    observation: LegRef<'a>,
    quantity: &Value,
    rows: &'a dyn Rows,
    side: &str,
    fee: &BigInt,
) -> Result<row::FinalizedRow<'a>, Error> {
    let mut base = base_row_with_rows(observation, rows)?;
    let original = if codec::truth(&base["amount"]) {
        &base["amount"]
    } else {
        &observation["amount_msat"]
    };
    // A machine integer decides the sign without a big integer.
    let positive = match original.as_i64() {
        Some(original) => original > 0,
        None => integer_big(original)? > BigInt::from(0),
    };
    if positive && money::empty(&base["fiat_rate"]) {
        let value = if money::empty(&base["fiat_value_exact"]) {
            &base["fiat_value"]
        } else {
            &base["fiat_value_exact"]
        };
        if !money::empty(value) {
            let original = integer_big(original)?;
            // Python catches arithmetic/parse failures here, but not base-row admission.
            if let Ok(rate) = money::Money::finite(value).and_then(|v| v.rate(&original)) {
                rate.require_finite()?;
                base.set("fiat_rate", rate.float()?);
                base.set("fiat_rate_exact", json!(rate.fixed()?));
            }
        }
    }
    let hash = || match &quantity["observation_hash"] {
        Value::String(hash) => std::borrow::Cow::Borrowed(hash.as_str()),
        hash => std::borrow::Cow::Owned(string(hash)),
    };
    let bounds = match (
        quantity["start_msat"].as_i64(),
        quantity["end_msat"].as_i64(),
    ) {
        (Some(start), Some(end)) => end.checked_sub(start).map(|amount| (start, end, amount)),
        _ => None,
    };
    if let Some((start, end, amount)) = bounds {
        base.set(
            "id",
            Value::String(format!("custody-tax:{}:{start}:{end}:{side}", hash())),
        );
        base.set_int("amount", amount);
        base.set_int("custody_slice_start_msat", start);
        base.set_int("custody_slice_end_msat", end);
    } else {
        let start = integer_big(&quantity["start_msat"])?;
        let end = integer_big(&quantity["end_msat"])?;
        base.set("id", json!(slice_row_id(&hash(), &start, &end, side)));
        base.set("amount", number(&(&end - &start)));
        base.set("custody_slice_start_msat", number(&start));
        base.set("custody_slice_end_msat", number(&end));
    }
    base.set_observed(
        "journal_transaction_id",
        observation,
        "anchor_transaction_id",
    );
    match i64::try_from(fee) {
        Ok(fee) => base.set_int("fee", fee),
        Err(_) => base.set("fee", number(fee)),
    }
    base.set_static("fiat_value", &NULL);
    base.set_static("fiat_value_exact", &NULL);
    base.set_static("custody_finalized_tax_projection", &TRUE);
    base.set_observed("custody_quantity_hash", observation, "quantity_hash");
    Ok(base)
}
