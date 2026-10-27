//! Kinds that exist only in test builds (feature `test-kinds`), so the
//! protocol machinery R1 ships without a business kind that uses it can be
//! driven end to end: streaming records from another thread, unsolicited
//! events, a record for a request id nobody sent, and a secret-store
//! control round trip. They are not classified capabilities and never
//! appear in a release binary.

use std::time::Duration;

use kassiber_envelope::{dict, get, success, AppError};
use kassiber_pyjson::Value;

use crate::control::{secret_store_request, SecretStoreRequest, SECRET_STORE_BRIDGE_TIMEOUT};
use crate::input::Input;
use crate::output::{with_request_id, RequestId};
use crate::{Daemon, Failure, Handled};

pub const KINDS: [&str; 4] = [
    "test.stream",
    "test.emit_event",
    "test.late",
    "test.secret_store",
];

fn arg_text(args: Option<&Value>, key: &str) -> String {
    match args.and_then(|args| get(args, key)) {
        Some(Value::Str(text)) => text.as_str().unwrap_or_default().to_owned(),
        _ => String::new(),
    }
}

fn arg_number(args: Option<&Value>, key: &str, default: u64) -> u64 {
    match args.and_then(|args| get(args, key)) {
        Some(Value::Int(n)) => u64::try_from(n.clone()).unwrap_or(default),
        _ => default,
    }
}

pub(crate) fn handle(
    daemon: &mut Daemon,
    kind: &kassiber_pyjson::PyStr,
    request: &Value,
    request_id: &RequestId,
    input: &mut Input,
) -> Option<Handled> {
    let kind = kind.as_str()?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let args = get(request, "args");
    Some(match kind {
        // `count` deltas `delay_ms` apart from a worker thread, then the
        // terminal record; the request loop keeps serving meanwhile.
        "test.stream" => {
            let count = arg_number(args, "count", 2);
            let delay = Duration::from_millis(arg_number(args, "delay_ms", 50));
            let output = daemon.output.clone();
            let request_id = request_id.clone();
            std::thread::spawn(move || {
                for index in 0..count {
                    std::thread::sleep(delay);
                    let delta = success("test.stream.delta", dict([("index", Value::from(index))]));
                    if output.write(&with_request_id(delta, &request_id)).is_err() {
                        return;
                    }
                }
                let done = success("test.stream", dict([("count", Value::from(count))]));
                let _ = output.write(&with_request_id(done, &request_id));
            });
            Ok((None, false))
        }
        "test.emit_event" => (|| {
            daemon
                .output
                .event("test.event", dict([("n", Value::from(1))]))?;
            Ok((
                Some(with_request_id(
                    success("test.emit_event", dict([("ok", Value::Bool(true))])),
                    request_id,
                )),
                false,
            ))
        })(),
        // A straggler for a request the supervisor never sent, then the
        // real answer.
        "test.late" => (|| {
            let ghost = success("test.late.delta", dict([("late", Value::Bool(true))]));
            daemon.output.write(&with_request_id(
                ghost,
                &RequestId::Value(kassiber_envelope::s("ghost-1")),
            ))?;
            Ok((
                Some(with_request_id(
                    success("test.late", dict([("ok", Value::Bool(true))])),
                    request_id,
                )),
                false,
            ))
        })(),
        "test.secret_store" => {
            let op = arg_text(args, "op");
            let provider_name = arg_text(args, "provider_name");
            let store_id = arg_text(args, "store_id");
            let service = arg_text(args, "service");
            let account = arg_text(args, "account");
            let timeout = Duration::from_millis(arg_number(
                args,
                "timeout_ms",
                u64::try_from(SECRET_STORE_BRIDGE_TIMEOUT.as_millis()).unwrap_or(15_000),
            ));
            let bridge = SecretStoreRequest {
                op: if op.is_empty() { "get" } else { &op },
                provider_name: &provider_name,
                store_id: &store_id,
                service: &service,
                account: &account,
                secret: None,
            };
            match secret_store_request(&daemon.output, input, &bridge, timeout) {
                Ok(data) => Ok((
                    Some(with_request_id(
                        success("test.secret_store", data),
                        request_id,
                    )),
                    false,
                )),
                Err(error) => Err(Failure::from(error)),
            }
        }
        _ => Err(Failure::App(AppError::new(
            "unsupported_kind",
            "unknown test kind",
        ))),
    })
}
