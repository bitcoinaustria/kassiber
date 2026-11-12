//! `fetch_binance_records`: four signed history reads, then
//! `normalize_binance_records`.
use super::*;

/// Binance.com spot trading launched in July 2017; Python reads from there.
const START_MS: &str = "1498867200000";

pub(super) fn fetch(client: &Client<'_>) -> Result<Vec<Py>, AppError> {
    let end = now()?.as_millis().to_string();
    let fiat = signed_get(
        client,
        "/sapi/v1/fiat/payments",
        &[
            ("transactionType", "0"),
            ("beginTime", START_MS),
            ("endTime", &end),
        ],
    )?;
    let fiat = attribute_get(&fiat, "data")?;
    let deposits = signed_get(
        client,
        "/sapi/v1/capital/deposit/hisrec",
        &[("coin", "BTC"), ("startTime", START_MS), ("endTime", &end)],
    )?;
    let withdrawals = signed_get(
        client,
        "/sapi/v1/capital/withdraw/history",
        &[("coin", "BTC"), ("startTime", START_MS), ("endTime", &end)],
    )?;
    let dividends = signed_get(
        client,
        "/sapi/v1/asset/assetDividend",
        &[
            ("asset", "BTC"),
            ("startTime", START_MS),
            ("endTime", &end),
            ("limit", "500"),
        ],
    )?;
    let dividends = attribute_get(&dividends, "rows")?;
    normalize(&[
        (Kind::Fiat, fiat),
        (Kind::Deposit, deposits),
        (Kind::Withdrawal, withdrawals),
        (Kind::Dividend, dividends),
    ])
}

/// Restrictions that only let a key read.
const READ_FLAGS: [&str; 2] = ["enableReading", "enableFixReadOnly"];

/// The desktop key check: `GET /sapi/v1/account/apiRestrictions` says what
/// the key may do. It must read, and every other `enable*`/`permits*` flag
/// (trading, margin, futures, options, withdrawals, transfers, and any flag
/// Binance adds later) must be off.
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    let restrictions = signed_get(client, "/sapi/v1/account/apiRestrictions", &[])?;
    let Py::Dict(flags) = &restrictions else {
        return Err(AppError::new(
            "protocol_error",
            "Binance API restrictions were not a JSON object",
        ));
    };
    if at(&restrictions, "enableReading") != &Py::Bool(true) {
        return Err(AppError::new(
            "auth_error",
            "This Binance API key cannot read account history",
        )
        .with_hint("Turn on Enable Reading for the key in Binance API Management."));
    }
    let mut granted: Vec<String> = flags
        .iter()
        .map(|(name, value)| (lossy(&key_text(name)), value))
        .filter(|(name, value)| {
            (name.starts_with("enable") || name.starts_with("permits"))
                && !READ_FLAGS.contains(&name.as_str())
                && **value == Py::Bool(true)
        })
        .map(|(name, _)| name)
        .collect();
    if !granted.is_empty() {
        granted.sort();
        return Err(AppError::new(
            "auth_error",
            "This Binance API key can do more than read; Kassiber only accepts read-only keys",
        )
        .with_hint(
            "Edit the key's restrictions in Binance API Management so only Enable Reading is on, or create a new read-only key.",
        )
        .with_details(dict([(
            "permissions",
            Py::List(granted.into_iter().map(s).collect()),
        )])));
    }
    Ok(Some(true))
}

/// `normalize_binance_records(payload)` for an assembled payload dict.
#[cfg(test)]
pub(super) fn normalize_payload(payload: &Py) -> Result<Vec<Py>, AppError> {
    let value = |key| get(payload, key).cloned().unwrap_or(Py::List(vec![]));
    normalize(&[
        (Kind::Fiat, value("fiat_payments")),
        (Kind::Deposit, value("deposits")),
        (Kind::Withdrawal, value("withdrawals")),
        (Kind::Dividend, value("dividends")),
    ])
}

/// `payload.get(key, [])` on whatever the server returned.
fn attribute_get(payload: &Py, key: &str) -> Result<Py, AppError> {
    match payload {
        Py::Dict(_) => Ok(get(payload, key).cloned().unwrap_or(Py::List(vec![]))),
        other => Err(AppError::new(
            "internal_error",
            format!(
                "'{}' object has no attribute 'get'",
                kassiber_envelope::type_name(other)
            ),
        )),
    }
}

/// `_binance_signed_get`: the query in insertion order, a fresh millisecond
/// timestamp and an HMAC-SHA256 signature over exactly that query.
fn signed_get(client: &Client<'_>, path: &str, params: &[(&str, &str)]) -> Result<Py, AppError> {
    let timestamp = now()?.as_millis().to_string();
    let query = params
        .iter()
        .copied()
        .chain([("timestamp", timestamp.as_str())])
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("&");
    let signature = hmac_sha256_hex(client.secret.as_bytes(), query.as_bytes());
    let url = format!("{}{path}?{query}&signature={signature}", client.base);
    client.read(
        "GET",
        &url,
        &[("Accept", "application/json"), ("X-MBX-APIKEY", client.key)],
        None,
    )
}

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Fiat,
    Deposit,
    Withdrawal,
    Dividend,
}

/// `normalize_binance_records(payload)`: each history's rows in order, as
/// `for row in payload.get(key, []) or []` iterates them.
fn normalize(payload: &[(Kind, Py)]) -> Result<Vec<Py>, AppError> {
    let mut records = vec![];
    for (kind, rows) in payload {
        let rows = match rows {
            Py::List(rows) => rows.as_slice(),
            // Iterating a dict or str yields keys or characters, never rows.
            Py::Dict(_) | Py::Str(_) => &[],
            other if !truthy(other) => &[],
            other => {
                return Err(AppError::new(
                    "internal_error",
                    format!(
                        "'{}' object is not iterable",
                        kassiber_envelope::type_name(other)
                    ),
                ))
            }
        };
        for row in rows.iter().filter(|row| matches!(row, Py::Dict(_))) {
            let record = match kind {
                Kind::Fiat => fiat_payment(row)?,
                Kind::Deposit => transfer(row, true)?,
                Kind::Withdrawal => transfer(row, false)?,
                Kind::Dividend => dividend(row)?,
            };
            records.extend(record);
        }
    }
    Ok(records)
}

/// `str(value or "").strip().casefold()`.
fn folded(value: &Py) -> String {
    let text = if truthy(value) {
        lossy(&text(value))
    } else {
        String::new()
    };
    kassiber_pytext::casefold(kassiber_pytext::strip(&text))
}

/// `normalize_binance_fiat_payment(row)`.
fn fiat_payment(row: &Py) -> Result<Option<Py>, AppError> {
    if folded(at(row, "status")) != "completed" {
        return Ok(None);
    }
    let currency = asset(at(row, "cryptoCurrency"));
    if !is_bitcoin(&currency) {
        return Ok(None);
    }
    let fiat_currency = asset(at(row, "fiatCurrency"));
    if !is_fiat(&fiat_currency) {
        return Err(AppError::new(
            "validation",
            "Binance fiat payment is not fiat-denominated",
        ));
    }
    let amount = decimal(at(row, "obtainAmount"), "0")?;
    let fiat_value = abs(decimal(at(row, "sourceAmount"), "0")?)?;
    let occurred_at = epoch(at(row, "createTime"), true)?;
    let reference = either(row, &["orderNo", "orderId"]);
    let reference = if truthy(reference) {
        text(reference)
    } else {
        occurred_at.clone().unwrap_or_else(|| "None".into())
    };
    let price = at(row, "price");
    let fiat_rate = if *price == Py::Null || *price == s("") {
        None
    } else {
        Some(decimal(price, "0")?)
    };
    record(Entry {
        provider: "Binance",
        external_id: concat(&["binance:".into(), reference.clone()]),
        occurred_at,
        inbound: true,
        asset: currency,
        amount,
        fee: PriceNumber::Finite(Decimal::zero()),
        kind: "buy",
        raw: row.clone(),
        description: None,
        pricing: Some(Pricing {
            fiat_currency,
            fiat_value,
            fiat_rate,
            method: "binance_api",
            external_ref: reference,
        }),
    })
    .map(Some)
}

/// `normalize_binance_transfer(row, inbound=...)`.
fn transfer(row: &Py, inbound: bool) -> Result<Option<Py>, AppError> {
    let currency = asset(either(row, &["coin", "asset"]));
    if !is_bitcoin(&currency) {
        return Ok(None);
    }
    let status = folded(at(row, "status"));
    if !status.is_empty()
        && !["1", "6", "success", "completed", "credited", "confirming"].contains(&status.as_str())
    {
        return Ok(None);
    }
    let amount = decimal(at(row, "amount"), "0")?;
    let fee = decimal(either(row, &["transactionFee", "fee"]), "0")?;
    let occurred_at = epoch(
        either(row, &["insertTime", "applyTime", "successTime"]),
        true,
    )?;
    let external_id =
        str_or_none(either(row, &["txId", "id", "withdrawOrderId"])).unwrap_or_else(|| {
            concat(&[
                "binance:".into(),
                occurred_at.clone().unwrap_or_else(|| "None".into()),
                format!(":{currency}:{}", repr(&amount)).into(),
            ])
        });
    record(Entry {
        provider: "Binance",
        external_id,
        occurred_at,
        inbound,
        asset: currency,
        amount,
        fee: if inbound {
            PriceNumber::Finite(Decimal::zero())
        } else {
            fee
        },
        kind: if inbound { "deposit" } else { "withdrawal" },
        raw: row.clone(),
        description: Some(
            if inbound {
                "Binance deposit"
            } else {
                "Binance withdrawal"
            }
            .into(),
        ),
        pricing: None,
    })
    .map(Some)
}

/// `normalize_binance_dividend(row)`.
fn dividend(row: &Py) -> Result<Option<Py>, AppError> {
    let currency = asset(either(row, &["asset", "coinName"]));
    if !is_bitcoin(&currency) {
        return Ok(None);
    }
    let amount = decimal(either(row, &["amount", "profitAmount"]), "0")?;
    if is_zero(&amount)? {
        return Ok(None);
    }
    let reference = text(either(row, &["id", "tranId", "time", "divTime"]));
    let occurred_at = epoch(either(row, &["divTime", "time"]), true)?;
    let info = str_or_none(either(row, &["enInfo", "type"]));
    let mining = info
        .as_ref()
        .is_some_and(|info| kassiber_pytext::casefold(&lossy(info)).contains("mining"));
    let description = match &info {
        Some(info) => concat(&["Binance income - ".into(), info.clone()]),
        None => "Binance income".into(),
    };
    record(Entry {
        provider: "Binance",
        external_id: concat(&["binance:".into(), reference]),
        occurred_at,
        inbound: true,
        asset: currency,
        amount,
        fee: PriceNumber::Finite(Decimal::zero()),
        kind: if mining { "mining" } else { "income" },
        raw: row.clone(),
        description: Some(description),
        pricing: None,
    })
    .map(Some)
}
