//! Block occurrences qualify identity independently of the acquisition grant.
use super::{digest, field, list, mapping, Text};
use crate::database_error;
use kassiber_envelope::{dict, py_str, s, truthy, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{types::Value as Sql, Connection, OptionalExtension};
use regex::Regex;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::LazyLock;

static BLOCK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\A[0-9a-f]{64}:(?:0|[1-9][0-9]*)\z").unwrap());
static TRANSACTION: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A(bitcoin|liquid):([^:]+):(?:domain:([^:]+):(?:occ:([0-9a-f]{64}:(?:0|[1-9][0-9]*)):)?)?tx:([0-9a-f]{64})\z").unwrap()
});
fn occurrence(value: &Value) -> Option<&str> {
    value.as_str().filter(|v| BLOCK.is_match(v))
}
fn sql(value: &Value) -> Sql {
    match value {
        Value::Null => Sql::Null,
        Value::Bool(v) => Sql::Integer(i64::from(*v)),
        Value::Int(v) => {
            i64::try_from(v.clone()).map_or_else(|_| Sql::Text(py_str(value)), Sql::Integer)
        }
        Value::Float(v) => Sql::Real(*v),
        v => Sql::Text(py_str(v)),
    }
}
fn table(conn: &Connection, name: &str) -> Result<bool, AppError> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?)",
        [name],
        |r| r.get(0),
    )
    .map_err(database_error)
}

#[derive(Clone, Debug)]
pub struct SourceIdentity {
    pub domain_id: Value,
    pub namespace: String,
    pub occurrence_id: Option<String>,
    pub trusted_reference: bool,
}
impl SourceIdentity {
    pub fn value(&self) -> Value {
        dict([
            ("domain_id", self.domain_id.clone()),
            ("namespace", s(self.namespace.as_str())),
            (
                "occurrence_id",
                self.occurrence_id.as_deref().map_or(Value::Null, s),
            ),
            ("trusted_reference", Value::Bool(self.trusted_reference)),
        ])
    }
}

/// Borrows the caller's read snapshot; never commits, refreshes or grants egress.
pub struct OccurrenceResolver<'a> {
    conn: &'a Connection,
    profile: String,
    has_assertions: bool,
    domains: BTreeMap<(String, String), Value>,
    binding: Value,
    cache: BTreeMap<(String, String), Vec<String>>,
    ambiguous: BTreeSet<String>,
}
impl<'a> OccurrenceResolver<'a> {
    pub fn read(conn: &'a Connection, profile: &str) -> Result<Self, AppError> {
        let mut domains = BTreeMap::new();
        let mut binding = dict([("state", s("unbound"))]);
        if table(conn, "book_network_bindings")? {
            let raw: Option<String> = conn
                .query_row(
                    "SELECT domains_json FROM book_network_bindings WHERE profile_id=?",
                    [profile],
                    |r| r.get(0),
                )
                .optional()
                .map_err(database_error)?;
            if let Some(raw) = raw {
                let parsed = kassiber_pytext::loads(&raw).map_err(|_| {
                    AppError::new("internal_error", "Stored book domains are invalid")
                })?;
                for item in list(&parsed) {
                    domains.insert(
                        (py_str(field(item, "chain")), py_str(field(item, "network"))),
                        item.clone(),
                    );
                }
                binding = kassiber_setup::networks::resolve(conn, profile)?;
            }
        }
        Ok(Self {
            conn,
            profile: profile.to_owned(),
            has_assertions: table(conn, "chain_analysis_reference_assertions")?,
            domains,
            binding,
            cache: BTreeMap::new(),
            ambiguous: BTreeSet::new(),
        })
    }
    pub fn accepts(&self, row: &Value, unscoped: bool) -> bool {
        if !kassiber_setup::networks::observation_matches_binding(&self.binding, row, unscoped) {
            return false;
        }
        let domain = [field(row, "_index_domain_id"), field(row, "domain_id")]
            .into_iter()
            .find(|v| truthy(v));
        domain.is_none()
            || self.domains.is_empty()
            || self
                .domains
                .values()
                .any(|v| Some(field(v, "domain_id")) == domain)
    }
    pub fn source(
        &self,
        row: &Value,
        chain: &str,
        network: &str,
    ) -> Result<SourceIdentity, AppError> {
        let raw = mapping(field(row, "raw_json"));
        let config = mapping(field(row, "wallet_config_json"));
        let instance = [row, &raw, &config]
            .into_iter()
            .map(|v| field(v, "chain_instance_id"))
            .find(|v| truthy(v));
        let default = digest(&Value::List(vec![s(chain), s(network), Value::Null]))?;
        let bound = self
            .domains
            .get(&(chain.to_owned(), network.to_owned()))
            .map(|v| field(v, "domain_id"))
            .unwrap_or(&Value::Null);
        let domain = if truthy(field(row, "_index_domain_id")) {
            field(row, "_index_domain_id").clone()
        } else if let Some(instance) = instance {
            s(digest(&Value::List(vec![
                s(chain),
                s(network),
                instance.clone(),
            ]))?)
        } else if truthy(bound) {
            bound.clone()
        } else {
            s(default.as_str())
        };
        Ok(SourceIdentity {
            namespace: if domain == s(default) {
                String::new()
            } else {
                format!("domain:{}:", py_str(&domain))
            },
            domain_id: domain,
            occurrence_id: occurrence(field(row, "_index_occurrence_id")).map(str::to_owned),
            trusted_reference: field(row, "_index_reference") == &Value::Bool(true),
        })
    }
    pub fn candidates(&mut self, domain: &Value, txid: &str) -> Result<&[String], AppError> {
        let key = (super::digest(domain)?, txid.to_owned());
        if !self.cache.contains_key(&key) {
            let mut values = BTreeSet::new();
            if self.has_assertions {
                let mut stmt = self.conn.prepare("SELECT occurrence_id,status_json FROM chain_analysis_reference_assertions WHERE profile_id=? AND active=1 AND txid=? AND domain_id=?").map_err(database_error)?;
                let rows = stmt
                    .query_map(
                        [
                            Sql::Text(self.profile.clone()),
                            Sql::Text(txid.to_owned()),
                            sql(domain),
                        ],
                        |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?)),
                    )
                    .map_err(database_error)?;
                for row in rows {
                    let (id, status) = row.map_err(database_error)?;
                    if !BLOCK.is_match(&id) {
                        continue;
                    }
                    let metadata = status
                        .as_deref()
                        .and_then(|v| kassiber_pytext::loads(v).ok());
                    if metadata.as_ref().is_some_and(|v| {
                        field(v, "block_hash") == &s(id.split(':').next().unwrap())
                    }) {
                        values.insert(id);
                    }
                }
            }
            self.cache.insert(key.clone(), values.into_iter().collect());
        }
        Ok(&self.cache[&key])
    }
    pub fn transaction_id(
        &mut self,
        chain: &str,
        network: &str,
        txid: &str,
        source: &SourceIdentity,
        explicit: &Value,
        resolve: bool,
    ) -> Result<String, AppError> {
        if let Some(occurrence) = occurrence(explicit) {
            return Ok(format!(
                "{chain}:{network}:domain:{}:occ:{occurrence}:tx:{txid}",
                py_str(&source.domain_id)
            ));
        }
        let candidates = if resolve {
            self.candidates(&source.domain_id, txid)?
        } else {
            &[]
        };
        if candidates.len() == 1 {
            return Ok(format!(
                "{chain}:{network}:domain:{}:occ:{}:tx:{txid}",
                py_str(&source.domain_id),
                candidates[0]
            ));
        }
        let id = format!("{chain}:{network}:{}tx:{txid}", source.namespace);
        if candidates.len() > 1 {
            self.ambiguous.insert(id.clone());
        }
        Ok(id)
    }
    pub fn ambiguous(&self) -> &BTreeSet<String> {
        &self.ambiguous
    }
}

/// Canonical physical subjects only. Explicit qualifiers never fall back.
pub fn parse_transaction_identity(
    subject: &str,
    chain: Option<&str>,
    network: Option<&str>,
) -> Option<Value> {
    if subject.len() == 64
        && subject
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Some(Value::List(vec![
            s(chain.filter(|v| !v.is_empty())?),
            s(network.filter(|v| !v.is_empty())?),
            s(subject),
            Value::Null,
            Value::Null,
        ]));
    }
    let captures = TRANSACTION.captures(subject)?;
    Some(Value::List(
        [1, 2, 5, 3, 4]
            .into_iter()
            .map(|n| captures.get(n).map_or(Value::Null, |v| s(v.as_str())))
            .collect(),
    ))
}
