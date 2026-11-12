//! Watch-only protocol clients. Construction is inert; every operation requires
//! current consent and a caller-supplied monotonic overall deadline. Wallet
//! scanning, persistence and ownership decisions belong to the next slice.
pub mod btcpay;
mod config;
mod core;
mod detection;
mod electrum;
mod http;
pub mod lightning;
pub mod probes;
mod time;

pub use config::Backend;
pub use core::CoreClient;
pub use electrum::{header_fingerprint as electrum_header_fingerprint, ElectrumClient};
pub use http::{credential_forms, encode_base64, utf8_error, withhold_credentials, HttpClient};
use kassiber_envelope::AppError;
pub use kassiber_net::Consent;
use serde_json::Value;
use std::time::{Duration, Instant};

// Pure timestamp helpers shared with compatibility-history cache policy.
pub use time::{epoch as format_unix_time, parse as parse_iso_time};

pub type Result<T> = std::result::Result<T, AppError>;
pub const RESPONSE_LIMIT: usize = 16 * 1024 * 1024;
pub const CORE_HISTORY_LIMIT: usize = 64 * 1024 * 1024;

fn app(message: impl Into<String>) -> AppError {
    AppError::new("app_error", message)
}
fn details(value: Value) -> kassiber_pyjson::Value {
    kassiber_pytext::loads(&value.to_string()).expect("serialized JSON")
}
fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|d| !d.is_zero())
        .ok_or_else(|| kassiber_net::Error::InvalidTimeout.app_error())
}
fn io_error(error: std::io::Error) -> AppError {
    kassiber_net::Error::from(error).app_error()
}
fn redacted(text: &str) -> String {
    use kassiber_envelope::redaction::{redact_operational_text, redact_secret_text};
    redact_operational_text(&redact_secret_text(text))
}
fn truthy(value: &Value) -> bool {
    kassiber_envelope::truthy(&details(value.clone()))
}

#[cfg(test)]
mod tests;
