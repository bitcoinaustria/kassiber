use super::Text;
#[cfg(any(test, feature = "test-harness"))]
use super::{field, list};
use bitcoin::hashes::{
    hmac::{Hmac, HmacEngine},
    sha256, Hash, HashEngine,
};
#[cfg(any(test, feature = "test-harness"))]
use kassiber_envelope::dict;
use kassiber_envelope::{s, AppError};
use kassiber_pyjson::{Options, Value};
use kassiber_store::rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, OnceLock};

const PREFIX: &str = "ca-ref:";
const CAPACITY: usize = 32768;

#[derive(Clone, PartialEq, Eq)]
pub struct BookScope(Vec<u8>);
impl BookScope {
    #[cfg(any(test, feature = "test-harness"))]
    pub(super) fn fixture(value: &str) -> Self {
        Self(value.as_bytes().to_vec())
    }
    pub fn read(conn: &Connection, profile: &str) -> Result<Self, AppError> {
        let path: String = conn
            .query_row(
                "SELECT file FROM pragma_database_list WHERE name='main'",
                [],
                |r| r.get(0),
            )
            .map_err(crate::database_error)?;
        let identity: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key='database_instance_id'",
                [],
                |r| r.get(0),
            )
            .optional()
            .map_err(crate::database_error)?;
        let location = if path.is_empty() {
            format!("memory-connection:{conn:p}")
        } else {
            path
        };
        let value = Value::List(vec![
            s(location),
            s(profile),
            identity.map(s).unwrap_or(Value::Null),
        ]);
        Ok(Self(
            kassiber_pyjson::dumps(&value, &Options::new().compact())
                .map_err(|_| super::invalid())?
                .into_bytes(),
        ))
    }
}
struct Entry {
    scope: BookScope,
    value: String,
    used: u64,
}
pub struct References {
    key: [u8; 32],
    process: u32,
    entries: BTreeMap<String, Entry>,
    clock: u64,
    capacity: usize,
}
impl References {
    fn new(conn: &Connection) -> Result<Self, AppError> {
        let key: Vec<u8> = conn
            .query_row("SELECT randomblob(32)", [], |r| r.get(0))
            .map_err(crate::database_error)?;
        Ok(Self::from_key(
            key.try_into().map_err(|_| super::invalid())?,
        ))
    }
    pub(super) fn from_key(key: [u8; 32]) -> Self {
        Self {
            key,
            process: std::process::id(),
            entries: BTreeMap::new(),
            clock: 0,
            capacity: CAPACITY,
        }
    }
    fn token(&self, scope: &BookScope, value: &str) -> String {
        let mut engine = HmacEngine::<sha256::Hash>::new(&self.key);
        engine.input(&scope.0);
        engine.input(&[0]);
        engine.input(value.as_bytes());
        format!("{PREFIX}{}", Hmac::<sha256::Hash>::from_engine(engine))
    }
    fn tick(&mut self) -> u64 {
        if self.clock == u64::MAX {
            let mut order: Vec<_> = self
                .entries
                .iter()
                .map(|(key, e)| (e.used, key.clone()))
                .collect();
            order.sort();
            for (n, (_, key)) in order.into_iter().enumerate() {
                self.entries.get_mut(&key).unwrap().used = n as u64;
            }
            self.clock = self.entries.len() as u64;
        }
        self.clock += 1;
        self.clock
    }
    pub fn issue(&mut self, scope: &BookScope, value: &str) -> Result<String, AppError> {
        if self.process != std::process::id() {
            return Err(stale());
        }
        let token = self.token(scope, value);
        let used = self.tick();
        self.entries.insert(
            token.clone(),
            Entry {
                scope: scope.clone(),
                value: value.to_owned(),
                used,
            },
        );
        while self.entries.len() > self.capacity {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.used)
                .map(|(key, _)| key.clone())
                .unwrap();
            self.entries.remove(&oldest);
        }
        Ok(token)
    }
    pub fn resolve(&mut self, scope: &BookScope, reference: &str) -> Result<String, AppError> {
        if self.process != std::process::id() {
            return Err(stale());
        }
        let entry = self
            .entries
            .get(reference)
            .filter(|entry| &entry.scope == scope)
            .ok_or_else(stale)?;
        if self.token(scope, &entry.value) != reference {
            return Err(stale());
        }
        let value = entry.value.clone();
        let used = self.tick();
        self.entries.get_mut(reference).unwrap().used = used;
        Ok(value)
    }
    pub fn clear(&mut self) {
        self.entries.clear();
    }
    pub fn decode(
        &mut self,
        args: &Value,
        view: &super::views::SqlView<'_>,
    ) -> Result<Value, AppError> {
        self.decode_scoped(&view.book_scope()?, args, view)
    }
    fn decode_scoped(
        &mut self,
        scope: &BookScope,
        args: &Value,
        view: &super::views::SqlView<'_>,
    ) -> Result<Value, AppError> {
        if !matches!(args, Value::Dict(_)) {
            return Err(AppError::new(
                "validation",
                "Chain analysis arguments must be an object",
            ));
        }
        let mut references = BTreeSet::new();
        collect(args, "", &mut references);
        let mut values = BTreeMap::new();
        for reference in references {
            let subject = self.resolve(scope, &reference)?;
            if !view.has_subject(&subject)? {
                return Err(stale());
            }
            values.insert(reference, subject);
        }
        Ok(decode(args, "", &values))
    }
}
impl Drop for References {
    fn drop(&mut self) {
        self.key.fill(0);
    }
}
fn stale() -> AppError {
    AppError::new(
        "chain_analysis_reference_stale",
        "Analysis reference expired or is not available in this book; refresh the investigation",
    )
    .retryable(true)
}
fn collect(value: &Value, key: &str, found: &mut BTreeSet<String>) {
    match value {
        Value::Dict(fields) => {
            for (key, value) in fields {
                if let Some(key) = key.as_str() {
                    collect(value, key, found);
                }
            }
        }
        Value::List(values) => {
            for value in values {
                collect(value, key, found);
            }
        }
        Value::Str(value) if ["subject", "target"].contains(&key) => {
            if let Some(value) = value.as_str().filter(|v| v.starts_with(PREFIX)) {
                found.insert(value.to_owned());
            }
        }
        _ => {}
    }
}
fn decode(value: &Value, key: &str, lookup: &BTreeMap<String, String>) -> Value {
    match value {
        Value::Dict(fields) => Value::Dict(
            fields
                .iter()
                .map(|(k, v)| (k.clone(), decode(v, k.as_str().unwrap_or(""), lookup)))
                .collect(),
        ),
        Value::List(values) => Value::List(values.iter().map(|v| decode(v, key, lookup)).collect()),
        Value::Str(value) if ["subject", "target"].contains(&key) => value
            .as_str()
            .and_then(|v| lookup.get(v))
            .map(|v| s(v.as_str()))
            .unwrap_or_else(|| Value::Str(value.clone())),
        _ => value.clone(),
    }
}
/// `decode_ai_args(conn, profile_id, args)`: `subject`/`target` handles back
/// to the subjects they stand for, revalidated against the current index.
/// Arguments without handles are returned unchanged without reading it.
pub fn decode_ai_args(conn: &Connection, profile: &str, args: &Value) -> Result<Value, AppError> {
    if !matches!(args, Value::Dict(_)) {
        return Err(AppError::new(
            "validation",
            "Chain analysis arguments must be an object",
        ));
    }
    let mut references = BTreeSet::new();
    collect(args, "", &mut references);
    if references.is_empty() {
        return Ok(args.clone());
    }
    let scope = BookScope::read(conn, profile)?;
    let mut values = with_process_references(conn, |refs| {
        Ok(references
            .iter()
            .filter_map(|reference| {
                refs.resolve(&scope, reference)
                    .ok()
                    .map(|subject| (reference.clone(), subject))
            })
            .collect::<BTreeMap<_, _>>())
    })?;
    // A remembered subject is never authority: only resolved handles are
    // revalidated, so a wholly stale request never reads the index.
    if !values.is_empty() {
        values = super::views::with_read(conn, profile, "owner", |view| {
            let mut current = BTreeMap::new();
            for (reference, subject) in values {
                if view.has_subject(&subject)? {
                    current.insert(reference, subject);
                }
            }
            Ok(current)
        })?;
    }
    if values.len() != references.len() {
        return Err(stale());
    }
    Ok(decode(args, "", &values))
}

/// `_reference(scope, value)` for each value, in order: the AI projection's
/// process- and book-scoped handles.
pub fn issue_ai_references(
    conn: &Connection,
    profile: &str,
    values: &[String],
) -> Result<Vec<String>, AppError> {
    let scope = BookScope::read(conn, profile)?;
    with_process_references(conn, |refs| {
        values
            .iter()
            .map(|value| refs.issue(&scope, value))
            .collect()
    })
}

static PROCESS_REFERENCES: OnceLock<Mutex<Option<References>>> = OnceLock::new();
pub fn with_process_references<R>(
    conn: &Connection,
    use_references: impl FnOnce(&mut References) -> Result<R, AppError>,
) -> Result<R, AppError> {
    let mut registry = PROCESS_REFERENCES
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| AppError::new("internal_error", "Local analysis references unavailable"))?;
    if registry.is_none() {
        *registry = Some(References::new(conn)?);
    }
    use_references(registry.as_mut().unwrap())
}

#[cfg(any(test, feature = "test-harness"))]
pub fn evaluate_fixture(case: &Value, conn: &Connection) -> Result<Value, AppError> {
    let mut references = if field(case, "production") == &Value::Bool(true) {
        References::new(conn)?
    } else {
        References::from_key([7; 32])
    };
    let mut issued = BTreeMap::new();
    let mut results = Vec::new();
    if let Some(size) = field(case, "capacity").as_str() {
        references.capacity = size.parse().unwrap();
    }
    for action in list(field(case, "actions")) {
        let scope = BookScope::fixture(field(action, "book").as_str().unwrap_or("book-p"));
        let result = match field(action, "operation").as_str() {
            Some("issue") => {
                let token = references.issue(&scope, field(action, "value").as_str().unwrap())?;
                issued.insert(
                    field(action, "name").as_str().unwrap_or("ref").to_owned(),
                    token.clone(),
                );
                Ok(s(token))
            }
            Some("restart") => {
                references = References::from_key([8; 32]);
                Ok(Value::Null)
            }
            Some("clear") => {
                references.clear();
                Ok(Value::Null)
            }
            Some("decode") => {
                fn expand(v: &Value, issued: &BTreeMap<String, String>) -> Value {
                    match v {
                        Value::Dict(entries) => Value::Dict(
                            entries
                                .iter()
                                .map(|(k, v)| (k.clone(), expand(v, issued)))
                                .collect(),
                        ),
                        Value::List(values) => {
                            Value::List(values.iter().map(|v| expand(v, issued)).collect())
                        }
                        Value::Str(text) => text
                            .as_str()
                            .and_then(|v| v.strip_prefix('$'))
                            .and_then(|key| issued.get(key))
                            .map(|v| s(v.as_str()))
                            .unwrap_or_else(|| v.clone()),
                        _ => v.clone(),
                    }
                }
                let args = expand(field(action, "args"), &issued);
                super::views::with_read(conn, "p", "owner", |view| {
                    references.decode_scoped(&scope, &args, view)
                })
            }
            _ => return Err(super::invalid()),
        };
        results.push(result.unwrap_or_else(|e| dict([("error", s(e.code))])));
    }
    Ok(Value::List(results))
}

#[cfg(test)]
mod tests;
