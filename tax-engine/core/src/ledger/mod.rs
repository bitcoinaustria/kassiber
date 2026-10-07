//! The ledger layer (schema version 2): the engine boundary at the finalized
//! custody projection, plan 20 phase 4.
//!
//! The layer builds the lot engine's entries from projection rows, runs the
//! existing lot engine on them, and composes the journal ledger. So far it
//! holds [`text`], which renders an entry's `str(transaction)` once the
//! constructor has derived its values.

pub mod text;
