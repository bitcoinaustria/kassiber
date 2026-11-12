//! Hidden-home (`~/.kassiber`) state and explicit selections. An explicit
//! `--data-root` wins: the old root is neither read for routing nor moved,
//! and a catalog there that Rust would refuse cannot block it. `--env-file`
//! and `backup import` skip the cutover and use the old root where it is;
//! only a plain default run moves it. Replaces the hidden-home tests of
//! `tests/test_cli_entrypoint_smoke.py`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::support::Sandbox;

/// A catalog in the shape Python writes, naming one project elsewhere.
fn python_catalog(project: &Path) -> String {
    let catalog = json!({
        "schema_version": 1,
        "selected_project_id": "elsewhere",
        "projects": [{
            "id": "elsewhere",
            "name": "Elsewhere",
            "path": project,
            "encrypted": false,
            "last_opened_at": null,
        }],
    });
    serde_json::to_string_pretty(&catalog).unwrap() + "\n"
}

/// A sandbox whose HOME holds `~/.kassiber/config/projects.json` with
/// `catalog`; the native root (`$XDG_DATA_HOME/kassiber`) is absent.
fn sandbox(catalog: &str) -> Sandbox {
    let mut sandbox = Sandbox::empty();
    let home = sandbox.path("home");
    fs::create_dir_all(legacy(&sandbox).join("config")).unwrap();
    fs::write(legacy(&sandbox).join("config/projects.json"), catalog).unwrap();
    sandbox.set("HOME", home.to_string_lossy().into_owned());
    sandbox
}

fn legacy(sandbox: &Sandbox) -> PathBuf {
    sandbox.path("home/.kassiber")
}

fn native(sandbox: &Sandbox) -> PathBuf {
    sandbox.path("xdg-data/kassiber")
}

/// Every entry under `root` with a file's bytes; a directory maps to None.
fn tree(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap().to_owned();
            if path.is_dir() {
                out.insert(relative, None);
                walk(root, &path, out);
            } else {
                out.insert(relative, Some(fs::read(&path).unwrap()));
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// No native root, nor the cutover's lock beside it.
fn assert_no_native(sandbox: &Sandbox) {
    let entries: Vec<_> = fs::read_dir(sandbox.path("xdg-data"))
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(entries.is_empty(), "{entries:?}");
    assert!(!native(sandbox).exists());
}

fn run(sandbox: &Sandbox, args: &[String]) -> (i32, Value) {
    let output = sandbox.raw(args, None);
    (output.code, output.json())
}

fn args(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn test_explicit_data_root_does_not_migrate_hidden_home_state() {
    let elsewhere = Sandbox::empty();
    // `{}` is a catalog Python reads as empty and Rust refuses as a default
    // root's live catalog; a Python-shaped one names another project.
    for catalog in ["{}\n".to_owned(), python_catalog(&elsewhere.path("p"))] {
        let sandbox = sandbox(&catalog);
        let before = tree(&legacy(&sandbox));
        let explicit = sandbox.path("explicit/data");
        let explicit_text = explicit.to_string_lossy().into_owned();
        let (code, payload) = run(
            &sandbox,
            &args(&["--machine", "--data-root", &explicit_text, "init"]),
        );
        assert_eq!(code, 0, "{payload}");
        assert_eq!(payload["kind"], "init");
        let data = &payload["data"];
        assert_eq!(data["data_root"], json!(explicit));
        assert_eq!(data["state_root"], json!(sandbox.path("explicit")));
        assert_eq!(data["project_id"], Value::Null);
        assert!(explicit.join("kassiber.sqlite3").is_file());
        // Not moved, not rewritten, not marked.
        assert_eq!(tree(&legacy(&sandbox)), before);
        assert_no_native(&sandbox);
        // Later reads of the explicit root keep away from it too.
        let (code, payload) = run(
            &sandbox,
            &args(&["--machine", "--data-root", &explicit_text, "status"]),
        );
        assert_eq!(code, 0, "{payload}");
        assert_eq!(tree(&legacy(&sandbox)), before);
        assert_no_native(&sandbox);
    }
}

#[test]
fn test_explicit_data_root_names_its_hidden_home_project_without_moving_it() {
    // A project registered in the hidden-home catalog, opened by its own
    // data root: Python's metadata lookup names it, and nothing moves.
    let mut sandbox = Sandbox::empty();
    let home = sandbox.path("home");
    let project = legacy(&sandbox).join("projects/default");
    sandbox.set("HOME", home.to_string_lossy().into_owned());
    let mut catalog: Value = serde_json::from_str(&python_catalog(&project)).unwrap();
    catalog["projects"][0]["id"] = json!("default");
    catalog["selected_project_id"] = json!("default");
    fs::create_dir_all(legacy(&sandbox).join("config")).unwrap();
    fs::write(
        legacy(&sandbox).join("config/projects.json"),
        serde_json::to_string_pretty(&catalog).unwrap() + "\n",
    )
    .unwrap();
    let data = project.join("data").to_string_lossy().into_owned();
    let (code, payload) = run(
        &sandbox,
        &args(&["--machine", "--data-root", &data, "init"]),
    );
    assert_eq!(code, 0, "{payload}");
    assert_eq!(payload["data"]["project_id"], "default");
    assert_eq!(payload["data"]["project_root"], json!(project));
    assert!(project.join("data/kassiber.sqlite3").is_file());
    assert!(!legacy(&sandbox).join(".state-root-migration.json").exists());
    assert_no_native(&sandbox);
}

#[test]
fn test_explicit_env_file_does_not_migrate_hidden_home_state() {
    // Python's catalog shape: Rust refuses a schema-less one in a live root.
    let empty = "{\"schema_version\": 1, \"selected_project_id\": null, \"projects\": []}\n";
    let sandbox = sandbox(empty);
    let env_file = sandbox.path("backends.env");
    fs::write(&env_file, "").unwrap();
    let env_text = env_file.to_string_lossy().into_owned();
    let (code, payload) = run(
        &sandbox,
        &args(&["--machine", "--env-file", &env_text, "init"]),
    );
    assert_eq!(code, 0, "{payload}");
    // The old root is used where it is, as Python's default_state_root does.
    let project = legacy(&sandbox).join("projects/default");
    assert_eq!(payload["data"]["project_id"], "default");
    assert_eq!(payload["data"]["data_root"], json!(project.join("data")));
    assert_eq!(payload["data"]["env_file"], json!(env_file));
    assert!(project.join("data/kassiber.sqlite3").is_file());
    assert!(legacy(&sandbox).join("config/projects.json").is_file());
    assert!(!legacy(&sandbox).join(".state-root-migration.json").exists());
    assert_no_native(&sandbox);
}

#[test]
fn test_backup_import_does_not_migrate_hidden_home_state() {
    let sandbox = sandbox("{}\n");
    let before = tree(&legacy(&sandbox));
    let missing = sandbox.path("missing.kassiber");
    let output = sandbox.raw(
        &args(&["--machine", "backup", "import", &missing.to_string_lossy()]),
        None,
    );
    assert_ne!(output.code, 0);
    // Neither the cutover's startup lock nor a moved root.
    assert_eq!(tree(&legacy(&sandbox)), before);
    assert_no_native(&sandbox);
}

#[test]
fn test_default_run_still_moves_hidden_home_state() {
    // Without an explicit selection the cutover runs first, as in Python.
    let empty = "{\"schema_version\": 1, \"selected_project_id\": null, \"projects\": []}\n";
    let sandbox = sandbox(empty);
    let (code, payload) = run(&sandbox, &args(&["--machine", "init"]));
    assert_eq!(code, 0, "{payload}");
    let project = native(&sandbox).join("projects/default");
    assert_eq!(payload["data"]["data_root"], json!(project.join("data")));
    assert!(native(&sandbox)
        .join(".state-root-migration.json")
        .is_file());
    assert!(!legacy(&sandbox).join("config").exists());
    assert!(legacy(&sandbox).join("run").is_dir());
}
