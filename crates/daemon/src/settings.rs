//! Freshness and maintenance settings: local policy reads and writes with no
//! job, source run or connection, and the explicit maintenance run.
use kassiber_envelope::{dict, get, s, set, success, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::Book;

pub use kassiber_wallets::freshness::settings::KINDS;

pub const RUN_KIND: &str = "ui.maintenance.run";

/// The read kinds ignore their arguments, as Python's handlers do; the
/// configure kinds coerce them like `_coerce_args_dict`.
pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = if kind.ends_with(".configure") {
        match args {
            None | Some(Value::Null) => &empty,
            Some(v @ Value::Dict(_)) => v,
            Some(v) => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(type_name(v)))])))
            }
        }
    } else {
        &empty
    };
    book.service(kind, args)
}

/// `_maintenance_run_payload`: an optional watch-only refresh (`never`,
/// `if_enabled` by the book's report-read policy, or `always` on this explicit
/// request), the local journal rebuild when stale, then the report blockers
/// and maintenance settings. Arguments are refused before any of it runs.
pub fn run(book: &mut dyn Book, args: Option<&Value>) -> Result<Value, AppError> {
    let args = match args {
        None | Some(Value::Null) => dict::<&str>([]),
        Some(v @ Value::Dict(_)) => v.clone(),
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))])))
        }
    };
    let Value::Dict(fields) = &args else {
        unreachable!()
    };
    let mut unknown: Vec<Value> = fields
        .iter()
        .filter(|(key, _)| *key != Key::from("sync"))
        .map(|(key, _)| match key {
            Key::Str(key) => Value::Str(key.clone()),
            _ => unreachable!("JSON object keys are strings"),
        })
        .collect();
    if !unknown.is_empty() {
        unknown.sort_by_key(kassiber_envelope::py_str);
        return Err(AppError::new(
            "validation",
            "ui.maintenance.run received unsupported arguments",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let mode = get(&args, "sync")
        .cloned()
        .unwrap_or_else(|| s("if_enabled"));
    if let Value::List(_) | Value::Dict(_) = mode {
        // `mode in {...}` hashes its operand first.
        return Err(AppError::new(
            "internal_error",
            format!("unhashable type: '{}'", type_name(&mode)),
        ));
    }
    let force = match &mode {
        Value::Str(text) if text.as_str() == Some("never") => None,
        Value::Str(text) if text.as_str() == Some("if_enabled") => Some(false),
        Value::Str(text) if text.as_str() == Some("always") => Some(true),
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.maintenance.run sync must be never, if_enabled, or always",
            )
            .with_details(dict([("sync", mode)])))
        }
    };
    let mut maintenance = dict::<&str>([]);
    let sync = match force {
        Some(force) => book.report_read_sync(force)?,
        None => None,
    };
    if let Some(sync) = &sync {
        set(
            &mut maintenance,
            "sync",
            success("ui.wallets.sync", sync.clone()),
        );
    }
    if let Some(journals) = book.auto_process_journals()? {
        set(
            &mut maintenance,
            "journals",
            success("ui.journals.process", journals),
        );
    }
    let blockers = book.report_blockers(sync.as_ref())?;
    let settings = book.service("ui.maintenance.settings", &dict::<&str>([]))?;
    let field = |value: &Value, key: &str| get(value, key).cloned().unwrap_or(Value::Null);
    Ok(dict([
        ("ready", field(&blockers, "ready")),
        ("sync_mode", mode),
        ("maintenance", maintenance),
        ("blockers", field(&blockers, "blockers")),
        ("health", field(&blockers, "health")),
        ("settings", field(&settings, "settings")),
    ]))
}
