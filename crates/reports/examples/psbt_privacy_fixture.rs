use kassiber_envelope::{dict, s};
use kassiber_pyjson::Value;
use kassiber_reports::psbt_privacy::{report_inventory, report_with_baseline, table_rows};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::psbt_privacy::{fixture_baseline, fixture_no_egress};
use std::io::{BufRead, Write};

fn main() {
    fixture_no_egress(|| {
        let mut stdout = std::io::stdout().lock();
        for line in std::io::stdin().lock().lines() {
            let case: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
            let conn = Connection::open(case["database"].as_str().unwrap()).unwrap();
            let args = kassiber_pytext::loads(&case["args"].to_string()).unwrap();
            let result = if case["native_history"] == true {
                kassiber_reports::psbt_privacy::report(&conn, &args, case["psbt"].as_str().unwrap())
            } else if case["native_inventory"] == true {
                report_inventory(&conn, &args, case["psbt"].as_str().unwrap())
            } else {
                report_with_baseline(
                    &conn,
                    &args,
                    case["psbt"].as_str().unwrap(),
                    |_, profile| {
                        assert_eq!(profile, case["baseline_profile"].as_str().unwrap());
                        Ok(fixture_baseline(&case["baseline"]))
                    },
                )
            };
            let response = match result {
                Ok(payload) => dict([("ok", payload.clone()), ("rows", table_rows(&payload))]),
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
