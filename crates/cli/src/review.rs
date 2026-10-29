//! Portable review family; only stored receipts are implemented.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub fn execute(book: &mut dyn Book, operation: &str, args: &Namespace) -> Result<Value, AppError> {
    book.transfers(
        operation,
        &dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}
