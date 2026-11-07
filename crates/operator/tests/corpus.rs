//! Replays `tests/data/protocol_corpus.json`, which
//! `scripts/generate_protocol_corpus.py` writes from the Python reference:
//! every frame byte for byte, every refusal by class, code and message, and
//! every client scenario by the exact bytes sent on each connection.

mod support;

use kassiber_envelope::get;
use kassiber_operator::client::{BrokerClient, PreparedArguments, SubmitOptions};
use kassiber_operator::protocol::{
    decode_json_body, encode_json_frame, encode_secret_frame, Channel, FrameChannel, Transport,
    MAX_SECRET_PAYLOAD_BYTES,
};
use kassiber_operator::values::py_eq;
use kassiber_operator::Error;
use kassiber_pyjson::{PyStr, Value};
use std::time::Duration;
use support::{Connection, Reply, ScriptedHost};

fn corpus() -> Value {
    kassiber_pytext::loads(include_str!("data/protocol_corpus.json")).unwrap()
}

fn text(value: &Value) -> String {
    match value {
        Value::Str(text) => text.as_str().expect("UTF-8 corpus text").to_owned(),
        other => panic!("expected text, found {other:?}"),
    }
}

fn list(value: Option<&Value>) -> &[Value] {
    match value {
        Some(Value::List(items)) => items,
        other => panic!("expected a list, found {other:?}"),
    }
}

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

struct Bytes(std::collections::VecDeque<u8>);

impl Transport for Bytes {
    fn recv_exact(&mut self, size: usize) -> std::io::Result<Vec<u8>> {
        if self.0.len() < size {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        Ok(self.0.drain(..size).collect())
    }
    fn send_all(&mut self, _: &[u8]) -> std::io::Result<()> {
        Ok(())
    }
    fn close(&mut self) {}
}

/// The recorded exception equals this error: class, and for an AppError
/// its code, message (code points), hint, details and retryable flag.
fn assert_raises(expected: &Value, error: &Error, context: &str) {
    let class = text(get(expected, "class").unwrap());
    assert_eq!(error.python_class(), class, "{context}: {error:?}");
    let Some(app) = error.app() else { return };
    assert_eq!(app.code, text(get(expected, "code").unwrap()), "{context}");
    let message = match get(expected, "message") {
        Some(Value::Str(message)) => message.clone(),
        other => panic!("{context}: message {other:?}"),
    };
    let actual = app
        .message_codepoints
        .as_deref()
        .cloned()
        .unwrap_or_else(|| PyStr::from(app.message.as_str()));
    assert_eq!(actual, message, "{context}");
    let hint = get(expected, "hint").unwrap();
    match hint {
        Value::Null => assert_eq!(app.hint, None, "{context}"),
        hint => assert_eq!(app.hint.as_deref(), Some(text(hint).as_str()), "{context}"),
    }
    let details = get(expected, "details").unwrap();
    match (&app.details, details) {
        (None, Value::Null) => {}
        (Some(actual), expected) => assert!(py_eq(actual, expected), "{context}: {actual:?}"),
        (None, other) => panic!("{context}: missing details {other:?}"),
    }
    assert_eq!(
        Value::Bool(app.retryable),
        *get(expected, "retryable").unwrap(),
        "{context}"
    );
}

#[test]
fn json_frames_match_python_bytes() {
    let corpus = corpus();
    let frames = list(get(&corpus, "json_frames"));
    assert!(frames.len() >= 10);
    for frame in frames {
        let payload = kassiber_pytext::loads(&text(get(frame, "payload").unwrap())).unwrap();
        let expected = hex(&text(get(frame, "frame_hex").unwrap()));
        assert_eq!(
            to_hex(&encode_json_frame(&payload).unwrap()),
            to_hex(&expected)
        );
        let decoded = decode_json_body(expected[0], &expected[5..]).unwrap();
        assert_eq!(encode_json_frame(&decoded).unwrap(), expected);
    }
}

#[test]
fn secret_frames_match_python_bytes() {
    let corpus = corpus();
    assert_eq!(
        get(get(&corpus, "limits").unwrap(), "max_secret_payload_bytes"),
        Some(&Value::from(MAX_SECRET_PAYLOAD_BYTES as u64))
    );
    for frame in list(get(&corpus, "secret_frames")) {
        let challenge = text(get(frame, "challenge").unwrap());
        let secret = hex(&text(get(frame, "secret_hex").unwrap()));
        let expected = hex(&text(get(frame, "frame_hex").unwrap()));
        assert_eq!(
            encode_secret_frame(&challenge, &secret).unwrap().as_slice(),
            expected.as_slice()
        );
        let mut channel = Channel::new(Bytes(expected.into()), None);
        assert_eq!(
            channel.receive_secret(&challenge).unwrap().as_slice(),
            secret.as_slice()
        );
    }
}

#[test]
fn refused_frames_raise_what_python_raises() {
    let corpus = corpus();
    for case in list(get(&corpus, "decode_errors")) {
        let frame = hex(&text(get(case, "frame_hex").unwrap()));
        let mut channel = Channel::new(Bytes(frame.clone().into()), None);
        let challenge = text(get(case, "challenge").unwrap());
        let context = to_hex(&frame[..frame.len().min(12)]);
        let result = if text(get(case, "kind").unwrap()) == "json" {
            channel.receive_json().map(|value| ("value", value))
        } else {
            channel
                .receive_secret(&challenge)
                .map(|secret| ("secret_hex", kassiber_envelope::s(to_hex(&secret))))
        };
        match (get(case, "raises"), result) {
            (Some(expected), Err(error)) => assert_raises(expected, &error, &context),
            (None, Ok((key, value))) => {
                assert!(py_eq(get(case, key).unwrap(), &value), "{context}")
            }
            (expected, actual) => panic!("{context}: expected {expected:?}, got {actual:?}"),
        }
    }
}

fn reply(value: &Value) -> Reply {
    if let Some(json) = get(value, "json") {
        return Reply::Json(json.clone());
    }
    if let Some(kind) = get(value, "io") {
        return Reply::Io(match text(kind).as_str() {
            "ConnectionResetError" => std::io::ErrorKind::ConnectionReset,
            other => panic!("unknown failure {other}"),
        });
    }
    Reply::Eof
}

fn connection(value: &Value) -> Connection {
    match value {
        Value::Str(refused) if refused.is("refused") => Connection::Refused,
        Value::List(replies) => Connection::Replies(replies.iter().map(reply).collect()),
        other => panic!("unknown connection {other:?}"),
    }
}

fn base64(value: &Value) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let Value::Str(encoded) = value else {
        return None;
    };
    let mut bits = 0u32;
    let mut count = 0;
    let mut out = Vec::new();
    for byte in encoded.as_str().unwrap().bytes().filter(|b| *b != b'=') {
        bits = bits << 6 | ALPHABET.iter().position(|c| *c == byte).unwrap() as u32;
        count += 6;
        if count >= 8 {
            count -= 8;
            out.push((bits >> count) as u8);
        }
    }
    Some(out)
}

fn opt_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        other => Some(text(other)),
    }
}

fn opt_int(value: &Value) -> Option<i64> {
    match value {
        Value::Null => None,
        Value::Int(n) => Some(i64::try_from(n.clone()).unwrap()),
        other => panic!("expected an int, found {other:?}"),
    }
}

fn flag(value: &Value) -> bool {
    matches!(value, Value::Bool(true))
}

fn run(client: &BrokerClient<ScriptedHost>, call: &[Value]) -> kassiber_operator::Result<Value> {
    let method = text(&call[0]);
    match method.as_str() {
        "ping" => client.ping(),
        "status" => client.status(opt_text(&call[1]).as_deref()),
        "unlock" => client.unlock(
            &text(&call[1]),
            &base64(&call[2]).unwrap(),
            opt_int(&call[3]).map(Into::into),
            &text(&call[4]),
            &text(&call[5]),
            flag(&call[6]),
            base64(&call[7]).as_deref(),
            opt_int(&call[8]),
        ),
        "lock" => client.lock(
            &text(&call[1]),
            opt_text(&call[2]).as_deref(),
            flag(&call[3]),
        ),
        "submit" => {
            let prepared = PreparedArguments {
                argv: list(Some(&call[2])).iter().map(text).collect(),
                secrets: list(Some(&call[3]))
                    .iter()
                    .map(|pair| {
                        let pair = list(Some(pair));
                        (
                            text(&pair[0]),
                            zeroize::Zeroizing::new(base64(&pair[1]).unwrap()),
                        )
                    })
                    .collect(),
            };
            let token = opt_text(&call[8]);
            client.submit(
                &text(&call[1]),
                &prepared,
                base64(&call[4]).as_deref(),
                &SubmitOptions {
                    start_broker: flag(&call[5]),
                    require_caller_context: flag(&call[6]),
                    require_same_build: flag(&call[7]),
                    agent_session_token: token.as_deref(),
                },
            )
        }
        "operation_status" => client.operation_status(&PyStr::from(text(&call[1]))),
        "cancel" => client.cancel(&PyStr::from(text(&call[1]))),
        "set_mode" => client.set_mode(&text(&call[1]), &text(&call[2]), &base64(&call[3]).unwrap()),
        "stop" => client.stop(),
        "open_agent_session" => client
            .open_agent_session(&text(&call[1]))
            .map(|mut handle| {
                let value = kassiber_envelope::dict([
                    ("session_id", Value::Str(handle.session_id.clone())),
                    ("token", Value::Str(handle.token.clone())),
                ]);
                handle.close();
                value
            }),
        "decide_agent_session" => client.decide_agent_session(
            &text(&call[1]),
            &text(&call[2]),
            flag(&call[3]),
            &base64(&call[4]).unwrap(),
        ),
        "unlock_touch_id" => client.unlock_touch_id(
            &text(&call[1]),
            opt_int(&call[2]).map(Into::into),
            &text(&call[3]),
            None,
        ),
        "configure_touch_id" => client.configure_touch_id(
            &text(&call[1]),
            &base64(&call[2]).unwrap(),
            flag(&call[3]),
            None,
        ),
        other => panic!("unknown corpus call {other}"),
    }
}

fn seconds(value: &Value) -> Option<Duration> {
    match value {
        Value::Null => None,
        Value::Float(f) => Some(Duration::from_secs_f64(*f)),
        Value::Int(n) => Some(Duration::from_secs(u64::try_from(n.clone()).unwrap())),
        other => panic!("expected seconds, found {other:?}"),
    }
}

#[test]
fn client_scenarios_send_python_bytes_and_return_python_results() {
    let corpus = corpus();
    let scenarios = list(get(&corpus, "client"));
    assert!(scenarios.len() >= 28);
    let mut actions = std::collections::BTreeSet::new();
    for scenario in scenarios {
        let name = text(get(scenario, "name").unwrap());
        let mut host = ScriptedHost::new(
            list(get(scenario, "connections"))
                .iter()
                .map(connection)
                .collect(),
        )
        .with_tokens(&[&"f".repeat(32)]);
        if let Some(Value::Dict(environment)) = get(scenario, "env") {
            for (key, value) in environment {
                let kassiber_pyjson::Key::Str(key) = key else {
                    unreachable!()
                };
                host = host.with_env(key.as_str().unwrap(), &text(value));
            }
        }
        let client = BrokerClient::new(host);
        let call = list(get(scenario, "call"));
        let result = run(&client, call);
        let log = client.host().log();
        let sent: Vec<String> = log
            .connections
            .iter()
            .map(|frames| to_hex(&frames.concat()))
            .collect();
        let expected: Vec<String> = list(get(scenario, "sent_hex")).iter().map(text).collect();
        assert_eq!(sent, expected, "{name}: bytes sent per connection");
        let timeouts: Vec<(Duration, Option<Duration>)> = list(get(scenario, "timeouts"))
            .iter()
            .map(|pair| {
                let pair = list(Some(pair));
                (seconds(&pair[0]).unwrap(), seconds(&pair[1]))
            })
            .collect();
        assert_eq!(log.connects, timeouts, "{name}: connect timeouts");
        assert_eq!(
            Value::from(log.launches as u64),
            *get(scenario, "launches").unwrap(),
            "{name}: launches"
        );
        match (get(scenario, "raises"), result) {
            (Some(expected), Err(error)) => assert_raises(expected, &error, &name),
            (None, Ok(value)) => assert!(
                py_eq(get(scenario, "value").unwrap(), &value),
                "{name}: {value:?}"
            ),
            (expected, actual) => panic!("{name}: expected {expected:?}, got {actual:?}"),
        }
        for frame in log.sent() {
            if let Some(Value::Str(action)) = get(&frame, "action") {
                actions.insert(action.as_str().unwrap().to_owned());
            }
        }
    }
    // Every action but the heartbeat (a json_frames entry) is exercised.
    assert_eq!(
        actions.into_iter().collect::<Vec<_>>(),
        [
            "agent_session_decide",
            "agent_session_open",
            "lock",
            "operation_cancel",
            "operation_status",
            "ping",
            "restart_for_native_auth",
            "set_mode",
            "status",
            "submit",
            "touch_id_configure",
            "unlock",
            "unlock_touch_id",
        ]
    );
}

#[test]
fn heartbeat_frame_is_in_the_corpus() {
    let corpus = corpus();
    let heartbeat = encode_json_frame(
        &kassiber_pytext::loads(r#"{"version": 3, "action": "agent_session_heartbeat"}"#).unwrap(),
    )
    .unwrap();
    assert!(list(get(&corpus, "json_frames"))
        .iter()
        .any(|frame| hex(&text(get(frame, "frame_hex").unwrap())) == heartbeat));
}
