use super::*;
use kassiber_net::test_support::{fixtures, FakeTls};
use kassiber_net::{Ledger, Offline};
use std::time::Instant;

const SELF_SIGNED_FINGERPRINT: &str = "81:4C:F5:98:8C:BF:B3:07:03:EA:D7:34:00:E1:33:39:A2:A4:53:E6:CE:4E:5F:FF:68:DD:8D:9C:5B:17:89:6A";
const OTHER_FINGERPRINT: &str = "01:F4:A0:43:BF:C6:EC:5F:FD:F3:BD:53:2D:B6:45:B8:A9:79:36:70:93:1B:07:96:90:3F:F0:C4:72:1C:B3:E8";

fn gate() -> Gate {
    Gate::new(|| Offline::Online, Ledger::default())
}

fn http_reply(_: &[u8]) -> Vec<u8> {
    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok".to_vec()
}

fn esplora(port: u16, extra: Value) -> Value {
    let mut config = json!({"name":"Home","kind":"esplora","url":format!("https://localhost:{port}/api"),"timeout":5});
    for (key, value) in extra.as_object().unwrap() {
        config[key] = value.clone();
    }
    config
}

fn probe(config: &Value) -> Result<u16> {
    let backend = Backend::new(config, false)?;
    let consent = Consent::explicit_trigger(backend.request());
    HttpClient::new(backend, gate(), "test")
        .probe(Some(&consent), Instant::now() + Duration::from_secs(5))
        .map(|response| response.status)
}

#[test]
fn fingerprints_normalize_and_reject_malformed_input() {
    assert_eq!(
        normalize_fingerprint(&SELF_SIGNED_FINGERPRINT.replace(':', "").to_lowercase()).unwrap(),
        SELF_SIGNED_FINGERPRINT
    );
    assert_eq!(
        normalize_fingerprint(&SELF_SIGNED_FINGERPRINT.replace(':', " ")).unwrap(),
        SELF_SIGNED_FINGERPRINT
    );
    for bad in [
        "",
        "81:4C",
        &format!("{SELF_SIGNED_FINGERPRINT}:00"),
        &"zz".repeat(32),
    ] {
        assert_eq!(normalize_fingerprint(bad).unwrap_err().code, "validation");
    }
}

#[test]
fn self_signed_server_is_untrusted_until_pinned_and_changed_after() {
    let server = FakeTls::start(fixtures::SELF_SIGNED, http_reply).unwrap();
    let port = server.port();
    // Unpinned: a distinct error the desktop can offer the trust flow for.
    assert_eq!(
        probe(&esplora(port, json!({}))).unwrap_err().code,
        "tls_certificate_untrusted"
    );
    let consent = CertificateConsent::explicit("home");
    let fetched = fetch(&gate(), &consent, &esplora(port, json!({}))).unwrap();
    assert_eq!(fetched.fingerprint(), SELF_SIGNED_FINGERPRINT);
    assert_eq!(fetched.host(), "localhost");
    assert_eq!(fetched.port(), port);
    let payload = fetched.payload();
    assert_eq!(payload["backend"], "home");
    assert_eq!(payload["subject"], "localhost");
    assert_eq!(payload["self_signed"], true);
    assert_eq!(payload["replaces_fingerprint_sha256"], Value::Null);
    assert!(payload.get("url").is_none() && payload.get("pem").is_none());
    // Fetches never send application data.
    assert!(server.requests().is_empty());
    // A wrong confirmation is refused; the right one returns the same PEM.
    let error = confirm(
        &gate(),
        &consent,
        &esplora(port, json!({})),
        OTHER_FINGERPRINT,
    )
    .err()
    .unwrap();
    assert_eq!(error.code, "backend_certificate_mismatch");
    let confirmed = confirm(
        &gate(),
        &consent,
        &esplora(port, json!({})),
        &SELF_SIGNED_FINGERPRINT.to_lowercase(),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pin = dir.path().join("home.pem");
    std::fs::write(&pin, confirmed.pem()).unwrap();
    let pinned = esplora(port, json!({"certificate": pin}));
    assert_eq!(probe(&pinned).unwrap(), 200);
    assert_eq!(server.requests().len(), 1);
    // Pinned and unchanged: nothing to trust again.
    assert_eq!(
        fetch(&gate(), &consent, &pinned).err().unwrap().code,
        "backend_certificate_already_trusted"
    );
    // The server's certificate changes: never accepted, never re-pinned.
    server.set_identity(fixtures::OTHER_SELF_SIGNED);
    let error = probe(&pinned).unwrap_err();
    assert_eq!(error.code, "tls_certificate_changed");
    assert!(!error.retryable);
    let replacement = fetch(&gate(), &consent, &pinned).unwrap();
    assert_eq!(replacement.fingerprint(), OTHER_FINGERPRINT);
    assert_eq!(
        replacement.payload()["replaces_fingerprint_sha256"],
        SELF_SIGNED_FINGERPRINT
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn electrum_ssl_and_insecure_backends_fetch_and_report_what_pinning_changes() {
    let server = FakeTls::start(fixtures::SELF_SIGNED, |_| {
        b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":[\"fake\",\"1.4\"]}\n".to_vec()
    })
    .unwrap();
    let config = json!({"name":"home","kind":"electrum","url":format!("ssl://localhost:{}", server.port()),"insecure":true,"timeout":5});
    let fetched = fetch(&gate(), &CertificateConsent::explicit("home"), &config).unwrap();
    assert_eq!(fetched.fingerprint(), SELF_SIGNED_FINGERPRINT);
    assert_eq!(fetched.payload()["clears_insecure"], true);
    assert!(server.requests().is_empty());
}

#[test]
fn refusals_before_any_connection() {
    let server = FakeTls::start(fixtures::SELF_SIGNED, http_reply).unwrap();
    let port = server.port();
    let consent = CertificateConsent::explicit("home");
    for (config, code) in [
        (
            json!({"name":"home","kind":"electrum","url":format!("tcp://localhost:{port}")}),
            "backend_certificate_plaintext",
        ),
        (
            json!({"name":"home","kind":"esplora","url":format!("http://localhost:{port}")}),
            "backend_certificate_plaintext",
        ),
        (
            json!({"name":"home","kind":"btcpay","url":format!("https://localhost:{port}")}),
            "backend_certificate_unsupported",
        ),
        (
            json!({"name":"home","kind":"lnd","url":format!("https://localhost:{port}")}),
            "backend_certificate_unsupported",
        ),
        // A grant names one backend; it cannot fetch another.
        (
            json!({"name":"other","kind":"esplora","url":format!("https://localhost:{port}")}),
            "interaction_required",
        ),
        (
            json!({"name":"home","kind":"electrum","url":"ssl://abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyzabcd.onion:50002"}),
            "network_proxy_required",
        ),
    ] {
        assert_eq!(fetch(&gate(), &consent, &config).err().unwrap().code, code);
    }
    let offline = Gate::new(|| Offline::Offline, Ledger::default());
    assert_eq!(
        fetch(&offline, &consent, &esplora(port, json!({})))
            .err()
            .unwrap()
            .code,
        "network_egress_disabled"
    );
    assert_eq!(server.connections(), 0);
}

#[test]
fn a_configured_ca_that_covers_the_server_leaves_nothing_to_pin() {
    let server = FakeTls::start(fixtures::CA_SIGNED, http_reply).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let ca = dir.path().join("ca.pem");
    std::fs::write(&ca, fixtures::TEST_CA).unwrap();
    let config = esplora(server.port(), json!({"certificate": ca}));
    assert_eq!(probe(&config).unwrap(), 200);
    assert_eq!(
        fetch(&gate(), &CertificateConsent::explicit("home"), &config)
            .err()
            .unwrap()
            .code,
        "backend_certificate_already_trusted"
    );
    // Reached by IP, the CA-issued leaf (no IP SAN) is not trusted by name,
    // but its exact certificate can still be offered for pinning.
    let config = json!({"name":"home","kind":"esplora","url":format!("https://127.0.0.1:{}/", server.port()),"timeout":5});
    let fetched = fetch(&gate(), &CertificateConsent::explicit("home"), &config).unwrap();
    assert_eq!(fetched.payload()["host"], "127.0.0.1");
}

#[test]
fn only_an_effective_pin_counts_as_trusted() {
    let server = FakeTls::start(fixtures::SELF_SIGNED, http_reply).unwrap();
    let port = server.port();
    let fetched = fetch(
        &gate(),
        &CertificateConsent::explicit("home"),
        &esplora(port, json!({})),
    )
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let pin = dir.path().join("pin.pem");
    std::fs::write(&pin, fetched.pem()).unwrap();
    let other = dir.path().join("other.pem");
    std::fs::write(&other, fixtures::OTHER_SELF_SIGNED.0).unwrap();
    verify_pinned(&esplora(port, json!({"certificate": pin})), &fetched).unwrap();
    for config in [
        // Verification stays off whatever the certificate says.
        esplora(port, json!({"certificate": pin, "insecure": "1"})),
        esplora(port, json!({})),
        esplora(port, json!({"certificate": other})),
        esplora(port, json!({"certificate": dir.path().join("missing.pem")})),
        json!({"name":"home","kind":"esplora","url":format!("http://localhost:{port}/api"),"certificate":pin}),
    ] {
        assert_eq!(
            verify_pinned(&config, &fetched).unwrap_err().code,
            "backend_certificate_not_effective",
            "{config}"
        );
    }
}
