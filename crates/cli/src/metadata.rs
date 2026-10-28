use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::metadata::Command;

use crate::argparse::Namespace;

pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let text = |key| ns.text(key).unwrap_or_default().to_owned();
    // Namespace::text intentionally treats an empty string as absent for
    // selectors. An authored empty note is a real value, distinct from null.
    let optional = |key| match ns.get(key) {
        Some(Value::Str(text)) => text.as_str().map(str::to_owned),
        _ => None,
    };
    let transaction = text("transaction");
    let reason = optional("reason");
    let command = match path {
        "metadata.tags.list" => Command::Tags,
        "metadata.tags.create" => Command::CreateTag {
            code: text("code"),
            label: text("label"),
        },
        "metadata.records.get" => Command::Get { transaction },
        "metadata.notes.set" | "metadata.records.note.set" => Command::Note {
            transaction,
            note: optional("note"),
            reason,
        },
        "metadata.notes.clear" | "metadata.records.note.clear" => Command::Note {
            transaction,
            note: None,
            reason,
        },
        "metadata.tags.add"
        | "metadata.records.tag.add"
        | "metadata.tags.remove"
        | "metadata.records.tag.remove" => Command::Tag {
            transaction,
            tag: text("tag"),
            remove: path.ends_with(".remove"),
            reason,
        },
        "metadata.exclude"
        | "metadata.records.excluded.set"
        | "metadata.include"
        | "metadata.records.excluded.clear" => Command::Excluded {
            transaction,
            excluded: path == "metadata.exclude" || path.ends_with(".set"),
            reason,
        },
        "metadata.records.kind.set" | "metadata.records.kind.clear" => Command::Kind {
            transaction,
            kind: optional("kind"),
            reason,
        },
        "metadata.records.history.stale" => Command::Stale,
        _ => unreachable!("registered metadata command"),
    };
    book.metadata(ns.get("workspace"), ns.get("profile"), &command)
}
