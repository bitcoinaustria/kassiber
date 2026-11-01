//! Desktop report transports; all computation lives in kassiber-reports.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const KINDS: &[&str] = &[
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
    "ui.reports.tax_summary",
    "ui.reports.balance_history",
    "ui.reports.export_csv",
    "ui.reports.export_capital_gains_csv",
    "ui.reports.export_austrian_e1kv_xlsx",
    "ui.reports.export_austrian_e1kv_csv",
    "ui.transactions.export_csv",
    "ui.transactions.export_xlsx",
];
pub fn execute(book: &mut dyn Book, kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    if matches!(
        kind,
        "ui.reports.export_csv"
            | "ui.reports.export_capital_gains_csv"
            | "ui.reports.export_austrian_e1kv_csv"
            | "ui.reports.export_austrian_e1kv_xlsx"
            | "ui.transactions.export_csv"
            | "ui.transactions.export_xlsx"
    ) {
        let empty = dict::<&str>([]);
        let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
        if !matches!(args, Value::Dict(_)) {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(args)))])));
        }
        return book.reports(kind, args);
    }
    if kind == "ui.reports.balance_history" {
        return book.reports(kind, &history_args(raw)?);
    }
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

fn history_args(raw: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = raw.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let allowed = [
        "interval", "start", "end", "wallet", "account", "asset", "limit",
    ];
    let mut unknown: Vec<_> = fields
        .iter()
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) if !allowed.iter().any(|a| k == &(*a).into()) => {
                Some(Value::Str(k.clone()))
            }
            _ => None,
        })
        .collect();
    unknown.sort_by(|a, b| match (a, b) {
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => unreachable!(), // JSON object keys are strings.
    });
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history received unsupported arguments",
        )
        .with_details(dict([("unknown", Value::List(unknown))])));
    }
    let interval = get(args, "interval").cloned().unwrap_or_else(|| s("month"));
    if !["hour", "day", "week", "month"]
        .iter()
        .any(|i| interval == s(*i))
    {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history interval is unsupported",
        )
        .with_details(dict([
            ("interval", interval),
            (
                "supported",
                Value::List(["hour", "day", "week", "month"].map(s).into()),
            ),
        ])));
    }
    let raw_limit = get(args, "limit")
        .cloned()
        .unwrap_or(Value::Int(120.into()));
    let limit = match &raw_limit {
        Value::Int(n) => Some(n.clone()),
        Value::Bool(b) => Some(i64::from(*b).into()),
        Value::Str(t) => t.as_str().and_then(|s| kassiber_pytext::parse_int(s).ok()),
        Value::Float(n) if n.is_finite() => {
            kassiber_pytext::parse_int(&format!("{:.0}", n.trunc())).ok()
        }
        Value::Float(n) if n.is_infinite() => {
            return Err(AppError::new(
                "internal_error",
                "cannot convert float infinity to integer",
            ))
        }
        _ => None,
    }
    .ok_or_else(|| {
        AppError::new(
            "validation",
            "ui.reports.balance_history limit must be an integer",
        )
        .with_details(dict([("value", raw_limit.clone())]))
    })?;
    if limit < 1.into() {
        return Err(AppError::new(
            "validation",
            "ui.reports.balance_history limit must be positive",
        )
        .with_details(dict([("value", raw_limit)])));
    }
    let limit = limit.min(500.into());
    let mut normalized = dict(allowed.map(|k| (k, get(args, k).cloned().unwrap_or(Value::Null))));
    if let Value::Dict(ref mut fields) = normalized {
        for (key, v) in fields {
            if key == &"limit".into() {
                *v = Value::Int(limit.clone());
            }
            if key == &"interval".into() {
                *v = interval.clone();
            }
        }
    }
    Ok(normalized)
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
    fn history_daemon_arguments() {
        for (raw, expected) in [
            ("true", 1),
            ("1", 1),
            ("2.9", 2),
            ("500", 500),
            ("501", 500),
            ("999999999999999999999999999999", 500),
            ("\" ５０１ \"", 500),
        ] {
            let input = dict([("limit", kassiber_pytext::loads(raw).unwrap())]);
            let parsed = history_args(Some(&input)).unwrap();
            assert_eq!(get(&parsed, "limit"), Some(&Value::Int(expected.into())));
            assert_eq!(get(&parsed, "interval"), Some(&s("month")));
        }
        for raw in ["0", "false", "-1", "null", "[]", "{}", "\"bad\"", "NaN"] {
            let input = dict([("limit", kassiber_pytext::loads(raw).unwrap())]);
            assert_eq!(history_args(Some(&input)).unwrap_err().code, "validation");
        }
        for raw in ["null", "[]", "false", "\"bad\""] {
            let input = dict([("interval", kassiber_pytext::loads(raw).unwrap())]);
            assert_eq!(
                history_args(Some(&input)).unwrap_err().message,
                "ui.reports.balance_history interval is unsupported"
            );
        }
        let unknown = dict([
            ("z", Value::Null),
            ("a", Value::Null),
            ("limit", Value::Null),
        ]);
        assert_eq!(
            get(
                history_args(Some(&unknown))
                    .unwrap_err()
                    .details
                    .as_ref()
                    .unwrap(),
                "unknown"
            ),
            Some(&Value::List(vec![s("a"), s("z")]))
        );
        assert!(history_args(Some(&Value::List(vec![]))).is_err());
        let defaults = history_args(None).unwrap();
        assert_eq!(get(&defaults, "limit"), Some(&Value::Int(120.into())));
        assert_eq!(get(&defaults, "interval"), Some(&s("month")));
        let unicode = kassiber_pytext::loads(r#"{"\ue000":null,"z":null,"\ud800":null}"#).unwrap();
        let expected = kassiber_pytext::loads(r#"["z","\ud800","\ue000"]"#).unwrap();
        assert_eq!(
            get(
                history_args(Some(&unicode))
                    .unwrap_err()
                    .details
                    .as_ref()
                    .unwrap(),
                "unknown"
            ),
            Some(&expected)
        );
    }
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
