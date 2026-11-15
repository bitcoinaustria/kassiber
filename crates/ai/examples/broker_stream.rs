//! The Rust side of `tests/test_rust_ai_broker.py`: drives a
//! [`BrokerClient`] through the chat legs a JSON scenario on stdin names and
//! prints every chunk, and each leg's outcome, as one JSON line, in the
//! shape `tests/rust_port/ai_broker_reference.py` prints for Python's
//! `BrokerAIClient`. The broker is found as the product finds it (the
//! `KASSIBER_AI_PROVIDER_BROKER` override); the armed observer admits only
//! that program.

use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use kassiber_ai::broker::{default_locate, BrokerChat, BrokerClient};
use kassiber_ai::transport::{ChatDelta, RequestContext};
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_net::{BrokerProgram, Gate, Ledger, Offline};
use kassiber_pyjson::Value;

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn dumps(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Default::default()).unwrap()
}

fn chunk(chunk: &ChatDelta) -> Value {
    dict([
        ("delta", chunk.delta.clone()),
        (
            "finish_reason",
            chunk.finish_reason.as_deref().map_or(Value::Null, s),
        ),
        ("raw", chunk.raw.clone()),
        (
            "response_output",
            chunk
                .response_output
                .clone()
                .map_or(Value::Null, Value::List),
        ),
        (
            "status_phase",
            chunk.status_phase.as_deref().map_or(Value::Null, s),
        ),
    ])
}

fn error(error: &AppError) -> Value {
    dict([
        ("code", s(error.code.as_str())),
        (
            "message",
            error.message_codepoints.as_deref().map_or_else(
                || s(error.message.as_str()),
                |text| Value::Str(text.clone()),
            ),
        ),
        ("hint", error.hint.as_deref().map_or(Value::Null, s)),
        ("retryable", Value::Bool(error.retryable)),
    ])
}

fn main() {
    let mut input = String::new();
    std::io::stdin().read_to_string(&mut input).unwrap();
    let scenario = kassiber_pytext::loads(&input).unwrap();
    let offline = *field(&scenario, "offline") == Value::Bool(true);
    let mut gate = Gate::new(
        move || {
            if offline {
                Offline::Offline
            } else {
                Offline::Online
            }
        },
        Ledger::default(),
    )
    .with_test_observer(false);
    // The fake broker the scenario names is the one program admitted.
    if let Some(program) = std::env::var_os("KASSIBER_AI_PROVIDER_BROKER") {
        gate = gate.with_broker_fixture(BrokerProgram::new(Some(program.into()), Vec::new()));
    }
    let timeout = match field(&scenario, "timeout") {
        Value::Int(seconds) => Duration::from_secs(u64::try_from(seconds.clone()).unwrap()),
        _ => Duration::from_secs(120),
    };
    let locator = kassiber_envelope::py_str(field(&scenario, "locator"));
    let mut client = BrokerClient::new(&locator, timeout, gate.clone())
        .unwrap()
        .with_locate(default_locate());
    let canceller = Arc::new(client.canceller());
    let stdout = std::io::stdout();
    let Value::List(legs) = field(&scenario, "legs").clone() else {
        panic!("legs must be a list")
    };
    for leg in legs {
        let context = match field(&leg, "context") {
            Value::Null => None,
            context => Some(RequestContext {
                instructions: match field(context, "instructions") {
                    Value::Str(text) => text.as_str().map(str::to_owned),
                    _ => None,
                },
                input_items: match field(context, "input_items") {
                    Value::List(items) => items.clone(),
                    _ => Vec::new(),
                },
            }),
        };
        let messages = match field(&leg, "messages") {
            Value::List(items) => Some(items.clone()),
            _ => None,
        };
        let tools = match field(&leg, "tools") {
            Value::List(items) => items.clone(),
            _ => Vec::new(),
        };
        let model = kassiber_envelope::py_str(field(&leg, "model"));
        let cancel_after = match field(&leg, "cancel_after") {
            Value::Int(count) => usize::try_from(count.clone()).ok(),
            _ => None,
        };
        let mut seen = 0;
        let result = client.stream_chat(
            &BrokerChat {
                messages: messages.as_deref(),
                context: context.as_ref(),
                model: &model,
                options: field(&leg, "options"),
                tools: &tools,
            },
            &mut |delta| {
                let mut out = stdout.lock();
                writeln!(out, "{}", dumps(&chunk(&delta))).unwrap();
                seen += 1;
                if cancel_after == Some(seen) {
                    canceller.cancel();
                }
                Ok(())
            },
        );
        let mut pending: Vec<Value> = client
            .pending_call_ids()
            .into_iter()
            .map(Value::Str)
            .collect();
        pending.sort_by_key(dumps);
        let end = dict([(
            "end",
            dict([
                ("error", result.err().as_ref().map_or(Value::Null, error)),
                (
                    "last_provider_session_id",
                    client.last_provider_session_id.clone(),
                ),
                ("pending", Value::List(pending)),
                ("alive", Value::Bool(client.has_process())),
            ]),
        )]);
        writeln!(stdout.lock(), "{}", dumps(&end)).unwrap();
    }
    let launches = gate.attempts().records.len();
    writeln!(
        stdout.lock(),
        "{}",
        dumps(&dict([("launches", Value::from(launches))]))
    )
    .unwrap();
}
