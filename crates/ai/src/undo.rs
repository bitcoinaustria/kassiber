//! `kassiber/ai/undo.py`: undo hints for reversible AI tool results.
//!
//! A reversible mutating tool (`ToolEntry::reversible`) returns
//! `undo: {"tool": <wire name>, "arguments": {...}}`, the call that takes the
//! change back. Hints come only from the tool's own arguments and result;
//! when the result lacks what the inverse needs (a no-op edit, a batch of
//! several components) there is no hint. Plan-bound inverses name their plan
//! tool: the model plans, shows the effect and then applies, as for any
//! change.

use kassiber_envelope::{dict, get};
use kassiber_pyjson::{BigInt, Key, Value};

use crate::pyval::s;

/// `_text(value)`: a non-blank string, stripped.
fn text(value: Option<&Value>) -> Option<Value> {
    match value {
        Some(Value::Str(text)) => {
            let stripped = kassiber_pytext::strip_py(text);
            stripped
                .code_points()
                .next()
                .is_some()
                .then_some(Value::Str(stripped))
        }
        _ => None,
    }
}

/// `_hint(tool, **arguments)`: arguments without the absent ones.
fn hint(tool: &str, arguments: Vec<(&str, Option<Value>)>) -> Option<Value> {
    Some(dict([
        ("tool", s(tool)),
        (
            "arguments",
            Value::Dict(
                arguments
                    .into_iter()
                    .filter_map(|(key, value)| value.map(|value| (Key::from(key), value)))
                    .collect(),
            ),
        ),
    ]))
}

fn mapping(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| matches!(value, Value::Dict(_)))
}

/// `_btc_text(msat)`: `format(Decimal(msat) / MSAT_PER_BTC, "f")` without
/// trailing zeros, for a positive integer; the division keeps Kassiber's 32
/// significant digits (`kassiber/__init__.py`), rounding half to even.
fn btc_text(msat: Option<&Value>) -> Option<Value> {
    const PRECISION: usize = 32;
    let Some(Value::Int(msat)) = msat else {
        return None;
    };
    if *msat <= BigInt::from(0) {
        return None;
    }
    let mut digits: Vec<u8> = msat.to_string().into_bytes();
    let mut exponent: i64 = -11;
    if digits.len() > PRECISION {
        let dropped = digits.split_off(PRECISION);
        exponent += dropped.len() as i64;
        let first = dropped[0];
        let rest_zero = dropped[1..].iter().all(|digit| *digit == b'0');
        let last_odd = (digits[PRECISION - 1] - b'0') % 2 == 1;
        if first > b'5' || (first == b'5' && (!rest_zero || last_odd)) {
            let mut at = digits.len();
            loop {
                at -= 1;
                if digits[at] == b'9' {
                    digits[at] = b'0';
                    if at == 0 {
                        digits.insert(0, b'1');
                        break;
                    }
                } else {
                    digits[at] += 1;
                    break;
                }
            }
        }
    }
    let digits = String::from_utf8(digits).expect("ASCII digits");
    let text = if exponent >= 0 {
        format!("{digits}{}", "0".repeat(exponent as usize))
    } else {
        let places = (-exponent) as usize;
        let padded = format!("{digits:0>width$}", width = places + 1);
        let (whole, fraction) = padded.split_at(padded.len() - places);
        format!("{whole}.{fraction}")
    };
    let text = if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    };
    Some(s(text))
}

/// `_remaining_days(expires_at)`: whole days left on a dismissal; `0` means
/// it never expired, `None` that it already did or cannot be read.
fn remaining_days(expires_at: Option<&Value>, now_us: i64) -> Option<Value> {
    let expires_at = match expires_at {
        None | Some(Value::Null) => return Some(Value::from(0)),
        Some(Value::Str(text)) => crate::pyval::lossy(text),
        Some(_) => return None,
    };
    let moment = kassiber_pytext::iso_datetime(&expires_at.replace('Z', "+00:00"))?;
    // An aware moment minus `datetime.now(timezone.utc)`; a naive one raises
    // TypeError there, which the builder answers with no hint.
    moment.offset_us?;
    let micros = moment.utc_us() - now_us;
    if micros <= 0 {
        return None;
    }
    // `math.ceil(seconds / 86_400)` over float seconds.
    let seconds = micros as f64 / 1_000_000.0;
    Some(Value::from((seconds / 86_400.0).ceil() as i64))
}

fn revert(transaction: Option<&Value>, event: Option<&Value>) -> Option<Value> {
    let (transaction, event) = (text(transaction)?, text(event)?);
    hint(
        "ui_transactions_history_revert",
        vec![("transaction", Some(transaction)), ("event", Some(event))],
    )
}

/// `undo_hint(tool_name, arguments, data)`: the call that takes this tool's
/// change back, or `None`. `now_us` is `datetime.now(timezone.utc)` in
/// microseconds since 0001-01-01.
pub fn undo_hint(tool: &str, arguments: &Value, data: &Value, now_us: i64) -> Option<Value> {
    if !matches!(data, Value::Dict(_)) {
        return None;
    }
    let empty = Value::Dict(Vec::new());
    let arguments = if matches!(arguments, Value::Dict(_)) {
        arguments
    } else {
        &empty
    };
    let id = || text(get(data, "id"));
    match tool {
        "ui.transfers.pair" => hint("ui_transfers_unpair", vec![("pair_id", Some(id()?))]),
        "ui.transfers.unpair" => {
            let pair = mapping(get(data, "pair"))?;
            let out_id = text(get(pair, "out_transaction_id"))?;
            let in_id = text(get(pair, "in_transaction_id"))?;
            hint(
                "ui_transfers_pair",
                vec![
                    ("tx_out", Some(out_id)),
                    ("tx_in", Some(in_id)),
                    ("kind", text(get(pair, "kind"))),
                    ("policy", text(get(pair, "policy"))),
                    ("notes", text(get(pair, "notes"))),
                    ("out_amount", btc_text(get(pair, "out_amount"))),
                ],
            )
        }
        "ui.transfers.dismiss" => hint(
            "ui_transfers_dismissals_delete",
            vec![("dismissal_id", Some(id()?))],
        ),
        "ui.transfers.dismissals.delete" => {
            let dismissal = mapping(get(data, "dismissal"))?;
            let out_id = text(get(dismissal, "out_transaction_id"))?;
            let in_id = text(get(dismissal, "in_transaction_id"))?;
            let days = remaining_days(get(dismissal, "expires_at"), now_us)?;
            hint(
                "ui_transfers_dismiss",
                vec![
                    ("tx_out", Some(out_id)),
                    ("tx_in", Some(in_id)),
                    ("reason", text(get(dismissal, "reason"))),
                    ("expires_in_days", Some(days)),
                ],
            )
        }
        "ui.transfers.rules.create" => {
            hint("ui_transfers_rules_delete", vec![("rule_id", Some(id()?))])
        }
        "ui.transfers.rules.delete" => {
            let rule = mapping(get(data, "rule"))?;
            if get(rule, "invalid_reason").is_some_and(kassiber_envelope::truthy) {
                return None;
            }
            let kind = text(get(rule, "kind"))?;
            let policy = text(get(rule, "policy"))?;
            hint(
                "ui_transfers_rules_create",
                vec![
                    ("name", text(get(rule, "name"))),
                    ("predicate", mapping(get(rule, "predicate")).cloned()),
                    ("kind", Some(kind)),
                    ("policy", Some(policy)),
                    (
                        "enabled",
                        Some(Value::Bool(
                            get(rule, "enabled").is_some_and(kassiber_envelope::truthy),
                        )),
                    ),
                ],
            )
        }
        "ui.transfers.rules.set_enabled" => {
            let rule_id = id()?;
            let Some(Value::Bool(enabled)) = get(data, "enabled") else {
                return None;
            };
            hint(
                "ui_transfers_rules_set_enabled",
                vec![
                    ("rule_id", Some(rule_id)),
                    ("enabled", Some(Value::Bool(!enabled))),
                ],
            )
        }
        "ui.transfers.payouts.create" => hint(
            "ui_transfers_payouts_delete",
            vec![("payout_id", Some(id()?))],
        ),
        "ui.transfers.components.apply" => components(arguments, data),
        "ui.custody.review.apply" => {
            let gap_id = text(get(arguments, "gap_id"))?;
            if get(arguments, "action") == Some(&s("reopen")) {
                return None;
            }
            hint(
                "ui_custody_review_plan",
                vec![("action", Some(s("reopen"))), ("gap_id", Some(gap_id))],
            )
        }
        "ui.saved_views.create" => hint("ui_saved_views_delete", vec![("view_id", Some(id()?))]),
        "ui.loans.mark" => hint(
            "ui_loans_unmark",
            vec![("txid", Some(text(get(arguments, "txid"))?))],
        ),
        "ui.transactions.metadata.update" => {
            if get(data, "updated") == Some(&Value::Bool(false)) {
                return None;
            }
            revert(get(data, "transaction_id"), get(data, "history_event_id"))
        }
        "ui.transactions.history.revert" => {
            if get(data, "updated") == Some(&Value::Bool(false)) {
                return None;
            }
            let transaction = mapping(get(data, "transaction"))
                .and_then(|transaction| get(transaction, "transaction_id"));
            revert(transaction, get(data, "history_event_id"))
        }
        "ui.journals.quarantine.resolve" => {
            let event = mapping(get(data, "resolution"))
                .and_then(|resolution| get(resolution, "history_event_id"));
            revert(get(data, "transaction_id"), event)
        }
        "ui.wallets.import_attachment.apply" => hint(
            "ui_imports_rollback_plan",
            vec![("batch", Some(text(get(data, "import_batch_id"))?))],
        ),
        "ui.chain_analysis.cases.save" => {
            hint("ui_chain_analysis_cases_delete", vec![("id", Some(id()?))])
        }
        "ui.chain_analysis.watches.create" => {
            let watch_id = id()?;
            let Some(revision @ Value::Int(_)) = get(data, "revision") else {
                return None;
            };
            hint(
                "ui_chain_analysis_watches_delete",
                vec![
                    ("id", Some(watch_id)),
                    ("expected_revision", Some(revision.clone())),
                ],
            )
        }
        _ => None,
    }
}

/// `_components(arguments, data)`: retire what was created; restore what a
/// revision or supersede replaced.
fn components(arguments: &Value, data: &Value) -> Option<Value> {
    let create = s("create");
    let action = get(arguments, "action").unwrap_or(&create);
    if matches!(action, Value::List(_) | Value::Dict(_)) {
        // Python's set lookup raises TypeError, which leaves no hint.
        return None;
    }
    let target = text(get(arguments, "component_id"));
    let is = |name: &str| *action == s(name);
    if (is("revise") || is("supersede")) && target.is_some() {
        return hint(
            "ui_transfers_components_plan",
            vec![("action", Some(s("undo"))), ("component_id", target)],
        );
    }
    if is("activate") && target.is_some() {
        return hint(
            "ui_transfers_components_plan",
            vec![("action", Some(s("supersede"))), ("component_id", target)],
        );
    }
    let created: Vec<&Value> = match (get(data, "components"), mapping(get(data, "component"))) {
        (Some(Value::List(items)), _) => items.iter().collect(),
        (_, Some(component)) => vec![component],
        _ => Vec::new(),
    };
    let ids: Vec<Option<Value>> = created
        .into_iter()
        .filter(|item| matches!(item, Value::Dict(_)) && get(item, "state") == Some(&s("active")))
        .map(|item| text(get(item, "id")))
        .collect();
    match ids.as_slice() {
        [Some(id)] => hint(
            "ui_transfers_components_plan",
            vec![
                ("action", Some(s("supersede"))),
                ("component_id", Some(id.clone())),
            ],
        ),
        _ => None,
    }
}
