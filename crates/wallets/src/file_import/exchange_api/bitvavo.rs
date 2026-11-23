//! Bitvavo, Rust only: the signed REST API (`api.bitvavo.com/v2`) and the
//! transaction-history CSV export share one mapping, so a transaction in
//! both gets the same record and id (`bitvavo:<transactionId>`). Reads only
//! `/account/history`, every page, then `/balance?symbol=BTC`. Bitvavo does
//! not reveal a key's permissions; Kassiber never calls a write endpoint.
//! The wallet's asset scope decides which assets are kept
//! (docs/plan/24-crypto-assets.md); a row Kassiber cannot book exactly, and
//! every type it does not map, refuses the import by name.
//! docs/reference/bitvavo.md.
use super::*;
use assets::Scope;
use std::ops::RangeInclusive;

mod export;

const PROVIDER: &str = "Bitvavo";
/// `maxItems`' documented maximum.
const PAGE_SIZE: usize = 100;
/// A key Bitvavo refuses: wrong, inactive, IP-restricted or without View access.
const REFUSED: &[u16] = &[401, 403];
/// `Bitvavo-Access-Window`, Bitvavo's default (milliseconds).
const WINDOW: &str = "10000";
/// How far a trade's quote amount may sit from amount × price (± the fee):
/// Bitvavo rounds the CSV's paid/received amount to cents.
const TOLERANCE: &str = "0.01";
/// What `/balance` reports: `available` plus `inOrder` for BTC.
pub(super) const BALANCE_SCOPE: &str = "spot_available_plus_in_order";
/// Bitvavo's documented error codes (<https://docs.bitvavo.com/docs/errors/>).
const ERROR_CODES: [RangeInclusive<i64>; 4] = [101..=112, 200..=240, 300..=317, 400..=430];
const REVIEW: &str = "Kassiber imports buys and sells for fiat, swaps, deposits and withdrawals, and staking and affiliate rewards from Bitvavo; record this transaction yourself (generic ledger import) after checking it in Bitvavo.";
const SWAP_FEE: &str = "Kassiber places a swap's fee only where Bitvavo's transaction-history CSV export states it, in the quote asset of a trade; record this transaction yourself (generic ledger import).";
/// The API does not place such a fee, and every sync reads the whole history
/// again, so it would refuse the same trade on every run.
const API_SWAP_FEE: &str = "Bitvavo's API does not say whether this swap's amounts include its fee, and every sync reads the whole history again, so this wallet cannot be synced while it holds this trade. Import Bitvavo's transaction-history CSV export into it instead, which states the fee, and import a newer export for later history.";
/// A fiat trade whose cash amount fits both fee readings (a fee under two
/// cents) has no execution value Kassiber can know. Stored unpriced,
/// `journals process` would value it from the rates cache, so the sync
/// refuses it, as it does an API swap with a fee.
const API_TRADE_FEE: &str = "Bitvavo's API does not say whether this trade's cash amount includes its fee, and with a fee this small both readings fit, so Kassiber cannot know what it cost or earned; every sync reads the whole history again, so this wallet cannot be synced while it holds this trade. Import Bitvavo's transaction-history CSV export into it instead, which states the fee, and import a newer export for later history.";

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "bitvavo",
    provider: PROVIDER,
    reads: contract::Reads::AllAssets,
    default_url: "https://api.bitvavo.com",
    key_only: false,
    fetch,
    probe,
    skipped: Some(&["fiat_only_transactions", "non_btc_transactions"]),
    files: &[contract::registry::FileFormat {
        format: Format::Bitvavo,
        reads: contract::Reads::AllAssets,
        load: export::load,
    }],
    shared_ids: true,
    same_second_tags: true,
    after_insert: None,
    before_insert: None,
    after_sync: None,
};

fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    // One fixed end for every page, so the page count cannot shift while a
    // sync reads. Without a range Bitvavo returns only recent history.
    let until = now()?.as_millis().to_string();
    let events = history(client, &until)?
        .iter()
        .map(event)
        .collect::<Result<Vec<_>, _>>()?;
    map(events, "bitvavo_api", FeeReading::Unknown, mapper)?;
    Ok(Some(balance(client)?))
}

/// The desktop key check: the BTC balance and one history row, the two reads
/// the sync makes. Bitvavo does not tell whether the key could also trade.
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    balance(client)?;
    let until = now()?.as_millis().to_string();
    let query = [
        ("fromDate", "0"),
        ("toDate", &until),
        ("page", "1"),
        ("maxItems", "1"),
    ];
    items(&read(client, "/account/history", &query)?)?;
    Ok(None)
}

/// `timestamp + method + path + body`, HMAC-SHA256 with the secret, as hex.
/// The path includes the base URL's own path, `/v2` and the query; a GET has
/// an empty body.
pub(super) fn signature(secret: &str, timestamp: &str, path: &str) -> String {
    hmac_sha256_hex(
        secret.as_bytes(),
        format!("{timestamp}GET{path}").as_bytes(),
    )
}

/// One signed read of `/v2{endpoint}?{query}`, as JSON.
fn read(client: &Client<'_>, endpoint: &str, query: &[(&str, &str)]) -> Result<Py, AppError> {
    let encoded: Vec<String> = query
        .iter()
        .map(|(key, value)| format!("{key}={}", bitpanda::escape(value)))
        .collect();
    let target = match encoded.is_empty() {
        true => format!("/v2{endpoint}"),
        false => format!("/v2{endpoint}?{}", encoded.join("&")),
    };
    let url = format!("{}{target}", client.base);
    let rest = client
        .base
        .split_once("://")
        .map_or(client.base, |(_, rest)| rest);
    let signed = format!("{}{target}", rest.find('/').map_or("", |at| &rest[at..]));
    let response = client.signed_response(&url, REFUSED, || {
        let timestamp = now()?.as_millis().to_string();
        Ok(vec![
            ("Accept", "application/json".to_owned()),
            ("Bitvavo-Access-Key", client.key.to_owned()),
            (
                "Bitvavo-Access-Signature",
                signature(client.secret, &timestamp, &signed),
            ),
            ("Bitvavo-Access-Timestamp", timestamp),
            ("Bitvavo-Access-Window", WINDOW.to_owned()),
        ])
    })?;
    let payload = std::str::from_utf8(&response.body)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok());
    if REFUSED.contains(&response.status) {
        return Err(refused(response.status, payload.unwrap_or(Py::Null)));
    }
    payload.ok_or_else(|| AppError::new("protocol_error", "Bitvavo response was not JSON"))
}

/// 401/403. Only Kassiber's own values reach the error: the status and a
/// documented `errorCode`, "unknown", or null; never Bitvavo's text.
fn refused(status: u16, payload: Py) -> AppError {
    let number = match at(&payload, "errorCode") {
        Py::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    };
    let hint = match number {
        Some(311) => "This key lacks View access. Create a Bitvavo API key with only View access ticked and store it in this backend.",
        Some(307) => "This key only works from the IP addresses on its whitelist; add this computer's address in Bitvavo, or create a new View-only key.",
        Some(305 | 306) => "Activate the key in Bitvavo (confirm the e-mail Bitvavo sent), then sync again.",
        _ => "Check that the key is active, allowed from this IP address, and has View access.",
    };
    AppError::new(
        "auth_error",
        format!("Bitvavo refused the API key (HTTP {status})"),
    )
    .with_hint(hint)
    .with_details(dict([
        ("http_status", Py::from(i64::from(status))),
        (
            "bitvavo_error_code",
            known_number(at(&payload, "errorCode"), &ERROR_CODES),
        ),
    ]))
}

fn protocol(message: &str) -> AppError {
    AppError::new("protocol_error", message.to_owned())
}

fn items(page: &Py) -> Result<&[Py], AppError> {
    if !matches!(page, Py::Dict(_)) {
        return Err(protocol("Bitvavo history page was not a JSON object"));
    }
    match at(page, "items") {
        Py::List(items) if items.len() <= PAGE_SIZE => Ok(items),
        Py::List(_) => Err(protocol(
            "Bitvavo history page holds more rows than requested",
        )),
        _ => Err(protocol("Bitvavo history items were not a JSON array")),
    }
}

/// Every `/account/history` row from the first transaction to `until`. The
/// page count is fixed by the first page; a page that is not the one asked
/// for, or a count that moves, fails closed.
fn history(client: &Client<'_>, until: &str) -> Result<Vec<Py>, AppError> {
    let whole = |value: &Py| match value {
        Py::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    };
    let (size, mut rows, mut total, mut page) = (PAGE_SIZE.to_string(), vec![], None, 1i64);
    loop {
        if page as usize > MAX_PAGES {
            return Err(limit());
        }
        let number = page.to_string();
        let query = [
            ("fromDate", "0"),
            ("toDate", until),
            ("page", &number),
            ("maxItems", &size),
        ];
        let payload = read(client, "/account/history", &query)?;
        let data = items(&payload)?;
        if whole(at(&payload, "currentPage")) != Some(page) {
            return Err(protocol("Bitvavo returned a different history page"));
        }
        let pages = whole(at(&payload, "totalPages"))
            .filter(|n| *n >= 0)
            .ok_or_else(|| protocol("Bitvavo history page has no page count"))?;
        if *total.get_or_insert(pages) != pages {
            return Err(protocol(
                "Bitvavo history changed while it was read; sync again",
            ));
        }
        rows.extend(data.iter().cloned());
        if rows.len() > MAX_ROWS {
            return Err(limit());
        }
        if page >= pages {
            return Ok(rows);
        }
        page += 1;
    }
}

/// `/balance?symbol=BTC`: `available` plus `inOrder`, each whole msat by its
/// own digits. An empty list is a zero balance (Bitvavo lists only assets
/// above zero).
fn balance(client: &Client<'_>) -> Result<Balance, AppError> {
    let payload = read(client, "/balance", &[("symbol", "BTC")])?;
    let as_of = utc_iso(now()?.as_secs() as f64)?;
    let Py::List(rows) = &payload else {
        return Err(protocol("Bitvavo balance was not a JSON array"));
    };
    let mut btc = rows.iter().filter(|row| asset(at(row, "symbol")) == "BTC");
    let mut total: i64 = 0;
    if let Some(row) = btc.next() {
        for key in ["available", "inOrder"] {
            let value = plain(at(row, key))
                .ok_or_else(|| protocol("Bitvavo BTC balance is not a plain decimal"))?;
            let part = exact_msat(&value)
                .ok_or_else(|| protocol("Bitvavo BTC balance is finer than a millisatoshi"))?;
            total = total
                .checked_add(part)
                .ok_or_else(|| protocol("Bitvavo BTC balance is out of range"))?;
        }
    }
    if btc.next().is_some() {
        return Err(protocol("Bitvavo listed its BTC balance twice"));
    }
    Ok(Balance {
        msat: Ok(total),
        as_of,
        scope: BALANCE_SCOPE,
    })
}

/// An unsigned plain decimal JSON string, as the API writes amounts.
fn plain(value: &Py) -> Option<Decimal> {
    match value {
        Py::Str(text) => strict_decimal(text.as_str()?, false),
        _ => None,
    }
}

/// One side of a transaction: an asset code and a non-negative amount.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Leg {
    pub asset: String,
    pub amount: Decimal,
}

/// One Bitvavo transaction in the shape both sources share. `sent` and
/// `received` are what left and reached the account besides `fee`.
#[derive(Debug)]
pub(super) struct Event {
    pub id: PyStr,
    pub kind: Py,
    /// Unix milliseconds, UTC: the order transactions are stored in.
    pub at: i64,
    pub sent: Option<Leg>,
    pub received: Option<Leg>,
    pub fee: Option<Leg>,
    /// The unit price, in the quote currency.
    pub price: Option<Leg>,
    /// A withdrawal's `sent` includes its fee (the CSV export); the record
    /// keeps what left on chain and the fee apart, as the API states them.
    pub fee_included: bool,
    pub raw: Py,
}

/// One `/account/history` row, read strictly.
fn event(row: &Py) -> Result<Event, AppError> {
    if !matches!(row, Py::Dict(_)) {
        return Err(protocol("Bitvavo history row was not a JSON object"));
    }
    let id = match at(row, "transactionId") {
        Py::Str(id) if !kassiber_pytext::strip(&lossy(id)).is_empty() => id.clone(),
        _ => return Err(protocol("Bitvavo history row without a transactionId")),
    };
    let shown = shown(&Py::Str(id.clone()));
    let blank = |v: &Py| {
        matches!(v, Py::Null) || matches!(v, Py::Str(t) if t.code_points().next().is_none())
    };
    let leg = |currency: &str, amount: &str| -> Result<Option<Leg>, AppError> {
        let (code, value) = (at(row, currency), at(row, amount));
        match (blank(code), blank(value)) {
            (true, true) => Ok(None),
            (false, false) => Ok(Some(Leg {
                asset: asset(code),
                amount: plain(value).ok_or_else(|| {
                    malformed(
                        PROVIDER,
                        &shown,
                        &format!("a {amount} that is not a plain decimal"),
                    )
                })?,
            })),
            _ => Err(malformed(
                PROVIDER,
                &shown,
                &format!("{currency} and {amount} that do not come together"),
            )),
        }
    };
    let executed = match at(row, "executedAt") {
        Py::Str(text) => text
            .as_str()
            .and_then(kassiber_pytext::iso_datetime)
            .filter(|parsed| parsed.offset_us.is_some())
            .map(|parsed| parsed.utc_us()),
        _ => None,
    }
    .ok_or_else(|| malformed(PROVIDER, &shown, "an execution time without a time zone"))?;
    Ok(Event {
        id,
        kind: at(row, "type").clone(),
        // Microseconds since 0001-01-01 to Unix milliseconds.
        at: executed.div_euclid(1_000) - 719_162 * 86_400_000,
        sent: leg("sentCurrency", "sentAmount")?,
        received: leg("receivedCurrency", "receivedAmount")?,
        fee: leg("feesCurrency", "feesAmount")?,
        price: leg("priceCurrency", "priceAmount")?,
        fee_included: false,
        raw: row.clone(),
    })
}

/// Server or file text in an error, cut short. Credentials are withheld from
/// every sync error afterwards.
fn shown(value: &Py) -> String {
    let text = match value {
        Py::Str(text) => lossy(text),
        Py::Null => "None".into(),
        other => kassiber_envelope::py_str(other),
    };
    text.chars().take(80).collect()
}

/// What a Bitvavo transaction type means: the one place that maps a type.
/// A type not listed refuses the import.
enum Kind {
    Trade { buy: bool },
    Convert,
    Movement { inbound: bool, kind: &'static str },
}
fn kind(name: &str) -> Result<Kind, &'static str> {
    Ok(match name {
        "buy" => Kind::Trade { buy: true },
        "sell" => Kind::Trade { buy: false },
        "deposit" => Kind::Movement {
            inbound: true,
            kind: "deposit",
        },
        "withdrawal" => Kind::Movement {
            inbound: false,
            kind: "withdrawal",
        },
        "staking" | "fixed_staking" => Kind::Movement {
            inbound: true,
            kind: "staking",
        },
        "affiliate" => Kind::Movement {
            inbound: true,
            kind: "income",
        },
        "convert_buy" | "convert_sell" => Kind::Convert,
        _ if name.starts_with("margin_") => {
            return Err("is a margin entry; margin trading is out of scope")
        }
        "loan" => return Err("is a loan entry; lending is out of scope"),
        _ => return Err("has a type Kassiber does not map"),
    })
}

/// How a source states a trade's quote amount.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FeeReading {
    /// After the fee: paid for a buy, received for a sale (the CSV export).
    AfterFee,
    /// Not documented (the API): the amounts decide, or nothing does.
    Unknown,
}

/// A trade's fiat value under a [`FeeReading`].
#[derive(Debug, PartialEq)]
pub(super) enum TradeValue {
    /// A buy's cost including the fee, a sale's proceeds net of it.
    Exact(Decimal),
    /// Both readings fit the amounts: imported without exact pricing.
    Ambiguous,
    /// The amounts fit no reading, or the proceeds are not positive.
    Mismatch,
}

/// Both sources' transactions to events under the wallet's asset scope. The
/// contract's filter leaves out and counts one without an asset the scope
/// keeps, before its type is read; any other is mapped or refuses the whole
/// import: a partial custodial history would misstate balance and basis.
pub(super) fn map(
    events: Vec<Event>,
    method: &'static str,
    reading: FeeReading,
    mapper: &mut Mapper,
) -> Result<(), AppError> {
    let scope = mapper.scope();
    let keep = format!(
        "A wallet whose asset_scope is bitcoin keeps no other crypto asset. To import both sides, run {}, then sync or import again.",
        mapper.keep_command()
    );
    // A page boundary may repeat a transaction; a repeat must be identical.
    let mut unique: Vec<Event> = Vec::with_capacity(events.len());
    let mut index = std::collections::HashMap::<String, usize>::new();
    for event in events {
        let id = lossy(&event.id);
        match index.get(&id) {
            Some(&at) if unique[at].raw == event.raw => continue,
            Some(_) => {
                return Err(protocol(&format!(
                    "Bitvavo returned transaction '{}' twice with different contents",
                    shown(&Py::Str(event.id.clone()))
                )))
            }
            None => index.insert(id, unique.len()),
        };
        unique.push(event);
    }
    // Both sources list the newest first; records are stored oldest first by
    // the millisecond time, so rows in one second keep the order they
    // happened in. The sort is stable for exact ties.
    unique.sort_by_key(|event| event.at);
    for event in &unique {
        let codes: Vec<&str> = [&event.sent, &event.received, &event.fee]
            .into_iter()
            .flatten()
            .map(|leg| leg.asset.as_str())
            .collect();
        if mapper.admit(&codes) {
            for record in record_for(event, method, reading, scope, &keep)? {
                mapper.push(record)?;
            }
        }
    }
    Ok(())
}

fn record_for(
    event: &Event,
    method: &'static str,
    reading: FeeReading,
    scope: Scope,
    keep: &str,
) -> Result<Vec<contract::Event>, AppError> {
    let name = match &event.kind {
        Py::Str(text) => kassiber_pytext::lower(kassiber_pytext::strip(&lossy(text))),
        _ => String::new(),
    };
    let (id, shown_kind) = (shown(&Py::Str(event.id.clone())), shown(&event.kind));
    let refuse = |reason: &dyn std::fmt::Display, hint: &str| {
        refusal(PROVIDER, &id, &shown_kind, reason, hint)
    };
    let legs = [&event.sent, &event.received, &event.fee];
    if legs
        .into_iter()
        .flatten()
        .any(|leg| is_bitcoin(&leg.asset) && exact_msat(&leg.amount).is_none())
    {
        return Err(refuse(
            &"has a BTC amount that is not a whole number of millisatoshi",
            "Bitvavo returned a transaction Kassiber cannot read exactly; nothing was imported.",
        ));
    }
    if let Some(leg) = legs.into_iter().flatten().find(|leg| !ticker(&leg.asset)) {
        let code = shown(&s(leg.asset.as_str()));
        return Err(refuse(
            &format!("names an asset '{code}' that is not a ticker"),
            REVIEW,
        ));
    }
    // The assets this wallet keeps: Bitcoin, and in `crypto` scope every
    // other crypto asset.
    let kept = |code: &str| is_bitcoin(code) || (scope == Scope::Crypto && !is_fiat(code));
    let shaped = match kind(&name) {
        Ok(Kind::Trade { buy }) => trade(event, &refuse, buy, method, reading, &kept, keep)?,
        Ok(Kind::Convert) => match (&event.sent, &event.received) {
            (Some(sent), Some(received)) if kept(&sent.asset) && kept(&received.asset) => {
                swap(event, &refuse, sent, received, None, SWAP_FEE)?
            }
            _ if scope == Scope::Bitcoin => {
                return Err(refuse(
                    &"is a Convert swap between BTC and another asset",
                    keep,
                ))
            }
            _ => return Err(refuse(&"is a Convert without two crypto assets", REVIEW)),
        },
        Ok(Kind::Movement { inbound, kind }) => {
            vec![movement(event, &refuse, inbound, kind, &kept)?]
        }
        Err(reason) => return Err(refuse(&reason, REVIEW)),
    };
    let reference = concat(&["bitvavo:".into(), event.id.clone()]);
    let occurred_at: PyStr = utc_iso(event.at.div_euclid(1_000) as f64)?.into();
    let mut records = vec![];
    for row in shaped {
        // A third-asset fee is a fee-only spend of that asset.
        let (amount, fee) = match row.kind {
            "fee" => assets::fee_only(canonical(&row.leg.amount)?),
            _ => (canonical(&row.leg.amount)?, canonical(&row.fee)?),
        };
        let entry = Entry {
            provider: PROVIDER,
            external_id: concat(&[reference.clone(), row.suffix.into()]),
            occurred_at: Some(occurred_at.clone()),
            inbound: row.inbound,
            asset: row.leg.asset,
            amount,
            fee,
            kind: row.kind,
            raw: event.raw.clone(),
            description: (!row.description.is_empty()).then(|| row.description.into()),
            pricing: row.pricing,
        };
        let kind = match row.kind {
            "swap" => Some(contract::Kind::Swap(reference.clone())),
            _ => None,
        };
        let event = contract::Event::from_entry(entry);
        records.push(match kind {
            Some(kind) => contract::Event { kind, ..event },
            None => event,
        });
    }
    Ok(records)
}

type Refuse<'a> = dyn Fn(&dyn std::fmt::Display, &str) -> AppError + 'a;
/// A mapped row: its id suffix, direction, leg, fee, kind, pricing and
/// description (empty: the record's own, "Bitvavo <kind>").
struct Shaped {
    suffix: &'static str,
    inbound: bool,
    leg: Leg,
    fee: Decimal,
    kind: &'static str,
    pricing: Option<Pricing>,
    description: &'static str,
}
/// An unpriced [`Shaped`] row with the record's own description.
fn row(suffix: &'static str, inbound: bool, leg: Leg, fee: Decimal, kind: &'static str) -> Shaped {
    let (pricing, description) = (None, "");
    Shaped {
        suffix,
        inbound,
        leg,
        fee,
        kind,
        pricing,
        description,
    }
}

/// A buy or sell of a kept asset against a fiat currency, its quote amount
/// checked against amount × price under the source's [`FeeReading`], or a
/// trade against another crypto asset as a [`swap`]. A fee in a third crypto
/// asset is its own `fee` row.
fn trade(
    event: &Event,
    refuse: &Refuse<'_>,
    buy: bool,
    method: &'static str,
    reading: FeeReading,
    kept: &dyn Fn(&str) -> bool,
    keep: &str,
) -> Result<Vec<Shaped>, AppError> {
    let (leg, cash) = match buy {
        true => (&event.received, &event.sent),
        false => (&event.sent, &event.received),
    };
    let (Some(leg), Some(cash)) = (leg, cash) else {
        return Err(refuse(
            &"does not move an asset and its counter asset",
            REVIEW,
        ));
    };
    let not_fiat = |code: &str| {
        format!(
            "trades {} against {code}, which is not a fiat currency",
            leg.asset
        )
    };
    if is_fiat(&leg.asset) {
        let reason = format!("trades the fiat currency {} as its asset", leg.asset);
        return Err(refuse(&reason, REVIEW));
    }
    if !kept(&leg.asset) {
        // `bitcoin` scope: BTC is the other side (an altcoin bought or sold
        // for BTC), or only the fee.
        let reason = match is_bitcoin(&cash.asset) {
            true => format!(
                "trades BTC against {}, which is not a fiat currency",
                leg.asset
            ),
            false => format!(
                "pays a BTC fee on a trade between {} and {}",
                cash.asset, leg.asset
            ),
        };
        return Err(refuse(&reason, keep));
    }
    if !is_fiat(&cash.asset) {
        if !kept(&cash.asset) {
            return Err(refuse(&not_fiat(&cash.asset), keep));
        }
        let (sent, received) = if buy { (cash, leg) } else { (leg, cash) };
        let quote = (reading == FeeReading::AfterFee).then_some(cash.asset.as_str());
        let hint = if quote.is_some() {
            SWAP_FEE
        } else {
            API_SWAP_FEE
        };
        return swap(event, refuse, sent, received, quote, hint);
    }
    if leg.amount.is_zero() || cash.amount.is_zero() {
        return Err(refuse(&"has no asset amount or no cash amount", REVIEW));
    }
    let mut rows = vec![];
    let fee = match &event.fee {
        Some(fee) if fee.amount.is_zero() || fee.asset == cash.asset => fee.amount.clone(),
        Some(fee) if fee.asset != leg.asset && kept(&fee.asset) => {
            rows.push(row(":fee", false, fee.clone(), Decimal::zero(), "fee"));
            Decimal::zero()
        }
        Some(fee) => {
            let reason = format!(
                "charges its fee in {} instead of {}; Kassiber cannot place it exactly",
                fee.asset, cash.asset
            );
            let hint = match is_fiat(&fee.asset) || fee.asset == leg.asset {
                true => REVIEW,
                false => keep,
            };
            return Err(refuse(&reason, hint));
        }
        None => Decimal::zero(),
    };
    let Some(price) = event.price.as_ref().filter(|p| p.asset == cash.asset) else {
        let reason = format!("has no {} price to check its amounts against", cash.asset);
        return Err(refuse(&reason, REVIEW));
    };
    let pricing = match trade_value(&leg.amount, &price.amount, &cash.amount, &fee, buy, reading)? {
        TradeValue::Exact(value) => Pricing {
            fiat_currency: cash.asset.clone(),
            fiat_value: canonical(&value)?,
            fiat_rate: None,
            method,
            external_ref: event.id.clone(),
        },
        TradeValue::Ambiguous => {
            return Err(refuse(
                &"has a cash amount that fits its price both with and without its fee",
                API_TRADE_FEE,
            ))
        }
        TradeValue::Mismatch => {
            return Err(refuse(
                &"has a cash amount that does not match its amount, price and fee",
                REVIEW,
            ))
        }
    };
    let description = if buy { "Bitvavo buy" } else { "Bitvavo sell" };
    let kind = if buy { "buy" } else { "sell" };
    let shaped = row("", buy, leg.clone(), Decimal::zero(), kind);
    rows.insert(
        0,
        Shaped {
            pricing: Some(pricing),
            description,
            ..shaped
        },
    );
    Ok(rows)
}

/// A crypto-to-crypto trade or a Convert: two unpriced `swap` legs sharing
/// the transaction's id, a fee in a third crypto asset as its own row. A fee
/// in a traded asset is placed only in the export's trades (`quote` set):
/// their paid amount includes the quote-asset fee, their received amount is
/// net of it. The API does not say, nor does either source for a Convert.
fn swap(
    event: &Event,
    refuse: &Refuse<'_>,
    sent: &Leg,
    received: &Leg,
    quote: Option<&str>,
    hint: &str,
) -> Result<Vec<Shaped>, AppError> {
    if sent.asset == received.asset || sent.amount.is_zero() || received.amount.is_zero() {
        return Err(refuse(&"does not swap one asset for another", REVIEW));
    }
    let (mut out, mut out_fee, mut rows) = (sent.amount.clone(), Decimal::zero(), vec![]);
    match &event.fee {
        Some(fee) if fee.amount.is_zero() => {}
        Some(fee) if quote == Some(sent.asset.as_str()) && fee.asset == sent.asset => {
            out = net_of_fee(&sent.amount, &fee.amount, refuse)?
                .ok_or_else(|| refuse(&"has a fee as large as its amount", REVIEW))?;
            out_fee = fee.amount.clone();
        }
        Some(fee) if quote == Some(received.asset.as_str()) && fee.asset == received.asset => {}
        Some(fee)
            if fee.asset != sent.asset && fee.asset != received.asset && !is_fiat(&fee.asset) =>
        {
            rows.push(row(":fee", false, fee.clone(), Decimal::zero(), "fee"))
        }
        Some(fee) => {
            let reason = format!(
                "charges a {} fee Kassiber cannot place on a swap",
                fee.asset
            );
            return Err(refuse(&reason, hint));
        }
        None => {}
    }
    let asset = sent.asset.clone();
    let legs = [
        row(":out", false, Leg { asset, amount: out }, out_fee, "swap"),
        row(":in", true, received.clone(), Decimal::zero(), "swap"),
    ];
    rows.splice(0..0, legs);
    Ok(rows)
}

/// `amount - fee` in the book's unit (msat for BTC, the same 10^-11 for
/// every other asset), never rounded; `None` when nothing is left.
fn net_of_fee(
    amount: &Decimal,
    fee: &Decimal,
    refuse: &Refuse<'_>,
) -> Result<Option<Decimal>, AppError> {
    let (Some(amount), Some(fee)) = (exact_msat(amount), exact_msat(fee)) else {
        return Err(refuse(
            &"has an amount finer than 11 decimal places",
            REVIEW,
        ));
    };
    (amount > fee)
        .then(|| btc_decimal(amount - fee))
        .transpose()
}

/// A trade's fiat value. The quote amount `cash` is read after the fee
/// (`AfterFee`), or, for the API, as whichever single reading fits amount ×
/// price within [`TOLERANCE`]; two fitting readings are ambiguous, none (or
/// proceeds that are not positive) a mismatch.
pub(super) fn trade_value(
    btc: &Decimal,
    price: &Decimal,
    cash: &Decimal,
    fee: &Decimal,
    buy: bool,
    reading: FeeReading,
) -> Result<TradeValue, AppError> {
    let e = decimal_error;
    let tolerance: Decimal = TOLERANCE.parse().expect("tolerance");
    // `a` with the fee added for a buy, taken off for a sale.
    let with_fee = |a: &Decimal| {
        if buy {
            a.add(fee, &CTX)
        } else {
            a.sub(fee, &CTX)
        }
        .map_err(e)
    };
    let gross = btc.mul(price, &CTX).map_err(e)?;
    let after_fee = with_fee(&gross)?;
    let fits = |expected: &Decimal| -> Result<bool, AppError> {
        let off = cash
            .sub(expected, &CTX)
            .and_then(|d| d.abs(&CTX))
            .map_err(e)?;
        Ok(!tolerance.sub(&off, &CTX).map_err(e)?.is_negative())
    };
    let value = match reading {
        FeeReading::AfterFee => fits(&after_fee)?.then(|| cash.clone()),
        FeeReading::Unknown if fee.is_zero() => fits(&gross)?.then(|| cash.clone()),
        FeeReading::Unknown => match (fits(&gross)?, fits(&after_fee)?) {
            (true, true) => return Ok(TradeValue::Ambiguous),
            (true, false) => Some(with_fee(cash)?),
            (false, true) => Some(cash.clone()),
            (false, false) => None,
        },
    };
    Ok(match value {
        Some(value) if value.is_positive() => TradeValue::Exact(value),
        _ => TradeValue::Mismatch,
    })
}

/// A deposit, withdrawal or reward of a kept asset: one leg on the side
/// `inbound` names and nothing on the other. Only a withdrawal may carry a
/// fee, in its own asset.
fn movement(
    event: &Event,
    refuse: &Refuse<'_>,
    inbound: bool,
    kind: &'static str,
    kept: &dyn Fn(&str) -> bool,
) -> Result<Shaped, AppError> {
    let (leg, other) = match inbound {
        true => (&event.received, &event.sent),
        false => (&event.sent, &event.received),
    };
    let Some(leg) = leg.as_ref().filter(|leg| kept(&leg.asset)) else {
        return Err(refuse(&"moves BTC the other way or only as a fee", REVIEW));
    };
    if other.is_some() {
        return Err(refuse(&"moves BTC alongside another asset", REVIEW));
    }
    let fee = match &event.fee {
        None => Decimal::zero(),
        Some(fee) if fee.amount.is_zero() => Decimal::zero(),
        Some(fee) if !inbound && fee.asset == leg.asset => fee.amount.clone(),
        Some(_) => return Err(refuse(&"charges a fee Kassiber cannot place", REVIEW)),
    };
    let mut amount = leg.amount.clone();
    if event.fee_included && !fee.is_zero() {
        amount = net_of_fee(&amount, &fee, refuse)?
            .ok_or_else(|| refuse(&"has a withdrawal fee as large as its amount", REVIEW))?;
    }
    if amount.is_zero() {
        return Err(refuse(&"has a zero amount", REVIEW));
    }
    let description = match kind {
        "deposit" => "Bitvavo deposit",
        "withdrawal" => "Bitvavo withdrawal",
        "staking" => "Bitvavo staking reward",
        _ => "Bitvavo affiliate reward",
    };
    let leg = Leg {
        asset: leg.asset.clone(),
        amount,
    };
    Ok(Shaped {
        description,
        ..row("", inbound, leg, fee, kind)
    })
}
