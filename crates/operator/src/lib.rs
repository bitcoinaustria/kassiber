//! The operator broker's protocol layer and client (plan 21, R6-1;
//! docs/reference/r6-ai-operator-port.md).
//!
//! - [`protocol`]: frames, limits and the secret challenge;
//! - [`runtime`]: the runtime directory, the endpoint, the startup
//!   election, stale-socket recovery, and the listener and client ends;
//! - [`build`]: the build identity brokers and clients compare;
//! - [`client`]: the client wire for all 14 actions;
//! - [`arguments`]: secret-bearing command arguments and durations.
//!
//! The broker service, server and children are R6-2. Platform calls live
//! in `kassiber-ipc`; this crate forbids `unsafe`.

pub mod arguments;
pub mod build;
pub mod client;
pub mod error;
pub mod protocol;
pub mod runtime;
pub mod values;

pub use error::{Error, Result};
