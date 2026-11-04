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
            if matches!(artifact, Value::Dict(_)) {
                match get(&artifact, "kind") {
                    // A set membership test hashes the kind first.
                    Some(kind @ (Value::List(_) | Value::Dict(_))) => {
                        return Err(AppError::new(
                            "internal_error",
                            format!("unhashable type: '{}'", kassiber_envelope::type_name(kind)),
                        ))
                    }
                    Some(kind) if *kind == s("review.plan") || *kind == s("ui.review.plan") => {
                        artifact = get(&artifact, "data").cloned().unwrap_or(Value::Null);
                    }
                    _ => {}
                }
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
