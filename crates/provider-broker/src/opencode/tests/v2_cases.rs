//! `opencode2.test.ts`: OpenCode 2 against a fake CLI and its loopback API.

use std::collections::BTreeSet;

use super::*;
use crate::opencode::v2::{environment, parse_config};

/// One API request the fake saw.
#[derive(Clone, Debug)]
pub(in crate::opencode) struct Seen {
    pub port: u16,
    pub method: String,
    pub path: String,
    pub body: Value,
    pub authorization: Option<String>,
}

/// How the fake API behaves.
#[derive(Clone, Default)]
pub(in crate::opencode) struct Behaviour {
    pub ending: Option<&'static str>,
    pub connected_delay: Option<Duration>,
    pub mcp_delay: Option<Duration>,
    pub oauth: bool,
    pub never_connected: bool,
}

/// The fake OpenCode 2 API on as many loopback ports as the turn starts
/// servers, one per `serve`, sharing one log.
pub(in crate::opencode) struct Api {
    pub services: Vec<FakeService>,
    pub seen: Arc<Mutex<Vec<Seen>>>,
}

impl Api {
    pub fn start(fake: &Fake, servers: usize, behaviour: Behaviour) -> Self {
        Self::start_with(fake, servers, behaviour, None)
    }

    /// `answer` replaces the fixture's answers (a server that fails).
    pub fn start_with(
        fake: &Fake,
        servers: usize,
        behaviour: Behaviour,
        answer: Option<fn(&HandledRequest) -> Reply>,
    ) -> Self {
        fake.write("serve-mode", "v2");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let feed = Feed::default();
        let state = Arc::new(Mutex::new((false, false, false)));
        let mut services = Vec::new();
        for index in 0..servers {
            let (seen, feed, state, behaviour) =
                (seen.clone(), feed.clone(), state.clone(), behaviour.clone());
            let port = Arc::new(Mutex::new(0u16));
            let own = port.clone();
            let service = FakeService::start(move |request| {
                let port = *own.lock().unwrap();
                seen.lock().unwrap().push(Seen {
                    port,
                    method: request.method.clone(),
                    path: path_of(request).to_owned(),
                    body: body(request),
                    authorization: request.header("authorization").map(str::to_owned),
                });
                if let Some(answer) = answer {
                    return answer(request);
                }
                answer_v2(request, &feed, &state, &behaviour)
            })
            .unwrap();
            *port.lock().unwrap() = service.port();
            fake.write(&format!("ports/{index}"), &service.port().to_string());
            services.push(service);
        }
        Self { services, seen }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }

    pub fn paths(&self) -> Vec<String> {
        self.seen().into_iter().map(|seen| seen.path).collect()
    }
}

fn data(value: Value) -> Reply {
    Reply::json(200, &json!({ "data": value }).to_string())
}

/// The fixture API of `opencode2.test.ts`. `state` is (connected,
/// mcpRequested, mcpReady).
fn answer_v2(
    request: &HandledRequest,
    feed: &Feed,
    state: &Arc<Mutex<(bool, bool, bool)>>,
    behaviour: &Behaviour,
) -> Reply {
    let path = path_of(request);
    match (request.method.as_str(), path) {
        (_, "/api/model") => data(json!([{
            "providerID": "fixture", "id": "namespaced/model", "name": "Saved fixture",
            "variants": [{"id": "high"}], "enabled": true, "capabilities": {"tools": true}
        }])),
        (_, "/api/provider") => data(json!([{
            "id": "fixture", "integrationID": "selected-account",
            "settings": {"baseURL": "http://127.0.0.1:1234/v1", "apiKey": "secret-key"}
        }])),
        ("GET", "/api/credential") => data(json!([
            {"integrationID": "selected-account", "active": true, "value": if behaviour.oauth {
                json!({"type": "oauth", "access": "synthetic-access", "refresh": "synthetic-refresh", "expires": 1})
            } else {
                json!({"type": "key", "key": "selected-secret"})
            }},
            {"integrationID": "selected-account", "active": false, "value": {"type": "key", "key": "inactive-secret"}},
            {"integrationID": "other-account", "active": true, "value": {"type": "key", "key": "other-secret"}},
        ])),
        (_, "/api/credential") => data(json!({"id": "created-credential"})),
        (_, "/api/experimental/mcp/kassiber") => Reply::raw(204, "application/json", b""),
        (_, "/api/experimental/mcp/kassiber/connect") => {
            state.lock().unwrap().1 = true;
            std::thread::sleep(behaviour.mcp_delay.unwrap_or_default());
            state.lock().unwrap().2 = true;
            Reply::raw(204, "application/json", b"")
        }
        (_, "/api/mcp") => data(json!([{"name": "kassiber", "status": {"status": "connected"}}])),
        (_, "/api/session") => {
            let (_, requested, ready) = *state.lock().unwrap();
            assert!(
                !requested || ready,
                "the session waits for the MCP connection"
            );
            data(json!({"id": "ses_fixture"}))
        }
        (_, "/api/session/ses_fixture") => {
            data(json!({"id": "ses_fixture", "outcome": "succeeded"}))
        }
        (_, "/api/event") => {
            let reply = feed.open();
            if !behaviour.never_connected {
                let (feed, state) = (feed.clone(), state.clone());
                let delay = behaviour.connected_delay.unwrap_or_default();
                std::thread::spawn(move || {
                    std::thread::sleep(delay);
                    state.lock().unwrap().0 = true;
                    feed.emit(json!({"type": "server.connected", "data": {}}));
                });
            }
            reply
        }
        (_, path) if path.ends_with("/prompt") => {
            assert!(
                state.lock().unwrap().0,
                "the prompt waits for the event stream"
            );
            let emit = |kind: &str, data: Value| feed.emit(json!({"type": kind, "data": data}));
            emit(
                "session.text.delta",
                json!({"sessionID": "ses_other", "delta": "foreign-secret"}),
            );
            emit(
                "session.reasoning.delta",
                json!({"sessionID": "ses_fixture", "delta": "Reasoning fixture"}),
            );
            emit(
                "session.text.delta",
                json!({"sessionID": "ses_fixture", "delta": "Answer fixture"}),
            );
            emit(
                behaviour.ending.unwrap_or("session.execution.succeeded"),
                json!({"sessionID": "ses_fixture", "name": "shell", "error": {"message": "private provider diagnostic"}}),
            );
            data(json!({}))
        }
        _ => Reply::json(404, "{}"),
    }
}

/// `fixture(config)`: a v2 CLI with `config` as the user's opencode.json.
pub(in crate::opencode) fn fixture(config: Value) -> (Fake, Host) {
    let fake = Fake::new("opencode v2.2.0");
    fake.write("config/opencode/opencode.json", &config.to_string());
    let host = fake.host_with(&[
        ("XDG_CONFIG_HOME", fake.path("config").to_str().unwrap()),
        ("XDG_DATA_HOME", fake.path("data").to_str().unwrap()),
    ]);
    (fake, host)
}

fn request(options: Value) -> ChatRequest {
    let mut request = chat_request("fixture/namespaced/model", options);
    request.instructions = Some(json!("Kassiber system fixture"));
    request
}

fn config_of(run: &Run) -> Value {
    serde_json::from_str(&run.vars["OPENCODE_CONFIG_CONTENT"]).unwrap()
}

fn basic(password: &str) -> String {
    const STANDARD: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = format!("opencode:{password}").into_bytes();
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk
            .iter()
            .enumerate()
            .fold(0u32, |acc, (i, b)| acc | (u32::from(*b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(STANDARD[((value >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    format!("Basic {out}")
}

#[test]
fn discovers_the_v2_catalog_and_preserves_local_routing_without_leaking_provider_settings() {
    let (fake, host) = fixture(json!({}));
    let api = Api::start(&fake, 1, Behaviour::default());
    let status = status(&host, &fake.turn());
    assert_eq!(status.state, ProviderState::Ready, "{}", status.message);
    assert_eq!(status.version.as_deref(), Some("opencode v2.2.0"));
    assert_eq!(status.models.len(), 1);
    let model = serde_json::to_value(&status.models[0]).unwrap();
    assert_eq!(
        model,
        json!({
            "id": "fixture/namespaced/model", "display_name": "Saved fixture",
            "owned_by": "OpenCode · fixture", "source_provider": "fixture",
            "privacy_posture": "local",
            "privacy_reason": "OpenCode reports a loopback endpoint for this model.",
            "supports_reasoning_effort": true, "reasoning_efforts": ["high"],
            "supports_tools": true,
        })
    );
    assert!(!serde_json::to_string(&status)
        .unwrap()
        .contains("secret-key"));
    assert_eq!(api.paths(), ["/api/model", "/api/provider"]);
    let server = &fake.serves()[0];
    assert_eq!(
        server.argv,
        ["serve", "--stdio", "--hostname=127.0.0.1", "--port=0"]
    );
    let password = &server.vars["OPENCODE_SERVER_PASSWORD"];
    assert_eq!(password.len(), 43);
    assert!(api
        .seen()
        .iter()
        .all(|seen| seen.authorization.as_deref() == Some(basic(password).as_str())));
}

#[test]
fn streams_reasoning_and_text_filters_other_sessions_and_completes_on_the_v2_execution_event() {
    let (fake, host) = fixture(json!({}));
    let api = Api::start(&fake, 1, Behaviour::default());
    let turn = request(json!({"reasoning_effort": "high", "provider_session_id": "old-v1-cursor"}));
    let (result, events, text) = run_chat(&fake, &host, &turn, None);
    result.unwrap();
    assert!(events.contains(&json!({"type": "delta", "content": "Answer fixture"})));
    assert!(events.contains(&json!({"type": "delta", "reasoning": "Reasoning fixture"})));
    assert!(events.contains(&json!({"type": "done", "finish_reason": "stop"})));
    assert_eq!(
        events[0],
        json!({"type": "status", "phase": "connecting", "message": "Starting OpenCode server"})
    );
    assert!(!text.contains("foreign-secret"));
    let seen = api.seen();
    let create = seen
        .iter()
        .find(|seen| seen.path == "/api/session")
        .unwrap();
    assert_eq!(
        create.body["model"],
        json!({"providerID": "fixture", "id": "namespaced/model", "variant": "high"})
    );
    assert_eq!(
        create.body["permissions"],
        json!([{"action": "*", "resource": "*", "effect": "deny"}])
    );
    let prompt = seen
        .iter()
        .find(|seen| seen.path.ends_with("/prompt"))
        .unwrap();
    assert!(prompt.body["text"]
        .as_str()
        .unwrap()
        .contains("Synthetic user message"));
    assert!(!seen.iter().any(|seen| seen.path.contains("old-v1-cursor")));
}

#[test]
fn fails_closed_on_failure_events_without_leaking_provider_diagnostics() {
    for ending in [
        "session.execution.failed",
        "session.execution.interrupted",
        "session.tool.input.started",
        "permission.asked",
    ] {
        let (fake, host) = fixture(json!({}));
        let _api = Api::start(
            &fake,
            1,
            Behaviour {
                ending: Some(ending),
                ..Behaviour::default()
            },
        );
        let (result, _, text) = run_chat(&fake, &host, &request(json!({})), None);
        let failure = result.unwrap_err();
        assert!(failure.message().contains("OpenCode"), "{ending}");
        assert!(!failure.message().contains("private provider diagnostic"));
        assert!(!text.contains("private provider diagnostic"));
    }
}

#[test]
fn exposes_only_the_named_kassiber_tools_and_disables_mcp_code_mode() {
    let (fake, host) = fixture(json!({}));
    let api = Api::start(
        &fake,
        1,
        Behaviour {
            mcp_delay: Some(Duration::from_millis(75)),
            connected_delay: Some(Duration::from_millis(75)),
            ..Behaviour::default()
        },
    );
    let mut turn = request(json!({}));
    turn.tools = vec![tool("book_read")];
    let (events, _) = EventSink::capture();
    let bridge = crate::ToolBridge::start(&turn.tools, events).unwrap();
    let (result, _, _) = run_chat(&fake, &host, &turn, Some(&bridge));
    result.unwrap();
    let seen = api.seen();
    let create = seen
        .iter()
        .find(|seen| seen.path == "/api/session")
        .unwrap();
    assert_eq!(
        create.body["permissions"],
        json!([
            {"action": "*", "resource": "*", "effect": "deny"},
            {"action": "kassiber_book_read", "resource": "*", "effect": "allow"},
        ])
    );
    let mcp = seen
        .iter()
        .find(|seen| seen.path == "/api/experimental/mcp/kassiber")
        .unwrap();
    assert_eq!(mcp.method, "PUT");
    assert_eq!(mcp.body["config"]["type"], "local");
    assert_eq!(mcp.body["config"]["codemode"], false);
    let command = mcp.body["config"]["command"].as_array().unwrap();
    assert_eq!(command[1], "--provider-broker-mcp");
    assert_eq!(command[2], bridge.socket_path());
    let paths = api.paths();
    let at = |path: &str| paths.iter().position(|seen| seen == path).unwrap();
    assert!(at("/api/session") > at("/api/experimental/mcp/kassiber/connect"));
    assert!(at("/api/session") > at("/api/mcp"));
    assert!(at("/api/session/ses_fixture/prompt") > at("/api/event"));
    bridge.close().unwrap();
}

#[test]
fn refuses_private_oauth_turns_before_copying_or_refreshing_the_persistent_credential() {
    let (fake, host) = fixture(json!({}));
    let api = Api::start(
        &fake,
        2,
        Behaviour {
            oauth: true,
            ..Behaviour::default()
        },
    );
    let (result, _, _) = run_chat(
        &fake,
        &host,
        &request(json!({"stateless_session": true})),
        None,
    );
    assert!(result
        .unwrap_err()
        .message()
        .contains("OAuth accounts cannot safely"));
    assert_eq!(fake.serves().len(), 1);
    assert!(!api
        .seen()
        .iter()
        .any(|seen| seen.path.contains("session") || seen.method != "GET"));
}

#[test]
fn times_out_a_server_that_sends_event_headers_without_its_ready_frame() {
    let (fake, mut host) = fixture(json!({}));
    host.event_ready = Duration::from_millis(300);
    let api = Api::start(
        &fake,
        1,
        Behaviour {
            never_connected: true,
            ..Behaviour::default()
        },
    );
    let (result, _, _) = run_chat(&fake, &host, &request(json!({})), None);
    assert!(result.unwrap_err().message().contains("aborted"));
    assert!(!api.paths().iter().any(|path| path.ends_with("/prompt")));
}

#[test]
fn isolates_private_turns_and_hands_off_only_the_active_selected_credential() {
    for options in [
        json!({"stateless_session": true}),
        json!({"sensitive_context": true}),
    ] {
        let (fake, host) = fixture(json!({}));
        let api = Api::start(&fake, 2, Behaviour::default());
        let (result, _, text) = run_chat(&fake, &host, &request(options.clone()), None);
        result.unwrap();
        let servers = fake.serves();
        assert_eq!(servers.len(), 2, "{options}");
        assert_eq!(
            servers[0].vars["OPENCODE_DB"],
            fake.path("data/opencode/opencode.db").to_str().unwrap()
        );
        assert_eq!(servers[1].vars["OPENCODE_DB"], ":memory:");
        assert!(!config_of(&servers[0])
            .to_string()
            .contains("Kassiber system fixture"));
        assert!(config_of(&servers[1])
            .to_string()
            .contains("Kassiber system fixture"));
        let second = api.services[1].port();
        let posted: Vec<_> = api
            .seen()
            .into_iter()
            .filter(|seen| seen.path == "/api/credential" && seen.method == "POST")
            .collect();
        assert_eq!(posted.len(), 1);
        assert_eq!(posted[0].port, second);
        assert_eq!(
            posted[0].body,
            json!({"integrationID": "selected-account", "value": {"type": "key", "key": "selected-secret"}, "activate": true})
        );
        assert!(api
            .seen()
            .iter()
            .filter(|seen| seen.path.contains("/session"))
            .all(|seen| seen.port == second));
        for secret in ["selected-secret", "inactive-secret", "other-secret"] {
            assert!(!text.contains(secret));
        }
    }
}

#[test]
fn loads_only_provider_configuration_and_discards_logs_before_launch() {
    let (fake, host) = fixture(json!({
        "plugins": ["evil-plugin"],
        "mcp": {"servers": {"evil": {"type": "local", "command": ["shell"]}}},
        "instructions": ["https://invalid.example"],
        "providers": {"fixture": {"settings": {"baseURL": "http://localhost:1234", "apiKey": "fake-key"}}},
    }));
    let root = fake.turn();
    let env = environment(&host, &root, Some(&request(json!({})))).unwrap();
    let config: Value =
        serde_json::from_str(&env.get_str("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(
        config["providers"],
        json!({"fixture": {"settings": {"baseURL": "http://localhost:1234", "apiKey": "fake-key"}}})
    );
    let text = config.to_string();
    assert!(!text.contains("evil") && !text.contains("invalid.example"));
    assert!(config["plugins"].as_array().unwrap().contains(&json!("-*")));
    assert_eq!(config["mcp"], json!({"servers": {}}));
    assert_eq!(config["share"], "disabled");
    assert_eq!(config["warming"], false);
    assert_eq!(
        env.get_str("OPENCODE_CONFIG_PROJECT_DISABLE").as_deref(),
        Some("1")
    );
    let data = PathBuf::from(env.get("XDG_DATA_HOME").unwrap());
    assert_eq!(
        std::fs::read_link(data.join("opencode/log/opencode.log")).unwrap(),
        Path::new("/dev/null")
    );
    // Ambient OpenCode overrides go; the provider variables a config names
    // come along.
    assert!(env
        .names()
        .filter(|name| name.starts_with("OPENCODE_"))
        .all(|name| [
            "OPENCODE_CONFIG_DIR",
            "OPENCODE_CONFIG_CONTENT",
            "OPENCODE_CLI_CONFIG_CONTENT",
            "OPENCODE_CONFIG_PROJECT_DISABLE",
            "OPENCODE_DISABLE_CLAUDE_CODE",
            "OPENCODE_DISABLE_MODELS_FETCH",
            "OPENCODE_DISABLE_FREE_MODELS",
            "OPENCODE_FILEWATCHER_DISABLE",
            "OPENCODE_DISABLE_FFF",
            "OPENCODE_TEST_HOME",
            "OPENCODE_DB",
        ]
        .contains(&name.as_str())));
}

#[test]
fn keeps_provider_variables_and_anchors_relative_credential_files() {
    let (fake, _) = fixture(json!({
        "provider": {"local": {
            "env": ["LOCAL_KEY"],
            "options": {"apiKey": "{env:OTHER_KEY}", "keyFile": "{file:keys/local.txt}", "home": "{file:~/k}"}
        }},
        "plugin": ["ignored"],
    }));
    let host = fake.host_with(&[
        ("XDG_CONFIG_HOME", fake.path("config").to_str().unwrap()),
        ("XDG_DATA_HOME", fake.path("data").to_str().unwrap()),
        ("LOCAL_KEY", "local-value"),
        ("OTHER_KEY", "other-value"),
        ("OPENCODE_SERVER_PASSWORD", "ambient"),
        (
            "OPENCODE_CONFIG_CONTENT",
            r#"{"enabled_providers":["local"], // inline
            "model":"x/y",}"#,
        ),
    ]);
    let root = fake.turn();
    let env = environment(&host, &root, None).unwrap();
    assert_eq!(env.get_str("LOCAL_KEY").as_deref(), Some("local-value"));
    assert_eq!(env.get_str("OTHER_KEY").as_deref(), Some("other-value"));
    assert_eq!(env.get("OPENCODE_SERVER_PASSWORD"), None);
    let config: Value =
        serde_json::from_str(&env.get_str("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    let options = &config["provider"]["local"]["options"];
    assert_eq!(
        options["keyFile"],
        format!(
            "{{file:{}}}",
            fake.path("config/opencode/keys/local.txt").display()
        )
    );
    assert_eq!(options["home"], "{file:~/k}");
    assert_eq!(config["enabled_providers"], json!(["local"]));
    assert!(config.get("model").is_none() && config.get("plugin").is_none());
    let keys: BTreeSet<_> = config.as_object().unwrap().keys().cloned().collect();
    assert!(keys.contains("agents") && keys.contains("default_agent"));
    assert_eq!(
        config["agents"]["kassiber"]["system"],
        "Kassiber model check."
    );
}

#[test]
fn refuses_system_managed_configuration() {
    let (fake, mut host) = fixture(json!({}));
    std::fs::create_dir_all(fake.path("managed-opencode")).unwrap();
    host.managed = Some(vec![fake.path("managed-opencode")]);
    let error = environment(&host, &fake.turn(), None).unwrap_err();
    assert!(error
        .message()
        .starts_with("System-managed OpenCode configuration"));
}

fn failing_503(_: &HandledRequest) -> Reply {
    Reply::raw(503, "text/plain", b"private prompt and credential")
}

fn malformed(_: &HandledRequest) -> Reply {
    Reply::raw(
        200,
        "application/json",
        b"private account data: invalid JSON",
    )
}

#[test]
fn reports_http_failures_using_only_bounded_status_codes() {
    let (fake, host) = fixture(json!({}));
    let _api = Api::start_with(&fake, 1, Behaviour::default(), Some(failing_503));
    let status = status(&host, &fake.turn());
    assert_eq!(status.state, ProviderState::Error);
    assert!(status.message.contains("HTTP 503"));
    assert!(!serde_json::to_string(&status)
        .unwrap()
        .contains("private prompt"));
}

#[test]
fn never_includes_malformed_api_payloads_in_a_check_error() {
    let (fake, host) = fixture(json!({}));
    let _api = Api::start_with(&fake, 1, Behaviour::default(), Some(malformed));
    let status = status(&host, &fake.turn());
    assert_eq!(status.state, ProviderState::Error);
    assert_eq!(status.message, "OpenCode returned an invalid API response.");
}

#[test]
fn rejects_a_replaced_log_sink_before_launching_opencode() {
    let (fake, host) = fixture(json!({}));
    let root = fake.turn();
    let env = environment(&host, &root, None).unwrap();
    let sink = PathBuf::from(env.get("XDG_DATA_HOME").unwrap()).join("opencode/log/opencode.log");
    std::fs::remove_file(&sink).unwrap();
    std::fs::write(&sink, "disposable fixture").unwrap();
    let error = environment(&host, &root, None).unwrap_err();
    assert!(error
        .message()
        .contains("discard sink could not be verified"));
    assert!(fake.serves().is_empty());
}

#[test]
fn sends_attached_images_as_inline_data_urls_only() {
    let (fake, host) = fixture(json!({}));
    let api = Api::start(&fake, 1, Behaviour::default());
    let mut turn = request(json!({}));
    turn.messages[0].images = Some(vec![
        json!({"media_type": "image/png", "data": "iVBORw0KGgo="}),
    ]);
    let (result, _, _) = run_chat(&fake, &host, &turn, None);
    result.unwrap();
    let prompt = api
        .seen()
        .into_iter()
        .find(|seen| seen.path.ends_with("/prompt"))
        .unwrap();
    assert_eq!(
        prompt.body["files"],
        json!([{"uri": "data:image/png;base64,iVBORw0KGgo="}])
    );
}

#[test]
fn jsonc_keeps_quoted_comments_and_commas_and_strips_real_ones() {
    let config =
        parse_config("{/*comment*/\"url\":\"http://localhost/\", \"text\":\"quoted ,} //\", // line\n \"list\":[1,],}")
            .unwrap();
    assert_eq!(
        stringify(&Js::Obj(config)),
        r#"{"url":"http://localhost/","text":"quoted ,} //","list":[1]}"#
    );
}

#[test]
fn invalid_config_errors_are_sanitized() {
    for text in ["[]", "null", "{\"key\":\"private broken value\""] {
        assert_eq!(
            parse_config(text).unwrap_err().message(),
            "OpenCode provider configuration could not be read. Check its JSON or JSONC outside Kassiber."
        );
    }
}
