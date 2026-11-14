//! The MCP child against a real bridge socket, through `kassiber_mcp`'s
//! request handler.

use super::*;
use crate::bridge::{mcp_command, ToolBridge};
use crate::protocol::{BrokerToolDefinition, Captured, EventSink};
use std::time::{Duration, Instant};

fn tools() -> Vec<BrokerToolDefinition> {
    let mut read = BrokerToolDefinition::new(
        "ui_reports_summary",
        "Read the summary",
        r#"{"type":"object","properties":{"year":{"type":"integer"},"book":{"type":"string"}},"required":["year"]}"#,
    )
    .unwrap();
    read.read_only = Some(serde_json::Value::Bool(true));
    read.destructive = Some(serde_json::Value::Bool(false));
    let mut write =
        BrokerToolDefinition::new("ui_review_apply", "Apply", r#"{"type":"object"}"#).unwrap();
    write.destructive = Some(serde_json::Value::Bool(true));
    vec![read, write]
}

fn json(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

fn text(value: &Value) -> String {
    let options = Options {
        ensure_ascii: false,
        separators: Some((",".into(), ":".into())),
        ..Options::default()
    };
    kassiber_pyjson::dumps(value, &options).unwrap()
}

struct Fixture {
    bridge: Arc<ToolBridge>,
    captured: Captured,
    server: Arc<McpServer>,
    _cwd: tempfile::TempDir,
}

fn fixture(manifest_tools: &[BrokerToolDefinition]) -> Fixture {
    let (events, captured) = EventSink::capture();
    let bridge = Arc::new(ToolBridge::start(&tools(), events).unwrap());
    let cwd = tempfile::tempdir().unwrap();
    let command = mcp_command(cwd.path(), manifest_tools, &bridge).unwrap();
    assert_eq!(command[1], "--provider-broker-mcp");
    let provider = BridgeTools::load(&command[2], Path::new(&command[3])).unwrap();
    let server = Arc::new(McpServer::new(
        Arc::new(provider),
        SERVER_NAME,
        SERVER_VERSION,
    ));
    let init = server
        .handle(
            &json(r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"claude-code","version":"1"}}}"#),
            None,
        )
        .unwrap();
    let init = text(&init);
    assert!(
        init.contains(r#""serverInfo":{"name":"kassiber","version":"1.0.0"}"#),
        "{init}"
    );
    assert!(
        init.contains(&format!(r#""instructions":"{INSTRUCTIONS}""#)),
        "{init}"
    );
    Fixture {
        bridge,
        captured,
        server,
        _cwd: cwd,
    }
}

fn call(server: &Arc<McpServer>, name: &str, arguments: &str) -> std::thread::JoinHandle<String> {
    let server = Arc::clone(server);
    let message = json(&format!(
        r#"{{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{{"name":"{name}","arguments":{arguments}}}}}"#
    ));
    std::thread::spawn(move || text(&server.handle(&message, None).unwrap()))
}

fn wait_for_event(captured: &Captured) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(event) = captured.events().pop() {
            return event;
        }
        assert!(Instant::now() < deadline, "no tool_call event");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn lists_the_manifest_with_its_schemas_and_hints() {
    let fixture = fixture(&tools());
    let listed = fixture
        .server
        .handle(
            &json(r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#),
            None,
        )
        .unwrap();
    assert_eq!(
        text(&listed),
        concat!(
            r#"{"jsonrpc":"2.0","id":1,"result":{"tools":["#,
            r#"{"name":"ui_reports_summary","description":"Read the summary","inputSchema":{"type":"object","properties":{"year":{"type":"integer"},"book":{"type":"string"}},"required":["year"]},"annotations":{"readOnlyHint":true,"destructiveHint":false}},"#,
            r#"{"name":"ui_review_apply","description":"Apply","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":false,"destructiveHint":true}}"#,
            r#"]}}"#
        )
    );
}

#[test]
fn a_call_reaches_the_bridge_and_returns_the_daemons_output() {
    let fixture = fixture(&tools());
    let reply = call(
        &fixture.server,
        "ui_reports_summary",
        r#"{"year":2025,"book":"Bücher"}"#,
    );
    let event = wait_for_event(&fixture.captured);
    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["name"], "ui_reports_summary");
    // The arguments cross unchanged, in their order.
    assert!(fixture
        .captured
        .text()
        .contains(r#""arguments":{"year":2025,"book":"Bücher"}"#));
    let call_id = event["call_id"].as_str().unwrap();
    fixture
        .bridge
        .resolve(&[serde_json::json!({"call_id": call_id, "output": "{\"total\":\"0.1\"}"})]);
    assert_eq!(
        reply.join().unwrap(),
        r#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"{\"total\":\"0.1\"}"}]}}"#
    );
}

#[test]
fn an_unadvertised_name_is_refused() {
    let fixture = fixture(&tools());
    let reply = call(&fixture.server, "ui_wallets_sync", "{}")
        .join()
        .unwrap();
    assert!(reply.contains(r#""code":-32602"#), "{reply}");
    assert!(reply.contains("Unknown tool: ui_wallets_sync"), "{reply}");
    assert_eq!(fixture.captured.text(), "");
}

#[test]
fn the_broker_refuses_a_name_the_manifest_added() {
    // A manifest naming more than the request advertised still cannot reach
    // the daemon: the broker checks its own catalog.
    let mut widened = tools();
    widened.push(
        BrokerToolDefinition::new("ui_wallets_sync", "Sync", r#"{"type":"object"}"#).unwrap(),
    );
    let fixture = fixture(&widened);
    let reply = call(&fixture.server, "ui_wallets_sync", "{}")
        .join()
        .unwrap();
    assert_eq!(
        reply,
        r#"{"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"Error: Provider requested a tool outside the advertised catalog."}],"isError":true}}"#
    );
    assert_eq!(fixture.captured.text(), "");
}

#[test]
fn a_closed_bridge_is_a_tool_error() {
    let fixture = fixture(&tools());
    let pending = call(&fixture.server, "ui_reports_summary", "{}");
    wait_for_event(&fixture.captured);
    fixture.bridge.close().unwrap();
    let reply = pending.join().unwrap();
    assert!(
        reply.contains(r#""text":"Error: Kassiber closed the tool bridge."}],"isError":true"#),
        "{reply}"
    );
    // Gone entirely: the connect itself fails.
    let reply = call(&fixture.server, "ui_reports_summary", "{}")
        .join()
        .unwrap();
    assert!(reply.contains(r#""isError":true"#), "{reply}");
    assert!(reply.contains("connect ENOENT "), "{reply}");
}

#[test]
fn a_bad_manifest_is_refused() {
    let cwd = tempfile::tempdir().unwrap();
    let manifest = cwd.path().join("m.json");
    for body in [
        "{}",
        "not json",
        r#"[{"description":"x","parameters":{}}]"#,
        r#"[{"name":"a"}]"#,
    ] {
        std::fs::write(&manifest, body).unwrap();
        assert!(BridgeTools::load("socket", &manifest).is_err(), "{body}");
    }
    std::fs::write(
        &manifest,
        r#"[{"name":"a","parameters":{}},{"name":"a","parameters":{}}]"#,
    )
    .unwrap();
    assert_eq!(
        BridgeTools::load("socket", &manifest)
            .err()
            .unwrap()
            .message(),
        "Tool a is already registered"
    );
}

#[test]
fn error_text_follows_javascript_string_conversion() {
    use serde_json::json;
    assert_eq!(js_string(&json!("x")), "x");
    assert_eq!(js_string(&json!(5)), "5");
    assert_eq!(js_string(&json!(true)), "true");
    assert_eq!(js_string(&json!({"a": 1})), "[object Object]");
    assert_eq!(js_string(&json!([1, null, "b"])), "1,,b");
}
