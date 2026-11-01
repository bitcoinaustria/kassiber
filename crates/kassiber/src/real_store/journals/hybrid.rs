//! Development candidate only. This module is absent from production builds.
use super::Refresh;
use kassiber_envelope::{dict, get, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::freshness::execution::{Failure, Progress};
use std::{
    io::{BufRead, BufReader, Read, Write},
    process::{Child, Command, Stdio},
    sync::mpsc,
    time::Duration,
};

pub(super) fn enabled() -> bool {
    std::env::var_os("KASSIBER_RUST_TEST_JOURNAL_PYTHON").is_some()
        && std::env::var_os("KASSIBER_RUST_TEST_JOURNAL_REFERENCE").is_some()
}

pub(super) fn replayable(error: &Failure) -> bool {
    !matches!(error, Failure::App(error) if matches!(error.code.as_str(),
        "cancelled" | "stale_context" | "interaction_required" | "backend_timeout"))
}

fn log(value: serde_json::Value) {
    if let Some(path) = std::env::var_os("KASSIBER_HYBRID_LOG") {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(file, "{value}");
        }
    }
}

pub(super) fn record(error: &kassiber_journals::Refusal, phase: &str, replay_safe: bool) {
    log(serde_json::json!({
        "command": "freshness.journal_refresh", "event": "refusal",
        "refusal_shape": format!("{phase}:{}", error.family), "replay_safe": replay_safe,
    }));
}

struct Reap(Child);
impl Drop for Reap {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn failed() -> Failure {
    AppError::new(
        "internal_error",
        "Development journal replay did not finish",
    )
    .into()
}

fn convert(value: &serde_json::Value) -> Result<Value, Failure> {
    kassiber_pytext::loads(&value.to_string()).map_err(|_| failed())
}

pub(super) fn replay(
    handler: &Refresh<'_>,
    conn: &Connection,
    job: &Value,
    progress: &mut Progress<'_>,
    paired_phase: bool,
) -> Result<Value, Failure> {
    handler.check(conn)?;
    progress.check(conn)?;
    let path = conn.path().ok_or_else(failed)?;
    let root = std::path::Path::new(path).parent().ok_or_else(failed)?;
    let payload = get(job, "payload").unwrap_or(&Value::Null);
    let mut request = serde_json::json!({
        "root": root, "passphrase": handler.test_passphrase,
        "profile_id": py_str(get(job, "profile_id").unwrap_or(&Value::Null)),
        "auto_pair": get(payload, "auto_pair").is_some_and(truthy),
        "skip_rebuild_when_current": get(payload, "skip_rebuild_when_current").is_some_and(truthy),
    });
    let mut child = Reap(
        Command::new(std::env::var_os("KASSIBER_RUST_TEST_JOURNAL_PYTHON").ok_or_else(failed)?)
            .arg(std::env::var_os("KASSIBER_RUST_TEST_JOURNAL_REFERENCE").ok_or_else(failed)?)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| failed())?,
    );
    let mut input = child.0.stdin.take().ok_or_else(failed)?;
    let written = serde_json::to_writer(&mut input, &request);
    if let Some(serde_json::Value::String(secret)) = request.get_mut("passphrase") {
        use zeroize::Zeroize;
        secret.zeroize();
    }
    written.map_err(|_| failed())?;
    writeln!(input).map_err(|_| failed())?;
    input.flush().map_err(|_| failed())?;
    let output = child.0.stdout.take().ok_or_else(failed)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut output = BufReader::new(output);
        loop {
            let mut line = String::new();
            if output
                .by_ref()
                .take(8 * 1024 * 1024 + 1)
                .read_line(&mut line)
                .is_err()
                || line.is_empty()
                || line.len() > 8 * 1024 * 1024
            {
                break;
            }
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let outcome = (|| {
        log(serde_json::json!({"command": "freshness.journal_refresh", "served_by": "python"}));
        loop {
            handler.check(conn)?;
            progress.check(conn)?;
            let line = match receiver.recv_timeout(Duration::from_millis(25)) {
                Ok(line) => line,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(_) => return Err(failed()),
            };
            let value: serde_json::Value = serde_json::from_str(&line).map_err(|_| failed())?;
            if value.get("phase").and_then(serde_json::Value::as_str) == Some("auto_pair") {
                if !paired_phase {
                    progress.emit(conn, &dict([("phase", s("auto_pair"))]))?;
                }
                handler.check(conn)?;
                progress.check(conn)?;
                writeln!(input, "{{\"continue\":true}}").map_err(|_| failed())?;
                input.flush().map_err(|_| failed())?;
            } else if let Some(result) = value.get("result") {
                return convert(result);
            } else if let Some(error) = value.get("error") {
                return Err(Failure::App(AppError {
                    code: error["code"].as_str().ok_or_else(failed)?.into(),
                    message: error["message"].as_str().ok_or_else(failed)?.into(),
                    hint: error["hint"].as_str().map(Into::into),
                    message_codepoints: None,
                    sqlite_origin: false,
                    details: (!error["details"].is_null())
                        .then(|| convert(&error["details"]))
                        .transpose()
                        .map_err(|_| failed())?,
                    retryable: error["retryable"].as_bool().unwrap_or(false),
                }));
            } else if let Some(error) = value.get("exception") {
                return Err(Failure::Unexpected {
                    message: error["message"].as_str().ok_or_else(failed)?.into(),
                    class: error["class"].as_str().ok_or_else(failed)?.into(),
                });
            } else {
                return Err(failed());
            }
        }
    })();
    drop(input);
    drop(child);
    drop(receiver);
    let _ = reader.join();
    outcome
}
