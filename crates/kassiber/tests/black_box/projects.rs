//! The desktop project catalog: creation, switching, encrypted projects and
//! refusals, with the catalog file and every project's tables recorded.
//! Replaces the kill-holding tests of `tests/test_rust_project_desktop.py`.

use serde_json::{json, Value};

use crate::shell::{requests, shell};
use crate::support::{dump_at, mask_after, Sandbox, Transcript};

const KEY: &str = "R5 project synthetic '🔑";
const KINDS: [&str; 3] = [
    "ui.projects.list",
    "ui.projects.create",
    "ui.projects.select",
];

fn auth(secret: &str) -> Value {
    json!({"auth_response": {"passphrase_secret": secret}})
}

fn with(mut args: Value, extra: Value) -> Value {
    for (key, value) in extra.as_object().unwrap() {
        args[key] = value.clone();
    }
    args
}

fn sandbox() -> Sandbox {
    let mut sandbox = Sandbox::empty();
    shell(&mut sandbox);
    sandbox
}

/// The default state root the desktop daemon resolves without --data-root.
fn state_root(sandbox: &Sandbox) -> std::path::PathBuf {
    sandbox.path("xdg-data/kassiber")
}

fn transcript(name: &str) -> Transcript {
    let mut t = Transcript::new(name);
    t.mask(|text| mask_after(text, "database_instance_id\" | value=t:\"", 32, "<random>"));
    t
}

/// The catalog file and each project book's tables (`occupied` as bytes).
fn state(t: &mut Transcript, sandbox: &Sandbox, keys: &[(&str, &str)], explicit: Option<&str>) {
    let root = state_root(sandbox);
    let catalog = root.join("config/projects.json");
    let text = std::fs::read_to_string(&catalog).unwrap_or_else(|_| "<absent>\n".to_owned());
    t.section("catalog", &sandbox.scrub(&text));
    let projects = root.join("projects");
    let mut names: Vec<String> = std::fs::read_dir(&projects)
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    for name in names {
        let data = projects.join(&name).join("data");
        let database = data.join("kassiber.sqlite3");
        if !database.exists() {
            continue;
        }
        let body = if name == "occupied" {
            format!("{:?}\n", std::fs::read(&database).unwrap())
        } else {
            let key = keys.iter().find(|(p, _)| *p == name).map(|(_, k)| *k);
            dump_at(&data, key)
        };
        t.section(&format!("project {name}"), &sandbox.scrub(&body));
    }
    if let Some(explicit) = explicit {
        let data = sandbox.path(explicit);
        if data.join("kassiber.sqlite3").exists() {
            t.section("explicit book", &sandbox.scrub(&dump_at(&data, None)));
        }
    }
}

fn exchange(
    t: &mut Transcript,
    sandbox: &Sandbox,
    argv: &[&str],
    pairs: &[(&str, Value)],
) -> Vec<Value> {
    let responses = t.daemon_argv(sandbox, argv, &requests(pairs));
    for response in &responses {
        assert_ne!(response["error"]["code"], "not_implemented", "{response}");
    }
    responses
}

#[test]
fn test_project_catalog_creation_and_switching_match_python() {
    let sandbox = sandbox();
    let mut t = transcript("projects/catalog_creation_and_switching");
    let pairs = vec![
        ("ui.projects.list", json!({})),
        ("ui.projects.list", json!([1])),
        ("ui.projects.create", json!([])),
        ("ui.projects.create", json!({"name": " \t"})),
        ("ui.projects.create", json!({"name": 7})),
        (
            "ui.projects.create",
            json!({"name": "Alpha", "project_id": 5}),
        ),
        (
            "ui.projects.create",
            with(json!({"name": "Alpha"}), auth("short")),
        ),
        ("ui.projects.create", json!({"name": " Alpha "})),
        ("status", json!({})),
        ("ui.projects.list", json!(null)),
        ("ui.projects.create", json!({"name": "ALPHA"})),
        (
            "ui.projects.create",
            json!({"name": "Beta", "select": false}),
        ),
        (
            "ui.projects.create",
            json!({"name": "Gamma", "project_id": "", "select": 0}),
        ),
        (
            "ui.projects.create",
            with(
                json!({"name": "Vault", "project_id": "Vault Book", "select": false}),
                auth(KEY),
            ),
        ),
        ("ui.projects.list", json!({})),
        ("ui.projects.select", json!({})),
        ("ui.projects.select", json!({"project_id": " "})),
        ("ui.projects.select", json!({"project_id": 3})),
        ("ui.projects.select", json!({"project_id": "missing"})),
        (
            "ui.projects.select",
            json!({"project_id": "", "id": "BETA"}),
        ),
        ("status", json!({})),
        ("ui.projects.select", json!({"project_id": "beta"})),
        ("ui.projects.select", json!({"project_id": "vault-book"})),
        (
            "ui.projects.select",
            with(
                json!({"project_id": "vault-book"}),
                auth("wrong passphrase value"),
            ),
        ),
        (
            "ui.projects.select",
            with(json!({"project_id": "vault-book"}), auth(KEY)),
        ),
        ("status", json!({})),
        ("ui.projects.select", json!({"project_id": "vault-book"})),
        ("daemon.lock", json!({})),
        ("ui.projects.select", json!({"project_id": "vault-book"})),
        (
            "ui.projects.select",
            with(
                json!({"project_id": "vault-book", "require_existing_project": true}),
                auth(KEY),
            ),
        ),
        (
            "ui.projects.create",
            with(json!({"name": "Delta"}), auth(KEY)),
        ),
        ("status", json!({})),
        ("ui.projects.select", json!({"project_id": "alpha"})),
        ("ui.projects.list", json!({})),
    ];
    let responses = exchange(&mut t, &sandbox, &["daemon"], &pairs);
    assert_eq!(responses[7]["data"]["unlocked"], true);
    assert_eq!(responses[22]["kind"], "auth_required");
    assert_eq!(responses[23]["error"]["code"], "local_auth_denied");
    assert_eq!(responses[24]["data"]["project"]["id"], "vault-book");
    assert_eq!(responses[30]["data"]["project"]["encrypted"], true);
    state(
        &mut t,
        &sandbox,
        &[("vault-book", KEY), ("delta", KEY)],
        None,
    );
    t.finish();
}

#[test]
fn test_project_creation_refuses_existing_databases_and_unlocked_lock_cycle() {
    let sandbox = sandbox();
    let occupied = state_root(&sandbox).join("projects/occupied/data");
    std::fs::create_dir_all(&occupied).unwrap();
    std::fs::write(occupied.join("kassiber.sqlite3"), b"not a database").unwrap();
    let mut t = transcript("projects/creation_refuses_existing_databases");
    let pairs = vec![
        ("ui.projects.create", json!({"name": "Occupied"})),
        ("ui.projects.list", json!({})),
        ("daemon.lock", json!({})),
        ("ui.projects.select", json!({"project_id": "default"})),
        (
            "ui.projects.create",
            json!({"name": "Next", "select": false}),
        ),
        (
            "ui.projects.select",
            json!({"id": "next", "require_existing_project": true}),
        ),
        ("ui.projects.select", json!({"id": "next"})),
        ("ui.projects.select", json!({"id": "default"})),
    ];
    let responses = exchange(&mut t, &sandbox, &["daemon"], &pairs);
    assert_eq!(responses[0]["error"]["code"], "database_exists");
    state(&mut t, &sandbox, &[], None);
    t.finish();
}

#[test]
fn test_explicit_root_switches_to_encrypted_project_and_reunlocks() {
    let sandbox = sandbox();
    let exact = "abcdefghijkl";
    let explicit = sandbox.path("explicit/data").to_string_lossy().into_owned();
    let mut t = transcript("projects/explicit_root_encrypted_switch");
    let pairs = vec![
        ("ui.projects.list", json!({})),
        ("status", json!({})),
        (
            "ui.projects.create",
            with(json!({"name": "Main", "select": false}), auth(&exact[..11])),
        ),
        (
            "ui.projects.create",
            with(json!({"name": "Main", "select": false}), auth(exact)),
        ),
        ("ui.projects.select", json!({"project_id": "main"})),
        (
            "ui.projects.select",
            with(json!({"project_id": "main"}), auth(exact)),
        ),
        ("ui.projects.list", json!({})),
        ("status", json!({})),
        ("daemon.lock", json!({})),
        ("ui.projects.list", json!({})),
        ("ui.projects.select", json!({"project_id": "MAIN"})),
        (
            "ui.projects.select",
            with(json!({"project_id": "main"}), auth(KEY)),
        ),
        (
            "ui.projects.select",
            with(json!({"project_id": "main"}), auth(exact)),
        ),
        ("status", json!({})),
        ("ui.projects.create", json!({"name": "Main"})),
    ];
    let responses = exchange(
        &mut t,
        &sandbox,
        &["--data-root", &explicit, "daemon"],
        &pairs,
    );
    assert_eq!(
        responses[0]["data"],
        json!({"selected_project_id": null, "projects": []})
    );
    assert_eq!(responses[2]["error"]["code"], "invalid_passphrase");
    assert_eq!(responses[5]["data"]["project"]["encrypted"], true);
    assert_eq!(responses[10]["kind"], "auth_required");
    assert_eq!(responses[11]["error"]["code"], "local_auth_denied");
    assert_eq!(responses[12]["kind"], "ui.projects.select");
    state(&mut t, &sandbox, &[("main", exact)], Some("explicit/data"));
    t.finish();
}

#[test]
fn test_require_existing_refuses_unrelated_sqlite_without_migrating_it() {
    let sandbox = sandbox();
    let foreign = sandbox.path("foreign/data");
    std::fs::create_dir_all(&foreign).unwrap();
    let database = foreign.join("kassiber.sqlite3");
    {
        let conn = kassiber_store::rusqlite::Connection::open(&database).unwrap();
        conn.execute_batch("CREATE TABLE unrelated(value TEXT)")
            .unwrap();
    }
    let root = state_root(&sandbox);
    let catalog = root.join("config/projects.json");
    std::fs::create_dir_all(catalog.parent().unwrap()).unwrap();
    let text = json!({
        "schema_version": 1, "selected_project_id": "default",
        "projects": [
            {"id": "default", "name": "Default",
             "path": root.join("projects/default").to_string_lossy(),
             "encrypted": false, "last_opened_at": null},
            {"id": "foreign", "name": "Foreign",
             "path": sandbox.path("foreign").to_string_lossy(),
             "encrypted": false, "last_opened_at": null},
        ],
    });
    std::fs::write(&catalog, text.to_string()).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&catalog, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let before = std::fs::read(&database).unwrap();
    let mut t = transcript("projects/require_existing_refuses_unrelated_sqlite");
    let pairs = vec![
        (
            "ui.projects.select",
            json!({"project_id": "foreign", "require_existing_project": true}),
        ),
        ("status", json!({})),
        ("ui.projects.list", json!({})),
    ];
    let responses = exchange(&mut t, &sandbox, &["daemon"], &pairs);
    assert_eq!(responses[0]["error"]["code"], "invalid_project_database");
    assert_eq!(std::fs::read(&database).unwrap(), before, "not migrated");
    state(&mut t, &sandbox, &[], None);
    t.finish();
}

#[test]
fn daemon_advertises_the_project_kinds() {
    let sandbox = sandbox();
    let out = sandbox.raw(
        &["daemon".to_owned()],
        Some(b"{\"request_id\":\"s\",\"kind\":\"daemon.shutdown\"}\n"),
    );
    let ready: Value = serde_json::from_str(out.text().lines().next().unwrap()).unwrap();
    let kinds = ready["data"]["supported_kinds"].as_array().unwrap();
    for kind in KINDS {
        assert!(kinds.contains(&json!(kind)), "{kind}");
    }
}
