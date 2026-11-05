//! Existing desktop history transports; domain rules stay in the shared service.
use kassiber_envelope::{dict, get, py_repr, s, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_pytext::{parse_int, strip};
use kassiber_runtime::store::Book;
use kassiber_transactions::metadata::{
    Bip329, Command, GroupedUpdate, HistoryOptions, PricingUpdate,
};

pub const KINDS: &[&str] = &[
    "ui.transactions.metadata.update",
    "ui.transactions.history",
    "ui.transactions.history.revert",
    "ui.activity.history",
    "ui.activity.stale",
    "ui.metadata.bip329.preview",
    "ui.metadata.bip329.import",
    "ui.metadata.bip329.export",
];

fn optional(args: &Value, key: &str) -> Result<Option<String>, AppError> {
    match get(args, key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Str(t)) if t.as_str().is_some() => Ok(t
            .as_str()
            .map(strip)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)),
        Some(value) => Err(
            AppError::new("validation", format!("{key} must be a string"))
                .with_details(dict([("type", s(type_name(value)))])),
        ),
    }
}

fn required(args: &Value, key: &str, label: &str) -> Result<String, AppError> {
    optional(args, key)?.ok_or_else(|| {
        AppError::new("validation", format!("{label} is required."))
            .with_hint(format!("Enter {}.", kassiber_pytext::lower(label)))
    })
}

fn flag(args: &Value, key: &str, default: bool) -> Result<bool, AppError> {
    match get(args, key) {
        None => Ok(default),
        Some(Value::Bool(b)) => Ok(*b),
        Some(v) => Err(
            AppError::new("validation", format!("{key} must be a boolean"))
                .with_details(dict([("type", s(type_name(v)))])),
        ),
    }
}

fn limit(args: &Value) -> Result<Option<i64>, AppError> {
    let integer = match get(args, "limit") {
        None | Some(Value::Null) => return Ok(None),
        Some(Value::Int(n)) => n.clone(),
        Some(Value::Bool(b)) => i64::from(*b).into(),
        Some(Value::Float(n)) if n.is_finite() => return Ok(Some(*n as i64)),
        Some(Value::Float(n)) => {
            return Err(AppError::new(
                "internal_error",
                if n.is_nan() {
                    "cannot convert float NaN to integer"
                } else {
                    "cannot convert float infinity to integer"
                },
            ))
        }
        Some(Value::Str(t)) => parse_int(t.as_str().unwrap_or_default()).map_err(|_| {
            AppError::new(
                "internal_error",
                format!(
                    "invalid literal for int() with base 10: {}",
                    py_repr(&Value::Str(t.clone()))
                ),
            )
        })?,
        Some(v) => {
            return Err(AppError::new(
                "internal_error",
                format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                type_name(v)
            ),
            ))
        }
    };
    let sign = integer > 0.into();
    Ok(Some(i64::try_from(integer).unwrap_or(if sign {
        i64::MAX
    } else {
        i64::MIN
    })))
}

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(pairs) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    if kind.starts_with("ui.metadata.bip329.") {
        let command = if kind == "ui.metadata.bip329.export" {
            let wallets = match get(args, "wallets") {
                None | Some(Value::Null) => match get(args, "wallet") {
                    None | Some(Value::Null) => None,
                    Some(Value::Str(t)) if t.as_str().is_some() => t
                        .as_str()
                        .filter(|s| !s.is_empty())
                        .map(|s| vec![s.to_owned()]),
                    _ => {
                        return Err(AppError::new(
                            "validation",
                            format!("{kind} wallet must be a string"),
                        ))
                    }
                },
                Some(value) => {
                    if get(args, "wallet").is_some_and(|v| !matches!(v, Value::Null)) {
                        return Err(AppError::new(
                            "validation",
                            format!("{kind} takes wallet or wallets, not both"),
                        ));
                    }
                    let Value::List(items) = value else {
                        return Err(AppError::new(
                            "validation",
                            format!("{kind} wallets must be an array of strings"),
                        ));
                    };
                    Some(
                        items
                            .iter()
                            .map(|v| match v {
                                Value::Str(t) if t.as_str().is_some() => {
                                    Ok(t.as_str().unwrap().to_owned())
                                }
                                _ => Err(AppError::new(
                                    "validation",
                                    format!("{kind} wallets must be an array of strings"),
                                )),
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    )
                }
            };
            Bip329::DesktopExport {
                wallets,
                mode: optional(args, "mode")?.unwrap_or_else(|| "stored".into()),
            }
        } else {
            let file = required(args, "file", "BIP329 label file")?;
            let path = if file == "~" || file.starts_with("~/") {
                std::env::var_os("HOME")
                    .map(std::path::PathBuf::from)
                    .map(|p| p.join(file.strip_prefix("~/").unwrap_or("")))
                    .unwrap_or_else(|| std::path::PathBuf::from(&file))
            } else {
                std::path::PathBuf::from(&file)
            };
            if !path.exists() {
                return Err(AppError::new(
                    "not_found",
                    format!("BIP329 label file not found: {file}"),
                )
                .with_hint("Choose an existing local JSONL label export."));
            }
            let file = path
                .canonicalize()
                .map_err(|e| AppError::new("internal_error", e.to_string()))?
                .to_string_lossy()
                .into_owned();
            if kind.ends_with(".preview") {
                Bip329::Preview { file }
            } else {
                Bip329::Import {
                    file,
                    apply_ambiguous: flag(args, "apply_ambiguous", false)?,
                    source: "gui".into(),
                }
            }
        };
        return book.metadata(None, None, &Command::Bip329(command));
    }
    let allowed: &[&str] = match kind {
        "ui.transactions.metadata.update" => &[
            "transaction",
            "note",
            "tags",
            "excluded",
            "source",
            "reason",
            "fiat_currency",
            "fiat_rate",
            "fiat_value",
            "pricing_source_kind",
            "pricing_quality",
            "pricing_external_ref",
            "review_status",
            "taxable",
            "at_regime",
            "at_category",
            "kind",
        ],
        "ui.activity.stale" => &[],
        "ui.transactions.history.revert" => &["transaction", "event", "field", "source", "reason"],
        "ui.transactions.history" => &[
            "transaction",
            "source",
            "field_family",
            "field",
            "pricing_only",
            "ai_only",
            "stale_only",
            "start",
            "end",
            "cursor",
            "limit",
            "include_stale",
        ],
        _ => &[
            "transaction",
            "wallet",
            "source",
            "field_family",
            "field",
            "pricing_only",
            "ai_only",
            "stale_only",
            "start",
            "end",
            "cursor",
            "limit",
            "include_stale",
        ],
    };
    let mut unknown: Vec<_> = pairs
        .iter()
        .filter_map(|(k, _)| match k {
            Key::Str(s) if !allowed.contains(&s.as_str().unwrap_or_default()) => {
                Some(Value::Str(s.clone()))
            }
            _ => None,
        })
        .collect();
    unknown.sort_by_key(kassiber_envelope::py_str);
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            if kind == "ui.activity.stale" {
                format!("{kind} does not accept arguments")
            } else {
                format!("{kind} received unsupported fields")
            },
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let command = match kind {
        "ui.transactions.metadata.update" => {
            let pricing_fields = [
                "fiat_currency",
                "fiat_rate",
                "fiat_value",
                "pricing_source_kind",
                "pricing_quality",
                "pricing_external_ref",
            ];
            let value = |key| get(args, key).cloned().unwrap_or(Value::Null);
            Command::Update(Box::new(GroupedUpdate {
                transaction: required(args, "transaction", "transaction id")?,
                source: optional(args, "source")?.unwrap_or_else(|| "gui".into()),
                reason: optional(args, "reason")?.map(Into::into),
                note: get(args, "note").cloned(),
                tags: get(args, "tags").cloned(),
                excluded: get(args, "excluded").cloned(),
                pricing: pricing_fields
                    .iter()
                    .any(|key| get(args, key).is_some())
                    .then(|| PricingUpdate {
                        fiat_currency: value("fiat_currency"),
                        fiat_rate: value("fiat_rate"),
                        fiat_value: value("fiat_value"),
                        source_kind: value("pricing_source_kind"),
                        quality: value("pricing_quality"),
                        external_ref: value("pricing_external_ref"),
                        method: Value::Null,
                    }),
                review_status: get(args, "review_status").cloned(),
                taxable: get(args, "taxable").cloned(),
                at_regime: get(args, "at_regime").cloned(),
                at_category: get(args, "at_category").cloned(),
                kind: get(args, "kind").cloned(),
                // Only a review plan accepts coarse prices.
                accept_coarse_price: false,
            }))
        }
        "ui.activity.stale" => Command::Stale,
        "ui.transactions.history.revert" => Command::Revert {
            transaction: required(args, "transaction", "transaction id")?,
            event: required(args, "event", "event id")?,
            field: optional(args, "field")?,
            source: optional(args, "source")?.unwrap_or_else(|| "gui".into()),
            reason: optional(args, "reason")?,
        },
        _ => Command::History(HistoryOptions {
            transaction: if kind == "ui.transactions.history" {
                Some(required(args, "transaction", "transaction id")?)
            } else {
                optional(args, "transaction")?
            },
            wallet: optional(args, "wallet")?,
            source: optional(args, "source")?,
            field_family: optional(args, "field_family")?,
            field: optional(args, "field")?,
            pricing_only: flag(args, "pricing_only", false)?,
            ai_only: flag(args, "ai_only", false)?,
            stale_only: flag(args, "stale_only", false)?,
            start: optional(args, "start")?,
            end: optional(args, "end")?,
            cursor: optional(args, "cursor")?,
            limit: limit(args)?,
            include_stale: flag(args, "include_stale", true)?,
        }),
    };
    book.metadata(None, None, &command)
}
