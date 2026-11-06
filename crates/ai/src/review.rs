//! The AI half of `_review_workflow_payload` and the custody conversion
//! boundary: a batch can never widen the authority of its individual AI
//! operations, and an AI-authored conversion component can neither attest
//! its own review nor activate itself.

use kassiber_envelope::{dict, get, json_ready, s, truthy, AppError};
use kassiber_pyjson::{dumps, Options, Value};

use crate::redact::redact_ai_tool_result;
use crate::validate::py_eq;
use crate::ToolError;

/// `_validate_ai_custody_conversion_boundary(components, activate=...)`.
pub fn conversion_boundary(components: &[Value], activate: bool) -> Result<(), AppError> {
    let conversions: Vec<usize> = components
        .iter()
        .enumerate()
        .filter(|(_, component)| {
            let mode = get(component, "conservation_mode")
                .filter(|mode| truthy(mode))
                .map(kassiber_envelope::py_str)
                .unwrap_or_else(|| "quantity".to_owned());
            mode == "conversion"
        })
        .map(|(index, _)| index)
        .collect();
    if conversions.is_empty() {
        return Ok(());
    }
    let indexes = |indexes: Vec<usize>| Value::List(indexes.into_iter().map(Value::from).collect());
    let attested: Vec<usize> = conversions
        .iter()
        .copied()
        .filter(|&index| get(&components[index], "conversion_reviewed") == Some(&Value::Bool(true)))
        .collect();
    if !attested.is_empty() {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components cannot attest their own review",
        )
        .with_hint(
            "Create the conversion as a draft, then review and activate it in Custody Review.",
        )
        .with_details(dict([("component_indexes", indexes(attested))])));
    }
    if activate {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components require separate human review before activation",
        )
        .with_hint(
            "Retry with activate=false, then review and activate the draft in Custody Review.",
        )
        .with_details(dict([("component_indexes", indexes(conversions))])));
    }
    Ok(())
}

/// `_AI_RESTRICTED_REVIEW_OPERATION_TYPES`: what a restricted provider may
/// plan or apply (component creation only, among custody operations).
pub const RESTRICTED_REVIEW_OPERATION_TYPES: [&str; 4] =
    ["custody_component", "exclude", "price_override", "unpair"];

/// Looks a custody component up in the active profile
/// (`get_component(..., include_local_evidence=False)`).
pub type Lookup<'a> = dyn FnMut(&str) -> Result<Value, AppError> + 'a;

/// The lookup `validate_operations` borrows.
pub type ComponentLookup<'a> = &'a mut Lookup<'a>;

/// Python's `value in {...}` over strings: `None` where hashing raises.
fn in_strings(value: Option<&Value>, set: &[&str]) -> Option<bool> {
    match value {
        Some(Value::List(_) | Value::Dict(_)) => None,
        Some(Value::Str(text)) => Some(set.iter().any(|item| text.is(item))),
        _ => Some(false),
    }
}

/// `_validate_ai_review_operations(operations, full_access=..., conn=...)`:
/// a batch cannot widen the authority of its individual AI operations.
/// `lookup` is `None` where Python has no connection. `Unexpected` where
/// Python raises `TypeError` (an unhashable type or action).
pub fn validate_operations(
    operations: Option<&Value>,
    full_access: bool,
    mut lookup: Option<ComponentLookup<'_>>,
) -> Result<(), ToolError> {
    let Some(Value::List(items)) = operations else {
        return Err(AppError::new("validation", "Review operations must be a list").into());
    };
    let list = Value::List(items.clone());
    if !py_eq(&redact_ai_tool_result(&list), &list) {
        return Err(AppError::new(
            "validation",
            "Review operations contain private paths, URLs or secret material",
        )
        .with_hint("Use local evidence identifiers and a brief audit reason instead.")
        .into());
    }
    let create = s("create");
    for operation in items {
        if !matches!(operation, Value::Dict(_)) {
            return Err(AppError::new("validation", "Review operation must be an object").into());
        }
        let kind = get(operation, "type");
        let request = get(operation, "request").filter(|request| matches!(request, Value::Dict(_)));
        let action = request.map(|request| get(request, "action").unwrap_or(&create));
        let component = kind.is_some_and(|kind| py_eq(kind, &s("custody_component")));
        if !full_access {
            let allowed = in_strings(kind, &RESTRICTED_REVIEW_OPERATION_TYPES)
                .ok_or(ToolError::Unexpected)?;
            if !allowed || (component && !action.is_some_and(|action| py_eq(action, &create))) {
                return Err(AppError::new(
                    "agent_access_restricted",
                    "This review operation is unavailable while this provider's agent access is restricted",
                )
                .with_hint("Set the provider's agent access to full in Settings → Assistant, or use the desktop workflow.")
                .with_details(dict([("type", kind.cloned().unwrap_or(Value::Null))]))
                .into());
            }
        }
        if !component {
            continue;
        }
        let (Some(request), Some(action)) = (request, action) else {
            return Err(
                AppError::new("validation", "Custody review requires a typed request").into(),
            );
        };
        if py_eq(action, &create) {
            let Some(Value::List(components)) = get(request, "components").filter(|components| {
                matches!(components, Value::List(items) if items.iter().all(|c| matches!(c, Value::Dict(_))))
            }) else {
                return Err(AppError::new("validation", "Custody components must be objects").into());
            };
            let activate = get(request, "activate").is_none_or(truthy);
            conversion_boundary(components, activate)?;
        } else if in_strings(Some(action), &["revise", "activate"]).ok_or(ToolError::Unexpected)? {
            component_conversion_change(request, action, lookup.as_deref_mut())?;
        }
    }
    Ok(())
}

/// `_validate_ai_component_conversion_change(conn, request, action)`: an AI
/// revision or activation never makes a conversion effective.
fn component_conversion_change<'a>(
    request: &Value,
    action: &Value,
    lookup: Option<&mut Lookup<'a>>,
) -> Result<(), AppError> {
    let empty = Value::Dict(Vec::new());
    let spec = get(request, "spec")
        .filter(|spec| matches!(spec, Value::Dict(_)))
        .unwrap_or(&empty);
    if get(spec, "conversion_reviewed") == Some(&Value::Bool(true)) {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components cannot attest their own review",
        )
        .with_hint(
            "Revise the conversion as a draft, then review and activate it in Custody Review.",
        ));
    }
    let mut mode = get(spec, "conservation_mode")
        .cloned()
        .unwrap_or(Value::Null);
    if mode == Value::Null {
        if let (Some(lookup), Some(Value::Str(id))) = (lookup, get(request, "component_id")) {
            match lookup(&kassiber_envelope::py_str(&Value::Str(
                kassiber_pytext::strip_py(id),
            ))) {
                // The core reports the missing component on planning.
                Err(_) => return Ok(()),
                Ok(component) => {
                    mode = get(&component, "conservation_mode")
                        .cloned()
                        .unwrap_or(Value::Null);
                }
            }
        }
    }
    let mode = if truthy(&mode) {
        kassiber_envelope::py_str(&mode)
    } else {
        "quantity".to_owned()
    };
    if mode != "conversion" {
        return Ok(());
    }
    if py_eq(action, &s("activate")) || get(request, "activate") == Some(&Value::Bool(true)) {
        return Err(AppError::new(
            "interaction_required",
            "AI-authored conversion components require separate human review before activation",
        )
        .with_hint("Keep the revision a draft, then review and activate it in Custody Review."));
    }
    if get(spec, "conversion_reviewed") != Some(&Value::Bool(false)) {
        return Err(AppError::new(
            "interaction_required",
            "An AI conversion revision must clear the inherited review",
        )
        .with_hint(
            "Set spec.conversion_reviewed to false; a human reviews the draft in Custody Review.",
        ));
    }
    Ok(())
}

/// Whether a stored receipt holds an operation a restricted provider may
/// not see (`_review_workflow_payload`'s receipt gate).
pub fn receipt_restricted(receipt: &Value) -> Result<bool, ToolError> {
    let Some(Value::List(operations)) = get(receipt, "proposed_operations") else {
        return Ok(false);
    };
    for operation in operations {
        if !in_strings(get(operation, "type"), &RESTRICTED_REVIEW_OPERATION_TYPES)
            .ok_or(ToolError::Unexpected)?
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The operations a review request carries: `args["artifact"]["operations"]`
/// for an artifact, otherwise `args["operations"]`.
pub fn operations(args: &Value) -> Option<&Value> {
    match get(args, "artifact") {
        Some(artifact @ Value::Dict(_)) => get(artifact, "operations"),
        _ => get(args, "operations"),
    }
}

/// The checks an AI-planned artifact passes before it reaches the model: it
/// must fit a later `ui.review.apply` call and contain nothing the result
/// redaction would change.
pub fn check_planned_artifact(payload: &Value) -> Result<(), AppError> {
    let probe = dict([
        ("artifact", json_ready(payload.clone())),
        ("idempotency_key", s("x".repeat(200))),
    ]);
    let size = dumps(&probe, &Options::new().separators(",", ":"))
        .map_err(|error| AppError::new("internal_error", error.to_string()))?
        .chars()
        .count();
    if size > 65_536 {
        return Err(AppError::new(
            "validation",
            "Review artifact exceeds the AI tool argument limit",
        )
        .with_hint(
            "Plan fewer operations in each review batch, or name fewer transaction_ids in accept_coarse_prices.",
        ));
    }
    if !py_eq(&redact_ai_tool_result(payload), payload) {
        return Err(AppError::new(
            "interaction_required",
            "This review artifact requires the local CLI workflow",
        )
        .with_hint("Its private fields cannot be sent unchanged to an AI provider."));
    }
    Ok(())
}
