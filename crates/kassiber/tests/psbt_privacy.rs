#![cfg(feature = "test-harness")]

#[path = "../examples/psbt_transport_fixture/book.rs"]
mod book;
use book::ReportBook;
use kassiber_envelope::{dict, s};
use kassiber_pyjson::{Options, PyStr, Value};
use kassiber_store::rusqlite::Connection;
use kassiber_wallets::analysis_index::fixture::tables;

fn canonical(value: &Value) -> String {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true).compact()).unwrap()
}
fn cli(book: &mut ReportBook, argv: &[String]) -> Result<Value, kassiber_envelope::AppError> {
    let args = kassiber_cli::argparse::parse_args(
        argv,
        kassiber_cli::argparse::Context {
            width: 78,
            version: "fixture",
        },
    )
    .unwrap();
    kassiber_cli::fixture_reports(book, "reports.psbt-privacy", &args)
}

#[test]
fn psbt_cli_file_and_daemon_adapters_match_python_bytes_and_all_112_tables() {
    let gate = kassiber_net::Gate::new(
        || kassiber_net::Offline::Unavailable,
        kassiber_net::Ledger::default(),
    );
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(include_str!("../../reports/tests/data/psbt_history.json")).unwrap();
    for name in [
        "inventory/unlinked-inputs",
        "spend/2/None/confirmed/both",
        "spend/2/coinjoin/confirmed/both",
    ] {
        let case = cases.iter().find(|c| c["name"] == name).unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(include_str!(
            "../../reports/tests/data/privacy_hygiene_seed.sql"
        ))
        .unwrap();
        conn.execute_batch(include_str!("../../reports/tests/data/book.sql"))
            .unwrap();
        conn.execute_batch(case["sql"].as_str().unwrap()).unwrap();
        let expected = kassiber_pytext::loads(&case["expected"]["ok"].to_string()).unwrap();
        let psbt = case["psbt"].as_str().unwrap();
        let before = tables(&conn);
        let changes = conn.total_changes();
        let mut book = ReportBook(conn);
        let scratch = tempfile::tempdir().unwrap();
        let file = scratch.path().join("proposal.psbt");
        std::fs::write(&file, format!("\x1c\r\n{psbt}\r\n ")).unwrap();
        let wrapped = scratch.path().join("wrapped.psbt");
        std::fs::write(
            &wrapped,
            format!(
                "{}{}{}",
                &psbt[..10],
                "\r\n".repeat(3 * 1024 * 1024),
                &psbt[10..]
            ),
        )
        .unwrap();
        for format in ["json", "table", "plain", "csv"] {
            for source in [
                vec!["--psbt".into(), psbt.into()],
                vec!["--psbt-file".into(), file.to_string_lossy().into_owned()],
            ] {
                let mut argv = vec![
                    "--machine".into(),
                    "--format".into(),
                    format.into(),
                    "reports".into(),
                    "psbt-privacy".into(),
                ];
                argv.extend(source);
                let actual = cli(&mut book, &argv).unwrap();
                let expected = if format == "json" {
                    expected.clone()
                } else {
                    kassiber_reports::psbt_privacy::table_rows(&expected)
                };
                assert_eq!(canonical(&actual), canonical(&expected), "{name}: {format}");
                assert_eq!(tables(&book.0), before);
                assert_eq!(book.0.total_changes(), changes);
            }
        }
        let wrapped_report = cli(
            &mut book,
            &[
                "--machine".into(),
                "reports".into(),
                "psbt-privacy".into(),
                "--psbt-file".into(),
                wrapped.to_string_lossy().into_owned(),
            ],
        )
        .unwrap();
        assert_eq!(canonical(&wrapped_report), canonical(&expected));
        assert_eq!(tables(&book.0), before);
        for psbt in [s(psbt), s(format!(" \n{psbt}\r\n"))] {
            let actual = kassiber_daemon::fixture_reports(
                &mut book,
                "ui.reports.psbt_privacy",
                Some(&dict([("psbt", psbt)])),
            )
            .unwrap();
            assert_eq!(canonical(&actual), canonical(&expected));
            assert_eq!(tables(&book.0), before);
        }
        let unknown = kassiber_daemon::fixture_reports(
            &mut book,
            "ui.reports.psbt_privacy",
            Some(&dict([("z", Value::Null), ("a", s("x"))])),
        )
        .unwrap_err();
        assert_eq!(
            unknown.details,
            Some(dict([("unknown", Value::List(vec![s("a"), s("z")]))]))
        );
        assert_eq!(
            unknown.message,
            "ui.reports.psbt_privacy received unsupported arguments"
        );
        for raw in [Value::List(vec![]), s(""), Value::Bool(false)] {
            let error =
                kassiber_daemon::fixture_reports(&mut book, "ui.reports.psbt_privacy", Some(&raw))
                    .unwrap_err();
            assert_eq!(error.message, "daemon args must be an object");
        }
        for psbt in [Value::Null, s("\x1c \r\n"), Value::Int(42.into())] {
            let error = kassiber_daemon::fixture_reports(
                &mut book,
                "ui.reports.psbt_privacy",
                Some(&dict([("psbt", psbt)])),
            )
            .unwrap_err();
            assert_eq!(error.message, "ui.reports.psbt_privacy requires psbt text");
            assert_eq!(
                error.details,
                Some(dict([("required", Value::List(vec![s("psbt")]))]))
            );
        }
        let other = cli(
            &mut book,
            &[
                "reports".into(),
                "psbt-privacy".into(),
                "--profile".into(),
                "other".into(),
                "--psbt".into(),
                psbt.into(),
            ],
        )
        .unwrap();
        assert_eq!(
            kassiber_envelope::get(
                kassiber_envelope::get(&other, "summary").unwrap(),
                "known_owned_input_count"
            ),
            Some(&Value::Int(0.into()))
        );
        let missing = cli(
            &mut book,
            &[
                "reports".into(),
                "psbt-privacy".into(),
                "--profile".into(),
                "missing".into(),
                "--psbt".into(),
                "invalid".into(),
            ],
        )
        .unwrap_err();
        assert_eq!(missing.code, "not_found");
        let mut points = psbt.chars().map(u32::from).collect::<Vec<_>>();
        points.push(0xd800);
        let raw = Value::Str(PyStr::from_code_points(points).unwrap());
        let error = kassiber_daemon::fixture_reports(
            &mut book,
            "ui.reports.psbt_privacy",
            Some(&dict([("psbt", raw)])),
        )
        .unwrap_err();
        assert_eq!(error.message, "Could not decode the PSBT");
        assert_eq!(tables(&book.0), before);
        assert_eq!(book.0.total_changes(), changes);
    }
    assert!(gate.attempts().records.is_empty());
}

#[test]
fn psbt_cli_file_refusals_preserve_reference_precedence_and_literal_paths() {
    let mut book = ReportBook(Connection::open_in_memory().unwrap());
    let root = tempfile::tempdir().unwrap();
    for (name, bytes, message) in [
        ("empty", vec![], "PSBT file is empty"),
        ("spaces", b"\x1c \r\n\t".to_vec(), "PSBT file is empty"),
        (
            "nul",
            b"before\0after".to_vec(),
            "PSBT file contains NUL bytes",
        ),
        (
            "utf8",
            vec![0xff],
            "'utf-8' codec can't decode byte 0xff in position 0: invalid start byte",
        ),
    ] {
        let path = root.path().join(name);
        std::fs::write(&path, bytes).unwrap();
        let error = cli(
            &mut book,
            &[
                "reports".into(),
                "psbt-privacy".into(),
                "--profile".into(),
                "missing".into(),
                "--psbt-file".into(),
                path.to_string_lossy().into_owned(),
            ],
        )
        .unwrap_err();
        assert_eq!(error.message, message);
        assert_eq!(
            error.code,
            if name == "utf8" {
                "internal_error"
            } else {
                "validation"
            }
        );
    }
    let path = "~/sentinel-missing-proposal";
    let error = cli(
        &mut book,
        &[
            "reports".into(),
            "psbt-privacy".into(),
            "--psbt-file".into(),
            path.into(),
        ],
    )
    .unwrap_err();
    assert_eq!(error.message, "Could not read PSBT file");
    assert_eq!(
        error.hint.as_deref(),
        Some("Choose a readable local file path.")
    );
    assert_eq!(error.details, Some(dict([("path", s(path))])));
}

#[test]
fn psbt_product_cli_daemon_bytes_all_tables_and_zero_fallback() {
    reference("tests/test_rust_psbt_product.py::test_psbt_product_bytes_files_all_tables_and_zero_hybrid_fallback");
}

#[test]
fn psbt_product_registration_and_wire_order_match_python() {
    reference(
        "tests/test_rust_psbt_product.py::test_psbt_registered_product_routes_and_wire_order",
    );
}

#[test]
fn psbt_product_unhandled_decoder_diagnostic_matches_python() {
    reference("tests/test_rust_psbt_product.py::test_psbt_decoder_daemon_preserves_native_unhandled_debug_contract");
}

fn reference(test: &str) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let python = std::env::var_os("KASSIBER_REFERENCE_PYTHON")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| root.join(".venv/bin/python"));
    let scratch = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(python)
        .current_dir(&root)
        .args(["-m", "pytest", test, "-q", "-s", "--basetemp"])
        .arg(scratch.path())
        .env("KASSIBER_RUST_BIN", env!("CARGO_BIN_EXE_kassiber"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    println!("{}", String::from_utf8_lossy(&output.stdout));
}
