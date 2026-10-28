//! Desktop report transports; all computation lives in kassiber-reports.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const KINDS: &[&str] = &[
    "ui.reports.summary",
    "ui.reports.balance_sheet",
    "ui.reports.portfolio_summary",
];
pub fn execute(book: &mut dyn Book, kind: &str, raw: Option<&Value>) -> Result<Value, AppError> {
    if kind != "ui.reports.summary" {
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
        .filter(|(k, _)| *k != "wallet".into())
        .filter_map(|(k, _)| match k {
            kassiber_pyjson::Key::Str(k) => Some(k.clone()),
            _ => None, // JSON object keys are strings.
        })
        .collect();
    unknown.sort();
    if !unknown.is_empty() {
        return Err(AppError::new(
            "validation",
            "ui.reports.summary received unsupported arguments",
        )
        .with_details(dict([(
            "unknown",
            Value::List(unknown.into_iter().map(Value::Str).collect()),
        )])));
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
