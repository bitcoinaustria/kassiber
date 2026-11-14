//! `providerEnvironment` (`executables.ts`): the environment a provider CLI
//! runs with. Only named variables and a few provider prefixes cross; the
//! rest of the daemon's environment stays behind.

use std::ffi::OsString;

use crate::protocol::ProviderId;

/// What every provider gets.
const ALLOWED: &[&str] = &[
    "HOME",
    "USER",
    "USERNAME",
    "PATH",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
    // Both cases: curl/Go/Python honour the lowercase spellings, and dropping
    // them makes every provider look offline for users who set only those.
    "HTTP_PROXY",
    "HTTPS_PROXY",
    "ALL_PROXY",
    "NO_PROXY",
    "http_proxy",
    "https_proxy",
    "all_proxy",
    "no_proxy",
];

/// Codex's and OpenCode's OpenAI credentials and configuration.
const OPENAI: &[&str] = &[
    "CODEX_HOME",
    "CODEX_API_KEY",
    "CODEX_ACCESS_TOKEN",
    "OPENAI_BASE_URL",
    "OPENAI_API_KEY",
];

/// Claude's and OpenCode's Anthropic credentials and configuration.
const ANTHROPIC: &[&str] = &[
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "CLAUDE_CODE_OAUTH_TOKEN",
];
const ANTHROPIC_PREFIXES: &[&str] = &["ANTHROPIC_", "CLAUDE_", "AWS_", "GOOGLE_", "CLOUDSDK_"];

/// Named, not a COPILOT_ prefix: COPILOT_ALLOW_ALL=true would approve every
/// tool and trust the working directory's hooks and MCP servers. The adapter
/// sets COPILOT_HOME itself.
const COPILOT: &[&str] = &[
    "COPILOT_GITHUB_TOKEN",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "GH_HOST",
    "COPILOT_GH_HOST",
    // Bring-your-own-model settings, by name. COPILOT_PROVIDER_API_KEY_COMMAND
    // is left out: Copilot runs it as a shell command on every request,
    // outside any tool call Kassiber could see or refuse.
    "COPILOT_PROVIDER_BASE_URL",
    "COPILOT_PROVIDER_TYPE",
    "COPILOT_PROVIDER_API_KEY",
    "COPILOT_PROVIDER_BEARER_TOKEN",
    "COPILOT_PROVIDER_WIRE_API",
    "COPILOT_PROVIDER_TRANSPORT",
    "COPILOT_PROVIDER_AZURE_API_VERSION",
    "COPILOT_PROVIDER_MODEL_ID",
    "COPILOT_PROVIDER_WIRE_MODEL",
    "COPILOT_PROVIDER_MAX_PROMPT_TOKENS",
    "COPILOT_PROVIDER_MAX_OUTPUT_TOKENS",
    "COPILOT_PROVIDER_HEADERS",
];

/// Ambient server-auth settings would make the loopback server demand
/// credentials the client here does not send, so every session call 401s for
/// users with a password-protected OpenCode config.
const DROPPED: &[&str] = &[
    "OPENCODE_SERVER_PASSWORD",
    "OPENCODE_SERVER_TOKEN",
    "OPENCODE_SERVER_AUTH",
    "OPENCODE_API_PASSWORD",
    "OPENCODE_PASSWORD",
    // Would upload every session to a public share link; OpenCode turns run
    // with sharing off (see `withoutSharing` in opencode.ts).
    "OPENCODE_AUTO_SHARE",
];

/// Set on every provider's environment so its own tooling can tell it runs
/// under Kassiber.
pub const BROKER_MARKER: (&str, &str) = ("KASSIBER_AI_BROKER", "1");

/// Whether `name` crosses into `provider`'s environment.
pub fn crosses(provider: ProviderId, name: &str) -> bool {
    if DROPPED.contains(&name) {
        return false;
    }
    let openai = matches!(provider, ProviderId::Codex | ProviderId::OpenCode);
    let anthropic = matches!(provider, ProviderId::Claude | ProviderId::OpenCode);
    ALLOWED.contains(&name)
        || (openai && OPENAI.contains(&name))
        || (anthropic && ANTHROPIC.contains(&name))
        || (provider == ProviderId::Copilot && COPILOT.contains(&name))
        || (anthropic
            && ANTHROPIC_PREFIXES
                .iter()
                .any(|prefix| name.starts_with(prefix)))
        || (provider == ProviderId::OpenCode && name.starts_with("OPENCODE_"))
}

/// `providerEnvironment(provider)`: the variables of `vars` that cross, in
/// their order, then `KASSIBER_AI_BROKER=1`. Names compare exactly, as
/// JavaScript object keys do (a Windows `Path` is not `PATH`); a name that
/// is not Unicode never crosses.
pub fn environment_from(
    provider: ProviderId,
    vars: impl IntoIterator<Item = (OsString, OsString)>,
) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = vars
        .into_iter()
        .filter(|(name, _)| name.to_str().is_some_and(|name| crosses(provider, name)))
        .filter(|(name, _)| name != BROKER_MARKER.0)
        .collect();
    env.push((BROKER_MARKER.0.into(), BROKER_MARKER.1.into()));
    env
}

/// `providerEnvironment(provider)` over this process's environment.
pub fn provider_environment(provider: ProviderId) -> Vec<(OsString, OsString)> {
    environment_from(provider, std::env::vars_os())
}

/// The value of `name` in an environment built here.
pub fn lookup<'a>(env: &'a [(OsString, OsString)], name: &str) -> Option<&'a OsString> {
    env.iter()
        .rev()
        .find(|(key, _)| key == name)
        .map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    #[test]
    fn keeps_provider_auth_and_config_isolated_and_drops_unrelated_variables() {
        // broker.test.ts: "keeps provider auth/config isolated and drops
        // unrelated ambient variables".
        let ambient = env(&[
            ("HOME", "/tmp/example-home"),
            ("CODEX_HOME", "/tmp/example-codex"),
            ("OPENAI_API_KEY", "openai-secret"),
            ("ANTHROPIC_API_KEY", "anthropic-secret"),
            ("CLAUDE_CODE_USE_BEDROCK", "1"),
            ("OPENCODE_CONFIG", "/tmp/example-opencode.json"),
            ("UNRELATED_PRIVATE_VALUE", "must-not-cross"),
        ]);
        let codex = environment_from(ProviderId::Codex, ambient.clone());
        let claude = environment_from(ProviderId::Claude, ambient.clone());
        let opencode = environment_from(ProviderId::OpenCode, ambient);
        let get = |env: &[(OsString, OsString)], name| {
            lookup(env, name).map(|value| value.to_str().unwrap().to_owned())
        };
        assert_eq!(get(&codex, "HOME").as_deref(), Some("/tmp/example-home"));
        assert_eq!(
            get(&codex, "CODEX_HOME").as_deref(),
            Some("/tmp/example-codex")
        );
        assert_eq!(
            get(&codex, "OPENAI_API_KEY").as_deref(),
            Some("openai-secret")
        );
        assert_eq!(get(&codex, "ANTHROPIC_API_KEY"), None);
        assert_eq!(
            get(&claude, "ANTHROPIC_API_KEY").as_deref(),
            Some("anthropic-secret")
        );
        assert_eq!(
            get(&claude, "CLAUDE_CODE_USE_BEDROCK").as_deref(),
            Some("1")
        );
        assert_eq!(get(&claude, "OPENAI_API_KEY"), None);
        assert_eq!(
            get(&opencode, "OPENCODE_CONFIG").as_deref(),
            Some("/tmp/example-opencode.json")
        );
        assert_eq!(
            get(&opencode, "OPENAI_API_KEY").as_deref(),
            Some("openai-secret")
        );
        assert_eq!(
            get(&opencode, "ANTHROPIC_API_KEY").as_deref(),
            Some("anthropic-secret")
        );
        assert_eq!(get(&opencode, "UNRELATED_PRIVATE_VALUE"), None);
        for provider in crate::protocol::PROVIDER_IDS {
            let built = environment_from(provider, Vec::new());
            assert_eq!(get(&built, "KASSIBER_AI_BROKER").as_deref(), Some("1"));
        }
    }

    #[test]
    fn copilot_takes_named_variables_only_and_opencode_drops_server_auth() {
        let ambient = env(&[
            ("COPILOT_GITHUB_TOKEN", "t"),
            ("COPILOT_ALLOW_ALL", "true"),
            ("COPILOT_PROVIDER_API_KEY_COMMAND", "curl evil"),
            ("COPILOT_HOME", "/elsewhere"),
            ("GH_TOKEN", "g"),
            ("OPENCODE_SERVER_PASSWORD", "p"),
            ("OPENCODE_AUTO_SHARE", "1"),
            ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
            ("AWS_PROFILE", "x"),
            ("KASSIBER_AI_BROKER", "0"),
        ]);
        let names = |provider| {
            environment_from(provider, ambient.clone())
                .into_iter()
                .map(|(name, value)| {
                    format!("{}={}", name.to_str().unwrap(), value.to_str().unwrap())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            names(ProviderId::Copilot),
            [
                "COPILOT_GITHUB_TOKEN=t",
                "GH_TOKEN=g",
                "KASSIBER_AI_BROKER=1"
            ]
        );
        assert_eq!(
            names(ProviderId::OpenCode),
            [
                "OPENCODE_DISABLE_AUTOUPDATE=1",
                "AWS_PROFILE=x",
                "KASSIBER_AI_BROKER=1"
            ]
        );
        assert_eq!(names(ProviderId::Codex), ["KASSIBER_AI_BROKER=1"]);
        assert_eq!(
            names(ProviderId::Claude),
            ["AWS_PROFILE=x", "KASSIBER_AI_BROKER=1"]
        );
        // JavaScript object keys are exact: a Windows `Path` is not `PATH`.
        assert!(!crosses(ProviderId::Codex, "Path"));
    }
}
