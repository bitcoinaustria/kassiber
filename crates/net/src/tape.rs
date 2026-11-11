//! Test builds only (`test-observer`): record a process's connections as
//! plaintext tapes, and replay them without opening a socket.
//!
//! `KASSIBER_TEST_NET_TAPE=record:<file>` keeps every application byte a
//! connection reads and writes (above TLS), how many addresses it tried, and
//! how it failed. `replay:<file>` serves the recorded bytes instead of
//! connecting. A recorded failure is replayed at connect time; a recorded
//! connection is bound to the replayed one at its first write, to the unused
//! recording for that endpoint whose first write was the same bytes, since
//! parallel connections open in any order (recorded order is the fallback).
//! `replay-strict:<file>` has no fallback: a first write no unused recording
//! began with gets a connection that delivers nothing, so a request the
//! recording never made fails instead of reading another request's reply.
//! Each such miss, a connect with no recording left, and every byte a
//! connection writes past what its recording wrote, is noted in
//! `<file>.misses`, so a harness can fail a
//! replay that asked for more than the recording answered.
//! The route's consent and offline checks, the egress observer and the egress
//! ledger run as they do for a real connection; an endpoint with nothing left
//! on the tape is refused. Product builds do not compile this module, and an
//! armed tape variable has no meaning there.
//!
//! The tape is one line per event, tab separated, payloads base64:
//! `open id tls host port`, `attempts id n`, `fail id error`, `r id bytes`,
//! `w id count bytes`, `re id error`, `eof id`.

use std::collections::{HashMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use base64::Engine;

use crate::{Connection, Error};

const VARIABLE: &str = "KASSIBER_TEST_NET_TAPE";

enum Mode {
    Record(Mutex<File>),
    Replay(Mutex<Replays>),
}

#[derive(Default)]
struct Replays {
    /// Recorded connections per endpoint key, in connection order; a
    /// replayed connection takes one out (`None`).
    by_key: HashMap<String, Vec<Option<Recorded>>>,
    /// `replay-strict:`: a first write must match a recording's.
    strict: bool,
    /// Where a strict replay notes its misses.
    misses: Option<std::path::PathBuf>,
}

#[derive(Clone, Default)]
pub(crate) struct Recorded {
    attempts: u32,
    failure: Option<String>,
    first_write: Option<Vec<u8>>,
    /// Bytes the recording wrote, and (replaying) the bytes written so far.
    recorded_written: usize,
    replay_written: usize,
    events: VecDeque<Event>,
}

#[derive(Clone)]
enum Event {
    Read(Vec<u8>),
    ReadError(String),
    Write,
}

static MODE: OnceLock<Option<Mode>> = OnceLock::new();
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn b64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn unb64(text: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(text)
        .unwrap_or_default()
}

fn mode() -> Option<&'static Mode> {
    MODE.get_or_init(|| {
        let value = std::env::var(VARIABLE).ok()?;
        if let Some(path) = value.strip_prefix("record:") {
            let file = OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()?;
            return Some(Mode::Record(Mutex::new(file)));
        }
        let (path, strict) = match value.strip_prefix("replay-strict:") {
            Some(path) => (path, true),
            None => (value.strip_prefix("replay:")?, false),
        };
        let mut replays = load(&std::fs::read_to_string(path).ok()?);
        replays.strict = strict;
        replays.misses = strict.then(|| format!("{path}.misses").into());
        Some(Mode::Replay(Mutex::new(replays)))
    })
    .as_ref()
}

fn key(host: &str, port: u16, tls: bool) -> String {
    format!("{}:{host}:{port}", if tls { "tls" } else { "tcp" })
}

fn load(text: &str) -> Replays {
    let mut connections: Vec<(u64, String, Recorded)> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        let (Some(op), Some(id)) = (parts.first(), parts.get(1)) else {
            continue;
        };
        if *op == "open" && parts.len() == 5 {
            let port = parts[4].parse().unwrap_or(0);
            let host = String::from_utf8_lossy(&unb64(parts[3])).into_owned();
            index.insert((*id).to_owned(), connections.len());
            connections.push((
                id.parse().unwrap_or(0),
                key(&host, port, parts[2] == "1"),
                Recorded::default(),
            ));
            continue;
        }
        let Some(&at) = index.get(*id) else { continue };
        let recorded = &mut connections[at].2;
        match (*op, parts.get(2)) {
            ("attempts", Some(n)) => recorded.attempts = n.parse().unwrap_or(1),
            ("fail", Some(error)) => recorded.failure = Some((*error).to_owned()),
            ("r", Some(data)) => recorded.events.push_back(Event::Read(unb64(data))),
            ("re", Some(error)) => recorded
                .events
                .push_back(Event::ReadError((*error).to_owned())),
            ("w", count) => {
                recorded.recorded_written +=
                    count.and_then(|n| n.parse::<usize>().ok()).unwrap_or(0);
                if recorded.first_write.is_none() {
                    recorded.first_write = Some(parts.get(3).map(|d| unb64(d)).unwrap_or_default());
                }
                recorded.events.push_back(Event::Write);
            }
            _ => {}
        }
    }
    connections.sort_by_key(|(id, _, _)| *id);
    let mut replays = Replays::default();
    for (_, k, recorded) in connections {
        replays.by_key.entry(k).or_default().push(Some(recorded));
    }
    replays
}

/// What a replayed connect does: fail as recorded, or connect to a tape
/// that binds to its recording on first use.
pub(crate) enum Replayed {
    Failed { attempts: u32, error: Error },
    Connected { attempts: u32, tape: Tape },
}

/// `None` when this process does not replay.
pub(crate) fn replaying(host: &str, port: u16, tls: bool) -> Option<Result<Replayed, Error>> {
    let Mode::Replay(replays) = mode()? else {
        return None;
    };
    let mut replays = replays.lock().unwrap_or_else(|e| e.into_inner());
    let k = key(host, port, tls);
    let refused = || {
        Error::Io(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            "no recorded connection left on the tape",
        ))
    };
    let left = replays
        .by_key
        .get(&k)
        .is_some_and(|slots| slots.iter().any(Option::is_some));
    if !left {
        let strict = replays.strict;
        drop(replays);
        if strict {
            miss(&k, "connect with no recorded connection left");
        }
        return Some(Err(refused()));
    }
    let slots = replays.by_key.get_mut(&k).expect("checked above");
    let next = slots
        .iter_mut()
        .find(|slot| slot.is_some())
        .expect("checked above");
    let recorded = next.as_ref().expect("an unused slot");
    let attempts = recorded.attempts.max(1);
    if let Some(failure) = recorded.failure.clone() {
        next.take();
        return Some(Ok(Replayed::Failed {
            attempts,
            error: decode_error(&failure),
        }));
    }
    Some(Ok(Replayed::Connected {
        attempts,
        tape: Tape::Unbound(k),
    }))
}

/// Take the unused recorded connection to `key` whose first write was
/// `first`, or the earliest one when none matches or nothing was written.
fn bind(key: &str, first: Option<&[u8]>) -> Option<Recorded> {
    let Some(Mode::Replay(replays)) = mode() else {
        return None;
    };
    let mut replays = replays.lock().unwrap_or_else(|e| e.into_inner());
    let strict = replays.strict;
    let slots = replays.by_key.get_mut(key)?;
    match pick(slots, first, strict) {
        Some(index) => slots[index].take(),
        None => {
            drop(replays);
            if strict {
                miss(
                    key,
                    &format!("unrecorded request: {}", b64(first.unwrap_or_default())),
                );
            }
            None
        }
    }
}

/// Note a strict replay's miss beside its tape.
fn miss(key: &str, what: &str) {
    let Some(Mode::Replay(replays)) = mode() else {
        return;
    };
    let replays = replays.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(path) = &replays.misses {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = writeln!(file, "{key}\t{what}");
        }
    }
}

/// The slot `bind` takes: the first unused recording whose first write was
/// `first`; otherwise, unless strict, the earliest unused one.
fn pick(slots: &[Option<Recorded>], first: Option<&[u8]>, strict: bool) -> Option<usize> {
    let open = |slot: &Option<Recorded>| slot.as_ref().is_some_and(|r| r.failure.is_none());
    let matching = first.and_then(|bytes| {
        slots.iter().position(|slot| {
            open(slot) && slot.as_ref().and_then(|r| r.first_write.as_deref()) == Some(bytes)
        })
    });
    if strict && first.is_some() {
        return matching;
    }
    matching.or_else(|| slots.iter().position(open))
}

/// A connection's handle while recording.
pub(crate) struct Recorder {
    id: u64,
}

fn append(line: &str) {
    if let Some(Mode::Record(file)) = mode() {
        let mut file = file.lock().unwrap_or_else(|e| e.into_inner());
        let _ = writeln!(file, "{line}");
        let _ = file.flush();
    }
}

/// Start recording one connection attempt, or `None` when not recording.
pub(crate) fn recording(host: &str, port: u16, tls: bool) -> Option<Recorder> {
    let Mode::Record(_) = mode()? else {
        return None;
    };
    let id = NEXT_ID.fetch_add(1, Ordering::SeqCst);
    append(&format!(
        "open\t{id}\t{}\t{}\t{port}",
        u8::from(tls),
        b64(host.as_bytes())
    ));
    Some(Recorder { id })
}

impl Recorder {
    pub(crate) fn attempts(&self, count: u32) {
        append(&format!("attempts\t{}\t{count}", self.id));
    }

    pub(crate) fn failed(&self, error: &Error) {
        append(&format!("fail\t{}\t{}", self.id, encode_error(error)));
    }

    pub(crate) fn read(&self, result: &io::Result<usize>, bytes: &[u8]) {
        match result {
            Ok(0) => append(&format!("eof\t{}", self.id)),
            Ok(n) => append(&format!("r\t{}\t{}", self.id, b64(&bytes[..*n]))),
            Err(error) => append(&format!("re\t{}\t{}", self.id, encode_io(error))),
        }
    }

    pub(crate) fn wrote(&self, bytes: &[u8]) {
        append(&format!("w\t{}\t{}\t{}", self.id, bytes.len(), b64(bytes)));
    }
}

fn encode_io(error: &io::Error) -> String {
    format!("{:?}:{}", error.kind(), b64(error.to_string().as_bytes()))
}

fn decode_io(text: &str) -> io::Error {
    let (kind, message) = text.split_once(':').unwrap_or((text, ""));
    let kind = match kind {
        "ConnectionRefused" => io::ErrorKind::ConnectionRefused,
        "ConnectionReset" => io::ErrorKind::ConnectionReset,
        "ConnectionAborted" => io::ErrorKind::ConnectionAborted,
        "NotConnected" => io::ErrorKind::NotConnected,
        "BrokenPipe" => io::ErrorKind::BrokenPipe,
        "TimedOut" => io::ErrorKind::TimedOut,
        "WouldBlock" => io::ErrorKind::WouldBlock,
        "UnexpectedEof" => io::ErrorKind::UnexpectedEof,
        "InvalidData" => io::ErrorKind::InvalidData,
        "Interrupted" => io::ErrorKind::Interrupted,
        "NotFound" => io::ErrorKind::NotFound,
        "HostUnreachable" => io::ErrorKind::HostUnreachable,
        "NetworkUnreachable" => io::ErrorKind::NetworkUnreachable,
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, String::from_utf8_lossy(&unb64(message)).into_owned())
}

fn encode_error(error: &Error) -> String {
    match error {
        Error::Io(error) => format!("Io={}", encode_io(error)),
        other => format!("{other:?}"),
    }
}

fn decode_error(text: &str) -> Error {
    if let Some(io) = text.strip_prefix("Io=") {
        return Error::Io(decode_io(io));
    }
    match text {
        "InvalidEndpoint" => Error::InvalidEndpoint,
        "InvalidTimeout" => Error::InvalidTimeout,
        "TrustInvalid" => Error::TrustInvalid,
        "Tls" => Error::Tls,
        "HttpInvalid" => Error::HttpInvalid,
        "ProxyUnsupported" => Error::ProxyUnsupported,
        "OnionProxyRequired" => Error::OnionProxyRequired,
        "SocksProtocol" => Error::SocksProtocol,
        other => Error::Io(io::Error::other(format!(
            "recorded connection failure: {other}"
        ))),
    }
}

/// A replayed connection's bytes, bound to a recording on first use.
pub(crate) enum Tape {
    Unbound(String),
    Bound(Recorded),
}

impl Tape {
    fn bound(&mut self, first: Option<&[u8]>) -> &mut Recorded {
        if let Tape::Unbound(key) = self {
            *self = Tape::Bound(bind(key, first).unwrap_or_default());
        }
        match self {
            Tape::Bound(recorded) => recorded,
            Tape::Unbound(_) => unreachable!("bound above"),
        }
    }
}

/// The byte source behind a transport: a socket, or a replayed tape.
pub(crate) enum Stream {
    Tcp(TcpStream),
    Replay(Arc<Mutex<Tape>>),
}

impl Stream {
    pub(crate) fn set_read_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Tcp(stream) => stream.set_read_timeout(timeout),
            Stream::Replay(_) => Ok(()),
        }
    }

    pub(crate) fn set_write_timeout(&self, timeout: Option<Duration>) -> io::Result<()> {
        match self {
            Stream::Tcp(stream) => stream.set_write_timeout(timeout),
            Stream::Replay(_) => Ok(()),
        }
    }

    /// A duplicate a cancel can shut down; a tape has none.
    pub(crate) fn cancel_handle(&self) -> io::Result<Option<TcpStream>> {
        match self {
            Stream::Tcp(stream) => stream.try_clone().map(Some),
            Stream::Replay(_) => Ok(None),
        }
    }

    pub(crate) fn replay(tape: Tape) -> Self {
        Stream::Replay(Arc::new(Mutex::new(tape)))
    }
}

impl Read for Stream {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(stream) => stream.read(bytes),
            Stream::Replay(tape) => {
                let mut tape = tape.lock().unwrap_or_else(|e| e.into_inner());
                let recorded = tape.bound(None);
                loop {
                    match recorded.events.pop_front() {
                        None => return Ok(0),
                        Some(Event::Write) => continue,
                        Some(Event::ReadError(error)) => return Err(decode_io(&error)),
                        Some(Event::Read(mut data)) => {
                            let count = data.len().min(bytes.len());
                            bytes[..count].copy_from_slice(&data[..count]);
                            if count < data.len() {
                                recorded
                                    .events
                                    .push_front(Event::Read(data.split_off(count)));
                            }
                            return Ok(count);
                        }
                    }
                }
            }
        }
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Stream::Tcp(stream) => stream.write(bytes),
            Stream::Replay(tape) => {
                let mut tape = tape.lock().unwrap_or_else(|e| e.into_inner());
                let recorded = tape.bound(Some(bytes));
                let before = recorded.replay_written;
                recorded.replay_written += bytes.len();
                // A strict replay notes writing past the recording once.
                if before <= recorded.recorded_written
                    && recorded.replay_written > recorded.recorded_written
                    && recorded.recorded_written > 0
                {
                    let key = format!(
                        "{} bytes past the recording",
                        recorded.replay_written - recorded.recorded_written
                    );
                    if matches!(mode(), Some(Mode::Replay(r)) if r.lock().unwrap_or_else(|e| e.into_inner()).strict)
                    {
                        miss("write", &key);
                    }
                }
                Ok(bytes.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Stream::Tcp(stream) => stream.flush(),
            Stream::Replay(_) => Ok(()),
        }
    }
}

/// Recording wrapper around a real connection: application bytes above TLS.
pub(crate) struct Taped {
    pub(crate) inner: Box<Connection>,
    pub(crate) recorder: Recorder,
}

impl Read for Taped {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let result = self.inner.read(bytes);
        self.recorder.read(&result, bytes);
        result
    }
}

impl Write for Taped {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let count = self.inner.write(bytes)?;
        self.recorder.wrote(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_strict_tape_has_no_fallback_binding() {
        let text = [
            format!("open\t1\t0\t{}\t5000", b64(b"127.0.0.1")),
            format!("w\t1\t5\t{}", b64(b"query")),
            format!("r\t1\t{}", b64(b"reply")),
            format!("open\t2\t0\t{}\t5000", b64(b"127.0.0.1")),
            format!("w\t2\t5\t{}", b64(b"other")),
        ]
        .join("\n");
        let replays = load(&text);
        let slots = &replays.by_key[&key("127.0.0.1", 5000, false)];
        assert_eq!(pick(slots, Some(b"other"), true), Some(1));
        assert_eq!(pick(slots, Some(b"query"), false), Some(0));
        assert_eq!(pick(slots, Some(b"never"), false), Some(0));
        assert_eq!(pick(slots, Some(b"never"), true), None);
        assert_eq!(pick(slots, None, true), Some(0));
    }

    #[test]
    fn a_tape_round_trips_reads_failures_and_order() {
        let text = [
            format!("open\t1\t0\t{}\t5000", b64(b"127.0.0.1")),
            "attempts\t1\t2".to_owned(),
            format!("w\t1\t5\t{}", b64(b"query")),
            format!("r\t1\t{}", b64(b"hello world")),
            format!(
                "re\t1\t{}",
                encode_io(&io::Error::new(io::ErrorKind::TimedOut, "slow"))
            ),
            format!("open\t2\t0\t{}\t5000", b64(b"127.0.0.1")),
            format!(
                "fail\t2\t{}",
                encode_error(&Error::Io(io::Error::new(
                    io::ErrorKind::ConnectionRefused,
                    "no"
                )))
            ),
        ]
        .join("\n");
        let mut replays = load(&text);
        let mut slots = replays
            .by_key
            .remove(&key("127.0.0.1", 5000, false))
            .unwrap();
        let first = slots[0].take().unwrap();
        assert_eq!(first.attempts, 2);
        assert_eq!(first.first_write.as_deref(), Some(&b"query"[..]));
        let mut stream = Stream::replay(Tape::Bound(first));
        let mut buffer = [0u8; 5];
        assert_eq!(stream.write(b"query").unwrap(), 5);
        assert_eq!(stream.read(&mut buffer).unwrap(), 5);
        assert_eq!(&buffer, b"hello");
        let mut rest = [0u8; 16];
        assert_eq!(stream.read(&mut rest).unwrap(), 6);
        assert_eq!(
            stream.read(&mut rest).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert_eq!(stream.read(&mut rest).unwrap(), 0);
        let second = slots[1].take().unwrap();
        assert!(matches!(decode_error(second.failure.as_deref().unwrap()),
            Error::Io(e) if e.kind() == io::ErrorKind::ConnectionRefused));
    }
}
