//! `kassiber --provider-broker` serving Claude and Codex turns against the
//! fake CLIs in `crates/provider-broker/tests/fake-cli` (plan 23, step 2):
//! status probes, a plain chat, a tool call through the real MCP child
//! (Claude) and through dynamic tools (Codex), selected-data and Incognito
//! turns, and a provider failure whose text carries a credential.
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

/// A fake CLI and its behaviour files in a fresh directory, which is the
/// broker's whole `PATH`.
struct Fake {
    dir: tempfile::TempDir,
}

impl Fake {
    fn new(name: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("fake-{name}-"))
            .tempdir()
            .unwrap();
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../provider-broker/tests/fake-cli");
        for file in [name, "record.sh"] {
            std::fs::copy(source.join(file), dir.path().join(file)).unwrap();
        }
        Self { dir }
    }

    fn write(&self, file: &str, contents: &str) -> &Self {
        std::fs::write(self.dir.path().join(file), contents).unwrap();
        self
    }

    fn lines(&self, file: &str, lines: &[Value]) -> &Self {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.write(file, &text)
    }

    /// Every recorded argv, oldest first.
    fn argvs(&self) -> Vec<Vec<String>> {
        let Ok(entries) = std::fs::read_dir(self.dir.path().join("calls")) else {
            return Vec::new();
        };
        let mut calls: Vec<(u64, PathBuf)> = entries
            .map(|entry| entry.unwrap().path())
            .map(|path| {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                (name.split('-').next().unwrap().parse().unwrap(), path)
            })
            .collect();
        calls.sort();
        calls
            .into_iter()
            .map(|(_, path)| {
                std::fs::read_to_string(path.join("argv"))
                    .unwrap()
                    .split_terminator('\0')
                    .map(str::to_owned)
                    .collect()
            })
            .collect()
    }

    /// The text of one file of the last recorded call.
    fn last_call(&self, file: &str) -> String {
        let entries = std::fs::read_dir(self.dir.path().join("calls")).unwrap();
        let last = entries
            .map(|entry| entry.unwrap().path())
            .max_by_key(|path| {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                name.split('-').next().unwrap().parse::<u64>().unwrap()
            })
            .unwrap();
        std::fs::read_to_string(last.join(file)).unwrap_or_default()
    }
}

/// What one broker run wrote and how it ended.
struct Run {
    events: Vec<Value>,
    code: Option<i32>,
    stderr: String,
}

/// Run the broker on `request` with `fake` as its `PATH`, answering each
/// `tool_call` with `output` as the daemon would.
fn broker(fake: &Fake, request: Value, output: &str) -> Run {
    let home = tempfile::tempdir().unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_kassiber"))
        .arg("--provider-broker")
        .env_clear()
        .env("PATH", fake.dir.path())
        .env("HOME", home.path())
        // Short enough for the tool bridge's socket path.
        .env("TMPDIR", std::env::temp_dir())
        .env("KASSIBER_AI_BROKER_TMPDIR", home.path())
        .env("ANTHROPIC_API_KEY", "sk-ant-fake-key-for-tests")
        .env("OPENAI_API_KEY", "sk-fake-openai-key")
        .env("UNRELATED_PRIVATE_VALUE", "must-not-cross")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(format!("{request}\n").as_bytes()).unwrap();
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let mut events = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(60);
    for line in stdout.lines() {
        assert!(Instant::now() < deadline, "the broker ran too long");
        let event: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if event["type"] == "tool_call" {
            let reply = json!({"command": "tool_results", "results": [
                {"call_id": event["call_id"], "output": output},
            ]});
            stdin.write_all(format!("{reply}\n").as_bytes()).unwrap();
            stdin.flush().unwrap();
        }
        events.push(event);
    }
    drop(stdin);
    let out = child.wait_with_output().unwrap();
    // Nothing of the turn is left in the broker's temporary root.
    let left: Vec<_> = std::fs::read_dir(home.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.starts_with("kassiber-ai-"))
        .collect();
    assert!(left.is_empty(), "{left:?}");
    Run {
        events,
        code: out.status.code(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn chat(provider: &str, options: Value) -> Value {
    json!({
        "command": "chat", "request_id": "e2e", "provider": provider, "model": "default",
        "messages": [{"role": "user", "content": "How is the book?"}],
        "options": options,
    })
}

fn with_tools(mut request: Value) -> Value {
    request["tools"] = json!([{
        "name": "status",
        "description": "Return the Kassiber book status.",
        "parameters": {"type": "object", "properties": {}, "additionalProperties": false},
        "read_only": true,
        "destructive": false,
    }]);
    request
}

#[test]
fn claude_reports_status_through_the_binary() {
    let fake = Fake::new("claude");
    let run = broker(
        &fake,
        json!({"command": "status", "provider": "claude"}),
        "",
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let status = &run.events[0]["data"][0];
    assert_eq!(status["state"], "ready");
    assert_eq!(status["executable"], "claude");
    assert_eq!(status["version"], "2.1.287 (Claude Code)");
    assert_eq!(status["models"][2]["supports_fast_mode"], true);

    fake.write("auth", r#"{"loggedIn":false}"#);
    let run = broker(
        &fake,
        json!({"command": "status", "provider": "claude"}),
        "",
    );
    assert_eq!(run.events[0]["data"][0]["state"], "authentication_required");
    // `models` fails only on an error state.
    let run = broker(
        &fake,
        json!({"command": "models", "provider": "claude"}),
        "",
    );
    assert_eq!(run.code, Some(0));
    assert_eq!(run.events[0]["data"].as_array().unwrap().len(), 5);
}

#[test]
fn claude_calls_a_kassiber_tool_through_the_mcp_child() {
    let fake = Fake::new("claude");
    fake.write(
        "mcp-call",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"status\",\"arguments\":{\"q\":1}}}\n",
    )
    .lines(
        "stream.jsonl",
        &[
            json!({"type": "system", "subtype": "init", "session_id": "s-tool", "fast_mode_state": "off"}),
            json!({"type": "stream_event", "event": {"type": "content_block_start", "content_block": {"type": "tool_use", "name": "mcp__kassiber__status"}}}),
            json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "The book is fine."}}}),
            json!({"type": "result", "subtype": "success", "session_id": "s-tool"}),
        ],
    );
    let run = broker(
        &fake,
        with_tools(chat("claude", json!({}))),
        "{\"ok\":true}",
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.events,
        [
            json!({"type": "status", "phase": "connecting", "message": "Starting Claude"}),
            json!({"type": "tool_call", "call_id": run.events[1]["call_id"], "name": "status", "arguments": {"q": 1}}),
            json!({"type": "delta", "content": "The book is fine."}),
            json!({"type": "done", "finish_reason": "stop", "provider_session_id": "s-tool"}),
        ]
    );
    // The MCP child answered the provider with the daemon's output.
    let mcp = fake.last_call("mcp-out");
    assert!(mcp.contains(r#"{\"ok\":true}"#), "{mcp}");
    let argv = &fake.argvs()[0];
    let config = &argv[argv.iter().position(|arg| arg == "--mcp-config").unwrap() + 1];
    let config: Value = serde_json::from_str(config).unwrap();
    assert_eq!(
        config["mcpServers"]["kassiber"]["args"][0],
        "--provider-broker-mcp"
    );
    assert_eq!(
        argv[argv.iter().position(|arg| arg == "--allowedTools").unwrap() + 1],
        "mcp__kassiber__*"
    );
    // Only the provider's own variables crossed.
    let names = fake.last_call("env-names");
    assert!(names.lines().any(|name| name == "ANTHROPIC_API_KEY"));
    assert!(!names.lines().any(|name| name == "OPENAI_API_KEY"));
    assert!(!names.lines().any(|name| name == "UNRELATED_PRIVATE_VALUE"));
}

#[test]
fn claude_redacts_a_credential_in_a_provider_failure() {
    let fake = Fake::new("claude");
    fake.write(
        "stderr",
        "upstream rejected api_key=sk-live-abcdefgh12345 at https://user:pa55word@example.com\n",
    )
    .write("exit", "1");
    let run = broker(&fake, chat("claude", json!({})), "");
    assert_eq!(run.code, Some(1));
    assert_eq!(
        run.events.last().unwrap(),
        &json!({"type": "error", "code": "provider_error", "message": "Claude ended without a terminal response. upstream rejected api_key=[redacted] at https://[redacted]@example.com"})
    );
}

#[test]
fn claude_selected_data_and_incognito_turns_keep_no_session() {
    let fake = Fake::new("claude");
    fake.lines(
        "stream.jsonl",
        &[json!({"type": "result", "subtype": "success", "session_id": "kept"})],
    );
    for options in [
        json!({"sensitive_context": true}),
        json!({"stateless_session": true}),
    ] {
        let run = broker(&fake, chat("claude", options.clone()), "");
        assert_eq!(run.code, Some(0), "{}", run.stderr);
        assert_eq!(
            run.events.last().unwrap(),
            &json!({"type": "done", "finish_reason": "stop"}),
            "{options}"
        );
        let argv = fake.argvs().pop().unwrap();
        assert!(argv.iter().any(|arg| arg == "--no-session-persistence"));
    }
    // A failed selected-data turn says nothing of the provider's text.
    fake.write("stream.jsonl", "")
        .write("stderr", "boom: synthetic evidence\n");
    let run = broker(
        &fake,
        chat("claude", json!({"sensitive_context": true})),
        "",
    );
    assert_eq!(
        run.events.last().unwrap()["message"],
        "Claude sensitive-context request failed; no provider diagnostic content was retained."
    );
}

#[test]
fn codex_reports_status_through_the_binary() {
    let fake = Fake::new("codex");
    let run = broker(&fake, json!({"command": "status", "provider": "codex"}), "");
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    let status = &run.events[0]["data"][0];
    assert_eq!(status["state"], "ready");
    assert_eq!(status["models"][0]["id"], "gpt-5.4");
    assert_eq!(status["models"][0]["supports_fast_mode"], true);

    fake.write("models.json", r#"{"data":[]}"#);
    let run = broker(&fake, json!({"command": "status", "provider": "codex"}), "");
    assert_eq!(run.events[0]["data"][0]["state"], "authentication_required");
}

#[test]
fn codex_calls_a_kassiber_tool_through_dynamic_tools() {
    let fake = Fake::new("codex");
    fake.write(
        "turn.jsonl",
        "{\"id\":\"tool-1\",\"method\":\"item/tool/call\",\"params\":{\"tool\":\"status\",\"callId\":\"call-9\",\"arguments\":{\"q\":1}}}\n",
    )
    .lines(
        "after-tool.jsonl",
        &[json!({"method": "item/agentMessage/delta", "params": {"itemId": "a", "delta": "The book is fine."}})],
    );
    let run = broker(&fake, with_tools(chat("codex", json!({}))), "{\"ok\":true}");
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(run.events[0]["phase"], "connecting");
    assert_eq!(
        run.events[1],
        json!({"type": "tool_call", "call_id": "call-9", "name": "status", "arguments": {"q": 1}})
    );
    assert_eq!(
        run.events[2],
        json!({"type": "delta", "content": "The book is fine."})
    );
    assert!(run.events[3]["provider_session_id"]
        .as_str()
        .unwrap()
        .ends_with(":thread-1"));
    let wire = std::fs::read_to_string(fake.dir.path().join("wire.jsonl")).unwrap();
    assert!(wire.contains(
        r#"{"id":"tool-1","result":{"contentItems":[{"type":"inputText","text":"{\"ok\":true}"}],"success":true}}"#
    ));
    let names = fake.last_call("env-names");
    assert!(names.lines().any(|name| name == "OPENAI_API_KEY"));
    assert!(!names.lines().any(|name| name == "ANTHROPIC_API_KEY"));
}

#[test]
fn codex_incognito_turns_return_no_cursor() {
    let fake = Fake::new("codex");
    let run = broker(
        &fake,
        with_tools(chat("codex", json!({"stateless_session": true}))),
        "",
    );
    assert_eq!(run.code, Some(0), "{}", run.stderr);
    assert_eq!(
        run.events.last().unwrap(),
        &json!({"type": "done", "finish_reason": "stop"})
    );
}

#[test]
fn codex_redacts_a_credential_in_a_provider_failure() {
    let fake = Fake::new("codex");
    fake.write(
        "stderr",
        "fatal: upstream rejected token=sk-live-abcdefgh12345\n",
    );
    let run = broker(&fake, chat("codex", json!({})), "");
    assert_eq!(run.code, Some(1));
    assert_eq!(
        run.events.last().unwrap(),
        &json!({"type": "error", "code": "provider_error", "message": "fatal: upstream rejected token=[redacted]"})
    );
}
