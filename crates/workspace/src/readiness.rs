//! `build_workspace_health_snapshot`, `build_next_actions_snapshot` and
//! `build_review_badges_snapshot`: persisted freshness, counts, report hints,
//! the grouped review queue and cached badge counts.
use std::collections::BTreeMap;

use kassiber_envelope::{dict, get, s, set, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_transactions::Scope;

use crate::{count, rows, sql_error, text, val, Result};

fn scalar(conn: &Connection, sql: &str, profile: &str) -> Result<i64> {
    conn.query_row(sql, [profile], |row| row.get(0))
        .map_err(sql_error)
}

/// `_active_context_and_profile(conn, workspace_id, profile_id)`. An explicit
/// scope must name an existing profile of that workspace.
pub fn scope(conn: &Connection, explicit: Option<(&str, &str)>) -> Result<(Value, Option<Value>)> {
    if let Some((workspace_id, profile_id)) = explicit {
        if workspace_id.is_empty() || profile_id.is_empty() {
            return Err(AppError::new(
                "validation",
                "workspace and profile must be supplied together",
            ));
        }
        let workspace = rows(
            conn,
            "SELECT id, label FROM workspaces WHERE id = ?",
            [workspace_id],
        )?
        .into_iter()
        .next();
        let profile = rows(
            conn,
            "SELECT * FROM profiles WHERE id = ? AND workspace_id = ?",
            [profile_id, workspace_id],
        )?
        .into_iter()
        .next();
        let (Some(workspace), Some(profile)) = (workspace, profile) else {
            return Err(AppError::new(
                "not_found",
                "the requested accounting scope was not found",
            ));
        };
        let context = dict([
            ("workspace_id", val(&workspace, "id")),
            ("workspace_label", val(&workspace, "label")),
            ("profile_id", val(&profile, "id")),
            ("profile_label", val(&profile, "label")),
        ]);
        return Ok((context, Some(profile)));
    }
    let context = kassiber_setup::context::snapshot(conn)?;
    if !truthy(&val(&context, "workspace_id")) || !truthy(&val(&context, "profile_id")) {
        return Ok((context, None));
    }
    let profile = rows(
        conn,
        "SELECT * FROM profiles WHERE id = ?",
        [text(&context, "profile_id")],
    )?
    .into_iter()
    .next();
    Ok((context, profile))
}

/// `_journal_freshness(conn, profile)`: canonical projection freshness plus
/// journal and quarantine counts. The no-profile shape has no `is_current`.
pub fn journal_freshness(conn: &Connection, profile: Option<&Value>) -> Result<Value> {
    let Some(profile) = profile else {
        return Ok(dict([
            ("status", s("no_profile")),
            ("needs_processing", Value::Bool(false)),
            ("last_processed_at", Value::Null),
            ("last_processed_tx_count", 0.into()),
            ("journal_input_version", 0.into()),
            ("last_processed_input_version", 0.into()),
            ("active_transaction_count", 0.into()),
            ("journal_entry_count", 0.into()),
            ("quarantine_count", 0.into()),
            ("reason", s("no active profile")),
        ]));
    };
    let id = text(profile, "id");
    let active = scalar(
        conn,
        "SELECT COUNT(*) FROM transactions WHERE profile_id = ? AND excluded = 0",
        &id,
    )?;
    let mut freshness = evaluate_freshness(profile, active)?;
    set(
        &mut freshness,
        "journal_entry_count",
        scalar(
            conn,
            "SELECT COUNT(*) FROM journal_entries WHERE profile_id = ?",
            &id,
        )?
        .into(),
    );
    set(
        &mut freshness,
        "quarantine_count",
        scalar(
            conn,
            "SELECT COUNT(*) FROM journal_quarantines WHERE profile_id = ?",
            &id,
        )?
        .into(),
    );
    Ok(freshness)
}

/// `evaluate_projection_freshness(profile, active_transaction_count)`.
pub fn evaluate_freshness(profile: &Value, active: i64) -> Result<Value> {
    let processed_at = val(profile, "last_processed_at");
    let processed = stored_int(profile, "last_processed_tx_count")?;
    let input = stored_int(profile, "journal_input_version")?;
    let previous = stored_int(profile, "last_processed_input_version")?;
    // A profile row without both semantics columns is never current.
    let semantics = match (
        get(profile, "required_journal_semantics_version"),
        get(profile, "journal_semantics_version"),
    ) {
        (Some(_), Some(_)) => {
            let required = stored_int(profile, "required_journal_semantics_version")?;
            let built = stored_int(profile, "journal_semantics_version")?;
            let version =
                Value::Int(kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION.into());
            !truthy(&required) || (built == version && ge(&built, &required))
        }
        _ => false,
    };
    let active_value = Value::Int(active.into());
    let is_current =
        truthy(&processed_at) && processed == active_value && input == previous && semantics;
    let (status, reason) = if active == 0 {
        ("no_transactions", "no active transactions")
    } else if !truthy(&processed_at) {
        ("not_processed", "journals have not been processed")
    } else if processed != active_value {
        (
            "stale",
            "active transaction count changed since last processing",
        )
    } else if input != previous {
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
        ("is_current", Value::Bool(is_current)),
        (
            "needs_processing",
            Value::Bool(matches!(status, "not_processed" | "stale")),
        ),
        ("reason", s(reason)),
        ("last_processed_at", processed_at),
        ("last_processed_tx_count", processed),
        ("journal_input_version", input),
        ("last_processed_input_version", previous),
        ("active_transaction_count", active_value),
    ]))
}

fn ge(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Int(left), Value::Int(right)) => left >= right,
        _ => false,
    }
}

/// `int(profile[key] or 0)`, with Python's conversion of stored text/REAL.
pub(crate) fn stored_int(row: &Value, key: &str) -> Result<Value> {
    let raw = val(row, key);
    if !truthy(&raw) {
        return Ok(0.into());
    }
    crate::py_int(&raw).map(Value::Int)
}

/// `_profile_readiness(wallet_count, transaction_count, freshness)`.
pub fn profile_readiness(wallets: i64, transactions: i64, freshness: &Value) -> (bool, Vec<Value>) {
    let mut hints = Vec::new();
    if wallets == 0 {
        hints.push(s(
            "Add a watch-only source before refreshing or importing transactions.",
        ));
    } else if transactions == 0 {
        hints.push(s(
            "Refresh sources or import files before journal processing.",
        ));
    }
    if truthy(&val(freshness, "needs_processing")) {
        hints.push(s("Run journal processing before trusting reports."));
    }
    let quarantines = count(freshness, "quarantine_count");
    if quarantines != 0 {
        hints.push(s("Review quarantined transactions before tax export."));
    }
    let ready = wallets > 0
        && transactions > 0
        && text(freshness, "status") == "current"
        && quarantines == 0;
    if ready {
        hints.push(s(
            "Reports are ready from the current processed journal state.",
        ));
    }
    (ready, hints)
}

/// `build_workspace_health_snapshot(conn, workspace_id, profile_id)`.
/// `WORKSPACE_HEALTH_COUNT_DEFINITIONS`: one sentence per counter, so a
/// reader can name why two surfaces count the same book differently.
const COUNT_DEFINITIONS: [(&str, &str); 7] = [
    ("wallets", "Wallets in the active book."),
    ("transactions", "Transaction rows in the active book, excluded rows included."),
    ("active_transactions", "Transaction rows that are not excluded."),
    ("journal_entries", "Journal entries from the last processing; stale when journals.needs_processing is true."),
    ("quarantines", "Quarantined rows from the last processing, one per transaction (ui.journals.quarantine summary.count)."),
    ("rate_pairs", "Distinct cached rate pairs on this device, shared by all books."),
    ("review_queue", "Transaction review status, not quarantine: a quarantined row counts as review, but so does a row the user marked, and a reviewed pair counts once while journals are current."),
];

pub fn health(conn: &Connection, context: &Value, profile: Option<&Value>) -> Result<Value> {
    let freshness = journal_freshness(conn, profile)?;
    let Some(profile) = profile else {
        return Ok(dict([
            ("workspace", Value::Null),
            ("profile", Value::Null),
            (
                "counts",
                dict([
                    ("wallets", 0.into()),
                    ("transactions", 0.into()),
                    ("active_transactions", 0.into()),
                    ("journal_entries", 0.into()),
                    ("quarantines", 0.into()),
                    ("rate_pairs", 0.into()),
                ]),
            ),
            ("journals", freshness),
            (
                "reports",
                dict([
                    ("ready", Value::Bool(false)),
                    (
                        "hints",
                        Value::List(vec![s("Create or select a workspace and profile first.")]),
                    ),
                ]),
            ),
        ]));
    };
    let id = text(profile, "id");
    let wallets = scalar(
        conn,
        "SELECT COUNT(*) FROM wallets WHERE profile_id = ?",
        &id,
    )?;
    let transactions = scalar(
        conn,
        "SELECT COUNT(*) FROM transactions WHERE profile_id = ?",
        &id,
    )?;
    let rate_pairs: i64 = conn
        .query_row("SELECT COUNT(DISTINCT pair) FROM rates_cache", [], |r| {
            r.get(0)
        })
        .map_err(sql_error)?;
    let (ready, hints) = profile_readiness(wallets, transactions, &freshness);
    let queue = kassiber_transactions::desktop::review_queue_counts(
        conn,
        &Scope {
            workspace_id: &text(context, "workspace_id"),
            profile_id: &id,
        },
    )?;
    Ok(dict([
        (
            "workspace",
            dict([
                ("id", val(context, "workspace_id")),
                ("label", val(context, "workspace_label")),
            ]),
        ),
        (
            "profile",
            dict([
                ("id", val(profile, "id")),
                ("label", val(profile, "label")),
                ("fiat_currency", val(profile, "fiat_currency")),
                ("tax_country", val(profile, "tax_country")),
                (
                    "tax_long_term_days",
                    stored_int(profile, "tax_long_term_days")?,
                ),
                ("gains_algorithm", val(profile, "gains_algorithm")),
            ]),
        ),
        (
            "counts",
            dict([
                ("wallets", wallets.into()),
                ("transactions", transactions.into()),
                (
                    "active_transactions",
                    val(&freshness, "active_transaction_count"),
                ),
                ("journal_entries", val(&freshness, "journal_entry_count")),
                ("quarantines", val(&freshness, "quarantine_count")),
                ("rate_pairs", rate_pairs.into()),
            ]),
        ),
        ("review_queue", queue),
        ("journals", freshness),
        (
            "reports",
            dict([("ready", Value::Bool(ready)), ("hints", Value::List(hints))]),
        ),
        (
            "definitions",
            dict(COUNT_DEFINITIONS.map(|(key, text)| (key, s(text)))),
        ),
    ]))
}

fn suggestion(
    id: &str,
    title: &str,
    reason: Value,
    mutating: bool,
    daemon_kind: Option<&str>,
) -> Value {
    let mut row = dict([
        ("id", s(id)),
        ("title", s(title)),
        ("reason", reason),
        ("mutating", Value::Bool(mutating)),
        ("requires_consent", Value::Bool(mutating)),
    ]);
    if let Some(kind) = daemon_kind {
        set(&mut row, "daemon_kind", s(kind));
    }
    row
}

/// `build_next_actions_snapshot`. Price coverage is read lazily and, as in
/// Python, always for the active book even under an explicit CLI scope.
pub fn next_actions(health: Value, coverage: impl FnOnce() -> Result<Value>) -> Result<Value> {
    let mut suggestions = Vec::new();
    if val(&health, "profile") == Value::Null {
        suggestions.push(suggestion(
            "create_workspace_profile",
            "Create a workspace and profile",
            s("Kassiber needs an active accounting scope before sources or reports."),
            true,
            None,
        ));
        return Ok(dict([
            ("health", health),
            ("suggestions", Value::List(suggestions)),
        ]));
    }
    let counts = val(&health, "counts");
    let journals = val(&health, "journals");
    let needs_processing = truthy(&val(&journals, "needs_processing"));
    if count(&counts, "wallets") == 0 {
        suggestions.push(suggestion(
            "create_wallet",
            "Add a watch-only source",
            s("No sources exist in the active profile."),
            true,
            None,
        ));
    } else if count(&counts, "transactions") == 0 {
        suggestions.push(suggestion(
            "sync_or_import",
            "Refresh sources or import transactions",
            s("Sources exist but the active profile has no transactions yet."),
            true,
            Some("ui.wallets.sync"),
        ));
    } else if needs_processing {
        suggestions.push(suggestion(
            "process_journals",
            "Process journals",
            val(&journals, "reason"),
            true,
            Some("ui.journals.process"),
        ));
    } else {
        let summary = val(&coverage()?, "summary");
        let missing = val(&summary, "missing_price_transactions");
        if truthy(&missing) {
            let uncovered = truthy(&val(&summary, "cache_uncovered_missing"));
            suggestions.push(suggestion(
                if uncovered {
                    "fetch_missing_prices"
                } else {
                    "apply_cached_prices"
                },
                if uncovered {
                    "Fetch missing spot prices"
                } else {
                    "Apply cached spot prices"
                },
                s(format!(
                    "{} transaction(s) still need fiat prices.",
                    kassiber_envelope::py_str(&missing)
                )),
                true,
                Some(if uncovered {
                    "ui.rates.rebuild"
                } else {
                    "ui.journals.process"
                }),
            ));
        }
    }
    let quarantines = count(&journals, "quarantine_count");
    if quarantines != 0 {
        suggestions.push(suggestion(
            "review_quarantine",
            "Review quarantine",
            s(format!(
                "{quarantines} transaction(s) need review before reports are complete."
            )),
            false,
            Some("ui.journals.quarantine"),
        ));
    }
    if count(&counts, "transactions") > 0 && !needs_processing && quarantines == 0 {
        suggestions.push(suggestion(
            "run_report",
            "Run reports",
            s("Journals are current and no quarantine is blocking report review."),
            false,
            Some("ui.reports.capital_gains"),
        ));
    }
    if suggestions.is_empty() {
        suggestions.push(suggestion(
            "inspect_workspace",
            "Inspect workspace",
            s("No urgent blocker was detected from the current safe snapshot."),
            false,
            Some("ui.workspace.health"),
        ));
    }
    Ok(dict([
        ("health", health),
        ("suggestions", Value::List(suggestions)),
    ]))
}

/// `_parse_detail(detail_json)`.
fn detail(raw: &Value) -> Value {
    match raw {
        Value::Str(text) => text
            .as_str()
            .and_then(|text| kassiber_pytext::loads(if text.is_empty() { "{}" } else { text }).ok())
            .filter(|value| matches!(value, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([])),
        Value::Null => dict::<&str>([]),
        _ => dict::<&str>([]),
    }
}

/// `catalog.reason_info(reason, detail).downstream`. Quantity blockers are
/// never downstream, so the detail does not change the answer.
fn downstream(reason: &str) -> bool {
    matches!(
        reason,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "basis_provenance_incomplete"
            | "derived_transfer_group_blocked"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}

/// A transfer group id as a Python dict key: `True == 1`, `"1" != 1`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum GroupKey {
    Text(String),
    Integer(kassiber_pyjson::BigInt),
}
fn group_key(value: &Value) -> Option<GroupKey> {
    if !truthy(value) {
        return None;
    }
    match value {
        Value::Str(text) => Some(GroupKey::Text(kassiber_envelope::py_str(&Value::Str(
            text.clone(),
        )))),
        Value::Int(n) => Some(GroupKey::Integer(n.clone())),
        Value::Bool(b) => Some(GroupKey::Integer(u8::from(*b).into())),
        _ => None,
    }
}

/// `[str(item) for item in value or []]`, including Python's iteration of
/// strings (characters) and objects (keys).
fn id_list(value: Option<&Value>) -> Result<Vec<String>> {
    let Some(value) = value.filter(|value| truthy(value)) else {
        return Ok(Vec::new());
    };
    Ok(match value {
        Value::List(items) => items.iter().map(kassiber_envelope::py_str).collect(),
        Value::Str(text) => text
            .code_points()
            .map(|point| {
                kassiber_envelope::py_str(&Value::Str(
                    kassiber_pyjson::PyStr::from_code_points([point]).expect("code point"),
                ))
            })
            .collect(),
        Value::Dict(items) => items
            .iter()
            .map(|(key, _)| {
                // Parsed JSON object keys are always strings.
                kassiber_envelope::py_str(&match key {
                    kassiber_pyjson::Key::Str(key) => Value::Str(key.clone()),
                    kassiber_pyjson::Key::Int(n) => Value::Int(n.clone()),
                    kassiber_pyjson::Key::Bool(b) => Value::Bool(*b),
                    kassiber_pyjson::Key::Float(f) => Value::Float(*f),
                    kassiber_pyjson::Key::Null => Value::Null,
                })
            })
            .collect(),
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                    "'{}' object is not iterable",
                    kassiber_envelope::type_name(other)
                ),
            ))
        }
    })
}

/// Bound the Rust stack where Python would raise RecursionError.
const ROOT_DEPTH_LIMIT: usize = 900;

/// `quarantine_review.waiting_roots(rows, details)`: downstream rows that
/// follow a named root. A row whose root cannot be named needs the user.
fn waiting_roots(
    rows: &BTreeMap<String, Value>,
    details: &BTreeMap<String, Value>,
) -> Result<BTreeMap<String, String>> {
    let mut groups: BTreeMap<GroupKey, Vec<String>> = BTreeMap::new();
    for (id, detail) in details {
        if let Some(key) = group_key(&val(detail, "transfer_group_id")) {
            groups.entry(key).or_default().push(id.clone());
        }
    }
    struct Context<'a> {
        rows: &'a BTreeMap<String, Value>,
        details: &'a BTreeMap<String, Value>,
        groups: BTreeMap<GroupKey, Vec<String>>,
    }
    fn resolve(
        context: &Context<'_>,
        id: &str,
        visited: &std::collections::BTreeSet<String>,
    ) -> Result<Option<String>> {
        if visited.len() > ROOT_DEPTH_LIMIT {
            return Err(AppError::new(
                "internal_error",
                "maximum recursion depth exceeded",
            ));
        }
        let is_root = |id: &str| {
            context
                .rows
                .get(id)
                .is_some_and(|row| !downstream(&text(row, "reason")))
        };
        let mut visited = visited.clone();
        visited.insert(id.to_owned());
        let row = &context.rows[id];
        let empty = dict::<&str>([]);
        let detail = context.details.get(id).unwrap_or(&empty);
        let candidates = match text(row, "reason").as_str() {
            "custody_basis_barrier" => id_list(get(detail, "root_transaction_ids"))?,
            "transfer_pair_dependency_blocked" => {
                id_list(get(detail, "blocked_by_transaction_ids"))?
            }
            "derived_transfer_group_blocked" => group_key(&val(detail, "transfer_group_id"))
                .and_then(|key| context.groups.get(&key).cloned())
                .unwrap_or_default(),
            "basis_provenance_incomplete" => {
                let since = val(detail, "lot_state_uncertain_since");
                let matches: Vec<String> = if truthy(&since) && matches!(since, Value::Str(_)) {
                    context
                        .rows
                        .iter()
                        .filter(|(other, row)| {
                            is_root(other)
                                && val(row, "occurred_at") == since
                                && val(row, "asset") == val(&context.rows[id], "asset")
                        })
                        .map(|(other, _)| other.clone())
                        .collect()
                } else {
                    Vec::new()
                };
                if matches.len() == 1 {
                    matches
                } else {
                    Vec::new()
                }
            }
            _ => Vec::new(),
        };
        let mut roots: Vec<String> = candidates
            .iter()
            .filter(|candidate| !visited.contains(*candidate) && is_root(candidate))
            .cloned()
            .collect();
        if roots.is_empty() {
            for candidate in &candidates {
                if visited.contains(candidate) || !context.rows.contains_key(candidate) {
                    continue;
                }
                if let Some(nested) = resolve(context, candidate, &visited)? {
                    if !visited.contains(&nested) {
                        roots.push(nested);
                    }
                }
            }
        }
        Ok(roots.into_iter().min_by(|left, right| {
            let key = |id: &String| (text(&context.rows[id], "occurred_at"), id.clone());
            key(left).cmp(&key(right))
        }))
    }
    let context = Context {
        rows,
        details,
        groups,
    };
    let mut waiting = BTreeMap::new();
    for (id, row) in rows {
        if !downstream(&text(row, "reason")) {
            continue;
        }
        if let Some(root) = resolve(&context, id, &std::collections::BTreeSet::new())? {
            waiting.insert(id.clone(), root);
        }
    }
    Ok(waiting)
}

/// `build_review_badges_snapshot`: the active book's cached counts. The swap
/// count is the matcher's cached column; the matcher never runs here.
pub fn badges(conn: &Connection, profile: Option<&Value>) -> Result<Value> {
    let freshness = journal_freshness(conn, profile)?;
    let mut attention = 0usize;
    let mut swaps = Value::Null;
    if let Some(profile) = profile {
        let held = rows(
            conn,
            "SELECT q.transaction_id, q.reason, q.detail_json, t.occurred_at, t.asset \
             FROM journal_quarantines q JOIN transactions t ON t.id = q.transaction_id \
             JOIN wallets w ON w.id = t.wallet_id WHERE q.profile_id = ?",
            [text(profile, "id")],
        )?;
        let mut by_id = BTreeMap::new();
        let mut details = BTreeMap::new();
        for row in held {
            let id = text(&row, "transaction_id");
            details.insert(id.clone(), detail(&val(&row, "detail_json")));
            by_id.insert(id, row);
        }
        attention = by_id.len() - waiting_roots(&by_id, &details)?.len();
        if let Some(raw) = get(profile, "swap_candidate_count") {
            swaps = match raw {
                Value::Null => Value::Null,
                // `int(raw)`, keeping None for values Python cannot convert.
                // OverflowError is not among the conversions Python catches.
                Value::Float(n) if n.is_infinite() => return Err(crate::py_int(raw).unwrap_err()),
                other => crate::py_int(other).map(Value::Int).unwrap_or(Value::Null),
            };
        }
    }
    // One number for every surface that says how much is left to review.
    let review = match &swaps {
        Value::Int(swaps) => Value::Int(swaps + attention),
        _ => attention.into(),
    };
    Ok(dict([
        ("quarantine", val(&freshness, "quarantine_count")),
        ("quarantine_attention", attention.into()),
        (
            "journals_needs_processing",
            Value::Bool(truthy(&val(&freshness, "needs_processing"))),
        ),
        ("swaps", swaps),
        ("review", review),
        (
            "definitions",
            dict(REVIEW_BADGE_DEFINITIONS.map(|(key, text)| (key, s(text)))),
        ),
    ]))
}

const REVIEW_BADGE_DEFINITIONS: [(&str, &str); 5] = [
    ("quarantine", "Every quarantined row in the active book; equals ui.journals.quarantine summary.count."),
    ("quarantine_attention", "Quarantined root causes plus downstream rows whose root cannot be named; equals ui.journals.quarantine attention_count. Rows waiting on a named root are left out."),
    ("journals_needs_processing", "True when journal inputs changed since the last processing; quarantine counts then describe that last processing."),
    ("swaps", "Transfer/swap candidates cached by the last matcher run; null until it has run."),
    ("review", "What the Review screen asks the user to decide: quarantine_attention plus swaps. Stale journals are flagged, not counted."),
];
