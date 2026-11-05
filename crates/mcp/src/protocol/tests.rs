//! `ProtocolEraTests`, `StdioLoopTests`, `SubscriptionLimitTests`,
//! `StdioRobustnessTests` and `StrictJsonTests` of `tests/test_mcp_server.py`,
//! plus the R6-8 mutation targets: era selection, error codes, the in-flight
//! limit and request-id typing.

use super::*;
use std::sync::Condvar;
use std::time::Instant;

/// `_FakeProvider`: one `echo` tool that waits for a release.
#[derive(Default)]
struct Fake {
    calls: Mutex<Vec<Value>>,
    gate: Gate,
}

#[derive(Default)]
struct Gate {
    open: Mutex<bool>,
    wake: Condvar,
}

impl Gate {
    fn new(open: bool) -> Self {
        Gate {
            open: Mutex::new(open),
            wake: Condvar::new(),
        }
    }

    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.wake.notify_all();
    }

    fn wait(&self, limit: Duration) {
        let open = self.open.lock().unwrap();
        let _ = self
            .wake
            .wait_timeout_while(open, limit, |open| !*open)
            .unwrap();
    }
}

impl Fake {
    fn held() -> Self {
        Fake {
            calls: Mutex::default(),
            gate: Gate::new(false),
        }
    }

    fn open() -> Self {
        Fake {
            calls: Mutex::default(),
            gate: Gate::new(true),
        }
    }
}

impl ToolProvider for Fake {
    fn instructions(&self) -> &str {
        "fake instructions"
    }

    fn tool_definitions(&self) -> Vec<Value> {
        vec![parse(
            r#"{"name":"echo","title":"Echo","description":"Echo arguments","inputSchema":{"type":"object","additionalProperties":true},"annotations":{"title":"Echo","readOnlyHint":true},"outputSchema":{"type":"object"}}"#,
        )]
    }

    fn has_tool(&self, name: &PyStr) -> bool {
        name.is("echo")
    }

    fn call_tool(
        &self,
        _name: &PyStr,
        arguments: &Value,
        _cancelled: &Cancelled,
    ) -> Result<ToolOutcome, Internal> {
        self.gate.wait(Duration::from_secs(5));
        self.calls.lock().unwrap().push(arguments.clone());
        if get(arguments, "panic").is_some() {
            panic!("a provider bug");
        }
        if get(arguments, "internal").is_some() {
            return Err(Internal);
        }
        Ok(ToolOutcome {
            structured: dict([("echo", arguments.clone())]),
            text: PyStr::from(
                kassiber_envelope::daemon_line(arguments)
                    .unwrap()
                    .trim_end(),
            ),
            is_error: false,
        })
    }
}

fn parse(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

fn line(value: &Value) -> String {
    kassiber_envelope::daemon_line(value).unwrap()
}

const MODERN: &str = r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}"#;

fn request(id: &str, method: &str, params: &str) -> String {
    if params.is_empty() {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}"}}"#)
    } else {
        format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{{{params}}}}}"#)
    }
}

fn modern(id: &str, method: &str, extra: &str) -> String {
    let params = if extra.is_empty() {
        MODERN.to_owned()
    } else {
        format!("{MODERN},{extra}")
    };
    request(id, method, &params)
}

fn server(provider: Fake) -> McpServer {
    McpServer::new(Arc::new(provider), "kassiber", "test")
}

fn handle(server: &McpServer, text: &str) -> Option<String> {
    server.handle(&parse(text), None).map(|value| line(&value))
}

fn error_code(response: &str) -> i64 {
    match get(get(&parse(response), "error").unwrap(), "code") {
        Some(Value::Int(code)) => i64::try_from(code.clone()).unwrap(),
        other => panic!("no error code: {other:?}"),
    }
}

/// A writer the test can read after the loop returns.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn serve_bytes(server: McpServer, input: Vec<u8>, drain: Duration) -> Vec<String> {
    let sink = Sink::default();
    StdioLoop::new(
        Arc::new(server),
        Box::new(std::io::Cursor::new(input)),
        Box::new(sink.clone()),
    )
    .drain(drain)
    .run();
    let bytes = sink.0.lock().unwrap().clone();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

fn serve_lines(server: McpServer, lines: &[String]) -> Vec<String> {
    let input: String = lines.iter().map(|line| format!("{line}\n")).collect();
    serve_bytes(server, input.into_bytes(), SHUTDOWN_DRAIN)
}

fn ids(responses: &[String]) -> Vec<String> {
    responses
        .iter()
        .map(|response| line(get(&parse(response), "id").unwrap_or(&Value::Null)))
        .collect()
}

#[test]
fn modern_discover_is_complete_cacheable_and_identified() {
    let response = handle(&server(Fake::open()), &modern("1", "server/discover", "")).unwrap();
    assert_eq!(
        response,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"supportedVersions\":[\"2026-07-28\"],\"capabilities\":{\"tools\":{}},\"ttlMs\":3600000,\"cacheScope\":\"public\",\"instructions\":\"fake instructions\",\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/serverInfo\":{\"name\":\"kassiber\",\"version\":\"test\"}}}}\n"
    );
}

#[test]
fn modern_requests_need_no_handshake_and_never_set_legacy_state() {
    let server = server(Fake::open());
    let listed = parse(&handle(&server, &modern("1", "tools/list", "")).unwrap());
    let result = get(&listed, "result").unwrap();
    assert_eq!(get(result, "resultType"), Some(&s("complete")));
    // A modern list keeps title and outputSchema.
    let Some(Value::List(tools)) = get(result, "tools") else {
        panic!("no tools");
    };
    assert!(get(&tools[0], "title").is_some() && get(&tools[0], "outputSchema").is_some());
    // A later request without metadata is still refused: modern requests
    // establish no session.
    let refused = handle(&server, &request("2", "tools/list", "")).unwrap();
    assert_eq!(
        refused,
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32602,\"message\":\"Requests must carry protocolVersion and clientCapabilities in _meta, or follow an initialize handshake\",\"data\":{\"supported\":[\"2026-07-28\"],\"legacy\":[\"2025-11-25\",\"2025-06-18\"]}},\"id\":2}\n"
    );
}

#[test]
fn unsupported_modern_version_lists_supported_versions() {
    let text = request(
        "1",
        "tools/list",
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"1900-01-01","io.modelcontextprotocol/clientCapabilities":{}}"#,
    );
    assert_eq!(
        handle(&server(Fake::open()), &text).unwrap(),
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32022,\"message\":\"Unsupported protocol version\",\"data\":{\"supported\":[\"2026-07-28\"],\"requested\":\"1900-01-01\"}},\"id\":1}\n"
    );
}

#[test]
fn modern_request_requires_client_capabilities() {
    let server = server(Fake::open());
    for meta in [
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28"}"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":[]}"#,
        r#""_meta":{"io.modelcontextprotocol/protocolVersion":20260728,"io.modelcontextprotocol/clientCapabilities":{}}"#,
    ] {
        let response = handle(&server, &request("1", "tools/list", meta)).unwrap();
        assert_eq!(error_code(&response), INVALID_PARAMS, "{meta}");
        assert!(response.contains("Request _meta requires protocolVersion and clientCapabilities"));
    }
    // `_meta` without the version key is a legacy request, not a modern one.
    let legacy = handle(
        &server,
        &request(
            "1",
            "ping",
            r#""_meta":{"io.modelcontextprotocol/clientCapabilities":{}}"#,
        ),
    )
    .unwrap();
    assert_eq!(legacy, "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
}

#[test]
fn legacy_initialize_echoes_supported_and_offers_newest_otherwise() {
    for (requested, expected) in [
        ("\"2025-06-18\"", "2025-06-18"),
        ("\"2025-11-25\"", "2025-11-25"),
        ("\"2024-01-01\"", LEGACY_PROTOCOL_VERSIONS[0]),
        ("\"2025-03-26\"", LEGACY_PROTOCOL_VERSIONS[0]),
        ("20250618", LEGACY_PROTOCOL_VERSIONS[0]),
        ("\"2026-07-28\"", LEGACY_PROTOCOL_VERSIONS[0]),
    ] {
        let server = server(Fake::open());
        let response = handle(
            &server,
            &request(
                "1",
                "initialize",
                &format!(r#""protocolVersion":{requested},"capabilities":{{}}"#),
            ),
        )
        .unwrap();
        assert_eq!(
            response,
            format!("{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"protocolVersion\":\"{expected}\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"kassiber\",\"version\":\"test\"}},\"instructions\":\"fake instructions\"}}}}\n"),
            "{requested}"
        );
        let listed = parse(&handle(&server, &request("2", "tools/list", "")).unwrap());
        let result = get(&listed, "result").unwrap();
        assert!(get(result, "resultType").is_none() && get(result, "ttlMs").is_none());
        // Both legacy revisions are structured: title and outputSchema stay.
        let Some(Value::List(tools)) = get(result, "tools") else {
            panic!("no tools");
        };
        assert!(get(&tools[0], "title").is_some());
    }
    assert!(!LEGACY_PROTOCOL_VERSIONS.contains(&"2025-03-26"));
}

#[test]
fn unstructured_eras_drop_title_and_output_schema() {
    let server = server(Fake::open());
    let unstructured = server.tools(Era {
        modern: false,
        version: None,
    });
    let Value::List(tools) = unstructured else {
        panic!("no tools");
    };
    assert!(get(&tools[0], "title").is_none() && get(&tools[0], "outputSchema").is_none());
    assert!(get(&tools[0], "annotations").is_some());
}

#[test]
fn legacy_ping_works_before_initialize_and_modern_ping_is_unknown() {
    let server = server(Fake::open());
    assert_eq!(
        handle(&server, &request("1", "ping", "")).unwrap(),
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n"
    );
    for method in ["ping", "initialize", "resources/list"] {
        let response = handle(&server, &modern("2", method, "")).unwrap();
        assert_eq!(
            response,
            format!("{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32601,\"message\":\"Method not found: {method}\"}},\"id\":2}}\n")
        );
    }
    // Legacy methods outside the lifecycle wait for initialize.
    assert_eq!(
        error_code(&handle(&server, &request("3", "server/discover", "")).unwrap()),
        INVALID_PARAMS
    );
    handle(&server, &request("4", "initialize", "")).unwrap();
    assert_eq!(
        error_code(&handle(&server, &request("5", "server/discover", "")).unwrap()),
        METHOD_NOT_FOUND
    );
}

#[test]
fn unknown_tool_and_method_are_protocol_errors() {
    let server = server(Fake::open());
    let unknown_tool = handle(
        &server,
        &modern("1", "tools/call", r#""name":"shell","arguments":{}"#),
    )
    .unwrap();
    assert_eq!(
        unknown_tool,
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32602,\"message\":\"Unknown tool: shell\"},\"id\":1}\n"
    );
    // Python formats the missing or non-string name with str().
    for (name, shown) in [
        ("", "None"),
        (r#""name":null,"#, "None"),
        (r#""name":5,"#, "5"),
        (r#""name":["a",1],"#, "['a', 1]"),
        (r#""name":{"a":true},"#, "{'a': True}"),
    ] {
        let response = handle(
            &server,
            &modern("1", "tools/call", &format!("{name}\"x\":1")),
        )
        .unwrap();
        assert!(
            response.contains(&format!(
                "\"message\":\"Unknown tool: {}\"",
                shown.replace('\'', "'")
            )),
            "{response}"
        );
    }
    let bad_arguments = handle(
        &server,
        &modern("1", "tools/call", r#""name":"echo","arguments":[1]"#),
    )
    .unwrap();
    assert_eq!(
        bad_arguments,
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32602,\"message\":\"Tool arguments must be an object\"},\"id\":1}\n"
    );
    let cursor = handle(&server, &modern("1", "tools/list", r#""cursor":"next""#)).unwrap();
    assert_eq!(
        cursor,
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32602,\"message\":\"Invalid cursor\"},\"id\":1}\n"
    );
    let null_cursor = handle(&server, &modern("1", "tools/list", r#""cursor":null"#)).unwrap();
    assert!(null_cursor.contains("\"result\""));
}

#[test]
fn modern_call_returns_structured_and_text_content() {
    let server = server(Fake::open());
    let response = handle(
        &server,
        &modern("1", "tools/call", r#""name":"echo","arguments":{"x":2}"#),
    )
    .unwrap();
    assert_eq!(
        response,
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"content\":[{\"type\":\"text\",\"text\":\"{\\\"x\\\":2}\"}],\"structuredContent\":{\"echo\":{\"x\":2}},\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/serverInfo\":{\"name\":\"kassiber\",\"version\":\"test\"}}}}\n"
    );
    // Omitted and null arguments are an empty object.
    for arguments in ["", r#","arguments":null"#] {
        let response = handle(
            &server,
            &modern("2", "tools/call", &format!(r#""name":"echo"{arguments}"#)),
        )
        .unwrap();
        assert!(
            response.contains("\"structuredContent\":{\"echo\":{}}"),
            "{response}"
        );
    }
    let internal = handle(
        &server,
        &modern(
            "3",
            "tools/call",
            r#""name":"echo","arguments":{"internal":1}"#,
        ),
    )
    .unwrap();
    assert_eq!(
        internal,
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"},\"id\":3}\n"
    );
}

#[test]
fn envelope_errors_follow_python_order() {
    let server = server(Fake::open());
    let cases: [(&str, Option<&str>); 11] = [
        ("[1]", Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid JSON-RPC message\"},\"id\":null}\n")),
        (r#"{"id":4,"method":"ping"}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid JSON-RPC message\"},\"id\":4}\n")),
        (r#"{"jsonrpc":"2.0","id":4.5,"method":7}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid JSON-RPC message\"},\"id\":null}\n")),
        (r#"{"jsonrpc":"2.0","id":4,"result":{}}"#, None),
        (r#"{"jsonrpc":"2.0","id":4,"error":{}}"#, None),
        (r#"{"jsonrpc":"2.0","method":"tools/list"}"#, None),
        (r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid request id\"},\"id\":null}\n")),
        (r#"{"jsonrpc":"2.0","id":true,"method":"ping"}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid request id\"},\"id\":null}\n")),
        (r#"{"jsonrpc":"2.0","id":1.0,"method":"ping"}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid request id\"},\"id\":null}\n")),
        (r#"{"jsonrpc":"2.0","id":"a","method":"ping","params":[]}"#, Some("{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32602,\"message\":\"Request params must be an object\"},\"id\":\"a\"}\n")),
        (r#"{"jsonrpc":"2.0","id":"a","method":"ping","params":null}"#, Some("{\"jsonrpc\":\"2.0\",\"id\":\"a\",\"result\":{}}\n")),
    ];
    for (input, expected) in cases {
        assert_eq!(handle(&server, input).as_deref(), expected, "{input}");
    }
}

#[test]
fn string_and_integer_ids_are_echoed_as_sent() {
    let server = server(Fake::open());
    assert_eq!(
        handle(&server, &request("\"1\"", "ping", "")).unwrap(),
        "{\"jsonrpc\":\"2.0\",\"id\":\"1\",\"result\":{}}\n"
    );
    assert_eq!(
        handle(
            &server,
            &request("123456789012345678901234567890", "ping", "")
        )
        .unwrap(),
        "{\"jsonrpc\":\"2.0\",\"id\":123456789012345678901234567890,\"result\":{}}\n"
    );
    assert_ne!(id_key(&parse("1")), id_key(&parse("\"1\"")));
    assert_eq!(id_key(&parse("1")), id_key(&parse("1")));
    assert_eq!(id_key(&parse("true")), None);
    assert_eq!(id_key(&parse("1.0")), None);
}

#[test]
fn every_accepted_call_is_answered_before_exit_on_eof() {
    let responses = serve_lines(
        server(Fake::open()),
        &[
            modern("1", "tools/call", r#""name":"echo","arguments":{"n":1}"#),
            modern("2", "tools/call", r#""name":"echo","arguments":{"n":2}"#),
        ],
    );
    let mut seen = ids(&responses);
    seen.sort();
    assert_eq!(seen, ["1\n", "2\n"]);
}

#[test]
fn cancelled_call_gets_no_reply() {
    let fake = Arc::new(Fake::held());
    let released = Arc::clone(&fake);
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        released.gate.release();
    });
    let sink = Sink::default();
    let input = [
        modern("1", "tools/call", r#""name":"echo","arguments":{}"#),
        modern("2", "tools/call", r#""name":"echo","arguments":{}"#),
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}"#
            .to_owned(),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect::<String>();
    StdioLoop::new(
        Arc::new(McpServer::new(fake.clone(), "kassiber", "test")),
        Box::new(std::io::Cursor::new(input.into_bytes())),
        Box::new(sink.clone()),
    )
    .run();
    timer.join().unwrap();
    let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<String> = output.lines().map(str::to_owned).collect();
    assert_eq!(ids(&lines), ["1\n"]);
    // The cancelled call never ran.
    assert_eq!(fake.calls.lock().unwrap().len(), 1);
}

#[test]
fn a_cancel_names_its_id_by_type() {
    // `"2"` does not cancel call 2: ids differ by JSON type.
    let fake = Arc::new(Fake::held());
    let released = Arc::clone(&fake);
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        released.gate.release();
    });
    let sink = Sink::default();
    let input = [
        modern("1", "tools/call", r#""name":"echo","arguments":{}"#),
        modern("2", "tools/call", r#""name":"echo","arguments":{}"#),
        r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":"2"}}"#
            .to_owned(),
        modern("\"2\"", "tools/call", r#""name":"echo","arguments":{}"#),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect::<String>();
    StdioLoop::new(
        Arc::new(McpServer::new(fake.clone(), "kassiber", "test")),
        Box::new(std::io::Cursor::new(input.into_bytes())),
        Box::new(sink.clone()),
    )
    .run();
    timer.join().unwrap();
    let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let mut seen = ids(&output.lines().map(str::to_owned).collect::<Vec<_>>());
    seen.sort();
    assert_eq!(seen, ["\"2\"\n", "1\n", "2\n"]);
}

#[test]
fn a_duplicate_in_flight_id_is_refused_and_a_finished_one_may_be_reused() {
    let fake = Arc::new(Fake::held());
    let released = Arc::clone(&fake);
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        released.gate.release();
    });
    let input = [
        modern("7", "tools/call", r#""name":"echo","arguments":{"n":1}"#),
        modern("7", "tools/call", r#""name":"echo","arguments":{"n":2}"#),
    ]
    .iter()
    .map(|line| format!("{line}\n"))
    .collect::<String>();
    let sink = Sink::default();
    StdioLoop::new(
        Arc::new(McpServer::new(fake.clone(), "kassiber", "test")),
        Box::new(std::io::Cursor::new(input.into_bytes())),
        Box::new(sink.clone()),
    )
    .run();
    timer.join().unwrap();
    let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    assert!(output.starts_with(
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Request id is already in flight\"},\"id\":7}\n"
    ));
    assert_eq!(output.lines().count(), 2);
    // Sequentially, the same id is fine once its call finished.
    let responses = serve_lines(
        server(Fake::open()),
        &[modern("7", "tools/call", r#""name":"echo","arguments":{}"#)],
    );
    assert_eq!(responses.len(), 1);
}

#[test]
fn malformed_lines_get_errors_and_the_loop_continues() {
    let mut input = b"not json\n\xff\xfe\n   \n\n".to_vec();
    input.extend_from_slice(format!("{}\n", modern("1", "server/discover", "")).as_bytes());
    let lines = serve_bytes(server(Fake::open()), input, SHUTDOWN_DRAIN);
    let parse_error =
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32700,\"message\":\"Parse error\"},\"id\":null}";
    assert_eq!(lines[0], parse_error);
    assert_eq!(lines[1], parse_error);
    assert_eq!(ids(&lines[2..]), ["1\n"]);
}

#[test]
fn a_final_line_without_newline_is_still_read() {
    let input = modern("1", "server/discover", "").into_bytes();
    let lines = serve_bytes(server(Fake::open()), input, SHUTDOWN_DRAIN);
    assert_eq!(ids(&lines), ["1\n"]);
}

#[test]
fn subscription_is_acknowledged_with_an_empty_filter() {
    let responses = serve_lines(
        server(Fake::open()),
        &[modern(
            "9",
            "subscriptions/listen",
            r#""notifications":{"toolsListChanged":true}"#,
        )],
    );
    assert_eq!(
        responses,
        [
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/subscriptions/acknowledged\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":9},\"notifications\":{}}}",
            "{\"jsonrpc\":\"2.0\",\"id\":9,\"result\":{\"resultType\":\"complete\",\"_meta\":{\"io.modelcontextprotocol/subscriptionId\":9,\"io.modelcontextprotocol/serverInfo\":{\"name\":\"kassiber\",\"version\":\"test\"}}}}",
        ]
    );
}

#[test]
fn subscriptions_need_the_modern_era_and_cancel_without_a_final_result() {
    let responses = serve_lines(
        server(Fake::open()),
        &[
            request("1", "subscriptions/listen", ""),
            request("2", "initialize", ""),
            request("3", "subscriptions/listen", ""),
            modern("4", "subscriptions/listen", ""),
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":4}}"#
                .to_owned(),
        ],
    );
    assert_eq!(error_code(&responses[0]), INVALID_PARAMS);
    assert!(responses[1].contains("\"protocolVersion\""));
    assert_eq!(
        responses[2],
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32601,\"message\":\"Method not found: subscriptions/listen\"},\"id\":3}"
    );
    assert!(responses[3].contains("notifications/subscriptions/acknowledged"));
    assert_eq!(
        responses.len(),
        4,
        "a cancelled stream gets no final result"
    );
}

#[test]
fn a_malformed_subscription_envelope_gets_an_error_not_a_stream() {
    let bad = modern("7", "subscriptions/listen", r#""notifications":{}"#)
        .replace("\"jsonrpc\":\"2.0\"", "\"jsonrpc\":\"invalid\"");
    let responses = serve_lines(server(Fake::open()), &[bad]);
    assert_eq!(
        responses,
        ["{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid JSON-RPC message\"},\"id\":7}"]
    );
}

#[test]
fn open_subscriptions_are_capped() {
    let requests: Vec<String> = (0..=MAX_SUBSCRIPTIONS)
        .map(|index| {
            modern(
                &(100 + index).to_string(),
                "subscriptions/listen",
                r#""notifications":{}"#,
            )
        })
        .collect();
    let responses = serve_lines(server(Fake::open()), &requests);
    let refused: Vec<&String> = responses
        .iter()
        .filter(|r| r.contains("\"error\""))
        .collect();
    assert_eq!(
        refused,
        [&format!(
            "{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32000,\"message\":\"Too many open subscriptions\",\"data\":{{\"limit\":16}}}},\"id\":{}}}",
            100 + MAX_SUBSCRIPTIONS
        )]
    );
    // Every open stream is closed in opening order at end of input.
    let closed: Vec<String> = responses
        .iter()
        .filter(|r| r.contains("\"resultType\""))
        .map(|r| line(get(&parse(r), "id").unwrap()))
        .collect();
    let expected: Vec<String> = (0..MAX_SUBSCRIPTIONS)
        .map(|index| format!("{}\n", 100 + index))
        .collect();
    assert_eq!(closed, expected);
}

#[test]
fn deeply_nested_json_is_a_parse_error_not_a_crash() {
    let mut input = vec![b'['; 200_000];
    input.extend(vec![b']'; 200_000]);
    input.push(b'\n');
    input.extend_from_slice(format!("{}\n", modern("1", "server/discover", "")).as_bytes());
    let lines = serve_bytes(server(Fake::open()), input, SHUTDOWN_DRAIN);
    assert_eq!(error_code(&lines[0]), PARSE_ERROR);
    assert_eq!(ids(&lines[1..]), ["1\n"]);
}

#[test]
fn oversized_messages_are_refused_and_skipped() {
    let mut input = vec![b'x'; MAX_MESSAGE_BYTES + 10];
    input.push(b'\n');
    input.extend_from_slice(format!("{}\n", modern("1", "server/discover", "")).as_bytes());
    let lines = serve_bytes(server(Fake::open()), input, SHUTDOWN_DRAIN);
    assert_eq!(
        lines[0],
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Message too large\"},\"id\":null}"
    );
    assert_eq!(ids(&lines[1..]), ["1\n"]);
    // A JSON string of the limit plus its newline is read and parsed, as
    // Python's `readline(limit + 1)` keeps a line that ends in time.
    let not_an_object =
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Invalid JSON-RPC message\"},\"id\":null}";
    for body in [MAX_MESSAGE_BYTES - 1, MAX_MESSAGE_BYTES] {
        let mut exact = vec![b'"'];
        exact.extend(vec![b'x'; body - 2]);
        exact.extend_from_slice(b"\"\n");
        let lines = serve_bytes(server(Fake::open()), exact, SHUTDOWN_DRAIN);
        assert_eq!(lines, [not_an_object], "{body}");
    }
    let mut over = vec![b'"'];
    over.extend(vec![b'x'; MAX_MESSAGE_BYTES - 1]);
    over.extend_from_slice(b"\"\n");
    let lines = serve_bytes(server(Fake::open()), over, SHUTDOWN_DRAIN);
    assert_eq!(
        lines,
        ["{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32600,\"message\":\"Message too large\"},\"id\":null}"]
    );
}

#[test]
fn calls_beyond_the_in_flight_limit_are_refused() {
    let fake = Arc::new(Fake::held());
    let released = Arc::clone(&fake);
    let timer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        released.gate.release();
    });
    let count = MAX_INFLIGHT_CALLS + 3;
    let input: String = (0..count)
        .map(|n| {
            format!(
                "{}\n",
                modern(
                    &n.to_string(),
                    "tools/call",
                    r#""name":"echo","arguments":{}"#
                )
            )
        })
        .collect();
    let sink = Sink::default();
    StdioLoop::new(
        Arc::new(McpServer::new(fake.clone(), "kassiber", "test")),
        Box::new(std::io::Cursor::new(input.into_bytes())),
        Box::new(sink.clone()),
    )
    .run();
    timer.join().unwrap();
    let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let responses: Vec<String> = output.lines().map(str::to_owned).collect();
    let refused: Vec<String> = responses
        .iter()
        .filter(|r| r.contains("-32000"))
        .cloned()
        .collect();
    // The first 32 were accepted while the provider held them; the rest
    // were refused at once.
    assert_eq!(refused.len(), 3);
    assert_eq!(
        refused[0],
        format!("{{\"jsonrpc\":\"2.0\",\"error\":{{\"code\":-32000,\"message\":\"Too many tool calls in flight; retry after earlier calls finish\",\"data\":{{\"limit\":32}}}},\"id\":{MAX_INFLIGHT_CALLS}}}")
    );
    assert_eq!(responses.len(), count);
    assert_eq!(fake.calls.lock().unwrap().len(), MAX_INFLIGHT_CALLS);
}

#[test]
fn non_json_constants_are_parse_errors() {
    for constant in ["NaN", "Infinity", "-Infinity"] {
        let input = format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/call\",\"params\":{{\"name\":\"echo\",\"arguments\":{{\"x\":{constant}}}}}}}\n"
        );
        let lines = serve_bytes(server(Fake::open()), input.into_bytes(), SHUTDOWN_DRAIN);
        assert_eq!(error_code(&lines[0]), -32700, "{constant}");
    }
    // A literal that overflows is a float, as in json.loads.
    let input = format!(
        "{}\n",
        request("1", "initialize", r#""protocolVersion":1e999"#)
    );
    let lines = serve_bytes(server(Fake::open()), input.into_bytes(), SHUTDOWN_DRAIN);
    assert!(lines[0].contains("\"result\""));
}

#[test]
fn a_panicking_call_is_answered_and_the_worker_survives() {
    let responses = serve_lines(
        server(Fake::open()),
        &[
            modern(
                "1",
                "tools/call",
                r#""name":"echo","arguments":{"panic":1}"#,
            ),
            modern("2", "tools/call", r#""name":"echo","arguments":{}"#),
        ],
    );
    assert_eq!(
        responses[0],
        "{\"jsonrpc\":\"2.0\",\"error\":{\"code\":-32603,\"message\":\"Internal error\"},\"id\":1}"
    );
    assert_eq!(ids(&responses[1..]), ["2\n"]);
}

#[test]
fn the_drain_is_bounded_and_unfinished_calls_get_no_reply() {
    let started = Instant::now();
    let fake = Arc::new(Fake::held());
    let input = format!(
        "{}\n{}\n",
        modern("1", "tools/call", r#""name":"echo","arguments":{}"#),
        modern("2", "subscriptions/listen", ""),
    );
    let sink = Sink::default();
    StdioLoop::new(
        Arc::new(McpServer::new(fake.clone(), "kassiber", "test")),
        Box::new(std::io::Cursor::new(input.into_bytes())),
        Box::new(sink.clone()),
    )
    .drain(Duration::from_millis(200))
    .run();
    assert!(started.elapsed() < Duration::from_secs(4));
    let output = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<String> = output.lines().map(str::to_owned).collect();
    // The subscription's acknowledgement and close; the held call, still
    // running past the drain, is never answered.
    assert_eq!(lines.len(), 2, "{output}");
    assert!(lines[1].contains("\"resultType\":\"complete\""));
    fake.gate.release();
}

#[test]
fn notifications_are_ignored() {
    let responses = serve_lines(
        server(Fake::open()),
        &[
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"tools/call","params":{"name":"echo"}}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled"}"#.to_owned(),
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":null}}"#
                .to_owned(),
        ],
    );
    assert!(responses.is_empty(), "{responses:?}");
}
