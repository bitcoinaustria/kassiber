//! Checks `tests/data/golden.txt`: envelopes and redactions Python produced
//! (`scripts/gen_golden.py`). Lines are read with kassiber-pytext's
//! `json.loads`, which keeps surrogates and ints as Python does.

use kassiber_envelope::redaction::{
    redact_error_details, redact_operational_text, redact_secret_text,
};
use kassiber_envelope::{
    build_envelope, build_error_envelope, build_event_envelope, cli_text, daemon_line, get,
};
use kassiber_pyjson::{dumps, Decimal, Key, Options, Value};
use kassiber_pytext::loads;

fn text(value: &Value) -> String {
    match value {
        Value::Str(s) => s.as_str().expect("plain text").to_owned(),
        other => panic!("expected a string, got {other:?}"),
    }
}

fn opt_text(value: Option<&Value>) -> Option<String> {
    match value {
        None | Some(Value::Null) => None,
        Some(v) => Some(text(v)),
    }
}

/// Rebuilds the `{"$decimal": "..."}` tags into `Decimal`s.
fn untag(value: &Value) -> Value {
    match value {
        Value::Dict(pairs) if pairs.len() == 1 && pairs[0].0 == Key::from("$decimal") => {
            Value::Decimal(text(&pairs[0].1).parse::<Decimal>().unwrap())
        }
        Value::Dict(pairs) => {
            Value::Dict(pairs.iter().map(|(k, v)| (k.clone(), untag(v))).collect())
        }
        Value::List(items) => Value::List(items.iter().map(untag).collect()),
        other => other.clone(),
    }
}

fn unsalt(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("amount#") {
        out.push_str(&rest[..at]);
        let tail = &rest[at + 7..];
        let hex = tail.chars().take(8).filter(char::is_ascii_hexdigit).count();
        if hex == 8 {
            out.push_str("amount#<salted>");
            rest = &tail[8..];
        } else {
            out.push_str("amount#");
            rest = tail;
        }
    }
    out.push_str(rest);
    out
}

#[test]
fn python_golden_vectors() {
    let mut counts = std::collections::BTreeMap::new();
    for (number, line) in include_str!("data/golden.txt").lines().enumerate() {
        let case = loads(line).unwrap();
        let op = text(get(&case, "op").unwrap());
        *counts.entry(op.clone()).or_insert(0usize) += 1;
        let at = format!("line {}", number + 1);
        match op.as_str() {
            "envelope" => {
                let kind = text(get(&case, "kind").unwrap());
                let data = untag(get(&case, "data").unwrap());
                let meta = match untag(get(&case, "meta").unwrap()) {
                    Value::Dict(pairs) => pairs
                        .into_iter()
                        .map(|(k, v)| match k {
                            Key::Str(k) => (k.as_str().unwrap().to_owned(), v),
                            other => panic!("{other:?}"),
                        })
                        .collect(),
                    _ => Vec::new(),
                };
                let envelope = build_envelope(&kind, data, meta).unwrap();
                assert_eq!(
                    cli_text(&envelope).unwrap(),
                    text(get(&case, "cli").unwrap()),
                    "{at}"
                );
                assert_eq!(
                    daemon_line(&envelope).unwrap(),
                    text(get(&case, "daemon").unwrap()),
                    "{at}"
                );
            }
            "event" => {
                let envelope = build_event_envelope(
                    &text(get(&case, "kind").unwrap()),
                    get(&case, "data").unwrap().clone(),
                );
                assert_eq!(
                    daemon_line(&envelope).unwrap(),
                    text(get(&case, "daemon").unwrap()),
                    "{at}"
                );
            }
            "error" => {
                let details = match untag(get(&case, "details").unwrap()) {
                    Value::Null => None,
                    other => Some(other),
                };
                let envelope = build_error_envelope(
                    &text(get(&case, "code").unwrap()),
                    &text(get(&case, "message").unwrap()),
                    details,
                    opt_text(get(&case, "hint")).as_deref(),
                    matches!(get(&case, "retryable"), Some(Value::Bool(true))),
                    opt_text(get(&case, "debug")).as_deref(),
                );
                assert_eq!(
                    cli_text(&envelope).unwrap(),
                    text(get(&case, "cli").unwrap()),
                    "{at}"
                );
                assert_eq!(
                    daemon_line(&envelope).unwrap(),
                    text(get(&case, "daemon").unwrap()),
                    "{at}"
                );
            }
            "secret_text" => {
                let input = text(get(&case, "in").unwrap());
                assert_eq!(
                    redact_secret_text(&input),
                    text(get(&case, "out").unwrap()),
                    "{at}: {input:?}"
                );
            }
            "operational_text" => {
                let input = text(get(&case, "in").unwrap());
                assert_eq!(
                    unsalt(&redact_operational_text(&input)),
                    text(get(&case, "out").unwrap()),
                    "{at}: {input:?}"
                );
            }
            "details" => {
                let redacted = redact_error_details(get(&case, "in").unwrap());
                let written = dumps(&redacted, &Options::new().compact()).unwrap();
                assert_eq!(unsalt(&written), text(get(&case, "out").unwrap()), "{at}");
            }
            other => panic!("unknown op {other}"),
        }
    }
    for (op, minimum) in [
        ("envelope", 10),
        ("error", 5),
        ("secret_text", 2000),
        ("operational_text", 2000),
        ("details", 8),
    ] {
        assert!(
            counts.get(op).copied().unwrap_or(0) >= minimum,
            "too few {op}"
        );
    }
}

#[test]
fn python_error_messages_preserve_codepoints_and_display_backslash_replacement() {
    let message = kassiber_pyjson::PyStr::from_code_points([0x31, 0xd800]).unwrap();
    let error = kassiber_envelope::AppError::new("validation", "unused")
        .with_python_message(message.clone());
    assert_eq!(error.message, "1\\ud800");
    let envelope = error.envelope(None);
    assert_eq!(
        get(get(&envelope, "error").unwrap(), "message"),
        Some(&Value::Str(message))
    );
    assert!(cli_text(&envelope)
        .unwrap()
        .contains(r#""message": "1\ud800""#));
    let plain = kassiber_envelope::AppError::new("validation", "ordinary");
    assert_eq!(
        plain.envelope(None),
        build_error_envelope("validation", "ordinary", None, None, false, None)
    );
}
