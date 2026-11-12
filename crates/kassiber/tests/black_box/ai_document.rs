//! The assistant reads a chat's photo with a local vision model and imports
//! the draft's ready rows; the reader's model call never holds the daemon's
//! request loop. Replaces the kill-holding tests of
//! `tests/test_rust_ai_document_import.py`.

use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use kassiber_net::test_support::{HandledProvider, Reply, ScriptedProvider};
use serde_json::{json, Value};

use crate::support::{Live, Sandbox, Transcript};

const SOURCE: &[u8] = b"Kassiber synthetic local OCR evidence\n";
const DESKTOP: &str = include_str!("../../../transactions/tests/data/desktop.sql");

fn rows() -> Value {
    serde_json::from_str(include_str!("ai/ocr_rows.json")).unwrap()
}

type Hits = Arc<Mutex<Vec<(String, String, Value)>>>;

/// The loopback OCR fixture: model listings on GET, the rows as the
/// model's answer on POST; each request is recorded.
fn ocr(rows: Value, gate: Option<Arc<Gate>>) -> (HandledProvider, Hits) {
    let hits: Hits = Arc::default();
    let seen = hits.clone();
    let provider = HandledProvider::start(move |request| {
        let body = if request.body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&request.body).unwrap_or(Value::Null)
        };
        seen.lock()
            .unwrap()
            .push((request.method.clone(), request.target.clone(), body));
        if request.method == "GET" {
            return (200, json!({"data": [{"id": "glm-ocr"}]}).to_string());
        }
        if let Some(gate) = &gate {
            gate.hold();
        }
        let text = json!({"rows": rows}).to_string();
        let reply = json!({"output": [{"type": "message",
            "content": [{"type": "output_text", "text": text}]}]})
        .to_string();
        if let Some(gate) = &gate {
            gate.answered();
        }
        (200, reply)
    })
    .unwrap();
    (provider, hits)
}

/// Holds the model's answer until released; records start and answer.
#[derive(Default)]
struct Gate {
    state: Mutex<(bool, bool, bool)>, // started, released, answered
    changed: Condvar,
}

impl Gate {
    fn hold(&self) {
        let mut state = self.state.lock().unwrap();
        state.0 = true;
        self.changed.notify_all();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !state.1 && std::time::Instant::now() < deadline {
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
    }

    fn answered(&self) {
        self.state.lock().unwrap().2 = true;
    }

    fn wait_started(&self) -> bool {
        let mut state = self.state.lock().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while !state.0 && std::time::Instant::now() < deadline {
            state = self
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap()
                .0;
        }
        state.0
    }

    fn release(&self) {
        self.state.lock().unwrap().1 = true;
        self.changed.notify_all();
    }

    fn is_answered(&self) -> bool {
        self.state.lock().unwrap().2
    }
}

fn tool(name: &str, arguments: Value, call_id: &str, mut state: Value) -> Value {
    state["consented_call_ids"] = json!([call_id]);
    json!({"kind": "test.ai_tool", "args": {"name": name, "arguments": arguments,
        "call_id": call_id, "state": state}})
}

fn output(record: &Value) -> Value {
    record["data"]["output"]
        .as_str()
        .map_or(Value::Null, |text| serde_json::from_str(text).unwrap())
}

#[test]
fn test_reading_and_importing_a_chat_photo_matches_python() {
    let (server, hits) = ocr(rows(), None);
    let mut sandbox = Sandbox::book(DESKTOP);
    sandbox
        .set("KASSIBER_NO_EGRESS", "1")
        .set("KASSIBER_TEST_NO_EGRESS", "1");
    let source = sandbox.path("receipt.png");
    std::fs::write(&source, SOURCE).unwrap();
    sandbox.sql(&format!(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at)
          VALUES('imports','w','p','Imports','custom','{{}}','2025');
         INSERT INTO settings(key,value) VALUES('ai_providers_seeded','1')
          ON CONFLICT(key) DO UPDATE SET value='1';
         INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at)
          VALUES('local','{}/v1','local','2026-01-01','2026-01-01');
         INSERT INTO ai_providers(name,base_url,kind,created_at,updated_at)
          VALUES('cloud','https://api.example.invalid/v1','openai','2026-01-01','2026-01-01');",
        server.base()
    ));
    let before = sandbox.dump();
    let source = source.to_string_lossy().into_owned();
    let local = json!({"provider_kind": "local", "provider_on_device": true});
    let remote = json!({"provider_kind": "remote", "provider_on_device": false});
    let state = |token: &str, access: &Value, sessions: bool, kind: &str| {
        let mut state = access.clone();
        state["scope_workspace_id"] = json!("w");
        state["scope_profile_id"] = json!("p");
        state["chat_files"] = json!([{"id": "file-1", "source_file": source, "filename": "receipt.png",
            "label": "Receipt", "kind": kind, "new": true, "token": token}]);
        if sessions {
            state["document_import_sessions"] = json!(true);
        }
        state
    };
    let mut t = Transcript::new("ai_document/reading_and_importing_a_chat_photo");
    let mut live = Live::start(&sandbox);
    let staged = live.send(
        &mut t,
        &sandbox,
        json!({"kind": "internal.document_import.stage",
            "args": {"source_file": source, "purpose": "chat_attachment"}}),
    );
    let token = staged["data"]["document_token"]
        .as_str()
        .unwrap()
        .to_owned();
    let st = |access: &Value, sessions: bool, kind: &str| state(&token, access, sessions, kind);
    let mut records = vec![staged.clone()];
    let mut send = |t: &mut Transcript, live: &mut Live, request: Value| {
        let record = live.send(t, &sandbox, request);
        records.push(record.clone());
        record
    };
    // Outside a chat there are no document grants; a spreadsheet kind and
    // an unknown file are refused before the reader is resolved.
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({}),
            "call_nochat",
            st(&local, false, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({}),
            "call_kind",
            st(&local, true, "delimited"),
        ),
    );
    let read = output(&send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({"file": "file-1"}),
            "call_read",
            st(&local, true, "image"),
        ),
    ));
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({"provider": "cloud"}),
            "call_cloud",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({"pages": [9]}),
            "call_pages",
            st(&remote, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_read_document",
            json!({}),
            "call_restricted",
            st(&remote, true, "image"),
        ),
    );
    let draft = if read["ok"] == true {
        read["envelope"]["data"]["draft"].clone()
    } else {
        json!("missing")
    };
    let planned = output(&send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_plan",
            json!({"draft": draft, "wallet": "Imports"}),
            "call_plan",
            st(&local, true, "image"),
        ),
    ));
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_plan",
            json!({"draft": draft, "new_wallet": {"label": "Receipts"}}),
            "call_plan_new",
            st(&remote, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_plan",
            json!({"draft": draft, "wallet": "cold"}),
            "call_plan_cold",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_plan",
            json!({"draft": "missing", "wallet": "Imports"}),
            "call_plan_missing",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_plan",
            json!({"wallet": "Imports"}),
            "call_plan_bare",
            st(&local, true, "image"),
        ),
    );
    let mut approved = json!({"draft": draft, "wallet": "Imports"});
    if planned["ok"] == true {
        for (key, value) in planned["envelope"]["data"]["apply_with"]
            .as_object()
            .unwrap()
        {
            approved[key] = value.clone();
        }
    }
    send(
        &mut t,
        &mut live,
        json!({"kind": "test.ai_consent_preview", "args": {
        "name": "ui_wallets_import_document_apply", "arguments": approved,
        "call_id": "call_apply", "state": st(&local, true, "image")}}),
    );
    let mut tampered = approved.clone();
    tampered["plan_digest"] = json!("0".repeat(64));
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_apply",
            tampered,
            "call_tampered",
            st(&local, true, "image"),
        ),
    );
    let mut changed = approved.clone();
    changed["expected_source_sha256"] = json!("0".repeat(64));
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_apply",
            changed,
            "call_changed",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_apply",
            approved.clone(),
            "call_apply",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_wallets_import_document_apply",
            approved,
            "call_again",
            st(&local, true, "image"),
        ),
    );
    send(
        &mut t,
        &mut live,
        tool(
            "ui_imports_list",
            json!({}),
            "call_runs",
            st(&local, true, "image"),
        ),
    );
    live.finish();
    // Each daemon issues its own grant and draft tokens.
    let mut tokens = vec![token.clone()];
    for record in &records {
        if let Some(draft) = output(record)["envelope"]["data"]["draft"].as_str() {
            tokens.push(draft.to_owned());
        }
    }
    tokens.sort_by_key(|t| std::cmp::Reverse(t.len()));
    let base = server.base().to_owned();
    t.mask(move |text| {
        let mut text = text.replace(&base, "<OCR>");
        for token in &tokens {
            text = text.replace(token.as_str(), "<token>");
        }
        text
    });
    let mut log = String::new();
    for (method, path, body) in hits.lock().unwrap().iter() {
        log.push_str(&format!("{method} {path} {body}\n"));
    }
    t.section("ocr requests", &sandbox.scrub(&log));
    t.changes_blanked(&sandbox, &before, "after the session");
    let reasons: Vec<Value> = records
        .iter()
        .map(|r| output(r)["reason"].clone())
        .collect();
    assert_eq!(
        reasons[1..3],
        [
            json!("ai_attachment_missing"),
            json!("ai_attachment_unsupported")
        ]
    );
    assert_eq!(read["ok"], true, "{read}");
    assert_eq!(read["envelope"]["data"]["summary"]["ready"], 1);
    assert!(!records[3]["data"]["output"]
        .as_str()
        .unwrap()
        .contains("evidence_text"));
    // A remote provider is refused before any contact: never acknowledged.
    assert_eq!(reasons[4], "ai_remote_ack_required");
    let restricted = &output(&records[6])["envelope"]["data"];
    assert_eq!(restricted["row_values_withheld"], true);
    assert!(restricted.get("rows").is_none());
    assert_eq!(planned["envelope"]["data"]["ready"], true, "{planned}");
    assert_eq!(
        planned["envelope"]["data"]["warnings"],
        json!(["rows_held_for_review"])
    );
    assert!(output(&records[8])["envelope"]["data"]
        .get("row_ids")
        .is_none());
    assert_eq!(
        reasons[13..15],
        [json!("import_plan_stale"), json!("attachment_changed")]
    );
    let applied = output(&records[15]);
    assert_eq!(applied["ok"], true, "{applied}");
    assert_eq!(applied["envelope"]["data"]["evidence_attached"], 1);
    // The draft was consumed by the apply.
    assert_eq!(reasons[16], "document_import_session_expired");
    assert!(hits
        .lock()
        .unwrap()
        .iter()
        .all(|(_, path, _)| path.starts_with("/v1/")));
    t.finish();
}

fn function_call(call_id: &str, name: &str, arguments: &str) -> String {
    json!({"type": "function_call", "id": format!("fc_{call_id}"), "call_id": call_id,
        "name": name, "arguments": arguments})
    .to_string()
}

fn responses(output: &[String], text: Option<&str>) -> Reply {
    let mut events = Vec::new();
    if let Some(text) = text {
        events.push(
            json!({"type": "response.output_text.delta", "output_index": 0, "delta": text})
                .to_string(),
        );
    }
    let output: Vec<Value> = output
        .iter()
        .map(|o| serde_json::from_str(o).unwrap())
        .collect();
    events.push(
        json!({"type": "response.completed",
            "response": {"id": "resp_1", "status": "completed", "output": output}})
        .to_string(),
    );
    let events: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply::sse(&events, true)
}

#[test]
fn test_other_requests_are_answered_while_the_model_reads() {
    let gate = Arc::new(Gate::default());
    let first = rows()[0].clone();
    let (server, _) = ocr(json!([first]), Some(gate.clone()));
    let chat = ScriptedProvider::new().unwrap();
    chat.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[function_call(
                    "call_read",
                    "ui_wallets_read_document",
                    r#"{"file":"file-1","provider":"ocr"}"#,
                )],
                None,
            ),
            responses(
                &[
                    json!({"type": "message", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": "One row is ready."}]})
                    .to_string(),
                ],
                Some("One row is ready."),
            ),
        ],
    );
    let mut sandbox = Sandbox::empty();
    sandbox
        .set("KASSIBER_RUST_TEST_AI_TIME", "2026-11-05T12:00:00Z")
        .set(
            "KASSIBER_RUST_TEST_AI_CHECKED_AT",
            "2026-11-05T12:00:00.123456Z",
        )
        .set("KASSIBER_NO_EGRESS", "1");
    for argv in [
        &["init"][..],
        &["workspaces", "create", "Demo"],
        &["profiles", "create", "Main"],
    ] {
        assert_eq!(sandbox.machine(argv).code, 0, "{argv:?}");
    }
    let source = sandbox.path("receipt.png");
    std::fs::write(&source, SOURCE).unwrap();
    let mut live = Live::start(&sandbox);
    let until = |live: &mut Live, request_id: &str| -> Value {
        loop {
            let record: Value = serde_json::from_str(&live.read()).unwrap();
            if record["request_id"] == request_id {
                return record;
            }
        }
    };
    let setup = [
        json!({"kind": "ai.providers.create", "args": {"name": "tool-local",
            "base_url": chat.base_url(), "kind": "local", "default_model": "test-model"}}),
        json!({"kind": "ai.providers.set_default", "args": {"name": "tool-local"}}),
        json!({"kind": "ai.providers.create", "args": {"name": "ocr", "kind": "local",
            "base_url": format!("{}/v1", server.base())}}),
        json!({"kind": "internal.document_import.stage",
            "args": {"source_file": source.to_string_lossy(), "purpose": "chat_attachment"}}),
    ];
    let mut reply = Value::Null;
    for (index, mut request) in setup.into_iter().enumerate() {
        let id = format!("s{index}");
        request["request_id"] = json!(id);
        live.write(&request);
        reply = until(&mut live, &id);
        assert_ne!(reply["kind"], "error", "{reply}");
    }
    let token = reply["data"]["document_token"].clone();
    live.write(&json!({"request_id": "chat", "kind": "ai.chat", "args": {
        "model": "test-model", "tools_enabled": true, "tool_profile": "full",
        "messages": [{"role": "user", "content": "Read the receipt"}],
        "attachments": [{"token": token, "new": true}]}}));
    let consent = loop {
        let record: Value = serde_json::from_str(&live.read()).unwrap();
        assert_eq!(record["request_id"], "chat", "{record}");
        if record["kind"] == "ai.chat.tool_consent_required" {
            break record;
        }
    };
    live.write(
        &json!({"request_id": "allow", "kind": "ai.tool_call.consent", "args": {
        "target_request_id": "chat", "call_id": consent["data"]["call_id"],
        "decision": "allow_once"}}),
    );
    assert!(gate.wait_started(), "the reader never called the model");
    // The model is still reading: another request is answered meanwhile.
    live.write(&json!({"request_id": "status", "kind": "status"}));
    let mut records = Vec::new();
    let status = loop {
        let record: Value = serde_json::from_str(&live.read()).unwrap();
        if record["request_id"] == "status" {
            break record;
        }
        records.push(record);
    };
    assert!(!gate.is_answered(), "status waited for the model");
    assert_eq!(status["kind"], "status");
    gate.release();
    let terminal = loop {
        let record: Value = serde_json::from_str(&live.read()).unwrap();
        if record["request_id"] == "chat"
            && (record["kind"] == "ai.chat" || record["kind"] == "error")
        {
            break record;
        }
        records.push(record);
    };
    live.write(&json!({"request_id": "stop", "kind": "daemon.shutdown"}));
    until(&mut live, "stop");
    live.finish();
    assert_eq!(terminal["kind"], "ai.chat", "{terminal}");
    let results: Vec<&Value> = records
        .iter()
        .chain([&terminal])
        .filter(|r| r["kind"] == "ai.chat.tool_result")
        .collect();
    assert_eq!(results.len(), 1, "{results:?}");
    assert_eq!(results[0]["data"]["ok"], true, "{}", results[0]);
    assert_eq!(results[0]["data"]["envelope"]["data"]["summary"]["rows"], 1);
}
