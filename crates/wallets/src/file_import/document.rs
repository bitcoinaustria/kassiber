//! Reviewed OCR observations, gated local providers and managed evidence.
//! Product registration remains a separate acceptance boundary.
use super::*;
use kassiber_envelope::{get, py_str, set, truthy};
use kassiber_pyjson::{Key, Options, PyStr};

mod coordinator;
pub(super) mod files;
mod pdf;
mod provider;
mod rows;
pub use coordinator::{import_draft, reviewed_records, ImportDraft};
pub use provider::{local_reader, DocumentConsent, DocumentPreview, Providers};

/// `_document_import_stage_payload`'s source checks: the canonical local
/// file, its size and its media type. Callers treat any failure as an
/// unavailable document.
pub fn staged_source(raw: &str) -> Result<(std::path::PathBuf, u64, &'static str), AppError> {
    let path = files::source_path(raw)?.canonicalize().map_err(files::io)?;
    let size = std::fs::metadata(&path).map_err(files::io)?.len();
    Ok((path.clone(), size, files::mime(&path)))
}

/// `core_document_import._mime_type(path)`: `mimetypes.guess_type` over
/// the chat's and the importer's file types.
pub fn media_type(path: &std::path::Path) -> &'static str {
    files::mime(path)
}

fn s(v: impl AsRef<str>) -> Py {
    kassiber_envelope::s(v.as_ref())
}

fn field<'a>(v: &'a Py, key: &str) -> &'a Py {
    get(v, key).unwrap_or(&Py::Null)
}
fn missing(v: &Py) -> bool {
    *v == Py::Null || *v == s("")
}
fn points(v: &Py) -> Vec<u32> {
    match v {
        Py::Str(v) => v.code_points().collect(),
        _ => py_str(v).chars().map(u32::from).collect(),
    }
}
fn cleaned(v: &Py) -> Py {
    if *v == Py::Null {
        return Py::Null;
    }
    let v = points(v);
    let first = v
        .iter()
        .position(|c| !kassiber_pytext::is_space(*c))
        .unwrap_or(v.len());
    let last = v
        .iter()
        .rposition(|c| !kassiber_pytext::is_space(*c))
        .map_or(first, |i| i + 1);
    if first == last {
        Py::Null
    } else {
        Py::Str(PyStr::from_code_points(v[first..last].iter().copied()).unwrap())
    }
}
fn truncated(v: Py, limit: usize) -> Py {
    if v == Py::Null {
        v
    } else {
        Py::Str(PyStr::from_code_points(points(&v).into_iter().take(limit)).unwrap())
    }
}
fn rendered(v: &Py) -> String {
    points(v)
        .into_iter()
        .map(|c| char::from_u32(c).unwrap_or(char::REPLACEMENT_CHARACTER))
        .collect()
}
fn upper(v: Py) -> Py {
    if v == Py::Null {
        v
    } else {
        let mut result = vec![];
        for c in points(&v) {
            if let Some(c) = char::from_u32(c) {
                result.extend(
                    kassiber_pytext::upper(&c.to_string())
                        .chars()
                        .map(u32::from),
                );
            } else {
                result.push(c);
            }
        }
        Py::Str(PyStr::from_code_points(result).unwrap())
    }
}
fn canonical(v: &Py) -> Result<PyStr, AppError> {
    let encoded = kassiber_pyjson::dumps(v, &Options::new().sort_keys(true).separators(",", ":"))
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let bytes = encoded.as_bytes();
    let mut points = vec![];
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\\' && bytes.get(at + 1) == Some(&b'u') {
            let mut point = u32::from_str_radix(&encoded[at + 2..at + 6], 16).unwrap();
            if point > 126 {
                at += 6;
                if (0xd800..=0xdbff).contains(&point) && encoded[at..].starts_with("\\u") {
                    let low = u32::from_str_radix(&encoded[at + 2..at + 6], 16).unwrap();
                    if (0xdc00..=0xdfff).contains(&low) {
                        point = 0x10000 + ((point - 0xd800) << 10) + low - 0xdc00;
                        at += 6;
                    }
                }
                points.push(point);
                continue;
            }
        }
        points.push(u32::from(bytes[at]));
        if bytes[at] == b'\\' && at + 1 < bytes.len() {
            at += 1;
            points.push(u32::from(bytes[at]));
        }
        at += 1;
    }
    Ok(PyStr::from_code_points(points).unwrap())
}
fn decimal(v: &Py) -> Option<Decimal> {
    if missing(v) {
        return None;
    }
    let text = rendered(&cleaned(v));
    if text.chars().count() > 512 {
        return None;
    }
    let mut text = text.replace(['\u{a0}', ' '], "");
    if text.contains(',') && text.contains('.') {
        text = if text.rfind(',') > text.rfind('.') {
            text.replace('.', "").replace(',', ".")
        } else {
            text.replace(',', "")
        };
    } else if text.contains(',') {
        if text.matches(',').count() != 1 {
            return None;
        }
        let (whole, fractional) = text.split_once(',').unwrap();
        if fractional.chars().count() == 3
            && !["", "0"].contains(&whole.trim_start_matches(['+', '-']))
        {
            return None;
        }
        text = text.replace(',', ".");
    }
    text.parse().ok()
}
fn plain(number: &Decimal) -> Option<String> {
    if number.is_zero() {
        return Some("0".into());
    }
    let digits = number.coefficient().to_string().len() as i128;
    let exp = i128::from(number.exponent());
    let sign = i128::from(number.is_sign_negative());
    let len = if exp >= 0 {
        digits + exp + sign
    } else if digits + exp > 0 {
        digits + 1 + sign
    } else {
        2 - exp + sign
    };
    if len > 512 {
        return None;
    }
    let mut text = number.to_fixed_string().ok()?;
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').into();
    }
    Some(text)
}
fn decimal_text(v: &Py) -> Py {
    decimal(v).and_then(|n| plain(&n)).map_or(Py::Null, s)
}
fn confidence(v: &Py) -> Decimal {
    let zero: Decimal = "0".parse().unwrap();
    let one: Decimal = "1".parse().unwrap();
    let n = decimal(v).unwrap_or_else(|| zero.clone());
    if n < zero {
        zero
    } else if n > one {
        one
    } else {
        n
    }
}
fn float(n: &Decimal) -> f64 {
    n.to_canonical().parse().unwrap()
}
fn exact_msat(n: &Decimal) -> bool {
    if n.is_negative() || n > &"92233720.36854775807".parse::<Decimal>().unwrap() {
        return false;
    }
    if n.is_zero() {
        return true;
    }
    if n < &"0.00000000001".parse::<Decimal>().unwrap() {
        return false;
    }
    let shift = n.exponent() + 11;
    shift >= 0
        || n.coefficient()
            .to_string()
            .chars()
            .rev()
            .take((-shift) as usize)
            .all(|c| c == '0')
}
fn storable_fiat(n: &Decimal) -> bool {
    n.is_positive()
        && n <= &"1.7976931348623157E+308".parse::<Decimal>().unwrap()
        && float(n).is_finite()
        && float(n) > 0.0
}
pub fn threshold(v: &Py) -> Result<Decimal, AppError> {
    if missing(v) {
        return Ok("0.78".parse().unwrap());
    }
    decimal(v)
        .filter(|n| !n.is_negative() && n <= &"1".parse::<Decimal>().unwrap())
        .ok_or_else(|| {
            AppError::new(
                "validation",
                "confidence_threshold must be a number between 0 and 1",
            )
        })
}
fn direction(v: &Py) -> Py {
    match kassiber_pytext::lower(&rendered(&cleaned(v))).as_str() {
        "in" | "incoming" | "inbound" | "receive" | "received" | "deposit" | "credit" | "buy" => {
            s("inbound")
        }
        "out" | "outgoing" | "outbound" | "send" | "sent" | "withdrawal" | "withdraw" | "debit"
        | "sell" => s("outbound"),
        _ => Py::Null,
    }
}
const CONFIDENCE_FIELDS: &[&str] = &[
    "occurred_at",
    "direction",
    "asset",
    "amount_btc",
    "fee_btc",
    "fiat_currency",
    "fiat_value",
    "fiat_rate",
    "counterparty",
    "description",
];
const STRUCTURAL: &[&str] = &[
    "missing_date",
    "invalid_date",
    "missing_direction",
    "missing_amount",
    "amount_not_representable",
    "unsupported_asset",
    "non_positive_amount",
    "invalid_fee",
    "negative_fee",
    "fee_not_representable",
    "invalid_fiat_value",
    "non_positive_fiat_value",
    "fiat_value_out_of_range",
    "invalid_fiat_rate",
    "non_positive_fiat_rate",
    "fiat_rate_out_of_range",
    "missing_fiat_currency",
    "fiat_currency_mismatch",
    "invalid_source_page",
];
fn cell_confidences(v: &Py) -> Py {
    let mut result = dict::<&str>([]);
    if let Py::Dict(fields) = v {
        for (key, v) in fields {
            if let Key::Str(key) = key {
                if let Some(key) = key.as_str().filter(|k| CONFIDENCE_FIELDS.contains(k)) {
                    set(&mut result, key, Py::Float(float(&confidence(v))));
                }
            }
        }
    }
    result
}
fn source_region(v: &Py, allowed: Option<&[i64]>) -> Py {
    if !matches!(v, Py::Dict(_)) {
        return Py::Null;
    }
    let Some(page) = decimal(field(v, "page")) else {
        return Py::Null;
    };
    let Some(page) = plain(&page) else {
        return Py::Null;
    };
    let Ok(page) = page.parse::<kassiber_pyjson::BigInt>() else {
        return Py::Null;
    };
    if page < 1.into() || allowed.is_some_and(|v| !v.iter().any(|n| page == (*n).into())) {
        return Py::Null;
    }
    let mut result = dict([("page", Py::Int(page))]);
    for key in ["x", "y", "width", "height"] {
        if let Some(n) = decimal(field(v, key)) {
            let absolute = n.abs(&Context::RP2).ok();
            if absolute.is_some_and(|v| v <= "1000000".parse::<Decimal>().unwrap()) {
                set(&mut result, key, Py::Float(float(&n)));
            }
        }
    }
    let unit = cleaned(field(v, "unit"));
    set(
        &mut result,
        "unit",
        truncated(
            if unit == Py::Null {
                s("relative")
            } else {
                unit
            },
            32,
        ),
    );
    result
}
pub fn confidence_flags(row: &Py, threshold: &Decimal) -> Vec<String> {
    let mut result = vec![];
    if confidence(field(row, "confidence")) < *threshold {
        result.push("low_row_confidence".into());
    }
    let draft = field(row, "record");
    if !matches!(draft, Py::Dict(_)) {
        result.push("missing_review_record".into());
        return result;
    }
    let cells = cell_confidences(field(row, "cell_confidences"));
    for key in CONFIDENCE_FIELDS {
        if missing(field(draft, key))
            || (*key == "fee_btc" && truthy(field(draft, "fee_defaulted")))
        {
            continue;
        }
        match get(&cells, key) {
            None => result.push(format!("missing_{key}_confidence")),
            Some(v) if decimal(v).unwrap() < *threshold => {
                result.push(format!("low_{key}_confidence"))
            }
            _ => (),
        }
    }
    result
}
pub fn reviewed_record(row: &Py, source_hash: &str, currency: &str) -> Option<Py> {
    let id = rendered(field(row, "id"));
    let pattern = format!(
        r"^docrow-{}-[0-9a-f]{{16}}-{}{{3}}$",
        regex::escape(&source_hash.chars().take(16).collect::<String>()),
        kassiber_pytext::REGEX_DIGIT
    );
    if !regex::Regex::new(&pattern).unwrap().is_match(&id) {
        return None;
    }
    if let Py::List(flags) = field(row, "flags") {
        if flags
            .iter()
            .any(|v| STRUCTURAL.contains(&rendered(v).as_str()))
        {
            return None;
        }
    }
    let draft = field(row, "record");
    if !matches!(draft, Py::Dict(_)) {
        return None;
    }
    let occurred = crate::time::timestamp(&rendered(field(draft, "occurred_at"))).ok()?;
    let side = direction(field(draft, "direction"));
    let asset = upper(cleaned(field(draft, "asset")));
    let amount = decimal(field(draft, "amount_btc"))?;
    let fee = if missing(field(draft, "fee_btc")) {
        "0".parse().unwrap()
    } else {
        decimal(field(draft, "fee_btc"))?
    };
    if side == Py::Null
        || ![s("BTC"), s("LBTC")].contains(&asset)
        || !amount.is_positive()
        || !exact_msat(&amount)
        || !exact_msat(&fee)
    {
        return None;
    }
    let fiat = truncated(upper(cleaned(field(draft, "fiat_currency"))), 16);
    if fiat != Py::Null
        && !currency.is_empty()
        && fiat != s(kassiber_pytext::upper(kassiber_pytext::strip(currency)))
    {
        return None;
    }
    for key in ["fiat_value", "fiat_rate"] {
        let raw = field(draft, key);
        if !missing(raw)
            && (fiat == Py::Null || !storable_fiat(&decimal(raw)?) || decimal_text(raw) == Py::Null)
        {
            return None;
        }
    }
    Some(dict([
        ("id", field(row, "id").clone()),
        ("occurred_at", s(occurred)),
        ("direction", side),
        ("asset", asset),
        ("amount", s(plain(&amount)?)),
        ("fee", s(plain(&fee)?)),
        ("fiat_currency", fiat),
        ("fiat_value", decimal_text(field(draft, "fiat_value"))),
        ("fiat_rate", decimal_text(field(draft, "fiat_rate"))),
        (
            "counterparty",
            truncated(cleaned(field(draft, "counterparty")), 500),
        ),
        (
            "description",
            truncated(cleaned(field(draft, "description")), 4000),
        ),
        (
            "raw_json",
            dict([
                ("source", s("document_import")),
                ("row_id", field(row, "id").clone()),
                (
                    "model_confidence",
                    Py::Float(float(&confidence(field(row, "confidence")))),
                ),
                (
                    "cell_confidences",
                    cell_confidences(field(row, "cell_confidences")),
                ),
                (
                    "fee_defaulted",
                    Py::Bool(truthy(field(draft, "fee_defaulted"))),
                ),
                (
                    "source_region",
                    source_region(field(row, "source_region"), None),
                ),
                (
                    "evidence_text",
                    truncated(cleaned(field(row, "evidence_text")), 4000),
                ),
            ]),
        ),
    ]))
}

pub fn draft_rows(
    payload: &Py,
    source_hash: &str,
    threshold: &Decimal,
    currency: &str,
    allowed_pages: Option<&[i64]>,
) -> Result<Vec<Py>, AppError> {
    rows::draft_rows(payload, source_hash, threshold, currency, allowed_pages)
}

#[cfg(test)]
mod tests;
