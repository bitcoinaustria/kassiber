//! Stored quantity identities use CPython's exact compact ASCII JSON preimage.
use crate::{codec, interpretation::python_value_string};
use kassiber_pyjson::{Options, Value as Py};

fn compact_hash(values: Vec<Py>) -> Result<(String, String), String> {
    let text = kassiber_pyjson::dumps(&Py::List(values), &Options::new().compact())
        .map_err(|e| e.to_string())?;
    Ok((codec::sha256(&text), text))
}
pub fn issue_id(parts: &[Py]) -> Result<(String, String), String> {
    compact_hash(
        parts
            .iter()
            .map(|p| match p {
                Py::Str(_) => p.clone(),
                _ => Py::Str(python_value_string(p).into()),
            })
            .collect(),
    )
}
/// A rejected canonical event's dataclass repr is part of the stored preimage.
pub fn rejected_event_issue_id(
    event: &serde_json::Value,
    code: &str,
) -> Result<(String, String), String> {
    issue_id(&[
        Py::Str("canonical".into()),
        Py::Str(crate::interpretation::canonical_event_repr(event).into()),
        Py::Str(code.into()),
    ])
}
/// Fields follow custody_quantity_store.custody_decision_id's ten arguments.
pub fn custody_decision_id(fields: &[Py]) -> Result<(String, String), String> {
    let mut values = vec![Py::Str("canonical-custody-decision-v1".into())];
    values.extend_from_slice(fields);
    compact_hash(values)
}
/// Fields follow custody_quantity_store.economic_relation_id's eight arguments.
pub fn economic_relation_id(fields: &[Py]) -> Result<(String, String), String> {
    let mut values = vec![Py::Str("canonical-custody-economic-relation-v3".into())];
    values.extend_from_slice(fields);
    compact_hash(values)
}
