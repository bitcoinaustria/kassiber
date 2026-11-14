//! `kassiber --provider-broker` driving GitHub Copilot over ACP, end to end:
//! the product binary as the daemon starts it, a fake `copilot` on `PATH`
//! replaying the ACP wire protocol (`fake-copilot.sh`), and for tool calls
//! the real MCP child (`kassiber --provider-broker-mcp`) started by that
//! fake from `--additional-mcp-config`, forwarding through the tool bridge.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::{json, Value};

const FAKE_SCRIPT: &str = include_str!("../../../provider-broker/src/acp/fake-copilot.sh");

/// A fake `copilot` in a directory of its own and the transcript it replays.
struct Fake {
    dir: tempfile::TempDir,
    steps: Vec<String>,
}

impl Fake {
    fn new() -> Self {
        let dir = tempfile::Builder::new()
            .prefix("acp-bin-")
            .tempdir()
            .unwrap();
        let script = dir.path().join("copilot");
        std::fs::write(&script, FAKE_SCRIPT).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(dir.path().join("version"), "GitHub Copilot CLI 1.0.93.\n").unwrap();
        Self {
            dir,
            steps: Vec::new(),
        }
    }

    fn step(mut self, step: impl Into<String>) -> Self {
        self.steps.push(step.into());
        self
    }

    fn read(self) -> Self {
        self.step("<")
    }

    fn send(self, message: Value) -> Self {
        self.step(format!("> {message}"))
    }

    fn reply(self, id: u64, result: Value) -> Self {
        self.send(json!({"jsonrpc": "2.0", "id": id, "result": result}))
    }

    /// `initialize` and `session/new` answered.
    fn handshake(self, session: Value) -> Self {
        self.read()
            .reply(0, json!({"protocolVersion": 1, "agentCapabilities": {}}))
            .read()
            .reply(1, session)
    }

    fn update(self, update: Value) -> Self {
        self.send(json!({"jsonrpc": "2.0", "method": "session/update",
                         "params": {"sessionId": "s1", "update": update}}))
    }

    fn write(&self) {
        let mut transcript = self.steps.join("\n");
        transcript.push('\n');
        std::fs::write(self.dir.path().join("transcript"), transcript).unwrap();
    }

    fn recorded(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.dir.path().join(name)).ok()
    }
}

/// [`super::broker`] (the product binary as the daemon starts it, with a
/// cleared environment, a private `HOME`, temporary root and broker root)
/// with the fake first on `PATH` (then the system tools the script uses),
/// and variables Copilot must never see.
fn broker(fake: &Fake, home: &Path) -> Command {
    fake.write();
    let path = format!("{}:/usr/bin:/bin", fake.dir.path().display());
    let mut command = super::broker(home, &["--provider-broker"]);
    command
        .env("PATH", path)
        // The tool bridge's socket path must stay within the AF_UNIX limit;
        // this process's temporary directory is short enough.
        .env("TMPDIR", std::env::temp_dir())
        .env("COPILOT_ALLOW_ALL", "true")
        .env("NODE_OPTIONS", "--require /tmp/evil.js")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    command
}

/// Run one request; answer each `tool_call` with `output`. Returns the
/// events and the exit code.
fn exchange(mut command: Command, request: Value, output: &str) -> (Vec<Value>, Option<i32>) {
    let mut child = command.spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(format!("{request}\n").as_bytes()).unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut events = Vec::new();
    for line in stdout.lines() {
        let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if event["type"] == "tool_call" {
            let answer = json!({"command": "tool_results",
                                "results": [{"call_id": event["call_id"], "output": output}]});
            stdin.write_all(format!("{answer}\n").as_bytes()).unwrap();
        }
        events.push(event);
    }
    drop(stdin);
    (events, child.wait().unwrap().code())
}

fn tool() -> Value {
    json!({"name": "status", "description": "Return the Kassiber book status.",
           "parameters": {"type": "object", "properties": {}, "additionalProperties": false},
           "read_only": true})
}

fn chat(options: Value, tools: Value) -> Value {
    json!({"command": "chat", "request_id": "r1", "provider": "copilot", "model": "default",
           "messages": [{"role": "user", "content": "How is my book?"}],
           "tools": tools, "options": options})
}

fn status_request() -> Value {
    json!({"command": "status", "provider": "copilot"})
}

#[test]
fn a_copilot_status_probe_reports_ready_from_a_private_turn_directory() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new().handshake(json!({"sessionId": "s1", "configOptions": [{
        "type": "select", "id": "model", "name": "Model", "category": "model",
        "options": [{"value": "gpt-x", "name": "GPT X"}]}]}));
    let (events, code) = exchange(broker(&fake, home.path()), status_request(), "");
    assert_eq!(code, Some(0), "{events:?}");
    let status = &events[0]["data"][0];
    assert_eq!(events[0]["type"], "result");
    assert_eq!(status["state"], "ready", "{status}");
    assert_eq!(status["executable"], "copilot");
    assert_eq!(status["version"], "GitHub Copilot CLI 1.0.93.");
    assert_eq!(
        status["models"]
            .as_array()
            .unwrap()
            .iter()
            .map(|model| model["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["default", "gpt-x"]
    );
    // The probe ran in a fresh directory under the daemon's root, now gone,
    // with Copilot's home inside it.
    let cwd = PathBuf::from(fake.recorded("cwd").unwrap().trim_end());
    assert_eq!(
        cwd.parent().unwrap().canonicalize().unwrap(),
        home.path().canonicalize().unwrap()
    );
    let name = cwd.file_name().unwrap().to_str().unwrap();
    assert!(name.starts_with("kassiber-ai-copilot-"), "{name}");
    assert!(!cwd.exists());
    assert_eq!(
        fake.recorded("home").unwrap().trim_end(),
        cwd.join("copilot-home").to_str().unwrap()
    );
    // Neither the allow-all switch nor Node's options cross.
    assert_eq!(fake.recorded("allow-all").unwrap(), "unset\n");
    let env = fake.recorded("env").unwrap();
    assert!(!env.lines().any(|name| name == "NODE_OPTIONS"), "{env}");
    assert!(env.lines().any(|name| name == "KASSIBER_AI_BROKER"));
}

#[test]
fn a_relative_broker_root_is_ignored() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new().handshake(json!({"sessionId": "s1"}));
    let mut command = broker(&fake, home.path());
    command
        .env("KASSIBER_AI_BROKER_TMPDIR", "relative/path")
        .env("TMPDIR", home.path());
    let (events, _) = exchange(command, status_request(), "");
    assert_eq!(events[0]["data"][0]["state"], "ready");
    let cwd = PathBuf::from(fake.recorded("cwd").unwrap().trim_end());
    assert_eq!(
        cwd.parent().unwrap().canonicalize().unwrap(),
        home.path().canonicalize().unwrap(),
        "the system temporary directory (TMPDIR)"
    );
}

#[test]
fn a_copilot_status_probe_reports_a_sign_in_problem() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new()
        .read()
        .reply(0, json!({"protocolVersion": 1}))
        .read()
        .send(json!({"jsonrpc": "2.0", "id": 1,
                     "error": {"code": -32000, "message": "Authentication required"}}));
    let (events, code) = exchange(broker(&fake, home.path()), status_request(), "");
    assert_eq!(code, Some(0));
    let status = &events[0]["data"][0];
    assert_eq!(status["state"], "authentication_required");
    assert_eq!(status["message"], "Run `copilot login` outside Kassiber.");
}

#[test]
fn a_copilot_turn_calls_a_kassiber_tool_through_the_mcp_child_and_the_bridge() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new()
        .handshake(json!({"sessionId": "s1"}))
        .read()
        .update(json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
                       "title": "kassiber-status", "kind": "other", "status": "pending"}))
        .step(r#"mcp status {"year":2025}"#)
        .update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "t1", "status": "completed"}),
        )
        .update(json!({"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": "Your book is fine."}}))
        .reply(2, json!({"stopReason": "end_turn"}));
    let (events, code) = exchange(
        broker(&fake, home.path()),
        chat(json!({}), json!([tool()])),
        "{\"synthetic\":\"ok\"}",
    );
    assert_eq!(code, Some(0), "{events:?}");
    assert_eq!(
        events,
        [
            json!({"type": "status", "phase": "connecting", "message": "Starting GitHub Copilot"}),
            json!({"type": "status", "phase": "streaming", "message": "GitHub Copilot is responding"}),
            json!({"type": "tool_call", "call_id": events[2]["call_id"], "name": "status",
                   "arguments": {"year": 2025}}),
            json!({"type": "delta", "content": "Your book is fine."}),
            json!({"type": "done", "finish_reason": "stop"}),
        ]
    );
    // The MCP child returned the daemon's output to the agent.
    let mcp = fake.recorded("mcp-out").unwrap();
    assert!(mcp.contains(r#"{\"synthetic\":\"ok\"}"#), "{mcp}");
    let argv = fake.recorded("argv").unwrap();
    let argv: Vec<&str> = argv.lines().collect();
    assert_eq!(
        &argv[argv.len() - 2..],
        ["--available-tools", "kassiber-status"]
    );
}

#[test]
fn a_copilot_turn_that_starts_a_native_tool_is_stopped() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new()
        .handshake(json!({"sessionId": "s1"}))
        .read()
        .update(json!({"sessionUpdate": "tool_call", "toolCallId": "t1",
                       "title": "Shell: cat ~/.ssh/id_rsa", "kind": "execute"}));
    let (events, code) = exchange(
        broker(&fake, home.path()),
        chat(json!({}), json!([tool()])),
        "",
    );
    assert_eq!(code, Some(1));
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "error", "code": "provider_error",
                "message": "GitHub Copilot attempted to use a provider-native tool; Kassiber stopped it."})
    );
}

#[test]
fn copilot_refuses_selected_data_and_answers_incognito_without_a_cursor() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new()
        .handshake(json!({"sessionId": "s1"}))
        .read()
        .update(json!({"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "text", "text": "Unsaved"}}))
        .reply(2, json!({"stopReason": "end_turn"}));
    let (events, code) = exchange(
        broker(&fake, home.path()),
        chat(json!({"sensitive_context": true}), json!([])),
        "",
    );
    assert_eq!(code, Some(1));
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["code"], "provider_error");
    assert!(events[0]["message"]
        .as_str()
        .unwrap()
        .contains("stateless, tool-free exchange"));
    assert_eq!(fake.recorded("argv"), None, "Copilot never started");

    let (events, code) = exchange(
        broker(&fake, home.path()),
        chat(json!({"stateless_session": true}), json!([tool()])),
        "",
    );
    assert_eq!(code, Some(0), "{events:?}");
    assert_eq!(events[2], json!({"type": "delta", "content": "Unsaved"}));
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "done", "finish_reason": "stop"})
    );
}

#[test]
fn a_copilot_failure_carrying_a_credential_is_redacted() {
    let home = tempfile::tempdir().unwrap();
    let fake = Fake::new()
        .handshake(json!({"sessionId": "s1"}))
        .read()
        .send(json!({"jsonrpc": "2.0", "id": 2, "error": {"code": -32603,
            "message": "upstream said: Bearer abcdefghijklmnopqrst for https://bob:hunter2@proxy.test at /home/bob/.copilot"}}));
    let (events, code) = exchange(broker(&fake, home.path()), chat(json!({}), json!([])), "");
    assert_eq!(code, Some(1));
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "error", "code": "provider_error",
                "message": "upstream said: Bearer [redacted] for https://[redacted]@proxy.test at ~/.copilot"})
    );
    let text = events.iter().map(Value::to_string).collect::<String>();
    assert!(!text.contains("abcdefghijklmnopqrst") && !text.contains("hunter2"));
}
