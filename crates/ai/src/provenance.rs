//! Per-turn provenance and the privacy receipt (`kassiber/daemon.py:7876`–
//! `8095`). The executor state is the Python `maintenance_state` dict, kept
//! as a JSON-shaped value so every `setdefault`/`isinstance` guard behaves as
//! it does in Python.

use kassiber_envelope::{dict, get, py_str, s, set, truthy};
use kassiber_pyjson::{BigInt, Key, PyStr, Value};

/// `PROVIDER_STATUS_NOTICES`, in order: the notice phases a receipt may name.
pub const PROVIDER_STATUS_NOTICES: [&str; 2] =
    ["fast_mode_unavailable", "reasoning_effort_unavailable"];

/// Tools whose results disclose other books of the workspace.
const CROSS_BOOK_TOOLS: [&str; 2] = ["ui.profiles.snapshot", "ui.workspace.overview.snapshot"];

/// `state.setdefault(key, [])`, then the list when it is one.
fn list_entry<'a>(state: &'a mut Value, key: &str) -> Option<&'a mut Vec<Value>> {
    if get(state, key).is_none() {
        set(state, key, Value::List(Vec::new()));
    }
    let Value::Dict(pairs) = state else {
        return None;
    };
    pairs
        .iter_mut()
        .rev()
        .find(|(name, _)| matches!(name, Key::Str(name) if name.is(key)))
        .and_then(|(_, value)| match value {
            Value::List(items) => Some(items),
            _ => None,
        })
}

fn copy_int(state: &mut Value, target: &str, source: &Value, key: &str) {
    if let Some(value @ Value::Int(_)) = get(source, key) {
        set(state, target, value.clone());
    }
}

/// `_record_ai_health_provenance(state, health)`.
fn record_health(state: &mut Value, health: &Value) {
    if let Some(counts @ Value::Dict(_)) = get(health, "counts") {
        copy_int(state, "active_transactions", counts, "active_transactions");
        copy_int(state, "quarantines", counts, "quarantines");
    }
    if let Some(journals @ Value::Dict(_)) = get(health, "journals") {
        if let Some(processed @ Value::Str(_)) = get(journals, "last_processed_at") {
            set(state, "journals_processed_at", processed.clone());
        }
        copy_int(state, "quarantines", journals, "quarantine_count");
    }
}

/// `_record_ai_rates_provenance(state, rates_coverage)`.
fn record_rates(state: &mut Value, coverage: &Value) {
    let Some(summary @ Value::Dict(_)) = get(coverage, "summary") else {
        return;
    };
    copy_int(
        state,
        "missing_price_transactions",
        summary,
        "missing_price_transactions",
    );
    if let Some(active @ Value::Int(_)) = get(summary, "active_transactions") {
        if get(state, "active_transactions").is_none() {
            set(state, "active_transactions", active.clone());
        }
    }
}

/// `_record_ai_provenance_envelope(state, envelope, automatic=...)`.
pub fn record_provenance_envelope(state: &mut Value, envelope: &Value, automatic: bool) {
    let (Some(Value::Str(kind)), Some(data @ Value::Dict(_))) =
        (get(envelope, "kind"), get(envelope, "data"))
    else {
        return;
    };
    match kind.as_str() {
        Some("ui.workspace.health") => record_health(state, data),
        Some("ui.report.blockers") => {
            if let Some(health @ Value::Dict(_)) = get(data, "health") {
                record_health(state, health);
            }
            if let Some(coverage @ Value::Dict(_)) = get(data, "rates_coverage") {
                record_rates(state, coverage);
            }
        }
        Some("ui.rates.coverage") => record_rates(state, data),
        Some("ui.journals.process") => {
            if automatic {
                set(state, "auto_journal_processed", Value::Bool(true));
            }
            if let Some(processed @ Value::Str(_)) = get(data, "processed_at") {
                set(state, "journals_processed_at", processed.clone());
            }
            copy_int(state, "active_transactions", data, "processed_transactions");
            copy_int(state, "quarantines", data, "quarantined");
        }
        Some("ui.wallets.sync") => {
            set(state, "auto_sync_attempted", Value::Bool(true));
            set(
                state,
                "auto_sync_ok",
                Value::Bool(get(data, "ok") != Some(&Value::Bool(false))),
            );
            if let Some(Value::List(results)) = get(data, "results") {
                set(state, "sync_wallet_count", Value::from(results.len()));
            }
        }
        Some("ui.maintenance.run") => {
            if let Some(health @ Value::Dict(_)) = get(data, "health") {
                record_health(state, health);
            }
            if let Some(maintenance @ Value::Dict(_)) = get(data, "maintenance") {
                if let Some(journals @ Value::Dict(_)) = get(maintenance, "journals") {
                    record_provenance_envelope(state, journals, true);
                }
                if let Some(sync @ Value::Dict(_)) = get(maintenance, "sync") {
                    record_provenance_envelope(state, sync, true);
                }
            }
        }
        _ => {}
    }
}

/// `_record_ai_tool_usage(runtime, tool_name, result)`.
pub fn record_tool_usage(state: &mut Value, tool: &PyStr, result: &Value) {
    if let Some(attempted) = list_entry(state, "tools_attempted") {
        attempted.push(Value::Str(tool.clone()));
    }
    if get(result, "ok") != Some(&Value::Bool(true)) {
        let reason = match get(result, "reason") {
            Some(reason) if truthy(reason) => py_str(reason),
            _ => "tool_error".to_owned(),
        };
        if let Some(denials) = list_entry(state, "tool_denials") {
            denials.push(dict([
                ("tool", Value::Str(tool.clone())),
                ("reason", s(reason)),
            ]));
        }
        return;
    }
    if let Some(used) = list_entry(state, "tools_used") {
        used.push(Value::Str(tool.clone()));
    }
    for key in ["auto_journal_process", "auto_sync"] {
        if let Some(envelope @ Value::Dict(_)) = get(result, key) {
            record_provenance_envelope(state, &envelope.clone(), true);
        }
    }
    if let Some(envelope @ Value::Dict(_)) = get(result, "envelope") {
        record_provenance_envelope(state, &envelope.clone(), false);
    }
}

/// The unique strings of `state[key]`, in first-seen order.
fn unique_strings(state: &Value, key: &str) -> Vec<Value> {
    let mut unique: Vec<Value> = Vec::new();
    if let Some(Value::List(items)) = get(state, key) {
        for item in items {
            if matches!(item, Value::Str(_)) && !unique.contains(item) {
                unique.push(item.clone());
            }
        }
    }
    unique
}

fn get_or_null(value: &Value, key: &str) -> Value {
    get(value, key).cloned().unwrap_or(Value::Null)
}

/// `int(record.get("bytes_out") or 0)` for ledger records.
fn bytes_out(record: &Value) -> BigInt {
    match get(record, "bytes_out") {
        Some(Value::Int(n)) => n.clone(),
        Some(Value::Bool(true)) => BigInt::from(1),
        _ => BigInt::from(0),
    }
}

/// `_ai_answer_provenance(provider_snapshot, validated, runtime)`.
///
/// `egress` is the egress ledger snapshot after the turn's cursor
/// (`{"records": [...], "gap": bool}`); `generated_at` is `now_iso()`.
pub fn answer_provenance(
    provider: &Value,
    validated: &Value,
    state: &Value,
    egress: &Value,
    generated_at: &str,
) -> Value {
    let tools_used = unique_strings(state, "tools_used");
    let tools_attempted = unique_strings(state, "tools_attempted");
    let tool_denials: Vec<Value> = match get(state, "tool_denials") {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .cloned()
            .collect(),
        _ => Vec::new(),
    };
    let all_records: &[Value] = match get(egress, "records") {
        Some(Value::List(records)) => records,
        _ => &[],
    };
    let records: Vec<&Value> = all_records
        .iter()
        .filter(|record| matches!(record, Value::Dict(_)))
        .collect();
    let egress_bytes = records
        .iter()
        .fold(BigInt::from(0), |sum, record| sum + bytes_out(record));
    let mut subsystems: Vec<String> = records
        .iter()
        .map(|record| match get(record, "subsystem") {
            Some(subsystem) if truthy(subsystem) => py_str(subsystem),
            _ => "unknown".to_owned(),
        })
        .collect();
    subsystems.sort();
    subsystems.dedup();
    let mut endpoints: Vec<(Value, Value)> = Vec::new();
    for record in &records {
        let host = get_or_null(record, "host");
        if !truthy(&host) {
            continue;
        }
        let endpoint = (host, get_or_null(record, "port"));
        if !endpoints.contains(&endpoint) {
            endpoints.push(endpoint);
        }
    }
    let advertised_tool_count = match get(state, "advertised_tools") {
        Some(Value::List(items)) => Value::from(items.len()),
        _ => Value::Null,
    };
    let notices: Vec<Value> = match get(state, "notices") {
        Some(Value::List(raw)) => PROVIDER_STATUS_NOTICES
            .iter()
            .filter(|phase| {
                raw.iter()
                    .any(|item| matches!(item, Value::Str(text) if text.is(phase)))
            })
            .map(|phase| s(*phase))
            .collect(),
        _ => Vec::new(),
    };
    let raw_attempts = match get(state, "tools_attempted") {
        Some(Value::List(items)) => items.len(),
        _ => 0,
    };
    let screen_route = match get(validated, "screen_context") {
        Some(context @ Value::Dict(_)) => get_or_null(context, "route"),
        _ => Value::Null,
    };
    let cross_book = tools_used
        .iter()
        .any(|tool| matches!(tool, Value::Str(name) if CROSS_BOOK_TOOLS.iter().any(|cross| name.is(cross))));
    dict([
        ("generated_at", s(generated_at)),
        ("provider", get_or_null(provider, "name")),
        ("model", get_or_null(validated, "model")),
        ("notices", Value::List(notices)),
        ("tools_used", Value::List(tools_used.clone())),
        ("tools_attempted", Value::List(tools_attempted)),
        ("tool_denials", Value::List(tool_denials.clone())),
        (
            "active_transactions",
            get_or_null(state, "active_transactions"),
        ),
        ("quarantines", get_or_null(state, "quarantines")),
        (
            "missing_price_transactions",
            get_or_null(state, "missing_price_transactions"),
        ),
        (
            "journals_processed_at",
            get_or_null(state, "journals_processed_at"),
        ),
        (
            "auto_journal_processed",
            Value::Bool(get(state, "auto_journal_processed").is_some_and(truthy)),
        ),
        (
            "auto_sync_attempted",
            Value::Bool(get(state, "auto_sync_attempted").is_some_and(truthy)),
        ),
        ("auto_sync_ok", get_or_null(state, "auto_sync_ok")),
        (
            "privacy_receipt",
            dict([
                ("provider_kind", get_or_null(provider, "kind")),
                (
                    "remote_provider",
                    Value::Bool(
                        !matches!(get(provider, "kind"), Some(Value::Str(kind)) if kind.is("local")),
                    ),
                ),
                ("screen_route", screen_route),
                ("advertised_tool_count", advertised_tool_count),
                ("tools_attempted", Value::from(raw_attempts)),
                ("tools_executed", Value::from(tools_used.len())),
                ("tools_denied", Value::from(tool_denials.len())),
                ("egress_records", Value::from(all_records.len())),
                ("egress_endpoints", Value::from(endpoints.len())),
                ("egress_bytes_out", Value::Int(egress_bytes)),
                (
                    "egress_subsystems",
                    Value::List(subsystems.into_iter().map(s).collect()),
                ),
                (
                    "egress_gap",
                    Value::Bool(get(egress, "gap").is_some_and(truthy)),
                ),
                ("history_intent", get_or_null(validated, "persist")),
                ("hostnames_disclosed_to_model", Value::Bool(false)),
                ("cross_book_data_disclosed", Value::Bool(cross_book)),
            ]),
        ),
    ])
}
