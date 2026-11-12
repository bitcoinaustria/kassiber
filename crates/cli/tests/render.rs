//! `emit` in every format against Python (`tests/data/render_cases.txt`,
//! from `scripts/gen_render_cases.py`).

use kassiber_cli::render::{emit, Format, Sink};
use kassiber_envelope::get;
use kassiber_pyjson::Value;

fn text(value: &Value) -> String {
    match value {
        Value::Str(s) => s.as_str().unwrap().to_owned(),
        other => panic!("{other:?}"),
    }
}

#[test]
fn emit_matches_python() {
    let mut failures = Vec::new();
    for (number, line) in include_str!("data/render_cases.txt").lines().enumerate() {
        let case = kassiber_pytext::loads(line).unwrap();
        let payload = get(&case, "payload").unwrap().clone();
        let format = Format::parse(&text(get(&case, "format").unwrap())).unwrap();
        let expected = get(&case, "out").unwrap();
        let mut out = Vec::new();
        let mut err = Vec::new();
        let mut sink = Sink {
            output: None,
            home: None,
            stdout: &mut out,
            stderr: &mut err,
        };
        let result = emit(&mut sink, format, "status", payload);
        let matched = match (&result, get(expected, "ok"), get(expected, "error")) {
            (Ok(()), Some(ok), _) => String::from_utf8(out.clone()).unwrap() == text(ok),
            (Err(error), _, Some(code)) => {
                error.code == text(code) && error.message == text(get(expected, "message").unwrap())
            }
            _ => false,
        };
        if !matched {
            failures.push(format!(
                "line {}: {format:?}\n got: {result:?} {:?}\nwant: {expected:?}",
                number + 1,
                String::from_utf8_lossy(&out)
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn report_csv_format_contract() {
    // Python envelope._write_csv_rows: first-record headers, JSON cells,
    // eight-place HALF_EVEN float display, QUOTE_MINIMAL and CRLF.
    let payload = kassiber_pytext::loads(
        r#"[{"amount":1.234567895,"missing":null,"unicode":"€","quoted":"a,\"b\"\r\nc","nested":{"z":1,"a":true},"negative_zero":-0.0,"tiny":1e-11},{"amount":2.5,"unicode":"line\nfeed","ignored":"extra"}]"#,
    ).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut sink = Sink {
        output: None,
        home: None,
        stdout: &mut out,
        stderr: &mut err,
    };
    emit(&mut sink, Format::Csv, "reports.journal_entries", payload).unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        concat!(
            "amount,missing,unicode,quoted,nested,negative_zero,tiny\r\n",
            "1.2345679,,€,\"a,\"\"b\"\"\r\nc\",\"{\"\"z\"\": 1, \"\"a\"\": true}\",-0,0\r\n",
            "2.5,,\"line\nfeed\",,,,\r\n"
        )
    );
    for (raw, expected) in [
        (r#"[{"":""}]"#, "\"\"\r\n\"\"\r\n"),
        (r#"[{"x":true},{"x":false}]"#, "x\r\nTrue\r\nFalse\r\n"),
        (
            r#"[{"x":"a,b"},{"x":"a\"b"},{"x":"a\rb"},{"x":"a\nb"}]"#,
            "x\r\n\"a,b\"\r\n\"a\"\"b\"\r\n\"a\rb\"\r\n\"a\nb\"\r\n",
        ),
        (
            r#"[{"x":{"z":"☃","a":[1,2]}}]"#,
            "x\r\n\"{\"\"z\"\": \"\"\\u2603\"\", \"\"a\"\": [1, 2]}\"\r\n",
        ),
        (
            r#"[{"x":NaN},{"x":Infinity},{"x":-Infinity}]"#,
            "x\r\nnan\r\ninf\r\n-inf\r\n",
        ),
        (
            r#"[{"x":0.001953125},{"x":0.005859375},{"x":-0.001953125}]"#,
            "x\r\n0.00195312\r\n0.00585938\r\n-0.00195312\r\n",
        ),
    ] {
        let mut out = Vec::new();
        let mut sink = Sink {
            output: None,
            home: None,
            stdout: &mut out,
            stderr: &mut err,
        };
        emit(
            &mut sink,
            Format::Csv,
            "reports.summary",
            kassiber_pytext::loads(raw).unwrap(),
        )
        .unwrap();
        assert_eq!(out, expected.as_bytes());
    }
    let mut out = Vec::new();
    let mut sink = Sink {
        output: None,
        home: None,
        stdout: &mut out,
        stderr: &mut err,
    };
    let error = emit(
        &mut sink,
        Format::Csv,
        "reports.tax_summary",
        Value::List(vec![]),
    )
    .unwrap_err();
    assert_eq!(error.code, "format_unsupported");
    assert!(out.is_empty());
}

#[test]
fn report_csv_file_publication_replaces_complete_artifacts() {
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("report.csv");
    let old = root.path().join("prior.csv");
    std::fs::write(&target, b"prior artifact\r\n").unwrap();
    std::fs::hard_link(&target, &old).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut sink = Sink {
        output: Some(target.to_str().unwrap()),
        home: None,
        stdout: &mut out,
        stderr: &mut err,
    };
    emit(
        &mut sink,
        Format::Csv,
        "reports.tax_summary",
        kassiber_pytext::loads(r#"[{"x":"€\nquoted,cell"}]"#).unwrap(),
    )
    .unwrap();
    assert_eq!(
        std::fs::read(&target).unwrap(),
        "x\r\n\"€\nquoted,cell\"\r\n".as_bytes()
    );
    assert_eq!(std::fs::read(&old).unwrap(), b"prior artifact\r\n");
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 2);
    assert!(out.is_empty());
    assert!(err.is_empty());
}

#[cfg(unix)]
#[test]
fn report_csv_publication_follows_explicit_output_symlinks() {
    use std::os::unix::fs::symlink;
    let root = tempfile::tempdir().unwrap();
    let target = root.path().join("report.csv");
    let link = root.path().join("link.csv");
    symlink("report.csv", &link).unwrap();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let mut sink = Sink {
        output: Some(link.to_str().unwrap()),
        home: None,
        stdout: &mut out,
        stderr: &mut err,
    };
    for _ in 0..2 {
        emit(
            &mut sink,
            Format::Csv,
            "reports.capital_gains",
            kassiber_pytext::loads(r#"[{"x":1}]"#).unwrap(),
        )
        .unwrap();
        assert!(std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(std::fs::read(&target).unwrap(), b"x\r\n1\r\n");
    }
    std::fs::remove_file(&link).unwrap();
    symlink("absent/parent/report.csv", &link).unwrap();
    assert!(emit(
        &mut sink,
        Format::Csv,
        "reports.capital_gains",
        kassiber_pytext::loads(r#"[{"x":1}]"#).unwrap()
    )
    .is_err());
    assert!(!root.path().join("absent").exists());
    std::fs::remove_file(&link).unwrap();
    symlink("link.csv", &link).unwrap();
    assert!(emit(
        &mut sink,
        Format::Csv,
        "reports.capital_gains",
        kassiber_pytext::loads(r#"[{"x":1}]"#).unwrap()
    )
    .is_err());
}
