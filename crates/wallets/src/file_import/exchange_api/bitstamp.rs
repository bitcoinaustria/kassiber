//! Bitstamp, Rust only: the API v2 (`www.bitstamp.net/api/v2/`) and the
//! RFC 4180 transaction CSV (`bitstamp/export.rs`) share one mapping, so
//! a transaction in both gets the same record and id (`bitstamp:<id>`). Reads
//! only every user transaction page, oldest first, then the BTC balance.
//! Bitstamp has no read that tells whether a key may trade or withdraw;
//! Kassiber never calls a write endpoint. Trades, deposits, withdrawals,
//! transfers between Bitstamp accounts and referral rewards of the assets
//! the wallet's asset scope keeps are imported; every other row of such an
//! asset refuses the import by name. docs/reference/bitstamp.md.
use super::*;
use assets::Scope;

const PROVIDER: &str = "Bitstamp";
/// Rows per page, Bitstamp's maximum (and its fixed size with `since_id`).
const PAGE: usize = 1000;
/// Statuses whose JSON body names Bitstamp's refusal.
const REFUSED: &[u16] = &[400, 401, 403];
const FORM: &str = "application/x-www-form-urlencoded";
/// The balance a sync reports: `total`, which must be `available + reserved`.
pub(super) const BALANCE_SCOPE: &str = "account_available_plus_reserved";
/// Keys of a user transaction that are neither amounts nor pair rates.
const META: [&str; 7] = [
    "id",
    "datetime",
    "type",
    "fee",
    "order_id",
    "self_trade",
    "self_trade_order_id",
];
/// Bitstamp's documented error codes: a refused key and the rate limits.
const KNOWN_CODES: [&str; 24] = [
    "API0001", "API0002", "API0003", "API0004", "API0005", "API0006", "API0007", "API0008",
    "API0009", "API0010", "API0011", "API0012", "API0013", "API0014", "API0015", "API0016",
    "API0017", "API0018", "API0019", "API0020", "API0021", "400.002", "400.067", "400.068",
];
const RATE_LIMITS: [&str; 3] = ["400.002", "400.067", "400.068"];
const REVIEW: &str = "Kassiber imports only deposits, withdrawals, transfers between Bitstamp accounts, referral rewards and trades from Bitstamp; record this transaction yourself (generic ledger import) after checking it in Bitstamp.";

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "bitstamp",
    provider: PROVIDER,
    reads: contract::Reads::AllAssets,
    default_url: "https://www.bitstamp.net",
    key_only: false,
    fetch,
    probe,
    skipped: Some(&["fiat_only_transactions", "non_btc_transactions"]),
    files: &[],
    shared_ids: true,
    same_second_tags: true,
};

fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    let mut events = transactions(client)?
        .iter()
        .map(event)
        .collect::<Result<Vec<_>, _>>()?;
    chronological(&mut events);
    map(&events, mapper)?;
    Ok(Some(balance(client)?))
}

/// The desktop key check: one read with each permission the sync needs
/// (`Account balance`, `User transactions`). Bitstamp does not say what
/// else a key may do.
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    balance(client)?;
    let page = read(
        client,
        "/api/v2/user_transactions/",
        "limit=1&offset=0&sort=asc",
    )?;
    if !matches!(page, Py::List(_)) {
        return Err(protocol("Bitstamp user transactions were not a JSON array"));
    }
    Ok(None)
}

fn protocol(message: &str) -> AppError {
    AppError::new("protocol_error", message.to_owned())
}

/// Every user transaction, oldest first, by Bitstamp's `since_id` cursor
/// (the documented way past the 200,000-row `offset` limit). A page's ids
/// must start at the cursor and rise; anything else could hide a row.
fn transactions(client: &Client<'_>) -> Result<Vec<Py>, AppError> {
    let (mut rows, mut since) = (vec![], 1u128);
    for _ in 0..MAX_PAGES {
        let query = format!("limit={PAGE}&since_id={since}&sort=asc");
        let Py::List(data) = read(client, "/api/v2/user_transactions/", &query)? else {
            return Err(protocol("Bitstamp user transactions were not a JSON array"));
        };
        let mut last: Option<u128> = None;
        for row in &data {
            let id = row_id(at(row, "id"))
                .ok_or_else(|| protocol("Bitstamp user transaction without an id"))?;
            let number: u128 = id.parse().map_err(|_| limit())?;
            if number < since || last.is_some_and(|last| number <= last) {
                return Err(protocol(&format!(
                    "Bitstamp returned transaction {id} out of id order; nothing was imported"
                )));
            }
            last = Some(number);
        }
        rows.extend(data.iter().cloned());
        if rows.len() > MAX_ROWS {
            return Err(limit());
        }
        match last {
            Some(last) if data.len() >= PAGE => since = last + 1,
            _ => return Ok(rows),
        }
    }
    Err(limit())
}

/// `POST /api/v2/account_balances/btc/`: `total`, which must be `available`
/// plus `reserved`, each whole msat by its own digits.
fn balance(client: &Client<'_>) -> Result<Balance, AppError> {
    let payload = read(client, "/api/v2/account_balances/btc/", "")?;
    let as_of = utc_iso(now()?.as_secs() as f64)?;
    if asset(at(&payload, "currency")) != "BTC" {
        return Err(protocol("Bitstamp balance is not the BTC balance"));
    }
    let field = |key: &str| match at(&payload, key) {
        Py::Str(text) => text
            .as_str()
            .and_then(amount)
            .filter(|a| !a.is_negative())
            .and_then(|a| exact_msat(&a)),
        _ => None,
    };
    let (Some(total), Some(available), Some(reserved)) =
        (field("total"), field("available"), field("reserved"))
    else {
        return Err(protocol("Bitstamp BTC balance was not an exact BTC amount"));
    };
    if available.checked_add(reserved) != Some(total) {
        return Err(protocol(
            "Bitstamp BTC balance total is not its available plus reserved amount",
        ));
    }
    Ok(Balance {
        msat: Ok(total),
        as_of,
        scope: BALANCE_SCOPE,
    })
}

/// One signed `POST` of a form `body` (empty: no body and no Content-Type),
/// signed afresh with a new nonce for every attempt.
fn read(client: &Client<'_>, path: &str, body: &str) -> Result<Py, AppError> {
    let (url, host, signed_path) = target(client.base, path)?;
    let (consent, deadline) = (client.consent, client.deadline);
    let response = client.http.authenticated_response(
        &url,
        consent,
        deadline,
        client.source,
        REFUSED,
        || {
            let (nonce, timestamp) = (nonce()?, now()?.as_millis().to_string());
            let content_type = if body.is_empty() { "" } else { FORM };
            let fields = [
                client.key,
                "POST",
                &host,
                &signed_path,
                content_type,
                &nonce,
                &timestamp,
                body,
            ];
            let signature = signature(client.secret, fields);
            let auth = format!("BITSTAMP {}", client.key);
            let mut request = HttpRequest::new("POST", &url).map_err(|e| e.app_error())?;
            for (name, value) in [
                ("Accept", "application/json"),
                ("X-Auth", &auth),
                ("X-Auth-Signature", &signature),
                ("X-Auth-Nonce", &nonce),
                ("X-Auth-Timestamp", &timestamp),
                ("X-Auth-Version", "v2"),
            ] {
                request = request.header(name, value).map_err(|e| e.app_error())?;
            }
            if !body.is_empty() {
                request = request
                    .header("Content-Type", FORM)
                    .map_err(|e| e.app_error())?
                    .body(body.as_bytes().to_vec());
            }
            Ok(request)
        },
    )?;
    let payload = serde_json::from_slice::<Json>(&response.body)
        .ok()
        .map(|value| exact(&value));
    if REFUSED.contains(&response.status) {
        return Err(rejected(
            Some(response.status),
            &payload.unwrap_or(Py::Null),
        ));
    }
    let payload = payload.ok_or_else(|| protocol("Bitstamp response was not JSON"))?;
    let status = match at(&payload, "status") {
        Py::Str(text) => kassiber_pytext::lower(kassiber_pytext::strip(&lossy(text))),
        _ => String::new(),
    };
    if status == "error" {
        return Err(rejected(None, &payload));
    }
    Ok(payload)
}

/// A JSON value with each number read from its exact lexeme: an integer as
/// such, anything else as its decimal text, never through `f64`.
fn exact(value: &Json) -> Py {
    match value {
        Json::Null => Py::Null,
        Json::Bool(b) => Py::Bool(*b),
        Json::String(text) => Py::Str(text.as_str().into()),
        Json::Number(number) => {
            let lexeme = number.to_string();
            match kassiber_pytext::loads(&lexeme) {
                Ok(integer @ Py::Int(_)) => integer,
                _ => Py::Str(lexeme.as_str().into()),
            }
        }
        Json::Array(items) => Py::List(items.iter().map(exact).collect()),
        Json::Object(items) => Py::Dict(
            items
                .iter()
                .map(|(key, item)| (Key::Str(key.as_str().into()), exact(item)))
                .collect(),
        ),
    }
}

/// The request URL, and the host and path the signature covers. A base URL
/// with its own path keeps it; a default port is not part of the host.
pub(super) fn target(base: &str, path: &str) -> Result<(String, String, String), AppError> {
    let invalid = || AppError::new("config_error", "Bitstamp backend URL is not an http(s) URL");
    let (scheme, rest) = base.split_once("://").ok_or_else(invalid)?;
    let default_port = match scheme {
        "https" => ":443",
        "http" => ":80",
        _ => return Err(invalid()),
    };
    let (authority, prefix) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    if authority.is_empty() || authority.contains(['@', '?', '#']) || prefix.contains(['?', '#']) {
        return Err(invalid());
    }
    let host = authority.strip_suffix(default_port).unwrap_or(authority);
    Ok((
        format!("{base}{path}"),
        host.to_owned(),
        format!("{prefix}{path}"),
    ))
}

/// Bitstamp's v2 signature: hex HMAC-SHA256, keyed with the secret, of
/// `BITSTAMP <key>`, method, host, path, query (always empty here), content
/// type, nonce, timestamp, `v2` and the body.
pub(super) fn signature(secret: &str, fields: [&str; 8]) -> String {
    let [key, method, host, path, content_type, nonce, timestamp, body] = fields;
    let message =
        format!("BITSTAMP {key}{method}{host}{path}{content_type}{nonce}{timestamp}v2{body}");
    hmac_sha256_hex(secret.as_bytes(), message.as_bytes())
}

/// A fresh lowercase 36-character nonce (UUID v4 form); Bitstamp refuses a
/// nonce it has seen in the last 300 seconds.
pub(super) fn nonce() -> Result<String, AppError> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|_| AppError::new("internal_error", "Operating system randomness unavailable"))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

/// Bitstamp's error object. Only Kassiber's own values reach the error: the
/// status and a documented code, "unknown", or null; never Bitstamp's text.
fn rejected(status: Option<u16>, payload: &Py) -> AppError {
    let code = match at(payload, "code") {
        Py::Null => Py::Null,
        Py::Str(code) => known_code(&lossy(code), &KNOWN_CODES),
        _ => s("unknown"),
    };
    let known = match &code {
        Py::Str(text) if text.as_str() != Some("unknown") => lossy(text),
        _ => String::new(),
    };
    let details = dict([
        (
            "http_status",
            status.map_or(Py::Null, |s| Py::from(i64::from(s))),
        ),
        ("bitstamp_code", code),
    ]);
    if RATE_LIMITS.contains(&known.as_str()) {
        return AppError::new("rate_limited", "Bitstamp rate limited the request")
            .retryable(true)
            .with_hint("Wait a few minutes, then sync again.")
            .with_details(details);
    }
    let hint = match known.as_str() {
        "API0002" => "Bitstamp does not allow this key from this IP address; allow it in the key's settings or remove the IP restriction.",
        "API0003" => "The key lacks a permission the sync needs: create a key with only Account balance and User transactions ticked.",
        "API0004" | "API0017" | "API0018" => "Check this computer's clock: Bitstamp refuses requests more than 150 seconds off.",
        "API0005" => "The API secret does not belong to this key; paste the secret Bitstamp showed when the key was created.",
        "API0006" => "Bitstamp has frozen the account; contact Bitstamp support.",
        _ => "Check that the key is activated (Bitstamp confirms it by email), current, and has the Account balance and User transactions permissions.",
    };
    let key_refused = matches!(status, Some(401 | 403)) || known.starts_with("API");
    let message = match (status, known.is_empty()) {
        (Some(status @ (401 | 403)), true) => {
            format!("Bitstamp refused the API key (HTTP {status})")
        }
        (Some(status @ (401 | 403)), false) => {
            format!("Bitstamp refused the API key (HTTP {status}, {known})")
        }
        _ if key_refused => format!("Bitstamp refused the API key ({known})"),
        _ => "Bitstamp refused the request".to_owned(),
    };
    let kind = if key_refused {
        "auth_error"
    } else {
        "protocol_error"
    };
    AppError::new(kind, message)
        .with_hint(hint)
        .with_details(details)
}

/// An id as canonical decimal digits: an integer, or a string of digits.
pub(super) fn row_id(value: &Py) -> Option<String> {
    let text = match value {
        Py::Int(n) => n.to_string(),
        Py::Str(text) => kassiber_pytext::strip(text.as_str()?).to_owned(),
        _ => return None,
    };
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let digits = text.trim_start_matches('0');
    let digits = if digits.is_empty() { "0" } else { digits };
    (digits.len() <= 20).then(|| digits.to_owned())
}

/// The record identity: zero-padded to 20 digits, `bitstamp:<id>` sorts as
/// Bitstamp numbers its transactions, which is the order Kassiber keeps for
/// rows that share a second (`kassiber_store::transaction_order` falls back
/// to the external id when no file position orders them).
pub(super) fn external_id(id: &str) -> String {
    format!("bitstamp:{id:0>20}")
}

/// Events oldest first: by time, then by id, as Bitstamp numbers them.
pub(super) fn chronological(events: &mut [Event]) {
    events.sort_by(|a, b| {
        (a.occurred_us, a.id.len(), &a.id).cmp(&(b.occurred_us, b.id.len(), &b.id))
    });
}

/// An exact decimal as Bitstamp writes it: optional `-`, digits, optional
/// fraction; no exponent, no `+`, no blanks.
pub(super) fn amount(text: &str) -> Option<Decimal> {
    let unsigned = text.strip_prefix('-').unwrap_or(text);
    let value = strict_decimal(unsigned, false)?;
    Some(match unsigned.len() < text.len() {
        true => value.copy_negate(),
        false => value,
    })
}

/// Digits as written (whole digits without leading zeros, every fraction
/// digit); at most 28 keep a trade's sums exact in the 32-digit context.
fn written(value: &Decimal) -> i64 {
    (value.adjusted() + 1).max(0) + (-value.exponent()).max(0)
}

/// One currency amount of a transaction, signed as it moves the account.
#[derive(Clone, Debug)]
pub(super) struct Leg {
    pub currency: String,
    pub amount: Decimal,
}

/// One Bitstamp transaction, from the API or the CSV export, read strictly.
pub(super) struct Event {
    /// The transaction id, as canonical digits.
    pub id: String,
    /// The API's numeric type; `None` for a CSV type Kassiber does not know.
    pub code: Option<u32>,
    /// The type as the source names it, for messages.
    pub label: String,
    /// UTC, microseconds since 0001-01-01.
    pub occurred_us: i64,
    /// Every non-zero currency amount.
    pub legs: Vec<Leg>,
    /// Exchange rates by (base, quote) currency.
    pub rates: Vec<(String, String, Decimal)>,
    /// `None` when the API's `fee` is absent or null: unknown, never zero.
    /// The CSV writes no fee as an empty column.
    pub fee: Option<Decimal>,
    /// The fee's currency where the source states it (the CSV); the API
    /// leaves it implied.
    pub fee_currency: Option<String>,
    pub raw: Py,
}

/// The API's numeric type, by name (Bitstamp's documented list).
pub(super) fn type_name(code: u32) -> Option<&'static str> {
    Some(match code {
        0 => "deposit",
        1 => "withdrawal",
        2 => "market trade",
        14 => "sub account transfer",
        25 => "credited with staked assets",
        26 => "sent assets to staking",
        27 => "staking reward",
        32 => "referral reward",
        33 => "settlement transfer",
        35 => "inter account transfer",
        36 => "simple buy",
        53 | 55 => "small balance conversion",
        58 => "derivatives periodic settlement",
        59 => "insurance fund claim",
        60 => "insurance fund premium",
        61 => "collateral liquidation",
        _ => return None,
    })
}

/// `YYYY-MM-DD HH:MM:SS[.ffffff]` (the API, `separator` ' ') or the CSV's
/// `YYYY-MM-DDTHH:MM:SS[.ffffff][Z|±HH:MM]`, as UTC microseconds. Neither
/// source documents a zone for a time without one; it is read as UTC.
pub(super) fn datetime(text: &str, separator: char) -> Option<i64> {
    let bytes = text.as_bytes();
    let digits =
        |from: usize, to: usize| (from..to).all(|i| bytes.get(i).is_some_and(u8::is_ascii_digit));
    let shape = [(0, 4), (5, 7), (8, 10), (11, 13), (14, 16), (17, 19)]
        .iter()
        .all(|(from, to)| digits(*from, *to))
        && [(4, b'-'), (7, b'-'), (13, b':'), (16, b':')]
            .iter()
            .all(|(i, b)| bytes.get(*i) == Some(b))
        && bytes.get(10).copied() == u8::try_from(separator).ok();
    if !shape {
        return None;
    }
    let mut rest = &text[19..];
    if let Some(fraction) = rest.strip_prefix('.') {
        let len = fraction.bytes().take_while(u8::is_ascii_digit).count();
        if !(1..=6).contains(&len) {
            return None;
        }
        rest = &fraction[len..];
    }
    let offset = |zone: &[u8]| {
        zone.len() == 6
            && matches!(zone[0], b'+' | b'-')
            && zone[3] == b':'
            && [1, 2, 4, 5].iter().all(|i| zone[*i].is_ascii_digit())
    };
    let zoned = match rest {
        "" => false,
        "Z" if separator == 'T' => true,
        zone if separator == 'T' && offset(zone.as_bytes()) => true,
        _ => return None,
    };
    let parsed = kassiber_pytext::iso_datetime(text)?;
    (zoned == parsed.offset_us.is_some()).then(|| parsed.utc_us())
}

/// A user transaction as an [`Event`]. A key that is not metadata is a
/// currency amount (`btc`, `eur`, ...) or a pair rate (`btc_eur`); each is
/// read before anything is filtered out, so a malformed field of any asset
/// refuses the row by name instead of vanishing.
fn event(row: &Py) -> Result<Event, AppError> {
    let Py::Dict(items) = row else {
        return Err(protocol("Bitstamp user transaction was not a JSON object"));
    };
    let id =
        row_id(at(row, "id")).ok_or_else(|| protocol("Bitstamp user transaction without an id"))?;
    let bad = |what: &str| malformed(PROVIDER, &id, what);
    let code = match at(row, "type") {
        Py::Int(n) => n.to_string().parse::<u32>().ok(),
        Py::Str(text) => text.as_str().and_then(|t| t.parse::<u32>().ok()),
        _ => None,
    }
    .ok_or_else(|| bad("no numeric type"))?;
    let occurred_us = match at(row, "datetime") {
        Py::Str(text) => text.as_str().and_then(|t| datetime(t, ' ')),
        _ => None,
    }
    .ok_or_else(|| bad("a date and time Kassiber cannot read"))?;
    // `None`: not an exact decimal; `Some(None)`: null.
    let number = |value: &Py| -> Option<Option<Decimal>> {
        match value {
            Py::Null => Some(None),
            Py::Str(text) => text.as_str().and_then(amount).map(Some),
            Py::Int(n) => amount(&n.to_string()).map(Some),
            _ => None,
        }
    };
    let fee = match number(at(row, "fee")) {
        Some(fee) if fee.as_ref().is_some_and(Decimal::is_negative) => {
            return Err(bad("a negative fee"))
        }
        Some(fee) => fee,
        None => return Err(bad("a fee that is not an exact decimal")),
    };
    let plain = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphanumeric());
    let (mut legs, mut rates) = (vec![], vec![]);
    for (key, value) in items {
        let Key::Str(key) = key else { continue };
        let key = lossy(key);
        if META.contains(&key.as_str()) {
            continue;
        }
        let pair = key.split_once('_').filter(|(b, q)| plain(b) && plain(q));
        if pair.is_none() && !plain(&key) {
            continue;
        }
        let name = match pair {
            Some((base, quote)) => format!("{}/{} rate", asset(&s(base)), asset(&s(quote))),
            None => format!("{} amount", asset(&s(key.as_str()))),
        };
        // A null amount or rate is unknown, not zero: it must never let the
        // row pass as BTC-only or a trade go unchecked.
        let value = match number(value) {
            Some(Some(value)) => value,
            Some(None) => return Err(bad(&format!("a {name} that is null"))),
            None => return Err(bad(&format!("a {name} that is not an exact decimal"))),
        };
        match pair {
            Some((base, quote)) => rates.push((asset(&s(base)), asset(&s(quote)), value)),
            None if !value.is_zero() => legs.push(Leg {
                currency: asset(&s(key.as_str())),
                amount: value,
            }),
            None => {}
        }
    }
    Ok(Event {
        id,
        code: Some(code),
        label: type_name(code).unwrap_or("an undocumented type").to_owned(),
        occurred_us,
        legs,
        rates,
        fee,
        fee_currency: None,
        raw: row.clone(),
    })
}

/// Both sources' events to records, in order, under the wallet's asset scope
/// (`None`: the `bitcoin` scope, naming no wallet). One that moves no asset
/// the scope keeps and charges no fee in one is skipped and counted; any
/// other is mapped or refuses the whole import: a partial custodial history
/// would misstate balance and basis.
pub(super) fn map(events: &[Event], mapper: &mut Mapper) -> Result<(), AppError> {
    let scope = mapper.scope();
    let keep = format!(
        "A wallet whose asset_scope is bitcoin keeps no other crypto asset. To import both sides, run {}, then sync or import again.",
        mapper.keep_command()
    );
    let kept = |code: &str| code == "BTC" || (scope == Scope::Crypto && !is_fiat(code));
    for event in events {
        let fee = event.fee.clone().unwrap_or_else(Decimal::zero);
        // A fee that moved nothing else names no currency (the API leaves it
        // implied): an unfamiliar balance change, never silently skipped.
        if event.legs.is_empty() && !fee.is_zero() {
            let reason = "charges a fee without moving any amount";
            return Err(refusal(PROVIDER, &event.id, &event.label, reason, REVIEW));
        }
        let mut codes: Vec<&str> = event.legs.iter().map(|l| l.currency.as_str()).collect();
        codes.extend(event.fee_currency.as_deref().filter(|_| !fee.is_zero()));
        if codes.is_empty() {
            mapper.leave(true);
            continue;
        }
        if mapper.admit(&codes) {
            for record in classify(event, &kept, &keep)? {
                mapper.push(record)?;
            }
        }
    }
    Ok(())
}

type Refuse<'a> = dyn Fn(String, &str) -> AppError + 'a;

/// One event touching a kept asset to its records: one row, or a swap's
/// two legs, plus a fee in a third kept asset as a fee-only row.
fn classify(
    event: &Event,
    kept: &dyn Fn(&str) -> bool,
    keep: &str,
) -> Result<Vec<contract::Event>, AppError> {
    let refuse = |reason: String, hint: &str| {
        let label: String = event.label.chars().take(60).collect();
        refusal(PROVIDER, &event.id, &label, reason, hint)
    };
    // An unstated fee could be any amount: no exact pricing or placing.
    let Some(fee) = &event.fee else {
        return Err(refuse("does not state its fee".into(), REVIEW));
    };
    if event
        .legs
        .iter()
        .any(|l| l.currency == "BTC" && exact_msat(&l.amount.copy_abs()).is_none())
    {
        return Err(refuse(
            "has a BTC amount that is not a whole number of millisatoshi".into(),
            REVIEW,
        ));
    }
    // A fee the export states in a third kept asset is its own fee-only row.
    let third = event.fee_currency.clone().filter(|currency| {
        !fee.is_zero() && kept(currency) && event.legs.iter().all(|l| l.currency != *currency)
    });
    let fee = &match third {
        Some(_) => Decimal::zero(),
        None => fee.clone(),
    };
    let fee_in = |expected: &str| match &event.fee_currency {
        Some(currency) if !fee.is_zero() && currency != expected => {
            let hint = if is_fiat(currency) || kept(currency) {
                REVIEW
            } else {
                keep
            };
            Err(refuse(
                format!("charges its fee in {currency}, not {expected}"),
                hint,
            ))
        }
        _ => Ok(()),
    };
    let held: Vec<&Leg> = event.legs.iter().filter(|l| kept(&l.currency)).collect();
    let others: Vec<&Leg> = event.legs.iter().filter(|l| !kept(&l.currency)).collect();
    let mut rows = match (event.code, held.as_slice()) {
        (Some(2 | 36), [first, second]) => swap(event, [first, second], fee, &refuse, &fee_in)?,
        (_, [leg]) => vec![single(
            event, leg, &others, fee, &refuse, &fee_in, kept, keep,
        )?],
        (_, []) => {
            let coin = event.fee_currency.as_deref().unwrap_or("BTC");
            let reason = format!("has a {coin} fee but no single {coin} amount");
            return Err(refuse(reason, REVIEW));
        }
        _ => {
            let coins: Vec<&str> = held.iter().map(|l| l.currency.as_str()).collect();
            let reason = format!("moves {} together", coins.join(" and "));
            return Err(refuse(reason, REVIEW));
        }
    };
    if let (Some(currency), Some(charged)) = (third, &event.fee) {
        let (paid, zero) = (charged.clone(), Decimal::zero());
        rows.push(("fee", false, currency, paid, zero, "fee", None));
    }
    let name = event.code.and_then(type_name).unwrap_or("unmapped type");
    let occurred_at = utc_iso((event.occurred_us.div_euclid(1_000_000) - 719_162 * 86_400) as f64)?;
    let id = external_id(&event.id);
    rows.into_iter()
        .map(|(suffix, inbound, asset, amount, fee, kind, pricing)| {
            // A third-asset fee is a fee-only spend of that asset.
            let (amount, fee) = match kind {
                "fee" => assets::fee_only(PriceNumber::Finite(amount)),
                _ => (PriceNumber::Finite(amount), PriceNumber::Finite(fee)),
            };
            let entry = Entry {
                provider: PROVIDER,
                external_id: match suffix {
                    "" => id.as_str().into(),
                    _ => format!("{id}:{suffix}").into(),
                },
                occurred_at: Some(occurred_at.clone().into()),
                inbound,
                asset,
                amount,
                fee,
                kind,
                raw: event.raw.clone(),
                // By type, so the API and the CSV describe a row alike.
                description: Some(format!("Bitstamp {name}").into()),
                pricing,
            };
            let event = contract::Event::from_entry(entry);
            Ok(match kind {
                "swap" => contract::Event {
                    kind: contract::Kind::Swap(id.as_str().into()),
                    ..event
                },
                _ => event,
            })
        })
        .collect()
}

/// A mapped row: its id suffix (empty: the transaction's own id),
/// direction, asset, amount, fee, kind and pricing.
type Row = (
    &'static str,
    bool,
    String,
    Decimal,
    Decimal,
    &'static str,
    Option<Pricing>,
);

/// An event with one kept asset: a movement, a referral reward, or a trade
/// against fiat.
#[allow(clippy::too_many_arguments)]
fn single(
    event: &Event,
    leg: &Leg,
    others: &[&Leg],
    fee: &Decimal,
    refuse: &Refuse<'_>,
    fee_in: &dyn Fn(&str) -> Result<(), AppError>,
    kept: &dyn Fn(&str) -> bool,
    keep: &str,
) -> Result<Row, AppError> {
    let amount = leg.amount.copy_abs();
    let inbound = leg.amount.is_positive();
    let (kind, fee, pricing) = match event.code {
        // Deposits and withdrawals, and moves between Bitstamp accounts.
        Some(code @ (0 | 1 | 14 | 35)) => {
            if let Some(other) = others.first() {
                let reason = format!("moves {} together with {}", leg.currency, other.currency);
                return Err(refuse(reason, REVIEW));
            }
            if (code == 0 && !inbound) || (code == 1 && inbound) {
                let reason = format!("moves {} the other way", leg.currency);
                return Err(refuse(reason, REVIEW));
            }
            fee_in(&leg.currency)?;
            let whole = leg.currency != "BTC" || exact_msat(fee).is_some();
            if !fee.is_zero() && (inbound || !whole) {
                let reason = format!("charges a {} fee Kassiber cannot place", leg.currency);
                return Err(refuse(reason, REVIEW));
            }
            let kind = if inbound { "deposit" } else { "withdrawal" };
            (kind, fee.clone(), None)
        }
        Some(2 | 36) => {
            let [quote] = others else {
                let reason = format!("is not {} traded against one other currency", leg.currency);
                return Err(refuse(reason, REVIEW));
            };
            if !is_fiat(&quote.currency) {
                let reason = format!(
                    "trades {} against {}, which is not a fiat currency",
                    leg.currency, quote.currency
                );
                let hint = if kept(&quote.currency) { REVIEW } else { keep };
                return Err(refuse(reason, hint));
            }
            let value = trade(event, leg, quote, fee, refuse, fee_in)?;
            let pricing = Pricing {
                fiat_currency: quote.currency.clone(),
                fiat_value: PriceNumber::Finite(value),
                fiat_rate: None,
                method: "bitstamp",
                external_ref: event.id.as_str().into(),
            };
            let kind = if inbound { "buy" } else { "sell" };
            (kind, Decimal::zero(), Some(pricing))
        }
        Some(32) if others.is_empty() && inbound && fee.is_zero() => {
            ("income", Decimal::zero(), None)
        }
        Some(32) => {
            let reason = format!("is not a plain {} credit", leg.currency);
            return Err(refuse(reason, REVIEW));
        }
        // Staking, conversions, settlements, derivatives: documented, and
        // none is a plain movement Kassiber can book.
        Some(code) if type_name(code).is_some() => {
            let reason = format!("is type {code}, which Kassiber does not import");
            return Err(refuse(reason, REVIEW));
        }
        _ => return Err(refuse("has a type Kassiber does not map".into(), REVIEW)),
    };
    Ok((
        "",
        inbound,
        leg.currency.clone(),
        amount,
        fee,
        kind,
        pricing,
    ))
}

/// A trade between two kept assets (`crypto` scope): two unpriced `swap`
/// legs. As in a fiat trade, the pair's quote amount is its base amount times
/// the rate, without the fee, which is in the quote asset: a buy's outgoing
/// quote leg pays it on top, a sale's incoming quote leg is net of it.
fn swap(
    event: &Event,
    legs: [&Leg; 2],
    fee: &Decimal,
    refuse: &Refuse<'_>,
    fee_in: &dyn Fn(&str) -> Result<(), AppError>,
) -> Result<Vec<Row>, AppError> {
    let [first, second] = legs;
    let (base, quote) = match event.rates.iter().find(|(base, quote, _)| {
        legs.iter().any(|l| l.currency == *base) && legs.iter().any(|l| l.currency == *quote)
    }) {
        Some((base, ..)) if *base == first.currency => (first, second),
        Some(_) => (second, first),
        None => return Err(refuse("has no rate between its two assets".into(), REVIEW)),
    };
    let buy = base.amount.is_positive();
    if quote.currency == "BTC" && exact_msat(fee).is_none() {
        let reason = "has a BTC fee that is not a whole number of millisatoshi".into();
        return Err(refuse(reason, REVIEW));
    }
    let value = trade(event, base, quote, fee, refuse, fee_in)?;
    let zero = Decimal::zero();
    let base = (base.currency.clone(), base.amount.copy_abs(), zero.clone());
    let quote = match buy {
        true => (quote.currency.clone(), quote.amount.copy_abs(), fee.clone()),
        false => (quote.currency.clone(), value, zero),
    };
    let (out, into) = if buy { (quote, base) } else { (base, quote) };
    Ok(vec![
        ("out", false, out.0, out.1, out.2, "swap", None),
        ("in", true, into.0, into.1, into.2, "swap", None),
    ])
}

/// A trade of `leg` against `quote` (a fiat currency, or in `crypto` scope
/// another crypto asset): the quote amount must be the leg's amount times
/// the stated rate within the rounding of their last digits (so it excludes
/// the fee), and the fee is in the quote currency. Returns what a buy costs
/// (the quote amount plus the fee) or a sale yields (minus the fee).
fn trade(
    event: &Event,
    leg: &Leg,
    quote: &Leg,
    fee: &Decimal,
    refuse: &Refuse<'_>,
    fee_in: &dyn Fn(&str) -> Result<(), AppError>,
) -> Result<Decimal, AppError> {
    let e = decimal_error;
    let buy = leg.amount.is_positive();
    if buy == quote.amount.is_positive() {
        let reason = format!("moves {} and {} the same way", leg.currency, quote.currency);
        return Err(refuse(reason, REVIEW));
    }
    let Some((_, _, rate)) = event
        .rates
        .iter()
        .find(|(base, q, _)| *base == leg.currency && *q == quote.currency)
    else {
        return Err(refuse(
            format!("has no {} exchange rate", leg.currency),
            REVIEW,
        ));
    };
    fee_in(&quote.currency)?;
    if [&quote.amount, fee, rate].iter().any(|v| written(v) > 28) {
        let reason = format!(
            "has a {} amount, rate or fee with more digits than Kassiber holds exactly",
            quote.currency
        );
        return Err(refuse(reason, REVIEW));
    }
    let unit = |v: &Decimal| format!("1E{}", v.exponent()).parse::<Decimal>().map_err(e);
    let (btc, cash) = (leg.amount.copy_abs(), quote.amount.copy_abs());
    let expected = btc.mul(rate, &CTX).map_err(e)?;
    let off = |cash: &Decimal| -> Result<Decimal, AppError> {
        Ok(cash.sub(&expected, &CTX).map_err(e)?.copy_abs())
    };
    let tolerance = unit(&quote.amount)?
        .add(&btc.mul(&unit(rate)?, &CTX).map_err(e)?, &CTX)
        .map_err(e)?;
    if off(&cash)?.rp2_gt(&tolerance, &CTX).map_err(e)? {
        let reason = format!(
            "has a {} amount that is not its {} amount times its rate",
            quote.currency, leg.currency
        );
        return Err(refuse(reason, REVIEW));
    }
    let (with_fee, without) = match buy {
        true => (cash.add(fee, &CTX), cash.sub(fee, &CTX)),
        false => (cash.sub(fee, &CTX), cash.add(fee, &CTX)),
    };
    // A fee within the tolerance fits both readings (the fiat amount with or
    // without it), so its exact value is unknown.
    if !fee.is_zero()
        && !off(&without.map_err(e)?)?
            .rp2_gt(&tolerance, &CTX)
            .map_err(e)?
    {
        let reason = format!(
            "has a fee too small to tell whether its {} amount includes it",
            quote.currency
        );
        return Err(refuse(reason, REVIEW));
    }
    let value = with_fee.map_err(e)?;
    if !value.is_positive() {
        return Err(refuse("has a fee that exceeds its proceeds".into(), REVIEW));
    }
    Ok(value)
}
