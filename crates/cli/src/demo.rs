//! `kassiber projects demo`: a disposable demo book outside the project catalog
//! (docs/reference/demo.md). The store writes the copy; `run` then opens it
//! like any book (migrations) and publishes its local analysis index.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::BookStore;

pub const COMMANDS: &[&str] = &["projects.demo"];

/// Writes the copy and returns its description (`data_root` among it).
pub fn create(store: &dyn BookStore, args: &Namespace) -> Result<Value, AppError> {
    store.local_service(
        "demo.create",
        &dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}

/// The written copy's data root.
pub fn data_root(payload: &Value) -> Result<String, AppError> {
    match kassiber_envelope::get(payload, "data_root") {
        Some(Value::Str(text)) => text
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| AppError::new("internal_error", "demo data root is not text")),
        _ => Err(AppError::new("internal_error", "demo data root missing")),
    }
}
