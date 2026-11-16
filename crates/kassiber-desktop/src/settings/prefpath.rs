//! Where the machine-wide preference files the daemon and the CLI read live, resolved the way
//! they resolve them (`kassiber_paths::Context` in `crates/release/src/update.rs` `state_file`
//! and `crates/setup/src/wallet_backend_lookup.rs` `preference_path`): the override variable
//! when it names a file, else `<default state root>/config/<name>`.
//!
//! The daemon's environment is this process's plus `Launch::env` (the demo's own preference
//! files), so the demo's value wins over the process's, exactly as the daemon sees it.
//!
//! Tooling runs (headless renders, scripts, scenes, unit tests) only use a file the environment
//! names: without one they keep the preference read-only rather than touch the account's real
//! state root.

use std::path::{Path, PathBuf};

/// The update-check consent (`<state root>/config/update-checks.json`).
pub const UPDATE_CHECKS: Pref = Pref {
    env: "KASSIBER_UPDATE_PREFERENCE_FILE",
    file: "update-checks.json",
};
/// "Fill in transaction details from the wallet's backend" (`wallet-backend-lookup.json`).
pub const WALLET_BACKEND_LOOKUP: Pref = Pref {
    env: "KASSIBER_WALLET_BACKEND_LOOKUP_FILE",
    file: "wallet-backend-lookup.json",
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pref {
    /// The override variable (tests, previews, the demo copy).
    pub env: &'static str,
    /// The file under `<state root>/config`.
    pub file: &'static str,
}

/// The pure rule: the launch's value, then the process's, then the default state root's file
/// (not in tooling runs). An empty value counts as unset, as in the core.
pub fn resolve(
    pref: Pref,
    launch_value: Option<&str>,
    process_value: Option<&str>,
    state_root: Option<&Path>,
    tooling: bool,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(v) = launch_value.or(process_value).filter(|v| !v.is_empty()) {
        return Some(expand_user(v, home));
    }
    if tooling {
        return None;
    }
    state_root.map(|r| r.join("config").join(pref.file))
}

/// `Context::expand_user` for the common forms (`~`, `~/…`).
fn expand_user(v: &str, home: Option<&Path>) -> PathBuf {
    match (v.strip_prefix('~'), home) {
        (Some(""), Some(h)) => h.to_path_buf(),
        (Some(rest), Some(h)) if rest.starts_with(['/', '\\']) => h.join(&rest[1..]),
        _ => PathBuf::from(v),
    }
}

/// The preference's file for a daemon started with `launch_env` (`app.daemon.launch.env`).
pub fn path(pref: Pref, launch_env: &[(String, String)]) -> Option<PathBuf> {
    let launch = launch_env
        .iter()
        .find(|(k, _)| k == pref.env)
        .map(|(_, v)| v.as_str());
    let process = std::env::var(pref.env).ok();
    let tooling = crate::persist::file::tooling_run();
    // Only resolved when it is used: `default_state_root` looks at the legacy `~/.kassiber`.
    let needs_root = launch.or(process.as_deref()).is_none_or(str::is_empty) && !tooling;
    let root = if needs_root {
        crate::daemon::default_state_root()
    } else {
        None
    };
    resolve(
        pref,
        launch,
        process.as_deref(),
        root.as_deref(),
        tooling,
        crate::daemon::home_dir().as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_the_variable_the_default_state_root_is_used() {
        let root = Path::new("/Users/u/Library/Application Support/at.bitcoinaustria.kassiber");
        assert_eq!(
            resolve(WALLET_BACKEND_LOOKUP, None, None, Some(root), false, None),
            Some(root.join("config/wallet-backend-lookup.json"))
        );
        assert_eq!(
            resolve(UPDATE_CHECKS, None, Some(""), Some(root), false, None),
            Some(root.join("config/update-checks.json")),
            "an empty value is unset"
        );
    }

    #[test]
    fn the_demos_file_wins_over_the_process_and_the_default() {
        let root = Path::new("/r");
        assert_eq!(
            resolve(
                WALLET_BACKEND_LOOKUP,
                Some("/demo/config/w.json"),
                Some("/p/w.json"),
                Some(root),
                false,
                None
            ),
            Some(PathBuf::from("/demo/config/w.json"))
        );
        assert_eq!(
            resolve(
                WALLET_BACKEND_LOOKUP,
                None,
                Some("/p/w.json"),
                Some(root),
                false,
                None
            ),
            Some(PathBuf::from("/p/w.json"))
        );
        assert_eq!(
            resolve(
                UPDATE_CHECKS,
                None,
                Some("~/u.json"),
                None,
                false,
                Some(Path::new("/home/u"))
            ),
            Some(PathBuf::from("/home/u/u.json"))
        );
    }

    #[test]
    fn tooling_runs_only_use_a_named_file() {
        let root = Path::new("/r");
        assert_eq!(
            resolve(UPDATE_CHECKS, None, None, Some(root), true, None),
            None
        );
        assert_eq!(
            resolve(
                UPDATE_CHECKS,
                None,
                Some("/p/u.json"),
                Some(root),
                true,
                None
            ),
            Some(PathBuf::from("/p/u.json"))
        );
    }

    #[test]
    fn a_release_launch_resolves_the_file_through_the_state_root() {
        // What `path` does outside tooling: the platform's default state root.
        let home = Path::new("/Users/u");
        let root = crate::daemon::state_root_for_platform("macos", Some(home), None, None).unwrap();
        let p = resolve(
            WALLET_BACKEND_LOOKUP,
            None,
            None,
            Some(&root),
            false,
            Some(home),
        )
        .unwrap();
        assert_eq!(
            p,
            Path::new(
                "/Users/u/Library/Application Support/at.bitcoinaustria.kassiber/config/wallet-backend-lookup.json"
            )
        );
    }
}
