//! `core/transaction_lookup.py`: exact transaction references, with
//! qualified txids for shared transactions.
//!
//! A txid is not a row key: a CoinJoin, a batched deposit or a self-transfer
//! between two owned wallets imports one row per wallet and direction under
//! the same txid. A reference resolves in this order and fails closed:
//!
//! 1. the Kassiber transaction id, exactly;
//! 2. the external id, exactly, when it names one row;
//! 3. a qualified `<external id>:<qualifier>`, when the bare external id
//!    names rows. The qualifier is a wallet id or label, `in` / `out`
//!    (`inbound` / `outbound`), or `<wallet>:<in|out>`.
//!
//! A reference that still names several rows raises `ambiguous_reference`
//! with every candidate and a remediation. Prefixes never resolve.

use kassiber_envelope::remediation::{build_remediation, Remediation};
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::{params, Connection};

use crate::sql::{rows, Result};

/// Candidates listed in an ambiguity error, at most.
pub const MAX_REFERENCE_CANDIDATES: usize = 20;

const CANDIDATE_SQL: &str = "SELECT t.*, w.label AS lookup_wallet_label FROM transactions t LEFT JOIN wallets w ON w.id = t.wallet_id WHERE t.profile_id = ? AND t.external_id = ? ORDER BY t.occurred_at DESC, t.created_at DESC, t.id DESC";

const AMBIGUOUS_HINT: &str = "Several rows share this reference, which is normal for CoinJoins and batched deposits. Use the full transaction id of the intended row, or qualify the reference as '<txid>:<wallet>' and/or ':in' / ':out'.";

/// `DIRECTION_QUALIFIERS[text.casefold()]`.
fn direction_qualifier(text: &str) -> Option<&'static str> {
    match kassiber_pytext::casefold(text).as_str() {
        "in" | "inbound" => Some("inbound"),
        "out" | "outbound" => Some("outbound"),
        _ => None,
    }
}

fn field(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}

fn text(row: &Value, key: &str) -> String {
    get(row, key).map(py_str).unwrap_or_default()
}

/// `int(value)` for a stored amount; `None` stays `None`.
pub fn python_int(value: Value) -> Result<Value> {
    Ok(match value {
        Value::Null | Value::Int(_) => value,
        Value::Bool(b) => Value::Int(i64::from(b).into()),
        Value::Float(f) if f.is_finite() => Value::Int(
            format!("{:.0}", f.trunc())
                .parse()
                .map_err(|_| AppError::new("internal_error", "cannot convert float to integer"))?,
        ),
        Value::Float(f) => {
            return Err(AppError::new(
                "internal_error",
                if f.is_nan() {
                    "cannot convert float NaN to integer"
                } else {
                    "cannot convert float infinity to integer"
                },
            ))
        }
        Value::Str(raw) => {
            let parsed = raw
                .as_str()
                .and_then(|t| kassiber_pytext::parse_int(t).ok())
                .ok_or_else(|| {
                    AppError::new(
                        "internal_error",
                        format!(
                            "invalid literal for int() with base 10: {}",
                            kassiber_pytext::repr_py(&raw)
                        ),
                    )
                })?;
            Value::Int(parsed)
        }
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kassiber_envelope::type_name(&other)
            ),
            ))
        }
    })
}

/// `reference_candidate(row)`: a compact description of one candidate row.
pub fn reference_candidate(row: &Value) -> Result<Value> {
    Ok(dict([
        ("transaction_id", field(row, "id")),
        ("external_id", field(row, "external_id")),
        ("wallet_id", field(row, "wallet_id")),
        ("wallet", field(row, "lookup_wallet_label")),
        ("direction", field(row, "direction")),
        ("asset", field(row, "asset")),
        ("amount_msat", python_int(field(row, "amount"))?),
        ("fee_msat", python_int(field(row, "fee"))?),
        ("occurred_at", field(row, "occurred_at")),
    ]))
}

fn candidates(rows: &[Value]) -> Result<Vec<Value>> {
    rows.iter()
        .take(MAX_REFERENCE_CANDIDATES)
        .map(reference_candidate)
        .collect()
}

/// `ambiguous_reference_error(reference, rows, message=...)`.
pub fn ambiguous_reference_error(
    reference: &str,
    rows: &[Value],
    message: Option<String>,
) -> Result<AppError> {
    let listed = candidates(rows)?;
    let ids = listed
        .iter()
        .map(|item| field(item, "transaction_id"))
        .collect();
    let remediation = build_remediation(
        "retry_with_full_transaction_id",
        Remediation {
            target: Some(dict([("transaction_ids", Value::List(ids))])),
            arguments_hint: Some(dict([
                ("transaction_id", s("<one candidate's transaction_id>")),
                (
                    "qualified_reference",
                    s(format!("{reference}:<wallet id or label>:<in|out>")),
                ),
            ])),
            message: Some(
                "Retry the same call with one candidate's full transaction_id. Do not choose between candidates yourself when the user's intent does not single one out; ask.",
            ),
            ..Remediation::default()
        },
    );
    let truncated = rows.len() > listed.len();
    Ok(AppError::new(
        "ambiguous_reference",
        message.unwrap_or_else(|| {
            format!(
                "Transaction reference '{reference}' matches {} transactions",
                rows.len()
            )
        }),
    )
    .with_hint(AMBIGUOUS_HINT)
    .with_details(dict([
        ("reference", s(reference)),
        ("candidate_count", Value::Int(rows.len().into())),
        ("candidates", Value::List(listed)),
        ("candidates_truncated", Value::Bool(truncated)),
        ("remediation", remediation),
    ])))
}

/// `str(reference or "").strip()`, kept as Python code points.
fn reference_text(reference: &Value) -> PyStr {
    let raw = match reference {
        _ if !truthy(reference) => PyStr::new(),
        Value::Str(text) => text.clone(),
        other => PyStr::from(py_str(other)),
    };
    kassiber_pytext::strip_py(&raw)
}

/// Binding a `str` with surrogates raises `UnicodeEncodeError`.
fn bindable(text: &PyStr) -> Result<&str> {
    text.as_str().ok_or_else(|| {
        AppError::new(
            "internal_error",
            kassiber_pytext::utf8_error(text).unwrap_or_default(),
        )
    })
}

fn filter_direction(rows: Vec<Value>, direction: Option<&str>) -> Vec<Value> {
    match direction {
        None => rows,
        Some(direction) => rows
            .into_iter()
            .filter(|row| matches!(get(row, "direction"), Some(Value::Str(d)) if d.as_str() == Some(direction)))
            .collect(),
    }
}

fn external_rows(conn: &Connection, profile_id: &str, external_id: &str) -> Result<Vec<Value>> {
    rows(conn, CANDIDATE_SQL, params![profile_id, external_id])
}

/// `_apply_qualifier(rows, qualifier)`.
fn apply_qualifier(rows: &[Value], qualifier: &str) -> Vec<Value> {
    let (mut direction, mut wallet) = (None, qualifier);
    if let Some(found) = direction_qualifier(qualifier) {
        (direction, wallet) = (Some(found), "");
    } else if let Some((head, tail)) = qualifier.rsplit_once(':') {
        if let Some(found) = direction_qualifier(tail) {
            (direction, wallet) = (Some(found), head);
        }
    }
    let wallet = kassiber_pytext::strip(wallet);
    let selected = filter_direction(rows.to_vec(), direction);
    if wallet.is_empty() {
        return selected;
    }
    let wanted = kassiber_pytext::casefold(wallet);
    selected
        .into_iter()
        .filter(|row| {
            matches!(get(row, "wallet_id"), Some(Value::Str(id)) if id.as_str() == Some(wallet))
                || kassiber_pytext::casefold(&match get(row, "lookup_wallet_label") {
                    Some(label) if truthy(label) => py_str(label),
                    _ => String::new(),
                }) == wanted
        })
        .collect()
}

/// The plain `transactions` row, without the joined lookup label.
fn plain_row(conn: &Connection, row: &Value) -> Result<Value> {
    rows(
        conn,
        "SELECT * FROM transactions WHERE id = ?",
        [text(row, "id")],
    )?
    .into_iter()
    .next()
    .ok_or_else(|| AppError::new("internal_error", "resolved transaction disappeared"))
}

/// `find_transaction(conn, profile_id, reference, direction=...)`: one
/// `transactions` row, or `None` when nothing matches. Several rows raise
/// `ambiguous_reference` instead of being picked between.
pub fn find_transaction(
    conn: &Connection,
    profile_id: &str,
    reference: &Value,
    direction: Option<&str>,
) -> Result<Option<Value>> {
    let text = reference_text(reference);
    if text.code_points().next().is_none() {
        return Ok(None);
    }
    let text = bindable(&text)?;
    let exact = match direction {
        None => rows(
            conn,
            "SELECT * FROM transactions WHERE profile_id = ? AND id = ?",
            params![profile_id, text],
        )?,
        Some(direction) => rows(
            conn,
            "SELECT * FROM transactions WHERE profile_id = ? AND id = ? AND direction = ?",
            params![profile_id, text, direction],
        )?,
    };
    if let Some(row) = exact.into_iter().next() {
        return Ok(Some(row));
    }
    let found = filter_direction(external_rows(conn, profile_id, text)?, direction);
    if found.len() == 1 {
        return plain_row(conn, &found[0]).map(Some);
    }
    if found.len() > 1 {
        let direction_hint = direction.map(|d| format!(" {d}")).unwrap_or_default();
        return Err(ambiguous_reference_error(
            text,
            &found,
            Some(format!(
                "Transaction external_id '{text}' matches multiple{direction_hint} transactions"
            )),
        )?);
    }
    // Only a reference with no exact match may be read as qualified, so an
    // external id that itself contains ':' keeps its exact meaning.
    for (index, _) in text.match_indices(':') {
        let base = kassiber_pytext::strip(&text[..index]);
        let qualifier = kassiber_pytext::strip(&text[index + 1..]);
        if base.is_empty() || qualifier.is_empty() {
            continue;
        }
        let shared = filter_direction(external_rows(conn, profile_id, base)?, direction);
        if shared.is_empty() {
            continue;
        }
        let selected = apply_qualifier(&shared, qualifier);
        if selected.len() == 1 {
            return plain_row(conn, &selected[0]).map(Some);
        }
        if selected.len() > 1 {
            return Err(ambiguous_reference_error(text, &selected, None)?);
        }
        return Err(AppError::new(
            "reference_qualifier_unmatched",
            format!(
                "Qualifier '{qualifier}' matches none of the {} transactions with external_id '{base}'",
                shared.len()
            ),
        )
        .with_details(dict([
            ("reference", s(text)),
            ("candidate_count", Value::Int(shared.len().into())),
            ("candidates", Value::List(candidates(&shared)?)),
        ])));
    }
    Ok(None)
}

/// `handlers.resolve_transaction(conn, profile_id, ref, direction)`: the
/// row, or "not found" naming the reference as given.
pub fn resolve_transaction(
    conn: &Connection,
    profile_id: &str,
    reference: &Value,
    direction: Option<&str>,
) -> Result<Value> {
    find_transaction(conn, profile_id, reference, direction)?.ok_or_else(|| {
        let shown = py_str(reference);
        AppError::new(
            "app_error",
            match direction {
                None => format!("Transaction '{shown}' not found"),
                Some(direction) => {
                    let mut chars = direction.chars();
                    let capitalized: String = chars
                        .next()
                        .map(|c| {
                            c.to_uppercase()
                                .chain(chars.flat_map(char::to_lowercase))
                                .collect()
                        })
                        .unwrap_or_default();
                    format!("{capitalized} transaction '{shown}' not found")
                }
            },
        )
    })
}

/// [`resolve_transaction`] for a reference that is already text.
pub fn resolve_transaction_str(
    conn: &Connection,
    profile_id: &str,
    reference: &str,
    direction: Option<&str>,
) -> Result<Value> {
    resolve_transaction(conn, profile_id, &s(reference), direction)
}

/// The row's `id`.
pub fn row_id(row: &Value) -> String {
    text(row, "id")
}
