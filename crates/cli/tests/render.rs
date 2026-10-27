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
