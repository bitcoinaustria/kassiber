//! `fetch_coinbase_records`: signed v2 account and transaction pages, then
//! `normalize_coinbase_records`, extended to other crypto assets
//! (docs/plan/23-crypto-assets.md).
//!
//! The accounts list names each account's currency and Coinbase's own type
//! for it (`crypto` or `fiat`). Bitcoin accounts are always read. Other crypto
//! accounts are counted, not read, in the `bitcoin` scope and read in the
//! `crypto` scope. Fiat accounts are never read: a buy or sell carries its
//! fiat value on the crypto side.
use super::*;
use crate::file_import::assets::{self, Class, Scope, Tally};
use kassiber_net::HttpUrl;

const TRADES: [&str; 4] = ["buy", "sell", "trade", "advanced_trade_fill"];
const MOVEMENTS: [&str; 7] = [
    "send",
    "receive",
    "exchange_deposit",
    "exchange_withdrawal",
    "pro_deposit",
    "pro_withdrawal",
    "prime_withdrawal",
];
const INCOME: [&str; 3] = ["interest", "staking_reward", "inflation_reward"];

/// One account whose transactions were read.
pub(super) struct Account {
    /// The currency code as Coinbase spells it.
    pub currency: PyStr,
    /// What Coinbase's own currency type makes it.
    pub class: Class,
    pub rows: Vec<Py>,
}

pub(super) fn fetch(client: &Client<'_>) -> Result<Fetched, AppError> {
    let mut budget = Budget::default();
    let mut tally = Tally::default();
    let accounts = paginated(client, "/v2/accounts", &mut budget)?;
    let mut read = vec![];
    for account in &accounts {
        if !matches!(account, Py::Dict(_)) {
            continue;
        }
        let currency = at(account, "currency");
        let code = match currency {
            Py::Dict(_) => at(currency, "code"),
            other => other,
        };
        let canonical = asset(code);
        let class = account_class(currency, &canonical);
        let id = at(account, "id");
        let id = if truthy(id) { text(id) } else { "".into() };
        if class != Class::Bitcoin {
            if canonical.is_empty() {
                if client.scope == Scope::Crypto {
                    return Err(AppError::new(
                        "protocol_error",
                        "Coinbase listed an account without a currency",
                    ));
                }
                continue;
            }
            // The accounts list names the currency, so an account the scope
            // leaves out is counted once and its history is never requested.
            if !tally.admit(client.scope, &canonical, class) {
                continue;
            }
            if id.code_points().next().is_none() {
                return Err(AppError::new(
                    "protocol_error",
                    format!("Coinbase listed an account in {canonical} without an id"),
                ));
            }
        }
        if id.code_points().next().is_none() {
            continue;
        }
        let path = format!("/v2/accounts/{}/transactions", quote(&id)?);
        let rows = paginated(client, &path, &mut budget)?;
        // str(item.get("currency") or ...): the code was truthy to get here.
        read.push(Account {
            currency: text(code),
            class,
            rows,
        });
    }
    let records = map(&read, client.scope, &mut tally)?;
    Ok(Fetched {
        records,
        skipped: None,
        tally: Some(tally),
        reported: None,
    })
}

/// An account's class from Coinbase's own currency type: `crypto` or `fiat`,
/// and anything else (a security or other holding) excluded. BTC is always
/// Bitcoin; a currency without a type falls back to its code.
fn account_class(currency: &Py, code: &str) -> Class {
    if is_bitcoin(code) {
        return Class::Bitcoin;
    }
    let kind = match currency {
        Py::Dict(_) => str_or_none(at(currency, "type")),
        _ => None,
    };
    match kind.map(|kind| kassiber_pytext::lower(&lossy(&kind))) {
        None => assets::class(code),
        Some(kind) if kind == "crypto" => Class::Crypto,
        Some(kind) if kind == "fiat" => Class::Fiat,
        Some(_) => Class::Excluded,
    }
}

/// `normalize_coinbase_records` over `{"currency", "transactions"}` items:
/// the Bitcoin-scope mapping the Python reference vectors pin.
#[cfg(test)]
pub(super) fn records(accounts: &[(PyStr, Vec<Py>)]) -> Result<Vec<Py>, AppError> {
    let accounts: Vec<Account> = accounts
        .iter()
        .map(|(currency, rows)| Account {
            currency: currency.clone(),
            class: assets::class(&asset(&Py::Str(currency.clone()))),
            rows: rows.clone(),
        })
        .collect();
    map(&accounts, Scope::Bitcoin, &mut Tally::default())
}

/// One kept row and its canonical asset.
struct Row<'a> {
    tx: &'a Py,
    currency: String,
}

/// The other side of a crypto-to-crypto trade, from the leg's view.
struct Leg {
    /// The `swap_ref` both legs share.
    reference: String,
    /// The other leg's asset.
    other: String,
}

/// The records of every read account's rows, in order. A row whose asset
/// `scope` does not keep is counted in `tally` instead.
pub(super) fn map(
    accounts: &[Account],
    scope: Scope,
    tally: &mut Tally,
) -> Result<Vec<Py>, AppError> {
    let mut kept = vec![];
    for account in accounts {
        let account_code = asset(&Py::Str(account.currency.clone()));
        for tx in account.rows.iter().filter(|tx| matches!(tx, Py::Dict(_))) {
            let amount = at(tx, "amount");
            let currency = match amount {
                Py::Dict(_) => at(amount, "currency"),
                _ => &Py::Null,
            };
            let currency = if truthy(currency) {
                asset(currency)
            } else {
                account_code.clone()
            };
            // Only an account without a currency, which the crypto scope
            // refuses before reading, gives a row no asset.
            if currency.is_empty() {
                continue;
            }
            let class = if currency == account_code {
                account.class
            } else {
                assets::class(&currency)
            };
            if tally.admit(scope, &currency, class) {
                kept.push(Row { tx, currency });
            }
        }
    }
    let legs = match scope {
        Scope::Crypto => swaps(&kept)?,
        Scope::Bitcoin => BTreeMap::new(),
    };
    let mut records = vec![];
    let mut fees = BTreeMap::new();
    for (index, row) in kept.iter().enumerate() {
        let leg = legs.get(&index);
        let record = match leg {
            Some(leg) => Some(swap(row, leg)?),
            None => normalize(row.tx, &row.currency)?,
        };
        let Some(record) = record else {
            continue;
        };
        records.push(record);
        if let Some(fee) = third_asset_fee(row, leg, scope, &mut fees)? {
            records.push(fee);
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

/// A row's lower-cased, stripped `type`.
fn kind(tx: &Py) -> PyStr {
    let kind = at(tx, "type");
    lower(&kassiber_pytext::strip_py(&if truthy(kind) {
        text(kind)
    } else {
        "".into()
    }))
}

/// `tx[detail][field]` as text, or empty.
fn detail_text(tx: &Py, detail: &str, field: &str) -> String {
    match at(tx, detail) {
        detail @ Py::Dict(_) => str_or_none(at(detail, field))
            .map(|value| lossy(&value))
            .unwrap_or_default(),
        _ => String::new(),
    }
}

/// How messages name a row's asset: `BTC` for Bitcoin, as Python did.
fn label(currency: &str) -> &str {
    if is_bitcoin(currency) {
        "BTC"
    } else {
        currency
    }
}

/// The row's id, external id and time.
fn identity(tx: &Py, currency: &str, amount: &PriceNumber) -> (PyStr, PyStr, Option<PyStr>) {
    let occurred_at = str_or_none(either(tx, &["created_at", "createdAt"]));
    let id = at(tx, "id");
    let id = if truthy(id) { text(id) } else { "".into() };
    let external_id = if id.code_points().next().is_some() {
        concat(&["coinbase:".into(), id.clone()])
    } else {
        concat(&[
            "coinbase:".into(),
            occurred_at.clone().unwrap_or_else(|| "None".into()),
            format!(":{currency}:{}", repr(amount)).into(),
        ])
    };
    (id, external_id, occurred_at)
}

/// The crypto-to-crypto trades among the kept rows, by row index. A convert
/// (`trade`) shows up in both accounts with the same `trade.id`; one whose
/// other side is a fiat account has one leg here and stays a buy or sell. An
/// Advanced Trade fill on a crypto-quoted product shows up in both accounts
/// with the same `order_id`; an order with several fills pairs each fill's
/// legs by their shared time and fill price.
fn swaps(rows: &[Row<'_>]) -> Result<BTreeMap<usize, Leg>, AppError> {
    let mut trades: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut fills: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (index, row) in rows.iter().enumerate() {
        let (id, ..) = identity(row.tx, &row.currency, &PriceNumber::parse("0")?);
        let name =
            |what: &str| format!("Coinbase {} {what} '{}'", label(&row.currency), lossy(&id));
        match kind(row.tx).as_str().unwrap_or("") {
            "trade" => {
                let trade = detail_text(row.tx, "trade", "id");
                if trade.is_empty() {
                    return Err(AppError::new(
                        "validation",
                        format!("{} has no trade id to find its other leg", name("trade")),
                    ));
                }
                trades.entry(trade).or_default().push(index);
            }
            "advanced_trade_fill" => {
                let product = detail_text(row.tx, "advanced_trade_fill", "product_id");
                let pair = product
                    .split_once('-')
                    .map(|(base, quote)| (asset(&s(base)), asset(&s(quote))));
                let Some((base, quote)) = pair.filter(|(base, quote)| {
                    !base.is_empty() && !quote.is_empty() && [base, quote].contains(&&row.currency)
                }) else {
                    return Err(AppError::new(
                        "validation",
                        format!(
                            "{} names product '{product}', which is not a pair of its asset",
                            name("fill")
                        ),
                    ));
                };
                // Against fiat it is a buy or a sell.
                if [&base, &quote]
                    .iter()
                    .any(|code| assets::class(code) == Class::Fiat)
                {
                    continue;
                }
                let order = detail_text(row.tx, "advanced_trade_fill", "order_id");
                if order.is_empty() {
                    return Err(AppError::new(
                        "validation",
                        format!("{} has no order id to find its other leg", name("fill")),
                    ));
                }
                fills.entry(order).or_default().push(index);
            }
            _ => {}
        }
    }
    let mut legs = BTreeMap::new();
    for (trade, indexes) in trades {
        if indexes.len() > 1 {
            let name = format!("Coinbase trade '{trade}'");
            pair(
                rows,
                &indexes,
                format!("coinbase:trade:{trade}"),
                &name,
                &mut legs,
            )?;
        }
    }
    for (order, indexes) in fills {
        let name = format!("Coinbase order '{order}'");
        if indexes.len() == 2 {
            pair(
                rows,
                &indexes,
                format!("coinbase:order:{order}"),
                &name,
                &mut legs,
            )?;
            continue;
        }
        let mut by_fill: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
        for index in indexes {
            let tx = rows[index].tx;
            let time = str_or_none(either(tx, &["created_at", "createdAt"]))
                .map(|value| lossy(&value))
                .unwrap_or_default();
            let price = detail_text(tx, "advanced_trade_fill", "fill_price");
            by_fill.entry((time, price)).or_default().push(index);
        }
        for (fill, ((time, price), indexes)) in by_fill.into_iter().enumerate() {
            let name = format!("{name} fill at {time} for {price}");
            let reference = format!("coinbase:order:{order}:{fill}");
            pair(rows, &indexes, reference, &name, &mut legs)?;
        }
    }
    Ok(legs)
}

/// Records the two legs of one crypto-to-crypto trade: one out, one in, in
/// two different assets, or the trade is refused.
fn pair(
    rows: &[Row<'_>],
    indexes: &[usize],
    reference: String,
    name: &str,
    legs: &mut BTreeMap<usize, Leg>,
) -> Result<(), AppError> {
    let hint = "Kassiber pairs a crypto-to-crypto trade only from the two legs Coinbase reports for it; check the trade in Coinbase.";
    let &[a, b] = indexes else {
        return Err(AppError::new(
            "validation",
            format!(
                "{name} needs one leg in each of two crypto accounts and has {}",
                indexes.len()
            ),
        )
        .with_hint(hint));
    };
    let sign = |index: usize| -> Result<Option<bool>, AppError> {
        let (amount, _) = money(rows[index].tx, "amount")?;
        Ok((!is_zero(&amount)?).then_some(positive(&amount)?))
    };
    let (Some(first), Some(second)) = (sign(a)?, sign(b)?) else {
        return Err(
            AppError::new("validation", format!("{name} has a leg without an amount"))
                .with_hint(hint),
        );
    };
    if first == second {
        return Err(AppError::new(
            "validation",
            format!("{name} does not have one leg out and one leg in"),
        )
        .with_hint(hint));
    }
    if rows[a].currency == rows[b].currency {
        return Err(AppError::new(
            "validation",
            format!("{name} has both legs in {}", rows[a].currency),
        )
        .with_hint(hint));
    }
    for (this, other) in [(a, b), (b, a)] {
        legs.insert(
            this,
            Leg {
                reference: reference.clone(),
                other: rows[other].currency.clone(),
            },
        );
    }
    Ok(())
}

/// One leg of a crypto-to-crypto trade: its own amount, valued at its fiat
/// `native_amount` when that is usable. Coinbase's fee for the trade is
/// already inside the two amounts.
fn swap(row: &Row<'_>, leg: &Leg) -> Result<Py, AppError> {
    let tx = row.tx;
    let (amount, _) = money(tx, "amount")?;
    let (native, native_currency) = money(tx, "native_amount")?;
    let (_, external_id, occurred_at) = identity(tx, &row.currency, &amount);
    let pricing = match native_currency {
        Some(code) if is_fiat(&code) && !less_than_cent(&abs(native.clone())?)? => Some(Pricing {
            fiat_currency: code,
            fiat_value: abs(native)?,
            fiat_rate: None,
            method: "coinbase_api",
            external_ref: leg.reference.as_str().into(),
        }),
        _ => None,
    };
    let token = kind(tx);
    let description = format!("Coinbase {}", lossy(&token).replace('_', " "));
    swap_leg(
        Entry {
            provider: "Coinbase",
            external_id,
            occurred_at,
            inbound: positive(&amount)?,
            asset: row.currency.clone(),
            amount,
            fee: PriceNumber::Finite(Decimal::zero()),
            kind: "swap",
            raw: tx.clone(),
            description: Some(description.into()),
            pricing,
        },
        &leg.reference.as_str().into(),
    )
}

/// A buy, sell or convert whose `fee` Coinbase states in a crypto asset that
/// is neither of the trade's assets: its own outbound `fee` row in the crypto
/// scope, once per trade. The Bitcoin scope refuses it rather than drop it.
fn third_asset_fee(
    row: &Row<'_>,
    leg: Option<&Leg>,
    scope: Scope,
    seen: &mut BTreeMap<String, String>,
) -> Result<Option<Py>, AppError> {
    let token = kind(row.tx);
    let token = token.as_str().unwrap_or("");
    if !["buy", "sell", "trade"].contains(&token) {
        return Ok(None);
    }
    let (fee, Some(currency)) = money(at(row.tx, token), "fee")? else {
        return Ok(None);
    };
    if is_zero(&fee)?
        || is_fiat(&currency)
        || currency == row.currency
        || leg.is_some_and(|leg| leg.other == currency)
    {
        return Ok(None);
    }
    let (id, _, occurred_at) = identity(row.tx, &row.currency, &fee);
    if scope == Scope::Bitcoin {
        return Err(AppError::new(
            "validation",
            format!(
                "Coinbase BTC {token} '{}' pays its fee in {currency}",
                lossy(&id)
            ),
        )
        .with_hint("In the bitcoin asset scope Kassiber does not drop a fee paid in another crypto asset; set the wallet's asset_scope to crypto to import it as its own fee row."));
    }
    let reference = match leg {
        Some(leg) => leg.reference.clone(),
        None => match detail_text(row.tx, token, "id") {
            detail if detail.is_empty() => format!("coinbase:{}", lossy(&id)),
            detail => format!("coinbase:{token}:{detail}"),
        },
    };
    let stated = format!("{} {currency}", repr(&abs(fee.clone())?));
    match seen.get(&reference) {
        Some(previous) if *previous == stated => return Ok(None),
        Some(previous) => {
            return Err(AppError::new(
                "validation",
                format!("Coinbase {reference} states two different fees: {previous} and {stated}"),
            ))
        }
        None => {
            seen.insert(reference.clone(), stated);
        }
    }
    record(Entry {
        provider: "Coinbase",
        external_id: format!("{reference}:fee").into(),
        occurred_at,
        inbound: false,
        asset: currency,
        amount: fee,
        fee: PriceNumber::Finite(Decimal::zero()),
        kind: "fee",
        raw: row.tx.clone(),
        description: Some(format!("Coinbase fee for {reference}").into()),
        pricing: None,
    })
    .map(Some)
}

/// `normalize_coinbase_transaction(tx, currency)`, for any kept asset.
fn normalize(tx: &Py, currency: &str) -> Result<Option<Py>, AppError> {
    let kind = kind(tx);
    let (amount, _) = money(tx, "amount")?;
    if is_zero(&amount)? {
        return Ok(None);
    }
    let (native, native_currency) = money(tx, "native_amount")?;
    let (id, external_id, occurred_at) = identity(tx, currency, &amount);
    let raw = tx.clone();
    let token = kind.as_str().unwrap_or("");
    let name = label(currency);
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
                    format!("Coinbase {name} trade '").into(),
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
        let inbound = positive(&amount)?;
        // A send's sign says which way it went; a receive only comes in.
        if token == "receive" && !inbound {
            return Err(
                AppError::new("validation", "").with_python_message(concat(&[
                    format!("Coinbase {name} receive '").into(),
                    id,
                    "' has a negative amount".into(),
                ])),
            );
        }
        let network = at(tx, "network");
        let external_id = match network {
            Py::Dict(_) => str_or_none(at(network, "hash")),
            _ => None,
        }
        .unwrap_or(external_id);
        let kind = if inbound { "deposit" } else { "withdrawal" };
        return record(entry(external_id, inbound, kind, description(), None)).map(Some);
    }
    let hint = if is_bitcoin(currency) {
        "Unsupported Coinbase BTC rows are not guessed; export them through the generic ledger with explicit semantics."
    } else {
        "Unsupported Coinbase rows are not guessed; export them through the generic ledger with explicit semantics."
    };
    Err(AppError::new("validation", "")
        .with_python_message(concat(&[
            format!("Coinbase {name} transaction '").into(),
            id,
            "' has unsupported type '".into(),
            kind,
            "'".into(),
        ]))
        .with_hint(hint))
}

/// `value < Decimal("0.01")`: comparing a NaN raises.
fn less_than_cent(value: &PriceNumber) -> Result<bool, AppError> {
    match value {
        _ if nan(value) => Err(decimal_error(DecimalError::InvalidOperation)),
        PriceNumber::Finite(v) => Ok(*v < "0.01".parse::<Decimal>().expect("decimal")),
        PriceNumber::Special(v) => Ok(v.starts_with('-')),
    }
}
