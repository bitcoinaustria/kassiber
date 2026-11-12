use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::documents::Command;
pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let args = dict(
        [
            "document_type",
            "label",
            "external_ref",
            "issuer",
            "counterparty",
            "issued_at",
            "due_at",
            "fiat_currency",
            "fiat_value",
            "notes",
            "document",
            "file",
            "url",
            "media_type",
            "provider",
            "tax_year",
        ]
        .map(|k| (k, ns.get(k).cloned().unwrap_or(Value::Null))),
    );
    let command = match path {
        "documents.list" => Command::List {
            limit: ns
                .get("limit")
                .and_then(|v| match v {
                    Value::Int(n) => Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() {
                        1000
                    } else {
                        -1
                    })),
                    _ => None,
                })
                .unwrap_or(100),
        },
        "documents.create" => Command::Create(args),
        "documents.attach" => Command::Attach(args),
        "documents.import-report" => Command::ImportReport(args),
        _ => unreachable!("registered documents command"),
    };
    book.documents(ns.get("workspace"), ns.get("profile"), &command)
}
