//! `_OutputChannel`: whole JSON lines on stdout, one writer at a time.

use std::io::Write;
use std::sync::{Arc, Mutex};

use kassiber_envelope::{build_event_envelope, daemon_line};
use kassiber_pyjson::{Key, Value};

/// Where a record's `request_id` comes from.
#[derive(Clone, Debug, PartialEq)]
pub enum RequestId {
    /// The request had no `request_id`: the record carries none.
    Missing,
    /// Echo this value, whatever JSON it is (`null` included).
    Value(Value),
}

impl RequestId {
    /// `request.get("request_id", MISSING)`.
    pub fn of(request: &Value) -> RequestId {
        match kassiber_envelope::get(request, "request_id") {
            Some(value) => RequestId::Value(value.clone()),
            None => RequestId::Missing,
        }
    }

    /// `request.get("request_id")`: `None` when absent.
    pub fn or_null(request: &Value) -> RequestId {
        RequestId::Value(
            kassiber_envelope::get(request, "request_id")
                .cloned()
                .unwrap_or(Value::Null),
        )
    }
}

/// `_with_request_id(envelope, request_id)`.
pub fn with_request_id(mut envelope: Value, request_id: &RequestId) -> Value {
    if let (Value::Dict(pairs), RequestId::Value(id)) = (&mut envelope, request_id) {
        match pairs
            .iter_mut()
            .find(|(key, _)| *key == Key::from("request_id"))
        {
            Some((_, slot)) => *slot = id.clone(),
            None => pairs.push((Key::from("request_id"), id.clone())),
        }
    }
    envelope
}

/// The stdout writer every producer (the request loop, streaming threads,
/// background events) shares.
#[derive(Clone)]
pub struct Output {
    stream: Arc<Mutex<Box<dyn Write + Send>>>,
}

/// stdout is gone; the daemon stops.
#[derive(Debug)]
pub struct OutputClosed;

pub(crate) struct Written(());

impl Output {
    pub fn new(stream: Box<dyn Write + Send>) -> Self {
        Output {
            stream: Arc::new(Mutex::new(stream)),
        }
    }

    pub(crate) fn write_for_handoff(
        &self,
        record: &Value,
    ) -> Result<Option<Written>, OutputClosed> {
        let exact = daemon_line(record).is_ok();
        self.write(record)?;
        Ok(exact.then_some(Written(())))
    }

    /// Writes one record as a compact JSON line and flushes it.
    pub fn write(&self, record: &Value) -> Result<(), OutputClosed> {
        // A record json.dumps cannot write (an int beyond 4,300 digits, or
        // nesting past 1,000) is an internal error in Python; report it the
        // same way rather than writing a partial line.
        let line = match daemon_line(record) {
            Ok(line) => line,
            Err(error) => {
                // Keep the record routable: a supervisor treats a record
                // without a request_id after daemon.ready as fatal.
                let request_id = match kassiber_envelope::get(record, "request_id") {
                    Some(id) => RequestId::Value(id.clone()),
                    None => RequestId::Missing,
                };
                let fallback = with_request_id(
                    kassiber_envelope::build_error_envelope(
                        "internal_error",
                        &error.to_string(),
                        None,
                        None,
                        false,
                        None,
                    ),
                    &request_id,
                );
                daemon_line(&fallback)
                    .or_else(|_| {
                        daemon_line(&kassiber_envelope::build_error_envelope(
                            "internal_error",
                            &error.to_string(),
                            None,
                            None,
                            false,
                            None,
                        ))
                    })
                    .map_err(|_| OutputClosed)?
            }
        };
        let mut stream = self.stream.lock().map_err(|_| OutputClosed)?;
        stream
            .write_all(line.as_bytes())
            .map_err(|_| OutputClosed)?;
        stream.flush().map_err(|_| OutputClosed)
    }

    /// An unsolicited record: `event: true` and never a `request_id`.
    pub fn event(&self, kind: &str, data: Value) -> Result<(), OutputClosed> {
        self.write(&build_event_envelope(kind, data))
    }
}

#[cfg(all(test, feature = "test-kinds"))]
mod handoff_tests {
    use super::*;
    use kassiber_envelope::{dict, success};

    struct Sink {
        bytes: Arc<Mutex<Vec<u8>>>,
        fail_write: bool,
        fail_flush: bool,
    }

    impl Write for Sink {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            if self.fail_write {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            self.bytes.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            if self.fail_flush {
                return Err(std::io::ErrorKind::BrokenPipe.into());
            }
            Ok(())
        }
    }

    #[test]
    fn handoff_requires_a_complete_flushed_terminal_record() {
        let record = success("finished", dict::<&str>([]));
        for (fail_write, fail_flush) in [(true, false), (false, true), (false, false)] {
            let bytes = Arc::new(Mutex::new(Vec::new()));
            let output = Output::new(Box::new(Sink {
                bytes: bytes.clone(),
                fail_write,
                fail_flush,
            }));
            let written = output.write_for_handoff(&record);
            if fail_write || fail_flush {
                assert!(written.is_err());
            } else {
                assert!(written.unwrap().is_some());
                assert_eq!(
                    *bytes.lock().unwrap(),
                    daemon_line(&record).unwrap().as_bytes()
                );
            }
        }
    }

    #[test]
    fn fallback_error_records_never_grant_a_handoff_receipt() {
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = Output::new(Box::new(Sink {
            bytes: bytes.clone(),
            fail_write: false,
            fail_flush: false,
        }));
        let record = success("finished", Value::Int("9".repeat(4301).parse().unwrap()));
        assert!(output.write_for_handoff(&record).unwrap().is_none());
        let line = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        let error = kassiber_pytext::loads(&line).unwrap();
        assert_eq!(
            kassiber_envelope::get(&error, "kind"),
            Some(&Value::from("error"))
        );
    }
}
