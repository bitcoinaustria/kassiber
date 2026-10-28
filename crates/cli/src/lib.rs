//! Kassiber's command line in Rust (plan 21, R1).
//!
//! [`run`] is `kassiber.cli.main.main`: argparse's grammar for all 313
//! leaves (generated from the Python tree into `spec_data.rs`), the global
//! flags, the usage-error envelope, `commands describe`, and the leaves
//! this stage implements (`status`, `daemon`, `commands describe`). Every
//! other leaf parses exactly as in Python and then fails
//! `not_implemented`.

pub mod argparse;
pub mod describe;
pub mod help;
pub mod render;
mod run;
pub mod spec;
#[rustfmt::skip]
mod spec_data;

#[cfg(feature = "test-probe")]
pub use run::PROBE_NOT_IMPLEMENTED;
pub use run::{run, Environment, Streams};
