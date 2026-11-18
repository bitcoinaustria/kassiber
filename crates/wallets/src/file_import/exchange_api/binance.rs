//! `fetch_binance_records`: four signed history reads, then
//! `normalize_binance_records`.
use super::*;

/// Binance.com spot trading launched in July 2017; Python reads from there.
const START_MS: &str = "1498867200000";

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "binance",
    provider: "Binance",
    reads: contract::Reads::BitcoinOnly,
    default_url: "https://api.binance.com",
    key_only: false,
    fetch,
    probe,
    skipped: None,
};

fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<(), AppError> {
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
    normalize(
        &[
            (History::Fiat, fiat),
            (History::Deposit, deposits),
            (History::Withdrawal, withdrawals),
            (History::Dividend, dividends),
        ],
        mapper,
    )
}

/// Restrictions that only let a key read.
const READ_FLAGS: [&str; 2] = ["enableReading", "enableFixReadOnly"];

/// The desktop key check: `GET /sapi/v1/account/apiRestrictions` says what
/// the key may do. It must read, and every other `enable*`/`permits*` flag
/// (trading, margin, futures, options, withdrawals, transfers, and any flag
/// Binance adds later) must be off.
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
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
pub(super) fn normalize_payload(payload: &Py, mapper: &mut Mapper) -> Result<(), AppError> {
    let value = |key| get(payload, key).cloned().unwrap_or(Py::List(vec![]));
    normalize(
        &[
            (History::Fiat, value("fiat_payments")),
            (History::Deposit, value("deposits")),
            (History::Withdrawal, value("withdrawals")),
            (History::Dividend, value("dividends")),
        ],
        mapper,
    )
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
enum History {
    Fiat,
    Deposit,
    Withdrawal,
    Dividend,
}

/// `normalize_binance_records(payload)`: each history's rows in order, as
/// `for row in payload.get(key, []) or []` iterates them.
fn normalize(payload: &[(History, Py)], mapper: &mut Mapper) -> Result<(), AppError> {
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
            let event = match kind {
                History::Fiat => fiat_payment(row, mapper)?,
                History::Deposit => transfer(row, true, mapper)?,
                History::Withdrawal => transfer(row, false, mapper)?,
                History::Dividend => dividend(row, mapper)?,
            };
            if let Some(event) = event {
                mapper.push(event)?;
            }
        }
    }
    Ok(())
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
fn fiat_payment(row: &Py, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    if folded(at(row, "status")) != "completed" {
        return Ok(None);
    }
    let currency = asset(at(row, "cryptoCurrency"));
    if !mapper.admit(&[&currency]) {
        return Ok(None);
    }
    let fiat_currency = fiat_code(asset(at(row, "fiatCurrency")));
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
    let external_id = concat(&["binance:".into(), reference.clone()]);
    Ok(Some(Event {
        occurred_at,
        raw: row.clone(),
        pricing: Some(Pricing {
            fiat_currency,
            fiat_value,
            fiat_rate,
            method: "binance_api",
            external_ref: reference,
        }),
        ..Event::one(external_id, Kind::Trade, true, currency, amount)
    }))
}

/// `normalize_binance_transfer(row, inbound=...)`.
fn transfer(row: &Py, inbound: bool, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    let currency = asset(either(row, &["coin", "asset"]));
    if !mapper.admit(&[&currency]) {
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
    let (kind, description) = match inbound {
        true => ("deposit", "Binance deposit"),
        false => ("withdrawal", "Binance withdrawal"),
    };
    Ok(Some(Event {
        occurred_at,
        fee: if inbound {
            PriceNumber::Finite(Decimal::zero())
        } else {
            fee
        },
        raw: row.clone(),
        description: Some(description.into()),
        ..Event::one(external_id, Kind::Movement(kind), inbound, currency, amount)
    }))
}

/// `normalize_binance_dividend(row)`.
fn dividend(row: &Py, mapper: &mut Mapper) -> Result<Option<Event>, AppError> {
    let currency = asset(either(row, &["asset", "coinName"]));
    if !mapper.admit(&[&currency]) {
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
    let kind = Kind::Movement(if mining { "mining" } else { "income" });
    let external_id = concat(&["binance:".into(), reference]);
    Ok(Some(Event {
        occurred_at,
        raw: row.clone(),
        description: Some(description),
        ..Event::one(external_id, kind, true, currency, amount)
    }))
}
