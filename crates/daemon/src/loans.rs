//! Desktop and AI loan transport. Accounting decisions stay in the service.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_pytext::strip;
use kassiber_runtime::store::Book;
use kassiber_transactions::loans::Command;

pub const KINDS: &[&str] = &[
    "ui.loans.list",
    "ui.loans.link",
    "ui.loans.mark",
    "ui.loans.unmark",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    let optional = |name: &str| -> Result<Option<String>, AppError> {
        match get(args, name) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Str(text)) => {
                let value = strip(text.as_str().ok_or_else(|| {
                    AppError::new("validation", format!("{name} is not valid UTF-8"))
                })?);
                Ok((!value.is_empty()).then(|| value.to_owned()))
            }
            Some(value) => Err(
                AppError::new("validation", format!("{name} must be a string"))
                    .with_details(dict([("type", s(type_name(value)))])),
            ),
        }
    };
    let required = |name: &str, label: &str| -> Result<String, AppError> {
        optional(name)?.ok_or_else(|| {
            AppError::new("validation", format!("{label} is required."))
                .with_hint(format!("Enter {}.", label.to_lowercase()))
        })
    };
    let command = match kind {
        "ui.loans.list" => Command::List {
            include_roles: true,
        },
        "ui.loans.mark" => Command::Mark {
            txid: required("txid", "transaction id")?,
            mark_as: required(
                "as",
                "mark target (collateral|returned|principal-received|principal-repaid)",
            )?,
            note: get(args, "note").cloned().unwrap_or(Value::Null),
            loan_id: optional("loan_id")?,
        },
        "ui.loans.link" => Command::Link {
            txids: match get(args, "txids") {
                Some(Value::List(items)) if items.iter().all(|v| matches!(v, Value::Str(_))) => {
                    items.iter().map(kassiber_envelope::py_str).collect()
                }
                _ => {
                    return Err(AppError::new(
                        "validation",
                        "txids must be a list of transaction ids",
                    )
                    .with_details(dict([("field", s("txids"))])))
                }
            },
            loan_id: optional("loan_id")?,
        },
        "ui.loans.unmark" => Command::Unmark {
            txid: required("txid", "transaction id")?,
        },
        _ => unreachable!("registered loan kind"),
    };
    book.loans(None, None, &command)
}
