//! Pure custody computation from raw facts; no storage, network or clock.
pub mod snapshot;
pub const ENGINE_VERSION: &str = env!("CARGO_PKG_VERSION");
pub const STAGES: [&str; 6] = ["components", "evidence", "interpretation", "gaps", "quantity", "finalized_projection"];
