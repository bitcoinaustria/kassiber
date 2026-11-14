//! OpenCode 1 (`opencode.ts`): `opencode serve` on a free loopback port,
//! driven through the HTTP API the `@opencode-ai/sdk` client used.
//!
//! Every turn runs with sharing off ([`without_sharing`], then
//! [`verify_sharing_off`] against the server's effective configuration),
//! deny-all session permissions plus the advertised Kassiber tools
//! ([`permissions_for`]), and native tools refused when they appear. An
//! Incognito turn keeps the session in an in-memory database inside the
//! turn's directory ([`stateless_environment`]); a selected-data turn also
//! locks the configuration down to the one selected model
//! ([`sensitive_environment`]) and reports nothing of a failure.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use kassiber_net::Cancel;

use super::http::{form_encode, Call, FrameError, Frames, Loopback};
use super::js::{encode_uri_component, number_string, parse, stringify, Js, Obj};
use super::{model_separator, Env, Host, NotReady, ServerProcess, PROVIDER};
use crate::bridge::mcp_command;
use crate::cleanup::track_child;
use crate::executables::{fs_failure, provider_command, run_provider, spawn_failure, RunOptions};
use crate::prompt::{prompt_from_messages, prompt_images, system_instructions};
use crate::protocol::{
    js_space, js_trim, safe_session_cursor, truthy, BrokerFailure, BrokerModel, ChatRequest, Event,
    EventSink, ModelPosture,
};
use crate::ToolBridge;

fn fail(message: &str) -> BrokerFailure {
    BrokerFailure::new(message)
}

/// `DENY_ALL`.
pub fn deny_all() -> Js {
    Js::obj([
        ("permission", Js::str("*")),
        ("pattern", Js::str("*")),
        ("action", Js::str("deny")),
    ])
}

/// `permissionsFor(request)`: deny everything, then allow each advertised
/// Kassiber tool by its MCP name.
pub fn permissions_for(request: &ChatRequest) -> Js {
    let mut rules = vec![deny_all()];
    rules.extend(request.tools.iter().map(|tool| {
        Js::obj([
            ("permission", Js::str(format!("kassiber_{}", tool.name))),
            ("pattern", Js::str("*")),
            ("action", Js::str("allow")),
        ])
    }));
    Js::Arr(rules)
}

/// `splitModel(model)`.
fn split_model(model: &str) -> Result<(&str, &str), BrokerFailure> {
    let at = model_separator(model)?;
    Ok((&model[..at], &model[at + 1..]))
}

/// `sensitiveOpenCodeConfig(model)`: the selected model for every purpose,
/// sharing, snapshots, updates, compaction and every permission off, and a
/// primary agent that can do nothing but answer.
pub fn sensitive_config(model: &str) -> Result<Obj, BrokerFailure> {
    let (provider, _) = split_model(model)?;
    let deny = || Js::obj([("*", Js::str("deny"))]);
    let disabled = || Js::obj([("disable", Js::Bool(true))]);
    Ok(Obj::from_entries([
        ("model", Js::str(model)),
        ("small_model", Js::str(model)),
        ("enabled_providers", Js::Arr(vec![Js::str(provider)])),
        ("share", Js::str("disabled")),
        ("snapshot", Js::Bool(false)),
        ("autoupdate", Js::Bool(false)),
        (
            "compaction",
            Js::obj([("auto", Js::Bool(false)), ("prune", Js::Bool(false))]),
        ),
        ("permission", deny()),
        ("default_agent", Js::str("kassiber_selected_context")),
        (
            "agent",
            Js::obj([
                ("title", disabled()),
                ("summary", disabled()),
                ("compaction", disabled()),
                (
                    "kassiber_selected_context",
                    Js::obj([
                        ("mode", Js::str("primary")),
                        ("model", Js::str(model)),
                        ("permission", deny()),
                    ]),
                ),
            ]),
        ),
    ]))
}

/// `verifySensitiveOpenCodeConfig(config, model)`: fail closed when a later
/// configuration layer changed anything the lockdown relies on.
pub fn verify_sensitive_config(config: &Js, model: &str) -> Result<(), BrokerFailure> {
    let is =
        |value: Option<&Js>, expected: &Js| value.is_some_and(|value| value.strict_eq(expected));
    let text = |value: &str| Js::str(value);
    let agents = config.get("agent");
    let compaction = config.get("compaction");
    let selected = agents.and_then(|agents| agents.get("kassiber_selected_context"));
    let permission = config.get("permission");
    let selected_permission = selected.and_then(|selected| selected.get("permission"));
    let disabled = |name: &str| {
        is(
            agents
                .and_then(|agents| agents.get(name))
                .and_then(|agent| agent.get("disable")),
            &Js::Bool(true),
        )
    };
    let all_deny = |value: Option<&Js>| match value {
        Some(Js::Obj(object)) => object
            .iter()
            .all(|(_, value)| value.strict_eq(&text("deny"))),
        Some(Js::Arr(items)) => items.iter().all(|value| value.strict_eq(&text("deny"))),
        Some(Js::Str(chars)) => chars.chars().all(|_| false),
        _ => true,
    };
    let ok = is(config.get("model"), &text(model))
        && is(config.get("small_model"), &text(model))
        && is(config.get("share"), &text("disabled"))
        && is(config.get("snapshot"), &Js::Bool(false))
        && is(compaction.and_then(|c| c.get("auto")), &Js::Bool(false))
        && is(compaction.and_then(|c| c.get("prune")), &Js::Bool(false))
        && is(
            config.get("default_agent"),
            &text("kassiber_selected_context"),
        )
        && is(selected.and_then(|s| s.get("model")), &text(model))
        && is(permission.and_then(|p| p.get("*")), &text("deny"))
        && is(selected_permission.and_then(|p| p.get("*")), &text("deny"))
        && disabled("title")
        && disabled("summary")
        && disabled("compaction")
        && all_deny(selected_permission)
        && all_deny(permission);
    if ok {
        Ok(())
    } else {
        Err(fail("OpenCode configuration overrides prevent private selected-context execution; no selected data was sent."))
    }
}

/// `probeEnvironment(executable, args, cwd, env)`: a short OpenCode run
/// whose trimmed stdout is the answer. Bounded in time and size; any failure
/// says only that the privacy check failed.
fn probe_environment(
    executable: &Path,
    args: &[&str],
    cwd: &Path,
    env: &Env,
) -> Result<String, BrokerFailure> {
    const BOUND: usize = 1_000_000;
    let failed = || fail("OpenCode privacy verification failed.");
    let mut command = provider_command(PROVIDER, executable, args, Some(cwd), Some(env.as_slice()));
    command.stdout(Stdio::piped());
    let mut child = command.spawn().map_err(|_| failed())?;
    let stdout = child.stdout.take();
    let child = track_child(child);
    let (sender, done) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut output = Vec::new();
        let mut exceeded = false;
        if let Some(mut stdout) = stdout {
            let mut chunk = [0u8; 8192];
            loop {
                match stdout.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(read) => {
                        if !exceeded {
                            output.extend_from_slice(&chunk[..read]);
                            if output.len() > BOUND
                                && super::js::utf16_len(&String::from_utf8_lossy(&output)) > BOUND
                            {
                                exceeded = true;
                                let _ = sender.send(());
                            }
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(_) => break,
                }
            }
        }
        (output, exceeded)
    });
    let stop = |child: &crate::cleanup::TrackedChild| {
        #[cfg(unix)]
        let _ = child.terminate();
        #[cfg(not(unix))]
        let _ = child.kill();
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if done.try_recv().is_ok() {
            stop(&child);
            return Err(fail("OpenCode privacy verification exceeded its bound."));
        }
        if reader.is_finished() && !matches!(child.try_wait(), Ok(None)) {
            break;
        }
        if Instant::now() >= deadline {
            stop(&child);
            return Err(fail("OpenCode privacy verification timed out."));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    let (output, exceeded) = reader.join().map_err(|_| failed())?;
    if exceeded {
        return Err(fail("OpenCode privacy verification exceeded its bound."));
    }
    let status = child.wait().map_err(|_| failed())?;
    if status.code() != Some(0) {
        return Err(failed());
    }
    Ok(js_trim(&String::from_utf8_lossy(&output)).to_owned())
}

/// Environment variables that would turn OpenCode sharing on. A share
/// uploads the session (prompts, answers, tool results) to a public link.
pub const SHARE_ENV: [&str; 1] = ["OPENCODE_AUTO_SHARE"];

/// `parseInlineConfig(env)`: the user's `OPENCODE_CONFIG_CONTENT`, which
/// must be a JSON object.
pub fn parse_inline_config(env: &Env) -> Result<Obj, BrokerFailure> {
    let isolated = || fail("OpenCode configuration could not be isolated.");
    let Some(content) = env
        .get_str("OPENCODE_CONFIG_CONTENT")
        .filter(|c| !c.is_empty())
    else {
        return Ok(Obj::new());
    };
    match parse(&content) {
        Some(Js::Obj(object)) => Ok(object),
        _ => Err(isolated()),
    }
}

/// `withoutSharing(env)`: every OpenCode turn runs with sharing off,
/// whatever the user's config or environment asks for: the inline config
/// (OpenCode's last local merge) says `share: "disabled"`,
/// `OPENCODE_DISABLE_SHARE` turns the share service off inside the process,
/// and share-enabling variables are dropped.
pub fn without_sharing(mut env: Env) -> Result<Env, BrokerFailure> {
    for name in SHARE_ENV {
        env.remove(name);
    }
    let mut config = parse_inline_config(&env)?;
    config.set("share", Js::str("disabled"));
    // The deprecated spelling, for releases that read only that.
    config.set("autoshare", Js::Bool(false));
    env.set("OPENCODE_CONFIG_CONTENT", stringify(&Js::Obj(config)));
    env.set("OPENCODE_DISABLE_SHARE", "1");
    Ok(env)
}

/// `verifyOpenCodeSharingOff(config)`: fail closed if a later config layer
/// (managed or organisation) re-enabled sharing.
pub fn verify_sharing_off(config: &Js) -> Result<(), BrokerFailure> {
    if config.get("share").and_then(Js::as_str) != Some("disabled") {
        return Err(fail(
            "OpenCode would share this session; Kassiber stopped before sending it.",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(target, link)
}

/// `mkdir(path, { recursive: true, mode: 0o700 })`.
pub fn make_private_dirs(path: &Path) -> Result<(), BrokerFailure> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
        .create(path)
        .map_err(|error| fs_failure("mkdir", path, &error))
}

/// `isolateOpenCodeStorage(env, cwd)`: OpenCode's data and state homes
/// inside the turn's directory (removed with it) and its database in
/// memory, keeping only the user's sign-in.
fn isolate_storage(host: &Host, env: &mut Env, cwd: &Path) -> Result<(), BrokerFailure> {
    let runtime = cwd.join("private-runtime");
    let data = runtime.join("data").join("opencode");
    make_private_dirs(&data.join("log"))?;
    // 1.18.4 always creates a file logger, including with --print-logs. A
    // sink is installed BEFORE launching it; no selected content is written
    // then deleted.
    let sink = if host.windows {
        Path::new(r"\\.\NUL")
    } else {
        Path::new("/dev/null")
    };
    let log = data.join("log").join("opencode.log");
    symlink(sink, &log).map_err(|error| fs_failure("symlink", &log, &error))?;
    let original = host
        .var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| host.home().join(".local").join("share"));
    let auth = original.join("opencode").join("auth.json");
    let linked = std::fs::symlink_metadata(&auth)
        .and_then(|_| std::fs::metadata(&auth))
        .and_then(|_| symlink(&auth, &data.join("auth.json")));
    match linked {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => {
            return Err(fail("OpenCode authentication isolation failed."));
        }
        _ => {}
    }
    env.set("XDG_DATA_HOME", runtime.join("data"));
    env.set("XDG_STATE_HOME", runtime.join("state"));
    env.set("OPENCODE_DB", ":memory:");
    Ok(())
}

/// `sensitiveOpenCodeEnvironment(executable, cwd, model)`.
pub fn sensitive_environment(
    host: &Host,
    executable: &Path,
    cwd: &Path,
    model: &str,
) -> Result<Env, BrokerFailure> {
    let mut env = host.provider_env();
    // The storage implementation is audited against this release. Unknown
    // storage semantics fail before a prompt, rather than guessing from a
    // newer version.
    if probe_environment(executable, &["--version"], cwd, &env)? != "1.18.4" {
        return Err(fail(
            "OpenCode sensitive context requires the audited 1.18.4 runtime.",
        ));
    }
    isolate_storage(host, &mut env, cwd)?;
    let mut config = parse_inline_config(&env)?;
    for (key, value) in sensitive_config(model)?.iter() {
        config.set(key, value.clone());
    }
    for (name, value) in [
        ("OPENCODE_DISABLE_AUTOUPDATE", "1"),
        ("OPENCODE_DISABLE_AUTOCOMPACT", "1"),
        ("OPENCODE_DISABLE_PRUNE", "1"),
        ("OPENCODE_DISABLE_MODELS_FETCH", "1"),
        ("OPENCODE_AUTO_SHARE", "0"),
        ("OPENCODE_PURE", "1"),
        ("OPENCODE_PRINT_LOGS", "0"),
        ("OPENCODE_LOG_LEVEL", "ERROR"),
    ] {
        env.set(name, value);
    }
    env.set("OPENCODE_CONFIG_CONTENT", stringify(&Js::Obj(config)));
    if probe_environment(executable, &["db", "path"], cwd, &env)? != ":memory:" {
        return Err(fail(
            "OpenCode did not enable its in-memory database; no selected data was sent.",
        ));
    }
    Ok(env)
}

/// `checkIncognitoPlatform(platform)`: keeping OpenCode's always-on log
/// file off disk needs a file symlink to the null device, which a normal
/// Windows account cannot create (no elevation or Developer Mode), and
/// OpenCode has no option to turn the file off. Say so before anything
/// starts rather than fail obscurely or write the log.
pub fn check_incognito_platform(windows: bool) -> Result<(), BrokerFailure> {
    if windows {
        return Err(fail(
            "Incognito isn't available for OpenCode on Windows; turn Incognito off or choose another provider.",
        ));
    }
    Ok(())
}

/// `statelessOpenCodeEnvironment(executable, cwd)`: an Incognito turn. The
/// session lives in an in-memory database and the turn's directory, so
/// OpenCode keeps nothing once the turn ends. Tools and the user's
/// configuration are unchanged.
pub fn stateless_environment(
    host: &Host,
    executable: &Path,
    cwd: &Path,
) -> Result<Env, BrokerFailure> {
    check_incognito_platform(host.windows)?;
    let mut env = host.provider_env();
    isolate_storage(host, &mut env, cwd)?;
    if probe_environment(executable, &["db", "path"], cwd, &env)? != ":memory:" {
        return Err(fail(
            "OpenCode did not enable its in-memory database; nothing was sent.",
        ));
    }
    Ok(env)
}

/// `isLoopbackEndpoint(baseUrl)`: loopback endpoints are the only ones we
/// can call local: the model never leaves the machine. Anything else,
/// including a LAN address, is remote.
pub fn is_loopback_endpoint(base_url: &str) -> bool {
    let Ok(url) = kassiber_net::HttpUrl::parse(base_url) else {
        return false;
    };
    let host = url.host_str().unwrap_or_default().to_lowercase();
    // URL keeps IPv6 hosts in brackets.
    let bare = host
        .strip_prefix('[')
        .unwrap_or(&host)
        .strip_suffix(']')
        .unwrap_or(host.strip_prefix('[').unwrap_or(&host));
    if bare == "localhost" || bare.ends_with(".localhost") || bare == "::1" {
        return true;
    }
    // 127.0.0.0/8, matched as a literal IPv4 address. A prefix test also
    // accepts hostnames such as `127.api.example.com`, which resolve
    // publicly; that would badge a remote model as local.
    let octets: Vec<&str> = bare.split('.').collect();
    octets.len() == 4
        && octets.iter().all(|octet| {
            (1..=3).contains(&octet.len())
                && octet.bytes().all(|byte| byte.is_ascii_digit())
                && octet.parse::<u16>().is_ok_and(|value| value <= 255)
        })
        && octets[0].parse::<u16>() == Ok(127)
}

/// A JavaScript `TypeError` reading a property of `null` or `undefined`.
fn type_error(base: &str, property: &str) -> BrokerFailure {
    BrokerFailure::new(format!(
        "Cannot read properties of {base} (reading '{property}')"
    ))
}

/// `loadProviderEndpoints(executable, cwd)`: the resolved
/// `provider.<id>.options.baseURL` per source provider, straight from
/// OpenCode's own merged config, so a model routed to a local runtime is not
/// mislabelled remote just because OpenCode proxies it.
///
/// Only the baseURL is read. That options block also holds `apiKey` for
/// some providers, which must never leave this function.
pub fn load_provider_endpoints(
    host: &Host,
    executable: &Path,
    cwd: &Path,
) -> Result<HashMap<String, String>, BrokerFailure> {
    let mut endpoints = HashMap::new();
    let env = host.provider_env();
    let run = || {
        run_provider(
            PROVIDER,
            executable,
            &["debug", "config"],
            RunOptions {
                cwd: Some(cwd),
                env: Some(env.as_slice()),
                ..RunOptions::default()
            },
        )
    };
    // `opencode debug config` intermittently exits non-zero with empty
    // stdout when other opencode processes are running concurrently. An
    // empty result here is not neutral: it downgrades a genuinely local
    // model's badge to remote. Fail safe but retry once.
    let mut result = run()?;
    if result.code != Some(0) || js_trim(&result.stdout).is_empty() {
        result = run()?;
    }
    if result.code != Some(0) {
        return Ok(endpoints);
    }
    let Some(config) = parse(&result.stdout) else {
        return Ok(endpoints);
    };
    if config == Js::Null {
        return Err(type_error("null", "provider"));
    }
    let entries: Vec<(String, &Js)> = match config.get("provider") {
        Some(Js::Obj(providers)) => providers
            .iter()
            .map(|(name, entry)| (name.to_owned(), entry))
            .collect(),
        Some(Js::Arr(providers)) => providers
            .iter()
            .enumerate()
            .map(|(index, entry)| (index.to_string(), entry))
            .collect(),
        _ => return Ok(endpoints),
    };
    for (name, entry) in entries {
        if *entry == Js::Null {
            return Err(type_error("null", "options"));
        }
        let base_url = entry
            .get("options")
            .and_then(|options| options.get("baseURL"))
            .and_then(Js::as_str)
            .map(js_trim)
            .filter(|url| !url.is_empty());
        if let Some(base_url) = base_url {
            endpoints.insert(name, base_url.to_owned());
        }
    }
    Ok(endpoints)
}

/// `\x1b[...m` colour codes.
fn strip_ansi(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(at) = rest.find('\x1b') {
        out.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let code = after.strip_prefix('[').map(|body| {
            let params = body
                .bytes()
                .take_while(|byte| byte.is_ascii_digit() || *byte == b';')
                .count();
            (body.as_bytes().get(params) == Some(&b'm')).then_some(params)
        });
        match code {
            Some(Some(params)) => rest = &after[1 + params + 1..],
            _ => {
                out.push('\x1b');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `/^[^\s/]+\/[^\s]+$/`: a provider segment plus an arbitrary remainder.
fn model_row(line: &str) -> bool {
    let Some((provider, rest)) = line.split_once('/') else {
        return false;
    };
    !provider.is_empty()
        && !provider.contains(js_space)
        && !rest.is_empty()
        && !rest.contains(js_space)
}

/// `parseOpenCodeModels(stdout, endpoints)`: `opencode models --verbose`.
pub fn parse_models(
    stdout: &str,
    endpoints: &HashMap<String, String>,
) -> Result<Vec<BrokerModel>, BrokerFailure> {
    let mut rows = Vec::new();
    let mut id: Option<String> = None;
    let mut json_lines: Vec<String> = Vec::new();
    let mut flush = |id: &mut Option<String>, json_lines: &mut Vec<String>| {
        let Some(row) = id.take() else {
            return Ok(());
        };
        // Plain `opencode models` output has no JSON metadata.
        let metadata = parse(&json_lines.join("\n")).unwrap_or(Js::Obj(Obj::new()));
        json_lines.clear();
        if metadata == Js::Null {
            return Err(type_error("null", "variants"));
        }
        let variants: Vec<String> = match metadata.get("variants") {
            Some(Js::Obj(variants)) => variants.iter().map(|(key, _)| key.to_owned()).collect(),
            _ => Vec::new(),
        };
        let source = row.split('/').next().unwrap_or(&row).to_owned();
        // The model row's own `api.url` is empty for locally-served models,
        // so the route is judged by the source provider's configured
        // endpoint instead. Unknown endpoint stays remote, the conservative
        // default.
        let endpoint = endpoints.get(&source);
        let local = endpoint.is_some_and(|endpoint| is_loopback_endpoint(endpoint));
        rows.push(BrokerModel {
            display_name: Some(match metadata.get("name") {
                Some(Js::Str(name)) => name.clone(),
                _ => row.split('/').nth(1).unwrap_or_default().to_owned(),
            }),
            owned_by: Some(format!("OpenCode · {source}")),
            source_provider: Some(source.clone()),
            privacy_posture: Some(if local {
                ModelPosture::Local
            } else {
                ModelPosture::Remote
            }),
            privacy_reason: Some(if local {
                format!("OpenCode routes {source} to a loopback endpoint on this machine.")
            } else if endpoint.is_none() {
                "OpenCode does not report an endpoint for this provider.".to_owned()
            } else {
                format!("OpenCode routes {source} off this machine.")
            }),
            supports_reasoning_effort: Some(!variants.is_empty()),
            reasoning_efforts: Some(variants),
            id: row,
            ..BrokerModel::default()
        });
        Ok(())
    };
    for line in split_lines(stdout) {
        let line = strip_ansi(line);
        let line = line.trim_end_matches(js_space);
        // Namespaced ids such as `openrouter/anthropic/claude-x` are valid and
        // must not be dropped, which would silently empty the inventory and
        // read as "authentication required".
        if model_row(line) {
            flush(&mut id, &mut json_lines)?;
            id = Some(line.to_owned());
        } else if id.is_some() {
            json_lines.push(line.to_owned());
        }
    }
    flush(&mut id, &mut json_lines)?;
    Ok(rows)
}

/// `text.split(/\r?\n/)`.
fn split_lines(text: &str) -> impl Iterator<Item = &str> {
    text.split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line))
}

/// The OpenCode 1 half of `openCodeStatus`.
pub fn models(
    host: &Host,
    executable: &Path,
    cwd: &Path,
) -> Result<Vec<BrokerModel>, BrokerFailure> {
    let endpoints = load_provider_endpoints(host, executable, cwd)?;
    let env = host.provider_env();
    let result = run_provider(
        PROVIDER,
        executable,
        &["models", "--verbose"],
        RunOptions {
            cwd: Some(cwd),
            env: Some(env.as_slice()),
            ..RunOptions::default()
        },
    )?;
    if result.code != Some(0) {
        return Err(fail(
            "OpenCode could not list models. Check its CLI version and configuration outside Kassiber.",
        ));
    }
    parse_models(&result.stdout, &endpoints)
}

/// `startServer(executable, cwd, env)`.
fn start_server(
    host: &Host,
    executable: &Path,
    cwd: &Path,
    env: &Env,
) -> Result<(ServerProcess, Loopback), BrokerFailure> {
    let port = (host.free_port)().map_err(|error| BrokerFailure::new(error.to_string()))?;
    // --pure: global plugins otherwise execute outside the session's DENY_ALL
    // permission set, with the full provider environment, before any
    // session permission applies.
    let args = [
        "serve".to_owned(),
        "--pure".to_owned(),
        "--hostname=127.0.0.1".to_owned(),
        format!("--port={port}"),
    ];
    let started = ServerProcess::start(executable, &args, cwd, env, true, |line| {
        line.to_lowercase()
            .contains("opencode server listening")
            .then_some(())
    });
    match started {
        Ok((server, ())) => Ok((server, Loopback::new(port))),
        Err(NotReady::Spawn(error)) => Err(spawn_failure(executable, &error)),
        Err(NotReady::Exited(code)) => Err(BrokerFailure::new(format!(
            "OpenCode server exited with code {}.",
            code.map_or("null".to_owned(), |code| code.to_string())
        ))),
        Err(NotReady::TimedOut) => Err(fail("OpenCode server startup timed out.")),
    }
}

/// A response body as the SDK decoded it.
#[derive(Debug)]
enum Data {
    /// `null` (an empty body of a stream-typed response).
    Null,
    Json(Js),
    /// A body the SDK kept as a stream, blob or form: truthy, and none of
    /// the fields the adapter reads.
    Opaque,
}

impl Data {
    fn get(&self, key: &str) -> Option<&Js> {
        match self {
            Data::Json(value) => value.get(key),
            _ => None,
        }
    }

    fn truthy(&self) -> bool {
        match self {
            Data::Null => false,
            Data::Json(value) => value.truthy(),
            Data::Opaque => true,
        }
    }

    /// `data ?? {}` as a JSON value.
    fn or_empty(self) -> Js {
        match self {
            Data::Json(value) => value,
            Data::Null => Js::Obj(Obj::new()),
            // No field of a stream reads as set.
            Data::Opaque => Js::Arr(Vec::new()),
        }
    }
}

/// What the SDK did with a response, by its `Content-Type`.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ParseAs {
    Json,
    Text,
    Opaque,
    Stream,
}

fn parse_as(content_type: Option<&str>) -> ParseAs {
    let Some(content_type) = content_type else {
        return ParseAs::Stream;
    };
    let clean = js_trim(content_type.split(';').next().unwrap_or_default());
    if clean.is_empty() {
        return ParseAs::Json;
    }
    if clean.starts_with("application/json") || clean.ends_with("+json") {
        ParseAs::Json
    } else if clean == "multipart/form-data"
        || ["application/", "audio/", "image/", "video/"]
            .iter()
            .any(|prefix| clean.starts_with(prefix))
    {
        ParseAs::Opaque
    } else if clean.starts_with("text/") {
        ParseAs::Text
    } else {
        ParseAs::Json
    }
}

/// The SDK's view of the loopback API: the directory travels as a query
/// parameter on reads and as `x-opencode-directory` otherwise; failures read
/// as the SDK's thrown errors.
struct Sdk {
    loopback: Loopback,
    directory: String,
}

/// undici's default header and body timeouts.
const FETCH_TIMEOUT: Duration = Duration::from_secs(300);

impl Sdk {
    fn url(&self, method: &str, path: &str) -> String {
        if method == "GET" {
            format!(
                "{}{path}?directory={}",
                self.loopback.origin(),
                form_encode(&self.directory)
            )
        } else {
            format!("{}{path}", self.loopback.origin())
        }
    }

    fn call(&self, method: &str, path: &str, body: Option<&Js>) -> Result<Data, BrokerFailure> {
        let fetch_failed = || fail("fetch failed");
        let url = self.url(method, path);
        let target = &url[self.loopback.origin().len()..];
        let mut headers = Vec::new();
        if body.is_some() {
            headers.push(("Content-Type", "application/json".to_owned()));
        }
        if method != "GET" {
            headers.push((
                "x-opencode-directory",
                encode_uri_component(&self.directory),
            ));
        }
        let mut stream = self
            .loopback
            .open(
                method,
                target,
                Call {
                    headers,
                    body: body.map(|body| stringify(body).into_bytes()),
                    timeout: Some(FETCH_TIMEOUT),
                    ..Call::default()
                },
            )
            .map_err(|_| fetch_failed())?;
        let content_type = stream.header("content-type").map(str::to_owned);
        if content_type.as_deref() == Some("text/html") {
            return Err(fail("Request is not supported by this version of OpenCode Server (Server responded with text/html)"));
        }
        let status = stream.status;
        let body = stream.read_to_end().map_err(|_| fetch_failed())?;
        if (200..300).contains(&status) {
            let parse_as = parse_as(content_type.as_deref());
            let empty = status == 204 || stream.header("content-length") == Some("0");
            return Ok(match parse_as {
                ParseAs::Json if empty || body.is_empty() => Data::Json(Js::Obj(Obj::new())),
                ParseAs::Json => Data::Json(
                    parse(&String::from_utf8_lossy(&body))
                        .ok_or_else(|| fail("OpenCode returned an invalid API response."))?,
                ),
                ParseAs::Text => Data::Json(Js::Str(String::from_utf8_lossy(&body).into_owned())),
                ParseAs::Opaque => Data::Opaque,
                ParseAs::Stream if status == 204 => Data::Null,
                ParseAs::Stream => Data::Opaque,
            });
        }
        let text = String::from_utf8_lossy(&body).into_owned();
        let error = match parse(&text) {
            None | Some(Js::Null) => Js::Str(text),
            Some(value) => value,
        };
        let describe = format!(
            "{method} {url} → {status}{}",
            if stream.reason.is_empty() {
                String::new()
            } else {
                format!(" {}", stream.reason)
            }
        );
        let nonempty = |value: Option<&Js>| {
            value
                .and_then(Js::as_str)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        let message = match &error {
            Js::Obj(object) if !object.is_empty() => {
                nonempty(error.get("data").and_then(|data| data.get("message")))
                    .or_else(|| nonempty(error.get("message")))
                    .or_else(|| nonempty(error.get("name")))
                    .unwrap_or(describe)
            }
            Js::Arr(items) if !items.is_empty() => describe,
            Js::Str(text) if !text.is_empty() => text.clone(),
            _ => format!("opencode server {describe}: (empty response body)"),
        };
        Err(BrokerFailure::new(message))
    }
}

/// `parseInt(text, 10)` after the SSE field name and its spaces.
fn parse_int(text: &str) -> Option<i64> {
    let text = js_trim(text);
    let (sign, digits) = match text.strip_prefix('-') {
        Some(rest) => (-1, rest),
        None => (1, text.strip_prefix('+').unwrap_or(text)),
    };
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    digits[..count]
        .parse::<i64>()
        .ok()
        .or(Some(i64::MAX))
        .map(|value| sign * value)
}

/// One `data` event of the SDK's server-sent event client.
enum SseEvent {
    Json(Js),
    Text,
}

/// What the event consumer saw first.
enum Opened {
    Stream,
    Failed(BrokerFailure),
}

/// The keys of the TypeScript `Map`s: any JavaScript value but an object
/// (two objects are never the same key).
fn map_key(value: Option<&Js>) -> Option<String> {
    Some(match value {
        None => "u".to_owned(),
        Some(Js::Null) => "null".to_owned(),
        Some(Js::Bool(flag)) => format!("b{flag}"),
        Some(Js::Num(number)) => format!("n{}", number_string(*number)),
        Some(Js::Str(text)) => format!("s{text}"),
        Some(Js::Arr(_) | Js::Obj(_)) => return None,
    })
}

/// The turn's event consumer: `client.event.subscribe()` and the loop over
/// its stream, ending with `done` on `session.idle`.
struct Consumer<'a> {
    sdk: &'a Sdk,
    session: String,
    allowed: HashSet<String>,
    events: &'a EventSink,
    cursor: Option<String>,
    cancel: &'a Cancel,
    stop: &'a AtomicBool,
}

impl Consumer<'_> {
    fn run(&self, opened: mpsc::Sender<Opened>) -> Result<(), BrokerFailure> {
        let mut roles: HashMap<String, Js> = HashMap::new();
        let mut emitted: HashMap<String, String> = HashMap::new();
        let mut retry_delay: i64 = 3000;
        let mut attempt: u32 = 0;
        let mut last_event_id: Option<String> = None;
        let mut announced = false;
        loop {
            if self.stop.load(Ordering::SeqCst) {
                return Err(fail("OpenCode event stream ended unexpectedly."));
            }
            attempt = attempt.saturating_add(1);
            let mut headers = Vec::new();
            if let Some(id) = &last_event_id {
                headers.push(("Last-Event-ID", id.clone()));
            }
            let target = format!("/event?directory={}", form_encode(&self.sdk.directory));
            let opened_stream = self.sdk.loopback.open(
                "GET",
                &target,
                Call {
                    headers,
                    timeout: Some(FETCH_TIMEOUT),
                    cancel: Some(self.cancel),
                    max_response_bytes: Some(usize::MAX),
                    ..Call::default()
                },
            );
            let ended = match opened_stream {
                Ok(stream) if (200..300).contains(&stream.status) => {
                    if !announced {
                        announced = true;
                        let _ = opened.send(Opened::Stream);
                    }
                    let mut frames = Frames::new(stream);
                    loop {
                        match frames.next(true, None) {
                            Ok(Some(frame)) => {
                                let mut data_lines = Vec::new();
                                for line in frame.split('\n') {
                                    if let Some(rest) = line.strip_prefix("data:") {
                                        data_lines
                                            .push(rest.trim_start_matches(js_space).to_owned());
                                    } else if let Some(rest) = line.strip_prefix("id:") {
                                        last_event_id =
                                            Some(rest.trim_start_matches(js_space).to_owned());
                                    } else if let Some(rest) = line.strip_prefix("retry:") {
                                        if let Some(delay) = parse_int(rest) {
                                            retry_delay = delay;
                                        }
                                    }
                                }
                                if data_lines.is_empty() {
                                    continue;
                                }
                                let raw = data_lines.join("\n");
                                let event = match parse(&raw) {
                                    Some(value) => SseEvent::Json(value),
                                    None => SseEvent::Text,
                                };
                                if let Some(done) = self.handle(event, &mut roles, &mut emitted)? {
                                    return Ok(done);
                                }
                            }
                            Ok(None) => break true,
                            Err(FrameError::Read(_) | FrameError::TooLarge) => break false,
                        }
                    }
                }
                _ => false,
            };
            if ended {
                // Exit the retry loop on normal completion.
                return Err(fail("OpenCode event stream ended unexpectedly."));
            }
            // Connection failed or aborted; retry after an exponential backoff
            // capped at 30 seconds.
            let factor = 2i64.saturating_pow(attempt.saturating_sub(1));
            let backoff = retry_delay.saturating_mul(factor).clamp(0, 30_000);
            let until = Instant::now() + Duration::from_millis(backoff as u64);
            while Instant::now() < until {
                if self.stop.load(Ordering::SeqCst) {
                    return Err(fail("OpenCode event stream ended unexpectedly."));
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }

    /// One event; `Some(())` once the turn is done.
    fn handle(
        &self,
        event: SseEvent,
        roles: &mut HashMap<String, Js>,
        emitted: &mut HashMap<String, String>,
    ) -> Result<Option<()>, BrokerFailure> {
        let invalid = || fail("OpenCode returned an invalid event.");
        let event = match event {
            SseEvent::Json(value @ (Js::Obj(_) | Js::Arr(_))) => value,
            // `"properties" in event` throws on anything but an object.
            SseEvent::Json(_) | SseEvent::Text => return Err(invalid()),
        };
        let Some(properties @ (Js::Obj(_) | Js::Arr(_))) = event.get("properties") else {
            return Ok(None);
        };
        let session = properties.get("sessionID").and_then(Js::as_str);
        if let Some(session) = session.filter(|id| !id.is_empty()) {
            if session != self.session {
                return Ok(None);
            }
        }
        match event.get("type").and_then(Js::as_str) {
            Some("message.updated") => {
                let info = properties.get("info");
                match info {
                    None => return Err(type_error("undefined", "id")),
                    Some(Js::Null) => return Err(type_error("null", "id")),
                    Some(info) => {
                        if let Some(key) = map_key(info.get("id")) {
                            roles.insert(key, info.get("role").cloned().unwrap_or(Js::Null));
                        }
                    }
                }
            }
            Some("message.part.updated") => {
                let part = match properties.get("part") {
                    None => return Err(type_error("undefined", "type")),
                    Some(Js::Null) => return Err(type_error("null", "type")),
                    Some(part) => part,
                };
                if part.get("type").and_then(Js::as_str) == Some("tool") {
                    let tool = part.get("tool").and_then(Js::as_str);
                    if !tool.is_some_and(|tool| self.allowed.contains(tool)) {
                        return Err(fail(
                            "OpenCode attempted to use a provider-native tool; Kassiber stopped it.",
                        ));
                    }
                    return Ok(None);
                }
                let role = map_key(part.get("messageID")).and_then(|key| roles.get(&key));
                if role.and_then(Js::as_str) != Some("assistant") {
                    return Ok(None);
                }
                let kind = part.get("type").and_then(Js::as_str);
                let (Some(kind @ ("text" | "reasoning")), Some(Js::Str(text))) =
                    (kind, part.get("text"))
                else {
                    return Ok(None);
                };
                let key = map_key(part.get("id"));
                let previous = key
                    .as_ref()
                    .and_then(|key| emitted.get(key))
                    .cloned()
                    .unwrap_or_default();
                let delta = match text.strip_prefix(previous.as_str()) {
                    Some(rest) => rest.to_owned(),
                    None => text.clone(),
                };
                if let Some(key) = key {
                    emitted.insert(key, text.clone());
                }
                if delta.is_empty() {
                    return Ok(None);
                }
                self.events.write(&if kind == "reasoning" {
                    Event::Delta {
                        content: None,
                        reasoning: Some(delta),
                    }
                } else {
                    Event::Delta {
                        content: Some(delta),
                        reasoning: None,
                    }
                });
            }
            Some("session.idle") => {
                self.events.write(&Event::Done {
                    finish_reason: "stop".into(),
                    provider_session_id: self.cursor.clone(),
                });
                return Ok(Some(()));
            }
            Some("session.error") => return Err(fail("OpenCode reported a provider error.")),
            _ => {}
        }
        Ok(None)
    }
}

/// The OpenCode 1 half of `openCodeChat`.
#[allow(clippy::too_many_arguments)]
pub fn chat(
    host: &Host,
    executable: &Path,
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &EventSink,
    sensitive: bool,
    stateless: bool,
) -> Result<(), BrokerFailure> {
    let env = if sensitive {
        sensitive_environment(host, executable, cwd, &request.model)?
    } else if stateless {
        stateless_environment(host, executable, cwd)?
    } else {
        host.provider_env()
    };
    events.write(&Event::Status {
        phase: "connecting".into(),
        message: "Starting OpenCode server".into(),
    });
    let (server, loopback) = start_server(host, executable, cwd, &without_sharing(env)?)?;
    let sdk = Sdk {
        loopback,
        directory: cwd.to_string_lossy().into_owned(),
    };
    let result = turn(&sdk, request, cwd, bridge, events, sensitive, stateless);
    server.terminate();
    match result {
        Err(_) if sensitive => Err(fail(
            "OpenCode sensitive-context request failed; no provider diagnostic content was retained.",
        )),
        result => result,
    }
}

fn turn(
    sdk: &Sdk,
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &EventSink,
    sensitive: bool,
    stateless: bool,
) -> Result<(), BrokerFailure> {
    let effective = sdk.call("GET", "/config", None)?.or_empty();
    verify_sharing_off(&effective)?;
    if sensitive {
        verify_sensitive_config(&effective, &request.model)?;
    }
    if let Some(bridge) = bridge.filter(|_| !request.tools.is_empty()) {
        let command = mcp_command(cwd, &request.tools, bridge)?;
        sdk.call(
            "POST",
            "/mcp",
            Some(&Js::obj([
                ("name", Js::str("kassiber")),
                (
                    "config",
                    Js::obj([
                        ("type", Js::str("local")),
                        (
                            "command",
                            Js::Arr(command.into_iter().map(Js::Str).collect()),
                        ),
                        ("enabled", Js::Bool(true)),
                    ]),
                ),
            ])),
        )?;
    }
    let permissions = permissions_for(request);
    let allowed: HashSet<String> = request
        .tools
        .iter()
        .map(|tool| format!("kassiber_{}", tool.name))
        .collect();
    let resume_id = safe_session_cursor(request.option("provider_session_id"));
    let mut session: Option<Data> = None;
    let mut resumed = false;
    if let Some(resume_id) = &resume_id {
        let path = format!("/session/{}", encode_uri_component(resume_id));
        // OpenCode may have pruned the prior session; start a fresh one.
        if let Ok(existing) = sdk.call("GET", &path, None) {
            if existing.truthy()
                && sdk
                    .call(
                        "PATCH",
                        &path,
                        Some(&Js::obj([("permission", permissions.clone())])),
                    )
                    .is_ok()
            {
                session = Some(existing);
                resumed = true;
            }
        }
    }
    let session = match session {
        Some(session) => session,
        None => {
            let mut body = Obj::from_entries([("permission", permissions.clone())]);
            if sensitive {
                body.set("title", Js::str("Kassiber selected context"));
            }
            sdk.call("POST", "/session", Some(&Js::Obj(body)))?
        }
    };
    let not_created = || fail("OpenCode did not create a chat session.");
    if !session.truthy() {
        return Err(not_created());
    }
    // The SDK would carry on with an undefined id; refuse instead.
    let session = session
        .get("id")
        .and_then(Js::as_str)
        .map(str::to_owned)
        .ok_or_else(not_created)?;

    let cancel = Cancel::new();
    let stop = AtomicBool::new(false);
    let consumer = Consumer {
        sdk,
        session: session.clone(),
        allowed,
        events,
        cursor: (!sensitive && !stateless).then(|| session.clone()),
        cancel: &cancel,
        stop: &stop,
    };
    std::thread::scope(|scope| {
        let (opened_sender, opened) = mpsc::channel();
        let (done_sender, done) = mpsc::channel();
        let consumer = &consumer;
        scope.spawn(move || {
            let result = consumer.run(opened_sender.clone());
            let _ = opened_sender.send(Opened::Failed(
                result.clone().err().unwrap_or_else(|| fail("finished")),
            ));
            let _ = done_sender.send(result);
        });
        let outcome = (|| {
            let (provider_id, model_id) = split_model(&request.model)?;
            // The SDK starts the subscription before the prompt; wait until
            // the stream is open so no early event is missed.
            match opened.recv() {
                Ok(Opened::Stream) => {}
                Ok(Opened::Failed(failure)) => return Err(failure),
                Err(_) => return Err(fail("OpenCode event stream ended unexpectedly.")),
            }
            let mut body = Obj::from_entries([(
                "model",
                Js::obj([
                    ("providerID", Js::str(provider_id)),
                    ("modelID", Js::str(model_id)),
                ]),
            )]);
            if sensitive {
                body.set("agent", Js::str("kassiber_selected_context"));
            }
            body.set("system", Js::str(system_instructions(request)));
            if let Some(effort) = request
                .option("reasoning_effort")
                .filter(|effort| truthy(effort) && effort.as_str() != Some("auto"))
            {
                body.set("variant", Js::from_serde(effort));
            }
            let mut parts = vec![Js::obj([
                ("type", Js::str("text")),
                (
                    "text",
                    Js::str(prompt_from_messages(&request.messages, resumed)),
                ),
            ])];
            // Inline data URLs only: never a path OpenCode would read itself.
            for (index, image) in prompt_images(&request.messages, resumed)?
                .iter()
                .enumerate()
            {
                let extension = image.media_type.split('/').nth(1).unwrap_or_default();
                parts.push(Js::obj([
                    ("type", Js::str("file")),
                    ("mime", Js::str(image.media_type.clone())),
                    (
                        "filename",
                        Js::str(format!("image-{}.{extension}", index + 1)),
                    ),
                    (
                        "url",
                        Js::str(format!("data:{};base64,{}", image.media_type, image.data)),
                    ),
                ]));
            }
            body.set("parts", Js::Arr(parts));
            let path = format!("/session/{}/prompt_async", encode_uri_component(&session));
            if let Err(failure) = sdk.call("POST", &path, Some(&Js::Obj(body))) {
                // A consumer failure that came first is the turn's failure.
                return Err(match done.try_recv() {
                    Ok(Err(consumer_failure)) => consumer_failure,
                    _ => failure,
                });
            }
            done.recv()
                .unwrap_or_else(|_| Err(fail("OpenCode event stream ended unexpectedly.")))
        })();
        stop.store(true, Ordering::SeqCst);
        cancel.cancel();
        outcome
    })
}
