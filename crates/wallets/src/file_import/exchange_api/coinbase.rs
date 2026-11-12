//! `fetch_coinbase_records`: signed v2 account and transaction pages, then
//! `normalize_coinbase_records`.
use super::*;
use kassiber_net::HttpUrl;

const TRADES: [&str; 4] = ["buy", "sell", "trade", "advanced_trade_fill"];
const MOVEMENTS: [&str; 6] = [
    "send",
    "exchange_deposit",
    "exchange_withdrawal",
    "pro_deposit",
    "pro_withdrawal",
    "prime_withdrawal",
];
const INCOME: [&str; 3] = ["interest", "staking_reward", "inflation_reward"];

pub(super) fn fetch(client: &Client<'_>) -> Result<Vec<Py>, AppError> {
    let mut budget = Budget::default();
    let accounts = paginated(client, "/v2/accounts", &mut budget)?;
    let mut transactions = vec![];
    for account in &accounts {
        if !matches!(account, Py::Dict(_)) {
            continue;
        }
        let currency = at(account, "currency");
        let code = match currency {
            Py::Dict(_) => at(currency, "code"),
            other => other,
        };
        if !is_bitcoin(&asset(code)) {
            continue;
        }
        let id = at(account, "id");
        let id = if truthy(id) { text(id) } else { "".into() };
        if id.code_points().next().is_none() {
            continue;
        }
        let path = format!("/v2/accounts/{}/transactions", quote(&id)?);
        let rows = paginated(client, &path, &mut budget)?;
        // str(item.get("currency") or ...): the code was truthy to get here.
        transactions.push((text(code), rows));
    }
    records(&transactions)
}

/// `normalize_coinbase_records` over `{"currency", "transactions"}` items.
pub(super) fn records(accounts: &[(PyStr, Vec<Py>)]) -> Result<Vec<Py>, AppError> {
    let mut records = vec![];
    for (account_currency, rows) in accounts {
        for tx in rows.iter().filter(|tx| matches!(tx, Py::Dict(_))) {
            let amount = at(tx, "amount");
            let currency = match amount {
                Py::Dict(_) => at(amount, "currency"),
                _ => &Py::Null,
            };
            let currency = if truthy(currency) {
                asset(currency)
            } else {
                asset(&Py::Str(account_currency.clone()))
            };
            if !is_bitcoin(&currency) {
                continue;
            }
            if let Some(record) = normalize(tx, &currency)? {
                records.push(record);
            }
        }
    }
    Ok(records)
}

#[derive(Default)]
struct Budget {
    pages: usize,
    rows: usize,
}

/// `_coinbase_get_paginated`. Python follows any `next_uri`; here a page
/// may only continue the collection it belongs to, on the same origin.
fn paginated(
    client: &Client<'_>,
    collection: &str,
    budget: &mut Budget,
) -> Result<Vec<Py>, AppError> {
    let mut rows = vec![];
    let mut seen = std::collections::BTreeSet::new();
    let mut next = Some(collection.to_owned());
    while let Some(path) = next {
        budget.pages += 1;
        if budget.pages > MAX_PAGES || !seen.insert(path.clone()) {
            return Err(limit());
        }
        let (data, pagination) = page(client, collection, &path)?;
        budget.rows += data.len();
        if budget.rows > MAX_ROWS {
            return Err(limit());
        }
        rows.extend(data);
        let next_uri = at(&pagination, "next_uri");
        next = truthy(next_uri).then(|| lossy(&text(next_uri)));
    }
    Ok(rows)
}

/// One signed page of `collection`: its `data` rows and its `pagination`.
fn page(client: &Client<'_>, collection: &str, path: &str) -> Result<(Vec<Py>, Py), AppError> {
    let url = page_url(client.base, collection, path)?;
    let timestamp = now()?.as_secs().to_string();
    let signature = hmac_sha256_hex(
        client.secret.as_bytes(),
        format!("{timestamp}GET{path}").as_bytes(),
    );
    let payload = client.read(
        "GET",
        &url,
        &[
            ("Accept", "application/json"),
            ("CB-VERSION", "2017-11-27"),
            ("CB-ACCESS-KEY", client.key),
            ("CB-ACCESS-SIGN", &signature),
            ("CB-ACCESS-TIMESTAMP", &timestamp),
        ],
        None,
    )?;
    if !matches!(payload, Py::Dict(_)) {
        return Err(AppError::new(
            "protocol_error",
            "Coinbase response was not a JSON object",
        ));
    }
    let Py::List(data) = at(&payload, "data") else {
        return Err(AppError::new(
            "protocol_error",
            "Coinbase response data was not a JSON array",
        ));
    };
    Ok((data.clone(), at(&payload, "pagination").clone()))
}

/// The desktop key check: the first accounts page, which needs
/// `wallet:accounts:read`. Coinbase does not say what else a key may do.
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    page(client, "/v2/accounts", "/v2/accounts")?;
    Ok(None)
}

/// `f"{base}{path}"`, refusing a path that leaves `collection` or that the
/// HTTP encoder would send differently from what was signed.
pub(super) fn page_url(base: &str, collection: &str, path: &str) -> Result<String, AppError> {
    let refused = || {
        AppError::new(
            "protocol_error",
            "Coinbase pagination left the requested history collection",
        )
    };
    let rest = path.strip_prefix(collection).ok_or_else(refused)?;
    if !(rest.is_empty() || rest.starts_with('?'))
        || !path.bytes().all(|b| b.is_ascii_graphic() && b != b'#')
    {
        return Err(refused());
    }
    let url = format!("{base}{path}");
    let parsed = HttpUrl::parse(&url).map_err(|_| refused())?;
    let prefix = HttpUrl::parse(base).map_err(|_| refused())?;
    let sent = format!(
        "{}{}",
        parsed.path(),
        parsed.query().map(|q| format!("?{q}")).unwrap_or_default()
    );
    if parsed.origin() != prefix.origin()
        || sent != format!("{}{path}", prefix.path().trim_end_matches('/'))
    {
        return Err(refused());
    }
    Ok(url)
}

/// `urllib.parse.quote(text)`: UTF-8 bytes, with `/` and `_.-~` kept.
pub(super) fn quote(text: &PyStr) -> Result<String, AppError> {
    let Some(text) = text.as_str() else {
        let (position, point) = text
            .code_points()
            .enumerate()
            .find(|(_, p)| char::from_u32(*p).is_none())
            .expect("a surrogate");
        return Err(AppError::new(
            "internal_error",
            format!(
                "'utf-8' codec can't encode character '\\u{point:04x}' in position {position}: surrogates not allowed"
            ),
        ));
    };
    let mut result = String::new();
    for b in text.bytes() {
        if b.is_ascii_alphanumeric() || b"/_.-~".contains(&b) {
            result.push(char::from(b));
        } else {
            result.push_str(&format!("%{b:02X}"));
        }
    }
    Ok(result)
}

/// `_coinbase_money(tx, key)`.
fn money(tx: &Py, key: &str) -> Result<(PriceNumber, Option<String>), AppError> {
    let value = at(tx, key);
    if !matches!(value, Py::Dict(_)) {
        return Ok((PriceNumber::parse("0")?, None));
    }
    let amount = decimal(at(value, "amount"), "0")?;
    let currency = at(value, "currency");
    Ok((amount, truthy(currency).then(|| asset(currency))))
}

/// `normalize_coinbase_transaction(tx, currency)`.
fn normalize(tx: &Py, currency: &str) -> Result<Option<Py>, AppError> {
    let kind = at(tx, "type");
    let kind = lower(&kassiber_pytext::strip_py(&if truthy(kind) {
        text(kind)
    } else {
        "".into()
    }));
    let (amount, _) = money(tx, "amount")?;
    if is_zero(&amount)? {
        return Ok(None);
    }
    let (native, native_currency) = money(tx, "native_amount")?;
    let occurred_at = str_or_none(either(tx, &["created_at", "createdAt"]));
    let id = at(tx, "id");
    let id = if truthy(id) { text(id) } else { "".into() };
    let external_id = if id.code_points().next().is_some() {
        concat(&["coinbase:".into(), id.clone()])
    } else {
        concat(&[
            "coinbase:".into(),
            occurred_at.clone().unwrap_or_else(|| "None".into()),
            format!(":{currency}:{}", repr(&amount)).into(),
        ])
    };
    let raw = tx.clone();
    let token = kind.as_str().unwrap_or("");
    let entry = |external_id, inbound, kind, description, pricing| Entry {
        provider: "Coinbase",
        external_id,
        occurred_at: occurred_at.clone(),
        inbound,
        asset: currency.to_owned(),
        amount: amount.clone(),
        fee: PriceNumber::Finite(Decimal::zero()),
        kind,
        raw: raw.clone(),
        description,
        pricing,
    };
    if TRADES.contains(&token) {
        let usable = match &native_currency {
            Some(code) if is_fiat(code) => !less_than_cent(&abs(native.clone())?)?,
            _ => false,
        };
        if !usable {
            return Err(
                AppError::new("validation", "").with_python_message(concat(&[
                    "Coinbase BTC trade '".into(),
                    id,
                    "' does not include usable fiat execution value".into(),
                ])),
            );
        }
        let inbound = positive(&amount)?;
        let detail = at(tx, token);
        let commission = if matches!(detail, Py::Dict(_)) {
            decimal(at(detail, "commission"), "0")?
        } else {
            PriceNumber::parse("0")?
        };
        let mut fiat_value = abs(native)?;
        if inbound && truth(&commission) {
            fiat_value = arithmetic(&fiat_value, &abs(commission)?, '+')?;
        }
        return record(entry(
            external_id,
            inbound,
            if inbound { "buy" } else { "sell" },
            None,
            Some(Pricing {
                fiat_currency: native_currency.expect("fiat currency"),
                fiat_value,
                fiat_rate: None,
                method: "coinbase_api",
                external_ref: id,
            }),
        ))
        .map(Some);
    }
    let description = || Some(format!("Coinbase {}", token.replace('_', " ")).into());
    if INCOME.contains(&token) {
        let kind = if token == "interest" {
            "interest"
        } else {
            "staking"
        };
        return record(entry(external_id, true, kind, description(), None)).map(Some);
    }
    if MOVEMENTS.contains(&token) {
        let network = at(tx, "network");
        let external_id = match network {
            Py::Dict(_) => str_or_none(at(network, "hash")),
            _ => None,
        }
        .unwrap_or(external_id);
        let inbound = positive(&amount)?;
        let kind = if inbound { "deposit" } else { "withdrawal" };
        return record(entry(external_id, inbound, kind, description(), None)).map(Some);
    }
    Err(AppError::new("validation", "")
        .with_python_message(concat(&[
            "Coinbase BTC transaction '".into(),
            id,
            "' has unsupported type '".into(),
            kind,
            "'".into(),
        ]))
        .with_hint(
            "Unsupported Coinbase BTC rows are not guessed; export them through the generic ledger with explicit semantics.",
        ))
}

/// `value < Decimal("0.01")`: comparing a NaN raises.
fn less_than_cent(value: &PriceNumber) -> Result<bool, AppError> {
    match value {
        _ if nan(value) => Err(decimal_error(DecimalError::InvalidOperation)),
        PriceNumber::Finite(v) => Ok(*v < "0.01".parse::<Decimal>().expect("decimal")),
        PriceNumber::Special(v) => Ok(v.starts_with('-')),
    }
}
