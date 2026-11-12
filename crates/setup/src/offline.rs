//! Machine-wide offline preference, shared by CLI and locked desktop sessions.
use super::*;
use kassiber_paths::Context;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

pub fn preference_path(context: &Context) -> Result<PathBuf> {
    if let Some(path) = context
        .environ
        .get(OsStr::new("KASSIBER_OFFLINE_PREFERENCE_FILE"))
        .filter(|v| !v.is_empty())
    {
        return context.expand_user(Path::new(path)).map_err(storage_error);
    }
    Ok(context
        .default_state_root()
        .map_err(storage_error)?
        .join("config/offline-mode.json"))
}

pub fn enabled(path: &Path) -> bool {
    // pathlib treats invalid names and a missing/non-directory/looping parent as
    // absent. A final dangling symlink is still present and fails closed below.
    if kassiber_paths::preference_path_is_absent(path) {
        return false;
    }
    let Some(raw) = kassiber_paths::read_small_regular_file(path, 4096) else {
        return true;
    };
    let Some(payload) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|s| kassiber_pytext::loads(s).ok())
    else {
        return true;
    };
    // Python compares schema versions numerically, but insists on an exact bool.
    let version = match get(&payload, "schema_version") {
        Some(Value::Int(v)) => v == &1.into(),
        Some(Value::Float(v)) => *v == 1.0,
        Some(Value::Bool(v)) => *v,
        _ => false,
    };
    match (version, get(&payload, "enabled")) {
        (true, Some(Value::Bool(v))) => *v,
        _ => true,
    }
}

pub fn execute(context: &Context, command: &str, args: &Value) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let set = matches!(
        command,
        "offline.on" | "offline.off" | "ui.network.offline.set"
    );
    if desktop {
        let mut unknown = if let Value::Dict(fields) = args {
            fields
                .iter()
                .map(|(k, _)| {
                    py_str(&Value::Str(match k {
                        kassiber_pyjson::Key::Str(k) => k.clone(),
                        _ => unreachable!(),
                    }))
                })
                .filter(|k| !(set && k == "enabled"))
                .collect::<Vec<_>>()
        } else {
            unreachable!("daemon coerces argument objects")
        };
        unknown.sort();
        if !unknown.is_empty() {
            return Err(AppError::new(
                "validation",
                format!("{command} received unsupported fields"),
            )
            .with_details(dict([(
                "unknown",
                Value::List(unknown.into_iter().map(s).collect()),
            )])));
        }
    }
    let next = if set {
        Some(if desktop {
            match get(args, "enabled") {
                Some(Value::Bool(v)) => *v,
                _ => {
                    return Err(AppError::new(
                        "validation",
                        "ui.network.offline.set requires a boolean enabled",
                    ))
                }
            }
        } else {
            command == "offline.on"
        })
    } else {
        None
    };
    let path = preference_path(context)?;
    if let Some(next) = next {
        let document = format!(
            "{{\"enabled\": {}, \"schema_version\": 1}}\n",
            if next { "true" } else { "false" }
        );
        kassiber_store::layout::write_private_preference(&path, document.as_bytes())
            .map_err(|e| projects::io_error(e, &path))?;
    }
    let blocked = context
        .environ
        .get(OsStr::new("KASSIBER_NO_EGRESS"))
        .map(|v| v.to_string_lossy())
        .is_some_and(|v| {
            ["1", "true", "yes", "on"].contains(&kassiber_pytext::strip(&v).to_lowercase().as_str())
        });
    Ok(dict([
        ("offline", Value::Bool(enabled(&path))),
        ("environment_blocked", Value::Bool(blocked)),
    ]))
}
