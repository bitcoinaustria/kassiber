//! Kassiber's command line in Rust (plan 21, R1).
//!
//! [`run`] is `kassiber.cli.main.main`: argparse's grammar for all 313
//! leaves (generated from the Python tree into `spec_data.rs`), the global
//! flags, the usage-error envelope, `commands describe`, and the leaves
//! this stage implements (see `describe::IMPLEMENTED`). Service families
//! register in their own modules. Every other leaf parses exactly as in
//! Python and then fails `not_implemented`.

pub mod argparse;
mod attachments;
mod commercial;
pub mod describe;
mod diagnostics;
mod document_review;
mod documents;
pub mod help;
mod rates;

mod imports;
mod loans;
mod metadata;
pub mod render;
mod run;
pub mod spec;
mod transactions;
mod update;
mod wallets;
#[rustfmt::skip]
mod spec_data;

#[cfg(feature = "test-probe")]
pub use run::PROBE_NOT_IMPLEMENTED;
pub use run::{run, Environment, Streams};

mod accounts;
mod backends;
mod backup;
mod context;
mod networks;
mod offline;
mod profiles;
mod projects;
mod reports;
mod secrets;
mod services;
mod views;
mod workspaces;

mod transfers;

mod chain_analysis_ops;
mod chain_analysis_reads;
mod review;

#[cfg(feature = "test-probe")]
pub use reports::execute as fixture_reports;
