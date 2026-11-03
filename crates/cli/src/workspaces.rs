//! `workspaces` CLI registration; business rules live in kassiber-setup.
use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const COMMANDS: &[&str] = &[
    "workspaces.list",
    "workspaces.create",
    "health",
    "next-actions",
];
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.service(
        command,
        &kassiber_envelope::dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone()))),
    )
}
