//! The harness binary end to end on synthetic books, pinned to golden
//! transcripts under `tests/black_box/golden/` (`KASSIBER_BLESS=1` rewrites
//! them). Each module replaces one former Python parity module; its tests
//! keep that module's test names so the mutation catalogs' trail reads.

#![cfg(feature = "test-harness")]

#[path = "black_box/support.rs"]
mod support;

#[path = "black_box/cli_leaves.rs"]
mod cli_leaves;
