use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub fn execute(book: &mut dyn Book, operation: &str, args: &Namespace) -> Result<Value, AppError> {
    let mut payload = dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())));
    if operation == "transfers.rules.create" {
        let predicate = args.text("predicate").unwrap_or("{}");
        let predicate = kassiber_pytext::loads(predicate).map_err(|e| {
            AppError::new(
                if e.is_decode_error() {
                    "validation"
                } else {
                    "internal_error"
                },
                format!("Invalid --predicate JSON: {e}"),
            )
        })?;
        kassiber_envelope::set(&mut payload, "predicate", predicate);
        kassiber_envelope::set(&mut payload, "enabled", Value::Bool(!args.flag("disabled")));
    }
    book.transfers(operation, &payload)
}
