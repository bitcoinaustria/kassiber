//! The daemon's protocol and R1 kinds, in process against the fake store.

use std::io::Write;
use std::sync::{Arc, Mutex};

use kassiber_daemon::{serve, supported_kinds, DaemonConfig};
use kassiber_envelope::get;
use kassiber_pyjson::Value;
use kassiber_runtime::fake::{FakeConfig, FakeStore};
use kassiber_runtime::store::{BookStore, Selection};
use kassiber_runtime::BuildInfo;

#[derive(Clone, Default)]
struct Captured(Arc<Mutex<Vec<u8>>>);

impl Write for Captured {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const BUILD: BuildInfo = BuildInfo {
    version: "0.22.78",
    channel: None,
    commit: None,
};

/// Runs the daemon over `input` and returns its exit code, the raw stdout
/// lines, and the store's operations.
fn run(config: FakeConfig, input: &str) -> (i32, Vec<String>, Vec<String>) {
    let store = FakeStore::new(config);
    let bootstrap = store.bootstrap(&Selection::default()).unwrap();
    let out = Captured::default();
    let code = serve(
        Arc::new(store.clone()),
        DaemonConfig {
            build: BUILD,
            bootstrap,
        },
        Box::new(std::io::Cursor::new(input.as_bytes().to_vec())),
        Box::new(out.clone()),
    );
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let lines = text.lines().map(str::to_owned).collect();
    (code, lines, store.events())
}

fn parse(line: &str) -> Value {
    kassiber_pytext::loads(line).unwrap()
}

fn text_of<'a>(value: &'a Value, key: &str) -> &'a str {
    text(value, key)
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    match get(value, key) {
        Some(Value::Str(text)) => text.as_str().unwrap(),
        other => panic!("{key}: {other:?}"),
    }
}

#[test]
fn ready_comes_first_without_a_request_id() {
    let (code, lines, events) = run(FakeConfig::default(), "");
    assert_eq!(code, 0);
    assert_eq!(
        lines[0],
        format!(
            "{{\"kind\":\"daemon.ready\",\"schema_version\":1,\"data\":{{\"version\":\"0.22.78\",\"supported_kinds\":[{}]}}}}",
            supported_kinds()
                .iter()
                .map(|k| format!("\"{k}\""))
                .collect::<Vec<_>>()
                .join(",")
        )
    );
    assert_eq!(lines.len(), 1);
    // Nothing is opened or owned until a request needs the book.
    assert_eq!(events, vec!["bootstrap /fake/kassiber/data".to_owned()]);
}

#[test]
fn malformed_requests_match_python() {
    let input = concat!(
        // Every kind Python classifies is native now, so no request reaches
        // the daemon's "not implemented in the Rust daemon" refusal.
        "{\"request_id\":\"a\",\"kind\":\"nope\"}\n",
        "not json\n",
        "[1]\n",
        "\n",
        "   \t\n",
        "{\"args\":{}}\n",
        "{\"request_id\":7,\"kind\":\"\"}\n",
        "{\"kind\":\"supervisor.ai_secret_store.response\",\"request_id\":\"x\"}\n",
        "{\"request_id\":\"c\",\"kind\":\"token=abc\"}\n",
    );
    let (_, lines, _) = run(FakeConfig::default(), input);
    let expected = [
        r#"{"kind":"error","schema_version":1,"error":{"code":"unsupported_kind","message":"daemon kind 'nope' is not classified or supported","hint":null,"details":{"kind":"nope"},"retryable":false,"debug":null},"request_id":"a"}"#,
        r#"{"kind":"error","schema_version":1,"error":{"code":"invalid_json","message":"daemon request line is not valid JSON","hint":null,"details":{"error":"Expecting value: line 1 column 1 (char 0)"},"retryable":false,"debug":null},"request_id":null}"#,
        r#"{"kind":"error","schema_version":1,"error":{"code":"validation","message":"daemon request must be a JSON object","hint":null,"details":{"type":"list"},"retryable":false,"debug":null},"request_id":null}"#,
        r#"{"kind":"error","schema_version":1,"error":{"code":"validation","message":"daemon request requires a non-empty string kind","hint":null,"details":{"keys":["args"]},"retryable":false,"debug":null}}"#,
        r#"{"kind":"error","schema_version":1,"error":{"code":"validation","message":"daemon request requires a non-empty string kind","hint":null,"details":{"keys":["kind","request_id"]},"retryable":false,"debug":null},"request_id":7}"#,
        r#"{"kind":"error","schema_version":1,"error":{"code":"unsupported_kind","message":"daemon kind 'token=[redacted]","hint":null,"details":{"kind":"token=[redacted]"},"retryable":false,"debug":null},"request_id":"c"}"#,
    ];
    assert_eq!(&lines[1..], &expected);
}

#[test]
fn oversized_lines_are_drained_then_reported() {
    let big = format!("{{\"kind\":\"{}\"}}\n", "x".repeat(1_000_100));
    let input = format!("{big}{{\"request_id\":\"after\",\"kind\":\"daemon.shutdown\"}}\n");
    let (code, lines, _) = run(FakeConfig::default(), &input);
    assert_eq!(code, 0);
    assert_eq!(
        lines[1],
        r#"{"kind":"error","schema_version":1,"error":{"code":"request_too_large","message":"daemon request line is too large","hint":null,"details":{"max_chars":1000000},"retryable":false,"debug":null},"request_id":null}"#
    );
    assert_eq!(
        lines[2],
        r#"{"kind":"daemon.shutdown","schema_version":1,"data":{},"request_id":"after"}"#
    );
}

#[test]
fn plaintext_status_opens_lazily_and_shutdown_releases() {
    let config = FakeConfig {
        project_id: Some("proj".into()),
        project_root: Some("/fake/kassiber".into()),
        ..FakeConfig::default()
    };
    let input = "{\"request_id\":\"s\",\"kind\":\"status\"}\n{\"request_id\":\"q\",\"kind\":\"daemon.shutdown\"}\n";
    let (code, lines, events) = run(config, input);
    assert_eq!(code, 0);
    let status = parse(&lines[1]);
    assert_eq!(text(&status, "kind"), "status");
    assert_eq!(text(&status, "request_id"), "s");
    let data = get(&status, "data").unwrap();
    assert_eq!(text(data, "project_id"), "proj");
    assert_eq!(text(data, "current_workspace"), "Personal");
    assert_eq!(text(data, "default_backend"), "mempool");
    assert!(lines[1].starts_with(
        "{\"kind\":\"status\",\"schema_version\":1,\"data\":{\"version\":\"0.22.78\",\"schema_version\":1,\"auth\":{\"mode\":\"local\",\"authenticated\":true},\"database_encrypted\":false,"
    ));
    assert_eq!(
        events[1..],
        [
            "acquire proj Desktop",
            "open /fake/kassiber/data",
            "opened proj select=true",
            "close",
            "release proj",
        ]
    );
}

#[test]
fn encrypted_books_need_the_passphrase() {
    let config = FakeConfig {
        encrypted: true,
        passphrase: Some("correct horse".into()),
        ..FakeConfig::default()
    };
    let input = concat!(
        "{\"request_id\":\"1\",\"kind\":\"status\"}\n",
        "{\"request_id\":\"2\",\"kind\":\"daemon.unlock\",\"args\":{}}\n",
        "{\"request_id\":\"3\",\"kind\":\"daemon.unlock\",\"args\":{\"auth_response\":{\"passphrase_secret\":\"wrong\"}}}\n",
        "{\"request_id\":\"4\",\"kind\":\"daemon.unlock\",\"args\":{\"auth_response\":{\"passphrase_secret\":\"correct horse\"}}}\n",
        "{\"request_id\":\"5\",\"kind\":\"status\"}\n",
        "{\"request_id\":\"6\",\"kind\":\"daemon.lock\"}\n",
        "{\"request_id\":\"7\",\"kind\":\"status\"}\n",
        "{\"request_id\":\"8\",\"kind\":\"daemon.unlock\",\"args\":[]}\n",
    );
    let (code, lines, events) = run(config, input);
    assert_eq!(code, 0);
    let locked = r#"{"kind":"auth_required","schema_version":1,"data":{"scope":"unlock_database","label":"Enter the SQLCipher database passphrase to unlock Kassiber."},"request_id":"1"}"#;
    assert_eq!(lines[1], locked);
    assert_eq!(lines[2], locked.replace("\"1\"", "\"2\""));
    assert_eq!(
        lines[3],
        r#"{"kind":"error","schema_version":1,"error":{"code":"local_auth_denied","message":"passphrase verification failed","hint":null,"details":null,"retryable":true,"debug":null},"request_id":"3"}"#
    );
    let unlocked = parse(&lines[4]);
    assert_eq!(text(&unlocked, "kind"), "daemon.unlock");
    let data = get(&unlocked, "data").unwrap();
    assert_eq!(get(data, "unlocked"), Some(&Value::Bool(true)));
    assert_eq!(
        get(get(data, "status").unwrap(), "database_encrypted"),
        Some(&Value::Bool(true))
    );
    assert_eq!(text(&parse(&lines[5]), "kind"), "status");
    assert_eq!(
        lines[6],
        r#"{"kind":"daemon.lock","schema_version":1,"data":{"locked":true},"request_id":"6"}"#
    );
    assert_eq!(lines[7], locked.replace("\"1\"", "\"7\""));
    assert_eq!(
        lines[8],
        r#"{"kind":"error","schema_version":1,"error":{"code":"validation","message":"daemon args must be an object","hint":null,"details":{"type":"list"},"retryable":false,"debug":null},"request_id":"8"}"#
    );
    // Wrong passphrases never open the book; lock closes before releasing.
    assert!(events.contains(&"verify unlock_database false".to_owned()));
    let close = events.iter().position(|e| e == "close").unwrap();
    let release = events
        .iter()
        .rposition(|e| e.starts_with("release"))
        .unwrap();
    assert!(close < release, "{events:?}");
}

#[test]
fn expected_scope_guards_the_book() {
    let input = concat!(
        "{\"request_id\":\"m\",\"kind\":\"status\",\"args\":{\"expected_scope\":{\"workspace_id\":\"ws-1\"}}}\n",
        "{\"request_id\":\"x\",\"kind\":\"status\",\"args\":{\"expected_scope\":{\"workspace_id\":\"ws-2\",\"profile_id\":\"pf-1\"}}}\n",
        "{\"request_id\":\"y\",\"kind\":\"status\",\"args\":{\"expected_scope\":{\"workspace_id\":\"ws-1\",\"profile_id\":\"pf-1\"},\"profile_id\":\"pf-9\"}}\n",
        "{\"request_id\":\"z\",\"kind\":\"status\",\"args\":{\"expected_scope\":{\"workspace_id\":\"ws-1\",\"profile_id\":\"pf-1\"},\"profile\":\"Main\"}}\n",
    );
    let (_, lines, _) = run(FakeConfig::default(), input);
    assert_eq!(
        lines[1],
        r#"{"kind":"error","schema_version":1,"error":{"code":"validation","message":"Expected book scope is malformed","hint":null,"details":null,"retryable":false,"debug":null},"request_id":"m"}"#
    );
    assert_eq!(
        lines[2],
        r#"{"kind":"error","schema_version":1,"error":{"code":"stale_context","message":"The active book changed before the requested input was supplied","hint":"Return to the original book and restart this input step.","details":null,"retryable":true,"debug":null},"request_id":"x"}"#
    );
    assert_eq!(
        lines[3],
        r#"{"kind":"error","schema_version":1,"error":{"code":"stale_context","message":"Requested input targets another book","hint":null,"details":null,"retryable":false,"debug":null},"request_id":"y"}"#
    );
    assert_eq!(text(&parse(&lines[4]), "kind"), "status");
}

#[test]
#[cfg(not(windows))]
fn a_lone_carriage_return_does_not_end_a_request() {
    // CPython reads stdin with newline="\n" outside Windows.
    let input = concat!(
        "{\"request_id\":\"a\",\"kind\":\"status\"}\r{\"request_id\":\"b\",\"kind\":\"daemon.shutdown\"}\n",
        "{\"kind\":\r\"daemon.shutdown\",\"request_id\":\"c\"}\n",
    );
    let (code, lines, _) = run(FakeConfig::default(), input);
    assert_eq!(code, 0);
    assert_eq!(lines.len(), 3, "{lines:?}");
    assert!(
        lines[1].contains("\"code\":\"invalid_json\""),
        "{}",
        lines[1]
    );
    assert!(
        lines[1].contains("Extra data: line 1 column 36 (char 35)"),
        "{}",
        lines[1]
    );
    assert_eq!(
        lines[2],
        r#"{"kind":"daemon.shutdown","schema_version":1,"data":{},"request_id":"c"}"#
    );
}

#[test]
fn status_reports_the_bootstrap_env_file() {
    let store = FakeStore::new(FakeConfig::default());
    let bootstrap = store
        .bootstrap(&Selection {
            env_file: Some("/custom/backends.env".into()),
            ..Selection::default()
        })
        .unwrap();
    let out = Captured::default();
    serve(
        Arc::new(store),
        DaemonConfig {
            build: BUILD,
            bootstrap,
        },
        Box::new(std::io::Cursor::new(
            b"{\"request_id\":\"s\",\"kind\":\"status\"}\n".to_vec(),
        )),
        Box::new(out.clone()),
    );
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let status = parse(text.lines().nth(1).unwrap());
    let data = get(&status, "data").unwrap();
    assert_eq!(text_of(data, "env_file"), "/custom/backends.env");
    assert_eq!(text_of(data, "default_backend"), "mempool");
}

#[test]
fn undecodable_lines_are_reported_not_fatal() {
    let mut input = b"\xff\xfe\n".to_vec();
    input.extend_from_slice(b"{\"request_id\":\"ok\",\"kind\":\"daemon.shutdown\"}\n");
    let store = FakeStore::new(FakeConfig::default());
    let bootstrap = store.bootstrap(&Selection::default()).unwrap();
    let out = Captured::default();
    serve(
        Arc::new(store),
        DaemonConfig {
            build: BUILD,
            bootstrap,
        },
        Box::new(std::io::Cursor::new(input)),
        Box::new(out.clone()),
    );
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[1].contains("\"code\":\"invalid_json\""));
    assert!(lines[2].starts_with("{\"kind\":\"daemon.shutdown\""));
}

#[test]
fn sensitive_actions_require_reauthorization_after_unlock_and_hold_the_owner() {
    let input=concat!(
        "{\"kind\":\"daemon.unlock\",\"args\":{\"auth_response\":{\"passphrase_secret\":\"correct horse\"}}}\n",
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\"}}\n",
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\",\"auth_response\":{\"passphrase_secret\":\"wrong\"}}}\n",
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\",\"auth_response\":{\"passphrase_secret\":\"correct horse\"}}}\n"
    );
    let (_, lines, events) = run(
        FakeConfig {
            encrypted: true,
            passphrase: Some("correct horse".into()),
            ..FakeConfig::default()
        },
        input,
    );
    assert_eq!(text(&parse(&lines[2]), "kind"), "auth_required");
    assert!(lines[2].contains("reveal_token"));
    assert!(lines[3].contains("local_auth_denied"));
    assert!(lines[4].contains("not_implemented")); // Fake store has no domain services.
    let owner = events
        .iter()
        .position(|e| e.starts_with("acquire"))
        .unwrap();
    assert!(events[owner].ends_with("Desktop"), "{events:?}");
    let verify = events
        .iter()
        .position(|e| e == "verify reveal_token false")
        .unwrap();
    assert!(owner < verify, "{events:?}");
    assert!(events.contains(&"verify reveal_token true".to_owned()));
}

#[test]
fn plaintext_sensitive_actions_require_the_exact_typed_acknowledgement() {
    let (_,lines,_)=run(FakeConfig::default(),concat!(
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\"}}\n",
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\",\"auth_response\":{\"plaintext_reveal_ack\":\"copy local secret\"}}}\n",
        "{\"kind\":\"backends.reveal_token\",\"args\":{\"name\":\"private\",\"auth_response\":{\"plaintext_reveal_ack\":\"COPY LOCAL SECRET\"}}}\n"
    ));
    assert!(lines[1].contains("requires plaintext acknowledgement"));
    assert!(lines[2].contains("requires plaintext acknowledgement"));
    assert!(lines[3].contains("not_implemented"));
}

#[test]
fn status_reports_storage_only_when_asked() {
    let store = FakeStore::new(FakeConfig::default());
    let bootstrap = store.bootstrap(&Selection::default()).unwrap();
    let out = Captured::default();
    serve(
        Arc::new(store),
        DaemonConfig {
            build: BUILD,
            bootstrap,
        },
        Box::new(std::io::Cursor::new(
            b"{\"request_id\":\"a\",\"kind\":\"status\"}\n{\"request_id\":\"b\",\"kind\":\"status\",\"args\":{\"storage\":true}}\n{\"request_id\":\"c\",\"kind\":\"status\",\"args\":{\"storage\":\"yes\"}}\n".to_vec(),
        )),
        Box::new(out.clone()),
    );
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let data = |line: usize| {
        let envelope = parse(text.lines().nth(line).unwrap());
        get(&envelope, "data").unwrap().clone()
    };
    // The default payload is unchanged; only an explicit `true` adds storage.
    assert_eq!(get(&data(1), "storage"), None);
    assert_eq!(get(&data(3), "storage"), None);
    let storage = data(2);
    let storage = get(&storage, "storage").expect("storage block");
    // The fake book has no file on disk: sizes are null, never guessed.
    assert_eq!(get(storage, "database_bytes"), Some(&Value::Null));
    assert_eq!(get(storage, "wal_bytes"), Some(&Value::Null));
}

#[test]
fn status_reports_a_revision_only_when_asked() {
    let store = FakeStore::new(FakeConfig::default());
    let bootstrap = store.bootstrap(&Selection::default()).unwrap();
    let out = Captured::default();
    serve(
        Arc::new(store),
        DaemonConfig {
            build: BUILD,
            bootstrap,
        },
        Box::new(std::io::Cursor::new(
            b"{\"request_id\":\"a\",\"kind\":\"status\"}\n{\"request_id\":\"b\",\"kind\":\"status\",\"args\":{\"revision\":true}}\n{\"request_id\":\"c\",\"kind\":\"status\",\"args\":{\"revision\":1}}\n".to_vec(),
        )),
        Box::new(out.clone()),
    );
    let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
    let data = |line: usize| {
        let envelope = parse(text.lines().nth(line).unwrap());
        get(&envelope, "data").unwrap().clone()
    };
    // The default payload the recorded sessions pin is unchanged; only an
    // explicit `true` adds the key.
    assert_eq!(get(&data(1), "revision"), None);
    assert_eq!(get(&data(3), "revision"), None);
    // The fake store cannot tell revisions apart: null, never a made-up token.
    assert_eq!(get(&data(2), "revision"), Some(&Value::Null));
}
