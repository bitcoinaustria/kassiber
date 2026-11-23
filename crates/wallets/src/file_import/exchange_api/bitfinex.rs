//! Bitfinex's authenticated REST API v2 (`api.bitfinex.com`), Rust only.
//! Reads only: a key with any write permission is refused, then the BTC
//! ledger, every trade, the BTC movements and the wallets are read and
//! mapped by [`mapping`], which the Reports CSV import shares.
//!
//! Bitfinex refuses a nonce not larger than the key's last one. This
//! process signs one request at a time, each with a larger nonce; another
//! program using the same key at once can still send larger ones, and the
//! sync then stops on Bitfinex's nonce error without importing anything.
//! docs/reference/bitfinex.md.
use super::*;
use bitcoin::hashes::sha384;
use std::sync::Mutex;

mod export;
pub(super) mod mapping;
use mapping::{map, Sources};

/// Statuses Bitfinex answers its `["error", code, message]` arrays with.
const ERRORS: &[u16] = &[400, 401, 403, 404, 500];
/// The error codes Bitfinex documents for its REST API (docs.bitfinex.com,
/// "Error codes").
const ERROR_CODES: [std::ops::RangeInclusive<i64>; 5] = [
    10000..=10001,
    10020..=10020,
    10100..=10100,
    10111..=10114,
    11000..=11000,
];
/// The last nonce this process sent; held from signing to the answer.
static NONCE: Mutex<u64> = Mutex::new(0);

pub(super) static EXCHANGE: Descriptor = Descriptor {
    kind: "bitfinex",
    provider: "Bitfinex",
    reads: contract::Reads::BitcoinOnly,
    default_url: "https://api.bitfinex.com",
    key_only: false,
    fetch,
    probe,
    skipped: None,
    files: &[contract::registry::FileFormat {
        format: Format::Bitfinex,
        reads: contract::Reads::BitcoinOnly,
        load: export::load,
    }],
    shared_ids: true,
    same_second_tags: false,
    after_insert: Some(mapping::tag_unchecked),
};

fn read<'r, T>(
    rows: &'r [Py],
    what: &'static str,
    length: usize,
    as_row: impl Fn(&Api<'r>) -> Result<T, AppError>,
) -> Result<Vec<T>, AppError> {
    rows.iter()
        .map(|row| as_row(&Api::of(row, what, length)?))
        .collect()
}

/// The BTC ledger, every trade and the BTC movements, in either asset scope:
/// Kassiber reads only BTC from Bitfinex, and other assets are counted.
fn fetch(client: &Client<'_>, mapper: &mut Mapper) -> Result<Option<Balance>, AppError> {
    refuse_write_keys(client)?;
    let ledger = history(client, "ledgers/BTC/hist", 2500, 3, "")?;
    let trades = history(client, "trades/hist", 2500, 2, r#","sort":-1"#)?;
    let movements = history(client, "movements/BTC/hist", 1000, 6, "")?;
    let sources = Sources {
        ledger: Some(&read(&ledger, "ledger", 9, mapping::read_ledger)?),
        trades: Some(&read(&trades, "trade", 11, mapping::read_trade)?),
        movements: Some(&read(&movements, "movement", 21, mapping::read_movement)?),
        spot_proof_required: false,
    };
    map(&sources, mapper)?;
    // Read last, so the balance is never older than the history.
    let wallets = post(client, "v2/auth/r/wallets", "{}")?;
    Ok(Some(balance(&wallets, utc_iso(now()?.as_secs() as f64)?)?))
}

/// The desktop key check: the permission check, then one ledger entry and
/// the wallets, the two permissions the sync reads with.
fn probe(client: &Client<'_>) -> Result<Option<bool>, AppError> {
    refuse_write_keys(client)?;
    let ledger = post(client, "v2/auth/r/ledgers/BTC/hist", r#"{"limit":1}"#)?;
    match (ledger, post(client, "v2/auth/r/wallets", "{}")?) {
        (Py::List(_), Py::List(_)) => Ok(Some(true)),
        _ => Err(protocol("Bitfinex response was not a JSON array")),
    }
}

fn protocol(message: &str) -> AppError {
    AppError::new("protocol_error", message.to_owned())
}

/// `bfx-signature`: hex HMAC-SHA384(secret, `/api/` ‖ path ‖ nonce ‖ body).
pub(super) fn signature(secret: &str, path: &str, nonce: &str, body: &str) -> String {
    let mut engine = hmac::HmacEngine::<sha384::Hash>::new(secret.as_bytes());
    for part in ["/api/", path, nonce, body] {
        engine.input(part.as_bytes());
    }
    hmac::Hmac::<sha384::Hash>::from_engine(engine).to_string()
}

/// One signed POST of `body` to `/{path}`, every JSON number kept as the
/// text Bitfinex wrote. Each attempt (the shared transport retries a
/// throttled one) signs with a nonce above any this process sent, the
/// microseconds since the epoch at least.
fn post(client: &Client<'_>, path: &str, body: &str) -> Result<Py, AppError> {
    let url = format!("{}/{path}", client.base);
    let mut last = NONCE.lock().unwrap_or_else(|e| e.into_inner());
    let sent = std::cell::Cell::new(*last);
    let (http, consent, deadline) = (&client.http, client.consent, client.deadline);
    let response =
        http.authenticated_response(&url, consent, deadline, client.source, ERRORS, || {
            let micros = u64::try_from(now()?.as_micros()).unwrap_or(u64::MAX);
            sent.set(micros.max(sent.get().saturating_add(1)));
            let nonce = sent.get().to_string();
            let signed = signature(client.secret, path, &nonce, body);
            let mut request = HttpRequest::new("POST", &url).map_err(|e| e.app_error())?;
            for (name, value) in [
                ("Accept", "application/json"),
                ("Content-Type", "application/json"),
                ("bfx-nonce", &nonce),
                ("bfx-apikey", client.key),
                ("bfx-signature", &signed),
            ] {
                request = request.header(name, value).map_err(|e| e.app_error())?;
            }
            Ok(request.body(body.as_bytes().to_vec()))
        });
    *last = sent.get();
    drop(last);
    let response = response?;
    let payload = serde_json::from_slice::<Json>(&response.body)
        .ok()
        .map(|json| exact(&json));
    let status = Py::from(i64::from(response.status));
    let details = |code: Py| dict([("http_status", status.clone()), ("bitfinex_code", code)]);
    // Only Kassiber's own values reach an error: the status and a documented
    // code, "unknown", or null; never Bitfinex's text.
    Err(match &payload {
        Some(Py::List(items)) if items.first() == Some(&s("error")) => {
            let number = items.get(1).and_then(integer);
            let code = number.map_or(s("unknown"), |n| known_number(&Py::from(n), &ERROR_CODES));
            let error = match number {
                Some(10114) => AppError::new("auth_error", "Bitfinex refused the request nonce (error 10114)")
                    .with_hint("Bitfinex needs each key's nonces to increase. Give Kassiber its own read-only key, run one sync or key check of it at a time, and sync again."),
                Some(n @ (10100 | 10111..=10113)) => AppError::new("auth_error", format!("Bitfinex refused the API key (error {n})"))
                    .with_hint("Check that the key is current, allowed from this IP address, and has the Account History and Wallets read permissions."),
                _ => AppError::new("protocol_error", "Bitfinex answered an error"),
            };
            error.with_details(details(code))
        }
        Some(payload) if at(payload, "error") == &s("ERR_RATE_LIMIT") => AppError::new(
            "rate_limited",
            "Bitfinex rate limited the request",
        )
        .with_hint("Bitfinex blocks requests for a minute after its limit; sync again in a minute.")
        .retryable(true),
        _ if !(200..300).contains(&response.status) => {
            let message = format!("HTTP {} from Bitfinex", response.status);
            AppError::new("protocol_error", message).with_details(details(Py::Null))
        }
        Some(payload) => return Ok(payload.clone()),
        None => protocol("Bitfinex response was not JSON"),
    })
}
/// A JSON value with every number as the text Bitfinex wrote (serde_json
/// keeps it exactly), so `0.1` followed by forty zeros and a `1` is not
/// rounded to `0.1`.
fn exact(json: &Json) -> Py {
    match json {
        Json::Null => Py::Null,
        Json::Bool(flag) => Py::Bool(*flag),
        Json::Number(number) => s(number.to_string()),
        Json::String(text) => s(text.as_str()),
        Json::Array(items) => Py::List(items.iter().map(exact).collect()),
        Json::Object(items) => dict(items.iter().map(|(k, v)| (k.as_str(), exact(v)))),
    }
}

/// Whether a permission entry is shaped like a Bitfinex scope name
/// (`orders`, `ui_withdraw`). Only such names are quoted in a refusal: any
/// other text could be an echoed key or signature, and the credential scrub
/// only withholds whole spellings (docs/reference/credential-echo.md).
fn scope_name(scope: &str) -> bool {
    (1..=32).contains(&scope.len()) && scope.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// `/v2/auth/r/permissions`: `[[scope, read, write], ...]` with 0/1 flags.
/// Any write flag refuses the key before any history is read; writable
/// entries that are not scope names are counted, never quoted.
fn refuse_write_keys(client: &Client<'_>) -> Result<(), AppError> {
    let Py::List(rows) = post(client, "v2/auth/r/permissions", "{}")? else {
        return Err(protocol("Bitfinex permissions were not a JSON array"));
    };
    let (mut writable, mut unnamed) = (vec![], 0i64);
    for row in &rows {
        let flag = |at: usize| match field(row, at).and_then(integer) {
            Some(n @ 0..=1) => Some(n == 1),
            _ => None,
        };
        let (Some(scope), Some(_), Some(write)) = (field(row, 0).and_then(text), flag(1), flag(2))
        else {
            return Err(protocol(
                "Bitfinex permission entry is not [scope, read, write]",
            ));
        };
        if write {
            if scope_name(scope) {
                writable.push(s(scope));
            } else {
                unnamed += 1;
            }
        }
    }
    if !writable.is_empty() || unnamed > 0 {
        let mut details = vec![("write_permissions", Py::List(writable))];
        if unnamed > 0 {
            details.push(("other_write_permissions", Py::from(unnamed)));
        }
        return Err(AppError::new("auth_error", "This Bitfinex API key has write permissions; Kassiber only accepts read-only keys")
            .with_hint("Create a Bitfinex API key with only read permissions (nothing to create, cancel, offer, transfer, withdraw or edit), store it in this backend, and delete the old key.")
            .with_details(dict(details)));
    }
    Ok(())
}

/// Every entry of one `/v2/auth/r/<path>` history, newest first. Each next
/// page ends at the oldest time the last one held, inclusively, so entries
/// of that millisecond are read twice (the mapping keeps identical repeats
/// once and refuses changed ones); a full page within one millisecond
/// cannot be paged past.
fn history(client: &Client<'_>, path: &str, size: usize, at: usize, extra: &str) -> Pages {
    let (mut rows, mut end) = (Vec::new(), None);
    for _ in 0..MAX_PAGES {
        let body = match end {
            None => format!(r#"{{"limit":{size}{extra}}}"#),
            Some(end) => format!(r#"{{"end":{end},"limit":{size}{extra}}}"#),
        };
        let Py::List(page) = post(client, &format!("v2/auth/r/{path}"), &body)? else {
            return Err(protocol("Bitfinex history page was not a JSON array"));
        };
        let times: Option<Vec<i64>> = page
            .iter()
            .map(|e| field(e, at).and_then(integer))
            .collect();
        let oldest = times.ok_or_else(|| protocol("Bitfinex history entry has no time"))?;
        let oldest = oldest.into_iter().min().filter(|_| page.len() >= size);
        rows.extend(page);
        let Some(next) = oldest.filter(|_| rows.len() <= MAX_ROWS) else {
            return if rows.len() > MAX_ROWS {
                Err(limit())
            } else {
                Ok(rows)
            };
        };
        if end.is_some_and(|end| next >= end) {
            return Err(protocol("Bitfinex has a full page in one millisecond"));
        }
        end = Some(next);
    }
    Err(limit())
}
type Pages = Result<Vec<Py>, AppError>;

/// `/v2/auth/r/wallets` rows `[TYPE, CURRENCY, BALANCE, ...]`: the BTC of
/// the exchange and funding wallets, the two the history covers (BTC reserved
/// by open orders or lent out included; the margin wallet is refused).
fn balance(payload: &Py, as_of: String) -> Result<Balance, AppError> {
    let Py::List(rows) = payload else {
        return Err(protocol("Bitfinex wallets were not a JSON array"));
    };
    let mut total: i64 = 0;
    for row in rows {
        let wallet = field(row, 0).and_then(text);
        if !matches!(wallet, Some("exchange" | "funding")) || field(row, 1) != Some(&s("BTC")) {
            continue;
        }
        let amount = field(row, 2)
            .and_then(decimal)
            .filter(|amount| !amount.is_negative());
        let sum = amount.and_then(|amount| total.checked_add(exact_msat(&amount)?));
        total = sum.ok_or_else(|| {
            protocol("Bitfinex reported a BTC balance that is not exact whole msat")
        })?;
    }
    let scope = "exchange_and_funding_wallets";
    Ok(Balance {
        msat: Ok(total),
        as_of,
        scope,
    })
}

fn field(row: &Py, at: usize) -> Option<&Py> {
    match row {
        Py::List(fields) => fields.get(at),
        _ => None,
    }
}
fn text(value: &Py) -> Option<&str> {
    match value {
        Py::Str(text) => text.as_str(),
        _ => None,
    }
}
fn integer(value: &Py) -> Option<i64> {
    let digits = |t: &&str| {
        t.trim_start_matches('-')
            .bytes()
            .all(|b| b.is_ascii_digit())
    };
    text(value).filter(digits)?.parse().ok()
}
fn identifier(value: &Py) -> Option<String> {
    let digits = |t: &&str| !t.is_empty() && t.bytes().all(|b| b.is_ascii_digit());
    text(value).filter(digits).map(str::to_owned)
}
fn decimal(value: &Py) -> Option<Decimal> {
    strict_decimal(text(value)?.trim(), true)
}

/// One history entry, read strictly.
struct Api<'a> {
    entry: &'a Py,
    what: &'static str,
}
impl<'a> Api<'a> {
    fn of(entry: &'a Py, what: &'static str, length: usize) -> Result<Self, AppError> {
        match entry {
            Py::List(fields) if fields.len() >= length => Ok(Api { entry, what }),
            _ => Err(protocol(&format!("Bitfinex {what} entry is too short"))),
        }
    }
    fn fail(&self, field: &str) -> AppError {
        protocol(&format!(
            "Bitfinex {} entry has an unreadable {field}",
            self.what
        ))
    }
    fn at(&self, at: usize) -> &Py {
        field(self.entry, at).unwrap_or(&Py::Null)
    }
}
impl mapping::Fields for Api<'_> {
    fn id(&self) -> Result<String, AppError> {
        identifier(self.at(0)).ok_or_else(|| self.fail("id"))
    }
    fn text(&self, (at, _): (usize, &str)) -> String {
        text(self.at(at)).unwrap_or_default().to_owned()
    }
    fn number(&self, (at, _): (usize, &str), optional: bool) -> Result<Decimal, AppError> {
        match self.at(at) {
            Py::Null if optional => Ok(Decimal::zero()),
            value => decimal(value).ok_or_else(|| self.fail("number")),
        }
    }
    fn time(&self, (at, _): (usize, &str)) -> Result<PyStr, AppError> {
        let mts = integer(self.at(at)).ok_or_else(|| self.fail("time"))?;
        epoch(&Py::from(mts), true)?.ok_or_else(|| self.fail("time"))
    }
    fn millis(&self, (at, _): (usize, &str)) -> Option<i64> {
        integer(self.at(at))
    }
    fn api_only(&self, at: usize) -> Option<String> {
        Some(self.text((at, "")))
    }
    fn raw(&self) -> Py {
        self.entry.clone()
    }
}
