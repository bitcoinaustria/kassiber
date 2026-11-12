//! Connections leaves (`btcpay ...`, `wallets attach-*`): adapters over the
//! shared book services; saved credentials never reach this layer.
use crate::argparse::Namespace;
use kassiber_envelope::{dict, AppError};
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const COMMANDS: &[&str] = &[
    "btcpay.key-url",
    "btcpay.provenance.sync",
    "btcpay.inspect",
    "btcpay.setup",
];

pub fn arguments(args: &Namespace) -> Value {
    dict(args.items().iter().map(|(k, v)| (k.as_str(), v.clone())))
}

pub fn execute(book: &mut dyn Book, command: &str, args: &Namespace) -> Result<Value, AppError> {
    book.service(command, &arguments(args))
}
