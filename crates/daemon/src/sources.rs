//! Source coverage kinds (Rust only; docs/reference/daemon.md#source-coverage).
//! All run in the book service: the reads are local, the attestation writes
//! one wallet config key. `ui.sources.groups` groups wallets by the BTCPay
//! store that pays into them (docs/reference/daemon.md#btcpay-store-groups).
pub const KINDS: &[&str] = &[
    "ui.sources.coverage",
    "ui.sources.attest_coverage",
    "ui.sources.groups",
];
