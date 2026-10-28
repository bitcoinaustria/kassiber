//! Machine preference commands retain layout bootstrap without opening a book.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::BookStore;
pub const COMMANDS: &[&str] = &["offline.status", "offline.on", "offline.off"];
pub fn execute(store: &dyn BookStore, command: &str, args: &Namespace) -> Result<Value, AppError> {
    store.local_service(
        command,
        &dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}
