//! Desktop journal reads. They use only the open book's connection.
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.journals.snapshot",
    "ui.journals.events.list",
    "ui.journals.transfers.list",
    "ui.journals.quarantine",
    "ui.journals.quarantine.resolve",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: Option<&Value>) -> Result<Value, AppError> {
    book.journal_view(kind, args)
}
