//! Which Kassiber build a broker or client is, without revealing paths
//! (`kassiber/operator/build.py`).
//!
//! Every build of one OS user shares the one per-user endpoint, and a
//! broker runs its own build's code for every queued child, so a passphrase
//! goes only to the same build. This guards against accidental mixing (a
//! development broker serving the release app), not against same-user code.
//!
//! The Rust binary reports `frozen: true` in release builds, as a packaged
//! Python sidecar does, and `frozen: false` in `test-harness` builds, which
//! admit the gated test runtime directory. Its `origin` hashes the resolved
//! executable (or the AppImage file mounted at `APPDIR`), as a frozen Python
//! build does; a Python source checkout hashes its package directory, so the
//! two never share an origin.

use crate::values::{dict, get, s, truthy};
use kassiber_pyjson::Value;
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// Whether this binary is a release build (Python's `sys.frozen`).
pub const FROZEN: bool = !cfg!(any(test, feature = "test-harness"));

/// The version, channel and commit a release job embedded.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BuildInputs<'a> {
    pub version: &'a str,
    pub channel: Option<&'a str>,
    pub commit: Option<&'a str>,
}

/// `build_identity()`: `{version, channel, commit, frozen, origin}` in
/// Python's key order.
pub fn build_identity(inputs: BuildInputs<'_>, frozen: bool, location: &Path) -> Value {
    let channel = inputs.channel.map(str::trim).filter(|c| !c.is_empty());
    let commit: Option<String> = inputs
        .commit
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(|c| c.chars().take(12).collect());
    dict([
        ("version", s(inputs.version)),
        ("channel", channel.map_or(Value::Null, s)),
        ("commit", commit.map_or(Value::Null, |c| s(c.as_str()))),
        ("frozen", Value::Bool(frozen)),
        ("origin", s(origin(location).as_str())),
    ])
}

/// The first 12 hex digits of sha256 of the location's text.
pub fn origin(location: &Path) -> String {
    let digest = Sha256::digest(location_bytes(location));
    digest
        .iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(unix)]
fn location_bytes(location: &Path) -> Vec<u8> {
    // Python encodes str(path) as UTF-8; a non-UTF-8 path would raise there.
    std::os::unix::ffi::OsStrExt::as_bytes(location.as_os_str()).to_vec()
}

#[cfg(windows)]
fn location_bytes(location: &Path) -> Vec<u8> {
    location.to_string_lossy().as_bytes().to_vec()
}

/// `_frozen_location()`: the resolved executable, or the AppImage file when
/// the executable lies inside the `APPDIR` mount.
pub fn frozen_location(
    executable: &Path,
    appimage: Option<OsString>,
    appdir: Option<OsString>,
) -> PathBuf {
    let executable = resolve(executable);
    if let (Some(appimage), Some(appdir)) = (
        appimage.filter(|v| !v.is_empty()),
        appdir.filter(|v| !v.is_empty()),
    ) {
        if executable.starts_with(resolve(Path::new(&appdir))) {
            return resolve(Path::new(&appimage));
        }
    }
    executable
}

/// `Path.resolve()` (non-strict).
fn resolve(path: &Path) -> PathBuf {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|cwd| cwd.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };
    kassiber_paths::canonicalize_allow_missing(&absolute).unwrap_or(absolute)
}

/// This process's identity: the running executable as its location.
pub fn current_build_identity(inputs: BuildInputs<'_>) -> Value {
    let executable = std::env::current_exe().unwrap_or_default();
    let location = frozen_location(
        &executable,
        std::env::var_os("APPIMAGE"),
        std::env::var_os("APPDIR"),
    );
    build_identity(inputs, FROZEN, &location)
}

/// `describe_build(build)`: how errors name another broker's build.
pub fn describe_build(build: Option<&Value>) -> String {
    let Some(build @ Value::Dict(_)) = build else {
        return "an older Kassiber build".into();
    };
    let field = |key: &str, fallback: &str| match get(build, key) {
        Some(value) if truthy(value) => kassiber_envelope::py_str(value),
        _ => fallback.to_owned(),
    };
    let version = field("version", "unknown");
    let kind = if get(build, "frozen").is_some_and(truthy) {
        "packaged"
    } else {
        "source"
    };
    let channel = match get(build, "channel") {
        Some(value) if truthy(value) => format!(" {}", kassiber_envelope::py_str(value)),
        _ => String::new(),
    };
    let origin = field("origin", "unknown origin");
    format!("Kassiber {version}{channel} ({kind}, {origin})")
}

#[cfg(test)]
mod tests {
    use super::*;

    const INPUTS: BuildInputs<'static> = BuildInputs {
        version: "0.22.78",
        channel: None,
        commit: None,
    };

    #[test]
    fn identity_has_python_keys_and_short_digests() {
        let identity = build_identity(
            BuildInputs {
                version: "0.22.78",
                channel: Some(" stable "),
                commit: Some(" 0123456789abcdef "),
            },
            true,
            Path::new("/opt/Kassiber/kassiber"),
        );
        let text = kassiber_pyjson::dumps(&identity, &kassiber_pyjson::Options::new()).unwrap();
        // hashlib.sha256(b"/opt/Kassiber/kassiber").hexdigest()[:12]
        assert_eq!(
            text,
            format!(
                r#"{{"version": "0.22.78", "channel": "stable", "commit": "0123456789ab", "frozen": true, "origin": "{}"}}"#,
                origin(Path::new("/opt/Kassiber/kassiber"))
            )
        );
        assert_eq!(origin(Path::new("/opt/Kassiber/kassiber")).len(), 12);
        let blank = build_identity(
            BuildInputs {
                channel: Some("  "),
                commit: Some(""),
                ..INPUTS
            },
            false,
            Path::new("/x"),
        );
        assert_eq!(get(&blank, "channel"), Some(&Value::Null));
        assert_eq!(get(&blank, "commit"), Some(&Value::Null));
        assert_eq!(get(&blank, "frozen"), Some(&Value::Bool(false)));
    }

    #[test]
    fn test_builds_are_not_frozen() {
        const { assert!(!FROZEN) };
    }

    #[test]
    fn appimage_location_is_trusted_only_inside_its_mount() {
        let mount = tempfile::tempdir().unwrap();
        let executable = mount.path().join("usr/bin/kassiber");
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, b"").unwrap();
        let appimage = mount.path().join("Kassiber.AppImage");
        assert_eq!(
            frozen_location(
                &executable,
                Some(appimage.clone().into()),
                Some(mount.path().into())
            ),
            resolve(&appimage)
        );
        let elsewhere = tempfile::tempdir().unwrap();
        assert_eq!(
            frozen_location(
                &executable,
                Some(appimage.into()),
                Some(elsewhere.path().into())
            ),
            resolve(&executable)
        );
    }

    #[test]
    fn descriptions_match_python() {
        let parse = |text: &str| kassiber_pytext::loads(text).unwrap();
        assert_eq!(describe_build(None), "an older Kassiber build");
        assert_eq!(
            describe_build(Some(&parse("[]"))),
            "an older Kassiber build"
        );
        assert_eq!(
            describe_build(Some(&parse("{}"))),
            "Kassiber unknown (source, unknown origin)"
        );
        assert_eq!(
            describe_build(Some(&parse(
                r#"{"version": "1.2", "channel": "beta", "frozen": 1, "origin": "abc"}"#
            ))),
            "Kassiber 1.2 beta (packaged, abc)"
        );
        assert_eq!(
            describe_build(Some(&parse(
                r#"{"version": 3, "channel": "", "origin": ""}"#
            ))),
            "Kassiber 3 (source, unknown origin)"
        );
    }
}
