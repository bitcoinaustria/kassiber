//! Current review cases, independent of stored (possibly stale) journals.
//!
//! The caller supplies the canonical build on this same connection. This does
//! not register a product route or admit the unfinished journal loader.
use super::*;
use kassiber_envelope::{s, set, truthy};
use kassiber_pyjson::{dumps, Options, PyStr};
use std::collections::{BTreeMap, BTreeSet};

fn validation(message: &str) -> AppError {
    AppError::new("validation", message)
}
fn int_or_zero(value: &Value) -> Result<kassiber_pyjson::BigInt> {
    integer(if truthy(value) {
        value
    } else {
        &Value::Bool(false)
    })
}
fn equal(a: &Value, b: &Value) -> bool {
    let number = |value: &Value| match value {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(n) => Some(u8::from(*n).into()),
        Value::Float(n) if n.is_finite() && n.fract() == 0.0 => {
            format!("{n:.0}").parse::<kassiber_pyjson::BigInt>().ok()
        }
        _ => None,
    };
    match (a, b) {
        (Value::List(a), Value::List(b)) => {
            a.len() == b.len() && a.iter().zip(b).all(|(a, b)| equal(a, b))
        }
        _ => match (number(a), number(b)) {
            (Some(a), Some(b)) => a == b,
            _ => a == b,
        },
    }
}
fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(values) => values,
        _ => &[],
    }
}
fn detail(value: &Value) -> Value {
    match value {
        Value::Dict(_) => value.clone(),
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::loads(text).ok())
            .filter(|value| matches!(value, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([])),
        _ => dict::<&str>([]),
    }
}
fn json(value: &Value, compact: bool) -> Result<String> {
    dumps(
        value,
        &Options {
            sort_keys: true,
            separators: compact.then(|| (",".into(), ":".into())),
            ..Options::default()
        },
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))
}

const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
fn encode(raw: &[u8]) -> String {
    let mut out = String::new();
    for chunk in raw.chunks(3) {
        let bits = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for index in 0..4 {
            out.push(if index > chunk.len() {
                '='
            } else {
                ALPHABET[((bits >> (18 - index * 6)) & 63) as usize] as char
            });
        }
    }
    out
}
// CPython's non-strict urlsafe decoder ignores punctuation, accepts either
// alphabet and stops at completed padding. Unlike gap cursors it adds no '='.
fn decode(raw: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut bits, mut count, mut padding) = (0u32, 0, 0);
    for byte in raw.bytes() {
        if byte == b'=' {
            if count >= 2 {
                padding += 1;
                if count + padding >= 4 {
                    return Some(out);
                }
            }
            continue;
        }
        let byte = match byte {
            b'+' => b'-',
            b'/' => b'_',
            byte => byte,
        };
        let Some(index) = ALPHABET.iter().position(|candidate| *candidate == byte) else {
            continue;
        };
        padding = 0;
        bits = (bits << 6) | index as u32;
        count += 1;
        if count >= 2 {
            out.push((bits >> (6 * count - 8 * (count - 1))) as u8);
        }
        if count == 4 {
            count = 0;
            bits = 0;
        }
    }
    (count == 0).then_some(out)
}
fn after(cursor: &Value, scope: &Value) -> Result<PyStr> {
    let malformed = || validation("Review cursor is malformed");
    let payload = match cursor {
        Value::Str(raw) => raw.as_str().and_then(decode),
        _ => None,
    }
    .and_then(|bytes| String::from_utf8(bytes).ok())
    .and_then(|text| kassiber_pytext::loads(&text).ok())
    .ok_or_else(malformed)?;
    let version = get(&payload, "scope").ok_or_else(malformed)?;
    if !equal(version, scope) {
        return Err(AppError::new(
            "review_cursor_stale",
            "Review cursor expired; inspect current evidence again",
        ));
    }
    match get(&payload, "after") {
        Some(Value::Str(after)) => Ok(after.clone()),
        _ => Err(malformed()),
    }
}

fn downstream(reason: &str) -> bool {
    matches!(
        reason,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "derived_transfer_group_blocked"
            | "basis_provenance_incomplete"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}
type Rows = BTreeMap<String, Value>;
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    Text(PyStr),
    Integer(kassiber_pyjson::BigInt),
}
fn group_key(value: &Value) -> Option<GroupKey> {
    if !truthy(value) {
        return None;
    }
    match value {
        Value::Str(value) => Some(GroupKey::Text(value.clone())),
        Value::Int(value) => Some(GroupKey::Integer(value.clone())),
        Value::Bool(value) => Some(GroupKey::Integer(u8::from(*value).into())),
        _ => None,
    }
}
type Groups = BTreeMap<GroupKey, Vec<String>>;
type RootTimes = BTreeMap<(PyStr, PyStr), Vec<String>>;

/// Only a named, unambiguous root removes a downstream row from attention.
pub fn waiting_roots(rows: &Rows, details: &Rows) -> BTreeMap<String, String> {
    let mut groups = Groups::new();
    for (id, detail) in details {
        if let Some(group) = group_key(&value(detail, "transfer_group_id")) {
            groups.entry(group).or_default().push(id.clone());
        }
    }
    let mut times = RootTimes::new();
    for (id, row) in rows {
        if !downstream(&text(row, "reason")) {
            if let (Value::Str(at), Value::Str(asset)) =
                (value(row, "occurred_at"), value(row, "asset"))
            {
                times.entry((at, asset)).or_default().push(id.clone());
            }
        }
    }
    fn resolve(
        id: &str,
        rows: &Rows,
        details: &Rows,
        groups: &Groups,
        times: &RootTimes,
        visited: &BTreeSet<String>,
    ) -> Option<String> {
        let mut visited = visited.clone();
        visited.insert(id.into());
        let row = rows.get(id)?;
        let detail = details.get(id).cloned().unwrap_or(Value::Null);
        let reason = text(row, "reason");
        let root = |id: &str| {
            rows.get(id)
                .is_some_and(|row| !downstream(&text(row, "reason")))
        };
        let candidates: Vec<String> = match reason.as_str() {
            "custody_basis_barrier" | "transfer_pair_dependency_blocked" => list(&value(
                &detail,
                if reason == "custody_basis_barrier" {
                    "root_transaction_ids"
                } else {
                    "blocked_by_transaction_ids"
                },
            ))
            .iter()
            .map(py_str)
            .collect(),
            "derived_transfer_group_blocked" => group_key(&value(&detail, "transfer_group_id"))
                .and_then(|key| groups.get(&key).cloned())
                .unwrap_or_default(),
            "basis_provenance_incomplete" => {
                let since = value(&detail, "lot_state_uncertain_since");
                let matches =
                    if let (Value::Str(since), Value::Str(asset)) = (&since, value(row, "asset")) {
                        if since.is("") {
                            vec![]
                        } else {
                            times
                                .get(&(since.clone(), asset))
                                .cloned()
                                .unwrap_or_default()
                        }
                    } else {
                        rows.iter()
                            .filter(|(id, other)| {
                                root(id)
                                    && truthy(&since)
                                    && matches!(&since, Value::Str(_))
                                    && value(other, "occurred_at") == since
                                    && value(other, "asset") == value(row, "asset")
                            })
                            .map(|(id, _)| id.clone())
                            .collect()
                    };
                if matches.len() == 1 {
                    matches
                } else {
                    vec![]
                }
            }
            _ => vec![],
        };
        let mut roots: Vec<_> = candidates
            .iter()
            .filter(|id| !visited.contains(*id) && root(id))
            .cloned()
            .collect();
        if roots.is_empty() {
            for candidate in candidates {
                if !visited.contains(&candidate) {
                    if let Some(nested) =
                        resolve(&candidate, rows, details, groups, times, &visited)
                    {
                        if !visited.contains(&nested) {
                            roots.push(nested);
                        }
                    }
                }
            }
        }
        roots.into_iter().min_by_key(|id| {
            let occurred = value(&rows[id], "occurred_at");
            (
                if truthy(&occurred) {
                    py_str(&occurred)
                } else {
                    String::new()
                },
                id.clone(),
            )
        })
    }
    rows.iter()
        .filter(|(_, row)| downstream(&text(row, "reason")))
        .filter_map(|(id, _)| {
            resolve(id, rows, details, &groups, &times, &BTreeSet::new())
                .map(|root| (id.clone(), root))
        })
        .collect()
}

fn pair_records(conn: &Connection, scope: &Scope<'_>) -> Result<Rows> {
    let mut result = Rows::new();
    for pair in current_pairs(conn, scope)? {
        for side in ["out", "in"] {
            let id = text(&pair, &format!("{side}_transaction_id"));
            if !id.is_empty() {
                result.entry(id).or_insert_with(|| pair.clone());
            }
        }
    }
    Ok(result)
}

pub fn current_pairs(conn: &Connection, scope: &Scope<'_>) -> Result<Vec<Value>> {
    let mut result = Vec::new();
    for mut pair in list(&records::pairs(conn, scope)?).to_vec() {
        for side in ["out", "in"] {
            let leg = value(&pair, side);
            for field in [
                "transaction_id",
                "asset",
                "full_amount_msat",
                "amount_msat",
                "external_id",
                "occurred_at",
                "wallet",
            ] {
                set(&mut pair, &format!("{side}_{field}"), value(&leg, field));
            }
        }
        result.push(pair);
    }
    Ok(result)
}

pub use crate::review_reading::pair_fingerprint;

pub fn pair_evidence(conn: &Connection, transaction_id: &str, pair: &Value) -> Result<Value> {
    let mut legs = dict::<&str>([]);
    for side in ["out", "in"] {
        let mut leg = dict::<&str>([]);
        for field in [
            "transaction_id",
            "wallet",
            "asset",
            "amount_msat",
            "occurred_at",
            "external_id",
        ] {
            let raw = value(pair, &format!("{side}_{field}"));
            set(
                &mut leg,
                field,
                if field == "amount_msat" {
                    Value::Int(int_or_zero(&value(
                        pair,
                        &format!("{side}_full_amount_msat"),
                    ))?)
                } else if field == "occurred_at" {
                    raw
                } else if truthy(&raw) {
                    s(py_str(&raw))
                } else {
                    s("")
                },
            );
        }
        set(&mut legs, side, leg);
    }
    let mut evidence = dict([
        ("pair_id", s(text(pair, "id"))),
        ("pair_fingerprint", s(pair_fingerprint(conn, pair)?)),
        (
            "pair_counterpart_transaction_id",
            value(
                pair,
                if transaction_id == text(pair, "out_transaction_id") {
                    "in_transaction_id"
                } else {
                    "out_transaction_id"
                },
            ),
        ),
        ("pair_legs", legs),
        (
            "pair_review",
            dict([
                ("kind", value(pair, "kind")),
                ("policy", value(pair, "policy")),
                (
                    "out_amount_msat",
                    Value::Int(int_or_zero(&value(pair, "out_amount_msat"))?),
                ),
                (
                    "in_amount_msat",
                    Value::Int(int_or_zero(&value(pair, "in_amount_msat"))?),
                ),
            ]),
        ),
    ]);
    let canonical = |field| {
        let raw = text(pair, field);
        let normalized = kassiber_pytext::lower(kassiber_pytext::strip(&raw));
        (normalized.len() == 64 && normalized.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .then_some(normalized)
    };
    if let (Some(out), Some(inbound)) = (canonical("out_external_id"), canonical("in_external_id"))
    {
        if kassiber_pytext::upper(&text(pair, "out_asset"))
            == kassiber_pytext::upper(&text(pair, "in_asset"))
        {
            set(
                &mut evidence,
                "pair_txids_differ",
                Value::Bool(out != inbound),
            );
        }
    }
    let (out, inbound) = (text(pair, "out_occurred_at"), text(pair, "in_occurred_at"));
    if !out.is_empty() && !inbound.is_empty() {
        set(
            &mut evidence,
            "pair_receipt_before_spend",
            Value::Bool(inbound < out),
        );
    }
    Ok(evidence)
}

fn freshness(conn: &Connection, profile: &Value) -> Result<Value> {
    let active: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
            [text(profile, "id")],
            |row| row.get(0),
        )
        .map_err(sql::error)?;
    let count = int_or_zero(&value(profile, "last_processed_tx_count"))?;
    let input = int_or_zero(&value(profile, "journal_input_version"))?;
    let processed = int_or_zero(&value(profile, "last_processed_input_version"))?;
    let at = value(profile, "last_processed_at");
    // A profile without the semantics columns can never be current.
    let semantics = match (
        get(profile, "required_journal_semantics_version"),
        get(profile, "journal_semantics_version"),
    ) {
        (Some(required), Some(built)) => {
            let required = int_or_zero(required)?;
            let built = int_or_zero(built)?;
            let version = kassiber_pyjson::BigInt::from(
                kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION,
            );
            required == 0.into() || (built == version && built >= required)
        }
        _ => false,
    };
    let (status, reason) = if active == 0 {
        ("no_transactions", "no active transactions")
    } else if !truthy(&at) {
        ("not_processed", "journals have not been processed")
    } else if count != active.into() {
        (
            "stale",
            "active transaction count changed since last processing",
        )
    } else if input != processed {
        ("stale", "journal inputs changed since last processing")
    } else if !semantics {
        (
            "stale",
            "journal calculation semantics changed; rebuild with current Kassiber",
        )
    } else {
        (
            "current",
            "journals match the active transaction count, input version and calculation semantics",
        )
    };
    Ok(dict([
        ("status", s(status)),
        (
            "is_current",
            Value::Bool(truthy(&at) && count == active.into() && input == processed && semantics),
        ),
        (
            "needs_processing",
            Value::Bool(matches!(status, "not_processed" | "stale")),
        ),
        ("reason", s(reason)),
        ("last_processed_at", at),
        ("last_processed_tx_count", Value::Int(count)),
        ("journal_input_version", Value::Int(input)),
        ("last_processed_input_version", Value::Int(processed)),
        ("active_transaction_count", active.into()),
    ]))
}

/// Build and page inside one read snapshot; preserve a caller-owned transaction.
/// `build` must return the canonical deduplicated, live-anchor quarantines.
pub fn inspect(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    build: impl FnOnce(&Connection, &Value) -> Result<Vec<Value>>,
) -> Result<Value> {
    inspect_with_error(conn, scope, args, build)
}

/// Retain the assembler's full refusal without flattening it into AppError.
pub fn inspect_with_error<E: From<AppError>>(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    build: impl FnOnce(&Connection, &Value) -> std::result::Result<Vec<Value>, E>,
) -> std::result::Result<Value, E> {
    // Transaction's Drop also rolls back an owned read when a build unwinds.
    let read = if conn.is_autocommit() {
        Some(conn.unchecked_transaction().map_err(sql::error)?)
    } else {
        None
    };
    let result = (|| {
        let limit = get(args, "limit").cloned().unwrap_or(20.into());
        if !matches!(&limit,Value::Int(n) if n>=&1.into() && n<=&100.into()) {
            return Err(validation("Review limit must be between 1 and 100").into());
        }
        let limit = usize::try_from(integer(&limit)?).expect("bounded limit");
        let profile = rows(
            conn,
            "SELECT * FROM profiles WHERE id=? AND workspace_id=?",
            [scope.profile_id, scope.workspace_id],
        )?
        .into_iter()
        .next()
        .ok_or_else(|| AppError::new("not_found", "Review profile was not found"))?;
        let version = int_or_zero(&value(&profile, "journal_input_version"))?;
        let cursor_scope = Value::List(vec![
            value(&profile, "id"),
            Value::Int(version.clone()),
            value(&profile, "last_processed_at"),
        ]);
        let cursor = get(args, "cursor").filter(|value| !matches!(value, Value::Null));
        let after = cursor
            .map(|cursor| after(cursor, &cursor_scope))
            .transpose()?
            .unwrap_or_default();
        let quarantines = build(conn, &profile)?;
        let reasons: Rows = quarantines
            .iter()
            .map(|row| (text(row, "transaction_id"), value(row, "reason")))
            .collect();
        let details: Rows = quarantines
            .iter()
            .map(|row| {
                (
                    text(row, "transaction_id"),
                    detail(&value(row, "detail_json")),
                )
            })
            .collect();
        let pairs = pair_records(conn, scope)?;
        let transactions = rows(
            conn,
            "SELECT id AS transaction_id,direction,asset,occurred_at
             FROM transactions WHERE profile_id=?",
            [scope.profile_id],
        )?;
        let held: Rows = transactions
            .iter()
            .filter_map(|row| {
                let id = text(row, "transaction_id");
                reasons.get(&id).map(|reason| {
                    let mut row = row.clone();
                    set(&mut row, "reason", reason.clone());
                    (id, row)
                })
            })
            .collect();
        let waiting = waiting_roots(&held, &details);
        // Transactions whose own stored price is coarse.
        let coarse: BTreeSet<String> = rows(
            conn,
            "SELECT id FROM transactions WHERE profile_id=? AND pricing_quality=?",
            [scope.profile_id, "coarse_fallback"],
        )?
        .iter()
        .map(|row| text(row, "id"))
        .collect();
        let mut waiting_counts = BTreeMap::<String, usize>::new();
        for root in waiting.values() {
            *waiting_counts.entry(root.clone()).or_default() += 1;
        }
        let ids: Vec<_> = reasons
            .keys()
            .filter(|id| PyStr::from(id.as_str()) > after && !waiting.contains_key(*id))
            .collect();
        let mut cases = Vec::new();
        for id in ids.iter().take(limit) {
            let Some(mut case) = transactions
                .iter()
                .find(|row| text(row, "transaction_id") == id.as_str())
                .cloned()
            else {
                continue;
            };
            let reason = py_str(&reasons[*id]);
            set(&mut case, "reason", s(reason.as_str()));
            set(&mut case, "case_id", s(format!("quarantine:{id}")));
            set(
                &mut case,
                "missing_evidence",
                Value::List(vec![dict([
                    ("code", s(reason.as_str())),
                    ("status", s("unresolved")),
                ])]),
            );
            let mut operations = if reason == "pricing_review_required" && coarse.contains(*id) {
                vec![s("accept_coarse_prices"), s("price_override")]
            } else if reason.contains("price") {
                vec![s("price_override")]
            } else if ["custody", "privacy", "transfer", "swap"]
                .iter()
                .any(|word| reason.contains(word))
            {
                vec![s("custody_component")]
            } else {
                vec![]
            };
            set(
                &mut case,
                "supported_operations",
                Value::List(operations.clone()),
            );
            set(
                &mut case,
                "waiting_count",
                waiting_counts.get(*id).copied().unwrap_or(0).into(),
            );
            if text(&details[*id], "blocker_code") == "reviewed_residual_suspense" {
                if let Some(pair) = pairs.get(*id) {
                    set(&mut case, "pair", pair_evidence(conn, id, pair)?);
                    operations.insert(0, s("unpair"));
                }
            }
            if reason == "synthetic_import_duplicate_review_required" {
                let detail = &details[*id];
                set(
                    &mut case,
                    "import_duplicate",
                    dict(
                        [
                            "group_fingerprint",
                            "transaction_ids",
                            "active_count",
                            "largest_single_batch_count",
                            "hold_reasons",
                            "refilled_excluded_transaction_ids",
                        ]
                        .map(|key| (key, value(detail, key))),
                    ),
                );
                operations = vec![s("exclude"), s("confirm_distinct_import_rows")];
            }
            if let Some(pair) = pairs.get(*id) {
                let guidance = crate::pair_guidance::pair_review_guidance(conn, scope, pair)?;
                if let Some(repair) = crate::pair_guidance::repair(&guidance) {
                    set(
                        &mut case,
                        "missing_evidence",
                        Value::List(
                            list(&value(&guidance, "review_codes"))
                                .iter()
                                .map(|code| {
                                    dict([("code", code.clone()), ("status", s("unresolved"))])
                                })
                                .collect(),
                        ),
                    );
                    set(&mut case, "repair", repair.clone());
                    set(&mut case, "pair", pair_evidence(conn, id, pair)?);
                    if truthy(&value(repair, "preview_operation")) {
                        operations.push(s("repair_legacy_pair"));
                    }
                }
            }
            set(&mut case, "supported_operations", Value::List(operations));
            cases.push(case);
        }
        let next = if ids.len() > limit {
            s(encode(
                json(
                    &dict([
                        ("scope", cursor_scope),
                        ("after", s(ids[limit - 1].as_str())),
                    ]),
                    true,
                )?
                .as_bytes(),
            ))
        } else {
            Value::Null
        };
        let freshness = freshness(conn, &profile)?;
        let receipts = if cursor.is_none() {
            rows(
                conn,
                "SELECT receipt_json FROM review_workflow_receipts
                 WHERE profile_id=? ORDER BY created_at DESC,id DESC LIMIT 10",
                [scope.profile_id],
            )?
            .iter()
            .map(|row| {
                let receipt = parse_json(&text(row, "receipt_json"))?;
                Ok(project(
                    &receipt,
                    &[
                        "id",
                        "created_at",
                        "result_input_version",
                        "artifact_digest",
                        "status",
                    ],
                ))
            })
            .collect::<Result<Vec<_>>>()?
        } else {
            vec![]
        };
        Ok(dict([
            ("schema_version", 1.into()),
            ("workspace_id", value(&profile, "workspace_id")),
            ("profile_id", value(&profile, "id")),
            ("input_version", Value::Int(version)),
            // What price_override's fiat_rate and fiat_value are stated in.
            (
                "fiat_currency",
                s(kassiber_pytext::upper(kassiber_pytext::strip(
                    &if truthy(&value(&profile, "fiat_currency")) {
                        text(&profile, "fiat_currency")
                    } else {
                        String::new()
                    },
                ))),
            ),
            ("freshness", freshness),
            ("cases", Value::List(cases)),
            ("next_cursor", next),
            ("waiting_count", waiting.len().into()),
            ("recent_receipts", Value::List(receipts)),
        ]))
    })();
    if let Some(transaction) = read {
        transaction.rollback().map_err(sql::error)?;
    }
    result
}
