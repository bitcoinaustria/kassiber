use super::*;
use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use std::net::TcpListener;
use std::thread;

const CA: &[u8] = include_bytes!("../tests/fixtures/ca.pem");
const CERT: &[u8] = include_bytes!("../tests/fixtures/server.pem");
const KEY: &[u8] = include_bytes!("../tests/fixtures/server-key.pem");

fn test_gate(offline: impl Fn() -> Offline + Send + Sync + 'static) -> Gate {
    Gate {
        offline: Arc::new(offline),
        ledger: Ledger::default(),
        attempts: Ledger::default(),
        observer: Observer::LoopbackAllowed,
        cln_fixture: None,
        broker_fixture: None,
    }
}

fn route(gate: &Gate, request: Request) -> Route {
    let consent = Consent::explicit_trigger(&request);
    gate.authorize(request, Some(consent)).unwrap()
}

fn listener() -> (TcpListener, u16) {
    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    listener.set_nonblocking(true).unwrap();
    (listener, port)
}

fn accept(listener: &TcpListener) -> TcpStream {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match listener.accept() {
            Ok((socket, _)) => {
                // macOS hands out accepted sockets in the listener's
                // non-blocking mode; Linux does not.
                socket.set_nonblocking(false).unwrap();
                socket
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                socket
                    .set_write_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                return socket;
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(e) => panic!("test accept failed: {e}"),
        }
    }
}

fn head(stream: &mut (impl Read + Write)) -> Vec<u8> {
    let mut data = Vec::new();
    while !data.ends_with(b"\r\n\r\n") {
        assert!(data.len() < 65536);
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        data.push(byte[0]);
    }
    data
}

fn tls_server(socket: TcpStream) -> rustls::StreamOwned<rustls::ServerConnection, TcpStream> {
    tls_server_with(socket, CERT, KEY)
}

fn tls_server_with(
    socket: TcpStream,
    cert: &[u8],
    key: &[u8],
) -> rustls::StreamOwned<rustls::ServerConnection, TcpStream> {
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from_pem_slice(cert).unwrap()],
        PrivateKeyDer::from_pem_slice(key).unwrap(),
    )
    .unwrap();
    rustls::StreamOwned::new(
        rustls::ServerConnection::new(Arc::new(config)).unwrap(),
        socket,
    )
}

#[test]
fn every_adapter_and_subsystem_has_no_attempts_without_consent_or_while_offline() {
    for subsystem in [
        Subsystem::Sync,
        Subsystem::Pricing,
        Subsystem::Ai,
        Subsystem::Update,
    ] {
        for tls in [
            TlsTrust::Plaintext,
            TlsTrust::BundledRoots,
            TlsTrust::backend_pem(CA).unwrap(),
            TlsTrust::ExplicitInsecure,
        ] {
            for proxy in [None, Some("socks5"), Some("socks5h"), Some("http")] {
                for host in ["localhost", "127.0.0.1", "example.invalid"] {
                    let mut request =
                        Request::new(subsystem, Endpoint::new(host, 443).unwrap(), tls.clone());
                    if let Some(scheme) = proxy {
                        request = request.with_proxy(
                            Proxy::new(scheme, Endpoint::new("proxy.invalid", 9050).unwrap(), None)
                                .unwrap(),
                        );
                    }
                    let gate = test_gate(|| Offline::Online);
                    assert!(matches!(
                        gate.authorize(request.clone(), None),
                        Err(Error::ConsentMissing)
                    ));
                    assert!(gate.attempts().records.is_empty());
                    let gate = test_gate(|| Offline::Offline);
                    let proof = Consent::explicit_trigger(&request);
                    assert!(matches!(
                        gate.authorize(request.clone(), Some(proof)),
                        Err(Error::Offline)
                    ));
                    assert!(gate.attempts().records.is_empty());
                    let offline = Arc::new(AtomicBool::new(false));
                    let state = offline.clone();
                    let gate = test_gate(move || {
                        if state.load(Ordering::SeqCst) {
                            Offline::Offline
                        } else {
                            Offline::Online
                        }
                    });
                    if tls == TlsTrust::ExplicitInsecure && subsystem != Subsystem::Sync {
                        let proof = Consent::explicit_trigger(&request);
                        assert!(matches!(
                            gate.authorize(request.clone(), Some(proof)),
                            Err(Error::TrustInvalid)
                        ));
                        continue;
                    }
                    let route = route(&gate, request.clone());
                    offline.store(true, Ordering::SeqCst);
                    let url = format!(
                        "{}://{host}:443/private?secret=hidden",
                        if tls == TlsTrust::Plaintext {
                            "http"
                        } else {
                            "https"
                        }
                    );
                    assert!(matches!(
                        HttpRequest::new("GET", &url)
                            .unwrap()
                            .send(route, HttpOptions::default()),
                        Err(Error::Offline)
                    ));
                    assert!(gate.attempts().records.is_empty());
                    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
                }
            }
        }
    }
}

#[test]
fn http_checks_destination_scheme_and_revocation_before_connect() {
    let gate = test_gate(|| Offline::Online);
    for url in ["http://other.invalid/", "https://node.invalid/"] {
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("node.invalid", 80).unwrap(),
            TlsTrust::Plaintext,
        );
        assert!(matches!(
            HttpRequest::new("GET", url)
                .unwrap()
                .send(route(&gate, request), HttpOptions::default()),
            Err(Error::ConsentScope)
        ));
    }
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("node.invalid", 80).unwrap(),
        TlsTrust::Plaintext,
    );
    let proof = Consent::explicit_trigger(&request);
    let route = gate.authorize(request, Some(proof.clone())).unwrap();
    proof.revoke();
    assert!(matches!(
        HttpRequest::new("GET", "http://node.invalid/")
            .unwrap()
            .send(route, HttpOptions::default()),
        Err(Error::ConsentRevoked)
    ));
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn direct_http_and_forward_proxy_keep_route_credentials_and_limits() {
    for forward in [false, true] {
        let (server, port) = listener();
        let handle = thread::spawn(move || {
            let mut socket = accept(&server);
            let bytes = head(&mut socket);
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.contains("Authorization: Bearer target-secret\r\n"));
            if forward {
                assert!(request
                    .starts_with("GET http://node.invalid/private?token=secret HTTP/1.1\r\n"));
                assert!(request.contains("Proxy-Authorization: Basic dTpw\r\n"));
            } else {
                assert!(request.starts_with("GET /private?token=secret HTTP/1.1\r\n"));
                assert!(!request.contains("Proxy-Authorization"));
            }
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\n\r\n",
                )
                .unwrap();
        });
        let gate = test_gate(|| Offline::Online);
        let (host, target_port) = if forward {
            ("node.invalid", 80)
        } else {
            ("127.0.0.1", port)
        };
        let mut request = Request::new(
            Subsystem::Sync,
            Endpoint::new(host, target_port).unwrap(),
            TlsTrust::Plaintext,
        );
        if forward {
            request = request.with_proxy(
                Proxy::new(
                    "http",
                    Endpoint::new("127.0.0.1", port).unwrap(),
                    Some(("u".into(), "p".into())),
                )
                .unwrap(),
            );
        }
        let response = HttpRequest::new(
            "GET",
            &format!("http://{host}:{target_port}/private?token=secret"),
        )
        .unwrap()
        .header("Authorization", "Bearer target-secret")
        .unwrap()
        .send(
            route(&gate, request),
            HttpOptions {
                max_response_bytes: 32,
                ..HttpOptions::default()
            },
        )
        .unwrap();
        assert_eq!(response.body, b"abc");
        handle.join().unwrap();
        assert!(gate
            .attempts()
            .records
            .iter()
            .all(|r| r.host == "127.0.0.1" && r.operation != Operation::Dns));
        assert!(!format!("{:?}", gate.ledger().snapshot(0, 2000)).contains("secret"));
    }
}

#[test]
fn verified_tls_rejects_untrusted_roots_and_wrong_names() {
    for (host, trust, success) in [
        ("localhost", TlsTrust::backend_pem(CA).unwrap(), true),
        ("localhost", TlsTrust::BundledRoots, false),
        ("127.0.0.1", TlsTrust::backend_pem(CA).unwrap(), false),
        ("127.0.0.1", TlsTrust::ExplicitInsecure, true),
    ] {
        let (server, port) = listener();
        let handle = thread::spawn(move || {
            let mut stream = tls_server(accept(&server));
            if success {
                let request = head(&mut stream);
                assert!(request.starts_with(b"GET / HTTP/1.1"));
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .unwrap();
                stream.flush().unwrap();
            } else {
                let mut byte = [0];
                assert!(stream.read(&mut byte).is_err());
            }
        });
        let gate = test_gate(|| Offline::Online);
        let request = Request::new(Subsystem::Sync, Endpoint::new(host, port).unwrap(), trust);
        let result = HttpRequest::new("GET", &format!("https://{host}:{port}/"))
            .unwrap()
            .send(route(&gate, request), HttpOptions::default());
        if success {
            assert_eq!(result.unwrap().body, b"ok");
        } else {
            assert!(matches!(result, Err(Error::Tls)));
        }
        handle.join().unwrap();
    }
}

#[test]
fn tls_through_socks_and_connect_uses_target_dns_sni_and_trust() {
    for scheme in ["socks5", "socks5h", "http"] {
        let (server, port) = listener();
        let handle = thread::spawn(move || {
            let mut socket = accept(&server);
            if scheme == "http" {
                let request = String::from_utf8(head(&mut socket)).unwrap();
                assert!(request.starts_with("CONNECT hidden.onion:443 HTTP/1.1\r\n"));
                assert!(request.contains("Proxy-Authorization: Basic dTpw\r\n"));
                assert!(!request.contains("target-secret"));
                socket
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .unwrap();
            } else {
                let mut greeting = [0; 4];
                socket.read_exact(&mut greeting).unwrap();
                assert_eq!(greeting, [5, 2, 0, 2]);
                socket.write_all(&[5, 2]).unwrap();
                let mut auth = [0; 5];
                socket.read_exact(&mut auth).unwrap();
                assert_eq!(auth, [1, 1, b'u', 1, b'p']);
                socket.write_all(&[1, 0]).unwrap();
                let mut target = [0; 19];
                socket.read_exact(&mut target).unwrap();
                assert_eq!(&target[..5], &[5, 1, 0, 3, 12]);
                assert_eq!(&target[5..17], b"hidden.onion");
                assert_eq!(&target[17..], &443_u16.to_be_bytes());
                socket.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]).unwrap();
            }
            let mut stream = tls_server(socket);
            let request = String::from_utf8(head(&mut stream)).unwrap();
            assert!(request.contains("Authorization: Bearer target-secret\r\n"));
            assert!(!request.contains("Proxy-Authorization"));
            assert_eq!(stream.conn.server_name(), Some("hidden.onion"));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                .unwrap();
            stream.flush().unwrap();
        });
        let gate = test_gate(|| Offline::Online);
        let proxy = Proxy::new(
            scheme,
            Endpoint::new("127.0.0.1", port).unwrap(),
            Some(("u".into(), "p".into())),
        )
        .unwrap();
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("hidden.onion", 443).unwrap(),
            TlsTrust::backend_pem(CA).unwrap(),
        )
        .with_proxy(proxy);
        let response = HttpRequest::new("GET", "https://hidden.onion/")
            .unwrap()
            .header("Authorization", "Bearer target-secret")
            .unwrap()
            .send(route(&gate, request), HttpOptions::default())
            .unwrap();
        assert_eq!(response.body, b"ok");
        handle.join().unwrap();
        let records = gate.ledger().snapshot(0, 2000).records;
        assert!(records
            .iter()
            .any(|record| record.operation == Operation::Write && record.method == Some("GET")));
        if scheme == "http" {
            assert!(records
                .iter()
                .any(|record| record.operation == Operation::ProxyHandshake
                    && record.host == "127.0.0.1"
                    && record.method == Some("CONNECT")));
        }
        assert!(!format!("{records:?}").contains("Proxy-Authorization"));
        assert!(gate
            .attempts()
            .records
            .iter()
            .all(|r| r.host == "127.0.0.1" && r.operation != Operation::Dns));
    }
}

#[test]
fn redirects_never_connect_to_an_unconsented_origin() {
    for policy in [RedirectPolicy::Refuse, RedirectPolicy::SameOrigin] {
        let (server, port) = listener();
        let (unconsented, other_port) = listener();
        let handle = thread::spawn(move || {
            let mut socket = accept(&server);
            head(&mut socket);
            socket.write_all(format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{other_port}/steal\r\nContent-Length: 0\r\n\r\n").as_bytes()).unwrap();
        });
        let gate = test_gate(|| Offline::Online);
        let request = Request::new(
            Subsystem::Ai,
            Endpoint::new("127.0.0.1", port).unwrap(),
            TlsTrust::Plaintext,
        );
        let error = HttpRequest::new("GET", &format!("http://127.0.0.1:{port}/"))
            .unwrap()
            .header("Authorization", "Bearer secret")
            .unwrap()
            .send(
                route(&gate, request),
                HttpOptions {
                    redirects: policy,
                    ..HttpOptions::default()
                },
            )
            .err()
            .unwrap();
        assert!(matches!(error, Error::RedirectRefused));
        handle.join().unwrap();
        assert_eq!(
            unconsented.accept().unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        assert!(gate
            .attempts()
            .records
            .iter()
            .all(|r| r.host == "127.0.0.1" && r.port == port));
    }
}

#[test]
fn same_origin_redirects_match_python_post_to_get_and_stop_on_revocation() {
    for revoke in [false, true] {
        let (server, port) = listener();
        let gate = test_gate(|| Offline::Online);
        let request = Request::new(
            Subsystem::Ai,
            Endpoint::new("localhost", port).unwrap(),
            TlsTrust::Plaintext,
        );
        let proof = Consent::explicit_trigger(&request);
        let revoker = proof.clone();
        let handle = thread::spawn(move || {
            let mut socket = accept(&server);
            let request = String::from_utf8(head(&mut socket)).unwrap();
            assert!(request.starts_with("POST /first HTTP/1.1\r\n"));
            let mut body = [0; 3];
            socket.read_exact(&mut body).unwrap();
            if revoke {
                revoker.revoke();
            }
            socket
                .write_all(b"HTTP/1.1 302 Found\r\nLocation: /second\r\nContent-Length: 0\r\n\r\n")
                .unwrap();
            drop(socket);
            if !revoke {
                let mut socket = accept(&server);
                let request = String::from_utf8(head(&mut socket)).unwrap();
                assert!(request.starts_with("GET /second HTTP/1.1\r\n"));
                assert!(!request.contains("Content-Length:"));
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            }
        });
        let result = HttpRequest::new("POST", &format!("http://localhost:{port}/first"))
            .unwrap()
            .body(b"abc".to_vec())
            .send(
                gate.authorize(request, Some(proof)).unwrap(),
                HttpOptions {
                    redirects: RedirectPolicy::SameOrigin,
                    ..HttpOptions::default()
                },
            );
        if revoke {
            assert!(matches!(result, Err(Error::ConsentRevoked)));
        } else {
            assert_eq!(result.unwrap().status, 200);
        }
        handle.join().unwrap();
    }
}

#[test]
fn upgraded_tls_stream_rechecks_offline_and_revocation_for_writes() {
    for revoke in [false, true] {
        let (server, port) = listener();
        let offline = Arc::new(AtomicBool::new(false));
        let state = offline.clone();
        let revision = offline.clone();
        let cache = OfflineCache::new(
            move || {
                if state.load(Ordering::SeqCst) {
                    Offline::Offline
                } else {
                    Offline::Online
                }
            },
            move || Some(u64::from(revision.load(Ordering::SeqCst))),
        );
        // This test changes generation only once; production generations may not be reused.
        let gate = test_gate(move || cache.read());
        let request = Request::new(
            Subsystem::Ai,
            Endpoint::new("localhost", port).unwrap(),
            TlsTrust::backend_pem(CA).unwrap(),
        );
        let proof = Consent::explicit_trigger(&request);
        let (sent, received) = std::sync::mpsc::channel();
        let handle = thread::spawn(move || {
            let mut stream = tls_server(accept(&server));
            head(&mut stream);
            stream.write_all(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\r\nx").unwrap();
            stream.flush().unwrap();
            received.recv_timeout(Duration::from_secs(3)).unwrap();
            let mut byte = [0];
            // Client closed with no WebSocket payload after policy changed.
            assert!(stream.read(&mut byte).is_err() || byte == [0]);
        });
        let response = HttpRequest::new("GET", &format!("https://localhost:{port}/"))
            .unwrap()
            .header("Upgrade", "websocket")
            .unwrap()
            .allow_upgrade()
            .send(
                gate.authorize(request, Some(proof.clone())).unwrap(),
                HttpOptions::default(),
            )
            .unwrap();
        let mut stream = response.into_upgrade().unwrap();
        let mut byte = [0];
        stream.read_exact(&mut byte).unwrap();
        assert_eq!(byte, [b'x']);
        let count = gate.ledger().snapshot(0, 2000).last_id;
        if revoke {
            proof.revoke();
        } else {
            offline.store(true, Ordering::SeqCst);
        }
        assert!(stream.write_all(b"secret").is_err());
        assert_eq!(gate.ledger().snapshot(0, 2000).last_id, count);
        drop(stream);
        sent.send(()).unwrap();
        handle.join().unwrap();
    }
}

#[test]
fn python_trust_selection_is_local_bounded_and_bound_to_consent() {
    let temp = tempfile::tempdir().unwrap();
    let host = temp.path().join("host.pem");
    let broken = temp.path().join("broken.pem");
    std::fs::write(&host, CA).unwrap();
    std::fs::write(&broken, b"invalid PEM").unwrap();
    assert!(matches!(
        TlsTrust::select_python_roots(None, &[]).unwrap(),
        TlsTrust::BundledRoots
    ));
    assert!(TlsTrust::select_python_roots(Some(&broken), &[&host]).is_err());
    assert!(TlsTrust::select_python_roots(None, &[&broken, &host]).is_ok());
    let trust = TlsTrust::ca_file(&host).unwrap();
    std::fs::write(&host, b"replaced").unwrap();
    // The reviewed trust snapshot is immutable even if its file changes.
    assert!(tls::config(&trust, true).unwrap().is_some());
    let gate = test_gate(|| Offline::Online);
    let original = Request::new(
        Subsystem::Sync,
        Endpoint::new("node.invalid", 443).unwrap(),
        trust,
    );
    let proof = Consent::explicit_trigger(&original);
    let changed = Request::new(
        Subsystem::Sync,
        Endpoint::new("node.invalid", 443).unwrap(),
        TlsTrust::BundledRoots,
    );
    assert!(matches!(
        gate.authorize(changed, Some(proof)),
        Err(Error::ConsentScope)
    ));
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn errors_and_envelopes_never_include_peer_headers_paths_or_secrets() {
    let error = Error::from(io::Error::other(
        "Authorization: Bearer secret; https://user:pw@host/private",
    ));
    assert!(!format!("{error:?} {error} {}", error.app_error()).contains("secret"));
    for error in [Error::NoEgress, Error::Offline, Error::OfflineUnavailable] {
        assert_eq!(error.app_error().code, "network_egress_disabled");
        assert!(!error.app_error().retryable);
    }
    assert_eq!(
        Error::HttpsProxyUnsupported.app_error().code,
        "network_proxy_unsupported"
    );
    assert_eq!(
        Error::OnionProxyRequired.app_error().code,
        "network_proxy_required"
    );
    assert_eq!(
        Error::TrustInvalid.app_error().code,
        "backend_tls_config_invalid"
    );
}

#[test]
fn revocation_during_tls_handshake_prevents_further_flights_or_alerts() {
    let (server, port) = listener();
    let gate = test_gate(|| Offline::Online);
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("localhost", port).unwrap(),
        TlsTrust::backend_pem(CA).unwrap(),
    );
    let proof = Consent::explicit_trigger(&request);
    let revoker = proof.clone();
    let handle = thread::spawn(move || {
        let mut socket = accept(&server);
        let mut record = [0; 5];
        socket.read_exact(&mut record).unwrap();
        assert_eq!(record[0], 22); // first TLS handshake record
        let length = u16::from_be_bytes([record[3], record[4]]) as usize;
        let mut hello = vec![0; length];
        socket.read_exact(&mut hello).unwrap();
        revoker.revoke();
        // An invalid server response would normally provoke a TLS alert. Even
        // that alert must not escape after revocation.
        let _ = socket.write_all(b"HTTP/1.1 200 wrong protocol\r\n\r\n");
        let mut remaining = Vec::new();
        let _ = socket.read_to_end(&mut remaining);
        assert!(remaining.is_empty());
        length + record.len()
    });
    let result = gate
        .authorize(request, Some(proof))
        .unwrap()
        .connect(Duration::from_secs(2));
    assert!(result.is_err());
    let hello_bytes = handle.join().unwrap();
    let writes: Vec<_> = gate
        .ledger()
        .snapshot(0, 2000)
        .records
        .into_iter()
        .filter(|record| record.operation == Operation::Write)
        .collect();
    assert_eq!(
        writes
            .iter()
            .map(|record| record.bytes_out as usize)
            .sum::<usize>(),
        hello_bytes
    );
}

#[test]
fn failed_proxy_never_falls_back_to_target_dns_or_direct() {
    for scheme in ["socks5h", "http"] {
        let (server, port) = listener();
        let handle = thread::spawn(move || {
            let mut socket = accept(&server);
            if scheme == "http" {
                head(&mut socket);
                socket
                    .write_all(b"HTTP/1.1 407 Refused\r\nContent-Length: 0\r\n\r\n")
                    .unwrap();
            } else {
                let mut greeting = [0; 3];
                socket.read_exact(&mut greeting).unwrap();
                socket.write_all(&[5, 255]).unwrap();
            }
        });
        let gate = test_gate(|| Offline::Online);
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("must-not-resolve.invalid", 443).unwrap(),
            TlsTrust::BundledRoots,
        )
        .with_proxy(Proxy::new(scheme, Endpoint::new("127.0.0.1", port).unwrap(), None).unwrap());
        assert!(HttpRequest::new("GET", "https://must-not-resolve.invalid/")
            .unwrap()
            .send(route(&gate, request), HttpOptions::default())
            .is_err());
        handle.join().unwrap();
        assert!(gate
            .attempts()
            .records
            .iter()
            .all(|record| record.host == "127.0.0.1" && record.operation != Operation::Dns));
    }
}

#[test]
fn http_timeout_and_expired_budget_are_bounded() {
    let (server, port) = listener();
    let (done, wait) = std::sync::mpsc::channel();
    let handle = thread::spawn(move || {
        let mut socket = accept(&server);
        head(&mut socket);
        wait.recv_timeout(Duration::from_secs(3)).unwrap();
    });
    let gate = test_gate(|| Offline::Online);
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("127.0.0.1", port).unwrap(),
        TlsTrust::Plaintext,
    );
    let result = HttpRequest::new("GET", &format!("http://127.0.0.1:{port}/"))
        .unwrap()
        .send(
            route(&gate, request),
            HttpOptions {
                timeout: Duration::from_millis(50),
                ..HttpOptions::default()
            },
        );
    assert!(
        matches!(result, Err(Error::Io(ref error)) if matches!(error.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut))
    );
    done.send(()).unwrap();
    handle.join().unwrap();
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("budget.invalid", 80).unwrap(),
        TlsTrust::Plaintext,
    );
    let count = gate.attempts().records.len();
    let result = HttpRequest::new("GET", "http://budget.invalid/")
        .unwrap()
        .send(
            route(&gate, request),
            HttpOptions {
                deadline: Some(Instant::now()),
                ..HttpOptions::default()
            },
        );
    assert!(matches!(result, Err(Error::InvalidTimeout)));
    assert_eq!(gate.attempts().records.len(), count);
}

#[test]
fn pinned_self_signed_shapes_match_python_on_owned_loopback() {
    for (pem, key, filename) in [
        (
            include_bytes!("../tests/fixtures/lnd-ca.pem").as_slice(),
            include_bytes!("../tests/fixtures/lnd-ca-key.pem").as_slice(),
            "lnd-ca.pem",
        ),
        (
            include_bytes!("../tests/fixtures/openssl-ca.pem").as_slice(),
            include_bytes!("../tests/fixtures/openssl-ca-key.pem").as_slice(),
            "openssl-ca.pem",
        ),
        (
            include_bytes!("../tests/fixtures/cn-only.pem").as_slice(),
            include_bytes!("../tests/fixtures/cn-only-key.pem").as_slice(),
            "cn-only.pem",
        ),
    ] {
        let (server, port) = listener();
        let handle = thread::spawn(move || {
            for _ in 0..2 {
                let mut stream = tls_server_with(accept(&server), pem, key);
                head(&mut stream);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")
                    .unwrap();
                stream.flush().unwrap();
            }
        });
        let gate = test_gate(|| Offline::Online);
        let request = Request::new(
            Subsystem::Sync,
            Endpoint::new("localhost", port).unwrap(),
            TlsTrust::backend_pem(pem).unwrap(),
        );
        assert_eq!(
            HttpRequest::new("GET", &format!("https://localhost:{port}/"))
                .unwrap()
                .send(route(&gate, request), HttpOptions::default())
                .unwrap()
                .body,
            b"ok"
        );
        let result = std::process::Command::new("python3")
            .args(["-c", r#"import socket, ssl, sys
ctx = ssl.create_default_context(cafile=sys.argv[1])
with ctx.wrap_socket(socket.create_connection(('127.0.0.1', int(sys.argv[2])), timeout=3), server_hostname='localhost') as s:
    s.sendall(b'GET / HTTP/1.1\r\nHost: localhost\r\n\r\n')
    assert s.recv(4096).endswith(b'ok')
"#])
            .arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures").join(filename)).arg(port.to_string()).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        handle.join().unwrap();
    }
}

#[test]
fn close_delimited_tls_response_accepts_missing_close_notify() {
    let (server, port) = listener();
    let handle = thread::spawn(move || {
        let mut stream = tls_server(accept(&server));
        head(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\n\r\ncomplete")
            .unwrap();
        stream.flush().unwrap();
        // Drop the socket without a TLS close_notify, as Python permits.
    });
    let gate = test_gate(|| Offline::Online);
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("localhost", port).unwrap(),
        TlsTrust::backend_pem(CA).unwrap(),
    );
    let response = HttpRequest::new("GET", &format!("https://localhost:{port}/"))
        .unwrap()
        .send(route(&gate, request), HttpOptions::default())
        .unwrap();
    assert_eq!(response.body, b"complete");
    handle.join().unwrap();
}

#[test]
fn tls_read_checks_offline_once_and_cached_timeouts_do_not_repeat_options() {
    use std::sync::atomic::AtomicUsize;
    let (server, port) = listener();
    let handle = thread::spawn(move || {
        let mut stream = tls_server(accept(&server));
        stream.write_all(b"xy").unwrap();
        stream.flush().unwrap();
    });
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let gate = test_gate(move || {
        count.fetch_add(1, Ordering::SeqCst);
        Offline::Online
    });
    let request = Request::new(
        Subsystem::Sync,
        Endpoint::new("localhost", port).unwrap(),
        TlsTrust::backend_pem(CA).unwrap(),
    );
    let mut connection = route(&gate, request)
        .connect(Duration::from_secs(3))
        .unwrap();
    let before = calls.load(Ordering::SeqCst);
    let mut bytes = [0];
    connection.read_exact(&mut bytes).unwrap();
    assert_eq!(bytes, [b'x']);
    assert_eq!(calls.load(Ordering::SeqCst) - before, 1);
    let transport = connection.transport();
    assert_eq!(transport.read_timeout, Some(Duration::from_secs(3)));
    transport.prepare_read().unwrap();
    assert_eq!(transport.read_timeout, Some(Duration::from_secs(3)));
    handle.join().unwrap();
}
