//! Kassiber's tax engine.
//!
//! The engine is a pure function from a versioned request to a versioned
//! response: no I/O, network, logging, clock, or global state. See
//! `docs/plan/20-kassiber-tax-engine.md` and `docs/reference/tax-engine.md`.
//!
//! Layout: [`model`] is the JSON boundary, [`entry`] reproduces RP2's
//! transaction constructors, and [`api`] runs requests. The per-asset
//! computation lives in `engine` (orchestration), `cursor` (the
//! taxable-event pass), `methods` (lot selection), `heap` (CPython's
//! `heapq`), and `outputs` (`ComputedData`'s derived views).

/// Version of this engine build, recorded in journal provenance.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod api;
mod cursor;
pub mod decimal;
mod engine;
pub mod entry;
pub mod error;
mod heap;
mod methods;
pub mod model;
mod num;
mod outputs;
pub mod time;

pub use api::{check_entry, compute};
pub use engine::Country;
