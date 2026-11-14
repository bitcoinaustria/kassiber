//! The OpenCode cases of `broker.test.ts` and `sensitive.test.ts`.

use super::*;
use crate::opencode::v1::{
    deny_all, is_loopback_endpoint, parse_models, permissions_for, sensitive_config,
    sensitive_environment, verify_sensitive_config,
};

#[test]
fn keeps_provider_auth_and_config_and_drops_unrelated_ambient_variables() {
    let fake = Fake::new("1.18.10");
    let host = fake.host_with(&[
        ("OPENCODE_CONFIG", "/tmp/example-opencode.json"),
        ("OPENAI_API_KEY", "openai-secret"),
        ("ANTHROPIC_API_KEY", "anthropic-secret"),
        ("UNRELATED_PRIVATE_VALUE", "must-not-cross"),
    ]);
    let env = host.provider_env();
    assert_eq!(
        env.get_str("OPENCODE_CONFIG").as_deref(),
        Some("/tmp/example-opencode.json")
    );
    assert_eq!(
        env.get_str("OPENAI_API_KEY").as_deref(),
        Some("openai-secret")
    );
    assert_eq!(
        env.get_str("ANTHROPIC_API_KEY").as_deref(),
        Some("anthropic-secret")
    );
    assert_eq!(env.get("UNRELATED_PRIVATE_VALUE"), None);
    assert_eq!(env.get_str("KASSIBER_AI_BROKER").as_deref(), Some("1"));
}

#[test]
fn removes_native_tools_and_grants_only_the_advertised_kassiber_tool() {
    assert_eq!(
        stringify(&deny_all()),
        r#"{"permission":"*","pattern":"*","action":"deny"}"#
    );
    let mut request = chat_request("provider/model", json!({}));
    request.tools = vec![tool("ui_reports_summary")];
    assert_eq!(
        stringify(&permissions_for(&request)),
        r#"[{"permission":"*","pattern":"*","action":"deny"},{"permission":"kassiber_ui_reports_summary","pattern":"*","action":"allow"}]"#
    );
}

#[test]
fn reports_model_sources_without_claiming_unverified_local_routing() {
    let stdout = "omlx/qwen-local\n{\"name\":\"Qwen Local\",\"variants\":{\"high\":{}}}\n";
    let endpoints = |url: Option<&str>| -> HashMap<String, String> {
        url.map(|url| HashMap::from([("omlx".to_owned(), url.to_owned())]))
            .unwrap_or_default()
    };
    // No endpoint reported for the source provider: stay conservative.
    let models = parse_models(stdout, &endpoints(None)).unwrap();
    assert_eq!(models.len(), 1);
    let model = serde_json::to_value(&models[0]).unwrap();
    assert_eq!(
        model,
        json!({
            "id": "omlx/qwen-local",
            "display_name": "Qwen Local",
            "owned_by": "OpenCode · omlx",
            "source_provider": "omlx",
            "privacy_posture": "remote",
            "privacy_reason": "OpenCode does not report an endpoint for this provider.",
            "supports_reasoning_effort": true,
            "reasoning_efforts": ["high"],
        })
    );
    // OpenCode routes the provider off-machine: still remote.
    let remote = parse_models(stdout, &endpoints(Some("https://api.example.com/v1"))).unwrap();
    assert_eq!(
        serde_json::to_value(&remote[0]).unwrap()["privacy_posture"],
        "remote"
    );
    // OpenCode routes it to a loopback runtime on this machine: local.
    let local = parse_models(stdout, &endpoints(Some("http://127.0.0.1:8000/v1"))).unwrap();
    assert_eq!(
        serde_json::to_value(&local[0]).unwrap()["privacy_posture"],
        "local"
    );
}

#[test]
fn counts_only_loopback_endpoints_as_on_machine() {
    for url in [
        "http://127.0.0.1:8000/v1",
        "http://127.2.3.4:1/v1",
        "http://localhost:11434/v1",
        "http://app.localhost:3000",
        "http://[::1]:8000/v1",
    ] {
        assert!(is_loopback_endpoint(url), "{url}");
    }
    for url in [
        "https://api.anthropic.com/v1",
        // A LAN box is still another machine.
        "http://192.168.1.20:11434/v1",
        "http://10.0.0.5:8000",
        "http://0.0.0.0:8000",
        "not-a-url",
        "",
    ] {
        assert!(!is_loopback_endpoint(url), "{url}");
    }
}

#[test]
fn treats_a_127_prefixed_hostname_as_remote() {
    // A prefix test on "127." also matches names that resolve publicly.
    assert!(!is_loopback_endpoint("http://127.api.example.com/v1"));
    assert!(!is_loopback_endpoint("http://127.0.0.1.example.com/v1"));
    assert!(!is_loopback_endpoint("http://127.999.0.1/v1"));
    assert!(is_loopback_endpoint("http://127.0.0.1:8000/v1"));
}

#[test]
fn keeps_namespaced_model_ids() {
    let models = parse_models("openrouter/anthropic/claude-x\n", &HashMap::new()).unwrap();
    assert_eq!(models.len(), 1);
    assert_eq!(models[0].id, "openrouter/anthropic/claude-x");
    assert_eq!(models[0].source_provider.as_deref(), Some("openrouter"));
    // `id.split("/", 2)[1]`.
    assert_eq!(models[0].display_name.as_deref(), Some("anthropic"));
}

#[test]
fn model_listings_ignore_colours_and_carriage_returns() {
    let stdout = "\x1b[1mlocal/a\x1b[0m\r\n{\"name\": \"A\",\r\n \"variants\": {\"2\": {}, \"low\": {}, \"1\": {}}}\r\nnoise\n";
    let models = parse_models(stdout, &HashMap::new()).unwrap();
    assert_eq!(models[0].id, "local/a");
    // The second line is part of the row's JSON, which then fails to parse
    // with "noise" appended: no metadata.
    assert_eq!(models[0].display_name.as_deref(), Some("a"));
    let stdout = "local/a\n{\"name\": \"A\", \"variants\": {\"2\": {}, \"low\": {}, \"1\": {}}}\n";
    let models = parse_models(stdout, &HashMap::new()).unwrap();
    assert_eq!(models[0].display_name.as_deref(), Some("A"));
    // `Object.keys` order: array indices first.
    assert_eq!(
        models[0].reasoning_efforts.as_deref(),
        Some(&["1".to_owned(), "2".to_owned(), "low".to_owned()][..])
    );
}

#[test]
fn sensitive_config_disables_secondary_models_and_rejects_effective_drift() {
    let model = "local/selected-model";
    let config = Js::Obj(sensitive_config(model).unwrap());
    verify_sensitive_config(&config, model).unwrap();
    let changed = |path: &[&str], value: Js| {
        let mut config = config.clone();
        set_path(&mut config, path, value);
        config
    };
    for drifted in [
        changed(&["model"], Js::str("remote/other")),
        changed(&["small_model"], Js::str("remote/title-model")),
        changed(&["share"], Js::str("auto")),
        changed(&["snapshot"], Js::Bool(true)),
        changed(&["compaction", "auto"], Js::Bool(true)),
        changed(&["permission", "read"], Js::str("allow")),
        changed(&["agent", "title", "disable"], Js::Bool(false)),
    ] {
        let error = verify_sensitive_config(&drifted, model).unwrap_err();
        assert!(error.message().contains("no selected data"), "{drifted:?}");
    }
}

/// `config.a.b = value`.
fn set_path(config: &mut Js, path: &[&str], value: Js) {
    let Js::Obj(object) = config else {
        panic!("not an object")
    };
    match path {
        [key] => object.set(*key, value),
        [key, rest @ ..] => set_path(object.get_mut(key).unwrap(), rest, value),
        [] => {}
    }
}

#[test]
fn installs_the_log_discard_before_the_in_memory_database_probe() {
    let fake = Fake::new("1.18.4");
    fake.write("write-log", "");
    let root = fake.turn();
    let env =
        sensitive_environment(&fake.host(), &fake.executable(), &root, "local/fixture").unwrap();
    let log = root.join("private-runtime/data/opencode/log/opencode.log");
    assert_eq!(std::fs::read_link(&log).unwrap(), Path::new("/dev/null"));
    assert_eq!(std::fs::read_to_string(&log).unwrap(), "");
    assert_eq!(env.get_str("OPENCODE_DB").as_deref(), Some(":memory:"));
    assert_eq!(env.get_str("OPENCODE_AUTO_SHARE").as_deref(), Some("0"));
    let config = parse(&env.get_str("OPENCODE_CONFIG_CONTENT").unwrap()).unwrap();
    assert_eq!(config.get("small_model"), Some(&Js::str("local/fixture")));
}

#[test]
fn a_sensitive_turn_needs_the_audited_release() {
    let fake = Fake::new("1.18.5");
    let error =
        sensitive_environment(&fake.host(), &fake.executable(), &fake.turn(), "a/b").unwrap_err();
    assert_eq!(
        error.message(),
        "OpenCode sensitive context requires the audited 1.18.4 runtime."
    );
}
