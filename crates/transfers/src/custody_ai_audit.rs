//! `core/custody_ai_audit.py`: append-only audit rows for AI-assisted custody
//! decisions, inside the book's SQLCipher boundary. Raw proposals stay local
//! and bounded to the closed custody proposal vocabulary; callers own the
//! surrounding transaction, so a successful consent record commits atomically
//! with the reviewed custody write.
use crate::sql::{Result, Sources};
use bitcoin_hashes::{sha256, Hash};
use kassiber_envelope::{dict, get, py_str, AppError};
use kassiber_pyjson::{dumps, Key, Options, PyStr, Value};
use kassiber_store::rusqlite::{params, Connection};

/// `_MAX_REASON_CHARS`.
const MAX_REASON_CHARS: usize = 500;
/// `_PROPOSAL_KEYS`, sorted as Python iterates them.
const PROPOSAL_KEYS: [&str; 7] = [
    "classification",
    "economic_kind",
    "expected_input_version",
    "gap_id",
    "notes",
    "reason",
    "residual_action",
];
/// `_CONSENT_DECISIONS`. `mode_auto` / `mode_full_access`: the chat's
/// permission mode approved the call without asking; `mcp_host`: an external
/// agent's MCP host approved it (#684, #686).
const CONSENT_DECISIONS: [&str; 8] = [
    "allow_once",
    "allow_session",
    "mode_auto",
    "mode_full_access",
    "mcp_host",
    "deny",
    "consent_timeout",
    "cancelled",
];
/// `_EXECUTION_STATUSES`.
const EXECUTION_STATUSES: [&str; 4] = ["executed", "failed", "denied", "cancelled"];
/// `_bounded_result`'s keys, in Python's order.
const RESULT_KEYS: [&str; 8] = [
    "gap_id",
    "status",
    "id",
    "review_id",
    "review_revision",
    "component_id",
    "classification",
    "economic_kind",
];

/// `_canonical_json(value)`.
fn canonical(value: &Value) -> Result<String> {
    dumps(value, &Options::new().sort_keys(true).compact())
        .map_err(|error| AppError::new("internal_error", error.to_string()))
}

/// `_sha256_json(value)`.
fn sha256_json(value: &Value) -> Result<String> {
    Ok(sha256::Hash::hash(canonical(value)?.as_bytes()).to_string())
}

/// `text[:limit]` in code points.
fn head(text: &PyStr, limit: usize) -> PyStr {
    PyStr::from_code_points(text.code_points().take(limit))
        .expect("code points of an existing string")
}

/// `str(value)[:limit]` as the row stores it.
fn bounded_text(value: &Value, limit: usize) -> Value {
    let text = match value {
        Value::Str(text) => text.clone(),
        other => PyStr::from(py_str(other)),
    };
    Value::Str(head(&text, limit))
}

/// `_bounded_proposal(raw)`: the closed custody vocabulary and bounded scalars.
fn bounded_proposal(raw: &Value) -> Value {
    Value::Dict(
        PROPOSAL_KEYS
            .iter()
            .filter_map(|key| {
                let value = match get(raw, key)? {
                    Value::Null => return None,
                    value @ (Value::Bool(_) | Value::Int(_)) => value.clone(),
                    Value::Str(text) => Value::Str(head(text, MAX_REASON_CHARS)),
                    _ => return None,
                };
                Some((Key::from(*key), value))
            })
            .collect(),
    )
}

/// `_bounded_result(result)`.
fn bounded_result(result: Option<&Value>) -> Value {
    let Some(result @ Value::Dict(_)) = result else {
        return Value::Dict(Vec::new());
    };
    let mut packet: Vec<(Key, Value)> = RESULT_KEYS
        .iter()
        .filter_map(|key| match get(result, key)? {
            value @ (Value::Str(_) | Value::Int(_)) => Some((Key::from(*key), value.clone())),
            _ => None,
        })
        .collect();
    if !packet.iter().any(|(key, _)| *key == Key::from("review_id")) {
        if let Some(id @ Value::Str(_)) = get(result, "id") {
            packet.push((Key::from("review_id"), id.clone()));
        }
    }
    Value::Dict(packet)
}

fn text_field(record: &Value, key: &str) -> Result<PyStr> {
    match get(record, key) {
        Some(Value::Str(text)) => Ok(text.clone()),
        Some(Value::Null) | None => Ok(PyStr::new()),
        Some(other) => Ok(PyStr::from(py_str(other))),
    }
}

fn sql_text(value: &Value) -> Result<Option<String>> {
    match value {
        Value::Null => Ok(None),
        Value::Str(text) => text
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| {
                AppError::new(
                    "internal_error",
                    "'utf-8' codec can't encode character: surrogates not allowed",
                )
            }),
        other => Ok(Some(py_str(other))),
    }
}

/// `append_assistance_record(conn, **record, result=result)` without
/// committing. `record` carries the keyword arguments by name.
pub fn append(
    conn: &Connection,
    record: &Value,
    result: Option<&Value>,
    sources: &dyn Sources,
) -> Result<Value> {
    let decision = text_field(record, "consent_decision")?;
    if !CONSENT_DECISIONS.iter().any(|known| decision.is(known)) {
        return Err(AppError::new(
            "internal_error",
            "unsupported custody AI consent decision",
        ));
    }
    let status = text_field(record, "execution_status")?;
    if !EXECUTION_STATUSES.iter().any(|known| status.is(known)) {
        return Err(AppError::new(
            "internal_error",
            "unsupported custody AI execution status",
        ));
    }
    let model_proposal = get(record, "model_proposal")
        .cloned()
        .unwrap_or(Value::Null);
    let model_packet = bounded_proposal(&model_proposal);
    let final_packet = match get(record, "final_proposal") {
        None | Some(Value::Null) => bounded_proposal(&model_proposal),
        Some(proposal) => bounded_proposal(proposal),
    };
    let user_edited = canonical(&final_packet)? != canonical(&model_packet)?;
    let gap_id = get(&final_packet, "gap_id").cloned().unwrap_or(Value::Null);
    let input_version = get(&final_packet, "expected_input_version")
        .cloned()
        .unwrap_or(Value::Null);
    let daemon_kind = get(record, "daemon_kind").cloned().unwrap_or(Value::Null);
    let facts = dict([
        ("daemon_kind", daemon_kind.clone()),
        ("gap_id", gap_id.clone()),
        ("candidate_fingerprint", Value::Null),
        ("input_version", input_version),
        ("proposal", final_packet.clone()),
    ]);
    let result_packet = bounded_result(result);
    let record_id = sources.uuid(conn)?;
    let created_at = sources.now(conn)?;
    let provider = match get(record, "provider_kind") {
        Some(kind) if kassiber_envelope::truthy(kind) => kind.clone(),
        _ => Value::Str("unknown".into()),
    };
    let execution_code = match get(record, "execution_code") {
        Some(code) if kassiber_envelope::truthy(code) => bounded_text(code, 128),
        _ => Value::Null,
    };
    let facts_sha256 = sha256_json(&facts)?;
    let result_sha256 = match result {
        Some(_) => Value::Str(sha256_json(&result_packet)?.into()),
        None => Value::Null,
    };
    let review_id = get(&result_packet, "review_id")
        .cloned()
        .unwrap_or(Value::Null);
    let component_id = get(&result_packet, "component_id")
        .cloned()
        .unwrap_or(Value::Null);
    let field = |key: &str| get(record, key).cloned().unwrap_or(Value::Null);
    let bound = |value: Value| -> Result<kassiber_store::rusqlite::types::Value> {
        use kassiber_store::rusqlite::types::Value as Sql;
        Ok(match value {
            Value::Int(n) => Sql::Integer(n.to_string().parse().map_err(|_| {
                AppError::new(
                    "internal_error",
                    "Python int too large to convert to SQLite INTEGER",
                )
            })?),
            Value::Bool(b) => Sql::Integer(i64::from(b)),
            other => sql_text(&other)?.map_or(Sql::Null, Sql::Text),
        })
    };
    conn.execute(
        "INSERT INTO custody_ai_assistance_audits(
            id, workspace_id, profile_id, tool_name, daemon_kind, call_id,
            provider_kind, model, gap_id, candidate_fingerprint,
            facts_sha256, model_proposal_json, final_proposal_json, user_edited,
            consent_decision, consent_requested_at, consent_decided_at,
            execution_status, execution_code, result_sha256, review_id,
            component_id, created_at
        ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        params![
            record_id,
            bound(field("workspace_id"))?,
            bound(field("profile_id"))?,
            bound(bounded_text(&field("tool_name"), 128))?,
            bound(bounded_text(&daemon_kind, 128))?,
            bound(bounded_text(&field("call_id"), 256))?,
            bound(bounded_text(&provider, 64))?,
            bound(bounded_text(&field("model"), 256))?,
            bound(if matches!(gap_id, Value::Null) {
                Value::Null
            } else {
                bounded_text(&gap_id, 256)
            })?,
            Option::<String>::None,
            facts_sha256,
            canonical(&model_packet)?,
            canonical(&final_packet)?,
            i64::from(user_edited),
            bound(Value::Str(decision.clone()))?,
            bound(field("consent_requested_at"))?,
            bound(field("consent_decided_at"))?,
            bound(Value::Str(status.clone()))?,
            bound(execution_code)?,
            bound(result_sha256)?,
            bound(review_id.clone())?,
            bound(component_id.clone())?,
            created_at,
        ],
    )
    .map_err(crate::sql::error)?;
    Ok(dict([
        ("id", Value::Str(record_id.as_str().into())),
        ("facts_sha256", Value::Str(sha256_json(&facts)?.into())),
        ("consent_decision", Value::Str(decision)),
        ("execution_status", Value::Str(status)),
        ("review_id", review_id),
        ("component_id", component_id),
        ("created_at", Value::Str(created_at.as_str().into())),
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::s;

    #[test]
    fn proposals_keep_only_the_custody_vocabulary_in_sorted_order() {
        let raw = dict([
            ("reason", s("x".repeat(600))),
            ("gap_id", s("gap-1")),
            ("expected_input_version", Value::from(3)),
            ("activate", Value::Bool(true)),
            ("notes", Value::Null),
            ("classification", Value::Bool(false)),
            ("residual_action", Value::List(vec![])),
        ]);
        let packet = bounded_proposal(&raw);
        assert_eq!(
            canonical(&packet).unwrap(),
            format!(
                "{{\"classification\":false,\"expected_input_version\":3,\"gap_id\":\"gap-1\",\"reason\":\"{}\"}}",
                "x".repeat(500)
            )
        );
        let Value::Dict(pairs) = packet else {
            unreachable!()
        };
        let keys: Vec<_> = pairs.iter().map(|(key, _)| format!("{key:?}")).collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
    }

    #[test]
    fn results_keep_identifiers_and_fall_back_to_id_for_the_review() {
        let result = dict([
            ("status", s("applied")),
            ("id", s("review-1")),
            ("review_revision", Value::from(2)),
            ("component_id", Value::Bool(true)),
            ("ignored", s("x")),
        ]);
        assert_eq!(
            canonical(&bounded_result(Some(&result))).unwrap(),
            "{\"id\":\"review-1\",\"review_id\":\"review-1\",\"review_revision\":2,\"status\":\"applied\"}"
        );
        assert_eq!(canonical(&bounded_result(Some(&s("x")))).unwrap(), "{}");
    }

    #[test]
    fn the_audit_admits_pythons_consent_decisions() {
        // kassiber/core/custody_ai_audit.py `_CONSENT_DECISIONS`.
        let python = [
            "allow_once",
            "allow_session",
            "mode_auto",
            "mode_full_access",
            "mcp_host",
            "deny",
            "consent_timeout",
            "cancelled",
        ];
        let mut have = CONSENT_DECISIONS.to_vec();
        let mut want = python.to_vec();
        have.sort_unstable();
        want.sort_unstable();
        assert_eq!(have, want);
    }
}
