//! `wallets sync-kraken|sync-coinbase|sync-binance`: Python's
//! `import_exchange_api`. One explicit command signs its own history reads,
//! then hands Python-shaped record dicts to the shared wallet coordinator,
//! exactly as a JSON file import would.
//!
//! Backend configuration is inert. The local wallet is resolved (and, as in
//! Python, created and committed) before any credential check or request, so
//! the caller mints consent only for the exact route this function reports.
use super::river::{arithmetic, nonnegative, positive, repr, truth};
use super::*;
use bitcoin::hashes::{hmac, sha256, sha512, Hash, HashEngine};
use kassiber_chain::{Backend, HttpClient};
use kassiber_envelope::{get, truthy};
use kassiber_net::{Consent, Gate, HttpRequest};
use kassiber_pyjson::{Key, PyStr};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod binance;
mod bitpanda;
mod coinbase;
pub mod desktop;
mod kraken;
#[cfg(test)]
mod tests;

/// Bounds Python leaves open. A hostile or broken server cannot keep one
/// explicit command paging forever.
const MAX_PAGES: usize = 10_000;
const MAX_ROWS: usize = 1_000_000;
/// Python has no overall budget; each read still has the backend timeout.
const BUDGET: Duration = Duration::from_secs(3600);

/// Bitpanda is Rust only; Python knows the first three.
pub const KINDS: [&str; 4] = ["kraken", "coinbase", "binance", "bitpanda"];

/// The resolved backend as Python's `runtime_config["backends"][name]`.
/// The trusted command boundary supplies a fresh consent for [`Route::request`].
pub struct Route {
    request: kassiber_net::Request,
}
impl Route {
    pub fn request(&self) -> &kassiber_net::Request {
        &self.request
    }
}

/// Python's `import_exchange_api(conn, runtime_config, ..., backend_ref,
/// wallet_ref, expected_backend_kind=expected)` for an already resolved backend.
/// `consent` is called once, after local preparation and before the first read.
#[allow(clippy::too_many_arguments)]
pub fn execute(
    conn: &Connection,
    profile: &str,
    wallet_ref: &str,
    backend: &Json,
    expected: &str,
    gate: &Gate,
    version: &str,
    consent: &mut dyn FnMut(&Route) -> Option<Consent>,
    identity: Option<&Identity>,
) -> Result<Py, AppError> {
    let backend = to_py(backend)?;
    let (name, kind) = backend_kind(&backend, expected)?;
    let mut ids = Ids { identity, at: 0 };
    let wallet = wallet(conn, profile, wallet_ref, &kind, &name, &mut ids)?;
    let label = provider(&kind);
    let prepared = prepare(&backend, &kind, &name)?;
    let book = book_id(conn)?;
    let binding: [u8; 32] = Sha256::digest(
        serde_json::to_vec(&json!([
            format!("wallets.sync-{kind}"),
            book,
            profile,
            wallet.id,
            prepared.transport
        ]))
        .expect("JSON binding"),
    )
    .into();
    let transport = Backend::new(&prepared.transport, false)?.with_action_binding(binding);
    let route = Route {
        request: transport.request().clone(),
    };
    let consent = consent(&route);
    let (key, secret, base) = (&prepared.key, &prepared.secret, &prepared.base);
    let client = Client {
        http: HttpClient::new(transport, gate.clone(), version).withholding(&[key, secret]),
        gate,
        request: &route.request,
        consent: consent.as_ref(),
        deadline: Instant::now() + BUDGET,
        base,
        key,
        secret,
        source: label,
    };
    // Each read rechecks the gate itself, after local steps Python takes first
    // (such as decoding the Kraken secret).
    let mut skipped = None;
    let records = match kind.as_str() {
        "kraken" => kraken::fetch(&client)?,
        "coinbase" => coinbase::fetch(&client)?,
        "bitpanda" => {
            let fetched = bitpanda::fetch(&client)?;
            skipped = Some(fetched.skipped);
            fetched.records
        }
        _ => binance::fetch(&client)?,
    };
    let fetched = records.len();
    let rows: Vec<csv::Row> = records
        .into_iter()
        .map(|record| csv::Row {
            headers: vec![],
            data: generic_json::projection(&record),
            rest: None,
            original: Some(record),
        })
        .collect();
    let normalized = rows.iter().map(normalize).collect::<Result<Vec<_>, _>>()?;
    let request = Request {
        profile_id: profile.into(),
        wallet: wallet.id.clone(),
        file: String::new(),
        format: Format::Json,
        mode: None,
    };
    kassiber_store::begin_write_savepoint(conn, "exchange_api_import").map_err(database_error)?;
    let result = apply_records(
        conn,
        &request,
        &wallet,
        &rows,
        &normalized,
        &mut ids,
        None,
        Some(&format!("{kind}:api:{}", lossy(&name))),
    )
    // A revoked or offline route never publishes a book.
    .and_then(|result| client.authorize().map(|()| result));
    match &result {
        Ok(_) => conn.execute_batch("RELEASE exchange_api_import"),
        Err(_) => conn.execute_batch("ROLLBACK TO exchange_api_import;RELEASE exchange_api_import"),
    }
    .map_err(database_error)?;
    let mut result = result?;
    let url = kassiber_setup::backends::redact_backend_url(at(&backend, "url"))?;
    for (key, item) in [
        ("backend", Py::Str(name)),
        ("backend_kind", s(kind.as_str())),
        ("backend_url", s(url)),
        ("wallet", s(wallet.label.as_str())),
        ("fetched", Py::from(fetched as i64)),
        ("input_format", s(format!("{kind}_api"))),
    ] {
        kassiber_envelope::set(&mut result, key, item);
    }
    if let Some(skipped) = skipped {
        kassiber_envelope::set(&mut result, "skipped", skipped);
    }
    Ok(result)
}

/// The backend's name and its exchange kind, which must be `expected` when
/// one is given.
fn backend_kind(backend: &Py, expected: &str) -> Result<(PyStr, String), AppError> {
    let name = text(at(backend, "name"));
    let kind = match at(backend, "kind") {
        raw if truthy(raw) => kassiber_pytext::lower(kassiber_pytext::strip(&lossy(&text(raw)))),
        _ => String::new(),
    };
    if !KINDS.contains(&kind.as_str()) {
        return Err(AppError::new("validation", "")
            .with_python_message(concat(&[
                "Backend '".into(),
                name.clone(),
                format!("' has kind '{kind}', expected an exchange API kind").into(),
            ]))
            .with_hint("Use a backend kind of kraken, coinbase, or binance."));
    }
    if !expected.is_empty() && kind != expected {
        return Err(
            AppError::new("validation", "").with_python_message(concat(&[
                "Backend '".into(),
                name.clone(),
                format!("' has kind '{kind}', expected '{expected}'").into(),
            ])),
        );
    }
    Ok((name, kind))
}

/// What one exchange command reads with: the credentials, the base URL and
/// the transport the route binds.
struct Prepared {
    key: String,
    secret: String,
    base: String,
    transport: Json,
}

/// `fetch_*_records`' local steps: credentials, then base URL, timeout and
/// opener, all before any request.
fn prepare(backend: &Py, kind: &str, name: &PyStr) -> Result<Prepared, AppError> {
    let label = provider(kind);
    let key = backend_value(backend, &["token", "api_key", "username"]);
    let secret = backend_value(backend, &["auth_header", "api_secret", "password"]);
    // Bitpanda (Rust only) authenticates with the key alone; a stray secret
    // is still withheld from every error.
    let (key, secret) = match (key, secret) {
        (Some(key), secret) if kind == "bitpanda" => (key, secret.unwrap_or_else(|| "".into())),
        (Some(key), Some(secret)) => (key, secret),
        _ if kind == "bitpanda" => {
            return Err(
                AppError::new("config_error", "Bitpanda backend is missing its API key").with_hint(
                    "Store a read-only Bitpanda API key in token (backends create --token-stdin).",
                ),
            )
        }
        _ => {
            return Err(AppError::new(
                "config_error",
                format!("{label} backend is missing API key/secret"),
            )
            .with_hint("Store the key in token and the secret in auth_header (or password)."))
        }
    };
    // Resolved configuration is stored text, so it never holds a surrogate.
    let (key, secret) = (lossy(&key), lossy(&secret));
    let base = backend_value(backend, &["url"])
        .map(|url| lossy(&url))
        .unwrap_or_else(|| default_url(kind).to_owned());
    let base = base.trim_end_matches('/').to_owned();
    let timeout = timeout(backend)?;
    // Python's opener uses system trust and only `tor_proxy`; chain-backend
    // trust overrides, certificates and batch sizes never apply here.
    let mut transport = json!({"name": lossy(name), "url": base, "timeout": timeout});
    if let Some(proxy) = backend_value(backend, &["tor_proxy"]) {
        transport["tor_proxy"] = json!(lossy(&proxy));
    }
    Ok(Prepared {
        key,
        secret,
        base,
        transport,
    })
}

fn book_id(conn: &Connection) -> Result<String, AppError> {
    Ok(conn
        .query_row(
            "SELECT value FROM settings WHERE key='database_instance_id'",
            [],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?
        .unwrap_or_default())
}

fn provider(kind: &str) -> &'static str {
    match kind {
        "kraken" => "Kraken",
        "coinbase" => "Coinbase",
        "bitpanda" => "Bitpanda",
        _ => "Binance",
    }
}
fn default_url(kind: &str) -> &'static str {
    match kind {
        "kraken" => "https://api.kraken.com",
        "coinbase" => "https://api.coinbase.com",
        "bitpanda" => "https://api.public.bitpanda.com",
        _ => "https://api.binance.com",
    }
}

/// `_get_or_create_exchange_api_wallet`: an explicit reference, else the
/// provider's default label, else a new wallet (Python commits it at once).
fn wallet(
    conn: &Connection,
    profile: &str,
    reference: &str,
    kind: &str,
    backend: &PyStr,
    ids: &mut Ids<'_>,
) -> Result<crate::Wallet, AppError> {
    if !reference.is_empty() {
        return resolve(conn, profile, reference);
    }
    let label = provider(kind);
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM wallets WHERE profile_id=? AND lower(label)=lower(?) LIMIT 1",
            [profile, label],
            |r| r.get(0),
        )
        .optional()
        .map_err(database_error)?;
    if let Some(id) = existing {
        return resolve(conn, profile, &id);
    }
    create_wallet(conn, profile, label, kind, &lossy(backend), ids)
}

/// The provider wallet `wallets sync-<kind>` creates on its first run:
/// `label`, `kind` and `{"backend": name}`, through the shared lifecycle.
fn create_wallet(
    conn: &Connection,
    profile: &str,
    label: &str,
    kind: &str,
    backend: &str,
    ids: &mut Ids<'_>,
) -> Result<crate::Wallet, AppError> {
    let workspace: String = conn
        .query_row(
            "SELECT workspace_id FROM profiles WHERE id=?",
            [profile],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    let mut created = None;
    lifecycle::execute_with_creation_source(
        conn,
        lifecycle::Operation::Create {
            workspace_id: workspace,
            profile_id: profile.into(),
            label: label.into(),
            kind: kind.into(),
            account: None,
            config: json!({"backend": backend}),
        },
        &mut |conn| {
            let id = ids.next(conn)?;
            created = Some(id.clone());
            Ok(lifecycle::CreationIdentity {
                id,
                created_at: ids.time(conn)?,
            })
        },
    )?;
    resolve(conn, profile, &created.expect("created wallet"))
}

/// One command's signed reads. Every request rechecks the exact route.
struct Client<'a> {
    http: HttpClient,
    gate: &'a Gate,
    request: &'a kassiber_net::Request,
    consent: Option<&'a Consent>,
    deadline: Instant,
    base: &'a str,
    key: &'a str,
    secret: &'a str,
    source: &'static str,
}
impl Client<'_> {
    fn authorize(&self) -> Result<(), AppError> {
        self.gate
            .authorize(self.request.clone(), self.consent.cloned())
            .map(drop)
            .map_err(|e| e.app_error())
    }
    /// `_read_json(opener, request, timeout, source)` for one signed request.
    fn read(
        &self,
        method: &str,
        url: &str,
        headers: &[(&str, &str)],
        body: Option<&[u8]>,
    ) -> Result<Py, AppError> {
        self.http
            .authenticated_json(url, self.consent, self.deadline, self.source, || {
                let mut request = HttpRequest::new(method, url).map_err(|e| e.app_error())?;
                for (name, value) in headers {
                    request = request.header(name, value).map_err(|e| e.app_error())?;
                }
                Ok(match body {
                    Some(body) => request.body(body.to_vec()),
                    None => request,
                })
            })
    }
    /// One authenticated GET whose `accepted` non-2xx statuses the caller reads.
    fn response(
        &self,
        url: &str,
        headers: &[(&str, &str)],
        accepted: &'static [u16],
    ) -> Result<kassiber_net::HttpResponse, AppError> {
        self.http.authenticated_response(
            url,
            self.consent,
            self.deadline,
            self.source,
            accepted,
            || {
                let mut request = HttpRequest::new("GET", url).map_err(|e| e.app_error())?;
                for (name, value) in headers {
                    request = request.header(name, value).map_err(|e| e.app_error())?;
                }
                Ok(request)
            },
        )
    }
}

fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    let mut engine = hmac::HmacEngine::<sha256::Hash>::new(key);
    engine.input(message);
    hmac::Hmac::<sha256::Hash>::from_engine(engine).to_string()
}
fn hmac_sha512(key: &[u8], parts: &[&[u8]]) -> Vec<u8> {
    let mut engine = hmac::HmacEngine::<sha512::Hash>::new(key);
    for part in parts {
        engine.input(part);
    }
    hmac::Hmac::<sha512::Hash>::from_engine(engine)
        .as_byte_array()
        .to_vec()
}
fn now() -> Result<Duration, AppError> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| AppError::new("internal_error", "Clock precedes the Unix epoch"))
}
fn limit() -> AppError {
    AppError::new("protocol_error", "Exchange history exceeds import limits")
}

fn to_py(value: &Json) -> Result<Py, AppError> {
    kassiber_pytext::loads(&value.to_string())
        .map_err(|_| AppError::new("config_error", "Invalid backend configuration"))
}
/// `row.get(key)` (None when absent or not a dict).
fn at<'a>(row: &'a Py, key: &str) -> &'a Py {
    get(row, key).unwrap_or(&Py::Null)
}
/// `row.get(a) or row.get(b) or ...`: the first truthy value, else the last.
fn either<'a>(row: &'a Py, keys: &[&str]) -> &'a Py {
    let mut value = &Py::Null;
    for key in keys {
        value = at(row, key);
        if truthy(value) {
            break;
        }
    }
    value
}
/// `str(value)`, keeping lone surrogates.
fn text(value: &Py) -> PyStr {
    match value {
        Py::Str(value) => value.clone(),
        other => kassiber_envelope::py_str(other).into(),
    }
}
fn lossy(value: &PyStr) -> String {
    value
        .code_points()
        .map(|p| char::from_u32(p).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
/// `str.lower()`. Lone surrogates are uncased and split the text into runs.
fn lower(value: &PyStr) -> PyStr {
    let mut points = vec![];
    let mut run = String::new();
    for point in value.code_points() {
        match char::from_u32(point) {
            Some(c) => run.push(c),
            None => {
                points.extend(kassiber_pytext::lower(&run).chars().map(u32::from));
                run.clear();
                points.push(point);
            }
        }
    }
    points.extend(kassiber_pytext::lower(&run).chars().map(u32::from));
    PyStr::from_code_points(points).expect("code points")
}
fn concat(parts: &[PyStr]) -> PyStr {
    PyStr::from_code_points(parts.iter().flat_map(PyStr::code_points)).expect("code points")
}
/// `str_or_none(value)`.
fn str_or_none(value: &Py) -> Option<PyStr> {
    if *value == Py::Null {
        return None;
    }
    let text = kassiber_pytext::strip_py(&text(value));
    (text.code_points().next().is_some()).then_some(text)
}
/// `backends.backend_value(backend, *keys)`.
fn backend_value(backend: &Py, keys: &[&str]) -> Option<PyStr> {
    keys.iter().find_map(|key| str_or_none(at(backend, key)))
}
/// `backend_timeout(backend)`: `parse_int(value, 30)`.
fn timeout(backend: &Py) -> Result<u64, AppError> {
    let Some(value) = backend_value(backend, &["timeout"]) else {
        return Ok(30);
    };
    let parsed = kassiber_pytext::parse_int(&lossy(&value)).map_err(|_| {
        AppError::new("app_error", "")
            .with_python_message(concat(&["Invalid integer value: ".into(), value.clone()]))
    })?;
    // urllib would accept no positive timeout either; refuse before I/O.
    u64::try_from(parsed)
        .ok()
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::new("config_error", "Backend timeout must be positive"))
}

/// `_asset(value)`: `normalize_asset_code`, then exchange aliases.
fn asset(value: &Py) -> String {
    let raw = if truthy(value) {
        lossy(&text(value))
    } else {
        String::new()
    };
    let raw = kassiber_pytext::strip(&raw);
    let code = if raw.is_empty() {
        String::new()
    } else {
        let lowered = kassiber_pytext::lower(raw);
        if lowered.chars().count() == 64
            && lowered
                .chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            lowered
        } else if lowered == "l-btc" || lowered == "lbtc" {
            "LBTC".into()
        } else {
            kassiber_pytext::upper(raw)
        }
    };
    match code.as_str() {
        "BTC" | "XBT" | "XXBT" => "BTC".into(),
        "LBTC" | "L-BTC" => "LBTC".into(),
        _ if code.starts_with('Z') && code.chars().count() == 4 => code[1..].into(),
        "XETH" => "ETH".into(),
        _ => code,
    }
}
fn is_bitcoin(code: &str) -> bool {
    matches!(code, "BTC" | "LBTC")
}
const FIAT: [&str; 29] = [
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];
fn is_fiat(code: &str) -> bool {
    FIAT.contains(&code)
}

/// `_decimal(value, default)`: `msat.dec`, i.e. `Decimal(str(value))`.
fn decimal(value: &Py, default: &str) -> Result<PriceNumber, AppError> {
    let raw = match value {
        Py::Null => return PriceNumber::parse(default),
        Py::Str(text) if text.code_points().next().is_none() => return PriceNumber::parse(default),
        other => text(other),
    };
    let invalid = || {
        AppError::new("app_error", "")
            .with_python_message(concat(&["Invalid decimal value: ".into(), raw.clone()]))
    };
    let parsed = raw.as_str().ok_or_else(invalid)?;
    PriceNumber::parse(parsed).map_err(|_| invalid())
}
fn number(value: &PriceNumber) -> Py {
    s(repr(value))
}
fn nan(value: &PriceNumber) -> bool {
    repr(value).contains("NaN")
}
/// `abs(value)`: a quiet NaN keeps its sign; a signaling NaN raises.
fn abs(value: PriceNumber) -> Result<PriceNumber, AppError> {
    match value {
        PriceNumber::Special(v) if v.contains("sNaN") => {
            Err(decimal_error(DecimalError::InvalidOperation))
        }
        PriceNumber::Special(v) if v.contains("NaN") => Ok(PriceNumber::Special(v)),
        other => super::river::abs(other),
    }
}
/// `value >= 0`.
fn at_least_zero(value: &PriceNumber) -> Result<bool, AppError> {
    match value {
        _ if nan(value) => Err(decimal_error(DecimalError::InvalidOperation)),
        PriceNumber::Finite(v) => Ok(!v.is_negative()),
        PriceNumber::Special(v) => Ok(!v.starts_with('-')),
    }
}
/// `value == 0`: quiet NaN compares unequal; a signaling NaN raises.
fn is_zero(value: &PriceNumber) -> Result<bool, AppError> {
    match value {
        PriceNumber::Special(v) if v.contains("sNaN") => {
            Err(decimal_error(DecimalError::InvalidOperation))
        }
        PriceNumber::Finite(v) => Ok(v.is_zero()),
        PriceNumber::Special(_) => Ok(false),
    }
}
/// `a / b` in the process context (prec 32, Etiny -1000030).
fn divide(a: &PriceNumber, b: &PriceNumber) -> Result<PriceNumber, AppError> {
    if let (PriceNumber::Finite(x), PriceNumber::Special(y)) = (a, b) {
        if y.ends_with("Infinity") {
            let negative = x.is_sign_negative() != y.starts_with('-');
            return PriceNumber::parse(if negative {
                "-0E-1000030"
            } else {
                "0E-1000030"
            });
        }
    }
    arithmetic(a, b, '/')
}
/// `float(value)`.
fn float(value: &PriceNumber) -> Result<f64, AppError> {
    match value {
        PriceNumber::Finite(v) => Ok(v.to_string().parse::<f64>().unwrap_or(f64::NAN)),
        PriceNumber::Special(v) if v.contains("sNaN") => Err(AppError::new(
            "internal_error",
            "cannot convert signaling NaN to float",
        )),
        PriceNumber::Special(v) if v.contains("NaN") => Ok(f64::NAN),
        PriceNumber::Special(v) if v.starts_with('-') => Ok(f64::NEG_INFINITY),
        PriceNumber::Special(_) => Ok(f64::INFINITY),
    }
}
/// `_seconds_epoch_to_iso` (scale 1) and `_ms_epoch_to_iso` (scale 1000).
fn epoch(value: &Py, milliseconds: bool) -> Result<Option<PyStr>, AppError> {
    if *value == Py::Null || matches!(value, Py::Str(v) if v.code_points().next().is_none()) {
        return Ok(None);
    }
    let mut seconds = decimal(value, "0")?;
    if milliseconds {
        seconds = divide(&seconds, &PriceNumber::parse("1000")?)?;
    }
    utc_iso(float(&seconds)?).map(|v| Some(v.into()))
}
/// `datetime.fromtimestamp(seconds, tz=utc).replace(microsecond=0)
/// .isoformat().replace("+00:00", "Z")`, with CPython's errors.
fn utc_iso(seconds: f64) -> Result<String, AppError> {
    let internal = |message: String| AppError::new("internal_error", message);
    if seconds.is_nan() {
        return Err(internal("Invalid value NaN (not a number)".into()));
    }
    // _PyTime_DoubleToDenominator(ROUND_HALF_EVEN) to whole microseconds.
    let mut whole = seconds.trunc();
    let micros = ((seconds - whole) * 1e6).round_ties_even();
    if micros >= 1e6 {
        whole += 1.0;
    } else if micros < 0.0 {
        whole -= 1.0;
    }
    if !(-9_223_372_036_854_775_808.0..=9_223_372_036_854_775_808.0).contains(&whole) {
        return Err(internal(
            "timestamp out of range for platform time_t".into(),
        ));
    }
    let whole = whole as i64;
    let days = whole.div_euclid(86_400);
    let second = whole.rem_euclid(86_400);
    let (year, month, day) = kassiber_pytext::zone::civil_from_days(days);
    // glibc gmtime_r refuses a tm_year that does not fit in int.
    if i32::try_from(year - 1900).is_err() {
        return Err(internal(
            "[Errno 75] Value too large for defined data type".into(),
        ));
    }
    if !(1..=9999).contains(&year) {
        return Err(internal(format!("year {year} is out of range")));
    }
    Ok(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        second / 3600,
        second / 60 % 60,
        second % 60
    ))
}

/// Python's `_record(...)`. Text values stay `str` so the coordinator sees
/// `str(Decimal)` exactly as it would for a JSON file record.
struct Entry {
    provider: &'static str,
    external_id: PyStr,
    occurred_at: Option<PyStr>,
    inbound: bool,
    asset: String,
    amount: PriceNumber,
    fee: PriceNumber,
    kind: &'static str,
    raw: Py,
    description: Option<PyStr>,
    pricing: Option<Pricing>,
}
struct Pricing {
    fiat_currency: String,
    fiat_value: PriceNumber,
    fiat_rate: Option<PriceNumber>,
    method: &'static str,
    external_ref: PyStr,
}
fn record(entry: Entry) -> Result<Py, AppError> {
    let Entry {
        provider,
        external_id,
        occurred_at,
        inbound,
        asset,
        amount,
        fee,
        kind,
        raw,
        description,
        pricing,
    } = entry;
    // A sell's amount is negative; its unit price is not. A value that is not
    // positive prices nothing, so the row stays unpriced.
    let rate = match &pricing {
        Some(Pricing {
            fiat_rate: None,
            fiat_value,
            ..
        }) if positive(fiat_value)? && truth(&amount) => {
            Some(divide(fiat_value, &abs(amount.clone())?)?)
        }
        Some(pricing) => pricing.fiat_rate.clone(),
        None => None,
    };
    let quantity = number(&abs(amount)?);
    let fee = number(&abs(fee)?);
    let occurred = occurred_at.map_or(Py::Null, Py::Str);
    let mut result = vec![
        ("txid", Py::Str(external_id)),
        ("occurred_at", occurred.clone()),
        ("direction", s(if inbound { "inbound" } else { "outbound" })),
        ("asset", s(asset.as_str())),
        ("amount", quantity),
        ("fee", fee),
    ];
    match pricing {
        Some(pricing) => result.extend([
            ("fiat_rate", rate.as_ref().map_or(Py::Null, number)),
            ("fiat_value", number(&pricing.fiat_value)),
            ("fiat_currency", s(pricing.fiat_currency.as_str())),
            ("pricing_source_kind", s("exchange_execution")),
            ("pricing_provider", s(provider)),
            (
                "pricing_pair",
                s(format!("{asset}-{}", pricing.fiat_currency)),
            ),
            ("pricing_timestamp", occurred),
            ("pricing_method", s(pricing.method)),
            ("pricing_external_ref", Py::Str(pricing.external_ref)),
            ("pricing_quality", s("exact")),
        ]),
        None => result.extend(
            [
                "fiat_rate",
                "fiat_value",
                "fiat_currency",
                "pricing_source_kind",
                "pricing_provider",
                "pricing_pair",
                "pricing_timestamp",
                "pricing_method",
                "pricing_external_ref",
                "pricing_quality",
            ]
            .map(|key| (key, Py::Null)),
        ),
    }
    let raw = kassiber_pyjson::dumps(
        &raw,
        &kassiber_pyjson::Options {
            sort_keys: true,
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    result.extend([
        ("kind", s(kind)),
        (
            "description",
            Py::Str(description.unwrap_or_else(|| format!("{provider} {kind}").into())),
        ),
        ("counterparty", s(provider)),
        ("raw_json", s(raw)),
    ]);
    Ok(dict(result))
}
/// `dict(row)` as a fresh mapping, or with extra keys merged as `{**a, k: v}`.
fn merged(first: &[(&str, Py)], row: &Py, last: &[(&str, Py)]) -> Py {
    let mut result = dict::<&str>([]);
    for (key, value) in first {
        kassiber_envelope::set(&mut result, key, value.clone());
    }
    if let (Py::Dict(items), Py::Dict(target)) = (row, &mut result) {
        for (key, value) in items {
            match target.iter_mut().find(|(k, _)| k == key) {
                Some((_, slot)) => *slot = value.clone(),
                None => target.push((key.clone(), value.clone())),
            }
        }
    }
    for (key, value) in last {
        kassiber_envelope::set(&mut result, key, value.clone());
    }
    result
}
fn key_text(key: &Key) -> PyStr {
    match key {
        Key::Str(text) => text.clone(),
        Key::Null => "None".into(),
        Key::Bool(true) => "True".into(),
        Key::Bool(false) => "False".into(),
        Key::Int(n) => n.to_string().into(),
        Key::Float(f) => kassiber_pyjson::float_repr(*f).into(),
    }
}
