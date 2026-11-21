//! Silent Payments scan readiness: a configured BIP352 wallet whose scan
//! range has not completed, or is degraded, blocks final reports. The
//! report-readiness snapshot (`kassiber-journals`) lists these blockers and
//! final export admission ([`crate::export_request`]) refuses on them, so
//! both read this one check.
use super::*;

fn text_or(row: &Value, key: &str, default: &str) -> String {
    match get(row, key) {
        Some(value) if truthy(value) => py_str(value),
        _ => default.to_owned(),
    }
}

/// `_silent_payment_report_blockers`: configured BIP352 wallets whose scan
/// range has not completed or is degraded.
pub fn silent_payment_blockers(conn: &Connection, profile_id: &str) -> Result<Vec<Value>> {
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
        let id = py_str(value(&wallet, "id"));
        let label = py_str(value(&wallet, "label"));
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
