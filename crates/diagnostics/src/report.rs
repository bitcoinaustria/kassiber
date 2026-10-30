//! Ordered report schema and explicitly requested export. No logging or egress.
use crate::{is_sensitive_key, sanitize_details, sanitize_text, _SAFE_ENUM_KEYS, _SKIP_ARG_KEYS};
use kassiber_envelope::{derive_kind, dict, get, py_str, s, truthy, AppError};
use kassiber_paths::{Context, ResolveOptions, RuntimePaths};
use kassiber_pyjson::{dumps, Options, Value};
use kassiber_pytext::repr_str;
use std::path::{Path, PathBuf};
fn n(v: usize) -> Value {
    Value::Int(v.into())
}
fn val<'a>(v: &'a Value, k: &str) -> &'a Value {
    get(v, k).unwrap_or(&Value::Null)
}
fn text<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    match get(v, k) {
        Some(Value::Str(s)) => s.as_str(),
        _ => None,
    }
}
fn flag(v: &Value, k: &str) -> bool {
    truthy(val(v, k))
}
/// Read-only resolution, including a fail-closed unknown-path projection.
pub fn paths(args: &Value) -> Option<RuntimePaths> {
    Context::from_env()
        .ok()?
        .resolve(&ResolveOptions {
            data_root: text(args, "data_root").map(PathBuf::from),
            env_file: text(args, "env_file").map(PathBuf::from),
            project: None,
        })
        .ok()
}
fn invocation(args: &Value) -> Value {
    let kind = derive_kind(|k| text(args, k), None);
    let mut arguments = Vec::new();
    if let Value::Dict(pairs) = args {
        let mut pairs = pairs
            .iter()
            .filter_map(|(key, v)| match key {
                kassiber_pyjson::Key::Str(k) => Some((k.as_str()?, v)),
                _ => None,
            })
            .collect::<Vec<_>>();
        pairs.sort_by_key(|(k, _)| *k);
        for (key, value) in pairs {
            // Python compares with False as a value, so zero is omitted as well.
            if _SKIP_ARG_KEYS.contains(&key) || key.ends_with("_command") || omitted_argument(value)
            {
                continue;
            }
            let mut fields = vec![("name", s(key))];
            match value {
                Value::Bool(v) => {
                    fields.extend([("value_class", s("boolean")), ("value", Value::Bool(*v))])
                }
                _ if is_sensitive_key(key) => fields.push(("value_class", s("redacted"))),
                Value::Str(v) if _SAFE_ENUM_KEYS.contains(&key) => fields.extend([
                    ("value_class", s("enum")),
                    ("value", Value::Str(crate::sanitize_py(v))),
                ]),
                Value::Int(_) | Value::Float(_) => fields.push(("value_class", s("number"))),
                Value::List(v) => {
                    fields.extend([("value_class", s("list")), ("items", n(v.len()))])
                }
                _ => fields.extend([
                    ("value_class", s("text")),
                    ("value", s(sanitize_text(&py_str(value)))),
                ]),
            }
            arguments.push(dict(fields));
        }
    }
    dict([
        (
            "command_path",
            Value::List(kind.split('.').filter(|v| !v.is_empty()).map(s).collect()),
        ),
        ("output_format", val(args, "format").clone()),
        ("machine", Value::Bool(flag(args, "machine"))),
        ("debug_requested", Value::Bool(flag(args, "debug"))),
        (
            "diagnostics_out_requested",
            Value::Bool(flag(args, "diagnostics_out")),
        ),
        ("provided_arguments", Value::List(arguments)),
    ])
}
fn storage(paths: Option<&RuntimePaths>) -> Value {
    let Some(p) = paths else {
        return dict([
            ("layout", s("unknown")),
            ("paths_available", Value::Bool(false)),
            ("diagnostics_location", s("exports/diagnostics")),
        ]);
    };
    dict([
        ("layout", s("app_state_root")),
        ("paths_available", Value::Bool(true)),
        ("database_exists", Value::Bool(p.database.exists())),
        (
            "settings_file_exists",
            Value::Bool(p.settings_file.exists()),
        ),
        ("exports_root_exists", Value::Bool(p.exports_root.exists())),
        (
            "attachments_root_exists",
            Value::Bool(p.attachments_root.exists()),
        ),
        ("diagnostics_location", s("exports/diagnostics")),
    ])
}
/// Runtime identity uses the historical schema keys, with honest Rust values.
fn environment(version: &str) -> Value {
    #[cfg(unix)]
    let (system, release, machine) = nix::sys::utsname::uname()
        .map(|u| {
            (
                u.sysname().to_string_lossy().into_owned(),
                u.release().to_string_lossy().into_owned(),
                u.machine().to_string_lossy().into_owned(),
            )
        })
        .unwrap_or_default();
    #[cfg(not(unix))]
    let (system, release, machine) = (
        std::env::consts::OS.to_string(),
        String::new(),
        std::env::consts::ARCH.to_string(),
    );
    dict([
        ("kassiber_version", s(version)),
        ("machine_schema_version", Value::Int(1.into())),
        (
            "python_version",
            s(env!("KASSIBER_DIAGNOSTICS_RUST_VERSION")),
        ),
        ("python_implementation", s("Rust")),
        (
            "platform",
            dict([
                ("system", s(system)),
                ("release", s(release)),
                ("machine", s(machine)),
            ]),
        ),
        ("sqlite_version", s(kassiber_store::rusqlite::version())),
    ])
}
/// Construct a report using already-redacted local state; the schema/order is Python's.
pub fn collect(
    args: &Value,
    version: &str,
    generated_at: &str,
    state: Option<Value>,
    checks: Value,
    error: Option<&AppError>,
) -> Value {
    let mut fields=vec![("report_schema_version",Value::Int(1.into())),("public_safe",Value::Bool(true)),("generated_at",s(generated_at)),("notice",s("This report is designed for public bug reports. It omits labels, paths, addresses, txids, exact amounts, notes, raw config, API payloads, and stack locals.")),("privacy_contract",kassiber_pytext::loads(include_str!("privacy.json")).expect("privacy contract")),("environment",environment(version)),("invocation",invocation(args)),("storage",storage(paths(args).as_ref())),("state",state.unwrap_or_else(||dict([("available",Value::Bool(false))]))),("checks",checks)];
    if let Some(error) = error {
        fields.push((
            "error",
            dict([
                ("type", s("AppError")),
                (
                    "code",
                    s(sanitize_text(if error.code.is_empty() {
                        "internal_error"
                    } else {
                        &error.code
                    })),
                ),
                (
                    "message",
                    s(sanitize_text(if error.message.is_empty() {
                        "AppError"
                    } else {
                        &error.message
                    })),
                ),
                (
                    "hint",
                    error
                        .hint
                        .as_deref()
                        .map(sanitize_text)
                        .map(s)
                        .unwrap_or(Value::Null),
                ),
                (
                    "details",
                    sanitize_details(error.details.as_ref().unwrap_or(&Value::Null)),
                ),
                ("retryable", Value::Bool(error.retryable)),
                ("unhandled", Value::Bool(false)),
            ]),
        ));
        // A native public-safe unwind trace is not available; never fabricate Python frames.
        fields.push(("stack", public_stack()));
    }
    dict(fields)
}
/// Empty is intentional: do not serialize backtrace Debug/Display or panic contexts.
pub fn public_stack() -> Value {
    Value::List(Vec::new())
}
/// UTC seconds, without consulting the network or opening any book.
pub fn now() -> Result<String, AppError> {
    #[cfg(feature = "test-hooks")]
    if let Ok(value) = std::env::var("KASSIBER_RUST_TEST_DIAGNOSTICS_TIME") {
        return Ok(value);
    }

    let conn = kassiber_store::rusqlite::Connection::open_in_memory()
        .map_err(|_| AppError::new("internal_error", "Unable to create diagnostics timestamp"))?;
    conn.query_row("SELECT strftime('%Y-%m-%dT%H:%M:%SZ','now')", [], |r| {
        r.get(0)
    })
    .map_err(|_| AppError::new("internal_error", "Unable to create diagnostics timestamp"))
}
pub struct Saved {
    pub path: PathBuf,
    pub relative_path: Option<String>,
    pub target: &'static str,
}
impl Saved {
    pub fn payload(&self) -> Value {
        dict([
            ("target", s(self.target)),
            (
                "relative_path",
                self.relative_path.as_deref().map(s).unwrap_or(Value::Null),
            ),
            (
                "filename",
                s(self
                    .path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()),
            ),
        ])
    }
    pub fn location(&self) -> String {
        self.relative_path
            .clone()
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }
}
/// Preserve Python's explicit-export behavior, including direct writes and umask.
pub fn save(report: &Value, target: &str, args: &Value) -> std::io::Result<Option<Saved>> {
    if target.is_empty() {
        return Ok(None);
    }
    let (mut path, mut relative_path, target) = if target == "auto" {
        let p = paths(args)
            .ok_or_else(|| std::io::Error::other("Unable to resolve diagnostics paths"))?;
        let fallback = now().map_err(std::io::Error::other)?;
        let stamp = text(report, "generated_at")
            .filter(|stamp| !stamp.is_empty())
            .unwrap_or(&fallback);
        let stamp = stamp
            .replace([':', '/'], "-")
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || "_.-".contains(c) {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>();
        let filename = format!("kassiber-diagnostics-{stamp}.json");
        (
            p.exports_root.join("diagnostics").join(&filename),
            Some(format!("exports/diagnostics/{filename}")),
            "auto",
        )
    } else {
        let p = Context::from_env()
            .map_err(|_| std::io::Error::other("Could not determine home directory."))?
            .expand_user(Path::new(target))
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        (p, None, "custom")
    };
    output_parents(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    let bytes = dumps(
        report,
        &Options {
            indent: Some(2.into()),
            ..Options::default()
        },
    )
    .map_err(std::io::Error::other)?
        + "\n";
    if target == "auto" {
        use std::io::Write;
        let original = path.clone();
        let stem = original.file_stem().expect("report stem").to_string_lossy();
        let mut attempt: u64 = 1;
        loop {
            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(bytes.as_bytes())
                        .map_err(|e| output_io_error(e, Some(&path)))?;
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    attempt += 1;
                    let filename = format!("{stem}-{attempt}.json");
                    path = original.with_file_name(&filename);
                    relative_path = Some(format!("exports/diagnostics/{filename}"));
                }
                Err(error) => return Err(output_io_error(error, Some(&path))),
            }
        }
    } else {
        std::fs::write(&path, bytes).map_err(|e| output_io_error(e, Some(&path)))?;
    }
    Ok(Some(Saved {
        path,
        relative_path,
        target,
    }))
}

fn output_io_error(error: std::io::Error, path: Option<&Path>) -> std::io::Error {
    let message = match error.raw_os_error() {
        Some(number) => {
            let text = error.to_string();
            let suffix = format!(" (os error {number})");
            let strerror = text.strip_suffix(&suffix).unwrap_or(&text);
            let label = if cfg!(windows) { "WinError" } else { "Errno" };
            let mut message = format!("[{label} {number}] {strerror}");
            if let Some(path) = path {
                message.push_str(": ");
                message.push_str(&repr_str(&path.to_string_lossy()));
            }
            message
        }
        None if path.is_some_and(|path| path.to_string_lossy().contains('\0')) => {
            "embedded null byte".to_owned()
        }
        None => error.to_string(),
    };
    std::io::Error::new(error.kind(), message)
}

fn output_parents(path: &Path) -> std::io::Result<()> {
    match std::fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = path.parent().filter(|parent| *parent != path) else {
                return Err(output_io_error(error, Some(path)));
            };
            output_parents(if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            })?;
            std::fs::create_dir(path).or_else(|error| {
                if path.is_dir() {
                    Ok(())
                } else {
                    Err(output_io_error(error, Some(path)))
                }
            })
        }
        Err(_) if path.is_dir() => Ok(()),
        Err(error) => Err(output_io_error(error, Some(path))),
    }
}

fn omitted_argument(value: &Value) -> bool {
    match value {
        Value::Null | Value::Bool(false) => true,
        Value::Int(v) => v == &0.into(),
        Value::Float(v) => *v == 0.0,
        Value::Decimal(v) => v.is_zero(),
        Value::List(v) => v.is_empty(),
        _ => false,
    }
}

/// Native provenance deliberately exports no frame strings, even in debug envelopes.
pub const EMPTY_NATIVE_DEBUG: &str = "";
pub fn io_exception_type(error: &std::io::Error) -> &'static str {
    use std::io::ErrorKind;
    match error.kind() {
        ErrorKind::NotFound => "FileNotFoundError",
        ErrorKind::PermissionDenied => "PermissionError",
        ErrorKind::AlreadyExists => "FileExistsError",
        ErrorKind::NotADirectory => "NotADirectoryError",
        ErrorKind::IsADirectory => "IsADirectoryError",
        ErrorKind::InvalidInput => "ValueError",
        _ => "OSError",
    }
}
/// Preserve the Python exception identity and handled/unhandled flags at the boundary.
pub fn exception_identity(report: &mut Value, exception_type: &str, unhandled: bool) {
    if let Some(mut error) = get(report, "error").cloned() {
        kassiber_envelope::set(&mut error, "type", s(exception_type));
        kassiber_envelope::set(&mut error, "unhandled", Value::Bool(unhandled));
        kassiber_envelope::set(report, "error", error);
    }
}

/// The CLI includes a traceback string only under --debug, even for unhandled failures.
pub fn native_debug(_unhandled: bool, requested: bool) -> Option<&'static str> {
    requested.then_some(EMPTY_NATIVE_DEBUG)
}
