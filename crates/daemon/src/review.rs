//! `_review_workflow_payload` for desktop requests. Cases, input requests and
//! plans write nothing; apply is authored `gui`. Receipts are historical and
//! never certify current readiness.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.review.cases",
    "ui.review.request_input",
    "ui.review.plan",
    "ui.review.apply",
    "ui.review.receipt",
];

fn fields(kind: &str) -> (&'static [&'static str], &'static [&'static str]) {
    match kind {
        "ui.review.cases" => (&["limit", "cursor"], &[]),
        "ui.review.request_input" => (
            &[
                "action",
                "case_ids",
                "expected_input_version",
                "explanation",
            ],
            &["action", "case_ids", "expected_input_version"],
        ),
        "ui.review.plan" => (
            &["operations", "expected_input_version"],
            &["operations", "expected_input_version"],
        ),
        "ui.review.apply" => (
            &["artifact", "idempotency_key"],
            &["artifact", "idempotency_key"],
        ),
        _ => (&["receipt_id", "idempotency_key"], &[]),
    }
}

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    execute_as(book, kind, args, "gui", false)
}

/// The AI rules' `TypeError` (an unhashable operation type or action), which
/// the AI executor answers as an unexpected failure.
fn ai_rules(error: kassiber_ai::ToolError) -> AppError {
    match error {
        kassiber_ai::ToolError::App(error) => error,
        _ => AppError::new("internal_error", "review operation check failed"),
    }
}

/// `get_component(conn, id, profile_id=..., include_local_evidence=False)`
/// for the AI conversion checks.
fn component(book: &mut dyn Book, id: &str) -> Result<Value, AppError> {
    crate::transfers::execute(
        book,
        "ui.transfers.components.get",
        Some(&dict([("component_id", s(id))])),
    )
}

fn validate_ai_operations(
    book: &mut dyn Book,
    operations: Option<&Value>,
    full_access: bool,
) -> Result<(), AppError> {
    let mut lookup = |id: &str| component(&mut *book, id);
    kassiber_ai::review::validate_operations(operations, full_access, Some(&mut lookup))
        .map_err(ai_rules)
}

/// `_review_workflow_payload(..., authored_source=...)`. An `ai_tool` call
/// stores a redacted explanation, may only plan or apply operations within
/// the AI's own authority (`full_access` is the provider's agent access),
/// and gets a plan back only if it can be sent to the provider unchanged.
pub fn execute_as(
    book: &mut dyn Book,
    kind: &str,
    args: Option<&Value>,
    source: &str,
    full_access: bool,
) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(entries) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let (allowed, required) = fields(kind);
    let named = |key: &kassiber_pyjson::Key, field: &str| *key == field.into();
    if entries
        .iter()
        .any(|(key, _)| !allowed.iter().any(|field| named(key, field)))
    {
        return Err(AppError::new("validation", "Unsupported review arguments"));
    }
    if required
        .iter()
        .any(|field| !entries.iter().any(|(key, _)| named(key, field)))
    {
        return Err(AppError::new(
            "validation",
            "Missing required review arguments",
        ));
    }
    let operation = format!("review.{}", kind.trim_start_matches("ui.review."));
    if source != "ai_tool" {
        return book.review(&operation, args, source);
    }
    book.resolve_scope(None, None)?;
    match kind {
        "ui.review.cases" => book.review(&operation, args, source),
        "ui.review.request_input" => book.review(
            &operation,
            &kassiber_ai::redact::redact_ai_tool_result(args),
            source,
        ),
        "ui.review.receipt" => {
            let receipt = book.review(&operation, args, source)?;
            if !full_access
                && kassiber_ai::review::receipt_restricted(&receipt).map_err(ai_rules)?
            {
                return Err(AppError::new(
                    "agent_access_restricted",
                    "This review receipt is unavailable while this provider's agent access is restricted",
                ));
            }
            Ok(receipt)
        }
        _ => {
            validate_ai_operations(book, kassiber_ai::review::operations(args), full_access)?;
            let payload = book.review(&operation, args, source)?;
            if kind == "ui.review.plan" {
                kassiber_ai::review::check_planned_artifact(&payload)?;
            }
            Ok(payload)
        }
    }
}

/// `_ai_review_consent_preview`'s scoped body: the accounting effects shown
/// before an AI `ui.review.apply` are recomputed here, never taken from the
/// model's artifact. A receipt under the same idempotency key reads as
/// already applied.
pub fn consent_preview(
    book: &mut dyn Book,
    arguments: &Value,
    full_access: bool,
) -> Result<Value, AppError> {
    use kassiber_envelope::{get, truthy};
    let artifact = match get(arguments, "artifact") {
        Some(artifact @ Value::Dict(_)) => artifact.clone(),
        _ => return Err(AppError::new("validation", "Review artifact is required")),
    };
    validate_ai_operations(book, get(&artifact, "operations"), full_access)?;
    book.resolve_scope(None, None)?;
    if let Some(key) = get(arguments, "idempotency_key").filter(|key| truthy(key)) {
        match book.review(
            "review.receipt",
            &dict([("idempotency_key", key.clone())]),
            "ai_tool",
        ) {
            Err(error) if error.code == "not_found" => {}
            Err(error) => return Err(error),
            Ok(receipt) => {
                if get(&receipt, "artifact_digest")
                    != get(&artifact, "digest").or(Some(&Value::Null))
                {
                    return Err(AppError::new(
                        "review_idempotency_conflict",
                        "Idempotency key belongs to another plan",
                    ));
                }
                return Ok(dict([("status", s("applied")), ("receipt", receipt)]));
            }
        }
    }
    let validated = book.review(
        "review.validate",
        &dict([("artifact", artifact)]),
        "ai_tool",
    )?;
    if !kassiber_ai::validate::py_eq(
        &kassiber_ai::redact::redact_ai_tool_result(&validated),
        &validated,
    ) {
        return Err(AppError::new(
            "interaction_required",
            "Review artifact requires local review",
        ));
    }
    Ok(dict([("status", s("ready")), ("artifact", validated)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_envelope::get;
    use kassiber_runtime::store::{BookContext, StatusFacts};

    struct Capture;
    impl Book for Capture {
        fn review(
            &mut self,
            operation: &str,
            args: &Value,
            authored_source: &str,
        ) -> Result<Value, AppError> {
            Ok(dict([
                ("operation", s(operation)),
                ("args", args.clone()),
                ("authored_source", s(authored_source)),
            ]))
        }
        fn resolve_scope(
            &self,
            _: Option<&Value>,
            _: Option<&Value>,
        ) -> Result<(String, String), AppError> {
            unreachable!()
        }
        fn is_initialized(&self) -> Result<bool, AppError> {
            unreachable!()
        }
        fn context(&self) -> Result<BookContext, AppError> {
            unreachable!()
        }
        fn status_facts(&self) -> Result<StatusFacts, AppError> {
            unreachable!()
        }
        fn rollback(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
        fn close(&mut self) -> Result<(), AppError> {
            unreachable!()
        }
    }

    #[test]
    fn desktop_review_arguments_refuse_in_python_order_and_author_gui() {
        let complete = [
            ("ui.review.cases", dict::<&str>([])),
            (
                "ui.review.request_input",
                dict([
                    ("action", s("x")),
                    ("case_ids", Value::Null),
                    ("expected_input_version", Value::Null),
                ]),
            ),
            (
                "ui.review.plan",
                dict([
                    ("operations", Value::Null),
                    ("expected_input_version", Value::Null),
                ]),
            ),
            (
                "ui.review.apply",
                dict([("artifact", Value::Null), ("idempotency_key", Value::Null)]),
            ),
            ("ui.review.receipt", dict::<&str>([])),
        ];
        assert_eq!(
            KINDS,
            complete
                .iter()
                .map(|(kind, _)| *kind)
                .collect::<Vec<_>>()
                .as_slice()
        );
        for (kind, args) in complete {
            let out = execute(&mut Capture, kind, Some(&args)).unwrap();
            assert_eq!(
                get(&out, "operation"),
                Some(&s(kind.replace("ui.review.", "review.")))
            );
            assert_eq!(get(&out, "authored_source"), Some(&s("gui")));
            assert_eq!(get(&out, "args"), Some(&args));
            // An unknown field refuses before a missing required one.
            let mut extra = args.clone();
            kassiber_envelope::set(&mut extra, "authored_source", s("ai_tool"));
            if let Value::Dict(entries) = &mut extra {
                entries.retain(|(key, _)| *key == "authored_source".into());
            }
            let error = execute(&mut Capture, kind, Some(&extra)).unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                ("validation", "Unsupported review arguments")
            );
            if let Value::Dict(entries) = &args {
                for index in 0..entries.len() {
                    let mut missing = args.clone();
                    if let Value::Dict(entries) = &mut missing {
                        entries.remove(index);
                    }
                    let error = execute(&mut Capture, kind, Some(&missing)).unwrap_err();
                    assert_eq!(error.message, "Missing required review arguments");
                }
            }
        }
        let error =
            execute(&mut Capture, "ui.review.cases", Some(&Value::List(vec![]))).unwrap_err();
        assert_eq!(error.message, "daemon args must be an object");
        assert!(execute(&mut Capture, "ui.review.cases", Some(&Value::Null)).is_ok());
    }
}
