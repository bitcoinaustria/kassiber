//! The dispatcher (`main()` in `index.ts`) with fake adapters.

use super::*;
use crate::protocol::{provider_status, BrokerModel, ProviderState};
use serde_json::json;
use std::io::Write;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Behaves by the request's model (chat) or provider (status).
struct Fake;

static FAKE: Fake = Fake;

impl Adapter for Fake {
    fn status(&self, cwd: &Path) -> Result<ProviderStatus, BrokerFailure> {
        assert!(cwd.is_dir());
        // Slow enough that sequential probes would show in the timing test.
        std::thread::sleep(Duration::from_millis(300));
        assert_eq!(
            std::fs::read_dir(cwd).unwrap().count(),
            0,
            "a fresh directory"
        );
        let name = cwd.file_name().unwrap().to_str().unwrap();
        if name.starts_with("kassiber-ai-copilot-") {
            return Ok(provider_status(
                ProviderId::Copilot,
                "GitHub Copilot",
                ProviderState::Error,
                "Copilot broke.",
            ));
        }
        let provider = PROVIDER_IDS_BY_PREFIX
            .iter()
            .find(|(prefix, _)| name.starts_with(prefix))
            .map(|(_, id)| *id)
            .unwrap();
        Ok(
            provider_status(provider, "Fake", ProviderState::Ready, "Ready.").with_models(vec![
                BrokerModel {
                    id: format!("{provider}-model"),
                    ..BrokerModel::default()
                },
            ]),
        )
    }

    fn chat(
        &self,
        request: &ChatRequest,
        _cwd: &Path,
        bridge: Option<&ToolBridge>,
        events: &EventSink,
    ) -> Result<(), BrokerFailure> {
        match request.model.as_str() {
            "fail" => Err(BrokerFailure::new("login required at /home/someone")),
            "tool" => {
                let bridge = bridge.expect("tools advertised");
                let arguments =
                    serde_json::value::RawValue::from_string(r#"{"q": 1}"#.into()).unwrap();
                let output = bridge.request("status", &arguments, Some("call-1"))?;
                events.write(&Event::Delta {
                    content: Some(format!("tool said {}", output.as_str().unwrap())),
                    reasoning: None,
                });
                events.write(&Event::Done {
                    finish_reason: "stop".into(),
                    provider_session_id: Some("session-1".into()),
                });
                Ok(())
            }
            "pending" => {
                // Ends the turn with a call still waiting: closing the
                // bridge must answer it.
                let bridge = bridge.expect("tools advertised").socket_path().to_owned();
                let handle = std::thread::spawn(move || {
                    let mut stream = crate::bridge::connect(&bridge).unwrap();
                    stream
                        .write_all(b"{\"call_id\":\"p\",\"name\":\"status\",\"arguments\":{}}\n")
                        .unwrap();
                    crate::bridge::read_line_capped(&mut stream)
                });
                let deadline = Instant::now() + Duration::from_secs(10);
                while !PENDING_SEEN.lock().unwrap().is_some() && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                *PENDING_REPLY.lock().unwrap() = Some(handle);
                Err(BrokerFailure::new("provider stopped"))
            }
            _ => {
                assert!(bridge.is_none());
                events.write(&Event::Delta {
                    content: Some("hello".into()),
                    reasoning: None,
                });
                events.write(&Event::Done {
                    finish_reason: "stop".into(),
                    provider_session_id: None,
                });
                Ok(())
            }
        }
    }
}

type ReplyHandle = std::thread::JoinHandle<Option<Vec<u8>>>;
static PENDING_SEEN: Mutex<Option<()>> = Mutex::new(None);
static PENDING_REPLY: Mutex<Option<ReplyHandle>> = Mutex::new(None);

const PROVIDER_IDS_BY_PREFIX: [(&str, ProviderId); 4] = [
    ("kassiber-ai-codex-", ProviderId::Codex),
    ("kassiber-ai-claude-", ProviderId::Claude),
    ("kassiber-ai-opencode-", ProviderId::OpenCode),
    ("kassiber-ai-copilot-", ProviderId::Copilot),
];

fn fake(_: ProviderId) -> &'static dyn Adapter {
    &FAKE
}

fn run(input: &str) -> (Result<(), BrokerFailure>, Vec<serde_json::Value>) {
    let (events, captured) = EventSink::capture();
    let result = dispatch(input_from(input), &events, &fake);
    (result, captured.events())
}

#[test]
fn status_probes_the_scope_in_parallel_in_fresh_directories() {
    let started = Instant::now();
    let (result, events) =
        run("{\"command\":\"status\",\"providers\":[\"codex\",\"claude\",\"opencode\"]}\n");
    result.unwrap();
    // Three 300 ms probes, run together.
    assert!(
        started.elapsed() < Duration::from_millis(850),
        "{:?}",
        started.elapsed()
    );
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["type"], "result");
    let providers: Vec<_> = events[0]["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["provider"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(providers, ["codex", "claude", "opencode"]);
    assert_eq!(events[0]["data"][0]["models"][0]["id"], "codex-model");
}

#[test]
fn a_malformed_scope_starts_nothing() {
    let (result, events) = run("{\"command\":\"status\",\"providers\":[\"codex\",\"gemini\"]}");
    assert_eq!(result.unwrap_err().message(), "Unknown provider.");
    assert!(events.is_empty());
}

#[test]
fn models_answers_from_the_status_snapshot_or_fails_with_its_message() {
    let (result, events) = run("{\"command\":\"models\",\"provider\":\"claude\"}");
    result.unwrap();
    assert_eq!(
        events,
        [json!({"type": "result", "data": [{"id": "claude-model"}]})]
    );
    let (result, events) = run("{\"command\":\"models\",\"provider\":\"copilot\"}");
    assert_eq!(result.unwrap_err().message(), "Copilot broke.");
    assert!(events.is_empty());
    let (result, _) = run("{\"command\":\"models\",\"provider\":\"gemini\"}");
    assert_eq!(result.unwrap_err().message(), "Unknown provider.");
}

#[test]
fn a_chat_without_tools_starts_no_bridge() {
    let (result, events) = run(
        "{\"command\":\"chat\",\"provider\":\"codex\",\"model\":\"plain\",\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}\r\n",
    );
    result.unwrap();
    assert_eq!(
        events,
        [
            json!({"type": "delta", "content": "hello"}),
            json!({"type": "done", "finish_reason": "stop"})
        ]
    );
}

#[test]
fn a_chat_with_tools_answers_calls_from_later_stdin_lines() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    writer
        .write_all(
            b"{\"command\":\"chat\",\"provider\":\"claude\",\"model\":\"tool\",\"messages\":[],\"tools\":[{\"name\":\"status\",\"description\":\"Status\",\"parameters\":{\"type\":\"object\"}}]}\n",
        )
        .unwrap();
    let (events, captured) = EventSink::capture();
    let turn = std::thread::spawn(move || dispatch(input_reader(reader), &events, &fake));
    let deadline = Instant::now() + Duration::from_secs(10);
    while captured.events().is_empty() {
        assert!(Instant::now() < deadline, "no tool_call");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(
        captured.text(),
        "{\"type\":\"tool_call\",\"call_id\":\"call-1\",\"name\":\"status\",\"arguments\":{\"q\":1}}\n"
    );
    // Lines that are not tool results are skipped.
    writer
        .write_all(b"not json\n{\"command\":\"other\"}\n")
        .unwrap();
    writer
        .write_all(b"{\"command\": \"tool_results\", \"results\": [{\"call_id\": \"call-1\", \"output\": \"ok\"}]}\n")
        .unwrap();
    turn.join().unwrap().unwrap();
    let events = captured.events();
    assert_eq!(
        events[1],
        json!({"type": "delta", "content": "tool said ok"})
    );
    assert_eq!(
        events[2],
        json!({"type": "done", "finish_reason": "stop", "provider_session_id": "session-1"})
    );
}

#[test]
fn a_failed_turn_closes_the_bridge_and_answers_its_pending_calls() {
    let (reader, mut writer) = std::io::pipe().unwrap();
    writer
        .write_all(
            b"{\"command\":\"chat\",\"provider\":\"codex\",\"model\":\"pending\",\"messages\":[],\"tools\":[{\"name\":\"status\",\"parameters\":{}}]}\n",
        )
        .unwrap();
    let (events, captured) = EventSink::capture();
    let turn = std::thread::spawn(move || dispatch(input_reader(reader), &events, &fake));
    let deadline = Instant::now() + Duration::from_secs(10);
    while captured.events().is_empty() {
        assert!(Instant::now() < deadline, "no tool_call");
        std::thread::sleep(Duration::from_millis(5));
    }
    *PENDING_SEEN.lock().unwrap() = Some(());
    assert_eq!(
        turn.join().unwrap().unwrap_err().message(),
        "provider stopped"
    );
    let reply = PENDING_REPLY
        .lock()
        .unwrap()
        .take()
        .unwrap()
        .join()
        .unwrap()
        .unwrap();
    assert_eq!(
        String::from_utf8(reply).unwrap(),
        r#"{"error":"Error: Kassiber closed the tool bridge."}"#
    );
    drop(writer);
}

#[test]
fn failures_end_the_request_with_their_raw_message() {
    let (result, events) =
        run("{\"command\":\"chat\",\"provider\":\"opencode\",\"model\":\"fail\",\"messages\":[]}");
    let failure = result.unwrap_err();
    assert_eq!(failure.message(), "login required at /home/someone");
    assert!(events.is_empty());
    // `serve` turns it into the redacted event.
    assert_eq!(
        serde_json::to_value(error_event(&failure)).unwrap(),
        json!({
            "type": "error",
            "code": "authentication_required",
            "message": "The provider requires authentication.",
            "hint": "Run the provider's normal login command outside Kassiber."
        })
    );
}

#[test]
fn malformed_requests_are_refused() {
    for (input, message) in [
        ("", "Kassiber provider broker received no request."),
        ("not json\n", "Kassiber provider broker received an invalid request."),
        ("[]\n", "Kassiber provider broker received an invalid request."),
        ("{\"command\":\"tool_results\"}\n", "Kassiber provider broker received an unknown command."),
        ("{\"provider\":\"codex\"}\n", "Kassiber provider broker received an unknown command."),
        ("{\"command\":\"chat\",\"provider\":\"gemini\",\"model\":\"m\",\"messages\":[]}", "Unknown provider."),
        ("{\"command\":\"chat\",\"provider\":\"codex\",\"messages\":[]}", "Kassiber provider broker received an invalid chat request."),
        (
            "{\"command\":\"chat\",\"provider\":\"codex\",\"model\":\"m\",\"messages\":[{\"role\":\"user\",\"content\":5}]}",
            "Kassiber provider broker received an invalid chat request.",
        ),
    ] {
        let (result, events) = run(input);
        assert_eq!(result.unwrap_err().message(), message, "{input}");
        assert!(events.is_empty());
    }
}

#[test]
fn every_provider_has_an_adapter() {
    for provider in crate::protocol::PROVIDER_IDS {
        let _ = adapter(provider);
    }
}
