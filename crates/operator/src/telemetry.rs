//! The broker's lifecycle log: Python's `kassiber.operator` logger feeding
//! the bounded RAM ring (`log_ring.install_ring_logging`).
//!
//! Records stay in this process's memory and are never written to disk.
//! Strings pass the secret floor at insert, and callers log only public ids,
//! command paths and states, never paths, argv or secrets.

use kassiber_envelope::redaction::redact_secret_text;
use kassiber_pyjson::Value;
use std::collections::VecDeque;
use std::sync::Mutex;

/// `logging.INFO`, `WARNING` and `ERROR`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Level {
    Info,
    Warning,
    Error,
}

/// One record: the message and its `kb_fields`.
#[derive(Clone, Debug, PartialEq)]
pub struct Record {
    pub level: Level,
    pub message: String,
    pub fields: Vec<(String, Value)>,
}

/// Where the service sends its lifecycle records.
pub trait Telemetry: Send + Sync {
    fn record(&self, level: Level, message: &str, fields: Vec<(&str, Value)>);
}

/// Discards every record.
pub struct Discard;

impl Telemetry for Discard {
    fn record(&self, _: Level, _: &str, _: Vec<(&str, Value)>) {}
}

/// A bounded in-memory ring with secret-floor redaction at insert.
pub struct LogRing {
    capacity: usize,
    records: Mutex<VecDeque<Record>>,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        LogRing {
            capacity: capacity.max(1),
            records: Mutex::new(VecDeque::new()),
        }
    }

    /// The retained records, oldest first.
    pub fn snapshot(&self) -> Vec<Record> {
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .iter()
            .cloned()
            .collect()
    }
}

fn redacted(value: Value) -> Value {
    match value {
        Value::Str(text) => match text.as_str() {
            Some(text) => kassiber_envelope::s(redact_secret_text(text).as_str()),
            None => Value::Str(kassiber_envelope::redaction::redact_secret_py(&text)),
        },
        other => other,
    }
}

impl Telemetry for LogRing {
    fn record(&self, level: Level, message: &str, fields: Vec<(&str, Value)>) {
        let record = Record {
            level,
            message: redact_secret_text(message),
            fields: fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), redacted(value)))
                .collect(),
        };
        let mut records = self
            .records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        while records.len() >= self.capacity {
            records.pop_front();
        }
        records.push_back(record);
    }
}
