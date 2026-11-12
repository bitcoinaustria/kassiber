use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::loans::Command;

use crate::argparse::Namespace;

pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let text = |name| ns.text(name).unwrap_or_default().to_owned();
    let loan_id = ns.text("loan_id").map(str::to_owned);
    let command = match path {
        "loans.mark" => Command::Mark {
            txid: text("txid"),
            mark_as: text("mark_as"),
            note: ns.get("note").cloned().unwrap_or(Value::Null),
            loan_id,
        },
        "loans.unmark" => Command::Unmark { txid: text("txid") },
        "loans.link" => Command::Link {
            txids: match ns.get("txids") {
                Some(Value::List(items)) => items.iter().map(kassiber_envelope::py_str).collect(),
                _ => Vec::new(),
            },
            loan_id,
        },
        "loans.list" => Command::List {
            include_roles: false,
        },
        _ => unreachable!("registered loan command"),
    };
    book.loans(ns.get("workspace"), ns.get("profile"), &command)
}
