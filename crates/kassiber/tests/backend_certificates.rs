//! Confirmed trust-on-first-use for a backend's self-signed certificate, end
//! to end through the CLI and the daemon against an owned loopback TLS server
//! (docs/reference/backends.md#self-signed-certificates).

#![cfg(feature = "test-harness")]

mod support;

use kassiber_net::test_support::{fixtures, FakeTls};
use kassiber_pyjson::Value;
use support::{kind_of, path, text, Daemon, Desk};

const FINGERPRINT: &str = "81:4C:F5:98:8C:BF:B3:07:03:EA:D7:34:00:E1:33:39:A2:A4:53:E6:CE:4E:5F:FF:68:DD:8D:9C:5B:17:89:6A";
const OTHER: &str = "01:F4:A0:43:BF:C6:EC:5F:FD:F3:BD:53:2D:B6:45:B8:A9:79:36:70:93:1B:07:96:90:3F:F0:C4:72:1C:B3:E8";

fn server() -> FakeTls {
    FakeTls::start(fixtures::SELF_SIGNED, |_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    })
    .unwrap()
}

/// A book with one Esplora backend at the fake, egress allowed to loopback.
fn desk(server: &FakeTls) -> Desk {
    let mut desk = Desk::new("backend_certificates");
    // The product switch blocks Sync routes; the test guard still refuses
    // everything but loopback.
    desk.env.remove("KASSIBER_NO_EGRESS");
    desk.setup_cli(&[
        "backends",
        "create",
        "home",
        "--kind",
        "esplora",
        "--url",
        &format!("https://localhost:{}/api", server.port()),
        "--insecure",
        "1",
    ]);
    desk
}

/// Exit code and the parsed envelope of one CLI run.
fn cli(desk: &Desk, args: &[&str]) -> (i32, Value) {
    let output = desk.run(args, b"");
    let stdout = String::from_utf8(output.stdout).unwrap();
    (
        output.status.code().unwrap(),
        kassiber_pytext::loads(stdout.trim()).unwrap_or_else(|_| panic!("{args:?}: {stdout}")),
    )
}

fn code(envelope: &Value) -> &str {
    path(envelope, &["error", "code"])
        .and_then(text)
        .unwrap_or("")
}

fn field<'a>(value: &'a Value, keys: &[&str]) -> &'a str {
    path(value, keys).and_then(text).unwrap_or("")
}

#[test]
fn cli_shows_the_fingerprint_then_pins_only_the_confirmed_certificate() {
    let server = server();
    let desk = desk(&server);
    let pins = desk.root.join("certificates");

    // Reads, status and listings never contact the server.
    for args in [
        &["backends", "list"][..],
        &["backends", "get", "home"],
        &["status"],
    ] {
        assert_eq!(cli(&desk, args).0, 0, "{args:?}");
    }
    assert_eq!(
        server.connections(),
        0,
        "no traffic before the explicit action"
    );

    // Without a fingerprint: show it, trust nothing, exit non-zero.
    let (exit, envelope) = cli(&desk, &["backends", "trust-certificate", "home"]);
    assert_ne!(exit, 0);
    assert_eq!(code(&envelope), "confirmation_required");
    assert_eq!(
        field(&envelope, &["error", "details", "fingerprint_sha256"]),
        FINGERPRINT
    );
    assert_eq!(field(&envelope, &["error", "details", "host"]), "localhost");
    assert!(field(&envelope, &["error", "message"]).contains(FINGERPRINT));
    assert!(field(&envelope, &["error", "hint"]).contains("--fingerprint"));
    assert_eq!(server.connections(), 1);
    assert!(
        server.requests().is_empty(),
        "a fetch sends no application data"
    );
    assert!(!pins.exists());

    // A fingerprint the server does not present writes nothing.
    let (exit, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            OTHER,
        ],
    );
    assert_ne!(exit, 0);
    assert_eq!(code(&envelope), "backend_certificate_mismatch");
    assert!(!pins.exists());
    let (_, backend) = cli(&desk, &["backends", "get", "home"]);
    assert_eq!(
        path(&backend, &["data", "has_certificate"]),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        path(&backend, &["data", "insecure"]),
        Some(&Value::Bool(true))
    );

    // The confirmed one (any case, no colons) is pinned owner-only.
    let lower = FINGERPRINT.replace(':', "").to_lowercase();
    let (exit, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            &lower,
        ],
    );
    assert_eq!(exit, 0, "{envelope:?}");
    assert_eq!(
        path(&envelope, &["data", "trusted"]),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        path(&envelope, &["data", "clears_insecure"]),
        Some(&Value::Bool(true))
    );
    let pin = std::path::PathBuf::from(field(&envelope, &["data", "certificate"]));
    assert!(pin.starts_with(&pins), "{}", pin.display());
    assert_eq!(
        std::fs::read(&pin).unwrap(),
        {
            let mut leaf = String::from_utf8(fixtures::SELF_SIGNED.0.to_vec()).unwrap();
            leaf.retain(|c| c != '\r');
            leaf.into_bytes()
        },
        "the pin is the server's leaf"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&pin).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
    // `backends get` shows presence, never the path; a pin replaces insecure.
    let (_, backend) = cli(&desk, &["backends", "get", "home"]);
    assert_eq!(
        path(&backend, &["data", "has_certificate"]),
        Some(&Value::Bool(true))
    );
    assert!(
        path(&backend, &["data", "insecure"]).is_none(),
        "{backend:?}"
    );

    // Pinned: the server's certificate now verifies, so nothing is left to trust.
    let (exit, envelope) = cli(&desk, &["backends", "trust-certificate", "home"]);
    assert_ne!(exit, 0);
    assert_eq!(code(&envelope), "backend_certificate_already_trusted");
    assert!(server.requests().is_empty());
}

#[test]
fn daemon_fetch_and_trust_and_a_changed_certificate_is_never_repinned_alone() {
    let server = server();
    let desk = desk(&server);
    // Opening settings reads backend rows only.
    for kind in ["ui.backends.list", "ui.backends.settings.list", "status"] {
        assert_ne!(kind_of(&desk.ask(kind, "{}")), "error", "{kind}");
    }
    assert_eq!(
        server.connections(),
        0,
        "no traffic before the explicit action"
    );

    let fetched = desk.ask("ui.backends.certificate.fetch", r#"{"name": "home"}"#);
    assert_eq!(
        kind_of(&fetched),
        "ui.backends.certificate.fetch",
        "{fetched:?}"
    );
    assert_eq!(
        field(&fetched, &["data", "fingerprint_sha256"]),
        FINGERPRINT
    );
    assert_eq!(field(&fetched, &["data", "subject"]), "localhost");
    assert!(path(&fetched, &["data", "url"]).is_none());
    assert!(path(&fetched, &["data", "pem"]).is_none());

    let revision = field(&fetched, &["data", "revision"]).to_owned();
    assert_eq!(revision.len(), 64, "{fetched:?}");

    // Trust needs the confirmed fingerprint and the reviewed revision.
    let refused = desk.ask("ui.backends.certificate.trust", r#"{"name": "home"}"#);
    assert_eq!(code(&refused), "validation");
    let refused = desk.ask(
        "ui.backends.certificate.trust",
        &format!(r#"{{"name": "home", "fingerprint": "{FINGERPRINT}"}}"#),
    );
    assert_eq!(code(&refused), "validation");
    let trusted = desk.ask(
        "ui.backends.certificate.trust",
        &format!(r#"{{"name": "home", "fingerprint": "{FINGERPRINT}", "revision": "{revision}"}}"#),
    );
    assert_eq!(
        kind_of(&trusted),
        "ui.backends.certificate.trust",
        "{trusted:?}"
    );

    let pin = field(&trusted, &["data", "certificate"]).to_owned();
    let pinned = std::fs::read(&pin).unwrap();

    // The server now presents another certificate: a fetch reports what a
    // trust would replace, and nothing changes until the user confirms it.
    server.set_identity(fixtures::OTHER_SELF_SIGNED);
    let fetched = desk.ask("ui.backends.certificate.fetch", r#"{"name": "home"}"#);
    assert_eq!(field(&fetched, &["data", "fingerprint_sha256"]), OTHER);
    assert_eq!(
        field(&fetched, &["data", "replaces_fingerprint_sha256"]),
        FINGERPRINT
    );
    assert_eq!(std::fs::read(&pin).unwrap(), pinned, "the pin is unchanged");
    let (_, backend) = cli(&desk, &["backends", "get", "home"]);
    assert_eq!(
        path(&backend, &["data", "has_certificate"]),
        Some(&Value::Bool(true))
    );
    assert!(
        server.requests().is_empty(),
        "no application data was ever sent"
    );
}

#[test]
fn a_backend_changed_since_review_is_refused_before_connecting() {
    let server = server();
    let desk = desk(&server);
    let fetched = desk.ask("ui.backends.certificate.fetch", r#"{"name": "home"}"#);
    let revision = field(&fetched, &["data", "revision"]).to_owned();
    assert_eq!(server.connections(), 1);

    // The backend now goes through a proxy the user never reviewed; the
    // proxy port is never reached.
    let proxy = crate::server();
    let proxy_url = format!("socks5h://127.0.0.1:{}", proxy.port());
    desk.setup_cli(&["backends", "update", "home", "--tor-proxy", &proxy_url]);
    let refused = desk.ask(
        "ui.backends.certificate.trust",
        &format!(r#"{{"name": "home", "fingerprint": "{FINGERPRINT}", "revision": "{revision}"}}"#),
    );
    assert_eq!(
        code(&refused),
        "backend_certificate_config_changed",
        "{refused:?}"
    );
    assert_eq!(
        server.connections(),
        1,
        "the server was not contacted again"
    );
    assert_eq!(proxy.connections(), 0, "nor the unreviewed proxy");
    assert!(pins(&desk).is_empty(), "nothing was written");

    // A fresh review of the backend as it is now carries another revision.
    desk.setup_cli(&["backends", "update", "home", "--clear", "tor-proxy"]);
    let fetched = desk.ask("ui.backends.certificate.fetch", r#"{"name": "home"}"#);
    assert_eq!(
        field(&fetched, &["data", "revision"]),
        revision,
        "the same setup again"
    );
}

#[test]
fn the_daemon_shows_the_pin_in_settings_right_after_trusting() {
    let server = server();
    let desk = desk(&server);
    let mut daemon = Daemon::spawn(&desk, &[]);
    let mut ask = |id: &str, kind: &str, args: &str| {
        daemon.request(kind, id, Some(args));
        kassiber_pytext::loads(&daemon.terminal(id)).unwrap()
    };
    let home = |listed: &Value| match path(listed, &["data", "backends"]) {
        Some(Value::List(backends)) => backends
            .iter()
            .find(|b| path(b, &["name"]).and_then(text) == Some("home"))
            .cloned()
            .unwrap(),
        other => panic!("{other:?}"),
    };
    let before = home(&ask("list-1", "ui.backends.settings.list", "{}"));
    assert!(path(&before, &["certificate"]).is_none(), "{before:?}");
    let fetched = ask(
        "fetch",
        "ui.backends.certificate.fetch",
        r#"{"name": "home"}"#,
    );
    let revision = field(&fetched, &["data", "revision"]).to_owned();
    let trusted = ask(
        "trust",
        "ui.backends.certificate.trust",
        &format!(r#"{{"name": "home", "fingerprint": "{FINGERPRINT}", "revision": "{revision}"}}"#),
    );
    let pin = field(&trusted, &["data", "certificate"]).to_owned();
    assert!(!pin.is_empty(), "{trusted:?}");
    // The same daemon's next settings read, the one the desktop refetches
    // and a later save starts from, shows the pin and verification on.
    let after = home(&ask("list-2", "ui.backends.settings.list", "{}"));
    assert_eq!(field(&after, &["certificate"]), pin, "{after:?}");
    assert!(
        !matches!(path(&after, &["insecure"]), Some(Value::Bool(true))),
        "{after:?}"
    );
    daemon.request("daemon.shutdown", "stop", None);
    daemon.terminal("stop");
    daemon.exit_cleanly();
}

#[test]
fn offline_mode_refuses_the_fetch_before_any_connection() {
    let server = server();
    let desk = desk(&server);
    assert_eq!(cli(&desk, &["offline", "on"]).0, 0);
    let (exit, envelope) = cli(&desk, &["backends", "trust-certificate", "home"]);
    assert_ne!(exit, 0);
    assert_eq!(code(&envelope), "network_egress_disabled");
    assert_eq!(server.connections(), 0);
}

fn pin_dir(desk: &Desk) -> std::path::PathBuf {
    desk.root.join("certificates")
}

fn pins(desk: &Desk) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(pin_dir(desk))
        .map(|entries| {
            entries
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[test]
fn an_environment_override_refuses_before_any_connection() {
    let server = server();
    let mut desk = desk(&server);
    // The override would keep verification off whatever the stored row says.
    // An environment-defined backend needs its kind and URL too; these
    // mirror the stored row.
    for (key, value) in [
        ("KASSIBER_BACKEND_HOME_KIND", "esplora".to_owned()),
        (
            "KASSIBER_BACKEND_HOME_URL",
            format!("https://localhost:{}/api", server.port()),
        ),
        ("KASSIBER_BACKEND_HOME_INSECURE", "1".to_owned()),
    ] {
        desk.env.insert(key.into(), value);
    }
    for args in [
        &["backends", "trust-certificate", "home"][..],
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            FINGERPRINT,
        ],
    ] {
        let (exit, envelope) = cli(&desk, args);
        assert_ne!(exit, 0);
        assert_eq!(
            code(&envelope),
            "backend_certificate_overridden",
            "{envelope:?}"
        );
        let message = field(&envelope, &["error", "message"]);
        assert!(
            message.contains(
                "KASSIBER_BACKEND_HOME_INSECURE, KASSIBER_BACKEND_HOME_KIND, KASSIBER_BACKEND_HOME_URL"
            ) && !message.contains("localhost"),
            "{message}"
        );
        assert!(
            !message.contains("=1") && !message.contains("INSECURE="),
            "{message}"
        );
    }
    let fetched = desk.ask("ui.backends.certificate.fetch", r#"{"name": "home"}"#);
    assert_eq!(code(&fetched), "backend_certificate_overridden");
    assert_eq!(server.connections(), 0);
    assert!(pins(&desk).is_empty());
}

#[test]
fn a_backend_changed_mid_handshake_is_not_trusted_and_keeps_its_pin() {
    let server = server();
    let desk = desk(&server);
    let (exit, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            FINGERPRINT,
        ],
    );
    assert_eq!(exit, 0, "{envelope:?}");
    let first = field(&envelope, &["data", "certificate"]).to_owned();
    assert_eq!(pins(&desk).len(), 1);

    // The server rotates its certificate; while Kassiber checks it, another
    // process points the backend somewhere else.
    server.set_identity(fixtures::OTHER_SELF_SIGNED);
    let database = kassiber_paths::resolve_database_path(&desk.root);
    server.on_accept(move |_| {
        let conn = kassiber_store::encryption::open_container(&database, None).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(10))
            .unwrap();
        conn.execute(
            "UPDATE backends SET url='https://localhost:1/api' WHERE name='home'",
            [],
        )
        .unwrap();
    });
    let (exit, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            OTHER,
        ],
    );
    assert_ne!(exit, 0);
    assert_eq!(
        code(&envelope),
        "backend_certificate_config_changed",
        "{envelope:?}"
    );
    // The new pin was never attached and its file is gone; the old pin and
    // the row's certificate setting are untouched.
    let names = pins(&desk);
    assert_eq!(names.len(), 1, "{names:?}");
    assert!(first.ends_with(&names[0]));
    let conn = desk.connection();
    let config: String = conn
        .query_row(
            "SELECT config_json FROM backends WHERE name='home'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(config.contains(&names[0]), "{config}");
}

#[test]
fn a_replaced_pin_is_removed_only_after_the_new_one_is_committed() {
    let server = server();
    let desk = desk(&server);
    let (_, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            FINGERPRINT,
        ],
    );
    let first = field(&envelope, &["data", "certificate"]).to_owned();
    server.set_identity(fixtures::OTHER_SELF_SIGNED);
    let (exit, envelope) = cli(
        &desk,
        &[
            "backends",
            "trust-certificate",
            "home",
            "--fingerprint",
            OTHER,
        ],
    );
    assert_eq!(exit, 0, "{envelope:?}");
    let second = field(&envelope, &["data", "certificate"]).to_owned();
    assert_ne!(first, second);
    assert!(!std::path::Path::new(&first).exists());
    assert!(std::path::Path::new(&second).exists());
    assert_eq!(pins(&desk).len(), 1);
}

#[test]
fn backends_whose_names_once_shared_a_pin_file_keep_their_own() {
    let one = server();
    let two = FakeTls::start(fixtures::OTHER_SELF_SIGNED, |_| {
        b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
    })
    .unwrap();
    let mut desk = Desk::new("backend_certificates");
    desk.env.remove("KASSIBER_NO_EGRESS");
    let (a, b) = (
        format!("{}4842", "home".repeat(10)),
        format!("{}63481", "home".repeat(10)),
    );
    for (name, port, fingerprint) in [(&a, one.port(), FINGERPRINT), (&b, two.port(), OTHER)] {
        desk.setup_cli(&[
            "backends",
            "create",
            name,
            "--kind",
            "esplora",
            "--url",
            &format!("https://localhost:{port}/api"),
        ]);
        let (exit, envelope) = cli(
            &desk,
            &[
                "backends",
                "trust-certificate",
                name,
                "--fingerprint",
                fingerprint,
            ],
        );
        assert_eq!(exit, 0, "{envelope:?}");
    }
    assert_eq!(pins(&desk).len(), 2);
    // Each pin still holds its own server's certificate.
    for name in [&a, &b] {
        let (_, envelope) = cli(&desk, &["backends", "trust-certificate", name]);
        assert_eq!(
            code(&envelope),
            "backend_certificate_already_trusted",
            "{name}"
        );
    }
}
