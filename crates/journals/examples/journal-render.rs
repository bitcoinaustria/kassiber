use kassiber_journals::{native, scalar};
use scalar::{dict, Value};
use std::{
    io::{self, BufRead, Write},
    time::Instant,
};

fn list(value: &Value) -> &[Value] {
    match value {
        Value::List(items) => items,
        _ => panic!("probe operand must be a list"),
    }
}

fn run(v: &Value, timing_only: bool) -> Value {
    let field = native::field;
    let tape = list(field(v, "entry_ids"));
    let mut consumed = 0;
    let cpu_start = cpu_seconds();
    let start = Instant::now();
    let result = native::build(
        field(v, "profile"),
        field(v, "custody"),
        field(v, "response"),
        || {
            let Value::Str(id) = tape.get(consumed).expect("declared UUID tape exhausted") else {
                panic!("probe UUID must be a string");
            };
            let id = id.as_str().expect("probe UUID must be UTF-8").into();
            consumed += 1;
            id
        },
    );
    let elapsed = start.elapsed().as_secs_f64();
    let cpu_elapsed = cpu_seconds() - cpu_start;
    let mut reply = vec![
        ("seconds".into(), Value::from(elapsed)),
        ("cpu_seconds".into(), Value::from(cpu_elapsed)),
        ("entry_ids_consumed".into(), Value::from(consumed)),
    ];
    match result {
        Ok(_) if timing_only => (),
        Ok(state) => reply.push((
            "state".into(),
            Value::from(scalar::encode(&state.snapshot()).expect("canonical state")),
        )),
        Err(error) => reply.push(("error".into(), error.0)),
    }
    Value::Dict(reply)
}

#[cfg(unix)]
fn cpu_seconds() -> f64 {
    use nix::sys::resource::{getrusage, UsageWho};
    let usage = getrusage(UsageWho::RUSAGE_SELF).expect("process CPU usage");
    let time = |t: nix::sys::time::TimeVal| t.tv_sec() as f64 + t.tv_usec() as f64 / 1_000_000.0;
    time(usage.user_time()) + time(usage.system_time())
}
#[cfg(not(unix))]
fn cpu_seconds() -> f64 {
    f64::NAN
}

fn decode(text: &str) -> Result<Value, kassiber_journals::Refusal> {
    scalar::decode(text)
}

fn main() {
    let timing_only = std::env::args().any(|a| a == "--timing-only");
    let mut cached: Option<(String, Value)> = None;
    for path in io::stdin().lock().lines() {
        let path = path.expect("path");
        if cached.as_ref().is_none_or(|(p, _)| p != &path) {
            let text = std::fs::read_to_string(&path).expect("input file");
            match decode(&text) {
                Ok(value) => cached = Some((path, value)),
                Err(error) => {
                    let error = native::Error::from(error);
                    println!(
                        "{}",
                        scalar::encode(&dict([("error", error.0)])).expect("decode refusal")
                    );
                    io::stdout().flush().expect("stdout");
                    continue;
                }
            }
        }
        let reply = run(&cached.as_ref().expect("loaded input").1, timing_only);
        println!("{}", scalar::encode(&reply).expect("reply"));
        io::stdout().flush().expect("stdout");
    }
}

#[cfg(test)]
#[path = "../tests/support/fidelity.rs"]
mod fidelity;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonfinite_probe_matches_python_states_refusals_and_uuid_consumption() {
        let cases = fidelity::cases(include_str!("../tests/data/render-nonfinite.json"));
        assert_eq!(cases.len(), 1776);
        for case in cases {
            let expected = native::field(&case, "expected");
            for timing_only in [false, true] {
                let reply = run(&case, timing_only);
                let name = scalar::repr(native::field(&case, "name"));
                assert_eq!(
                    native::field(&reply, "entry_ids_consumed"),
                    &Value::from(list(native::field(&case, "entry_ids")).len()),
                    "{name}"
                );
                let key = if matches!(native::field(expected, "state"), Value::Null) {
                    "error"
                } else {
                    "state"
                };
                if !timing_only || key == "error" {
                    assert_eq!(
                        scalar::encode(native::field(&reply, key)).unwrap(),
                        scalar::encode(native::field(expected, key)).unwrap(),
                        "{name}"
                    );
                }
            }
        }
    }

    #[test]
    fn lossless_probe_matches_python_states_refusals_and_uuid_consumption() {
        let mut count = 0;
        for source in [
            include_str!("../tests/data/render.json"),
            include_str!("../tests/data/render-legacy-floats.json"),
            include_str!("../tests/data/render-response-refusals.json"),
            include_str!("../tests/data/render-scalar-coercions.json"),
            include_str!("../tests/data/render-branches.json"),
            include_str!("../tests/data/render-native-builder.json"),
            include_str!("../tests/data/render-native-graph.json"),
        ] {
            let cases = decode(source).unwrap();
            for case in list(&cases) {
                count += 1;
                let expected = native::field(case, "expected");
                for timing_only in [false, true] {
                    let reply = run(case, timing_only);
                    let name = scalar::repr(native::field(case, "name"));
                    let consumed = native::field(&reply, "entry_ids_consumed");
                    assert_eq!(
                        consumed,
                        &Value::from(list(native::field(case, "entry_ids")).len()),
                        "{name}"
                    );
                    let key = if matches!(native::field(expected, "state"), Value::Null) {
                        "error"
                    } else {
                        "state"
                    };
                    if !timing_only || key == "error" {
                        assert_eq!(
                            scalar::encode(native::field(&reply, key)).unwrap(),
                            scalar::encode(native::field(expected, key)).unwrap(),
                            "{name}"
                        );
                    } else {
                        assert!(
                            matches!(native::field(&reply, "state"), Value::Null),
                            "{name}"
                        );
                    }
                }
            }
        }
        assert_eq!(count, 1438);
    }
}
