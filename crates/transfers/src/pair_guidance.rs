//! `custody_review_terms.pair_review_guidance`: explain a legacy pair fee
//! hold without revising any authored fact.
use super::*;
use kassiber_envelope::{s, truthy};
use std::collections::BTreeSet;

pub const LEGACY_PAIR_FEE: &str = "legacy_pair_fee_allocation_review_required";

/// The pair's validation codes and, for a legacy fee hold, the concrete
/// repair: the pair update command, the unpair alternative and, for a simple
/// pair, the `repair_legacy_pair` review operation.
pub fn pair_review_guidance(conn: &Connection, scope: &Scope<'_>, pair: &Value) -> Result<Value> {
    Ok(guidance(&authored_active_components(conn, scope)?, pair))
}

/// [`pair_review_guidance`] over components the caller already loaded, so a
/// pair list reads them once.
pub(crate) fn guidance(components: &[serde_json::Value], pair: &Value) -> Value {
    let component_id = py_str(&get(pair, "component_id").cloned().unwrap_or(Value::Null));
    let component = components
        .iter()
        .find(|c| c["id"].as_str() == Some(component_id.as_str()));
    let codes: BTreeSet<String> = component
        .and_then(|c| c["validation"]["issues"].as_array())
        .map(|issues| {
            issues
                .iter()
                .filter_map(|issue| issue["code"].as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let mut result = vec![
        ("review_required", Value::Bool(!codes.is_empty())),
        (
            "review_codes",
            Value::List(codes.iter().map(|code| s(code.as_str())).collect()),
        ),
    ];
    if codes.contains(LEGACY_PAIR_FEE) {
        let pair_id = py_str(&get(pair, "id").cloned().unwrap_or(Value::Null));
        let simple = component
            .and_then(|c| c["economic_terms"].as_array())
            .is_some_and(|terms| terms.len() == 1);
        result.push((
            "repair",
            dict([
                ("code", s(LEGACY_PAIR_FEE)),
                ("pair_id", s(pair_id.as_str())),
                (
                    "command",
                    s(format!(
                        "transfers update --pair-id {pair_id} --note \"Reviewed complete debit and residual\""
                    )),
                ),
                (
                    "alternative",
                    s(format!(
                        "transfers unpair --pair-id {pair_id}, then transfers pair with the reviewed legs"
                    )),
                ),
                (
                    "preview_operation",
                    if simple { s("repair_legacy_pair") } else { Value::Null },
                ),
            ]),
        ));
    }
    dict(result)
}

/// Whether guidance offers a repair.
pub fn repair(guidance: &Value) -> Option<&Value> {
    get(guidance, "repair").filter(|value| truthy(value))
}
