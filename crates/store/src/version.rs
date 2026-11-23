//! The Kassiber version the product reports: in `--version` and status, the
//! MCP and daemon handshakes, HTTP User-Agents, stored settings, record
//! fingerprints and exports (the XLSX Verify sheet, the year handoff). Every
//! use reads it here.
//!
//! Test builds (feature `test-version`, which `kassiber/test-harness` and
//! `kassiber-reports/test-harness` enable) report `KASSIBER_RUST_TEST_VERSION`
//! instead when it is set, so a recorded session replays with the version it
//! was recorded at and a version bump changes no recording
//! (docs/reference/testing.md#test-builds). A release build never reads the
//! variable; `scripts/check-no-test-tape.sh` checks that it does not carry
//! its name.

/// The test-only override of the reported version.
#[cfg(feature = "test-version")]
pub const OVERRIDE: &str = "KASSIBER_RUST_TEST_VERSION";

/// The version of the `kassiber` package, the product's release version:
/// the first `version = "..."` line of its manifest (this crate's own
/// version is not the product's).
pub fn package() -> &'static str {
    include_str!("../../kassiber/Cargo.toml")
        .lines()
        .find_map(|line| line.strip_prefix("version = \"")?.strip_suffix('"'))
        .expect("the kassiber package version")
}

/// The version every caller reports. In a test build an override that is
/// not a version like `1.2.3` ends the process with exit 78, before it can
/// write the value anywhere.
pub fn current() -> &'static str {
    #[cfg(feature = "test-version")]
    {
        use std::sync::OnceLock;
        static REPORTED: OnceLock<&'static str> = OnceLock::new();
        REPORTED.get_or_init(|| {
            let value = std::env::var(OVERRIDE).ok();
            match reported(value.as_deref()) {
                Ok(version) => version,
                Err(message) => {
                    eprintln!("{message}");
                    std::process::exit(78);
                }
            }
        })
    }
    #[cfg(not(feature = "test-version"))]
    {
        package()
    }
}

/// The version a test build reports for an override value: the package
/// version when there is none, the value when it is a release version, and
/// an error for anything else.
#[cfg(feature = "test-version")]
fn reported(value: Option<&str>) -> Result<&'static str, String> {
    let Some(value) = value else {
        return Ok(package());
    };
    let plausible = value.as_bytes().first().is_some_and(u8::is_ascii_digit)
        && value.contains('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte));
    if !plausible {
        return Err(format!("{OVERRIDE} is not a version like 1.2.3: {value:?}"));
    }
    Ok(Box::leak(value.to_owned().into_boxed_str()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_package_version_is_the_kassiber_release() {
        let manifest = include_str!("../../kassiber/Cargo.toml");
        assert!(manifest.contains(&format!("\nversion = \"{}\"\n", super::package())));
        assert!(super::package().starts_with(|c: char| c.is_ascii_digit()));
    }

    #[cfg(feature = "test-version")]
    #[test]
    fn a_test_build_reports_a_valid_override() {
        use super::{package, reported};
        assert_eq!(reported(None), Ok(package()));
        assert_eq!(reported(Some("0.22.78")), Ok("0.22.78"));
        assert_eq!(
            reported(Some("1.0.0-alpha.1+build")),
            Ok("1.0.0-alpha.1+build")
        );
        for invalid in ["", "ai", "1", "v1.0.0", "1.0.0 extra", "1.0.0\n"] {
            assert!(reported(Some(invalid)).is_err(), "{invalid:?}");
        }
    }

    /// A build without `test-version` (every release build) has no override:
    /// whatever the environment holds, it reports the package version.
    #[cfg(not(feature = "test-version"))]
    #[test]
    fn a_release_build_ignores_the_override() {
        // Test threads share the environment; only this test reads or writes
        // the variable in this build.
        std::env::set_var("KASSIBER_RUST_TEST_VERSION", "0.22.78");
        assert_eq!(super::current(), super::package());
        assert_ne!(super::current(), "0.22.78");
        std::env::remove_var("KASSIBER_RUST_TEST_VERSION");
    }
}
