//! The journal reproducibility check (docs/reference/replay-bundle.md) for
//! the CLI, the daemon and the AI.
use kassiber_envelope::{get, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;

pub(super) fn py(value: &serde_json::Value) -> Result<Value, AppError> {
    kassiber_custody::codec::from_json(value)
        .map_err(|message| AppError::new("internal_error", message))
}

pub(super) fn arg<'a>(args: &'a Value, name: &str) -> Option<&'a Value> {
    get(args, name).filter(|value| !matches!(value, Value::Null))
}

pub(super) fn limit(args: &Value) -> Result<usize, AppError> {
    match arg(args, "limit") {
        None => Ok(kassiber_journals::reproduce::DEFAULT_LIMIT),
        Some(Value::Int(n)) => Ok(kassiber_journals::reproduce::clamp_limit(
            usize::try_from(n.clone()).ok().or(Some(1)),
        )),
        Some(_) => Err(AppError::new("validation", "limit must be an integer")),
    }
}

/// Reject daemon arguments outside `allowed`; the desktop and the assistant
/// cannot widen a request's scope.
fn only(kind: &str, args: Option<&Value>, allowed: &[&str]) -> Result<(), AppError> {
    match args {
        None | Some(Value::Null) => Ok(()),
        Some(Value::Dict(items)) => {
            for (key, _) in items {
                let key = match key {
                    kassiber_pyjson::Key::Str(text) => text.as_str().unwrap_or_default().to_owned(),
                    other => format!("{other:?}"),
                };
                if !allowed.contains(&key.as_str()) {
                    return Err(AppError::new(
                        "validation",
                        format!("{kind} does not accept argument {key}"),
                    ));
                }
            }
            Ok(())
        }
        Some(other) => Err(AppError::new("validation", "daemon args must be an object")
            .with_details(kassiber_envelope::dict([(
                "type",
                s(kassiber_envelope::type_name(other)),
            )]))),
    }
}

/// `journals verify` and `ui.journals.verify`: read-only.
pub(super) fn verify_journals(
    conn: &Connection,
    kind: &str,
    args: Option<&Value>,
    sources: &impl kassiber_transfers::Sources,
) -> Result<Value, AppError> {
    let empty = kassiber_envelope::dict::<&str>([]);
    let (workspace, profile) = if kind == "ui.journals.verify" {
        only(kind, args, &["limit"])?;
        (None, None)
    } else {
        let args = args.unwrap_or(&empty);
        (arg(args, "workspace"), arg(args, "profile"))
    };
    let limit = limit(args.unwrap_or(&empty))?;
    let report = kassiber_journals::reproduce::check(conn, workspace, profile, limit, sources)
        .map_err(kassiber_journals::service::application_error)?;
    py(&report)
}
