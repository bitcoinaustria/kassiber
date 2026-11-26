//! Balance checkpoint kinds (docs/reference/balance-checkpoints.md), Rust
//! only. The book service owns the model; every kind is local and opens no
//! connection.
pub const KINDS: &[&str] = &[
    "ui.wallets.checkpoints.list",
    "ui.wallets.checkpoints.add",
    "ui.wallets.checkpoints.remove",
    "ui.wallets.checkpoints.import",
    "ui.wallets.checkpoints.export",
];
