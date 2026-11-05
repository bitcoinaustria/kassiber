//! The AI half of `_review_workflow_payload` and the custody conversion
//! boundary: a batch can never widen the authority of its individual AI
//! operations, and an AI-authored conversion component can neither attest
//! its own review nor activate itself.

use kassiber_envelope::{dict, get, json_ready, s, truthy, AppError};
use kassiber_pyjson::{dumps, Options, Value};

use crate::redact::redact_ai_tool_result;
use crate::validate::py_eq;

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

/// `_validate_ai_review_operations(operations)`.
pub fn validate_operations(operations: Option<&Value>) -> Result<(), AppError> {
    let Some(Value::List(items)) = operations else {
        return Err(AppError::new(
            "validation",
            "Review operations must be a list",
        ));
    };
    let list = Value::List(items.clone());
    if !py_eq(&redact_ai_tool_result(&list), &list) {
        return Err(AppError::new(
            "validation",
            "Review operations contain private paths, URLs or secret material",
        )
        .with_hint("Use local evidence identifiers and a brief audit reason instead."));
    }
    for operation in items {
        if !matches!(operation, Value::Dict(_)) {
            return Err(AppError::new(
                "validation",
                "Review operation must be an object",
            ));
        }
        let kind = get(operation, "type");
        if kind == Some(&s("kind_override")) {
            return Err(AppError::new(
                "interaction_required",
                "Acquisition effect review is a local desktop/CLI workflow",
            ));
        }
        if kind != Some(&s("custody_component")) {
            continue;
        }
        let request = get(operation, "request");
        let creates = match request {
            Some(request @ Value::Dict(_)) => match get(request, "action") {
                None => true,
                Some(action) => py_eq(action, &s("create")),
            },
            _ => false,
        };
        if !creates {
            return Err(AppError::new(
                "interaction_required",
                "AI review batches may create custody components only",
            )
            .with_hint("Review existing component revisions in the local custody workflow."));
        }
        let request = request.expect("checked above");
        let Some(Value::List(components)) = get(request, "components")
            .filter(|components| matches!(components, Value::List(items) if items.iter().all(|c| matches!(c, Value::Dict(_)))))
        else {
            return Err(AppError::new("validation", "Custody components must be objects"));
        };
        let activate = get(request, "activate").is_none_or(truthy);
        conversion_boundary(components, activate)?;
    }
    Ok(())
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
        .with_hint("Plan fewer operations in each review batch."));
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
