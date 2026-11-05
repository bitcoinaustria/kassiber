use super::*;
use kassiber_net::{test_support::FakeProvider, Ledger};
use std::path::Path;

fn fixture() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let schema: Json =
        serde_json::from_str(include_str!("../../../../../store/fixtures/creation.json")).unwrap();
    for sql in schema["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO settings(key,value) VALUES('database_instance_id','disposable-book');",
    )
    .unwrap();
    conn
}
fn error(e: AppError) -> Py {
    dict([
        ("code", s(e.code)),
        ("message", s(e.message)),
        ("hint", e.hint.map_or(Py::Null, |v| s(v.as_ref()))),
        ("details", e.details.unwrap_or(Py::Null)),
        ("retryable", Py::Bool(e.retryable)),
    ])
}
fn dump(v: &Py) -> String {
    kassiber_pyjson::dumps(v, &Options::new().sort_keys(true)).unwrap()
}
#[test]
fn python_document_provider_vectors() {
    let Py::List(cases) = kassiber_pytext::loads(include_str!(
        "../../../../tests/vectors/python_document_providers.json"
    ))
    .unwrap() else {
        panic!()
    };
    for (index, case) in cases.iter().enumerate() {
        let operation = rendered(field(case, "operation"));
        let actual = match operation.as_str() {
            "pages" => super::super::pdf::selected(
                field(case, "pages"),
                rendered(field(case, "total")).parse().unwrap(),
                rendered(field(case, "maximum")).parse().unwrap(),
            )
            .map(|(pages, explicit)| {
                Py::List(vec![
                    Py::List(pages.into_iter().map(Py::from).collect()),
                    Py::Bool(explicit),
                ])
            }),
            "choose" => {
                let Py::List(models) = field(case, "models") else {
                    panic!()
                };
                let requested = match field(case, "requested") {
                    Py::Str(v) => v.as_str(),
                    _ => None,
                };
                choose_model(field(case, "provider"), models, requested).map(|(model, ids)| {
                    Py::List(vec![s(model), Py::List(ids.iter().map(s).collect())])
                })
            }
            "extract" => extract(&rendered(field(case, "text"))),
            "content" => http::content(
                field(case, "payload"),
                field(case, "wire") == &s("responses"),
            )
            .map(s),
            "store" => Ok(Py::Bool(http::names_store(
                &AppError::new("ai_request_invalid", "invalid")
                    .with_details(field(case, "details").clone()),
            ))),
            "body" => Ok(http::request_body(
                "glm-ocr",
                &[dict([
                    ("type", s("image_url")),
                    (
                        "image_url",
                        dict([("url", s("data:image/png;base64,ZmFrZQ=="))]),
                    ),
                ])],
                &rendered(field(case, "wire")),
                truthy(field(case, "omit")),
            )),
            _ => panic!("{operation}"),
        };
        let (actual, expected) = if let Some(expected) = get(case, "error") {
            (error(actual.unwrap_err()), expected)
        } else {
            (actual.unwrap(), field(case, "result"))
        };
        assert_eq!(dump(&actual), dump(expected), "{operation} {index}");
    }
}
#[test]
fn document_transport_errors_keep_ai_contract_and_redact_diagnostics() {
    let error = http::transport_error(kassiber_net::Error::Io(std::io::Error::other(
        "secret document path and provider key",
    )));
    assert_eq!(error.code, "ai_unavailable");
    assert_eq!(error.message, "AI provider is unreachable");
    assert!(error.retryable);
    assert_eq!(
        field(error.details.as_ref().unwrap(), "reason"),
        &s("network I/O failed")
    );
    assert_eq!(
        http::transport_error(kassiber_net::Error::ConsentRevoked).code,
        "interaction_required"
    );
    assert_eq!(
        http::transport_error(kassiber_net::Error::NoEgress).code,
        "network_egress_disabled"
    );
}
fn request(source: &Path) -> DocumentPreview {
    DocumentPreview {
        source_file: source.to_string_lossy().into_owned(),
        provider_name: Some("local".into()),
        model: None,
        confidence_threshold: Py::Null,
        max_pages: Py::Null,
        pages: Py::Null,
        expected_fiat_currency: Some("EUR".into()),
    }
}
fn configure(conn: &Connection, base: &str) {
    conn.execute("INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at) VALUES('local',?,'local','2026-01-01','2026-01-01')",[format!("{base}/v1")]).unwrap();
}
fn run(client: &Providers, conn: &Connection, request: &DocumentPreview) -> Result<Py, AppError> {
    client.preview(
        conn,
        request,
        Some(DocumentConsent::explicit(conn, request)?),
    )
}
#[test]
fn document_providers_seed_locally_once_and_preserve_tombstones() {
    let conn = fixture();
    let provider = resolve_provider(&conn, None).unwrap();
    assert_eq!(field(&provider, "name"), &s("ollama"));
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM ai_providers", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        6
    );
    conn.execute("DELETE FROM ai_providers", []).unwrap();
    assert_eq!(resolve_provider(&conn, None).unwrap_err().code, "not_found");
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM ai_providers", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    conn.execute("DELETE FROM settings WHERE key='default_ai_provider'", [])
        .unwrap();
    assert_eq!(
        resolve_provider(&conn, None).unwrap_err().code,
        "ai_provider_not_configured"
    );
}
#[test]
fn document_missing_stale_disabled_remote_and_secret_grants_make_no_attempt() {
    let conn = fixture();
    let server = FakeProvider::new(r#"{"data":[]}"#).unwrap();
    let other = FakeProvider::new(r#"{"data":[]}"#).unwrap();
    configure(&conn, server.base());
    let root = files::Temp::new("provider-policy").unwrap();
    let source = root.path().join("image.png");
    std::fs::write(&source, b"fake").unwrap();
    let request = request(&source);
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let client = Providers::new(gate.clone());
    assert_eq!(
        client.preview(&conn, &request, None).unwrap_err().code,
        "interaction_required"
    );
    let consent = DocumentConsent::explicit(&conn, &request).unwrap();
    let mut changed = request.clone();
    changed.model = Some("glm-ocr".into());
    assert_eq!(
        client
            .preview(&conn, &changed, Some(consent))
            .unwrap_err()
            .code,
        "interaction_required"
    );
    let consent = DocumentConsent::explicit(&conn, &request).unwrap();
    conn.execute(
        "UPDATE ai_providers SET base_url=? WHERE name='local'",
        [format!("{}/v1", other.base())],
    )
    .unwrap();
    assert_eq!(
        client
            .preview(&conn, &request, Some(consent))
            .unwrap_err()
            .code,
        "interaction_required"
    );
    conn.execute(
        "UPDATE ai_providers SET base_url=? WHERE name='local'",
        [format!("{}/v1", server.base())],
    )
    .unwrap();
    let consent = DocumentConsent::explicit(&conn, &request).unwrap();
    conn.execute(
        "INSERT INTO settings(key,value) VALUES('context_profile','switched')",
        [],
    )
    .unwrap();
    assert_eq!(
        client
            .preview(&conn, &request, Some(consent))
            .unwrap_err()
            .code,
        "interaction_required"
    );
    let consent = DocumentConsent::explicit(&conn, &request).unwrap();
    conn.execute(
        "UPDATE settings SET value='other' WHERE key='database_instance_id'",
        [],
    )
    .unwrap();
    assert_eq!(
        client
            .preview(&conn, &request, Some(consent))
            .unwrap_err()
            .code,
        "interaction_required"
    );
    for (sql,code) in [
        ("UPDATE ai_providers SET enabled=0 WHERE name='local'","ai_provider_disabled"),
        ("UPDATE ai_providers SET enabled=1,kind='remote' WHERE name='local'","ai_remote_ack_required"),
        ("UPDATE ai_providers SET acknowledged_at='yes' WHERE name='local'","document_import_local_ai_required"),
        ("UPDATE ai_providers SET kind='local',base_url='http://example.invalid/v1' WHERE name='local'","document_import_remote_ai_disabled"),
        ("UPDATE ai_providers SET base_url='http://127.0.0.1:9/v1' WHERE name='local'; INSERT INTO ai_provider_secret_refs(provider_name,store_id,service,account,state,created_at) VALUES('local','linux_secret_service','untrusted','foreign','ok','2026-01-01')","secret_ref_unavailable"),
    ] {
        conn.execute_batch(sql).unwrap();
        if code == "secret_ref_unavailable" {
            conn.execute("UPDATE ai_providers SET base_url=? WHERE name='local'", [format!("{}/v1", server.base())]).unwrap();
        }
        assert_eq!(run(&client,&conn,&request).unwrap_err().code,code);
    }
    assert!(gate.attempts().records.is_empty());
    assert!(gate.ledger().snapshot(0, 2000).records.is_empty());
    assert!(server.requests().is_empty());
    assert!(other.requests().is_empty());
}
#[test]
fn document_http_preview_uses_local_offline_route_and_keeps_evidence_in_memory() {
    let server=FakeProvider::replies(vec![(200,r#"{"data":[{"id":"text"},{"id":"glm-ocr"}]}"#.into()),(200,r#"{"output":[{"type":"message","content":[{"type":"output_text","text":"{\"rows\":[]}"}]}]}"#.into())]).unwrap();
    let conn = fixture();
    configure(&conn, server.base());
    let root = files::Temp::new("provider-success").unwrap();
    let source = root.path().join("image.png");
    std::fs::write(&source, b"fake").unwrap();
    let gate = Gate::new(|| Offline::Offline, Ledger::default());
    let client = Providers::new(gate.clone());
    let result = run(&client, &conn, &request(&source)).unwrap();
    assert_eq!(field(&result, "model"), &s("glm-ocr"));
    assert_eq!(field(field(&result, "summary"), "rows"), &Py::from(0));
    let hits = server.requests();
    assert_eq!(hits.len(), 2);
    assert!(hits[0].starts_with("GET /v1/models "));
    assert!(hits[1].starts_with("POST /v1/responses "));
    let body = kassiber_pytext::loads(hits[1].split_once("\r\n\r\n").unwrap().1).unwrap();
    assert_eq!(field(&body, "store"), &Py::Bool(false));
    assert!(dump(&body).contains("data:image/png;base64,ZmFrZQ=="));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 1);
    let ledger = format!("{:?}", gate.ledger().snapshot(0, 2000));
    assert!(!ledger.contains("ZmFrZQ"));
    assert!(!ledger.contains("image.png"));
    assert!(!ledger.contains("instructions"));
}
#[test]
fn document_http_fallback_is_origin_bound_and_never_weakens_retention() {
    let server = FakeProvider::replies(vec![
        (200, r#"{"data":[{"id":"glm-ocr"}]}"#.into()),
        (404, "Not found".into()),
        (
            200,
            r#"{"choices":[{"message":{"content":"{\"rows\":[]}"}}]}"#.into(),
        ),
    ])
    .unwrap();
    let conn = fixture();
    configure(&conn, server.base());
    let root = files::Temp::new("provider-fallback").unwrap();
    let source = root.path().join("image.png");
    std::fs::write(&source, b"fake").unwrap();
    let client = Providers::new(Gate::new(|| Offline::Online, Ledger::default()));
    run(&client, &conn, &request(&source)).unwrap();
    assert_eq!(server.requests().len(), 3);
    assert!(server.requests()[2].starts_with("POST /v1/chat/completions "));
    assert_eq!(
        conn.query_row(
            "SELECT detected_api_format FROM ai_providers WHERE name='local'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "chat_completions"
    );
    for (status, body, expected) in [
        (
            400,
            r#"{"error":{"param":"store","message":"unsupported"}}"#,
            "ai_store_refused",
        ),
        (
            404,
            r#"{"error":{"message":"model not found"}}"#,
            "ai_request_invalid",
        ),
        (401, "unauthorized", "ai_auth_failed"),
        (429, "limited", "ai_rate_limited"),
    ] {
        let server = FakeProvider::replies(vec![
            (200, r#"{"data":[{"id":"glm-ocr"}]}"#.into()),
            (status, body.into()),
        ])
        .unwrap();
        conn.execute(
            "UPDATE ai_providers SET base_url=?,detected_api_format=NULL WHERE name='local'",
            [format!("{}/v1", server.base())],
        )
        .unwrap();
        assert_eq!(
            run(&client, &conn, &request(&source)).unwrap_err().code,
            expected
        );
        assert_eq!(server.requests().len(), 2);
    }
    let server = FakeProvider::script(vec![(
        302,
        vec![("Location".into(), "http://192.0.2.1/v1/models".into())],
        String::new(),
    )])
    .unwrap();
    conn.execute(
        "UPDATE ai_providers SET base_url=? WHERE name='local'",
        [format!("{}/v1", server.base())],
    )
    .unwrap();
    assert_eq!(
        run(&client, &conn, &request(&source)).unwrap_err().code,
        "ai_request_invalid"
    );
    assert_eq!(server.requests().len(), 1);
}
