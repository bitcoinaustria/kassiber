//! Frames, limits and the secret challenge (`kassiber/operator/protocol.py`).
//!
//! A frame is a `!cI` header, one kind byte and a big-endian u32 length,
//! then the body. `J` bodies are `json.dumps(payload, separators=(",", ":"),
//! sort_keys=True)`: compact, sorted, ASCII-escaped. `S` bodies are
//! `challenge\0secret` and never hold JSON. Secret buffers are zeroized.

use crate::error::{app, Error, Result};
use kassiber_pyjson::{dumps, Options, Value};
use std::io;
use zeroize::Zeroizing;

/// 2: brokers name their build and honour `expected_build`. 3: conditional
/// unlock and lock (`only_if_locked`, `expected_lease_id`, `restore_manual`).
pub const PROTOCOL_VERSION: i64 = 3;
pub const MAX_JSON_FRAME: usize = 8 * 1024 * 1024;
pub const MAX_SECRET_FRAME: usize = 16 * 1024;
pub const SECRET_CHALLENGE_ENTROPY_BYTES: usize = 24;
pub const SECRET_CHALLENGE_WIRE_BYTES: usize = SECRET_CHALLENGE_ENTROPY_BYTES * 2;
pub const MAX_SECRET_PAYLOAD_BYTES: usize = MAX_SECRET_FRAME - SECRET_CHALLENGE_WIRE_BYTES - 1;
pub const HEADER_BYTES: usize = 5;

/// A byte stream with Python's `recv_exact`/`send_all` contract: end of file
/// mid-read is `UnexpectedEof`, a passed deadline is `TimedOut`.
pub trait Transport: Send {
    fn recv_exact(&mut self, size: usize) -> io::Result<Vec<u8>>;
    fn send_all(&mut self, payload: &[u8]) -> io::Result<()>;
    fn close(&mut self);
}

/// What a client or server does with one connection. [`Channel`] is the
/// real implementation; tests script replies through the same trait.
pub trait FrameChannel: Send {
    fn send_json(&mut self, payload: &Value) -> Result<()>;
    fn receive_json(&mut self) -> Result<Value>;
    fn send_secret(&mut self, challenge: &str, secret: &[u8]) -> Result<()>;
    fn receive_secret(&mut self, expected_challenge: &str) -> Result<Zeroizing<Vec<u8>>>;
    /// The connecting process as the OS reports it (server side only).
    fn peer_pid(&self) -> Option<u32> {
        None
    }
    fn close(&mut self);
}

fn too_large(message: &str, code: &str) -> Error {
    Error::App(app(code, message))
}

/// The bytes of one `J` frame for `payload`, or the refusal Python raises.
pub fn encode_json_frame(payload: &Value) -> Result<Vec<u8>> {
    let body = dumps(payload, &Options::new().compact().sort_keys(true))
        .map_err(|error| Error::Value(error.to_string()))?;
    // ensure_ascii output is ASCII, so code points equal UTF-8 bytes.
    if body.len() > MAX_JSON_FRAME {
        return Err(too_large(
            "operator request is too large",
            "operator_frame_too_large",
        ));
    }
    let mut frame = Vec::with_capacity(HEADER_BYTES + body.len());
    frame.push(b'J');
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body.as_bytes());
    Ok(frame)
}

/// The bytes of one `S` frame, wiped when dropped.
pub fn encode_secret_frame(challenge: &str, secret: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    // challenge.encode("ascii") raises UnicodeEncodeError, a ValueError.
    if !challenge.is_ascii() {
        return Err(Error::Value(
            "'ascii' codec can't encode the challenge".into(),
        ));
    }
    if challenge.is_empty() || challenge.as_bytes().contains(&0) {
        return Err(Error::Value("invalid challenge".into()));
    }
    if secret.len() > MAX_SECRET_PAYLOAD_BYTES {
        return Err(Error::App(
            app("operator_secret_too_large", "operator secret is too large").retryable(false),
        ));
    }
    let body_length = challenge.len() + 1 + secret.len();
    if body_length > MAX_SECRET_FRAME {
        return Err(too_large(
            "operator secret is too large",
            "operator_secret_too_large",
        ));
    }
    let mut frame = Zeroizing::new(Vec::with_capacity(HEADER_BYTES + body_length));
    frame.push(b'S');
    frame.extend_from_slice(&(body_length as u32).to_be_bytes());
    frame.extend_from_slice(challenge.as_bytes());
    frame.push(0);
    frame.extend_from_slice(secret);
    Ok(frame)
}

/// Parse a header: the kind byte and the body length, refusing a length
/// over `limit` before reading any body.
pub fn decode_header(header: &[u8], limit: usize) -> Result<(u8, usize)> {
    debug_assert_eq!(header.len(), HEADER_BYTES);
    let size = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
    if size > limit {
        return Err(Error::App(app(
            "operator_frame_too_large",
            "operator protocol frame exceeds its limit",
        )));
    }
    Ok((header[0], size))
}

/// `receive_json` after the frame was read: kind, UTF-8, JSON, object.
pub fn decode_json_body(kind: u8, body: &[u8]) -> Result<Value> {
    if kind != b'J' {
        return Err(crate::error::protocol(
            "expected operator protocol JSON frame",
        ));
    }
    let text = std::str::from_utf8(body)
        .map_err(|_| crate::error::protocol("invalid operator protocol JSON"))?;
    let payload = match kassiber_pytext::loads(text) {
        Ok(payload) => payload,
        Err(error) if error.is_decode_error() => {
            return Err(crate::error::protocol("invalid operator protocol JSON"))
        }
        // Python lets ValueError (int digits) and RecursionError escape.
        Err(error) => return Err(Error::Value(error.to_string())),
    };
    if !matches!(payload, Value::Dict(_)) {
        return Err(crate::error::protocol(
            "operator protocol payload must be an object",
        ));
    }
    Ok(payload)
}

/// `receive_secret` after the frame was read: kind, separator, challenge,
/// payload limit.
pub fn decode_secret_body(
    kind: u8,
    body: &[u8],
    expected_challenge: &str,
) -> Result<Zeroizing<Vec<u8>>> {
    if kind != b'S' {
        return Err(crate::error::protocol(
            "expected operator protocol secret frame",
        ));
    }
    if !expected_challenge.is_ascii() {
        return Err(Error::Value(
            "'ascii' codec can't encode the expected challenge".into(),
        ));
    }
    let separator = body.iter().position(|byte| *byte == 0);
    let matched = separator
        .is_some_and(|index| constant_time_eq(&body[..index], expected_challenge.as_bytes()));
    let Some(index) = separator.filter(|_| matched) else {
        return Err(Error::App(app(
            "operator_secret_challenge_mismatch",
            "operator secret challenge did not match",
        )));
    };
    let secret = &body[index + 1..];
    if secret.len() > MAX_SECRET_PAYLOAD_BYTES {
        return Err(Error::App(app(
            "operator_secret_too_large",
            "operator secret is too large",
        )));
    }
    Ok(Zeroizing::new(secret.to_vec()))
}

/// `hmac.compare_digest`: different lengths are unequal; equal lengths are
/// compared without an early exit.
pub fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut difference = 0u8;
    for (a, b) in left.iter().zip(right) {
        difference |= a ^ b;
    }
    difference == 0
}

/// `secrets.token_hex(SECRET_CHALLENGE_ENTROPY_BYTES)`: 48 lowercase hex
/// characters from the OS generator.
pub fn new_challenge() -> Result<String> {
    token_hex(SECRET_CHALLENGE_ENTROPY_BYTES)
}

/// `secrets.token_hex(bytes)`.
pub fn token_hex(bytes: usize) -> Result<String> {
    let mut raw = Zeroizing::new(vec![0u8; bytes]);
    getrandom::fill(&mut raw).map_err(|_| Error::Io(io::Error::other("entropy unavailable")))?;
    Ok(raw.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Length-prefixed frames over one transport, with a distinct non-JSON
/// secret frame type.
pub struct Channel<T: Transport> {
    transport: T,
    peer_pid: Option<u32>,
}

impl<T: Transport> Channel<T> {
    pub fn new(transport: T, peer_pid: Option<u32>) -> Self {
        Channel {
            transport,
            peer_pid,
        }
    }

    fn io(error: io::Error) -> Error {
        if error.kind() == io::ErrorKind::UnexpectedEof {
            Error::Eof("operator IPC connection closed")
        } else {
            Error::Io(error)
        }
    }

    fn receive_frame(&mut self, limit: usize) -> Result<(u8, Vec<u8>)> {
        let header = self.transport.recv_exact(HEADER_BYTES).map_err(Self::io)?;
        let (kind, size) = decode_header(&header, limit)?;
        let body = self.transport.recv_exact(size).map_err(Self::io)?;
        Ok((kind, body))
    }
}

impl<T: Transport> FrameChannel for Channel<T> {
    fn send_json(&mut self, payload: &Value) -> Result<()> {
        let frame = encode_json_frame(payload)?;
        self.transport.send_all(&frame).map_err(Self::io)
    }

    fn receive_json(&mut self) -> Result<Value> {
        let (kind, body) = self.receive_frame(MAX_JSON_FRAME)?;
        decode_json_body(kind, &body)
    }

    fn send_secret(&mut self, challenge: &str, secret: &[u8]) -> Result<()> {
        let frame = encode_secret_frame(challenge, secret)?;
        self.transport.send_all(&frame).map_err(Self::io)
    }

    fn receive_secret(&mut self, expected_challenge: &str) -> Result<Zeroizing<Vec<u8>>> {
        let (kind, body) = self.receive_frame(MAX_SECRET_FRAME)?;
        let body = Zeroizing::new(body);
        decode_secret_body(kind, &body, expected_challenge)
    }

    fn peer_pid(&self) -> Option<u32> {
        self.peer_pid
    }

    fn close(&mut self) {
        self.transport.close();
    }
}

impl<T: Transport> Drop for Channel<T> {
    fn drop(&mut self) {
        self.transport.close();
    }
}

/// The error Python reports with an `AppError` code but no field the
/// caller should read, for tests and callers matching codes.
pub fn is_code(error: &Error, code: &str) -> bool {
    error.code() == Some(code)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    /// `LoopbackTransport` from test_operator_protocol.py.
    #[derive(Clone, Default)]
    struct Loopback(Arc<Mutex<VecDeque<u8>>>);

    impl Transport for Loopback {
        fn recv_exact(&mut self, size: usize) -> io::Result<Vec<u8>> {
            let mut buffer = self.0.lock().unwrap();
            if buffer.len() < size {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            Ok(buffer.drain(..size).collect())
        }
        fn send_all(&mut self, payload: &[u8]) -> io::Result<()> {
            self.0.lock().unwrap().extend(payload);
            Ok(())
        }
        fn close(&mut self) {}
    }

    fn pair() -> (Channel<Loopback>, Channel<Loopback>, Loopback) {
        let shared = Loopback::default();
        (
            Channel::new(shared.clone(), None),
            Channel::new(shared.clone(), None),
            shared,
        )
    }

    fn parse(text: &str) -> Value {
        kassiber_pytext::loads(text).unwrap()
    }

    #[test]
    fn json_and_challenge_bound_secret_frames_are_distinct() {
        let (mut sender, mut receiver, _) = pair();
        sender
            .send_json(&parse(r#"{"action": "unlock", "version": 1}"#))
            .unwrap();
        assert_eq!(
            receiver.receive_json().unwrap(),
            parse(r#"{"action": "unlock", "version": 1}"#)
        );
        sender.send_secret("challenge", b"not-in-json").unwrap();
        assert_eq!(
            receiver.receive_secret("challenge").unwrap().as_slice(),
            b"not-in-json"
        );
        // A secret frame where JSON is expected, and the reverse.
        sender.send_secret("challenge", b"x").unwrap();
        let error = receiver.receive_json().unwrap_err();
        assert_eq!(error.code(), Some("operator_protocol_error"));
        assert_eq!(error.to_string(), "expected operator protocol JSON frame");
        sender.send_json(&parse("{}")).unwrap();
        let error = receiver.receive_secret("challenge").unwrap_err();
        assert_eq!(error.to_string(), "expected operator protocol secret frame");
    }

    #[test]
    fn wrong_secret_challenge_fails_closed() {
        let (mut sender, mut receiver, shared) = pair();
        sender.send_secret("first", b"secret").unwrap();
        let error = receiver.receive_secret("second").unwrap_err();
        assert_eq!(error.code(), Some("operator_secret_challenge_mismatch"));
        assert_eq!(error.to_string(), "operator secret challenge did not match");
        // A prefix of the expected challenge and a frame without separator.
        sender.send_secret("firs", b"secret").unwrap();
        assert!(is_code(
            &receiver.receive_secret("first").unwrap_err(),
            "operator_secret_challenge_mismatch"
        ));
        shared
            .0
            .lock()
            .unwrap()
            .extend(b"S\x00\x00\x00\x05first".iter());
        assert!(is_code(
            &receiver.receive_secret("first").unwrap_err(),
            "operator_secret_challenge_mismatch"
        ));
    }

    #[test]
    fn challenge_comparison_is_exact_and_length_checked() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(!constant_time_eq(b"", b"a"));
        assert!(constant_time_eq(b"", b""));
        let challenge = new_challenge().unwrap();
        assert_eq!(challenge.len(), SECRET_CHALLENGE_WIRE_BYTES);
        assert!(challenge
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()));
        assert_ne!(challenge, new_challenge().unwrap());
    }

    #[test]
    fn secret_payload_limit_round_trips_at_the_shared_boundary() {
        let (mut sender, mut receiver, _) = pair();
        let challenge = "a".repeat(SECRET_CHALLENGE_WIRE_BYTES);
        let payload = vec![b's'; MAX_SECRET_PAYLOAD_BYTES];
        sender.send_secret(&challenge, &payload).unwrap();
        assert_eq!(
            receiver.receive_secret(&challenge).unwrap().as_slice(),
            payload.as_slice()
        );
        let mut over = payload.clone();
        over.push(b's');
        let error = sender.send_secret(&challenge, &over).unwrap_err();
        assert_eq!(error.code(), Some("operator_secret_too_large"));
        // A longer challenge pushes the frame, not the payload, over 16 KiB.
        let long = "a".repeat(SECRET_CHALLENGE_WIRE_BYTES + 1);
        let error = sender.send_secret(&long, &payload).unwrap_err();
        assert_eq!(error.code(), Some("operator_secret_too_large"));
        assert_eq!(MAX_SECRET_PAYLOAD_BYTES, 16_335);
        // A short challenge leaves room in the frame, not in the payload.
        let mut raw = vec![b'S'];
        raw.extend_from_slice(&((2 + MAX_SECRET_PAYLOAD_BYTES + 1) as u32).to_be_bytes());
        raw.extend_from_slice(b"c\0");
        raw.extend(std::iter::repeat_n(b's', MAX_SECRET_PAYLOAD_BYTES + 1));
        let mut receiver = Channel::new(Loopback(Arc::new(Mutex::new(raw.into()))), None);
        let error = receiver.receive_secret("c").unwrap_err();
        assert_eq!(error.code(), Some("operator_secret_too_large"));
    }

    #[test]
    fn frame_limits_hold_at_their_boundaries() {
        let header = |kind: u8, size: u32| {
            let mut bytes = vec![kind];
            bytes.extend_from_slice(&size.to_be_bytes());
            bytes
        };
        assert_eq!(
            decode_header(&header(b'J', MAX_JSON_FRAME as u32), MAX_JSON_FRAME).unwrap(),
            (b'J', MAX_JSON_FRAME)
        );
        let error =
            decode_header(&header(b'J', MAX_JSON_FRAME as u32 + 1), MAX_JSON_FRAME).unwrap_err();
        assert_eq!(error.code(), Some("operator_frame_too_large"));
        assert_eq!(
            error.to_string(),
            "operator protocol frame exceeds its limit"
        );
        assert!(decode_header(&header(b'S', MAX_SECRET_FRAME as u32), MAX_SECRET_FRAME).is_ok());
        assert!(
            decode_header(&header(b'S', MAX_SECRET_FRAME as u32 + 1), MAX_SECRET_FRAME).is_err()
        );
        assert!(decode_header(&header(b'J', u32::MAX), MAX_JSON_FRAME).is_err());

        // A body of exactly 8 MiB is sent; one byte more is refused.
        let filler = |length: usize| {
            // {"x":"..."} is 8 bytes of structure around the string.
            Value::Dict(vec![(
                kassiber_pyjson::Key::from("x"),
                Value::Str("y".repeat(length - 8).as_str().into()),
            )])
        };
        let frame = encode_json_frame(&filler(MAX_JSON_FRAME)).unwrap();
        assert_eq!(frame.len(), HEADER_BYTES + MAX_JSON_FRAME);
        let error = encode_json_frame(&filler(MAX_JSON_FRAME + 1)).unwrap_err();
        assert_eq!(error.code(), Some("operator_frame_too_large"));
        assert_eq!(error.to_string(), "operator request is too large");
        // Non-ASCII counts as its escaped length, as Python measures it.
        let escaped = Value::Dict(vec![(
            kassiber_pyjson::Key::from("x"),
            Value::Str(
                "\u{e4}"
                    .repeat((MAX_JSON_FRAME - 8) / 6 + 1)
                    .as_str()
                    .into(),
            ),
        )]);
        assert!(encode_json_frame(&escaped).is_err());
    }

    #[test]
    fn malformed_json_frames_are_protocol_errors() {
        for (body, message) in [
            (&b"\xff"[..], "invalid operator protocol JSON"),
            (b"{", "invalid operator protocol JSON"),
            (b"[]", "operator protocol payload must be an object"),
            (b"1", "operator protocol payload must be an object"),
        ] {
            let error = decode_json_body(b'J', body).unwrap_err();
            assert_eq!(error.code(), Some("operator_protocol_error"));
            assert_eq!(error.to_string(), message);
            assert!(!error.app().unwrap().retryable);
        }
        assert!(matches!(
            decode_json_body(b'J', format!("{{\"x\": {}}}", "9".repeat(5000)).as_bytes()),
            Err(Error::Value(_))
        ));
        // Python's json.loads accepts NaN and lone surrogate escapes.
        let decoded = decode_json_body(b'J', br#"{"a": NaN, "b": "\ud800"}"#).unwrap();
        assert_eq!(
            encode_json_frame(&decoded).unwrap(),
            b"J\x00\x00\x00\x16{\"a\":NaN,\"b\":\"\\ud800\"}"
        );
    }

    #[test]
    fn invalid_challenges_are_programming_errors() {
        for challenge in ["", "a\0b", "\u{e4}"] {
            assert!(matches!(
                encode_secret_frame(challenge, b"x"),
                Err(Error::Value(_))
            ));
        }
    }

    #[test]
    fn end_of_file_mid_frame_is_eof() {
        let (_, mut receiver, shared) = pair();
        shared.0.lock().unwrap().extend(b"J\x00\x00".iter());
        assert!(matches!(receiver.receive_json(), Err(Error::Eof(_))));
    }
}
