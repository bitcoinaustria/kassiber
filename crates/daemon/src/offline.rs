//! Machine preference operations remain available while the book is locked.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::BookStore;
pub const KINDS: &[&str] = &["ui.network.offline", "ui.network.offline.set"];
pub fn execute(store: &dyn BookStore, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(v @ Value::Dict(_)) => v,
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(v)))])))
        }
    };
    store.local_service(kind, args)
}
