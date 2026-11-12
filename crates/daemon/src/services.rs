use kassiber_envelope::{dict, type_name, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
pub fn kinds() -> impl Iterator<Item = &'static str> {
    crate::offline::KINDS
        .iter()
        .copied()
        .chain(crate::backends::KINDS.iter().copied())
        .chain(crate::views::KINDS.iter().copied())
        .chain(crate::networks::KINDS.iter().copied())
        .chain(crate::profiles::KINDS.iter().copied())
        .chain(crate::workspaces::KINDS.iter().copied())
        .chain(crate::accounts::KINDS.iter().copied())
        .chain(crate::context::KINDS.iter().copied())
        .chain(crate::connections::KINDS.iter().copied())
}
pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    let empty = dict::<&str>([]);
    // These reads ignore their arguments, as Python's handlers do.
    if kind == "ui.profiles.snapshot" || kind == "ui.backends.list" {
        return book.service(kind, &empty);
    }
    if kind == "ui.connections.sources" {
        return crate::connections::execute(book, kind, &empty);
    }
    let args = match args {
        None | Some(Value::Null) => &empty,
        Some(v @ Value::Dict(_)) => v,
        Some(v) => {
            return Err(AppError::new("validation", "daemon args must be an object")
                .with_details(dict([("type", kassiber_envelope::s(type_name(v)))])))
        }
    };
    if crate::backends::KINDS.contains(&kind) {
        return crate::backends::execute(book, kind, args);
    }
    if crate::connections::KINDS.contains(&kind) {
        return crate::connections::execute(book, kind, args);
    }
    if crate::workspaces::KINDS.contains(&kind) {
        return book.service(kind, args);
    }
    if crate::profiles::KINDS.contains(&kind) {
        return crate::profiles::execute(book, kind, args);
    }
    if crate::networks::KINDS.contains(&kind) {
        return crate::networks::execute(book, kind, args);
    }
    if crate::views::KINDS.contains(&kind) {
        return crate::views::execute(book, kind, args);
    }
    Err(kassiber_runtime::store::not_implemented("daemon", kind))
}
