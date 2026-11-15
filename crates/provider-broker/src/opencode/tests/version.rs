//! `opencode-version.test.ts`: the version switch.

use super::*;

fn argvs(fake: &Fake) -> Vec<Vec<String>> {
    fake.runs().into_iter().map(|run| run.argv).collect()
}

#[test]
fn refuses_unaudited_channel_builds_before_starting_a_server() {
    for version in ["opencode v2.2.0-beta.1", "opencode v2.2.0+custom"] {
        let fake = Fake::new(version);
        let status = status(&fake.host(), &fake.turn());
        assert_eq!(status.state, ProviderState::Error, "{version}");
        assert!(status.message.contains("stable OpenCode 2"), "{version}");
        assert_eq!(argvs(&fake), [["--version"]]);
    }
}

#[test]
fn reports_an_unknown_future_major_before_config_discovery_or_chat_starts() {
    let fake = Fake::new("opencode v3.0.0");
    let status = status(&fake.host(), &fake.turn());
    assert_eq!(status.state, ProviderState::Error);
    assert_eq!(
        status.message,
        "This OpenCode major version is not supported by this build."
    );
    assert!(status.models.is_empty());
    let request = chat_request("provider/model", json!({}));
    let (result, _, _) = run_chat(&fake, &fake.host(), &request, None);
    assert_eq!(
        result.unwrap_err().message(),
        "This OpenCode major version is not supported by this build."
    );
    assert_eq!(argvs(&fake), [["--version"], ["--version"]]);
}

#[test]
fn still_discovers_models_on_a_compatible_v1_cli() {
    let fake = Fake::new("1.18.10");
    let status = status(&fake.host(), &fake.turn());
    assert_eq!(status.state, ProviderState::Ready);
    assert_eq!(status.version.as_deref(), Some("1.18.10"));
    assert_eq!(status.executable(), Some("opencode"));
    let ids: Vec<_> = status
        .models
        .iter()
        .map(|model| model.id.as_str())
        .collect();
    assert_eq!(ids, ["provider/model"]);
    assert_eq!(
        argvs(&fake),
        [
            vec!["--version"],
            vec!["debug", "config"],
            vec!["models", "--verbose"]
        ]
    );
}

#[test]
fn reports_a_failed_model_command_as_a_check_error_rather_than_a_login_problem() {
    let fake = Fake::new("1.18.10");
    fake.write("models-code", "1");
    let status = status(&fake.host(), &fake.turn());
    assert_eq!(status.state, ProviderState::Error);
    assert!(status.message.contains("could not list models"));
    assert!(status.models.is_empty());
}

#[test]
fn the_version_check_discards_the_cli_log_on_posix() {
    let fake = Fake::new("1.18.10");
    let cwd = fake.turn();
    check_version(&fake.host(), &fake.executable(), &cwd).unwrap();
    let logs = cwd.join("version/opencode-runtime/data/opencode/log");
    for name in ["opencode.log", "opencode-latest.log"] {
        assert_eq!(
            std::fs::read_link(logs.join(name)).unwrap(),
            Path::new("/dev/null")
        );
    }
    let run = &fake.runs()[0];
    assert_eq!(
        run.vars.get("XDG_DATA_HOME").map(PathBuf::from),
        Some(cwd.join("version/opencode-runtime/data"))
    );
}

#[test]
fn version_strings_follow_the_typescript_patterns() {
    let fake = Fake::new("");
    for (printed, expected) in [
        ("1.18.10", Some("1.18.10")),
        ("opencode v2.2.0", Some("opencode v2.2.0")),
        ("opencode   2.1.3", Some("opencode   2.1.3")),
        ("  1.2.3-rc.1  ", Some("1.2.3-rc.1")),
        ("1.2", None),
        ("1.2.3.4", None),
        ("v1.2.3", None),
        ("something else", None),
    ] {
        fake.write("version", printed);
        let version = check_version(&fake.host(), &fake.executable(), &fake.turn()).unwrap();
        assert_eq!(version.as_deref(), expected, "{printed:?}");
    }
    assert!(is_v2("opencode v2.2.0"));
    assert!(is_v2("2.0.1"));
    assert!(!is_v2("1.18.4"));
    assert!(!is_v2("02.1.0"));
}
