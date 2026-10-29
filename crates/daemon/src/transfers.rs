use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.transfers.list",
    "ui.transfers.payouts.list",
    "ui.custody.gaps.history",
    "ui.transfers.rules.list",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    let operation = if kind == "ui.custody.gaps.history" {
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
    if kind == "ui.custody.gaps.history" {
        if let Value::Dict(fields) = &mut args {
            for (k, v) in fields {
                if *k == "gap_id".into() {
                    *v = s(kassiber_pytext::strip(&kassiber_envelope::py_str(v)));
                }
            }
        }
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
