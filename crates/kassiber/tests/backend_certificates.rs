//! Confirmed trust-on-first-use for a backend's self-signed certificate, end
//! to end through the CLI and the daemon against an owned loopback TLS server
//! (docs/reference/backends.md#self-signed-certificates).

#![cfg(feature = "test-harness")]

mod support;

use kassiber_net::test_support::{fixtures, FakeTls};
use kassiber_pyjson::Value;
use support::{kind_of, path, text, Desk};

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

    // Trust needs the confirmed fingerprint.
    let refused = desk.ask("ui.backends.certificate.trust", r#"{"name": "home"}"#);
    assert_eq!(code(&refused), "validation");
    let trusted = desk.ask(
        "ui.backends.certificate.trust",
        &format!(r#"{{"name": "home", "fingerprint": "{FINGERPRINT}"}}"#),
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
fn offline_mode_refuses_the_fetch_before_any_connection() {
    let server = server();
    let desk = desk(&server);
    assert_eq!(cli(&desk, &["offline", "on"]).0, 0);
    let (exit, envelope) = cli(&desk, &["backends", "trust-certificate", "home"]);
    assert_ne!(exit, 0);
    assert_eq!(code(&envelope), "network_egress_disabled");
    assert_eq!(server.connections(), 0);
}
