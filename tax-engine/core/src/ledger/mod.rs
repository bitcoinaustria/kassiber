//! The ledger layer (schema version 2): the engine boundary at the finalized
//! custody projection, plan 20 phase 4.
//!
//! The layer builds the lot engine's entries from projection rows, runs the
//! existing lot engine on them, and composes the journal ledger. It holds
//! the boundary documents, [`request`] and [`response`]; [`text`], which
//! renders an entry's `str(transaction)` once the constructor has derived
//! its values; `normalize`, the adapter's normalization of one asset's rows
//! and moves (generic books); `order`, `walk`, and `describe`, which put the
//! normalized events and moves in the walk's order, keep the adapter's
//! balance mirror and gates, and build the lot engine's entries with the
//! notes the adapter composed; `compose`, which turns the lot engine's
//! results into journal entries, holdings, and the tax summary; and `run`,
//! the pipeline, which answers the `ledger` and `prepare` operations for a
//! generic book without the Bitcoin rail carry.

pub(crate) mod compose;
pub(crate) mod describe;
pub(crate) mod normalize;
pub(crate) mod order;
pub mod request;
pub mod response;
pub(crate) mod run;
pub mod text;
pub(crate) mod walk;
