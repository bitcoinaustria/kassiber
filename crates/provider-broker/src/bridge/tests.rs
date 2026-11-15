//! Ports of the native tool bridge cases in `broker.test.ts`, plus the
//! refusals `native-tools.ts` makes that the TypeScript tests left implicit.

use super::*;
use crate::protocol::Captured;
use std::time::{Duration, Instant};

fn tools() -> Vec<BrokerToolDefinition> {
    vec![BrokerToolDefinition::new(
        "ui_reports_summary",
        "Read the summary",
        r#"{"type":"object","properties":{"b":{},"a":{}}}"#,
    )
    .unwrap()]
}

fn start() -> (ToolBridge, Captured) {
    let (events, captured) = EventSink::capture();
    (ToolBridge::start(&tools(), events).unwrap(), captured)
}

/// One request line over the socket, the reply line back (on a thread, so
/// the test can answer the call in between).
fn ask(bridge: &ToolBridge, line: String) -> std::thread::JoinHandle<Option<String>> {
    let socket = bridge.socket_path().to_owned();
    std::thread::spawn(move || {
        let mut stream = connect(&socket).ok()?;
        stream.write_all(line.as_bytes()).ok()?;
        let reply = read_line_capped(&mut stream)?;
        Some(String::from_utf8(reply).unwrap())
    })
}

fn call(call_id: &str, name: &str) -> String {
    format!("{{\"call_id\":\"{call_id}\",\"name\":\"{name}\",\"arguments\":{{\"z\": 1, \"a\": [true]}}}}\n")
}

/// Wait until the sink holds `count` events.
fn events(captured: &Captured, count: usize) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let events = captured.events();
        if events.len() >= count || Instant::now() > deadline {
            return events;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn round_trips_an_advertised_call_over_a_private_socket_and_cleans_up() {
    let (bridge, captured) = start();
    // No TCP port and no shared secret: the socket path is the whole boundary.
    assert!(!bridge.socket_path().chars().all(|c| c.is_ascii_digit()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let directory = Path::new(bridge.socket_path()).parent().unwrap();
        let mode = std::fs::metadata(directory).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o700);
    }

    let reply = ask(&bridge, call("call-1", "ui_reports_summary"));
    // The daemon answers out of band, exactly as the broker's stdin loop does.
    let seen = events(&captured, 1);
    assert_eq!(
        captured.text(),
        "{\"type\":\"tool_call\",\"call_id\":\"call-1\",\"name\":\"ui_reports_summary\",\"arguments\":{\"z\":1,\"a\":[true]}}\n"
    );
    assert_eq!(seen.len(), 1);
    bridge.resolve(&[serde_json::json!({"call_id": "call-1", "output": "{\"ok\":true}"})]);
    assert_eq!(
        reply.join().unwrap().as_deref(),
        Some(r#"{"output":"{\"ok\":true}"}"#)
    );

    let socket = bridge.socket_path().to_owned();
    bridge.close().unwrap();
    #[cfg(unix)]
    {
        assert!(!Path::new(&socket).exists());
        assert!(!Path::new(&socket).parent().unwrap().exists());
    }
    let _ = socket;
}

#[test]
fn refuses_a_tool_outside_the_advertised_catalog() {
    let (bridge, captured) = start();
    let reply = ask(&bridge, call("call-1", "ui_wallets_sync"))
        .join()
        .unwrap()
        .unwrap();
    let reply: Value = serde_json::from_str(&reply).unwrap();
    assert_eq!(reply.get("output"), None);
    assert_eq!(
        reply["error"],
        "Error: Provider requested a tool outside the advertised catalog."
    );
    // Nothing reached the daemon.
    assert_eq!(captured.text(), "");
    // A missing name is outside the catalog too.
    let reply = ask(&bridge, "{\"call_id\":\"x\",\"arguments\":{}}\n".into())
        .join()
        .unwrap()
        .unwrap();
    assert!(reply.contains("outside the advertised catalog"), "{reply}");
}

#[test]
fn refuses_a_repeated_pending_call_id() {
    let (bridge, captured) = start();
    let first = ask(&bridge, call("same", "ui_reports_summary"));
    events(&captured, 1);
    let second = ask(&bridge, call("same", "ui_reports_summary"))
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(
        second,
        r#"{"error":"Error: Provider repeated a pending tool call id."}"#
    );
    bridge.resolve(&[serde_json::json!({"call_id": "same", "output": "done"})]);
    assert_eq!(
        first.join().unwrap().as_deref(),
        Some(r#"{"output":"done"}"#)
    );
    // Answered calls free their id.
    let third = ask(&bridge, call("same", "ui_reports_summary"));
    events(&captured, 2);
    bridge.resolve(&[serde_json::json!({"call_id": "same", "output": "again"})]);
    assert_eq!(
        third.join().unwrap().as_deref(),
        Some(r#"{"output":"again"}"#)
    );
}

#[test]
fn ignores_results_for_unknown_calls_and_passes_a_missing_output_as_null() {
    let (bridge, captured) = start();
    let reply = ask(&bridge, call("c", "ui_reports_summary"));
    events(&captured, 1);
    bridge.resolve(&[
        serde_json::json!({"call_id": "other", "output": "x"}),
        serde_json::json!({"output": "no id"}),
        serde_json::json!(null),
        serde_json::json!({"call_id": "c"}),
    ]);
    assert_eq!(reply.join().unwrap().as_deref(), Some(r#"{"output":null}"#));
}

#[test]
fn closing_rejects_pending_calls() {
    let (bridge, captured) = start();
    let reply = ask(&bridge, call("pending", "ui_reports_summary"));
    events(&captured, 1);
    bridge.close().unwrap();
    assert_eq!(
        reply.join().unwrap().as_deref(),
        Some(r#"{"error":"Error: Kassiber closed the tool bridge."}"#)
    );
    // A direct request after close is refused, never left waiting.
    let arguments = RawValue::from_string("{}".into()).unwrap();
    assert_eq!(
        bridge
            .request("ui_reports_summary", &arguments, None)
            .unwrap_err()
            .message(),
        "Kassiber closed the tool bridge."
    );
}

#[test]
fn closes_a_connection_that_sends_too_much_or_no_json() {
    let (bridge, captured) = start();
    let flood = format!("{}\n", "x".repeat(MAX_BRIDGE_MESSAGE_BYTES + 10));
    assert_eq!(ask(&bridge, flood).join().unwrap(), None);
    assert_eq!(ask(&bridge, "not json\n".into()).join().unwrap(), None);
    assert_eq!(captured.text(), "");
}

#[test]
fn a_direct_request_gets_a_random_call_id_and_the_daemon_output() {
    let (events_sink, captured) = EventSink::capture();
    let bridge = Arc::new(ToolBridge::start(&tools(), events_sink).unwrap());
    let asking = Arc::clone(&bridge);
    let handle = std::thread::spawn(move || {
        let arguments = RawValue::from_string("{}".into()).unwrap();
        asking.request("ui_reports_summary", &arguments, None)
    });
    let seen = events(&captured, 1);
    let call_id = seen[0]["call_id"].as_str().unwrap().to_owned();
    assert_eq!(call_id.len(), 36);
    bridge.resolve(&[serde_json::json!({"call_id": call_id, "output": "ok"})]);
    assert_eq!(handle.join().unwrap().unwrap(), Value::String("ok".into()));
    let arguments = RawValue::from_string("{}".into()).unwrap();
    assert_eq!(
        bridge
            .request("other", &arguments, Some("x"))
            .unwrap_err()
            .message(),
        "Provider requested a tool outside the advertised catalog."
    );
}

#[test]
fn mcp_command_writes_a_private_manifest_and_names_this_executable() {
    let (bridge, _captured) = start();
    let cwd = tempfile::tempdir().unwrap();
    let command = mcp_command(cwd.path(), &tools(), &bridge).unwrap();
    let manifest = cwd.path().join(MANIFEST_NAME);
    assert_eq!(
        command,
        [
            std::env::current_exe()
                .unwrap()
                .to_str()
                .unwrap()
                .to_owned(),
            "--provider-broker-mcp".to_owned(),
            bridge.socket_path().to_owned(),
            manifest.to_str().unwrap().to_owned(),
        ]
    );
    assert_eq!(
        std::fs::read_to_string(&manifest).unwrap(),
        r#"[{"name":"ui_reports_summary","description":"Read the summary","parameters":{"type":"object","properties":{"b":{},"a":{}}}}]"#
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&manifest).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}
