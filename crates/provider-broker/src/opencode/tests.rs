//! The OpenCode adapter against fake CLIs: ports of `opencode-share.test.ts`,
//! `opencode-version.test.ts`, `opencode2.test.ts` and the OpenCode cases of
//! `broker.test.ts`, `sensitive.test.ts` and `stateless.test.ts`, then whole
//! requests through the dispatcher.
//!
//! The fake `opencode` is a shell script that records each run (argv, the
//! names of its environment, chosen values) and answers from files the test
//! wrote. Its `serve` announces a loopback server the test itself runs
//! (`kassiber_net::test_support::FakeService`): OpenCode 1 is handed that
//! server's port through [`Host::free_port`], OpenCode 2 prints it.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kassiber_net::test_support::{FakeService, HandledRequest, Reply};
use serde_json::{json, Value};

use super::js::{parse, stringify, Js};
use super::*;
use crate::protocol::{BrokerToolDefinition, ChatMessage, EventSink};
use crate::{dispatch, input_from, Adapter};

mod broker;
mod e2e;
mod share;
mod v2_cases;
mod version;

/// The values a fake run records, by name.
const RECORDED: &[&str] = &[
    "OPENCODE_CONFIG_CONTENT",
    "OPENCODE_DISABLE_SHARE",
    "OPENCODE_AUTO_SHARE",
    "OPENCODE_DB",
    "OPENCODE_PURE",
    "OPENCODE_SERVER_PASSWORD",
    "OPENCODE_CONFIG_DIR",
    "OPENCODE_CONFIG_PROJECT_DISABLE",
    "OPENCODE_CONFIG",
    "XDG_DATA_HOME",
    "XDG_STATE_HOME",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "UNRELATED_PRIVATE_VALUE",
    "KASSIBER_AI_BROKER",
];

/// A fake `opencode` in its own directory.
pub(super) struct Fake {
    pub root: tempfile::TempDir,
}

/// One recorded run of the fake.
#[derive(Debug, Clone)]
pub(super) struct Run {
    pub argv: Vec<String>,
    pub names: Vec<String>,
    pub vars: HashMap<String, String>,
}

impl Fake {
    pub fn new(version: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let fake = Self { root };
        std::fs::create_dir_all(fake.path("bin")).unwrap();
        std::fs::create_dir_all(fake.path("home")).unwrap();
        fake.write("version", &format!("{version}\n"));
        fake.write("debug", "{}\n");
        fake.write("models", "provider/model\n");
        fake.write("serve-mode", "exit");
        let script = format!(
            r#"#!/bin/sh
root='{root}'
n=0
while [ -e "$root/calls/$n" ]; do n=$((n+1)); done
mkdir -p "$root/calls/$n"
for arg in "$@"; do printf '%s\n' "$arg"; done > "$root/calls/$n/argv"
export -p > "$root/calls/$n/env"
for name in {names}; do
  eval "if [ -n \"\${{$name+x}}\" ]; then printf '%s' \"\${{$name}}\" > \"$root/calls/$n/var.$name\"; fi"
done
case "$1" in
--version) cat "$root/version"; exit 0 ;;
debug) cat "$root/debug"; exit "$(cat "$root/debug-code" 2>/dev/null || echo 0)" ;;
models) cat "$root/models"; exit "$(cat "$root/models-code" 2>/dev/null || echo 0)" ;;
db)
  if [ -e "$root/write-log" ]; then printf 'synthetic log content must be discarded' >> "$XDG_DATA_HOME/opencode/log/opencode.log"; fi
  if [ -e "$root/disk-db" ]; then echo /home/user/.local/share/opencode/opencode.db
  else printf '%s\n' "${{OPENCODE_DB:-/home/user/.local/share/opencode/opencode.db}}"; fi
  exit 0 ;;
serve)
  s=0
  while [ -e "$root/serves/$s" ]; do s=$((s+1)); done
  mkdir -p "$root/serves/$s"
  case "$(cat "$root/serve-mode")" in
  v1) echo "opencode server listening on http://127.0.0.1:$(cat "$root/v1-port")"; read -r _; exit 0 ;;
  v2) printf '{{"url":"http://127.0.0.1:%s"}}\n' "$(cat "$root/ports/$s")"; while read -r _; do :; done; exit 0 ;;
  *) exit 1 ;;
  esac ;;
esac
exit 2
"#,
            root = fake.root.path().display(),
            names = RECORDED.join(" "),
        );
        let executable = fake.path("bin/opencode");
        std::fs::write(&executable, script).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        fake
    }

    pub fn path(&self, relative: &str) -> PathBuf {
        self.root.path().join(relative)
    }

    pub fn write(&self, relative: &str, content: &str) {
        let path = self.path(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    pub fn executable(&self) -> PathBuf {
        self.path("bin/opencode")
    }

    /// A host whose `PATH` finds this fake first, with a private home and
    /// no managed configuration.
    pub fn host(&self) -> Host {
        self.host_with(&[])
    }

    pub fn host_with(&self, extra: &[(&str, &str)]) -> Host {
        let mut vars: Vec<(std::ffi::OsString, std::ffi::OsString)> = vec![
            (
                "PATH".into(),
                format!("{}:/usr/bin:/bin", self.path("bin").display()).into(),
            ),
            ("HOME".into(), self.path("home").into()),
        ];
        for (name, value) in extra {
            vars.push(((*name).into(), (*value).into()));
        }
        Host {
            vars,
            fallbacks: Vec::new(),
            windows: false,
            macos: false,
            managed: Some(vec![self.path("managed-opencode")]),
            free_port: Arc::new(kassiber_net::free_loopback_port),
            event_ready: Duration::from_secs(30),
        }
    }

    /// A fresh turn directory under the fake's root.
    pub fn turn(&self) -> PathBuf {
        let parent = self.path("turns");
        std::fs::create_dir_all(&parent).unwrap();
        tempfile::Builder::new()
            .prefix("turn-")
            .tempdir_in(parent)
            .unwrap()
            .keep()
    }

    /// Every run so far, in order.
    pub fn runs(&self) -> Vec<Run> {
        let mut runs = Vec::new();
        for index in 0.. {
            let dir = self.path(&format!("calls/{index}"));
            if !dir.exists() {
                break;
            }
            let argv = std::fs::read_to_string(dir.join("argv")).unwrap_or_default();
            let env = std::fs::read_to_string(dir.join("env")).unwrap_or_default();
            let names = env
                .lines()
                .filter_map(|line| {
                    let rest = line
                        .strip_prefix("export ")
                        .or_else(|| line.strip_prefix("declare -x "))?;
                    let name = rest.split('=').next()?;
                    (!name.is_empty()
                        && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
                    .then(|| name.to_owned())
                })
                .collect();
            let mut vars = HashMap::new();
            for name in RECORDED {
                if let Ok(value) = std::fs::read_to_string(dir.join(format!("var.{name}"))) {
                    vars.insert((*name).to_owned(), value);
                }
            }
            runs.push(Run {
                argv: argv.lines().map(str::to_owned).collect(),
                names,
                vars,
            });
        }
        runs
    }

    /// The runs of `serve`.
    pub fn serves(&self) -> Vec<Run> {
        self.runs()
            .into_iter()
            .filter(|run| run.argv.first().map(String::as_str) == Some("serve"))
            .collect()
    }
}

/// A server-sent event stream a fake server feeds as the test drives it.
#[derive(Default, Clone)]
pub(super) struct Feed(Arc<Mutex<Option<mpsc::Sender<Vec<u8>>>>>);

impl Feed {
    /// Answer the event request: a held stream fed from here.
    pub fn open(&self) -> Reply {
        let (sender, pieces) = mpsc::channel();
        *self.0.lock().unwrap() = Some(sender);
        Reply::fed("text/event-stream", pieces)
    }

    pub fn raw(&self, text: &str) {
        if let Some(sender) = self.0.lock().unwrap().as_ref() {
            let _ = sender.send(text.as_bytes().to_vec());
        }
    }

    /// One `data:` frame.
    pub fn emit(&self, value: Value) {
        self.raw(&format!("data: {value}\n\n"));
    }

    /// End the stream.
    pub fn close(&self) {
        self.0.lock().unwrap().take();
    }
}

/// The request's body as JSON.
pub(super) fn body(request: &HandledRequest) -> Value {
    serde_json::from_slice(&request.body).unwrap_or(Value::Null)
}

/// The request target's path.
pub(super) fn path_of(request: &HandledRequest) -> &str {
    request.target.split('?').next().unwrap_or_default()
}

pub(super) fn chat_request(model: &str, options: Value) -> ChatRequest {
    ChatRequest::new(
        ProviderId::OpenCode,
        model,
        vec![ChatMessage::new("user", "Synthetic user message")],
    )
    .with_options(options)
}

pub(super) fn tool(name: &str) -> BrokerToolDefinition {
    BrokerToolDefinition::new(name, "Fixture tool", r#"{"type":"object","properties":{}}"#).unwrap()
}

/// Run one chat turn with `host` in a fresh directory of `fake`.
pub(super) fn run_chat(
    fake: &Fake,
    host: &Host,
    request: &ChatRequest,
    bridge: Option<&crate::ToolBridge>,
) -> (Result<(), BrokerFailure>, Vec<Value>, String) {
    let (events, captured) = EventSink::capture();
    let cwd = fake.turn();
    let result = chat(host, request, &cwd, bridge, &events);
    (result, captured.events(), captured.text())
}

/// A dispatcher whose OpenCode adapter uses `host`.
pub(super) fn adapters(host: Host) -> impl Sync + Fn(ProviderId) -> &'static dyn Adapter {
    let adapted: &'static Adapted = Box::leak(Box::new(Adapted { host }));
    move |_| adapted as &'static dyn Adapter
}

/// One whole request through the dispatcher: the result and its events.
pub(super) fn dispatch_line(host: Host, line: &str) -> (Result<(), BrokerFailure>, Vec<Value>) {
    let (events, captured) = EventSink::capture();
    let adapters = adapters(host);
    let result = dispatch(input_from(line.to_owned()), &events, &adapters);
    if let Err(failure) = &result {
        events.write(&crate::protocol::error_event(failure));
    }
    (result, captured.events())
}

#[test]
fn javascript_json_round_trips_a_provider_config() {
    let text = r#"{"provider":{"local":{"options":{"baseURL":"http://127.0.0.1:1/v1","temperature":1.0}}},"2":[1e3]}"#;
    assert_eq!(
        stringify(&parse(text).unwrap()),
        r#"{"2":[1000],"provider":{"local":{"options":{"baseURL":"http://127.0.0.1:1/v1","temperature":1}}}}"#
    );
}
