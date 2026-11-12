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
struct PreviewSources(std::cell::Cell<usize>);
impl kassiber_setup::Sources for PreviewSources {
    fn now(&self, conn: &Connection) -> Result<String, kassiber_envelope::AppError> {
        Fixed.now(conn)
    }
    fn uuid(&self, _: &Connection) -> Result<String, kassiber_envelope::AppError> {
        let next = self.0.get() + 1;
        self.0.set(next);
        Ok(format!("00000000-0000-4000-8000-{next:012}"))
    }
}
fn main() {
    let mut stdout = io::stdout().lock();
    for line in io::stdin().lock().lines() {
        let request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let result = {
            let conn = Connection::open(Path::new(request["database"].as_str().unwrap())).unwrap();
            if let Some(passphrase) = request["passphrase"].as_str() {
                conn.pragma_update(None, "key", passphrase).unwrap();
                conn.execute_batch("PRAGMA cipher_log_level=NONE;PRAGMA cipher_compatibility=4;PRAGMA kdf_iter=256000;PRAGMA cipher_page_size=4096;").unwrap();
            }
            let args = kassiber_pytext::loads(request["args_json"].as_str().unwrap()).unwrap();
            let command = request["command"]
                .as_str()
                .unwrap_or("reports.export-transactions-xlsx");
            if matches!(
                command,
                "reports.cross-check"
                    | "reports.source-funds"
                    | "ui.reports.cross_check"
                    | "reports.exit-tax"
                    | "reports.explain-capital-gain"
                    | "ui.reports.explain_capital_gain"
                    | "ui.reports.capital_gains"
            ) || command.starts_with("source-funds.")
                || command.starts_with("ui.source_funds.")
            {
                if command == "source-funds.suggest" {
                    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
                    kassiber_reports::execute_with_sources(
                        &conn,
                        command,
                        &args,
                        &PreviewSources(std::cell::Cell::new(0)),
                    )
                } else if command == "reports.cross-check" {
                    kassiber_reports::export_request::execute(
                        &conn,
                        command,
                        &args,
                        Path::new("."),
                        &Fixed,
                    )
                } else {
                    kassiber_reports::execute_with_sources(&conn, command, &args, &Fixed)
                }
            } else {
                kassiber_reports::export::export(
                    &conn,
                    command,
                    &args,
                    Path::new(request["path"].as_str().unwrap()),
                    &Fixed,
                )
            }
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
