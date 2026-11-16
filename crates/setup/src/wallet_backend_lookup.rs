//! Machine-wide preference for the desktop's automatic transaction-graph
//! lookup from a wallet's own sync backend ("Fill in transaction details from
//! the wallet's backend", Settings → Privacy). On by default, as the owner
//! chose: the user already syncs that wallet through that backend. Offline
//! mode and `KASSIBER_NO_EGRESS` override it; the daemon reads all three
//! before any request. The desktop writes this owner-only file itself, as it
//! writes the update-check preference: `{"enabled": bool, "schema_version": 1}`.
use super::*;
use kassiber_paths::Context;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

/// Overrides the preference file's path (disposable test and preview roots).
pub const ENV: &str = "KASSIBER_WALLET_BACKEND_LOOKUP_FILE";

/// `<state root>/config/wallet-backend-lookup.json`, unless [`ENV`] names a file.
pub fn preference_path(context: &Context) -> Result<PathBuf> {
    if let Some(path) = context
        .environ
        .get(OsStr::new(ENV))
        .filter(|v| !v.is_empty())
    {
        return context.expand_user(Path::new(path)).map_err(storage_error);
    }
    Ok(context
        .default_state_root()
        .map_err(storage_error)?
        .join("config/wallet-backend-lookup.json"))
}

/// Whether the automatic lookup is allowed. A missing file is the default
/// (on); a file that exists but is unreadable, symlinked, or of an unknown
/// schema counts as off, so a damaged preference never widens egress.
pub fn enabled(path: &Path) -> bool {
    if kassiber_paths::preference_path_is_absent(path) {
        return true;
    }
    let Some(raw) = kassiber_paths::read_small_regular_file(path, 4096) else {
        return false;
    };
    let Some(payload) = std::str::from_utf8(&raw)
        .ok()
        .and_then(|s| kassiber_pytext::loads(s).ok())
    else {
        return false;
    };
    get(&payload, "schema_version") == Some(&Value::Int(1.into()))
        && get(&payload, "enabled") == Some(&Value::Bool(true))
}

/// The preference as the daemon reads it before a lookup, failing closed when
/// the state root cannot be resolved.
pub fn enabled_for(context: &Context) -> bool {
    preference_path(context).is_ok_and(|path| enabled(&path))
}
