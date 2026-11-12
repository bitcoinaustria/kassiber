//! The desktop shell's daemon kinds: workspace reads, onboarding, desktop
//! encryption (`ui.secrets.init`) and the logs and egress snapshots.
//! Replaces the kill-holding tests of `tests/test_rust_workspace_shell.py`
//! and `tests/test_rust_daemon_diagnostics.py`.

use serde_json::{json, Value};

use crate::support::{mask_after, mask_until, Sandbox, Transcript};

pub const NOW: &str = "2026-01-01T00:00:00Z";
pub const KEY: &str = "R5 workspace synthetic '🔑";
/// The project clock `tests/test_rust_project_desktop.py` fixed for every
/// shell module.
pub const PROJECT_NOW: &str = "2030-01-02T03:04:05Z";

/// The shell tests' environment: the strict egress guard, the product
/// switch, and fixed setup, secrets and project clocks.
pub fn shell(sandbox: &mut Sandbox) {
    sandbox
        .unset("KASSIBER_STATE_ROOT")
        .set("KASSIBER_NO_EGRESS", "1")
        .set("KASSIBER_TEST_NO_EGRESS", "strict")
        .set("KASSIBER_RUST_TEST_PROJECT_TIMESTAMP", PROJECT_NOW)
        .set("KASSIBER_RUST_TEST_SETUP_IDS", "sequential")
        .set("KASSIBER_RUST_TEST_SETUP_TIMESTAMP", NOW)
        .set("KASSIBER_RUST_TEST_SECRETS_TIME", NOW);
}

/// `(kind, args)` requests, numbered from 0, `args` always present.
pub fn requests(pairs: &[(&str, Value)]) -> Vec<Value> {
    pairs
        .iter()
        .enumerate()
        .map(|(index, (kind, args))| json!({"request_id": index, "kind": kind, "args": args}))
        .collect()
}

/// A shell session: responses recorded, then the rows it changed.
pub fn session(t: &mut Transcript, sandbox: &Sandbox, pairs: &[(&str, Value)]) -> Vec<Value> {
    let before = sandbox.dump();
    let responses = t.daemon(sandbox, &requests(pairs));
    t.changes(sandbox, &before, "after the session");
    for response in &responses {
        assert_ne!(response["error"]["code"], "not_implemented", "{response}");
    }
    responses
}

/// An initialized empty book, as `kassiber init` leaves it.
pub fn initialized() -> Sandbox {
    let mut sandbox = Sandbox::empty();
    shell(&mut sandbox);
    let out = sandbox.machine(&["init"]);
    assert_eq!(out.code, 0, "{}", out.text());
    sandbox
}

fn vectors() -> Vec<Value> {
    serde_json::from_str(include_str!("../../../workspace/tests/data/vectors.json")).unwrap()
}

const SHOWN: &[&str] = &[
    "empty",
    "unprocessed",
    "current",
    "semantics_built",
    "cycle",
    "ov_journal_rates",
    "ov_canonical_issues",
    "ov_inventory_current",
    "ov_pairs",
    "ov_books_current",
    "ov_generic",
];

fn reads() -> Vec<(&'static str, Value)> {
    vec![
        ("ui.overview.snapshot", json!({})),
        ("ui.overview.snapshot", json!([1])),
        (
            "ui.workspace.overview.snapshot",
            json!({"workspace_id": " w "}),
        ),
        (
            "ui.workspace.overview.snapshot",
            json!({"workspace_id": "missing"}),
        ),
        (
            "ui.workspace.overview.snapshot",
            json!({"workspace_id": "w", "extra": 1, "another": 2}),
        ),
        (
            "ui.workspace.overview.snapshot",
            json!({"workspace_id": " "}),
        ),
        ("ui.workspace.overview.snapshot", json!([])),
        ("ui.workspace.overview.snapshot", json!(null)),
        ("ui.workspace.health", json!({})),
        ("ui.workspace.health", json!("ignored")),
        ("ui.next_actions", json!({})),
        ("ui.review.badges", json!({"ignored": true})),
    ]
}

#[test]
fn test_workspace_reads_match_python_bytes_and_leave_books_unchanged() {
    let cases = vectors();
    let mut t = Transcript::new("shell/workspace_reads");
    for name in SHOWN {
        let case = cases
            .iter()
            .find(|c| c["name"] == *name)
            .unwrap_or_else(|| panic!("no vector {name}"));
        let mut sandbox = Sandbox::book(case["sql"].as_str().unwrap());
        shell(&mut sandbox);
        t.note(name);
        let before = sandbox.dump();
        let responses = session(&mut t, &sandbox, &reads());
        assert_eq!(sandbox.dump(), before, "reads leave {name} unchanged");
        assert_eq!(responses[0]["kind"], "ui.overview.snapshot");
    }
    t.finish();
}

fn onboarding() -> Vec<(&'static str, Value)> {
    let base = |extra: Value| {
        let mut args = json!({"workspace_label": "Main", "profile_label": "Book"});
        for (k, v) in extra.as_object().unwrap() {
            args[k] = v.clone();
        }
        ("ui.onboarding.complete", args)
    };
    vec![
        ("ui.onboarding.complete", json!([1])),
        ("ui.onboarding.complete", json!({})),
        (
            "ui.onboarding.complete",
            json!({"workspace_label": 5, "profile_label": "Book"}),
        ),
        (
            "ui.onboarding.complete",
            json!({"workspace_label": "Main", "profile_label": " "}),
        ),
        base(json!({"tax_long_term_days": "x"})),
        base(json!({"backend_setup_mode": "odd"})),
        base(json!({"backend_setup_mode": "skip", "backend": {}})),
        base(json!({"include_liquid_backends": 1})),
        base(json!({"backend": []})),
        base(json!({"backend": {"name": "x"}})),
        base(json!({"backend": {"name": "x", "kind": "Gopher", "url": "u"}})),
        base(json!({"backend": {}})),
    ]
}

#[test]
fn test_onboarding_matches_python_wire_and_all_tables() {
    let finals = [
        (
            "custom",
            json!({"workspace_label": "Main", "profile_label": "Book",
                "backend": {"name": " Node ", "kind": "ESPLORA", "url": "http://127.0.0.1:9",
                    "chain": "bitcoin", "network": "regtest", "insecure": 0,
                    "certificate": " cert ", "notes": "n"}}),
        ),
        (
            "default",
            json!({"workspace_label": " Main ", "profile_label": "Book", "tax_country": "at",
                "fiat_currency": "eur", "tax_long_term_days": "400"}),
        ),
        (
            "liquid",
            json!({"workspace_label": "Main", "profile_label": "Book",
                "include_liquid_backends": true, "gains_algorithm": "hifo",
                "tax_long_term_days": 7.9}),
        ),
        (
            "skip",
            json!({"workspace_label": "Main", "profile_label": "Book",
                "backend_setup_mode": "skip"}),
        ),
    ];
    let mut t = Transcript::new("shell/onboarding");
    for (mode, last) in finals {
        t.note(mode);
        let sandbox = initialized();
        let mut pairs = onboarding();
        let count = pairs.len();
        pairs.extend([
            ("ui.onboarding.complete", last.clone()),
            ("ui.onboarding.complete", last),
            ("status", json!({})),
            ("ui.profiles.snapshot", json!({})),
            ("ui.workspace.health", json!({})),
        ]);
        let responses = session(&mut t, &sandbox, &pairs);
        assert_eq!(responses[count]["kind"], "ui.onboarding.complete");
        assert_eq!(responses[count + 1]["error"]["code"], "conflict");
    }
    t.finish();
}

/// Files under the sandbox's book layout (not the database or owner
/// markers), with the credential backup's stamp masked.
fn files(sandbox: &Sandbox) -> String {
    fn walk(dir: &std::path::Path, base: &std::path::Path, out: &mut Vec<String>) {
        let mut entries: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, base, out);
                continue;
            }
            let name = path.file_name().unwrap().to_string_lossy();
            if name.contains(".operator-owner") {
                continue;
            }
            let relative = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            let body = if name.contains("sqlite3") {
                "<database>".to_owned()
            } else {
                String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned()
            };
            out.push(format!("{relative}:\n{body}"));
        }
    }
    let mut out = Vec::new();
    for top in ["data", "config"] {
        let dir = sandbox.path(top);
        if dir.exists() {
            walk(&dir, &sandbox.base, &mut out);
        }
    }
    stamped(&out.join("\n"))
}

fn stamped(text: &str) -> String {
    mask_until(text, "pre-credentials-migration-", ".bak", "<ts>")
}

#[test]
fn test_secrets_init_matches_python_and_migrates_dotenv_credentials() {
    let mut t = Transcript::new("shell/secrets_init");
    t.mask(stamped);
    t.mask(|text| mask_after(text, "database_instance_id\" | value=t:\"", 32, "<random>"));
    for existing in [true, false] {
        t.note(&format!("existing={existing}"));
        let mut sandbox = Sandbox::empty();
        shell(&mut sandbox);
        let env_file = sandbox.path("config/backends.env");
        std::fs::create_dir_all(env_file.parent().unwrap()).unwrap();
        std::fs::write(
            &env_file,
            "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc\nKASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332\n\
             KASSIBER_BACKEND_CORE_RPCUSER=alice\nKASSIBER_BACKEND_CORE_RPCPASSWORD=hunter2\n",
        )
        .unwrap();
        if existing {
            let out = sandbox.machine(&["init"]);
            assert_eq!(out.code, 0, "{}", out.text());
        } else {
            std::fs::create_dir_all(&sandbox.root).unwrap();
        }
        let pairs = [
            ("ui.secrets.init", json!([])),
            ("ui.secrets.init", json!({})),
            (
                "ui.secrets.init",
                json!({"auth_response": {"passphrase_secret": "short"}}),
            ),
            (
                "ui.secrets.init",
                json!({"auth_response": {"passphrase_secret": KEY}}),
            ),
            ("status", json!({})),
            (
                "ui.secrets.init",
                json!({"auth_response": {"passphrase_secret": "wrong passphrase!"}}),
            ),
            (
                "ui.secrets.init",
                json!({"auth_response": {"passphrase_secret": KEY}, "migrate_credentials": false}),
            ),
            ("ui.backends.settings.list", json!({})),
        ];
        let responses = t.daemon(&sandbox, &requests(&pairs));
        assert_eq!(responses[3]["data"]["encrypted"], true);
        t.section("files", &sandbox.scrub(&files(&sandbox)));
        sandbox.passphrase = Some(KEY.to_owned());
        t.dump(&sandbox, "after the session");
        // Without a stored backend row the secret stays in the env file.
        let env = std::fs::read_to_string(&env_file).unwrap();
        assert_eq!(env.contains("hunter2"), !existing);
    }
    t.finish();
}

/// Per-process clocks and the byte counts that follow them.
fn normalize(record: &mut Value) {
    const STAMP: &str = "<stamp>";
    let kind = record["kind"].as_str().unwrap_or_default().to_owned();
    let Some(data) = record.get_mut("data").filter(|d| d.is_object()) else {
        return;
    };
    if kind == "ui.logs.snapshot" {
        data["started_at"] = json!(STAMP);
        data["buffer_bytes"] = json!(STAMP);
        for row in data["records"].as_array_mut().unwrap() {
            row["ts"] = json!(STAMP);
            row["file"] = json!(STAMP);
            row["line"] = json!(STAMP);
            if let Some(duration) = row["fields"].get_mut("duration_ms") {
                if !duration.is_null() {
                    duration["value"] = json!(STAMP);
                }
            }
        }
    }
    if kind == "ui.egress.snapshot" {
        data["started_at"] = json!(STAMP);
    }
}

#[test]
fn test_snapshots_match_python_on_plaintext_book() {
    let sandbox = initialized();
    let env_file = sandbox.path("config/backends.env");
    std::fs::create_dir_all(env_file.parent().unwrap()).unwrap();
    // A dotenv backend joins the runtime allowlist; its token never appears.
    std::fs::write(
        &env_file,
        "KASSIBER_BACKEND_NODE_KIND=esplora\nKASSIBER_BACKEND_NODE_URL=HTTPS://Node.Example.:8443/api\n\
         KASSIBER_BACKEND_NODE_TOKEN=synthetic-egress-token\n",
    )
    .unwrap();
    sandbox.sql_raw(
        "INSERT INTO backends(name,kind,url,created_at,updated_at) VALUES('core','bitcoinrpc','http://127.0.0.1:8332','2025','2025');
         INSERT INTO ai_providers(name,base_url,kind,enabled,api_format,omit_store_field,created_at,updated_at) VALUES
         ('local','http://127.0.0.1:11434/v1','ollama',1,'chat',0,'2025','2025'),
         ('remote','https://api.example.com/v1','',1,'chat',0,'2025','2025'),
         ('socket','unix:///tmp/x','x',1,'chat',0,'2025','2025');",
    );
    // Settle compatibility backfills (e.g. provider display names) first.
    sandbox.with_book(true, |_| ());
    let pairs = [
        ("ui.egress.snapshot", json!({})),
        ("ui.logs.snapshot", json!({})),
        ("status", json!({})),
        ("ui.workspace.health", json!({})),
        ("nope", json!({})),
        ("ui.projects.select", json!({})),
        ("ui.logs.snapshot", json!(null)),
        ("ui.logs.snapshot", json!({"after_id": 2})),
        ("ui.logs.snapshot", json!({"after_id": 1, "limit": 1})),
        ("ui.logs.snapshot", json!({"limit": 0})),
        ("ui.logs.snapshot", json!({"limit": 2001})),
        ("ui.logs.snapshot", json!({"after_id": -1})),
        ("ui.logs.snapshot", json!({"after_id": true})),
        ("ui.logs.snapshot", json!({"after_id": "1"})),
        (
            "ui.logs.snapshot",
            serde_json::from_str::<Value>(r#"{"after_id": 1000000000000000000000000000000}"#)
                .unwrap(),
        ),
        (
            "ui.logs.snapshot",
            json!({"zeta": 1, "alpha": 2, "limit": 5}),
        ),
        ("ui.logs.snapshot", json!([1])),
        ("ui.egress.snapshot", json!({"after_id": 0, "limit": 2000})),
        ("ui.egress.snapshot", json!({"limit": 1.5})),
        ("ui.egress.snapshot", json!({"beta": null})),
        ("ui.egress.snapshot", json!("x")),
        ("daemon.lock", json!({})),
        ("ui.egress.snapshot", json!({})),
        ("ui.logs.snapshot", json!({"after_id": 3})),
    ];
    let before = sandbox.dump();
    let mut t = Transcript::new("shell/diagnostics_snapshots");
    let responses = t.daemon_normalized(&sandbox, &requests(&pairs), normalize);
    assert_eq!(sandbox.dump(), before);
    let egress = &responses[0]["data"];
    assert_eq!(egress["records"], json!([]));
    assert_eq!(egress["allowlist_complete"], false);
    assert_eq!(egress["db_header"]["classification"], "plaintext-sqlite");
    assert_eq!(responses[17]["data"]["allowlist_complete"], true);
    assert!(responses[17]["data"]["allowlist"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["label"] == "ai:local"));
    let messages: Vec<&str> = responses[6]["data"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["msg"].as_str().unwrap())
        .collect();
    assert_eq!(messages[..2], ["request started", "request finished"]);
    for response in &responses {
        assert!(!response.to_string().contains("synthetic-egress-token"));
    }
    t.finish();
}
