use super::*;

/// Construct one principal slice, retaining the raw unit rate when supplied.
pub fn slice_row(
    observation: &Value,
    quantity: &Value,
    rows: &Value,
    side: &str,
    fee: &BigInt,
) -> Result<Value, Error> {
    slice_row_with_rows(observation, quantity, rows, side, fee).map(row::FinalizedRow::into_json)
}

pub(super) fn slice_row_with_rows<'a>(
    observation: &Value,
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
    let original = integer_big(original)?;
    if original > BigInt::from(0) && money::empty(&base["fiat_rate"]) {
        let value = if money::empty(&base["fiat_value_exact"]) {
            &base["fiat_value"]
        } else {
            &base["fiat_value_exact"]
        };
        if !money::empty(value) {
            // Python catches arithmetic/parse failures here, but not base-row admission.
            if let Ok(rate) = money::Money::finite(value).and_then(|v| v.rate(&original)) {
                rate.require_finite()?;
                base["fiat_rate"] = rate.float()?;
                base["fiat_rate_exact"] = json!(rate.fixed()?);
            }
        }
    }
    let start = integer_big(&quantity["start_msat"])?;
    let end = integer_big(&quantity["end_msat"])?;
    base["id"] = json!(slice_row_id(
        &string(&quantity["observation_hash"]),
        &start,
        &end,
        side
    ));
    base["journal_transaction_id"] = observation["anchor_transaction_id"].clone();
    base["amount"] = number(&(&end - &start));
    base["fee"] = number(fee);
    base["fiat_value"] = Value::Null;
    base["fiat_value_exact"] = Value::Null;
    base["custody_finalized_tax_projection"] = json!(true);
    base["custody_quantity_hash"] = observation["quantity_hash"].clone();
    base["custody_slice_start_msat"] = number(&start);
    base["custody_slice_end_msat"] = number(&end);
    Ok(base)
}
