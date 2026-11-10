use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_reports::legacy_privacy::{fixture_probes, snapshot};
use kassiber_store::rusqlite::Connection;
use std::io::{BufRead, Write};

fn main() {
    kassiber_wallets::psbt_privacy::fixture_no_egress(|| {
        let mut stdout = std::io::stdout().lock();
        for line in std::io::stdin().lock().lines() {
            let case: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
            let conn = Connection::open(case["database"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(case["args_json"].as_str().unwrap()).unwrap();
            let response = match snapshot(&conn, &args) {
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
            let probes = fixture_probes(&conn).unwrap();
            writeln!(
                stdout,
                "{}",
                kassiber_pyjson::dumps(
                    &dict([
                        ("result", response),
                        (
                            "probes",
                            kassiber_pytext::loads(&probes.to_string()).unwrap()
                        )
                    ]),
                    &kassiber_pyjson::Options {
                        sort_keys: true,
                        separators: Some((",".into(), ":".into())),
                        ..Default::default()
                    }
                )
                .unwrap()
            )
            .unwrap();
            stdout.flush().unwrap();
        }
    });
}
