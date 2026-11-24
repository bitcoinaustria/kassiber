//! Bitcoin.de, Rust only: the Trading API v4 (`api.bitcoin.de/v4`) and the
//! BTC account statement CSV share one mapping ([`records`]), so a booking
//! in both is stored once. The API refuses a key granted anything but reads,
//! then reads the BTC statement and the BTC/EUR trades. Only trades have an
//! id; every other booking is named by its second and the running balance
//! before and after it, which must chain from zero in one possible order.
//! docs/reference/bitcoinde.md.
use super::*;
use kassiber_store::rusqlite::OptionalExtension;
use std::cell::Cell;
use std::collections::BTreeSet;
use std::sync::Mutex;

mod export;

const PROVIDER: &str = "Bitcoin.de";
/// The MD5 of the empty POST parameter string every GET signs.
const EMPTY_MD5: &str = "d41d8cd98f00b204e9800998ecf8427e";
/// Bitcoin.de opened in 2011; the statement otherwise starts ten days ago.
const SINCE: &str = "2011-01-01T00:00:00+01:00";
/// Statuses Bitcoin.de answers with an `errors` list.
const ANSWERED: &[u16] = &[400, 401, 403, 404, 422];
/// The longest wait for credits (one regained per idle second).
const MAX_CREDIT_WAIT: i64 = 120;
const REQUIRED: [&str; 3] = ["showAccountLedger", "showMyTrades", "showPermissions"];
/// Every documented v4 read; a key granted any other method is refused.
const READ_ONLY: &str = "listAddressPool showAccountInfo showAccountLedger showDeposit showDeposits showMyOrderDetails showMyOrders showMyTradeDetails showMyTrades showOrderDetails showOrderbook showOrderbookCompact showPermissions showPublicTradeHistory showRates showWithdrawal showWithdrawalMinNetworkFee showWithdrawals";
/// Error codes that mean the key itself was refused.
const KEY_REFUSED: [i64; 10] = [1, 2, 3, 5, 9, 10, 32, 33, 46, 94];
const GRANT: &str = "Create a Bitcoin.de API key with only showPermissions, showAccountLedger and showMyTrades enabled, store it in this backend, and delete the old key.";
const KEY_HINT: &str = "Check that the key and secret are current, the key is active, and it is allowed from this IP address.";
const REVIEW: &str = "Kassiber imports only plain BTC/EUR trades, BTC deposits, payouts with their network fees, and affiliate, kickback and welcome credits from Bitcoin.de; record this booking yourself (generic ledger import, with its fiat value) after checking it in Bitcoin.de.";
const GAP: &str = "Download the BTC statement with type all for the whole period, or sync again; a running balance that does not follow means bookings are missing.";
/// The last nonce this process sent, held while a request is signed, sent
/// and retried, so nonces reach Bitcoin.de in order. Taken only through
/// [`Nonce::lock`], so a thread knows whether it holds it.
static NONCE: Mutex<u64> = Mutex::new(0);
thread_local! {
    /// Whether this thread holds [`NONCE`]: a credit wait never may, or
    /// every other Bitcoin.de sync would wait with it.
    static HOLDS_NONCE: Cell<bool> = const { Cell::new(false) };
    /// The credit waits this thread went through, in seconds, each with
    /// whether it held [`NONCE`] meanwhile.
    #[cfg(test)]
    pub(super) static CREDIT_WAITS: std::cell::RefCell<Vec<(u64, bool)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
/// [`NONCE`], marked as held by this thread until dropped.
struct Nonce(std::sync::MutexGuard<'static, u64>);
impl Nonce {
    fn lock() -> Self {
        let guard = NONCE.lock().unwrap_or_else(|e| e.into_inner());
        HOLDS_NONCE.set(true);
        Self(guard)
    }
}
impl Drop for Nonce {
    fn drop(&mut self) {
        HOLDS_NONCE.set(false);
    }
}

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "bitcoinde",
    provider: PROVIDER,
    reads: contract::Reads::BitcoinOnly,
    default_url: "https://api.bitcoin.de",
    key_only: false,
    fetch,
    probe,
    skipped: None,
    files: &[contract::registry::FileFormat {
        format: Format::Bitcoinde,
        reads: contract::Reads::BitcoinOnly,
        load: export::load,
    }],
    shared_ids: true,
    same_second_tags: false,
    after_insert: None,
    before_insert: Some(bind),
    after_sync: Some(record_run),
};

/// The BTC statement and the BTC/EUR trades, in either asset scope: they
/// name no other coin, and Kassiber reads only BTC from Bitcoin.de.
fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    let api = Api(client, Cell::new(None));
    api.check_key()?;
    let since = [("datetime_start", SINCE)];
    let ledger = api.pages("/v4/btc/account/ledger", &since, "account_ledger")?;
    let trades = api.pages("/v4/btceur/trades", &[("state", "1")], "trades")?;
    let fetched = normalize(&ledger, &trades)?;
    if mapper.scope() == assets::Scope::Crypto {
        // The endpoints name no other coin, so nothing can be counted.
        mapper.notice("Kassiber reads only the BTC statement from Bitcoin.de, in either asset scope; record other coins yourself (generic CSV/JSON import)".into());
    }
    mapper.counted(fetched.skipped);
    for event in fetched.records {
        mapper.push(event)?;
    }
    Ok(fetched.balance)
}

/// The desktop key check: the permission check, then one statement page.
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    let api = Api(client, Cell::new(None));
    api.check_key()?;
    api.get("/v4/btc/account/ledger", &[], 3)
        .map(|_| Some(true))
}

fn protocol(message: &str) -> AppError {
    AppError::new("protocol_error", message.to_owned())
}
fn whole(value: &Py) -> Option<i64> {
    match value {
        Py::Int(n) => i64::try_from(n.clone()).ok(),
        _ => None,
    }
}

/// One sync's reads, with the credits the last answer reported and when.
struct Api<'c, 'a>(&'c Client<'a>, Cell<Option<(i64, Instant)>>);
impl Api<'_, '_> {
    /// `showPermissions`: refuses a key that may call anything but reads,
    /// or that lacks a read the sync needs.
    fn check_key(&self) -> Result<(), AppError> {
        let payload = self.get("/v4/permissions", &[], 2)?;
        let Py::List(granted) = at(&payload, "permissions") else {
            return Err(protocol("Bitcoin.de permissions were not a JSON array"));
        };
        let methods = granted
            .iter()
            .map(|method| match method {
                Py::Str(m) => Ok(kassiber_pytext::strip(&lossy(m)).to_owned()),
                _ => Err(protocol("Bitcoin.de listed a permission that is not text")),
            })
            .collect::<Result<BTreeSet<String>, _>>()?;
        let refuse = |message: &str, key: &str, names: Vec<String>| {
            let names = Py::List(names.into_iter().map(s).collect());
            Err(AppError::new("auth_error", message)
                .with_hint(GRANT)
                .with_details(dict([(key, names)])))
        };
        let writes: Vec<String> = methods
            .iter()
            .filter(|m| !READ_ONLY.split(' ').any(|read| read == m.as_str()))
            .map(|m| m.chars().take(80).collect())
            .collect();
        if !writes.is_empty() {
            let message = "This Bitcoin.de API key can do more than read; Kassiber only accepts read-only keys";
            return refuse(message, "permissions", writes);
        }
        let missing: Vec<String> = REQUIRED
            .into_iter()
            .filter(|m| !methods.contains(*m))
            .map(str::to_owned)
            .collect();
        if !missing.is_empty() {
            let message = "This Bitcoin.de API key cannot read the account statement and trades";
            return refuse(message, "missing_permissions", missing);
        }
        Ok(())
    }

    /// One signed GET as a JSON object without errors, after waiting for
    /// credits (before taking the process-wide nonce lock, so another sync
    /// is not held up by this wait). Every attempt takes the next nonce and signs
    /// `GET#URL#KEY#NONCE#MD5` (hex HMAC-SHA256 with the secret).
    fn get(&self, path: &str, query: &[(&str, &str)], cost: i64) -> Result<Py, AppError> {
        let client = self.0;
        let mut url = format!("{}{path}", client.base);
        for (at, (key, value)) in query.iter().enumerate() {
            let separator = if at == 0 { '?' } else { '&' };
            url.push_str(&format!("{separator}{key}={}", bitpanda::escape(value)));
        }
        self.wait_for(cost)?;
        let mut last = Nonce::lock();
        let nonce = Cell::new(*last.0);
        let response = client.signed_response(&url, ANSWERED, || {
            let next = u64::try_from(now()?.as_millis()).unwrap_or(u64::MAX);
            nonce.set(next.max(nonce.get().saturating_add(1)));
            let sent = nonce.get().to_string();
            let signed = format!("GET#{url}#{}#{sent}#{EMPTY_MD5}", client.key);
            let signature = hmac_sha256_hex(client.secret.as_bytes(), signed.as_bytes());
            Ok(vec![
                ("Accept", "application/json".to_owned()),
                ("X-API-KEY", client.key.to_owned()),
                ("X-API-SIGNATURE", signature),
                ("X-API-NONCE", sent),
            ])
        });
        *last.0 = nonce.get();
        drop(last);
        let response = response?;
        let payload = std::str::from_utf8(&response.body)
            .ok()
            .and_then(exact_json);
        if let Some(credits) = payload.as_ref().and_then(|p| whole(at(p, "credits"))) {
            self.1.set(Some((credits, Instant::now())));
        }
        if ANSWERED.contains(&response.status) {
            return Err(refused(response.status, &payload.unwrap_or(Py::Null)));
        }
        let payload = payload.ok_or_else(|| protocol("Bitcoin.de response was not JSON"))?;
        if !matches!(payload, Py::Dict(_)) {
            return Err(protocol("Bitcoin.de response was not a JSON object"));
        }
        match at(&payload, "errors") {
            Py::Null => Ok(payload),
            errors if *errors == Py::List(vec![]) => Ok(payload),
            _ => Err(refused(response.status, &payload)),
        }
    }

    /// Waits until the last reported credits, plus one per second since,
    /// cover `cost`; a deficit over [`MAX_CREDIT_WAIT`] is refused.
    fn wait_for(&self, cost: i64) -> Result<(), AppError> {
        let Some((credits, seen)) = self.1.get() else {
            return Ok(());
        };
        let regained = i64::try_from(seen.elapsed().as_secs()).unwrap_or(i64::MAX);
        let wait = cost.saturating_sub(credits.saturating_add(regained));
        if wait > MAX_CREDIT_WAIT {
            return Err(AppError::new(
                "rate_limited",
                format!("Bitcoin.de API credits are exhausted ({credits} left)"),
            )
            .retryable(true)
            .with_details(dict([("retry_after_seconds", Py::from(wait))])));
        }
        let wait = wait.max(0) as u64;
        debug_assert!(!HOLDS_NONCE.get(), "a credit wait holds the nonce lock");
        #[cfg(test)]
        CREDIT_WAITS.with_borrow_mut(|waits| waits.push((wait, HOLDS_NONCE.get())));
        std::thread::sleep(Duration::from_secs(wait));
        Ok(())
    }

    /// Every `field` row from page 1 until the page says it is the last.
    fn pages(&self, path: &str, query: &[(&str, &str)], field: &str) -> Result<Vec<Py>, AppError> {
        let mut rows = vec![];
        for page in 1..=MAX_PAGES as i64 {
            let number = page.to_string();
            let payload = self.get(path, &[query, &[("page", &number)]].concat(), 3)?;
            let Py::List(data) = at(&payload, field) else {
                return Err(protocol(&format!(
                    "Bitcoin.de response has no {field} list"
                )));
            };
            rows.extend(data.iter().cloned());
            if rows.len() > MAX_ROWS {
                return Err(limit());
            }
            let paging = at(&payload, "page");
            let (Some(current), Some(last)) =
                (whole(at(paging, "current")), whole(at(paging, "last")))
            else {
                return Err(protocol("Bitcoin.de page did not say which page it is"));
            };
            // An empty collection may say it has no pages at all.
            if current != page && !(data.is_empty() && last < page) {
                return Err(protocol(
                    "Bitcoin.de returned a different page than requested",
                ));
            }
            if current >= last {
                return Ok(rows);
            }
        }
        Err(limit())
    }
}

/// A refused read: the HTTP status and the first error's code as a
/// documented number or "unknown", never Bitcoin.de's text.
fn refused(status: u16, payload: &Py) -> AppError {
    let first = match at(payload, "errors") {
        Py::List(errors) => errors.first().cloned().unwrap_or(Py::Null),
        _ => Py::Null,
    };
    let documented = [1..=5, 9..=10, 12..=14, 31..=33, 46..=46, 94..=94];
    let code = known_number(at(&first, "code"), &documented);
    let (kind, what, hint) = match whole(&code) {
        Some(4) => ("auth_error", "refused the request nonce", NONCE_HINT),
        Some(14) => ("auth_error", "refused a method the sync needs", GRANT),
        Some(12 | 13) => ("upstream_error", "Trading API is unavailable", LATER),
        Some(n) if KEY_REFUSED.contains(&n) => ("auth_error", "refused the API key", KEY_HINT),
        _ if matches!(status, 401 | 403) => ("auth_error", "refused the API key", KEY_HINT),
        _ => ("protocol_error", "refused the request", UNEXPECTED),
    };
    let error = AppError::new(kind, format!("Bitcoin.de {what} (HTTP {status})")).with_hint(hint);
    let details = [
        ("http_status", Py::from(i64::from(status))),
        ("bitcoinde_error_code", code),
    ];
    error
        .with_details(dict(details))
        .retryable(kind == "upstream_error")
}
const NONCE_HINT: &str = "Another program or sync used this key at the same moment or with larger nonces, or this computer's clock is behind the one that last used the key (a nonce is the current time in milliseconds). Check that the system clock is correct, then sync again, or create a separate read-only key for Kassiber.";
const LATER: &str = "Bitcoin.de's Trading API is currently unavailable; try again later.";
const UNEXPECTED: &str =
    "Bitcoin.de answered with an error Kassiber does not expect; nothing was imported.";

/// A response body as JSON whose fractional numbers keep their written
/// digits as text, so no amount passes through a float.
fn exact_json(text: &str) -> Option<Py> {
    fn exact(value: &Json) -> Py {
        match value {
            Json::Number(n) if n.to_string().contains(['.', 'e', 'E']) => s(n.to_string()),
            Json::Array(items) => Py::List(items.iter().map(exact).collect()),
            Json::Object(map) => Py::Dict(
                map.iter()
                    .map(|(k, v)| (Key::from(k.as_str()), exact(v)))
                    .collect(),
            ),
            other => kassiber_pytext::loads(&other.to_string()).unwrap_or(Py::Null),
        }
    }
    serde_json::from_str::<Json>(text)
        .ok()
        .map(|value| exact(&value))
}

/// `-?digits(.digits)?`: no exponent, grouping, `+` or decimal comma.
pub(super) fn plain(text: &str) -> Option<Decimal> {
    strict_decimal(text.strip_prefix('-').unwrap_or(text), false)?;
    text.parse().ok()
}
/// A plain decimal string, or a JSON number by its written digits.
fn api_decimal(value: &Py) -> Option<Decimal> {
    match value {
        Py::Str(text) => plain(kassiber_pytext::strip(text.as_str()?)),
        Py::Int(_) => plain(&kassiber_envelope::py_str(value)),
        _ => None,
    }
}

/// The statement rows and the BTC/EUR trades, to records.
fn normalize(ledger: &[Py], trades: &[Py]) -> Result<Fetched, AppError> {
    let terms = trade_terms(trades)?;
    let (mut lines, mut booked) = (vec![], BTreeSet::new());
    for (index, row) in ledger.iter().enumerate() {
        let mut line = api_line(index, row)?;
        let mut raw = vec![("ledger", line.raw.clone())];
        if matches!(classify(&line.kind), Type::Buy | Type::Sell) {
            let Some((trade, projected)) = terms.get(&line.reference) else {
                let pair = match at(at(row, "trade"), "trading_pair") {
                    Py::Str(pair) => {
                        format!(" on {}", lossy(pair).chars().take(20).collect::<String>())
                    }
                    _ => String::new(),
                };
                let reason = format!("is a trade{pair} that is not in the BTC/EUR trade list");
                return Err(line.refuse(reason, REVIEW));
            };
            booked.insert(line.reference.clone());
            line.trade = Some(trade.clone());
            raw.push(("trade", projected.clone()));
        }
        line.raw = dict(raw);
        lines.push(line);
    }
    let mut fetched = records(&lines)?;
    // Trades the statement does not list yet: it lists up to the previous day.
    let pending = terms.keys().filter(|id| !booked.contains(*id)).count();
    let key = "trades_not_yet_in_statement";
    kassiber_envelope::set(&mut fetched.skipped, key, Py::from(pending as i64));
    Ok(fetched)
}

/// The BTC/EUR trades by id, with the projection Kassiber stores: the
/// trading partner's name, bank and rating stay out of the book.
fn trade_terms(trades: &[Py]) -> Result<BTreeMap<String, (Terms, Py)>, AppError> {
    let mut result: BTreeMap<String, (Terms, Py)> = BTreeMap::new();
    for trade in trades {
        let word = |key: &str| match at(trade, key) {
            Py::Str(text) => kassiber_pytext::lower(kassiber_pytext::strip(&lossy(text))),
            _ => String::new(),
        };
        let id = match at(trade, "trade_id") {
            Py::Str(id) => kassiber_pytext::strip(&lossy(id)).to_owned(),
            _ => String::new(),
        };
        if id.is_empty() {
            return Err(protocol("Bitcoin.de trade without an id"));
        }
        let bad = |what: &str| malformed(PROVIDER, &id, what);
        let pair = word("trading_pair");
        if pair != "btceur" {
            let pair: String = pair.chars().take(20).collect();
            return Err(bad(&format!(
                "trading pair '{pair}' in the BTC/EUR trade list"
            )));
        }
        if at(trade, "is_external_wallet_trade") == &Py::Bool(true) {
            return Err(bad("an external-wallet settlement"));
        }
        if whole(at(trade, "state")) != Some(1) {
            return Err(bad("a state other than successful"));
        }
        let kind = word("type");
        if kind != "buy" && kind != "sell" {
            return Err(bad("a type other than buy or sell"));
        }
        let number = |key: &str| {
            api_decimal(at(trade, key))
                .filter(|v| !v.is_negative())
                .ok_or_else(|| bad(&format!("a {key} that is not a plain non-negative decimal")))
        };
        let terms = Terms {
            kind,
            btc_before: number("amount_currency_to_trade")?,
            btc_after: number("amount_currency_to_trade_after_fee")?,
            fiat_currency: "EUR".into(),
            fiat: number("volume_currency_to_pay_after_fee")?,
        };
        let kept = "trade_id trading_pair is_external_wallet_trade type amount_currency_to_trade price volume_currency_to_pay amount_currency_to_trade_after_fee volume_currency_to_pay_after_fee fee_currency_to_pay fee_currency_to_trade state payment_method created_at successfully_finished_at";
        let kept = kept
            .split(' ')
            .filter_map(|key| get(trade, key).map(|v| (key, v.clone())));
        let projected = dict(kept.collect::<Vec<_>>());
        // A page boundary may repeat a trade; a repeat must be identical.
        match result.get(&id) {
            Some((_, seen)) if *seen == projected => {}
            Some(_) => {
                let id: String = id.chars().take(40).collect();
                let message =
                    format!("Bitcoin.de returned trade '{id}' twice with different contents");
                return Err(protocol(&message));
            }
            None => drop(result.insert(id, (terms, projected))),
        }
    }
    Ok(result)
}

/// One statement row, read strictly.
fn api_line(index: usize, row: &Py) -> Result<Line, AppError> {
    let position = format!("account statement entry {}", index + 1);
    let bad = |what: &str| malformed(PROVIDER, &position, what);
    let text = |key: &str| match at(row, key) {
        Py::Str(value) => Some(kassiber_pytext::strip(&lossy(value)).to_owned()),
        Py::Null => Some(String::new()),
        _ => None,
    };
    let kind = text("type")
        .filter(|kind| !kind.is_empty())
        .ok_or_else(|| bad("no type"))?;
    let reference = text("reference").ok_or_else(|| bad("a reference that is not text"))?;
    let at_ = match at(row, "date") {
        Py::Str(date) => date
            .as_str()
            .and_then(kassiber_pytext::iso_datetime)
            .filter(|parsed| parsed.offset_us.is_some())
            // Microseconds since 0001-01-01 to Unix seconds.
            .map(|parsed| parsed.utc_us().div_euclid(1_000_000) - 719_162 * 86_400),
        _ => None,
    }
    .ok_or_else(|| bad("a date without a time zone"))?;
    let decimal = |key: &str| {
        api_decimal(at(row, key))
            .ok_or_else(|| bad(&format!("a {key} that is not a plain decimal")))
    };
    let (cashflow, balance) = (decimal("cashflow")?, decimal("balance")?);
    Ok(Line {
        position,
        at: at_,
        kind,
        reference,
        cashflow,
        balance,
        trade: None,
        raw: row.clone(),
    })
}

/// What one buy or sell exchanged, from the trade list (API) or the
/// statement row (CSV): the agreed BTC, the BTC after the whole fee (what a
/// buyer receives) and the cash that changed hands.
#[derive(Clone, Debug)]
pub(super) struct Terms {
    pub kind: String,
    pub btc_before: Decimal,
    pub btc_after: Decimal,
    pub fiat_currency: String,
    pub fiat: Decimal,
}

/// One statement booking, from the API or the CSV: when (UTC seconds), the
/// signed change of the BTC balance and the balance after it.
#[derive(Clone, Debug)]
pub(super) struct Line {
    /// Where the booking is (`account statement entry 3`, `CSV line 4`).
    pub position: String,
    pub at: i64,
    pub kind: String,
    pub reference: String,
    pub cashflow: Decimal,
    pub balance: Decimal,
    pub trade: Option<Terms>,
    /// The stored provider evidence.
    pub raw: Py,
}
impl Line {
    fn refuse(&self, reason: impl std::fmt::Display, hint: &str) -> AppError {
        let short = |text: &str| text.chars().take(80).collect::<String>();
        let (position, kind) = (&self.position, short(&self.kind));
        let reference = short(&self.reference);
        let message =
            format!("Bitcoin.de {position} (type '{kind}', reference '{reference}') {reason}");
        AppError::new("validation", message).with_hint(hint.to_owned())
    }
}

/// The statement's types in the API's, the German CSV's and the English
/// CSV's names: the one place to extend the vocabulary.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(super) enum Type {
    Buy,
    Sell,
    Inpayment,
    Payout,
    NetworkFee,
    Affiliate,
    Kickback,
    Welcome,
    Registration,
    /// BTC spent in a Bitcoin.de shop (YubiKey, gold, diamonds).
    Shop,
    Unknown,
}
pub(super) fn classify(kind: &str) -> Type {
    match kassiber_pytext::lower(kassiber_pytext::strip(kind)).as_str() {
        "buy" | "kauf" | "purchase" => Type::Buy,
        "sell" | "verkauf" | "sale" => Type::Sell,
        "inpayment" | "einzahlung" | "deposit" => Type::Inpayment,
        "payout" | "auszahlung" | "disbursement" => Type::Payout,
        "outgoing_fee_voluntary" | "netzwerk-gebühr" | "network fee" => Type::NetworkFee,
        "affiliate" | "partnerprogramm" | "partner programme" => Type::Affiliate,
        "kickback" => Type::Kickback,
        "welcome_btc" | "welcome btc" => Type::Welcome,
        "registrierung" | "registration" => Type::Registration,
        "buy_yubikey" | "buy_goldshop" | "buy_diamondshop" => Type::Shop,
        _ => Type::Unknown,
    }
}

pub(super) struct Fetched {
    pub records: Vec<contract::Event>,
    /// Bookings left out on purpose, by reason; reported in the envelope.
    pub skipped: Py,
    /// The balance after the newest booking.
    pub balance: Option<Balance>,
}

/// A BTC decimal as signed whole msat.
fn signed_msat(value: &Decimal) -> Option<i64> {
    exact_msat(&value.copy_abs()).map(|m| if value.is_negative() { -m } else { m })
}

/// Every booking to its record; one refused booking stops the whole import.
/// Records come out in the order the running balance proves, which the
/// import keeps as their source position.
pub(super) fn records(lines: &[Line]) -> Result<Fetched, AppError> {
    let mut trades = BTreeSet::new();
    for line in lines {
        let kind = classify(&line.kind);
        if exact_msat(&line.cashflow.copy_abs()).is_none() {
            return Err(line.refuse("has a BTC amount finer than a millisatoshi", REVIEW));
        }
        if line.cashflow.is_zero() && kind == Type::Registration {
            continue;
        }
        let inbound = match kind {
            _ if line.cashflow.is_zero() => return Err(line.refuse("moves no BTC", REVIEW)),
            Type::Buy | Type::Inpayment | Type::Affiliate | Type::Kickback | Type::Welcome => true,
            Type::Sell | Type::Payout | Type::NetworkFee => false,
            Type::Shop => {
                let reason = "spends BTC in a Bitcoin.de shop, a disposal without a fiat value";
                return Err(line.refuse(reason, REVIEW));
            }
            Type::Registration | Type::Unknown => {
                return Err(line.refuse("has a type Kassiber does not map", REVIEW))
            }
        };
        if inbound != line.cashflow.is_positive() {
            return Err(line.refuse("moves BTC the other way", REVIEW));
        }
        let credit = matches!(kind, Type::Affiliate | Type::Kickback | Type::Welcome);
        if !credit && line.reference.is_empty() {
            return Err(line.refuse("has no reference", REVIEW));
        }
        if matches!(kind, Type::Buy | Type::Sell) {
            if !trades.insert(line.reference.as_str()) {
                return Err(line.refuse("repeats a trade id", REVIEW));
            }
            trade(line, kind)?;
        }
    }
    let order = chain(lines)?;
    // Each network fee belongs to the payout of its transaction next to it.
    let mut fees: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, &(index, _, _)) in order.iter().enumerate() {
        let fee = &lines[index];
        if classify(&fee.kind) != Type::NetworkFee {
            continue;
        }
        let payout = [position.wrapping_sub(1), position + 1]
            .into_iter()
            .filter_map(|p| order.get(p).map(|step| step.0))
            .find(|&candidate| {
                let line = &lines[candidate];
                classify(&line.kind) == Type::Payout
                    && (line.reference.as_str(), line.at) == (fee.reference.as_str(), fee.at)
                    && !fees.contains_key(&candidate)
            });
        let reason = "is a network fee without a payout in the same transaction";
        fees.insert(payout.ok_or_else(|| fee.refuse(reason, REVIEW))?, index);
    }
    let (mut records, mut registrations) = (vec![], 0);
    for &(index, before, after) in &order {
        let line = &lines[index];
        let kind = classify(&line.kind);
        if line.cashflow.is_zero() {
            registrations += 1;
            continue;
        }
        let when = utc_iso(line.at as f64)?;
        let compact: String = when.chars().filter(char::is_ascii_alphanumeric).collect();
        let movement = |kind: &'static str, fee: Decimal, raw: Py, description: &str| Entry {
            provider: PROVIDER,
            external_id: format!("bitcoinde:{compact}:{before}:{after}").into(),
            occurred_at: Some(when.clone().into()),
            inbound: line.cashflow.is_positive(),
            asset: "BTC".into(),
            amount: PriceNumber::Finite(line.cashflow.copy_abs()),
            fee: PriceNumber::Finite(fee),
            kind,
            raw,
            description: Some(description.into()),
            pricing: None,
        };
        let entry = match kind {
            Type::Buy | Type::Sell => trade(line, kind)?,
            Type::NetworkFee => continue,
            Type::Payout | Type::Inpayment => {
                // The txid travels in raw_json for transfer pairing; it is
                // never the record's identity.
                let mut raw = vec![("ledger", line.raw.clone())];
                let reference = &line.reference;
                if reference.len() == 64 && reference.bytes().all(|b| b.is_ascii_hexdigit()) {
                    raw.insert(0, ("txid", s(reference.to_ascii_lowercase())));
                }
                let mut fee = Decimal::zero();
                if let Some(&paid) = fees.get(&index) {
                    fee = lines[paid].cashflow.copy_abs();
                    raw.push(("network_fee", lines[paid].raw.clone()));
                }
                match kind {
                    Type::Payout => movement("withdrawal", fee, dict(raw), "Bitcoin.de payout"),
                    _ => movement("deposit", fee, dict(raw), "Bitcoin.de deposit"),
                }
            }
            _ => {
                let description = match kind {
                    Type::Affiliate => "Bitcoin.de affiliate credit",
                    Type::Kickback => "Bitcoin.de API kickback",
                    _ => "Bitcoin.de welcome credit",
                };
                movement("income", Decimal::zero(), line.raw.clone(), description)
            }
        };
        records.push(contract::Event::from_entry(entry));
    }
    let balance = match order.last() {
        Some(&(index, _, after)) if after >= 0 => Some(Balance {
            msat: Ok(after),
            as_of: utc_iso(lines[index].at as f64)?,
            scope: "account_statement_balance_after_last_booking",
        }),
        _ => None,
    };
    let skipped = dict([("registration_rows", Py::from(registrations))]);
    Ok(Fetched {
        records,
        skipped,
        balance,
    })
}

/// The bookings in running-balance order from a zero opening balance, each
/// with its balance before and after (msat). Within one second the next
/// booking is the one starting at the current balance; none means missing
/// history, two mean the statement does not decide the order (which decides
/// lots and fees), and both refuse the import.
fn chain(lines: &[Line]) -> Result<Vec<(usize, i64, i64)>, AppError> {
    let mut seconds: BTreeMap<i64, BTreeMap<i64, Vec<(usize, i64)>>> = BTreeMap::new();
    for (index, line) in lines.iter().enumerate() {
        let before = line
            .balance
            .sub(&line.cashflow, &CTX)
            .map_err(decimal_error)?;
        let (Some(before), Some(after)) = (signed_msat(&before), signed_msat(&line.balance)) else {
            return Err(line.refuse("has a balance finer than a millisatoshi", GAP));
        };
        let second = seconds.entry(line.at).or_default();
        second.entry(before).or_default().push((index, after));
    }
    let (mut current, mut order) = (0i64, Vec::with_capacity(lines.len()));
    for (at, mut from) in seconds {
        let when = utc_iso(at as f64)?;
        let undecided = |why: &str| {
            let message = format!(
                "Bitcoin.de's statement does not decide the order of the bookings at {when}: {why}"
            );
            AppError::new("validation", message).with_hint("The order decides which lots a sale or payout uses. Record these bookings through the generic ledger in their real order; nothing was imported.")
        };
        if from.values().map(Vec::len).sum::<usize>() > 20_000 {
            return Err(undecided("more than 20000 bookings share it"));
        }
        while let Some((&start, _)) = from.first_key_value() {
            let Some(next) = from.remove(&current) else {
                let (first, _) = from[&start][0];
                if !order.is_empty() {
                    let reason = "has a balance that does not follow the booking before it";
                    return Err(lines[first].refuse(reason, GAP));
                }
                // The opening is a balance no booking of the second ends at.
                let ends: BTreeSet<i64> = from.values().flatten().map(|step| step.1).collect();
                let start = from.keys().find(|b| !ends.contains(b)).unwrap_or(&start);
                let reason = format!("starts from a balance of {start} msat, not zero: the history before it is missing");
                return Err(lines[first].refuse(reason, "Kassiber needs the whole history from the account's first booking: import the statement for the whole period, or use wallets sync-bitcoinde, which reads it all."));
            };
            let [(index, after)] = next[..] else {
                return Err(undecided("their balances allow more than one order"));
            };
            order.push((index, current, after));
            current = after;
        }
    }
    Ok(order)
}

/// A BTC/EUR buy or sell. A buyer receives the BTC after the whole fee, a
/// seller gives the agreed BTC; the cash is what changed hands.
fn trade(line: &Line, kind: Type) -> Result<Entry, AppError> {
    let Some(terms) = &line.trade else {
        return Err(line.refuse("is a trade without its trade details", REVIEW));
    };
    let (buy, word, expected) = match kind {
        Type::Buy => (true, "buy", &terms.btc_after),
        _ => (false, "sell", &terms.btc_before),
    };
    let moved = line.cashflow.copy_abs();
    let reason = if terms.kind != word {
        format!("is a {word} but its trade is a {}", terms.kind)
    } else if !is_fiat(&terms.fiat_currency) {
        format!(
            "trades BTC against {}, which is not fiat",
            terms.fiat_currency
        )
    } else if exact_msat(&moved) != exact_msat(expected) {
        "moves a different BTC amount than its trade details".into()
    } else if terms.btc_after > terms.btc_before {
        "has a fee that adds BTC".into()
    } else if !terms.fiat.is_positive() {
        "has no cash value".into()
    } else {
        String::new()
    };
    if !reason.is_empty() {
        return Err(line.refuse(reason, REVIEW));
    }
    Ok(Entry {
        provider: PROVIDER,
        external_id: format!("bitcoinde:{}", line.reference).into(),
        occurred_at: Some(utc_iso(line.at as f64)?.into()),
        inbound: buy,
        asset: "BTC".into(),
        amount: PriceNumber::Finite(moved),
        fee: PriceNumber::Finite(Decimal::zero()),
        kind: word,
        raw: line.raw.clone(),
        description: None,
        pricing: Some(Pricing {
            fiat_currency: terms.fiat_currency.clone(),
            fiat_value: PriceNumber::Finite(terms.fiat.clone()),
            fiat_rate: None,
            method: "bitcoinde_trade",
            external_ref: line.reference.as_str().into(),
        }),
    })
}

/// Append-only binding for both sources. Rows earlier Bitcoin.de runs
/// stored in `wallet` (known by their import run) must all come back
/// unchanged (identity, kind, direction, amount, fee, time), and new rows
/// must lie after the newest stored second; otherwise nothing is imported.
pub(super) fn bind(conn: &Connection, wallet: &str, records: &[Record]) -> Result<(), AppError> {
    type Booked = (String, String, String, i64, i64, String);
    let number = |fields: &Fields, key: &str| match fields.get(key) {
        Some(Sql::Integer(value)) => *value,
        _ => 0,
    };
    let incoming: BTreeSet<Booked> = records
        .iter()
        .map(|r| {
            let (f, text) = (&r.fields, |key| string(&r.fields, key));
            let (kind, direction) = (text("kind"), text("direction"));
            let (amount, fee) = (number(f, "amount"), number(f, "fee"));
            (
                text("external_id"),
                kind,
                direction,
                amount,
                fee,
                text("occurred_at"),
            )
        })
        .collect();
    let stored: Vec<Booked> = conn
        .prepare(
            "SELECT COALESCE(external_id,''), kind, direction, amount, fee, occurred_at FROM transactions \
             WHERE wallet_id=?1 AND id IN (SELECT x.transaction_id FROM import_batch_transactions x \
             JOIN import_batches b ON b.id=x.batch_id WHERE b.source_format IN ('bitcoinde_csv','bitcoinde_api'))",
        )
        .and_then(|mut statement| {
            let row = |r: &kassiber_store::rusqlite::Row<'_>| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            };
            statement.query_map([wallet], row)?.collect()
        })
        .map_err(database_error)?;
    // A priced booking must keep its stored fiat value: the CSV and the API
    // state a trade's cash differently, and a re-import never changes a
    // stored price silently.
    let price = |id: &str| -> Result<Option<String>, AppError> {
        let sql = "SELECT fiat_value_exact FROM transactions WHERE wallet_id=? AND external_id=?";
        let price = conn.query_row(sql, [wallet, id], |r| r.get::<_, Option<String>>(0));
        Ok(price.optional().map_err(database_error)?.flatten())
    };
    for record in records {
        let (id, value) = (
            string(&record.fields, "external_id"),
            string(&record.fields, "fiat_value_exact"),
        );
        let Some(old) = price(&id)?.filter(|_| !value.is_empty()) else {
            continue;
        };
        if !dec(&old)?
            .sub(&dec(&value)?, &CTX)
            .map_err(decimal_error)?
            .is_zero()
        {
            let message = format!("Bitcoin.de booking '{id}' has a fiat value of {value} where Kassiber imported {old} earlier");
            return Err(AppError::new("validation", message).with_hint(REIMPORT));
        }
    }
    let known: BTreeSet<&str> = stored.iter().map(|b| b.0.as_str()).collect();
    let newest = stored.iter().map(|b| b.5.as_str()).max();
    let changed = stored.iter().filter(|b| !incoming.contains(*b));
    let late = incoming
        .iter()
        .filter(|b| !known.contains(b.0.as_str()) && newest.is_some_and(|n| b.5.as_str() <= n));
    let Some(at) = changed.chain(late).map(|b| b.5.as_str()).min() else {
        return Ok(());
    };
    Err(AppError::new(
        "validation",
        format!("Bitcoin.de's statement differs from what Kassiber imported earlier (first difference at {at})"),
    )
    .with_hint(REIMPORT))
}
const REIMPORT: &str = "Kassiber imports a Bitcoin.de statement once and later only adds newer bookings. For a corrected statement, roll back the earlier Bitcoin.de import runs (kassiber imports list, then imports rollback) and import the full statement again; nothing was imported.";

/// An API sync is an import run of its own, as a CSV import is: its new
/// rows keep the proven order as their source position, which orders
/// same-second bookings for lot selection, and [`bind`] finds them.
pub(super) fn record_run(
    conn: &Connection,
    profile: &str,
    wallet: &str,
    result: &mut Py,
    ids: &mut Ids<'_>,
) -> Result<(), AppError> {
    let Py::List(inserted) = at(result, "inserted_records").clone() else {
        return Ok(());
    };
    if inserted.is_empty() {
        return Ok(());
    }
    let batch = ids.next(conn)?;
    conn.execute(
        "INSERT INTO import_batches(id,workspace_id,profile_id,wallet_id,source_format,source_filename,imported_at,rows_inserted,rows_updated,rows_skipped) \
         SELECT ?1,workspace_id,?2,?3,'bitcoinde_api','',?4,?5,0,0 FROM profiles WHERE id=?2",
        params![batch, profile, wallet, ids.time(conn)?, inserted.len() as i64],
    )
    .map_err(database_error)?;
    for (position, item) in (1_i64..).zip(&inserted) {
        conn.execute(
            "INSERT INTO import_batch_transactions(batch_id,transaction_id,source_position) VALUES(?,?,?)",
            params![batch, lossy(&text(at(item, "transaction_id"))), position],
        )
        .map_err(database_error)?;
    }
    kassiber_envelope::set(result, "import_batch_id", s(batch));
    Ok(())
}
