//! `kassiber projects demo` (docs/reference/demo.md): the bundled demo book is
//! written, migrated and indexed locally, and opening it in the daemon finds
//! nothing to contact. Product egress stays allowed here (no
//! `KASSIBER_NO_EGRESS`, offline mode off), so only the book's own
//! configuration keeps it local; the strict test observer refuses any
//! connection, loopback included, that a regression would attempt.

#![cfg(feature = "test-harness")]

mod support;

use std::fs;
use std::path::PathBuf;

use kassiber_pyjson::Value;
use support::{kind_of, list, path, text, Desk};

fn demo_desk() -> (Desk, Value) {
    let mut desk = Desk::with_seed("demo_book", None);
    desk.env.remove("KASSIBER_NO_EGRESS");
    desk.env
        .insert("KASSIBER_TEST_NO_EGRESS".into(), "strict".into());
    let offline = desk.env["KASSIBER_OFFLINE_PREFERENCE_FILE"].clone();
    fs::write(&offline, "{\"schema_version\": 1, \"enabled\": false}\n").unwrap();
    let base = desk.base_path().join("demo");
    let output = desk
        .raw_command(&[
            "--machine",
            "projects",
            "demo",
            "--path",
            base.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let envelope = kassiber_pytext::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(kind_of(&envelope), "projects.demo");
    let data = path(&envelope, &["data"]).unwrap().clone();
    desk.root = PathBuf::from(text(path(&data, &["data_root"]).unwrap()).unwrap());
    (desk, data)
}

#[test]
fn the_demo_book_opens_with_nothing_to_contact() {
    let (desk, data) = demo_desk();

    // Written under --path, indexed, the user's preferences copied in.
    assert!(desk.root.starts_with(desk.base_path().join("demo")));
    assert!(path(&data, &["index", "snapshot_id"])
        .and_then(text)
        .is_some());
    assert_eq!(
        path(&data, &["preferences_seeded", "offline_mode"]),
        Some(&Value::Bool(true))
    );
    let state = desk.root.parent().unwrap();
    assert!(state.join("config/demo.json").is_file());
    assert!(state.join("config/settings.json").is_file());

    let conn = desk.connection();
    let count = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
    assert_eq!(count("SELECT count(*) FROM backends"), 0);
    assert!(
        count("SELECT count(*) FROM chain_index_nodes") > 0,
        "index built"
    );
    let tx: String = conn
        .query_row(
            "SELECT t.id FROM transactions t JOIN wallets w ON w.id = t.wallet_id
             WHERE w.label = 'Regtest Treasury' AND t.raw_json LIKE '%\"vin\"%'
             ORDER BY t.occurred_at LIMIT 1",
            [],
            |r| r.get(0),
        )
        .unwrap();
    drop(conn);

    // No backend is listed, and no built-in default was seeded on open.
    let backends = desk.run(&["backends", "list"], b"");
    let backends = kassiber_pytext::loads(&String::from_utf8(backends.stdout).unwrap()).unwrap();
    assert!(list(&backends, &["data"]).is_empty(), "{backends:?}");

    // The desktop's automatic lookup from the wallet's backend has none.
    let graph = desk.ask(
        "ui.transactions.graph",
        &format!(r#"{{"transaction": "{tx}", "lookup": "wallet_backend"}}"#),
    );
    assert_eq!(kind_of(&graph), "ui.transactions.graph", "{graph:?}");
    let codes: Vec<&str> = list(&graph, &["data", "warnings"])
        .iter()
        .filter_map(|w| path(w, &["code"]).and_then(text))
        .collect();
    assert!(
        codes.contains(&"wallet_backend_lookup_unavailable"),
        "{codes:?}"
    );

    // An explicit sync fails closed before any transport.
    let sync = desk.ask("ui.wallets.sync", r#"{"wallet": "Regtest Treasury"}"#);
    let results = list(&sync, &["data", "results"]);
    assert_eq!(results.len(), 1, "{sync:?}");
    assert_eq!(
        path(&results[0], &["code"]).and_then(text),
        Some("backend_not_configured")
    );
}

#[test]
fn a_new_demo_replaces_the_old_and_leaves_preferences_alone() {
    let (desk, first) = demo_desk();
    let offline = desk.env["KASSIBER_OFFLINE_PREFERENCE_FILE"].clone();
    let before = fs::read(&offline).unwrap();
    // Turning offline mode on inside the demo writes the demo's own file.
    let demo_offline = PathBuf::from(
        text(
            path(
                &first,
                &["preference_environment", "KASSIBER_OFFLINE_PREFERENCE_FILE"],
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let output = desk
        .raw_command(&[
            "--machine",
            "--data-root",
            desk.root.to_str().unwrap(),
            "offline",
            "on",
        ])
        .env("KASSIBER_OFFLINE_PREFERENCE_FILE", &demo_offline)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert_eq!(
        fs::read(&offline).unwrap(),
        before,
        "the user's preference is untouched"
    );

    // The wallet-backend lookup preference is the demo's own as well: the
    // desktop and the demo daemon read the file the demo names.
    let state = desk.root.parent().unwrap().to_path_buf();
    let demo_lookup = state.join("config/wallet-backend-lookup.json");
    assert_eq!(
        path(
            &first,
            &[
                "preference_environment",
                "KASSIBER_WALLET_BACKEND_LOOKUP_FILE"
            ]
        )
        .and_then(text),
        Some(demo_lookup.to_str().unwrap())
    );
    assert_eq!(
        path(
            &first,
            &["mcp_server", "env", "KASSIBER_WALLET_BACKEND_LOOKUP_FILE"]
        )
        .and_then(text),
        Some(demo_lookup.to_str().unwrap())
    );
    assert_eq!(
        path(&first, &["preferences_seeded", "wallet_backend_lookup"]),
        Some(&Value::Bool(false))
    );
    // The user's choice (off) is copied into the next demo, and the demo's
    // copy is what a change inside it writes.
    let lookup = desk.env["KASSIBER_WALLET_BACKEND_LOOKUP_FILE"].clone();
    let off = "{\"schema_version\": 1, \"enabled\": false}\n";
    fs::write(&lookup, off).unwrap();

    let base = desk.base_path().join("demo");
    let output = desk
        .raw_command(&[
            "--machine",
            "projects",
            "demo",
            "--path",
            base.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let second = kassiber_pytext::loads(&String::from_utf8(output.stdout).unwrap()).unwrap();
    assert_eq!(
        path(
            &second,
            &["data", "preferences_seeded", "wallet_backend_lookup"]
        ),
        Some(&Value::Bool(true))
    );
    let second_lookup = PathBuf::from(
        text(
            path(
                &second,
                &[
                    "data",
                    "preference_environment",
                    "KASSIBER_WALLET_BACKEND_LOOKUP_FILE",
                ],
            )
            .unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(fs::read_to_string(&second_lookup).unwrap(), off);
    fs::write(
        &second_lookup,
        "{\"schema_version\": 1, \"enabled\": true}\n",
    )
    .unwrap();
    assert_eq!(
        fs::read_to_string(&lookup).unwrap(),
        off,
        "the user's stays off"
    );
    assert_eq!(
        path(&second, &["data", "removed_copies"]),
        Some(&Value::Int(1.into()))
    );
    assert!(!desk.root.exists(), "the earlier copy is gone");
    assert_eq!(fs::read_dir(&base).unwrap().count(), 1);

    // --data-root selects an existing book; projects demo refuses it.
    let output = desk
        .raw_command(&[
            "--machine",
            "--data-root",
            "/nonexistent",
            "projects",
            "demo",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("invalid_flag_combination"));
}
