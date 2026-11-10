//! A scripted host: each connect takes the next scripted connection, each
//! receive the next scripted reply; everything sent is recorded.
#![allow(dead_code)]

use kassiber_operator::client::Host;
use kassiber_operator::protocol::{encode_json_frame, encode_secret_frame, FrameChannel};
use kassiber_operator::{Error, Result};
use kassiber_pyjson::{PyStr, Value};
use std::collections::{HashMap, VecDeque};
use std::io;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use zeroize::Zeroizing;

pub fn parse(text: &str) -> Value {
    kassiber_pytext::loads(text).unwrap()
}

/// What one receive answers.
#[derive(Clone, Debug)]
pub enum Reply {
    Json(Value),
    Io(io::ErrorKind),
    Eof,
}

pub fn ok(data: &str) -> Reply {
    Reply::Json(parse(&format!(r#"{{"ok": true, "data": {data}}}"#)))
}

pub fn reply(text: &str) -> Reply {
    Reply::Json(parse(text))
}

pub fn failure(code: &str) -> Reply {
    Reply::Json(parse(&format!(
        r#"{{"ok": false, "error": {{"code": "{code}", "message": "{code}", "hint": null, "details": {{}}, "retryable": false}}}}"#
    )))
}

/// What one connect answers.
#[derive(Clone, Debug)]
pub enum Connection {
    Refused,
    Replies(Vec<Reply>),
}

#[derive(Default)]
pub struct Log {
    /// One entry per connection: the frames sent on it, as bytes.
    pub connections: Vec<Vec<Vec<u8>>>,
    /// (connect timeout, I/O timeout) per connect attempt.
    pub connects: Vec<(Duration, Option<Duration>)>,
    pub secrets: Vec<(String, Vec<u8>)>,
    pub launches: usize,
    pub warnings: Vec<String>,
    pub closed: usize,
}

impl Log {
    /// The JSON frames sent on every connection, decoded.
    pub fn sent(&self) -> Vec<Value> {
        self.connections
            .iter()
            .flatten()
            .filter(|frame| frame[0] == b'J')
            .map(|frame| parse(std::str::from_utf8(&frame[5..]).unwrap()))
            .collect()
    }

    pub fn actions(&self) -> Vec<String> {
        self.sent()
            .iter()
            .filter_map(|frame| match kassiber_envelope::get(frame, "action") {
                Some(Value::Str(action)) => action.as_str().map(str::to_owned),
                _ => None,
            })
            .collect()
    }

    /// Every frame of every connection, in order, as raw bytes.
    pub fn bytes(&self) -> Vec<u8> {
        self.connections
            .iter()
            .flatten()
            .flatten()
            .copied()
            .collect()
    }
}

pub struct ScriptedHost {
    pub script: Mutex<VecDeque<Connection>>,
    /// Scripted connections a launch adds (the broker it starts).
    pub after_launch: Mutex<VecDeque<Connection>>,
    pub log: Arc<Mutex<Log>>,
    pub build: Value,
    pub tokens: Mutex<VecDeque<String>>,
    pub cwd: io::Result<PyStr>,
    pub env: HashMap<String, String>,
    pub clock: Mutex<Instant>,
    pub startup: Duration,
}

pub fn this_build() -> Value {
    parse(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": false, "origin": "0123456789ab"}"#,
    )
}

pub fn other_build() -> Value {
    parse(
        r#"{"version": "0.22.78", "channel": null, "commit": null, "frozen": false, "origin": "ffffffffffff"}"#,
    )
}

impl ScriptedHost {
    pub fn new(script: Vec<Connection>) -> Self {
        ScriptedHost {
            script: Mutex::new(script.into()),
            after_launch: Mutex::new(VecDeque::new()),
            log: Arc::default(),
            build: this_build(),
            tokens: Mutex::new(VecDeque::new()),
            cwd: Ok(PyStr::from("/caller")),
            env: HashMap::new(),
            clock: Mutex::new(Instant::now()),
            startup: Duration::from_secs(5),
        }
    }

    pub fn with_tokens(self, tokens: &[&str]) -> Self {
        *self.tokens.lock().unwrap() = tokens.iter().map(|t| t.to_string()).collect();
        self
    }

    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    pub fn launching(self, connections: Vec<Connection>) -> Self {
        *self.after_launch.lock().unwrap() = connections.into();
        self
    }

    pub fn log(&self) -> std::sync::MutexGuard<'_, Log> {
        self.log.lock().unwrap()
    }

    pub fn remaining(&self) -> usize {
        self.script.lock().unwrap().len()
    }
}

struct ScriptedChannel {
    replies: VecDeque<Reply>,
    log: Arc<Mutex<Log>>,
    index: usize,
}

impl FrameChannel for ScriptedChannel {
    fn send_json(&mut self, payload: &Value) -> Result<()> {
        let frame = encode_json_frame(payload)?;
        self.log.lock().unwrap().connections[self.index].push(frame);
        Ok(())
    }

    fn receive_json(&mut self) -> Result<Value> {
        match self.replies.pop_front() {
            Some(Reply::Json(value)) => Ok(value),
            Some(Reply::Io(kind)) => Err(Error::Io(kind.into())),
            Some(Reply::Eof) | None => Err(Error::Eof("operator IPC connection closed")),
        }
    }

    fn send_secret(&mut self, challenge: &str, secret: &[u8]) -> Result<()> {
        let frame = encode_secret_frame(challenge, secret)?;
        let mut log = self.log.lock().unwrap();
        log.connections[self.index].push(frame.to_vec());
        log.secrets.push((challenge.to_owned(), secret.to_vec()));
        Ok(())
    }

    fn receive_secret(&mut self, _: &str) -> Result<Zeroizing<Vec<u8>>> {
        unimplemented!("clients never receive secrets")
    }

    fn close(&mut self) {
        self.log.lock().unwrap().closed += 1;
    }
}

impl Host for ScriptedHost {
    fn connect(
        &self,
        timeout: Duration,
        io_timeout: Option<Duration>,
    ) -> Result<Box<dyn FrameChannel>> {
        self.log().connects.push((timeout, io_timeout));
        let next = self.script.lock().unwrap().pop_front();
        match next.unwrap_or(Connection::Refused) {
            Connection::Refused => Err(Error::Io(io::ErrorKind::ConnectionRefused.into())),
            Connection::Replies(replies) => {
                let mut log = self.log();
                log.connections.push(Vec::new());
                let index = log.connections.len() - 1;
                Ok(Box::new(ScriptedChannel {
                    replies: replies.into(),
                    log: Arc::clone(&self.log),
                    index,
                }))
            }
        }
    }

    fn launch_broker(&self) -> Result<()> {
        self.log().launches += 1;
        let mut launched = self.after_launch.lock().unwrap();
        let mut script = self.script.lock().unwrap();
        while let Some(connection) = launched.pop_back() {
            script.push_front(connection);
        }
        Ok(())
    }

    fn startup_timeout(&self) -> Duration {
        self.startup
    }

    fn build_identity(&self) -> Value {
        self.build.clone()
    }

    fn token_hex(&self, bytes: usize) -> Result<String> {
        Ok(self
            .tokens
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or_else(|| "a".repeat(bytes * 2)))
    }

    fn current_dir(&self) -> io::Result<PyStr> {
        match &self.cwd {
            Ok(text) => Ok(text.clone()),
            Err(error) => Err(io::Error::from(error.kind())),
        }
    }

    fn env_var(&self, key: &str) -> Option<String> {
        self.env.get(key).cloned()
    }

    fn now(&self) -> Instant {
        *self.clock.lock().unwrap()
    }

    fn sleep(&self, duration: Duration) {
        *self.clock.lock().unwrap() += duration;
    }

    fn warn(&self, text: &str) {
        self.log().warnings.push(text.to_owned());
    }
}

/// A ping reply from a broker of `build` that supports caller context.
pub fn ping(build: &Value) -> Reply {
    let mut data = parse(
        r#"{"broker": "running", "generation": "g", "native_auth_available": false, "native_auth_identity": null, "caller_context": true}"#,
    );
    kassiber_envelope::set(&mut data, "build", build.clone());
    Reply::Json(kassiber_envelope::dict([
        ("ok", Value::Bool(true)),
        ("data", data),
    ]))
}

pub fn ping_connection(build: &Value) -> Connection {
    Connection::Replies(vec![ping(build)])
}
