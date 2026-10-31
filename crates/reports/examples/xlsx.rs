//! Disposable-book service probe for the XLSX differential tests.
use kassiber_envelope::{dict, s};
use kassiber_store::rusqlite::Connection;
use std::io::{self, BufRead, Write};
use std::path::Path;

struct Fixed;
impl kassiber_setup::Sources for Fixed {
    fn now(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("2026-01-02T03:04:05Z".into())
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Ok("00000000-0000-4000-8000-000000000001".into())
    }
}
fn main() {
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let result = {
            let conn = Connection::open(Path::new(request["database"].as_str().unwrap())).unwrap();
            let args = kassiber_pytext::loads(request["args_json"].as_str().unwrap()).unwrap();
            kassiber_reports::export::export(
                &conn,
                request["command"]
                    .as_str()
                    .unwrap_or("reports.export-transactions-xlsx"),
                &args,
                Path::new(request["path"].as_str().unwrap()),
                &Fixed,
            )
        };
        let response = match result {
            Ok(value) => dict([("ok", value)]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    (
                        "hint",
                        e.hint.map(s).unwrap_or(kassiber_pyjson::Value::Null),
                    ),
                    ("details", e.details.unwrap_or(kassiber_pyjson::Value::Null)),
                    ("retryable", kassiber_pyjson::Value::Bool(e.retryable)),
                ]),
            )]),
        };
        writeln!(
            stdout,
            "{}",
            kassiber_pyjson::dumps(&response, &Default::default()).unwrap()
        )
        .unwrap();
        stdout.flush().unwrap();
    }
}
