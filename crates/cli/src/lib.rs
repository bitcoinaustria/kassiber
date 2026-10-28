//! Kassiber's command line in Rust (plan 21, R1).
//!
//! [`run`] is `kassiber.cli.main.main`: argparse's grammar for all 313
//! leaves (generated from the Python tree into `spec_data.rs`), the global
//! flags, the usage-error envelope, `commands describe`, and the leaves
//! the binary implements (runtime and registered local setup services). Every
//! other leaf parses exactly as in Python and then fails
//! `not_implemented`.

pub mod argparse;
mod attachments;
pub mod describe;
pub mod help;
mod rates;

mod loans;
mod metadata;
pub mod render;
mod run;
pub mod spec;
#[rustfmt::skip]
mod spec_data;

#[cfg(feature = "test-probe")]
pub use run::PROBE_NOT_IMPLEMENTED;
pub use run::{run, Environment, Streams};

mod accounts;
mod context;
mod networks;
mod offline;
mod profiles;
mod projects;
mod services;
mod views;
mod workspaces;
