//! Kassiber's tax engine.
//!
//! The engine is a pure function from a versioned request to a versioned
//! response: no I/O, network, logging, clock, or global state. See
//! `docs/plan/20-kassiber-tax-engine.md`.

/// Version of this engine build, recorded in journal provenance.
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod decimal;
