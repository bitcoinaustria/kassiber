//! The reverse control channel: the daemon asks the desktop's native secret
//! store and waits for its reply (`_secret_store_bridge_request`).
//!
//! The request goes out as `supervisor.ai_secret_store.request` with a
//! `secret-store-<n>` request id; the supervisor answers on stdin with
//! `supervisor.ai_secret_store.response` and the same id. Lines that arrive
//! meanwhile are set aside, in order, for the request loop. The wait ends
//! after 15 seconds or at end of input.
//!
//! Two differences from Python, both where Python hangs or spins: while
//! waiting, only newly read lines are examined (Python re-reads lines it
//! set aside itself, so a second wait in one request spins until its
//! deadline), and end of input stays visible to the request loop afterwards
//! (Python's wait consumes it and the loop then polls forever).

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use kassiber_envelope::{dict, get, py_str, s, success, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_pytext::{loads, strip};

use crate::input::{Chunk, Input};
use crate::output::{with_request_id, Output, OutputClosed, RequestId};

pub const SECRET_STORE_CONTROL_REQUEST_KIND: &str = "supervisor.ai_secret_store.request";
pub const SECRET_STORE_CONTROL_RESPONSE_KIND: &str = "supervisor.ai_secret_store.response";
/// `_SECRET_STORE_BRIDGE_TIMEOUT_SECONDS`.
pub const SECRET_STORE_BRIDGE_TIMEOUT: Duration = Duration::from_secs(15);

/// A secret-store operation the daemon asks the desktop to perform.
#[derive(Debug)]
pub struct SecretStoreRequest<'a> {
    pub op: &'a str,
    pub provider_name: &'a str,
    pub store_id: &'a str,
    pub service: &'a str,
    pub account: &'a str,
    pub secret: Option<&'a str>,
}

/// A control id like Python's `secret-store-{time.monotonic_ns()}`: from the
/// system clock in nanoseconds, so ids differ across daemon restarts, and
/// strictly increasing within the process.
fn control_id() -> String {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_nanos()).unwrap_or(u64::MAX)
        });
    let mut previous = LAST.load(Ordering::SeqCst);
    loop {
        let next = now.max(previous + 1);
        match LAST.compare_exchange(previous, next, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return format!("secret-store-{next}"),
            Err(actual) => previous = actual,
        }
    }
}

fn unavailable_refs(request: &SecretStoreRequest<'_>) -> Value {
    dict([(
        "refs",
        Value::List(vec![dict([
            ("provider_name", s(request.provider_name)),
            ("store_id", s(request.store_id)),
            ("state", s("unavailable")),
        ])]),
    )])
}

/// Why the bridge failed: the typed error Python raises, or stdout closed.
#[derive(Debug)]
pub enum BridgeError {
    App(AppError),
    Closed(OutputClosed),
}

impl From<AppError> for BridgeError {
    fn from(error: AppError) -> Self {
        BridgeError::App(error)
    }
}

/// Sends `request` to the desktop and returns its `data` object.
pub fn secret_store_request(
    output: &Output,
    input: &mut Input,
    request: &SecretStoreRequest<'_>,
    timeout: Duration,
) -> Result<Value, BridgeError> {
    let id = control_id();
    let mut payload = vec![
        ("op", s(request.op)),
        ("provider_name", s(request.provider_name)),
        ("store_id", s(request.store_id)),
        ("service", s(request.service)),
        ("account", s(request.account)),
    ];
    if let Some(secret) = request.secret {
        payload.push(("secret", s(secret)));
    }
    let record = with_request_id(
        success(SECRET_STORE_CONTROL_REQUEST_KIND, dict(payload)),
        &RequestId::Value(s(id.as_str())),
    );
    output.write(&record).map_err(BridgeError::Closed)?;

    let deadline = Instant::now() + timeout;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(AppError::new(
                "secret_ref_unavailable",
                "desktop secret store did not answer",
            )
            .with_hint(
                "Try again after restarting the desktop app or re-enter the provider API key.",
            )
            .with_details(unavailable_refs(request))
            .retryable(true)
            .into());
        }
        let Some(chunk) = input.next_new(Some(remaining)) else {
            continue;
        };
        let line = match &chunk {
            Chunk::Eof => {
                return Err(AppError::new(
                    "secret_ref_unavailable",
                    "desktop secret store bridge closed",
                )
                .with_details(unavailable_refs(request))
                .retryable(true)
                .into());
            }
            Chunk::Undecodable => {
                input.defer(chunk);
                continue;
            }
            Chunk::Text(line) => line.clone(),
        };
        let raw = strip(&line);
        if raw.is_empty() {
            continue;
        }
        let Ok(response) = loads(raw) else {
            input.defer(chunk);
            continue;
        };
        let is_reply = matches!(get(&response, "kind"), Some(Value::Str(k)) if k.as_str() == Some(SECRET_STORE_CONTROL_RESPONSE_KIND))
            && matches!(get(&response, "request_id"), Some(Value::Str(r)) if r.as_str() == Some(id.as_str()));
        if !is_reply {
            input.defer(chunk);
            continue;
        }
        if let Some(Value::Dict(_)) = get(&response, "error") {
            let error = get(&response, "error").cloned().unwrap_or(Value::Null);
            let message = get(&error, "message").filter(|m| truthy(m)).map_or_else(
                || "desktop secret store operation failed".to_owned(),
                py_str,
            );
            let code = get(&error, "code")
                .filter(|c| truthy(c))
                .map_or_else(|| "secret_ref_unavailable".to_owned(), py_str);
            let retryable = get(&error, "retryable").is_none_or(truthy);
            return Err(AppError::new(code, message)
                .with_details(unavailable_refs(request))
                .retryable(retryable)
                .into());
        }
        return match get(&response, "data") {
            Some(data @ Value::Dict(_)) => Ok(data.clone()),
            _ => Err(AppError::new(
                "secret_ref_unavailable",
                "desktop secret store response was malformed",
            )
            .retryable(true)
            .into()),
        };
    }
}
