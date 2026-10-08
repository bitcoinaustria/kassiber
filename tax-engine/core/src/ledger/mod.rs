//! The ledger layer (schema version 2): the engine boundary at the finalized
//! custody projection, plan 20 phase 4.
//!
//! The layer builds the lot engine's entries from projection rows, runs the
//! existing lot engine on them, and composes the journal ledger. So far it
//! holds the boundary documents, [`request`] and [`response`]; [`text`],
//! which renders an entry's `str(transaction)` once the constructor has
//! derived its values; `normalize`, the adapter's normalization of one
//! asset's rows and moves (generic books); and `run`, the pipeline, which
//! so far answers the `prepare` operation up to the normalized stage.

pub(crate) mod normalize;
pub mod request;
pub mod response;
pub(crate) mod run;
pub mod text;
