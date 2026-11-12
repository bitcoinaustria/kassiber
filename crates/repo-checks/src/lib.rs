//! Repository contracts checked by this crate's tests: GitHub Actions pins,
//! the generated test-data lock, the product version across packages, and
//! the locked package-manager workflow. Nothing here ships.

use std::path::{Path, PathBuf};

/// The repository root.
pub fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}
