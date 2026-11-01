use super::*;

struct Interrupt;
impl Read for Interrupt {
    fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
        Err(std::io::ErrorKind::Interrupted.into())
    }
}

#[test]
fn document_cli_review_matches_python_scripted_stdin() {
    let Value::List(cases) =
        kassiber_pytext::loads(include_str!("../../tests/data/document_review.json")).unwrap()
    else {
        panic!()
    };
    for case in cases {
        let name = py_str(field(&case, "name"));
        let mut input = std::io::Cursor::new(py_str(field(&case, "stdin")).into_bytes());
        let mut interrupt = Interrupt;
        let reader: &mut dyn Read = if truthy(field(&case, "interrupt")) {
            &mut interrupt
        } else {
            &mut input
        };
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let mut draft = field(&case, "draft").clone();
        if let Value::List(mut rows) = field(&draft, "rows").clone() {
            for row in &mut rows {
                if let Value::Dict(mut cells) = field(row, "cell_confidences").clone() {
                    cells.reverse();
                    kassiber_envelope::set(row, "cell_confidences", Value::Dict(cells));
                }
            }
            kassiber_envelope::set(&mut draft, "rows", Value::List(rows));
        }
        let result = select(
            &draft,
            "EUR",
            truthy(field(&case, "include")),
            truthy(field(&case, "interactive")),
            reader,
            &mut stdout,
            &mut stderr,
        );
        let expected = field(&case, "error");
        match result {
            Ok(ids) => assert_eq!(
                Value::List(ids.iter().map(|id| s(id.as_str())).collect()),
                *field(&case, "selected"),
                "{name}"
            ),
            Err(e) => {
                assert_ne!(*expected, Value::Null, "{name}: {}", e.message);
                assert_eq!(s(e.code), *field(expected, "code"), "{name}");
                assert_eq!(s(e.message), *field(expected, "message"), "{name}");
                let canonical = |v: &Value| {
                    kassiber_pyjson::dumps(v, &kassiber_pyjson::Options::new().sort_keys(true))
                        .unwrap()
                };
                assert_eq!(
                    canonical(&e.details.unwrap_or(Value::Null)),
                    canonical(field(expected, "details")),
                    "{name}"
                );
                assert_eq!(
                    e.hint.map_or(Value::Null, |h| s(h.as_ref())),
                    *field(expected, "hint"),
                    "{name}"
                );
                assert_eq!(
                    Value::Bool(e.retryable),
                    *field(expected, "retryable"),
                    "{name}"
                );
            }
        }
        assert_eq!(
            String::from_utf8(stdout).unwrap(),
            py_str(field(&case, "stdout")),
            "{name}"
        );
        assert_eq!(
            String::from_utf8(stderr).unwrap(),
            py_str(field(&case, "stderr")),
            "{name}"
        );
    }
}
