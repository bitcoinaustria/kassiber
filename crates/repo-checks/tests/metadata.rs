//! Package metadata that has to agree across the Rust, desktop and pnpm
//! sides: the product version, and the locked package-manager workflow.

use std::fs;

use kassiber_repo_checks::root;

fn toml_version(path: &str, table: &str) -> String {
    let text = fs::read_to_string(root().join(path)).unwrap();
    let body = text
        .split(&format!("[{table}]"))
        .nth(1)
        .unwrap_or_else(|| panic!("[{table}] not found in {path}"));
    let body = body.split("\n[").next().unwrap();
    body.lines()
        .find_map(|line| line.trim().strip_prefix("version = \""))
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or_else(|| panic!("[{table}] in {path} declares no version"))
        .to_owned()
}

fn json(path: &str) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(root().join(path)).unwrap()).unwrap()
}

#[test]
fn app_version_is_consistent_across_package_metadata() {
    let product = toml_version("crates/kassiber/Cargo.toml", "package");
    assert_eq!(json("ui-tauri/package.json")["version"], product.as_str());
    assert_eq!(
        json("ui-tauri/src-tauri/tauri.conf.json")["version"],
        product.as_str()
    );
    assert_eq!(
        toml_version("ui-tauri/src-tauri/Cargo.toml", "package"),
        product
    );
}

#[test]
fn the_desktop_uses_the_pinned_pnpm_and_its_lockfile_only() {
    let package = json("ui-tauri/package.json");
    assert_eq!(package["packageManager"], "pnpm@10.33.0");
    let scripts = package["scripts"].as_object().unwrap();
    assert!(scripts
        .values()
        .all(|s| !s.as_str().unwrap_or_default().contains("npx ")));
    let root = root();
    assert!(root.join("ui-tauri/pnpm-lock.yaml").is_file());
    for stray in [
        "package-lock.json",
        "ui-tauri/package-lock.json",
        "yarn.lock",
        "ui-tauri/yarn.lock",
    ] {
        assert!(!root.join(stray).exists(), "{stray}");
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
