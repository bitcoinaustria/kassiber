//! Family registrations. Keep orchestration here and domain rules in services.
use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    if crate::backends::COMMANDS.contains(&command) {
        return crate::backends::execute(book, command, args);
    }
    if crate::reports::COMMANDS.contains(&command) {
        return crate::reports::execute(book, command, args);
    }
    if crate::networks::COMMANDS.contains(&command) {
        return crate::networks::execute(book, command, args);
    }
    if crate::workspaces::COMMANDS.contains(&command) {
        return crate::workspaces::execute(book, command, args);
    }
    if crate::profiles::COMMANDS.contains(&command) {
        return crate::profiles::execute(book, command, args);
    }
    if crate::accounts::COMMANDS.contains(&command) {
        return crate::accounts::execute(book, command, args);
    }
    if crate::context::COMMANDS.contains(&command) {
        return crate::context::execute(book, command, args);
    }
    if crate::connections::COMMANDS.contains(&command) {
        return crate::connections::execute(book, command, args);
    }
    if crate::views::COMMANDS.contains(&command) {
        return crate::views::execute(book, command, args);
    }
    Err(kassiber_runtime::store::not_implemented("cli", command))
}
