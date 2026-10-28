//! Desktop report transports; all computation lives in kassiber-reports.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const KINDS: &[&str] = &[
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.reports.tax_summary",
];
pub fn execute(book: &mut dyn Book, kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    if !matches!(kind, "ui.reports.summary" | "ui.reports.tax_summary") {
        return book.reports(kind, &dict::<&str>([]));
    }
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let mut unknown: Vec<_> = fields
        .iter()
        .filter(|(k, _)| {
            *k != if kind == "ui.reports.tax_summary" {
                "year"
            } else {
                "wallet"
            }
            .into()
        })
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) => Some(k.clone()),
            _ => None, // JSON object keys are strings.
        })
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            format!("{kind} received unsupported arguments"),
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(Value::Str).collect()),
        )])));
    }
    if kind == "ui.reports.tax_summary" {
        return book.reports(kind, &tax_summary_args(args)?);
    }
    let wallet = match get(args, "wallet") {
        None | Some(Value::Null) => Value::Null,
        Some(Value::Str(t))
            if t.as_str()
                .is_some_and(|t| !kassiber_pytext::strip(t).is_empty()) =>
        {
            s(kassiber_pytext::strip(t.as_str().unwrap()))
        }
        _ => {
            return Err(AppError::new(
                "validation",
                "ui.reports.summary wallet must be a non-empty string",
            ))
        }
    };
    book.reports(kind, &dict([("wallet", wallet)]))
}

fn tax_summary_args(args: &Value) -> Result<Value, AppError> {
    let raw = get(args, "year").unwrap_or(&Value::Null);
    let parsed = match raw {
        Value::Null => Some(Value::Null),
        Value::Int(_) => Some(raw.clone()),
        Value::Bool(v) => Some(Value::Int(i64::from(*v).into())),
        Value::Str(v) => v
            .as_str()
            .and_then(|s| kassiber_pytext::parse_int(s).ok())
            .map(Value::Int),
        Value::Float(v) if v.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", v.trunc()))
                .ok()
                .map(Value::Int)
        }
        Value::Float(v) if v.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        _ => None,
    };
    let year = parsed.ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.reports.tax_summary year must be an integer",
        )
        .with_details(dict([("year", raw.clone())]))
    })?;
    Ok(dict([("year", year)]))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tax_summary_daemon_arguments() {
        for (raw, expected) in [
            ("null", "null"),
            ("true", "1"),
            ("false", "0"),
            ("2024", "2024"),
            ("2024.9", "2024"),
            ("-2024.9", "-2024"),
            ("\" ２０２４ \"", "2024"),
            ("\"2_024\"", "2024"),
            ("99999999999999999999999999", "99999999999999999999999999"),
        ] {
            let args = dict([("year", kassiber_pytext::loads(raw).unwrap())]);
            let result = tax_summary_args(&args).unwrap();
            assert_eq!(
                get(&result, "year"),
                Some(&kassiber_pytext::loads(expected).unwrap())
            );
        }
        for raw in ["[]", "{}", "\"bad\"", "\"2024.5\"", "NaN"] {
            let args = dict([("year", kassiber_pytext::loads(raw).unwrap())]);
            let error = tax_summary_args(&args).unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.reports.tax_summary year must be an integer"
            );
            if raw == "NaN" {
                assert!(
                    matches!(get(error.details.as_ref().unwrap(),"year"),Some(Value::Float(n)) if n.is_nan())
                );
            } else {
                assert_eq!(error.details, Some(args));
            }
        }
    }
}
