//! Import runs for the assistant (#682): `_imports_list_ai_payload`,
//! `core_import_batches.review_rollback` and `apply_reviewed_rollback`, and
//! `_imports_rollback_ai_payload`.
//!
//! An applied import is an ordinary run; the assistant can list runs and
//! roll one back through a plan bound to the book's input version and a
//! digest of what the user saw, after once-only consent. A rollback that
//! would also delete reviewed work attached to the run's rows is the
//! desktop's decision. A restricted provider never gets the recorded
//! filename.

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Key, Options, Value};
use sha2::{Digest, Sha256};

use crate::executor::Book;
use crate::validate::py_eq;
use crate::ToolError;

/// What the assistant asks of the book's import runs.
pub enum ImportRuns<'a> {
    /// `_imports_list_payload(conn, args)`: `{"batches": [...]}`.
    List(&'a Value),
    /// `plan_rollback` with the book's `input_version` appended.
    Review(&'a str),
    /// `rollback_batch`: delete what the run created, then the run.
    Rollback(&'a str),
    /// The profile's journal input version.
    InputVersion,
}

/// `WALLET_INPUT_SOURCES`: the wallet kinds the desktop sets up itself.
pub const WALLET_INPUT_SOURCES: [&str; 11] = [
    "descriptor",
    "address-list",
    "silent-payment",
    "liquid-descriptor",
    "samourai",
    "core-ln",
    "lnd",
    "btcpay",
    "kraken",
    "coinbase",
    "binance",
];

/// `_wallet_input_request_payload(conn, arguments)`: a typed handoff for a
/// wallet the assistant must not create itself. Descriptor, xpub, address
/// and credential-backed sources need material that never belongs in a
/// prompt, so the assistant can only ask; the desktop turns the packet into
/// its own setup dialog. Its digest is a UI correlation id, never an
/// authorization token.
pub fn wallet_input_request(book: &mut dyn Book, arguments: &Value) -> Result<Value, ToolError> {
    let source = match get(arguments, "source") {
        Some(Value::Str(source)) if WALLET_INPUT_SOURCES.iter().any(|known| source.is(known)) => {
            source.clone()
        }
        _ => {
            return Err(AppError::new("validation", "Unsupported wallet source")
                .with_details(dict([(
                    "allowed",
                    Value::List(
                        WALLET_INPUT_SOURCES
                            .iter()
                            .map(|source| s(*source))
                            .collect(),
                    ),
                )]))
                .into())
        }
    };
    let explanation = match get(arguments, "explanation") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Str(text)) => {
            let raw = crate::pyval::lossy(text);
            let stripped = kassiber_pytext::strip(&raw);
            let length = stripped.chars().count();
            let control = raw
                .chars()
                .any(|c| (c as u32) < 32 && c != '\n' && c != '\t');
            if !(1..=500).contains(&length) || control {
                return Err(
                    AppError::new("validation", "explanation must be bounded plain text").into(),
                );
            }
            s(stripped)
        }
        Some(_) => {
            return Err(
                AppError::new("validation", "explanation must be bounded plain text").into(),
            )
        }
    };
    let (workspace_id, profile_id) = book.context()?;
    let input_version = book.import_runs(ImportRuns::InputVersion)?;
    let packet = dict([
        ("schema_version", Value::from(1)),
        ("workspace_id", s(kassiber_envelope::py_str(&workspace_id))),
        ("profile_id", s(kassiber_envelope::py_str(&profile_id))),
        ("input_version", input_version),
        ("action", s("connect_wallet")),
        ("source", Value::Str(source)),
        ("explanation", explanation),
        ("desktop_only", Value::Bool(true)),
    ]);
    let encoded = dumps(
        &packet,
        &Options::new().sort_keys(true).separators(",", ":"),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let digest: String = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let mut payload = packet;
    kassiber_envelope::set(&mut payload, "request_id", s(digest));
    Ok(payload)
}

fn without_filename(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) => Value::Dict(
            pairs
                .iter()
                .filter(|(key, _)| *key != Key::from("source_filename"))
                .cloned()
                .collect(),
        ),
        other => other.clone(),
    }
}

/// `_imports_list_ai_payload(conn, runtime, arguments)`: recorded filenames
/// are routinely personal, so a restricted provider gets each run's shape,
/// not what the user named the file.
pub fn list_for_ai(
    book: &mut dyn Book,
    arguments: &Value,
    full_access: bool,
) -> Result<Value, ToolError> {
    let payload = book.import_runs(ImportRuns::List(arguments))?;
    if full_access {
        return Ok(payload);
    }
    let batches = match get(&payload, "batches") {
        Some(Value::List(batches)) => batches.iter().map(without_filename).collect(),
        _ => Vec::new(),
    };
    Ok(dict([("batches", Value::List(batches))]))
}

/// `review_rollback(conn, profile, batch_ref)`: `plan_rollback` bound to the
/// book version, for a caller that needs consent.
pub fn review(book: &mut dyn Book, batch: &str) -> Result<Value, ToolError> {
    let mut reviewed = book.import_runs(ImportRuns::Review(batch))?;
    let mut blockers = Vec::new();
    let to_delete = get(&reviewed, "transactions_to_delete").is_some_and(kassiber_envelope::truthy);
    if !to_delete {
        blockers.push(dict([
            ("code", s("nothing_to_roll_back")),
            ("message", s("This import run has no rows left to delete.")),
        ]));
    }
    if get(&reviewed, "also_removed").is_some_and(kassiber_envelope::truthy) {
        let run = get(&reviewed, "batch")
            .and_then(|batch| get(batch, "id"))
            .cloned()
            .unwrap_or(Value::Null);
        blockers.push(dict([
            ("code", s("rollback_removes_reviewed_work")),
            (
                "message",
                s("Rolling back would also delete reviewed work attached to these rows."),
            ),
            (
                "remediation",
                dict([
                    ("action", s("rollback_locally")),
                    ("tool", Value::Null),
                    ("target", dict([("batch", run)])),
                    ("desktop_only", Value::Bool(true)),
                ]),
            ),
        ]));
    }
    let ready = blockers.is_empty();
    kassiber_envelope::set(&mut reviewed, "blockers", Value::List(blockers));
    kassiber_envelope::set(&mut reviewed, "ready", Value::Bool(ready));
    let encoded = dumps(
        &reviewed,
        &Options::new().sort_keys(true).separators(",", ":"),
    )
    .map_err(|error| AppError::new("internal_error", error.to_string()))?;
    let digest: String = Sha256::digest(encoded.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    kassiber_envelope::set(&mut reviewed, "plan_digest", s(digest));
    Ok(reviewed)
}

/// `apply_reviewed_rollback(...)`: roll back only if the reviewed plan still
/// describes the book exactly.
fn apply(
    book: &mut dyn Book,
    batch: &str,
    plan_digest: &Value,
    expected_input_version: &Value,
) -> Result<Value, ToolError> {
    let current = review(book, batch)?;
    let field = |key: &str| get(&current, key).cloned().unwrap_or(Value::Null);
    if !py_eq(&field("input_version"), expected_input_version)
        || !py_eq(&field("plan_digest"), plan_digest)
    {
        return Err(AppError::new(
            "rollback_plan_stale",
            "The import run changed since the rollback was planned",
        )
        .with_hint("Plan the rollback again and ask for consent again.")
        .with_details(dict([(
            "remediation",
            dict([
                ("action", s("replan")),
                ("tool", s("ui_imports_rollback_plan")),
                ("target", dict([("batch", s(batch))])),
                ("desktop_only", Value::Bool(false)),
            ]),
        )]))
        .into());
    }
    if matches!(get(&current, "blockers"), Some(Value::List(blockers)) if !blockers.is_empty()) {
        return Err(
            AppError::new("rollback_blocked", "This rollback is blocked")
                .with_details(dict([("blockers", field("blockers"))]))
                .into(),
        );
    }
    book.import_runs(ImportRuns::Rollback(batch))
}

/// `_imports_rollback_ai_payload(conn, runtime, arguments, apply=...)`.
pub fn rollback_for_ai(
    book: &mut dyn Book,
    arguments: &Value,
    full_access: bool,
    applying: bool,
) -> Result<Value, ToolError> {
    // `resolve_scope(conn, None, None)` before the argument, as Python.
    book.active_profile()?;
    let batch = crate::mutate::required_str_arg(arguments, "batch", "Import run")?;
    let batch = crate::pyval::lossy(&batch);
    let mut payload = if applying {
        let field = |key: &str| get(arguments, key).cloned().unwrap_or(Value::Null);
        apply(
            book,
            &batch,
            &field("plan_digest"),
            &field("expected_input_version"),
        )?
    } else {
        review(book, &batch)?
    };
    if !full_access {
        payload = without_filename(&payload);
        if let Some(inner @ Value::Dict(_)) = get(&payload, "batch") {
            let inner = without_filename(inner);
            kassiber_envelope::set(&mut payload, "batch", inner);
        }
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::executor::{Maintenance, ReadHost};

    /// A book with one import run and the rollback it would make.
    struct Runs {
        plan: Value,
        rolled_back: bool,
    }

    impl Book for Runs {
        fn database_in_scope(&mut self, _: &str) -> Result<bool, ToolError> {
            Ok(true)
        }
        fn context(&mut self) -> Result<(Value, Value), ToolError> {
            Ok((s("w"), s("p")))
        }
        fn maintain_for_read(&mut self, _: bool) -> Result<Maintenance, ToolError> {
            Ok(Maintenance {
                auto_sync: None,
                journals: None,
            })
        }
        fn read(&mut self, _: &str, _: &Value, _: &'static str) -> Result<Value, ToolError> {
            unreachable!()
        }
        fn active_profile(&mut self) -> Result<String, ToolError> {
            Ok("p".into())
        }
        fn mutate(&mut self, _: &str, _: &Value, _: &'static str) -> Result<Value, ToolError> {
            unreachable!()
        }
        fn import_runs(&mut self, request: ImportRuns<'_>) -> Result<Value, ToolError> {
            match request {
                ImportRuns::Review(_) => Ok(self.plan.clone()),
                ImportRuns::Rollback(_) => {
                    self.rolled_back = true;
                    Ok(dict([
                        ("source_filename", s("Kraken_max.csv")),
                        ("rolled_back", Value::Bool(true)),
                    ]))
                }
                ImportRuns::List(_) => unreachable!(),
                ImportRuns::InputVersion => Ok(Value::from(7)),
            }
        }
    }

    impl ReadHost for Runs {
        fn on_request_loop(
            &mut self,
            work: &mut dyn FnMut(&mut dyn Book) -> Result<Value, ToolError>,
        ) -> Result<Value, ToolError> {
            work(self)
        }
    }

    fn plan(also_removed: Value) -> Value {
        dict([
            (
                "batch",
                dict([("id", s("run-1")), ("source_filename", s("Kraken_max.csv"))]),
            ),
            ("transactions_to_delete", Value::from(2)),
            ("also_removed", also_removed),
            ("journals_invalidated", Value::Bool(true)),
            ("input_version", Value::from(7)),
        ])
    }

    #[test]
    fn a_rollback_runs_only_on_the_plan_the_user_saw() {
        let mut book = Runs {
            plan: plan(Value::Dict(Vec::new())),
            rolled_back: false,
        };
        let reviewed =
            rollback_for_ai(&mut book, &dict([("batch", s("run-1"))]), false, false).unwrap();
        assert_eq!(get(&reviewed, "ready"), Some(&Value::Bool(true)));
        assert!(!format!("{reviewed:?}").contains("Kraken_max"));
        let digest = get(&reviewed, "plan_digest").unwrap().clone();
        let stale = dict([
            ("batch", s("run-1")),
            ("plan_digest", digest.clone()),
            ("expected_input_version", Value::from(6)),
        ]);
        let ToolError::App(error) = rollback_for_ai(&mut book, &stale, true, true).unwrap_err()
        else {
            panic!()
        };
        assert_eq!(error.code, "rollback_plan_stale");
        assert!(!book.rolled_back);
        let current = dict([
            ("batch", s("run-1")),
            ("plan_digest", digest),
            ("expected_input_version", Value::from(7)),
        ]);
        rollback_for_ai(&mut book, &current, true, true).unwrap();
        assert!(book.rolled_back);
    }

    #[test]
    fn reviewed_work_blocks_a_rollback_from_chat() {
        let mut book = Runs {
            plan: plan(dict([("transfer_pairs", Value::from(1))])),
            rolled_back: false,
        };
        let reviewed = review(&mut book, "run-1").unwrap();
        assert_eq!(get(&reviewed, "ready"), Some(&Value::Bool(false)));
        let request = dict([
            ("batch", s("run-1")),
            (
                "plan_digest",
                get(&reviewed, "plan_digest").unwrap().clone(),
            ),
            ("expected_input_version", Value::from(7)),
        ]);
        let ToolError::App(error) = rollback_for_ai(&mut book, &request, true, true).unwrap_err()
        else {
            panic!()
        };
        assert_eq!(error.code, "rollback_blocked");
        assert!(!book.rolled_back);
    }
}
