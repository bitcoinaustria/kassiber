//! Silent-payment scans: `build_plan`, `validate_backend_capability` and
//! `normalize_scan_payload`. Scanner output keeps Python's value model, so
//! every `str()`, `int()` and identity test reads as it does there; records,
//! outputs and the checkpoint come out in Python's shapes and key order.
use crate::{bitcoin_records::btc, descriptors, time};
use kassiber_envelope::{contains_key, dict, get, py_str, redaction::redact_secret_value, s, set};
use kassiber_pydecimal::{Context, Decimal, Rounding};
use kassiber_pyjson::{BigInt, Value};
use serde_json::Value as Json;
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, AppError>;
use kassiber_envelope::AppError;

const NULL: Value = Value::Null;
const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";
const GENESIS: &str = "2009-01-03T00:00:00Z";
pub(crate) const SCAN_FILE_FIELDS: [&str; 2] = ["silent_payment_scan_file", "sp_scan_file"];
pub(crate) const SCAN_PATH_FIELDS: [&str; 2] = ["silent_payment_scan_path", "sp_scan_path"];

/// A failure Python raises as something other than `AppError`; the route
/// wraps it as its coordinator does (`type(exc).__name__`, `str(exc)`).
#[derive(Debug)]
pub(crate) enum Failure {
    App(AppError),
    Raw { kind: &'static str, message: String },
}

impl From<AppError> for Failure {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

/// `SilentPaymentPlan`.
pub(crate) struct Plan {
    pub(crate) chain: String,
    pub(crate) network: String,
    pub(crate) scan_mode: String,
    pub(crate) material_format: String,
    pub(crate) start_height: Option<BigInt>,
    pub(crate) start_date: Option<String>,
    pub(crate) full_history: bool,
    pub(crate) descriptor_fingerprint: String,
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&NULL)
}

/// `mapping.get(key, default)`: a present key's value, even `None`.
fn get_or<'a>(value: &'a Value, key: &str, default: &'a Value) -> &'a Value {
    get(value, key).unwrap_or(default)
}

fn validation(message: impl Into<String>) -> AppError {
    AppError::new("validation", message)
}

/// `util.str_or_none`.
pub(crate) fn str_or_none(value: &Value) -> Option<String> {
    if matches!(value, Value::Null) {
        return None;
    }
    let text = kassiber_pytext::strip(&py_str(value)).to_owned();
    (!text.is_empty()).then_some(text)
}

/// `util.parse_int(value, None)`.
fn parse_int(value: &Value) -> Result<Option<BigInt>> {
    if matches!(value, Value::Null) || matches!(value, Value::Str(t) if t.is("")) {
        return Ok(None);
    }
    let text = py_str(value);
    kassiber_pytext::parse_int(kassiber_pytext::strip(&text))
        .map(Some)
        .map_err(|_| AppError::new("app_error", format!("Invalid integer value: {text}")))
}

/// `util.parse_bool(value, default)`.
pub(crate) fn parse_bool(value: &Value, default: bool) -> Result<bool> {
    if matches!(value, Value::Null) {
        return Ok(default);
    }
    let text = py_str(value);
    match kassiber_pytext::lower(kassiber_pytext::strip(&text)).as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(AppError::new(
            "app_error",
            format!("Invalid boolean value: {text}"),
        )),
    }
}

/// `value is True`.
fn is_true(value: &Value) -> bool {
    matches!(value, Value::Bool(true))
}

fn is_dict(value: &Value) -> bool {
    matches!(value, Value::Dict(_))
}

/// `build_plan(config)`.
pub(crate) fn build_plan(config: &Json) -> Result<Plan> {
    let validated = super::validate(config)?;
    let text = |key: &str| validated[key].as_str().unwrap_or("").to_owned();
    Ok(Plan {
        chain: text("chain"),
        network: text("network"),
        scan_mode: text("sp_scan_mode"),
        material_format: text("sp_material_format"),
        start_height: validated
            .get("sp_scan_start_height")
            .map(|v| v.to_string().parse().expect("validated integer")),
        start_date: validated
            .get("sp_scan_start_date")
            .and_then(Json::as_str)
            .map(str::to_owned),
        full_history: validated["sp_full_history"] == true,
        descriptor_fingerprint: Sha256::digest(text("sp_descriptor").as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    })
}

/// `silent_payments._backend_value`: the flat value, else the nested
/// `config` mapping's when the flat one is `None` or `""`.
fn backend_value<'a>(backend: &'a Value, key: &str) -> &'a Value {
    let value = field(backend, key);
    if matches!(value, Value::Null) || matches!(value, Value::Str(t) if t.is("")) {
        if let Some(config) = get(backend, "config").filter(|c| is_dict(c)) {
            return field(config, key);
        }
    }
    value
}

/// `urlsplit(url).scheme.lower()`.
fn scheme(url: &str) -> String {
    let url: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    match url.find(':') {
        Some(end)
            if end > 0
                && url.as_bytes()[0].is_ascii_alphabetic()
                && url[..end]
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"+-.".contains(&b)) =>
        {
            url[..end].to_ascii_lowercase()
        }
        _ => String::new(),
    }
}

/// `backend_supports_silent_payments(backend)`.
fn supports(backend: &Value) -> Result<bool> {
    let mut declared = false;
    for key in ["silent_payments", "sp_capable", "bip352"] {
        let value = backend_value(backend, key);
        if str_or_none(value).is_none() {
            continue;
        }
        declared = true;
        if parse_bool(value, false)? {
            return Ok(true);
        }
    }
    if declared {
        return Ok(false);
    }
    Ok(SCAN_FILE_FIELDS
        .iter()
        .chain(&SCAN_PATH_FIELDS)
        .any(|key| str_or_none(backend_value(backend, key)).is_some()))
}

/// `validate_backend_capability(backend, plan, kind=kind)`.
pub(crate) fn validate_backend_capability(backend: &Value, plan: &Plan, kind: &str) -> Result<()> {
    let name = py_str(field(backend, "name"));
    let app = |e: descriptors::Error| e.into_app();
    if let Some(chain) = str_or_none(backend_value(backend, "chain")) {
        if descriptors::normalize_chain(&chain).map_err(app)? != plan.chain {
            return Err(validation(format!(
                "Backend '{name}' is not configured for Bitcoin Silent Payments"
            )));
        }
    }
    if let Some(network) = str_or_none(backend_value(backend, "network")) {
        if descriptors::normalize_network(&plan.chain, &network).map_err(app)? != plan.network {
            return Err(validation(format!(
                "Backend '{name}' network does not match the Silent Payments wallet"
            )));
        }
    }
    if !["esplora", "electrum", "bitcoinrpc", "custom"].contains(&kind) {
        return Err(AppError::new(
            "silent_payment_backend_unsupported",
            format!("Backend kind '{kind}' cannot scan Silent Payments"),
        )
        .with_hint("Use a deliberately configured SP-capable backend or local scanner."));
    }
    if plan.scan_mode == "server_assisted" {
        let scheme = scheme(&str_or_none(backend_value(backend, "url")).unwrap_or_default());
        if kind == "electrum" || !["http", "https"].contains(&scheme.as_str()) {
            let over = if scheme.is_empty() { kind } else { &scheme };
            return Err(AppError::new(
                "silent_payment_backend_unsupported",
                format!("Backend '{name}' cannot run server-assisted Silent Payments scans over {over}"),
            )
            .with_hint("Use an HTTPS SP scanner endpoint, or switch the wallet to local scanner output mode."));
        }
    }
    if !supports(backend)? {
        return Err(AppError::new(
            "silent_payment_backend_unsupported",
            format!("Backend '{name}' is not marked Silent Payments capable"),
        )
        .with_hint("Ordinary Esplora/Electrum scripthash sync cannot discover BIP352 outputs. Configure an SP-capable backend/local scanner explicitly.")
        .with_details(dict([
            ("backend", field(backend, "name").clone()),
            ("backend_kind", s(kind)),
            ("sp_capable", Value::Bool(false)),
        ])));
    }
    Ok(())
}

/// `sync_target(plan)`.
pub(crate) fn sync_target(plan: &Plan) -> Json {
    serde_json::json!({"chain":plan.chain,"network":plan.network,"branch_index":null,
        "branch_label":"silent-payment","address_index":null,"address":"",
        "unconfidential_address":null,"script_pubkey":""})
}

/// `_normalize_txid`.
fn txid(value: &Value) -> Result<String> {
    match str_or_none(value) {
        Some(text) if text.len() == 64 && text.bytes().all(|b| b.is_ascii_hexdigit()) => {
            Ok(text.to_ascii_lowercase())
        }
        _ => Err(validation(
            "Silent Payments scan result contains an invalid txid",
        )),
    }
}

/// `_normalize_vout`.
fn vout(value: &Value) -> Result<BigInt> {
    match parse_int(value)? {
        Some(n) if n >= BigInt::from(0) => Ok(n),
        _ => Err(validation(
            "Silent Payments scan result contains an invalid vout",
        )),
    }
}

/// `_normalize_sats(value, field=name)`.
fn sats(value: &Value, name: &str) -> Result<BigInt> {
    let parsed = parse_int(value)?
        .ok_or_else(|| validation(format!("Silent Payments scan result is missing {name}")))?;
    if parsed < BigInt::from(0) {
        return Err(validation(format!(
            "Silent Payments scan result has a negative {name}"
        )));
    }
    Ok(parsed)
}

/// `_amount_sats(row)`: the first present sats key, else BTC `amount`.
fn amount_sats(row: &Value) -> std::result::Result<BigInt, Failure> {
    let name = "amount_sats";
    for key in [name, "amount_sat", "value_sats", "value_sat"] {
        if contains_key(row, key) {
            return Ok(sats(field(row, key), name)?);
        }
    }
    if !contains_key(row, "amount") {
        return Err(validation(format!("Silent Payments scan result is missing {name}")).into());
    }
    // `msat.dec(value, default="0")`, then `value < 0` and
    // `int((value * Decimal("100000000")).to_integral_value())`.
    let value = field(row, "amount");
    let text = py_str(value);
    let negative = || validation(format!("Silent Payments scan result has a negative {name}"));
    let signal = |name: &str| Failure::Raw {
        kind: if name == "Overflow" {
            "Overflow"
        } else {
            "InvalidOperation"
        },
        message: format!("[<class 'decimal.{name}'>]"),
    };
    let amount: Decimal = if matches!(value, Value::Null) || text.is_empty() {
        Decimal::zero()
    } else {
        let stripped = kassiber_pytext::strip(&text);
        match stripped.parse() {
            Ok(amount) => amount,
            Err(kassiber_pydecimal::DecimalError::NonFinite) => {
                // NaN cannot compare; an infinity overflows `int()`.
                let word = stripped.trim_start_matches(['+', '-']).to_ascii_lowercase();
                if word.contains("nan") {
                    return Err(signal("InvalidOperation"));
                }
                if stripped.starts_with('-') {
                    return Err(negative().into());
                }
                return Err(Failure::Raw {
                    kind: "OverflowError",
                    message: "cannot convert Infinity to integer".into(),
                });
            }
            Err(_) => {
                return Err(
                    AppError::new("app_error", format!("Invalid decimal value: {text}")).into(),
                )
            }
        }
    };
    if amount.is_negative() && !amount.is_zero() {
        return Err(negative().into());
    }
    let context = Context::RP2;
    let product = amount
        .mul(&Decimal::from(100_000_000_u64), &context)
        .map_err(|_| signal("Overflow"))?;
    let digits = product
        .to_fixed_places(0, Rounding::HalfEven)
        .map_err(|_| signal("InvalidOperation"))?;
    digits.parse().map_err(|_| signal("InvalidOperation"))
}

/// `_normalize_script_pubkey`.
fn script_pubkey(value: &Value) -> Result<String> {
    match str_or_none(value) {
        Some(text)
            if text.len() == 68
                && text[..4].eq_ignore_ascii_case("5120")
                && text.bytes().all(|b| b.is_ascii_hexdigit()) =>
        {
            Ok(text.to_ascii_lowercase())
        }
        _ => Err(validation(
            "Silent Payments detected outputs must be concrete Taproot scriptPubKeys",
        )),
    }
}

/// `int(value)` for `timestamp_to_iso`: `None` where CPython raises.
fn int(value: &Value) -> Option<BigInt> {
    match value {
        Value::Bool(b) => Some(BigInt::from(u8::from(*b))),
        Value::Int(n) => Some(n.clone()),
        Value::Float(f) if f.is_finite() => format!("{:.0}", f.trunc()).parse().ok(),
        Value::Str(t) => t.as_str().and_then(|t| kassiber_pytext::parse_int(t).ok()),
        _ => None,
    }
}

/// `_block_time(tx, default)`: the first usable unix time or ISO stamp.
fn block_time(tx: &Value, default: Option<&str>) -> Result<Option<String>> {
    for key in [
        "block_time",
        "confirmed_at",
        "time",
        "timestamp",
        "blocktime",
    ] {
        let value = field(tx, key);
        // `value in (None, "", 0, "0")`, where `False == 0` and `0.0 == 0`.
        let skipped = match value {
            Value::Null => true,
            Value::Str(t) => t.is("") || t.is("0"),
            Value::Bool(b) => !b,
            Value::Int(n) => *n == BigInt::from(0),
            Value::Float(f) => *f == 0.0,
            _ => false,
        };
        if skipped {
            continue;
        }
        if let Some(stamp) = int(value).and_then(|seconds| time::from_unix(&seconds)) {
            return Ok(Some(stamp));
        }
        if let Some(us) = time::instant(&py_str(value))? {
            return Ok(Some(time::iso_seconds(us)));
        }
    }
    Ok(default.map(str::to_owned))
}

fn opt(value: Option<String>) -> Value {
    value.map_or(Value::Null, |v| s(v.as_str()))
}

fn int_value(value: Option<BigInt>) -> Value {
    value.map_or(Value::Null, Value::Int)
}

/// `raw.get("outputs", raw.get("vout", []))` style lists of mappings.
fn mappings<'a>(row: &'a Value, key: &str, fallback: &str) -> Vec<&'a Value> {
    // A missing fallback is `[]`, which holds no mappings either.
    match get(row, key).or_else(|| get(row, fallback)) {
        Some(Value::List(items)) => items.iter().filter(|v| is_dict(v)).collect(),
        _ => vec![],
    }
}

/// `_owned_outputs(tx)` / `_owned_inputs(tx)`.
fn owned<'a>(tx: &'a Value, key: &str, fallback: &str) -> Vec<&'a Value> {
    mappings(tx, key, fallback)
        .into_iter()
        .filter(|row| {
            is_true(field(row, "owned"))
                || is_true(field(row, "matched"))
                || is_true(field(row, "silent_payment"))
        })
        .collect()
}

fn sum(rows: &[&Value]) -> std::result::Result<BigInt, Failure> {
    let mut total = BigInt::from(0);
    for row in rows {
        total += amount_sats(row)?;
    }
    Ok(total)
}

fn btc_text(sats: &BigInt) -> std::result::Result<Value, Failure> {
    let sats = i128::try_from(sats.clone())
        .ok()
        .filter(|n| n.unsigned_abs() < 10_u128.pow(28))
        .ok_or_else(|| Failure::Raw {
            kind: "OverflowError",
            message: "Python int too large to convert to SQLite INTEGER".into(),
        })?;
    Ok(s(btc(sats).as_str()))
}

fn sorted_dumps(value: &Value) -> Result<String> {
    kassiber_pyjson::dumps(value, &kassiber_pyjson::Options::new().sort_keys(true))
        .map_err(|_| AppError::new("internal_error", "Scanner output cannot be encoded"))
}

/// `_record_from_tx(tx, backend_name=...)`.
fn record(tx: &Value, backend_name: &str) -> std::result::Result<Option<Value>, Failure> {
    let id = txid(field(tx, "txid"))?;
    let received = sum(&owned(tx, "outputs", "vout"))?;
    let spent = sum(&owned(tx, "inputs", "vin"))?;
    let zero = BigInt::from(0);
    if spent > zero && !(contains_key(tx, "fee_sats") || contains_key(tx, "fee_sat")) {
        return Err(validation("Silent Payments spend transactions require fee_sats")
            .with_hint("Scanner output must include explicit fee_sats so accounting rows remain idempotent.")
            .into());
    }
    let fee = parse_int(get_or(tx, "fee_sats", field(tx, "fee_sat")))?.unwrap_or_default();
    if received == zero && spent == zero {
        return Ok(None);
    }
    let fee = fee.max(zero.clone());
    let (direction, amount, fee, kind) = if received > spent {
        ("inbound", &received - &spent, zero.clone(), "deposit")
    } else {
        let amount = (&spent - &received - &fee).max(zero.clone());
        let kind = if amount > zero { "withdrawal" } else { "fee" };
        ("outbound", amount, fee, kind)
    };
    let occurred = block_time(tx, Some(UNKNOWN_OCCURRED_AT))?;
    let confirmed = block_time(tx, None)?;
    Ok(Some(dict([
        ("txid", s(id.as_str())),
        ("occurred_at", opt(occurred)),
        ("confirmed_at", opt(confirmed)),
        ("direction", s(direction)),
        ("asset", s("BTC")),
        ("amount", btc_text(&amount)?),
        ("fee", btc_text(&fee)?),
        ("fiat_rate", Value::Null),
        ("fiat_value", Value::Null),
        ("kind", s(kind)),
        (
            "description",
            s(format!("Silent Payment sync from {backend_name}").as_str()),
        ),
        ("counterparty", Value::Null),
        (
            "raw_json",
            s(sorted_dumps(&redact_secret_value(tx))?.as_str()),
        ),
    ])))
}

/// `_utxo_from_output(output, tx, chain=..., network=...)`.
fn utxo(output: &Value, tx: &Value, plan: &Plan) -> std::result::Result<Value, Failure> {
    let own = field(output, "txid");
    let id = txid(if kassiber_envelope::truthy(own) {
        own
    } else {
        field(tx, "txid")
    })?;
    let index = vout(get_or(output, "vout", field(output, "n")))?;
    let mut height = parse_int(get_or(output, "block_height", field(output, "height")))?;
    if height.is_none() {
        height = parse_int(get_or(tx, "block_height", field(tx, "height")))?;
    }
    let block_time = match str_or_none(field(output, "block_time")) {
        Some(text) => Some(text),
        None => block_time(tx, None)?,
    };
    let mut confirmations = parse_int(field(output, "confirmations"))?;
    if confirmations.is_none() {
        confirmations = parse_int(field(tx, "confirmations"))?.filter(|n| *n >= BigInt::from(0));
    }
    let spent_by = str_or_none(get_or(output, "spent_by", field(output, "spentBy")));
    let amount = amount_sats(output)?;
    let zero = BigInt::from(0);
    let confirmed = height.as_ref().is_some_and(|h| *h > zero)
        || confirmations.as_ref().is_some_and(|c| *c > zero);
    let script = script_pubkey(get_or(
        output,
        "script_pubkey",
        field(output, "scriptPubKey"),
    ))?;
    let spent = spent_by.is_some() || parse_bool(field(output, "spent"), false)?;
    let mut raw = dict([("source", s("silent_payment_scan"))]);
    if let Value::Dict(pairs) = redact_secret_value(field(output, "raw")) {
        for (key, value) in pairs {
            match &mut raw {
                Value::Dict(items) => match items.iter_mut().find(|(k, _)| *k == key) {
                    Some(slot) => slot.1 = value,
                    None => items.push((key, value)),
                },
                _ => unreachable!(),
            }
        }
    }
    Ok(dict([
        ("txid", s(id.as_str())),
        ("vout", Value::Int(index)),
        ("asset", s("BTC")),
        ("amount_sats", Value::Int(amount)),
        (
            "confirmation_status",
            s(if confirmed { "confirmed" } else { "mempool" }),
        ),
        ("confirmations", int_value(confirmations)),
        ("block_height", int_value(height)),
        ("block_time", opt(block_time)),
        ("chain", s(plan.chain.as_str())),
        ("network", s(plan.network.as_str())),
        ("address", s("")),
        ("script_pubkey", s(script.as_str())),
        ("address_label", s("Silent Payment")),
        ("branch_label", s("silent-payment")),
        ("branch_index", Value::Null),
        ("address_index", Value::Null),
        ("spent_by", opt(spent_by)),
        ("spent", Value::Bool(spent)),
        ("raw", raw),
    ]))
}

/// `_outpoint_from_mapping(row, fallback_txid)`.
fn outpoint(row: &Value, fallback: &Value) -> Result<String> {
    let own = field(row, "txid");
    let id = txid(if kassiber_envelope::truthy(own) {
        own
    } else {
        fallback
    })?;
    let index = vout(get_or(row, "vout", field(row, "n")))?;
    Ok(format!("{id}:{index}"))
}

fn mismatch(message: &str) -> AppError {
    AppError::new("silent_payment_scan_wallet_mismatch", message)
}

/// `a.get(k1) or a.get(k2) or ...` then `str_or_none`.
fn first_text(binding: &Value, keys: &[&str]) -> Option<String> {
    let value = keys
        .iter()
        .map(|key| field(binding, key))
        .find(|v| kassiber_envelope::truthy(v))
        .unwrap_or_else(|| field(binding, keys[keys.len() - 1]));
    str_or_none(value)
}

/// `_validate_payload_binding(payload, plan=..., wallet_id=..., wallet_label=...)`.
fn binding(payload: &Value, plan: &Plan, wallet_id: &str, wallet_label: &str) -> Result<()> {
    let binding = get(payload, "wallet")
        .filter(|w| is_dict(w))
        .unwrap_or(payload);
    let fingerprint = first_text(
        binding,
        &[
            "descriptor_fingerprint",
            "sp_descriptor_fingerprint",
            "wallet_fingerprint",
        ],
    );
    let label = first_text(binding, &["wallet_label", "label"]);
    if label.is_some_and(|label| label != wallet_label) {
        return Err(mismatch(
            "Silent Payments scanner output wallet_label does not match this wallet",
        ));
    }
    if let Some(fingerprint) = fingerprint {
        if kassiber_pytext::lower(&fingerprint) != plan.descriptor_fingerprint {
            return Err(mismatch(
                "Silent Payments scanner output is for a different wallet",
            ));
        }
        return Ok(());
    }
    if let Some(actual) = first_text(binding, &["wallet_id", "kassiber_wallet_id"]) {
        if actual != wallet_id {
            return Err(mismatch(
                "Silent Payments scanner output wallet_id does not match this wallet",
            ));
        }
        return Ok(());
    }
    Err(
        mismatch("Silent Payments scanner output must be bound to the wallet")
            .with_hint("Include descriptor_fingerprint or wallet_id in the scanner JSON."),
    )
}

/// `_range_start_height(range)`: the first present key, parsed.
fn range_height(range: &Value) -> Result<Option<BigInt>> {
    for key in [
        "from_height",
        "start_height",
        "from_block_height",
        "start_block_height",
    ] {
        if contains_key(range, key) {
            return parse_int(field(range, key));
        }
    }
    Ok(None)
}

/// `_range_start_date(range)`.
fn range_date(range: &Value) -> Option<String> {
    ["from_date", "start_date", "from_time", "start_time"]
        .iter()
        .find_map(|key| str_or_none(field(range, key)))
}

/// `parse_iso_datetime_or_none(value)` for a `str_or_none` result.
fn instant(value: Option<&str>) -> Result<Option<i64>> {
    value.map_or(Ok(None), time::instant)
}

/// `_scan_range_covers_plan(range, plan)`.
fn covers(range: &Value, plan: &Plan) -> Result<bool> {
    let from_height = range_height(range)?;
    if plan.full_history {
        if let Some(height) = &from_height {
            if *height > BigInt::from(0) {
                return Ok(false);
            }
        } else {
            let from = instant(range_date(range).as_deref())?;
            let genesis = time::instant(GENESIS)?;
            match (from, genesis) {
                (Some(from), Some(genesis)) if from <= genesis => {}
                _ => return Ok(false),
            }
        }
    } else if let Some(start) = &plan.start_height {
        if from_height.as_ref().is_none_or(|height| height > start) {
            return Ok(false);
        }
    }
    if let Some(start) = &plan.start_date {
        let from = instant(range_date(range).as_deref())?;
        match (from, time::instant(start)?) {
            (Some(from), Some(start)) if from <= start => {}
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// What `normalize_scan_payload` returns: the records and the adapter's meta.
pub(crate) struct Normalized {
    pub(crate) records: Vec<Value>,
    pub(crate) meta: Value,
    pub(crate) complete: bool,
}

/// `normalize_scan_payload(payload, backend_name=..., backend_kind=...,
/// plan=..., checkpoint=..., wallet_id=..., wallet_label=...)`.
pub(crate) fn normalize(
    payload: &Value,
    backend: (&str, &str),
    plan: &Plan,
    checkpoint: &Value,
    wallet: (&str, &str),
) -> std::result::Result<Normalized, Failure> {
    let (backend_name, backend_kind) = backend;
    if !is_dict(payload) {
        return Err(
            validation("Silent Payments scanner returned an unexpected payload")
                .retryable(true)
                .into(),
        );
    }
    binding(payload, plan, wallet.0, wallet.1)?;
    let txs = mappings(payload, "transactions", "txs");
    let mut records = vec![];
    for tx in &txs {
        records.extend(record(tx, backend_name)?);
    }
    let mut owned_outputs = std::collections::HashSet::new();
    for tx in &txs {
        for output in owned(tx, "outputs", "vout") {
            owned_outputs.insert(outpoint(output, field(tx, "txid"))?);
        }
    }
    // A dict comprehension: a later input's spender replaces an earlier one's.
    let mut spent_by: Vec<(String, String)> = vec![];
    for tx in &txs {
        for input in owned(tx, "inputs", "vin") {
            let key = outpoint(input, &NULL)?;
            let spender = txid(field(tx, "txid"))?;
            match spent_by.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = spender,
                None => spent_by.push((key, spender)),
            }
        }
    }
    let lowered = |value: &Value| {
        if kassiber_envelope::truthy(value) {
            kassiber_pytext::lower(&py_str(value))
        } else {
            String::new()
        }
    };
    let mut by_id: Vec<(String, &Value)> = vec![];
    for tx in &txs {
        let key = lowered(field(tx, "txid"));
        match by_id.iter_mut().find(|(k, _)| *k == key) {
            Some(slot) => slot.1 = tx,
            None => by_id.push((key, tx)),
        }
    }
    let empty = dict::<&str>([]);
    let mut outputs: Vec<(String, Value)> = vec![];
    for row in mappings(payload, "utxos", "outputs") {
        let key = lowered(field(row, "txid"));
        let tx = by_id
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(&empty, |(_, tx)| *tx);
        let point = outpoint(row, field(tx, "txid"))?;
        if !owned_outputs.contains(&point) {
            return Err(validation("Silent Payments UTXO rows must correspond to owned transaction outputs")
                .with_hint("Mark detected transaction outputs with silent_payment=true/owned=true/matched=true.")
                .into());
        }
        let value = utxo(row, tx, plan)?;
        match outputs.iter_mut().find(|(k, _)| *k == point) {
            Some(slot) => slot.1 = value,
            None => outputs.push((point, value)),
        }
    }
    for tx in &txs {
        for output in owned(tx, "outputs", "vout") {
            let script = get_or(output, "script_pubkey", &NULL);
            if kassiber_envelope::truthy(script)
                || kassiber_envelope::truthy(field(output, "scriptPubKey"))
            {
                let value = utxo(output, tx, plan)?;
                let key = format!(
                    "{}:{}",
                    py_str(field(&value, "txid")),
                    py_str(field(&value, "vout"))
                );
                if !outputs.iter().any(|(k, _)| *k == key) {
                    outputs.push((key, value));
                }
            }
        }
    }
    for (point, spender) in &spent_by {
        if let Some((_, output)) = outputs.iter_mut().find(|(k, _)| k == point) {
            set(output, "spent_by", s(spender.as_str()));
            set(output, "spent", Value::Bool(true));
        }
    }
    let complete = parse_bool(
        get_or(payload, "complete", field(payload, "scan_complete")),
        false,
    )?;
    let range = get(payload, "range")
        .filter(|r| is_dict(r))
        .cloned()
        .unwrap_or(empty);
    let covered = covers(&range, plan)?;
    let complete = complete && covered;
    let reason = str_or_none(get_or(
        payload,
        "degraded_reason",
        field(payload, "stop_reason"),
    ))
    .or_else(|| (!covered).then(|| "scan_range_incomplete".to_owned()));
    let degraded = parse_bool(field(payload, "degraded"), false)? || !complete;
    let mut next = if is_dict(checkpoint) {
        checkpoint.clone()
    } else {
        dict::<&str>([])
    };
    set(
        &mut next,
        "backend",
        dict([("name", s(backend_name)), ("kind", s(backend_kind))]),
    );
    let checkpoint_reason = match &reason {
        Some(reason) => s(reason.as_str()),
        None if !complete => s("scan_incomplete"),
        None => Value::Null,
    };
    set(
        &mut next,
        "silent_payment",
        dict([
            ("scan_complete", Value::Bool(complete)),
            ("degraded", Value::Bool(degraded)),
            ("degraded_reason", checkpoint_reason),
            ("scan_mode", s(plan.scan_mode.as_str())),
            ("material_format", s(plan.material_format.as_str())),
            ("start_height", int_value(plan.start_height.clone())),
            ("start_date", opt(plan.start_date.clone())),
            ("full_history", Value::Bool(plan.full_history)),
            ("range", redact_secret_value(&range)),
        ]),
    );
    let meta_reason = reason.unwrap_or_else(|| {
        if complete {
            String::new()
        } else {
            "scan_incomplete".into()
        }
    });
    let mut meta = dict([
        ("freshness_checkpoint", next),
        ("silent_payment_scan_complete", Value::Bool(complete)),
        ("silent_payment_degraded", Value::Bool(degraded)),
        ("silent_payment_degraded_reason", s(meta_reason.as_str())),
        (
            "silent_payment_transactions_seen",
            Value::Int(txs.len().into()),
        ),
        (
            "silent_payment_outputs_seen",
            Value::Int(outputs.len().into()),
        ),
    ]);
    if degraded {
        for key in [
            "partial_success",
            "blocking_reports",
            "utxos_skipped_partial",
        ] {
            set(&mut meta, key, Value::Bool(true));
        }
    } else {
        set(
            &mut meta,
            "utxos",
            Value::List(outputs.into_iter().map(|(_, v)| v).collect()),
        );
    }
    Ok(Normalized {
        records,
        meta,
        complete,
    })
}
