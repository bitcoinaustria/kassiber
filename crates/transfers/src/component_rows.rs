//! Python's authored component contract: normalization, immutable revisions,
//! activation and materialization. Interpretation stays in the custody crate.
use super::component_store as store;
use super::*;
use kassiber_custody::{
    codec,
    components::{remediation, review},
    snapshot::{Component, Record, Snapshot},
};
use kassiber_envelope::{set, truthy};
use kassiber_pyjson::{BigInt, Key, PyStr};
use kassiber_store::rusqlite::params;
use sql::{bind, error};
use std::collections::BTreeSet;

pub(super) const TYPES: &[&str] = &["manual_bridge", "swap"];
pub(super) const SOURCES: &[&str] = &["user", "cli", "gui", "ai_tool", "migration"];
const MODES: &[&str] = &["conversion", "quantity"];
const ROLES: &[&str] = &[
    "destination",
    "external",
    "fee",
    "retained",
    "source",
    "suspense",
];
const LEG_FIELDS: &[&str] = &[
    "id",
    "component_id",
    "workspace_id",
    "profile_id",
    "ordinal",
    "role",
    "rail",
    "chain",
    "network",
    "asset",
    "exposure",
    "conservation_unit",
    "amount_msat",
    "valuation_unit",
    "valuation_amount",
    "occurred_at",
    "transaction_id",
    "anchor_transaction_id",
    "wallet_id",
    "location_ref",
    "notes",
    "created_at",
];
const ALLOCATION_FIELDS: &[&str] = &[
    "id",
    "component_id",
    "workspace_id",
    "profile_id",
    "ordinal",
    "source_leg_id",
    "sink_leg_id",
    "source_amount_msat",
    "sink_amount_msat",
    "created_at",
];
const TERM_FIELDS: &[&str] = &[
    "id",
    "component_id",
    "workspace_id",
    "profile_id",
    "ordinal",
    "source_leg_id",
    "target_leg_id",
    "term_kind",
    "legacy_source_id",
    "source_row_hash",
    "review_kind",
    "tax_policy",
    "reviewed_source_amount_msat",
    "swap_fee_msat",
    "swap_fee_kind",
    "confidence_at_review",
    "review_source",
    "review_notes",
    "payout_asset",
    "payout_amount_msat",
    "payout_occurred_at",
    "payout_fiat_value_exact",
    "payout_external_id",
    "counterparty",
    "created_at",
];

pub(super) fn s(v: impl AsRef<str>) -> Value {
    kassiber_envelope::s(v.as_ref())
}
pub(super) fn int(n: impl Into<BigInt>) -> Value {
    Value::Int(n.into())
}
/// `custody_components._error`: details are always a (possibly empty) dict.
pub(super) fn fail(
    code: &str,
    message: impl Into<String>,
    details: Vec<(&str, Value)>,
) -> AppError {
    AppError::new(code, message).with_details(dict(details))
}
fn invalid(message: impl Into<String>, details: Vec<(&str, Value)>) -> AppError {
    fail("custody_component_validation", message, details)
}
/// `issues_remediation(issues)`: the first remediation an issue carries.
pub(super) fn issues_remediation(issues: &Value) -> Option<Value> {
    let Value::List(items) = issues else {
        return None;
    };
    items
        .iter()
        .find_map(|issue| match get(issue, "remediation") {
            Some(remediation @ Value::Dict(_)) => Some(remediation.clone()),
            _ => None,
        })
}
/// `_activation_error(component)`: the projected validation, and the
/// remediation its issues carry.
pub(super) fn activation_error(id: &str, validation: Value) -> AppError {
    let mut details = vec![("component_id", s(id))];
    let remediation =
        issues_remediation(&get(&validation, "issues").cloned().unwrap_or(Value::Null));
    details.push(("validation", validation));
    if let Some(remediation) = remediation {
        details.push(("remediation", remediation));
    }
    fail(
        "custody_component_incomplete",
        "custody component is incomplete or does not conserve",
        details,
    )
}
pub(super) fn internal(message: impl Into<String>) -> AppError {
    AppError::new("internal_error", message)
}
/// `int(text, 10)` failures as CPython words them.
pub(super) fn int_error(error: kassiber_pytext::NumberError, text: &str) -> AppError {
    match error {
        kassiber_pytext::NumberError::TooManyDigits(n) => internal(format!(
            "Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"
        )),
        kassiber_pytext::NumberError::Invalid => internal(format!(
            "invalid literal for int() with base 10: {}",
            kassiber_pytext::repr_str(text)
        )),
    }
}
fn token(text: &str) -> bool {
    let mut chars = text.bytes();
    chars.next().is_some_and(|b| b.is_ascii_lowercase())
        && chars.all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.:-".contains(&b))
}
fn stripped(text: &PyStr) -> PyStr {
    kassiber_pytext::strip_py(text)
}
fn is_empty(text: &PyStr) -> bool {
    text.code_points().next().is_none()
}
fn token_text(text: &PyStr) -> bool {
    text.as_str().is_some_and(token)
}
pub(super) fn required_text(v: &Value, field: &str, check: bool) -> Result<Value> {
    let Value::Str(raw) = v else {
        return Err(invalid(
            format!("{field} is required"),
            vec![("field", s(field))],
        ));
    };
    let text = stripped(raw);
    if is_empty(&text) {
        return Err(invalid(
            format!("{field} is required"),
            vec![("field", s(field))],
        ));
    }
    if check && !token_text(&text) {
        return Err(invalid(
            format!("{field} must be a lowercase stable token"),
            vec![("field", s(field)), ("value", Value::Str(text))],
        ));
    }
    Ok(Value::Str(text))
}
pub(super) fn optional_text(v: &Value, field: &str, check: bool) -> Result<Value> {
    let raw = match v {
        Value::Null => return Ok(Value::Null),
        Value::Str(raw) => raw,
        _ => {
            return Err(invalid(
                format!("{field} must be text"),
                vec![("field", s(field))],
            ))
        }
    };
    let text = stripped(raw);
    if is_empty(&text) {
        return Ok(Value::Null);
    }
    if check && !token_text(&text) {
        return Err(invalid(
            format!("{field} must be a lowercase stable token"),
            vec![("field", s(field)), ("value", Value::Str(text))],
        ));
    }
    Ok(Value::Str(text))
}
fn max() -> BigInt {
    BigInt::from(i64::MAX)
}
fn digits(text: &PyStr, signed: bool) -> Option<&str> {
    let text = text.as_str()?;
    let body = if signed {
        text.strip_prefix('-').unwrap_or(text)
    } else {
        text
    };
    (!body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())).then_some(text)
}
pub(super) fn nonnegative(v: &Value, field: &str) -> Result<Value> {
    let range = || {
        invalid(
            format!("{field} must fit SQLite's non-negative integer range"),
            vec![("field", s(field)), ("value", v.clone())],
        )
    };
    let parsed = match v {
        Value::Int(n) => n.clone(),
        Value::Str(text) if digits(text, false).is_some() => {
            let raw = digits(text, false).unwrap();
            let trimmed = raw.trim_start_matches('0');
            let normalized = if trimmed.is_empty() { "0" } else { trimmed };
            if normalized.len() > 19 {
                return Err(range());
            }
            normalized.parse().map_err(|_| range())?
        }
        _ => {
            return Err(invalid(
                format!("{field} must be an exact non-negative integer"),
                vec![("field", s(field)), ("value", v.clone())],
            ))
        }
    };
    if parsed < BigInt::from(0) || parsed > max() {
        return Err(range());
    }
    Ok(Value::Int(parsed))
}
fn signed(v: &Value, field: &str) -> Result<Value> {
    let parsed = match v {
        Value::Null => return Ok(Value::Null),
        Value::Int(n) => n.clone(),
        Value::Str(text) if digits(text, true).is_some() => {
            let raw = digits(text, true).unwrap();
            kassiber_pytext::parse_int(raw).map_err(|e| int_error(e, raw))?
        }
        _ => {
            return Err(invalid(
                format!("{field} must be an exact integer"),
                vec![("field", s(field)), ("value", v.clone())],
            ))
        }
    };
    if parsed < -max() || parsed > max() {
        return Err(invalid(
            format!("{field} must fit SQLite's signed integer range"),
            vec![("field", s(field)), ("value", v.clone())],
        ));
    }
    Ok(Value::Int(parsed))
}
fn compact(v: &Value, allow_nan: bool) -> std::result::Result<String, kassiber_pyjson::Error> {
    kassiber_pyjson::dumps(
        v,
        &kassiber_pyjson::Options {
            sort_keys: true,
            allow_nan,
            ..kassiber_pyjson::Options::default().compact()
        },
    )
}
/// Bare `NaN`/`Infinity` outside strings; `json.loads(parse_constant=...)` refuses them.
fn has_constant(text: &str) -> bool {
    let mut quoted = false;
    let mut escaped = false;
    for c in text.chars() {
        if quoted {
            match (escaped, c) {
                (true, _) => escaped = false,
                (false, '\\') => escaped = true,
                (false, '"') => quoted = false,
                _ => {}
            }
        } else if c == '"' {
            quoted = true;
        } else if c == 'N' || c == 'I' {
            return true;
        }
    }
    false
}
pub(super) fn json_object(v: &Value, field: &str) -> Result<Value> {
    let parsed;
    let value = match v {
        Value::Null => return Ok(dict::<&str>([])),
        Value::Str(text) if is_empty(text) => return Ok(dict::<&str>([])),
        Value::Str(text) => {
            let raw: String = text.code_points().filter_map(char::from_u32).collect();
            let valid = || {
                invalid(
                    format!("{field} must contain valid JSON"),
                    vec![("field", s(field))],
                )
            };
            parsed = match kassiber_pytext::loads(&raw) {
                Ok(_) if has_constant(&raw) => return Err(valid()),
                Ok(value) => value,
                Err(e) if matches!(e.kind, kassiber_pytext::JsonErrorKind::TooDeep { .. }) => {
                    return Err(internal(e.to_string()))
                }
                Err(_) => return Err(valid()),
            };
            &parsed
        }
        other => other,
    };
    if !matches!(value, Value::Dict(_)) {
        return Err(invalid(
            format!("{field} must be a JSON object"),
            vec![("field", s(field))],
        ));
    }
    let text = compact(value, false).map_err(|_| {
        invalid(
            format!("{field} contains a value that cannot be encoded as JSON"),
            vec![("field", s(field))],
        )
    })?;
    kassiber_pytext::loads(&text).map_err(|e| internal(e.to_string()))
}
pub(super) fn json_text(v: &Value, field: &str) -> Result<String> {
    compact(&json_object(v, field)?, false).map_err(|e| internal(e.to_string()))
}
/// `time_utils.parse_timestamp`.
pub(super) fn timestamp(v: &Value) -> Result<Value> {
    if !truthy(v) {
        return Err(AppError::new("app_error", "Missing occurred_at/date value"));
    }
    review::canonical_timestamp(&codec::to_json(v))
        .map(s)
        .map_err(|e| {
            if e.code == "validation" {
                AppError::new(
                    "validation",
                    format!("Invalid occurred_at/date value '{}'", py_str(v)),
                )
                .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
            } else {
                internal(e.message)
            }
        })
}
fn choice(v: &Value, field: &str, supported: &[&str], label: &str) -> Result<Value> {
    let text = required_text(v, field, true)?;
    if !supported.iter().any(|x| text == s(x)) {
        return Err(invalid(
            format!("{field} is not supported"),
            vec![
                (label, text),
                ("supported", Value::List(supported.iter().map(s).collect())),
            ],
        ));
    }
    Ok(text)
}
pub(super) fn component_type(v: &Value) -> Result<Value> {
    choice(v, "component_type", TYPES, "component_type")
}
pub(super) fn mode(v: &Value) -> Result<Value> {
    choice(v, "conservation_mode", MODES, "conservation_mode")
}
pub(super) fn authored_source(v: &Value) -> Result<Value> {
    let source = required_text(v, "authored_source", false)?;
    if !SOURCES.iter().any(|x| source == s(x)) {
        return Err(invalid(
            "authored_source is invalid",
            vec![("authored_source", source)],
        ));
    }
    Ok(source)
}
fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}

/// `normalize_component_header`; absent keyword arguments take Python's defaults.
pub(super) fn header(spec: &Value) -> Result<Value> {
    let component_id = optional_text(field(spec, "component_id"), "component_id", false)?;
    let lineage_id = optional_text(field(spec, "lineage_id"), "lineage_id", false)?;
    let source = authored_source(get(spec, "authored_source").unwrap_or(&s("user")))?;
    let reviewed = get(spec, "conversion_reviewed")
        .cloned()
        .unwrap_or(Value::Bool(false));
    if !matches!(reviewed, Value::Bool(_)) {
        return Err(invalid("conversion_reviewed must be a boolean", vec![]));
    }
    let lineage = if matches!(lineage_id, Value::Null) {
        component_id.clone()
    } else {
        lineage_id
    };
    let fields = vec![
        (
            "component_type",
            component_type(field(spec, "component_type"))?,
        ),
        (
            "conservation_mode",
            mode(get(spec, "conservation_mode").unwrap_or(&s("quantity")))?,
        ),
        (
            "evidence_kind",
            optional_text(field(spec, "evidence_kind"), "evidence_kind", true)?,
        ),
        (
            "evidence_grade",
            optional_text(field(spec, "evidence_grade"), "evidence_grade", true)?,
        ),
        (
            "evidence",
            json_object(field(spec, "evidence"), "evidence")?,
        ),
        (
            "conversion_policy",
            optional_text(field(spec, "conversion_policy"), "conversion_policy", true)?,
        ),
        ("conversion_reviewed", reviewed),
        (
            "conversion_metadata",
            json_object(field(spec, "conversion_metadata"), "conversion_metadata")?,
        ),
        (
            "notes",
            optional_text(field(spec, "notes"), "notes", false)?,
        ),
        (
            "change_reason",
            optional_text(field(spec, "change_reason"), "change_reason", false)?,
        ),
        ("component_id", component_id),
        ("lineage_id", lineage),
        (
            "created_at",
            match field(spec, "created_at") {
                Value::Null => Value::Null,
                v => timestamp(v)?,
            },
        ),
        ("authored_source", source),
    ];
    Ok(dict(fields))
}

fn leg(raw: &Value, ordinal: usize, sources: &dyn Sources, c: &Connection) -> Result<Value> {
    if !matches!(raw, Value::Dict(_)) {
        return Err(invalid(
            "each custody leg must be an object",
            vec![("ordinal", int(ordinal))],
        ));
    }
    let role = required_text(field(raw, "role"), "role", true)?;
    if !ROLES.iter().any(|r| role == s(r)) {
        return Err(invalid(
            "custody leg role is not supported",
            vec![
                ("ordinal", int(ordinal)),
                ("role", role),
                ("supported", Value::List(ROLES.iter().map(s).collect())),
            ],
        ));
    }
    let amount = nonnegative(field(raw, "amount_msat"), "amount_msat")?;
    let unit = optional_text(field(raw, "valuation_unit"), "valuation_unit", true)?;
    let valuation = match field(raw, "valuation_amount") {
        Value::Null => Value::Null,
        v => nonnegative(v, "valuation_amount")?,
    };
    if matches!(unit, Value::Null) != matches!(valuation, Value::Null) {
        return Err(invalid(
            "valuation_unit and valuation_amount must be supplied together",
            vec![("ordinal", int(ordinal))],
        ));
    }
    let tx = optional_text(field(raw, "transaction_id"), "transaction_id", false)?;
    let anchor = optional_text(
        field(raw, "anchor_transaction_id"),
        "anchor_transaction_id",
        false,
    )?;
    if !matches!(tx, Value::Null) && !matches!(anchor, Value::Null) && tx != anchor {
        return Err(invalid(
            "anchor_transaction_id must identify the live transaction anchor",
            vec![("ordinal", int(ordinal))],
        ));
    }
    let mut occurred = optional_text(field(raw, "occurred_at"), "occurred_at", false)?;
    if !matches!(occurred, Value::Null) {
        occurred = timestamp(&occurred)?;
    }
    let id = match optional_text(field(raw, "id"), "id", false)? {
        Value::Null => s(sources.uuid(c)?),
        id => id,
    };
    Ok(dict([
        ("id", id),
        ("ordinal", int(ordinal)),
        ("role", role),
        ("rail", required_text(field(raw, "rail"), "rail", true)?),
        ("chain", optional_text(field(raw, "chain"), "chain", true)?),
        (
            "network",
            optional_text(field(raw, "network"), "network", true)?,
        ),
        ("asset", required_text(field(raw, "asset"), "asset", false)?),
        (
            "exposure",
            required_text(field(raw, "exposure"), "exposure", true)?,
        ),
        (
            "conservation_unit",
            required_text(field(raw, "conservation_unit"), "conservation_unit", true)?,
        ),
        ("amount_msat", amount),
        ("valuation_unit", unit),
        ("valuation_amount", valuation),
        ("occurred_at", occurred),
        ("transaction_id", tx.clone()),
        (
            "anchor_transaction_id",
            if matches!(anchor, Value::Null) {
                tx
            } else {
                anchor
            },
        ),
        (
            "wallet_id",
            optional_text(field(raw, "wallet_id"), "wallet_id", false)?,
        ),
        (
            "location_ref",
            optional_text(field(raw, "location_ref"), "location_ref", false)?,
        ),
        ("notes", optional_text(field(raw, "notes"), "notes", false)?),
    ]))
}
pub(super) fn legs(raw: &[Value], sources: &dyn Sources, c: &Connection) -> Result<Vec<Value>> {
    let legs = raw
        .iter()
        .enumerate()
        .map(|(i, raw)| leg(raw, i, sources, c))
        .collect::<Result<Vec<_>>>()?;
    let ids: BTreeSet<_> = legs.iter().map(|l| py_str(field(l, "id"))).collect();
    if ids.len() != legs.len() {
        return Err(invalid("custody leg ids must be unique", vec![]));
    }
    Ok(legs)
}
fn resolve_leg<'a>(
    raw: &Value,
    kind: &str,
    legs: &'a [Value],
    strict: bool,
    detail: (&str, usize),
    label: &str,
) -> Result<&'a Value> {
    let id = optional_text(
        field(raw, &format!("{kind}_leg_id")),
        &format!("{kind}_leg_id"),
        false,
    )?;
    let found = if let Value::Str(id) = &id {
        legs.iter()
            .rev()
            .find(|l| matches!(field(l, "id"), Value::Str(x) if x == id))
    } else {
        match field(raw, &format!("{kind}_ordinal")) {
            Value::Int(n) => legs
                .iter()
                .rev()
                .find(|l| field(l, "ordinal") == &Value::Int(n.clone())),
            _ if strict => {
                return Err(invalid(
                    format!("{kind}_leg_id or {kind}_ordinal is required"),
                    vec![(detail.0, int(detail.1))],
                ))
            }
            _ => None,
        }
    };
    found.ok_or_else(|| {
        invalid(
            format!("{label} {kind} leg was not found"),
            vec![(detail.0, int(detail.1))],
        )
    })
}
pub(super) fn allocations(
    raw: &[Value],
    legs: &[Value],
    sources: &dyn Sources,
    c: &Connection,
) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for (ordinal, raw) in raw.iter().enumerate() {
        if !matches!(raw, Value::Dict(_)) {
            return Err(invalid(
                "each custody allocation must be an object",
                vec![("ordinal", int(ordinal))],
            ));
        }
        let detail = ("allocation_ordinal", ordinal);
        let source = resolve_leg(raw, "source", legs, true, detail, "allocation")?;
        let sink = resolve_leg(raw, "sink", legs, true, detail, "allocation")?;
        if field(source, "role") != &s("source") || field(sink, "role") == &s("source") {
            return Err(invalid(
                "allocation edges must run from a source leg to a sink leg",
                vec![("allocation_ordinal", int(ordinal))],
            ));
        }
        let id = match optional_text(field(raw, "id"), "id", false)? {
            Value::Null => s(sources.uuid(c)?),
            id => id,
        };
        result.push(dict([
            ("id", id),
            ("ordinal", int(ordinal)),
            ("source_leg_id", field(source, "id").clone()),
            ("sink_leg_id", field(sink, "id").clone()),
            (
                "source_amount_msat",
                nonnegative(field(raw, "source_amount_msat"), "source_amount_msat")?,
            ),
            (
                "sink_amount_msat",
                nonnegative(field(raw, "sink_amount_msat"), "sink_amount_msat")?,
            ),
        ]));
    }
    let ids: BTreeSet<_> = result.iter().map(|a| py_str(field(a, "id"))).collect();
    let edges: BTreeSet<_> = result
        .iter()
        .map(|a| {
            (
                py_str(field(a, "source_leg_id")),
                py_str(field(a, "sink_leg_id")),
            )
        })
        .collect();
    if ids.len() != result.len() || edges.len() != result.len() {
        return Err(invalid(
            "custody allocation ids and source/sink edges must be unique",
            vec![],
        ));
    }
    Ok(result)
}
pub(super) fn economic_terms(
    raw: &[Value],
    legs: &[Value],
    sources: &dyn Sources,
    c: &Connection,
) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for (ordinal, raw) in raw.iter().enumerate() {
        if !matches!(raw, Value::Dict(_)) {
            return Err(invalid(
                "each economic term must be an object",
                vec![("ordinal", int(ordinal))],
            ));
        }
        let detail = ("term_ordinal", ordinal);
        let source = resolve_leg(raw, "source", legs, false, detail, "economic term")?;
        let target = resolve_leg(raw, "target", legs, false, detail, "economic term")?;
        if field(source, "role") != &s("source") || field(target, "role") == &s("source") {
            return Err(invalid(
                "economic terms must bind a source leg to a non-source leg",
                vec![("ordinal", int(ordinal))],
            ));
        }
        let kind = required_text(field(raw, "term_kind"), "term_kind", true)?;
        if kind != s("transaction_pair") && kind != s("direct_swap_payout") {
            return Err(invalid(
                "economic term kind is not supported",
                vec![("ordinal", int(ordinal)), ("term_kind", kind)],
            ));
        }
        let hash = required_text(field(raw, "source_row_hash"), "source_row_hash", false)?;
        if !matches!(&hash, Value::Str(h) if h.as_str().is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))))
        {
            return Err(invalid(
                "source_row_hash must be lowercase SHA-256 hex",
                vec![("ordinal", int(ordinal))],
            ));
        }
        let price = field(raw, "payout_fiat_value_exact");
        if !matches!(price, Value::Null) && price != &s("") {
            let value = kassiber_pydecimal::finite_price(&py_str(price))
                .map_err(|e| AppError::new("validation", e.message("payout_fiat_value_exact")))?;
            // The value per BTC of both reviewed quantities must be a
            // supported price as well (`payout_value / msat_to_btc(q)` at
            // the process context, precision 32).
            for (key, leg) in [
                ("reviewed_source_amount_msat", source),
                ("payout_amount_msat", target),
            ] {
                let quantity = match signed(field(raw, key), key)? {
                    Value::Null => field(leg, "amount_msat").clone(),
                    quantity => quantity,
                };
                let Value::Int(quantity) = quantity else {
                    continue;
                };
                if quantity <= BigInt::default() {
                    continue;
                }
                let context = &kassiber_pydecimal::Context::RP2;
                let rate = quantity
                    .to_string()
                    .parse::<kassiber_pydecimal::Decimal>()
                    .and_then(|q| {
                        q.div(
                            &kassiber_pydecimal::Decimal::from(100_000_000_000_i64),
                            context,
                        )
                    })
                    .and_then(|btc| value.div(&btc, context))
                    .map_err(|e| internal(e.to_string()))?;
                kassiber_pydecimal::finite_price(&rate.to_string())
                    .map_err(|e| AppError::new("validation", e.message("payout_fiat_rate")))?;
            }
        }
        let id = match optional_text(field(raw, "id"), "id", false)? {
            Value::Null => s(sources.uuid(c)?),
            id => id,
        };
        let text = |key: &str| optional_text(field(raw, key), key, false);
        result.push(dict([
            ("id", id),
            ("ordinal", int(ordinal)),
            ("source_leg_id", field(source, "id").clone()),
            ("target_leg_id", field(target, "id").clone()),
            ("term_kind", kind),
            (
                "legacy_source_id",
                required_text(field(raw, "legacy_source_id"), "legacy_source_id", false)?,
            ),
            ("source_row_hash", hash),
            (
                "review_kind",
                required_text(field(raw, "review_kind"), "review_kind", false)?,
            ),
            (
                "tax_policy",
                required_text(field(raw, "tax_policy"), "tax_policy", false)?,
            ),
            (
                "reviewed_source_amount_msat",
                signed(
                    field(raw, "reviewed_source_amount_msat"),
                    "reviewed_source_amount_msat",
                )?,
            ),
            (
                "swap_fee_msat",
                signed(field(raw, "swap_fee_msat"), "swap_fee_msat")?,
            ),
            ("swap_fee_kind", text("swap_fee_kind")?),
            ("confidence_at_review", text("confidence_at_review")?),
            ("review_source", text("review_source")?),
            ("review_notes", text("review_notes")?),
            ("payout_asset", text("payout_asset")?),
            (
                "payout_amount_msat",
                signed(field(raw, "payout_amount_msat"), "payout_amount_msat")?,
            ),
            ("payout_occurred_at", text("payout_occurred_at")?),
            ("payout_fiat_value_exact", text("payout_fiat_value_exact")?),
            ("payout_external_id", text("payout_external_id")?),
            ("counterparty", text("counterparty")?),
        ]));
    }
    let ids: BTreeSet<_> = result.iter().map(|t| py_str(field(t, "id"))).collect();
    let identities: BTreeSet<_> = result
        .iter()
        .map(|t| {
            (
                py_str(field(t, "term_kind")),
                py_str(field(t, "legacy_source_id")),
            )
        })
        .collect();
    if ids.len() != result.len() || identities.len() != result.len() {
        return Err(invalid(
            "economic term ids and legacy source identities must be unique",
            vec![],
        ));
    }
    Ok(result)
}

/// `_scope`: the component profile must belong to the workspace.
pub(super) fn scope_check(c: &Connection, scope: &Scope<'_>) -> Result<()> {
    let row = rows(
        c,
        "SELECT workspace_id FROM profiles WHERE id=?",
        [scope.profile_id],
    )?;
    let Some(row) = row.first() else {
        return Err(fail(
            "not_found",
            "profile was not found",
            vec![("profile_id", s(scope.profile_id))],
        ));
    };
    if text(row, "workspace_id") != scope.workspace_id {
        return Err(fail(
            "custody_component_scope_mismatch",
            "profile does not belong to the workspace",
            vec![
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(scope.profile_id)),
            ],
        ));
    }
    Ok(())
}
/// `_validate_leg_anchors`; anchored legs take the canonical occurrence time.
pub(super) fn validate_anchors(
    c: &Connection,
    scope: &Scope<'_>,
    legs: &mut [Value],
    planned_wallets: &BTreeSet<String>,
) -> Result<()> {
    for leg in legs.iter_mut() {
        let ordinal = field(leg, "ordinal").clone();
        let tx = field(leg, "transaction_id").clone();
        if !matches!(tx, Value::Null) {
            let found = rows(
                c,
                "SELECT workspace_id,profile_id,occurred_at FROM transactions WHERE id=?",
                [bind(&tx, 1)?],
            )?;
            let Some(row) = found.first() else {
                return Err(fail(
                    "not_found",
                    "custody leg transaction was not found",
                    vec![("transaction_id", tx), ("ordinal", ordinal)],
                ));
            };
            if field(row, "workspace_id") != &s(scope.workspace_id)
                || field(row, "profile_id") != &s(scope.profile_id)
            {
                return Err(fail(
                    "custody_component_scope_mismatch",
                    "custody leg transaction is outside the component scope",
                    vec![("transaction_id", tx), ("ordinal", ordinal)],
                ));
            }
            let canonical = timestamp(field(row, "occurred_at"))?;
            let occurred = field(leg, "occurred_at").clone();
            if !matches!(occurred, Value::Null) && timestamp(&occurred)? != canonical {
                return Err(fail(
                    "custody_component_anchor_time_mismatch",
                    "anchored custody legs must use the transaction occurrence time",
                    vec![
                        ("transaction_id", tx),
                        ("ordinal", ordinal),
                        ("transaction_occurred_at", canonical),
                        ("leg_occurred_at", occurred),
                    ],
                ));
            }
            set(leg, "occurred_at", canonical);
        }
        let wallet = field(leg, "wallet_id").clone();
        if !matches!(wallet, Value::Null) {
            let found = rows(
                c,
                "SELECT workspace_id,profile_id FROM wallets WHERE id=?",
                [bind(&wallet, 1)?],
            )?;
            if found.is_empty() && planned_wallets.contains(&py_str(&wallet)) {
                continue;
            }
            let Some(row) = found.first() else {
                return Err(fail(
                    "not_found",
                    "custody leg wallet was not found",
                    vec![("wallet_id", wallet), ("ordinal", ordinal)],
                ));
            };
            if field(row, "workspace_id") != &s(scope.workspace_id)
                || field(row, "profile_id") != &s(scope.profile_id)
            {
                return Err(fail(
                    "custody_component_scope_mismatch",
                    "custody leg wallet is outside the component scope",
                    vec![("wallet_id", wallet), ("ordinal", ordinal)],
                ));
            }
        }
    }
    Ok(())
}

pub(super) fn to_json(v: &Value) -> serde_json::Value {
    codec::to_json(v)
}
fn from_json(v: &serde_json::Value) -> Result<Value> {
    codec::from_json(v).map_err(internal)
}
fn custody_error(e: kassiber_custody::components::Error) -> AppError {
    if matches!(e.code, "validation" | "custody_component_validation") {
        AppError::new(e.code, e.message)
    } else {
        internal(e.message)
    }
}
pub(super) fn project_validation(v: &serde_json::Value) -> Result<Value> {
    review::validation_projection(v).map_err(internal)
}

/// `validate_component_plan`: normalization, anchors, conservation and
/// raw-active arbitration for rows that are not stored yet.
pub(super) struct PlanCheck {
    pub legs: Vec<Value>,
    pub allocations: Vec<Value>,
    pub validation: Value,
}
#[allow(clippy::too_many_arguments)]
pub(super) fn validate_plan(
    c: &Connection,
    scope: &Scope<'_>,
    snapshot: &Snapshot,
    head: &Value,
    raw_legs: &[Value],
    raw_allocations: &[Value],
    replacing_lineage: Option<&str>,
    planned_wallets: &BTreeSet<String>,
    sources: &dyn Sources,
) -> Result<PlanCheck> {
    component_type(field(head, "component_type"))?;
    mode(field(head, "conservation_mode"))?;
    let mut legs = legs(raw_legs, sources, c)?;
    let allocations = allocations(raw_allocations, &legs, sources, c)?;
    scope_check(c, scope)?;
    validate_anchors(c, scope, &mut legs, planned_wallets)?;
    let header = serde_json::json!({
        "conservation_mode": to_json(field(head, "conservation_mode")),
        "conversion_policy": to_json(field(head, "conversion_policy")),
        "conversion_reviewed": to_json(field(head, "conversion_reviewed")),
        "component_type": to_json(field(head, "component_type")),
        "evidence_grade": to_json(field(head, "evidence_grade")),
    });
    let json_legs: Vec<_> = legs.iter().map(to_json).collect();
    let json_allocations: Vec<_> = allocations.iter().map(to_json).collect();
    let mut validation = review::plan_validation(snapshot, &json_legs, &json_allocations, &header)
        .map_err(custody_error)?;
    let ids: BTreeSet<String> = legs
        .iter()
        .filter_map(|l| {
            let anchor = field(l, "anchor_transaction_id");
            let id = if truthy(anchor) {
                anchor
            } else {
                field(l, "transaction_id")
            };
            truthy(id).then(|| py_str(id))
        })
        .collect();
    let mut conflicts = Vec::new();
    if !ids.is_empty() {
        let marks = vec!["?"; ids.len()].join(",");
        let mut bound = vec![scope.profile_id.to_string()];
        bound.extend(ids.iter().cloned());
        let found = rows(
            c,
            &format!("SELECT DISTINCT COALESCE(l.anchor_transaction_id,l.transaction_id) AS transaction_id,l.component_id,c.lineage_id FROM custody_component_legs l JOIN custody_components c ON c.id=l.component_id WHERE l.profile_id=? AND c.state='active' AND COALESCE(l.anchor_transaction_id,l.transaction_id) IN ({marks}) ORDER BY transaction_id,l.component_id"),
            kassiber_store::rusqlite::params_from_iter(bound),
        )?;
        for row in found {
            if replacing_lineage.is_some_and(|lineage| text(&row, "lineage_id") == lineage) {
                continue;
            }
            conflicts.push(serde_json::json!({
                "transaction_id": text(&row, "transaction_id"),
                "component_id": text(&row, "component_id"),
            }));
        }
    }
    if !conflicts.is_empty() {
        validation["activatable"] = serde_json::json!(false);
        let mut issues = validation["issues"].as_array().cloned().unwrap_or_default();
        issues.push(remediation::membership_conflict_issue(
            conflicts,
            "a transaction belongs to another active component",
        ));
        validation["issues"] = serde_json::json!(issues);
    }
    Ok(PlanCheck {
        legs,
        allocations,
        validation: project_validation(&validation)?,
    })
}

/// Python-order public projection of a materialized component (no local evidence).
fn projection(c: &Connection, v: &serde_json::Value) -> Result<Value> {
    let pick = |v: &serde_json::Value, keys: &[&str]| -> Result<Value> {
        let mut pairs = Vec::new();
        for key in keys {
            if let Some(item) = v.get(*key) {
                pairs.push((*key, from_json(item)?));
            }
        }
        Ok(dict(pairs))
    };
    let active = v["state"] == "active";
    let evidence = if active {
        &[
            "expected_count",
            "commitment_count",
            "valid",
            "usable",
            "status",
            "reason",
            "current_count",
            "detail",
        ][..]
    } else {
        &["valid", "status", "expected_count", "commitment_count"][..]
    };
    let native = if active {
        &[
            "boundary_count",
            "supported_boundary_count",
            "partially_supported_boundary_count",
            "contradicted_boundary_count",
            "status",
            "usable",
        ][..]
    } else {
        &[
            "status",
            "usable",
            "boundary_count",
            "supported_boundary_count",
            "partially_supported_boundary_count",
            "contradicted_boundary_count",
        ][..]
    };
    let mut terms_order: Vec<String> = rows(
        c,
        "SELECT name FROM pragma_table_info('custody_component_economic_terms') ORDER BY cid",
        [],
    )?
    .iter()
    .map(|r| text(r, "name"))
    .collect();
    if terms_order.is_empty() {
        terms_order = TERM_FIELDS.iter().map(|k| k.to_string()).collect();
    }
    let terms_order: Vec<&str> = terms_order.iter().map(String::as_str).collect();
    let list = |key: &str, order: &[&str]| -> Result<Value> {
        Ok(Value::List(
            v[key]
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or(&[])
                .iter()
                .map(|item| pick(item, order))
                .collect::<Result<_>>()?,
        ))
    };
    let leg_order: Vec<&str> = LEG_FIELDS
        .iter()
        .copied()
        .filter(|k| *k != "location_ref")
        .collect();
    let mut pairs = Vec::new();
    for key in [
        "id",
        "lineage_id",
        "workspace_id",
        "profile_id",
        "revision",
        "component_type",
        "conservation_mode",
        "state",
        "effective_state",
        "evidence_kind",
        "evidence_grade",
        "conversion_policy",
        "conversion_reviewed",
        "expected_leg_count",
        "expected_allocation_count",
        "expected_economic_term_count",
        "expected_evidence_count",
        "evidence_status",
        "native_support_status",
        "authored_source",
        "notes",
        "change_reason",
        "supersedes_component_id",
        "superseded_by_component_id",
        "activated_at",
        "superseded_at",
        "created_at",
        "legs",
        "allocations",
        "economic_terms",
        "validation",
    ] {
        let item = match key {
            "evidence_status" => pick(&v[key], evidence)?,
            "native_support_status" => pick(&v[key], native)?,
            "legs" => list(key, &leg_order)?,
            "allocations" => list(key, ALLOCATION_FIELDS)?,
            "economic_terms" => list(key, &terms_order)?,
            "validation" => project_validation(&v[key])?,
            _ => from_json(v.get(key).unwrap_or(&serde_json::Value::Null))?,
        };
        pairs.push((key, item));
    }
    Ok(dict(pairs))
}

fn stored(snapshot: &Snapshot, id: &str) -> Option<usize> {
    snapshot
        .components
        .iter()
        .position(|c| c.header.get("id").and_then(|v| v.as_str()) == Some(id))
}
fn row(c: &Connection, id: &str) -> Result<Value> {
    rows(c, "SELECT * FROM custody_components WHERE id=?", [id])?
        .into_iter()
        .next()
        .ok_or_else(|| {
            fail(
                "not_found",
                "custody component was not found",
                vec![("component_id", s(id))],
            )
        })
}
/// Load a stored component outside the profile snapshot (cross-profile reads fail later).
fn load(c: &Connection, header: &Value) -> Result<Component> {
    let record = |v: &Value| match to_json(v) {
        serde_json::Value::Object(m) => Record::from(m),
        _ => Record::default(),
    };
    let id = text(header, "id");
    let child = |table: &str| -> Result<Vec<Record>> {
        Ok(rows(
            c,
            &format!("SELECT * FROM {table} WHERE component_id=? ORDER BY ordinal,id"),
            [&id],
        )?
        .iter()
        .map(record)
        .collect())
    };
    Ok(Component {
        header: record(header),
        legs: child("custody_component_legs")?,
        allocations: child("custody_component_allocations")?,
        economic_terms: child("custody_component_economic_terms")?,
        commitments: child("custody_component_evidence_commitments")?,
    })
}
fn materialize(c: &Connection, snapshot: &Snapshot, header: &Value) -> Result<serde_json::Value> {
    let loaded;
    let component = match stored(snapshot, &text(header, "id")) {
        Some(at) => &snapshot.components[at],
        None => {
            loaded = load(c, header)?;
            &loaded
        }
    };
    review::materialize_component(snapshot, component).map_err(custody_error)
}

/// `get_component(..., include_local_evidence=False)`.
pub(super) fn get_component(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
    profile_scoped: bool,
) -> Result<Value> {
    let header = row(c, id)?;
    if profile_scoped && text(&header, "profile_id") != scope.profile_id {
        return Err(fail(
            "not_found",
            "custody component was not found in the profile",
            vec![("component_id", s(id)), ("profile_id", s(scope.profile_id))],
        ));
    }
    let snapshot = store::snapshot(c, scope)?;
    materialize_read(c, &snapshot, &header)
}

/// Share a profile snapshot across stored list views, as Python's list does.
pub(super) fn materialize_read(
    c: &Connection,
    snapshot: &Snapshot,
    header: &Value,
) -> Result<Value> {
    projection(c, &materialize(c, snapshot, header)?)
}

/// Persisted child rows in Python's insertion order.
pub(super) struct Content<'a> {
    pub legs: &'a [Value],
    pub allocations: &'a [Value],
    pub terms: &'a [Value],
}
fn insert_children(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
    content: &Content<'_>,
    now: &Value,
) -> Result<()> {
    let base = |row: &Value, fields: &[&str]| -> Value {
        let mut out = Vec::new();
        for key in fields {
            let v = match *key {
                "component_id" => s(id),
                "workspace_id" => s(scope.workspace_id),
                "profile_id" => s(scope.profile_id),
                "created_at" => now.clone(),
                _ => field(row, key).clone(),
            };
            out.push((*key, v));
        }
        dict(out)
    };
    for leg in content.legs {
        store::insert(c, "custody_component_legs", &base(leg, LEG_FIELDS))?;
    }
    for allocation in content.allocations {
        store::insert(
            c,
            "custody_component_allocations",
            &base(allocation, ALLOCATION_FIELDS),
        )?;
    }
    for term in content.terms {
        store::insert(
            c,
            "custody_component_economic_terms",
            &base(term, TERM_FIELDS),
        )?;
    }
    Ok(())
}

/// `create_component` for an already prepared plan; returns the new id.
pub(super) fn create(
    c: &Connection,
    scope: &Scope<'_>,
    spec: &Value,
    source: &str,
    sources: &dyn Sources,
) -> Result<String> {
    let mut legs = legs(list(spec, "legs"), sources, c)?;
    let allocations = allocations(list(spec, "allocations"), &legs, sources, c)?;
    let terms = economic_terms(list(spec, "economic_terms"), &legs, sources, c)?;
    let id = match optional_text(field(spec, "component_id"), "component_id", false)? {
        Value::Null => sources.uuid(c)?,
        id => py_str(&id),
    };
    let mut input = spec.clone();
    set(&mut input, "component_id", s(&id));
    set(&mut input, "authored_source", s(source));
    let head = header(&input)?;
    let now = match field(&head, "created_at") {
        Value::Null => s(sources.now_precise(c)?),
        v => v.clone(),
    };
    scope_check(c, scope)?;
    validate_anchors(c, scope, &mut legs, &BTreeSet::new())?;
    let lineage = text(&head, "lineage_id");
    if let Some(existing) = rows(
        c,
        "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? LIMIT 1",
        [scope.profile_id, &lineage],
    )?
    .first()
    {
        return Err(fail(
            "custody_component_lineage_exists",
            "component lineage already exists; create a revision instead",
            vec![
                ("lineage_id", s(&lineage)),
                ("component_id", value(existing, "id")),
            ],
        ));
    }
    insert_header(
        c,
        scope,
        &id,
        &head,
        1,
        None,
        &now,
        &Content {
            legs: &legs,
            allocations: &allocations,
            terms: &terms,
        },
    )?;
    insert_children(
        c,
        scope,
        &id,
        &Content {
            legs: &legs,
            allocations: &allocations,
            terms: &terms,
        },
        &now,
    )?;
    Ok(id)
}
#[allow(clippy::too_many_arguments)]
fn insert_header(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
    head: &Value,
    revision: i64,
    supersedes: Option<&str>,
    now: &Value,
    content: &Content<'_>,
) -> Result<()> {
    let mut fields = vec![
        ("id", s(id)),
        ("lineage_id", field(head, "lineage_id").clone()),
        ("workspace_id", s(scope.workspace_id)),
        ("profile_id", s(scope.profile_id)),
        ("revision", int(revision)),
        ("component_type", field(head, "component_type").clone()),
        (
            "conservation_mode",
            field(head, "conservation_mode").clone(),
        ),
        ("state", s("draft")),
        ("evidence_kind", field(head, "evidence_kind").clone()),
        ("evidence_grade", field(head, "evidence_grade").clone()),
        ("evidence_json", field(head, "evidence_json").clone()),
        (
            "conversion_policy",
            field(head, "conversion_policy").clone(),
        ),
        (
            "conversion_reviewed",
            int(u8::from(truthy(field(head, "conversion_reviewed")))),
        ),
        (
            "conversion_metadata_json",
            field(head, "conversion_metadata_json").clone(),
        ),
        ("expected_leg_count", int(content.legs.len())),
        ("expected_allocation_count", int(content.allocations.len())),
        ("expected_economic_term_count", int(content.terms.len())),
        ("authored_source", field(head, "authored_source").clone()),
        ("notes", field(head, "notes").clone()),
        ("change_reason", field(head, "change_reason").clone()),
    ];
    if let Some(prior) = supersedes {
        fields.push(("supersedes_component_id", s(prior)));
    }
    fields.push(("created_at", now.clone()));
    let mut row = dict(fields);
    if matches!(field(head, "evidence_json"), Value::Null) {
        set(
            &mut row,
            "evidence_json",
            s(json_text(field(head, "evidence"), "evidence")?),
        );
        set(
            &mut row,
            "conversion_metadata_json",
            s(json_text(
                field(head, "conversion_metadata"),
                "conversion_metadata",
            )?),
        );
    }
    store::insert(c, "custody_components", &row)
}

/// `update_component(..., preserve_planned_row_ids=True)` with every field supplied.
pub(super) fn revise(
    c: &Connection,
    old_id: &str,
    new_id: &str,
    spec: &Value,
    sources: &dyn Sources,
) -> Result<String> {
    let old = row(c, old_id)?;
    let profile = text(&old, "profile_id");
    let lineage = text(&old, "lineage_id");
    if let Some(draft) = rows(
        c,
        "SELECT id FROM custody_components WHERE profile_id=? AND lineage_id=? AND state='draft' AND id!=?",
        params![profile, lineage, old_id],
    )?
    .first()
    {
        return Err(fail(
            "custody_component_draft_exists",
            "component lineage already has a draft revision",
            vec![("draft_component_id", value(draft, "id"))],
        ));
    }
    let mut legs = legs(list(spec, "legs"), sources, c)?;
    let allocations = allocations(list(spec, "allocations"), &legs, sources, c)?;
    let terms = economic_terms(list(spec, "economic_terms"), &legs, sources, c)?;
    let old_scope = Scope {
        workspace_id: &text(&old, "workspace_id"),
        profile_id: &profile,
    };
    validate_anchors(c, &old_scope, &mut legs, &BTreeSet::new())?;
    let revision = rows(
        c,
        "SELECT COALESCE(MAX(revision),0)+1 AS revision FROM custody_components WHERE profile_id=? AND lineage_id=?",
        [&profile, &lineage],
    )?;
    let revision = match value(&revision[0], "revision") {
        Value::Int(n) => i64::try_from(n).map_err(|_| internal("revision overflow"))?,
        _ => return Err(internal("revision is not an integer")),
    };
    let new_id = py_str(&optional_text(&s(new_id), "new_component_id", false)?);
    let now = s(sources.now_precise(c)?);
    let source = authored_source(field(spec, "authored_source"))?;
    let head = dict([
        ("lineage_id", s(&lineage)),
        (
            "component_type",
            component_type(field(spec, "component_type"))?,
        ),
        ("conservation_mode", mode(field(spec, "conservation_mode"))?),
        (
            "evidence_kind",
            optional_text(field(spec, "evidence_kind"), "evidence_kind", true)?,
        ),
        (
            "evidence_grade",
            optional_text(field(spec, "evidence_grade"), "evidence_grade", true)?,
        ),
        (
            "evidence_json",
            s(json_text(field(spec, "evidence"), "evidence")?),
        ),
        (
            "conversion_policy",
            optional_text(field(spec, "conversion_policy"), "conversion_policy", true)?,
        ),
        ("conversion_reviewed", {
            let v = field(spec, "conversion_reviewed");
            if !matches!(v, Value::Bool(_)) {
                return Err(invalid("conversion_reviewed must be a boolean", vec![]));
            }
            v.clone()
        }),
        (
            "conversion_metadata_json",
            s(json_text(
                field(spec, "conversion_metadata"),
                "conversion_metadata",
            )?),
        ),
        ("authored_source", source),
        (
            "notes",
            optional_text(field(spec, "notes"), "notes", false)?,
        ),
        (
            "change_reason",
            optional_text(field(spec, "change_reason"), "change_reason", false)?,
        ),
    ]);
    if text(&old, "state") == "draft" {
        c.execute(
            "UPDATE custody_components SET state='superseded',superseded_at=?,superseded_by_component_id=?,change_reason=COALESCE(?,change_reason) WHERE id=? AND state='draft'",
            params![py_str(&now), new_id, bind(field(&head, "change_reason"), 3)?, old_id],
        )
        .map_err(error)?;
    }
    let content = Content {
        legs: &legs,
        allocations: &allocations,
        terms: &terms,
    };
    insert_header(
        c,
        &old_scope,
        &new_id,
        &head,
        revision,
        Some(old_id),
        &now,
        &content,
    )?;
    insert_children(c, &old_scope, &new_id, &content, &now)?;
    Ok(new_id)
}

/// `validate_component_activation`: the current materialization plus external
/// raw-active overlaps. Returns the snapshot and materialized component.
pub(super) fn activation(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
) -> Result<(Snapshot, serde_json::Value)> {
    let header = row(c, id)?;
    let snapshot = store::snapshot(c, scope)?;
    let mut component = materialize(c, &snapshot, &header)?;
    if text(&header, "state") == "superseded" {
        return Err(fail(
            "custody_component_superseded",
            "superseded revisions cannot be activated directly",
            vec![("component_id", s(id))],
        ));
    }
    let lineage = text(&header, "lineage_id");
    let conflicts: Vec<_> = rows(
        c,
        "SELECT DISTINCT COALESCE(mine.anchor_transaction_id,mine.transaction_id) AS transaction_id,other.component_id,oc.lineage_id FROM custody_component_legs mine JOIN custody_component_legs other ON other.profile_id=mine.profile_id AND COALESCE(other.anchor_transaction_id,other.transaction_id)=COALESCE(mine.anchor_transaction_id,mine.transaction_id) AND other.component_id!=mine.component_id JOIN custody_components oc ON oc.id=other.component_id AND oc.state='active' WHERE mine.component_id=? AND mine.profile_id=? AND COALESCE(mine.anchor_transaction_id,mine.transaction_id) IS NOT NULL ORDER BY transaction_id,other.component_id",
        params![id, text(&header, "profile_id")],
    )?
    .into_iter()
    .filter(|r| text(r, "lineage_id") != lineage)
    .map(|r| serde_json::json!({"transaction_id":to_json(&value(&r,"transaction_id")),"component_id":to_json(&value(&r,"component_id"))}))
    .collect();
    if !conflicts.is_empty() {
        component["validation"]["activatable"] = serde_json::json!(false);
        if let Some(issues) = component["validation"]["issues"].as_array_mut() {
            issues.push(remediation::membership_conflict_issue(
                conflicts,
                "a transaction belongs to another active component",
            ));
        }
    }
    if component["validation"]["activatable"] != true {
        return Err(activation_error(
            id,
            project_validation(&component["validation"])?,
        ));
    }
    Ok((snapshot, component))
}
pub(super) fn validated(c: &Connection, scope: &Scope<'_>, id: &str) -> Result<Value> {
    let (_, component) = activation(c, scope, id)?;
    projection(c, &component)
}

/// `activate_component`: membership, author-bound evidence and invalidation.
pub(super) fn activate(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
    sources: &dyn Sources,
) -> Result<()> {
    let (snapshot, component) = activation(c, scope, id)?;
    if component["state"] == "active" {
        return Ok(());
    }
    let now = sources.now_precise(c)?;
    let profile = text(&row(c, id)?, "profile_id");
    let lineage = component["lineage_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let txids: BTreeSet<String> = component["legs"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|l| !l["transaction_id"].is_null())
        .map(|l| codec::pystr(&l["transaction_id"]))
        .collect();
    c.execute(
        "UPDATE custody_components SET state='superseded',superseded_at=?,superseded_by_component_id=? WHERE profile_id=? AND lineage_id=? AND state='active' AND id!=?",
        params![now, id, profile, lineage, id],
    )
    .map_err(error)?;
    c.execute(
        "DELETE FROM custody_component_transaction_memberships WHERE component_id=?",
        [id],
    )
    .map_err(error)?;
    if !txids.is_empty() {
        let marks = vec!["?"; txids.len()].join(",");
        let mut bound = vec![profile.clone()];
        bound.extend(txids.iter().cloned());
        c.execute(
            &format!("DELETE FROM custody_component_transaction_memberships WHERE profile_id=? AND transaction_id IN ({marks}) AND component_id IN (SELECT id FROM custody_components WHERE state!='active')"),
            kassiber_store::rusqlite::params_from_iter(bound),
        )
        .map_err(error)?;
    }
    for tx in &txids {
        let inserted = c.execute(
            "INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES(?,?,?,?)",
            params![id, profile, tx, now],
        );
        if let Err(e) = inserted {
            if matches!(&e, kassiber_store::rusqlite::Error::SqliteFailure(f, _) if f.code == kassiber_store::rusqlite::ErrorCode::ConstraintViolation)
            {
                return Err(fail(
                    "custody_component_membership_conflict",
                    "a transaction already belongs to another active custody component",
                    vec![
                        ("component_id", s(id)),
                        (
                            "transaction_ids",
                            Value::List(txids.iter().map(s).collect()),
                        ),
                    ],
                ));
            }
            return Err(error(e));
        }
    }
    let stored_component = load(c, &row(c, id)?)?;
    let review::ActivationEvidence {
        snapshots,
        commitments,
    } = review::activation_evidence(&snapshot, &stored_component).map_err(custody_error)?;
    for item in snapshots {
        let item = from_json(&item)?;
        store::insert(
            c,
            "custody_authored_evidence_snapshots",
            &dict([
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(&profile)),
                ("subject_kind", s("custody_component")),
                ("subject_id", s(id)),
                ("detail_hash", value(&item, "detail_hash")),
                ("quantity_hash", value(&item, "quantity_hash")),
                ("payload_json", value(&item, "payload_json")),
                ("created_at", s(&now)),
            ]),
        )?;
    }
    for (ordinal, (quantity, detail)) in commitments.iter().enumerate() {
        let commitment_id = store::hash(
            &Value::List(vec![
                s("custody-component-evidence-v1"),
                s(id),
                int(ordinal),
            ]),
            true,
        )?;
        store::insert(
            c,
            "custody_component_evidence_commitments",
            &dict([
                ("id", s(commitment_id)),
                ("component_id", s(id)),
                ("workspace_id", s(scope.workspace_id)),
                ("profile_id", s(&profile)),
                ("ordinal", int(ordinal)),
                ("quantity_hash", s(quantity)),
                ("detail_hash", s(detail)),
                ("created_at", s(&now)),
            ]),
        )?;
    }
    c.execute(
        "UPDATE custody_components SET state='active',activated_at=?,superseded_at=NULL,superseded_by_component_id=NULL,expected_evidence_count=? WHERE id=? AND state='draft'",
        params![now, commitments.len() as i64, id],
    )
    .map_err(error)?;
    if rows(
        c,
        "SELECT 1 FROM custody_components WHERE id=? AND state='active'",
        [id],
    )?
    .is_empty()
    {
        return Err(fail(
            "custody_component_state_conflict",
            "custody component changed before activation",
            vec![("component_id", s(id))],
        ));
    }
    c.execute(
        "UPDATE custody_components SET superseded_by_component_id=? WHERE id=? AND state='superseded'",
        params![id, to_sql_text(&component["supersedes_component_id"])],
    )
    .map_err(error)?;
    store::invalidate(c, &profile)?;
    Ok(())
}
fn to_sql_text(v: &serde_json::Value) -> Option<String> {
    (!v.is_null()).then(|| codec::pystr(v))
}

/// `supersede_component`, rebuilding effective memberships after an active change.
/// `superseded_at` is the caller's explicit time; otherwise the clock's.
pub(super) fn supersede(
    c: &Connection,
    scope: &Scope<'_>,
    id: &str,
    reason: &Value,
    superseded_at: Option<&Value>,
    sources: &dyn Sources,
) -> Result<()> {
    let header = row(c, id)?;
    let state = text(&header, "state");
    if state == "superseded" {
        return Ok(());
    }
    let now = match superseded_at.filter(|v| truthy(v)) {
        Some(at) => at.clone(),
        None => s(sources.now_precise(c)?),
    };
    let reason = optional_text(reason, "reason", false)?;
    let profile = text(&header, "profile_id");
    c.execute(
        "UPDATE custody_components SET state='superseded',superseded_at=?,change_reason=COALESCE(?,change_reason) WHERE id=? AND state!='superseded'",
        params![bind(&now, 1)?, bind(&reason, 2)?, id],
    )
    .map_err(error)?;
    if state == "active" {
        store::invalidate(c, &profile)?;
        reconcile(c, scope, &profile, sources)?;
    }
    Ok(())
}
/// `reconcile_active_memberships`: only effective active components keep memberships.
pub(super) fn reconcile(
    c: &Connection,
    scope: &Scope<'_>,
    profile: &str,
    sources: &dyn Sources,
) -> Result<()> {
    let snapshot = store::snapshot(c, scope)?;
    let computed = kassiber_custody::components::compute(&snapshot).map_err(custody_error)?;
    let now = sources.now_precise(c)?;
    c.execute(
        "DELETE FROM custody_component_transaction_memberships WHERE profile_id=?",
        [profile],
    )
    .map_err(error)?;
    for component in computed["components"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .filter(|c| c["effective_state"] == "active")
    {
        let ids: BTreeSet<_> = component["legs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter(|l| !l["transaction_id"].is_null())
            .map(|l| codec::pystr(&l["transaction_id"]))
            .collect();
        for tx in ids {
            c.execute(
                "INSERT INTO custody_component_transaction_memberships(component_id,profile_id,transaction_id,created_at) VALUES(?,?,?,?)",
                params![codec::pystr(&component["id"]), profile, tx, now],
            )
            .map_err(error)?;
        }
    }
    Ok(())
}

/// `require_review_input_version`.
pub(super) fn require_input_version(
    c: &Connection,
    scope: &Scope<'_>,
    expected: &Value,
) -> Result<i64> {
    let valid = matches!(expected, Value::Int(n) if *n >= BigInt::from(0));
    if !valid {
        return Err(fail(
            "custody_review_plan_invalid",
            "Custody review plan input version is invalid",
            vec![("expected_input_version", expected.clone())],
        ));
    }
    let found = rows(
        c,
        "SELECT journal_input_version FROM profiles WHERE id=? AND workspace_id=?",
        [scope.profile_id, scope.workspace_id],
    )?;
    let Some(row) = found.first() else {
        return Err(fail(
            "not_found",
            "Custody review profile was not found",
            vec![],
        ));
    };
    let current = input_version(row);
    if Value::Int(current.into()) != *expected {
        return Err(fail(
            "custody_review_plan_stale",
            "Custody review plan is stale",
            vec![
                ("expected_input_version", expected.clone()),
                ("current_input_version", int(current)),
            ],
        ));
    }
    Ok(current)
}
pub(super) fn input_version(profile: &Value) -> i64 {
    match value(profile, "journal_input_version") {
        Value::Int(n) => i64::try_from(n).unwrap_or(0),
        _ => 0,
    }
}
pub(super) fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    store::list(v, key)
}
pub(super) fn keys(v: &Value) -> Vec<String> {
    match v {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(k, _)| match k {
                Key::Str(k) => Some(k.code_points().filter_map(char::from_u32).collect()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}
