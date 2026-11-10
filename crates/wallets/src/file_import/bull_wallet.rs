//! Bull Bitcoin's unified wallet-history export (`bullbitcoin_wallet_csv`):
//! Bitcoin, Liquid and Lightning rows in one file, imported per network
//! wallet. Preimages are hashed into the payment hash and redacted from the
//! stored row; failed, expired and refunded rows are skipped.
use super::exchange::Columns;
use super::*;

const REQUIRED: &[&str] = &[
    "date",
    "type",
    "direction",
    "amount_sats",
    "amount_btc",
    "fee_sats",
    "status",
    "txid",
    "network",
    "address",
    "swap_id",
    "preimage",
    "total_swap_fees_sats",
    "send_network",
    "receive_network",
    "send_txid",
    "receive_txid",
];

fn app(message: String) -> AppError {
    AppError::new("app_error", message)
}

/// `load_bullbitcoin_wallet_csv_records` over loaded CSV rows.
pub(super) fn parse(rows: Vec<csv::Row>) -> Result<Vec<csv::Row>, AppError> {
    exchange::headers(&rows, REQUIRED, "Bull Bitcoin wallet")?;
    let mut out = Vec::new();
    for (index, row) in rows.into_iter().enumerate() {
        if let Some(record) = record(row, index + 1)? {
            out.push(record);
        }
    }
    Ok(out)
}

/// `str(value or "").strip().casefold()`.
fn folded(value: Option<String>) -> String {
    kassiber_pytext::casefold(kassiber_pytext::strip(&value.unwrap_or_default()))
}

/// `abs(_bullbitcoin_wallet_sats(...))` as the digits of an exact integer.
fn sats(columns: &Columns, column: &str, default: bool) -> Result<String, AppError> {
    let value = match river::number(columns.get(&[column]))? {
        Some(PriceNumber::Finite(value)) => value,
        Some(PriceNumber::Special(text)) if text.contains("sNaN") => {
            return Err(decimal_error(DecimalError::InvalidOperation));
        }
        // `NaN != NaN`, while `int(Infinity)` overflows.
        Some(PriceNumber::Special(text)) if text.contains("NaN") => {
            return Err(app(format!(
                "Bull Bitcoin wallet CSV has non-integer {column}: {text}"
            )));
        }
        Some(PriceNumber::Special(_)) => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert Infinity to integer",
            ));
        }
        None if default => return Ok("0".into()),
        None => {
            return Err(app(format!(
                "Bull Bitcoin wallet CSV has a row with empty {column}"
            )))
        }
    };
    let fixed = value.to_fixed_string().map_err(decimal_error)?;
    let (integral, fraction) = fixed.split_once('.').unwrap_or((&fixed, ""));
    if fraction.bytes().any(|b| b != b'0') {
        return Err(app(format!(
            "Bull Bitcoin wallet CSV has non-integer {column}: {value}"
        )));
    }
    Ok(integral.trim_start_matches('-').to_owned())
}

fn btc(sats: &str) -> Result<String, AppError> {
    Ok(dec(sats)?
        .div(&dec("100000000")?, &CTX)
        .map_err(decimal_error)?
        .to_string())
}

fn direction(raw: Option<String>) -> Result<Option<&'static str>, AppError> {
    let value = folded(raw.clone());
    if ["incoming", "inbound", "received", "receive"].contains(&value.as_str()) {
        return Ok(Some("inbound"));
    }
    if ["outgoing", "outbound", "sent", "send"].contains(&value.as_str()) {
        return Ok(Some("outbound"));
    }
    if value == "self" {
        return Ok(None);
    }
    Err(app(format!(
        "Unsupported Bull Bitcoin wallet direction '{}'",
        raw.unwrap_or_else(|| "None".into())
    )))
}

fn network(columns: &Columns, kind: &str, direction: &str) -> Result<&'static str, AppError> {
    let mut found = columns.opt(&["network"]);
    if found.is_none() && kind == "chain_swap" {
        found = columns.opt(&[if direction == "inbound" {
            "receive_network"
        } else {
            "send_network"
        }]);
    }
    if found.is_none() {
        found = match kind {
            "liquid" => Some("liquid".into()),
            "onchain" | "payjoin_send" | "payjoin_receive" => Some("bitcoin".into()),
            "lightning_send" | "lightning_receive" => Some("lightning".into()),
            _ => None,
        };
    }
    match folded(found.clone()).as_str() {
        "bitcoin" | "btc" | "onchain" => Ok("bitcoin"),
        "liquid" | "lbtc" | "liquidv1" => Ok("liquid"),
        "lightning" | "ln" => Ok("lightning"),
        _ => Err(app(format!(
            "Unsupported Bull Bitcoin wallet network '{}'",
            found.unwrap_or_else(|| "None".into())
        ))),
    }
}

/// `str_or_none(value).strip().lower()` when it is 64 characters that
/// `bytes.fromhex` accepts (ASCII whitespace may separate byte pairs).
fn hex64(value: Option<String>) -> Option<(String, Vec<u8>)> {
    let text = kassiber_pytext::lower(kassiber_pytext::strip(&value?));
    if text.chars().count() != 64 {
        return None;
    }
    let mut bytes = Vec::new();
    let mut chars = text.chars().peekable();
    loop {
        while chars.next_if(|c| " \t\n\r\x0b\x0c".contains(*c)).is_some() {}
        let Some(high) = chars.next() else { break };
        let pair = [high.to_digit(16)?, chars.next()?.to_digit(16)?];
        bytes.push((pair[0] * 16 + pair[1]) as u8);
    }
    Some((text, bytes))
}

fn record(row: csv::Row, index: usize) -> Result<Option<csv::Row>, AppError> {
    let columns = Columns::new(&row);
    let kind = folded(columns.get(&["type"]));
    let status = folded(columns.get(&["status"]));
    if ["failed", "expired", "refunded"].contains(&status.as_str()) {
        return Ok(None);
    }
    let Some(direction) = direction(columns.get(&["direction"]))? else {
        return Ok(None);
    };
    let amount = sats(&columns, "amount_sats", false)?;
    let fee = sats(&columns, "fee_sats", true)?;
    let network = network(&columns, &kind, direction)?;
    let asset = if network == "liquid" || kind == "liquid" {
        "LBTC"
    } else {
        "BTC"
    };
    let external_id = ["txid", "receive_txid", "send_txid", "swap_id"]
        .iter()
        .find_map(|c| columns.opt(&[c]))
        .unwrap_or_else(|| {
            let cell = |c: &str| columns.get(&[c]);
            format!(
                "bullbitcoin-wallet:{}:{}:{}:{}",
                cell("date").unwrap_or_else(|| index.to_string()),
                cell("type").unwrap_or_else(|| "transaction".into()),
                cell("direction").unwrap_or_default(),
                cell("amount_sats").unwrap_or_default()
            )
        });
    let payment_hash = if ["lightning_send", "lightning_receive"].contains(&kind.as_str()) {
        hex64(columns.get(&["preimage"]))
            .map(|(_, preimage)| {
                use sha2::Digest;
                sha2::Sha256::digest(preimage)
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()
            })
            .or_else(|| hex64(Some(external_id.clone())).map(|(text, _)| text))
    } else {
        None
    };
    let mut raw = columns.json();
    let mut redacted = false;
    if let Json::Object(fields) = &mut raw {
        for (key, value) in fields.iter_mut() {
            let present = value
                .as_str()
                .is_some_and(|v| !kassiber_pytext::strip(v).is_empty());
            if river::column(key) == "preimage" && present {
                *value = json!("[redacted]");
                redacted = true;
            }
        }
        if redacted {
            fields.insert("preimage_redacted".into(), json!(true));
        }
        fields.insert("source".into(), json!("bullbitcoin_wallet_csv"));
        fields.insert("normalized_network".into(), json!(network));
    }
    let date = columns.get(&["date"]);
    let label = if kind.is_empty() {
        "transaction".to_owned()
    } else {
        kind.clone()
    };
    let mut data = json!({
        "txid": external_id,
        "occurred_at": date,
        "confirmed_at": if ["confirmed", "completed"].contains(&status.as_str()) { date.clone() } else { None },
        "direction": direction,
        "asset": asset,
        "amount": btc(&amount)?,
        "fee": btc(&fee)?,
        "fiat_rate": null,
        "fiat_value": null,
        "kind": label,
        "description": format!("Bull Bitcoin wallet {label}"),
        "counterparty": null,
        "payment_hash": payment_hash,
        "payment_hash_source": payment_hash.as_ref().map(|_| "importer"),
        "_bullbitcoin_wallet_network": network,
        "_match_existing_ignore_fee": "True",
        "raw_json": config::stored_json(&raw),
    });
    if ["payjoin_send", "payjoin_receive"].contains(&kind.as_str()) {
        data["privacy_boundary"] = json!("payjoin");
    }
    Ok(Some(csv::Row {
        headers: vec![],
        original: None,
        data,
        rest: None,
    }))
}

/// `filter_bullbitcoin_wallet_records` for the wallet's configured network.
pub(super) fn filter(rows: Vec<csv::Row>, network: Option<&str>) -> Vec<csv::Row> {
    match network {
        None => rows,
        Some(network) => rows
            .into_iter()
            .filter(|row| row.data["_bullbitcoin_wallet_network"].as_str() == Some(network))
            .collect(),
    }
}
