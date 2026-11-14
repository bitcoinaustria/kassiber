//! Whole `ai.chat` streams through the daemon: a CLI agent through the
//! provider broker, and writes asked, applied and undone through the
//! executor against a scripted loopback provider. Replaces the kill-holding
//! tests of `tests/test_rust_ai_chat.py`.
//!
//! The fake broker is an executable POSIX shell script, which
//! `KASSIBER_AI_PROVIDER_BROKER` runs in place of `kassiber --provider-broker`:
//! it logs every line it reads, runs one tool round when the chat offers
//! tools, and answers with a provider session cursor.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use kassiber_net::test_support::{Reply, ScriptedProvider};
use serde_json::{json, Value};

use crate::support::{Live, Sandbox, Transcript};

const TIME: &str = "2026-11-05T12:00:00Z";
const CHECKED_AT: &str = "2026-11-05T12:00:00.123456Z";
const DESKTOP: &str = include_str!("../../../transactions/tests/data/desktop.sql");

const FAKE_BROKER: &str = r#"#!/bin/sh
# A CLI agent run through the broker: one tool round, then an answer, and a
# provider session cursor. It logs every request line it reads.
LOG="$0.log"
IFS= read -r line || exit 0
printf '%s\n' "$line" >> "$LOG"
if ! printf '%s' "$line" | grep -Eq '"command": ?"chat"'; then
  printf '%s\n' '{"type": "result", "data": []}'
  exit 0
fi
if printf '%s' "$line" | grep -Eq '"tools": ?\[ *[^] ]'; then
  printf '%s\n' '{"type": "status", "phase": "fast_mode_unavailable"}'
  printf '%s\n' '{"type": "delta", "content": "Checking ", "reasoning": "plan"}'
  printf '%s\n' '{"type": "tool_call", "call_id": "broker-call-1", "name": "status", "arguments": {}}'
  IFS= read -r reply
  printf '%s\n' "$reply" >> "$LOG"
  printf '%s\n' '{"type": "delta", "content": "all good"}'
  printf '%s\n' '{"type": "done", "finish_reason": "stop", "provider_session_id": "native-cursor-1"}'
else
  printf '%s\n' '{"type": "delta", "content": "plain answer"}'
  printf '%s\n' '{"type": "done", "provider_session_id": "native-cursor-2"}'
fi
"#;

/// A book made by `init`, `workspaces create Demo` and `profiles create
/// Main`, with the AI clocks pinned and the fake broker in place.
fn book() -> Sandbox {
    let mut sandbox = Sandbox::empty();
    let broker = sandbox.path("broker/fake_broker.sh");
    std::fs::create_dir_all(broker.parent().unwrap()).unwrap();
    std::fs::write(&broker, FAKE_BROKER).unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&broker, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let broker = broker.to_string_lossy().into_owned();
    sandbox
        .set("KASSIBER_RUST_TEST_AI_TIME", TIME)
        .set("KASSIBER_RUST_TEST_AI_CHECKED_AT", CHECKED_AT)
        // AI keeps its own online gate; the observer admits loopback.
        .set("KASSIBER_NO_EGRESS", "1")
        .set("KASSIBER_AI_PROVIDER_BROKER", broker.clone())
        .set("KASSIBER_RUST_TEST_AI_BROKER_FIXTURE", broker);
    for argv in [
        &["init"][..],
        &["workspaces", "create", "Demo"],
        &["profiles", "create", "Main"],
    ] {
        let out = sandbox.machine(argv);
        assert_eq!(out.code, 0, "{argv:?}: {}", out.text());
    }
    sandbox
}

/// One scenario step: a request (built from what earlier steps saw), the
/// consent decisions its chat answers in order.
/// Builds a request from the values earlier records showed.
type Build = Box<dyn Fn(&BTreeMap<String, Value>) -> Value>;

struct Step {
    request: Build,
    consent: Vec<&'static str>,
}

fn step(request: Value) -> Step {
    Step {
        request: Box::new(move |_| request.clone()),
        consent: Vec::new(),
    }
}

fn is_terminal(kind: &str) -> bool {
    matches!(kind, "ai.chat" | "error" | "auth_required")
}

/// Run the steps on one daemon, waiting for each request's terminal record
/// and answering consent prompts as the step says. Records are grouped by
/// request id in arrival order and recorded group by group.
fn session(
    t: &mut Transcript,
    sandbox: &Sandbox,
    steps: Vec<Step>,
) -> BTreeMap<String, Vec<Value>> {
    let mut live = Live::start(sandbox);
    let mut seen: BTreeMap<String, Value> = BTreeMap::new();
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    let shutdown = step(json!({"kind": "daemon.shutdown"}));
    for (index, step) in steps.iter().chain(std::iter::once(&shutdown)).enumerate() {
        let rid = format!("r{index}");
        let mut request = (step.request)(&seen);
        request["request_id"] = json!(rid);
        live.write(&request);
        let streaming = request["kind"] == "ai.chat";
        let mut waiting = vec![rid.clone()];
        let mut consent = step.consent.clone();
        let mut extra = 0;
        while !waiting.is_empty() {
            let line = live.read();
            let record: Value = serde_json::from_str(&line).unwrap();
            let record_id = record["request_id"].as_str().unwrap_or_default().to_owned();
            let kind = record["kind"].as_str().unwrap_or_default().to_owned();
            match groups.iter_mut().find(|(id, _)| *id == record_id) {
                Some((_, lines)) => lines.push(line.clone()),
                None => groups.push((record_id.clone(), vec![line.clone()])),
            }
            if record_id == rid && streaming {
                if is_terminal(&kind) {
                    waiting.retain(|w| *w != rid);
                    if kind == "ai.chat" {
                        seen.insert("session_id".into(), record["data"]["session_id"].clone());
                    }
                } else if kind == "ai.chat.tool_consent_required" && !consent.is_empty() {
                    let decision = consent.remove(0);
                    extra += 1;
                    let control = format!("{rid}c{extra}");
                    live.write(
                        &json!({"request_id": control, "kind": "ai.tool_call.consent",
                        "args": {"target_request_id": rid, "call_id": record["data"]["call_id"],
                            "decision": decision}}),
                    );
                    waiting.push(control);
                }
            } else if let Some(at) = waiting.iter().position(|w| *w == record_id) {
                waiting.remove(at);
            }
        }
    }
    live.finish();
    let order = |rid: &str| -> (u64, String) {
        let digits: String = rid
            .chars()
            .skip(1)
            .take_while(char::is_ascii_digit)
            .collect();
        (
            digits.parse().unwrap_or(u64::MAX),
            rid[1 + digits.len()..].to_owned(),
        )
    };
    groups.sort_by_key(|(rid, _)| order(rid));
    let mut records = BTreeMap::new();
    for (rid, lines) in groups {
        let _ = writeln!(t.text_mut(), "## {rid}");
        for line in &lines {
            let _ = writeln!(t.text_mut(), "< {}", sandbox.scrub(line));
        }
        records.insert(
            rid,
            lines
                .iter()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect(),
        );
    }
    records
}

fn broker_log(sandbox: &Sandbox) -> String {
    std::fs::read_to_string(sandbox.path("broker/fake_broker.sh.log")).unwrap_or_default()
}

fn chat(args: Value) -> Value {
    json!({"kind": "ai.chat", "args": args})
}

#[test]
fn test_broker_chat_tool_round_and_resumed_cursor() {
    let sandbox = book();
    let before = sandbox.dump();
    let mut t = Transcript::new("ai_chat/broker_chat_tool_round_and_resumed_cursor");
    let hello = json!({"role": "user", "content": "hello"});
    let shown = json!({"role": "assistant", "content": "Checking all good"});
    let thanks = json!({"role": "user", "content": "thanks"});
    let more = json!({"role": "user", "content": "more"});
    let last_leg = json!({"role": "assistant", "content": "all good"});
    let turn = |messages: Value, tools: bool| Step {
        request: Box::new(move |seen| {
            let mut args = json!({"provider": "codex", "model": "default",
                "session_id": seen["session_id"], "messages": messages});
            if tools {
                args["tools_enabled"] = json!(true);
            }
            chat(args)
        }),
        consent: Vec::new(),
    };
    session(
        &mut t,
        &sandbox,
        vec![
            step(json!({"kind": "ai.providers.acknowledge", "args": {"name": "codex"}})),
            step(
                json!({"kind": "ai.providers.update", "args": {"name": "codex", "enabled": true}}),
            ),
            step(json!({"kind": "ui.chat.history.configure", "args": {"history": "on"}})),
            step(chat(
                json!({"provider": "codex", "model": "default", "tools_enabled": true,
                "persist": true, "options": {"fast_mode": true}, "messages": [hello]}),
            )),
            // A tool turn in the saved session: the broker streams
            // "Checking ", runs a tool, then "all good".
            turn(json!([hello, shown, thanks]), true),
            // The next prompt resumes the cursor: it was bound to every
            // leg's text the user saw, the transcript before the prompt.
            turn(json!([hello, shown, thanks, shown, more]), false),
            // A branch holding only the tool turn's last leg is not the
            // answer the user saw.
            turn(json!([hello, shown, thanks, last_leg, more]), false),
            // The plain turn's cursor resumes on the prompt after it.
            turn(
                json!([hello, shown, thanks, last_leg, more,
                    {"role": "assistant", "content": "plain answer"},
                    {"role": "user", "content": "again"}]),
                false,
            ),
            step(chat(
                json!({"provider": "codex", "model": "default", "persist": false,
                "messages": [{"role": "user", "content": "private"}]}),
            )),
        ],
    );
    let log = broker_log(&sandbox);
    t.section("broker log", &sandbox.scrub(&log));
    t.changes(&sandbox, &before, "after the session");
    let chats: Vec<Value> = log
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .filter(|line| line["command"] == "chat")
        .collect();
    assert_eq!(chats.len(), 6);
    let cursors: Vec<Value> = chats
        .iter()
        .map(|chat| chat["options"]["provider_session_id"].clone())
        .collect();
    assert_eq!(
        cursors,
        [
            Value::Null,
            Value::Null,
            json!("native-cursor-1"),
            Value::Null,
            json!("native-cursor-2"),
            Value::Null
        ]
    );
    assert_eq!(chats[5]["options"]["stateless_session"], true);
    t.finish();
}

fn message_item(text: &str) -> Value {
    json!({"type": "message", "role": "assistant", "status": "completed",
        "content": [{"type": "output_text", "text": text}]})
}

fn function_call(call_id: &str, name: &str, arguments: &str) -> Value {
    json!({"type": "function_call", "id": format!("fc_{call_id}"), "call_id": call_id,
        "name": name, "arguments": arguments})
}

/// `responses_stream`: an optional text delta, then the completed response.
fn responses(output: Vec<Value>, text: Option<&str>) -> Reply {
    let mut events = Vec::new();
    if let Some(text) = text {
        events.push(python_dumps(&json!({"type": "response.output_text.delta",
            "output_index": 0, "delta": text})));
    }
    events.push(python_dumps(&json!({"type": "response.completed",
        "response": {"id": "resp_1", "status": "completed", "output": output}})));
    let events: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply::sse(&events, true)
}

fn answer(text: &str) -> Reply {
    responses(vec![message_item(text)], Some(text))
}

/// `json.dumps` with its default separators (keys in insertion order are
/// not kept by serde_json; the provider reads JSON, not bytes).
fn python_dumps(value: &Value) -> String {
    match value {
        Value::Object(map) => format!(
            "{{{}}}",
            map.iter()
                .map(|(k, v)| format!("{}: {}", Value::from(k.as_str()), python_dumps(v)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(python_dumps)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        other => other.to_string(),
    }
}

/// The model follows each applied call's undo hint, as the prompt tells it.
fn undo_calls(body: &[u8], call_ids: &[&str]) -> Reply {
    let request: Value = serde_json::from_slice(body).unwrap();
    let outputs: BTreeMap<String, Value> = request["input"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|item| item["type"] == "function_call_output")
        .map(|item| {
            (
                item["call_id"].as_str().unwrap().to_owned(),
                serde_json::from_str(item["output"].as_str().unwrap()).unwrap(),
            )
        })
        .collect();
    let calls = call_ids
        .iter()
        .map(|id| {
            let undo = &outputs[*id]["envelope"]["data"]["undo"];
            function_call(
                &format!("undo_{id}"),
                undo["tool"].as_str().unwrap(),
                &python_dumps(&undo["arguments"]),
            )
        })
        .collect();
    responses(calls, None)
}

#[test]
fn test_writes_ask_apply_and_undo_through_the_executor() {
    let sandbox = book();
    // The fixture book's rows beside the driver's workspace; its context
    // replaces the driver's.
    sandbox.sql(&DESKTOP.replace("INSERT INTO settings", "INSERT OR REPLACE INTO settings"));
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                vec![
                    message_item("Updating."),
                    function_call(
                        "call_note",
                        "ui_transactions_metadata_update",
                        r#"{"transaction": "b", "note": "from chat"}"#,
                    ),
                    function_call(
                        "call_view",
                        "ui_saved_views_create",
                        r#"{"surface": "transactions", "name": "Chat view"}"#,
                    ),
                ],
                Some("Updating."),
            ),
            Reply::computed(|body| undo_calls(body, &["call_note", "call_view"])),
            answer("Done and undone."),
            // `auto`: a reversible local write runs at once; a plan-bound
            // rollback still asks, and shows the book's plan.
            responses(
                vec![
                    function_call(
                        "call_auto",
                        "ui_transactions_metadata_update",
                        r#"{"transaction": "b", "note": "auto"}"#,
                    ),
                    function_call(
                        "call_rollback",
                        "ui_imports_rollback",
                        &format!(
                            r#"{{"batch": "missing", "plan_digest": "{}", "expected_input_version": 0}}"#,
                            "0".repeat(64)
                        ),
                    ),
                    function_call(
                        "call_unsaved",
                        "ui_saved_views_delete",
                        r#"{"view_id": "missing"}"#,
                    ),
                ],
                None,
            ),
            answer("Auto."),
            // `full_access`: nothing asks, not even a once-only change.
            responses(
                vec![
                    function_call("call_process", "ui_journals_process", "{}"),
                    function_call(
                        "call_configure",
                        "ui_maintenance_configure",
                        r#"{"bitcoin_rail_carrying_value": true}"#,
                    ),
                ],
                None,
            ),
            answer("Processed."),
        ],
    );
    let base_url = provider.base_url();
    let before = sandbox.dump();
    let mut t = Transcript::new("ai_chat/writes_ask_apply_and_undo");
    t.mask(move |text| text.replace(&base_url, "<PROVIDER>"));
    let turn = |text: &str, mode: Option<&str>, consent: Vec<&'static str>| {
        let mut args = json!({"model": "test-model", "tools_enabled": true, "tool_profile": "full",
            "persist": true, "system_prompt_kind": "raw", "system_prompt": "Use tools.",
            "messages": [{"role": "user", "content": text}]});
        if let Some(mode) = mode {
            args["permission_mode"] = json!(mode);
        }
        Step {
            request: Box::new(move |_| chat(args.clone())),
            consent,
        }
    };
    let records = session(
        &mut t,
        &sandbox,
        vec![
            step(
                json!({"kind": "ai.providers.create", "args": {"name": "tool-local",
                "base_url": provider.base_url(), "kind": "local", "default_model": "test-model"}}),
            ),
            step(json!({"kind": "ai.providers.set_default", "args": {"name": "tool-local"}})),
            step(json!({"kind": "ui.chat.history.configure", "args": {"history": "on"}})),
            turn(
                "note it",
                None,
                vec!["allow_once", "allow_session", "allow_once", "deny"],
            ),
            turn("auto", Some("auto"), vec!["deny", "deny"]),
            turn("process", Some("full_access"), vec![]),
        ],
    );
    let mut sent = String::new();
    for request in provider.requests() {
        let _ = writeln!(
            sent,
            "{} {} {:?}\n{}",
            request.method,
            request.path,
            request.authorization,
            String::from_utf8_lossy(&request.body)
        );
    }
    t.section("provider requests", &sandbox.scrub(&sent));
    t.changes(&sandbox, &before, "after the session");
    let results = |rid: &str| -> BTreeMap<String, Value> {
        records[rid]
            .iter()
            .filter(|r| r["kind"] == "ai.chat.tool_result")
            .map(|r| {
                (
                    r["data"]["call_id"].as_str().unwrap().to_owned(),
                    r["data"].clone(),
                )
            })
            .collect()
    };
    let approvals = |rid: &str| -> Vec<(Value, Value, Value)> {
        records[rid]
            .iter()
            .filter(|r| r["kind"] == "ai.chat.tool_call")
            .map(|r| {
                (
                    r["data"]["call_id"].clone(),
                    r["data"]["approval"].clone(),
                    r["data"]["needs_consent"].clone(),
                )
            })
            .collect()
    };
    let first = results("r3");
    for call_id in ["call_note", "call_view", "undo_call_note"] {
        assert_eq!(first[call_id]["ok"], true, "{}", first[call_id]);
    }
    assert_eq!(
        first["call_note"]["envelope"]["data"]["undo"]["tool"],
        "ui_transactions_history_revert"
    );
    assert_eq!(first["undo_call_view"]["reason"], "user_denied");
    let second = results("r4");
    assert_eq!(second["call_auto"]["ok"], true, "{}", second["call_auto"]);
    assert_eq!(second["call_rollback"]["reason"], "user_denied");
    assert_eq!(second["call_unsaved"]["reason"], "user_denied");
    assert!(approvals("r4").contains(&(json!("call_auto"), json!("auto"), json!(false))));
    let third = results("r5");
    // The fixture mixes networks: the rebuild is refused.
    assert_eq!(
        third["call_process"]["reason"],
        "book_network_review_required"
    );
    assert_eq!(third["call_configure"]["ok"], true);
    assert!(approvals("r5").contains(&(json!("call_process"), json!("full_access"), json!(false))));
    assert!(!records["r5"]
        .iter()
        .any(|r| r["kind"] == "ai.chat.tool_consent_required"));
    t.finish();
}
