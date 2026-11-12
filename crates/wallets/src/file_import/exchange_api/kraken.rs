//! `fetch_kraken_records`: signed `TradesHistory` then `Ledgers` pages, and
//! `normalize_kraken_records` over the merged payloads.
use super::*;
use kassiber_pyjson::BigInt;

pub(super) fn fetch(client: &Client<'_>) -> Result<Vec<Py>, AppError> {
    let mut pages = 0;
    let trades = paginated(client, "/0/private/TradesHistory", "trades", &mut pages)?;
    let ledgers = paginated(client, "/0/private/Ledgers", "ledger", &mut pages)?;
    normalize(&ledgers, &trades)
}

/// The desktop key check: the first page of both histories the sync reads,
/// so a key without either query permission fails here. Kraken does not say
/// what else a key may do.
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    for path in ["/0/private/TradesHistory", "/0/private/Ledgers"] {
        if !matches!(at(&post(client, path, "0")?, "result"), Py::Dict(_)) {
            return Err(AppError::new(
                "protocol_error",
                "Kraken response was not a JSON result object",
            ));
        }
    }
    Ok(None)
}

/// `_kraken_private_paginated`: rows keep their first position and take their
/// latest value, and the reply is `{"result": {field: rows, "count": n}}`.
fn paginated(
    client: &Client<'_>,
    path: &str,
    field: &str,
    pages: &mut usize,
) -> Result<Py, AppError> {
    let mut rows: Vec<(Key, Py)> = vec![];
    let mut offset = BigInt::from(0);
    let mut total: Option<BigInt> = None;
    while total.as_ref().is_none_or(|total| offset < *total) {
        *pages += 1;
        if *pages > MAX_PAGES {
            return Err(limit());
        }
        let payload = post(client, path, &offset.to_string())?;
        let result = at(&payload, "result");
        if !matches!(result, Py::Dict(_)) {
            return Err(AppError::new(
                "protocol_error",
                "Kraken response was not a JSON result object",
            ));
        }
        let Py::Dict(page) = at(result, field) else {
            return Err(AppError::new(
                "protocol_error",
                format!("Kraken response result.{field} was not a JSON object"),
            ));
        };
        for (id, row) in page {
            if !matches!(row, Py::Dict(_)) {
                continue;
            }
            let id = Key::Str(key_text(id));
            match rows.iter_mut().find(|(k, _)| *k == id) {
                Some((_, slot)) => *slot = row.clone(),
                None => rows.push((id, row.clone())),
            }
        }
        if rows.len() > MAX_ROWS {
            return Err(limit());
        }
        let count = BigInt::from(rows.len());
        let parsed = match get(result, "count") {
            None => count.clone(),
            Some(value) => integer(value)?.unwrap_or_else(|| count.clone()),
        };
        if page.is_empty() || count >= parsed {
            break;
        }
        total = Some(parsed);
        offset += BigInt::from(page.len());
    }
    let count = Py::from(rows.len() as i64);
    Ok(dict([(
        "result",
        dict([(field, Py::Dict(rows)), ("count", count)]),
    )]))
}

/// `int(value)`; `TypeError` and `ValueError` fall back to the row count.
fn integer(value: &Py) -> Result<Option<BigInt>, AppError> {
    Ok(match value {
        Py::Int(n) => Some(n.clone()),
        Py::Bool(b) => Some(BigInt::from(i32::from(*b))),
        Py::Float(f) if f.is_nan() => None,
        Py::Float(f) if f.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        Py::Float(f) => f.trunc().to_string().parse().ok(),
        Py::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::parse_int(text).ok()),
        _ => None,
    })
}

/// `_kraken_private_post`: one nonce-signed form POST and its error check.
fn post(client: &Client<'_>, path: &str, offset: &str) -> Result<Py, AppError> {
    let nonce = now()?.as_nanos().to_string();
    let body = format!("nonce={nonce}&ofs={offset}");
    let mut secret = base64_decode(client.secret)
        .ok_or_else(|| AppError::new("config_error", "Kraken API secret must be base64 encoded"))?;
    let digest = sha256::Hash::hash(format!("{nonce}{body}").as_bytes());
    let signature = kassiber_chain::encode_base64(&hmac_sha512(
        &secret,
        &[path.as_bytes(), digest.as_byte_array()],
    ));
    secret.fill(0);
    let url = format!("{}{path}", client.base);
    let payload = client.read(
        "POST",
        &url,
        &[
            ("API-Key", client.key),
            ("API-Sign", &signature),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        Some(body.as_bytes()),
    )?;
    let errors = at(&payload, "error");
    if !truthy(errors) {
        return Ok(payload);
    }
    // One error string is one item, so an echoed credential stays whole.
    let items: Vec<PyStr> = match errors {
        Py::List(items) => items.iter().map(text).collect(),
        Py::Str(text) => vec![text.clone()],
        Py::Dict(items) => items.iter().map(|(k, _)| key_text(k)).collect(),
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
    let permission = items
        .iter()
        .any(|item| lossy(&lower(item)).contains("permission"));
    let items = items
        .iter()
        .map(|item| Py::Str(scrub(item, &[client.key, client.secret])))
        .collect();
    Err(AppError::new(
        if permission {
            "auth_error"
        } else {
            "protocol_error"
        },
        "Kraken API returned an error",
    )
    .with_details(dict([("kraken_errors", Py::List(items))])))
}

/// `withhold_credentials(item, (key, secret))` over code points, since an
/// error item may hold lone surrogates.
fn scrub(text: &PyStr, secrets: &[&str]) -> PyStr {
    let mut points: Vec<u32> = text.code_points().collect();
    let replacement: Vec<u32> = "<redacted>".chars().map(u32::from).collect();
    for form in kassiber_chain::credential_forms(secrets) {
        let needle: Vec<u32> = form.chars().map(u32::from).collect();
        let mut result = Vec::with_capacity(points.len());
        let mut index = 0;
        while index < points.len() {
            if points[index..].starts_with(&needle) {
                result.extend(&replacement);
                index += needle.len();
            } else {
                result.push(points[index]);
                index += 1;
            }
        }
        points = result;
    }
    PyStr::from_code_points(points).expect("code points")
}

/// `base64.b64decode(text)`: non-validating `binascii.a2b_base64`, which skips
/// characters outside the alphabet and stops at a complete pad sequence.
pub(super) fn base64_decode(text: &str) -> Option<Vec<u8>> {
    if !text.is_ascii() {
        return None;
    }
    let value = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let (mut result, mut quad, mut left, mut pads) = (vec![], 0, 0u8, 0);
    for c in text.bytes() {
        if c == b'=' {
            pads += 1;
            if quad >= 2 && quad + pads >= 4 {
                return Some(result);
            }
            continue;
        }
        let Some(v) = value(c) else { continue };
        pads = 0;
        match quad {
            0 => {
                quad = 1;
                left = v;
            }
            1 => {
                quad = 2;
                result.push((left << 2) | (v >> 4));
                left = v & 0x0f;
            }
            2 => {
                quad = 3;
                result.push((left << 4) | (v >> 2));
                left = v & 0x03;
            }
            _ => {
                quad = 0;
                result.push((left << 6) | v);
            }
        }
    }
    (quad == 0).then_some(result)
}

/// `_mapping_rows(payload, *keys)`. Paging always assembles mappings, so the
/// reference's list form is unreachable here.
fn mapping_rows(payload: &Py, keys: &[&str]) -> Vec<(PyStr, Py)> {
    let mut value = payload;
    for key in keys {
        if let Some(found) = get(value, key) {
            value = found;
        }
    }
    match value {
        Py::Dict(items) => items
            .iter()
            .filter(|(_, item)| matches!(item, Py::Dict(_)))
            .map(|(key, item)| (key_text(key), item.clone()))
            .collect(),
        _ => vec![],
    }
}

/// `normalize_kraken_records(ledger_payload, trades_payload)`.
pub(super) fn normalize(ledgers: &Py, trades: &Py) -> Result<Vec<Py>, AppError> {
    let mut trade_rows = mapping_rows(trades, &["result", "trades"]);
    if trade_rows.is_empty() {
        trade_rows = mapping_rows(trades, &["trades"]);
    }
    // dict(rows): a repeated id keeps its first position and its last value.
    let mut by_id: Vec<(PyStr, Py)> = vec![];
    for (id, row) in trade_rows {
        match by_id.iter_mut().find(|(k, _)| *k == id) {
            Some((_, slot)) => *slot = row,
            None => by_id.push((id, row)),
        }
    }
    let mut rows = mapping_rows(ledgers, &["result", "ledger"]);
    if rows.is_empty() {
        rows = mapping_rows(ledgers, &["ledger"]);
    }
    let mut records = vec![];
    for (ledger_id, row) in rows {
        let currency = asset(at(&row, "asset"));
        if !is_bitcoin(&currency) {
            continue;
        }
        let kind = at(&row, "type");
        let kind = lower(&kassiber_pytext::strip_py(&if truthy(kind) {
            text(kind)
        } else {
            "".into()
        }));
        let amount = decimal(at(&row, "amount"), "0")?;
        let fee = decimal(at(&row, "fee"), "0")?;
        let occurred_at = epoch(at(&row, "time"), false)?;
        let external_id = concat(&["kraken:".into(), ledger_id.clone()]);
        let raw = merged(&[("ledger_id", Py::Str(ledger_id.clone()))], &row, &[]);
        let token = kind.as_str().unwrap_or("");
        if matches!(token, "deposit" | "withdrawal") {
            let inbound = at_least_zero(&amount)?;
            records.push(record(Entry {
                provider: "Kraken",
                external_id,
                occurred_at,
                inbound,
                asset: currency,
                amount,
                fee,
                kind: if inbound { "deposit" } else { "withdrawal" },
                raw,
                description: None,
                pricing: None,
            })?);
            continue;
        }
        if token != "trade" {
            return Err(AppError::new("validation", "")
                .with_python_message(concat(&[
                    "Kraken BTC ledger row '".into(),
                    ledger_id,
                    "' has unsupported type '".into(),
                    kind,
                    "'".into(),
                ]))
                .with_hint(
                    "Only BTC/LBTC deposits, withdrawals, and fiat-quoted trades are imported automatically.",
                ));
        }
        let refid = at(&row, "refid");
        let refid = truthy(refid).then(|| text(refid));
        let wanted = refid.clone().unwrap_or_else(|| "".into());
        let trade = by_id
            .iter()
            .find(|(id, _)| *id == wanted)
            .map(|(_, trade)| trade)
            .filter(|trade| truthy(trade));
        let Some(trade) = trade else {
            return Err(AppError::new("validation", "")
                .with_python_message(concat(&[
                    "Kraken trade ledger row '".into(),
                    ledger_id,
                    "' is missing its TradesHistory row".into(),
                ]))
                .with_hint(
                    "Provide both Kraken ledger and trade history so execution prices can be verified.",
                ));
        };
        let pair = at(trade, "pair");
        let pair = if truthy(pair) {
            lossy(&text(pair))
        } else {
            String::new()
        };
        let Some(fiat_currency) = quote(&pair, &currency)
            .map(|quote| asset(&s(quote.as_str())))
            .filter(|code| is_fiat(code))
        else {
            return Err(AppError::new("validation", "")
                .with_python_message(concat(&[
                    "Kraken BTC trade '".into(),
                    ledger_id,
                    "' is not fiat-quoted".into(),
                ]))
                .with_hint(
                    "Cross-asset Kraken trades need explicit user review through the generic ledger.",
                ));
        };
        let cost = abs(decimal(at(trade, "cost"), "0")?)?;
        let trade_fee = abs(decimal(at(trade, "fee"), "0")?)?;
        let inbound = at_least_zero(&amount)?;
        let fiat_value = if inbound {
            cost.clone()
        } else {
            nonnegative(arithmetic(&cost, &trade_fee, '-')?)?
        };
        let raw = merged(&[], &raw, &[("trade", trade.clone())]);
        // Kraken's price is cost / volume; without it, rebuild that rather
        // than divide a sell's net-of-fee proceeds.
        let price = at(trade, "price");
        let fiat_rate = if *price != Py::Null && *price != s("") {
            Some(decimal(price, "0")?)
        } else if truth(&cost) && truth(&amount) {
            Some(divide(&cost, &abs(amount.clone())?)?)
        } else {
            None
        };
        records.push(record(Entry {
            provider: "Kraken",
            external_id,
            occurred_at,
            inbound,
            asset: currency,
            amount,
            fee,
            kind: if inbound { "buy" } else { "sell" },
            raw,
            description: None,
            pricing: Some(Pricing {
                fiat_currency,
                fiat_value,
                fiat_rate,
                method: "kraken_api",
                external_ref: refid.unwrap_or(ledger_id),
            }),
        })?);
    }
    Ok(records)
}

/// `_kraken_quote_from_pair(pair, base_asset)`. Of the fiat suffixes, the
/// four-character `Z` forms sort first; at most one suffix per length matches.
pub(super) fn quote(pair: &str, base: &str) -> Option<String> {
    let normalized = kassiber_pytext::upper(pair).replace('/', "");
    let alias = if base == "BTC" { "XBT" } else { base };
    for candidate in [base, alias] {
        for prefix in [candidate.to_owned(), format!("X{candidate}")] {
            if let Some(rest) = normalized.strip_prefix(&prefix) {
                return (!rest.is_empty()).then(|| rest.to_owned());
            }
        }
    }
    FIAT.iter()
        .map(|code| format!("Z{code}"))
        .chain(FIAT.iter().map(|code| (*code).to_owned()))
        .find(|code| normalized.ends_with(code.as_str()))
}
