//! Desktop/AI import-run kinds. The core owns row selection and rollback.
use kassiber_envelope::{dict, get, s, set, success, truthy, type_name, AppError};
use kassiber_imports::Operation;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

use crate::output::{with_request_id, RequestId};
use crate::{Daemon, Handled};

pub const KINDS: &[&str] = &["ui.imports.list", "ui.imports.rollback"];

pub(super) fn optional_string(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(text)) => {
            let text = kassiber_pytext::strip_py(text);
            let text = text.as_str().ok_or_else(|| {
                let points = text.code_points().collect::<Vec<_>>();
                let start = points
                    .iter()
                    .position(|p| (0xd800..=0xdfff).contains(p))
                    .expect("surrogate");
                let end = start
                    + points[start..]
                        .iter()
                        .take_while(|p| (0xd800..=0xdfff).contains(*p))
                        .count();
                let description = if end == start + 1 {
                    format!("character '\\u{:04x}' in position {start}", points[start])
                } else {
                    format!("characters in position {start}-{}", end - 1)
                };
                AppError::new(
                    "internal_error",
                    format!("'utf-8' codec can't encode {description}: surrogates not allowed"),
                )
            })?;
            Ok((!text.is_empty()).then(|| text.to_owned()))
        }
        Some(value) => Err(
            AppError::new("validation", format!("{key} must be a string"))
                .with_details(dict([("type", s(type_name(value)))])),
        ),
    }
}

/// `_imports_list_payload(conn, args)`: the active book's import runs,
/// newest first.
pub(crate) fn list(book: &mut dyn Book, args: &Value) -> Result<Value, AppError> {
    let (_, profile_id) = book.resolve_scope(None, None)?;
    let wallet = optional_string(args, "wallet")?;
    let limit = get(args, "limit").cloned().unwrap_or(Value::Null);
    let batches = book.import_runs(Operation::List {
        profile_id,
        wallet,
        limit,
    })?;
    Ok(dict([("batches", batches)]))
}

pub(super) fn handle(
    daemon: &mut Daemon,
    kind: &str,
    args: Option<&Value>,
    request_id: &RequestId,
) -> Handled {
    if let Some(locked) = daemon.open_lazily(request_id)? {
        return Ok((Some(locked), false));
    }
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(value @ Value::Dict(_)) => value,
        Some(value) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(value)))]))
                .into())
        }
    };
    let book = daemon.book.as_mut().expect("opened book");
    let payload = if kind == "ui.imports.list" {
        list(book.as_mut(), args)?
    } else {
        let (_, profile_id) = book.resolve_scope(None, None)?;
        let batch = optional_string(args, "batch")?.ok_or_else(|| {
            AppError::new("validation", "Import run is required.").with_hint("Enter import run.")
        })?;
        let mut plan = book.import_runs(Operation::Plan {
            profile_id: profile_id.clone(),
            batch: batch.clone(),
        })?;
        if get(args, "dry_run").is_some_and(truthy) {
            set(&mut plan, "applied", Value::Bool(false));
            plan
        } else {
            if get(args, "confirm") != Some(&s("DELETE")) {
                return Err(AppError::new(
                    "validation",
                    "ui.imports.rollback requires confirm='DELETE'",
                )
                .with_hint("Show the rollback plan and let the user confirm before deleting.")
                .with_details(dict([(
                    "transactions_to_delete",
                    get(&plan, "transactions_to_delete")
                        .cloned()
                        .unwrap_or(Value::Null),
                )]))
                .into());
            }
            let mut payload = book.import_runs(Operation::Rollback { profile_id, batch })?;
            set(&mut payload, "applied", Value::Bool(true));
            payload
        }
    };
    let shutdown = get(&payload, "applied") == Some(&Value::Bool(true));
    Ok((
        Some(with_request_id(success(kind, payload), request_id)),
        shutdown,
    ))
}
