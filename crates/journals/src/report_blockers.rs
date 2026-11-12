//! `build_report_blockers_snapshot`: the report-readiness barrier over the
//! workspace health, rate coverage, freshness failures, custody quantity
//! issues, authored component integrity, source overlaps, journal state and
//! unreviewed transfer/swap candidates. Every input is stored; nothing here
//! processes journals, writes the book or opens a connection. A candidate or
//! overlap is a reason to review, never ownership or a booked transfer.
use std::collections::{BTreeMap, BTreeSet, HashSet};

use kassiber_envelope::{dict, get, py_str, s, set, truthy, AppError};
use kassiber_pyjson::{BigInt, Value};
use kassiber_store::rusqlite::{types::ValueRef, Connection, ErrorCode, Params};
use kassiber_transfers::pairing::{suggest, MatchOptions, Population, ProjectionGuards};
use kassiber_transfers::Scope;

type Result<T> = std::result::Result<T, AppError>;

const QUALIFICATION: &str = "This reports gaps detectable from current imported evidence; it does not assert that every wallet was imported.";
const BLOCKING_SWAP_KINDS: &[&str] = &[
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];

pub(crate) fn sql_error(error: kassiber_store::rusqlite::Error) -> AppError {
    let e = kassiber_store::map_sqlite_error(error);
    AppError::new(e.code, e.message).retryable(e.retryable)
}

/// `sqlite3.OperationalError`: the generic SQLite error family (missing
/// tables or columns, integer overflow), which a few reads tolerate.
pub(crate) fn operational(error: &kassiber_store::rusqlite::Error) -> bool {
    matches!(error, kassiber_store::rusqlite::Error::SqliteFailure(code, _) if code.code == ErrorCode::Unknown)
}

pub(crate) fn query(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> std::result::Result<Vec<Value>, kassiber_store::rusqlite::Error> {
    let mut statement = conn.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .iter()
        .map(|name| (*name).to_owned())
        .collect();
    let mut cursor = statement.query(params)?;
    let mut output = Vec::new();
    while let Some(row) = cursor.next()? {
        let mut fields = Vec::with_capacity(names.len());
        for (index, name) in names.iter().enumerate() {
            let value = match row.get_ref(index)? {
                ValueRef::Null => Value::Null,
                ValueRef::Integer(n) => Value::Int(n.into()),
                ValueRef::Real(n) => Value::Float(n),
                ValueRef::Text(bytes) => s(String::from_utf8_lossy(bytes).as_ref()),
                ValueRef::Blob(bytes) => Value::Bytes(bytes.to_vec()),
            };
            fields.push((name.as_str(), value));
        }
        output.push(dict(fields));
    }
    Ok(output)
}

pub(crate) fn rows(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>> {
    query(conn, sql, params).map_err(sql_error)
}

pub(crate) fn val(row: &Value, key: &str) -> Value {
    get(row, key).cloned().unwrap_or(Value::Null)
}

/// `str(value or default)`.
fn text_or(row: &Value, key: &str, default: &str) -> String {
    match get(row, key) {
        Some(value) if truthy(value) => py_str(value),
        _ => default.to_owned(),
    }
}

/// Python's `int(value or 0)` over a stored SQLite value.
pub(crate) fn int(value: &Value) -> Result<BigInt> {
    match value {
        Value::Null => Ok(BigInt::from(0)),
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(b) => Ok(BigInt::from(u8::from(*b))),
        Value::Float(n) if n.is_finite() => format!("{:.0}", n.trunc())
            .parse()
            .map_err(|_| AppError::new("internal_error", "cannot convert float to integer")),
        Value::Float(n) if n.is_nan() => Err(AppError::new(
            "ValueError",
            "cannot convert float NaN to integer",
        )),
        Value::Float(_) => Err(AppError::new(
            "internal_error",
            "cannot convert float infinity to integer",
        )),
        Value::Str(_) if !truthy(value) => Ok(BigInt::from(0)),
        Value::Str(text) => {
            kassiber_pytext::parse_int(text.as_str().unwrap_or_default()).map_err(|_| {
                AppError::new(
                    "internal_error",
                    format!(
                        "invalid literal for int() with base 10: {}",
                        kassiber_envelope::py_repr(value)
                    ),
                )
            })
        }
        other => Err(AppError::new(
            "internal_error",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                kassiber_envelope::type_name(other)
            ),
        )),
    }
}

fn count(value: &Value) -> i64 {
    match value {
        Value::Int(n) => i64::try_from(n.clone()).unwrap_or_default(),
        _ => 0,
    }
}

fn blocker(id: &str, severity: &str, title: &str, detail: Value, kind: &str) -> Value {
    dict([
        ("id", s(id)),
        ("severity", s(severity)),
        ("title", s(title)),
        ("detail", detail),
        ("daemon_kind", s(kind)),
    ])
}

/// The custody summary when no book is active.
fn unavailable_custody_quantity() -> Value {
    dict([
        ("status", s("unavailable")),
        (
            "status_text",
            s("Custody gap status unavailable: no active profile"),
        ),
        ("derived_state_current", Value::Bool(false)),
        ("issue_count", 0.into()),
        ("quantified_issue_count", 0.into()),
        ("unquantified_issue_count", 0.into()),
        ("unresolved_by_asset", Value::List(vec![])),
        ("by_state", Value::List(vec![])),
        ("blocked_from", Value::Null),
        (
            "presumed_external",
            dict([
                ("slice_count", 0.into()),
                ("transaction_count", 0.into()),
                ("by_asset", Value::List(vec![])),
                ("treatment", s("warning_not_blocker")),
            ]),
        ),
        ("warnings", Value::List(vec![])),
        ("qualification", s(QUALIFICATION)),
    ])
}

/// `custody_quantity_readiness_summary(conn, profile_id, journal_status=...)`.
pub fn custody_quantity_readiness(
    conn: &Connection,
    profile_id: &str,
    journal_status: &str,
    include_journal_state: bool,
) -> Result<Value> {
    let issues = query(
        conn,
        "SELECT issue_id, issue_type, state, asset, amount_msat, occurred_at, reason, blocks_from \
         FROM journal_quantity_issues WHERE profile_id = ? ORDER BY COALESCE(occurred_at, ''), issue_id",
        [profile_id],
    )
    .map_err(|error| {
        let error_type = match &error {
            kassiber_store::rusqlite::Error::SqliteFailure(code, _)
                if code.code == ErrorCode::DatabaseCorrupt || code.code == ErrorCode::NotADatabase =>
            {
                "DatabaseError"
            }
            _ => "OperationalError",
        };
        AppError::new(
            "custody_quantity_state_unavailable",
            "Custody quantity blocker state is unavailable.",
        )
        .with_hint(
            "Repair or migrate the local book, then rebuild journals before relying on reports.",
        )
        .with_details(dict([
            ("profile_id", s(profile_id)),
            ("operation", s("read_journal_quantity_issues")),
            ("error_type", s(error_type)),
        ]))
    })?;
    let presumed = match query(
        conn,
        "SELECT asset, COUNT(*) AS slice_count, COUNT(DISTINCT transaction_id) AS transaction_count, \
         SUM(amount_msat) AS amount_msat FROM journal_quantity_postings WHERE profile_id = ? \
         AND location_kind = 'external' AND state = 'external_presumed' AND amount_msat > 0 \
         GROUP BY asset ORDER BY asset",
        [profile_id],
    ) {
        Ok(rows) => rows,
        Err(error) if operational(&error) => Vec::new(),
        Err(error) => return Err(sql_error(error)),
    };
    let mut presumed_by_asset = Vec::new();
    let (mut slices, mut transactions) = (BigInt::from(0), BigInt::from(0));
    for row in &presumed {
        let slice_count = int(&val(row, "slice_count"))?;
        let transaction_count = int(&val(row, "transaction_count"))?;
        slices += slice_count.clone();
        transactions += transaction_count.clone();
        presumed_by_asset.push(dict([
            ("asset", s(py_str(&val(row, "asset")))),
            ("amount_msat", Value::Int(int(&val(row, "amount_msat"))?)),
            ("slice_count", Value::Int(slice_count)),
            ("transaction_count", Value::Int(transaction_count)),
        ]));
    }
    let presumed_any = slices != BigInt::from(0);
    let presumed_external = dict([
        ("slice_count", Value::Int(slices)),
        ("transaction_count", Value::Int(transactions)),
        ("by_asset", Value::List(presumed_by_asset)),
        ("treatment", s("warning_not_blocker")),
    ]);
    let mut by_asset: BTreeMap<String, (BigInt, i64)> = BTreeMap::new();
    let mut by_state: BTreeMap<String, (i64, BTreeMap<String, BigInt>)> = BTreeMap::new();
    let mut quantified = 0;
    for issue in &issues {
        let state = text_or(issue, "state", "unknown");
        let entry = by_state.entry(state).or_default();
        entry.0 += 1;
        let amount = val(issue, "amount_msat");
        if amount == Value::Null {
            continue;
        }
        quantified += 1;
        let asset = text_or(issue, "asset", "UNKNOWN");
        let amount = int(&amount)?;
        let summary = by_asset.entry(asset.clone()).or_default();
        summary.0 += amount.clone();
        summary.1 += 1;
        *entry.1.entry(asset).or_default() += amount;
    }
    let unresolved_by_asset: Vec<Value> = by_asset
        .into_iter()
        .map(|(asset, (amount, issues))| {
            dict([
                ("asset", s(asset)),
                ("amount_msat", Value::Int(amount)),
                ("issue_count", issues.into()),
            ])
        })
        .collect();
    let state_rows: Vec<Value> = by_state
        .into_iter()
        .map(|(state, (issues, assets))| {
            dict([
                ("state", s(state)),
                ("issue_count", issues.into()),
                (
                    "unresolved_by_asset",
                    Value::List(
                        assets
                            .into_iter()
                            .map(|(asset, amount)| {
                                dict([("asset", s(asset)), ("amount_msat", Value::Int(amount))])
                            })
                            .collect(),
                    ),
                ),
            ])
        })
        .collect();
    let blocked_from = issues
        .iter()
        .filter_map(|issue| get(issue, "blocks_from").filter(|v| truthy(v)).map(py_str))
        .min()
        .map_or(Value::Null, s);
    let (status, status_text) = match journal_status {
        _ if !include_journal_state => {
            ("excluded", "Custody gap status excluded by export options")
        }
        "current" if issues.is_empty() => ("no_known_custody_gaps", "No known custody gaps"),
        "current" => ("known_custody_gaps", "Known custody gaps require review"),
        "no_transactions" => (
            "not_evaluated",
            "Custody gaps not evaluated: no active transactions",
        ),
        _ => (
            "needs_processing",
            "Custody gap status needs journal processing",
        ),
    };
    let warnings = if presumed_any {
        vec![dict([
            ("code", s("external_custody_presumed")),
            ("severity", s("warning")),
            (
                "message",
                s("Unmatched outflows are treated as presumed external; this is visible but does not block reports."),
            ),
        ])]
    } else {
        Vec::new()
    };
    Ok(dict([
        ("status", s(status)),
        ("status_text", s(status_text)),
        (
            "derived_state_current",
            Value::Bool(journal_status == "current"),
        ),
        ("issue_count", issues.len().into()),
        ("quantified_issue_count", quantified.into()),
        (
            "unquantified_issue_count",
            (issues.len() - quantified).into(),
        ),
        ("unresolved_by_asset", Value::List(unresolved_by_asset)),
        ("by_state", Value::List(state_rows)),
        ("blocked_from", blocked_from),
        ("presumed_external", presumed_external),
        ("warnings", Value::List(warnings)),
        ("qualification", s(QUALIFICATION)),
    ]))
}

/// `_silent_payment_report_blockers`: configured BIP352 wallets whose scan
/// range has not completed or is degraded.
fn silent_payment_blockers(conn: &Connection, profile_id: &str) -> Result<Vec<Value>> {
    let wallets = rows(
        conn,
        "SELECT id, label, config_json FROM wallets WHERE profile_id = ? AND kind = 'silent-payment' ORDER BY label ASC",
        [profile_id],
    )?;
    let mut blockers = Vec::new();
    for wallet in wallets {
        let config = match get(&wallet, "config_json") {
            Some(Value::Str(raw)) => {
                let raw = raw.as_str().unwrap_or_default();
                kassiber_pytext::loads(if raw.is_empty() { "{}" } else { raw })
                    .ok()
                    .filter(|v| matches!(v, Value::Dict(_)))
                    .unwrap_or_else(|| dict::<&str>([]))
            }
            _ => dict::<&str>([]),
        };
        let material = get(&config, "sp_descriptor")
            .filter(|v| !matches!(v, Value::Null))
            .is_some_and(|v| {
                kassiber_pytext::strip_py(&kassiber_pyjson::PyStr::from(py_str(v).as_str()))
                    .code_points()
                    .next()
                    .is_some()
            });
        if !material {
            continue;
        }
        let id = py_str(&val(&wallet, "id"));
        let label = py_str(&val(&wallet, "label"));
        let state = rows(
            conn,
            "SELECT status, stale_reason, blocking_reports, checkpoint_json FROM freshness_source_states WHERE profile_id = ? AND source_key = ?",
            [profile_id, &format!("onchain_wallet:{id}")],
        )?
        .into_iter()
        .next();
        let Some(state) = state else {
            blockers.push(dict([
                ("id", s(format!("silent_payment_scan_pending:{id}"))),
                ("code", s("silent_payment_scan_pending")),
                ("severity", s("blocking")),
                ("title", s("Silent Payments scan pending")),
                (
                    "detail",
                    s(format!(
                        "{label} has not completed its configured BIP352 scan range yet."
                    )),
                ),
                ("daemon_kind", s("ui.wallets.sync")),
            ]));
            continue;
        };
        let checkpoint = match get(&state, "checkpoint_json") {
            Some(raw @ Value::Str(text)) if truthy(raw) => {
                kassiber_pytext::loads(text.as_str().unwrap_or_default()).unwrap_or(Value::Null)
            }
            _ => dict::<&str>([]),
        };
        let sp_state = get(&checkpoint, "silent_payment")
            .filter(|v| matches!(v, Value::Dict(_)))
            .cloned()
            .unwrap_or_else(|| dict::<&str>([]));
        let complete = get(&sp_state, "scan_complete").is_some_and(truthy);
        let degraded = get(&sp_state, "degraded").is_some_and(truthy)
            || get(&state, "blocking_reports").is_some_and(truthy);
        if complete && !degraded {
            continue;
        }
        let reason = match get(&sp_state, "degraded_reason").filter(|v| truthy(v)) {
            Some(reason) => py_str(reason),
            None => text_or(&state, "stale_reason", "scan_incomplete"),
        };
        blockers.push(dict([
            ("id", s(format!("silent_payment_scan_degraded:{id}"))),
            ("code", s("silent_payment_scan_incomplete")),
            ("severity", s("blocking")),
            ("title", s("Silent Payments scan incomplete")),
            (
                "detail",
                s(format!(
                    "{label} has incomplete BIP352 scan coverage ({reason})."
                )),
            ),
            ("daemon_kind", s("ui.wallets.sync")),
        ]));
    }
    Ok(blockers)
}

/// `_unreviewed_swap_candidate_blocker`: matcher candidates over the whole
/// book that need pairing, payout review or dismissal before final reports.
fn swap_candidate_blocker(conn: &Connection, scope: &Scope<'_>) -> Result<Option<Value>> {
    let rows_ = rows(
        conn,
        "SELECT t.id, t.profile_id, t.wallet_id, t.external_id, t.external_id_kind, t.payment_hash, \
         t.payment_hash_source, t.swap_refund_funding_txid, t.swap_refund_funding_vout, t.occurred_at, \
         t.direction, t.asset, t.amount, t.amount_includes_fee, t.fee, t.kind, t.raw_json, t.excluded, \
         w.label AS wallet_label, w.kind AS wallet_kind, w.config_json AS config_json, \
         observation.authority_version AS observation_authority_version, \
         observation.observer_kinds_json AS observation_observer_kinds_json, \
         observation.graph_hash AS observation_graph_hash, observation.quantity_hash AS observation_quantity_hash \
         FROM transactions t JOIN wallets w ON w.id = t.wallet_id \
         LEFT JOIN chain_observation_provenance observation ON observation.transaction_id = t.id \
         WHERE t.profile_id = ?",
        [scope.profile_id],
    )?;
    let pairs = rows(
        conn,
        "SELECT out_transaction_id, in_transaction_id, kind, policy, NULL AS deleted_at FROM journal_custody_projection_relations WHERE profile_id = ?",
        [scope.profile_id],
    )?;
    let dismissals = rows(
        conn,
        "SELECT out_transaction_id, in_transaction_id, expires_at FROM transaction_pair_dismissals WHERE profile_id = ?",
        [scope.profile_id],
    )?;
    let booked: HashSet<String> = ProjectionGuards::load(conn, scope)?.booked_move_transaction_ids;
    let population = Population::new(&rows_)?;
    let candidates: Vec<_> = suggest(
        &population,
        &pairs,
        &dismissals,
        &booked,
        &MatchOptions::default(),
    )?
    .into_iter()
    .filter(|c| {
        BLOCKING_SWAP_KINDS.contains(&c.str("default_kind"))
            || kassiber_pytext::upper(c.str("out_asset"))
                != kassiber_pytext::upper(c.str("in_asset"))
            || ["payment_hash", "provider_swap_id", "htlc_refund"].contains(&c.str("method"))
    })
    .collect();
    if candidates.is_empty() {
        return Ok(None);
    }
    let exact = candidates
        .iter()
        .filter(|c| c.str("confidence") == "exact")
        .count();
    let strong = candidates
        .iter()
        .filter(|c| c.str("confidence") == "strong")
        .count();
    let routes = candidates
        .iter()
        .take(5)
        .map(|c| {
            let data = c.clone().into_value();
            dict([
                ("out_asset", val(&data, "out_asset")),
                ("in_asset", val(&data, "in_asset")),
                ("confidence", val(&data, "confidence")),
                ("method", val(&data, "method")),
                ("default_kind", val(&data, "default_kind")),
                ("conflict_size", val(&data, "conflict_size")),
            ])
        })
        .collect();
    let mut output = blocker(
        "unreviewed_swap_candidates",
        "blocking",
        "Unreviewed swap candidates",
        s(format!(
            "{} swap-shaped candidate(s) need pairing, payout review, or dismissal before final reports.",
            candidates.len()
        )),
        "ui.transfers.suggest",
    );
    set(
        &mut output,
        "counts",
        dict([
            ("total", candidates.len().into()),
            ("exact", exact.into()),
            ("strong", strong.into()),
        ]),
    );
    set(&mut output, "routes", Value::List(routes));
    Ok(Some(output))
}

/// `_ownership_review_candidate_blocker`: the cached ownership-proven
/// transfer review counts from the last journal run.
fn ownership_blocker(conn: &Connection, profile_id: &str) -> Result<Option<Value>> {
    let Some(profile) = rows(
        conn,
        "SELECT ownership_review_counts_json FROM profiles WHERE id = ?",
        [profile_id],
    )?
    .into_iter()
    .next() else {
        return Ok(None);
    };
    let raw = val(&profile, "ownership_review_counts_json");
    if !truthy(&raw) {
        return Ok(None);
    }
    let Value::Str(raw) = raw else {
        return Ok(None);
    };
    let Ok(cached) = kassiber_pytext::loads(raw.as_str().unwrap_or_default()) else {
        return Ok(None);
    };
    // `cached.get(...)` on a non-mapping is an AttributeError, as is a
    // non-mapping `by_reason`; both mean no blocker.
    if !matches!(cached, Value::Dict(_)) {
        return Ok(None);
    }
    let total = match int(&val(&cached, "total")) {
        Ok(total) => total,
        Err(error) if error.code == "internal_error" && error.message.contains("infinity") => {
            return Err(error)
        }
        Err(_) => return Ok(None),
    };
    let mut by_reason = Vec::new();
    let reasons = val(&cached, "by_reason");
    match &reasons {
        Value::Dict(pairs) => {
            for (key, value) in pairs {
                let n = match int_strict(value) {
                    Ok(n) => n,
                    Err(error)
                        if error.code == "internal_error" && error.message.contains("infinity") =>
                    {
                        return Err(error)
                    }
                    Err(_) => return Ok(None),
                };
                if n > BigInt::from(0) {
                    by_reason.push((key_text(key), Value::Int(n)));
                }
            }
        }
        value if !truthy(value) => {}
        _ => return Ok(None),
    }
    if total <= BigInt::from(0) {
        return Ok(None);
    }
    let mut output = blocker(
        "ownership_transfer_review",
        "blocking",
        "Ownership transfers need review",
        s(format!(
            "{total} ownership-proven transfer candidate(s) need pairing or wallet-data review before final reports."
        )),
        "ui.transfers.suggest",
    );
    set(
        &mut output,
        "daemon_args",
        dict([
            ("candidate_type", s("transfer")),
            ("method", s("ownership_graph")),
        ]),
    );
    set(
        &mut output,
        "counts",
        dict([("total", Value::Int(total)), ("by_reason", dict(by_reason))]),
    );
    Ok(Some(output))
}

/// `int(count)` without the `or 0` default: `int(None)` is a TypeError.
fn int_strict(value: &Value) -> Result<BigInt> {
    if *value == Value::Null {
        return Err(AppError::new("internal_error", "int() of None"));
    }
    int(value)
}

fn key_text(key: &kassiber_pyjson::Key) -> String {
    match key {
        kassiber_pyjson::Key::Str(text) => py_str(&Value::Str(text.clone())),
        kassiber_pyjson::Key::Int(n) => n.to_string(),
        kassiber_pyjson::Key::Bool(b) => py_str(&Value::Bool(*b)),
        kassiber_pyjson::Key::Float(f) => py_str(&Value::Float(*f)),
        kassiber_pyjson::Key::Null => "None".into(),
    }
}

fn component_blocker(conn: &Connection, scope: &Scope<'_>) -> Result<Option<Value>> {
    let components = kassiber_transfers::authored_active_components(conn, scope)?;
    let ineffective: Vec<_> = components
        .iter()
        .filter(|c| c["effective_state"] != "active")
        .collect();
    if ineffective.is_empty() {
        return Ok(None);
    }
    let py = |v: &serde_json::Value| {
        kassiber_custody::codec::from_json(v).map_err(|e| AppError::new("internal_error", e))
    };
    let mut listed = Vec::new();
    for component in ineffective.iter().take(20) {
        let mut codes = BTreeSet::new();
        for issue in component["validation"]["issues"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
        {
            let code = py(&issue["code"])?;
            codes.insert(if truthy(&code) {
                py_str(&code)
            } else {
                "unknown".into()
            });
        }
        let anchors = component["legs"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter(|leg| !leg["transaction_id"].is_null())
            .count();
        listed.push(dict([
            ("id", py(&component["id"])?),
            ("lineage_id", py(&component["lineage_id"])?),
            ("revision", py(&component["revision"])?),
            (
                "issue_codes",
                Value::List(codes.into_iter().map(s).collect()),
            ),
            ("known_anchor_count", anchors.into()),
        ]));
    }
    let mut output = blocker(
        "custody_component_integrity",
        "blocking",
        "Incomplete custody interpretation",
        s(format!(
            "{} authored active custody component(s) are incomplete or conflicting. Repair or supersede them before relying on reports.",
            ineffective.len()
        )),
        "ui.transfers.components.list",
    );
    set(&mut output, "components", Value::List(listed));
    Ok(Some(output))
}

/// `build_report_blockers_snapshot(conn)` over the caller's workspace health
/// and `build_rates_coverage_snapshot(conn, {"limit": 10})`.
pub fn snapshot(conn: &Connection, health: Value, rates_coverage: Value) -> Result<Value> {
    let mut blockers = Vec::new();
    let mut custody_quantity = unavailable_custody_quantity();
    let profile = val(&health, "profile");
    if profile == Value::Null {
        blockers.push(blocker(
            "no_active_profile",
            "blocking",
            "No active profile",
            s("Create or select a workspace and profile first."),
            "ui.profiles.snapshot",
        ));
    } else {
        let profile_id = py_str(&val(&profile, "id"));
        let workspace_id = py_str(&val(&val(&health, "workspace"), "id"));
        let scope = Scope {
            workspace_id: &workspace_id,
            profile_id: &profile_id,
        };
        let counts = val(&health, "counts");
        let journals = val(&health, "journals");
        let failed_count = rows(
            conn,
            "SELECT COUNT(*) AS count FROM freshness_source_states WHERE profile_id = ? AND blocking_reports = 1",
            [&profile_id],
        )?
        .first()
        .map_or(Ok(BigInt::from(0)), |row| int(&val(row, "count")))?;
        if failed_count != BigInt::from(0) {
            let sources = rows(
                conn,
                "SELECT source_type, source_label, status, stale_reason, last_error_code FROM freshness_source_states \
                 WHERE profile_id = ? AND blocking_reports = 1 ORDER BY source_type ASC, source_label ASC, source_key ASC LIMIT ?",
                kassiber_store::rusqlite::params![profile_id, 20],
            )?;
            let mut item = blocker(
                "sync_failed",
                "blocking",
                "Connection refresh failed",
                s(format!(
                    "{failed_count} source refresh failure(s) must be repaired before final reports can be exported."
                )),
                "ui.freshness.run",
            );
            set(
                &mut item,
                "counts",
                dict([("sources", Value::Int(failed_count))]),
            );
            set(&mut item, "sources", Value::List(sources));
            blockers.push(item);
        }
        custody_quantity = custody_quantity_readiness(
            conn,
            &profile_id,
            &py_str(&val(&journals, "status")),
            true,
        )?;
        let processed = int(&val(&journals, "last_processed_input_version"))?;
        let edit_stale = kassiber_transactions::metadata::stale_summary(
            conn,
            &profile_id,
            i64::try_from(processed).unwrap_or(i64::MAX),
            val(&journals, "last_processed_at"),
        )?;
        if let Some(item) = component_blocker(conn, &scope)? {
            blockers.push(item);
        }
        if count(&val(&counts, "wallets")) == 0 {
            blockers.push(blocker(
                "no_wallets",
                "blocking",
                "No sources",
                s("Add a watch-only source before refreshing, importing, or reporting."),
                "ui.wallets.list",
            ));
        }
        if count(&val(&counts, "transactions")) == 0 {
            blockers.push(blocker(
                "no_transactions",
                "blocking",
                "No transactions",
                s("Refresh sources or import transactions before reports can be useful."),
                "ui.wallets.sync",
            ));
        }
        blockers.extend(silent_payment_blockers(conn, &profile_id)?);
        if let Some((overlap, repair_preview)) =
            crate::prebuild::source_overlap_blocker(conn, &profile_id)
                .map_err(crate::service::application_error)?
        {
            let mut item = blocker(
                "source_overlap",
                "blocking",
                "Overlapping wallet sources",
                s(format!(
                    "{} concrete watched script(s) are owned by multiple active sources. Kassiber can resolve descriptor/xpub versus address-list duplicates by trimming the address-list source and excluding exact duplicate rows before rebuilding journals.",
                    py_str(&val(&overlap, "overlap_count"))
                )),
                "ui.journals.process",
            );
            set(&mut item, "overlap", overlap);
            set(&mut item, "repair_preview", repair_preview);
            blockers.push(item);
        }
        if truthy(&val(&journals, "needs_processing")) {
            let edits = int(&val(&edit_stale, "edit_count"))?;
            let reason = val(&journals, "reason");
            let detail = if edits != BigInt::from(0) {
                s(format!(
                    "{}; {edits} metadata edit(s) changed report inputs.",
                    py_str(&reason)
                ))
            } else {
                reason
            };
            let mut item = blocker(
                "journals_stale",
                "blocking",
                "Journals need processing",
                detail,
                "ui.journals.process",
            );
            set(&mut item, "activity_kind", s("ui.activity.history"));
            set(&mut item, "edit_history", edit_stale);
            blockers.push(item);
        }
        let quarantines = val(&journals, "quarantine_count");
        if truthy(&quarantines) {
            blockers.push(blocker(
                "journal_quarantine",
                "blocking",
                "Quarantined journal rows",
                s(format!(
                    "{} transaction(s) need review.",
                    py_str(&quarantines)
                )),
                "ui.journals.quarantine",
            ));
        }
        let issues = val(&custody_quantity, "issue_count");
        if truthy(&issues) {
            let unresolved = val(&custody_quantity, "unresolved_by_asset");
            let states = match val(&custody_quantity, "by_state") {
                Value::List(items) => items.iter().map(|item| val(item, "state")).collect(),
                _ => Vec::new(),
            };
            let single = match &unresolved {
                Value::List(items) if items.len() == 1 => val(&items[0], "amount_msat"),
                _ => Value::Null,
            };
            let mut item = blocker(
                "custody_quantity_unresolved",
                "blocking",
                "Unresolved custody quantity",
                s(format!(
                    "{} exact quantity issue(s) block final basis from the earliest affected event onward.",
                    py_str(&issues)
                )),
                "ui.journals.process",
            );
            set(
                &mut item,
                "blocked_from",
                val(&custody_quantity, "blocked_from"),
            );
            set(&mut item, "states", Value::List(states));
            set(&mut item, "unresolved_by_asset", unresolved);
            set(&mut item, "unresolved_msat", single);
            set(
                &mut item,
                "unquantified_issue_count",
                val(&custody_quantity, "unquantified_issue_count"),
            );
            blockers.push(item);
        }
        if let Some(item) = ownership_blocker(conn, &profile_id)? {
            blockers.push(item);
        }
        if let Some(item) = swap_candidate_blocker(conn, &scope)? {
            blockers.push(item);
        }
        let summary = val(&rates_coverage, "summary");
        let missing = val(&summary, "missing_price_transactions");
        if truthy(&missing) {
            let uncovered = truthy(&val(&summary, "cache_uncovered_missing"));
            blockers.push(blocker(
                "missing_prices",
                "review",
                "Missing transaction prices",
                s(format!(
                    "{} transaction(s) are missing fiat price fields.",
                    py_str(&missing)
                )),
                if uncovered {
                    "ui.rates.rebuild"
                } else {
                    "ui.journals.process"
                },
            ));
        }
    }
    let warnings = val(&custody_quantity, "warnings");
    Ok(dict([
        ("ready", Value::Bool(blockers.is_empty())),
        ("blockers", Value::List(blockers)),
        ("warnings", warnings),
        ("custody_quantity", custody_quantity),
        ("health", health),
        ("rates_coverage", rates_coverage),
    ]))
}

/// `_sync_failure_blocker` + `_apply_sync_failure_blocker`: an automatic
/// report-read refresh that failed makes the snapshot not ready.
pub fn apply_sync_failure(mut payload: Value, sync: Option<&Value>) -> Value {
    let Some(sync) = sync.filter(|v| matches!(v, Value::Dict(_))) else {
        return payload;
    };
    let errors = match get(sync, "results") {
        Some(Value::List(results)) => results
            .iter()
            .filter(|row| {
                matches!(row, Value::Dict(_))
                    && kassiber_pytext::lower(&text_or(row, "status", "")) == "error"
            })
            .count(),
        _ => 0,
    };
    if get(sync, "ok") != Some(&Value::Bool(false)) && errors == 0 {
        return payload;
    }
    let detail = if errors > 0 {
        format!("Automatic watch-only refresh failed for {errors} source(s); reports may be stale.")
    } else {
        "Automatic watch-only refresh failed; reports may be stale.".into()
    };
    let item = blocker(
        "sync_failed",
        "blocking",
        "Connection refresh failed",
        s(detail),
        "ui.wallets.sync",
    );
    let mut blockers = match get(&payload, "blockers") {
        Some(Value::List(items)) => items.clone(),
        _ => Vec::new(),
    };
    if !blockers.iter().any(|existing| {
        matches!(existing, Value::Dict(_)) && get(existing, "id") == Some(&s("sync_failed"))
    }) {
        blockers.insert(0, item);
    }
    set(&mut payload, "blockers", Value::List(blockers));
    set(&mut payload, "ready", Value::Bool(false));
    payload
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(payload: &Value) -> Vec<String> {
        match get(payload, "blockers") {
            Some(Value::List(items)) => items.iter().map(|b| py_str(&val(b, "id"))).collect(),
            _ => Vec::new(),
        }
    }

    #[test]
    fn sync_failure_leads_existing_blockers() {
        // A refresh that failed before any job ran (an AppError or a busy slot)
        // left no blocking source state, so the snapshot has no sync blocker.
        let payload = dict([
            ("ready", Value::Bool(true)),
            (
                "blockers",
                Value::List(vec![blocker(
                    "no_transactions",
                    "blocking",
                    "t",
                    s("d"),
                    "k",
                )]),
            ),
        ]);
        let failed = dict([
            ("ok", Value::Bool(false)),
            ("reason", s("project_operation_in_progress")),
        ]);
        let applied = apply_sync_failure(payload.clone(), Some(&failed));
        assert_eq!(ids(&applied), ["sync_failed", "no_transactions"]);
        assert_eq!(get(&applied, "ready"), Some(&Value::Bool(false)));
        // An existing snapshot sync blocker is kept, not duplicated.
        assert_eq!(
            ids(&apply_sync_failure(applied, Some(&failed))),
            ["sync_failed", "no_transactions"]
        );
        // A refresh without errors, or none at all, leaves the snapshot alone.
        let fine = dict([("ok", Value::Bool(true)), ("results", Value::List(vec![]))]);
        assert_eq!(apply_sync_failure(payload.clone(), Some(&fine)), payload);
        assert_eq!(apply_sync_failure(payload.clone(), None), payload);
    }
}
