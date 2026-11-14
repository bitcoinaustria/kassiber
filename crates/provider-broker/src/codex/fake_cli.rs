//! Fake provider CLIs for the Claude and Codex adapter tests: `tests/fake-cli/<name>` copied
//! into a fresh directory with the behaviour files a test writes beside it.
//! The fakes are POSIX shell scripts, so the tests need no Node, no real
//! provider CLI and no network, and the same scripts serve the comparison
//! with the TypeScript broker.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::protocol::{ChatMessage, ChatRequest, EventSink, ProviderId};

/// One recorded invocation of a fake.
#[derive(Debug)]
pub struct Call {
    pub argv: Vec<String>,
    pub env_names: Vec<String>,
    /// `NAME=value` for the few names the fake records, `<unset>` if absent.
    pub env_values: Vec<String>,
    pub stdin: Option<String>,
    pub cwd: PathBuf,
}

impl Call {
    pub fn has(&self, arg: &str) -> bool {
        self.argv.iter().any(|item| item == arg)
    }

    /// The argument after `flag`.
    pub fn after(&self, flag: &str) -> Option<&str> {
        let index = self.argv.iter().position(|item| item == flag)?;
        self.argv.get(index + 1).map(String::as_str)
    }

    pub fn env_value(&self, name: &str) -> &str {
        self.env_values
            .iter()
            .find_map(|line| line.strip_prefix(&format!("{name}=")))
            .unwrap_or("<unset>")
    }
}

pub struct Fixture {
    dir: tempfile::TempDir,
    name: &'static str,
}

impl Fixture {
    /// `name` (`claude` or `codex`) in a fresh directory.
    pub fn new(name: &'static str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("kassiber-fake-{name}-"))
            .tempdir()
            .unwrap();
        let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fake-cli");
        for file in [name, "record.sh"] {
            std::fs::copy(source.join(file), dir.path().join(file)).unwrap();
        }
        Self { dir, name }
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    pub fn executable(&self) -> PathBuf {
        self.dir.path().join(self.name)
    }

    /// Write a behaviour file beside the fake.
    pub fn write(&self, file: &str, contents: &str) -> &Self {
        std::fs::write(self.dir.path().join(file), contents).unwrap();
        self
    }

    /// Lines of JSON values as a `.jsonl` file.
    pub fn write_lines(&self, file: &str, lines: &[Value]) -> &Self {
        let text: String = lines.iter().map(|line| format!("{line}\n")).collect();
        self.write(file, &text)
    }

    /// Every recorded invocation, oldest first.
    pub fn calls(&self) -> Vec<Call> {
        let Ok(entries) = std::fs::read_dir(self.dir.path().join("calls")) else {
            return Vec::new();
        };
        let mut dirs: Vec<(u64, u64, PathBuf)> = entries
            .map(|entry| entry.unwrap().path())
            .map(|path| {
                let name = path.file_name().unwrap().to_str().unwrap().to_owned();
                let (order, pid) = name.split_once('-').unwrap();
                (order.parse().unwrap(), pid.parse().unwrap(), path)
            })
            .collect();
        dirs.sort();
        dirs.into_iter()
            .map(|(_, _, dir)| {
                let read = |file: &str| std::fs::read_to_string(dir.join(file)).ok();
                Call {
                    argv: read("argv")
                        .unwrap_or_default()
                        .split_terminator('\0')
                        .map(str::to_owned)
                        .collect(),
                    env_names: read("env-names")
                        .unwrap_or_default()
                        .lines()
                        .map(str::to_owned)
                        .collect(),
                    env_values: read("env-values")
                        .unwrap_or_default()
                        .lines()
                        .map(str::to_owned)
                        .collect(),
                    stdin: read("stdin"),
                    cwd: PathBuf::from(read("cwd").unwrap_or_default().trim_end()),
                }
            })
            .collect()
    }

    /// The lines the fake app-server received.
    pub fn wire(&self) -> Vec<Value> {
        std::fs::read_to_string(self.dir.path().join("wire.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    /// The raw wire text.
    pub fn wire_text(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("wire.jsonl")).unwrap_or_default()
    }
}

/// A chat request as the TypeScript tests build it.
pub fn chat(provider: ProviderId, model: &str, options: Value) -> ChatRequest {
    ChatRequest::new(
        provider,
        model,
        vec![ChatMessage::new("user", "Reply with just: ok")],
    )
    .with_options(options)
}

/// A request from its wire JSON.
pub fn request(value: Value) -> ChatRequest {
    serde_json::from_value(value).unwrap()
}

/// A turn's events and outcome.
pub struct Turn {
    pub events: Vec<Value>,
    pub error: Option<String>,
}

impl Turn {
    pub fn done(&self) -> Option<&Value> {
        self.events.iter().find(|event| event["type"] == "done")
    }

    pub fn phases(&self, phase: &str) -> usize {
        self.events
            .iter()
            .filter(|event| event["phase"] == phase)
            .count()
    }
}

/// Run `turn` with a capturing sink.
pub fn capture(
    turn: impl FnOnce(&EventSink) -> Result<(), crate::protocol::BrokerFailure>,
) -> Turn {
    let (events, captured) = EventSink::capture();
    let error = turn(&events)
        .err()
        .map(|failure| failure.message().to_owned());
    Turn {
        events: captured.events(),
        error,
    }
}

/// A fresh, empty working directory for one turn.
pub fn workdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("kassiber-ai-turn-")
        .tempdir()
        .unwrap()
}
