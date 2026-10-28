//! Attachment transport; local storage and evidence reuse live in the service.
use kassiber_envelope::{dict, get, s, truthy, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_pytext::{parse_int, strip};
use kassiber_runtime::store::Book;
use kassiber_transactions::attachments::Command;

pub const KINDS: &[&str] = &[
    "ui.attachments.list",
    "ui.attachments.add",
    "ui.attachments.copy",
    "ui.attachments.rename",
    "ui.attachments.remove",
    "ui.attachments.open",
];

fn string(value: &Value) -> Option<&str> {
    match value {
        Value::Str(t) => t.as_str(),
        _ => None,
    }
}
fn optional(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => string(v)
            .map(|s| Some(s.to_owned()))
            .ok_or_else(|| AppError::new("validation", format!("{key} must be a string"))),
    }
}
fn either<'a>(args: &'a Value, first: &str, second: &str) -> Option<&'a Value> {
    get(args, first)
        .filter(|v| truthy(v))
        .or_else(|| get(args, second))
}
fn required(args: &Value, kind: &str, first: &str, second: &str) -> Result<String, AppError> {
    either(args, first, second)
        .and_then(string)
        .filter(|s| !strip(s).is_empty())
        .map(str::to_owned)
        .ok_or_else(|| AppError::new("validation", format!("{kind} requires args.{first}")))
}

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    if kind == "ui.attachments.list" {
        let Value::Dict(pairs) = args else {
            unreachable!()
        };
        let mut unknown: Vec<_> = pairs
            .iter()
            .filter_map(|(k, _)| match k {
                kassiber_pyjson::Key::Str(t)
                    if !["transaction", "limit", "cursor"]
                        .contains(&t.as_str().unwrap_or_default()) =>
                {
                    Some(Value::Str(t.clone()))
                }
                _ => None,
            })
            .collect();
        unknown.sort_by_key(kassiber_envelope::py_str);
        if !unknown.is_empty() {
            return Err(AppError::new(
                "validation",
                "ui.attachments.list received unsupported fields",
            )
            .with_details(dict([("unknown", Value::List(unknown))])));
        }
        let transaction = match get(args, "transaction") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                string(v)
                    .map(strip)
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| {
                        AppError::new(
                            "validation",
                            "ui.attachments.list transaction must be a non-empty string",
                        )
                    })?
                    .to_owned(),
            ),
        };
        let cursor = match get(args, "cursor") {
            None | Some(Value::Null) => 0,
            Some(v) => {
                let cursor = string(v)
                    .filter(|s| {
                        !s.is_empty()
                            && s.chars()
                                .all(|c| kassiber_pytext::decimal_value(c as u32).is_some())
                    })
                    .ok_or_else(|| {
                        AppError::new("validation", "ui.attachments.list cursor is invalid")
                    })?;
                let integer = parse_int(cursor).map_err(|_| {
                    AppError::new("validation", "ui.attachments.list cursor is invalid")
                })?;
                i64::try_from(integer)
                    .ok()
                    .filter(|n| *n <= i32::MAX as i64)
                    .ok_or_else(|| {
                        AppError::new("validation", "ui.attachments.list cursor is out of range")
                    })?
            }
        };
        let limit = match get(args, "limit") {
            None | Some(Value::Null) => None,
            Some(v) => {
                let parsed = match v {
                    Value::Int(n) => Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() {
                        200
                    } else {
                        -1
                    })),
                    Value::Bool(b) => Some(i64::from(*b)),
                    Value::Float(n) if n.is_finite() => Some(*n as i64),
                    Value::Str(t) => t.as_str().and_then(|s| parse_int(s).ok()).map(|n| {
                        i64::try_from(n.clone()).unwrap_or(if n > 0.into() { 200 } else { -1 })
                    }),
                    _ => None,
                };
                let n = parsed.ok_or_else(|| {
                    AppError::new("validation", "limit must be an integer")
                        .with_details(dict([("value", v.clone())]))
                })?;
                if n <= 0 {
                    return Err(AppError::new("validation", "limit must be positive")
                        .with_details(dict([("value", v.clone())])));
                }
                Some(n.min(200))
            }
        };
        let found = book.attachments(
            None,
            None,
            &Command::List {
                transaction,
                limit: limit.map(|n| n + 1),
                offset: cursor,
            },
        )?;
        let Value::List(mut found) = found else {
            return Err(AppError::new(
                "internal_error",
                "attachment service did not return a list",
            ));
        };
        let next = limit
            .filter(|n| found.len() > *n as usize)
            .map(|n| s((cursor + n).to_string()))
            .unwrap_or(Value::Null);
        if let Some(n) = limit {
            found.truncate(n as usize);
        }
        return Ok(dict([
            ("attachments", Value::List(found)),
            ("next_cursor", next),
        ]));
    }
    let command = match kind {
        "ui.attachments.add" => Command::Add {
            transaction: required(args, kind, "transaction", "transaction")?,
            file: either(args, "file_path", "file")
                .map(|v| optional(&dict([("file", v.clone())]), "file"))
                .transpose()?
                .flatten(),
            url: optional(args, "url")?,
            label: optional(args, "label")?,
            media_type: optional(args, "media_type")?,
        },
        "ui.attachments.copy" => Command::Copy {
            transaction: strip(&required(args, kind, "transaction", "target_transaction")?)
                .to_owned(),
            attachments: match either(args, "attachments", "attachment_ids") {
                Some(Value::List(items)) => items.clone(),
                _ => {
                    return Err(AppError::new(
                        "validation",
                        "ui.attachments.copy requires args.attachments",
                    ))
                }
            },
            source_transaction: strip(&required(
                args,
                kind,
                "source_transaction",
                "source_transaction",
            )?)
            .to_owned(),
        },
        "ui.attachments.rename" => Command::Rename {
            attachment: strip(&required(args, kind, "attachment", "attachment_id")?).to_owned(),
            label: required(args, kind, "label", "label")?,
        },
        "ui.attachments.remove" => Command::Remove {
            attachment: required(args, kind, "attachment", "attachment_id")?,
        },
        "ui.attachments.open" => Command::Open {
            attachment: required(args, kind, "attachment", "attachment_id")?,
        },
        _ => unreachable!("registered attachment kind"),
    };
    book.attachments(None, None, &command)
}
