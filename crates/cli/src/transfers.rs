use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub fn execute(book: &mut dyn Book, operation: &str, args: &Namespace) -> Result<Value, AppError> {
    if matches!(operation, "transfers.gaps.plan" | "transfers.gaps.apply") {
        book.resolve_scope(args.get("workspace"), args.get("profile"))?;
        let mut payload = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
        kassiber_envelope::set(&mut payload, "authored_source", kassiber_envelope::s("cli"));
        return book.transfers(operation, &payload);
    }
    if matches!(
        operation,
        "transfers.components.plan" | "transfers.components.apply"
    ) {
        return components(book, operation, args);
    }
    let mut payload = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    if matches!(
        operation,
        "transfers.pair" | "transfers.update" | "transfers.payouts.create"
    ) {
        if let Some(note) = args
            .items()
            .iter()
            .find(|(k, v)| k == "note" && !matches!(v, Value::Null))
            .map(|(_, v)| v.clone())
        {
            kassiber_envelope::set(&mut payload, "notes", note);
        }
    }
    if operation == "transfers.rules.create" {
        let predicate = args.text("predicate").unwrap_or("{}");
        let predicate = kassiber_pytext::loads(predicate).map_err(|e| {
            AppError::new(
                if e.is_decode_error() {
                    "validation"
                } else {
                    "internal_error"
                },
                format!("Invalid --predicate JSON: {e}"),
            )
        })?;
        kassiber_envelope::set(&mut payload, "predicate", predicate);
        kassiber_envelope::set(&mut payload, "enabled", Value::Bool(!args.flag("disabled")));
    }
    book.transfers(operation, &payload)
}

/// `transfers components plan|apply`: the handler's argument shaping after scope.
fn components(book: &mut dyn Book, operation: &str, args: &Namespace) -> Result<Value, AppError> {
    use kassiber_envelope::set;
    book.resolve_scope(args.get("workspace"), args.get("profile"))?;
    let action = args.text("action").unwrap_or_default().to_owned();
    let (draft, activate) = (args.flag("draft"), args.flag("activate"));
    if action == "revise" && draft {
        return Err(AppError::new(
            "validation",
            "revise uses --activate; --draft is not accepted",
        ));
    }
    let inline = args.get("json_text").filter(|v| !matches!(v, Value::Null));
    let file = args.text("json_file");
    let has_json = inline.is_some() || file.is_some();
    let document = if has_json {
        let label = if action == "revise" {
            "component revision"
        } else {
            "component array or bulk object"
        };
        Some(json_document(args.text("json_text"), file, label)?)
    } else {
        None
    };
    let mut components = if action == "create" {
        document.clone().unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    if matches!(components, Value::Dict(_)) {
        components = kassiber_envelope::get(&components, "components")
            .cloned()
            .unwrap_or(Value::Null);
    }
    let spec = if action == "revise" {
        document.unwrap_or(Value::Null)
    } else if has_json && action != "create" {
        dict::<&str>([])
    } else {
        Value::Null
    };
    let activate = match action.as_str() {
        "create" => Value::Bool(!draft),
        "revise" => Value::Bool(activate),
        _ if draft || activate => Value::Bool(false),
        _ => Value::Null,
    };
    let mut payload = dict([
        ("action", kassiber_envelope::s(action.as_str())),
        ("components", components),
        (
            "component_id",
            args.get("component_id").cloned().unwrap_or(Value::Null),
        ),
        ("spec", spec),
        ("activate", activate),
        ("reason", args.get("reason").cloned().unwrap_or(Value::Null)),
        ("authored_source", kassiber_envelope::s("cli")),
    ]);
    for key in ["workspace", "profile", "expected_input_version"] {
        if let Some(v) = args.get(key) {
            set(&mut payload, key, v.clone());
        }
    }
    book.transfers(operation, &payload)
}

/// `_read_json_document` over `read_text_argument`, with CPython's messages.
fn json_document(inline: Option<&str>, file: Option<&str>, label: &str) -> Result<Value, AppError> {
    let raw = match (inline.filter(|t| !t.is_empty()), file) {
        (Some(text), _) => kassiber_pytext::strip(text).to_owned(),
        (None, Some(path)) => {
            let text = read_text(path)?;
            let text = kassiber_pytext::strip(&text).to_owned();
            if text.is_empty() {
                return Err(AppError::new(
                    "app_error",
                    format!("{label} file '{path}' is empty"),
                ));
            }
            text
        }
        (None, None) => {
            return Err(AppError::new(
                "validation",
                format!("{label} JSON input is required"),
            ))
        }
    };
    kassiber_pytext::loads(&raw).map_err(|e| {
        if !e.is_decode_error() {
            return AppError::new("internal_error", e.to_string());
        }
        let source = match inline.filter(|t| !t.is_empty()) {
            Some(_) => "inline JSON".to_owned(),
            None => file.unwrap_or_default().to_owned(),
        };
        AppError::new(
            "validation",
            format!("Invalid {label} JSON from {source}: {e}"),
        )
    })
}

/// `Path(path).expanduser().read_text(encoding="utf-8")`.
fn read_text(path: &str) -> Result<String, AppError> {
    let home = std::env::var("HOME").ok();
    let expanded = crate::render::expand_user(path, home.as_deref())
        .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    let bytes = std::fs::read(&expanded).map_err(|error| {
        let message = match error.raw_os_error() {
            Some(number) => {
                let text = error.to_string();
                let suffix = format!(" (os error {number})");
                format!(
                    "[Errno {number}] {}: {}",
                    text.strip_suffix(&suffix).unwrap_or(&text),
                    kassiber_pytext::repr_str(&expanded.to_string_lossy())
                )
            }
            None => error.to_string(),
        };
        AppError::new("internal_error", message)
    })?;
    let text = String::from_utf8(bytes).map_err(|e| {
        let bytes = e.as_bytes();
        let error = e.utf8_error();
        let at = error.valid_up_to();
        let (end, reason) = match error.error_len() {
            Some(1) if matches!(bytes[at], 0x80..=0xC1 | 0xF5..=0xFF) => (at, "invalid start byte"),
            Some(n) => (at + n - 1, "invalid continuation byte"),
            None => (bytes.len() - 1, "unexpected end of data"),
        };
        AppError::new(
            "internal_error",
            if end == at {
                format!(
                    "'utf-8' codec can't decode byte 0x{:02x} in position {at}: {reason}",
                    bytes[at]
                )
            } else {
                format!("'utf-8' codec can't decode bytes in position {at}-{end}: {reason}")
            },
        )
    })?;
    Ok(text.replace("\r\n", "\n").replace('\r', "\n"))
}
