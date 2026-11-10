//! Network configuration; services do no transport I/O.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub const COMMANDS: &[&str] = &[
    "networks.inventory",
    "networks.plan",
    "networks.bind",
    "networks.split-plan",
    "networks.split",
];
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.service(
        command,
        &dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}
