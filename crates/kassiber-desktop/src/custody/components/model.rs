//! The guided custody-component builder's pure model (`transfers-custody/guidedComponentModel.ts`)
//! and the local validator it runs on every edit (`lib/custodyComponentBulk.ts`'s
//! `previewCustodyComponentBatch`, `decimalBtcToMsat`, `buildCustodyBulkRequest`), plus the
//! issue texts (`custodyComponentIssues.tsx`). Exact msat as `i128`; no float on any amount.
//!
//! The form serializes to the very spec object `ui.transfers.components.{plan,apply}` accept;
//! the validator reads that object, so what is checked is exactly what would be sent.

use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use serde_json::{Map, Value, json};

use crate::i18n::{t, tr};

// ---- Exact integers ----------------------------------------------------------------------------

const SQLITE_MAX: i128 = 9_223_372_036_854_775_807;
const MSAT_PER_BTC: i128 = 100_000_000_000;

/// `exactNonnegativeIntegerAsBigInt`: a safe non-negative JSON integer, or an unsigned decimal
/// string of at most 19 digits within SQLite's range.
pub fn exact_nonneg(v: &Value) -> Option<i128> {
    match v {
        Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                return (u <= 9_007_199_254_740_991).then_some(u as i128);
            }
            // A float that happens to be a safe integer (`Number.isSafeInteger(3.0)`).
            let f = n.as_f64()?;
            (f >= 0.0 && f.fract() == 0.0 && f <= 9_007_199_254_740_991.0).then_some(f as i128)
        }
        Value::String(s) => {
            if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let norm = s.trim_start_matches('0');
            let norm = if norm.is_empty() { "0" } else { norm };
            if norm.len() > 19 {
                return None;
            }
            let p: i128 = norm.parse().ok()?;
            (p <= SQLITE_MAX).then_some(p)
        }
        _ => None,
    }
}

/// `decimalBtcToMsat`: the daemon's Decimal + ROUND_HALF_UP BTC-to-msat boundary.
pub fn decimal_btc_to_msat(value: &str) -> Option<i128> {
    if value.len() > 128 {
        return None;
    }
    let s = value.trim();
    let s = s.strip_prefix('+').unwrap_or(s);
    let (mantissa, exp) = match s.find(['e', 'E']) {
        Some(i) => (&s[..i], Some(&s[i + 1..])),
        None => (s, None),
    };
    let exponent: i64 = match exp {
        Some(e) => {
            let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            e.parse().ok()?
        }
        None => 0,
    };
    let (whole, fraction) = match mantissa.split_once('.') {
        Some((w, f)) => (w, f),
        None => (mantissa, ""),
    };
    let digits_ok = |x: &str| x.bytes().all(|b| b.is_ascii_digit());
    if !digits_ok(whole) || !digits_ok(fraction) {
        return None;
    }
    // `\d+(\.\d*)?` or `\.\d+`.
    if whole.is_empty() && (fraction.is_empty() || !mantissa.starts_with('.')) {
        return None;
    }
    if exponent.abs() > 1000 {
        return None;
    }
    let whole = if whole.is_empty() { "0" } else { whole };
    let coeff_digits = format!("{whole}{fraction}");
    let trimmed = coeff_digits.trim_start_matches('0');
    let trimmed = if trimmed.is_empty() { "0" } else { trimmed };
    if trimmed.len() > 19 {
        return None;
    }
    let coefficient: i128 = trimmed.parse().ok()?;
    let scale = exponent + 11 - fraction.len() as i64;
    if scale >= 0 {
        let p = 10i128.checked_pow(u32::try_from(scale).ok()?)?;
        return coefficient.checked_mul(p);
    }
    let neg = u32::try_from(-scale).ok()?;
    let Some(divisor) = 10i128.checked_pow(neg) else {
        return Some(0);
    };
    let q = coefficient / divisor;
    let r = coefficient % divisor;
    Some(q + if r * 2 >= divisor { 1 } else { 0 })
}

/// `msatToBtcInput`: exact msat as a lossless BTC decimal string.
pub fn msat_to_btc_input(v: &Value) -> String {
    let msat = match v {
        Value::Number(n) => match n.as_i64() {
            Some(i) => i as i128,
            None => return String::new(),
        },
        Value::String(s) => match s.trim().parse::<i128>() {
            Ok(i) => i,
            Err(_) => return String::new(),
        },
        _ => return String::new(),
    };
    let neg = msat < 0;
    let a = msat.abs();
    let whole = a / MSAT_PER_BTC;
    let frac = format!("{:011}", a % MSAT_PER_BTC);
    let frac = frac.trim_end_matches('0');
    format!(
        "{}{whole}{}",
        if neg { "-" } else { "" },
        if frac.is_empty() {
            String::new()
        } else {
            format!(".{frac}")
        }
    )
}

/// `formatCustodyMsat`: `1.5 BTC`, a sign for a negative difference.
pub fn format_custody_msat(msat: i128, asset: &str) -> String {
    let sign = if msat < 0 { "−" } else { "" };
    let a = msat.abs();
    let frac = format!("{:011}", a % MSAT_PER_BTC);
    let frac = frac.trim_end_matches('0');
    format!(
        "{sign}{}{} {asset}",
        a / MSAT_PER_BTC,
        if frac.is_empty() {
            String::new()
        } else {
            format!(".{frac}")
        }
    )
}

/// `formatCustodyMsat` of a JSON exact value (`— BTC` when unreadable).
pub fn format_custody_msat_v(v: &Value, asset: &str) -> String {
    let parsed = match v {
        Value::Number(n) => n.as_i64().map(i128::from),
        Value::String(s) => s.trim().parse::<i128>().ok(),
        _ => None,
    };
    match parsed {
        Some(m) => format_custody_msat(m, asset),
        None => format!("— {asset}"),
    }
}

/// `formatCustodyExactInteger` in the UI locale (en `1,234`, de-AT `1 234`).
pub fn format_exact_integer(v: &Value) -> String {
    match exact_nonneg(v) {
        Some(n) => {
            let digits = n.to_string();
            let sep = if crate::i18n::lang() == crate::i18n::Lang::De {
                '\u{a0}'
            } else {
                ','
            };
            let mut out = String::new();
            for (i, c) in digits.chars().enumerate() {
                if i > 0 && (digits.len() - i) % 3 == 0 {
                    out.push(sep);
                }
                out.push(c);
            }
            out
        }
        None => match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        },
    }
}

// ---- Time ------------------------------------------------------------------------------------

/// The local zone's offset from UTC (seconds) at a UTC instant, as the web's `Date` uses.
fn local_offset(utc_secs: i64) -> i64 {
    crate::localtime::offset_secs(utc_secs)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Parses an ISO date-time the way `Date` does for the shapes the form produces: a date
/// alone is UTC, a date-time without a zone is local, `Z` or `±HH:MM` is explicit. Epoch ms.
pub fn parse_instant(s: &str) -> Option<i64> {
    let s = s.trim();
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> {
        let x = s.get(r)?;
        x.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| x.parse().ok())
            .flatten()
    };
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) {
        return None;
    }
    let day_ms = days_from_civil(y, mo, d) * 86_400_000;
    if b.len() == 10 {
        return Some(day_ms);
    }
    if b[10] != b'T' && b[10] != b' ' {
        return None;
    }
    let rest = &s[11..];
    let (time, zone) = match rest.find(['Z', 'z', '+', '-']) {
        Some(i) => (&rest[..i], Some(&rest[i..])),
        None => (rest, None),
    };
    let tb = time.as_bytes();
    if tb.len() < 5 || tb[2] != b':' {
        return None;
    }
    let tn = |r: std::ops::Range<usize>| -> Option<i64> {
        let x = time.get(r)?;
        x.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| x.parse().ok())
            .flatten()
    };
    let (h, mi) = (tn(0..2)?, tn(3..5)?);
    let mut sec = 0;
    let mut ms = 0;
    if tb.len() > 5 {
        if tb[5] != b':' || tb.len() < 8 {
            return None;
        }
        sec = tn(6..8)?;
        if tb.len() > 8 {
            if tb[8] != b'.' {
                return None;
            }
            let frac = &time[9..];
            if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let f3: String = frac.chars().chain("000".chars()).take(3).collect();
            ms = f3.parse().ok()?;
        }
    }
    if h > 24 || mi > 59 || sec > 59 {
        return None;
    }
    let naive = day_ms + ((h * 60 + mi) * 60 + sec) * 1000 + ms;
    match zone {
        Some(z) if z.eq_ignore_ascii_case("z") => Some(naive),
        Some(z) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            let body = z[1..].replace(':', "");
            if body.len() != 4 || !body.bytes().all(|c| c.is_ascii_digit()) {
                return None;
            }
            let off = body[..2].parse::<i64>().ok()? * 60 + body[2..].parse::<i64>().ok()?;
            Some(naive - sign * off * 60_000)
        }
        None => {
            // Local wall-clock time: guess the offset, then refine once across a DST change.
            let guess = naive / 1000 - local_offset(naive.div_euclid(1000));
            let off = local_offset(guess);
            Some(naive - off * 1000)
        }
    }
}

/// `Date.toISOString()`: `YYYY-MM-DDTHH:MM:SS.sssZ`.
pub fn iso_string(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

/// `occurredAtToRfc3339`: a `datetime-local` value as RFC 3339 UTC; unparsable stays raw.
pub fn occurred_at_to_rfc3339(value: &str) -> String {
    let raw = value.trim();
    if raw.is_empty() {
        return String::new();
    }
    match parse_instant(raw) {
        Some(ms) => iso_string(ms),
        None => raw.to_owned(),
    }
}

/// `rfc3339ToDatetimeLocal`: local `YYYY-MM-DDTHH:mm:ss`.
pub fn rfc3339_to_datetime_local(value: Option<&str>) -> String {
    let Some(ms) = value.filter(|v| !v.is_empty()).and_then(parse_instant) else {
        return String::new();
    };
    let secs = ms.div_euclid(1000);
    let local = secs + local_offset(secs);
    let days = local.div_euclid(86_400);
    let rem = local.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}",
        rem / 3600,
        rem / 60 % 60,
        rem % 60
    )
}

// ---- The form ---------------------------------------------------------------------------------

pub const ROLES: [&str; 6] = [
    "source",
    "destination",
    "fee",
    "retained",
    "external",
    "suspense",
];
pub const LOCATION_KINDS: [&str; 3] = ["transaction", "wallet", "untracked"];

pub fn is_sink_role(role: &str) -> bool {
    matches!(
        role,
        "destination" | "fee" | "external" | "retained" | "suspense"
    )
}

/// A loaded leg's resolved location and carried metadata (`GuidedLegOrigin`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Origin {
    pub transaction_id: Option<String>,
    pub anchor_transaction_id: Option<String>,
    pub wallet_id: Option<String>,
    pub rail: Option<String>,
    pub chain: Option<String>,
    pub network: Option<String>,
    pub exposure: Option<String>,
    pub conservation_unit: Option<String>,
}

/// `GuidedLegForm`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leg {
    pub key: String,
    pub origin_id: String,
    pub role: String,
    pub amount_btc: String,
    /// `manual` = the alias fields; `origin` = a loaded leg's resolved location.
    pub origin_mode: bool,
    pub origin: Option<Origin>,
    pub location_kind: String,
    pub transaction_ref: String,
    pub wallet_ref: String,
    pub untracked_wallet: String,
    pub occurred_at: String,
    pub original_occurred_at: Option<String>,
    pub original_occurred_at_input: Option<String>,
    pub asset: String,
    pub valuation_unit: String,
    pub valuation_amount: String,
    pub notes: String,
}

/// `GuidedAllocationForm`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub key: String,
    pub source_key: String,
    pub sink_key: String,
    pub amount_btc: String,
    pub sink_amount_btc: String,
}

/// `GuidedComponentFormState`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    pub component_type: String,
    pub conservation_mode: String,
    pub evidence_kind: String,
    pub evidence_grade: String,
    pub conversion_policy: String,
    pub conversion_reviewed: bool,
    pub notes: String,
    pub legs: Vec<Leg>,
    pub allocations: Vec<Allocation>,
}

static KEYS: AtomicUsize = AtomicUsize::new(0);

pub fn next_key(prefix: &str) -> String {
    format!("{prefix}-{}", KEYS.fetch_add(1, Ordering::Relaxed) + 1)
}

/// `createGuidedLeg`.
pub fn new_leg(role: &str) -> Leg {
    Leg {
        key: next_key("leg"),
        origin_id: String::new(),
        role: role.to_owned(),
        amount_btc: String::new(),
        origin_mode: false,
        origin: None,
        location_kind: "transaction".into(),
        transaction_ref: String::new(),
        wallet_ref: String::new(),
        untracked_wallet: String::new(),
        occurred_at: String::new(),
        original_occurred_at: None,
        original_occurred_at_input: None,
        asset: "BTC".into(),
        valuation_unit: String::new(),
        valuation_amount: String::new(),
        notes: String::new(),
    }
}

/// `createGuidedAllocation`.
pub fn new_allocation() -> Allocation {
    Allocation {
        key: next_key("alloc"),
        source_key: String::new(),
        sink_key: String::new(),
        amount_btc: String::new(),
        sink_amount_btc: String::new(),
    }
}

/// `createInitialGuidedForm`: a one-source migration with a destination and a fee.
pub fn initial_form() -> Form {
    Form {
        component_type: "manual_bridge".into(),
        conservation_mode: "quantity".into(),
        evidence_kind: "manual_migration_review".into(),
        evidence_grade: "reviewed".into(),
        conversion_policy: String::new(),
        conversion_reviewed: false,
        notes: String::new(),
        legs: vec![new_leg("source"), new_leg("destination"), new_leg("fee")],
        allocations: vec![],
    }
}

fn leg_occurred_at(leg: &Leg) -> String {
    match (&leg.original_occurred_at, &leg.original_occurred_at_input) {
        (Some(orig), Some(input)) if &leg.occurred_at == input => orig.clone(),
        _ => occurred_at_to_rfc3339(&leg.occurred_at),
    }
}

fn leg_to_spec(leg: &Leg, mode: &str) -> Value {
    let mut spec = Map::new();
    spec.insert(
        "id".into(),
        json!(if leg.origin_id.is_empty() {
            &leg.key
        } else {
            &leg.origin_id
        }),
    );
    spec.insert("role".into(), json!(leg.role));
    let amount = leg.amount_btc.trim();
    if !amount.is_empty() {
        spec.insert("amount_btc".into(), json!(amount));
    }
    let origin = leg.origin.as_ref().filter(|_| leg.origin_mode);
    if leg.role == "suspense" {
        let at = leg_occurred_at(leg);
        if !at.is_empty() {
            spec.insert("occurred_at".into(), json!(at));
        }
    } else if let Some(o) = origin {
        if let Some(tx) = &o.transaction_id {
            spec.insert("transaction_id".into(), json!(tx));
            spec.insert(
                "anchor_transaction_id".into(),
                json!(
                    o.anchor_transaction_id
                        .clone()
                        .unwrap_or_else(|| tx.clone())
                ),
            );
        }
        if let Some(a) = &o.anchor_transaction_id {
            spec.insert("anchor_transaction_id".into(), json!(a));
        }
        if let Some(w) = &o.wallet_id {
            spec.insert("wallet_id".into(), json!(w));
        }
        let at = leg_occurred_at(leg);
        if !at.is_empty() {
            spec.insert("occurred_at".into(), json!(at));
        }
    } else {
        if !leg.origin_id.is_empty() {
            spec.insert("location_ref".into(), Value::Null);
        }
        let put = |spec: &mut Map<String, Value>, k: &str, v: &str| {
            let v = v.trim();
            if !v.is_empty() {
                spec.insert(k.into(), json!(v));
            }
        };
        match leg.location_kind.as_str() {
            "transaction" => put(&mut spec, "transaction", &leg.transaction_ref),
            "wallet" => put(&mut spec, "wallet", &leg.wallet_ref),
            "untracked" => put(&mut spec, "untracked_wallet", &leg.untracked_wallet),
            _ => {}
        }
        if leg.location_kind != "transaction" {
            let at = leg_occurred_at(leg);
            if !at.is_empty() {
                spec.insert("occurred_at".into(), json!(at));
            }
        }
    }
    if let Some(o) = origin {
        for (k, v) in [
            ("rail", &o.rail),
            ("chain", &o.chain),
            ("network", &o.network),
            ("exposure", &o.exposure),
            ("conservation_unit", &o.conservation_unit),
        ] {
            if let Some(v) = v.as_ref().filter(|v| !v.is_empty()) {
                spec.insert(k.into(), json!(v));
            }
        }
    }
    let asset = leg.asset.trim();
    if !asset.is_empty() && (leg.origin_mode || !asset.eq_ignore_ascii_case("BTC")) {
        spec.insert("asset".into(), json!(asset));
    }
    if mode == "conversion" || leg.origin_mode {
        let unit = leg.valuation_unit.trim();
        let amount = leg.valuation_amount.trim();
        if !unit.is_empty() {
            spec.insert("valuation_unit".into(), json!(unit));
        }
        if !amount.is_empty() {
            spec.insert("valuation_amount".into(), json!(amount));
        }
    }
    let notes = leg.notes.trim();
    if !notes.is_empty() {
        spec.insert("notes".into(), json!(notes));
    }
    Value::Object(spec)
}

fn allocation_to_spec(a: &Allocation, legs: &[Leg], mode: &str) -> Value {
    let ord = |k: &str| {
        legs.iter()
            .position(|l| l.key == k)
            .map(|i| i as i64)
            .unwrap_or(-1)
    };
    let amount = decimal_btc_to_msat(a.amount_btc.trim())
        .map(|m| m.to_string())
        .unwrap_or_default();
    let sink = if mode == "conversion" || !a.sink_amount_btc.trim().is_empty() {
        decimal_btc_to_msat(a.sink_amount_btc.trim())
            .map(|m| m.to_string())
            .unwrap_or_default()
    } else {
        amount.clone()
    };
    json!({"source_ordinal": ord(&a.source_key), "sink_ordinal": ord(&a.sink_key), "source_amount_msat": amount, "sink_amount_msat": sink})
}

/// `formToComponentSpec`: the exact spec object the daemon takes.
pub fn form_to_spec(form: &Form) -> Value {
    let mut spec = Map::new();
    spec.insert("component_type".into(), json!(form.component_type));
    spec.insert("conservation_mode".into(), json!(form.conservation_mode));
    let ek = form.evidence_kind.trim();
    let eg = form.evidence_grade.trim();
    if !ek.is_empty() {
        spec.insert("evidence_kind".into(), json!(ek));
    }
    if !eg.is_empty() {
        spec.insert("evidence_grade".into(), json!(eg));
    }
    let policy = form.conversion_policy.trim();
    spec.insert(
        "conversion_policy".into(),
        if policy.is_empty() {
            Value::Null
        } else {
            json!(policy)
        },
    );
    spec.insert(
        "conversion_reviewed".into(),
        json!(form.conversion_reviewed),
    );
    spec.insert("notes".into(), json!(form.notes.trim()));
    spec.insert(
        "legs".into(),
        Value::Array(
            form.legs
                .iter()
                .map(|l| leg_to_spec(l, &form.conservation_mode))
                .collect(),
        ),
    );
    let allocs: Vec<Value> = form
        .allocations
        .iter()
        .map(|a| allocation_to_spec(a, &form.legs, &form.conservation_mode))
        .collect();
    if !allocs.is_empty() {
        spec.insert("allocations".into(), Value::Array(allocs));
    }
    Value::Object(spec)
}

/// `formToDocument`: `{ components: [spec] }` as text, the reviewed plan's commitment.
pub fn form_to_document(form: &Form) -> String {
    json!({"components": [form_to_spec(form)]}).to_string()
}

/// `isGuidedEditableComponentType`.
pub fn is_guided_editable(component_type: &str) -> bool {
    component_type == "manual_bridge" || component_type == "swap"
}

fn opt_str(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}

/// `componentToFormState`: an existing component as the form, or Err when its shape cannot be
/// edited safely here (`guided_component_shape_unsupported`).
pub fn component_to_form(c: &Value) -> Result<Form, ()> {
    let ctype = c["component_type"].as_str().unwrap_or("");
    let mode = c["conservation_mode"].as_str().unwrap_or("");
    let legs = c["legs"].as_array().cloned().unwrap_or_default();
    let allocs = c["allocations"].as_array().cloned().unwrap_or_default();
    let ids: Vec<String> = legs
        .iter()
        .map(|l| l["id"].as_str().unwrap_or("").to_owned())
        .collect();
    let unit_bad = |l: &Value| {
        l["conservation_unit"]
            .as_str()
            .is_some_and(|u| !u.is_empty() && u != "msat")
    };
    let amount_bad = |l: &Value| matches!(&l["amount_msat"], Value::Number(n) if n.as_i64().is_none_or(|i| i.unsigned_abs() > 9_007_199_254_740_991));
    let suspense_bad = |l: &Value| {
        l["role"] == "suspense"
            && ["transaction_id", "wallet_id", "anchor_transaction_id"]
                .iter()
                .any(|k| l[*k].as_str().is_some_and(|s| !s.is_empty()))
    };
    let unsupported = !is_guided_editable(ctype)
        || !matches!(mode, "quantity" | "conversion")
        || legs
            .iter()
            .any(|l| unit_bad(l) || amount_bad(l) || suspense_bad(l))
        || ids.iter().collect::<HashSet<_>>().len() != ids.len()
        || allocs.iter().any(|a| {
            !ids.iter().any(|i| a["source_leg_id"].as_str() == Some(i))
                || !ids.iter().any(|i| a["sink_leg_id"].as_str() == Some(i))
        });
    if unsupported {
        return Err(());
    }
    let mut key_of: BTreeMap<String, String> = BTreeMap::new();
    let mut out_legs = vec![];
    for l in &legs {
        let role = l["role"].as_str().unwrap_or("");
        if !ROLES.contains(&role) {
            return Err(());
        }
        let mut base = new_leg(role);
        key_of.insert(l["id"].as_str().unwrap_or("").to_owned(), base.key.clone());
        let at = opt_str(l, "occurred_at");
        base.origin_id = l["id"].as_str().unwrap_or("").to_owned();
        base.amount_btc = msat_to_btc_input(&l["amount_msat"]);
        base.asset = opt_str(l, "asset")
            .filter(|a| !a.is_empty())
            .unwrap_or_else(|| "BTC".into());
        base.valuation_unit = opt_str(l, "valuation_unit").unwrap_or_default();
        base.valuation_amount = match &l["valuation_amount"] {
            Value::Null => String::new(),
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
        base.notes = opt_str(l, "notes").unwrap_or_default();
        base.occurred_at = rfc3339_to_datetime_local(at.as_deref());
        base.original_occurred_at = at.clone();
        base.original_occurred_at_input = Some(rfc3339_to_datetime_local(at.as_deref()));
        base.origin_mode = true;
        base.origin = Some(Origin {
            transaction_id: opt_str(l, "transaction_id"),
            anchor_transaction_id: opt_str(l, "anchor_transaction_id"),
            wallet_id: opt_str(l, "wallet_id"),
            rail: opt_str(l, "rail"),
            chain: opt_str(l, "chain"),
            network: opt_str(l, "network"),
            exposure: opt_str(l, "exposure"),
            conservation_unit: opt_str(l, "conservation_unit"),
        });
        out_legs.push(base);
    }
    let allocations = allocs
        .iter()
        .map(|a| Allocation {
            key: next_key("alloc"),
            source_key: key_of
                .get(a["source_leg_id"].as_str().unwrap_or(""))
                .cloned()
                .unwrap_or_default(),
            sink_key: key_of
                .get(a["sink_leg_id"].as_str().unwrap_or(""))
                .cloned()
                .unwrap_or_default(),
            amount_btc: msat_to_btc_input(&a["source_amount_msat"]),
            sink_amount_btc: msat_to_btc_input(&a["sink_amount_msat"]),
        })
        .collect();
    Ok(Form {
        component_type: ctype.to_owned(),
        conservation_mode: mode.to_owned(),
        evidence_kind: opt_str(c, "evidence_kind").unwrap_or_default(),
        evidence_grade: opt_str(c, "evidence_grade").unwrap_or_default(),
        conversion_policy: opt_str(c, "conversion_policy").unwrap_or_default(),
        conversion_reviewed: c["conversion_reviewed"].as_bool().unwrap_or(false),
        notes: opt_str(c, "notes").unwrap_or_default(),
        legs: out_legs,
        allocations,
    })
}

/// A reviewed plan's binding: the exact document and the book it was planned in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commitment {
    pub document: String,
    pub scope: String,
    pub input_version: i64,
}

/// `canConfirmGuidedPlan`.
pub fn can_confirm(c: Option<&Commitment>, document: &str, scope: &str) -> bool {
    c.is_some_and(|c| {
        c.input_version >= 0
            && c.input_version <= 9_007_199_254_740_991
            && c.document == document
            && c.scope == scope
    })
}

/// `hasGuidedFormInput`: keep the untouched builder quiet.
pub fn has_input(form: &Form) -> bool {
    !form.notes.trim().is_empty()
        || form.legs.iter().any(|l| {
            [
                &l.amount_btc,
                &l.transaction_ref,
                &l.wallet_ref,
                &l.untracked_wallet,
                &l.occurred_at,
                &l.valuation_unit,
                &l.valuation_amount,
                &l.notes,
            ]
            .iter()
            .any(|v| !v.trim().is_empty())
        })
        || form.allocations.iter().any(|a| {
            [
                &a.source_key,
                &a.sink_key,
                &a.amount_btc,
                &a.sink_amount_btc,
            ]
            .iter()
            .any(|v| !v.trim().is_empty())
        })
}

/// `bindGuidedPlanScope`: apply to the scope the server planned in, never a display identity.
pub fn bind_plan_scope(args: &Value, plan: &Value) -> Option<Value> {
    let w = plan["workspace_id"]
        .as_str()
        .filter(|s| !s.trim().is_empty())?;
    let p = plan["profile_id"]
        .as_str()
        .filter(|s| !s.trim().is_empty())?;
    let mut out = args.clone();
    out["workspace"] = json!(w);
    out["profile"] = json!(p);
    Some(out)
}

/// `buildCustodyBulkRequest`: the plan/apply payload for new components.
pub fn bulk_request(preview: &Preview, activate: bool) -> Value {
    let comps: Vec<Value> = preview
        .components
        .iter()
        .map(|c| {
            let mut c = c.clone();
            if let Some(o) = c.as_object_mut() {
                o.remove("activate");
            }
            c
        })
        .collect();
    json!({"components": comps, "activate": activate})
}

// ---- Local validation (previewCustodyComponentBatch) ------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Issue {
    pub code: &'static str,
    pub values: Vec<(&'static str, String)>,
}

fn issue(code: &'static str, values: &[(&'static str, String)]) -> Issue {
    Issue {
        code,
        values: values.to_vec(),
    }
}

#[derive(Clone, Debug, Default)]
pub struct Preview {
    pub components: Vec<Value>,
    pub structural: Vec<Issue>,
    pub activation: Vec<Issue>,
}

fn has_text(v: &Value) -> bool {
    v.as_str().is_some_and(|s| !s.trim().is_empty())
}

fn amount_valid(leg: &Value) -> bool {
    let has_msat = leg.get("amount_msat").is_some();
    let has_btc = leg.get("amount_btc").is_some();
    if has_msat == has_btc {
        return false;
    }
    if has_msat {
        return exact_nonneg(&leg["amount_msat"]).is_some();
    }
    match &leg["amount_btc"] {
        Value::Number(n) => {
            n.as_f64().is_some_and(|f| f.is_finite() && f >= 0.0)
                && decimal_btc_to_msat(&n.to_string()).is_some_and(|a| a <= SQLITE_MAX)
        }
        Value::String(s) => decimal_btc_to_msat(s).is_some_and(|a| a <= SQLITE_MAX),
        _ => false,
    }
}

fn amount_msat(leg: &Value) -> Option<i128> {
    if let Some(m) = exact_nonneg(&leg["amount_msat"]) {
        return Some(m);
    }
    match &leg["amount_btc"] {
        Value::String(s) => decimal_btc_to_msat(s),
        Value::Number(n) => decimal_btc_to_msat(&n.to_string()),
        _ => None,
    }
}

fn norm_asset(leg: &Value) -> String {
    if has_text(&leg["asset"]) {
        leg["asset"].as_str().unwrap().trim().to_uppercase()
    } else {
        "BTC".into()
    }
}

fn norm_exposure(leg: &Value) -> String {
    if has_text(&leg["exposure"]) {
        return leg["exposure"].as_str().unwrap().trim().to_owned();
    }
    let a = norm_asset(leg);
    if a == "BTC" || a == "LBTC" {
        "bitcoin".into()
    } else {
        a.to_lowercase()
    }
}

fn norm_unit(leg: &Value) -> String {
    if has_text(&leg["conservation_unit"]) {
        return leg["conservation_unit"].as_str().unwrap().trim().to_owned();
    }
    let a = norm_asset(leg);
    if a == "BTC" || a == "LBTC" {
        "msat".into()
    } else {
        "asset-quantum".into()
    }
}

fn has_anchor(leg: &Value) -> bool {
    [
        "transaction",
        "transaction_ref",
        "transaction_id",
        "anchor_transaction_id",
    ]
    .iter()
    .any(|k| has_text(&leg[*k]))
}

fn has_wallet(leg: &Value) -> bool {
    ["wallet", "wallet_ref", "wallet_id", "untracked_wallet"]
        .iter()
        .any(|k| has_text(&leg[*k]))
}

fn valuation_token_ok(s: &str) -> bool {
    let mut c = s.chars();
    c.next().is_some_and(|f| f.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || "_.:-".contains(x))
}

fn alloc_ordinal(a: &Value, kind: &str, legs: &[&Value]) -> Option<usize> {
    if let Some(n) = a[format!("{kind}_ordinal")].as_i64()
        && n >= 0
    {
        return Some(n as usize);
    }
    let id = a[format!("{kind}_leg_id")]
        .as_str()
        .filter(|s| !s.trim().is_empty())?;
    legs.iter().position(|l| l["id"].as_str() == Some(id))
}

/// `previewCustodyComponentBatch` over a parsed document.
pub fn preview(doc: &Value) -> Preview {
    let mut st: Vec<Issue> = vec![];
    let mut act: Vec<Issue> = vec![];
    let raw = match doc {
        Value::Array(a) => Some(a.clone()),
        Value::Object(o) => o.get("components").and_then(Value::as_array).cloned(),
        _ => None,
    };
    let Some(raw) = raw.filter(|r| !r.is_empty()) else {
        return Preview {
            structural: vec![issue("documentRequired", &[])],
            ..Preview::default()
        };
    };
    let components: Vec<Value> = raw.iter().filter(|c| c.is_object()).cloned().collect();
    for (i, c) in raw.iter().enumerate() {
        if !c.is_object() {
            st.push(issue(
                "componentObjectRequired",
                &[("component", (i + 1).to_string())],
            ));
        }
    }
    for (ci, comp) in raw.iter().enumerate() {
        if !comp.is_object() {
            continue;
        }
        let cn = (ci + 1).to_string();
        let cv = [("component", cn.clone())];
        let ctype = comp
            .get("component_type")
            .cloned()
            .unwrap_or(json!("manual_bridge"));
        let ctype_s = ctype.as_str().unwrap_or("");
        if !has_text(&ctype) || !["swap", "manual_bridge"].contains(&ctype_s) {
            st.push(issue("componentTypeUnsupported", &cv));
        }
        let mode = comp
            .get("conservation_mode")
            .cloned()
            .unwrap_or(json!("quantity"));
        let mode_s = mode.as_str().unwrap_or("").to_owned();
        if mode != json!("quantity") && mode != json!("conversion") {
            st.push(issue("conservationModeUnsupported", &cv));
        }
        match comp.get("conversion_policy") {
            None | Some(Value::Null) => {}
            Some(Value::String(p)) if p.trim().is_empty() || valuation_token_ok(p.trim()) => {}
            Some(_) => st.push(issue("conversionPolicyInvalid", &cv)),
        }
        if comp
            .get("conversion_reviewed")
            .is_some_and(|v| !v.is_boolean())
        {
            st.push(issue("conversionReviewedInvalid", &cv));
        }
        let Some(raw_legs) = comp["legs"].as_array().filter(|l| l.len() >= 2) else {
            st.push(issue("legsRequired", &cv));
            continue;
        };
        let legs: Vec<&Value> = raw_legs.iter().filter(|l| l.is_object()).collect();
        for (li, l) in raw_legs.iter().enumerate() {
            if !l.is_object() {
                st.push(issue(
                    "legObjectRequired",
                    &[("component", cn.clone()), ("leg", (li + 1).to_string())],
                ));
            }
        }
        let mut ids: HashSet<String> = HashSet::new();
        let mut balance: Vec<(String, i128, i128)> = vec![];
        let mut val_balance: Vec<(String, i128, i128)> = vec![];
        let (mut sources, mut owned, mut anchors, mut suspense) = (0, 0, 0, 0);
        for (li, leg) in legs.iter().enumerate() {
            let values = [("component", cn.clone()), ("leg", (li + 1).to_string())];
            let role = leg["role"].as_str().unwrap_or("");
            if !has_text(&leg["role"]) || !ROLES.contains(&role) {
                st.push(issue("roleUnsupported", &values));
                continue;
            }
            if has_text(&leg["id"]) {
                let id = leg["id"].as_str().unwrap().to_owned();
                if ids.contains(&id) {
                    let mut v = values.to_vec();
                    v.push(("id", id.clone()));
                    st.push(Issue {
                        code: "legIdDuplicate",
                        values: v,
                    });
                }
                ids.insert(id);
            }
            if !amount_valid(leg) {
                st.push(issue("amountInvalid", &values));
            }
            let amount = amount_msat(leg);
            let positive = amount.is_some_and(|a| a > 0);
            if positive && ["source", "destination", "retained"].contains(&role) && !has_anchor(leg)
            {
                if !has_wallet(leg) {
                    st.push(issue("transactionlessWalletRequired", &values));
                }
                if !has_text(&leg["occurred_at"]) {
                    st.push(issue("transactionlessTimeRequired", &values));
                }
            }
            if has_anchor(leg) {
                anchors += 1;
            }
            if role == "source" && positive {
                sources += 1;
            }
            if ["destination", "retained"].contains(&role) && positive {
                owned += 1;
            }
            if role == "suspense" && positive {
                suspense += 1;
                if has_anchor(leg) || has_wallet(leg) {
                    act.push(issue("suspenseLocationInvalid", &values));
                }
                if !has_text(&leg["occurred_at"]) {
                    act.push(issue("suspenseTimeRequired", &values));
                }
            }
            if let Some(a) = amount {
                let key = format!("{}\u{0}{}", norm_exposure(leg), norm_unit(leg));
                let row = match balance.iter_mut().find(|r| r.0 == key) {
                    Some(r) => r,
                    None => {
                        balance.push((key, 0, 0));
                        balance.last_mut().unwrap()
                    }
                };
                if role == "source" {
                    row.1 += a;
                } else if is_sink_role(role) {
                    row.2 += a;
                }
            }
            let vunit = leg["valuation_unit"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.trim().to_owned());
            let vprovided = !leg["valuation_amount"].is_null();
            let vamount = exact_nonneg(&leg["valuation_amount"]);
            if vunit.is_some() != vprovided {
                st.push(issue("valuationPairRequired", &values));
            }
            if vprovided && vamount.is_none() {
                st.push(issue("valuationAmountInvalid", &values));
            }
            if vunit.as_deref().is_some_and(|u| !valuation_token_ok(u)) {
                st.push(issue("valuationTokenInvalid", &values));
            }
            if mode_s == "conversion" {
                let material = positive || vamount.is_some_and(|v| v > 0);
                if material && (vunit.is_none() || vamount.is_none()) {
                    act.push(issue("conversionValuationRequired", &values));
                }
                if let (Some(u), Some(v)) = (&vunit, vamount) {
                    let row = match val_balance.iter_mut().find(|r| &r.0 == u) {
                        Some(r) => r,
                        None => {
                            val_balance.push((u.clone(), 0, 0));
                            val_balance.last_mut().unwrap()
                        }
                    };
                    if role == "source" {
                        row.1 += v;
                    } else if is_sink_role(role) {
                        row.2 += v;
                    }
                }
            }
        }
        if sources == 0 {
            act.push(issue("sourceRequired", &cv));
        }
        if owned == 0 {
            act.push(issue("ownedDestinationRequired", &cv));
        }
        if anchors == 0 {
            act.push(issue("anchorRequired", &cv));
        }
        if suspense > 0 {
            let grade_ok = comp["evidence_grade"]
                .as_str()
                .is_some_and(|g| !g.trim().is_empty() && g.trim().eq_ignore_ascii_case("reviewed"));
            if ctype_s != "manual_bridge" || !grade_ok {
                act.push(issue("suspenseReviewRequired", &cv));
            }
            if mode_s != "quantity" {
                act.push(issue("suspenseQuantityModeRequired", &cv));
            }
        }
        if mode_s == "quantity" {
            for (key, s, k) in &balance {
                if s != k {
                    let (exp, unit) = key.split_once('\u{0}').unwrap_or((key, ""));
                    act.push(issue(
                        "quantityUnbalanced",
                        &[
                            ("component", cn.clone()),
                            ("exposure", exp.to_owned()),
                            ("unit", unit.to_owned()),
                            ("sources", s.to_string()),
                            ("sinks", k.to_string()),
                        ],
                    ));
                }
            }
        } else {
            if !has_text(&comp["conversion_policy"]) || comp["conversion_reviewed"] != json!(true) {
                act.push(issue("conversionReviewRequired", &cv));
            }
            if val_balance.is_empty() {
                act.push(issue(
                    "conversionValuationRequired",
                    &[("component", cn.clone()), ("leg", "0".into())],
                ));
            }
            for (unit, s, k) in &val_balance {
                if s != k {
                    act.push(issue(
                        "conversionValuationUnbalanced",
                        &[
                            ("component", cn.clone()),
                            ("unit", unit.clone()),
                            ("sources", s.to_string()),
                            ("sinks", k.to_string()),
                        ],
                    ));
                }
            }
        }
        let pos = |l: &&&Value| amount_msat(l).unwrap_or(0) > 0;
        let positive_sources: Vec<&&Value> = legs
            .iter()
            .filter(|l| l["role"] == "source")
            .filter(pos)
            .collect();
        let positive_sinks: Vec<&&Value> = legs
            .iter()
            .filter(|l| is_sink_role(l["role"].as_str().unwrap_or("")))
            .filter(pos)
            .collect();
        let attributed = positive_sinks
            .iter()
            .filter(|l| ["fee", "external", "suspense"].contains(&l["role"].as_str().unwrap_or("")))
            .count();
        let owned_sinks = positive_sinks
            .iter()
            .filter(|l| ["destination", "retained"].contains(&l["role"].as_str().unwrap_or("")))
            .count();
        let needs_allocations = if suspense > 0 {
            true
        } else if mode_s == "conversion" {
            positive_sources.len() != 1 || positive_sinks.len() != 1
        } else {
            positive_sources.len() > 1 && !(owned_sinks == 1 && attributed == 0)
        };
        if mode_s == "conversion" && (positive_sources.len() != 1 || owned_sinks != 1) {
            act.push(issue("conversionTopologyUnsupported", &cv));
        }
        if comp
            .get("allocations")
            .is_some_and(|a| !a.is_null() && !a.is_array())
        {
            st.push(issue("allocationsInvalid", &cv));
        }
        let allocations = comp["allocations"].as_array().cloned().unwrap_or_default();
        if needs_allocations && allocations.is_empty() {
            act.push(issue(
                if suspense > 0 {
                    "suspenseAllocationRequired"
                } else {
                    "allocationsRequired"
                },
                &cv,
            ));
        }
        let mut src_cov: BTreeMap<usize, i128> = BTreeMap::new();
        let mut sink_cov: BTreeMap<usize, i128> = BTreeMap::new();
        let mut edges: HashSet<String> = HashSet::new();
        for (ai, a) in allocations.iter().enumerate() {
            let av = [
                ("component", cn.clone()),
                ("allocation", (ai + 1).to_string()),
            ];
            if !a.is_object() {
                st.push(issue("allocationObjectRequired", &av));
                continue;
            }
            let so = alloc_ordinal(a, "source", &legs);
            let ko = alloc_ordinal(a, "sink", &legs);
            let src = so.and_then(|i| legs.get(i)).copied();
            let snk = ko.and_then(|i| legs.get(i)).copied();
            if src.is_none_or(|l| l["role"] != "source") {
                st.push(issue("allocationSourceInvalid", &av));
            }
            if snk.is_none_or(|l| !is_sink_role(l["role"].as_str().unwrap_or(""))) {
                st.push(issue("allocationSinkInvalid", &av));
            }
            for field in ["source_amount_msat", "sink_amount_msat"] {
                if exact_nonneg(&a[field]).is_none() {
                    let mut v = av.to_vec();
                    v.push(("field", field.to_owned()));
                    st.push(Issue {
                        code: "allocationAmountInvalid",
                        values: v,
                    });
                }
            }
            if let (Some(si), Some(ki), Some(sl), Some(kl)) = (so, ko, src, snk)
                && sl["role"] == "source"
                && is_sink_role(kl["role"].as_str().unwrap_or(""))
            {
                let edge = format!("{si}:{ki}");
                if edges.contains(&edge) {
                    let mut v = av.to_vec();
                    v.push(("edge", edge.clone()));
                    st.push(Issue {
                        code: "allocationEdgeDuplicate",
                        values: v,
                    });
                }
                edges.insert(edge);
                let sa = exact_nonneg(&a["source_amount_msat"]);
                let ka = exact_nonneg(&a["sink_amount_msat"]);
                if let Some(x) = sa {
                    *src_cov.entry(si).or_default() += x;
                }
                if let Some(x) = ka {
                    *sink_cov.entry(ki).or_default() += x;
                }
                if mode_s == "quantity"
                    && let (Some(x), Some(y)) = (sa, ka)
                    && (x != y
                        || norm_exposure(sl) != norm_exposure(kl)
                        || norm_unit(sl) != norm_unit(kl))
                {
                    act.push(issue("allocationQuantityMismatch", &av));
                }
                if kl["role"] == "suspense" {
                    if !has_anchor(sl) {
                        act.push(issue("suspenseObservedSourceRequired", &av));
                    }
                    if norm_asset(sl) != norm_asset(kl) {
                        act.push(issue("suspenseAssetMismatch", &av));
                    }
                    let ts = sl["occurred_at"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .and_then(parse_instant);
                    let tk = kl["occurred_at"]
                        .as_str()
                        .filter(|s| !s.trim().is_empty())
                        .and_then(parse_instant);
                    if let (Some(x), Some(y)) = (ts, tk)
                        && x != y
                    {
                        act.push(issue("suspenseTimeMismatch", &av));
                    }
                }
            }
        }
        if !allocations.is_empty() {
            for l in &positive_sources {
                let ord = legs.iter().position(|x| std::ptr::eq(*x, **l)).unwrap();
                if let Some(expected) = amount_msat(l) {
                    let covered = src_cov.get(&ord).copied();
                    if covered != Some(expected) {
                        act.push(issue(
                            "allocationSourceCoverage",
                            &[
                                ("component", cn.clone()),
                                ("leg", (ord + 1).to_string()),
                                ("covered", covered.unwrap_or(0).to_string()),
                                ("expected", expected.to_string()),
                            ],
                        ));
                    }
                }
            }
            for l in &positive_sinks {
                let ord = legs.iter().position(|x| std::ptr::eq(*x, **l)).unwrap();
                if let Some(expected) = amount_msat(l) {
                    let covered = sink_cov.get(&ord).copied();
                    if covered != Some(expected) {
                        act.push(issue(
                            "allocationSinkCoverage",
                            &[
                                ("component", cn.clone()),
                                ("leg", (ord + 1).to_string()),
                                ("covered", covered.unwrap_or(0).to_string()),
                                ("expected", expected.to_string()),
                            ],
                        ));
                    }
                }
            }
        }
    }
    Preview {
        components,
        structural: st,
        activation: act,
    }
}

/// The local validation of a form.
pub fn preview_form(form: &Form) -> Preview {
    preview(&json!({"components": [form_to_spec(form)]}))
}

// ---- Issue texts (custodyComponentIssues.tsx) --------------------------------------------------

const BACKEND_ISSUES: [&str; 51] = [
    "active_lineage_conflict",
    "active_transaction_membership_conflict",
    "allocation_leg_invalid",
    "allocation_network_mismatch",
    "allocation_network_scope_invalid",
    "allocation_quantity_mismatch",
    "allocation_required",
    "allocation_sink_coverage_mismatch",
    "allocation_source_coverage_mismatch",
    "anchor_asset_mismatch",
    "anchor_chain_mismatch",
    "anchor_coverage_mismatch",
    "anchor_network_mismatch",
    "anchor_occurred_at_mismatch",
    "anchor_rail_mismatch",
    "anchor_transaction_identity_mismatch",
    "anchor_transaction_missing",
    "anchor_transaction_excluded",
    "anchor_transaction_retracted",
    "anchor_wallet_mismatch",
    "component_allocation_count_mismatch",
    "component_content_commitment_missing",
    "component_leg_count_mismatch",
    "conversion_not_reviewed",
    "conversion_fee_quantity_mismatch",
    "conversion_fee_valuation_mismatch",
    "conversion_policy_missing",
    "conversion_topology_unsupported",
    "conversion_valuation_missing",
    "custody_component_value_only_loss_unsupported",
    "custody_component_fee_orphaned",
    "custody_location_continuity_mismatch",
    "destination_anchor_direction_mismatch",
    "fee_source_asset_mismatch",
    "fee_source_scope_mismatch",
    "fee_source_wallet_mismatch",
    "leg_occurred_at_invalid",
    "leg_occurred_at_missing",
    "loss_anchor_direction_mismatch",
    "missing_owned_destination",
    "missing_source",
    "no_legs",
    "owned_leg_wallet_missing",
    "revision_link_invalid",
    "revision_link_missing",
    "source_anchor_direction_mismatch",
    "transaction_anchor_missing",
    "transactionless_leg_wallet_missing",
    "unbalanced_conversion_valuation",
    "unbalanced_quantity",
    "unresolved_value",
];

const BACKEND_ERRORS: [&str; 13] = [
    "custody_component_anchor_time_mismatch",
    "custody_component_draft_exists",
    "custody_component_incomplete",
    "custody_component_lineage_exists",
    "custody_component_membership_conflict",
    "custody_component_not_superseded",
    "custody_component_scope_mismatch",
    "custody_component_state_conflict",
    "custody_component_superseded",
    "custody_component_validation",
    "conflict",
    "not_found",
    "validation",
];

/// `custodyPreviewIssueText`.
pub fn local_issue_text(i: &Issue) -> String {
    let args: Vec<(&str, &str)> = i.values.iter().map(|(k, v)| (*k, v.as_str())).collect();
    tr(
        &format!("review:swap.components.localIssue.{}", i.code),
        &args,
    )
}

/// `custodyBackendIssueText`.
pub fn backend_issue_text(issue: &Value) -> String {
    let code = issue["code"].as_str().unwrap_or("");
    if BACKEND_ISSUES.contains(&code) {
        t(&format!("review:swap.components.backendIssue.{code}"))
    } else {
        let c = if code.is_empty() {
            t("review:swap.components.unknownIssue")
        } else {
            code.to_owned()
        };
        tr(
            "review:swap.components.backendIssue.unknown",
            &[("code", &c)],
        )
    }
}

/// `custodyMutationError`: a daemon refusal as its code's text and any validation issues.
pub fn mutation_error(error_detail: &Value) -> String {
    if !error_detail.is_object() {
        return t("review:swap.components.backendError.unexpected");
    }
    let code = error_detail["code"].as_str().unwrap_or("");
    let base = if BACKEND_ERRORS.contains(&code) {
        t(&format!("review:swap.components.backendError.{code}"))
    } else {
        let c = if code.is_empty() {
            t("review:swap.components.unknownIssue")
        } else {
            code.to_owned()
        };
        tr(
            "review:swap.components.backendError.unknown",
            &[("code", &c)],
        )
    };
    let mut lines = vec![base];
    if let Some(issues) = error_detail["details"]["validation"]["issues"].as_array() {
        lines.extend(
            issues
                .iter()
                .filter(|i| i.is_object())
                .map(backend_issue_text),
        );
    }
    lines.join("\n")
}

/// `custodyRoleLabel`.
pub fn role_label(role: &str) -> String {
    match role {
        "source" | "destination" | "fee" | "external" | "retained" | "suspense" | "unresolved" => {
            t(&format!("review:swap.components.role.{role}"))
        }
        _ => tr("review:swap.components.role.unknown", &[("role", role)]),
    }
}

/// `t(key, { defaultValue })`: the translation, or the raw value when the bundle lacks it.
pub fn t_or(key: &str, fallback: &str) -> String {
    if crate::i18n::has(key) {
        t(key)
    } else {
        fallback.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codes(v: &[Issue]) -> Vec<&'static str> {
        v.iter().map(|i| i.code).collect()
    }

    /// A balanced 1-source → destination + fee migration anchored to transactions.
    fn migration_form() -> Form {
        let mut f = initial_form();
        f.legs[0].amount_btc = "1.0".into();
        f.legs[0].transaction_ref = "tx-out".into();
        f.legs[1].amount_btc = "0.99".into();
        f.legs[1].transaction_ref = "tx-in".into();
        f.legs[2].amount_btc = "0.01".into();
        f.legs[2].transaction_ref = "tx-out".into();
        f
    }

    #[test]
    fn decimal_btc_rounds_like_the_daemon() {
        assert_eq!(decimal_btc_to_msat("1"), Some(100_000_000_000));
        assert_eq!(decimal_btc_to_msat("0.00000000001"), Some(1));
        assert_eq!(decimal_btc_to_msat("0.000000000005"), Some(1));
        assert_eq!(decimal_btc_to_msat("0.000000000004"), Some(0));
        assert_eq!(decimal_btc_to_msat(".5"), Some(50_000_000_000));
        assert_eq!(decimal_btc_to_msat("1e-3"), Some(100_000_000));
        assert_eq!(decimal_btc_to_msat("1."), Some(100_000_000_000));
        assert_eq!(decimal_btc_to_msat("abc"), None);
        assert_eq!(decimal_btc_to_msat("-1"), None);
        assert_eq!(decimal_btc_to_msat(""), None);
        assert_eq!(decimal_btc_to_msat("."), None);
    }

    #[test]
    fn serializes_a_migration_into_the_daemon_spec_shape() {
        let spec = form_to_spec(&migration_form());
        assert_eq!(spec["component_type"], "manual_bridge");
        assert_eq!(spec["conservation_mode"], "quantity");
        let legs = spec["legs"].as_array().unwrap();
        assert_eq!(legs.len(), 3);
        assert_eq!(legs[0]["role"], "source");
        assert_eq!(legs[0]["amount_btc"], "1.0");
        assert_eq!(legs[0]["transaction"], "tx-out");
        assert_eq!(legs[2]["amount_btc"], "0.01");
        assert!(spec.get("allocations").is_none());
    }

    #[test]
    fn balanced_passes_and_unbalanced_blocks_activation() {
        let p = preview_form(&migration_form());
        assert!(p.structural.is_empty(), "{:?}", p.structural);
        assert!(p.activation.is_empty(), "{:?}", p.activation);
        let mut f = migration_form();
        f.legs[1].amount_btc = "0.98".into();
        let p = preview_form(&f);
        assert!(p.structural.is_empty());
        assert!(codes(&p.activation).contains(&"quantityUnbalanced"));
    }

    #[test]
    fn transactionless_owned_leg_needs_a_time() {
        let mut f = initial_form();
        let mut s = new_leg("source");
        s.amount_btc = "1.0".into();
        s.transaction_ref = "tx-out".into();
        let mut r = new_leg("retained");
        r.amount_btc = "1.0".into();
        r.location_kind = "untracked".into();
        r.untracked_wallet = "Old migration wallet".into();
        f.legs = vec![s, r];
        assert!(codes(&preview_form(&f).structural).contains(&"transactionlessTimeRequired"));
        f.legs[1].occurred_at = "2024-01-15T12:00".into();
        let spec = form_to_spec(&f);
        assert_eq!(spec["legs"][1]["untracked_wallet"], "Old migration wallet");
        let at = spec["legs"][1]["occurred_at"].as_str().unwrap();
        assert!(at.ends_with('Z'));
        assert_eq!(parse_instant(at), parse_instant("2024-01-15T12:00"));
        assert!(!codes(&preview_form(&f).structural).contains(&"transactionlessTimeRequired"));
    }

    #[test]
    fn requires_and_satisfies_n_to_m_allocations() {
        let mut f = initial_form();
        let mut s1 = new_leg("source");
        s1.amount_btc = "0.5".into();
        s1.transaction_ref = "tx-a".into();
        let mut s2 = new_leg("source");
        s2.amount_btc = "0.5".into();
        s2.transaction_ref = "tx-b".into();
        let mut d = new_leg("destination");
        d.amount_btc = "0.99".into();
        d.transaction_ref = "tx-c".into();
        let mut fee = new_leg("fee");
        fee.amount_btc = "0.01".into();
        fee.transaction_ref = "tx-a".into();
        let (k1, k2, kd, kf) = (
            s1.key.clone(),
            s2.key.clone(),
            d.key.clone(),
            fee.key.clone(),
        );
        f.legs = vec![s1, s2, d, fee];
        assert!(codes(&preview_form(&f).activation).contains(&"allocationsRequired"));
        let alloc = |s: &str, k: &str, a: &str| Allocation {
            source_key: s.into(),
            sink_key: k.into(),
            amount_btc: a.into(),
            ..new_allocation()
        };
        f.allocations = vec![
            alloc(&k1, &kd, "0.5"),
            alloc(&k2, &kd, "0.49"),
            alloc(&k2, &kf, "0.01"),
        ];
        let spec = form_to_spec(&f);
        assert_eq!(
            spec["allocations"][0],
            json!({"source_ordinal": 0, "sink_ordinal": 2, "source_amount_msat": "50000000000", "sink_amount_msat": "50000000000"})
        );
        let p = preview_form(&f);
        assert!(
            p.structural.is_empty() && p.activation.is_empty(),
            "{:?} {:?}",
            p.structural,
            p.activation
        );
    }

    fn conversion(reviewed: bool) -> Form {
        let mut f = initial_form();
        f.component_type = "swap".into();
        f.conservation_mode = "conversion".into();
        if reviewed {
            f.conversion_policy = "taxable_swap".into();
            f.conversion_reviewed = true;
        }
        let mut s = new_leg("source");
        s.amount_btc = "1.0".into();
        s.transaction_ref = "tx-a".into();
        s.valuation_unit = "eur_cents".into();
        s.valuation_amount = "5000000".into();
        let mut d = new_leg("destination");
        d.amount_btc = "1.0".into();
        d.transaction_ref = "tx-b".into();
        d.asset = if reviewed {
            "LBTC".into()
        } else {
            "BTC".into()
        };
        d.valuation_unit = "eur_cents".into();
        d.valuation_amount = "5000000".into();
        f.legs = vec![s, d];
        f
    }

    #[test]
    fn serializes_a_reviewed_conversion() {
        let f = conversion(true);
        let spec = form_to_spec(&f);
        assert_eq!(spec["conversion_policy"], "taxable_swap");
        assert_eq!(spec["conversion_reviewed"], true);
        assert_eq!(spec["legs"][0]["valuation_amount"], "5000000");
        assert_eq!(spec["legs"][1]["asset"], "LBTC");
        let p = preview_form(&f);
        assert!(
            p.structural.is_empty() && p.activation.is_empty(),
            "{:?} {:?}",
            p.structural,
            p.activation
        );
        assert!(
            codes(&preview_form(&conversion(false)).activation)
                .contains(&"conversionReviewRequired")
        );
    }

    #[test]
    fn activates_a_suspense_component_with_an_allocation() {
        let mut f = initial_form();
        let mut s = new_leg("source");
        s.amount_btc = "1.0".into();
        s.transaction_ref = "tx-a".into();
        let mut d = new_leg("destination");
        d.amount_btc = "0.6".into();
        d.transaction_ref = "tx-b".into();
        let mut x = new_leg("suspense");
        x.amount_btc = "0.4".into();
        x.occurred_at = "2024-02-01T10:00".into();
        let (ks, kd, kx) = (s.key.clone(), d.key.clone(), x.key.clone());
        f.legs = vec![s, d, x];
        f.allocations = vec![
            Allocation {
                source_key: ks.clone(),
                sink_key: kd,
                amount_btc: "0.6".into(),
                ..new_allocation()
            },
            Allocation {
                source_key: ks,
                sink_key: kx,
                amount_btc: "0.4".into(),
                ..new_allocation()
            },
        ];
        let spec = form_to_spec(&f);
        assert_eq!(spec["legs"][2]["role"], "suspense");
        assert!(
            spec["legs"][2].get("transaction").is_none()
                && spec["legs"][2].get("untracked_wallet").is_none()
        );
        assert!(spec["legs"][2].get("occurred_at").is_some());
        let p = preview_form(&f);
        assert!(
            p.structural.is_empty() && p.activation.is_empty(),
            "{:?} {:?}",
            p.structural,
            p.activation
        );
    }

    #[test]
    fn round_trips_an_existing_component_for_revise() {
        let c = json!({"component_type":"manual_bridge","conservation_mode":"quantity","evidence_kind":"manual_migration_review","evidence_grade":"reviewed","conversion_policy":null,"conversion_reviewed":false,"notes":"migration","legs":[
            {"id":"L1","role":"source","asset":"BTC","amount_msat":100000000000u64,"transaction_id":"tx-out","anchor_transaction_id":"tx-out","wallet_id":"w1","occurred_at":null,"rail":"bitcoin","exposure":"bitcoin","conservation_unit":"msat"},
            {"id":"L2","role":"destination","asset":"BTC","amount_msat":99000000000u64,"transaction_id":"tx-in","wallet_id":"w2","occurred_at":null},
            {"id":"L3","role":"fee","asset":"BTC","amount_msat":1000000000u64,"transaction_id":"tx-out","wallet_id":"w1","occurred_at":null}],"allocations":[]});
        let f = component_to_form(&c).unwrap();
        assert!(f.legs[0].origin_mode);
        assert_eq!(
            f.legs[0].origin.as_ref().unwrap().transaction_id.as_deref(),
            Some("tx-out")
        );
        assert_eq!(f.legs[0].amount_btc, "1");
        assert_eq!(f.legs[1].amount_btc, "0.99");
        let spec = form_to_spec(&f);
        let l0 = &spec["legs"][0];
        assert_eq!(
            (
                l0["id"].as_str(),
                l0["transaction_id"].as_str(),
                l0["anchor_transaction_id"].as_str(),
                l0["wallet_id"].as_str(),
                l0["amount_btc"].as_str(),
                l0["rail"].as_str()
            ),
            (
                Some("L1"),
                Some("tx-out"),
                Some("tx-out"),
                Some("w1"),
                Some("1"),
                Some("bitcoin")
            )
        );
        assert!(l0.get("transaction").is_none() && l0.get("wallet").is_none());
        let p = preview_form(&f);
        assert!(
            p.structural.is_empty() && p.activation.is_empty(),
            "{:?} {:?}",
            p.structural,
            p.activation
        );
    }

    #[test]
    fn formats_lossless_btc_inputs_and_times() {
        assert_eq!(msat_to_btc_input(&json!(100_000_000_000u64)), "1");
        assert_eq!(msat_to_btc_input(&json!(99_000_000_000u64)), "0.99");
        assert_eq!(msat_to_btc_input(&json!(1)), "0.00000000001");
        assert_eq!(msat_to_btc_input(&json!(0)), "0");
        assert_eq!(msat_to_btc_input(&json!("250000000000")), "2.5");
        let c = occurred_at_to_rfc3339("2024-01-15T12:00");
        assert!(c.ends_with('Z'));
        assert_eq!(parse_instant(&c), parse_instant("2024-01-15T12:00"));
        assert_eq!(occurred_at_to_rfc3339(""), "");
        assert_eq!(
            iso_string(parse_instant("2024-10-27T01:30:00.123Z").unwrap()),
            "2024-10-27T01:30:00.123Z"
        );
        let local = rfc3339_to_datetime_local(Some("2024-10-27T01:30:00Z"));
        assert_eq!(parse_instant(&local), parse_instant("2024-10-27T01:30:00Z"));
    }

    fn persisted_conversion() -> Value {
        json!({"component_type":"swap","conservation_mode":"conversion","evidence_kind":"reviewed_export","evidence_grade":"reviewed","conversion_policy":"taxable_swap","conversion_reviewed":true,"notes":"reviewed route","legs":[
            {"id":"source","role":"source","asset":"BTC","amount_msat":"100000000001","transaction_id":"out","anchor_transaction_id":"out","wallet_id":"bitcoin","occurred_at":"2024-10-27T01:30:00.123456Z","rail":"bitcoin","chain":"bitcoin","network":"testnet","exposure":"bitcoin","conservation_unit":"msat","valuation_unit":"eur_cents","valuation_amount":"5000000","notes":"source evidence"},
            {"id":"target","role":"destination","asset":"LBTC","amount_msat":"99000000001","transaction_id":null,"wallet_id":"liquid","occurred_at":"2024-10-27T01:30:00.123456Z","rail":"liquid","chain":"liquid","network":"testnet","exposure":"bitcoin","conservation_unit":"msat","valuation_unit":"eur_cents","valuation_amount":"5000000","notes":"target evidence"}],
            "allocations":[{"source_leg_id":"source","sink_leg_id":"target","source_amount_msat":"100000000001","sink_amount_msat":"99000000001"}]})
    }

    #[test]
    fn revision_preserves_conversion_terms_and_timestamps() {
        let original = persisted_conversion();
        let mut f = component_to_form(&original).unwrap();
        let spec = form_to_spec(&f);
        assert_eq!(
            spec["allocations"][0],
            json!({"source_ordinal": 0, "sink_ordinal": 1, "source_amount_msat": "100000000001", "sink_amount_msat": "99000000001"})
        );
        assert_eq!(spec["legs"][0]["amount_btc"], "1.00000000001");
        assert_eq!(spec["legs"][1]["amount_btc"], "0.99000000001");
        assert_eq!(spec["legs"][1]["network"], "testnet");
        let p = preview_form(&f);
        assert!(
            p.structural.is_empty() && p.activation.is_empty(),
            "{:?} {:?}",
            p.structural,
            p.activation
        );
        // The exact original timestamp survives until edited.
        assert_eq!(
            spec["legs"][0]["occurred_at"],
            "2024-10-27T01:30:00.123456Z"
        );
        f.legs[1].occurred_at = "2024-11-01T12:00:00".into();
        assert_eq!(
            form_to_spec(&f)["legs"][1]["occurred_at"]
                .as_str()
                .map(str::to_owned),
            Some(iso_string(parse_instant("2024-11-01T12:00:00").unwrap()))
        );
    }

    #[test]
    fn mode_switch_surfaces_unequal_allocations() {
        let mut f = component_to_form(&persisted_conversion()).unwrap();
        f.conservation_mode = "quantity".into();
        f.notes.clear();
        f.conversion_policy.clear();
        f.conversion_reviewed = false;
        let spec = form_to_spec(&f);
        assert_eq!(
            (
                spec["notes"].clone(),
                spec["conversion_policy"].clone(),
                spec["conversion_reviewed"].clone()
            ),
            (json!(""), Value::Null, json!(false))
        );
        assert!(codes(&preview_form(&f).activation).contains(&"allocationQuantityMismatch"));
    }

    #[test]
    fn refuses_unsupported_persisted_shapes() {
        for shape in ["type", "role", "unit", "edge", "duplicate"] {
            let mut c = persisted_conversion();
            match shape {
                "type" => c["component_type"] = json!("future_native_type"),
                "role" => c["legs"][0]["role"] = json!("future_role"),
                "unit" => c["legs"][0]["conservation_unit"] = json!("asset_quantum"),
                "edge" => c["allocations"][0]["sink_leg_id"] = json!("absent"),
                _ => c["legs"][1]["id"] = json!("source"),
            }
            assert!(component_to_form(&c).is_err(), "{shape}");
        }
    }

    #[test]
    fn incomplete_allocation_stays_visible() {
        let mut f = migration_form();
        f.allocations.push(new_allocation());
        assert_eq!(form_to_spec(&f)["allocations"].as_array().unwrap().len(), 1);
        assert!(codes(&preview_form(&f).structural).contains(&"allocationSourceInvalid"));
    }

    #[test]
    fn plan_confirmation_is_bound() {
        let doc = form_to_document(&migration_form());
        let c = Commitment {
            document: doc.clone(),
            scope: "book-A/session-1".into(),
            input_version: 0,
        };
        assert!(can_confirm(Some(&c), &doc, &c.scope));
        assert!(!can_confirm(None, &doc, &c.scope));
        assert!(!can_confirm(Some(&c), &format!("{doc} "), &c.scope));
        assert!(!can_confirm(Some(&c), &doc, "book-B/session-1"));
        assert!(!can_confirm(
            Some(&Commitment {
                input_version: -1,
                ..c.clone()
            }),
            &doc,
            &c.scope
        ));
    }

    #[test]
    fn pristine_form_is_quiet() {
        let f = initial_form();
        assert!(!has_input(&f));
        let mut g = f.clone();
        g.legs[0].amount_btc = "1".into();
        assert!(has_input(&g));
    }

    #[test]
    fn apply_is_pinned_to_the_plan_scope() {
        let args = json!({"components": [], "activate": false, "workspace": "stale"});
        assert_eq!(
            bind_plan_scope(&args, &json!({"workspace_id": "w", "profile_id": "p"})).unwrap()["workspace"],
            "w"
        );
        assert!(bind_plan_scope(&args, &json!({"workspace_id": "w"})).is_none());
    }
}
