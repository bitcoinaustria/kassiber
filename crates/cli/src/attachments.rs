use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::attachments::Command;

use crate::argparse::Namespace;

pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let text = |name| ns.text(name).unwrap_or_default().to_owned();
    let transaction = ns.text("transaction").map(str::to_owned);
    let command = match path {
        "attachments.add" => Command::Add {
            transaction: text("transaction"),
            file: ns.text("file").map(str::to_owned),
            url: ns.text("url").map(str::to_owned),
            label: ns.text("label").map(str::to_owned),
            media_type: ns.text("media_type").map(str::to_owned),
        },
        "attachments.list" => Command::List {
            transaction,
            limit: None,
            offset: 0,
        },
        "attachments.rename" => Command::Rename {
            attachment: text("attachment_id"),
            label: text("label"),
        },
        "attachments.remove" => Command::Remove {
            attachment: text("attachment_id"),
        },
        "attachments.verify" => Command::Verify { transaction },
        "attachments.gc" => Command::Gc {
            dry_run: ns.flag("dry_run"),
        },
        _ => unreachable!("registered attachment command"),
    };
    book.attachments(ns.get("workspace"), ns.get("profile"), &command)
}
