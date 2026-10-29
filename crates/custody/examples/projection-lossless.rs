//! Unpackaged comparison driver. Operands and diagnostics have separate codecs.
use kassiber_custody::{codec, finalized_projection::compose_lossless};
use kassiber_pyjson::{Key, Options, Value as Py};
use std::io::{self, BufRead};
fn field<'a>(value: &'a Py, name: &str) -> &'a Py {
    let Py::Dict(entries) = value else {
        panic!("fixture mapping")
    };
    &entries
        .iter()
        .find(|(k, _)| *k == Key::Str(name.into()))
        .expect("fixture field")
        .1
}
fn main() {
    for line in io::stdin().lock().lines() {
        let request = codec::loads(&line.expect("fixture line")).expect("CPython fixture JSON");
        let Py::Str(operands) = field(&request, "operands_json") else {
            panic!("operand JSON string")
        };
        let operands = serde_json::from_str(operands.as_str().expect("ordinary operand JSON"))
            .expect("typed operands");
        let Py::List(diagnostics) = field(&request, "diagnostics") else {
            panic!("diagnostic population")
        };
        let result = match compose_lossless(&operands, diagnostics) {
            Ok(result) => Py::Dict(vec![(Key::Str("expected".into()), result)]),
            Err(error) => Py::Dict(vec![(Key::Str("error".into()), Py::Str(error.into()))]),
        };
        println!(
            "{}",
            kassiber_pyjson::dumps(&result, &Options::new()).expect("lossless result")
        );
    }
}
