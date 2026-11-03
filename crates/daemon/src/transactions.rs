//! Desktop transaction transports; selection and presentation live in the domain service.
use kassiber_envelope::{dict, get, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub use kassiber_transactions::desktop::{AUDIT_KINDS, KINDS};

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    if kind == "ui.transactions.dashboard" {
        return dashboard(book, args);
    }
    let projection = kassiber_transactions::desktop::Projection::from_kind(kind)
        .ok_or_else(|| AppError::new("unsupported_kind", "Unsupported desktop transaction kind"))?;
    book.transaction_desktop(projection, args)
}

/// Python computes the whole-book suggestion list before validating dashboard
/// filters; candidates only shape the presentation and author nothing.
fn dashboard(book: &mut dyn Book, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    let candidates = if book.context()?.profile_id.is_empty() {
        Vec::new()
    } else {
        let payload = crate::transfers::pairing_args("ui.transfers.suggest", &empty)?;
        match get(
            &book.transfers("transfers.suggest", &payload)?,
            "candidates",
        ) {
            Some(Value::List(candidates)) => candidates.clone(),
            _ => Vec::new(),
        }
    };
    book.transaction_dashboard(args, &candidates)
}
