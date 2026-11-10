use super::*;
use std::net::TcpListener;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::thread;

fn request(host: &str) -> Request {
    Request::new(
        Subsystem::Sync,
        Endpoint::new(host, 50001).unwrap(),
        TlsTrust::Plaintext,
    )
}

fn gate(offline: Offline, observer: Observer) -> Gate {
    Gate {
        offline: Arc::new(move || offline),
        ledger: Ledger::default(),
        attempts: Ledger::default(),
        observer,
        cln_fixture: None,
        broker_fixture: None,
    }
}

fn authorize(gate: &Gate, request: Request) -> Route {
    let proof = Consent::explicit_trigger(&request);
    gate.authorize(request, Some(proof)).unwrap()
}

#[test]
fn offline_missing_consent_and_unavailable_refuse_without_attempts() {
    for (offline, expected) in [
        (
            Offline::Offline,
            "Outbound requests are disabled: Kassiber is in offline mode",
        ),
        (
            Offline::Unavailable,
            "Outbound requests are disabled: Kassiber is in offline mode",
        ),
    ] {
        let gate = gate(offline, Observer::Strict);
        let req = request("127.0.0.1");
        let proof = Consent::explicit_trigger(&req);
        assert_eq!(
            gate.authorize(req, Some(proof)).err().unwrap().to_string(),
            expected
        );
        assert!(gate.attempts().records.is_empty());
        assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    }
    let gate = gate(Offline::Online, Observer::Strict);
    assert!(matches!(
        gate.authorize(request("127.0.0.1"), None),
        Err(Error::ConsentMissing)
    ));
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn consent_is_bound_to_host_port_proxy_trust_subsystem_and_local_classification() {
    let gate = gate(Offline::Online, Observer::Strict);
    let original = request("node.invalid");
    let proof = Consent::narrowly_scoped(&original);
    let mut cases = vec![request("other.invalid")];
    let mut port = original.clone();
    port.endpoint.port += 1;
    cases.push(port);
    let mut subsystem = original.clone();
    subsystem.subsystem = Subsystem::Pricing;
    cases.push(subsystem);
    let mut tls = original.clone();
    tls.tls = TlsTrust::BundledRoots;
    cases.push(tls);
    cases.push(original.clone().with_proxy(
        Proxy::new("socks5h", Endpoint::new("127.0.0.1", 9050).unwrap(), None).unwrap(),
    ));
    cases.push(original.clone().with_action_binding([1; 32]));
    for changed in cases {
        assert!(matches!(
            gate.authorize(changed, Some(proof.clone())),
            Err(Error::ConsentScope)
        ));
    }
    let local = Request::new(
        Subsystem::Ai,
        Endpoint::new("127.0.0.1", 1234).unwrap(),
        TlsTrust::Plaintext,
    );
    assert!(matches!(
        gate.authorize(
            local.clone().on_device_ai(),
            Some(Consent::explicit_trigger(&local))
        ),
        Err(Error::ConsentScope)
    ));
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn action_bound_consent_cannot_be_reused_or_stripped() {
    let gate = gate(Offline::Online, Observer::Strict);
    let original = request("node.invalid").with_action_binding([1; 32]);
    let proof = Consent::explicit_trigger(&original);
    for changed in [
        request("node.invalid"),
        request("node.invalid").with_action_binding([2; 32]),
    ] {
        assert!(matches!(
            gate.authorize(changed, Some(proof.clone())),
            Err(Error::ConsentScope)
        ));
    }
    assert!(gate.authorize(original, Some(proof)).is_ok());
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn onion_and_unsupported_proxies_fail_closed() {
    let gate = gate(Offline::Online, Observer::Strict);
    let req = request("secret.onion.");
    assert!(matches!(
        gate.authorize(req.clone(), Some(Consent::explicit_trigger(&req))),
        Err(Error::OnionProxyRequired)
    ));
    assert!(matches!(
        Proxy::new("https", Endpoint::new("127.0.0.1", 443).unwrap(), None),
        Err(Error::HttpsProxyUnsupported)
    ));
    assert!(Proxy::new("http", Endpoint::new("127.0.0.1", 8080).unwrap(), None).is_ok());
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn offline_allows_only_explicitly_local_loopback_ai() {
    let gate = gate(Offline::Offline, Observer::Strict);
    let local = Request::new(
        Subsystem::Ai,
        Endpoint::new("localhost", 1234).unwrap(),
        TlsTrust::Plaintext,
    )
    .on_device_ai();
    assert!(gate
        .authorize(local.clone(), Some(Consent::explicit_trigger(&local)))
        .is_ok());
    for req in [
        request("localhost").on_device_ai(),
        request("example.invalid").on_device_ai(),
    ] {
        assert!(matches!(
            gate.authorize(req.clone(), Some(Consent::explicit_trigger(&req))),
            Err(Error::InvalidLocalProvider)
        ));
    }
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn gate_rechecks_offline_and_revocation_before_connect() {
    let offline = Arc::new(AtomicBool::new(false));
    let read = offline.clone();
    let mut gate = gate(Offline::Online, Observer::Strict);
    gate.offline = Arc::new(move || {
        if read.load(Ordering::SeqCst) {
            Offline::Offline
        } else {
            Offline::Online
        }
    });
    let route = authorize(&gate, request("127.0.0.1"));
    offline.store(true, Ordering::SeqCst);
    assert!(matches!(
        route.connect(Duration::from_secs(1)),
        Err(Error::Offline)
    ));
    offline.store(false, Ordering::SeqCst);
    let req = request("127.0.0.1");
    let proof = Consent::explicit_trigger(&req);
    let route = gate.authorize(req, Some(proof.clone())).unwrap();
    proof.revoke();
    assert!(matches!(
        route.connect(Duration::from_secs(1)),
        Err(Error::ConsentRevoked)
    ));
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn observer_records_dns_connect_and_strict_loopback_before_refusal() {
    for (host, operation) in [
        ("example.invalid", Operation::Dns),
        ("192.0.2.1", Operation::Connect),
        ("127.0.0.1", Operation::Connect),
        ("::1", Operation::Connect),
        ("localhost", Operation::Connect),
    ] {
        let gate = gate(Offline::Online, Observer::Strict);
        let route = authorize(&gate, request(host));
        let trip = catch_unwind(AssertUnwindSafe(|| route.connect(Duration::from_secs(1))));
        assert!(trip.err().unwrap().is::<TestEgressBlocked>());
        let attempts = gate.attempts();
        assert_eq!(attempts.records.len(), 1);
        assert_eq!(attempts.records[0].host, host);
        assert_eq!(attempts.records[0].operation, operation);
        assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    }
}

#[test]
fn startup_and_configuration_reads_have_zero_attempts() {
    let gate = gate(Offline::Online, Observer::Strict);
    let _route = authorize(&gate, request("localhost"));
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
}

#[test]
fn ledger_is_bounded_redacted_and_reports_gaps() {
    for host in [
        "https://user:secret@example.invalid/path?token=secret",
        "host/path",
        "host?secret",
        "host#secret",
        "user@host",
        "host\nAuthorization:secret",
        "host%25zone",
        "\u{2603}.invalid",
    ] {
        assert!(Endpoint::new(host, 443).is_err());
    }
    let proxy = Proxy::new(
        "socks5h",
        Endpoint::new("localhost", 9050).unwrap(),
        Some(("secret-user".into(), "secret-password".into())),
    )
    .unwrap();
    let req = request("NODE.INVALID.").with_proxy(proxy);
    let ledger = Ledger::with_limits(2, MAX_BYTES);
    for _ in 0..3 {
        ledger.record(&req, &req.endpoint, Operation::Write, 12);
    }
    let snap = ledger.snapshot(0, 2000);
    assert_eq!(snap.last_id, 3);
    assert!(snap.gap);
    assert_eq!(snap.records.len(), 2);
    assert_eq!(snap.records[0].id, 2);
    assert_eq!(snap.records[0].host, "node.invalid");
    assert_eq!(snap.records[0].bytes_out, 12);
    assert!(snap.records[0].via_proxy);
    assert!(!format!("{snap:?} {req:?}").contains("secret"));
    assert!(ledger.snapshot(2, 0).records.is_empty());
    assert!(!ledger.snapshot(2, 1).gap);
    let tiny = Ledger::with_limits(10, 1);
    tiny.record(&req, &req.endpoint, Operation::Write, 99);
    assert_eq!(tiny.snapshot(0, 2000).buffer_bytes, 0);
    assert!(tiny.snapshot(0, 2000).gap);
    let full = Ledger::default();
    for _ in 0..MAX_RECORDS + 1 {
        full.record(&req, &req.endpoint, Operation::Connect, 0);
    }
    let snap = full.snapshot(0, usize::MAX);
    assert_eq!(snap.records.len(), 2000);
    assert!(snap.buffer_bytes <= MAX_BYTES);
    assert!(snap.gap);
}

#[test]
fn socks5_remote_dns_authentication_and_byte_accounting_use_owned_fake() {
    // The test deliberately owns this listener; default observer permits its
    // loopback endpoint. The .onion target must never reach the system resolver.
    let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let address = server.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut socket, _) = server.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut hello = [0; 4];
        socket.read_exact(&mut hello).unwrap();
        assert_eq!(hello, [5, 2, 0, 2]);
        socket.write_all(&[5, 2]).unwrap();
        let mut auth = [0; 6];
        socket.read_exact(&mut auth).unwrap();
        assert_eq!(&auth, &[1, 1, b'u', 2, b'p', b'w']);
        socket.write_all(&[1, 0]).unwrap();
        let mut header = [0; 5];
        socket.read_exact(&mut header).unwrap();
        assert_eq!(header, [5, 1, 0, 3, 12]);
        let mut target = [0; 13]; // 12-byte hostname and first port byte
        socket.read_exact(&mut target).unwrap();
        assert_eq!(&target[..12], b"hidden.onion");
        let mut port_low = [0];
        socket.read_exact(&mut port_low).unwrap();
        assert_eq!(u16::from_be_bytes([target[12], port_low[0]]), 50001);
        socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
        let mut payload = [0; 4];
        socket.read_exact(&mut payload).unwrap();
        assert_eq!(&payload, b"ping");
        socket.write_all(b"pong").unwrap();
    });
    let gate = gate(Offline::Online, Observer::LoopbackAllowed);
    let proxy = Proxy::new(
        "socks5h",
        Endpoint::new("127.0.0.1", address.port()).unwrap(),
        Some(("u".into(), "pw".into())),
    )
    .unwrap();
    let mut connection = authorize(&gate, request("hidden.onion").with_proxy(proxy))
        .connect(Duration::from_secs(3))
        .unwrap();
    connection.write_all(b"ping").unwrap();
    let mut reply = [0; 4];
    connection.read_exact(&mut reply).unwrap();
    assert_eq!(&reply, b"pong");
    handle.join().unwrap();
    assert!(gate
        .attempts()
        .records
        .iter()
        .all(|r| r.host == "127.0.0.1" && r.operation != Operation::Dns));
    let snap = gate.ledger().snapshot(0, 2000);
    assert_eq!(
        snap.records.iter().map(|r| r.bytes_out).sum::<u64>(),
        4 + 6 + 19 + 4
    );
    assert_eq!(snap.records.last().unwrap().host, "hidden.onion");
}

#[test]
fn existing_stream_stops_on_offline_and_revocation() {
    for revoke in [false, true] {
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let offline = Arc::new(AtomicBool::new(false));
        let read = offline.clone();
        let mut gate = gate(Offline::Online, Observer::LoopbackAllowed);
        gate.offline = Arc::new(move || {
            if read.load(Ordering::SeqCst) {
                Offline::Offline
            } else {
                Offline::Online
            }
        });
        let req = Request::new(
            Subsystem::Sync,
            Endpoint::new("127.0.0.1", port).unwrap(),
            TlsTrust::Plaintext,
        );
        let proof = Consent::explicit_trigger(&req);
        let mut connection = gate
            .authorize(req, Some(proof.clone()))
            .unwrap()
            .connect(Duration::from_secs(1))
            .unwrap();
        let (mut peer, _) = server.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        connection.write_all(b"a").unwrap();
        if revoke {
            proof.revoke();
        } else {
            offline.store(true, Ordering::SeqCst);
        }
        assert!(connection.write_all(b"b").is_err());
        drop(connection);
        let mut payload = Vec::new();
        peer.read_to_end(&mut payload).unwrap();
        assert_eq!(payload, b"a");
        assert_eq!(
            gate.ledger()
                .snapshot(0, 2000)
                .records
                .iter()
                .map(|r| r.bytes_out)
                .sum::<u64>(),
            1
        );
    }
}

#[test]
fn socks_rechecks_consent_before_revealing_credentials() {
    let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = server.local_addr().unwrap().port();
    let gate = gate(Offline::Online, Observer::LoopbackAllowed);
    let req = request("hidden.onion").with_proxy(
        Proxy::new(
            "socks5h",
            Endpoint::new("127.0.0.1", port).unwrap(),
            Some(("private-user".into(), "private-password".into())),
        )
        .unwrap(),
    );
    let proof = Consent::explicit_trigger(&req);
    let revoke = proof.clone();
    let handle = thread::spawn(move || {
        let (mut socket, _) = server.accept().unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let mut greeting = [0; 4];
        socket.read_exact(&mut greeting).unwrap();
        assert_eq!(greeting, [5, 2, 0, 2]);
        revoke.revoke();
        socket.write_all(&[5, 2]).unwrap();
        let mut remaining = Vec::new();
        socket.read_to_end(&mut remaining).unwrap();
        assert!(remaining.is_empty());
    });
    assert!(matches!(
        gate.authorize(req, Some(proof))
            .unwrap()
            .connect(Duration::from_secs(2)),
        Err(Error::ConsentRevoked)
    ));
    handle.join().unwrap();
    assert_eq!(
        gate.ledger()
            .snapshot(0, 2000)
            .records
            .iter()
            .map(|r| r.bytes_out)
            .sum::<u64>(),
        4
    );
}

#[test]
fn socks_refuses_malformed_or_unoffered_authentication() {
    for reply in [[4, 0], [5, 2], [5, 255]] {
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            let (mut socket, _) = server.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut greeting = [0; 3];
            socket.read_exact(&mut greeting).unwrap();
            assert_eq!(greeting, [5, 1, 0]);
            socket.write_all(&reply).unwrap();
        });
        let gate = gate(Offline::Online, Observer::LoopbackAllowed);
        let req = request("node.invalid").with_proxy(
            Proxy::new("socks5h", Endpoint::new("127.0.0.1", port).unwrap(), None).unwrap(),
        );
        assert!(matches!(
            authorize(&gate, req).connect(Duration::from_secs(2)),
            Err(Error::SocksProtocol)
        ));
        handle.join().unwrap();
    }
}

#[test]
fn raw_http_proxy_is_refused_before_any_dns_or_socket_even_with_tls() {
    let gate = gate(Offline::Online, Observer::Strict);
    for trust in [
        TlsTrust::Plaintext,
        TlsTrust::BundledRoots,
        TlsTrust::ExplicitInsecure,
    ] {
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("target.invalid", 443).unwrap(),
            trust,
        )
        .with_proxy(
            Proxy::new("http", Endpoint::new("proxy.invalid", 8080).unwrap(), None).unwrap(),
        );
        assert!(matches!(
            authorize(&gate, request).connect(Duration::from_secs(1)),
            Err(Error::ProxyUnsupported)
        ));
    }
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn insecure_tls_is_backend_only_before_any_io() {
    let gate = gate(Offline::Online, Observer::Strict);
    for subsystem in [Subsystem::Pricing, Subsystem::Ai, Subsystem::Update] {
        let req = Request::new(
            subsystem,
            Endpoint::new("localhost", 443).unwrap(),
            TlsTrust::ExplicitInsecure,
        );
        assert!(matches!(
            gate.authorize(req.clone(), Some(Consent::explicit_trigger(&req))),
            Err(Error::TrustInvalid)
        ));
    }
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn cached_offline_keeps_revocation_and_generation_checks_current() {
    use std::sync::atomic::AtomicU64;
    let generation = Arc::new(AtomicU64::new(1));
    let revision = generation.clone();
    let state = generation.clone();
    let cache = OfflineCache::new(
        move || {
            if state.load(Ordering::SeqCst) == 1 {
                Offline::Online
            } else {
                Offline::Offline
            }
        },
        move || Some(revision.load(Ordering::SeqCst)),
    );
    let gate = Gate::with_offline_cache(cache, Ledger::default());
    let req = request("localhost");
    let proof = Consent::explicit_trigger(&req);
    let route = gate.authorize(req, Some(proof.clone())).unwrap();
    route.check().unwrap();
    proof.revoke();
    assert!(matches!(route.check(), Err(Error::ConsentRevoked)));
    let route = authorize(&gate, request("localhost"));
    generation.store(2, Ordering::SeqCst);
    assert!(matches!(route.check(), Err(Error::Offline)));
    assert!(gate.attempts().records.is_empty());
}
