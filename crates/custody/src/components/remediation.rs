//! `custody_components.membership_conflict_remediation` and
//! `issues_remediation`: what resolves a blocking component issue, on which
//! records, with which tool (`errors.build_remediation`'s shape).
use crate::codec::{pystr, truth};
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

/// The AI tool that plans component revisions, supersessions and merges.
pub const COMPONENT_PLAN_TOOL: &str = "ui_transfers_components_plan";

/// `build_remediation(action, target=..., tool=..., arguments_hint=...,
/// message=..., alternatives=...)` with `desktop_only` false.
pub fn build(
    action: &str,
    target: Value,
    tool: Option<&str>,
    arguments_hint: Value,
    message: &str,
    alternatives: Vec<Value>,
) -> Value {
    let mut remediation = Map::new();
    remediation.insert("action".into(), json!(action));
    remediation.insert("target".into(), target);
    remediation.insert("tool".into(), json!(tool));
    remediation.insert("arguments_hint".into(), arguments_hint);
    remediation.insert("desktop_only".into(), json!(false));
    if !message.is_empty() {
        remediation.insert("message".into(), json!(message));
    }
    if !alternatives.is_empty() {
        remediation.insert("alternatives".into(), Value::Array(alternatives));
    }
    Value::Object(remediation)
}

/// Name the active component(s) that already interpret these anchors: revise
/// the conflicting lineage with `activate`, or supersede it.
pub fn membership_conflict(conflicts: &[Value]) -> Value {
    let mut component_ids = BTreeSet::new();
    let mut transaction_ids = BTreeSet::new();
    for conflict in conflicts {
        match conflict.get("component_id").filter(|v| truth(v)) {
            Some(id) => {
                component_ids.insert(pystr(id));
            }
            None => {
                for id in conflict
                    .get("component_ids")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    component_ids.insert(pystr(id));
                }
            }
        }
        if let Some(id) = conflict.get("transaction_id").filter(|v| truth(v)) {
            transaction_ids.insert(pystr(id));
        }
    }
    let first = component_ids.iter().next().cloned();
    let mut message = String::from(
        "These transactions already belong to an active custody component. To replace that interpretation, revise the conflicting component (same anchors allowed) with activate=true; to retire it without a replacement, supersede it.",
    );
    if component_ids.len() > 1 {
        message.push_str(" Several components conflict: supersede all but one, then revise the remaining one to the full corrected interpretation.");
    }
    build(
        "revise_conflicting_component",
        json!({"component_ids": component_ids, "transaction_ids": transaction_ids}),
        Some(COMPONENT_PLAN_TOOL),
        json!({
            "action": "revise",
            "component_id": first,
            "activate": true,
            "spec": {"legs": "<corrected legs>", "allocations": "<if N:M>"},
        }),
        &message,
        vec![json!({
            "action": "supersede",
            "tool": COMPONENT_PLAN_TOOL,
            "arguments_hint": {
                "action": "supersede",
                "component_id": first,
                "reason": "<why the old interpretation was wrong>",
            },
        })],
    )
}

/// `_membership_conflict_issue(conflicts, message)`.
pub fn membership_conflict_issue(conflicts: Vec<Value>, message: &str) -> Value {
    let remediation = membership_conflict(&conflicts);
    json!({
        "code": "active_transaction_membership_conflict",
        "message": message,
        "conflicts": conflicts,
        "remediation": remediation,
    })
}
