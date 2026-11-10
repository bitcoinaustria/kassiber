//! The Bark plugin's wallet pages (`kassiber/btcpay/bark.py` `row_id`,
//! `sync_btcpay._bark_page_stable_ids` / `_bark_page_fingerprint`). Bark rows
//! can share a transaction hash, so the row id is their identity, and kind,
//! fee and the payment keys decide how a row books.
use super::normalize::{field, sha256_hex, sorted_dumps, stripped};
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::{PyStr, Value};

type Result<T> = std::result::Result<T, AppError>;

fn malformed(message: &str) -> AppError {
    AppError::new("protocol_error", message)
}

/// `bark.row_id`: the row's id; it becomes a stored identity, so lone
/// surrogates refuse.
pub fn row_id(row: &Value) -> Result<PyStr> {
    if !matches!(row, Value::Dict(_)) {
        return Err(malformed(
            "Bark wallet transaction record was not a JSON object",
        ));
    }
    let id = stripped(field(row, "id"))
        .ok_or_else(|| malformed("Bark wallet transaction record is missing 'id'"))?;
    if id.has_surrogates() {
        return Err(malformed(
            "Bark wallet transaction record has an invalid 'id'",
        ));
    }
    Ok(id)
}

/// `_bark_page_stable_ids`.
pub fn page_ids(page: &[Value]) -> Result<Value> {
    let mut ids = page.iter().map(row_id).collect::<Result<Vec<_>>>()?;
    ids.sort();
    Ok(Value::List(ids.into_iter().map(Value::Str).collect()))
}

/// `_bark_page_fingerprint`.
pub fn page_fingerprint(page: &[Value]) -> Result<String> {
    let mut rows = Vec::new();
    for tx in page {
        let id = row_id(tx)?;
        let mut row = vec![("id", Value::Str(id.clone()))];
        for key in [
            "kind",
            "timestamp",
            "amount",
            "fee",
            "confirmations",
            "status",
            "comment",
            "outpoint",
            "paymentHash",
            "transactionHash",
            "labels",
        ] {
            row.push((key, field(tx, key).clone()));
        }
        rows.push((id, dict(row)));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let rows = Value::List(rows.into_iter().map(|(_, row)| row).collect());
    Ok(sha256_hex(&sorted_dumps(&rows)?))
}
