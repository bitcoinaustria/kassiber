//! The backend's secret floor, compiled from the same source as the `kassiber-envelope` crate
//! (`crates/envelope/src/redaction/`). Those files use only the standard library, so the desktop
//! shares the exact credential scanner and key/descriptor/token rules without depending on the
//! backend workspace. `reports::diag::redact` runs this first, then its own extra patterns.
//! The envelope crate is edition 2021 and formatted there; this crate's edition 2024 lints and
//! rustfmt must not rewrite it.

#[path = "../../envelope/src/redaction/scanner.rs"]
#[allow(dead_code, clippy::collapsible_if)]
#[rustfmt::skip]
pub mod scanner;

#[path = "../../envelope/src/redaction/floor.rs"]
#[allow(dead_code, clippy::collapsible_if)]
#[rustfmt::skip]
pub mod floor;
