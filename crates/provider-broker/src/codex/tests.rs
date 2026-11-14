//! The Codex adapter: the Codex cases of `fast-mode.test.ts`,
//! `sensitive.test.ts`, `stateless.test.ts` and `broker.test.ts`, then turns
//! against the fake app-server in `tests/fake-cli/codex`.

use super::*;
use crate::protocol::BrokerToolDefinition;
use serde_json::json;

fn status_tool() -> BrokerToolDefinition {
    BrokerToolDefinition::new(
        "status",
        "Return the Kassiber book status.",
        r#"{"type":"object","properties":{},"additionalProperties":false}"#,
    )
    .unwrap()
}

// fast-mode.test.ts: "Codex fast tier".

#[test]
fn reads_the_fast_service_tier_and_its_description_from_model_list() {
    assert_eq!(
        fast_service_tier(&json!({
            "serviceTiers": [{"id": "priority", "name": "Fast", "description": "2x speed, increased usage"}],
        })),
        Some(("priority".into(), Some("2x speed, increased usage".into())))
    );
    assert_eq!(
        fast_service_tier(&json!({"serviceTiers": [{"id": "priority", "name": "Fast"}]})),
        Some(("priority".into(), None))
    );
    // The backend owns tier ids: a Fast tier under another id keeps that id.
    assert_eq!(
        fast_service_tier(&json!({"serviceTiers": [{"id": "turbo-1", "name": "Fast"}]})),
        Some(("turbo-1".into(), None))
    );
    // Older app-servers only send the deprecated speed-tier list.
    assert_eq!(
        fast_service_tier(&json!({"additionalSpeedTiers": ["fast"]})),
        Some(("fast".into(), None))
    );
    assert_eq!(
        fast_service_tier(&json!({"serviceTiers": [], "additionalSpeedTiers": []})),
        None
    );
    assert_eq!(
        fast_service_tier(&json!({"serviceTiers": [{"id": "flex", "name": "Flex"}]})),
        None
    );
    // Names match case-insensitively; blank descriptions and empty ids are
    // left out; long descriptions are cut to 120 characters.
    assert_eq!(
        fast_service_tier(&json!({"serviceTiers": [
            {"id": "", "name": "Fast"},
            {"id": "t", "name": "FAST", "description": "  "},
        ]})),
        Some(("t".into(), None))
    );
    let long = fast_service_tier(&json!({"serviceTiers": [
        {"id": "priority", "description": format!("  {}  ", "x".repeat(200))},
    ]}))
    .unwrap();
    assert_eq!(long.1.unwrap().len(), 120);
}

#[test]
fn reads_per_turn_tier_support_from_the_app_servers_version() {
    for agent in [
        "codex_cli_rs/0.151.0 (Mac OS 15.1; arm64) Apple_Terminal",
        "kassiber/0.160.0 (Linux 6.0; x86_64) xterm",
        "kassiber/1.0.0",
        "  kassiber/0.151.1rc",
    ] {
        assert!(supports_per_turn_service_tier(Some(agent)), "{agent}");
    }
    for agent in [
        Some("codex_cli_rs/0.150.1 (Linux 6.0; x86_64)"),
        Some("codex_cli_rs/0.151.0-alpha.3 (Linux 6.0; x86_64)"),
        // Source builds report 0.0.0; anything unparseable counts as older.
        Some("codex_cli_rs/0.0.0 (Linux 6.0; x86_64)"),
        Some("0.160.0"),
        Some(""),
        None,
        Some("/0.160.0"),
        Some("co dex/0.160.0"),
        Some("codex/0.160"),
    ] {
        assert!(!supports_per_turn_service_tier(agent), "{agent:?}");
    }
}

#[test]
fn describes_terminal_codex_failures_by_category_only() {
    for (info, expected) in [
        (
            json!("unauthorized"),
            "Codex requires authentication. Run `codex login` outside Kassiber.",
        ),
        (
            json!("usageLimitExceeded"),
            "Codex reported that the account's usage limit was reached.",
        ),
        (
            json!("sessionBudgetExceeded"),
            "Codex reported that the account's usage limit was reached.",
        ),
        (
            json!("rateLimitExceeded"),
            "Codex is rate limited. Try again later.",
        ),
        (
            json!("contextWindowExceeded"),
            "Codex reached the context limit. Start a new chat.",
        ),
        (
            json!("modelNotFound"),
            "Codex could not find the selected model. Check models and choose another.",
        ),
        (
            json!("badRequest"),
            "Codex rejected the request. Check the selected model and reasoning level.",
        ),
        (
            json!("serverOverloaded"),
            "Codex reported a provider server error. Try again later.",
        ),
        (
            json!("internalServerError"),
            "Codex reported a provider server error. Try again later.",
        ),
        (
            json!("sandboxError"),
            "Codex could not start its restricted execution environment.",
        ),
        (
            json!({"httpConnectionFailed": {"httpStatusCode": 503}}),
            "Codex could not connect to the provider (HTTP 503).",
        ),
        (
            json!({"responseStreamDisconnected": {"httpStatusCode": 429.0}}),
            "Codex could not connect to the provider (HTTP 429).",
        ),
        (
            json!({"responseTooManyFailedAttempts": {"httpStatusCode": 200}}),
            "Codex lost its connection to the provider. Try again.",
        ),
        (
            json!({"responseStreamConnectionFailed": {}}),
            "Codex lost its connection to the provider. Try again.",
        ),
        (
            json!({"httpConnectionFailed": null}),
            "Codex reported a provider error.",
        ),
        (
            json!("token=private-unknown-value"),
            "Codex reported a provider error.",
        ),
    ] {
        let error = json!({"codexErrorInfo": info, "message": "private prompt", "additionalDetails": "secret"});
        assert_eq!(codex_failure_message(&error), expected, "{info}");
    }
    assert_eq!(
        codex_failure_message(&Value::Null),
        "Codex reported a provider error."
    );
    assert_eq!(
        codex_failure_message(&json!("unauthorized")),
        "Codex reported a provider error."
    );
}

#[test]
fn recognises_only_a_legacy_servers_rejection_of_the_default_tier() {
    for message in [
        "Invalid request: unknown variant `default`, expected `fast` or `flex`",
        "UNKNOWN VARIANT 'DEFAULT'",
        "unknown variant  default",
    ] {
        assert!(legacy_default_tier_rejected(message), "{message}");
    }
    for message in [
        "unknown variant default",
        "unknown variant `defaults`",
        "unknown variant `priority`",
        "invalid tier default",
    ] {
        // `\W+` needs at least one non-word character before the id; a
        // space is one.
        assert_eq!(
            legacy_default_tier_rejected(message),
            message == "unknown variant default",
            "{message}"
        );
    }
}

// sensitive.test.ts.

#[test]
fn installs_restricted_startup_settings_before_opening_any_thread() {
    let config = serde_json::from_str::<Value>(&sensitive_codex_config().to_json()).unwrap();
    assert_eq!(
        config["permissions"][SENSITIVE_CODEX_PROFILE],
        json!({
            "filesystem": {":root": "deny", ":minimal": "read", ":workspace_roots": "read"},
            "network": {"enabled": false},
        })
    );
    let features = config["features"].as_object().unwrap();
    assert_eq!(features.len(), SENSITIVE_DISABLED_FEATURES.len());
    assert!(features.values().all(|value| *value == json!(false)));
    for feature in ["shell_tool", "code_mode_host", "multi_agent", "browser_use"] {
        assert_eq!(features[feature], json!(false));
    }
    let args = sensitive_codex_args();
    assert!(args.contains(&r#"history={"persistence"="none"}"#.to_owned()));
    assert_eq!(config["project_doc_max_bytes"], json!(0));
    assert_eq!(config["tools"]["view_image"], json!(false));
    // `-c key=<toml>` pairs, in codex.ts's order and spelling.
    assert_eq!(args.len(), 28);
    assert_eq!(
        args[..6],
        [
            "-c",
            "project_doc_max_bytes=0",
            "-c",
            r#"default_permissions="kassiber-selected-context""#,
            "-c",
            r#"permissions={"kassiber-selected-context"={"filesystem"={":root"="deny",":minimal"="read",":workspace_roots"="read"},"network"={"enabled"=false}}}"#,
        ]
    );
    assert_eq!(
        args[17],
        r#"shell_environment_policy={"inherit"="none","set"={},"experimental_use_profile"=false}"#
    );
    assert!(args[27].starts_with(r#"features={"shell_tool"=false,"unified_exec"=false,"#));
    assert!(args[27].ends_with(r#""chronicle"=false,"goals"=false}"#));
}

// broker.test.ts.

#[test]
fn treats_every_item_but_text_reasoning_and_kassiber_tools_as_native() {
    assert!(CODEX_NON_TOOL_ITEM_TYPES.contains(&"userMessage"));
    assert!(!CODEX_NON_TOOL_ITEM_TYPES.contains(&"commandExecution"));
    assert!(!CODEX_NON_TOOL_ITEM_TYPES.contains(&"fileChange"));
}

#[test]
fn fingerprints_the_tool_catalog_as_the_typescript_broker_does() {
    // sha256('[]') and sha256 of the tool as JSON.stringify writes it.
    assert_eq!(tool_fingerprint(&[]), "4f53cda18c2baa0c");
    let spaced = BrokerToolDefinition::new(
        "status",
        "Return the Kassiber book status.",
        r#"{ "type": "object", "properties": {}, "additionalProperties": false }"#,
    )
    .unwrap();
    assert_eq!(
        tool_fingerprint(&[spaced]),
        tool_fingerprint(&[status_tool()])
    );
    let text = r#"[{"name":"status","description":"Return the Kassiber book status.","parameters":{"type":"object","properties":{},"additionalProperties":false}}]"#;
    let expected: String = Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(tool_fingerprint(&[status_tool()]), expected);
}

// Turns against the fake app-server.

#[cfg(unix)]
mod fake {
    use super::super::fake_cli::{capture, chat, request, workdir, Fixture, Turn};
    use super::*;
    use std::sync::Arc;

    const CATALOG: &str = r#"{"data":[{"id":"gpt-5.4","model":"gpt-5.4","isDefault":true,"serviceTiers":[{"id":"priority","name":"Fast","description":"2x speed, increased usage"}]},{"id":"gpt-5.4-mini","model":"gpt-5.4-mini","serviceTiers":[]}]}"#;

    /// The TypeScript fixture's knobs.
    #[derive(Default)]
    struct Setup<'a> {
        model: Option<&'a str>,
        options: Option<Value>,
        catalog: Option<Value>,
        thread_model: Option<&'a str>,
        saved_tier: Option<&'a str>,
        stall_catalog: bool,
        legacy_server: bool,
        /// `Some(None)` leaves `userAgent` out.
        version: Option<Option<&'a str>>,
        reject_turn_start: Option<&'a str>,
        notifications: Vec<Value>,
        completion: Option<Value>,
    }

    struct Outcome {
        turn: Turn,
        wire: Vec<Value>,
        fixture: Fixture,
    }

    impl Outcome {
        fn methods(&self) -> Vec<String> {
            self.wire
                .iter()
                .filter_map(|message| message["method"].as_str().map(str::to_owned))
                .collect()
        }

        fn turn_starts(&self) -> Vec<&Value> {
            self.wire
                .iter()
                .filter(|message| message["method"] == "turn/start")
                .map(|message| &message["params"])
                .collect()
        }

        fn turn_start(&self) -> &Value {
            self.turn_starts().last().copied().unwrap()
        }

        /// Every tier `turn/start` was sent with, retries included.
        fn tiers(&self) -> Vec<Value> {
            self.turn_starts()
                .iter()
                .map(|params| {
                    params
                        .get("serviceTierForTurn")
                        .or_else(|| params.get("serviceTier"))
                        .cloned()
                        .unwrap_or(Value::Null)
                })
                .collect()
        }

        fn unavailable(&self) -> usize {
            self.turn.phases("fast_mode_unavailable")
        }

        fn session_id(&self) -> Option<&str> {
            self.turn.done()?.get("provider_session_id")?.as_str()
        }

        fn thread_start(&self) -> &Value {
            &self
                .wire
                .iter()
                .find(|message| message["method"] == "thread/start")
                .unwrap()["params"]
        }
    }

    fn fixture(setup: &Setup<'_>) -> Fixture {
        let fixture = Fixture::new("codex");
        fixture.write(
            "thread.json",
            &json!({
                "thread": {"id": "fixture"},
                "model": setup.thread_model.unwrap_or("gpt-5.4"),
                // A user's saved `service_tier = "fast"` shows up here.
                "serviceTier": setup.saved_tier,
            })
            .to_string(),
        );
        fixture.write(
            "models.json",
            &setup
                .catalog
                .as_ref()
                .map_or(CATALOG.to_owned(), |catalog| {
                    json!({"data": catalog}).to_string()
                }),
        );
        fixture.write(
            "initialize.json",
            &match setup.version {
                Some(None) => json!({}),
                version => json!({"userAgent": format!(
                    "kassiber/{} (Linux 6.0; x86_64) xterm",
                    version.flatten().unwrap_or("0.160.0")
                )}),
            }
            .to_string(),
        );
        if setup.stall_catalog {
            fixture.write("stall-catalog", "");
        }
        if setup.legacy_server {
            fixture.write("legacy", "");
        }
        if let Some(message) = setup.reject_turn_start {
            fixture.write("reject-turn", message);
        }
        fixture.write_lines("turn.jsonl", &setup.notifications);
        fixture.write_lines(
            "completion.json",
            &[json!({"method": "turn/completed", "params": {"turn": setup.completion.clone().unwrap_or(json!({"status": "completed"}))}})],
        );
        fixture
    }

    fn run(codex: &Codex, fixture: Fixture, request: &ChatRequest) -> Outcome {
        let cwd = workdir();
        let turn = capture(|events| {
            codex.chat_with(&fixture.executable(), request, cwd.path(), None, events)
        });
        Outcome {
            turn,
            wire: fixture.wire(),
            fixture,
        }
    }

    fn codex_turn(setup: Setup<'_>) -> Outcome {
        let request = chat(
            ProviderId::Codex,
            setup.model.unwrap_or("gpt-5.4"),
            setup.options.clone().unwrap_or(json!({})),
        );
        run(&CODEX, fixture(&setup), &request)
    }

    // fast-mode.test.ts.

    #[test]
    fn lets_codex_recover_from_retryable_provider_errors() {
        let turn = codex_turn(Setup {
            notifications: vec![
                json!({"method": "error", "params": {"threadId": "fixture", "willRetry": true,
                    "error": {"codexErrorInfo": "internalServerError", "message": "private provider detail"}}}),
                json!({"method": "item/agentMessage/delta", "params": {"itemId": "reply", "delta": "Recovered"}}),
            ],
            ..Setup::default()
        });
        assert_eq!(turn.turn.error, None);
        assert!(turn.turn.events.contains(
            &json!({"type": "status", "phase": "retrying", "message": "Codex is retrying the request."})
        ));
        assert!(turn
            .turn
            .events
            .contains(&json!({"type": "delta", "content": "Recovered"})));
        assert!(turn.session_id().is_some());
        assert!(!serde_json::to_string(&turn.turn.events)
            .unwrap()
            .contains("private provider detail"));
    }

    #[test]
    fn describes_terminal_codex_failures_safely() {
        for (info, expected) in [
            (
                json!("unauthorized"),
                "Codex requires authentication. Run `codex login` outside Kassiber.",
            ),
            (
                json!("usageLimitExceeded"),
                "Codex reported that the account's usage limit was reached.",
            ),
            (
                json!("badRequest"),
                "Codex rejected the request. Check the selected model and reasoning level.",
            ),
            (
                json!({"httpConnectionFailed": {"httpStatusCode": 503}}),
                "Codex could not connect to the provider (HTTP 503).",
            ),
            (
                json!("token=private-unknown-value"),
                "Codex reported a provider error.",
            ),
        ] {
            let error = json!({"codexErrorInfo": info, "message": "private prompt and credential", "additionalDetails": "secret"});
            let turn = codex_turn(Setup {
                notifications: vec![
                    json!({"method": "error", "params": {"threadId": "fixture", "willRetry": false, "error": error}}),
                ],
                completion: Some(json!({"status": "failed", "error": error})),
                ..Setup::default()
            });
            assert_eq!(turn.turn.error.as_deref(), Some(expected), "{info}");
            assert_eq!(turn.session_id(), None);
        }
    }

    #[test]
    fn describes_a_failed_completed_turn_even_without_an_error_notification() {
        let turn = codex_turn(Setup {
            completion: Some(
                json!({"status": "failed", "error": {"codexErrorInfo": "contextWindowExceeded", "message": "private prompt"}}),
            ),
            ..Setup::default()
        });
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("Codex reached the context limit. Start a new chat.")
        );
        assert_eq!(turn.session_id(), None);
    }

    #[test]
    fn overrides_the_tier_for_one_turn_only_never_the_threads() {
        let fast = codex_turn(Setup {
            options: Some(json!({"fast_mode": true})),
            ..Setup::default()
        });
        assert_eq!(
            fast.turn_start()["serviceTierForTurn"],
            CODEX_FAST_SERVICE_TIER
        );
        // The thread's saved tier is left alone, so Codex itself never
        // inherits fast.
        assert!(fast.turn_start().get("serviceTier").is_none());
        assert_eq!(fast.unavailable(), 0);
        for options in [json!({}), json!({"fast_mode": false})] {
            let standard = codex_turn(Setup {
                options: Some(options),
                saved_tier: Some("priority"),
                ..Setup::default()
            });
            assert_eq!(
                standard.turn_start()["serviceTierForTurn"],
                CODEX_STANDARD_SERVICE_TIER
            );
            assert!(standard.turn_start().get("serviceTier").is_none());
        }
    }

    #[test]
    fn declines_fast_on_codex_versions_without_the_per_turn_field() {
        for version in [Some("0.150.1"), Some("0.151.0-alpha.3"), None] {
            let declined = codex_turn(Setup {
                version: Some(version),
                options: Some(json!({"fast_mode": true})),
                ..Setup::default()
            });
            assert_eq!(declined.turn.error, None);
            assert_eq!(declined.unavailable(), 1);
            // No fast tier anywhere: not on the thread, not for the turn,
            // and no catalog lookup to choose one.
            assert_eq!(declined.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
            assert!(declined.turn_start().get("serviceTierForTurn").is_none());
            assert!(!declined.methods().contains(&"model/list".to_owned()));

            let standard = codex_turn(Setup {
                version: Some(version),
                saved_tier: Some("priority"),
                ..Setup::default()
            });
            assert_eq!(standard.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
            assert!(standard.turn_start().get("serviceTierForTurn").is_none());
            assert_eq!(standard.unavailable(), 0);
        }
    }

    #[test]
    fn offers_fast_mode_only_on_codex_versions_that_can_honour_it() {
        let fast_models = |version: Option<&str>| -> Vec<String> {
            let fixture = fixture(&Setup {
                version: Some(version),
                ..Setup::default()
            });
            let cwd = workdir();
            CODEX
                .status_with(&fixture.executable(), cwd.path())
                .models
                .into_iter()
                .filter(|model| model.supports_fast_mode == Some(true))
                .map(|model| model.id)
                .collect()
        };
        assert_eq!(fast_models(Some("0.160.0")), ["gpt-5.4"]);
        assert!(fast_models(Some("0.150.1")).is_empty());
        assert!(fast_models(None).is_empty());
    }

    #[test]
    fn sends_the_tier_id_the_catalog_advertises_as_fast() {
        let renamed = codex_turn(Setup {
            options: Some(json!({"fast_mode": true})),
            catalog: Some(
                json!([{"id": "gpt-5.4", "serviceTiers": [{"id": "turbo-1", "name": "Fast"}]}]),
            ),
            ..Setup::default()
        });
        assert_eq!(renamed.tiers(), [json!("turbo-1")]);
        assert_eq!(renamed.unavailable(), 0);
        // Only the deprecated list: Codex still takes its `fast` literal.
        let legacy = codex_turn(Setup {
            options: Some(json!({"fast_mode": true})),
            catalog: Some(json!([{"id": "gpt-5.4", "additionalSpeedTiers": ["fast"]}])),
            ..Setup::default()
        });
        assert_eq!(legacy.tiers(), [json!("fast")]);
        assert_eq!(legacy.unavailable(), 0);
    }

    #[test]
    fn keeps_ordinary_chat_working_on_app_servers_without_the_default_tier_id() {
        // The rejected "default" is retried as null, which that version
        // reads as standard speed; a fast request is declined first, with no
        // lookup.
        for (options, notes) in [(json!({}), 0), (json!({"fast_mode": true}), 1)] {
            let turn = codex_turn(Setup {
                version: Some(None),
                legacy_server: true,
                saved_tier: Some("fast"),
                options: Some(options),
                ..Setup::default()
            });
            assert_eq!(turn.turn.error, None);
            assert_eq!(
                turn.tiers(),
                [json!(CODEX_STANDARD_SERVICE_TIER), Value::Null]
            );
            assert!(!turn.methods().contains(&"model/list".to_owned()));
            assert_eq!(turn.unavailable(), notes);
        }
    }

    #[test]
    fn never_retries_a_rejected_per_turn_tier() {
        let turn = codex_turn(Setup {
            options: Some(json!({"fast_mode": true})),
            reject_turn_start: Some("Invalid request: unknown variant `default`"),
            ..Setup::default()
        });
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("Invalid request: unknown variant `default`")
        );
        assert_eq!(turn.tiers(), [json!(CODEX_FAST_SERVICE_TIER)]);
    }

    #[test]
    fn resolves_kassibers_default_model_through_the_thread_before_choosing_fast() {
        let supported = codex_turn(Setup {
            model: Some("default"),
            options: Some(json!({"fast_mode": true})),
            ..Setup::default()
        });
        assert_eq!(supported.tiers(), [json!(CODEX_FAST_SERVICE_TIER)]);
        assert!(supported.turn_start().get("model").is_none());
        let unsupported = codex_turn(Setup {
            model: Some("default"),
            thread_model: Some("gpt-5.4-mini"),
            options: Some(json!({"fast_mode": true})),
            ..Setup::default()
        });
        assert_eq!(unsupported.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
        assert_eq!(unsupported.unavailable(), 1);
    }

    #[test]
    fn keeps_the_standard_tier_with_a_hint_for_unsupported_or_unknown_models() {
        for model in ["gpt-5.4-mini", "gpt-unknown"] {
            let turn = codex_turn(Setup {
                model: Some(model),
                options: Some(json!({"fast_mode": true})),
                ..Setup::default()
            });
            assert_eq!(turn.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
            assert!(turn.turn_start().get("serviceTier").is_none());
            assert_eq!(turn.unavailable(), 1);
        }
    }

    #[test]
    fn answers_at_standard_speed_when_the_catalog_lookup_stalls() {
        let quick = Codex {
            fast_tier_lookup: Duration::from_millis(100),
        };
        let setup = Setup {
            stall_catalog: true,
            ..Setup::default()
        };
        let turn = run(
            &quick,
            fixture(&setup),
            &chat(ProviderId::Codex, "gpt-5.4", json!({"fast_mode": true})),
        );
        assert!(turn.methods().contains(&"model/list".to_owned()));
        assert_eq!(turn.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
        assert_eq!(turn.unavailable(), 1);
    }

    #[test]
    fn pins_the_standard_tier_when_fast_is_off_overriding_a_saved_fast_config() {
        for options in [json!({}), json!({"fast_mode": false})] {
            let turn = codex_turn(Setup {
                options: Some(options),
                saved_tier: Some("fast"),
                ..Setup::default()
            });
            assert_eq!(turn.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
            assert_eq!(turn.unavailable(), 0);
            // Fast off needs no catalog lookup at all.
            assert!(!turn.methods().contains(&"model/list".to_owned()));
        }
    }

    #[test]
    fn returns_a_resumed_fast_thread_to_the_standard_tier_once_fast_is_off() {
        let first = codex_turn(Setup {
            options: Some(json!({"fast_mode": true})),
            ..Setup::default()
        });
        assert_eq!(first.tiers(), [json!(CODEX_FAST_SERVICE_TIER)]);
        // A thread whose saved tier is priority (set in Codex itself).
        let next = codex_turn(Setup {
            options: Some(json!({"provider_session_id": first.session_id().unwrap()})),
            saved_tier: Some(CODEX_FAST_SERVICE_TIER),
            ..Setup::default()
        });
        assert!(next.methods().contains(&"thread/resume".to_owned()));
        assert_eq!(next.tiers(), [json!(CODEX_STANDARD_SERVICE_TIER)]);
    }

    #[test]
    fn rejects_an_invalid_fast_mode_option_after_opening_the_thread() {
        let turn = codex_turn(Setup {
            options: Some(json!({"fast_mode": "true"})),
            ..Setup::default()
        });
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("Invalid fast mode option.")
        );
        assert!(!turn.methods().contains(&"turn/start".to_owned()));
    }

    // sensitive.test.ts.

    fn selected() -> ChatRequest {
        request(json!({
            "request_id": "selected", "provider": "codex", "model": "fable",
            "messages": [{"role": "user", "content": "synthetic selected evidence"}],
            "options": {"sensitive_context": true, "reasoning_effort": "high"},
        }))
    }

    #[test]
    fn rejects_an_unverified_codex_thread_before_sending_any_selected_prompt() {
        let fixture = Fixture::new("codex");
        fixture.write(
            "thread.json",
            r#"{"thread":{"id":"fixture","ephemeral":false},"activePermissionProfile":{"id":"kassiber-selected-context"},"instructionSources":[]}"#,
        );
        let turn = run(&CODEX, fixture, &selected());
        assert!(turn
            .turn
            .error
            .unwrap()
            .contains("sensitive-context request failed"));
        let wire = turn.fixture.wire_text();
        assert!(!wire.contains("turn/start"));
        assert!(!wire.contains("synthetic selected evidence"));
        assert!(!wire.contains("thread/resume"));
    }

    #[test]
    fn runs_a_verified_selected_data_turn_restricted_unsaved_and_without_a_cursor() {
        for (thread, verified) in [
            (
                r#"{"thread":{"id":"t","ephemeral":true},"activePermissionProfile":{"id":"kassiber-selected-context"},"instructionSources":[]}"#,
                true,
            ),
            (
                r#"{"thread":{"id":"t","ephemeral":true},"activePermissionProfile":{"id":"other"},"instructionSources":[]}"#,
                false,
            ),
            (
                r#"{"thread":{"id":"t","ephemeral":true},"activePermissionProfile":{"id":"kassiber-selected-context"},"instructionSources":["AGENTS.md"]}"#,
                false,
            ),
            (
                r#"{"thread":{"id":"t","ephemeral":true},"activePermissionProfile":{"id":"kassiber-selected-context"}}"#,
                false,
            ),
        ] {
            let fixture = Fixture::new("codex");
            fixture.write("thread.json", thread);
            fixture.write_lines(
                "turn.jsonl",
                &[json!({"method": "item/agentMessage/delta", "params": {"itemId": "a", "delta": "Seen."}})],
            );
            let turn = run(&CODEX, fixture, &selected());
            if !verified {
                assert_eq!(
                    turn.turn.error.as_deref(),
                    Some("Codex sensitive-context request failed; no provider diagnostic content was retained."),
                    "{thread}"
                );
                assert!(!turn
                    .fixture
                    .wire_text()
                    .contains("synthetic selected evidence"));
                continue;
            }
            assert_eq!(turn.turn.error, None);
            assert_eq!(
                turn.turn.done(),
                Some(&json!({"type": "done", "finish_reason": "stop"}))
            );
            let call = &turn.fixture.calls()[0];
            let mut expected = vec!["app-server".to_owned(), "--stdio".to_owned()];
            expected.extend(sensitive_codex_args());
            assert_eq!(call.argv, expected);
            assert_eq!(call.env_value("RUST_LOG"), "off");
            let start = turn.thread_start();
            assert_eq!(start["permissions"], SENSITIVE_CODEX_PROFILE);
            assert!(start.get("sandbox").is_none());
            assert_eq!(start["ephemeral"], true);
            assert_eq!(start["environments"], json!([]));
            assert_eq!(start["baseInstructions"], CHAT_ONLY_INSTRUCTIONS);
            assert_eq!(start["developerInstructions"], CHAT_ONLY_INSTRUCTIONS);
            assert_eq!(start["config"]["history"], json!({"persistence": "none"}));
            assert_eq!(
                start["config"]["default_permissions"],
                SENSITIVE_CODEX_PROFILE
            );
            assert_eq!(start["dynamicTools"], json!([]));
            let params = turn.turn_start();
            assert_eq!(params["permissions"], SENSITIVE_CODEX_PROFILE);
            assert!(params.get("sandboxPolicy").is_none());
            assert_eq!(params["effort"], "high");
            assert_eq!(
                params["input"][0]["text"],
                format!("{CHAT_ONLY_INSTRUCTIONS}\n\nUSER:\nsynthetic selected evidence")
            );
        }
    }

    #[test]
    fn stops_a_selected_data_turn_that_calls_even_a_kassiber_tool() {
        let fixture = Fixture::new("codex");
        fixture.write(
            "thread.json",
            r#"{"thread":{"id":"t","ephemeral":true},"activePermissionProfile":{"id":"kassiber-selected-context"},"instructionSources":[]}"#,
        );
        fixture.write_lines(
            "turn.jsonl",
            &[json!({"method": "item/started", "params": {"item": {"type": "dynamicToolCall", "id": "x"}}})],
        );
        let turn = run(&CODEX, fixture, &selected());
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("Codex sensitive-context request failed; no provider diagnostic content was retained.")
        );
    }

    // stateless.test.ts: Codex.

    fn incognito(options: Value) -> ChatRequest {
        let mut request = ChatRequest::new(
            ProviderId::Codex,
            "default",
            vec![crate::protocol::ChatMessage::new(
                "user",
                "synthetic incognito prompt",
            )],
        )
        .with_options(options);
        request.tools = vec![status_tool()];
        request
    }

    fn stateless_turn(options: Value, honours_ephemeral: bool) -> Outcome {
        let fixture = Fixture::new("codex");
        if !honours_ephemeral {
            fixture.write(
                "thread.json",
                r#"{"thread":{"id":"thread-1","ephemeral":false},"model":"gpt-5.4"}"#,
            );
        }
        run(&CODEX, fixture, &incognito(options))
    }

    #[test]
    fn opens_an_ephemeral_thread_that_keeps_kassibers_tools() {
        let turn = stateless_turn(json!({"stateless_session": true}), true);
        assert_eq!(turn.turn.error, None);
        let start = turn.thread_start();
        assert_eq!(start["ephemeral"], true);
        let names: Vec<&str> = start["dynamicTools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["status"]);
        assert_eq!(start["config"]["history"], json!({"persistence": "none"}));
        // Not the selected-data profile: ordinary read-only sandbox and
        // instructions.
        assert_eq!(start["sandbox"], "read-only");
        assert!(!turn.methods().contains(&"thread/resume".to_owned()));
        assert_eq!(
            turn.turn.done(),
            Some(&json!({"type": "done", "finish_reason": "stop"}))
        );
    }

    #[test]
    fn keeps_an_ordinary_thread_and_its_cursor_otherwise() {
        let turn = stateless_turn(json!({}), true);
        let start = turn.thread_start();
        assert_eq!(start["ephemeral"], false);
        assert!(start["config"].get("history").is_none());
        assert!(turn.session_id().unwrap().ends_with(":thread-1"));
    }

    #[test]
    fn sends_nothing_when_codex_does_not_open_an_unsaved_thread() {
        let turn = stateless_turn(json!({"stateless_session": true}), false);
        assert!(turn
            .turn
            .error
            .as_deref()
            .unwrap()
            .contains("could not open an unsaved thread"));
        assert!(!turn.methods().contains(&"turn/start".to_owned()));
        assert_eq!(turn.turn.done(), None);
    }

    // A plain turn, the wire it speaks, and its failures.

    #[test]
    fn speaks_the_app_server_protocol_in_codex_ts_order() {
        let fixture = Fixture::new("codex");
        fixture.write_lines(
            "turn.jsonl",
            &[
                json!({"method": "item/started", "params": {"item": {"type": "reasoning", "id": "r"}}}),
                json!({"method": "item/agentMessage/delta", "params": {"itemId": "a", "delta": "Hel"}}),
                json!({"method": "item/agentMessage/delta", "params": {"itemId": "a", "delta": ""}}),
                json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "a", "text": "Hello."}}}),
                json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "b", "text": "Bye."}}}),
                json!({"method": "item/completed", "params": {"item": {"type": "agentMessage", "id": "a", "text": "Other"}}}),
                // Answered before the turn completes (see after-tool.jsonl).
                json!({"id": "tool-1", "method": "item/commandExecution/requestApproval", "params": {}}),
            ],
        );
        fixture.write_lines("after-tool.jsonl", &[]);
        let mut request = ChatRequest::new(
            ProviderId::Codex,
            "gpt-5.4",
            vec![crate::protocol::ChatMessage::new("user", "hi")],
        )
        .with_options(json!({"reasoning_effort": "low"}));
        request.instructions = Some(json!("Book: demo."));
        let turn = run(&CODEX, fixture, &request);
        assert_eq!(turn.turn.error, None);
        let prefix = format!("kdt-{}:", tool_fingerprint(&[]));
        assert_eq!(
            turn.turn.events,
            [
                json!({"type": "status", "phase": "connecting", "message": "Starting Codex app-server"}),
                json!({"type": "delta", "content": "Hel"}),
                json!({"type": "delta", "content": "lo."}),
                json!({"type": "delta", "content": "Bye."}),
                json!({"type": "done", "finish_reason": "stop", "provider_session_id": format!("{prefix}thread-1")}),
            ]
        );
        let lines: Vec<&str> = turn.fixture.wire_text().leak().lines().collect();
        let cwd = turn.wire[2]["params"]["cwd"].as_str().unwrap().to_owned();
        let instructions = serde_json::to_string(&system_instructions(&request)).unwrap();
        assert_eq!(
            lines[0],
            r#"{"id":1,"method":"initialize","params":{"clientInfo":{"name":"kassiber","title":"Kassiber","version":"0.22.55"},"capabilities":{"experimentalApi":true}}}"#
        );
        assert_eq!(lines[1], r#"{"method":"initialized"}"#);
        assert_eq!(
            lines[2],
            format!(
                r#"{{"id":2,"method":"thread/start","params":{{"cwd":{},"model":"gpt-5.4","approvalPolicy":"untrusted","approvalsReviewer":"user","sandbox":"read-only","ephemeral":false,"baseInstructions":{instructions},"developerInstructions":{instructions},"config":{{"web_search":"disabled","mcp_servers":{{}},"multi_agent_mode":"explicitRequestOnly"}},"dynamicTools":[]}}}}"#,
                serde_json::to_string(&cwd).unwrap()
            )
        );
        assert_eq!(
            lines[3],
            r#"{"id":3,"method":"turn/start","params":{"threadId":"thread-1","input":[{"type":"text","text":"USER:\nhi"}],"approvalPolicy":"untrusted","approvalsReviewer":"user","sandboxPolicy":{"type":"readOnly","networkAccess":false},"model":"gpt-5.4","effort":"low","serviceTierForTurn":"default"}}"#
        );
        // The approval request was refused as a provider-native tool.
        assert_eq!(
            lines[4],
            r#"{"id":"tool-1","error":{"code":-32001,"message":"Provider-native tools are disabled by Kassiber."}}"#
        );
        let call = &turn.fixture.calls()[0];
        assert_eq!(call.argv, ["app-server", "--stdio"]);
        assert_eq!(call.cwd, Path::new(&cwd));
        assert_eq!(call.env_value("RUST_LOG"), "<unset>");
        assert_eq!(call.env_value("KASSIBER_AI_BROKER"), "1");
    }

    #[test]
    fn stops_a_turn_that_starts_a_provider_native_item() {
        for item in [
            json!({"type": "commandExecution"}),
            json!({"type": "fileChange"}),
            json!({}),
            json!({"type": 5}),
            json!([]),
        ] {
            let fixture = Fixture::new("codex");
            fixture.write_lines(
                "turn.jsonl",
                &[json!({"method": "item/started", "params": {"item": item}})],
            );
            let turn = run(
                &CODEX,
                fixture,
                &chat(ProviderId::Codex, "gpt-5.4", json!({})),
            );
            assert_eq!(
                turn.turn.error.as_deref(),
                Some("Codex attempted to use a provider-native tool; Kassiber stopped it."),
                "{item}"
            );
        }
    }

    #[test]
    fn sends_attached_images_as_inline_data_urls() {
        let request = request(json!({
            "provider": "codex", "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "look", "images": [{"media_type": "image/png", "data": "iVBORw0KGgo="}]}],
        }));
        let turn = run(&CODEX, Fixture::new("codex"), &request);
        assert_eq!(turn.turn.error, None);
        assert_eq!(
            turn.turn_start()["input"],
            json!([
                {"type": "text", "text": "USER:\nlook"},
                {"type": "image", "url": "data:image/png;base64,iVBORw0KGgo="},
            ])
        );
    }

    #[test]
    fn resumes_only_a_cursor_issued_for_the_same_tools() {
        let mut with_tools = incognito(json!({}));
        let fingerprint = tool_fingerprint(&with_tools.tools);
        for (cursor, resumed) in [
            (format!("kdt-{fingerprint}:thread-7"), Some("thread-7")),
            ("kdt-0000000000000000:thread-7".to_owned(), None),
            ("thread-7".to_owned(), None),
            (format!("kdt-{fingerprint}:"), None),
        ] {
            with_tools.options = Some(json!({"provider_session_id": cursor}));
            let turn = run(&CODEX, Fixture::new("codex"), &with_tools);
            assert_eq!(turn.turn.error, None);
            let resume = turn
                .wire
                .iter()
                .find(|message| message["method"] == "thread/resume");
            assert_eq!(
                resume.map(|message| message["params"]["threadId"].as_str().unwrap()),
                resumed,
                "{cursor}"
            );
        }
        // Without tools a bare cursor (an older Kassiber's) is resumed.
        let bare = chat(
            ProviderId::Codex,
            "gpt-5.4",
            json!({"provider_session_id": "thread-7"}),
        );
        let turn = run(&CODEX, Fixture::new("codex"), &bare);
        assert!(turn.methods().contains(&"thread/resume".to_owned()));
        // A resumed thread replays only the newest user message.
        let resumed = request(json!({
            "provider": "codex", "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "first"}, {"role": "assistant", "content": "a"}, {"role": "user", "content": "second"}],
            "options": {"provider_session_id": "thread-7"},
        }));
        let turn = run(&CODEX, Fixture::new("codex"), &resumed);
        assert_eq!(turn.turn_start()["input"][0]["text"], "USER:\nsecond");
        assert_eq!(turn.turn_start()["threadId"], "thread-1");
    }

    #[test]
    fn starts_a_fresh_thread_when_the_resumed_one_is_gone() {
        let resumed = request(json!({
            "provider": "codex", "model": "gpt-5.4",
            "messages": [{"role": "user", "content": "first"}, {"role": "user", "content": "second"}],
            "options": {"provider_session_id": "thread-7"},
        }));
        let fixture = Fixture::new("codex");
        fixture.write("resume-error", "thread not found: thread-7");
        let turn = run(&CODEX, fixture, &resumed);
        assert_eq!(turn.turn.error, None);
        assert_eq!(turn.methods()[2..4], ["thread/resume", "thread/start"]);
        // The whole transcript, since the thread is new.
        assert_eq!(
            turn.turn_start()["input"][0]["text"],
            "USER:\nfirst\n\nUSER:\nsecond"
        );
        let fixture = Fixture::new("codex");
        fixture.write("resume-error", "permission denied for token=abc");
        let turn = run(&CODEX, fixture, &resumed);
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("permission denied for token=abc")
        );
        assert!(!turn.methods().contains(&"thread/start".to_owned()));
    }

    #[test]
    fn fails_with_the_app_servers_redacted_stderr_when_it_exits() {
        let fixture = Fixture::new("codex");
        fixture
            .write(
                "stderr",
                "fatal: upstream rejected api_key=sk-live-abcdefgh12345 at /home/alice/.codex\nmore detail\n",
            )
            .write("exit", "1");
        let turn = run(
            &CODEX,
            fixture,
            &chat(ProviderId::Codex, "gpt-5.4", json!({})),
        );
        let message = turn.turn.error.unwrap();
        assert_eq!(
            message,
            "fatal: upstream rejected api_key=[redacted] at ~/.codex"
        );
        let quiet = Fixture::new("codex");
        quiet.write("stderr", "").write("exit", "4");
        let turn = run(
            &CODEX,
            quiet,
            &chat(ProviderId::Codex, "gpt-5.4", json!({})),
        );
        assert_eq!(
            turn.turn.error.as_deref(),
            Some("Codex app-server exited with code 4.")
        );
    }

    #[test]
    fn answers_a_dynamic_tool_call_through_the_bridge() {
        let fixture = Fixture::new("codex");
        // Raw text: the arguments' key order must survive to the event.
        fixture.write(
            "turn.jsonl",
            concat!(
                r#"{"method":"item/started","params":{"item":{"type":"dynamicToolCall","id":"d"}}}"#,
                "\n",
                r#"{"id":"tool-1","method":"item/tool/call","params":{"tool":"status","callId":"call-1","arguments":{"z":1,"a":[true]}}}"#,
                "\n",
            ),
        );
        fixture.write_lines(
            "after-tool.jsonl",
            &[json!({"method": "item/agentMessage/delta", "params": {"itemId": "a", "delta": "Book is fine."}})],
        );
        let request = incognito(json!({}));
        let (events, captured) = EventSink::capture();
        let bridge = Arc::new(ToolBridge::start(&request.tools, events.clone()).unwrap());
        let answering = Arc::clone(&bridge);
        let watcher = captured.clone();
        let daemon = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(20);
            while Instant::now() < deadline {
                if watcher
                    .events()
                    .iter()
                    .any(|event| event["type"] == "tool_call")
                {
                    answering.resolve(&[json!({"call_id": "call-1", "output": "{\"ok\":true}"})]);
                    return;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        });
        let cwd = workdir();
        let outcome = CODEX.chat_with(
            &fixture.executable(),
            &request,
            cwd.path(),
            Some(&bridge),
            &events,
        );
        daemon.join().unwrap();
        assert_eq!(outcome, Ok(()));
        let events = captured.events();
        assert!(events.contains(&json!({"type": "tool_call", "call_id": "call-1", "name": "status", "arguments": {"z": 1, "a": [true]}})));
        assert!(captured
            .text()
            .contains(r#""arguments":{"z":1,"a":[true]}"#));
        assert!(events.contains(&json!({"type": "delta", "content": "Book is fine."})));
        assert_eq!(events.last().unwrap()["type"], "done");
        let reply = fixture
            .wire_text()
            .lines()
            .find(|line| line.starts_with(r#"{"id":"tool-1""#))
            .unwrap()
            .to_owned();
        assert_eq!(
            reply,
            r#"{"id":"tool-1","result":{"contentItems":[{"type":"inputText","text":"{\"ok\":true}"}],"success":true}}"#
        );
    }

    #[test]
    fn denies_a_tool_call_outside_the_catalog_and_any_without_a_bridge() {
        let fixture = Fixture::new("codex");
        fixture.write_lines(
            "turn.jsonl",
            &[json!({"id": "tool-1", "method": "item/tool/call", "params": {"tool": "ui_wallets_sync", "callId": "c", "arguments": 5}})],
        );
        fixture.write_lines("after-tool.jsonl", &[]);
        let request = incognito(json!({}));
        let (events, captured) = EventSink::capture();
        let bridge = ToolBridge::start(&request.tools, events.clone()).unwrap();
        let cwd = workdir();
        let outcome = CODEX.chat_with(
            &fixture.executable(),
            &request,
            cwd.path(),
            Some(&bridge),
            &events,
        );
        assert_eq!(outcome, Ok(()));
        assert!(!captured.text().contains("tool_call"));
        assert!(fixture.wire_text().contains(
            r#"{"id":"tool-1","result":{"contentItems":[{"type":"inputText","text":"Kassiber denied the tool call."}],"success":false}}"#
        ));
        // No tools advertised: the call is refused like any native request.
        let fixture = Fixture::new("codex");
        fixture.write_lines(
            "turn.jsonl",
            &[json!({"id": "tool-1", "method": "item/tool/call", "params": {"tool": "status"}})],
        );
        fixture.write_lines("after-tool.jsonl", &[]);
        let turn = run(
            &CODEX,
            fixture,
            &chat(ProviderId::Codex, "gpt-5.4", json!({})),
        );
        assert_eq!(turn.turn.error, None);
        assert!(turn.fixture.wire_text().contains(
            r#"{"id":"tool-1","error":{"code":-32001,"message":"Provider-native tools are disabled by Kassiber."}}"#
        ));
    }

    // Status.

    #[test]
    fn reports_ready_models_with_efforts_and_the_fast_tier() {
        let fixture = Fixture::new("codex");
        fixture.write(
            "models.json",
            r#"{"data":[{"id":"gpt-5.4","displayName":"GPT-5.4","supportedReasoningEfforts":[{"reasoningEffort":"low"},"high",{"reasoningEffort":null},7,{}],"serviceTiers":[{"id":"priority","name":"Fast","description":"2x speed"}]},{"model":"slug-only"},{"id":0,"model":"zero"}],"nextCursor":"p2"}"#,
        );
        fixture.write(
            "models-p2.json",
            r#"{"data":[{"id":"gpt-5.4-mini"}],"nextCursor":""}"#,
        );
        let cwd = workdir();
        let status = CODEX.status_with(&fixture.executable(), cwd.path());
        assert_eq!(status.state, ProviderState::Ready);
        assert_eq!(status.message, "Ready using the existing Codex login.");
        assert_eq!(status.executable(), Some("codex"));
        assert_eq!(status.version, None);
        assert_eq!(
            serde_json::to_value(&status.models).unwrap(),
            json!([
                {"id": "gpt-5.4", "display_name": "GPT-5.4", "owned_by": "OpenAI Codex", "supports_reasoning_effort": true, "reasoning_efforts": ["low", "high"], "supports_fast_mode": true, "fast_mode_description": "2x speed"},
                {"id": "slug-only", "owned_by": "OpenAI Codex", "supports_reasoning_effort": false, "reasoning_efforts": []},
                {"id": "zero", "owned_by": "OpenAI Codex", "supports_reasoning_effort": false, "reasoning_efforts": []},
                {"id": "gpt-5.4-mini", "owned_by": "OpenAI Codex", "supports_reasoning_effort": false, "reasoning_efforts": []},
            ])
        );
        let methods: Vec<String> = fixture
            .wire()
            .iter()
            .filter_map(|message| message["method"].as_str().map(str::to_owned))
            .collect();
        assert_eq!(
            methods,
            ["initialize", "initialized", "model/list", "model/list"]
        );
        assert!(fixture.wire_text().contains(r#""params":{"cursor":"p2"}"#));
    }

    #[test]
    fn asks_for_a_login_when_codex_lists_no_models_or_says_so() {
        let fixture = Fixture::new("codex");
        fixture.write("models.json", r#"{"data":[]}"#);
        let cwd = workdir();
        let status = CODEX.status_with(&fixture.executable(), cwd.path());
        assert_eq!(status.state, ProviderState::AuthenticationRequired);
        assert_eq!(status.message, "Run `codex login` outside Kassiber.");

        let fixture = Fixture::new("codex");
        fixture.write("stderr", "Error: not logged in, run codex login\n");
        let status = CODEX.status_with(&fixture.executable(), cwd.path());
        assert_eq!(status.state, ProviderState::AuthenticationRequired);
        assert_eq!(status.message, "Run `codex login` outside Kassiber.");

        let fixture = Fixture::new("codex");
        fixture.write("stderr", "boom: token=sk-live-abcdefgh12345\n");
        let status = CODEX.status_with(&fixture.executable(), cwd.path());
        assert_eq!(status.state, ProviderState::Error);
        assert_eq!(status.message, "boom: token=[redacted]");
    }
}
