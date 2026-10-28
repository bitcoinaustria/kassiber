//! Desktop transaction transports; selection and presentation live in the domain service.
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub use kassiber_transactions::desktop::KINDS;

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let projection = kassiber_transactions::desktop::Projection::from_kind(kind)
        .ok_or_else(|| AppError::new("unsupported_kind", "Unsupported desktop transaction kind"))?;
    book.transaction_desktop(projection, args)
}
