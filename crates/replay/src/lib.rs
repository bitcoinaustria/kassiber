//! Journal replay bundles (docs/reference/replay-bundle.md): one journal
//! run's pseudonymized inputs and canonical outputs in the backups' age
//! passphrase envelope, and the verify that re-runs them without a book.
//!
//! Nothing here performs network I/O. Export writes only the sealed bytes the
//! caller asks for; no plaintext copy reaches disk.
pub mod pseudonym;
pub mod registry;
pub mod typed;
