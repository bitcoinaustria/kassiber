//! Guarded adapter acceptance without registering a product route.
use kassiber_envelope::{dict, s};
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::{Connection, OpenFlags};
use std::io::{BufRead, Write};

#[path = "psbt_transport_fixture/book.rs"]
mod book;
use book::ReportBook;

fn main() {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let mut stdout = std::io::stdout().lock();
    for line in std::io::stdin().lock().lines() {
        let case = kassiber_pytext::loads(&line.unwrap()).unwrap();
        let field = |key| kassiber_envelope::get(&case, key).unwrap_or(&Value::Null);
        let conn = Connection::open_with_flags(
            kassiber_envelope::py_str(field("database")),
            OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let mut book = ReportBook(conn);
        let result = if field("transport") == &s("cli") {
            let Value::List(values) = field("argv") else {
                panic!("fixture argv must be a list")
            };
            let argv = values
                .iter()
                .map(kassiber_envelope::py_str)
                .collect::<Vec<_>>();
            let args = kassiber_cli::argparse::parse_args(
                &argv,
                kassiber_cli::argparse::Context {
                    width: 78,
                    version: "fixture",
                },
            )
            .unwrap();
            kassiber_cli::fixture_reports(&mut book, "reports.psbt-privacy", &args)
        } else {
            kassiber_daemon::fixture_reports(
                &mut book,
                "ui.reports.psbt_privacy",
                kassiber_envelope::get(&case, "args"),
            )
        };
        let response = match result {
            Ok(payload) => dict([("ok", payload)]),
            Err(e) => dict([(
                "error",
                dict([
                    ("code", s(e.code)),
                    ("message", s(e.message)),
                    ("hint", e.hint.map(s).unwrap_or(Value::Null)),
                    ("details", e.details.unwrap_or(Value::Null)),
                    ("retryable", Value::Bool(e.retryable)),
                ]),
            )]),
        };
        writeln!(
            stdout,
            "{}",
            kassiber_pyjson::dumps(
                &response,
                &Options::new()
                    .sort_keys(field("wire_order") != &Value::Bool(true))
                    .compact()
            )
            .unwrap()
        )
        .unwrap();
        assert!(gate.attempts().records.is_empty());
    }
}
