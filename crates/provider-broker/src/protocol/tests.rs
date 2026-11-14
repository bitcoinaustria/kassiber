//! Ports of the protocol cases in `broker.test.ts`, `fast-mode.test.ts`,
//! `sensitive.test.ts` and `stateless.test.ts`.

use super::*;
use serde_json::json;

fn chat(provider: ProviderId, options: Value) -> ChatRequest {
    ChatRequest::new(
        provider,
        "default",
        vec![ChatMessage::new("user", "synthetic prompt")],
    )
    .with_options(options)
}

fn tool() -> BrokerToolDefinition {
    BrokerToolDefinition::new(
        "status",
        "Return the Kassiber book status.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    )
    .unwrap()
}

#[test]
fn stamps_the_remote_tools_disabled_invariants_on_every_provider_status() {
    let status = provider_status(ProviderId::Claude, "Claude", ProviderState::Ready, "Ready.");
    assert_eq!(
        serde_json::to_string(&status).unwrap(),
        r#"{"provider":"claude","display_name":"Claude","privacy_posture":"remote","native_tools":"disabled","models":[],"state":"ready","message":"Ready."}"#
    );
    let named = status
        .with_executable("/Users/alice/.local/bin/claude")
        .with_version("2.1.0");
    assert_eq!(named.executable(), Some("claude"));
    let value = serde_json::to_value(&named).unwrap();
    assert_eq!(value["executable"], "claude");
    assert_eq!(value["version"], "2.1.0");
    // Windows separators too; a path ending in a separator keeps itself.
    let windows = provider_status(
        ProviderId::Copilot,
        "GitHub Copilot",
        ProviderState::Error,
        "x",
    )
    .with_executable(r"C:\Users\bob\bin\copilot.exe");
    assert_eq!(windows.executable(), Some("copilot.exe"));
    let trailing = provider_status(ProviderId::Codex, "Codex", ProviderState::Error, "x")
        .with_executable("/odd/");
    assert_eq!(trailing.executable(), Some("/odd/"));
}

#[test]
fn models_leave_absent_fields_out() {
    let model = BrokerModel {
        id: "gpt-5.4".into(),
        privacy_posture: Some(ModelPosture::Local),
        supports_fast_mode: Some(true),
        ..BrokerModel::default()
    };
    assert_eq!(
        serde_json::to_string(&model).unwrap(),
        r#"{"id":"gpt-5.4","privacy_posture":"local","supports_fast_mode":true}"#
    );
}

#[test]
fn redacts_credential_like_values_from_provider_errors() {
    let safe = safe_error_message("authorization: Bearer-secret token=abc api_key=def");
    assert!(!safe.contains("Bearer-secret"), "{safe}");
    assert!(!safe.contains("abc"), "{safe}");
    assert!(!safe.contains("def"), "{safe}");
}

#[test]
fn redacts_the_secret_itself_not_merely_its_label() {
    for (input, secrets) in [
        ("authorization: Bearer sk-secret-123", vec!["sk-secret-123"]),
        (r#"{"apiKey":"sk-live-abcdefgh"}"#, vec!["sk-live-abcdefgh"]),
        (
            "connect https://user:pa55word@example.com/v1 failed",
            vec!["pa55word"],
        ),
        (
            "token=ghp_abcdefghijklmnop rejected",
            vec!["ghp_abcdefghijklmnop"],
        ),
        (
            "cannot read /Users/alice/.config/opencode/auth.json",
            vec!["alice"],
        ),
    ] {
        let safe = safe_error_message(input);
        for secret in secrets {
            assert!(!safe.contains(secret), "{input} -> {safe}");
        }
    }
}

/// The exact output of the TypeScript `safeErrorMessage` on the same inputs
/// (run under Node during the port).
#[test]
fn redaction_matches_the_typescript_rules_exactly() {
    for (input, expected) in [
        (
            "authorization: Bearer sk-secret-123",
            "authorization: [redacted] [redacted]",
        ),
        (
            r#"{"apiKey":"sk-live-abcdefgh"}"#,
            r#"{"apiKey":[redacted]}"#,
        ),
        (
            "connect https://user:pa55word@example.com/v1 failed",
            "connect https://[redacted]@example.com/v1 failed",
        ),
        (
            "token=ghp_abcdefghijklmnop rejected",
            "token=[redacted] rejected",
        ),
        (
            "cannot read /Users/alice/.config/opencode/auth.json",
            "cannot read ~/.config/opencode/auth.json",
        ),
        (
            "authorization: Bearer-secret token=abc api_key=def",
            "authorization: [redacted] token=[redacted] api_key=[redacted]",
        ),
        ("Basic dXNlcjpwYXNz failed", "Basic [redacted] failed"),
        (
            "key sk-abcdefghijk and xoxb_12345678x",
            "key [redacted] and [redacted]",
        ),
        ("ask-abcdefghijk stays", "ask-abcdefghijk stays"),
        ("/home/bob/x and /homeless/y", "~/x and /homeless/y"),
        ("PASSWORD = 'hunter22' end", "PASSWORD = [redacted] end"),
        // An unclosed quote leaves the value alone, as the backreference does.
        (r#"token: "abc"#, r#"token: "abc"#),
        ("first line\r\nsecond line token=x", "first line"),
        ("   \n anything", "Provider request failed."),
        ("", "Provider request failed."),
        (
            "not logged in: token=abc",
            "not logged in: token=[redacted]",
        ),
        (
            "ſecret=abc and SECRET=def",
            "ſecret=abc and SECRET=[redacted]",
        ),
        (
            "Bearer tok and bearer x",
            "Bearer [redacted] and bearer [redacted]",
        ),
        (
            "api-key: 'q' and API_KEY=\"w\" and apikey=e",
            "api-key: [redacted] and API_KEY=[redacted] and apikey=[redacted]",
        ),
        (
            "_sk-abcdefghij Xsk-abcdefghij é-sk-abcdefghij",
            "_sk-abcdefghij Xsk-abcdefghij é-[redacted]",
        ),
    ] {
        assert_eq!(safe_error_message(input), expected, "{input}");
    }
}

#[test]
fn redaction_caps_the_message_at_400_utf16_units() {
    assert_eq!(safe_error_message(&"x".repeat(1000)).len(), 400);
    // 399 units, then a pair the cut would split: the pair goes (TypeScript
    // keeps its lone high surrogate, which Rust strings cannot hold).
    let text = format!("{}\u{1F600}tail", "y".repeat(399));
    assert_eq!(safe_error_message(&text), "y".repeat(399));
}

#[test]
fn classifies_errors_as_index_ts_does() {
    assert_eq!(
        error_code("Codex is not installed."),
        ErrorCode::MissingExecutable
    );
    assert_eq!(
        error_code("spawn /x/codex ENOENT"),
        ErrorCode::MissingExecutable
    );
    assert_eq!(error_code("model NOT FOUND"), ErrorCode::MissingExecutable);
    assert_eq!(
        error_code("Run `codex LOGIN`"),
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        error_code("Unauthorized"),
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(
        error_code("bad credentials"),
        ErrorCode::AuthenticationRequired
    );
    assert_eq!(error_code("Unknown provider."), ErrorCode::ProviderError);
    // JavaScript's `i` flag folds ASCII only.
    assert_eq!(error_code("\u{17f}ign in"), ErrorCode::ProviderError);
}

#[test]
fn error_events_hide_authentication_detail_behind_a_fixed_hint() {
    let line =
        |failure: &str| serde_json::to_string(&error_event(&BrokerFailure::new(failure))).unwrap();
    assert_eq!(
        line("Run `claude login` first: token=abc"),
        r#"{"type":"error","code":"authentication_required","message":"The provider requires authentication.","hint":"Run the provider's normal login command outside Kassiber."}"#
    );
    assert_eq!(
        line("Claude is not installed."),
        r#"{"type":"error","code":"missing_executable","message":"Claude is not installed."}"#
    );
    assert_eq!(
        line("boom at /home/carol/x"),
        r#"{"type":"error","code":"provider_error","message":"boom at ~/x"}"#
    );
}

#[test]
fn events_keep_the_typescript_field_order() {
    let lines: Vec<String> = [
        Event::Status {
            phase: "thinking".into(),
            message: "Working".into(),
        },
        Event::Delta {
            content: Some("hi".into()),
            reasoning: None,
        },
        Event::Delta {
            content: None,
            reasoning: Some("why".into()),
        },
        Event::ToolCall {
            call_id: "c1".into(),
            name: "status".into(),
            arguments: minify_json(
                &RawValue::from_string("{ \"z\": 1,\n \"a\": \" x \" }".into()).unwrap(),
            ),
        },
        Event::Done {
            finish_reason: "stop".into(),
            provider_session_id: None,
        },
        Event::Done {
            finish_reason: "stop".into(),
            provider_session_id: Some("s1".into()),
        },
        Event::result(&vec![1, 2]).unwrap(),
    ]
    .iter()
    .map(|event| serde_json::to_string(event).unwrap())
    .collect();
    assert_eq!(
        lines,
        [
            r#"{"type":"status","phase":"thinking","message":"Working"}"#,
            r#"{"type":"delta","content":"hi"}"#,
            r#"{"type":"delta","reasoning":"why"}"#,
            r#"{"type":"tool_call","call_id":"c1","name":"status","arguments":{"z":1,"a":" x "}}"#,
            r#"{"type":"done","finish_reason":"stop"}"#,
            r#"{"type":"done","finish_reason":"stop","provider_session_id":"s1"}"#,
            r#"{"type":"result","data":[1,2]}"#,
        ]
    );
}

#[test]
fn the_sink_writes_one_line_per_event() {
    let (sink, captured) = EventSink::capture();
    sink.write(&Event::Delta {
        content: Some("a\nb".into()),
        reasoning: None,
    });
    sink.write(&Event::Done {
        finish_reason: "stop".into(),
        provider_session_id: None,
    });
    assert_eq!(
        captured.text(),
        "{\"type\":\"delta\",\"content\":\"a\\nb\"}\n{\"type\":\"done\",\"finish_reason\":\"stop\"}\n"
    );
    assert_eq!(captured.events().len(), 2);
}

#[test]
fn refuses_a_session_cursor_that_could_be_read_as_a_cli_flag() {
    for hostile in [
        json!("--dangerously-skip-permissions"),
        json!("-r"),
        json!("--mcp-config=/tmp/evil.json"),
        json!("--debug-file=/tmp/x"),
        json!("id with space"),
        json!("id;rm -rf /"),
        json!(""),
        json!("  "),
        json!("a".repeat(201)),
        json!(null),
        json!(42),
    ] {
        assert_eq!(safe_session_cursor(Some(&hostile)), None, "{hostile}");
    }
    assert_eq!(safe_session_cursor(None), None);
    assert_eq!(
        safe_session_cursor(Some(&json!("3146c223-ddaa-4e7a-b1ed-6940080a5a26"))).as_deref(),
        Some("3146c223-ddaa-4e7a-b1ed-6940080a5a26")
    );
    assert_eq!(
        safe_session_cursor(Some(&json!("  ses_01ABC.def:ghi-jkl  "))).as_deref(),
        Some("ses_01ABC.def:ghi-jkl")
    );
    assert_eq!(
        safe_session_cursor(Some(&json!("a".repeat(200)))).map(|cursor| cursor.len()),
        Some(200)
    );
}

#[test]
fn starts_only_the_requested_runtimes_and_refuses_unknown_names() {
    let scope = |request: Value| status_scope(request.as_object().unwrap());
    use ProviderId::*;
    assert_eq!(
        scope(json!({"command": "status"})).unwrap(),
        [Codex, Claude, OpenCode, Copilot]
    );
    assert_eq!(
        scope(json!({"command": "status", "provider": "copilot"})).unwrap(),
        [Copilot]
    );
    assert_eq!(
        scope(json!({"command": "status", "providers": ["claude", "opencode", "claude"]})).unwrap(),
        [Claude, OpenCode]
    );
    assert_eq!(
        scope(json!({"command": "status", "providers": []})).unwrap(),
        []
    );
    for bad in [
        json!({"command": "status", "providers": ["claude", "gemini"]}),
        json!({"command": "status", "providers": "codex"}),
        json!({"command": "status", "provider": "gemini"}),
        // A present null is given, and unknown.
        json!({"command": "status", "provider": null}),
        json!({"command": "status", "providers": null}),
        // `provider` wins even when `providers` is valid.
        json!({"command": "status", "provider": "x", "providers": ["codex"]}),
    ] {
        assert_eq!(scope(bad).unwrap_err().message(), "Unknown provider.");
    }
}

#[test]
fn fast_mode_accepts_only_booleans() {
    let request = |options| chat(ProviderId::Codex, options);
    assert!(requested_fast_mode(&request(json!({"fast_mode": true}))).unwrap());
    assert!(!requested_fast_mode(&request(json!({"fast_mode": false}))).unwrap());
    assert!(!requested_fast_mode(&request(json!({}))).unwrap());
    assert!(!requested_fast_mode(&ChatRequest::new(ProviderId::Codex, "m", Vec::new())).unwrap());
    for bad in [json!({"fast_mode": "true"}), json!({"fast_mode": null})] {
        assert_eq!(
            requested_fast_mode(&request(bad)).unwrap_err().message(),
            "Invalid fast mode option."
        );
    }
}

#[test]
fn sensitive_context_rejects_session_continuation_tools_and_malformed_options() {
    let selected = || {
        chat(
            ProviderId::Claude,
            json!({"sensitive_context": true, "reasoning_effort": "high"}),
        )
    };
    assert!(sensitive_context(&selected()).unwrap());
    assert!(!sensitive_context(&chat(ProviderId::Claude, json!({}))).unwrap());
    let resumed = chat(
        ProviderId::Claude,
        json!({"sensitive_context": true, "provider_session_id": "prior"}),
    );
    assert!(sensitive_context(&resumed).is_err());
    let mut with_tools = selected();
    with_tools.tools = vec![tool()];
    assert!(sensitive_context(&with_tools).is_err());
    assert_eq!(
        sensitive_context(&chat(
            ProviderId::Claude,
            json!({"sensitive_context": "true"})
        ))
        .unwrap_err()
        .message(),
        "Invalid sensitive context mode."
    );
    // An empty cursor is not a session to resume.
    let empty = chat(
        ProviderId::Claude,
        json!({"sensitive_context": true, "provider_session_id": ""}),
    );
    assert!(sensitive_context(&empty).unwrap());
}

#[test]
fn stateless_session_accepts_only_booleans_and_never_a_session_to_resume() {
    assert!(
        stateless_session(&chat(ProviderId::Codex, json!({"stateless_session": true}))).unwrap()
    );
    assert!(!stateless_session(&chat(ProviderId::Codex, json!({}))).unwrap());
    assert!(stateless_session(&chat(
        ProviderId::Codex,
        json!({"stateless_session": "yes"})
    ))
    .is_err());
    let resumed = chat(
        ProviderId::Codex,
        json!({"stateless_session": true, "provider_session_id": "prior"}),
    );
    assert!(stateless_session(&resumed)
        .unwrap_err()
        .message()
        .contains("cannot resume"));
    // Tools stay allowed.
    let mut with_tools = chat(ProviderId::Codex, json!({"stateless_session": true}));
    with_tools.tools = vec![tool()];
    assert!(stateless_session(&with_tools).unwrap());
}

#[test]
fn chat_requests_parse_as_the_daemon_writes_them() {
    let line = r#"{"command": "chat", "request_id": "daemon-chat", "provider": "claude", "model": "default", "messages": [{"role": "user", "content": "hi", "images": []}], "instructions": null, "tools": [{"name": "ui_reports_summary", "description": "Summary", "parameters": {"type": "object", "properties": {"z": {}, "a": {}}}, "read_only": true, "destructive": false}], "options": {"reasoning_effort": "high"}}"#;
    let request: ChatRequest = serde_json::from_str(line).unwrap();
    assert_eq!(request.provider, ProviderId::Claude);
    assert_eq!(request.instructions(), None);
    assert_eq!(request.option("reasoning_effort"), Some(&json!("high")));
    assert_eq!(request.option("missing"), None);
    assert_eq!(request.tools.len(), 1);
    // The schema keeps the daemon's key order.
    assert_eq!(
        serde_json::to_string(&request.tools).unwrap(),
        r#"[{"name":"ui_reports_summary","description":"Summary","parameters":{"type": "object", "properties": {"z": {}, "a": {}}},"read_only":true,"destructive":false}]"#
    );
    let without: ChatRequest = serde_json::from_str(
        r#"{"provider": "codex", "model": "m", "messages": [], "tools": null}"#,
    )
    .unwrap();
    assert!(without.tools.is_empty());
    assert!(serde_json::from_str::<ChatRequest>(
        r#"{"provider": "gemini", "model": "m", "messages": []}"#
    )
    .is_err());
}
