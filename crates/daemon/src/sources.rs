//! Source coverage kinds (Rust only; docs/reference/daemon.md#source-coverage).
//! Both run in the book service: the read is local, the attestation writes
//! one wallet config key.
pub const KINDS: &[&str] = &["ui.sources.coverage", "ui.sources.attest_coverage"];
