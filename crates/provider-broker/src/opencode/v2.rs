//! OpenCode 2 (`opencode2.ts`): its HTTP API, with no SDK and no
//! provider-native tool dispatch.
//!
//! The server runs `serve --stdio` on an announced loopback port behind a
//! random password, with its log sinks pre-installed as `/dev/null` links,
//! and a configuration built in RAM: only the user's provider settings are
//! read, so user plugins, MCP servers and instructions never load. Private
//! (Incognito or selected-data) turns run against an in-memory database
//! that receives only the selected integration's active key.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use kassiber_net::Cancel;

use super::http::{form_encode, Call, FrameError, Frames, Loopback};
use super::js::{encode_uri_component, object, parse, stringify, utf16_len, Js, Obj};
use super::v1::{is_loopback_endpoint, make_private_dirs};
use super::{model_separator, Env, Host, NotReady, ServerProcess};
use crate::bridge::mcp_command;
use crate::executables::fs_failure;
use crate::prompt::{prompt_from_messages, prompt_images, system_instructions};
use crate::protocol::{
    js_space, safe_session_cursor, truthy, BrokerFailure, BrokerModel, ChatRequest, Event,
    EventSink, ModelPosture,
};
use crate::workdir::random_bytes;
use crate::ToolBridge;

fn fail(message: &str) -> BrokerFailure {
    BrokerFailure::new(message)
}

const AGENT: &str = "kassiber";
/// Bounds API responses, event buffers and configuration files.
pub const LIMIT: usize = 2 * 1024 * 1024;
const PLUGINS: [&str; 6] = [
    "-*",
    "opencode.models-dev",
    "opencode.provider.*",
    "opencode.config.provider",
    "opencode.config.agent",
    "opencode.config.compaction",
];
const SINKS: [&str; 2] = ["opencode.log", "opencode-latest.log"];
const NULL_DEVICE: &str = "/dev/null";

/// `openCodeDiscardEnvironment(cwd)`: install the logger's discard sink
/// before starting any v2 process.
pub fn discard_environment(host: &Host, cwd: &Path) -> Result<Env, BrokerFailure> {
    if host.windows {
        return Err(fail(
            "OpenCode 2 requires a log discard sink unavailable on Windows. Choose another provider.",
        ));
    }
    let root = cwd.join("opencode-runtime");
    let logs = root.join("data").join("opencode").join("log");
    make_private_dirs(&logs)?;
    let unverified =
        || fail("OpenCode's log discard sink could not be verified; nothing was sent.");
    for name in SINKS {
        let path = logs.join(name);
        #[cfg(unix)]
        match std::os::unix::fs::symlink(NULL_DEVICE, &path) {
            Err(error) if error.kind() != std::io::ErrorKind::AlreadyExists => {
                return Err(fs_failure("symlink", &path, &error));
            }
            _ => {}
        }
        match std::fs::read_link(&path) {
            Ok(target) if target == Path::new(NULL_DEVICE) => {}
            _ => return Err(unverified()),
        }
    }
    let mut env = host.provider_env();
    env.set("XDG_DATA_HOME", root.join("data"));
    env.set("XDG_STATE_HOME", root.join("state"));
    env.set("XDG_CACHE_HOME", root.join("cache"));
    Ok(env)
}

/// Strip JSONC comments (pass 1) or trailing commas (pass 2) the way the
/// TypeScript regular expressions do: a quoted string is matched first and
/// kept; anything else they match is rewritten.
fn jsonc_pass(text: &str, comments: bool) -> String {
    let chars: Vec<char> = text.chars().collect();
    let line_end = |c: char| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}');
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '"' {
            // `"(?:\\.|[^"\\])*"`: a backslash takes the next character
            // unless that is a line terminator, which fails the match.
            let mut end = at + 1;
            let closed = loop {
                match chars.get(end) {
                    None => break None,
                    Some('"') => break Some(end),
                    Some('\\') => match chars.get(end + 1) {
                        Some(&next) if !line_end(next) => end += 2,
                        _ => break None,
                    },
                    Some(_) => end += 1,
                }
            };
            if let Some(end) = closed {
                out.extend(&chars[at..=end]);
                at = end + 1;
                continue;
            }
        } else if comments && c == '/' && chars.get(at + 1) == Some(&'/') {
            let mut end = at;
            while end < chars.len() && chars[end] != '\r' && chars[end] != '\n' {
                end += 1;
            }
            for &skipped in &chars[at..end] {
                out.push_str(if skipped.len_utf16() == 2 { "  " } else { " " });
            }
            at = end;
            continue;
        } else if comments && c == '/' && chars.get(at + 1) == Some(&'*') {
            let close = (at + 2..chars.len().saturating_sub(1))
                .find(|&index| chars[index] == '*' && chars[index + 1] == '/');
            if let Some(close) = close {
                for &skipped in &chars[at..close + 2] {
                    match skipped {
                        '\r' | '\n' => out.push(skipped),
                        other if other.len_utf16() == 2 => out.push_str("  "),
                        _ => out.push(' '),
                    }
                }
                at = close + 2;
                continue;
            }
        } else if !comments && c == ',' {
            // `,(\s*[}\]])`: drop the comma before a closing bracket.
            let mut end = at + 1;
            while end < chars.len() && js_space(chars[end]) {
                end += 1;
            }
            if matches!(chars.get(end), Some('}' | ']')) {
                at += 1;
                continue;
            }
        }
        out.push(c);
        at += 1;
    }
    out
}

/// `parseOpenCodeConfig(text)`: JSONC's comments and trailing commas,
/// without ever editing quoted strings.
pub fn parse_config(text: &str) -> Result<Obj, BrokerFailure> {
    let clean = jsonc_pass(&jsonc_pass(text, true), false);
    match parse(&clean) {
        Some(Js::Obj(config)) => Ok(config),
        _ => Err(fail(
            "OpenCode provider configuration could not be read. Check its JSON or JSONC outside Kassiber.",
        )),
    }
}

/// `merge(a, b)`: `b` over `a`, objects merged key by key.
fn merge(a: &Obj, b: &Obj) -> Obj {
    let mut result = a.clone();
    for (key, value) in b.iter() {
        let merged = match value {
            Js::Obj(value) => Js::Obj(merge(&object(result.get(key)), value)),
            other => other.clone(),
        };
        result.set(key, merged);
    }
    result
}

/// `access(path)`: `false` when it does not exist.
fn exists(path: &Path) -> Result<bool, BrokerFailure> {
    match std::fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(fail(
            "OpenCode configuration isolation could not be verified.",
        )),
    }
}

/// `path.resolve(...)`: absolute and lexically normalized.
pub fn resolve(base: &Path, path: &Path) -> PathBuf {
    let joined = if path.is_absolute() {
        path.to_path_buf()
    } else if base.is_absolute() {
        base.join(path)
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(base)
            .join(path)
    };
    let mut out = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                if out.parent().is_some() {
                    out.pop();
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// The system-managed configuration locations of this platform.
fn managed_locations(host: &Host) -> Vec<PathBuf> {
    if let Some(managed) = &host.managed {
        return managed.clone();
    }
    if host.macos {
        let mut paths = vec![
            PathBuf::from("/Library/Application Support/opencode"),
            PathBuf::from("/Library/Managed Preferences/ai.opencode.managed.plist"),
        ];
        if let Some(user) = host.var("USER") {
            paths.push(PathBuf::from(format!(
                "/Library/Managed Preferences/{user}/ai.opencode.managed.plist"
            )));
        }
        paths
    } else if host.windows {
        vec![PathBuf::from(
            host.var("ProgramData")
                .unwrap_or_else(|| r"C:\ProgramData".to_owned()),
        )
        .join("opencode")]
    } else {
        vec![PathBuf::from("/etc/opencode")]
    }
}

/// The provider settings of the user's configuration files, with the
/// variables they name.
struct Retained {
    settings: Obj,
    variables: Vec<String>,
}

impl Retained {
    fn variable(&mut self, name: &str) {
        if !self.variables.iter().any(|known| known == name) {
            self.variables.push(name.to_owned());
        }
    }

    /// `providerValue(value, directory)`: note `{env:NAME}` references and
    /// anchor relative `{file:...}` references at the document's directory.
    fn provider_value(&mut self, value: &Js, directory: &Path) -> Js {
        match value {
            Js::Str(text) => {
                for name in env_references(text) {
                    self.variable(&name);
                }
                Js::Str(anchor_files(text, directory))
            }
            Js::Arr(items) => Js::Arr(
                items
                    .iter()
                    .map(|item| self.provider_value(item, directory))
                    .collect(),
            ),
            Js::Obj(entries) => Js::Obj(Obj::from_entries(
                entries
                    .iter()
                    .map(|(key, item)| (key.to_owned(), self.provider_value(item, directory)))
                    .collect::<Vec<_>>(),
            )),
            other => other.clone(),
        }
    }

    /// `retain(config, directory)`.
    fn retain(&mut self, config: &Obj, directory: &Path) {
        let mut kept = Obj::new();
        for key in [
            "provider",
            "providers",
            "enabled_providers",
            "disabled_providers",
            "policies",
        ] {
            if let Some(value) = config.get(key) {
                kept.set(key, value.clone());
            }
        }
        // Keep provider-relative credential-file references relative to
        // their original document. OpenCode itself performs the
        // substitution.
        let kept = self.provider_value(&Js::Obj(kept), directory);
        self.settings = merge(&self.settings, &object(Some(&kept)));
        for source in [config.get("provider"), config.get("providers")] {
            for (_, provider) in object(source).iter() {
                if let Some(Js::Arr(names)) = provider.get("env") {
                    for name in names.iter().filter_map(Js::as_str) {
                        self.variable(name);
                    }
                }
            }
        }
    }
}

/// `/\{env:([A-Za-z_][A-Za-z0-9_]*)\}/g` captures.
fn env_references(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("{env:") {
        let after = &rest[at + 5..];
        let length = after
            .bytes()
            .enumerate()
            .take_while(|(index, byte)| {
                byte.is_ascii_alphabetic() || *byte == b'_' || (*index > 0 && byte.is_ascii_digit())
            })
            .count();
        if length > 0 && after.as_bytes().get(length) == Some(&b'}') {
            names.push(after[..length].to_owned());
            rest = &after[length + 1..];
        } else {
            rest = &rest[at + 1..];
        }
    }
    names
}

/// `value.replace(/\{file:([^}]+)\}/g, ...)`: relative file references
/// resolved against `directory`.
fn anchor_files(text: &str, directory: &Path) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("{file:") {
        let after = &rest[at + 6..];
        match after.find('}') {
            Some(length) if length > 0 => {
                let path = &after[..length];
                out.push_str(&rest[..at]);
                let anchored = if Path::new(path).is_absolute() || path.starts_with('~') {
                    path.to_owned()
                } else {
                    resolve(directory, Path::new(path))
                        .to_string_lossy()
                        .into_owned()
                };
                out.push_str("{file:");
                out.push_str(&anchored);
                out.push('}');
                rest = &after[length + 1..];
            }
            _ => {
                out.push_str(&rest[..at + 1]);
                rest = &rest[at + 1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// `openCode2Permissions(request)`.
pub fn permissions(request: Option<&ChatRequest>) -> Js {
    let mut rules = vec![Js::obj([
        ("action", Js::str("*")),
        ("resource", Js::str("*")),
        ("effect", Js::str("deny")),
    ])];
    for tool in request
        .map(|request| &request.tools[..])
        .unwrap_or_default()
    {
        rules.push(Js::obj([
            ("action", Js::str(format!("kassiber_{}", tool.name))),
            ("resource", Js::str("*")),
            ("effect", Js::str("allow")),
        ]));
    }
    Js::Arr(rules)
}

/// `openCode2Environment(cwd, request)`: read only provider settings into
/// RAM. Passing the original config to v2 would import user plugins before
/// a later `-*` could disable them. Credentials in provider settings never
/// enter a broker result or disk file.
pub fn environment(
    host: &Host,
    cwd: &Path,
    request: Option<&ChatRequest>,
) -> Result<Env, BrokerFailure> {
    // The CLI cannot disable managed sources. Refuse them before starting
    // it; their plugins could load before our private inline document is
    // applied.
    for path in managed_locations(host) {
        if exists(&path)? {
            return Err(fail(
                "System-managed OpenCode configuration cannot be isolated by this build. Choose another provider.",
            ));
        }
    }
    let mut env = discard_environment(host, cwd)?;
    let root = cwd.join("opencode2-config");
    make_private_dirs(&root)?;
    let config_root = match host.var_os("OPENCODE_CONFIG_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => match host.var_os("XDG_CONFIG_HOME") {
            Some(config) => PathBuf::from(config),
            None => host.home().join(".config"),
        }
        .join("opencode"),
    };
    let mut files = vec![
        config_root.join("opencode.json"),
        config_root.join("opencode.jsonc"),
    ];
    if let Some(explicit) = host
        .var_os("OPENCODE_CONFIG")
        .filter(|path| !path.is_empty())
    {
        files.push(resolve(Path::new(""), Path::new(explicit)));
    }
    let mut retained = Retained {
        settings: Obj::new(),
        variables: Vec::new(),
    };
    let mut seen = HashSet::new();
    for file in files {
        if !seen.insert(file.clone()) || !exists(&file)? {
            continue;
        }
        let size = std::fs::metadata(&file)
            .map_err(|error| fs_failure("stat", &file, &error))?
            .len();
        if size > LIMIT as u64 {
            return Err(fail(
                "OpenCode provider configuration exceeds its size limit.",
            ));
        }
        let bytes = std::fs::read(&file).map_err(|error| fs_failure("read", &file, &error))?;
        let config = parse_config(&String::from_utf8_lossy(&bytes))?;
        let directory = file.parent().map(Path::to_path_buf).unwrap_or_default();
        retained.retain(&config, &directory);
    }
    if let Some(content) = host
        .var("OPENCODE_CONFIG_CONTENT")
        .filter(|content| !content.is_empty())
    {
        retained.retain(&parse_config(&content)?, cwd);
    }
    let rules = permissions(request);
    let disabled = || Js::obj([("disabled", Js::Bool(true))]);
    let mut config = retained.settings.clone();
    let system = match request {
        Some(request) => system_instructions(request),
        None => "Kassiber model check.".to_owned(),
    };
    for (key, value) in [
        (
            "plugins",
            Js::Arr(PLUGINS.iter().map(|plugin| Js::str(*plugin)).collect()),
        ),
        ("share", Js::str("disabled")),
        ("update", Js::str("disable")),
        ("snapshots", Js::Bool(false)),
        ("warming", Js::Bool(false)),
        ("compaction", Js::obj([("auto", Js::Bool(false))])),
        ("instructions", Js::Arr(Vec::new())),
        ("skills", Js::Arr(Vec::new())),
        ("permissions", rules.clone()),
        ("default_agent", Js::str(AGENT)),
        (
            "agents",
            Js::obj([
                (
                    AGENT,
                    Js::obj([
                        ("mode", Js::str("primary")),
                        ("system", Js::str(system)),
                        ("permissions", rules),
                    ]),
                ),
                ("title", disabled()),
                ("summary", disabled()),
                ("compaction", disabled()),
            ]),
        ),
        ("mcp", Js::obj([("servers", Js::obj::<&str>([]))])),
    ] {
        config.set(key, value);
    }
    // Preserve only provider configuration variables, never ambient OpenCode
    // server/plugin/runtime overrides. Other vendor credentials stay subject
    // to providerEnvironment's existing allowlist.
    let ambient: Vec<String> = env
        .names()
        .filter(|name| name.starts_with("OPENCODE_"))
        .collect();
    for name in ambient {
        env.remove(&name);
    }
    for name in &retained.variables {
        if name.starts_with("OPENCODE_") {
            continue;
        }
        if let Some(value) = host.var_os(name) {
            env.set(name, value.to_owned());
        }
    }
    // An absolute path keeps credentials and ordinary sessions in OpenCode's
    // own DB, while its always-on logger writes into our preinstalled sink.
    let database = match host.var("OPENCODE_DB") {
        Some(db) if db == ":memory:" => PathBuf::from(":memory:"),
        db => {
            let data = match host.var_os("XDG_DATA_HOME") {
                Some(data) => PathBuf::from(data),
                None => host.home().join(".local").join("share"),
            };
            resolve(
                &resolve(Path::new(""), &data.join("opencode")),
                Path::new(&db.unwrap_or_else(|| "opencode.db".to_owned())),
            )
        }
    };
    for (name, value) in [
        ("OPENCODE_CONFIG_DIR", root.clone().into_os_string()),
        (
            "OPENCODE_CONFIG_CONTENT",
            stringify(&Js::Obj(config)).into(),
        ),
        (
            "OPENCODE_CLI_CONFIG_CONTENT",
            stringify(&Js::obj([("update", Js::str("disable"))])).into(),
        ),
        ("OPENCODE_CONFIG_PROJECT_DISABLE", "1".into()),
        ("OPENCODE_DISABLE_CLAUDE_CODE", "1".into()),
        ("OPENCODE_DISABLE_MODELS_FETCH", "1".into()),
        ("OPENCODE_DISABLE_FREE_MODELS", "1".into()),
        ("OPENCODE_FILEWATCHER_DISABLE", "1".into()),
        ("OPENCODE_DISABLE_FFF", "1".into()),
        ("OPENCODE_TEST_HOME", root.clone().into_os_string()),
        ("OPENCODE_DB", database.clone().into_os_string()),
    ] {
        env.set(name, value);
    }
    if database != Path::new(":memory:") {
        if let Some(parent) = database.parent() {
            make_private_dirs(parent)?;
        }
    }
    Ok(env)
}

/// A running OpenCode 2 server and how to reach it.
pub struct Server {
    process: ServerProcess,
    loopback: Loopback,
    authorization: String,
}

/// `base64url` without padding.
fn base64url(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    encode64(bytes, ALPHABET, false)
}

fn encode64(bytes: &[u8], alphabet: &[u8], pad: bool) -> String {
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let value = chunk.iter().enumerate().fold(0u32, |acc, (index, byte)| {
            acc | (u32::from(*byte) << (16 - 8 * index))
        });
        for index in 0..=chunk.len() {
            out.push(alphabet[((value >> (18 - 6 * index)) & 63) as usize] as char);
        }
        if pad {
            out.push_str(&"=".repeat(3 - chunk.len()));
        }
    }
    out
}

/// The port of a structured loopback announcement (`{"url": ...}`).
fn announced_port(line: &str) -> Option<u16> {
    if utf16_len(line) > 4096 {
        return None;
    }
    let url = parse(line)?.get("url").cloned();
    let text = match url {
        None => "undefined".to_owned(),
        Some(value) => value.to_js_string(),
    };
    let url = kassiber_net::HttpUrl::parse(&text).ok()?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return None;
    }
    url.port()
}

/// `startServer(executable, cwd, env)`.
pub fn start_server(executable: &Path, cwd: &Path, env: &Env) -> Result<Server, BrokerFailure> {
    let password = base64url(&random_bytes::<32>()?);
    let mut env = env.clone();
    env.set("OPENCODE_SERVER_PASSWORD", password.clone());
    let args = [
        "serve".to_owned(),
        "--stdio".to_owned(),
        "--hostname=127.0.0.1".to_owned(),
        "--port=0".to_owned(),
    ];
    // Only accept the CLI's structured loopback announcement.
    let (process, port) =
        match ServerProcess::start(executable, &args, cwd, &env, false, announced_port) {
            Ok(started) => started,
            Err(NotReady::TimedOut) => return Err(fail("OpenCode server startup timed out.")),
            Err(NotReady::Refused(failure)) => return Err(failure),
            Err(NotReady::Spawn(_) | NotReady::Exited(_)) => {
                return Err(fail("OpenCode server could not start."))
            }
        };
    let verified = (|| {
        let data = env
            .get("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let logs = data.join("opencode").join("log");
        let entries =
            std::fs::read_dir(&logs).map_err(|error| fs_failure("scandir", &logs, &error))?;
        for entry in entries {
            let entry = entry.map_err(|error| fs_failure("scandir", &logs, &error))?;
            let name = entry.file_name();
            if !SINKS.iter().any(|sink| name == *sink) {
                return Err(fail(
                    "OpenCode's log destination could not be isolated; nothing was sent.",
                ));
            }
            let path = logs.join(&name);
            let target =
                std::fs::read_link(&path).map_err(|error| fs_failure("readlink", &path, &error))?;
            if target != Path::new(NULL_DEVICE) {
                return Err(fail(
                    "OpenCode's log discard sink could not be verified; nothing was sent.",
                ));
            }
        }
        Ok(())
    })();
    if let Err(failure) = verified {
        process.kill();
        return Err(failure);
    }
    const STANDARD: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    Ok(Server {
        process,
        loopback: Loopback::new(port),
        authorization: format!(
            "Basic {}",
            encode64(format!("opencode:{password}").as_bytes(), STANDARD, true)
        ),
    })
}

impl Server {
    /// `stop(server)`: end its stdin, SIGTERM, and SIGKILL after a second.
    pub fn stop(&mut self) {
        if self.process.exited() {
            return;
        }
        self.process.close_stdin();
        self.process.terminate();
        if !self.process.wait_timeout(Duration::from_secs(1)) {
            self.process.kill();
            self.process.wait();
        }
    }

    fn call<'a>(
        &self,
        cancel: Option<&'a Cancel>,
        deadline: Option<Instant>,
        body: Option<&Js>,
        max_response_bytes: usize,
    ) -> Call<'a> {
        Call {
            headers: vec![
                ("Authorization", self.authorization.clone()),
                ("Content-Type", "application/json".to_owned()),
            ],
            body: body.map(|body| stringify(body).into_bytes()),
            timeout: Some(FETCH_TIMEOUT),
            deadline,
            cancel,
            max_response_bytes: Some(max_response_bytes),
        }
    }

    /// `json(server, path, method, body)`.
    pub fn json(&self, path: &str, method: &str, body: Option<&Js>) -> Result<Obj, BrokerFailure> {
        self.json_cancellable(path, method, body, None)
    }

    /// [`Server::json`] that `cancel` can stop.
    fn json_cancellable(
        &self,
        path: &str,
        method: &str,
        body: Option<&Js>,
        cancel: Option<&Cancel>,
    ) -> Result<Obj, BrokerFailure> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        // A body over 3 bytes per UTF-16 unit of the limit is certainly over
        // the limit.
        let call = self.call(cancel, Some(deadline), body, 3 * LIMIT + 16);
        let opened = self.loopback.open(method, &format!("/api/{path}"), call);
        let mut stream = match opened {
            Ok(stream) => stream,
            Err(kassiber_net::Error::ResponseTooLarge) => {
                return Err(fail("OpenCode API response exceeds its size limit."))
            }
            Err(_) => return Err(fail("OpenCode local API connection failed.")),
        };
        if !(200..300).contains(&stream.status) {
            return Err(BrokerFailure::new(format!(
                "OpenCode local API request failed (HTTP {}).",
                stream.status
            )));
        }
        if stream.status == 204 {
            return Ok(Obj::new());
        }
        let body = match stream.read_to_end() {
            Ok(body) => body,
            Err(kassiber_net::Error::ResponseTooLarge) => {
                return Err(fail("OpenCode API response exceeds its size limit."))
            }
            Err(_) if Instant::now() >= deadline => {
                return Err(fail("The operation was aborted due to timeout"))
            }
            Err(_) => return Err(fail("terminated")),
        };
        let text = String::from_utf8_lossy(&body);
        if utf16_len(&text) > LIMIT {
            return Err(fail("OpenCode API response exceeds its size limit."));
        }
        match parse(&text) {
            Some(value) => Ok(object(Some(&value))),
            None => Err(fail("OpenCode returned an invalid API response.")),
        }
    }
}

/// `AbortSignal.timeout(30_000)` of an ordinary API request.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// undici's default header and body timeouts, which bound the event stream.
const FETCH_TIMEOUT: Duration = Duration::from_secs(300);

/// `?location[directory]=<cwd>`.
fn location_query(cwd: &Path) -> String {
    format!(
        "location%5Bdirectory%5D={}",
        form_encode(&cwd.to_string_lossy())
    )
}

fn data(payload: &Obj) -> Vec<Js> {
    payload
        .get("data")
        .and_then(Js::as_arr)
        .map(<[Js]>::to_vec)
        .unwrap_or_default()
}

/// `parseOpenCode2Models(payload, providers)`.
pub fn parse_models(payload: &Obj, providers: &Obj) -> Vec<BrokerModel> {
    let mut routes: Vec<(String, Option<Js>)> = Vec::new();
    for raw in data(providers) {
        let row = object(Some(&raw));
        let id = row.get("id").cloned().unwrap_or(Js::Null);
        let key = if row.get("id").is_none() {
            "undefined".to_owned()
        } else {
            id.to_js_string()
        };
        let base_url = object(row.get("settings")).get("baseURL").cloned();
        match routes.iter_mut().find(|(known, _)| *known == key) {
            Some(route) => route.1 = base_url,
            None => routes.push((key, base_url)),
        }
    }
    data(payload)
        .iter()
        .filter_map(|raw| {
            let row = object(Some(raw));
            let id = row.get("id")?.as_str()?.to_owned();
            let provider = row.get("providerID")?.as_str()?.to_owned();
            if row.get("enabled") == Some(&Js::Bool(false)) {
                return None;
            }
            let endpoint = match object(row.get("settings")).get("baseURL") {
                None | Some(Js::Null) => routes
                    .iter()
                    .find(|(known, _)| *known == provider)
                    .and_then(|(_, url)| url.clone()),
                Some(url) => Some(url.clone()),
            };
            let local = endpoint
                .as_ref()
                .and_then(Js::as_str)
                .is_some_and(is_loopback_endpoint);
            let variants: Vec<String> = row
                .get("variants")
                .and_then(Js::as_arr)
                .unwrap_or_default()
                .iter()
                .filter_map(|variant| variant.get("id").and_then(Js::as_str).map(str::to_owned))
                .collect();
            Some(BrokerModel {
                id: format!("{provider}/{id}"),
                display_name: Some(match row.get("name") {
                    Some(Js::Str(name)) => name.clone(),
                    _ => id.clone(),
                }),
                source_provider: Some(provider.clone()),
                owned_by: Some(format!("OpenCode · {provider}")),
                privacy_posture: Some(if local {
                    ModelPosture::Local
                } else {
                    ModelPosture::Remote
                }),
                privacy_reason: Some(
                    if local {
                        "OpenCode reports a loopback endpoint for this model."
                    } else {
                        "OpenCode does not report a loopback endpoint for this model."
                    }
                    .to_owned(),
                ),
                supports_reasoning_effort: Some(!variants.is_empty()),
                reasoning_efforts: Some(variants),
                supports_tools: Some(
                    object(row.get("capabilities")).get("tools") == Some(&Js::Bool(true)),
                ),
                ..BrokerModel::default()
            })
        })
        .collect()
}

/// `openCode2Models(executable, cwd)`.
pub fn models(
    host: &Host,
    executable: &Path,
    cwd: &Path,
) -> Result<Vec<BrokerModel>, BrokerFailure> {
    let mut server = start_server(executable, cwd, &environment(host, cwd, None)?)?;
    let query = location_query(cwd);
    // The model read awaits plugin/catalog activation. Provider endpoints are
    // read afterwards, and only their projected routing metadata escapes.
    let result = server
        .json(&format!("model?{query}"), "GET", None)
        .and_then(|models| {
            let providers = server.json(&format!("provider?{query}"), "GET", None)?;
            Ok(parse_models(&models, &providers))
        });
    server.stop();
    result
}

/// The event stream of one server.
struct Stream {
    frames: Frames,
    cancel: Cancel,
    aborted: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Stream {
    /// The next event (`object(JSON.parse(data))`).
    fn next(&mut self) -> Result<Obj, BrokerFailure> {
        loop {
            let frame = match self.frames.next(false, Some(LIMIT)) {
                Ok(Some(frame)) => frame,
                Ok(None) => return Err(fail("OpenCode event stream ended unexpectedly.")),
                Err(FrameError::TooLarge) => {
                    return Err(fail("OpenCode event stream exceeds its size limit."))
                }
                Err(FrameError::Read(_)) => {
                    return Err(fail(
                        if self.aborted.load(std::sync::atomic::Ordering::SeqCst)
                            || self.cancel.is_cancelled()
                        {
                            "This operation was aborted"
                        } else {
                            "terminated"
                        },
                    ))
                }
            };
            let data = frame
                .split('\n')
                .filter_map(|line| line.strip_prefix("data:"))
                .map(|line| line.trim_start_matches(js_space))
                .collect::<Vec<_>>()
                .join("\n");
            if data.is_empty() {
                continue;
            }
            return match parse(&data) {
                Some(value) => Ok(object(Some(&value))),
                None => Err(fail("OpenCode returned an invalid event frame.")),
            };
        }
    }
}

/// Which side of the turn finished first.
enum Outcome {
    Consumer(Result<(), BrokerFailure>),
    Prompt(Result<Obj, BrokerFailure>),
}

/// `openCode2Chat(executable, request, cwd, bridge)`.
pub fn chat(
    host: &Host,
    executable: &Path,
    request: &ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &EventSink,
) -> Result<(), BrokerFailure> {
    let slash = model_separator(&request.model)?;
    let (provider_prefix, model_id) = (&request.model[..slash], &request.model[slash + 1..]);
    let mut env = environment(host, cwd, Some(request))?;
    let private_turn = request.option("sensitive_context").is_some_and(truthy)
        || request.option("stateless_session").is_some_and(truthy);
    let mut credentials: Vec<Obj> = Vec::new();
    if private_turn {
        // Only the chosen integration's active credential crosses this local,
        // authenticated API, in RAM. No prompt is sent to the persistent DB.
        // The persistent source needs provider settings only, never this
        // turn's system context or tool schemas, even as configuration.
        let mut source = start_server(executable, cwd, &environment(host, cwd, None)?)?;
        let copied = (|| {
            // provider.list waits for catalog/plugin activation; provider.get
            // does not, and can incorrectly return 404 for a configured
            // provider at boot.
            let providers =
                source.json(&format!("provider?{}", location_query(cwd)), "GET", None)?;
            let provider = data(&providers)
                .iter()
                .map(|item| object(Some(item)))
                .find(|item| item.get("id").and_then(Js::as_str) == Some(provider_prefix))
                .unwrap_or_default();
            let integration = match provider.get("integrationID") {
                None | Some(Js::Null) => Js::str(provider_prefix),
                Some(integration) => integration.clone(),
            };
            let saved = source.json("credential", "GET", None)?;
            let credentials: Vec<Obj> = data(&saved)
                .iter()
                .map(|entry| object(Some(entry)))
                .filter(|entry| {
                    entry
                        .get("integrationID")
                        .is_some_and(|id| id.strict_eq(&integration))
                        && entry.get("active") == Some(&Js::Bool(true))
                })
                .collect();
            // Refreshing a copied OAuth token can invalidate the persistent
            // token. Until v2 exposes a shared credential store independent
            // of its session database, never risk rotating that credential
            // inside a private DB.
            if credentials.iter().any(|entry| {
                object(entry.get("value")).get("type").and_then(Js::as_str) == Some("oauth")
            }) {
                return Err(fail("OpenCode 2 OAuth accounts cannot safely use Incognito or selected-data chats yet. Use a key-backed provider or another CLI provider."));
            }
            Ok(credentials)
        })();
        source.stop();
        credentials = copied?;
        env.set("OPENCODE_DB", ":memory:");
    }
    events.write(&Event::Status {
        phase: "connecting".into(),
        message: "Starting OpenCode server".into(),
    });
    let mut server = start_server(executable, cwd, &env)?;
    let cancel = Cancel::new();
    let prompt_cancel = Cancel::new();
    let result = std::thread::scope(|scope| {
        let result = turn(
            scope,
            &server,
            (&cancel, &prompt_cancel),
            credentials,
            request,
            cwd,
            bridge,
            events,
            (provider_prefix, model_id),
            host.event_ready,
        );
        // `finally`: abort the stream before the turn's threads are joined.
        // The server stops right after, which ends a prompt still in flight;
        // here that request is stopped directly.
        cancel.cancel();
        prompt_cancel.cancel();
        result
    });
    server.stop();
    result
}

#[allow(clippy::too_many_arguments)]
fn turn<'scope>(
    scope: &'scope std::thread::Scope<'scope, '_>,
    server: &'scope Server,
    (cancel, prompt_cancel): (&'scope Cancel, &'scope Cancel),
    mut credentials: Vec<Obj>,
    request: &'scope ChatRequest,
    cwd: &Path,
    bridge: Option<&ToolBridge>,
    events: &'scope EventSink,
    (provider_id, model_id): (&str, &str),
    event_ready: Duration,
) -> Result<(), BrokerFailure> {
    for entry in credentials.drain(..) {
        let mut body = Obj::new();
        if let Some(integration) = entry.get("integrationID") {
            body.set("integrationID", integration.clone());
        }
        if let Some(value) = entry.get("value") {
            body.set("value", value.clone());
        }
        body.set("activate", Js::Bool(true));
        server.json("credential", "POST", Some(&Js::Obj(body)))?;
    }
    if let Some(bridge) = bridge.filter(|_| !request.tools.is_empty()) {
        let query = location_query(cwd);
        let command = mcp_command(cwd, &request.tools, bridge)?;
        server.json(
            &format!("experimental/mcp/kassiber?{query}"),
            "PUT",
            Some(&Js::obj([(
                "config",
                Js::obj([
                    ("type", Js::str("local")),
                    (
                        "command",
                        Js::Arr(command.into_iter().map(Js::Str).collect()),
                    ),
                    ("codemode", Js::Bool(false)),
                ]),
            )])),
        )?;
        // add() starts discovery in the background. connect() awaits the MCP
        // handshake and tools/list; otherwise the first inference can have
        // no tools at all. v2.2 then debounces its tool-registry reload by
        // 100ms, without a public flush API. Allow that reconciliation to
        // settle before constructing the session or sending any prompt.
        server.json(
            &format!("experimental/mcp/kassiber/connect?{query}"),
            "POST",
            None,
        )?;
        let connected = server.json(&format!("mcp?{query}"), "GET", None)?;
        let mcp = data(&connected)
            .iter()
            .map(|entry| object(Some(entry)))
            .find(|entry| entry.get("name").and_then(Js::as_str) == Some("kassiber"));
        let status = mcp.and_then(|mcp| object(mcp.get("status")).get("status").cloned());
        if status.as_ref().and_then(Js::as_str) != Some("connected") {
            return Err(fail("OpenCode could not connect Kassiber's tool bridge."));
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    let mut model = Obj::from_entries([
        ("providerID", Js::str(provider_id)),
        ("id", Js::str(model_id)),
    ]);
    if let Some(effort) = request
        .option("reasoning_effort")
        .filter(|effort| truthy(effort) && effort.as_str() != Some("auto"))
    {
        model.set("variant", Js::from_serde(effort));
    }
    // Rebuild from Kassiber's transcript. Native v1 cursors or older v2
    // agents must not resume a session with a different system/tool
    // configuration.
    let created = server.json(
        "session",
        "POST",
        Some(&Js::obj([
            ("title", Js::str("Kassiber assistant")),
            ("agent", Js::str(AGENT)),
            (
                "location",
                Js::obj([("directory", Js::str(cwd.to_string_lossy()))]),
            ),
            ("model", Js::Obj(model)),
            ("permissions", permissions(Some(request))),
        ])),
    )?;
    let session_value = object(created.get("data")).get("id").map(|id| match id {
        Js::Str(text) => serde_json::Value::String(text.clone()),
        _ => serde_json::Value::Null,
    });
    let id = safe_session_cursor(session_value.as_ref())
        .ok_or_else(|| fail("OpenCode did not create a chat session."))?;
    let allowed: HashSet<String> = request
        .tools
        .iter()
        .map(|tool| format!("kassiber_{}", tool.name))
        .collect();

    // Abort the actual read on timeout, so a server that sends headers but
    // no first frame cannot leave an unobserved stream reader behind.
    let aborted = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let (first_frame, watchdog) = mpsc::channel::<()>();
    {
        let cancel = cancel.clone();
        let aborted = aborted.clone();
        scope.spawn(move || {
            if let Err(mpsc::RecvTimeoutError::Timeout) = watchdog.recv_timeout(event_ready) {
                aborted.store(true, std::sync::atomic::Ordering::SeqCst);
                cancel.cancel();
            }
        });
    }
    let opened = server.loopback.open(
        "GET",
        "/api/event",
        // The stream as a whole is bounded only by the transport's cap.
        server.call(Some(cancel), None, None, usize::MAX),
    );
    let stream = match opened {
        Ok(stream) if (200..300).contains(&stream.status) => stream,
        Ok(stream) => {
            let _ = first_frame.send(());
            return Err(BrokerFailure::new(format!(
                "OpenCode local API request failed (HTTP {}).",
                stream.status
            )));
        }
        Err(_) => {
            let _ = first_frame.send(());
            return Err(fail("OpenCode local API connection failed."));
        }
    };
    let mut stream = Stream {
        frames: Frames::new(stream),
        cancel: cancel.clone(),
        aborted,
    };
    let first = stream.next();
    let _ = first_frame.send(());
    if first?.get("type").and_then(Js::as_str) != Some("server.connected") {
        return Err(fail("OpenCode did not establish its event stream."));
    }

    let (outcomes, outcome) = mpsc::channel();
    {
        let outcomes = outcomes.clone();
        let id = id.clone();
        scope.spawn(move || {
            let result = consume(&mut stream, server, &id, &allowed, events);
            let _ = outcomes.send(Outcome::Consumer(result));
        });
    }
    let text = prompt_from_messages(&request.messages, false);
    let files: Vec<Js> = prompt_images(&request.messages, false)?
        .iter()
        .map(|image| {
            Js::obj([(
                "uri",
                Js::str(format!("data:{};base64,{}", image.media_type, image.data)),
            )])
        })
        .collect();
    let path = format!("session/{}/prompt", encode_uri_component(&id));
    scope.spawn(move || {
        let body = Js::obj([("text", Js::Str(text)), ("files", Js::Arr(files))]);
        let result = server.json_cancellable(&path, "POST", Some(&body), Some(prompt_cancel));
        let _ = outcomes.send(Outcome::Prompt(result));
    });
    loop {
        match outcome.recv() {
            Ok(Outcome::Consumer(result)) => return result,
            Ok(Outcome::Prompt(Err(failure))) => return Err(failure),
            Ok(Outcome::Prompt(Ok(_))) => continue,
            Err(_) => return Err(fail("OpenCode event stream ended unexpectedly.")),
        }
    }
}

/// The event loop of a turn, until the session completes.
fn consume(
    stream: &mut Stream,
    server: &Server,
    id: &str,
    allowed: &HashSet<String>,
    events: &EventSink,
) -> Result<(), BrokerFailure> {
    loop {
        let event = stream.next()?;
        let data = object(event.get("data"));
        if data.get("sessionID").and_then(Js::as_str) != Some(id) {
            continue;
        }
        let delta = data.get("delta").and_then(Js::as_str);
        match event.get("type").and_then(Js::as_str) {
            Some("session.text.delta") if delta.is_some() => events.write(&Event::Delta {
                content: delta.map(str::to_owned),
                reasoning: None,
            }),
            Some("session.reasoning.delta") if delta.is_some() => events.write(&Event::Delta {
                content: None,
                reasoning: delta.map(str::to_owned),
            }),
            Some("session.tool.input.started") => {
                let name = match data.get("name") {
                    None => "undefined".to_owned(),
                    Some(name) => name.to_js_string(),
                };
                if !allowed.contains(&name) {
                    return Err(fail(
                        "OpenCode attempted to use a provider-native tool; Kassiber stopped it.",
                    ));
                }
            }
            Some("permission.asked") => {
                return Err(fail(
                    "OpenCode requested a permission outside Kassiber's tool bridge.",
                ))
            }
            Some("session.execution.failed" | "session.execution.interrupted") => {
                return Err(fail("OpenCode did not complete the response."))
            }
            Some("session.execution.succeeded" | "session.idle") => {
                let session = server.json(
                    &format!("session/{}", encode_uri_component(id)),
                    "GET",
                    None,
                )?;
                if object(session.get("data"))
                    .get("outcome")
                    .and_then(Js::as_str)
                    != Some("succeeded")
                {
                    return Err(fail("OpenCode did not complete the response."));
                }
                events.write(&Event::Done {
                    finish_reason: "stop".into(),
                    provider_session_id: None,
                });
                return Ok(());
            }
            _ => {}
        }
    }
}
