//! Bitpanda's public API (`api.public.bitpanda.com`), Rust only: Python has
//! no reference importer. Reads only: a key that can trade is refused, then
//! BTC's asset id, the fiat currencies and every `/v1/operations` page are
//! read and mapped. Bitpanda does not document its operation and transaction
//! type values, so BTC legs are mapped by structure where structure decides
//! and fail closed otherwise. docs/reference/bitpanda.md records the API, the
//! mapping and every refusal.
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

pub(super) struct Fetched {
    pub records: Vec<Py>,
    /// Operations left out on purpose, by reason; reported in the envelope.
    pub skipped: Py,
}

pub(super) fn fetch(client: &Client<'_>) -> Result<Fetched, AppError> {
    refuse_trading_keys(client)?;
    let mut budget = Budget::default();
    let btc = btc_asset(&pages(
        client,
        "/v1/assets",
        &[("symbol", "BTC")],
        &mut budget,
    )?)?;
    let currencies = currencies(&read(client, "/v1/currencies")?)?;
    let operations = pages(client, "/v1/operations", &[], &mut budget)?;
    normalize(&operations, &btc, &currencies, &[client.key, client.secret])
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
    Btc,
    /// A fiat currency code (empty when the id is not a listed currency).
    Fiat(String),
    /// Any other asset: altcoins, BEST, metals, stocks, ETFs, indices.
    Other,
    Unknown,
}

struct Scope<'a> {
    btc: &'a str,
    currencies: &'a BTreeMap<String, String>,
    secrets: &'a [&'a str],
}
impl Scope<'_> {
    fn holding(&self, amount: &Py) -> Holding {
        if let Some(asset) = id(at(amount, "asset_id")) {
            return if asset == self.btc {
                Holding::Btc
            } else {
                Holding::Other
            };
        }
        match id(at(amount, "currency_id")) {
            Some(currency) => {
                Holding::Fiat(self.currencies.get(&currency).cloned().unwrap_or_default())
            }
            None => Holding::Unknown,
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
    context: &'a Scope<'a>,
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
    if holding == Holding::Btc
        && (amount.is_zero() || fraction_digits(at(amount_object, "value")) > BTC_DECIMALS)
    {
        return Err(refuse.malformed("a BTC amount that is zero or finer than a millisatoshi"));
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

/// Whether an operation touches BTC anywhere: a leg, its own asset or a fee.
fn touches_btc(transactions: &[Py], btc: &str) -> bool {
    transactions.iter().any(|tx| {
        [
            at(tx, "asset_id"),
            at(at(tx, "asset_amount"), "asset_id"),
            at(at(tx, "fee_amount"), "asset_id"),
        ]
        .into_iter()
        .any(|value| id(value).as_deref() == Some(btc))
    })
}

/// Every BTC leg's time, balance after and id, by Bitpanda wallet.
type Ledger = BTreeMap<String, Vec<(i64, Option<Decimal>, PyStr)>>;

/// A BTC-touching operation, read and checked as a whole.
struct Operation<'a> {
    id: &'a PyStr,
    kind: &'a Py,
    legs: Vec<Leg<'a>>,
}

pub(super) fn normalize(
    operations: &[Py],
    btc: &str,
    currencies: &BTreeMap<String, String>,
    secrets: &[&str],
) -> Result<Fetched, AppError> {
    let context = Scope {
        btc,
        currencies,
        secrets,
    };
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
    let mut relevant = vec![];
    for (id, operation) in unique {
        let refuse = Refusal {
            operation: id,
            context: &context,
        };
        let Py::List(transactions) = at(operation, "transactions") else {
            return Err(refuse.malformed("no transaction list"));
        };
        if !touches_btc(transactions, btc) {
            let fiat = !transactions.is_empty()
                && transactions
                    .iter()
                    .all(|tx| matches!(context.holding(at(tx, "asset_amount")), Holding::Fiat(_)));
            if fiat {
                fiat_only += 1;
            } else {
                other += 1;
            }
            continue;
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
    // Every BTC leg per wallet, oldest first, for withdrawal fee checks.
    let mut ledger = Ledger::new();
    for operation in &relevant {
        for leg in operation.legs.iter().filter(|l| l.holding == Holding::Btc) {
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
        match classify(operation, &context, &ledger)? {
            Some(record) => records.push(record),
            None => internal += 1,
        }
    }
    Ok(Fetched {
        records,
        skipped: dict([
            ("fiat_only_operations", Py::from(fiat_only)),
            ("non_btc_operations", Py::from(other)),
            ("internal_btc_moves", Py::from(internal)),
        ]),
    })
}

const REVIEW: &str = "Kassiber imports only plain BTC buys, sells, deposits, withdrawals and rewards from Bitpanda; record this operation yourself (generic ledger import) after checking it in Bitpanda.";

/// One BTC-touching operation to its record, `None` for an internal move.
fn classify(
    operation: &Operation<'_>,
    context: &Scope<'_>,
    ledger: &Ledger,
) -> Result<Option<Py>, AppError> {
    let refuse = Refusal {
        operation: operation.id,
        context,
    };
    let kind = folded(operation.kind);
    let shown_kind = context.shown(operation.kind);
    let btc: Vec<&Leg> = operation
        .legs
        .iter()
        .filter(|l| l.holding == Holding::Btc)
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
    if btc.is_empty() || btc.len() + fiat.len() != operation.legs.len() {
        return Err(refuse.because(
            format!("(type '{shown_kind}') involves BTC only as a fee or alongside another asset"),
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
    if btc.iter().any(|l| l.trade.is_some()) {
        return trade(operation, &refuse, &btc, &fiat, &kind, &shown_kind).map(Some);
    }
    if !fiat.is_empty() {
        return Err(refuse.because(
            format!("(type '{shown_kind}') moves BTC and cash without trade details"),
            REVIEW,
        ));
    }
    if btc.len() > 1 {
        // Both sides of a move between the user's own Bitpanda wallets: the
        // account's BTC does not change.
        let total = |inbound: bool| {
            btc.iter()
                .filter(|l| l.inbound == inbound)
                .try_fold(Decimal::zero(), |sum, l| sum.add(&l.amount, &CTX))
                .map_err(decimal_error)
        };
        if btc.iter().all(|l| l.fee.is_none()) && total(true)? == total(false)? {
            return Ok(None);
        }
        return Err(refuse.because(
            format!("(type '{shown_kind}') has several BTC transactions that do not balance"),
            REVIEW,
        ));
    }
    let leg = btc[0];
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
        Some((Holding::Btc, fee)) if !inbound => withdrawal_fee(leg, fee, ledger, &refuse)?,
        Some(_) => {
            return Err(refuse.because(
                format!("(type '{shown_kind}') charges a fee Kassiber cannot place"),
                REVIEW,
            ))
        }
    };
    record(Entry {
        provider: "Bitpanda",
        external_id: concat(&["bitpanda:".into(), leg.id.clone()]),
        occurred_at: Some(occurred_at(leg.occurred)?),
        inbound,
        asset: "BTC".into(),
        amount: PriceNumber::Finite(amount),
        fee: PriceNumber::Finite(fee),
        kind: record_kind,
        raw: evidence(operation, leg, None),
        description: Some(
            match record_kind {
                "income" => "Bitpanda reward",
                "deposit" => "Bitpanda deposit",
                _ => "Bitpanda withdrawal",
            }
            .into(),
        ),
        pricing: None,
    })
    .map(Some)
}

/// A BTC buy or sell: one BTC leg with trade details against one cash leg
/// flowing the other way. The cash leg is what the user paid or received,
/// so it is the exact execution value; a fee on either leg is ambiguous.
fn trade(
    operation: &Operation<'_>,
    refuse: &Refusal<'_>,
    btc: &[&Leg],
    fiat: &[&Leg],
    kind: &str,
    shown_kind: &str,
) -> Result<Py, AppError> {
    let ([leg], [cash]) = (btc, fiat) else {
        return Err(refuse.because(
            format!("(type '{shown_kind}') is not one BTC leg against one cash leg"),
            REVIEW,
        ));
    };
    if MOVEMENTS.contains(&kind) {
        return Err(refuse.because(
            format!("has type '{shown_kind}' but carries trade details"),
            REVIEW,
        ));
    }
    let Holding::Fiat(currency) = &cash.holding else {
        unreachable!("cash leg")
    };
    if !is_fiat(currency) {
        return Err(refuse.because("is quoted in a currency Kassiber does not price", REVIEW));
    }
    if leg.inbound == cash.inbound {
        return Err(refuse.because("moves BTC and cash the same way", REVIEW));
    }
    if leg.fee.is_some() || cash.fee.is_some() {
        return Err(refuse.because(
            "charges a separate fee; Kassiber cannot tell whether the cash amount includes it",
            REVIEW,
        ));
    }
    if cash.amount.is_zero() {
        return Err(refuse.because("has no cash value", REVIEW));
    }
    let trade = leg.trade.expect("trade details");
    let reference = match at(trade, "trade_id") {
        Py::Str(id) if id.code_points().next().is_some() => id.clone(),
        _ => operation.id.clone(),
    };
    let inbound = leg.inbound;
    record(Entry {
        provider: "Bitpanda",
        external_id: concat(&["bitpanda:".into(), leg.id.clone()]),
        occurred_at: Some(occurred_at(leg.occurred)?),
        inbound,
        asset: "BTC".into(),
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
            external_ref: reference,
        }),
    })
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
