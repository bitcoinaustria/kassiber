//! Local evidence services. Raw observations are never reclassified here.
pub mod attachments;
pub mod btcpay_records;
pub mod commercial;
pub mod desktop;
pub mod documents;
pub mod listing;
pub mod loans;
pub mod lookup;
pub mod metadata;
mod mime;
mod sql;

pub use sql::Scope;
