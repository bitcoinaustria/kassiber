//! Existing desktop stale-edit summary; other metadata kinds remain unregistered.
use kassiber_envelope::{dict, s, type_name, AppError};
use kassiber_pyjson::{Key, Value};
use kassiber_runtime::store::Book;
use kassiber_transactions::metadata::Command;

pub const KINDS: &[&str] = &["ui.activity.stale"];

pub fn execute(book: &mut dyn Book, args: Option<&Value>) -> Result<Value, AppError> {
    match args {
        None | Some(Value::Null) => {}
        Some(Value::Dict(fields)) if fields.is_empty() => {}
        Some(Value::Dict(fields)) => {
            let mut keys: Vec<_> = fields
                .iter()
                .filter_map(|(k, _)| match k {
                    Key::Str(s) => Some(Value::Str(s.clone())),
                    _ => None,
                })
                .collect();
            keys.sort_by_key(kassiber_envelope::py_str);
            return Err(
                AppError::new("validation", "ui.activity.stale does not accept arguments")
                    .with_details(dict([("unknown", Value::List(keys))])),
            );
        }
        Some(value) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", s(type_name(value)))])))
        }
    }
    book.metadata(None, None, &Command::Stale)
}
