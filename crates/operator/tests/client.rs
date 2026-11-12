//! `tests/test_operator_client.py`, ported against a scripted host. Python
//! patches methods of the client; here the host scripts what each connect
//! and receive answers, which exercises the same decisions end to end.

mod support;

use kassiber_operator::client::{BrokerClient, PreparedArguments, SubmitOptions};
use kassiber_pyjson::{PyStr, Value};
use std::time::Duration;
use support::*;

fn code(error: &kassiber_operator::Error) -> &str {
    error.code().unwrap_or("<not an AppError>")
}

fn details(error: &kassiber_operator::Error) -> Value {
    error.app().unwrap().details.clone().unwrap_or(Value::Null)
}

fn generation_ping(generation: &str, caller_context: bool) -> Connection {
    Connection::Replies(vec![reply(&format!(
        r#"{{"ok": true, "data": {{"generation": "{generation}", "caller_context": {caller_context}}}}}"#
    ))])
}

fn status_connection(text: &str) -> Connection {
    Connection::Replies(vec![ok(text)])
}

#[test]
fn signed_cli_restarts_an_idle_mismatched_helper_broker() {
    let old = r#"{"broker": "running", "generation": "old-generation", "native_auth_available": true, "native_auth_identity": "untrusted-helper"}"#;
    let replacement = r#"{"broker": "running", "generation": "new-generation", "native_auth_available": true, "native_auth_identity": "signed-helper"}"#;
    let host = ScriptedHost::new(vec![
        Connection::Replies(vec![ok(old)]),
        status_connection(r#"{"restart": "accepted"}"#),
        // The old broker is gone: pings are refused until a launch.
        Connection::Refused,
        Connection::Refused,
    ])
    .launching(vec![Connection::Replies(vec![ok(replacement)])]);
    let client = BrokerClient::new(host);
    assert_eq!(
        client
            .ensure_native_auth_running(Some(&|| Ok("signed-helper".to_owned())))
            .unwrap(),
        parse(replacement)
    );
    let log = client.host().log();
    assert_eq!(log.actions(), ["ping", "restart_for_native_auth", "ping"]);
    assert_eq!(log.launches, 1);
}

#[test]
fn native_auth_handoff_is_skipped_without_a_signed_helper() {
    let host = ScriptedHost::new(vec![ping_connection(&this_build())]);
    let client = BrokerClient::new(host);
    client.ensure_native_auth_running(None).unwrap();
    assert_eq!(client.host().log().actions(), ["ping"]);
    let failing = ScriptedHost::new(vec![
        Connection::Replies(vec![ok(r#"{"generation": "old"}"#)]),
        status_connection(r#"{"restart": "accepted"}"#),
        Connection::Replies(vec![ok(
            r#"{"generation": "new", "native_auth_available": true, "native_auth_identity": "other"}"#,
        )]),
        Connection::Replies(vec![ok(
            r#"{"generation": "new", "native_auth_available": true, "native_auth_identity": "other"}"#,
        )]),
    ]);
    let error = BrokerClient::new(failing)
        .ensure_native_auth_running(Some(&|| Ok("signed".to_owned())))
        .unwrap_err();
    assert_eq!(code(&error), "native_auth_unavailable");
    assert_eq!(
        error.to_string(),
        "the restarted operator broker cannot use native authentication"
    );
}

/// `submit` where the first acknowledgement is lost and the retry fails.
fn lost_acknowledgement(retry: Connection, status: Connection) -> kassiber_operator::Result<Value> {
    let host = ScriptedHost::new(vec![
        generation_ping("old-generation", true),
        Connection::Replies(vec![Reply::Io(std::io::ErrorKind::ConnectionReset)]),
        retry,
        status,
    ])
    .with_tokens(&["fixed-operation"]);
    let client = BrokerClient::new(host);
    client.submit(
        "/project",
        &PreparedArguments::new(vec!["status".into()]),
        None,
        &SubmitOptions::new(),
    )
}

#[test]
fn retry_app_error_from_new_broker_reports_result_unknown() {
    let error = lost_acknowledgement(
        Connection::Replies(vec![failure("interaction_required")]),
        status_connection(
            r#"{"operation_id": "old-generation.client.fixed-operation", "state": "result_unknown", "reason": "broker_generation_changed"}"#,
        ),
    )
    .unwrap_err();
    assert_eq!(code(&error), "operator_submission_result_unknown");
    assert!(!error.app().unwrap().retryable);
    assert_eq!(
        error.to_string(),
        "the broker could not prove the result of the submitted operation"
    );
    assert_eq!(
        details(&error),
        parse(
            r#"{"operation_id": "old-generation.client.fixed-operation", "state": "result_unknown", "reason": "broker_generation_changed"}"#
        )
    );
}

#[test]
fn retry_app_error_is_unknown_when_status_is_not_retained() {
    let error = lost_acknowledgement(
        Connection::Replies(vec![failure("operator_capability_denied")]),
        status_connection(
            r#"{"operation_id": "old-generation.client.fixed-operation", "state": "result_unknown", "reason": "result_not_retained"}"#,
        ),
    )
    .unwrap_err();
    assert_eq!(code(&error), "operator_submission_result_unknown");
    assert_eq!(
        details(&error),
        parse(
            r#"{"operation_id": "old-generation.client.fixed-operation", "state": "result_unknown", "reason": "result_not_retained"}"#
        )
    );
}

#[test]
fn retry_error_returns_status_when_original_operation_is_known() {
    let known = r#"{"operation_id": "old-generation.client.fixed-operation", "state": "completed", "exit_code": 0}"#;
    assert_eq!(
        lost_acknowledgement(
            Connection::Replies(vec![failure("operator_lease_ended")]),
            status_connection(known)
        )
        .unwrap(),
        parse(known)
    );
}

#[test]
fn unverifiable_generation_reports_result_unknown() {
    // The retry is refused, and so is the status request.
    let error = lost_acknowledgement(Connection::Refused, Connection::Refused).unwrap_err();
    assert_eq!(
        details(&error),
        parse(
            r#"{"operation_id": "old-generation.client.fixed-operation", "state": "result_unknown", "reason": "broker_unreachable"}"#
        )
    );
}

#[test]
fn a_status_without_a_reason_names_the_lost_acknowledgement() {
    let error = lost_acknowledgement(
        Connection::Refused,
        status_connection(r#"{"state": [1], "reason": 5}"#),
    )
    .unwrap_err();
    assert_eq!(
        kassiber_envelope::get(&details(&error), "reason"),
        Some(&kassiber_envelope::s("submission_acknowledgement_lost"))
    );
    let error = lost_acknowledgement(
        Connection::Refused,
        Connection::Replies(vec![failure("operator_internal_error")]),
    )
    .unwrap_err();
    assert_eq!(
        kassiber_envelope::get(&details(&error), "reason"),
        Some(&kassiber_envelope::s("broker_status_unavailable"))
    );
}

fn submitted_request(environment: &[(&str, &str)]) -> (Value, Value) {
    let mut host = ScriptedHost::new(vec![Connection::Replies(vec![
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("continue", kassiber_envelope::s("argv")),
            ("build", this_build()),
        ])),
        ok(r#"{"operation_id": "generation.client.op", "state": "queued"}"#),
    ])]);
    host.cwd = Ok(PyStr::from("/home/caller/project"));
    for (key, value) in environment {
        host = host.with_env(key, value);
    }
    let client = BrokerClient::new(host);
    client
        .submit_once(
            "/project",
            &PreparedArguments::new(vec![
                "--output".into(),
                "relative.json".into(),
                "status".into(),
            ]),
            &PyStr::from("generation.client.op"),
            None,
            &SubmitOptions::new(),
        )
        .unwrap();
    let sent = client.host().log().sent();
    // The command line waits for the broker to name its build.
    assert!(kassiber_envelope::get(&sent[0], "argv").is_none());
    assert_eq!(
        sent[1],
        parse(r#"{"argv": ["--output", "relative.json", "status"]}"#)
    );
    assert_eq!(
        kassiber_envelope::get(&sent[0], "working_directory"),
        Some(&kassiber_envelope::s("/home/caller/project"))
    );
    (sent[0].clone(), sent[1].clone())
}

#[test]
fn submit_carries_the_callers_working_directory() {
    let (request, _) = submitted_request(&[("KASSIBER_NO_EGRESS", "")]);
    assert_eq!(
        kassiber_envelope::get(&request, "no_egress"),
        Some(&Value::Bool(false))
    );
}

#[test]
fn submit_carries_the_callers_egress_kill_switch() {
    for value in ["yes", " TRUE ", "1", "on"] {
        let (request, _) = submitted_request(&[("KASSIBER_NO_EGRESS", value)]);
        assert_eq!(
            kassiber_envelope::get(&request, "no_egress"),
            Some(&Value::Bool(true)),
            "{value}"
        );
    }
    for value in ["0", "no", "off", "y"] {
        let (request, _) = submitted_request(&[("KASSIBER_NO_EGRESS", value)]);
        assert_eq!(
            kassiber_envelope::get(&request, "no_egress"),
            Some(&Value::Bool(false)),
            "{value}"
        );
    }
}

#[test]
fn egress_kill_switch_fails_closed_against_an_older_broker() {
    let host =
        ScriptedHost::new(vec![generation_ping("g", false)]).with_env("KASSIBER_NO_EGRESS", "1");
    let client = BrokerClient::new(host);
    let error = client
        .submit(
            "/project",
            &PreparedArguments::new(vec!["status".into()]),
            None,
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_outdated");
    assert_eq!(
        error.to_string(),
        "the running operator broker cannot honor KASSIBER_NO_EGRESS"
    );
    // Nothing was queued.
    assert_eq!(client.host().log().actions(), ["ping"]);
    let strict = ScriptedHost::new(vec![generation_ping("g", false)]);
    let error = BrokerClient::new(strict)
        .submit(
            "/project",
            &PreparedArguments::new(vec!["status".into()]),
            None,
            &SubmitOptions {
                require_caller_context: true,
                ..SubmitOptions::new()
            },
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_outdated");
}

#[test]
fn older_broker_without_kill_switch_submits_with_a_warning() {
    let host = ScriptedHost::new(vec![
        generation_ping("g", false),
        Connection::Replies(vec![
            Reply::Json(kassiber_envelope::dict([
                ("ok", Value::Bool(true)),
                ("continue", kassiber_envelope::s("argv")),
                ("build", this_build()),
            ])),
            ok(r#"{"operation_id": "g.client.x", "state": "queued"}"#),
        ]),
    ])
    .with_env("KASSIBER_NO_EGRESS", "");
    let client = BrokerClient::new(host);
    client
        .submit(
            "/project",
            &PreparedArguments::new(vec!["status".into()]),
            None,
            &SubmitOptions::new(),
        )
        .unwrap();
    let log = client.host().log();
    assert_eq!(log.actions(), ["ping", "submit"]);
    assert_eq!(log.warnings.len(), 1);
    assert!(log.warnings[0].contains("absolute paths"));
    assert!(log.warnings[0].ends_with('\n'));
}

#[test]
fn an_unreadable_cwd_fails_instead_of_using_the_brokers() {
    let mut host = ScriptedHost::new(vec![]);
    host.cwd = Err(std::io::ErrorKind::NotFound.into());
    let client = BrokerClient::new(host);
    let error = client
        .submit_once(
            "/project",
            &PreparedArguments::new(vec!["status".into()]),
            &PyStr::from("g.client.1"),
            None,
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_working_directory_unavailable");
    // It failed before connecting.
    assert!(client.host().log().connects.is_empty());
}

#[cfg(unix)]
#[test]
fn broker_starts_in_the_package_location_not_the_callers_directory() {
    use std::os::unix::fs::PermissionsExt;
    let install = tempfile::tempdir().unwrap();
    let report = install.path().join("started-in");
    let executable = install.path().join("kassiber");
    std::fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s|%s|%s' \"$PWD\" \"$1\" \"${{KASSIBER_OPERATOR_DIRECT-unset}}\" > '{}'\n",
            report.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut host = kassiber_operator::client::SystemHost::new(this_build());
    host.executable = executable;
    use kassiber_operator::client::Host;
    host.launch_broker().unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !report.exists() && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    std::thread::sleep(Duration::from_millis(50));
    let started = std::fs::read_to_string(&report).unwrap();
    let canonical = install.path().canonicalize().unwrap();
    assert_eq!(
        started,
        format!("{}|--operator-broker-server|unset", canonical.display())
    );
    assert_ne!(
        canonical,
        std::env::current_dir().unwrap().canonicalize().unwrap()
    );
}

#[test]
fn unlock_never_sends_a_passphrase_to_another_builds_broker() {
    let host = ScriptedHost::new(vec![ping_connection(&other_build())]);
    let client = BrokerClient::new(host);
    let error = client
        .unlock(
            "/project", b"secret", None, "read", "password", false, None, None,
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert_eq!(
        kassiber_envelope::get(&details(&error), "broker_build"),
        Some(&other_build())
    );
    assert_eq!(
        error.to_string(),
        "the operator broker is Kassiber 0.22.78 (source, ffffffffffff), not this build"
    );
    // Only the ping connected.
    assert_eq!(client.host().log().connects.len(), 1);
    assert!(client.host().log().secrets.is_empty());
}

#[test]
fn a_broker_without_build_identity_is_another_build() {
    let host = ScriptedHost::new(vec![generation_ping("g", true)]);
    let error = BrokerClient::new(host)
        .set_mode("/project", "brokered", b"secret")
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert_eq!(
        error.to_string(),
        "the operator broker is an older Kassiber build, not this build"
    );
}

#[test]
fn build_identity_equality_is_pythons() {
    let client = BrokerClient::new(ScriptedHost::new(vec![]));
    let same = |text: &str| {
        let mut broker = kassiber_envelope::dict::<&str>([]);
        kassiber_envelope::set(&mut broker, "build", parse(text));
        client.require_same_build(&broker).is_ok()
    };
    assert!(same(
        r#"{"origin": "0123456789ab", "frozen": false, "commit": null, "channel": null, "version": "0.22.78"}"#
    ));
    assert!(same(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": 0, "origin": "0123456789ab"}"#
    ));
    assert!(!same(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": false, "origin": "0123456789ac"}"#
    ));
    assert!(!same(
        r#"{"version": "0.22.79", "channel": null, "commit": null, "frozen": false, "origin": "0123456789ab"}"#
    ));
    assert!(!same(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": true, "origin": "0123456789ab"}"#
    ));
    assert!(!same(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "origin": "0123456789ab"}"#
    ));
    assert!(!same(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": false, "origin": "0123456789ab", "extra": 1}"#
    ));
    assert!(!same("null"));
}

#[test]
fn protocol_mismatch_fails_fast_without_spawning_a_broker() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![failure(
        "operator_protocol_version_mismatch",
    )])]);
    let client = BrokerClient::new(host);
    let error = client.ensure_running().unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert_eq!(
        error.to_string(),
        "the running operator broker speaks another protocol version"
    );
    assert_eq!(
        details(&error),
        kassiber_envelope::dict([("this_build", this_build())])
    );
    assert_eq!(client.host().log().launches, 0);
}

#[test]
fn other_app_errors_and_refusals_launch_then_time_out_with_the_reason() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![failure(
        "operator_internal_error",
    )])]);
    let client = BrokerClient::new(host);
    let error = client.ensure_running().unwrap_err();
    assert_eq!(code(&error), "operator_broker_start_failed");
    assert_eq!(
        details(&error),
        parse(r#"{"reason": "ConnectionRefusedError"}"#)
    );
    assert!(error.app().unwrap().retryable);
    let log = client.host().log();
    assert_eq!(log.launches, 1);
    // 5 s of 50 ms polls on the virtual clock.
    assert_eq!(log.connects.len(), 1 + 100);
}

#[test]
fn stop_asks_an_idle_broker_to_exit_and_explains_a_busy_one() {
    let host = ScriptedHost::new(vec![
        Connection::Replies(vec![ok(r#"{"build": {"version": "x"}}"#)]),
        status_connection(r#"{"restart": "accepted"}"#),
    ]);
    let client = BrokerClient::new(host);
    assert_eq!(
        client.stop().unwrap(),
        parse(r#"{"broker": "stopping", "broker_build": {"version": "x"}}"#)
    );
    assert_eq!(
        client.host().log().actions(),
        ["ping", "restart_for_native_auth"]
    );
    let busy = ScriptedHost::new(vec![
        Connection::Replies(vec![ok("{}")]),
        Connection::Replies(vec![failure("operator_broker_busy")]),
    ]);
    let error = BrokerClient::new(busy).stop().unwrap_err();
    assert_eq!(code(&error), "operator_broker_busy");
    assert!(error
        .app()
        .unwrap()
        .hint
        .as_deref()
        .unwrap()
        .contains("operator lock"));
    assert!(error.app().unwrap().retryable);
    let stopped = ScriptedHost::new(vec![]);
    assert_eq!(
        BrokerClient::new(stopped).stop().unwrap(),
        parse(r#"{"broker": "stopped"}"#)
    );
}

#[test]
fn stop_explains_a_broker_of_another_protocol() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![failure(
        "operator_protocol_version_mismatch",
    )])]);
    let client = BrokerClient::new(host);
    let error = client.stop().unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert!(error
        .app()
        .unwrap()
        .hint
        .as_deref()
        .unwrap()
        .contains("from that build"));
    assert_eq!(client.host().log().actions(), ["ping"]);
}

fn secret_continuation(build: &Value) -> Reply {
    Reply::Json(kassiber_envelope::dict([
        ("ok", Value::Bool(true)),
        ("continue", kassiber_envelope::s("secret")),
        ("challenge", kassiber_envelope::s("c")),
        ("build", build.clone()),
    ]))
}

#[test]
fn the_broker_asking_for_the_secret_must_be_this_build() {
    // The ping reached this build, but another broker took the endpoint
    // before the secret-bearing connection.
    let unlock = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![secret_continuation(&other_build())]),
    ]);
    let client = BrokerClient::new(unlock);
    let error = client
        .unlock(
            "/project", b"secret", None, "read", "password", false, None, None,
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert!(client.host().log().secrets.is_empty());
    let mode = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![secret_continuation(&other_build())]),
    ]);
    let client = BrokerClient::new(mode);
    let error = client
        .set_mode("/project", "brokered", b"secret")
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    assert!(client.host().log().secrets.is_empty());
}

#[test]
fn secrets_never_go_to_another_build_even_for_ordinary_commands() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![Reply::Json(
        kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("continue", kassiber_envelope::s("argv")),
            ("build", other_build()),
        ]),
    )])]);
    let client = BrokerClient::new(host);
    let error = client
        .submit_once(
            "/project",
            &PreparedArguments::new(
                ["backends", "update", "x", "--token", "inline-secret"]
                    .map(String::from)
                    .into(),
            ),
            &PyStr::from("g.client.1"),
            Some(b"fresh-passphrase"),
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_broker_build_mismatch");
    let log = client.host().log();
    assert!(log.secrets.is_empty());
    // Nothing but the opening request (no command line) reached it.
    assert_eq!(log.connections[0].len(), 1);
    assert!(!String::from_utf8_lossy(&log.bytes()).contains("inline-secret"));
}

fn accepted_connection(build: Option<&Value>) -> Connection {
    let mut data = parse(r#"{"operation_id": "g.client.1", "state": "queued"}"#);
    if let Some(build) = build {
        kassiber_envelope::set(&mut data, "build", build.clone());
    }
    Connection::Replies(vec![
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("continue", kassiber_envelope::s("argv")),
            ("build", build.cloned().unwrap_or_else(this_build)),
        ])),
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("data", data),
        ])),
    ])
}

fn submit_without_start(
    host: ScriptedHost,
    required: bool,
) -> (kassiber_operator::Result<Value>, Vec<String>) {
    let client = BrokerClient::new(host);
    let result = client.submit(
        "/project",
        &PreparedArguments::new(vec!["status".into()]),
        None,
        &SubmitOptions {
            start_broker: false,
            require_same_build: required,
            ..SubmitOptions::default()
        },
    );
    let actions = client.host().log().actions();
    (result, actions)
}

/// A broker that names its build in the continuation but not acceptance.
fn named_continuation_unnamed_acceptance(continuation: &Value) -> Connection {
    Connection::Replies(vec![
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("continue", kassiber_envelope::s("argv")),
            ("build", continuation.clone()),
        ])),
        ok(r#"{"operation_id": "g.client.1", "state": "queued"}"#),
    ])
}

#[test]
fn work_another_build_accepted_is_withdrawn_when_this_build_is_required() {
    // The continuation named this build, the acceptance another.
    let mut data = parse(r#"{"operation_id": "g.client.1", "state": "queued"}"#);
    kassiber_envelope::set(&mut data, "build", other_build());
    let switched = Connection::Replies(vec![
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("continue", kassiber_envelope::s("argv")),
            ("build", this_build()),
        ])),
        Reply::Json(kassiber_envelope::dict([
            ("ok", Value::Bool(true)),
            ("data", data),
        ])),
    ]);
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        switched.clone(),
        status_connection(r#"{"operation_id": "g.client.1", "state": "cancelled"}"#),
    ]);
    let (result, actions) = submit_without_start(host, true);
    assert_eq!(code(&result.unwrap_err()), "operator_broker_build_mismatch");
    assert_eq!(actions, ["ping", "submit", "operation_cancel"]);
    // Ordinary commands run where the lease is, and say so.
    let host = ScriptedHost::new(vec![ping_connection(&this_build()), switched]);
    let (result, _) = submit_without_start(host, false);
    let ordinary = result.unwrap();
    assert_eq!(
        kassiber_envelope::get(&ordinary, "broker_build"),
        Some(&other_build())
    );
    assert!(kassiber_envelope::get(&ordinary, "build").is_none());
}

#[test]
fn an_acceptance_without_a_build_falls_back_to_the_ping() {
    // Only mocking could make another build's broker accept: the argv
    // continuation must always name this build. The ping can still differ.
    let host = ScriptedHost::new(vec![
        ping_connection(&other_build()),
        named_continuation_unnamed_acceptance(&this_build()),
    ]);
    let (result, _) = submit_without_start(host, false);
    assert_eq!(
        kassiber_envelope::get(&result.unwrap(), "broker_build"),
        Some(&other_build())
    );
    // Required binding never trusts the ping for an unnamed acceptance.
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        named_continuation_unnamed_acceptance(&this_build()),
        status_connection("{}"),
    ]);
    let (result, actions) = submit_without_start(host, true);
    assert_eq!(code(&result.unwrap_err()), "operator_broker_build_mismatch");
    assert_eq!(actions.last().map(String::as_str), Some("operation_cancel"));
}

#[test]
fn a_retry_accepted_by_another_build_is_still_refused() {
    let mut data = parse(r#"{"operation_id": "g.client.1", "state": "queued"}"#);
    kassiber_envelope::set(&mut data, "build", other_build());
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![Reply::Io(std::io::ErrorKind::ConnectionReset)]),
        Connection::Replies(vec![
            Reply::Json(kassiber_envelope::dict([
                ("ok", Value::Bool(true)),
                ("continue", kassiber_envelope::s("argv")),
                ("build", this_build()),
            ])),
            Reply::Json(kassiber_envelope::dict([
                ("ok", Value::Bool(true)),
                ("data", data),
            ])),
        ]),
        status_connection("{}"),
    ]);
    let (result, actions) = submit_without_start(host, true);
    assert_eq!(code(&result.unwrap_err()), "operator_broker_build_mismatch");
    // Withdrawn, never reconciled through operation_status.
    assert_eq!(actions, ["ping", "submit", "submit", "operation_cancel"]);
}

#[test]
fn submit_asks_the_broker_to_refuse_another_build() {
    let host = ScriptedHost::new(vec![
        accepted_connection(Some(&this_build())),
        named_continuation_unnamed_acceptance(&this_build()),
    ]);
    let client = BrokerClient::new(host);
    for (operation, required) in [("g.client.1", true), ("g.client.2", false)] {
        client
            .submit_once(
                "/project",
                &PreparedArguments::new(vec!["status".into()]),
                &PyStr::from(operation),
                None,
                &SubmitOptions {
                    require_same_build: required,
                    ..SubmitOptions::new()
                },
            )
            .unwrap();
    }
    let sent = client.host().log().sent();
    assert!(kassiber_operator::values::py_eq(
        kassiber_envelope::get(&sent[0], "expected_build").unwrap(),
        &this_build()
    ));
    assert!(kassiber_envelope::get(&sent[2], "expected_build").is_none());
}

#[test]
fn a_retried_unnamed_acceptance_reports_no_build() {
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![Reply::Io(std::io::ErrorKind::ConnectionReset)]),
        named_continuation_unnamed_acceptance(&this_build()),
    ]);
    let (result, _) = submit_without_start(host, false);
    assert_eq!(
        kassiber_envelope::get(&result.unwrap(), "broker_build"),
        Some(&Value::Null)
    );
}

#[test]
fn a_retried_submit_still_names_the_brokers_build() {
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![Reply::Eof]),
        accepted_connection(Some(&this_build())),
    ]);
    let (result, _) = submit_without_start(host, true);
    assert_eq!(
        kassiber_envelope::get(&result.unwrap(), "broker_build"),
        Some(&this_build())
    );
}

#[test]
fn submit_without_a_running_broker_never_launches_one() {
    let (result, actions) = submit_without_start(ScriptedHost::new(vec![]), false);
    let error = result.unwrap_err();
    assert_eq!(code(&error), "interaction_required");
    assert_eq!(
        error.to_string(),
        "this project has no active operator lease"
    );
    assert_eq!(
        details(&error),
        parse(r#"{"reason": "operator_lease_required", "unlock_mode": "brokered"}"#)
    );
    assert!(error.app().unwrap().retryable);
    assert!(actions.is_empty());
    let host = ScriptedHost::new(vec![Connection::Replies(vec![failure(
        "operator_protocol_version_mismatch",
    )])]);
    let (result, _) = submit_without_start(host, false);
    assert_eq!(code(&result.unwrap_err()), "operator_broker_build_mismatch");
}

#[test]
fn broker_errors_keep_their_fields() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![reply(
        r#"{"ok": false, "error": {"code": "operator_lease_required", "message": "lease \ud800 ended", "hint": "unlock", "details": {"limit": 64}, "retryable": 1}}"#,
    )])]);
    let error = BrokerClient::new(host)
        .cancel(&PyStr::from("x"))
        .unwrap_err();
    let app = error.app().unwrap();
    assert_eq!(app.code, "operator_lease_required");
    assert_eq!(app.hint.as_deref(), Some("unlock"));
    assert!(app.retryable);
    assert_eq!(app.details, Some(parse(r#"{"limit": 64}"#)));
    assert_eq!(
        app.message_codepoints
            .as_deref()
            .unwrap()
            .code_points()
            .collect::<Vec<_>>(),
        "lease \u{fffd} ended"
            .chars()
            .map(u32::from)
            .map(|p| if p == 0xfffd { 0xd800 } else { p })
            .collect::<Vec<_>>()
    );
    for (text, code, message) in [
        (
            r#"{"ok": false, "error": {}}"#,
            "operator_error",
            "operator broker error",
        ),
        (
            r#"{"ok": false, "error": {"code": 7, "message": 0}}"#,
            "7",
            "operator broker error",
        ),
        (
            r#"{"ok": false, "error": {"message": ["a"]}}"#,
            "operator_error",
            "['a']",
        ),
    ] {
        let host = ScriptedHost::new(vec![Connection::Replies(vec![reply(text)])]);
        let error = BrokerClient::new(host)
            .cancel(&PyStr::from("x"))
            .unwrap_err();
        assert_eq!(
            (error.code(), error.to_string().as_str()),
            (Some(code), message),
            "{text}"
        );
    }
    for (text, message) in [
        (
            r#"{"ok": false, "error": "text"}"#,
            "invalid broker error response",
        ),
        (r#"{"ok": true, "data": []}"#, "invalid broker response"),
        (r#"{"ok": 0, "data": {}}"#, "<accepted>"),
    ] {
        let host = ScriptedHost::new(vec![Connection::Replies(vec![reply(text)])]);
        match BrokerClient::new(host).cancel(&PyStr::from("x")) {
            Ok(_) => assert_eq!(message, "<accepted>"),
            Err(error) => {
                assert_eq!(error.code(), Some("operator_protocol_error"));
                assert_eq!(error.to_string(), message);
            }
        }
    }
}

#[test]
fn connection_timeouts_follow_each_action() {
    let host = ScriptedHost::new(vec![]);
    let client = BrokerClient::new(host);
    let _ = client.ping();
    let _ = client.status(None);
    let _ = client.decide_agent_session("/p", "s", true, b"control");
    let _ = client.open_agent_session("/p");
    let connects = client.host().log().connects.clone();
    assert_eq!(
        connects,
        [
            (Duration::from_secs(1), Some(Duration::from_secs(1))),
            (Duration::from_secs(2), Some(Duration::from_secs(2))),
            (Duration::from_secs(2), Some(Duration::from_secs(5))),
            (Duration::from_secs(2), Some(Duration::from_secs(5))),
        ]
    );
}

#[test]
fn operation_status_of_an_unreachable_broker_is_result_unknown() {
    let client = BrokerClient::new(ScriptedHost::new(vec![]));
    assert_eq!(
        client.operation_status(&PyStr::from("g.client.1")).unwrap(),
        parse(
            r#"{"operation_id": "g.client.1", "state": "result_unknown", "reason": "broker_unreachable", "hint": "Reconcile project state before retrying the operation."}"#
        )
    );
    let waiting = ScriptedHost::new(vec![
        status_connection(r#"{"state": "queued"}"#),
        status_connection(r#"{"state": "running"}"#),
        status_connection(r#"{"state": "completed", "exit_code": 0}"#),
    ]);
    let client = BrokerClient::new(waiting);
    let started = client.host().now();
    use kassiber_operator::client::Host;
    assert_eq!(
        client.wait(&PyStr::from("g.client.1")).unwrap(),
        parse(r#"{"state": "completed", "exit_code": 0}"#)
    );
    // 0.05 s, then 0.07 s of backoff on the virtual clock.
    let waited = client.host().now() - started;
    assert!((waited.as_secs_f64() - 0.12).abs() < 1e-9, "{waited:?}");
}

#[test]
fn agent_sessions_heartbeat_until_closed() {
    let host = ScriptedHost::new(vec![Connection::Replies(vec![ok(
        r#"{"session_id": "s-1", "state": "pending", "token": "t-1"}"#,
    )])]);
    let client = BrokerClient::new(host);
    let mut handle = client.open_agent_session("/project").unwrap();
    assert!(handle.session_id.is("s-1"));
    assert!(handle.token.is("t-1"));
    assert!(handle.alive());
    handle.close();
    assert!(!handle.alive());
    let log = client.host().log();
    assert_eq!(log.actions(), ["agent_session_open"]);
    assert_eq!(log.closed, 1);
    drop(log);
    let missing = ScriptedHost::new(vec![Connection::Replies(vec![ok(
        r#"{"session_id": "s"}"#,
    )])]);
    let error = BrokerClient::new(missing)
        .open_agent_session("/p")
        .err()
        .unwrap();
    assert_eq!(error.to_string(), "broker did not return an agent session");
}

#[test]
fn unlock_with_agent_control_sends_both_secrets_in_order() {
    let mut continuation = parse(
        r#"{"ok": true, "continue": "secret", "challenge": "pass-challenge", "agent_control_challenge": "control-challenge"}"#,
    );
    kassiber_envelope::set(&mut continuation, "build", this_build());
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![
            Reply::Json(continuation.clone()),
            ok(r#"{"lease": "unlocked"}"#),
        ]),
    ]);
    let client = BrokerClient::new(host);
    client
        .unlock(
            "/project",
            b"pass",
            Some(3600.into()),
            "operator",
            "password",
            true,
            Some(b"control"),
            Some(900),
        )
        .unwrap();
    let log = client.host().log();
    assert_eq!(
        log.secrets,
        [
            ("pass-challenge".to_owned(), b"pass".to_vec()),
            ("control-challenge".to_owned(), b"control".to_vec())
        ]
    );
    assert_eq!(
        log.sent()[1],
        parse(
            r#"{"action": "unlock", "agent_scope": true, "authentication_method": "password", "capability": "operator", "data_root": "/project", "duration_seconds": 3600, "idle_timeout_seconds": 900, "only_if_locked": true, "version": 3}"#
        )
    );
    drop(log);
    // A control challenge without an agent request is refused.
    let host = ScriptedHost::new(vec![
        ping_connection(&this_build()),
        Connection::Replies(vec![Reply::Json(continuation)]),
    ]);
    let error = BrokerClient::new(host)
        .unlock(
            "/project", b"pass", None, "read", "password", false, None, None,
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid broker unlock challenge");
}

#[test]
fn submit_sends_command_secrets_in_label_order_then_admin() {
    let mut continuation = parse(
        r#"{"ok": true, "continue": "secrets", "challenges": {"b-label": "cb", "a-label": "ca"}, "admin_challenge": "cadmin"}"#,
    );
    kassiber_envelope::set(&mut continuation, "build", this_build());
    let script = |continuation: &Value| {
        Connection::Replies(vec![
            Reply::Json(kassiber_envelope::dict([
                ("ok", Value::Bool(true)),
                ("continue", kassiber_envelope::s("argv")),
                ("build", this_build()),
            ])),
            Reply::Json(continuation.clone()),
            ok(r#"{"operation_id": "g.client.1", "state": "queued"}"#),
        ])
    };
    let prepared = PreparedArguments {
        argv: vec!["x".into()],
        secrets: vec![
            ("b-label".into(), zeroize::Zeroizing::new(b"bee".to_vec())),
            ("a-label".into(), zeroize::Zeroizing::new(b"ay".to_vec())),
        ],
    };
    let client = BrokerClient::new(ScriptedHost::new(vec![script(&continuation)]));
    client
        .submit_once(
            "/p",
            &prepared,
            &PyStr::from("g.client.1"),
            Some(b"admin"),
            &SubmitOptions::new(),
        )
        .unwrap();
    let log = client.host().log();
    assert_eq!(
        log.secrets,
        [
            ("ca".to_owned(), b"ay".to_vec()),
            ("cb".to_owned(), b"bee".to_vec()),
            ("cadmin".to_owned(), b"admin".to_vec())
        ]
    );
    assert_eq!(
        kassiber_envelope::get(&log.sent()[0], "secret_labels"),
        Some(&parse(r#"["b-label", "a-label"]"#))
    );
    drop(log);
    let client = BrokerClient::new(ScriptedHost::new(vec![script(&continuation)]));
    let error = client
        .submit_once(
            "/p",
            &prepared,
            &PyStr::from("g.client.1"),
            None,
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(code(&error), "operator_admin_auth_required");
    let mut unknown = continuation.clone();
    kassiber_envelope::set(&mut unknown, "challenges", parse(r#"{"z-label": "cz"}"#));
    let client = BrokerClient::new(ScriptedHost::new(vec![script(&unknown)]));
    let error = client
        .submit_once(
            "/p",
            &prepared,
            &PyStr::from("g.client.1"),
            Some(b"admin"),
            &SubmitOptions::new(),
        )
        .unwrap_err();
    assert_eq!(error.to_string(), "invalid broker secret challenge");
    assert!(client.host().log().secrets.is_empty());
}
