use super::*;
use kassiber_pytext::{lower, strip, upper};
use kassiber_setup::Sources;
use kassiber_store::rusqlite::{params, types::Value as SqlValue};

const SOURCE_TYPES: &[&str] = &[
    "fiat_purchase",
    "exchange_withdrawal",
    "mining",
    "income",
    "gift",
    "opening_balance_attestation",
    "missing_history",
    "unknown",
];

pub fn handles(command: &str) -> bool {
    matches!(
        command,
        "source-funds.sources.create"
            | "source-funds.sources.attach"
            | "ui.source_funds.sources.create"
            | "ui.source_funds.sources.attach"
    )
}

pub(super) fn optional(v: &Value) -> bool {
    matches!(v, Value::Null) || matches!(v, Value::Str(raw) if raw.as_str() == Some(""))
}

pub(super) fn parameter(conn: &Connection, v: &Value, index: usize) -> Result<SqlValue> {
    Ok(match v {
        Value::Null => SqlValue::Null,
        Value::Bool(v) => SqlValue::Integer(i64::from(*v)),
        Value::Int(v) => SqlValue::Integer(i64::try_from(v.clone()).map_err(|_| {
            AppError::new(
                "internal_error",
                "Python int too large to convert to SQLite INTEGER",
            )
        })?),
        Value::Float(v) => SqlValue::Real(*v),
        Value::Str(v) => SqlValue::Text(
            v.as_str()
                .ok_or_else(|| AppError::new("internal_error", "surrogates not allowed"))?
                .into(),
        ),
        _ => {
            let encrypted = !rows(conn, "PRAGMA cipher_salt", [])?.is_empty();
            return Err(AppError::new(
                "internal_error",
                if encrypted {
                    format!(
                        "Error binding parameter {} - probably unsupported type.",
                        index - 1
                    )
                } else {
                    format!(
                        "Error binding parameter {index}: type '{}' is not supported",
                        kassiber_envelope::type_name(v)
                    )
                },
            ));
        }
    })
}

pub(super) enum Amount {
    Missing,
    Msat(i64),
    Overflow,
}

pub(super) fn amount(v: &Value) -> Result<Amount> {
    if optional(v) {
        return Ok(Amount::Missing);
    }
    let raw = py_str(v);
    let d: Decimal = strip(&raw).parse().map_err(|error| {
        let normalized = lower(strip(&raw));
        if error != kassiber_pydecimal::DecimalError::NonFinite {
            AppError::new("app_error", format!("Invalid decimal value: {raw}"))
        } else if normalized.contains("snan") {
            AppError::new("internal_error", "[<class 'decimal.InvalidOperation'>]")
        } else if normalized.contains("nan") {
            AppError::new("internal_error", "cannot convert NaN to integer")
        } else if matches!(
            normalized.as_str(),
            "inf" | "+inf" | "-inf" | "infinity" | "+infinity" | "-infinity"
        ) {
            AppError::new("internal_error", "cannot convert Infinity to integer")
        } else {
            AppError::new("app_error", format!("Invalid decimal value: {raw}"))
        }
    })?;
    let scaled = d
        .mul(&Decimal::from(100_000_000_000i64), &DECIMAL)
        .map_err(arithmetic)?;
    if scaled.is_negative() && scaled.copy_abs() >= "0.5".parse().unwrap() {
        return Err(AppError::new("validation", "--amount must not be negative"));
    }
    if scaled.adjusted() > 18 {
        return Ok(Amount::Overflow);
    }
    let rounded = scaled
        .to_fixed_places(0, Rounding::HalfUp)
        .map_err(arithmetic)?;
    Ok(rounded
        .parse()
        .map(Amount::Msat)
        .unwrap_or(Amount::Overflow))
}

pub(super) fn required_text(args: &Value, key: &str, command: &str) -> Result<String> {
    match value(args, key) {
        Value::Str(raw) if raw.as_str().is_some_and(|v| !strip(v).is_empty()) => {
            Ok(strip(raw.as_str().unwrap()).into())
        }
        _ => Err(AppError::new(
            "validation",
            format!("{command} requires args.{key}"),
        )),
    }
}

pub(super) fn require_attachment(conn: &Connection, profile: &str, attachment: &str) -> Result<()> {
    if rows(
        conn,
        "SELECT id FROM attachments WHERE profile_id=? AND id=?",
        [profile, attachment],
    )?
    .is_empty()
    {
        return Err(AppError::new(
            "not_found",
            format!("Attachment '{attachment}' not found"),
        ));
    }
    Ok(())
}

pub(super) fn resolve_source(conn: &Connection, profile: &str, reference: &str) -> Result<Value> {
    let found = rows(conn, "SELECT * FROM source_funds_sources WHERE profile_id=? AND (id=? OR lower(label)=lower(?)) ORDER BY created_at DESC,id DESC LIMIT 2", [profile, reference, reference])?;
    if found.len() > 1 {
        return Err(AppError::new(
            "ambiguous_reference",
            format!("Source-funds source '{reference}' is ambiguous"),
        )
        .with_hint("Use the source id from `source-funds sources list`."));
    }
    found.into_iter().next().ok_or_else(|| {
        AppError::new(
            "not_found",
            format!("Source-funds source '{reference}' not found"),
        )
    })
}

pub fn execute(
    conn: &Connection,
    command: &str,
    args: &Value,
    sources: &dyn Sources,
) -> Result<Value> {
    let desktop = command.starts_with("ui.");
    let empty = dict::<&str>([]);
    let args = if desktop {
        match args {
            Value::Null => &empty,
            Value::Dict(_) => args,
            _ => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(kassiber_envelope::type_name(args)))])))
            }
        }
    } else {
        args
    };
    let mut attachments = Vec::new();
    let creating = command.ends_with(".create");
    let (source, attachment) = if !creating && desktop {
        (
            required_text(args, "source", command)?,
            required_text(args, "attachment_id", command)?,
        )
    } else {
        (text(args, "source"), text(args, "attachment"))
    };
    if creating {
        let raw = value(
            args,
            if desktop {
                "attachment_ids"
            } else {
                "attachments"
            },
        );
        if matches!(raw, Value::Null) && desktop {
            if let Value::Str(id) = value(args, "attachment_id") {
                if !id.is("") {
                    attachments.push(py_str(&Value::Str(id.clone())));
                }
            }
        } else if let Value::List(ids) = raw {
            attachments = ids.iter().map(py_str).collect();
        } else if desktop {
            return Err(AppError::new(
                "validation",
                "ui.source_funds.sources.create attachment_ids must be a list",
            ));
        }
    }
    let (workspace, profile) =
        kassiber_setup::references::scope(conn, if desktop { &empty } else { args })?;
    let profile_id = text(&profile, "id");
    crate::export::admitted(conn, || {
        if !creating {
            let source = resolve_source(conn, &profile_id, &source)?;
            require_attachment(conn, &profile_id, &attachment)?;
            conn.execute("INSERT OR IGNORE INTO source_funds_source_attachments(source_id,attachment_id,created_at) VALUES(?,?,?)", params![text(&source,"id"),attachment,sources.now(conn)?]).map_err(sql_error)?;
            return super::source_funds_reads::source(conn, &source);
        }
        let id = sources.uuid(conn)?;
        let now = sources.now(conn)?;
        let fallback = |key: &str, default: &str| {
            let v = value(args, key);
            if desktop && !truthy(v) {
                default.into()
            } else {
                py_str(v)
            }
        };
        let asset = kassiber_wallets::config::asset_code(&fallback("asset", "BTC"));
        let amount = amount(value(args, "amount"))?;
        let raw_type = fallback("source_type", "");
        let source_type = lower(strip(&raw_type)).replace('-', "_");
        if !SOURCE_TYPES.contains(&source_type.as_str()) {
            return Err(AppError::new(
                "validation",
                format!("Unsupported source type '{raw_type}'"),
            )
            .with_hint(format!("Choose one of: {}", SOURCE_TYPES.join(", "))));
        }
        let fiat = if optional(value(args, "fiat_value")) {
            None
        } else {
            let d = kassiber_pydecimal::finite_price(&py_str(value(args, "fiat_value")))
                .map_err(|e| AppError::new("validation", e.message("fiat_value")))?;
            Some(
                d.to_string()
                    .parse::<f64>()
                    .map_err(|_| AppError::new("internal_error", "invalid fiat value"))?,
            )
        };
        let currency = value(args, "fiat_currency");
        let currency = if truthy(currency) {
            currency
        } else {
            value(&profile, "fiat_currency")
        };
        let currency = if !truthy(currency) {
            String::new()
        } else {
            match currency {
                Value::Str(raw) => upper(strip(raw.as_str().unwrap_or_default())),
                _ => {
                    return Err(AppError::new(
                        "internal_error",
                        format!(
                            "'{}' object has no attribute 'strip'",
                            kassiber_envelope::type_name(currency)
                        ),
                    ))
                }
            }
        };
        let acquired = value(args, "acquired_at");
        let acquired = if optional(acquired) {
            None
        } else if !truthy(acquired) {
            return Err(AppError::new("app_error", "Missing occurred_at/date value"));
        } else {
            Some(kassiber_wallets::time::timestamp(&py_str(acquired))?)
        };
        let label = fallback("label", "");
        let label = strip(&label);
        if label.is_empty() {
            return Err(AppError::new("validation", "--label cannot be empty"));
        }
        let amount = match amount {
            Amount::Missing => None,
            Amount::Msat(n) => Some(n),
            Amount::Overflow => {
                return Err(AppError::new(
                    "internal_error",
                    "Python int too large to convert to SQLite INTEGER",
                ))
            }
        };
        let description = parameter(conn, value(args, "description"), 11)?;
        conn.execute("INSERT INTO source_funds_sources(id,workspace_id,profile_id,source_type,label,asset,amount,fiat_currency,fiat_value,acquired_at,description,review_state,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,'reviewed',?,?)", params![id,text(&workspace,"id"),profile_id,source_type,label,asset,amount,(!currency.is_empty()).then_some(currency),fiat,acquired,description,now,now]).map_err(sql_error)?;
        for attachment in &attachments {
            require_attachment(conn, &profile_id, attachment)?;
            conn.execute("INSERT OR IGNORE INTO source_funds_source_attachments(source_id,attachment_id,created_at) VALUES(?,?,?)", params![id,attachment,now]).map_err(sql_error)?;
        }
        let found = rows(conn, "SELECT * FROM source_funds_sources WHERE id=?", [&id])?;
        let row = found.first().ok_or_else(|| {
            AppError::new("internal_error", "'NoneType' object is not subscriptable")
        })?;
        super::source_funds_reads::source(conn, row)
    })
}
