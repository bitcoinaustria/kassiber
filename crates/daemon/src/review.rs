//! `_review_workflow_payload` for desktop requests. Cases, input requests and
//! plans write nothing; apply is authored `gui`. Receipts are historical and
//! never certify current readiness.
use crate::output::{with_request_id, Output, RequestId};
use kassiber_envelope::{dict, s, success, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::{Book, ReviewPlanJob};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

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
            &["operations", "expected_input_version", "preview"],
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

/// A desktop `ui.review.plan` sent with `preview: true`: checked like any
/// plan and taken off the book now, computed by the returned job on a
/// worker. `None` for every other request, which `execute` answers.
pub fn preview_job(
    book: &mut dyn Book,
    kind: &str,
    args: Option<&Value>,
) -> Result<Option<Box<dyn ReviewPlanJob>>, AppError> {
    if kind != "ui.review.plan" {
        return Ok(None);
    }
    let Some(args @ Value::Dict(_)) = args else {
        return Ok(None);
    };
    match kassiber_envelope::get(args, "preview") {
        Some(Value::Bool(true)) => {}
        None | Some(Value::Bool(false)) => return Ok(None),
        Some(_) => {
            return Err(AppError::new(
                "validation",
                "Review preview must be true or false",
            ))
        }
    }
    admit(kind, args)?;
    Ok(Some(book.review_plan_job(&without_preview(args))?))
}

/// The arguments the review core reads: `preview` only chooses how the
/// daemon runs a plan.
fn without_preview(args: &Value) -> Value {
    match args {
        Value::Dict(entries) => Value::Dict(
            entries
                .iter()
                .filter(|(key, _)| *key != "preview".into())
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Refuse arguments the kind does not take, then missing required ones.
fn admit(kind: &str, args: &Value) -> Result<(), AppError> {
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
    Ok(())
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
    admit(kind, args)?;
    // `preview` only chooses how the desktop's plan runs; the assistant's
    // plans refuse it as they refuse any argument they do not take.
    if source == "ai_tool" && kassiber_envelope::get(args, "preview").is_some() {
        return Err(AppError::new("validation", "Unsupported review arguments"));
    }
    let args = &without_preview(args);
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

/// Desktop review previews running on workers. A newer preview cancels the
/// ones still running, as the desktop only shows the latest; closing the
/// book or stopping the daemon cancels them and waits for each to answer.
/// A cancelled preview answers `review_plan_cancelled` at its next step.
#[derive(Default)]
pub(crate) struct Previews {
    running: Vec<(Arc<AtomicBool>, std::thread::JoinHandle<()>)>,
}

impl Previews {
    /// Cancel every running preview without waiting for it.
    pub(crate) fn cancel(&mut self) {
        for (cancelled, _) in &self.running {
            cancelled.store(true, Ordering::SeqCst);
        }
        self.running.retain(|(_, worker)| !worker.is_finished());
    }

    /// Cancel every running preview and wait until each has answered.
    pub(crate) fn stop(&mut self) {
        self.cancel();
        for (_, worker) in self.running.drain(..) {
            let _ = worker.join();
        }
    }

    /// Run `job` on a worker that answers `request_id` itself, as the
    /// request loop would have answered it.
    pub(crate) fn start(
        &mut self,
        job: Box<dyn ReviewPlanJob>,
        output: Output,
        request_id: RequestId,
    ) -> Result<(), AppError> {
        let cancelled = Arc::new(AtomicBool::new(false));
        let flag = cancelled.clone();
        let worker = std::thread::Builder::new()
            .name("kassiber-review-preview".into())
            .spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    job.run(&|| flag.load(Ordering::SeqCst))
                }));
                let record = match outcome {
                    Ok(Ok(payload)) => {
                        with_request_id(success("ui.review.plan", payload), &request_id)
                    }
                    // A non-AppError reference failure is unhandled, as on the request loop.
                    Ok(Err(error)) if error.code == "internal_error" => {
                        crate::unhandled_error_envelope(&error, &request_id)
                    }
                    Ok(Err(error)) => {
                        crate::app_error_envelope(&error, &request_id, Some("ui.review.plan"))
                    }
                    Err(panic) => {
                        let message = panic
                            .downcast_ref::<&str>()
                            .map(|m| (*m).to_owned())
                            .or_else(|| panic.downcast_ref::<String>().cloned())
                            .unwrap_or_else(|| "panic".to_owned());
                        // As on the request loop; not eprintln!, so a closed
                        // stderr cannot panic again outside catch_unwind.
                        let _ = std::io::Write::write_fmt(
                            &mut std::io::stderr(),
                            format_args!("kassiber daemon: review preview panicked: {message}\n"),
                        );
                        crate::error_envelope(
                            "internal_error",
                            &message,
                            &request_id,
                            None,
                            None,
                            false,
                        )
                    }
                };
                let _ = output.write(&record);
            })
            .map_err(|_| AppError::new("internal_error", "Could not start the review preview"))?;
        self.running.push((cancelled, worker));
        Ok(())
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

    /// The plan arguments the book was asked to take a preview off.
    struct Planned(Arc<std::sync::Mutex<Vec<Value>>>);
    impl ReviewPlanJob for Planned {
        fn run(self: Box<Self>, _: &(dyn Fn() -> bool + Sync)) -> Result<Value, AppError> {
            Ok(dict([("digest", s("planned"))]))
        }
    }
    impl Book for Planned {
        fn review_plan_job(&mut self, args: &Value) -> Result<Box<dyn ReviewPlanJob>, AppError> {
            self.0.lock().unwrap().push(args.clone());
            Ok(Box::new(Planned(self.0.clone())))
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
    fn only_a_desktop_plan_marked_preview_becomes_a_job_and_the_core_never_sees_the_mark() {
        let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
        let plan = |preview: Option<Value>| {
            let mut args = dict([
                ("operations", Value::List(vec![])),
                ("expected_input_version", Value::from(3)),
            ]);
            if let Some(preview) = preview {
                kassiber_envelope::set(&mut args, "preview", preview);
            }
            args
        };
        let mut book = Planned(asked.clone());
        for args in [plan(None), plan(Some(Value::Bool(false)))] {
            assert!(preview_job(&mut book, "ui.review.plan", Some(&args))
                .unwrap()
                .is_none());
        }
        assert!(preview_job(
            &mut book,
            "ui.review.cases",
            Some(&plan(Some(Value::Bool(true))))
        )
        .unwrap()
        .is_none());
        let error = preview_job(&mut book, "ui.review.plan", Some(&plan(Some(s("yes")))))
            .err()
            .unwrap();
        assert_eq!(error.message, "Review preview must be true or false");
        let mut extra = plan(Some(Value::Bool(true)));
        kassiber_envelope::set(&mut extra, "authored_source", s("ai_tool"));
        let error = preview_job(&mut book, "ui.review.plan", Some(&extra))
            .err()
            .unwrap();
        assert_eq!(error.message, "Unsupported review arguments");
        let mut missing = plan(Some(Value::Bool(true)));
        if let Value::Dict(entries) = &mut missing {
            entries.retain(|(key, _)| *key != "expected_input_version".into());
        }
        let error = preview_job(&mut book, "ui.review.plan", Some(&missing))
            .err()
            .unwrap();
        assert_eq!(error.message, "Missing required review arguments");
        assert!(asked.lock().unwrap().is_empty());
        let job = preview_job(
            &mut book,
            "ui.review.plan",
            Some(&plan(Some(Value::Bool(true)))),
        )
        .unwrap()
        .unwrap();
        assert_eq!(*asked.lock().unwrap(), vec![plan(None)]);
        assert_eq!(
            job.run(&|| false).unwrap(),
            dict([("digest", s("planned"))])
        );
        // A plain plan reaches the core without the mark too.
        let out = execute(
            &mut Capture,
            "ui.review.plan",
            Some(&plan(Some(Value::Bool(false)))),
        )
        .unwrap();
        assert_eq!(get(&out, "args"), Some(&plan(None)));
        // The assistant's plans refuse the mark, as they did before it existed.
        for preview in [Value::Bool(true), Value::Bool(false)] {
            let error = execute_as(
                &mut Capture,
                "ui.review.plan",
                Some(&plan(Some(preview))),
                "ai_tool",
                true,
            )
            .unwrap_err();
            assert_eq!(error.message, "Unsupported review arguments");
        }
    }

    #[derive(Clone, Default)]
    struct Lines(Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// A plan that runs until it is cancelled, then answers as the core does.
    struct Endless(Arc<AtomicBool>);
    impl ReviewPlanJob for Endless {
        fn run(self: Box<Self>, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<Value, AppError> {
            self.0.store(true, Ordering::SeqCst);
            while !cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(AppError::new("review_plan_cancelled", "cancelled"))
        }
    }

    #[test]
    fn a_newer_preview_cancels_the_running_one_and_stopping_waits_for_every_answer() {
        let lines = Lines::default();
        let output = Output::new(Box::new(lines.clone()));
        let mut previews = Previews::default();
        let started = Arc::new(AtomicBool::new(false));
        previews
            .start(
                Box::new(Endless(started.clone())),
                output.clone(),
                RequestId::Value(s("old")),
            )
            .unwrap();
        while !started.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        previews.cancel();
        previews
            .start(
                Box::new(Planned(Default::default())),
                output.clone(),
                RequestId::Value(s("new")),
            )
            .unwrap();
        let last = Arc::new(AtomicBool::new(false));
        previews
            .start(
                Box::new(Endless(last.clone())),
                output,
                RequestId::Value(s("closing")),
            )
            .unwrap();
        while !last.load(Ordering::SeqCst) {
            std::thread::yield_now();
        }
        previews.stop();
        assert!(previews.running.is_empty());
        let text = String::from_utf8(lines.0.lock().unwrap().clone()).unwrap();
        let mut records: Vec<Value> = text
            .lines()
            .map(|line| kassiber_pytext::loads(line).unwrap())
            .collect();
        records.sort_by_key(|record| kassiber_envelope::py_str(get(record, "request_id").unwrap()));
        let answer = |record: &Value| {
            (
                kassiber_envelope::py_str(get(record, "request_id").unwrap()),
                kassiber_envelope::py_str(get(record, "kind").unwrap()),
                get(record, "error")
                    .and_then(|error| get(error, "code"))
                    .map(kassiber_envelope::py_str),
            )
        };
        assert_eq!(
            records.iter().map(answer).collect::<Vec<_>>(),
            [
                (
                    "closing".to_owned(),
                    "error".to_owned(),
                    Some("review_plan_cancelled".to_owned())
                ),
                ("new".to_owned(), "ui.review.plan".to_owned(), None),
                (
                    "old".to_owned(),
                    "error".to_owned(),
                    Some("review_plan_cancelled".to_owned())
                ),
            ]
        );
    }
}
