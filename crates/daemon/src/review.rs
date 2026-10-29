//! Historical receipt reads do not recompute journals or certify current readiness.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &["ui.review.receipt"];
pub fn execute(book: &mut dyn Book, _kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = args.filter(|v| !matches!(v, Value::Null)).unwrap_or(&empty);
    let Value::Dict(fields) = args else {
        return Err(AppError::new("validation", "daemon args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    };
    if fields
        .iter()
        .any(|(k, _)| *k != "receipt_id".into() && *k != "idempotency_key".into())
    {
        return Err(AppError::new("validation", "Unsupported review arguments"));
    }
    book.transfers("review.receipt", args)
}
