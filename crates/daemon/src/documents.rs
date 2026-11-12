use kassiber_envelope::{dict, get, s, truthy, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_pytext::parse_int;
use kassiber_runtime::store::Book;
use kassiber_transactions::{commercial, documents};
pub const KINDS: &[&str] = &[
    "ui.btcpay.provenance.list",
    "ui.btcpay.provenance.suggest",
    "ui.btcpay.provenance.links",
    "ui.btcpay.provenance.review",
    "ui.transactions.commercial_context",
    "ui.documents.list",
    "ui.documents.create",
    "ui.documents.attach",
    "ui.documents.import_report",
];
fn required(args: &Value, kind: &str, key: &str) -> Result<String, AppError> {
    required_value(args, kind, key)?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| AppError::new("internal_error", "text cannot be encoded as UTF-8"))
}
fn required_value<'a>(
    args: &'a Value,
    kind: &str,
    key: &str,
) -> Result<&'a kassiber_pyjson::PyStr, AppError> {
    get(args, key)
        .and_then(|v| match v {
            Value::Str(t) if truthy(v) => Some(t),
            _ => None,
        })
        .ok_or_else(|| AppError::new("validation", format!("{kind} requires args.{key}")))
}
fn limit(args: &Value, default: i64) -> Result<i64, AppError> {
    let Some(v) = get(args, "limit").filter(|v| truthy(v)) else {
        return Ok(default);
    };
    let n = match v {
        Value::Int(n) => Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() {
            i64::MAX
        } else {
            i64::MIN
        })),
        Value::Bool(b) => Some(i64::from(*b)),
        Value::Float(n) if n.is_finite() => Some(*n as i64),
        Value::Str(t) => t.as_str().and_then(|s| parse_int(s).ok()).map(|n| {
            i64::try_from(n.clone()).unwrap_or(if n > 0.into() { i64::MAX } else { i64::MIN })
        }),
        _ => None,
    };
    n.ok_or_else(|| AppError::new("internal_error", "limit must be an integer"))
}
pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    let doc = match kind {
        "ui.documents.list" => Some(documents::Command::List {
            limit: limit(args, 100)?,
        }),
        "ui.documents.create" => {
            required_value(args, kind, "label")?;
            let doc_type = get(args, "document_type")
                .filter(|v| truthy(v))
                .or_else(|| get(args, "type"))
                .cloned()
                .unwrap_or(Value::Null);
            required_value(
                &dict([("document_type", doc_type.clone())]),
                kind,
                "document_type",
            )?;
            let Value::Dict(mut pairs) = args.clone() else {
                unreachable!()
            };
            pairs.retain(|(k, _)| *k != kassiber_pyjson::Key::from("document_type"));
            pairs.push(("document_type".into(), doc_type));
            Some(documents::Command::Create(Value::Dict(pairs)))
        }
        "ui.documents.attach" => {
            required(args, kind, "document")?;
            Some(documents::Command::Attach(args.clone()))
        }
        "ui.documents.import_report" => Some(documents::Command::ImportReport(args.clone())),
        _ => None,
    };
    if let Some(command) = doc {
        let value = book.documents(None, None, &command)?;
        return Ok(if kind == "ui.documents.list" {
            dict([("documents", value)])
        } else {
            value
        });
    }
    let command = match kind {
        "ui.transactions.commercial_context" => commercial::Command::Context {
            transaction: required(args, kind, "transaction")?,
        },
        "ui.btcpay.provenance.suggest" => commercial::Command::Suggest {
            limit: limit(args, 500)?,
        },
        "ui.btcpay.provenance.list" => commercial::Command::Records {
            record_type: get(args, "record_type").cloned().unwrap_or(Value::Null),
            limit: limit(args, 100)?,
        },
        "ui.btcpay.provenance.links" => commercial::Command::Links {
            state: get(args, "state").cloned().unwrap_or(Value::Null),
            limit: limit(args, 100)?,
        },
        "ui.btcpay.provenance.review" => {
            let link = required(args, kind, "link")?;
            required_value(args, kind, "state")?;
            let field = |key: &str| get(args, key).cloned().unwrap_or(Value::Null);
            commercial::Command::Review(commercial::Review {
                link,
                state: field("state"),
                reconciliation_state: field("reconciliation_state"),
                commercial_kind: field("commercial_kind"),
                notes: field("notes"),
            })
        }
        _ => unreachable!("registered commercial kind"),
    };
    let result = book.commercial(None, None, &command)?;
    Ok(match kind {
        "ui.btcpay.provenance.list" => dict([("records", result)]),
        "ui.btcpay.provenance.links" => dict([("links", result)]),
        _ => result,
    })
}
