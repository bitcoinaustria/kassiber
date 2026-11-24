//! Repository contracts checked by this crate's tests: GitHub Actions pins,
//! the generated test-data lock, and repository metadata (no JavaScript
//! package manager). Nothing here ships.

use std::path::{Path, PathBuf};

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
