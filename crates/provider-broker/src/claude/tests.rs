//! The Claude adapter: the Claude cases of `fast-mode.test.ts`,
//! `images.test.ts`, `sensitive.test.ts`, `stateless.test.ts` and
//! `broker.test.ts`, then turns against the fake CLI in
//! `tests/fake-cli/claude`.

use super::*;
use crate::codex::fake_cli::{capture, chat, request, workdir, Fixture};
use crate::protocol::{BrokerToolDefinition, ChatMessage};
use serde_json::json;

const OPUS_SESSION: &str = "322f88aa-f2bf-4a55-bf83-ed3b41fd535b";

fn args(request: &ChatRequest) -> Vec<String> {
    chat_args(request, Path::new("/tmp/empty-fixture"), None).unwrap()
}

fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
    let index = args.iter().position(|arg| arg == flag).unwrap();
    &args[index + 1]
}

fn claude(model: &str, options: Value) -> ChatRequest {
    chat(ProviderId::Claude, model, options)
}

// fast-mode.test.ts: "Claude fast mode".

#[test]
fn advertises_fast_mode_only_on_the_opus_alias() {
    let fast: Vec<String> = claude_models()
        .into_iter()
        .filter(|model| model.supports_fast_mode == Some(true))
        .map(|model| model.id)
        .collect();
    assert_eq!(fast, ["opus"]);
    let ids: Vec<String> = claude_models().into_iter().map(|model| model.id).collect();
    assert_eq!(ids, ["default", "fable", "opus", "sonnet", "haiku"]);
}

#[test]
fn turns_fast_mode_on_only_when_requested_on_opus_and_off_explicitly_otherwise() {
    let fast = args(&claude("opus", json!({"fast_mode": true})));
    assert_eq!(after(&fast, "--settings"), r#"{"fastMode":true}"#);
    // The variadic tool denial must stay last.
    let index = fast
        .iter()
        .position(|arg| arg == "--disallowed-tools")
        .unwrap();
    assert_eq!(fast[index + 1..], CLAUDE_DISABLED_TOOLS.map(str::to_owned));
    // Other settings sources stay off.
    assert_eq!(after(&fast, "--setting-sources"), "");

    for request in [
        claude("opus", json!({})),
        claude("opus", json!({"fast_mode": false})),
        // A resumed session an earlier turn ran fast: leaving the setting
        // out would not be enough to turn it off.
        claude("opus", json!({"provider_session_id": OPUS_SESSION})),
        // Accepted by the CLI but silently ignored there, so never requested.
        claude("sonnet", json!({"fast_mode": true})),
        claude("default", json!({"fast_mode": true})),
    ] {
        assert!(!claude_fast_mode_applies(&request).unwrap());
        let off = args(&request);
        assert_eq!(off.iter().filter(|arg| *arg == "--settings").count(), 1);
        assert_eq!(after(&off, "--settings"), r#"{"fastMode":false}"#);
        assert!(!off.join(" ").contains(r#""fastMode":true"#));
    }
    assert_eq!(
        claude_fast_mode_applies(&claude("opus", json!({"fast_mode": "true"})))
            .unwrap_err()
            .message(),
        "Invalid fast mode option."
    );
}

// fast-mode.test.ts: "Claude fast mode version floor".

#[test]
fn offers_fast_mode_only_from_the_first_cli_that_honours_it_under_print() {
    for version in [
        "2.1.205 (Claude Code)",
        "2.1.287 (Claude Code)",
        "2.2.0",
        "3.0.0",
    ] {
        assert!(claude_version_supports_fast(Some(version)), "{version}");
    }
    for version in [
        Some("2.1.204 (Claude Code)"),
        Some("2.0.999"),
        Some("1.9.300"),
        None,
        Some("unknown"),
    ] {
        assert!(!claude_version_supports_fast(version), "{version:?}");
    }
    // The first full triple counts, wherever it starts.
    assert!(claude_version_supports_fast(Some("claude v2.1.300")));
    assert!(claude_version_supports_fast(Some("1.2 then 2.1.300")));
    assert!(!claude_version_supports_fast(Some("2.1 then 2.1.204")));
    assert!(claude_version_supports_fast(Some("x2.1.205.9")));
}

#[test]
fn withholds_the_opus_fast_toggle_from_an_older_cli() {
    let fast_ids = |version: Option<&str>| -> Vec<String> {
        claude_models_for(version)
            .into_iter()
            .filter(|model| model.supports_fast_mode == Some(true))
            .map(|model| model.id)
            .collect()
    };
    assert_eq!(fast_ids(Some("2.1.287 (Claude Code)")), ["opus"]);
    assert!(fast_ids(Some("2.1.204 (Claude Code)")).is_empty());
    assert!(fast_ids(None).is_empty());
    let opus = claude_models_for(Some("2.1.204"))
        .into_iter()
        .find(|model| model.id == "opus")
        .unwrap();
    assert_eq!(opus.fast_mode_description, None);
    assert!(!serde_json::to_string(&opus).unwrap().contains("fast_mode"));
}

/// The documented out-of-credits notice, as Claude Code 2.1.287 prints it.
fn credits_notice() -> Value {
    json!({
        "type": "system",
        "subtype": "notification",
        "key": "fast-mode-overage-rejected",
        "text": "Fast mode disabled \u{b7} usage credits exhausted",
        "priority": "immediate",
        "color": "error",
        "session_id": "s1",
    })
}

#[test]
fn recognises_only_claudes_fast_fallback_notification() {
    let mut without_key = credits_notice();
    without_key.as_object_mut().unwrap().remove("key");
    let with = |field: &str, value: &str| {
        let mut message = without_key.clone();
        message[field] = json!(value);
        message
    };
    for message in [
        credits_notice(),
        without_key.clone(),
        with(
            "text",
            "Fast mode disabled \u{b7} usage credit limit reached",
        ),
        json!({"type": "system", "subtype": "notification", "key": "fast-mode-overage-rejected"}),
        with("text", "  Fast mode disabled"),
    ] {
        assert!(claude_fast_fallback_notification(&message), "{message}");
    }
    for message in [
        with("subtype", "init"),
        with("type", "assistant"),
        with("text", "Fast mode ON"),
        with(
            "text",
            "Note: Fast mode disabled \u{b7} usage credits exhausted",
        ),
        with("text", "Fast mode disabledness"),
        json!({"type": "system", "subtype": "notification"}),
        json!(null),
        json!(5),
    ] {
        assert!(!claude_fast_fallback_notification(&message), "{message}");
    }
}

// fast-mode.test.ts: "desktop mirrors of fixed broker capabilities".

#[test]
fn matches_the_desktops_mirror_of_claudes_effort_levels() {
    let source = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../ui-tauri/src/lib/aiCapabilities.ts"),
    )
    .unwrap();
    let line = source
        .lines()
        .skip_while(|line| !line.contains("NATIVE_RUNTIME_REASONING_EFFORTS"))
        .find(|line| line.trim_start().starts_with("claude:"))
        .unwrap();
    let mirrored: String = CLAUDE_EFFORTS
        .iter()
        .map(|effort| format!("\"{effort}\""))
        .collect::<Vec<_>>()
        .join(", ");
    assert_eq!(line.trim(), format!("claude: [{mirrored}],"));
}

// broker.test.ts: "removes native tools ...".

#[test]
fn names_the_file_and_exec_tools_it_disables() {
    assert!(CLAUDE_DISABLED_TOOLS.contains(&"Bash"));
    assert!(CLAUDE_DISABLED_TOOLS.contains(&"Read"));
}

#[test]
fn builds_the_layered_tool_denial_invocation() {
    let mut request = claude("default", json!({"reasoning_effort": "high"}));
    request.instructions = Some(json!("Book: demo."));
    let built = args(&request);
    assert_eq!(
        built[..12],
        [
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--include-partial-messages",
            "--setting-sources",
            "",
            "--disable-slash-commands",
            "--strict-mcp-config",
            "--permission-mode",
            "dontAsk",
            "--system-prompt",
        ]
    );
    assert_eq!(built[12], system_instructions(&request));
    assert_eq!(built[13..15], ["--tools", ""]);
    assert!(!built.iter().any(|arg| arg == "--model"));
    assert!(!built.iter().any(|arg| arg.starts_with("--resume")));
    assert_eq!(after(&built, "--effort"), "high");
    for effort in [json!("auto"), json!(""), json!(null), json!(false)] {
        let quiet = args(&claude("sonnet", json!({"reasoning_effort": effort})));
        assert!(!quiet.iter().any(|arg| arg == "--effort"), "{effort}");
        assert_eq!(after(&quiet, "--model"), "sonnet");
    }
    let resumed = args(&claude(
        "opus",
        json!({"provider_session_id": OPUS_SESSION}),
    ));
    assert!(resumed.contains(&format!("--resume={OPUS_SESSION}")));
    // A cursor that could be read as a flag starts a fresh session.
    let hostile = args(&claude(
        "opus",
        json!({"provider_session_id": "--dangerously-skip-permissions"}),
    ));
    assert!(!hostile.iter().any(|arg| arg.contains("dangerously")));
}

// images.test.ts.

const PNG: &str = "iVBORw0KGgo=";

fn with_images(content: &str, images: Value) -> ChatMessage {
    serde_json::from_value(json!({"role": "user", "content": content, "images": images})).unwrap()
}

#[test]
fn gives_claude_one_structured_message_only_when_there_are_images() {
    let latest = with_images("second", json!([{"media_type": "image/jpeg", "data": PNG}]));
    let with = ChatRequest::new(ProviderId::Claude, "default", vec![latest]);
    let built = args(&with);
    assert!(built.join(" ").contains("--input-format stream-json"));
    // The variadic flag stays last.
    let input = built
        .iter()
        .position(|arg| arg == "--input-format")
        .unwrap();
    let denied = built
        .iter()
        .position(|arg| arg == "--disallowed-tools")
        .unwrap();
    assert!(input < denied);
    let plain = args(&ChatRequest::new(
        ProviderId::Claude,
        "default",
        vec![ChatMessage::new("user", "x")],
    ));
    assert!(!plain.iter().any(|arg| arg == "--input-format"));

    let image = BrokerImage {
        media_type: "image/png".into(),
        data: PNG.into(),
    };
    let line = claude_stream_input("USER:\nsecond", &[image]);
    assert!(line.ends_with('\n'));
    assert_eq!(
        line,
        format!(
            "{}\n",
            r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"USER:\nsecond"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}}]}}"#
        )
    );
    let bad = ChatRequest::new(
        ProviderId::Claude,
        "default",
        vec![with_images(
            "x",
            json!([{"media_type": "image/svg+xml", "data": PNG}]),
        )],
    );
    assert_eq!(
        chat_args(&bad, Path::new("/tmp/empty"), None)
            .unwrap_err()
            .message(),
        "Invalid attached image."
    );
}

// sensitive.test.ts and stateless.test.ts.

fn selected() -> ChatRequest {
    request(json!({
        "request_id": "selected", "provider": "claude", "model": "fable",
        "messages": [{"role": "user", "content": "synthetic selected evidence"}],
        "options": {"sensitive_context": true, "reasoning_effort": "high"},
    }))
}

fn status_tool() -> BrokerToolDefinition {
    BrokerToolDefinition::new(
        "status",
        "Return the Kassiber book status.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    )
    .unwrap()
}

#[test]
fn disables_persistence_for_selected_data_without_changing_the_model() {
    let built = args(&selected());
    assert!(built.iter().any(|arg| arg == "--no-session-persistence"));
    assert_eq!(after(&built, "--model"), "fable");
    assert!(!built.iter().any(|arg| arg.starts_with("--resume")));
    assert_eq!(after(&built, "--tools"), "");
    let ordinary = selected().with_options(json!({}));
    assert!(!args(&ordinary)
        .iter()
        .any(|arg| arg == "--no-session-persistence"));
}

#[test]
fn passes_no_session_persistence_only_for_a_stateless_turn() {
    let mut stateless = claude("default", json!({"stateless_session": true}));
    stateless.tools = vec![status_tool()];
    assert!(args(&stateless)
        .iter()
        .any(|arg| arg == "--no-session-persistence"));
    assert!(!args(&claude("default", json!({})))
        .iter()
        .any(|arg| arg == "--no-session-persistence"));
}

#[test]
fn sensitive_environment_drops_diagnostics_and_turns_off_telemetry() {
    let env = sensitive_claude_environment();
    let value = |name: &str| {
        env.iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.to_string_lossy().into_owned())
    };
    assert_eq!(
        value("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC").as_deref(),
        Some("1")
    );
    assert_eq!(value("DISABLE_TELEMETRY").as_deref(), Some("1"));
    assert_eq!(value("DISABLE_ERROR_REPORTING").as_deref(), Some("1"));
    assert_eq!(value("KASSIBER_AI_BROKER").as_deref(), Some("1"));
    for (name, _) in &env {
        let name = name.to_string_lossy();
        for word in ["DEBUG", "TRACE", "LOGGING", "TELEMETRY", "OTEL"] {
            assert!(
                !name.contains(word) || name == "DISABLE_TELEMETRY",
                "{name} crossed"
            );
        }
    }
}

// Turns against the fake CLI.

#[cfg(unix)]
mod fake {
    use super::*;

    fn turn(fixture: &Fixture, request: &ChatRequest) -> crate::codex::fake_cli::Turn {
        let cwd = workdir();
        capture(|events| CLAUDE.chat_with(&fixture.executable(), request, cwd.path(), None, events))
    }

    fn answer(fixture: &Fixture, state: Option<&str>, final_state: Option<&str>, extra: &[Value]) {
        let mut lines = vec![json!({"type": "system", "subtype": "init", "session_id": "s1"})];
        if let Some(state) = state {
            lines[0]["fast_mode_state"] = json!(state);
        }
        lines.extend_from_slice(extra);
        lines.push(json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "ok"}}}));
        let mut result = json!({"type": "result", "subtype": "success", "session_id": "s1"});
        if let Some(state) = final_state {
            result["fast_mode_state"] = json!(state);
        }
        lines.push(result);
        fixture.write_lines("stream.jsonl", &lines);
    }

    fn fast_events(state: Option<&str>, final_state: Option<&str>, extra: &[Value]) -> Vec<Value> {
        let fixture = Fixture::new("claude");
        answer(&fixture, state, final_state, extra);
        let turn = turn(&fixture, &claude("opus", json!({"fast_mode": true})));
        assert_eq!(turn.error, None);
        turn.events
    }

    fn unavailable(events: &[Value]) -> usize {
        events
            .iter()
            .filter(|event| event["phase"] == "fast_mode_unavailable")
            .count()
    }

    // fast-mode.test.ts: "Claude fast mode state".

    #[test]
    fn notes_a_declined_fast_mode_once_and_still_completes_the_turn() {
        let declined = fast_events(Some("off"), Some("off"), &[]);
        assert_eq!(unavailable(&declined), 1);
        assert_eq!(declined.last().unwrap()["type"], "done");
        let honoured = fast_events(Some("on"), Some("on"), &[]);
        assert_eq!(unavailable(&honoured), 0);
        assert_eq!(honoured.last().unwrap()["type"], "done");
    }

    #[test]
    fn notes_a_cli_that_never_reports_a_fast_mode_state() {
        let silent = fast_events(None, None, &[]);
        assert_eq!(unavailable(&silent), 1);
        assert_eq!(silent.last().unwrap()["type"], "done");
    }

    #[test]
    fn notes_a_rate_limit_cooldown_reported_after_a_fast_start() {
        let cooled = fast_events(Some("on"), Some("cooldown"), &[]);
        assert_eq!(unavailable(&cooled), 1);
        assert_eq!(cooled[cooled.len() - 2]["phase"], "fast_mode_unavailable");
        assert_eq!(cooled.last().unwrap()["type"], "done");
    }

    #[test]
    fn notes_a_fast_turn_claude_retried_at_standard_speed_after_running_out_of_credits() {
        // Fast mode stays "on" throughout; only the notification tells.
        let retried = fast_events(
            Some("on"),
            Some("on"),
            &[credits_notice(), credits_notice()],
        );
        assert_eq!(unavailable(&retried), 1);
        assert_eq!(retried[retried.len() - 2]["phase"], "fast_mode_unavailable");
        assert_eq!(retried.last().unwrap()["type"], "done");
        // Claude's own text never crosses.
        assert!(!serde_json::to_string(&retried)
            .unwrap()
            .contains("usage credits"));

        let mut other = credits_notice();
        other["key"] = json!("error-compacting-conversation");
        other["text"] = json!("Error compacting conversation");
        assert_eq!(
            unavailable(&fast_events(Some("on"), Some("on"), &[other])),
            0
        );
    }

    // fast-mode.test.ts: "Claude's direct fast-mode availability check".

    #[test]
    fn is_never_enabled_by_status_or_a_turn_without_fast() {
        let fixture = Fixture::new("claude");
        answer(&fixture, Some("off"), Some("off"), &[]);
        let status = CLAUDE.status_with(&fixture.executable());
        // The status check still advertises fast mode for this CLI version.
        assert!(status
            .models
            .iter()
            .any(|model| model.supports_fast_mode == Some(true)));
        let status_calls: Vec<String> = fixture
            .calls()
            .iter()
            .flat_map(|call| call.argv.clone())
            .collect();
        assert!(!status_calls.join(" ").contains("--settings"));
        assert!(!status_calls.join(" ").contains("fastMode"));

        assert_eq!(turn(&fixture, &claude("opus", json!({}))).error, None);
        assert_eq!(
            turn(&fixture, &claude("default", json!({"fast_mode": true}))).error,
            None
        );
        let calls = fixture.calls();
        let all: Vec<&str> = calls
            .iter()
            .flat_map(|call| call.argv.iter().map(String::as_str))
            .collect();
        assert!(!all.join(" ").contains(r#""fastMode":true"#));
        // Both turns ran, each stating fast mode off.
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.has(r#"{"fastMode":false}"#))
                .count(),
            2
        );
    }

    // stateless.test.ts: Claude.

    #[test]
    fn returns_no_session_cursor_from_a_stateless_turn() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[json!({"type": "result", "subtype": "success", "session_id": "claude-session"})],
        );
        let mut stateless = claude("default", json!({"stateless_session": true}));
        stateless.tools = vec![status_tool()];
        let first = turn(&fixture, &stateless);
        assert_eq!(first.error, None);
        assert_eq!(
            first.done(),
            Some(&json!({"type": "done", "finish_reason": "stop"}))
        );
        let ordinary = turn(&fixture, &claude("default", json!({})));
        assert_eq!(
            ordinary.done().unwrap()["provider_session_id"],
            "claude-session"
        );
        let chats: Vec<bool> = fixture
            .calls()
            .iter()
            .filter(|call| call.has("--print"))
            .map(|call| call.has("--no-session-persistence"))
            .collect();
        assert_eq!(chats, [true, false]);
        // The --help probe ran first, once.
        assert_eq!(fixture.calls()[0].argv, ["--help"]);
    }

    #[test]
    fn refuses_a_stateless_turn_on_a_cli_that_would_save_it_anyway() {
        let fixture = Fixture::new("claude");
        fixture.write("help", "older help without the flag\n");
        let refused = turn(
            &fixture,
            &claude("default", json!({"stateless_session": true})),
        );
        assert!(refused.error.unwrap().contains("cannot skip saving"));
        // Only the --help probe ran; the prompt was never sent.
        let calls = fixture.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].argv, ["--help"]);
        assert!(refused.events.is_empty());
    }

    // sensitive.test.ts.

    #[test]
    fn rejects_old_claude_before_launching_a_sensitive_chat() {
        let fixture = Fixture::new("claude");
        fixture.write("help", "older help without ephemeral support\n");
        let refused = turn(&fixture, &selected());
        assert!(refused
            .error
            .unwrap()
            .contains("does not support stateless"));
        assert_eq!(fixture.calls().len(), 1);
    }

    #[test]
    fn runs_a_selected_data_turn_unsaved_with_telemetry_off_and_no_cursor() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[
                json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Seen."}}}),
                json!({"type": "result", "subtype": "success", "session_id": "kept-private"}),
            ],
        );
        let done = turn(&fixture, &selected());
        assert_eq!(done.error, None);
        assert_eq!(
            done.done(),
            Some(&json!({"type": "done", "finish_reason": "stop"}))
        );
        let calls = fixture.calls();
        let chat = calls.iter().find(|call| call.has("--print")).unwrap();
        assert!(chat.has("--no-session-persistence"));
        assert_eq!(chat.env_value("DISABLE_TELEMETRY"), "1");
        assert_eq!(chat.env_value("DISABLE_ERROR_REPORTING"), "1");
        assert_eq!(
            chat.env_value("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC"),
            "1"
        );
        assert_eq!(
            chat.stdin.as_deref(),
            Some("USER:\nsynthetic selected evidence")
        );
    }

    #[test]
    fn says_nothing_of_a_failed_selected_data_turn() {
        let fixture = Fixture::new("claude");
        fixture
            .write("stderr", "upstream said: synthetic selected evidence\n")
            .write("exit", "3");
        let failed = turn(&fixture, &selected());
        assert_eq!(
            failed.error.as_deref(),
            Some("Claude sensitive-context request failed; no provider diagnostic content was retained.")
        );
    }

    // A plain turn, end to end.

    #[test]
    fn streams_a_plain_turn_and_returns_its_session() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[
                json!({"type": "system", "subtype": "init", "session_id": "s-plain"}),
                json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "thinking_delta", "thinking": "hmm"}}}),
                json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "Hello"}}}),
                json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": ""}}}),
                json!(5),
                json!({"type": "result", "subtype": "success", "session_id": "s-plain"}),
                json!({"type": "stream_event", "event": {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "after the end"}}}),
            ],
        );
        let mut request = ChatRequest::new(
            ProviderId::Claude,
            "sonnet",
            vec![
                ChatMessage::new("user", "first"),
                ChatMessage::new("assistant", "answer"),
                ChatMessage::new("user", "second"),
            ],
        );
        request.instructions = Some(json!("Book: demo."));
        let done = turn(&fixture, &request);
        assert_eq!(done.error, None);
        assert_eq!(
            done.events,
            [
                json!({"type": "status", "phase": "connecting", "message": "Starting Claude"}),
                json!({"type": "delta", "reasoning": "hmm"}),
                json!({"type": "delta", "content": "Hello"}),
                json!({"type": "done", "finish_reason": "stop", "provider_session_id": "s-plain"}),
            ]
        );
        let calls = fixture.calls();
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(
            call.stdin.as_deref(),
            Some("USER:\nfirst\n\nASSISTANT:\nanswer\n\nUSER:\nsecond")
        );
        assert_eq!(call.argv, args(&request));
        assert_eq!(call.env_value("KASSIBER_AI_BROKER"), "1");
        assert_eq!(call.env_value("DISABLE_TELEMETRY"), "<unset>");
    }

    #[test]
    fn resumes_with_only_the_latest_user_message_and_its_images() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[json!({"type": "result", "subtype": "success", "session_id": OPUS_SESSION})],
        );
        let request = request(json!({
            "provider": "claude", "model": "opus",
            "messages": [
                {"role": "user", "content": "first", "images": [{"media_type": "image/png", "data": PNG}]},
                {"role": "assistant", "content": "answer"},
                {"role": "user", "content": "second", "images": [{"media_type": "image/jpeg", "data": PNG}]},
            ],
            "options": {"provider_session_id": OPUS_SESSION},
        }));
        let done = turn(&fixture, &request);
        assert_eq!(done.error, None);
        let call = &fixture.calls()[0];
        assert!(call.has(&format!("--resume={OPUS_SESSION}")));
        assert_eq!(call.after("--input-format"), Some("stream-json"));
        let image = BrokerImage {
            media_type: "image/jpeg".into(),
            data: PNG.into(),
        };
        assert_eq!(
            call.stdin.as_deref(),
            Some(claude_stream_input("USER:\nsecond", &[image]).as_str())
        );
    }

    #[test]
    fn stops_a_turn_that_reaches_for_a_provider_native_tool() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[
                json!({"type": "stream_event", "event": {"type": "content_block_start", "content_block": {"type": "tool_use", "name": "Bash"}}}),
                json!({"type": "result", "subtype": "success", "session_id": "s1"}),
            ],
        );
        let mut request = claude("default", json!({}));
        request.tools = vec![status_tool()];
        let stopped = turn(&fixture, &request);
        assert_eq!(
            stopped.error.as_deref(),
            Some("Claude attempted to use a provider-native tool; Kassiber stopped it.")
        );
        assert_eq!(stopped.done(), None);
        // Kassiber's own advertised tool is fine.
        fixture.write_lines(
            "stream.jsonl",
            &[
                json!({"type": "stream_event", "event": {"type": "content_block_start", "content_block": {"type": "tool_use", "name": "mcp__kassiber__status"}}}),
                json!({"type": "result", "subtype": "success", "session_id": "s1"}),
            ],
        );
        assert_eq!(turn(&fixture, &request).error, None);
    }

    #[test]
    fn fails_a_turn_that_ends_without_success() {
        let fixture = Fixture::new("claude");
        fixture.write_lines(
            "stream.jsonl",
            &[json!({"type": "result", "subtype": "error_max_turns"})],
        );
        let failed = turn(&fixture, &claude("default", json!({})));
        assert_eq!(
            failed.error.as_deref(),
            Some("Claude did not complete the response.")
        );
    }

    #[test]
    fn reports_a_turn_without_a_terminal_response_by_its_redacted_stderr() {
        let fixture = Fixture::new("claude");
        fixture
            .write(
                "stderr",
                "request failed: api_key=sk-live-abcdefgh12345 via https://user:pa55word@example.com from /home/alice/x\nsecond line\n",
            )
            .write("exit", "1");
        let failed = turn(&fixture, &claude("default", json!({})));
        let message = failed.error.unwrap();
        assert!(message.starts_with(
            "Claude ended without a terminal response. request failed: api_key=[redacted]"
        ));
        for secret in ["sk-live-abcdefgh12345", "pa55word", "alice", "second line"] {
            assert!(!message.contains(secret), "{message}");
        }
        // Silent stderr: the exit code.
        let quiet = Fixture::new("claude");
        quiet.write("exit", "7");
        assert_eq!(
            turn(&quiet, &claude("default", json!({}))).error.as_deref(),
            Some("Claude ended without a terminal response. Claude exited with code 7")
        );
    }

    #[test]
    fn reports_a_missing_or_unrunnable_cli_as_node_names_it() {
        let fixture = Fixture::new("claude");
        let missing = fixture.path().join("absent");
        let failed = capture(|events| {
            CLAUDE.chat_with(
                &missing,
                &claude("default", json!({})),
                fixture.path(),
                None,
                events,
            )
        });
        assert_eq!(
            failed.error,
            Some(format!("spawn {} ENOENT", missing.display()))
        );
        let status = CLAUDE.status_with(&missing);
        assert_eq!(status.state, ProviderState::Error);
        assert!(status.message.starts_with("spawn ") && status.message.ends_with(" ENOENT"));
        assert_eq!(status.executable(), Some("absent"));
    }

    // Status.

    #[test]
    fn reports_ready_with_the_version_and_models() {
        let fixture = Fixture::new("claude");
        let status = CLAUDE.status_with(&fixture.executable());
        assert_eq!(status.state, ProviderState::Ready);
        assert_eq!(status.message, "Ready using the existing Claude login.");
        assert_eq!(status.version.as_deref(), Some("2.1.287 (Claude Code)"));
        assert_eq!(status.executable(), Some("claude"));
        assert_eq!(status.models, claude_models());
        let mut argv: Vec<Vec<String>> =
            fixture.calls().into_iter().map(|call| call.argv).collect();
        argv.sort();
        assert_eq!(argv, [vec!["--version"], vec!["auth", "status", "--json"]]);
    }

    #[test]
    fn asks_for_a_login_when_the_cli_is_signed_out() {
        let fixture = Fixture::new("claude");
        fixture.write("auth", r#"{"loggedIn":false}"#);
        let status = CLAUDE.status_with(&fixture.executable());
        assert_eq!(status.state, ProviderState::AuthenticationRequired);
        assert_eq!(status.message, "Run `claude login` outside Kassiber.");
        // Older CLIs say it through the exit code alone.
        for (auth, exit, state) in [
            ("Not logged in", "1", ProviderState::AuthenticationRequired),
            ("Logged in", "0", ProviderState::Ready),
            ("null", "0", ProviderState::Ready),
            ("[1]", "0", ProviderState::AuthenticationRequired),
        ] {
            fixture.write("auth", auth).write("auth-exit", exit);
            assert_eq!(
                CLAUDE.status_with(&fixture.executable()).state,
                state,
                "{auth}"
            );
        }
    }

    #[test]
    fn withholds_fast_mode_and_trims_the_version_of_an_older_cli() {
        let fixture = Fixture::new("claude");
        fixture.write(
            "version",
            &format!("  2.1.204 (Claude Code){}\n", " ".repeat(100)),
        );
        let status = CLAUDE.status_with(&fixture.executable());
        assert_eq!(status.version.as_deref(), Some("2.1.204 (Claude Code)"));
        assert!(!status
            .models
            .iter()
            .any(|model| model.supports_fast_mode.is_some()));
        fixture.write("version", "");
        assert_eq!(CLAUDE.status_with(&fixture.executable()).version, None);
        fixture.write("version", &"9".repeat(100));
        assert_eq!(
            CLAUDE
                .status_with(&fixture.executable())
                .version
                .unwrap()
                .len(),
            80
        );
    }
}
