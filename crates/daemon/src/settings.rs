//! Freshness and maintenance settings: local policy reads and writes with no
//! job, source run or connection.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub use kassiber_wallets::freshness::settings::KINDS;

/// The read kinds ignore their arguments, as Python's handlers do; the
/// configure kinds coerce them like `_coerce_args_dict`.
pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = if kind.ends_with(".configure") {
        match args {
            None | Some(Value::Null) => &empty,
            Some(v @ Value::Dict(_)) => v,
            Some(v) => {
                return Err(AppError::new("validation", "daemon args must be an object")
                    .with_details(dict([("type", s(type_name(v)))])))
            }
        }
    } else {
        &empty
    };
    book.service(kind, args)
}
