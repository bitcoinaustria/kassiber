//! Ports of `tests/test_ai_broker.py` (`BrokerClientTest`) and the broker
//! classes of `tests/test_ai_fast_mode.py`. The fake brokers are POSIX `sh`
//! scripts the broker program runs as `/bin/sh <script>` in place of
//! `kassiber --provider-broker`, standing in for the Python fakes the pytest
//! suite installed through `KASSIBER_AI_PROVIDER_BROKER`. Every gate here arms the attempt observer,
//! so each launch is counted.
#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kassiber_ai::broker::{runtime_status, single_result, BrokerChat, BrokerClient, Locate, Scope};
use kassiber_ai::providers::cli_provider_for_locator;
use kassiber_ai::transport::{ChatDelta, RequestContext};
use kassiber_envelope::{dict, s, AppError};
use kassiber_net::{BrokerProgram, Gate, Ledger, Offline, Operation};
use kassiber_pyjson::Value;

/// Shared helpers every fake starts with: `emit` writes one JSONL record,
/// `field` reads a string field of the request, `require` fails the broker
/// (exit 3, so the client sees no `done`) unless the request holds one of the
/// given fragments, and `refuse` fails it when the request holds the fragment.
const FAKE_PRELUDE: &str = r#"emit() { printf '%s\n' "$1"; }
field() { printf '%s\n' "$2" | sed -n "s/.*\"$1\": \"\([^\"]*\)\".*/\1/p"; }
require() {
  for fragment in "$@"; do
    case "$request" in *"$fragment"*) return 0 ;; esac
  done
  printf 'fake broker: request lacks %s\n' "$1" >&2
  exit 3
}
refuse() {
  case "$request" in
    *"$1"*) printf 'fake broker: request holds %s\n' "$1" >&2; exit 3 ;;
  esac
}
"#;

const FAKE_BROKER: &str = r#"
IFS= read -r request
printf '%s\n' "$request" >> "$LOG"
case "$request" in
  '{"command": "status"'*)
    emit '{"type":"result","data":[{"provider":"codex","state":"ready","models":[]}]}'
    exit 0
    ;;
  '{"command": "models"'*)
    emit '{"type":"result","data":[{"id":"model-a"}]}'
    exit 0
    ;;
esac
case "$(field model "$request")" in
  wait)
    sleep 30
    ;;
  sensitive)
    # Option order is not part of the contract; the set is.
    require '"tools": [], "options": {"sensitive_context": true, "reasoning_effort": "high"}}' \
      '"tools": [], "options": {"reasoning_effort": "high", "sensitive_context": true}}'
    emit '{"type":"delta","content":"selected only"}'
    emit '{"type":"done","finish_reason":"stop","provider_session_id":"must-not-retain"}'
    ;;
  stateless)
    # Exactly one tool, by name, and exactly these options.
    require '"tools": [{"name": "ui_reports_tax_summary", '
    refuse '}, {"name": '
    require '}], "options": {"stateless_session": true, "reasoning_effort": "high"}}' \
      '}], "options": {"reasoning_effort": "high", "stateless_session": true}}'
    emit '{"type":"delta","content":"unsaved"}'
    emit '{"type":"done","finish_reason":"stop","provider_session_id":"must-not-retain"}'
    ;;
  tool-call)
    emit '{"type":"tool_call","call_id":"native-call-1","name":"ui_reports_tax_summary","arguments":{}}'
    IFS= read -r request
    printf '%s\n' "$request" >> "$LOG"
    require '{"command": "tool_results", "results": [{"call_id": "native-call-1", '
    emit '{"type":"delta","content":"report ready"}'
    emit '{"type":"done","finish_reason":"stop","provider_session_id":"native-session-1"}'
    ;;
  *)
    emit '{"type":"delta","content":"hello "}'
    emit '{"type":"delta","content":"world"}'
    emit '{"type":"done","finish_reason":"stop","provider_session_id":"native-session-1"}'
    ;;
esac
"#;

/// A scratch directory under the system temporary directory, removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(label: &str) -> Self {
        static COUNTER: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "kassiber-broker-test-{label}-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The shell that runs the fake brokers.
fn shell() -> PathBuf {
    kassiber_net::which("sh").expect("sh on PATH for the fake brokers")
}

/// The broker program for a fake: the shell with the script.
fn program(script: &Path) -> BrokerProgram {
    BrokerProgram::new(Some(shell()), vec![script.as_os_str().to_owned()])
}

/// A fake broker under `scratch`, with `LOG` naming its request log.
struct Fake {
    _scratch: Scratch,
    script: PathBuf,
    log: PathBuf,
    gate: Gate,
}

impl Fake {
    fn new(body: &str) -> Self {
        let scratch = Scratch::new("fake");
        let script = scratch.path().join("fake_broker.sh");
        let log = scratch.path().join("requests.jsonl");
        let log_path = log.to_str().unwrap();
        assert!(!log_path.contains('\''), "{log_path}");
        std::fs::write(&script, format!("LOG='{log_path}'\n{FAKE_PRELUDE}{body}")).unwrap();
        let gate = Gate::new(|| Offline::Online, Ledger::default())
            .with_test_observer(false)
            .with_broker_fixture(program(&script));
        Self {
            _scratch: scratch,
            script,
            log,
            gate,
        }
    }

    fn standard() -> Self {
        Self::new(FAKE_BROKER)
    }

    fn program(&self) -> BrokerProgram {
        program(&self.script)
    }

    fn locate(&self) -> Locate {
        let program = self.program();
        Arc::new(move || program.clone())
    }

    fn client(&self, locator: &str) -> BrokerClient {
        BrokerClient::new(locator, Duration::from_secs(120), self.gate.clone())
            .unwrap()
            .with_locate(self.locate())
    }

    fn launches(&self) -> usize {
        self.gate
            .attempts()
            .records
            .iter()
            .filter(|record| {
                record.operation == Operation::Process && record.host == "provider-broker"
            })
            .count()
    }

    fn requests(&self) -> Vec<Value> {
        std::fs::read_to_string(&self.log)
            .unwrap_or_default()
            .lines()
            .map(|line| kassiber_pytext::loads(line).unwrap())
            .collect()
    }
}

fn json(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

fn messages(text: &str) -> Vec<Value> {
    vec![dict([("role", s("user")), ("content", s(text))])]
}

fn chat<'a>(
    messages: Option<&'a [Value]>,
    context: Option<&'a RequestContext>,
    model: &'a str,
    options: &'a Value,
    tools: &'a [Value],
) -> BrokerChat<'a> {
    BrokerChat {
        messages,
        context,
        model,
        options,
        tools,
    }
}

fn collect(
    client: &mut BrokerClient,
    request: &BrokerChat<'_>,
) -> Result<Vec<ChatDelta>, AppError> {
    let mut chunks = Vec::new();
    client.stream_chat(request, &mut |chunk| {
        chunks.push(chunk);
        Ok(())
    })?;
    Ok(chunks)
}

fn content(chunks: &[ChatDelta]) -> String {
    chunks
        .iter()
        .filter_map(|chunk| kassiber_envelope::get(&chunk.delta, "content"))
        .map(kassiber_envelope::py_str)
        .collect()
}

fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    kassiber_envelope::get(value, key).unwrap_or(&Value::Null)
}

/// Whether `pid` still runs; a zombie awaiting its reaper does not.
fn process_running(pid: u32) -> bool {
    match std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        Ok(stat) => stat
            .rsplit_once(')')
            .is_some_and(|(_, rest)| rest.split_whitespace().next() != Some("Z")),
        Err(_) => false,
    }
}

fn wait_until_gone(pid: u32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if !process_running(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

#[test]
fn cancel_is_latched_before_resolution_spawn_registration_and_send() {
    for stage in ["before", "resolution", "spawn", "serialization"] {
        let fake = Fake::standard();
        let mut client = fake.client("claude-cli://default");
        if stage == "before" {
            client.canceller().cancel();
        } else {
            client = client.with_stage_hook(Box::new(move |name, canceller| {
                if name == stage {
                    canceller.cancel();
                }
            }));
        }
        let options = json(r#"{"sensitive_context": true}"#);
        let messages = messages("selected sensitive fixture");
        let error = collect(
            &mut client,
            &chat(Some(&messages), None, "test", &options, &[]),
        )
        .unwrap_err();
        assert_eq!(error.code, "ai_cancelled", "{stage}");
        assert!(fake.requests().is_empty(), "{stage}: a request was sent");
        assert!(!client.has_process(), "{stage}");
        if matches!(stage, "before" | "resolution") {
            assert_eq!(fake.launches(), 0, "{stage}");
        }
    }
}

#[test]
fn cancellation_does_not_wait_for_a_blocked_prompt_writer() {
    // The fake never reads its stdin, so a prompt larger than the pipe
    // buffer blocks the writer.
    let fake = Fake::new("sleep 30\n");
    let writing = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&writing);
    let mut client =
        fake.client("codex-cli://default")
            .with_stage_hook(Box::new(move |name, _| {
                if name == "serialization" {
                    flag.store(true, Ordering::SeqCst);
                }
            }));
    let canceller = client.canceller();
    let outcome = Arc::new(Mutex::new(None));
    let seen = Arc::clone(&outcome);
    let reader = std::thread::spawn(move || {
        let messages = messages(&"x".repeat(4 * 1024 * 1024));
        let options = json(r#"{"sensitive_context": true}"#);
        let result = collect(
            &mut client,
            &chat(Some(&messages), None, "test", &options, &[]),
        );
        *seen.lock().unwrap() = Some(result.map(|_| ()).map_err(|e| e.code));
        client.has_process()
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !writing.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    std::thread::sleep(Duration::from_millis(200));
    let started = Instant::now();
    canceller.cancel();
    assert!(started.elapsed() < Duration::from_millis(500));
    let deadline = Instant::now() + Duration::from_secs(2);
    while !reader.is_finished() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        reader.is_finished(),
        "cancel did not stop the blocked writer"
    );
    assert!(!reader.join().unwrap(), "the process stayed registered");
    assert_eq!(
        outcome.lock().unwrap().take(),
        Some(Err("ai_cancelled".to_owned()))
    );
}

#[test]
fn sensitive_context_crosses_broker_and_cannot_retain_session() {
    let fake = Fake::standard();
    let mut client = fake.client("claude-cli://default");
    let messages = messages("selected fixture");
    let options = json(r#"{"sensitive_context": true, "reasoning_effort": "high"}"#);
    let chunks = collect(
        &mut client,
        &chat(Some(&messages), None, "sensitive", &options, &[]),
    )
    .unwrap();
    assert_eq!(
        chunks.last().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(client.last_provider_session_id, Value::Null);
}

#[test]
fn stateless_session_crosses_broker_with_tools_and_keeps_no_cursor() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    client.last_provider_session_id = s("earlier-thread");
    let messages = messages("incognito fixture");
    let options = json(r#"{"stateless_session": true, "reasoning_effort": "high"}"#);
    let tools = vec![json(
        r#"{"type": "function", "function": {"name": "ui_reports_tax_summary", "parameters": {}}}"#,
    )];
    let chunks = collect(
        &mut client,
        &chat(Some(&messages), None, "stateless", &options, &tools),
    )
    .unwrap();
    assert_eq!(
        chunks.last().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(client.last_provider_session_id, Value::Null);
}

#[test]
fn stateless_session_rejects_resume_and_nonboolean_before_spawn() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    for options in [
        r#"{"stateless_session": "true"}"#,
        r#"{"stateless_session": true, "provider_session_id": "old"}"#,
    ] {
        let options = json(options);
        let error = collect(&mut client, &chat(None, None, "default", &options, &[])).unwrap_err();
        assert_eq!(error.code, "ai_request_invalid");
    }
    assert_eq!(fake.launches(), 0);
}

#[test]
fn sensitive_mode_rejects_resume_tools_and_nonboolean_before_spawn() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let tool = [json(r#"{"name": "tool"}"#)];
    for (options, tools) in [
        (r#"{"sensitive_context": "true"}"#, &[][..]),
        (
            r#"{"sensitive_context": true, "provider_session_id": "old"}"#,
            &[][..],
        ),
        (r#"{"sensitive_context": true}"#, &tool[..]),
    ] {
        let options = json(options);
        assert!(collect(&mut client, &chat(None, None, "default", &options, tools)).is_err());
    }
    assert_eq!(fake.launches(), 0);
}

#[test]
fn sensitive_mode_rejects_hostile_broker_tool_event() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let options = json(r#"{"sensitive_context": true}"#);
    let error = collect(&mut client, &chat(None, None, "tool-call", &options, &[])).unwrap_err();
    assert_eq!(error.code, "ai_request_invalid");
    assert!(client.pending_call_ids().is_empty());
    assert!(!client.has_process());
}

#[test]
fn runtime_status_and_model_discovery_translate_jsonl() {
    let fake = Fake::standard();
    let rows = runtime_status(&fake.gate, &fake.locate(), Scope::All).unwrap();
    let Value::List(rows) = rows else {
        panic!("a list")
    };
    assert_eq!(*get(&rows[0], "state"), s("ready"));
    let client = fake.client("codex-cli://default");
    assert_eq!(
        client.list_models().unwrap(),
        vec![json(r#"{"id": "model-a"}"#)]
    );
    assert_eq!(fake.launches(), 2);
    assert_eq!(
        fake.requests(),
        vec![
            json(r#"{"command": "status"}"#),
            json(r#"{"command": "models", "provider": "codex"}"#)
        ]
    );
}

#[test]
fn runtime_status_scopes_and_an_empty_list_starts_nothing() {
    let fake = Fake::standard();
    let locate = fake.locate();
    assert_eq!(
        runtime_status(&fake.gate, &locate, Scope::Many(&[])).unwrap(),
        Value::List(vec![])
    );
    assert_eq!(fake.launches(), 0);
    let error = runtime_status(&fake.gate, &locate, Scope::Many(&["gemini"])).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        ("validation", "Unsupported CLI AI provider 'gemini'")
    );
    assert_eq!(fake.launches(), 0);
    // The broker's echo is not trusted: only the scope asked for comes back.
    assert_eq!(
        runtime_status(&fake.gate, &locate, Scope::Many(&["claude", "claude"])).unwrap(),
        Value::List(vec![])
    );
    assert_eq!(
        runtime_status(&fake.gate, &locate, Scope::One("codex")).unwrap(),
        json(r#"[{"provider": "codex", "state": "ready", "models": []}]"#)
    );
    assert_eq!(
        fake.requests()[0],
        json(r#"{"command": "status", "providers": ["claude"]}"#)
    );
    assert_eq!(
        fake.requests()[1],
        json(r#"{"command": "status", "provider": "codex"}"#)
    );
}

fn hanging_probe(stubborn: bool) {
    let scratch = Scratch::new("hang");
    let pid_file = scratch.path().join("child.pid");
    let root_file = scratch.path().join("root.txt");
    // The stubborn one also has stdio of its own, so only the group kill
    // can end it; the other holds the broker's stdout open.
    // Signals ignored before `exec` stay ignored in `sleep`.
    let child = if stubborn {
        "( trap '' TERM; exec sleep 60 ) </dev/null >/dev/null 2>&1 &"
    } else {
        "sleep 60 &"
    };
    let body = format!(
        "root=\"$KASSIBER_AI_BROKER_TMPDIR\"\n\
         printf '{{}}' > \"$root/session-log.jsonl\"\n\
         printf '%s' \"$root\" > '{root}'\n\
         {child}\n\
         printf '%s' \"$!\" > '{pid}'\n\
         sleep 60\n",
        root = root_file.to_str().unwrap(),
        pid = pid_file.to_str().unwrap(),
    );
    let fake = Fake::new(&body);
    let started = Instant::now();
    let error = single_result(
        &fake.gate,
        &fake.locate(),
        &json(r#"{"command": "status"}"#),
        Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(error.code, "ai_unavailable");
    assert!(started.elapsed() < Duration::from_secs(8));
    let child: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    // A survivor ends on its own within a minute.
    assert!(
        wait_until_gone(child),
        "the provider CLI outlived the timed-out probe"
    );
    // The per-turn state under the root is gone though the broker never
    // cleaned up.
    let root = std::fs::read_to_string(&root_file).unwrap();
    assert!(!Path::new(&root).exists());
}

#[test]
fn probe_timeout_stops_the_provider_cli_too() {
    hanging_probe(false);
}

#[test]
fn probe_timeout_stops_a_provider_that_ignores_sigterm() {
    hanging_probe(true);
}

#[test]
fn native_provider_locators_share_one_registry() {
    assert_eq!(
        cli_provider_for_locator(&s(" CODEX-CLI://DEFAULT ")),
        Some("codex")
    );
    assert_eq!(
        cli_provider_for_locator(&s("claude-cli://default")),
        Some("claude")
    );
    assert_eq!(
        cli_provider_for_locator(&s("opencode-cli://default")),
        Some("opencode")
    );
    assert_eq!(
        cli_provider_for_locator(&s("Copilot-CLI://default")),
        Some("copilot")
    );
    assert_eq!(cli_provider_for_locator(&s("gemini-cli://default")), None);
    assert_eq!(
        cli_provider_for_locator(&s("https://example.test/v1")),
        None
    );
    let gate = Gate::new(|| Offline::Online, Ledger::default());
    let error = BrokerClient::new("gemini-cli://default", Duration::from_secs(1), gate)
        .err()
        .unwrap();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "validation",
            "Unsupported CLI AI provider locator 'gemini-cli://default'"
        )
    );
}

#[test]
fn streaming_and_provider_session_continuation_metadata() {
    let fake = Fake::standard();
    let mut client = fake.client("opencode-cli://default");
    let messages = messages("hi");
    let options = json(r#"{"provider_session_id": "native-session-0"}"#);
    let chunks = collect(
        &mut client,
        &chat(Some(&messages), None, "provider/model", &options, &[]),
    )
    .unwrap();
    assert_eq!(content(&chunks), "hello world");
    assert_eq!(client.last_provider_session_id, s("native-session-1"));
    assert_eq!(
        chunks.last().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(
        fake.requests(),
        vec![json(
            r#"{"command": "chat", "request_id": "daemon-chat", "provider": "opencode", "model": "provider/model", "messages": [{"role": "user", "content": "hi"}], "instructions": null, "tools": [], "options": {"provider_session_id": "native-session-0"}}"#
        )]
    );
}

#[test]
fn kassiber_typed_tools_cross_the_chat_only_broker() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let tools = vec![json(
        r#"{"type": "function", "name": "ui_reports_tax_summary", "description": "Read the tax summary", "parameters": {"type": "object", "properties": {}}}"#,
    )];
    let first = RequestContext {
        instructions: Some("You are Kassiber's assistant.".into()),
        input_items: vec![json(
            r#"{"type": "message", "role": "user", "content": "Generate my report"}"#,
        )],
    };
    let none = Value::Null;
    let chunks = collect(
        &mut client,
        &chat(None, Some(&first), "tool-call", &none, &tools),
    )
    .unwrap();
    let last = chunks.last().unwrap();
    assert_eq!(last.finish_reason.as_deref(), Some("tool_calls"));
    let call = match get(&last.delta, "tool_calls") {
        Value::List(calls) => calls[0].clone(),
        other => panic!("{other:?}"),
    };
    assert_eq!(
        *get(get(&call, "function"), "name"),
        s("ui_reports_tax_summary")
    );
    assert_eq!(*get(get(&call, "function"), "arguments"), s("{}"));
    assert!(
        client.has_process(),
        "the broker stays alive for the results"
    );
    let second = RequestContext {
        instructions: Some("You are Kassiber's assistant.".into()),
        input_items: vec![dict([
            ("type", s("function_call_output")),
            ("call_id", get(&call, "id").clone()),
            ("output", s(r#"{"ok":true}"#)),
        ])],
    };
    let continued = collect(
        &mut client,
        &chat(None, Some(&second), "tool-call", &none, &tools),
    )
    .unwrap();
    assert_eq!(content(&continued), "report ready");
    assert_eq!(
        continued.last().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    assert_eq!(fake.launches(), 1, "one process across the tool call");
    let requests = fake.requests();
    assert_eq!(
        *get(&requests[0], "tools"),
        json(
            r#"[{"name": "ui_reports_tax_summary", "description": "Read the tax summary", "parameters": {"type": "object", "properties": {}}, "read_only": true, "destructive": false}]"#
        )
    );
    assert_eq!(
        *get(&requests[0], "instructions"),
        s("You are Kassiber's assistant.")
    );
    assert_eq!(
        requests[1],
        json(
            r#"{"command": "tool_results", "results": [{"call_id": "native-call-1", "output": "{\"ok\":true}"}]}"#
        )
    );
}

#[test]
fn missing_tool_results_keep_the_broker_waiting() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let none = Value::Null;
    collect(&mut client, &chat(None, None, "tool-call", &none, &[])).unwrap();
    let empty = RequestContext::default();
    let error = collect(
        &mut client,
        &chat(None, Some(&empty), "tool-call", &none, &[]),
    )
    .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "ai_request_invalid",
            "Kassiber did not provide every pending CLI tool result"
        )
    );
    assert!(client.has_process());
    assert_eq!(client.pending_call_ids().len(), 1);
}

#[test]
fn cancel_terminates_active_broker_process_group() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let canceller = client.canceller();
    let finished = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&finished);
    let worker = std::thread::spawn(move || {
        let messages = messages("wait");
        let none = Value::Null;
        let result = collect(
            &mut client,
            &chat(Some(&messages), None, "wait", &none, &[]),
        );
        done.store(true, Ordering::SeqCst);
        result.map(|_| ()).map_err(|e| e.code)
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    while fake.requests().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    canceller.cancel();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !finished.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        finished.load(Ordering::SeqCst),
        "cancel did not stop the broker"
    );
    assert_eq!(worker.join().unwrap(), Err("ai_cancelled".to_owned()));
}

#[test]
fn a_silent_broker_is_killed_by_the_leg_watchdog() {
    let fake = Fake::new("IFS= read -r request\nsleep 30\n");
    let mut client = BrokerClient::new(
        "codex-cli://default",
        Duration::from_secs(1),
        fake.gate.clone(),
    )
    .unwrap()
    .with_locate(fake.locate());
    let none = Value::Null;
    let started = Instant::now();
    let error = collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str(), error.retryable),
        (
            "ai_unavailable",
            "The CLI provider stopped responding",
            true
        )
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn a_broker_that_exits_without_done_fails_closed() {
    let fake =
        Fake::new("IFS= read -r request\nemit '{\"type\":\"delta\",\"content\":\"half\"}'\n");
    let mut client = fake.client("codex-cli://default");
    let none = Value::Null;
    let mut chunks = Vec::new();
    let error = client
        .stream_chat(&chat(None, None, "m", &none, &[]), &mut |chunk| {
            chunks.push(chunk);
            Ok(())
        })
        .unwrap_err();
    assert_eq!(content(&chunks), "half");
    assert_eq!(
        error.message,
        "The CLI provider stopped before completing the response"
    );
}

#[test]
fn broker_errors_map_like_python() {
    let fake = Fake::new(
        r#"IFS= read -r request
emit "not json"
emit "[1, 2]"
case "$(field model "$request")" in
  auth) emit '{"type": "error", "code": "authentication_required", "hint": "Run codex login."}' ;;
  missing) emit '{"type": "error", "code": "missing_executable"}' ;;
  other) emit '{"type": "error", "message": "model not found"}' ;;
  bare) emit '{"type": "error"}' ;;
  *) exit 3 ;;
esac
"#,
    );
    let none = Value::Null;
    let cases = [
        (
            "auth",
            "ai_auth_failed",
            "The selected CLI provider requires authentication",
            Some("Run codex login."),
            false,
        ),
        (
            "missing",
            "ai_unavailable",
            "The selected CLI provider is not installed",
            Some("Install the provider CLI and authenticate it outside Kassiber."),
            true,
        ),
        (
            "other",
            "ai_request_invalid",
            "model not found",
            Some("Check the provider status in the model picker."),
            false,
        ),
        (
            "bare",
            "ai_request_invalid",
            "The CLI provider request failed.",
            Some("Check the provider status in the model picker."),
            false,
        ),
    ];
    for (model, code, message, hint, retryable) in cases {
        let mut client = fake.client("codex-cli://default");
        let error = collect(&mut client, &chat(None, None, model, &none, &[])).unwrap_err();
        assert_eq!(
            (
                error.code.as_str(),
                error.message.as_str(),
                error.hint.as_deref(),
                error.retryable
            ),
            (code, message, hint, retryable),
            "{model}"
        );
    }
    // A private request hides the provider's text.
    let mut client = fake.client("codex-cli://default");
    let options = json(r#"{"sensitive_context": true}"#);
    let error = collect(&mut client, &chat(None, None, "other", &options, &[])).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str()),
        (
            "ai_unavailable",
            "The CLI provider could not complete the private selected-context request"
        )
    );
}

#[test]
fn a_probe_without_a_result_or_with_broken_output_is_unavailable() {
    let silent = Fake::new("IFS= read -r request\nemit '{\"type\": \"other\"}'\n");
    let error = single_result(
        &silent.gate,
        &silent.locate(),
        &json(r#"{"command": "status"}"#),
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.message.as_str(), error.retryable),
        (
            "ai_unavailable",
            "The local AI provider broker returned no result",
            true
        )
    );
    let broken = Fake::new("IFS= read -r request\nprintf '\\377\\n'\n");
    let error = single_result(
        &broken.gate,
        &broken.locate(),
        &json(r#"{"command": "status"}"#),
        Duration::from_secs(10),
    )
    .unwrap_err();
    assert_eq!(
        error.message,
        "The local AI provider broker returned invalid output"
    );
}

#[test]
fn a_missing_broker_reports_unavailable_without_credentials() {
    let fake = Fake::standard();
    let missing: Locate = Arc::new(|| BrokerProgram::new(None, vec!["--provider-broker".into()]));
    let error = runtime_status(&fake.gate, &missing, Scope::All).unwrap_err();
    assert_eq!(error.code, "ai_unavailable");
    assert!(!error.message.to_lowercase().contains("token"));
    assert_eq!(fake.launches(), 0);
    let no_program: Locate =
        Arc::new(|| BrokerProgram::new(Some(PathBuf::from("/nonexistent/kassiber")), Vec::new()));
    let mut client = fake.client("codex-cli://default").with_locate(no_program);
    let none = Value::Null;
    let error = collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap_err();
    assert_eq!(
        (error.code.as_str(), error.hint.as_deref()),
        (
            "ai_unavailable",
            Some("Reinstall Kassiber, or use an HTTP/Ollama/oMLX provider.")
        )
    );
    assert_eq!(fake.launches(), 0);
}

// --- test_ai_fast_mode.py: BrokerFastModeTest, FastModeNoticeTest -----------

fn options_sent(options: &str) -> Value {
    let fake = Fake::new(
        "IFS= read -r request\nprintf '%s\\n' \"$request\" >> \"$LOG\"\nemit '{\"type\":\"delta\",\"content\":\"ok\"}'\nemit '{\"type\":\"done\",\"finish_reason\":\"stop\"}'\n",
    );
    let mut client = fake.client("codex-cli://default");
    let messages = messages("Reply with just: ok");
    let options = json(options);
    let chunks = collect(
        &mut client,
        &chat(Some(&messages), None, "gpt-5.4", &options, &[]),
    )
    .unwrap();
    assert_eq!(
        chunks.last().unwrap().finish_reason.as_deref(),
        Some("stop")
    );
    get(&fake.requests()[0], "options").clone()
}

#[test]
fn forwards_only_an_explicit_true() {
    assert_eq!(
        options_sent(r#"{"fast_mode": true, "reasoning_effort": "high"}"#),
        json(r#"{"reasoning_effort": "high", "fast_mode": true}"#)
    );
    assert_eq!(options_sent(r#"{"fast_mode": false}"#), json("{}"));
}

#[test]
fn rejects_non_boolean_fast_mode_before_spawn() {
    let fake = Fake::standard();
    let mut client = fake.client("claude-cli://default");
    for value in [r#""true""#, "1", "null"] {
        let options = json(&format!(r#"{{"fast_mode": {value}}}"#));
        let error = collect(&mut client, &chat(None, None, "opus", &options, &[])).unwrap_err();
        assert_eq!(error.code, "ai_request_invalid", "{value}");
    }
    assert_eq!(fake.launches(), 0);
}

#[test]
fn broker_client_forwards_only_known_notices() {
    let fake = Fake::new(
        r#"IFS= read -r request
emit '{"type":"status","phase":"connecting","message":"Starting Claude"}'
emit '{"type":"status","phase":"fast_mode_unavailable","message":"ignored text"}'
emit '{"type":"status","phase":"provider_says_anything","message":"<b>x</b>"}'
emit '{"type":"delta","content":"ok"}'
emit '{"type":"done","finish_reason":"stop"}'
"#,
    );
    let mut client = fake.client("claude-cli://default");
    let messages = messages("Reply with just: ok");
    let options = json(r#"{"fast_mode": true}"#);
    let chunks = collect(
        &mut client,
        &chat(Some(&messages), None, "opus", &options, &[]),
    )
    .unwrap();
    let notices: Vec<_> = chunks
        .iter()
        .filter_map(|c| c.status_phase.clone())
        .collect();
    assert_eq!(notices, ["fast_mode_unavailable"]);
    let notice = chunks.iter().find(|c| c.status_phase.is_some()).unwrap();
    assert_eq!(notice.delta, dict::<&str>([]));
    assert_eq!(content(&chunks), "ok");
}

// --- privacy fences --------------------------------------------------------

#[test]
fn offline_mode_refuses_every_broker_use_before_a_launch() {
    let fake = Fake::standard();
    let gate = Gate::new(|| Offline::Offline, Ledger::default())
        .with_test_observer(false)
        .with_broker_fixture(fake.program());
    let unavailable = Gate::new(|| Offline::Unavailable, Ledger::default())
        .with_test_observer(false)
        .with_broker_fixture(fake.program());
    for gate in [gate, unavailable] {
        let error = runtime_status(&gate, &fake.locate(), Scope::All).unwrap_err();
        assert_eq!(
            (error.code.as_str(), error.message.as_str()),
            (
                "network_egress_disabled",
                "Remote AI providers are disabled: Kassiber is in offline mode"
            )
        );
        let mut client =
            BrokerClient::new("codex-cli://default", Duration::from_secs(5), gate.clone())
                .unwrap()
                .with_locate(fake.locate());
        assert_eq!(
            client.list_models().unwrap_err().code,
            "network_egress_disabled"
        );
        let none = Value::Null;
        let error = collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap_err();
        assert_eq!(error.code, "network_egress_disabled");
        assert!(gate.attempts().records.is_empty());
    }
    assert!(fake.requests().is_empty());
}

#[test]
fn offline_mode_mid_tool_session_refuses_the_next_leg() {
    let fake = Fake::standard();
    let offline = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&offline);
    let gate = Gate::new(
        move || {
            if flag.load(Ordering::SeqCst) {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    )
    .with_test_observer(false)
    .with_broker_fixture(fake.program());
    let mut client = BrokerClient::new("codex-cli://default", Duration::from_secs(30), gate)
        .unwrap()
        .with_locate(fake.locate());
    let none = Value::Null;
    collect(&mut client, &chat(None, None, "tool-call", &none, &[])).unwrap();
    offline.store(true, Ordering::SeqCst);
    let results = RequestContext {
        instructions: None,
        input_items: vec![json(
            r#"{"type": "function_call_output", "call_id": "native-call-1", "output": "{}"}"#,
        )],
    };
    let error = collect(
        &mut client,
        &chat(None, Some(&results), "tool-call", &none, &[]),
    )
    .unwrap_err();
    assert_eq!(error.code, "network_egress_disabled");
    assert_eq!(fake.requests().len(), 1, "no tool results were sent");
}

#[test]
fn every_launch_is_seen_by_the_observer_and_a_strict_one_blocks_it() {
    let fake = Fake::standard();
    let none = Value::Null;
    for _ in 0..3 {
        let mut client = fake.client("codex-cli://default");
        collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap();
    }
    runtime_status(&fake.gate, &fake.locate(), Scope::One("codex")).unwrap();
    assert_eq!(fake.launches(), 4);
    assert_eq!(fake.requests().len(), 4);
    let records = fake.gate.attempts().records;
    assert!(records.iter().all(|r| r.scheme == "broker" && r.port == 1));
    // The product ledger records no launch, as Python records none.
    assert!(fake.gate.ledger().snapshot(0, 100).records.is_empty());

    // A strict observer, or a broker script nobody named, trips the guard
    // before anything starts.
    for gate in [
        Gate::new(|| Offline::Online, Ledger::default())
            .with_test_observer(true)
            .with_broker_fixture(fake.program()),
        Gate::new(|| Offline::Online, Ledger::default()).with_test_observer(false),
    ] {
        let locate = fake.locate();
        let tripped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = runtime_status(&gate, &locate, Scope::All);
        }));
        assert!(tripped.unwrap_err().is::<kassiber_net::TestEgressBlocked>());
        assert_eq!(gate.attempts().records.len(), 1);
    }
    assert_eq!(fake.requests().len(), 4);
}

#[test]
fn the_broker_gets_only_node_and_script_and_a_private_root() {
    let fake = Fake::new(
        r#"IFS= read -r request
root="$KASSIBER_AI_BROKER_TMPDIR"
# `ls -ld` is the portable way to read permission bits (no GNU/BSD stat split).
case "$(ls -ld "$root" | cut -c1-10)" in
  drwx------) mode=448 ;;
  *) mode=-1 ;;
esac
if [ -d "$root" ]; then dir=true; else dir=false; fi
name=$(basename "$root" | cut -c1-19)
if [ "$(ps -o pgid= -p $$ | tr -d ' ')" = "$$" ]; then leader=true; else leader=false; fi
printf '{"argv": [%s], "mode": %s, "dir": %s, "name": "%s", "group_leader": %s}\n' \
  "$(for arg in "$@"; do printf '"%s", ' "$arg"; done)" "$mode" "$dir" "$name" "$leader" >> "$LOG"
emit '{"type": "result", "data": []}'
"#,
    );
    single_result(
        &fake.gate,
        &fake.locate(),
        &json(r#"{"command": "status"}"#),
        Duration::from_secs(10),
    )
    .unwrap();
    let seen = &fake.requests()[0];
    assert_eq!(*get(seen, "argv"), Value::List(vec![]));
    assert_eq!(*get(seen, "mode"), Value::from(0o700_usize));
    assert_eq!(*get(seen, "dir"), Value::Bool(true));
    assert_eq!(*get(seen, "name"), s("kassiber-ai-broker-"));
    assert_eq!(*get(seen, "group_leader"), Value::Bool(true));
}

#[test]
fn a_cancelled_client_cannot_start_another_leg() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let none = Value::Null;
    collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap();
    client.canceller().cancel();
    assert!(client.canceller().is_cancelled());
    let error = collect(&mut client, &chat(None, None, "m", &none, &[])).unwrap_err();
    assert_eq!(error.code, "ai_cancelled");
    assert_eq!(fake.launches(), 1);
}

#[test]
fn an_emit_failure_ends_the_leg_and_the_process() {
    let fake = Fake::standard();
    let mut client = fake.client("codex-cli://default");
    let none = Value::Null;
    let error = client
        .stream_chat(&chat(None, None, "m", &none, &[]), &mut |_| {
            Err(AppError::new("ai_cancelled", "stop"))
        })
        .unwrap_err();
    assert_eq!(error.code, "ai_cancelled");
    assert!(!client.has_process());
}
