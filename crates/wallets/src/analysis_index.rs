//! Immutable local analysis snapshots and physical occurrence identities.
use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{dumps, Options, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub mod analytics;
pub mod confirmations;
pub mod datasets;
pub mod durable;
pub mod entropy;
#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;
pub mod handles;
pub mod ingestion;
pub mod mirror;
mod observer;
pub mod occurrences;
pub mod reads;
pub mod runtime;
pub mod sources;
pub mod traversal;
mod value;
pub mod views;

pub(super) trait Text {
    fn as_str(&self) -> Option<&str>;
}
impl Text for Value {
    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(v) => v.as_str(),
            _ => None,
        }
    }
}
impl Text for kassiber_pyjson::Key {
    fn as_str(&self) -> Option<&str> {
        match self {
            kassiber_pyjson::Key::Str(v) => v.as_str(),
            _ => None,
        }
    }
}

pub(super) fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    get(v, key).unwrap_or(&Value::Null)
}
pub(super) fn list(v: &Value) -> &[Value] {
    match v {
        Value::List(v) => v,
        _ => &[],
    }
}
pub(super) fn fields(v: &Value) -> impl Iterator<Item = (&str, &Value)> {
    match v {
        Value::Dict(v) => v.as_slice(),
        _ => &[],
    }
    .iter()
    .map(|(k, v)| (k.as_str().expect("validated analysis key"), v))
}
pub(super) fn remove(v: &mut Value, key: &str) {
    if let Value::Dict(v) = v {
        v.retain(|(k, _)| k != &key.into());
    }
}
pub fn digest(value: &Value) -> Result<String, AppError> {
    let text = dumps(
        value,
        &Options::new().sort_keys(true).compact().default_str(true),
    )
    .map_err(|_| invalid())?;
    Ok(Sha256::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}
fn invalid() -> AppError {
    AppError::new("validation", "Invalid local analysis snapshot")
}

/// Owned data, with no database handle or mutable access escaping the capture.
/// Map iteration retains the explicit source order; lookup uses immutable indexes.
#[derive(Clone, Debug)]
pub struct AnalysisIndex {
    snapshot: Value,
    positions: BTreeMap<&'static str, BTreeMap<kassiber_pyjson::PyStr, usize>>,
}
impl AnalysisIndex {
    pub fn from_snapshot(snapshot: Value) -> Result<Self, AppError> {
        let mut positions = BTreeMap::new();
        if field(&snapshot, "snapshot_id").as_str().is_none()
            || !matches!(field(&snapshot, "coverage"), Value::Dict(_))
        {
            return Err(invalid());
        }
        for name in [
            "nodes",
            "edges",
            "outgoing",
            "incoming",
            "subjects",
            "transaction_facts",
            "output_facts",
        ] {
            let Value::Dict(entries) = field(&snapshot, name) else {
                return Err(invalid());
            };
            let mut index = BTreeMap::new();
            for (position, (key, value)) in entries.iter().enumerate() {
                let kassiber_pyjson::Key::Str(key) = key else {
                    return Err(invalid());
                };
                if name != "subjects" && key.as_str().is_none() {
                    return Err(invalid());
                }
                if index.insert(key.to_owned(), position).is_some()
                    || if ["outgoing", "incoming", "subjects"].contains(&name) {
                        !matches!(value, Value::List(_))
                    } else {
                        !matches!(value, Value::Dict(_))
                    }
                {
                    return Err(invalid());
                }
            }
            positions.insert(name, index);
        }
        for name in ["findings", "labels", "profile_seeds"] {
            if !matches!(field(&snapshot, name), Value::List(_)) {
                return Err(invalid());
            }
        }
        Ok(Self {
            snapshot,
            positions,
        })
    }
    pub fn snapshot(&self) -> &Value {
        &self.snapshot
    }
    pub fn lookup(&self, map: &str, key: &str) -> Option<&Value> {
        let at = *self
            .positions
            .get(map)?
            .get(&kassiber_pyjson::PyStr::from(key))?;
        let Value::Dict(values) = field(&self.snapshot, map) else {
            return None;
        };
        Some(&values[at].1)
    }
    pub fn observer(&self, observer: &str) -> Result<Self, AppError> {
        if observer != "public"
            || field(field(&self.snapshot, "coverage"), "observer_knowledge")
                == &s("public_chain_facts")
        {
            return Ok(self.clone());
        }
        observer::public(self)
    }
}

pub(super) fn mapping(value: &Value) -> Value {
    match value {
        Value::Dict(_) => value.clone(),
        Value::Str(raw) => raw
            .as_str()
            .and_then(|raw| kassiber_pytext::loads(if raw.is_empty() { "{}" } else { raw }).ok())
            .filter(|v| matches!(v, Value::Dict(_)))
            .unwrap_or_else(|| dict::<&str>([])),
        _ => dict::<&str>([]),
    }
}

#[cfg(test)]
mod tests;
