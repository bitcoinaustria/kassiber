//! Whole requests through the dispatcher against a fake OpenCode 1 CLI and
//! its loopback API: status probes, plain, resumed, selected-data and
//! Incognito turns, a tool call through the bridge, and failures.

use std::io::Write;
use std::time::Instant;

use super::v2_cases::{fixture, Api, Behaviour};
use super::*;

type OnPrompt = dyn Fn(&Feed, &HandledRequest) -> Reply + Send + Sync;

/// The OpenCode 1 API: `config` as the effective configuration, sessions
/// `ses_1` (new) and `ses_prev` (resumable), and `on_prompt` for the prompt.
fn v1_api(config: Value, on_prompt: Arc<OnPrompt>) -> (FakeService, Feed) {
    let feed = Feed::default();
    let events = feed.clone();
    let service =
        FakeService::start(
            move |request| match (request.method.as_str(), path_of(request)) {
                ("GET", "/config") => Reply::json(200, &config.to_string()),
                ("POST", "/mcp") => Reply::json(200, "{\"kassiber\":{\"status\":\"connected\"}}"),
                ("GET", "/session/ses_prev") => Reply::json(200, "{\"id\":\"ses_prev\"}"),
                ("PATCH", "/session/ses_prev") => Reply::json(200, "{\"id\":\"ses_prev\"}"),
                ("POST", "/session") => Reply::json(200, "{\"id\":\"ses_1\",\"title\":\"x\"}"),
                ("GET", "/event") => {
                    let reply = events.open();
                    events.emit(json!({"type": "server.connected", "properties": {}}));
                    reply
                }
                ("POST", path) if path.ends_with("/prompt_async") => on_prompt(&events, request),
                _ => Reply::json(
                    404,
                    "{\"name\":\"NotFoundError\",\"data\":{\"message\":\"Not found\"}}",
                ),
            },
        )
        .unwrap();
    (service, feed)
}

/// An answer streamed the way OpenCode 1 streams it: whole-part updates.
fn answer(feed: &Feed, session: &str, text: &str) {
    feed.emit(json!({"type": "message.updated", "properties": {"info": {"id": "msg_user", "role": "user", "sessionID": session}}}));
    feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "prt_u", "messageID": "msg_user", "sessionID": session, "type": "text", "text": "echoed user text"}}}));
    feed.emit(json!({"type": "message.updated", "properties": {"info": {"id": "msg_1", "role": "assistant", "sessionID": session}}}));
    feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "prt_r", "messageID": "msg_1", "sessionID": session, "type": "reasoning", "text": "Thinking"}}}));
    let half = text.len() / 2;
    feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "prt_1", "messageID": "msg_1", "sessionID": session, "type": "text", "text": &text[..half]}}}));
    feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "prt_1", "messageID": "msg_1", "sessionID": session, "type": "text", "text": text}}}));
    feed.emit(json!({"type": "session.idle", "properties": {"sessionID": "ses_other"}}));
    feed.emit(json!({"type": "session.idle", "properties": {"sessionID": session}}));
}

fn answering(text: &'static str) -> Arc<OnPrompt> {
    Arc::new(move |feed, request| {
        let session = path_of(request)
            .split('/')
            .nth(2)
            .unwrap_or_default()
            .to_owned();
        answer(feed, &session, text);
        Reply::raw(204, "application/json", b"")
    })
}

/// A fake OpenCode 1 whose `serve` reports the given service's port.
fn v1_fake(version: &str, service: &FakeService) -> (Fake, Host) {
    let fake = Fake::new(version);
    fake.write("serve-mode", "v1");
    fake.write("v1-port", &service.port().to_string());
    let mut host = fake.host();
    let port = service.port();
    host.free_port = Arc::new(move || Ok(port));
    (fake, host)
}

fn chat_line(model: &str, extra: Value) -> String {
    let mut request = json!({
        "command": "chat", "request_id": "fixture", "provider": "opencode", "model": model,
        "messages": [
            {"role": "user", "content": "first question"},
            {"role": "assistant", "content": "first answer"},
            {"role": "user", "content": "Synthetic user message"},
        ],
    });
    for (key, value) in extra.as_object().unwrap() {
        request[key] = value.clone();
    }
    format!("{request}\n")
}

fn kinds(events: &[Value]) -> Vec<&str> {
    events
        .iter()
        .map(|event| event["type"].as_str().unwrap())
        .collect()
}

#[test]
fn status_reports_ready_authentication_required_and_missing() {
    let fake = Fake::new("1.18.10");
    fake.write(
        "debug",
        r#"{"provider":{"omlx":{"options":{"baseURL":" http://127.0.0.1:8000/v1 ","apiKey":"sk-config-secret"}}}}"#,
    );
    fake.write("models", "omlx/qwen\n{\"name\":\"Qwen\"}\n");
    let line = "{\"command\":\"status\",\"provider\":\"opencode\"}\n";
    let (result, events) = dispatch_line(fake.host(), line);
    result.unwrap();
    assert_eq!(
        events,
        [json!({"type": "result", "data": [{
            "provider": "opencode", "display_name": "OpenCode", "privacy_posture": "remote",
            "native_tools": "disabled",
            "models": [{
                "id": "omlx/qwen", "display_name": "Qwen", "owned_by": "OpenCode · omlx",
                "source_provider": "omlx", "privacy_posture": "local",
                "privacy_reason": "OpenCode routes omlx to a loopback endpoint on this machine.",
                "supports_reasoning_effort": false, "reasoning_efforts": [],
            }],
            "state": "ready", "message": "Ready using the existing OpenCode configuration.",
            "version": "1.18.10", "executable": "opencode",
        }]})]
    );
    assert!(!events[0].to_string().contains("sk-config-secret"));

    fake.write("models", "");
    let (_, events) = dispatch_line(fake.host(), line);
    let status = &events[0]["data"][0];
    assert_eq!(status["state"], "authentication_required");
    assert_eq!(
        status["message"],
        "Run `opencode auth login` outside Kassiber."
    );

    let empty = tempfile::tempdir().unwrap();
    let mut host = fake.host();
    host.vars = vec![("PATH".into(), empty.path().into())];
    let (_, events) = dispatch_line(host, line);
    assert_eq!(
        events[0]["data"][0],
        json!({
            "provider": "opencode", "display_name": "OpenCode", "privacy_posture": "remote",
            "native_tools": "disabled", "models": [], "state": "missing_executable",
            "message": "Install OpenCode, then run `opencode auth login` outside Kassiber.",
        })
    );
}

#[test]
fn models_answers_from_the_probe_or_fails_with_its_message() {
    let fake = Fake::new("1.18.10");
    let line = "{\"command\":\"models\",\"provider\":\"opencode\"}\n";
    let (result, events) = dispatch_line(fake.host(), line);
    result.unwrap();
    assert_eq!(events[0]["data"][0]["id"], "provider/model");
    fake.write("version", "opencode v3.1.0");
    let (result, events) = dispatch_line(fake.host(), line);
    assert!(result.is_err());
    assert_eq!(
        events,
        [
            json!({"type": "error", "code": "provider_error", "message": "This OpenCode major version is not supported by this build."})
        ]
    );
}

#[test]
fn a_plain_turn_streams_deltas_and_returns_its_session_cursor() {
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("Hello there"));
    let (fake, host) = v1_fake("1.18.10", &service);
    let line = chat_line(
        "provider/model",
        json!({"options": {"reasoning_effort": "high"}}),
    );
    let (result, events) = dispatch_line(host, &line);
    result.unwrap();
    assert_eq!(
        events,
        [
            json!({"type": "status", "phase": "connecting", "message": "Starting OpenCode server"}),
            json!({"type": "delta", "reasoning": "Thinking"}),
            json!({"type": "delta", "content": "Hello"}),
            json!({"type": "delta", "content": " there"}),
            json!({"type": "done", "finish_reason": "stop", "provider_session_id": "ses_1"}),
        ]
    );
    let requests = service.requests();
    let paths: Vec<_> = requests
        .iter()
        .map(|request| format!("{} {}", request.method, path_of(request)))
        .collect();
    assert_eq!(
        paths,
        [
            "GET /config",
            "POST /session",
            "GET /event",
            "POST /session/ses_1/prompt_async"
        ]
    );
    // Reads carry the directory as a query parameter, writes as a header.
    assert!(requests[0].target.starts_with("/config?directory=%2F"));
    assert!(requests[1]
        .header("x-opencode-directory")
        .unwrap()
        .starts_with("%2F"));
    assert_eq!(
        body(&requests[1]),
        json!({"permission": [{"permission": "*", "pattern": "*", "action": "deny"}]})
    );
    let prompt = body(&requests[3]);
    assert_eq!(
        prompt["model"],
        json!({"providerID": "provider", "modelID": "model"})
    );
    assert_eq!(prompt["variant"], "high");
    assert!(prompt.get("agent").is_none());
    assert!(prompt["system"]
        .as_str()
        .unwrap()
        .starts_with("You are the chat model inside Kassiber."));
    let text = prompt["parts"][0]["text"].as_str().unwrap();
    assert!(text.contains("first question") && text.contains("USER:\nSynthetic user message"));
    // The server got sharing off and the deny-all argv.
    let serve = &fake.serves()[0];
    assert_eq!(serve.argv[..2], ["serve", "--pure"]);
    assert_eq!(serve.argv[3], format!("--port={}", service.port()));
    assert_eq!(serve.vars["OPENCODE_DISABLE_SHARE"], "1");
}

#[test]
fn a_resumed_turn_reuses_the_session_and_sends_only_the_latest_message() {
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("Again"));
    let (_fake, host) = v1_fake("1.18.10", &service);
    let line = chat_line(
        "provider/model",
        json!({"options": {"provider_session_id": "ses_prev"}, "tools": null}),
    );
    let (result, events) = dispatch_line(host, &line);
    result.unwrap();
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "done", "finish_reason": "stop", "provider_session_id": "ses_prev"})
    );
    let requests = service.requests();
    let patch = requests
        .iter()
        .find(|request| request.method == "PATCH")
        .unwrap();
    assert_eq!(
        body(patch),
        json!({"permission": [{"permission": "*", "pattern": "*", "action": "deny"}]})
    );
    let prompt = requests
        .iter()
        .find(|request| path_of(request).ends_with("/prompt_async"))
        .unwrap();
    assert_eq!(path_of(prompt), "/session/ses_prev/prompt_async");
    assert_eq!(
        body(prompt)["parts"][0]["text"],
        "USER:\nSynthetic user message"
    );
    // A pruned session starts afresh.
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("Fresh"));
    let (_fake, host) = v1_fake("1.18.10", &service);
    let line = chat_line(
        "provider/model",
        json!({"options": {"provider_session_id": "ses_gone"}}),
    );
    let (result, events) = dispatch_line(host, &line);
    result.unwrap();
    assert_eq!(events.last().unwrap()["provider_session_id"], "ses_1");
}

#[test]
fn a_provider_native_tool_stops_the_turn() {
    let on_prompt: Arc<OnPrompt> = Arc::new(|feed, _| {
        feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "p", "messageID": "m", "sessionID": "ses_1", "type": "tool", "tool": "bash"}}}));
        Reply::raw(204, "application/json", b"")
    });
    let (service, _feed) = v1_api(json!({"share": "disabled"}), on_prompt);
    let (_fake, host) = v1_fake("1.18.10", &service);
    let (result, events) = dispatch_line(host, &chat_line("provider/model", json!({})));
    assert!(result.is_err());
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "error", "code": "provider_error", "message": "OpenCode attempted to use a provider-native tool; Kassiber stopped it."})
    );
}

#[test]
fn a_provider_failure_reaches_the_event_without_its_credential() {
    let on_prompt: Arc<OnPrompt> = Arc::new(|_, _| {
        Reply::json(
            500,
            r#"{"name":"ProviderError","data":{"message":"Upstream rejected token=sk-live-abcdefghijklmnop for /home/someone/x"}}"#,
        )
    });
    let (service, _feed) = v1_api(json!({"share": "disabled"}), on_prompt);
    let (_fake, host) = v1_fake("1.18.10", &service);
    let (result, events) = dispatch_line(host, &chat_line("provider/model", json!({})));
    assert!(result
        .unwrap_err()
        .message()
        .contains("sk-live-abcdefghijklmnop"));
    let error = events.last().unwrap();
    assert_eq!(error["type"], "error");
    let message = error["message"].as_str().unwrap();
    assert!(message.starts_with("Upstream rejected"), "{message}");
    assert!(!message.contains("sk-live-abcdefghijklmnop"), "{message}");
    assert!(!message.contains("someone"), "{message}");
}

#[test]
fn an_empty_error_body_reads_as_the_sdk_describes_it() {
    let on_prompt: Arc<OnPrompt> = Arc::new(|_, _| Reply::raw(502, "application/json", b""));
    let (service, _feed) = v1_api(json!({"share": "disabled"}), on_prompt);
    let (fake, host) = v1_fake("1.18.10", &service);
    let request = chat_request("provider/model", json!({}));
    let (events, _) = EventSink::capture();
    let cwd = fake.turn();
    let failure = chat(&host, &request, &cwd, None, &events).unwrap_err();
    assert_eq!(
        failure.message(),
        format!(
            "opencode server POST http://127.0.0.1:{}/session/ses_1/prompt_async → 502 Scripted: (empty response body)",
            service.port()
        )
    );
}

#[test]
fn a_selected_data_turn_is_locked_down_and_says_nothing_of_a_failure() {
    let model = "local/selected-model";
    let config = Js::Obj(crate::opencode::v1::sensitive_config(model).unwrap());
    let config: Value = serde_json::from_str(&stringify(&config)).unwrap();
    let (service, _feed) = v1_api(config.clone(), answering("Private answer"));
    let (fake, host) = v1_fake("1.18.4", &service);
    let line = chat_line(model, json!({"options": {"sensitive_context": true}}));
    let (result, events) = dispatch_line(host.clone(), &line);
    result.unwrap();
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "done", "finish_reason": "stop"})
    );
    let requests = service.requests();
    let create = requests.iter().find(|r| path_of(r) == "/session").unwrap();
    assert_eq!(body(create)["title"], "Kassiber selected context");
    let prompt = requests
        .iter()
        .find(|r| path_of(r).ends_with("/prompt_async"))
        .unwrap();
    assert_eq!(body(prompt)["agent"], "kassiber_selected_context");
    // Probes, then the server, all in the in-memory store; the share
    // switch the lockdown sets is dropped again before the server starts.
    let runs = fake.runs();
    let argvs: Vec<_> = runs.iter().map(|run| run.argv.join(" ")).collect();
    assert_eq!(argvs[..3], ["--version", "--version", "db path"]);
    assert!(argvs[3].starts_with("serve --pure"));
    let serve = &runs[3];
    assert_eq!(serve.vars["OPENCODE_DB"], ":memory:");
    assert_eq!(serve.vars.get("OPENCODE_AUTO_SHARE"), None);
    assert_eq!(serve.vars["OPENCODE_PURE"], "1");
    let inline = parse(&serve.vars["OPENCODE_CONFIG_CONTENT"]).unwrap();
    assert_eq!(inline.get("share"), Some(&Js::str("disabled")));
    assert_eq!(inline.get("autoshare"), Some(&Js::Bool(false)));
    assert_eq!(inline.get("small_model"), Some(&Js::str(model)));

    // A failure says nothing of the provider's diagnostic.
    let failing: Arc<OnPrompt> =
        Arc::new(|_, _| Reply::json(500, r#"{"data":{"message":"private diagnostic"}}"#));
    let (service, _feed) = v1_api(config, failing);
    let (_fake, host) = v1_fake("1.18.4", &service);
    let (result, events) = dispatch_line(host, &line);
    assert!(result.is_err());
    assert_eq!(
        events.last().unwrap()["message"],
        "OpenCode sensitive-context request failed; no provider diagnostic content was retained."
    );
    // Selected data never takes tools or a cursor.
    let line = chat_line(
        model,
        json!({"options": {"sensitive_context": true, "provider_session_id": "ses_prev"}}),
    );
    let (_, events) = dispatch_line(fake.host(), &line);
    assert_eq!(
        events.last().unwrap()["message"],
        "Sensitive context cannot use tools or resume a provider session."
    );
}

#[test]
fn a_selected_data_turn_refuses_a_drifted_effective_configuration() {
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("never"));
    let (_fake, host) = v1_fake("1.18.4", &service);
    let line = chat_line("local/m", json!({"options": {"sensitive_context": true}}));
    let (result, events) = dispatch_line(host, &line);
    assert!(result.is_err());
    assert_eq!(service.paths().len(), 1, "only the config was read");
    assert_eq!(
        events.last().unwrap()["message"],
        "OpenCode sensitive-context request failed; no provider diagnostic content was retained."
    );
}

#[test]
fn an_incognito_turn_keeps_tools_but_returns_no_cursor() {
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("Incognito"));
    let (fake, host) = v1_fake("1.18.10", &service);
    let line = chat_line(
        "provider/model",
        json!({"options": {"stateless_session": true}}),
    );
    let (result, events) = dispatch_line(host, &line);
    result.unwrap();
    assert_eq!(
        events.last().unwrap(),
        &json!({"type": "done", "finish_reason": "stop"})
    );
    let serve = fake.serves().pop().unwrap();
    assert_eq!(serve.vars["OPENCODE_DB"], ":memory:");
    assert!(serve.vars["XDG_DATA_HOME"].ends_with("private-runtime/data"));
}

#[test]
fn images_reach_opencode_as_inline_data_urls() {
    let (service, _feed) = v1_api(json!({"share": "disabled"}), answering("Seen"));
    let (_fake, host) = v1_fake("1.18.10", &service);
    let mut request: Value = serde_json::from_str(&chat_line("provider/model", json!({}))).unwrap();
    request["messages"][2]["images"] =
        json!([{"media_type": "image/jpeg", "data": "iVBORw0KGgo="}]);
    let (result, _) = dispatch_line(host, &format!("{request}\n"));
    result.unwrap();
    let prompt = service
        .requests()
        .into_iter()
        .find(|r| path_of(r).ends_with("/prompt_async"))
        .unwrap();
    assert_eq!(
        body(&prompt)["parts"][1],
        json!({"type": "file", "mime": "image/jpeg", "filename": "image-1.jpeg", "url": "data:image/jpeg;base64,iVBORw0KGgo="})
    );
}

#[test]
fn a_tool_call_goes_through_the_bridge_and_its_result_back_to_opencode() {
    let commands: Arc<Mutex<Vec<Value>>> = Arc::default();
    let seen = commands.clone();
    let feed = Feed::default();
    let events_feed = feed.clone();
    let service = FakeService::start(move |request| {
        match (request.method.as_str(), path_of(request)) {
            ("GET", "/config") => Reply::json(200, "{\"share\":\"disabled\"}"),
            ("POST", "/mcp") => {
                seen.lock().unwrap().push(body(request));
                Reply::json(200, "{}")
            }
            ("POST", "/session") => Reply::json(200, "{\"id\":\"ses_1\"}"),
            ("GET", "/event") => events_feed.open(),
            ("POST", "/session/ses_1/prompt_async") => {
                // OpenCode starts the MCP child, which forwards the call.
                let socket = seen.lock().unwrap()[0]["config"]["command"][2]
                    .as_str()
                    .unwrap()
                    .to_owned();
                let feed = events_feed.clone();
                std::thread::spawn(move || {
                    let mut stream = crate::bridge::connect(&socket).unwrap();
                    stream
                        .write_all(b"{\"call_id\":\"call-1\",\"name\":\"status\",\"arguments\":{\"q\": 1}}\n")
                        .unwrap();
                    let reply = crate::bridge::read_line_capped(&mut stream).unwrap();
                    let reply: Value = serde_json::from_slice(&reply).unwrap();
                    feed.emit(json!({"type": "message.part.updated", "properties": {"part": {"id": "t", "messageID": "msg_1", "sessionID": "ses_1", "type": "tool", "tool": "kassiber_status"}}}));
                    answer(&feed, "ses_1", &format!("tool said {}", reply["output"].as_str().unwrap()));
                });
                Reply::raw(204, "application/json", b"")
            }
            _ => Reply::json(404, "{}"),
        }
    })
    .unwrap();
    let (_fake, host) = v1_fake("1.18.10", &service);
    let (reader, mut writer) = std::io::pipe().unwrap();
    let line = chat_line(
        "provider/model",
        json!({"tools": [{"name": "status", "description": "Status", "parameters": {"type": "object"}}]}),
    );
    writer.write_all(line.as_bytes()).unwrap();
    let (events, captured) = EventSink::capture();
    let adapters = adapters(host);
    let turn = std::thread::spawn(move || {
        crate::dispatch(crate::input_reader(reader), &events, &adapters)
    });
    let deadline = Instant::now() + Duration::from_secs(20);
    while !captured
        .events()
        .iter()
        .any(|event| event["type"] == "tool_call")
    {
        assert!(
            Instant::now() < deadline,
            "no tool_call: {}",
            captured.text()
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    writer
        .write_all(b"{\"command\": \"tool_results\", \"results\": [{\"call_id\": \"call-1\", \"output\": \"ok\"}]}\n")
        .unwrap();
    turn.join().unwrap().unwrap();
    let events = captured.events();
    assert_eq!(
        kinds(&events),
        ["status", "tool_call", "delta", "delta", "delta", "done"]
    );
    assert_eq!(
        events[1],
        json!({"type": "tool_call", "call_id": "call-1", "name": "status", "arguments": {"q": 1}})
    );
    assert_eq!(events[3], json!({"type": "delta", "content": "tool s"}));
    assert_eq!(events[4], json!({"type": "delta", "content": "aid ok"}));
    let command = &commands.lock().unwrap()[0];
    assert_eq!(command["name"], "kassiber");
    assert_eq!(command["config"]["type"], "local");
    assert_eq!(command["config"]["enabled"], true);
    assert_eq!(command["config"]["command"][1], "--provider-broker-mcp");
    let session = service
        .requests()
        .into_iter()
        .find(|r| path_of(r) == "/session")
        .unwrap();
    assert_eq!(
        body(&session)["permission"][1],
        json!({"permission": "kassiber_status", "pattern": "*", "action": "allow"})
    );
}

#[test]
fn an_opencode_2_turn_runs_through_the_dispatcher() {
    let (fake, host) = fixture(json!({}));
    let _api = Api::start(&fake, 1, Behaviour::default());
    let (result, events) = dispatch_line(host, &chat_line("fixture/namespaced/model", json!({})));
    result.unwrap();
    assert_eq!(
        events,
        [
            json!({"type": "status", "phase": "connecting", "message": "Starting OpenCode server"}),
            json!({"type": "delta", "reasoning": "Reasoning fixture"}),
            json!({"type": "delta", "content": "Answer fixture"}),
            json!({"type": "done", "finish_reason": "stop"}),
        ]
    );
}

#[test]
fn an_event_stream_that_ends_early_fails_the_turn() {
    let on_prompt: Arc<OnPrompt> = Arc::new(|feed, _| {
        feed.close();
        Reply::raw(204, "application/json", b"")
    });
    let (service, _feed) = v1_api(json!({"share": "disabled"}), on_prompt);
    let (_fake, host) = v1_fake("1.18.10", &service);
    let (result, _) = dispatch_line(host, &chat_line("provider/model", json!({})));
    assert_eq!(
        result.unwrap_err().message(),
        "OpenCode event stream ended unexpectedly."
    );
}
