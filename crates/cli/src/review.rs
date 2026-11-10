//! `kassiber review`: cases, plans, applications and receipts over the shared
//! review service. Applied history is authored `cli`.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use std::io::Read;

const MAX_REVIEW_FILE_BYTES: u64 = 1_000_000;

/// `_read_json_file`: bounded bytes, CPython's JSON encoding detection.
fn read_json_file(filename: &str) -> Result<Value, AppError> {
    let home = std::env::var("HOME").ok();
    let path = crate::render::expand_user(filename, home.as_deref())
        .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let unreadable = || AppError::new("validation", "Unable to read the review input file");
    let mut payload = Vec::new();
    std::fs::File::open(&path)
        .and_then(|file| {
            file.take(MAX_REVIEW_FILE_BYTES + 1)
                .read_to_end(&mut payload)
        })
        .map_err(|_| unreadable())?;
    if payload.len() as u64 > MAX_REVIEW_FILE_BYTES {
        return Err(
            AppError::new("validation", "Review input file is too large").with_details(dict([(
                "maximum_bytes",
                Value::from(MAX_REVIEW_FILE_BYTES as i64),
            )])),
        );
    }
    let invalid = || AppError::new("validation", "Review input file must contain valid JSON");
    let text = kassiber_pyjson::json_bytes_string(&payload).map_err(|_| invalid())?;
    kassiber_pytext::loads_py(&text).map_err(|error| {
        if error.is_decode_error() {
            invalid()
        } else {
            AppError::new("internal_error", error.to_string())
        }
    })
}

fn arg(args: &Namespace, name: &str) -> Value {
    args.get(name).cloned().unwrap_or(Value::Null)
}

pub fn execute(book: &mut dyn Book, operation: &str, args: &Namespace) -> Result<Value, AppError> {
    let workspace = arg(args, "workspace");
    let profile = arg(args, "profile");
    // The scope resolves before any input file is read, as in Python.
    book.resolve_scope(Some(&workspace), Some(&profile))?;
    let mut payload = dict([("workspace", workspace), ("profile", profile)]);
    let mut put = |key: &str, value: Value| kassiber_envelope::set(&mut payload, key, value);
    match operation {
        "review.cases" => {
            put("limit", arg(args, "limit"));
            put("cursor", arg(args, "cursor"));
        }
        "review.plan" => {
            let mut operations =
                read_json_file(&kassiber_envelope::py_str(&arg(args, "operations_file")))?;
            if matches!(operations, Value::Dict(_)) {
                operations = get(&operations, "operations")
                    .cloned()
                    .unwrap_or(Value::Null);
            }
            if !matches!(operations, Value::List(_)) {
                return Err(AppError::new(
                    "validation",
                    "Review operations file must contain an array or an operations object",
                ));
            }
            put("operations", operations);
            put(
                "expected_input_version",
                arg(args, "expected_input_version"),
            );
        }
        "review.apply" => {
            let mut artifact =
                read_json_file(&kassiber_envelope::py_str(&arg(args, "artifact_file")))?;
            // Only a string kind names the plan envelope; any other file goes
            // to the artifact validator, which refuses it as malformed.
            if matches!(
                get(&artifact, "kind"),
                Some(kind) if *kind == s("review.plan") || *kind == s("ui.review.plan")
            ) {
                artifact = get(&artifact, "data").cloned().unwrap_or(Value::Null);
            }
            if !matches!(artifact, Value::Dict(_)) {
                return Err(AppError::new(
                    "validation",
                    "Review artifact file must contain a plan object",
                ));
            }
            put("artifact", artifact);
            put("idempotency_key", arg(args, "idempotency_key"));
        }
        _ => {
            put("receipt_id", arg(args, "receipt_id"));
            put("idempotency_key", arg(args, "idempotency_key"));
        }
    }
    book.review(operation, &payload, "cli")
}

#[cfg(test)]
mod tests {
    use super::*;
    use kassiber_runtime::store::{BookContext, StatusFacts};

    struct Capture {
        scope: Result<(), AppError>,
    }
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
            self.scope.clone().map(|()| ("ws".into(), "profile".into()))
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

    fn namespace(fields: &[(&str, Value)]) -> Namespace {
        let mut namespace = Namespace::default();
        for (name, value) in fields {
            namespace.set(name, value.clone());
        }
        namespace
    }

    fn run(operation: &str, fields: &[(&str, Value)]) -> Result<Value, AppError> {
        execute(
            &mut Capture { scope: Ok(()) },
            operation,
            &namespace(fields),
        )
    }

    fn file(directory: &std::path::Path, name: &str, payload: &[u8]) -> Value {
        let path = directory.join(name);
        std::fs::write(&path, payload).unwrap();
        s(path.to_str().unwrap())
    }

    #[test]
    fn review_cli_reads_bounded_files_unwraps_envelopes_and_authors_cli() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let version = ("expected_input_version", Value::from(3));
        let plan = |payload: &[u8]| {
            run(
                "review.plan",
                &[
                    ("operations_file", file(dir, "ops.json", payload)),
                    version.clone(),
                ],
            )
        };
        let out = plan(br#"{"operations":[{"type":"exclude"}]}"#).unwrap();
        assert_eq!(get(&out, "authored_source"), Some(&s("cli")));
        let args = get(&out, "args").unwrap();
        assert_eq!(
            kassiber_pyjson::dumps(get(args, "operations").unwrap(), &Default::default()).unwrap(),
            r#"[{"type": "exclude"}]"#
        );
        assert_eq!(get(args, "expected_input_version"), Some(&Value::from(3)));
        let mut exact = b"[]".to_vec();
        exact.resize(1_000_000, b' ');
        assert!(plan(&exact).is_ok(), "exactly the bound is admitted");
        exact.push(b' ');
        let error = plan(&exact).unwrap_err();
        assert_eq!(error.message, "Review input file is too large");
        assert_eq!(
            error.details,
            Some(dict([("maximum_bytes", Value::from(1_000_000))]))
        );
        let utf16: Vec<u8> = "\u{feff}[]"
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect();
        assert!(plan(&utf16).is_ok(), "CPython detects UTF-16 JSON");
        for (payload, message) in [
            (
                &br#"{"ops":[]}"#[..],
                "Review operations file must contain an array or an operations object",
            ),
            (
                b"\"text\"",
                "Review operations file must contain an array or an operations object",
            ),
            (b"[1,", "Review input file must contain valid JSON"),
            (b"", "Review input file must contain valid JSON"),
            (b"\xff\xfe\x00", "Review input file must contain valid JSON"),
        ] {
            assert_eq!(plan(payload).unwrap_err().message, message);
        }
        let missing = run(
            "review.plan",
            &[
                ("operations_file", s(dir.join("missing").to_str().unwrap())),
                version.clone(),
            ],
        )
        .unwrap_err();
        assert_eq!(missing.message, "Unable to read the review input file");

        let apply = |payload: &[u8]| {
            run(
                "review.apply",
                &[
                    ("artifact_file", file(dir, "artifact.json", payload)),
                    ("idempotency_key", s("key")),
                ],
            )
        };
        for envelope in [
            &br#"{"kind":"review.plan","data":{"digest":"d"}}"#[..],
            br#"{"kind":"ui.review.plan","data":{"digest":"d"}}"#,
            br#"{"digest":"d"}"#,
        ] {
            let out = apply(envelope).unwrap();
            assert_eq!(get(&out, "authored_source"), Some(&s("cli")));
            let args = get(&out, "args").unwrap();
            assert_eq!(get(args, "artifact"), Some(&dict([("digest", s("d"))])));
            assert_eq!(get(args, "idempotency_key"), Some(&s("key")));
        }
        for (payload, kind) in [
            (&br#"{"kind":"other","data":{}}"#[..], s("other")),
            // Not a string: no envelope, so the validator sees the file itself.
            (
                br#"{"kind":["review.plan"],"data":{}}"#,
                Value::List(vec![s("review.plan")]),
            ),
            (br#"{"kind":{},"data":{}}"#, dict::<&str>([])),
        ] {
            let kept = apply(payload).unwrap();
            assert_eq!(
                get(get(&kept, "args").unwrap(), "artifact").and_then(|a| get(a, "kind")),
                Some(&kind)
            );
        }
        for (payload, code, message) in [
            (
                &br#"{"kind":"review.plan"}"#[..],
                "validation",
                "Review artifact file must contain a plan object",
            ),
            (
                b"[]",
                "validation",
                "Review artifact file must contain a plan object",
            ),
        ] {
            let error = apply(payload).unwrap_err();
            assert_eq!(
                (error.code.as_str(), error.message.as_str()),
                (code, message)
            );
        }
    }

    #[test]
    fn review_cli_resolves_scope_before_reading_any_file() {
        let refused = AppError::new("not_found", "Profile not found");
        for (operation, field) in [
            ("review.plan", "operations_file"),
            ("review.apply", "artifact_file"),
        ] {
            let error = execute(
                &mut Capture {
                    scope: Err(refused.clone()),
                },
                operation,
                &namespace(&[(field, s("/nonexistent/review.json"))]),
            )
            .unwrap_err();
            assert_eq!(error.code, "not_found");
        }
        let out = run(
            "review.cases",
            &[
                ("limit", Value::from(5)),
                ("cursor", Value::Null),
                ("profile", s("p")),
            ],
        )
        .unwrap();
        assert_eq!(get(&out, "operation"), Some(&s("review.cases")));
        let args = get(&out, "args").unwrap();
        assert_eq!(get(args, "limit"), Some(&Value::from(5)));
        assert_eq!(get(args, "profile"), Some(&s("p")));
        let receipt = run("review.receipt", &[("idempotency_key", s("k"))]).unwrap();
        assert_eq!(
            get(get(&receipt, "args").unwrap(), "idempotency_key"),
            Some(&s("k"))
        );
    }
}
