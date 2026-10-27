//! The engine's decimal type, from the shared `kassiber-pydecimal` crate.
//!
//! The type moved to `crates/pydecimal` so every backend crate computes with
//! the same CPython-exact arithmetic (docs/plan/21-rust-backend.md); this
//! module keeps the engine's `crate::decimal` paths.

pub use kassiber_pydecimal::*;
