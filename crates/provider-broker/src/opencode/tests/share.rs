//! `opencode-share.test.ts` and the OpenCode cases of `stateless.test.ts`.
//!
//! OpenCode sharing uploads a session (prompts, answers, tool results) to a
//! public link. Every OpenCode turn runs with it off, whatever the user's
//! config or environment asks for, and fails closed if a later config layer
//! turns it back on. Incognito turns keep the session in an in-memory
//! database inside the turn's directory; on Windows they are refused up
//! front.

use super::*;
use crate::opencode::v1::{stateless_environment, verify_sharing_off, without_sharing};

fn env(pairs: &[(&str, &str)]) -> Env {
    Env(pairs
        .iter()
        .map(|(name, value)| ((*name).into(), (*value).into()))
        .collect())
}

/// A v1 API that answers `GET /config` with `config` and records the rest.
fn config_only(config: Value) -> FakeService {
    FakeService::start(move |request| {
        if request.method == "GET" && path_of(request) == "/config" {
            Reply::json(200, &config.to_string())
        } else {
            Reply::json(500, "{}")
        }
    })
    .unwrap()
}

#[test]
fn overrides_a_users_auto_share_in_config_and_environment() {
    let shared = without_sharing(env(&[
        ("OPENCODE_AUTO_SHARE", "1"),
        (
            "OPENCODE_CONFIG_CONTENT",
            r#"{"share":"auto","autoshare":true,"model":"x/y"}"#,
        ),
    ]))
    .unwrap();
    assert_eq!(shared.get("OPENCODE_AUTO_SHARE"), None);
    assert_eq!(
        shared.get_str("OPENCODE_DISABLE_SHARE").as_deref(),
        Some("1")
    );
    // The user's other settings stay; only sharing is forced off.
    assert_eq!(
        shared.get_str("OPENCODE_CONFIG_CONTENT").as_deref(),
        Some(r#"{"share":"disabled","autoshare":false,"model":"x/y"}"#)
    );
    let error = without_sharing(env(&[("OPENCODE_CONFIG_CONTENT", "[1]")])).unwrap_err();
    assert!(error.message().contains("could not be isolated"));
    // No inline config: one with sharing off.
    let fresh = without_sharing(env(&[])).unwrap();
    assert_eq!(
        fresh.get_str("OPENCODE_CONFIG_CONTENT").as_deref(),
        Some(r#"{"share":"disabled","autoshare":false}"#)
    );
}

#[test]
fn never_passes_the_auto_share_variable_to_opencode() {
    let fake = Fake::new("1.18.10");
    let host = fake.host_with(&[
        ("OPENCODE_AUTO_SHARE", "1"),
        ("OPENCODE_CONFIG", "/tmp/example-opencode.json"),
    ]);
    let env = host.provider_env();
    assert_eq!(env.get("OPENCODE_AUTO_SHARE"), None);
    assert_eq!(
        env.get_str("OPENCODE_CONFIG").as_deref(),
        Some("/tmp/example-opencode.json")
    );
}

#[test]
fn refuses_any_effective_setting_other_than_disabled() {
    verify_sharing_off(&parse(r#"{"share":"disabled"}"#).unwrap()).unwrap();
    for config in [
        r#"{"share":"auto"}"#,
        r#"{"share":"manual"}"#,
        "{}",
        "[]",
        "null",
    ] {
        let error = verify_sharing_off(&parse(config).unwrap()).unwrap_err();
        assert!(
            error.message().contains("would share this session"),
            "{config}"
        );
    }
}

#[test]
fn starts_every_opencode_turn_with_sharing_off() {
    let fake = Fake::new("1.18.10");
    let host = fake.host_with(&[
        ("OPENCODE_AUTO_SHARE", "1"),
        ("OPENCODE_CONFIG_CONTENT", r#"{"share":"auto"}"#),
    ]);
    for options in [json!({}), json!({"stateless_session": true})] {
        let request = chat_request("provider/model", options);
        let (result, _, _) = run_chat(&fake, &host, &request, None);
        assert!(
            result.unwrap_err().message().contains("server exited"),
            "the fake server exits at once"
        );
    }
    let serves = fake.serves();
    assert_eq!(serves.len(), 2);
    for serve in serves {
        assert_eq!(serve.vars.get("OPENCODE_AUTO_SHARE"), None);
        assert!(!serve.names.iter().any(|name| name == "OPENCODE_AUTO_SHARE"));
        assert!(serve.names.iter().any(|name| name == "KASSIBER_AI_BROKER"));
        assert_eq!(
            serve.vars.get("OPENCODE_DISABLE_SHARE").map(String::as_str),
            Some("1")
        );
        let config = parse(&serve.vars["OPENCODE_CONFIG_CONTENT"]).unwrap();
        assert_eq!(config.get("share"), Some(&Js::str("disabled")));
        assert_eq!(
            &serve.argv[..3],
            ["serve", "--pure", "--hostname=127.0.0.1"]
        );
        assert!(serve.argv[3].starts_with("--port="));
    }
}

#[test]
fn stops_before_any_session_when_opencode_still_reports_sharing() {
    // A managed or organisation config layer merged after Kassiber's.
    let fake = Fake::new("1.18.10");
    fake.write("serve-mode", "v1");
    let service = config_only(json!({"share": "auto"}));
    fake.write("v1-port", &service.port().to_string());
    let mut host = fake.host();
    let port = service.port();
    host.free_port = Arc::new(move || Ok(port));
    let request = chat_request("provider/model", json!({}));
    let (result, _, _) = run_chat(&fake, &host, &request, None);
    assert!(result
        .unwrap_err()
        .message()
        .contains("would share this session"));
    // No session was created and no prompt was sent.
    assert_eq!(service.paths().len(), 1);
    assert!(service.paths()[0].starts_with("GET /config?directory="));
    // The control: with sharing off the turn goes on to open a session.
    let allowed = config_only(json!({"share": "disabled"}));
    fake.write("v1-port", &allowed.port().to_string());
    let port = allowed.port();
    host.free_port = Arc::new(move || Ok(port));
    let _ = run_chat(&fake, &host, &request, None);
    assert_eq!(allowed.paths()[1], "POST /session");
}

#[test]
fn incognito_on_windows_is_refused_before_anything_is_created_or_started() {
    let fake = Fake::new("1.18.10");
    let mut host = fake.host();
    host.windows = true;
    let cwd = fake.turn();
    let error = stateless_environment(&host, &fake.executable(), &cwd).unwrap_err();
    assert!(error
        .message()
        .starts_with("Incognito isn't available for OpenCode on Windows"));
    assert_eq!(std::fs::read_dir(&cwd).unwrap().count(), 0);
    // A whole turn says so before even the version check.
    let request = chat_request("provider/model", json!({"stateless_session": true}));
    let (result, events, _) = run_chat(&fake, &host, &request, None);
    assert!(result.unwrap_err().message().contains("on Windows"));
    assert!(events.is_empty());
    assert!(fake.runs().is_empty());
}

#[test]
fn the_process_host_follows_the_platform_the_broker_runs_on() {
    assert_eq!(Host::process().windows, cfg!(windows));
}

#[test]
fn incognito_keeps_the_session_in_memory_and_the_turns_folder_on_any_version() {
    let fake = Fake::new("1.18.35");
    let host = fake.host_with(&[("OPENCODE_CONFIG_CONTENT", r#"{"model":"a/b"}"#)]);
    let cwd = fake.turn();
    let env = stateless_environment(&host, &fake.executable(), &cwd).unwrap();
    assert_eq!(env.get_str("OPENCODE_DB").as_deref(), Some(":memory:"));
    assert_eq!(
        env.get("XDG_DATA_HOME").map(PathBuf::from),
        Some(cwd.join("private-runtime").join("data"))
    );
    assert_eq!(
        env.get("XDG_STATE_HOME").map(PathBuf::from),
        Some(cwd.join("private-runtime").join("state"))
    );
    // Not the selected-data lockdown: the user's configuration, and so the
    // models and Kassiber's per-session tool rules, are unchanged.
    assert_eq!(
        env.get("OPENCODE_CONFIG_CONTENT"),
        host.provider_env().get("OPENCODE_CONFIG_CONTENT")
    );
    assert_eq!(env.get("OPENCODE_PURE"), None);
}

#[test]
fn incognito_links_only_the_users_sign_in() {
    let fake = Fake::new("1.18.35");
    fake.write("data/opencode/auth.json", "{}");
    let data = fake.path("data");
    let host = fake.host_with(&[("XDG_DATA_HOME", data.to_str().unwrap())]);
    let cwd = fake.turn();
    stateless_environment(&host, &fake.executable(), &cwd).unwrap();
    let linked = cwd.join("private-runtime/data/opencode/auth.json");
    assert_eq!(
        std::fs::read_link(linked).unwrap(),
        data.join("opencode/auth.json")
    );
}

#[test]
fn starts_the_server_stateless_only_for_a_stateless_turn() {
    let fake = Fake::new("1.18.35");
    let host = fake.host();
    for options in [json!({"stateless_session": true}), json!({})] {
        let request = chat_request("provider/model", options);
        let (result, _, _) = run_chat(&fake, &host, &request, None);
        assert!(result.unwrap_err().message().contains("server exited"));
    }
    let serves = fake.serves();
    assert_eq!(serves.len(), 2);
    let (stateless, ordinary) = (&serves[0], &serves[1]);
    assert_eq!(
        stateless.vars.get("OPENCODE_DB").map(String::as_str),
        Some(":memory:")
    );
    assert!(stateless.vars["XDG_DATA_HOME"].starts_with(fake.root.path().to_str().unwrap()));
    assert_eq!(ordinary.vars.get("OPENCODE_DB"), None);
}

#[test]
fn sends_nothing_when_opencode_keeps_a_database_on_disk() {
    let fake = Fake::new("1.18.35");
    fake.write("disk-db", "");
    let request = chat_request("provider/model", json!({"stateless_session": true}));
    let (result, _, _) = run_chat(&fake, &fake.host(), &request, None);
    assert!(result
        .unwrap_err()
        .message()
        .contains("in-memory database; nothing was sent"));
    // The server never started.
    assert!(fake.serves().is_empty());
}
