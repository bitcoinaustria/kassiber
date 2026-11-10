//! The chat turn (R6-7) against the scripted loopback provider, with a fake
//! daemon: record order, consent, cancellation, the persistence policy, the
//! privacy receipt and the turn's network consent. Whole streams are
//! compared with Python in `tests/test_rust_ai_chat.py`; these pin the
//! rules one at a time.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_ai::chat::history::{self, Persist};
use kassiber_ai::chat::{
    registry_key, validate, ActiveChats, ChatClient, ProviderSnapshot, Turn, TurnHost, Worker,
};
use kassiber_ai::executor::{Book, Maintenance, ReadHost};
use kassiber_ai::transport::{ChatRequest, ClientConfig, HttpClient, Receipt, RequestContext};
use kassiber_ai::ToolError;
use kassiber_envelope::{dict, get, AppError};
use kassiber_net::test_support::{Reply, ScriptedProvider};
use kassiber_net::{Gate, Ledger, Offline};
use kassiber_pyjson::{PyStr, Value};
use kassiber_store::rusqlite::Connection;

const KEY: &str = "chat-1";

fn s(text: &str) -> Value {
    kassiber_envelope::s(text)
}

fn load(raw: &str) -> Value {
    kassiber_pytext::loads(raw).unwrap()
}

fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

fn online() -> Gate {
    Gate::new(|| Offline::Online, Ledger::default())
}

fn message_item(text: &str) -> String {
    format!(
        r#"{{"type": "message", "role": "assistant", "status": "completed", "content": [{{"type": "output_text", "text": {}}}]}}"#,
        dumps(&s(text))
    )
}

fn function_call(call_id: &str, name: &str) -> String {
    format!(
        r#"{{"type": "function_call", "id": "fc_{call_id}", "call_id": "{call_id}", "name": "{name}", "arguments": "{{}}"}}"#
    )
}

fn responses(output: &[String], text: Option<&str>) -> Reply {
    let mut events = Vec::new();
    if let Some(text) = text {
        events.push(format!(
            r#"{{"type": "response.output_text.delta", "output_index": 0, "delta": {}}}"#,
            dumps(&s(text))
        ));
    }
    events.push(format!(
        r#"{{"type": "response.completed", "response": {{"id": "resp_1", "status": "completed", "output": [{}]}}}}"#,
        output.join(", ")
    ));
    let events: Vec<&str> = events.iter().map(String::as_str).collect();
    Reply::sse(&events, true)
}

fn answer(text: &str) -> Reply {
    responses(&[message_item(text)], Some(text))
}

/// What the fake book was asked to do: `(operation, value)`.
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<(String, Value)>>>);

impl Log {
    fn push(&self, operation: String, value: Value) {
        self.0.lock().unwrap().push((operation, value));
    }

    fn entries(&self) -> Vec<(String, Value)> {
        self.0.lock().unwrap().clone()
    }

    fn operations(&self) -> Vec<String> {
        self.entries()
            .into_iter()
            .map(|(operation, _)| operation)
            .collect()
    }
}

/// A book answering every read with a small payload and recording writes.
struct FakeBook<'a> {
    log: &'a Log,
}

impl Book for FakeBook<'_> {
    fn database_in_scope(&mut self, _data_root: &str) -> Result<bool, ToolError> {
        Ok(true)
    }
    fn context(&mut self) -> Result<(Value, Value), ToolError> {
        Ok((s("w1"), s("p1")))
    }
    fn maintain_for_read(&mut self, _sync_attempted: bool) -> Result<Maintenance, ToolError> {
        Ok(Maintenance::default())
    }
    fn read(
        &mut self,
        kind: &str,
        _arguments: &Value,
        _source: &'static str,
    ) -> Result<Value, ToolError> {
        Ok(dict([("read", s(kind))]))
    }
    fn mutate(
        &mut self,
        kind: &str,
        arguments: &Value,
        source: &'static str,
    ) -> Result<Value, ToolError> {
        self.log
            .push(format!("mutate:{kind}:{source}"), arguments.clone());
        Ok(dict([("wrote", s(kind))]))
    }
    fn custody_assisted_write(
        &mut self,
        kind: &str,
        _arguments: &Value,
        audit: &Value,
    ) -> Result<Value, ToolError> {
        self.log.push(format!("custody:{kind}"), audit.clone());
        Ok(dict([("wrote", s(kind))]))
    }
    fn append_custody_audit(&mut self, audit: &Value) -> Result<Value, ToolError> {
        self.log.push("audit".into(), audit.clone());
        Ok(audit.clone())
    }
    fn review_consent_preview(
        &mut self,
        arguments: &Value,
        full_access: bool,
    ) -> Result<Value, ToolError> {
        self.log.push("review_preview".into(), arguments.clone());
        Ok(dict([
            ("status", s("ready")),
            ("full_access", Value::Bool(full_access)),
        ]))
    }
}

#[derive(Default)]
struct Reads {
    log: Log,
}

impl ReadHost for Reads {
    fn on_request_loop(
        &mut self,
        work: &mut dyn FnMut(&mut dyn Book) -> Result<Value, ToolError>,
    ) -> Result<Value, ToolError> {
        work(&mut FakeBook { log: &self.log })
    }
}

/// Answers a consent as soon as the record asking for it is written.
type OnRecord = Box<dyn FnMut(&Value)>;

struct Host {
    records: Vec<Value>,
    reads: Reads,
    conn: Connection,
    encrypted: bool,
    on_record: Option<OnRecord>,
}

impl Host {
    fn new() -> Self {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE settings(key TEXT PRIMARY KEY, value TEXT);
             CREATE TABLE workspaces(id TEXT PRIMARY KEY, label TEXT);
             CREATE TABLE profiles(id TEXT PRIMARY KEY, workspace_id TEXT, label TEXT);
             CREATE TABLE ai_chat_sessions(id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL, profile_id TEXT NOT NULL, title TEXT NOT NULL, provider TEXT, model TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
             CREATE TABLE ai_chat_messages(id TEXT PRIMARY KEY, session_id TEXT NOT NULL, seq INTEGER NOT NULL, role TEXT NOT NULL, content TEXT NOT NULL, tool_calls_json TEXT, provenance_json TEXT, finish_reason TEXT, created_at TEXT NOT NULL);
             INSERT INTO workspaces VALUES('w1','Demo');
             INSERT INTO profiles VALUES('p1','w1','Main');",
        )
        .unwrap();
        Host {
            records: Vec::new(),
            reads: Reads::default(),
            conn,
            encrypted: false,
            on_record: None,
        }
    }

    fn kinds(&self) -> Vec<String> {
        self.records
            .iter()
            .map(|record| kassiber_envelope::py_str(get(record, "kind").unwrap()))
            .collect()
    }

    fn terminal(&self) -> &Value {
        let last = self.records.last().unwrap();
        assert_eq!(get(last, "kind"), Some(&s("ai.chat")), "{last:?}");
        get(last, "data").unwrap()
    }

    fn result(&self, call_id: &str) -> Value {
        self.records
            .iter()
            .filter(|record| get(record, "kind") == Some(&s("ai.chat.tool_result")))
            .map(|record| get(record, "data").unwrap().clone())
            .find(|data| get(data, "call_id") == Some(&s(call_id)))
            .unwrap()
    }
}

impl TurnHost for Host {
    fn emit(&mut self, record: Value) -> bool {
        if let Some(hook) = self.on_record.as_mut() {
            hook(&record);
        }
        self.records.push(record);
        true
    }
    fn emit_error(&mut self, error: &AppError) -> bool {
        self.records.push(error.envelope(None));
        true
    }
    fn reads(&mut self) -> &mut dyn ReadHost {
        &mut self.reads
    }
    fn persist(&mut self, mut request: Persist) -> Result<Option<PyStr>, AppError> {
        request.encrypted = self.encrypted;
        let mut unused =
            || -> Result<(String, String), AppError> { Ok(("w1".into(), "p1".into())) };
        history::persist(&self.conn, &request, &mut unused)
    }
    fn record_detected_api_format(&mut self, _: &str, _: &str, _: &str) -> Result<(), AppError> {
        Ok(())
    }
    fn now(&self) -> String {
        "2026-11-05T12:00:00Z".into()
    }
}

fn snapshot(provider: &ScriptedProvider) -> ProviderSnapshot {
    ProviderSnapshot::from_value(&dict([
        ("name", s("tool-local")),
        ("base_url", s(&provider.base_url())),
        ("kind", s("local")),
        ("on_device", Value::Bool(true)),
    ]))
}

fn turn(provider: &ScriptedProvider, registry: &ActiveChats, args: &str) -> Turn {
    let validated = validate(&load(args)).unwrap();
    Turn {
        key: Some(KEY.into()),
        provider: snapshot(provider),
        validated,
        state: dict([
            ("provider_kind", s("local")),
            ("provider_on_device", Value::Bool(true)),
            ("scope_workspace_id", s("w1")),
            ("scope_profile_id", s("p1")),
        ]),
        data_root: "/unused".into(),
        chat: registry.register(Some(KEY), None),
        registry: registry.clone(),
        gate: online(),
        locate: kassiber_ai::broker::default_locate(),
        consent_timeout: Some(Duration::from_millis(300)),
    }
}

const PLAIN: &str =
    r#"{"model": "test-model", "messages": [{"role": "user", "content": "hello"}]}"#;
const TOOLS: &str = r#"{"model": "test-model", "tools_enabled": true, "tool_profile": "full", "system_prompt_kind": "raw", "system_prompt": "Use tools.", "messages": [{"role": "user", "content": "hello"}]}"#;

#[test]
fn records_follow_python_order() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "responses", vec![answer("Hi there")]);
    let registry = ActiveChats::default();
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, PLAIN)).run(&mut host);
    let kinds = host.kinds();
    assert_eq!(
        &kinds[..3],
        ["ai.chat.status", "ai.chat.status", "ai.chat.status"]
    );
    let phases: Vec<Value> = host.records[..3]
        .iter()
        .map(|record| get(get(record, "data").unwrap(), "phase").unwrap().clone())
        .collect();
    assert_eq!(
        phases,
        [s("preparing"), s("connecting"), s("waiting_for_model")]
    );
    assert!(kinds[3..kinds.len() - 1]
        .iter()
        .all(|kind| kind == "ai.chat.delta"));
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("stop")));
    // The worker leaves the registry.
    assert!(registry.is_empty());
}

#[test]
fn consent_is_registered_before_the_tool_call_record() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let answering = registry.clone();
    // A client that answers the moment the call is announced, before the
    // consent record: the daemon must already be waiting for it.
    host.on_record = Some(Box::new(move |record| {
        let data = get(record, "data").cloned().unwrap_or(Value::Null);
        if get(record, "kind") == Some(&s("ai.chat.tool_call"))
            && get(&data, "needs_consent") == Some(&Value::Bool(true))
        {
            let call = kassiber_envelope::py_str(get(&data, "call_id").unwrap());
            assert!(answering.record_consent(KEY, &call, "deny"));
        }
    }));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    assert_eq!(
        get(&host.result("call_mut"), "reason"),
        Some(&s("user_denied"))
    );
    // A consent nobody waits on is not recorded.
    assert!(!registry.record_consent(KEY, "call_mut", "allow_once"));
}

/// Answers every consent request for `call_id` with `decision`.
fn answer_consent(
    registry: &ActiveChats,
    call_id: &'static str,
    decision: &'static str,
) -> OnRecord {
    let answering = registry.clone();
    Box::new(move |record| {
        if get(record, "kind") == Some(&s("ai.chat.tool_consent_required")) {
            assert!(answering.record_consent(KEY, call_id, decision));
        }
    })
}

fn consent_card(host: &Host, call_id: &str) -> Value {
    host.records
        .iter()
        .filter(|record| get(record, "kind") == Some(&s("ai.chat.tool_consent_required")))
        .map(|record| get(record, "data").unwrap().clone())
        .find(|data| get(data, "call_id") == Some(&s(call_id)))
        .unwrap()
}

const CUSTODY_ARGS: &str =
    r#"{"action": "reopen", "gap_id": "gap-1", "expected_input_version": 0}"#;

#[test]
fn an_allowed_write_runs_through_the_mutating_executor() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    host.on_record = Some(answer_consent(&registry, "call_mut", "allow_once"));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    let result = host.result("call_mut");
    assert_eq!(get(&result, "ok"), Some(&Value::Bool(true)), "{result:?}");
    // The route the desktop uses, authored by the assistant.
    assert_eq!(
        host.reads.log.operations(),
        ["mutate:ui.journals.process:ai_tool"]
    );
    // Allowed: the call is announced again without a consent flag.
    let announced: Vec<Value> = host
        .records
        .iter()
        .filter(|record| get(record, "kind") == Some(&s("ai.chat.tool_call")))
        .map(|record| {
            get(get(record, "data").unwrap(), "approval")
                .unwrap()
                .clone()
        })
        .collect();
    assert_eq!(announced, [s("required"), s("required")]);
}

#[test]
fn a_denied_write_runs_nothing() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    host.on_record = Some(answer_consent(&registry, "call_mut", "deny"));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    assert_eq!(
        get(&host.result("call_mut"), "reason"),
        Some(&s("user_denied"))
    );
    // Not a custody proposal: nothing is audited either.
    assert!(host.reads.log.entries().is_empty());
}

#[test]
fn a_full_access_chat_writes_without_asking() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let turn = turn(&provider, &registry, TOOLS);
    turn.chat.consent.set_permission_mode("full_access");
    Worker::new(turn).run(&mut host);
    assert!(!host
        .kinds()
        .contains(&"ai.chat.tool_consent_required".into()));
    assert_eq!(
        get(&host.result("call_mut"), "ok"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        host.reads.log.operations(),
        ["mutate:ui.journals.process:ai_tool"]
    );
}

#[test]
fn an_allowed_custody_write_carries_its_consent_audit() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[function_call_with(
                    "call_gap",
                    "ui_custody_review_apply",
                    CUSTODY_ARGS,
                )],
                None,
            ),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    host.on_record = Some(answer_consent(&registry, "call_gap", "allow_once"));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    let result = host.result("call_gap");
    assert_eq!(get(&result, "ok"), Some(&Value::Bool(true)), "{result:?}");
    let entries = host.reads.log.entries();
    assert_eq!(entries.len(), 1);
    let (operation, audit) = &entries[0];
    assert_eq!(operation, "custody:ui.custody.review.apply");
    let field = |key: &str| get(audit, key).cloned().unwrap();
    assert_eq!(field("call_id"), s("call_gap"));
    assert_eq!(field("provider_kind"), s("local"));
    assert_eq!(field("model"), s("test-model"));
    assert_eq!(field("consent_decision"), s("allow_once"));
    assert_eq!(field("consent_requested_at"), s("2026-11-05T12:00:00Z"));
    assert_eq!(field("consent_decided_at"), s("2026-11-05T12:00:00Z"));
    assert_eq!(field("execution_status"), s("executed"));
    assert_eq!(field("workspace_id"), s("w1"));
}

#[test]
fn denied_and_timed_out_custody_proposals_are_audited() {
    for (decision, code) in [("deny", "user_denied"), ("", "consent_timeout")] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![
                responses(
                    &[function_call_with(
                        "call_gap",
                        "ui_custody_review_apply",
                        CUSTODY_ARGS,
                    )],
                    None,
                ),
                answer("ok"),
            ],
        );
        let registry = ActiveChats::default();
        let mut host = Host::new();
        if !decision.is_empty() {
            host.on_record = Some(answer_consent(&registry, "call_gap", "deny"));
        }
        Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
        assert_eq!(get(&host.result("call_gap"), "reason"), Some(&s(code)));
        let entries = host.reads.log.entries();
        assert_eq!(entries.len(), 1, "{decision}");
        let (operation, audit) = &entries[0];
        assert_eq!(operation, "audit");
        assert_eq!(get(audit, "execution_status"), Some(&s("denied")));
        assert_eq!(get(audit, "execution_code"), Some(&s(code)));
        let recorded = if decision.is_empty() {
            "consent_timeout"
        } else {
            decision
        };
        assert_eq!(get(audit, "consent_decision"), Some(&s(recorded)));
    }
}

#[test]
fn cancelled_custody_proposals_are_audited() {
    // Cancelled while the card waits, and cancelled before the card.
    for (on_kind, code) in [
        ("ai.chat.tool_consent_required", "cancelled"),
        ("ai.chat.tool_call", "cancelled_before_consent"),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![responses(
                &[function_call_with(
                    "call_gap",
                    "ui_custody_review_apply",
                    CUSTODY_ARGS,
                )],
                None,
            )],
        );
        let registry = ActiveChats::default();
        let mut host = Host::new();
        let cancelling = registry.clone();
        host.on_record = Some(Box::new(move |record| {
            if get(record, "kind") == Some(&s(on_kind)) {
                cancelling.cancel(KEY);
            }
        }));
        Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
        assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("cancelled")));
        let entries = host.reads.log.entries();
        assert_eq!(entries.len(), 1, "{on_kind}");
        let (operation, audit) = &entries[0];
        assert_eq!(operation, "audit");
        assert_eq!(get(audit, "consent_decision"), Some(&s("cancelled")));
        assert_eq!(get(audit, "execution_status"), Some(&s("cancelled")));
        assert_eq!(get(audit, "execution_code"), Some(&s(code)));
    }
}

#[test]
fn consent_cards_show_what_the_book_recomputes() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[
                    function_call_with(
                        "call_review",
                        "ui_review_apply",
                        r#"{"artifact": {"operations": [{"op": "x"}]}}"#,
                    ),
                    function_call_with(
                        "call_rollback",
                        "ui_imports_rollback",
                        r#"{"batch": "b1"}"#,
                    ),
                    function_call_with(
                        "call_acquire",
                        "ui_chain_analysis_acquire_apply",
                        r#"{"plan": {"args": {}}}"#,
                    ),
                ],
                None,
            ),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let answering = registry.clone();
    host.on_record = Some(Box::new(move |record| {
        if get(record, "kind") == Some(&s("ai.chat.tool_consent_required")) {
            let data = get(record, "data").unwrap();
            let call = kassiber_envelope::py_str(get(data, "call_id").unwrap());
            assert!(answering.record_consent(KEY, &call, "deny"));
        }
    }));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    // The review card carries the recomputed accounting effects.
    let review = consent_card(&host, "call_review");
    assert_eq!(
        get(&review, "review_preview"),
        Some(&dict([
            ("status", s("ready")),
            ("full_access", Value::Bool(true))
        ]))
    );
    // A plan-bound apply shows the daemon's plan (this book has none).
    let rollback = consent_card(&host, "call_rollback");
    assert_eq!(
        get(&rollback, "arguments_preview"),
        Some(&dict([
            ("status", s("unavailable")),
            ("kind", s("ui.imports.rollback")),
            ("code", s("not_implemented")),
        ]))
    );
    // An acquisition is consented only on a plan the book recomputes.
    let acquire = consent_card(&host, "call_acquire");
    assert_eq!(
        get(&acquire, "arguments_preview"),
        Some(&dict([
            ("status", s("blocked")),
            ("code", s("not_implemented"))
        ]))
    );
    assert!(get(&acquire, "review_preview").is_none());
}

#[test]
fn a_cancel_ends_with_a_cancelled_terminal() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![responses(
            &[function_call("call_mut", "ui_journals_process")],
            None,
        )],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let cancelling = registry.clone();
    host.on_record = Some(Box::new(move |record| {
        if get(record, "kind") == Some(&s("ai.chat.tool_consent_required")) {
            assert_eq!(cancelling.cancel(KEY), (true, false));
        }
    }));
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("cancelled")));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn an_early_cancel_contacts_nothing() {
    let provider = ScriptedProvider::new().unwrap();
    let registry = ActiveChats::default();
    assert_eq!(registry.cancel(KEY), (false, true));
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, PLAIN)).run(&mut host);
    assert_eq!(host.kinds(), ["ai.chat"]);
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("cancelled")));
    assert!(provider.requests().is_empty());
}

#[test]
fn history_is_stored_only_under_the_policy() {
    let run = |encrypted: bool, mode: Option<&str>, args: &str| {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "responses", vec![answer("stored")]);
        let registry = ActiveChats::default();
        let mut host = Host::new();
        host.encrypted = encrypted;
        if let Some(mode) = mode {
            history::set_history_mode(&host.conn, mode).unwrap();
        }
        Worker::new(turn(&provider, &registry, args)).run(&mut host);
        get(host.terminal(), "session_id").cloned().unwrap()
    };
    let auto =
        r#"{"model": "m", "persist": "auto", "messages": [{"role": "user", "content": "hello"}]}"#;
    // `auto` writes only into an encrypted book.
    assert_eq!(run(false, None, auto), Value::Null);
    assert!(matches!(run(true, None, auto), Value::Str(_)));
    // `on` writes anywhere, but only for a request that asked.
    assert!(matches!(run(false, Some("on"), auto), Value::Str(_)));
    assert_eq!(run(false, Some("on"), PLAIN), Value::Null);
    // `off` never writes; Incognito never writes.
    assert_eq!(run(true, Some("off"), auto), Value::Null);
    let incognito =
        r#"{"model": "m", "persist": false, "messages": [{"role": "user", "content": "hello"}]}"#;
    assert_eq!(run(true, Some("on"), incognito), Value::Null);
}

#[test]
fn the_privacy_receipt_counts_what_was_sent() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_status", "status")], None),
            answer("ready"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    let receipt = get(
        get(host.terminal(), "provenance").unwrap(),
        "privacy_receipt",
    )
    .unwrap();
    assert_eq!(get(receipt, "egress_records"), Some(&Value::from(2usize)));
    assert_eq!(get(receipt, "egress_endpoints"), Some(&Value::from(1usize)));
    assert_eq!(
        get(receipt, "egress_subsystems"),
        Some(&Value::List(vec![s("ai")]))
    );
    // Python's estimate: method, path, each header as key + value + 4, body.
    let headers = "Accept".len()
        + "text/event-stream".len()
        + 4
        + "User-agent".len()
        + "kassiber/ai".len()
        + 4
        + "Content-type".len()
        + "application/json".len()
        + 4;
    let expected: usize = provider
        .bodies("responses")
        .iter()
        .map(|body| "POST".len() + "/v1/responses".len() + headers + body.len())
        .sum();
    assert_eq!(
        get(receipt, "egress_bytes_out"),
        Some(&Value::from(expected))
    );
    assert_eq!(get(receipt, "tools_executed"), Some(&Value::from(1usize)));
}

#[test]
fn cancelling_revokes_the_turns_network_consent() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue("POST", "responses", vec![answer("never")]);
    let client = ChatClient::for_provider(
        &snapshot(&provider),
        Duration::from_secs(5),
        online(),
        kassiber_ai::broker::default_locate(),
        Receipt::default(),
    )
    .unwrap();
    let revocation = client.revocation().unwrap();
    assert!(!revocation.is_revoked());
    (client.canceller())();
    assert!(revocation.is_revoked());
    // A revoked consent alone refuses the next request.
    let mut config = ClientConfig::new(provider.base_url());
    config.on_device = true;
    let mut http = HttpClient::new(config, online());
    http.revocation().revoke();
    let context =
        RequestContext::from_messages(&[dict([("role", s("user")), ("content", s("x"))])]);
    let result = http.stream_chat(
        &ChatRequest {
            context: &context,
            model: "m",
            options: &dict::<&str>([]),
            tools: None,
            tool_choice: &Value::Null,
        },
        &mut |_| Ok(()),
    );
    assert!(result.is_err());
    assert!(provider.requests().is_empty());
}

#[test]
fn review_and_source_funds_questions_get_16_iterations() {
    let budget = |args: &str| validate(&load(args)).unwrap().tool_loop_max_iterations;
    assert_eq!(
        budget(
            r#"{"model": "m", "tools_enabled": true, "messages": [{"role": "user", "content": "review my quarantine"}]}"#
        ),
        16
    );
    assert_eq!(
        budget(
            r#"{"model": "m", "tools_enabled": true, "messages": [{"role": "user", "content": "hello"}]}"#
        ),
        8
    );
    // An explicit budget, or a tool-free chat, keeps its own.
    assert_eq!(
        budget(
            r#"{"model": "m", "tools_enabled": true, "tool_loop_max_iterations": 3, "messages": [{"role": "user", "content": "review my quarantine"}]}"#
        ),
        3
    );
    assert_eq!(
        budget(
            r#"{"model": "m", "messages": [{"role": "user", "content": "review my quarantine"}]}"#
        ),
        8
    );
}

#[test]
fn cross_book_reads_need_an_explicit_book_set_phrase() {
    let ask = |text: &str| {
        kassiber_ai::planner::cross_book_read_allowed(&[dict([
            ("role", s("user")),
            ("content", s(text)),
        ])])
    };
    for text in [
        "Show all books",
        "Gesamtvermögen bitte",
        "the treasury view",
        "über alle Bücher",
    ] {
        assert!(ask(text), "{text}");
    }
    for text in ["show my books", "this book", "workspace"] {
        assert!(!ask(text), "{text}");
    }
}

#[test]
fn the_tool_loop_stops_at_its_budget() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_1", "status")], None),
            responses(&[function_call("call_2", "status")], None),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let args = TOOLS.replace(
        r#""tools_enabled": true,"#,
        r#""tools_enabled": true, "tool_loop_max_iterations": 2,"#,
    );
    Worker::new(turn(&provider, &registry, &args)).run(&mut host);
    assert_eq!(
        get(host.terminal(), "finish_reason"),
        Some(&s("tool_loop_max_iterations"))
    );
    assert_eq!(provider.requests().len(), 2);
}

#[test]
fn registry_keys_are_python_str_of_the_request_id() {
    assert_eq!(registry_key(Some(&Value::from(3i64))).as_deref(), Some("3"));
}

fn truncated(output: &[String]) -> Reply {
    let event = format!(
        r#"{{"type": "response.incomplete", "response": {{"id": "resp_1", "status": "incomplete", "incomplete_details": {{"reason": "max_output_tokens"}}, "output": [{}]}}}}"#,
        output.join(", ")
    );
    Reply::sse(&[event.as_str()], true)
}

#[test]
fn a_cut_off_answer_runs_no_tool() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![truncated(&[
            message_item("Partial"),
            function_call("call_status", "status"),
        ])],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("length")));
    assert!(!host
        .kinds()
        .iter()
        .any(|kind| kind == "ai.chat.tool_result"));
    assert_eq!(provider.requests().len(), 1);
}

#[test]
fn a_repeated_call_id_is_denied_without_consent() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[
                    function_call("dup", "status"),
                    function_call("dup", "status"),
                ],
                None,
            ),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    let results: Vec<Value> = host
        .records
        .iter()
        .filter(|record| get(record, "kind") == Some(&s("ai.chat.tool_result")))
        .map(|record| {
            get(get(record, "data").unwrap(), "reason")
                .cloned()
                .unwrap_or(Value::Null)
        })
        .collect();
    assert_eq!(results, [Value::Null, s("duplicate_tool_call_id")]);
}

#[test]
fn a_tool_not_offered_this_turn_never_runs() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[function_call("call_sf", "ui_source_funds_sources_create")],
                None,
            ),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let core = TOOLS.replace(r#""tool_profile": "full""#, r#""tool_profile": "core""#);
    Worker::new(turn(&provider, &registry, &core)).run(&mut host);
    assert_eq!(
        get(&host.result("call_sf"), "reason"),
        Some(&s("tool_not_advertised"))
    );
    // Not offered: no consent is asked for it either.
    assert!(!host
        .kinds()
        .iter()
        .any(|kind| kind == "ai.chat.tool_consent_required"));
}

#[test]
fn on_device_tools_are_never_offered_to_other_providers() {
    let offered = |on_device: bool| {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue("POST", "responses", vec![answer("ok")]);
        let registry = ActiveChats::default();
        let mut host = Host::new();
        let mut chat = turn(&provider, &registry, TOOLS);
        kassiber_envelope::set(
            &mut chat.state,
            "provider_on_device",
            Value::Bool(on_device),
        );
        Worker::new(chat).run(&mut host);
        String::from_utf8(provider.bodies("responses")[0].clone()).unwrap()
    };
    assert!(offered(true).contains("ui_chain_analysis_sources_"));
    let remote = offered(false);
    assert!(!remote.contains("ui_chain_analysis_sources_"));
    assert!(!remote.contains("ui_chain_analysis_acquire_apply"));
}

fn function_call_with(call_id: &str, name: &str, arguments: &str) -> String {
    format!(
        r#"{{"type": "function_call", "id": "fc_{call_id}", "call_id": "{call_id}", "name": "{name}", "arguments": {}}}"#,
        dumps(&s(arguments))
    )
}

#[test]
fn a_tool_turn_saves_every_leg_of_its_answer() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[message_item("Checking. "), function_call("c1", "status")],
                Some("Checking. "),
            ),
            answer("All good."),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    history::set_history_mode(&host.conn, "on").unwrap();
    let args = r#"{"model": "test-model", "tools_enabled": true, "tool_profile": "full", "persist": true, "system_prompt_kind": "raw", "system_prompt": "Use tools.", "messages": [{"role": "user", "content": "hello"}]}"#;
    Worker::new(turn(&provider, &registry, args)).run(&mut host);
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("stop")));
    // The answer the user saw, every leg of it (a26567dfa).
    let saved: String = host
        .conn
        .query_row(
            "SELECT content FROM ai_chat_messages WHERE role = 'assistant'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(saved, "Checking. All good.");
}

#[test]
fn a_call_the_consent_card_cannot_describe_is_refused_before_any_card() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(
                &[function_call_with(
                    "bad",
                    "ui_transfers_components_apply",
                    r#"{"action": ["a"]}"#,
                )],
                None,
            ),
            answer("Sorry."),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    Worker::new(turn(&provider, &registry, TOOLS)).run(&mut host);
    // No consent card; the model gets the validation result and the turn
    // goes on (b2c0d9447).
    assert!(!host
        .kinds()
        .contains(&"ai.chat.tool_consent_required".to_owned()));
    let call = host
        .records
        .iter()
        .find(|record| get(record, "kind") == Some(&s("ai.chat.tool_call")))
        .unwrap();
    assert_eq!(
        get(get(call, "data").unwrap(), "needs_consent"),
        Some(&Value::Bool(false))
    );
    let result = host.result("bad");
    assert_eq!(get(&result, "reason"), Some(&s("validation")));
    assert_eq!(
        get(&result, "message"),
        Some(&s("Custody component review action is unsupported"))
    );
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("stop")));
}

#[test]
fn every_tool_is_offered_unless_the_caller_narrows_it() {
    // #684: the default profile is `full`.
    assert_eq!(validate(&load(PLAIN)).unwrap().tool_profile, "full");
    let scoped = r#"{"model": "m", "tool_profile": "scoped", "messages": [{"role": "user", "content": "hi"}]}"#;
    assert_eq!(validate(&load(scoped)).unwrap().tool_profile, "scoped");
    // An absent permission mode asks for every change.
    assert_eq!(validate(&load(PLAIN)).unwrap().permission_mode, "approval");
    let auto = r#"{"model": "m", "permission_mode": "auto", "messages": [{"role": "user", "content": "hi"}]}"#;
    assert_eq!(validate(&load(auto)).unwrap().permission_mode, "auto");
    let bad = r#"{"model": "m", "permission_mode": "yolo", "messages": [{"role": "user", "content": "hi"}]}"#;
    let Err(error) = validate(&load(bad)) else {
        panic!("an unknown mode is refused")
    };
    assert_eq!(
        error.message,
        "ai.chat permission_mode must be approval, auto, or full_access"
    );
}

fn approvals(host: &Host, call_id: &str) -> Vec<Value> {
    host.records
        .iter()
        .filter(|record| get(record, "kind") == Some(&s("ai.chat.tool_call")))
        .map(|record| get(record, "data").unwrap())
        .filter(|data| get(data, "call_id") == Some(&s(call_id)))
        .map(|data| get(data, "approval").cloned().unwrap_or(Value::Null))
        .collect()
}

#[test]
fn a_permission_mode_approves_without_asking() {
    // `auto` approves a reversible tool that stays on this machine;
    // `full_access` approves a network tool too (#684).
    for (mode, call, tool, approval) in [
        ("auto", "call_mut", "ui_journals_process", "auto"),
        ("full_access", "call_sync", "ui_wallets_sync", "full_access"),
    ] {
        let provider = ScriptedProvider::new().unwrap();
        provider.queue(
            "POST",
            "responses",
            vec![responses(&[function_call(call, tool)], None), answer("ok")],
        );
        let registry = ActiveChats::default();
        let mut host = Host::new();
        let args = TOOLS.replace(
            r#""tool_profile": "full","#,
            &format!(r#""tool_profile": "full", "permission_mode": "{mode}","#),
        );
        let turn = turn(&provider, &registry, &args);
        turn.chat.consent.set_permission_mode(mode);
        Worker::new(turn).run(&mut host);
        assert!(!host
            .kinds()
            .contains(&"ai.chat.tool_consent_required".to_owned()));
        assert_eq!(approvals(&host, call), [s(approval), s(approval)], "{mode}");
    }
    // `auto` still asks for a tool that leaves the machine.
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_sync", "ui_wallets_sync")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let turn = turn(&provider, &registry, TOOLS);
    turn.chat.consent.set_permission_mode("auto");
    Worker::new(turn).run(&mut host);
    let card = host
        .records
        .iter()
        .find(|record| get(record, "kind") == Some(&s("ai.chat.tool_consent_required")))
        .map(|record| get(record, "data").unwrap().clone())
        .unwrap();
    assert_eq!(get(&card, "egresses"), Some(&Value::Bool(true)));
    assert_eq!(get(&card, "reversible"), Some(&Value::Bool(false)));
    assert_eq!(get(&card, "can_allow_session"), Some(&Value::Bool(true)));
    assert_eq!(approvals(&host, "call_sync"), [s("required")]);
}

#[test]
fn switching_modes_releases_a_waiting_call() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let turn = turn(&provider, &registry, TOOLS);
    let chat = std::sync::Arc::clone(&turn.chat);
    let switching = registry.clone();
    let switcher = std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while chat.consent.waiting() == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        switching.set_permission_mode(KEY, "auto")
    });
    Worker::new(turn).run(&mut host);
    assert_eq!(switcher.join().unwrap(), Some(vec!["call_mut".to_owned()]));
    // Asked first; the switch approved it, and says so.
    assert_eq!(approvals(&host, "call_mut"), [s("required"), s("auto")]);
    // An unknown chat is not found.
    assert_eq!(registry.set_permission_mode("nobody", "auto"), None);
}

#[test]
fn an_allow_for_this_chat_carries_to_the_next_turn() {
    let registry = ActiveChats::default();
    let allowed: std::collections::HashSet<String> =
        ["ui.journals.process".to_owned()].into_iter().collect();
    registry.remember_session_allows(Some("session-1"), &allowed);
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let mut host = Host::new();
    let mut turn = turn(&provider, &registry, TOOLS);
    turn.chat = registry.register(Some(KEY), Some("session-1"));
    Worker::new(turn).run(&mut host);
    assert!(!host
        .kinds()
        .contains(&"ai.chat.tool_consent_required".to_owned()));
    assert_eq!(approvals(&host, "call_mut"), [s("session"), s("session")]);
}

/// A turn on a CLI proxy provider, as `internal.ai.chat.provider` resolves
/// it: the chat model's format pinned from the saved model list, off this
/// device, with full agent access.
fn proxy_turn(
    provider: &ScriptedProvider,
    registry: &ActiveChats,
    format: &str,
    args: &str,
) -> Turn {
    let validated = validate(&load(args)).unwrap();
    Turn {
        key: Some(KEY.into()),
        provider: ProviderSnapshot::from_value(&dict([
            ("name", s("cli-proxy")),
            ("base_url", s(&provider.base_url())),
            ("api_key", s("sk-proxy")),
            ("kind", s("cli_proxy")),
            ("api_format", s(format)),
            ("detected_api_format", Value::Null),
            ("cli_proxy", Value::Bool(true)),
            ("on_device", Value::Bool(false)),
            ("full_access", Value::Bool(true)),
        ])),
        validated,
        state: dict([
            ("provider_kind", s("cli_proxy")),
            ("provider_on_device", Value::Bool(false)),
            ("provider_full_access", Value::Bool(true)),
            ("scope_workspace_id", s("w1")),
            ("scope_profile_id", s("p1")),
        ]),
        data_root: "/unused".into(),
        chat: registry.register(Some(KEY), None),
        registry: registry.clone(),
        gate: online(),
        locate: kassiber_ai::broker::default_locate(),
        consent_timeout: Some(Duration::from_millis(300)),
    }
}

fn chat_completions_call(call_id: &str, name: &str) -> Reply {
    let call = format!(
        r#"{{"choices":[{{"index":0,"delta":{{"role":"assistant","tool_calls":[{{"index":0,"id":"{call_id}","type":"function","function":{{"name":"{name}","arguments":"{{}}"}}}}]}},"finish_reason":null}}]}}"#
    );
    let finish = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#;
    Reply::sse(&[call.as_str(), finish], true)
}

fn chat_completions_answer(text: &str) -> Reply {
    let delta = format!(
        r#"{{"choices":[{{"index":0,"delta":{{"role":"assistant","content":{}}},"finish_reason":null}}]}}"#,
        dumps(&s(text))
    );
    let finish = r#"{"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}"#;
    Reply::sse(&[delta.as_str(), finish], true)
}

const PROXY_TOOLS: &str = r#"{"model": "claude-sonnet-5-5", "tools_enabled": true, "tool_profile": "full", "system_prompt_kind": "raw", "system_prompt": "Use tools.", "options": {"reasoning_effort": "xhigh"}, "messages": [{"role": "user", "content": "process the journals"}]}"#;

#[test]
fn a_cli_proxy_chat_asks_consent_and_writes_through_chat_completions() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "chat/completions",
        vec![
            chat_completions_call("call_mut", "ui_journals_process"),
            chat_completions_answer("Processed."),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    host.on_record = Some(answer_consent(&registry, "call_mut", "allow_once"));
    Worker::new(proxy_turn(
        &provider,
        &registry,
        "chat_completions",
        PROXY_TOOLS,
    ))
    .run(&mut host);
    assert!(host
        .kinds()
        .contains(&"ai.chat.tool_consent_required".into()));
    assert_eq!(
        get(&host.result("call_mut"), "ok"),
        Some(&Value::Bool(true))
    );
    assert_eq!(
        host.reads.log.operations(),
        ["mutate:ui.journals.process:ai_tool"]
    );
    assert_eq!(get(host.terminal(), "finish_reason"), Some(&s("stop")));
    // Every leg went to the model's format, never the Responses API, with
    // the proxy's own effort level and the key.
    let requests = provider.requests();
    assert!(requests.iter().all(|r| r.path == "/v1/chat/completions"));
    assert_eq!(requests.len(), 2);
    assert_eq!(
        requests[0].authorization.as_deref(),
        Some("Bearer sk-proxy")
    );
    for body in provider.bodies("chat/completions") {
        let body = load(&String::from_utf8_lossy(&body));
        assert_eq!(get(&body, "reasoning_effort"), Some(&s("xhigh")));
        assert_eq!(get(&body, "store"), Some(&Value::Bool(false)));
    }
}

#[test]
fn a_cli_proxy_codex_chat_in_full_access_writes_over_responses_without_asking() {
    let provider = ScriptedProvider::new().unwrap();
    provider.queue(
        "POST",
        "responses",
        vec![
            responses(&[function_call("call_mut", "ui_journals_process")], None),
            answer("ok"),
        ],
    );
    let registry = ActiveChats::default();
    let mut host = Host::new();
    let args = PROXY_TOOLS
        .replace("claude-sonnet-5-5", "gpt-6.1-sol")
        .replace("xhigh", "ultra");
    let turn = proxy_turn(&provider, &registry, "responses", &args);
    turn.chat.consent.set_permission_mode("full_access");
    Worker::new(turn).run(&mut host);
    assert!(!host
        .kinds()
        .contains(&"ai.chat.tool_consent_required".into()));
    assert_eq!(
        host.reads.log.operations(),
        ["mutate:ui.journals.process:ai_tool"]
    );
    let bodies = provider.bodies("responses");
    assert_eq!(bodies.len(), 2);
    let first = load(&String::from_utf8_lossy(&bodies[0]));
    assert_eq!(
        get(get(&first, "reasoning").unwrap(), "effort"),
        Some(&s("max"))
    );
    assert!(provider.bodies("chat/completions").is_empty());
}
