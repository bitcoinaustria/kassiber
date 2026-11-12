//! `core/btcpay_ledger.py`: payout fees inside BTCPay sends, and payment
//! ledgers booked from stored provenance for rails without wallet history.
use super::Result;
use crate::file_import::{self, csv::Row};
use crate::publication::row::{field, or_text, rows};
use kassiber_chain::btcpay::payment_methods as methods;
use kassiber_envelope::{dict, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{params_from_iter, types::Value as Sql, Connection};
use std::collections::BTreeMap;

pub const SOURCE_MODE: &str = "payments";
pub const RAW_SOURCE: &str = "btcpay_payment_ledger";
const MAX_DERIVED_FEE_MSAT: i64 = 200_000_000;
const SQL_CHUNK: usize = 400;

fn int(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}

fn database(error: kassiber_store::rusqlite::Error) -> AppError {
    crate::database_error(error)
}

/// `payout_totals_by_txid`: summed msat and payout count per proving txid.
pub fn payout_totals(
    conn: &Connection,
    profile_id: &str,
    asset: &str,
) -> Result<BTreeMap<String, (i64, i64)>> {
    let mut totals: BTreeMap<String, (i64, i64)> = BTreeMap::new();
    for row in rows(
        conn,
        "SELECT txid, amount FROM btcpay_provenance_records WHERE profile_id = ? AND record_type = 'payout' AND txid IS NOT NULL AND amount IS NOT NULL AND asset = ? AND LOWER(COALESCE(status, '')) IN ('inprogress', 'completed')",
        [profile_id, asset],
    )? {
        let entry = totals
            .entry(kassiber_pytext::lower(&py_str(field(&row, "txid"))))
            .or_default();
        entry.0 += int(field(&row, "amount")).abs();
        entry.1 += 1;
    }
    Ok(totals)
}

/// `derived_payout_fee_msat`.
pub fn derived_fee(net_msat: i64, payout: i64) -> Option<i64> {
    if payout <= 0 || net_msat <= payout {
        return None;
    }
    let fee = net_msat - payout;
    (fee <= MAX_DERIVED_FEE_MSAT && fee * 2 < net_msat).then_some(fee)
}

/// `outbound_fees_by_txid`.
pub fn outbound_fees(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
) -> Result<BTreeMap<String, i64>> {
    Ok(rows(
        conn,
        "SELECT external_id, fee FROM transactions WHERE profile_id = ? AND wallet_id = ? AND direction = 'outbound' AND external_id IS NOT NULL",
        [profile_id, wallet_id],
    )?
    .iter()
    .map(|row| {
        (
            kassiber_pytext::lower(&py_str(field(row, "external_id"))),
            int(field(row, "fee")),
        )
    })
    .collect())
}

/// `apply_payout_fees`: split the fee out of fee-inclusive sends whose
/// payouts are known; a send already in the book keeps its stored fee.
pub fn apply_payout_fees(
    records: &mut [Row],
    totals: &BTreeMap<String, (i64, i64)>,
    stored: &BTreeMap<String, i64>,
) -> Result<i64> {
    let mut applied = 0;
    for record in records {
        if *file_import::row_field(record, "direction") != s("outbound")
            || !truthy(file_import::row_field(record, "amount_includes_fee"))
        {
            continue;
        }
        let txid = file_import::row_field(record, "txid");
        let txid = if truthy(txid) {
            kassiber_pytext::lower(&py_str(txid))
        } else {
            String::new()
        };
        if let Some(fee) = stored.get(&txid) {
            if *fee != 0 {
                file_import::set_row_fee(record, &file_import::msat_to_btc(*fee));
            }
            continue;
        }
        let Some((total, _)) = totals.get(&txid) else {
            continue;
        };
        let amount = file_import::row_field(record, "amount");
        let net_msat = match amount {
            Value::Null => 0,
            amount => file_import::btc_to_msat(&py_str(amount))?,
        };
        let Some(fee) = derived_fee(net_msat, *total) else {
            continue;
        };
        file_import::set_row_fee(record, &file_import::msat_to_btc(fee));
        applied += 1;
    }
    Ok(applied)
}

/// `backfill_payout_fees`: re-fingerprint already-imported sends once their
/// payouts complete, unless the new fingerprint is taken.
pub fn backfill_payout_fees(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    totals: &BTreeMap<String, (i64, i64)>,
) -> Result<i64> {
    if totals.is_empty() {
        return Ok(0);
    }
    let mut updated = 0;
    for row in rows(
        conn,
        "SELECT id, external_id, occurred_at, direction, asset, amount FROM transactions WHERE profile_id = ? AND wallet_id = ? AND direction = 'outbound' AND amount_includes_fee = 1 AND COALESCE(fee, 0) = 0 AND external_id IS NOT NULL",
        [profile_id, wallet_id],
    )? {
        let external = py_str(field(&row, "external_id"));
        let Some((total, _)) = totals.get(&kassiber_pytext::lower(&external)) else {
            continue;
        };
        let amount = int(field(&row, "amount"));
        let Some(fee) = derived_fee(amount, *total) else {
            continue;
        };
        let fingerprint = file_import::fingerprint_parts(
            wallet_id,
            &external,
            &or_text(field(&row, "occurred_at")),
            &or_text(field(&row, "direction")),
            &or_text(field(&row, "asset")),
            &file_import::msat_to_btc(amount),
            &file_import::msat_to_btc(fee),
        );
        let id = or_text(field(&row, "id"));
        let taken = rows(
            conn,
            "SELECT 1 FROM transactions WHERE fingerprint = ? AND id != ?",
            [&fingerprint, &id],
        )?;
        if !taken.is_empty() {
            continue;
        }
        conn.execute(
            "UPDATE transactions SET fee = ?, fingerprint = ? WHERE id = ?",
            kassiber_store::rusqlite::params![fee, fingerprint, id],
        )
        .map_err(database)?;
        updated += 1;
    }
    Ok(updated)
}

/// `is_payment_ledger_config`.
pub fn is_ledger(config: &serde_json::Value) -> bool {
    let mode = match config.get("source_mode") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Null) | None => String::new(),
        Some(other) if crate::descriptors::truthy(other) => other.to_string(),
        Some(_) => String::new(),
    };
    kassiber_pytext::lower(kassiber_pytext::strip(&mode)) == SOURCE_MODE
}

/// `ledger_payment_method_ids`.
pub fn payment_method_ids(config: &serde_json::Value) -> Vec<String> {
    let values: Vec<serde_json::Value> = match config.get("payment_method_ids") {
        Some(serde_json::Value::Array(items)) => items.clone(),
        _ => vec![config
            .get("payment_method_id")
            .cloned()
            .unwrap_or(serde_json::Value::Null)],
    };
    let mut out: Vec<String> = Vec::new();
    for value in values {
        let text = match &value {
            serde_json::Value::String(text) => text.clone(),
            other if crate::descriptors::truthy(other) => other.to_string(),
            _ => String::new(),
        };
        let method = methods::canonical(&text);
        if !method.is_empty() && methods::ledger_group(&method).is_some() && !out.contains(&method)
        {
            out.push(method);
        }
    }
    out
}

fn description(row: &Value) -> String {
    let text = |key: &str| {
        let value = field(row, key);
        truthy(value).then(|| py_str(value))
    };
    if or_text(field(row, "record_type")) == "payout" {
        if or_text(field(row, "origin_kind")) == "refund" {
            return text("invoice_id").map_or_else(
                || "BTCPay refund".into(),
                |id| format!("BTCPay refund for invoice {id}"),
            );
        }
        return text("origin_label").map_or_else(
            || "BTCPay payout".into(),
            |label| kassiber_pytext::strip(&format!("BTCPay payout {label}")).to_owned(),
        );
    }
    let label = text("origin_label")
        .or_else(|| text("order_id"))
        .or_else(|| text("invoice_id"));
    label.map_or_else(
        || "BTCPay payment".into(),
        |label| format!("BTCPay payment · {label}"),
    )
}

/// `ledger_records`: settled payments and completed payouts of a store's
/// ledger rails, as import records keyed by provenance stable key.
pub fn records(
    conn: &Connection,
    profile_id: &str,
    store_id: &str,
    methods_ids: &[String],
) -> Result<(Vec<Row>, Value)> {
    let groups: Vec<String> = methods_ids
        .iter()
        .filter_map(|m| methods::ledger_group(m))
        .collect();
    let mut counts = [0i64; 3];
    if groups.is_empty() {
        return Ok((Vec::new(), counts_value(counts)));
    }
    let mut out = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for row in rows(
        conn,
        "SELECT * FROM btcpay_provenance_records WHERE profile_id = ? AND store_id = ? AND record_type IN ('payment', 'payout') AND amount IS NOT NULL AND amount > 0 AND occurred_at IS NOT NULL ORDER BY occurred_at ASC, stable_key ASC",
        [profile_id, store_id],
    )? {
        let method = py_str(field(&row, "payment_method_id"));
        let key = or_text(field(&row, "stable_key"));
        if !methods::ledger_group(&method).is_some_and(|g| groups.contains(&g)) || seen.contains(&key)
        {
            continue;
        }
        let status = kassiber_pytext::lower(kassiber_pytext::strip(&or_text(field(&row, "status"))));
        let payout = or_text(field(&row, "record_type")) == "payout";
        if status != if payout { "completed" } else { "settled" } {
            counts[2] += 1;
            continue;
        }
        seen.push(key.clone());
        counts[usize::from(payout)] += 1;
        let raw = dict([
            ("source", s(RAW_SOURCE)),
            ("stable_key", field(&row, "stable_key").clone()),
            ("store_id", field(&row, "store_id").clone()),
            ("payment_method_id", field(&row, "payment_method_id").clone()),
            ("record_type", field(&row, "record_type").clone()),
            ("invoice_id", field(&row, "invoice_id").clone()),
        ]);
        let raw = kassiber_pyjson::dumps(&raw, &kassiber_pyjson::Options::new().sort_keys(true))
            .map_err(|e| AppError::new("internal_error", e.to_string()))?;
        let hash = field(&row, "payment_hash").clone();
        let asset = field(&row, "asset");
        out.push(dict([
            ("id", field(&row, "stable_key").clone()),
            ("occurred_at", field(&row, "occurred_at").clone()),
            ("confirmed_at", field(&row, "occurred_at").clone()),
            ("direction", s(if payout { "outbound" } else { "inbound" })),
            (
                "asset",
                if truthy(asset) { asset.clone() } else { s("BTC") },
            ),
            (
                "amount",
                s(file_import::msat_to_btc(int(field(&row, "amount")).abs())),
            ),
            ("fee", s("0")),
            ("amount_includes_fee", Value::Bool(false)),
            ("kind", s(if payout { "withdrawal" } else { "deposit" })),
            ("description", s(description(&row))),
            ("counterparty", Value::Null),
            ("payment_hash", hash.clone()),
            (
                "payment_hash_source",
                if truthy(&hash) { s("btcpay") } else { Value::Null },
            ),
            ("raw_json", s(raw)),
        ]));
    }
    Ok((file_import::record_rows(out), counts_value(counts)))
}

fn counts_value(counts: [i64; 3]) -> Value {
    dict([
        ("payments", Value::Int(counts[0].into())),
        ("payouts", Value::Int(counts[1].into())),
        ("pending", Value::Int(counts[2].into())),
    ])
}

/// `_payment_hashes_elsewhere`: `(hash, direction)` another active wallet books.
fn hashes_elsewhere(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    keys: &[(String, String)],
) -> Result<Vec<(String, String)>> {
    let mut unique: Vec<(String, String)> = keys
        .iter()
        .filter(|(hash, _)| !hash.is_empty())
        .map(|(hash, direction)| (kassiber_pytext::lower(hash), direction.clone()))
        .collect();
    unique.sort();
    unique.dedup();
    let mut hashes: Vec<String> = unique.iter().map(|(hash, _)| hash.clone()).collect();
    hashes.dedup();
    let mut found = Vec::new();
    for chunk in hashes.chunks(SQL_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        let mut params = vec![Sql::Text(profile_id.into()), Sql::Text(wallet_id.into())];
        params.extend(chunk.iter().map(|hash| Sql::Text(hash.clone())));
        for row in rows(
            conn,
            &format!("SELECT DISTINCT LOWER(payment_hash) AS payment_hash, direction FROM transactions WHERE profile_id = ? AND wallet_id != ? AND excluded = 0 AND LOWER(payment_hash) IN ({marks})"),
            params_from_iter(params),
        )? {
            found.push((
                or_text(field(&row, "payment_hash")),
                or_text(field(&row, "direction")),
            ));
        }
    }
    Ok(unique
        .into_iter()
        .filter(|key| found.contains(key))
        .collect())
}

fn record_key(record: &Row) -> (String, String) {
    let hash = file_import::row_field(record, "payment_hash");
    let hash = if truthy(hash) {
        kassiber_pytext::lower(&py_str(hash))
    } else {
        String::new()
    };
    let direction = file_import::row_field(record, "direction");
    (
        hash,
        if truthy(direction) {
            py_str(direction)
        } else {
            String::new()
        },
    )
}

/// `split_tracked_elsewhere`: hold back rows another wallet already books.
pub fn split_tracked_elsewhere(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    records: Vec<Row>,
) -> Result<(Vec<Row>, Vec<Row>)> {
    let keys: Vec<_> = records.iter().map(record_key).collect();
    let elsewhere = hashes_elsewhere(conn, profile_id, wallet_id, &keys)?;
    let (mut keep, mut held) = (Vec::new(), Vec::new());
    for (record, key) in records.into_iter().zip(keys) {
        if elsewhere.contains(&key) {
            held.push(record);
        } else {
            keep.push(record);
        }
    }
    Ok((keep, held))
}

/// `existing_rows_tracked_elsewhere`: `(count, labels of the other wallets)`.
pub fn existing_tracked_elsewhere(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
) -> Result<(usize, Vec<String>)> {
    let keys: Vec<(String, String)> = rows(
        conn,
        "SELECT payment_hash, direction FROM transactions WHERE profile_id = ? AND wallet_id = ? AND payment_hash IS NOT NULL AND excluded = 0",
        [profile_id, wallet_id],
    )?
    .iter()
    .map(|row| (py_str(field(row, "payment_hash")), py_str(field(row, "direction"))))
    .collect();
    let duplicates = hashes_elsewhere(conn, profile_id, wallet_id, &keys)?;
    let mut hashes: Vec<String> = duplicates.iter().map(|(hash, _)| hash.clone()).collect();
    hashes.sort();
    hashes.dedup();
    let mut wallets: Vec<String> = Vec::new();
    for chunk in hashes.chunks(SQL_CHUNK) {
        let marks = vec!["?"; chunk.len()].join(",");
        let mut params = vec![Sql::Text(profile_id.into()), Sql::Text(wallet_id.into())];
        params.extend(chunk.iter().map(|hash| Sql::Text(hash.clone())));
        for row in rows(
            conn,
            &format!("SELECT DISTINCT w.label FROM transactions t JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND t.wallet_id != ? AND t.excluded = 0 AND LOWER(t.payment_hash) IN ({marks}) ORDER BY w.label"),
            params_from_iter(params),
        )? {
            let label = or_text(field(&row, "label"));
            if !wallets.contains(&label) {
                wallets.push(label);
            }
        }
    }
    Ok((duplicates.len(), wallets))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(csv: &[(&str, &str)]) -> Vec<Row> {
        file_import::btcpay_rows(
            csv.iter()
                .map(|(txid, amount)| {
                    dict([
                        ("TransactionId", s(*txid)),
                        ("Timestamp", s("2023-11-14T22:13:20Z")),
                        ("confirmed_at", s("2023-11-14T22:13:20Z")),
                        ("Currency", s("BTC")),
                        ("Amount", s(*amount)),
                        ("Comment", s("")),
                        ("Labels", Value::List(vec![])),
                    ])
                })
                .collect(),
        )
        .unwrap()
    }

    #[test]
    fn derived_fees_stay_inside_their_bounds() {
        assert_eq!(derived_fee(60, 50), Some(10));
        assert_eq!(derived_fee(60, 0), None);
        assert_eq!(derived_fee(50, 50), None);
        assert_eq!(derived_fee(100, 50), None, "half the send is no fee");
        assert_eq!(
            derived_fee(1_000_000_000, 799_999_999),
            None,
            "above the cap"
        );
        assert_eq!(derived_fee(1_000_000_000, 800_000_000), Some(200_000_000));
    }

    #[test]
    fn payout_fees_split_sends_and_keep_stored_fees() {
        let mut records = rows(&[
            ("AA", "-0.0006"),
            ("bb", "-0.0006"),
            ("cc", "0.0006"),
            ("dd", "-0.0006"),
        ]);
        let totals = BTreeMap::from([
            ("aa".to_owned(), (50_000_000, 1)),
            ("bb".to_owned(), (50_000_000, 1)),
            ("cc".to_owned(), (50_000_000, 1)),
        ]);
        let stored = BTreeMap::from([("bb".to_owned(), 7), ("dd".to_owned(), 0)]);
        assert_eq!(
            apply_payout_fees(&mut records, &totals, &stored).unwrap(),
            1
        );
        let fees: Vec<String> = records
            .iter()
            .map(|r| py_str(file_import::row_field(r, "fee")))
            .collect();
        assert_eq!(fees, ["0.0001", "7E-11", "0", "0"]);
    }

    #[test]
    fn ledger_config_mode_and_methods() {
        let config = |text: &str| serde_json::from_str::<serde_json::Value>(text).unwrap();
        assert!(is_ledger(&config(r#"{"source_mode": " Payments "}"#)));
        assert!(!is_ledger(&config(r#"{"source_mode": "history"}"#)));
        assert!(!is_ledger(&config("{}")));
        assert_eq!(
            payment_method_ids(&config(
                r#"{"payment_method_ids": ["btc-ln", "BTC-LightningNetwork", "BTC-CHAIN", "BTC-LNURL"]}"#
            )),
            ["BTC-LN", "BTC-LNURL"]
        );
        assert_eq!(
            payment_method_ids(&config(r#"{"payment_method_id": "BTC-LN"}"#)),
            ["BTC-LN"]
        );
        assert!(payment_method_ids(&config(r#"{"payment_method_ids": "BTC-LN"}"#)).is_empty());
    }

    #[test]
    fn ledger_descriptions_follow_the_record() {
        let row = |text: &str| kassiber_pytext::loads(text).unwrap();
        for (raw, expected) in [
            (
                r#"{"record_type": "payout", "origin_kind": "refund", "invoice_id": "i1"}"#,
                "BTCPay refund for invoice i1",
            ),
            (
                r#"{"record_type": "payout", "origin_kind": "refund"}"#,
                "BTCPay refund",
            ),
            (
                r#"{"record_type": "payout", "origin_label": " Batch "}"#,
                "BTCPay payout  Batch",
            ),
            (r#"{"record_type": "payout"}"#, "BTCPay payout"),
            (
                r#"{"record_type": "payment", "order_id": "o1", "invoice_id": "i1"}"#,
                "BTCPay payment · o1",
            ),
            (r#"{"record_type": "payment"}"#, "BTCPay payment"),
        ] {
            assert_eq!(description(&row(raw)), expected, "{raw}");
        }
    }
}
