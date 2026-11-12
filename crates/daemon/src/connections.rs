//! Connections screen kinds. The catalog is local; every other kind runs in
//! the book service, which owns configuration, secrets and network consent.
use kassiber_envelope::AppError;
use kassiber_pyjson::Value;
use kassiber_runtime::store::Book;

pub const KINDS: &[&str] = &[
    "ui.connections.sources",
    "ui.connections.btcpay.key_guide",
    "ui.connections.btcpay.test",
    "ui.btcpay.provenance.sync",
    "ui.connections.btcpay.discover",
    "ui.connections.btcpay.create",
    "ui.connections.bullbitcoin_wallet.create",
    // Rust only (docs/reference/exchange-desktop.md); never AI tools.
    "ui.connections.exchange.status",
    "ui.connections.exchange.test",
    "ui.connections.exchange.create",
    "ui.connections.exchange.sync",
];

pub fn execute(book: &mut dyn Book, kind: &str, args: &Value) -> Result<Value, AppError> {
    match kind {
        // `_connections_sources_payload()` takes no arguments at all.
        "ui.connections.sources" => Ok(kassiber_wallets::connections::sources()),
        _ => book.service(kind, args),
    }
}
