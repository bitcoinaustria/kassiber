//! Local evidence services. Raw observations are never reclassified here.
pub mod attachments;
pub mod bank_statements;
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

/// The fiat currencies importers accept, by ISO 4217 code (the list the
/// exchange and tax-platform importers shared). Any other code, a crypto
/// asset or stablecoin included, is not fiat.
pub const FIAT_CODES: &[&str] = &[
    "AED", "ARS", "AUD", "BRL", "CAD", "CHF", "CLP", "CNY", "CZK", "DKK", "EUR", "GBP", "HKD",
    "HUF", "ILS", "INR", "JPY", "KRW", "MXN", "NOK", "NZD", "PLN", "RON", "SEK", "SGD", "THB",
    "TRY", "USD", "ZAR",
];
