//! The rates family adapts parsed arguments to the shared service boundary.
use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::Book;
pub(crate) fn run(book: &mut dyn Book, namespace: &Namespace) -> Result<Value, AppError> {
    let args = Value::Dict(
        namespace
            .items()
            .iter()
            .map(|(k, v)| (Key::from(k.as_str()), v.clone()))
            .collect(),
    );
    book.rates(namespace.text("rates_command").unwrap_or(""), &args)
}
