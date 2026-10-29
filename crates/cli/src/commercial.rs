use crate::argparse::Namespace;
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::commercial::Command;
pub fn execute(book: &mut dyn Book, path: &str, ns: &Namespace) -> Result<Value, AppError> {
    let limit = ns
        .get("limit")
        .and_then(|v| match v {
            Value::Int(n) => {
                Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() { 1000 } else { -1 }))
            }
            _ => None,
        })
        .unwrap_or(100);
    let command = match path {
        "btcpay.provenance.suggest" => Command::Suggest { limit },
        "btcpay.provenance.links" => Command::Links {
            state: ns.get("state").cloned().unwrap_or(Value::Null),
            limit,
        },
        "btcpay.provenance.list" => Command::Records {
            record_type: ns.get("record_type").cloned().unwrap_or(Value::Null),
            limit,
        },
        _ => unreachable!("registered stored commercial command"),
    };
    book.commercial(ns.get("workspace"), ns.get("profile"), &command)
}
