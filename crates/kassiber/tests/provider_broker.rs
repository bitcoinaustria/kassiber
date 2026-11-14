//! `kassiber --provider-broker` and `kassiber --provider-broker-mcp` as the
//! daemon and a provider CLI start them (plan 23): the JSONL status answer
//! for a runtime whose CLI cannot be found, a malformed scope refused,
//! termination signals, and the MCP child forwarding a call through a real
//! tool-bridge socket.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Output, Stdio};
use std::time::{Duration, Instant};

use kassiber_provider_broker::bridge::mcp_command;
use kassiber_provider_broker::protocol::{BrokerToolDefinition, EventSink};
use kassiber_provider_broker::ToolBridge;
use serde_json::{json, Value};

/// The product binary with nothing of this process's environment but a
/// `PATH` holding no provider CLI, a private `HOME` and temporary root.
fn broker(home: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_kassiber"));
    let empty = home.join("empty-bin");
    std::fs::create_dir_all(&empty).unwrap();
    command
        .args(args)
        .env_clear()
        .env("PATH", &empty)
        .env("HOME", home)
        .env("TMPDIR", home)
        .env("KASSIBER_AI_BROKER_TMPDIR", home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn run(home: &Path, request: &str) -> Output {
    let mut child = broker(home, &["--provider-broker"]).spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// The CLI each runtime looks for, and the status the broker reports when
/// it is missing (the TypeScript adapters' wording).
const MISSING: [(&str, &str, &str, &str); 4] = [
    (
        "codex",
        "Codex",
        "codex",
        "Install Codex, then run `codex login` outside Kassiber.",
    ),
    (
        "claude",
        "Claude",
        "claude",
        "Install Claude Code, then run `claude login` outside Kassiber.",
    ),
    (
        "opencode",
        "OpenCode",
        "opencode",
        "Install OpenCode, then run `opencode auth login` outside Kassiber.",
    ),
    (
        "copilot",
        "GitHub Copilot",
        "copilot",
        "Install the GitHub Copilot CLI, then sign in outside Kassiber.",
    ),
];

#[test]
fn a_status_scoped_to_a_missing_cli_reports_it_and_starts_nothing_else() {
    let home = tempfile::tempdir().unwrap();
    // The lookup also tries fixed system directories; use a runtime this
    // machine does not have there.
    let Some((provider, display_name, _, message)) = MISSING.into_iter().find(|(_, _, cli, _)| {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .iter()
            .all(|dir| !Path::new(dir).join(cli).exists())
    }) else {
        return;
    };
    let out = run(
        home.path(),
        &format!("{{\"command\": \"status\", \"provider\": \"{provider}\"}}\n"),
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    // Byte for byte, in the TypeScript broker's field order.
    assert_eq!(
        stdout,
        format!(
            "{{\"type\":\"result\",\"data\":[{{\"provider\":\"{provider}\",\"display_name\":\"{display_name}\",\"privacy_posture\":\"remote\",\"native_tools\":\"disabled\",\"models\":[],\"state\":\"missing_executable\",\"message\":{}}}]}}\n",
            json!(message)
        )
    );
    assert!(out.stderr.is_empty());
    // The probe's working directory is gone.
    let left: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("kassiber-ai-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

#[test]
fn a_malformed_scope_is_refused_as_one_error_event() {
    let home = tempfile::tempdir().unwrap();
    for request in [
        "{\"command\": \"status\", \"providers\": [\"codex\", \"gemini\"]}\n",
        "{\"command\": \"status\", \"provider\": null}\n",
        "{\"command\": \"models\", \"provider\": \"gemini\"}\n",
    ] {
        let out = run(home.path(), request);
        assert_eq!(out.status.code(), Some(1), "{request}");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "{\"type\":\"error\",\"code\":\"provider_error\",\"message\":\"Unknown provider.\"}\n",
            "{request}"
        );
    }
    let out = run(home.path(), "");
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"type\":\"error\",\"code\":\"provider_error\",\"message\":\"Kassiber provider broker received no request.\"}\n"
    );
}

#[test]
fn a_chat_on_a_missing_cli_reads_as_not_installed() {
    let home = tempfile::tempdir().unwrap();
    let Some((provider, display_name, _, _)) = MISSING.into_iter().find(|(_, _, cli, _)| {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .iter()
            .all(|dir| !Path::new(dir).join(cli).exists())
    }) else {
        return;
    };
    let out = run(
        home.path(),
        &format!(
            "{{\"command\": \"chat\", \"provider\": \"{provider}\", \"model\": \"m\", \"messages\": [{{\"role\": \"user\", \"content\": \"hi\"}}]}}\n"
        ),
    );
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!(
            "{{\"type\":\"error\",\"code\":\"missing_executable\",\"message\":\"{display_name} is not installed.\"}}\n"
        )
    );
}

/// Whether the process catches SIGTERM yet (its handlers are installed).
fn catches_sigterm(pid: u32) -> bool {
    let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return false;
    };
    status
        .lines()
        .find_map(|line| line.strip_prefix("SigCgt:"))
        .and_then(|mask| u64::from_str_radix(mask.trim(), 16).ok())
        .is_some_and(|mask| mask & (1 << (15 - 1)) != 0)
}

fn signal(pid: u32, name: &str) {
    let status = Command::new("/bin/kill")
        .args([format!("-{name}"), pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

#[test]
fn termination_signals_exit_with_128_plus_the_signal() {
    let home = tempfile::tempdir().unwrap();
    for (name, code) in [("TERM", 143), ("HUP", 129), ("INT", 130)] {
        // Waiting for its request: stdin stays open.
        let mut child: Child = broker(home.path(), &["--provider-broker"]).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        if Path::new("/proc/self/status").exists() {
            while !catches_sigterm(child.id()) {
                assert!(Instant::now() < deadline, "the broker never caught SIGTERM");
                std::thread::sleep(Duration::from_millis(10));
            }
        } else {
            std::thread::sleep(Duration::from_secs(1));
        }
        signal(child.id(), name);
        let status = child.wait().unwrap();
        assert_eq!(status.code(), Some(code), "{name}");
    }
}

/// One JSON-RPC exchange with the MCP child.
fn exchange(
    stdin: &mut std::process::ChildStdin,
    stdout: &mut BufReader<std::process::ChildStdout>,
    message: Value,
) -> Value {
    stdin.write_all(format!("{message}\n").as_bytes()).unwrap();
    stdin.flush().unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}

#[test]
fn the_mcp_child_forwards_calls_through_the_bridge_socket() {
    let tools = vec![BrokerToolDefinition::new(
        "ui_reports_summary",
        "Read the summary",
        r#"{"type":"object","properties":{"year":{"type":"integer"}}}"#,
    )
    .unwrap()];
    let (events, captured) = EventSink::capture();
    let bridge = std::sync::Arc::new(ToolBridge::start(&tools, events).unwrap());
    let cwd = tempfile::tempdir().unwrap();
    let command = mcp_command(cwd.path(), &tools, &bridge).unwrap();
    let home = tempfile::tempdir().unwrap();
    let mut child = broker(
        home.path(),
        &["--provider-broker-mcp", &command[2], &command[3]],
    )
    .spawn()
    .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    let init = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": "2025-06-18", "capabilities": {},
            "clientInfo": {"name": "fake-cli", "version": "1"}}}),
    );
    assert_eq!(
        init["result"]["serverInfo"],
        json!({"name": "kassiber", "version": "1.0.0"})
    );
    assert_eq!(
        init["result"]["instructions"],
        "Typed, capability-scoped Kassiber accounting tools."
    );
    stdin
        .write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n")
        .unwrap();

    let listed = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    assert_eq!(
        listed["result"]["tools"],
        json!([{
            "name": "ui_reports_summary",
            "description": "Read the summary",
            "inputSchema": {"type": "object", "properties": {"year": {"type": "integer"}}},
            "annotations": {"readOnlyHint": false, "destructiveHint": false},
        }])
    );

    // The call blocks until the daemon answers through the bridge.
    let answering = std::sync::Arc::clone(&bridge);
    let answer = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(event) = captured.events().pop() {
                let call_id = event["call_id"].as_str().unwrap().to_owned();
                answering.resolve(&[json!({"call_id": call_id, "output": "{\"total\":\"1\"}"})]);
                return event;
            }
            assert!(Instant::now() < deadline, "no tool_call event");
            std::thread::sleep(Duration::from_millis(5));
        }
    });
    let called = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "ui_reports_summary", "arguments": {"year": 2025}}}),
    );
    let event = answer.join().unwrap();
    assert_eq!(event["type"], "tool_call");
    assert_eq!(event["name"], "ui_reports_summary");
    assert_eq!(event["arguments"], json!({"year": 2025}));
    assert_eq!(
        called["result"],
        json!({"content": [{"type": "text", "text": "{\"total\":\"1\"}"}]})
    );

    let refused = exchange(
        &mut stdin,
        &mut stdout,
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
               "params": {"name": "ui_wallets_sync", "arguments": {}}}),
    );
    assert_eq!(refused["error"]["code"], -32602);
    assert_eq!(refused["error"]["message"], "Unknown tool: ui_wallets_sync");

    drop(stdin);
    assert_eq!(child.wait().unwrap().code(), Some(0));
    bridge.close().unwrap();
}

#[test]
fn the_mcp_child_refuses_a_missing_manifest() {
    let home = tempfile::tempdir().unwrap();
    let manifest = home.path().join("absent.json");
    let out = broker(
        home.path(),
        &[
            "--provider-broker-mcp",
            "/nonexistent.sock",
            manifest.to_str().unwrap(),
        ],
    )
    .output()
    .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.starts_with("ENOENT: "), "{stderr}");
}

/// A fake `opencode` in `dir/bin`: version and model listing from fixed
/// text, and an OpenCode 2 `serve` announcing `port` and waiting for its
/// stdin to close. It records the names of its environment per run.
fn fake_opencode(dir: &Path, version: &str, models: &str, port: u16) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(dir.join("models"), models).unwrap();
    let script = format!(
        r#"#!/bin/sh
root='{root}'
case "$1" in
--version) echo '{version}' ;;
debug) echo '{{}}' ;;
models) cat "$root/models" ;;
serve)
  export -p > "$root/serve-env"
  printf '%s\n' "$@" > "$root/serve-argv"
  printf '{{"url":"http://127.0.0.1:%s"}}\n' '{port}'
  while read -r _; do :; done ;;
*) exit 2 ;;
esac
"#,
        root = dir.display()
    );
    let executable = bin.join("opencode");
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

fn run_with_path(home: &Path, bin: &Path, request: &str) -> Output {
    let mut child = broker(home, &["--provider-broker"])
        .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(request.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

#[test]
fn opencode_status_reports_a_ready_cli_and_one_that_needs_a_login() {
    let home = tempfile::tempdir().unwrap();
    let bin = fake_opencode(home.path(), "1.18.10", "provider/model\n", 1);
    let request = "{\"command\": \"status\", \"provider\": \"opencode\"}\n";
    let out = run_with_path(home.path(), &bin, request);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"type\":\"result\",\"data\":[{\"provider\":\"opencode\",\"display_name\":\"OpenCode\",\"privacy_posture\":\"remote\",\"native_tools\":\"disabled\",\"models\":[{\"id\":\"provider/model\",\"display_name\":\"model\",\"owned_by\":\"OpenCode · provider\",\"source_provider\":\"provider\",\"privacy_posture\":\"remote\",\"privacy_reason\":\"OpenCode does not report an endpoint for this provider.\",\"supports_reasoning_effort\":false,\"reasoning_efforts\":[]}],\"state\":\"ready\",\"message\":\"Ready using the existing OpenCode configuration.\",\"version\":\"1.18.10\",\"executable\":\"opencode\"}]}\n"
    );
    std::fs::write(home.path().join("models"), "").unwrap();
    let out = run_with_path(home.path(), &bin, request);
    let events: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(events["data"][0]["state"], "authentication_required");
    assert_eq!(
        events["data"][0]["message"],
        "Run `opencode auth login` outside Kassiber."
    );
}

#[test]
fn an_opencode_2_turn_streams_through_the_binary() {
    use kassiber_net::test_support::{FakeService, Reply};
    use std::sync::{mpsc, Arc, Mutex};
    if Path::new("/etc/opencode").exists() {
        // OpenCode 2 refuses machine-managed configuration.
        return;
    }
    let feed: Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>> = Arc::default();
    let emit = |feed: &Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>, value: Value| {
        if let Some(sender) = feed.lock().unwrap().as_ref() {
            let _ = sender.send(format!("data: {value}\n\n").into_bytes());
        }
    };
    let events = feed.clone();
    let service = FakeService::start(move |request| {
        let path = request.target.split('?').next().unwrap_or_default();
        match path {
            "/api/session" => Reply::json(200, "{\"data\":{\"id\":\"ses_1\"}}"),
            "/api/session/ses_1" => {
                Reply::json(200, "{\"data\":{\"id\":\"ses_1\",\"outcome\":\"succeeded\"}}")
            }
            "/api/event" => {
                let (sender, pieces) = mpsc::channel();
                *events.lock().unwrap() = Some(sender);
                emit(&events, json!({"type": "server.connected", "data": {}}));
                Reply::fed("text/event-stream", pieces)
            }
            "/api/session/ses_1/prompt" => {
                emit(&events, json!({"type": "session.text.delta", "data": {"sessionID": "ses_1", "delta": "Hello"}}));
                emit(&events, json!({"type": "session.idle", "data": {"sessionID": "ses_1"}}));
                Reply::json(200, "{\"data\":{}}")
            }
            _ => Reply::json(404, "{}"),
        }
    })
    .unwrap();
    let home = tempfile::tempdir().unwrap();
    let bin = fake_opencode(home.path(), "opencode v2.2.0", "", service.port());
    let out = run_with_path(
        home.path(),
        &bin,
        "{\"command\":\"chat\",\"provider\":\"opencode\",\"model\":\"fixture/model\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}\n",
    );
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "{\"type\":\"status\",\"phase\":\"connecting\",\"message\":\"Starting OpenCode server\"}\n{\"type\":\"delta\",\"content\":\"Hello\"}\n{\"type\":\"done\",\"finish_reason\":\"stop\"}\n"
    );
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        std::fs::read_to_string(home.path().join("serve-argv")).unwrap(),
        "serve\n--stdio\n--hostname=127.0.0.1\n--port=0\n"
    );
    let env = std::fs::read_to_string(home.path().join("serve-env")).unwrap();
    for name in [
        "OPENCODE_SERVER_PASSWORD",
        "OPENCODE_CONFIG_CONTENT",
        "KASSIBER_AI_BROKER",
    ] {
        assert!(env.contains(name), "{name}");
    }
    let paths = service.paths();
    assert_eq!(paths[0], "POST /api/session");
    assert_eq!(paths[1], "GET /api/event");
    assert!(paths.contains(&"POST /api/session/ses_1/prompt".to_owned()));
    // The turn's directory is gone.
    let left: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("kassiber-ai-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
}

// GitHub Copilot over ACP against a fake `copilot` (plan 23, step 2).
#[path = "provider_broker/acp.rs"]
mod acp;
