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
    book.review(&operation, args, "gui")
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
