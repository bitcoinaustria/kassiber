use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;
use kassiber_transactions::listing::Options;

use crate::argparse::Namespace;

pub fn execute(book: &mut dyn Book, ns: &Namespace) -> Result<Value, AppError> {
    let optional = |key| ns.text(key).map(str::to_owned);
    let limit = match ns.get("limit") {
        Some(Value::Int(n)) => Some(i64::try_from(n.clone()).unwrap_or(if *n > 0.into() {
            i64::MAX
        } else {
            i64::MIN
        })),
        _ => None,
    };
    let txids = match ns.get("txids") {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|v| match v {
                Value::Str(t) => t.as_str().map(str::to_owned),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    book.transactions(
        ns.get("workspace"),
        ns.get("profile"),
        &Options {
            wallet: optional("wallet"),
            limit,
            direction: optional("direction"),
            asset: optional("asset"),
            start: optional("start"),
            end: optional("end"),
            txids,
            period: optional("period"),
            status: optional("status"),
            flow: optional("flow"),
            payment_method: optional("payment_method"),
            network: optional("network"),
            with_fees: ns.flag("with_fees"),
            quick: optional("quick"),
            cursor: optional("cursor"),
            sort: optional("sort"),
            order: optional("order"),
        },
    )
}
