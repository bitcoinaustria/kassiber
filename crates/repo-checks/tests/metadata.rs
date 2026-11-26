//! Repository metadata contracts: the product is Rust only, with no
//! JavaScript package manifest or lockfile, and shared defaults agree
//! across crates.

use std::fs;

use kassiber_repo_checks::root;

#[test]
fn no_javascript_package_manager_files_remain() {
    // The React desktop was removed; the desktop app is Rust
    // (crates/kassiber-desktop) and nothing in the tree installs npm packages.
    let root = root();
    for dir in ["", "crates/kassiber-desktop"] {
        for stray in [
            "package.json",
            "package-lock.json",
            "pnpm-lock.yaml",
            "pnpm-workspace.yaml",
            "yarn.lock",
            ".npmrc",
        ] {
            let path = root.join(dir).join(stray);
            assert!(!path.exists(), "{}", path.display());
        }
    }
}

#[test]
fn reports_balance_history_defaults_to_the_shared_interval() {
    // The CLI parser's default for `reports balance-history --interval` and
    // the assistant's visual default are the same interval.
    let visuals = fs::read_to_string(root().join("crates/ai/src/visuals.rs")).unwrap();
    let shared = visuals
        .lines()
        .find_map(|l| {
            l.trim()
                .strip_prefix("const DEFAULT_BALANCE_HISTORY_INTERVAL: &str = \"")
        })
        .and_then(|rest| rest.strip_suffix("\";"))
        .expect("the visual default");
    let spec = fs::read_to_string(root().join("crates/cli/src/spec_data.rs")).unwrap();
    let parser = spec
        .split("balance-history")
        .skip(1)
        .find_map(|after| {
            let start = after.find("dest: Some(\"interval\")")?;
            let action = &after[start..start + after[start..].find("},")?];
            action
                .split("default: Lit::Str(\"")
                .nth(1)?
                .split('"')
                .next()
                .map(str::to_owned)
        })
        .expect("the parser default for --interval");
    assert_eq!(parser, shared);
}
