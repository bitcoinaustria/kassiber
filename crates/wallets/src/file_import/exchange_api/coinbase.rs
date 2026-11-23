//! `fetch_coinbase_records`: signed v2 account and transaction pages, then
//! `normalize_coinbase_records`, extended to other crypto assets
//! (docs/plan/24-crypto-assets.md).
//!
//! The accounts list names each account's currency and Coinbase's own type
//! for it (`crypto` or `fiat`). Bitcoin accounts are always read. Other crypto
//! accounts are counted, not read, in the `bitcoin` scope and read in the
//! `crypto` scope. Fiat accounts are never read: a buy or sell carries its
//! fiat value on the crypto side.
use super::*;
use crate::file_import::assets::{self, Class, Scope};
use kassiber_net::HttpUrl;
use std::collections::BTreeSet;

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

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "coinbase",
    provider: "Coinbase",
    reads: contract::Reads::AllAssets,
    default_url: "https://api.coinbase.com",
    key_only: false,
    fetch,
    probe,
    skipped: None,
    files: &[],
    shared_ids: true,
    same_second_tags: false,
    after_insert: None,
};

/// Every crypto account's transactions, in every asset scope; the scope only
/// filters which are kept. Fiat accounts are never read (a buy or sell
/// carries its fiat value on the crypto side), and an account in an asset
/// no scope keeps is counted, not read.
fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    let mut budget = Budget::default();
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
        if canonical.is_empty() {
            // Its asset is unknown: never Bitcoin the filter could keep in
            // the bitcoin scope; refused in the crypto scope.
            if mapper.scope() == Scope::Crypto {
                return Err(AppError::new(
                    "protocol_error",
                    "Coinbase listed an account without a currency",
                ));
            }
            continue;
        }
        if matches!(class, Class::Fiat | Class::Excluded) {
            mapper.admit_classed(&[(&canonical, class)]);
            continue;
        }
        if id.code_points().next().is_none() {
            if class == Class::Bitcoin {
                continue;
            }
            return Err(AppError::new(
                "protocol_error",
                format!("Coinbase listed an account in {canonical} without an id"),
            ));
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
    map(&read, mapper)?;
    Ok(None)
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
pub(super) fn records(accounts: &[(PyStr, Vec<Py>)], mapper: &mut Mapper) -> Result<(), AppError> {
    let accounts: Vec<Account> = accounts
        .iter()
        .map(|(currency, rows)| Account {
            currency: currency.clone(),
            class: assets::class(&asset(&Py::Str(currency.clone()))),
            rows: rows.clone(),
        })
        .collect();
    map(&accounts, mapper)
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

/// Every read account's rows as events, in order. Swap legs are paired
/// before the scope filter: in the `bitcoin` scope only BTC rows and the rows
/// that share a trade or order with one, so another coin's trades never stop
/// a Bitcoin wallet; a BTC leg of a crypto-to-crypto trade is refused there
/// with the command that sets the `crypto` scope.
pub(super) fn map(accounts: &[Account], mapper: &mut Mapper) -> Result<(), AppError> {
    let mut rows = vec![];
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
            if currency.is_empty() {
                continue;
            }
            let class = if currency == account_code {
                account.class
            } else {
                assets::class(&currency)
            };
            rows.push((Row { tx, currency }, class));
        }
    }
    let scope = mapper.scope();
    let pairable: Vec<usize> = match scope {
        Scope::Crypto => (0..rows.len())
            .filter(|&i| matches!(rows[i].1, Class::Bitcoin | Class::Crypto))
            .collect(),
        Scope::Bitcoin => {
            let btc: BTreeSet<String> = rows
                .iter()
                .filter(|(_, class)| *class == Class::Bitcoin)
                .filter_map(|(row, _)| trade_key(row))
                .collect();
            (0..rows.len())
                .filter(|&i| {
                    rows[i].1 == Class::Bitcoin
                        || (rows[i].1 == Class::Crypto
                            && trade_key(&rows[i].0).is_some_and(|key| btc.contains(&key)))
                })
                .collect()
        }
    };
    let selected: Vec<Row> = pairable
        .iter()
        .map(|&i| Row {
            tx: rows[i].0.tx,
            currency: rows[i].0.currency.clone(),
        })
        .collect();
    let legs: BTreeMap<usize, Leg> = swaps(&selected)?
        .into_iter()
        .map(|(at, leg)| (pairable[at], leg))
        .collect();
    let mut fees = BTreeMap::new();
    for (index, (row, class)) in rows.iter().enumerate() {
        if !mapper.admit_classed(&[(&row.currency, *class)]) {
            // The bitcoin scope leaves another asset's buy or sell out, but a
            // fee it paid in BTC is Bitcoin activity: the same fee row the
            // crypto scope books, never dropped.
            if scope == Scope::Bitcoin && !legs.contains_key(&index) && btc_fee(row)? {
                if let Some(fee) = third_asset_fee(row, None, Scope::Crypto, &mut fees)? {
                    mapper.push(fee)?;
                }
            }
            continue;
        }
        let leg = legs.get(&index);
        if scope == Scope::Bitcoin {
            if let Some(leg) = leg {
                crypto_trade(row, &leg.other, &mapper.keep_command())?;
            }
        }
        let event = match leg {
            Some(leg) => Some(swap(row, leg)?),
            None => normalize(row.tx, &row.currency)?,
        };
        let Some(event) = event else {
            continue;
        };
        mapper.push(event)?;
        if let Some(fee) = third_asset_fee(row, leg, scope, &mut fees)? {
            mapper.push(fee)?;
        }
    }
    Ok(())
}

/// The convert or crypto-quoted fill a row may be one leg of.
fn trade_key(row: &Row<'_>) -> Option<String> {
    match kind(row.tx).as_str().unwrap_or("") {
        "trade" => Some(format!("trade:{}", detail_text(row.tx, "trade", "id"))),
        "advanced_trade_fill" => Some(format!(
            "order:{}",
            detail_text(row.tx, "advanced_trade_fill", "order_id")
        )),
        _ => None,
    }
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
    let mut seen = BTreeSet::new();
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
            if indexes.len() <= 2 {
                pair(rows, &indexes, reference, &name, &mut legs)?;
                continue;
            }
            // Several fills at one time and price: every fill keeps that
            // price, so ordered by size the n-th outgoing leg is the n-th
            // incoming leg's other side (fills of equal size are alike).
            let (mut sent, mut received) = (vec![], vec![]);
            for &index in &indexes {
                let (amount, _) = money(rows[index].tx, "amount")?;
                let PriceNumber::Finite(value) = amount else {
                    return Err(AppError::new(
                        "validation",
                        format!("{name} has a leg without an exact amount"),
                    ));
                };
                if value.is_sign_negative() {
                    sent.push((value.copy_abs(), index));
                } else {
                    received.push((value, index));
                }
            }
            if sent.len() != received.len() {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "{name} has {} outgoing and {} incoming legs",
                        sent.len(),
                        received.len()
                    ),
                )
                .with_hint("Kassiber pairs a crypto-to-crypto trade only from the two legs Coinbase reports for it; check the trade in Coinbase."));
            }
            sent.sort();
            received.sort();
            for (n, ((_, out), (_, into))) in sent.iter().zip(&received).enumerate() {
                pair(
                    rows,
                    &[*out, *into],
                    format!("{reference}:{n}"),
                    &name,
                    &mut legs,
                )?;
            }
        }
    }
    Ok(legs)
}

/// In the `bitcoin` scope, refuses the BTC leg of a crypto-to-crypto trade:
/// booked alone it would be a fiat-priced sale or purchase, not the swap it
/// is; the `crypto` scope imports both legs.
fn crypto_trade(row: &Row<'_>, other: &str, keep: &str) -> Result<(), AppError> {
    let token = kind(row.tx);
    let what = match token.as_str().unwrap_or("") {
        "trade" => "convert",
        _ => "fill",
    };
    let (id, ..) = identity(row.tx, &row.currency, &PriceNumber::parse("0")?);
    Err(AppError::new(
        "validation",
        format!(
            "Coinbase {} {what} '{}' trades against {other}, which the bitcoin asset scope does not import",
            label(&row.currency),
            lossy(&id)
        ),
    )
    .with_hint(format!(
        "Kassiber does not book one leg of a swap as a sale or purchase. To import both legs, run {keep} and sync again."
    )))
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
fn swap(row: &Row<'_>, leg: &Leg) -> Result<Event, AppError> {
    let tx = row.tx;
    let (amount, _) = money(tx, "amount")?;
    let (native, native_currency) = money(tx, "native_amount")?;
    let native_currency = native_currency.map(fiat_code);
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
    let inbound = positive(&amount)?;
    let kind = Kind::Swap(leg.reference.as_str().into());
    Ok(Event {
        occurred_at,
        raw: tx.clone(),
        description: Some(description.into()),
        pricing,
        ..Event::one(external_id, kind, inbound, row.currency.clone(), amount)
    })
}

/// Whether a buy, sell or convert states its fee in BTC (or LBTC).
fn btc_fee(row: &Row<'_>) -> Result<bool, AppError> {
    let token = kind(row.tx);
    let token = token.as_str().unwrap_or("");
    if !["buy", "sell", "trade"].contains(&token) {
        return Ok(false);
    }
    let (fee, currency) = money(at(row.tx, token), "fee")?;
    Ok(currency.is_some_and(|code| kassiber_store::assets::is_bitcoin(&code)) && !is_zero(&fee)?)
}

/// A buy, sell or convert whose `fee` Coinbase states in a crypto asset that
/// is neither of the trade's assets: its own outbound `fee` row in the crypto
/// scope, once per trade. The Bitcoin scope refuses it rather than drop it.
fn third_asset_fee(
    row: &Row<'_>,
    leg: Option<&Leg>,
    scope: Scope,
    seen: &mut BTreeMap<String, String>,
) -> Result<Option<Event>, AppError> {
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
    // A BTC fee must be whole msat: the record builder's exactness check
    // skips Bitcoin, and the coordinator would round it.
    if kassiber_store::assets::is_bitcoin(&currency) {
        if let PriceNumber::Finite(value) = &fee {
            if crate::file_import::assets::exact_units("BTC", "fee", value).is_err() {
                return Err(AppError::new(
                    "validation",
                    format!(
                        "Coinbase {token} '{}' pays a BTC fee of {}, finer than 1 msat",
                        lossy(&id),
                        repr(&fee)
                    ),
                ));
            }
        }
    }
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
    let external_id: PyStr = format!("{reference}:fee").into();
    Ok(Some(Event {
        occurred_at,
        raw: row.tx.clone(),
        description: Some(format!("Coinbase fee for {reference}").into()),
        ..Event::one(external_id, Kind::Fee, false, currency, fee)
    }))
}

/// `normalize_coinbase_transaction(tx, currency)`, for any kept asset.
fn normalize(tx: &Py, currency: &str) -> Result<Option<Event>, AppError> {
    let kind = kind(tx);
    let (amount, _) = money(tx, "amount")?;
    if is_zero(&amount)? {
        return Ok(None);
    }
    let (native, native_currency) = money(tx, "native_amount")?;
    let native_currency = native_currency.map(fiat_code);
    let (id, external_id, occurred_at) = identity(tx, currency, &amount);
    let raw = tx.clone();
    let token = kind.as_str().unwrap_or("");
    let name = label(currency);
    let event = |external_id, kind, inbound, description, pricing| Event {
        occurred_at: occurred_at.clone(),
        raw: raw.clone(),
        description,
        pricing,
        ..Event::one(
            external_id,
            kind,
            inbound,
            currency.to_owned(),
            amount.clone(),
        )
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
        return Ok(Some(event(
            external_id,
            Kind::Trade,
            inbound,
            None,
            Some(Pricing {
                fiat_currency: native_currency.expect("fiat currency"),
                fiat_value,
                fiat_rate: None,
                method: "coinbase_api",
                external_ref: id,
            }),
        )));
    }
    let description = || Some(format!("Coinbase {}", token.replace('_', " ")).into());
    if INCOME.contains(&token) {
        let kind = if token == "interest" {
            "interest"
        } else {
            "staking"
        };
        let kind = Kind::Movement(kind);
        return Ok(Some(event(external_id, kind, true, description(), None)));
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
        let kind = Kind::Movement(if inbound { "deposit" } else { "withdrawal" });
        return Ok(Some(event(external_id, kind, inbound, description(), None)));
    }
    let hint = if is_bitcoin(currency) {
        "Unsupported Coinbase BTC rows are not guessed; export them through the generic ledger with explicit semantics."
    } else {
        "Unsupported Coinbase rows are not guessed; record them yourself with the generic CSV/JSON import with explicit semantics."
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
