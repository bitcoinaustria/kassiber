//! Python's ordered, lossless JSON semantics for retained quarantine details.
use super::*;
use kassiber_custody::codec;
use kassiber_pyjson::{Key, Value as Py};

fn key(name: &str) -> Key {
    Key::Str(name.into())
}
fn dump(v: &Py, compact: bool) -> Result<String, Refusal> {
    let mut options = kassiber_pyjson::Options::new().sort_keys(true);
    if compact {
        options = options.compact();
    }
    kassiber_pyjson::dumps(v, &options).map_err(|e| Refusal::new("ValueError", e.to_string()))
}
fn pop(v: &mut Vec<(Key, Py)>, name: &str) -> Py {
    v.iter()
        .position(|(k, _)| *k == key(name))
        .map(|i| v.remove(i).1)
        .unwrap_or(Py::Null)
}
fn list(v: Py) -> Result<Vec<Py>, Refusal> {
    Ok(match v {
        Py::Null | Py::Bool(false) => vec![],
        Py::Int(n) if n == 0.into() => vec![],
        Py::Float(0.0) => vec![],
        Py::List(v) => v,
        Py::Bytes(v) => v.into_iter().map(|b| Py::from(i64::from(b))).collect(),
        Py::Dict(v) => v
            .into_iter()
            .map(|(k, _)| match k {
                Key::Str(s) => Py::Str(s),
                _ => unreachable!("JSON object keys are strings"),
            })
            .collect(),
        Py::Str(s) => s
            .code_points()
            .map(|p| {
                Py::Str(
                    kassiber_pyjson::PyStr::from_code_points([p]).expect("valid Python code point"),
                )
            })
            .collect(),
        Py::Bool(_) => return Err(Refusal::new("TypeError", "'bool' object is not iterable")),
        Py::Int(_) => return Err(Refusal::new("TypeError", "'int' object is not iterable")),
        Py::Float(_) => return Err(Refusal::new("TypeError", "'float' object is not iterable")),
        Py::Decimal(_) => {
            return Err(Refusal::new(
                "TypeError",
                "'decimal.Decimal' object is not iterable",
            ))
        }
    })
}
fn parsed(s: &str) -> Py {
    codec::loads(s).unwrap_or_else(|_| Py::from(s))
}
fn downstream(r: &str) -> bool {
    matches!(
        r,
        "custody_basis_barrier"
            | "transfer_pair_dependency_blocked"
            | "basis_provenance_incomplete"
            | "derived_transfer_group_blocked"
            | "at_swap_basis_carry_unresolved"
            | "bitcoin_rail_carry_basis_unresolved"
    )
}

/// First-seen transaction order, root precedence, duplicate and carried reasons.
pub fn dedupe_quarantines(rows: &[QuarantineRow]) -> Result<Vec<QuarantineRow>, Refusal> {
    let mut positions = HashMap::new();
    let mut out: Vec<QuarantineRow> = Vec::new();
    for q in rows {
        let id = string(&q.transaction_id);
        let index = match positions.get(&id) {
            Some(i) => *i,
            None => {
                positions.insert(id, out.len());
                out.push(q.clone());
                continue;
            }
        };
        let old = &out[index];
        if old.reason == q.reason && old.detail_json == q.detail_json {
            continue;
        }
        let (mut primary, secondary) = if downstream(&old.reason) && !downstream(&q.reason) {
            (q.clone(), old)
        } else {
            (old.clone(), q)
        };
        let mut detail = match parsed(&primary.detail_json) {
            Py::Dict(v) => v,
            v => vec![(key("detail"), v)],
        };
        let mut additional = list(pop(&mut detail, "additional_reasons"))?;
        let mut secondary_detail = parsed(&secondary.detail_json);
        let carried = if let Py::Dict(v) = &mut secondary_detail {
            list(pop(v, "additional_reasons"))?
        } else {
            vec![]
        };
        additional.push(Py::Dict(vec![
            (key("reason"), Py::from(secondary.reason.as_str())),
            (key("detail"), secondary_detail),
        ]));
        additional.extend(carried);
        detail.push((key("additional_reasons"), Py::List(additional)));
        primary.detail_json = dump(&Py::Dict(detail), false)?;
        out[index] = primary;
    }
    Ok(out)
}

/// Invalid/non-object detail becomes an empty object. Valid legacy nonfinite
/// floats and escaped surrogate code points remain intact in the stored text.
pub fn enrich_quarantine_detail(
    detail_json: &str,
    candidates: &[Value],
) -> Result<String, Refusal> {
    let mut detail = match codec::loads(if detail_json.is_empty() {
        "{}"
    } else {
        detail_json
    }) {
        Ok(Py::Dict(v)) => v,
        _ => vec![],
    };
    if !candidates.is_empty() {
        pop(&mut detail, "ownership_review_candidates");
        detail.push((
            key("ownership_review_candidates"),
            codec::from_json(&json!(candidates))?,
        ));
    }
    dump(&Py::Dict(detail), true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quarantine_list_coercion_and_parse_fallbacks_match_python() {
        for value in [
            Py::Null,
            Py::Bool(false),
            Py::Int(0.into()),
            Py::Float(0.0),
            Py::Float(-0.0),
        ] {
            assert!(list(value).unwrap().is_empty());
        }
        assert_eq!(
            list(Py::List(vec![Py::from("value")])).unwrap(),
            vec![Py::from("value")]
        );
        assert_eq!(
            list(Py::Dict(vec![(key("z"), Py::Null), (key("a"), Py::Null)])).unwrap(),
            vec![Py::from("z"), Py::from("a")]
        );
        assert_eq!(
            list(codec::loads("\"🔥\\ud800\"").unwrap()).unwrap(),
            vec![
                codec::loads("\"🔥\"").unwrap(),
                codec::loads("\"\\ud800\"").unwrap()
            ]
        );
        for (value, typename) in [
            (Py::Bool(true), "bool"),
            (Py::Int(1.into()), "int"),
            (Py::Float(1.5), "float"),
        ] {
            let refusal = list(value).unwrap_err();
            assert_eq!(refusal.family, "TypeError");
            assert_eq!(
                refusal.message,
                format!("'{typename}' object is not iterable")
            );
        }
        let mut object = vec![(key("first"), Py::from("value")), (key("last"), Py::Null)];
        assert_eq!(pop(&mut object, "missing"), Py::Null);
        assert_eq!(object.len(), 2);
        assert_eq!(pop(&mut object, "first"), Py::from("value"));
        assert_eq!(object, vec![(key("last"), Py::Null)]);
        assert_eq!(parsed("bad json"), Py::from("bad json"));
        assert_eq!(parsed("[1]"), Py::List(vec![Py::Int(1.into())]));
        let value = Py::Dict(vec![
            (key("b"), Py::Int(2.into())),
            (key("a"), Py::Int(1.into())),
        ]);
        assert_eq!(dump(&value, true).unwrap(), "{\"a\":1,\"b\":2}");
        assert_eq!(dump(&value, false).unwrap(), "{\"a\": 1, \"b\": 2}");
    }

    #[test]
    fn quarantine_internal_only_values_keep_refusals_and_key_invariants() {
        let decimal = Py::Decimal("1.00".parse().unwrap());
        let refusal = list(decimal.clone()).unwrap_err();
        assert_eq!(refusal.family, "TypeError");
        assert_eq!(refusal.message, "'decimal.Decimal' object is not iterable");
        let refusal = dump(&decimal, false).unwrap_err();
        assert_eq!(refusal.family, "ValueError");
        assert_eq!(
            refusal.message,
            "object of type Decimal is not JSON serializable"
        );
        // JSON decoding cannot produce Decimal values or non-string keys.
        // Construct them here to pin the defensive helper-only paths.
        assert!(
            std::panic::catch_unwind(|| list(Py::Dict(vec![(Key::Int(1.into()), Py::Null)])))
                .is_err()
        );
    }
}
