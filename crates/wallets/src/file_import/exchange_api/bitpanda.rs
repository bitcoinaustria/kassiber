//! Bitpanda's public API (`api.public.bitpanda.com`), Rust only: Python has
//! no reference importer. Reads only: a key that can trade is refused, then
//! the asset list, the fiat currencies and every `/v1/operations` page are
//! read and mapped. BTC is always imported; other crypto assets only in the
//! wallet's `crypto` asset scope (docs/plan/23-crypto-assets.md), and
//! securities, metals and indices never. Bitpanda does not document its
//! operation and transaction type values, so legs are mapped by structure
//! where structure decides and fail closed otherwise.
//! docs/reference/bitpanda.md records the API, the mapping and every refusal.
use super::assets::Class;
use super::*;
use std::collections::BTreeSet;

const PAGE_SIZE: &str = "100";
/// A key the API refuses, or a scope the key lacks.
const REFUSED: &[u16] = &[401, 403];
/// `GET /v1/trading-limits` needs `Trade (Write)`: 200 and its documented
/// 404 ("no limits configured for this API key") both mean the key can trade.
const LIMITS: &[u16] = &[401, 403, 404];
/// Operation types (lowercased) a single BTC leg without a trade may carry.
/// Taken from Bitpanda's export vocabulary; the API leaves them undocumented.
const MOVEMENTS: [&str; 3] = ["deposit", "withdrawal", "reward"];
/// Words that, as a transaction type, contradict a different operation type.
const NAMED: [&str; 5] = ["deposit", "withdrawal", "reward", "buy", "sell"];
/// Bitpanda's BTC quantities must be whole msat.
const BTC_DECIMALS: usize = 11;

/// Bitpanda's BTC balance as its history states it, or why it is unknown.
#[derive(Clone, PartialEq, Debug)]
pub(super) enum Reported {
    Msat(i64),
    Unavailable(&'static str),
}

pub(super) fn fetch(client: &Client<'_>) -> Result<Fetched, AppError> {
    refuse_trading_keys(client)?;
    let mut budget = Budget::default();
    // Every listed asset, so other assets are named by their own ticker (and
    // imported in crypto scope); BTC is the one crypto asset `BTC`.
    let assets = pages(client, "/v1/assets", &[], &mut budget)?;
    let btc = btc_asset(&assets)?;
    let listed = listing(&assets);
    let currencies = currencies(&read(client, "/v1/currencies")?)?;
    let operations = pages(client, "/v1/operations", &[], &mut budget)?;
    let context = Context {
        btc: &btc,
        listed: &listed,
        currencies: &currencies,
        secrets: &[client.key, client.secret],
        scope: client.scope,
    };
    normalize(&operations, &context)
}

/// The desktop key check: the write-scope refusal, then one read each with
/// the two scopes the sync needs (`Trade (Read)` for currencies,
/// `Transaction` for operations).
pub(super) fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    refuse_trading_keys(client)?;
    currencies(&read(client, "/v1/currencies")?)?;
    let page = read(client, "/v1/operations?page_size=1")?;
    if !matches!(at(&page, "data"), Py::List(_)) {
        return Err(AppError::new(
            "protocol_error",
            "Bitpanda operations page has no data list",
        ));
    }
    Ok(Some(true))
}

/// The one request that tells whether this key may trade. Kassiber never
/// writes to Bitpanda and refuses a key that could.
fn refuse_trading_keys(client: &Client<'_>) -> Result<(), AppError> {
    let url = format!("{}/v1/trading-limits", client.base);
    let response = client.response(&url, &headers(client), LIMITS)?;
    if REFUSED.contains(&response.status) {
        return Ok(());
    }
    Err(AppError::new(
        "auth_error",
        "This Bitpanda API key can trade; Kassiber only accepts read-only keys",
    )
    .with_hint(
        "Create a Bitpanda API key with only the Transaction and Trade (Read) scopes, store it in this backend, and delete the old key.",
    )
    .with_details(dict([("http_status", Py::from(i64::from(response.status)))])))
}

fn headers<'a>(client: &'a Client<'_>) -> [(&'static str, &'a str); 2] {
    [("Accept", "application/json"), ("x-api-key", client.key)]
}

/// One authenticated read of `path` (with its query), as JSON.
fn read(client: &Client<'_>, path: &str) -> Result<Py, AppError> {
    let url = format!("{}{path}", client.base);
    let response = client.response(&url, &headers(client), REFUSED)?;
    if REFUSED.contains(&response.status) {
        return Err(refused(client, response.status, &response.body));
    }
    std::str::from_utf8(&response.body)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
        .ok_or_else(|| AppError::new("protocol_error", "Bitpanda response was not JSON"))
}

/// 401/403: the key is wrong, expired, IP-restricted or lacks a scope. The
/// server's error code is kept, with any echoed credential withheld.
fn refused(client: &Client<'_>, status: u16, body: &[u8]) -> AppError {
    let code = std::str::from_utf8(body)
        .ok()
        .and_then(|text| kassiber_pytext::loads(text).ok())
        .and_then(|payload| match get(&payload, "error") {
            Some(error @ Py::Dict(_)) => match at(error, "code") {
                Py::Str(code) => Some(lossy(code)),
                _ => None,
            },
            _ => None,
        })
        .map(|code| {
            let code = kassiber_chain::withhold_credentials(&code, &[client.key, client.secret]);
            s(code.chars().take(100).collect::<String>())
        })
        .unwrap_or(Py::Null);
    AppError::new(
        "auth_error",
        format!("Bitpanda refused the API key (HTTP {status})"),
    )
    .with_hint(
        "Check that the key is current, allowed from this IP address, and has the Transaction and Trade (Read) scopes.",
    )
    .with_details(dict([
        ("http_status", Py::from(i64::from(status))),
        ("bitpanda_error", code),
    ]))
}

#[derive(Default)]
struct Budget {
    pages: usize,
    rows: usize,
}

/// A cursor-paged collection: `data` rows until `has_next_page` is false.
/// Each page must say whether another follows and name a fresh cursor.
fn pages(
    client: &Client<'_>,
    path: &str,
    query: &[(&str, &str)],
    budget: &mut Budget,
) -> Result<Vec<Py>, AppError> {
    let protocol = |message: &str| AppError::new("protocol_error", message.to_owned());
    let mut rows = vec![];
    let mut cursors = BTreeSet::new();
    let mut cursor: Option<String> = None;
    loop {
        budget.pages += 1;
        if budget.pages > MAX_PAGES {
            return Err(limit());
        }
        let mut params: Vec<(&str, &str)> = query.to_vec();
        params.push(("page_size", PAGE_SIZE));
        if let Some(cursor) = &cursor {
            params.push(("cursor", cursor));
        }
        let encoded = params
            .iter()
            .map(|(key, value)| format!("{key}={}", escape(value)))
            .collect::<Vec<_>>()
            .join("&");
        let payload = read(client, &format!("{path}?{encoded}"))?;
        if !matches!(payload, Py::Dict(_)) {
            return Err(protocol("Bitpanda response was not a JSON object"));
        }
        let Py::List(data) = at(&payload, "data") else {
            return Err(protocol("Bitpanda response data was not a JSON array"));
        };
        budget.rows += data.len();
        if budget.rows > MAX_ROWS {
            return Err(limit());
        }
        rows.extend(data.iter().cloned());
        match at(&payload, "has_next_page") {
            Py::Bool(false) => return Ok(rows),
            Py::Bool(true) => {}
            _ => {
                return Err(protocol(
                    "Bitpanda page did not say whether another follows",
                ))
            }
        }
        let next = match at(&payload, "next_cursor") {
            Py::Str(next) if next.code_points().next().is_some() => lossy(next),
            _ => {
                return Err(protocol(
                    "Bitpanda page announced another page without a cursor",
                ))
            }
        };
        if !cursors.insert(next.clone()) {
            return Err(protocol("Bitpanda paging repeated a cursor"));
        }
        cursor = Some(next);
    }
}

/// Query escaping: everything but RFC 3986's unreserved characters.
pub(super) fn escape(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(char::from(b));
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A UUID-shaped id from the API, compared case-insensitively.
fn id(value: &Py) -> Option<String> {
    match value {
        Py::Str(text) => {
            let text = kassiber_pytext::strip(&lossy(text)).to_ascii_lowercase();
            (!text.is_empty()).then_some(text)
        }
        _ => None,
    }
}
fn folded(value: &Py) -> String {
    match value {
        Py::Str(text) => kassiber_pytext::lower(kassiber_pytext::strip(&lossy(text))),
        _ => String::new(),
    }
}

/// BTC's asset id. Another asset may share the ticker (a stock or ETF), so
/// several matches narrow to the crypto one; anything but one id fails.
pub(super) fn btc_asset(rows: &[Py]) -> Result<String, AppError> {
    let matches: Vec<&Py> = rows
        .iter()
        .filter(|row| folded(at(row, "symbol")) == "btc")
        .collect();
    let crypto: Vec<&Py> = if matches.len() > 1 {
        matches
            .into_iter()
            .filter(|row| {
                folded(at(row, "type")) == "crypto" || folded(at(row, "group")) == "cryptocoin"
            })
            .collect()
    } else {
        matches
    };
    let ids: BTreeSet<String> = crypto.iter().filter_map(|row| id(at(row, "id"))).collect();
    match (crypto.len(), ids.into_iter().next()) {
        (1, Some(id)) => Ok(id),
        _ => Err(AppError::new(
            "protocol_error",
            "Bitpanda did not list exactly one BTC crypto asset",
        )),
    }
}

/// How Bitpanda lists an asset.
struct Listed {
    /// The canonical ticker.
    code: String,
    /// Bitpanda types it as crypto (`type` crypto, or `group` cryptocoin or
    /// stablecoin); anything else (stocks, ETFs, ETCs, metals, indices,
    /// leveraged products) is never imported.
    crypto: bool,
    /// Another listed crypto asset has the same ticker: it cannot be imported
    /// under that ticker without mixing two assets.
    shared: bool,
}

/// `/v1/assets`: asset ids to how Bitpanda lists them.
fn listing(rows: &[Py]) -> BTreeMap<String, Listed> {
    let mut listed: BTreeMap<String, Listed> = BTreeMap::new();
    for row in rows {
        let Some(asset_id) = id(at(row, "id")) else {
            continue;
        };
        let code = asset(at(row, "symbol"));
        if code.is_empty() {
            continue;
        }
        let crypto = folded(at(row, "type")) == "crypto"
            || ["cryptocoin", "stablecoin"].contains(&folded(at(row, "group")).as_str());
        listed.insert(
            asset_id,
            Listed {
                code,
                crypto,
                shared: false,
            },
        );
    }
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for entry in listed.values().filter(|l| l.crypto) {
        *seen.entry(entry.code.clone()).or_default() += 1;
    }
    for entry in listed.values_mut().filter(|l| l.crypto) {
        entry.shared = seen[&entry.code] > 1;
    }
    listed
}

/// `/v1/currencies`: fiat currency ids to their codes.
pub(super) fn currencies(payload: &Py) -> Result<BTreeMap<String, String>, AppError> {
    let Py::List(rows) = at(payload, "data") else {
        return Err(AppError::new(
            "protocol_error",
            "Bitpanda currencies were not a JSON array",
        ));
    };
    let mut result = BTreeMap::new();
    for row in rows {
        if let Some(currency) = id(at(row, "id")) {
            result.insert(currency, asset(at(row, "symbol")));
        }
    }
    Ok(result)
}

/// What one amount object holds.
#[derive(Clone, PartialEq, Debug)]
enum Holding {
    /// A listed asset: its ticker, and whether it is BTC, another crypto
    /// asset or excluded (securities, metals, indices).
    Asset(String, Class),
    /// A fiat currency code (empty when the id is not a listed currency).
    Fiat(String),
    /// An asset id Bitpanda does not list (a delisted asset, say).
    Unlisted(String),
    Unknown,
}
impl Holding {
    fn is_btc(&self) -> bool {
        matches!(self, Holding::Asset(_, Class::Bitcoin))
    }
    /// The ticker (or unlisted id) of an asset holding.
    fn code(&self) -> &str {
        match self {
            Holding::Asset(code, _) | Holding::Unlisted(code) | Holding::Fiat(code) => code,
            Holding::Unknown => "an unknown asset",
        }
    }
}

pub(super) struct Context<'a> {
    btc: &'a str,
    listed: &'a BTreeMap<String, Listed>,
    currencies: &'a BTreeMap<String, String>,
    secrets: &'a [&'a str],
    /// The wallet's asset scope: whether other crypto assets are imported.
    scope: assets::Scope,
}
impl Context<'_> {
    fn asset(&self, asset: &str) -> Holding {
        if asset == self.btc {
            return Holding::Asset("BTC".into(), Class::Bitcoin);
        }
        match self.listed.get(asset) {
            Some(listed) if listed.crypto => Holding::Asset(listed.code.clone(), Class::Crypto),
            Some(listed) => Holding::Asset(listed.code.clone(), Class::Excluded),
            None => Holding::Unlisted(asset.into()),
        }
    }
    fn holding(&self, amount: &Py) -> Holding {
        if let Some(asset) = id(at(amount, "asset_id")) {
            return self.asset(&asset);
        }
        match id(at(amount, "currency_id")) {
            Some(currency) => {
                Holding::Fiat(self.currencies.get(&currency).cloned().unwrap_or_default())
            }
            None => Holding::Unknown,
        }
    }
    /// Whether the wallet imports this holding: BTC always, another crypto
    /// asset in `crypto` scope.
    fn admitted(&self, holding: &Holding) -> bool {
        match holding {
            Holding::Asset(_, Class::Bitcoin) => true,
            Holding::Asset(_, Class::Crypto) => self.scope == assets::Scope::Crypto,
            _ => false,
        }
    }
    /// Server text in an error, with any echoed credential withheld.
    fn shown(&self, value: &Py) -> String {
        let text = match value {
            Py::Str(text) => lossy(text),
            Py::Null => "None".into(),
            other => kassiber_envelope::py_str(other),
        };
        let text = kassiber_chain::withhold_credentials(&text, self.secrets);
        text.chars().take(80).collect()
    }
}

/// One transaction of an operation, read strictly.
struct Leg<'a> {
    raw: &'a Py,
    id: PyStr,
    holding: Holding,
    inbound: bool,
    amount: Decimal,
    fee: Option<(Holding, Decimal)>,
    trade: Option<&'a Py>,
    occurred: i64,
    wallet: Option<String>,
    balance_after: Option<Decimal>,
}

struct Refusal<'a> {
    operation: &'a PyStr,
    context: &'a Context<'a>,
}
impl Refusal<'_> {
    fn because(&self, reason: impl std::fmt::Display, hint: &str) -> AppError {
        AppError::new(
            "validation",
            format!(
                "Bitpanda operation '{}' {reason}",
                self.context.shown(&Py::Str(self.operation.clone()))
            ),
        )
        .with_hint(hint.to_owned())
    }
    fn malformed(&self, what: &str) -> AppError {
        self.because(
            format!("has {what}"),
            "Bitpanda returned a transaction Kassiber cannot read exactly; nothing was imported.",
        )
    }
}

/// A plain non-negative decimal string, as the API documents its amounts.
fn quantity(value: &Py) -> Option<Decimal> {
    let Py::Str(text) = value else { return None };
    let text = text.as_str()?;
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    let digits = |part: &str| part.bytes().all(|b| b.is_ascii_digit());
    if whole.is_empty() || !digits(whole) || !digits(fraction) || text.ends_with('.') {
        return None;
    }
    text.parse::<Decimal>().ok()
}
fn fraction_digits(value: &Py) -> usize {
    match value {
        Py::Str(text) => text
            .as_str()
            .and_then(|t| t.split_once('.'))
            .map_or(0, |(_, f)| f.trim_end_matches('0').len()),
        _ => 0,
    }
}

/// `credited_at` with an explicit offset, as microseconds since 0001-01-01 UTC.
fn credited(value: &Py) -> Option<i64> {
    let Py::Str(text) = value else { return None };
    let parsed = kassiber_pytext::iso_datetime(text.as_str()?)?;
    parsed.offset_us?;
    Some(parsed.utc_us())
}
/// Seconds-precision UTC text, as the other exchange syncs store it.
fn occurred_at(utc_us: i64) -> Result<PyStr, AppError> {
    // Days from 0001-01-01 to 1970-01-01.
    let unix = utc_us.div_euclid(1_000_000) - 719_162 * 86_400;
    utc_iso(unix as f64).map(PyStr::from)
}

fn leg<'a>(raw: &'a Py, refuse: &Refusal<'_>) -> Result<Leg<'a>, AppError> {
    let context = refuse.context;
    let Py::Dict(_) = raw else {
        return Err(refuse.malformed("a transaction that is not an object"));
    };
    let tx_id = match at(raw, "transaction_id") {
        Py::Str(tx_id) if tx_id.code_points().next().is_some() => tx_id.clone(),
        _ => return Err(refuse.malformed("a transaction without an id")),
    };
    let amount_object = at(raw, "asset_amount");
    let holding = context.holding(amount_object);
    // The transaction's own asset must agree with its amount's.
    if let (Some(own), Some(amount)) = (id(at(raw, "asset_id")), id(at(amount_object, "asset_id")))
    {
        if own != amount {
            return Err(refuse.malformed("a transaction whose asset and amount disagree"));
        }
    }
    let amount = quantity(at(amount_object, "value"))
        .ok_or_else(|| refuse.malformed("an amount that is not a plain non-negative decimal"))?;
    let finer = fraction_digits(at(amount_object, "value")) > BTC_DECIMALS;
    if holding.is_btc() && (amount.is_zero() || finer) {
        return Err(refuse.malformed("a BTC amount that is zero or finer than a millisatoshi"));
    }
    if refuse.context.admitted(&holding) && (amount.is_zero() || finer) {
        return Err(refuse.because(
            format!(
                "has a {} amount that is zero or finer than 11 decimal places",
                holding.code()
            ),
            REVIEW,
        ));
    }
    let inbound = match folded(at(raw, "flow")).as_str() {
        "incoming" => true,
        "outgoing" => false,
        _ => return Err(refuse.malformed("a transaction flow other than INCOMING or OUTGOING")),
    };
    let fee = match at(raw, "fee_amount") {
        Py::Null => None,
        fee @ Py::Dict(_) => {
            let value = quantity(at(fee, "value")).ok_or_else(|| {
                refuse.malformed("a fee that is not a plain non-negative decimal")
            })?;
            (!value.is_zero()).then(|| (context.holding(fee), value))
        }
        _ => return Err(refuse.malformed("a fee that is not an object")),
    };
    let trade = match at(raw, "trade") {
        Py::Null => None,
        trade @ Py::Dict(_) => Some(trade),
        _ => return Err(refuse.malformed("trade details that are not an object")),
    };
    let occurred = credited(at(raw, "credited_at"))
        .ok_or_else(|| refuse.malformed("a credit time without a time zone"))?;
    let balance_after = match at(raw, "asset_balance_after") {
        balance @ Py::Dict(_) => quantity(at(balance, "value")),
        _ => None,
    };
    Ok(Leg {
        raw,
        id: tx_id,
        holding,
        inbound,
        amount,
        fee,
        trade,
        occurred,
        wallet: id(at(raw, "wallet_id")),
        balance_after,
    })
}

/// The assets an operation names anywhere: a leg, its own asset or a fee.
fn named_assets(transactions: &[Py], context: &Context<'_>) -> Vec<Holding> {
    let mut named = vec![];
    for tx in transactions {
        for value in [
            at(tx, "asset_id"),
            at(at(tx, "asset_amount"), "asset_id"),
            at(at(tx, "fee_amount"), "asset_id"),
        ] {
            if let Some(asset) = id(value) {
                let holding = context.asset(&asset);
                if !named.contains(&holding) {
                    named.push(holding);
                }
            }
        }
    }
    named
}

/// Every imported leg's time, balance after and id, by Bitpanda wallet (each
/// Bitpanda wallet holds one asset).
type Ledger = BTreeMap<String, Vec<(i64, Option<Decimal>, PyStr)>>;

/// A BTC-touching operation, read and checked as a whole.
struct Operation<'a> {
    id: &'a PyStr,
    kind: &'a Py,
    legs: Vec<Leg<'a>>,
}

pub(super) fn normalize(operations: &[Py], context: &Context<'_>) -> Result<Fetched, AppError> {
    // A page boundary may repeat an operation; a repeat must be identical.
    let mut unique: Vec<(&PyStr, &Py)> = vec![];
    for operation in operations {
        let id = match at(operation, "operation_id") {
            Py::Str(id) if id.code_points().next().is_some() => id,
            _ => {
                return Err(AppError::new(
                    "protocol_error",
                    "Bitpanda operation without an id",
                ))
            }
        };
        match unique.iter().find(|(seen, _)| *seen == id) {
            Some((_, seen)) if *seen == operation => {}
            Some(_) => {
                return Err(AppError::new(
                    "protocol_error",
                    format!(
                        "Bitpanda returned operation '{}' twice with different contents",
                        context.shown(&Py::Str(id.clone()))
                    ),
                ))
            }
            None => unique.push((id, operation)),
        }
    }
    let (mut fiat_only, mut other, mut internal) = (0i64, 0i64, 0i64);
    let mut tally = assets::Tally::default();
    let mut relevant = vec![];
    for (id, operation) in unique {
        let refuse = Refusal {
            operation: id,
            context,
        };
        let Py::List(transactions) = at(operation, "transactions") else {
            return Err(refuse.malformed("no transaction list"));
        };
        let named = named_assets(transactions, context);
        if !named.iter().any(|holding| context.admitted(holding)) {
            let fiat = !transactions.is_empty()
                && transactions
                    .iter()
                    .all(|tx| matches!(context.holding(at(tx, "asset_amount")), Holding::Fiat(_)));
            if fiat {
                fiat_only += 1;
                continue;
            }
            other += 1;
            for holding in &named {
                match holding {
                    Holding::Asset(code, class) => {
                        tally.admit(context.scope, code, *class);
                    }
                    // An asset Bitpanda no longer lists cannot be named or
                    // classified; in crypto scope it may be crypto the user
                    // held, so it is refused rather than left out unseen.
                    Holding::Unlisted(asset) if context.scope == assets::Scope::Crypto => {
                        return Err(refuse.because(
                            format!(
                                "names asset '{}', which Bitpanda does not list",
                                context.shown(&s(asset.as_str()))
                            ),
                            REVIEW,
                        ));
                    }
                    Holding::Unlisted(asset) => {
                        tally.admit(context.scope, asset, Class::Crypto);
                    }
                    _ => {}
                }
            }
            continue;
        }
        if let Some(shared) = named.iter().find_map(|holding| match holding {
            Holding::Asset(code, Class::Crypto)
                if context.admitted(holding)
                    && context
                        .listed
                        .values()
                        .any(|l| l.crypto && l.shared && &l.code == code) =>
            {
                Some(code)
            }
            _ => None,
        }) {
            return Err(refuse.because(
                format!("trades {shared}, a ticker several Bitpanda assets share"),
                REVIEW,
            ));
        }
        let legs = transactions
            .iter()
            .map(|tx| leg(tx, &refuse))
            .collect::<Result<Vec<_>, _>>()?;
        relevant.push(Operation {
            id,
            kind: at(operation, "operation_type"),
            legs,
        });
    }
    // Every imported leg per wallet, for withdrawal fee checks.
    let mut ledger = Ledger::new();
    for operation in &relevant {
        for leg in operation
            .legs
            .iter()
            .filter(|l| context.admitted(&l.holding))
        {
            if let Some(wallet) = &leg.wallet {
                ledger.entry(wallet.clone()).or_default().push((
                    leg.occurred,
                    leg.balance_after.clone(),
                    leg.id.clone(),
                ));
            }
        }
    }
    let mut records = vec![];
    for operation in &relevant {
        let mapped = classify(operation, context, &ledger)?;
        if mapped.is_empty() {
            internal += 1;
        }
        records.extend(mapped);
    }
    Ok(Fetched {
        records,
        tally: Some(tally),
        reported: Some(reported(&relevant, context.btc)),
        skipped: Some(dict([
            ("fiat_only_operations", Py::from(fiat_only)),
            ("non_btc_operations", Py::from(other)),
            ("internal_btc_moves", Py::from(internal)),
        ])),
    })
}

/// Bitpanda's BTC balance: over its BTC wallets, the `asset_balance_after`
/// of each wallet's latest BTC transaction. The whole history was read, so
/// that is the current balance; it is unknown, never guessed, when a leg
/// names no wallet, a latest leg lacks a whole-msat BTC balance, or two legs
/// at a wallet's latest time disagree.
fn reported(relevant: &[Operation<'_>], btc: &str) -> Reported {
    let mut latest: BTreeMap<&str, (i64, Vec<&Leg<'_>>)> = BTreeMap::new();
    let legs = relevant
        .iter()
        .flat_map(|operation| &operation.legs)
        .filter(|leg| leg.holding.is_btc());
    for leg in legs {
        let Some(wallet) = leg.wallet.as_deref() else {
            return Reported::Unavailable("a BTC transaction names no Bitpanda wallet");
        };
        let entry = latest.entry(wallet).or_insert((leg.occurred, vec![]));
        if leg.occurred > entry.0 {
            *entry = (leg.occurred, vec![leg]);
        } else if leg.occurred == entry.0 {
            entry.1.push(leg);
        }
    }
    if latest.is_empty() {
        return Reported::Unavailable("Bitpanda listed no BTC transactions");
    }
    let mut total = 0i64;
    for (_, legs) in latest.values() {
        let mut balance: Option<&Decimal> = None;
        for leg in legs {
            let raw = at(leg.raw, "asset_balance_after");
            let whole = fraction_digits(at(raw, "value")) <= BTC_DECIMALS;
            let foreign = id(at(raw, "asset_id")).is_some_and(|asset| asset != btc)
                || id(at(raw, "currency_id")).is_some();
            let (Some(value), true, false) = (&leg.balance_after, whole, foreign) else {
                return Reported::Unavailable(
                    "a BTC wallet's latest transaction has no whole-msat BTC balance",
                );
            };
            match balance {
                Some(seen) if seen != value => {
                    return Reported::Unavailable(
                        "a BTC wallet's latest transactions show different balances",
                    )
                }
                _ => balance = Some(value),
            }
        }
        let msat = balance.map(crate::file_import::msat);
        let Some(Ok(msat)) = msat else {
            return Reported::Unavailable(
                "a BTC wallet's latest transaction has no whole-msat BTC balance",
            );
        };
        let Some(sum) = total.checked_add(msat) else {
            return Reported::Unavailable("Bitpanda's BTC balances overflow");
        };
        total = sum;
    }
    Reported::Msat(total)
}

const REVIEW: &str = "Kassiber imports plain buys, sells, swaps, deposits, withdrawals and rewards from Bitpanda; record this operation yourself (generic ledger import) after checking it in Bitpanda.";
const SCOPE: &str = "This operation involves another crypto asset. To import it, set the wallet's asset_scope to crypto (kassiber wallets update --wallet <wallet> --config '{\"asset_scope\": \"crypto\"}') and sync again; or record it yourself (generic ledger import).";

/// One operation that touches an imported asset to its records: none for a
/// move between the user's own Bitpanda wallets.
fn classify(
    operation: &Operation<'_>,
    context: &Context<'_>,
    ledger: &Ledger,
) -> Result<Vec<Py>, AppError> {
    let refuse = Refusal {
        operation: operation.id,
        context,
    };
    let kind = folded(operation.kind);
    let shown_kind = context.shown(operation.kind);
    let held: Vec<&Leg> = operation
        .legs
        .iter()
        .filter(|l| context.admitted(&l.holding))
        .collect();
    let fiat: Vec<&Leg> = operation
        .legs
        .iter()
        .filter(|l| matches!(l.holding, Holding::Fiat(_)))
        .collect();
    if operation.legs.iter().any(|l| {
        !matches!(at(l.raw, "compensates"), Py::Null)
            && !matches!(at(l.raw, "compensates"), Py::Str(text) if text.code_points().next().is_none())
    }) {
        return Err(refuse.because("reverses an earlier transaction", REVIEW));
    }
    if let Some(other) = operation
        .legs
        .iter()
        .find(|l| !context.admitted(&l.holding) && !matches!(l.holding, Holding::Fiat(_)))
    {
        let code = other.holding.code();
        let hint = match other.holding {
            Holding::Asset(_, Class::Crypto) => SCOPE,
            _ => REVIEW,
        };
        return Err(refuse.because(
            format!("(type '{shown_kind}') involves {code} alongside another asset"),
            hint,
        ));
    }
    if held.is_empty() {
        let code = named_assets(
            &operation
                .legs
                .iter()
                .map(|l| l.raw.clone())
                .collect::<Vec<_>>(),
            context,
        )
        .into_iter()
        .find(|holding| context.admitted(holding))
        .map_or_else(|| "BTC".to_owned(), |holding| holding.code().to_owned());
        return Err(refuse.because(
            format!(
                "(type '{shown_kind}') involves {code} only as a fee or alongside another asset"
            ),
            REVIEW,
        ));
    }
    // A transaction type that names a different event contradicts the operation.
    for leg in &operation.legs {
        let named = folded(at(leg.raw, "transaction_type"));
        if NAMED.contains(&named.as_str()) && MOVEMENTS.contains(&kind.as_str()) && named != kind {
            return Err(refuse.because(
                format!(
                    "has type '{shown_kind}' but transaction type '{}'",
                    context.shown(at(leg.raw, "transaction_type"))
                ),
                REVIEW,
            ));
        }
    }
    if held.iter().any(|l| l.trade.is_some()) {
        if MOVEMENTS.contains(&kind.as_str()) {
            return Err(refuse.because(
                format!("has type '{shown_kind}' but carries trade details"),
                REVIEW,
            ));
        }
        return match (held.as_slice(), fiat.as_slice()) {
            ([leg], [cash]) => trade(operation, context, &refuse, leg, cash),
            ([a, b], []) if a.holding != b.holding => swap(operation, context, &refuse, [a, b]),
            _ => {
                let code = held[0].holding.code();
                Err(refuse.because(
                    format!("(type '{shown_kind}') is not one {code} leg against one cash leg or one leg of another asset"),
                    REVIEW,
                ))
            }
        };
    }
    if !fiat.is_empty() {
        let code = held[0].holding.code();
        return Err(refuse.because(
            format!("(type '{shown_kind}') moves {code} and cash without trade details"),
            REVIEW,
        ));
    }
    if held.len() > 1 {
        // Both sides of a move between the user's own Bitpanda wallets: the
        // account's holding does not change.
        let code = held[0].holding.code();
        let total = |inbound: bool| {
            held.iter()
                .filter(|l| l.inbound == inbound)
                .try_fold(Decimal::zero(), |sum, l| sum.add(&l.amount, &CTX))
                .map_err(decimal_error)
        };
        if held
            .iter()
            .all(|l| l.holding == held[0].holding && l.fee.is_none())
            && total(true)? == total(false)?
        {
            return Ok(vec![]);
        }
        return Err(refuse.because(
            format!("(type '{shown_kind}') has several {code} transactions that do not balance"),
            REVIEW,
        ));
    }
    let leg = held[0];
    let code = leg.holding.code().to_owned();
    let (inbound, record_kind) = match kind.as_str() {
        "deposit" => (true, "deposit"),
        "withdrawal" => (false, "withdrawal"),
        "reward" => (true, "income"),
        _ => {
            return Err(refuse.because(
                format!(
                    "has unsupported type '{shown_kind}' (transaction type '{}')",
                    context.shown(at(leg.raw, "transaction_type"))
                ),
                REVIEW,
            ))
        }
    };
    if leg.inbound != inbound {
        return Err(refuse.because(
            format!("has type '{shown_kind}' but flows the other way"),
            REVIEW,
        ));
    }
    let (amount, fee) = match &leg.fee {
        None => (leg.amount.clone(), Decimal::zero()),
        Some((holding, fee)) if !inbound && *holding == leg.holding => {
            withdrawal_fee(leg, fee, ledger, &refuse)?
        }
        Some(_) => {
            return Err(refuse.because(
                format!("(type '{shown_kind}') charges a fee Kassiber cannot place"),
                REVIEW,
            ))
        }
    };
    let description = match record_kind {
        "income" => "reward",
        "deposit" => "deposit",
        _ => "withdrawal",
    };
    Ok(vec![record(Entry {
        provider: "Bitpanda",
        external_id: concat(&["bitpanda:".into(), leg.id.clone()]),
        occurred_at: Some(occurred_at(leg.occurred)?),
        inbound,
        asset: code.clone(),
        amount: PriceNumber::Finite(amount),
        fee: PriceNumber::Finite(fee),
        kind: record_kind,
        raw: evidence(operation, leg, None),
        description: Some(if leg.holding.is_btc() {
            format!("Bitpanda {description}").into()
        } else {
            format!("Bitpanda {code} {description}").into()
        }),
        pricing: None,
    })?])
}

/// The trade's reference: its trade id, else the operation id.
fn reference(operation: &Operation<'_>, leg: &Leg<'_>) -> PyStr {
    match leg.trade.map(|trade| at(trade, "trade_id")) {
        Some(Py::Str(id)) if id.code_points().next().is_some() => id.clone(),
        _ => operation.id.clone(),
    }
}

/// A fee in a third crypto asset (BEST, say): a separate outflow that cannot
/// be part of either leg, so it is its own `fee` row, unpriced. A fee in the
/// leg's own asset or in cash leaves it unknown whether the leg includes it.
fn fees(
    operation: &Operation<'_>,
    context: &Context<'_>,
    refuse: &Refusal<'_>,
    legs: &[&Leg<'_>],
) -> Result<Vec<Py>, AppError> {
    let mut rows = vec![];
    for leg in legs {
        let Some((holding, fee)) = &leg.fee else {
            continue;
        };
        let third = context.admitted(holding) && legs.iter().all(|other| other.holding != *holding);
        if !third {
            return Err(refuse.because(
                "charges a separate fee; Kassiber cannot tell whether the cash amount includes it",
                REVIEW,
            ));
        }
        rows.push(record(Entry {
            provider: "Bitpanda",
            external_id: concat(&["bitpanda:".into(), leg.id.clone(), ":fee".into()]),
            occurred_at: Some(occurred_at(leg.occurred)?),
            inbound: false,
            asset: holding.code().to_owned(),
            amount: PriceNumber::Finite(fee.clone()),
            fee: PriceNumber::Finite(Decimal::zero()),
            kind: "fee",
            raw: evidence(operation, leg, None),
            description: Some(format!("Bitpanda fee in {}", holding.code()).into()),
            pricing: None,
        })?);
    }
    Ok(rows)
}

/// A buy or sell: one asset leg with trade details against one cash leg
/// flowing the other way. The cash leg is what the user paid or received,
/// so it is the exact execution value.
fn trade(
    operation: &Operation<'_>,
    context: &Context<'_>,
    refuse: &Refusal<'_>,
    leg: &Leg<'_>,
    cash: &Leg<'_>,
) -> Result<Vec<Py>, AppError> {
    let code = leg.holding.code().to_owned();
    let Holding::Fiat(currency) = &cash.holding else {
        unreachable!("cash leg")
    };
    if !is_fiat(currency) {
        return Err(refuse.because("is quoted in a currency Kassiber does not price", REVIEW));
    }
    if leg.inbound == cash.inbound {
        return Err(refuse.because(format!("moves {code} and cash the same way"), REVIEW));
    }
    let mut rows = fees(operation, context, refuse, &[leg, cash])?;
    if cash.amount.is_zero() {
        return Err(refuse.because("has no cash value", REVIEW));
    }
    let inbound = leg.inbound;
    rows.insert(
        0,
        record(Entry {
            provider: "Bitpanda",
            external_id: concat(&["bitpanda:".into(), leg.id.clone()]),
            occurred_at: Some(occurred_at(leg.occurred)?),
            inbound,
            asset: code,
            amount: PriceNumber::Finite(leg.amount.clone()),
            fee: PriceNumber::Finite(Decimal::zero()),
            kind: if inbound { "buy" } else { "sell" },
            raw: evidence(operation, leg, Some(cash)),
            description: None,
            pricing: Some(Pricing {
                fiat_currency: currency.clone(),
                fiat_value: PriceNumber::Finite(cash.amount.clone()),
                fiat_rate: None,
                method: "bitpanda_api",
                external_ref: reference(operation, leg),
            }),
        })?,
    );
    Ok(rows)
}

/// A swap: two legs of different imported assets flowing opposite ways, one
/// of them with trade details. Each leg is its own `swap` row sharing the
/// trade's reference, so the import pairs them and the profile's country
/// decides the tax treatment. Bitpanda states no exact fiat value for a
/// swap, so both legs stay unpriced.
fn swap(
    operation: &Operation<'_>,
    context: &Context<'_>,
    refuse: &Refusal<'_>,
    legs: [&Leg<'_>; 2],
) -> Result<Vec<Py>, AppError> {
    let [a, b] = legs;
    if a.inbound == b.inbound {
        return Err(refuse.because(
            format!(
                "moves {} and {} the same way",
                a.holding.code(),
                b.holding.code()
            ),
            REVIEW,
        ));
    }
    let mut rows = fees(operation, context, refuse, &legs)?;
    let traded = if a.trade.is_some() { a } else { b };
    let reference = concat(&["bitpanda:".into(), reference(operation, traded)]);
    let (out, into) = if a.inbound { (b, a) } else { (a, b) };
    for leg in [out, into] {
        let code = leg.holding.code().to_owned();
        rows.push(swap_leg(
            Entry {
                provider: "Bitpanda",
                external_id: concat(&["bitpanda:".into(), leg.id.clone()]),
                occurred_at: Some(occurred_at(leg.occurred)?),
                inbound: leg.inbound,
                asset: code,
                amount: PriceNumber::Finite(leg.amount.clone()),
                fee: PriceNumber::Finite(Decimal::zero()),
                kind: "swap",
                raw: evidence(operation, leg, None),
                description: Some(
                    format!(
                        "Bitpanda swap {} -> {}",
                        out.holding.code(),
                        into.holding.code()
                    )
                    .into(),
                ),
                pricing: None,
            },
            &reference,
        )?);
    }
    Ok(rows)
}

/// A withdrawal's BTC fee: whether `asset_amount` includes it is not
/// documented, so the wallet's balance before and after decides. Without an
/// unambiguous previous balance the withdrawal is refused.
fn withdrawal_fee(
    leg: &Leg<'_>,
    fee: &Decimal,
    ledger: &Ledger,
    refuse: &Refusal<'_>,
) -> Result<(Decimal, Decimal), AppError> {
    let unknown = || {
        refuse.because(
            "charges a BTC fee, and its balances do not show whether the amount includes it",
            REVIEW,
        )
    };
    let (Some(wallet), Some(after)) = (&leg.wallet, &leg.balance_after) else {
        return Err(unknown());
    };
    let entries = ledger.get(wallet).ok_or_else(unknown)?;
    if entries
        .iter()
        .any(|(at, _, id)| *at == leg.occurred && *id != leg.id)
    {
        return Err(unknown());
    }
    let earlier = entries.iter().filter(|(at, _, _)| *at < leg.occurred);
    let latest = earlier
        .clone()
        .map(|(at, _, _)| *at)
        .max()
        .ok_or_else(unknown)?;
    let mut previous = earlier.filter(|(at, _, _)| *at == latest);
    let (Some((_, Some(before), _)), None) = (previous.next(), previous.next()) else {
        return Err(unknown());
    };
    let sub = |a: &Decimal, b: &Decimal| a.sub(b, &CTX).map_err(decimal_error);
    let left = sub(before, &leg.amount)?;
    if sub(&left, fee)? == *after {
        // The amount left the wallet besides the fee.
        return Ok((leg.amount.clone(), fee.clone()));
    }
    if left == *after {
        // The amount includes the fee.
        let sent = sub(&leg.amount, fee)?;
        if sent.is_positive() {
            return Ok((sent, fee.clone()));
        }
    }
    Err(unknown())
}

/// The stored provider evidence: the operation's identity, the BTC leg and,
/// for a trade, its cash leg.
fn evidence(operation: &Operation<'_>, leg: &Leg<'_>, cash: Option<&Leg<'_>>) -> Py {
    let mut items = vec![
        ("operation_id", Py::Str(operation.id.clone())),
        ("operation_type", operation.kind.clone()),
        ("transaction", leg.raw.clone()),
    ];
    if let Some(cash) = cash {
        items.push(("cash_transaction", cash.raw.clone()));
    }
    dict(items)
}
