use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.transfers.list",
    "ui.transfers.payouts.list",
    "ui.transfers.payouts.delete",
    "ui.transfers.pair",
    "ui.transfers.unpair",
    "ui.transfers.update",
    "ui.transfers.dismiss",
    "ui.custody.gaps.history",
    "ui.transfers.rules.list",
    "ui.transfers.rules.create",
    "ui.transfers.rules.delete",
    "ui.transfers.rules.set_enabled",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let mut operation = if kind == "ui.custody.gaps.history" {
        let allowed = ["workspace", "profile", "gap_id", "limit"];
        let mut unknown: Vec<_> = fields
            .iter()
            .filter(|(k, _)| !allowed.contains(&key_text(k).as_str()))
            .map(|(k, _)| s(key_text(k)))
            .collect();
        unknown.sort_by_key(kassiber_envelope::py_str);
        if !unknown.is_empty() {
            return Err(
                AppError::new("validation", format!("{kind} received unsupported fields"))
                    .with_details(dict([("unknown", Value::List(unknown))])),
            );
        }
        let limit = get(args, "limit")
            .cloned()
            .unwrap_or(Value::Int(100.into()));
        if !matches!(&limit,Value::Int(n) if n>=&1.into() && n<=&200.into()) {
            return Err(AppError::new(
                "validation",
                format!("{kind} limit must be an integer between 1 and 200"),
            ));
        }
        if !matches!(
            get(args, "cursor"),
            None | Some(Value::Null) | Some(Value::Str(_))
        ) {
            return Err(AppError::new(
                "validation",
                format!("{kind} cursor is invalid"),
            ));
        }
        if !matches!(get(args,"gap_id"),Some(Value::Str(v)) if v.as_str().is_some_and(|s| !kassiber_pytext::strip(s).is_empty()))
        {
            return Err(AppError::new(
                "validation",
                format!("{kind} requires gap_id"),
            ));
        }
        "transfers.gaps.history"
    } else {
        kind.strip_prefix("ui.").expect("registered kind")
    };
    let mut args = args.clone();
    normalize_pair_args(kind, &mut args)?;
    normalize_payout_delete_args(kind, &mut args)?;
    if kind == "ui.custody.gaps.history" {
        if let Value::Dict(fields) = &mut args {
            for (k, v) in fields {
                if *k == "gap_id".into() {
                    *v = s(kassiber_pytext::strip(&kassiber_envelope::py_str(v)));
                }
            }
        }
    }
    if kind == "ui.transfers.rules.create" {
        let predicate = get(&args, "predicate")
            .filter(|v| !matches!(v, Value::Null))
            .cloned()
            .unwrap_or_else(|| dict::<&str>([]));
        if !matches!(predicate, Value::Dict(_)) {
            return Err(AppError::new(
                "validation",
                "ui.transfers.rules.create predicate must be an object",
            ));
        }
        kassiber_envelope::set(&mut args, "predicate", predicate);
        let kind = get(&args, "kind")
            .filter(|v| kassiber_envelope::truthy(v))
            .map(kassiber_envelope::py_str)
            .unwrap_or_else(|| "manual".into());
        kassiber_envelope::set(&mut args, "kind", s(kind));
        if let Some(policy) = get(&args, "policy")
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
        {
            kassiber_envelope::set(&mut args, "policy", s(policy));
        }
    }
    if matches!(
        kind,
        "ui.transfers.rules.delete" | "ui.transfers.rules.set_enabled"
    ) {
        let id = get(&args, "rule_id")
            .filter(|v| kassiber_envelope::truthy(v))
            .ok_or_else(|| AppError::new("validation", format!("{kind} requires rule_id")))?;
        let id = kassiber_envelope::py_str(id);
        kassiber_envelope::set(&mut args, "rule_id", s(id));
        if kind == "ui.transfers.rules.set_enabled" {
            operation = if get(&args, "enabled").is_none_or(kassiber_envelope::truthy) {
                "transfers.rules.enable"
            } else {
                "transfers.rules.disable"
            };
        }
    }
    if kind == "ui.transfers.dismiss" {
        for (name, alias) in [("tx_out", "out_id"), ("tx_in", "in_id")] {
            let id = get(&args, name)
                .filter(|v| kassiber_envelope::truthy(v))
                .or_else(|| get(&args, alias))
                .cloned()
                .unwrap_or(Value::Null);
            kassiber_envelope::set(&mut args, name, id);
        }
        // Preserve Python's desktop `value or 90` independently of the CLI's
        // explicit zero (never expire).
        let raw = get(&args, "expires_in_days")
            .filter(|v| kassiber_envelope::truthy(v))
            .cloned()
            .unwrap_or(90.into());
        kassiber_envelope::set(&mut args, "expires_in_days", Value::Int(expiry_days(&raw)?));
    }
    let result = book.transfers(operation, &args)?;
    let result = if matches!(kind, "ui.transfers.list" | "ui.custody.gaps.history") {
        exact_integers(result)
    } else {
        result
    };
    match kind {
        "ui.transfers.list" => Ok(dict([("pairs", result)])),
        "ui.transfers.payouts.list" => Ok(dict([("payouts", result)])),
        "ui.transfers.rules.list" => Ok(dict([("rules", result)])),
        _ => Ok(result),
    }
}

fn normalize_payout_delete_args(kind: &str, args: &mut Value) -> Result<(), AppError> {
    if kind == "ui.transfers.payouts.delete" {
        let id = get(args, "payout_id")
            .filter(|v| kassiber_envelope::truthy(v))
            .ok_or_else(|| {
                AppError::new(
                    "validation",
                    "ui.transfers.payouts.delete requires payout_id",
                )
            })?;
        let id = kassiber_envelope::py_str(id);
        kassiber_envelope::set(args, "payout_id", s(id));
        kassiber_envelope::set(args, "authored_source", s("gui"));
    }

    Ok(())
}

// The desktop calls int() before dispatching to the scoped domain service.
// Preserve error precedence even when a transaction reference is missing.
fn normalize_pair_args(kind: &str, args: &mut Value) -> Result<(), AppError> {
    if matches!(
        kind,
        "ui.transfers.pair" | "ui.transfers.unpair" | "ui.transfers.update"
    ) {
        kassiber_envelope::set(args, "authored_source", s("gui"));
        if kind == "ui.transfers.pair" {
            for (name, alias) in [("tx_out", "out_id"), ("tx_in", "in_id")] {
                let id = get(args, name)
                    .filter(|v| kassiber_envelope::truthy(v))
                    .or_else(|| get(args, alias))
                    .cloned()
                    .unwrap_or(Value::Null);
                kassiber_envelope::set(args, name, id);
            }
            for (name, default) in [("kind", "manual"), ("pair_source", "manual")] {
                let v = get(args, name)
                    .filter(|v| kassiber_envelope::truthy(v))
                    .map(kassiber_envelope::py_str)
                    .unwrap_or_else(|| default.into());
                kassiber_envelope::set(args, name, s(v));
            }
        } else {
            let id = get(args, "pair_id")
                .filter(|v| kassiber_envelope::truthy(v))
                .ok_or_else(|| AppError::new("validation", format!("{kind} requires pair_id")))?;
            let id = kassiber_envelope::py_str(id);
            kassiber_envelope::set(args, "pair_id", s(id));
            if let Some(v) = get(args, "kind")
                .filter(|v| !matches!(v, Value::Null))
                .map(kassiber_envelope::py_str)
            {
                kassiber_envelope::set(args, "kind", s(v));
            }
        }
        if let Some(v) = get(args, "policy")
            .filter(|v| !matches!(v, Value::Null))
            .map(kassiber_envelope::py_str)
        {
            kassiber_envelope::set(args, "policy", s(v));
        }
        if kind == "ui.transfers.pair"
            || get(args, "notes").is_some()
            || get(args, "note").is_some()
        {
            let notes = get(args, "notes")
                .filter(|v| kassiber_envelope::truthy(v))
                .or_else(|| get(args, "note"))
                .cloned()
                .unwrap_or(Value::Null);
            kassiber_envelope::set(args, "notes", notes);
        }
    }
    Ok(())
}

fn expiry_days(value: &Value) -> Result<kassiber_pyjson::BigInt, AppError> {
    match value {
        Value::Int(n) => Ok(n.clone()),
        Value::Bool(n) => Ok(u8::from(*n).into()),
        Value::Float(n) if n.is_finite() => Ok(format!("{:.0}", n.trunc())
            .parse()
            .expect("finite float has an integer truncation")),
        Value::Float(n) => Err(AppError::new(
            "internal_error",
            if n.is_nan() {
                "cannot convert float NaN to integer"
            } else {
                "cannot convert float infinity to integer"
            },
        )),
        Value::Str(t) => kassiber_pytext::parse_int(t.as_str().unwrap_or_default()).map_err(|_| {
            AppError::new(
                "internal_error",
                format!(
                    "invalid literal for int() with base 10: {}",
                    kassiber_envelope::py_repr(value)
                ),
            )
        }),
        _ => Err(AppError::new(
            "internal_error",
            format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                type_name(value)
            ),
        )),
    }
}

fn key_text(key: &kassiber_pyjson::Key) -> String {
    use kassiber_pyjson::Key;
    kassiber_envelope::py_str(&match key {
        Key::Str(v) => Value::Str(v.clone()),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Bool(v) => Value::Bool(*v),
        Key::Float(v) => Value::Float(*v),
        Key::Null => Value::Null,
    })
}

fn exact_integers(value: Value) -> Value {
    match value {
        Value::Int(ref n)
            if n < &(-9007199254740991_i64).into() || n > &9007199254740991_i64.into() =>
        {
            s(n.to_string())
        }
        Value::List(items) => Value::List(items.into_iter().map(exact_integers).collect()),
        Value::Dict(fields) => Value::Dict(
            fields
                .into_iter()
                .map(|(k, v)| (k, exact_integers(v)))
                .collect(),
        ),
        v => v,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_payout_delete_requires_id_and_overrides_source() {
        let mut args = dict([
            ("payout_id", Value::Int(123.into())),
            ("authored_source", s("untrusted")),
        ]);
        normalize_payout_delete_args("ui.transfers.payouts.delete", &mut args).unwrap();
        assert_eq!(get(&args, "payout_id"), Some(&s("123")));
        assert_eq!(get(&args, "authored_source"), Some(&s("gui")));
        for id in [Value::Null, Value::Bool(false), s(""), Value::Int(0.into())] {
            let mut args = dict([("payout_id", id)]);
            let error =
                normalize_payout_delete_args("ui.transfers.payouts.delete", &mut args).unwrap_err();
            assert_eq!(error.code, "validation");
            assert_eq!(
                error.message,
                "ui.transfers.payouts.delete requires payout_id"
            );
        }
    }

    #[test]
    fn desktop_pair_aliases_defaults_and_absent_notes_match_python() {
        assert_eq!(
            &KINDS[..6],
            &[
                "ui.transfers.list",
                "ui.transfers.payouts.list",
                "ui.transfers.payouts.delete",
                "ui.transfers.pair",
                "ui.transfers.unpair",
                "ui.transfers.update"
            ]
        );
        let mut args = dict([
            ("tx_out", s("")),
            ("out_id", s("out")),
            ("tx_in", s("in")),
            ("in_id", s("ignored")),
            ("kind", Value::Bool(false)),
            ("pair_source", s("")),
            ("policy", 42.into()),
            ("notes", s("")),
            ("note", s("alias note")),
            ("authored_source", s("untrusted")),
        ]);
        normalize_pair_args("ui.transfers.pair", &mut args).unwrap();
        for (key, expected) in [
            ("tx_out", "out"),
            ("tx_in", "in"),
            ("kind", "manual"),
            ("pair_source", "manual"),
            ("policy", "42"),
            ("notes", "alias note"),
            ("authored_source", "gui"),
        ] {
            assert_eq!(get(&args, key), Some(&s(expected)), "{key}");
        }
        let mut args = dict([("pair_id", 17.into()), ("kind", Value::Bool(false))]);
        normalize_pair_args("ui.transfers.update", &mut args).unwrap();
        assert_eq!(get(&args, "pair_id"), Some(&s("17")));
        assert_eq!(get(&args, "kind"), Some(&s("False")));
        assert!(get(&args, "notes").is_none());
        kassiber_envelope::set(&mut args, "notes", s(""));
        normalize_pair_args("ui.transfers.update", &mut args).unwrap();
        assert_eq!(get(&args, "notes"), Some(&Value::Null));
        for kind in ["ui.transfers.update", "ui.transfers.unpair"] {
            for id in [Value::Null, s(""), Value::Bool(false)] {
                let mut args = dict([("pair_id", id)]);
                let error = normalize_pair_args(kind, &mut args).unwrap_err();
                assert_eq!(error.code, "validation");
                assert_eq!(error.message, format!("{kind} requires pair_id"));
            }
        }
    }

    #[test]
    fn desktop_dismissal_expiry_uses_python_integer_coercion() {
        for (input, expected) in [
            (s(" １_２ "), 12),
            (Value::Float(-1.9), -1),
            (Value::Bool(true), 1),
        ] {
            assert_eq!(expiry_days(&input).unwrap(), expected.into());
        }
        for (input, message) in [
            (s("bad"), "invalid literal for int() with base 10: 'bad'"),
            (
                Value::Float(f64::NAN),
                "cannot convert float NaN to integer",
            ),
            (
                Value::Float(f64::INFINITY),
                "cannot convert float infinity to integer",
            ),
            (
                Value::List(vec![]),
                "int() argument must be a string, a bytes-like object or a real number, not 'list'",
            ),
        ] {
            let error = expiry_days(&input).unwrap_err();
            assert_eq!(error.code, "internal_error");
            assert_eq!(error.message, message);
        }
    }
}
