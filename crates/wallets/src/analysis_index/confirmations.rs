use super::{field, mapping, Text};
use crate::{database_error, publication::row::rows};
use kassiber_envelope::{dict, s, set, AppError};
use kassiber_pyjson::{BigInt, Value};
use kassiber_store::rusqlite::Connection;
use regex::Regex;
use std::collections::BTreeMap;
use std::sync::LazyLock;

static OCCURRENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\A[^:]+:[^:]+:domain:([^:]+):occ:([0-9a-f]{64}:(?:0|[1-9][0-9]*)):(?:tx|out):([0-9a-f]{64})(?::[0-9]+)?\z").unwrap()
});

pub const TIP_FIELDS: [&str; 5] = [
    "verified_tip_height",
    "verified_tip_hash",
    "verified_tip_at",
    "last_code",
    "status",
];

pub struct ConfirmationOverlay<'a> {
    conn: &'a Connection,
    profile: String,
    available: bool,
    cache: BTreeMap<(String, String, String), Value>,
}

fn nonnegative(value: &Value) -> Option<&BigInt> {
    match value {
        Value::Int(n) if n >= &BigInt::from(0) => Some(n),
        _ => None,
    }
}

fn empty() -> Value {
    dict([
        ("confirmations", Value::Null),
        ("confirmed", Value::Null),
        ("block_height", Value::Null),
        ("confirmation_observed_at", Value::Null),
    ])
}

impl<'a> ConfirmationOverlay<'a> {
    pub fn read(conn: &'a Connection, profile: &str) -> Result<Self, AppError> {
        let mut stmt = conn
            .prepare("PRAGMA table_info(chain_analysis_acquisition_grants)")
            .map_err(database_error)?;
        let columns = stmt
            .query_map([], |r| r.get::<_, String>(1))
            .map_err(database_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(database_error)?;
        Ok(Self {
            conn,
            profile: profile.to_owned(),
            available: TIP_FIELDS
                .iter()
                .all(|name| columns.iter().any(|v| v == name)),
            cache: BTreeMap::new(),
        })
    }

    pub fn apply(&mut self, node: &Value) -> Result<Value, AppError> {
        let Some(id) = field(node, "id").as_str() else {
            return Ok(node.clone());
        };
        let Some(captures) = OCCURRENCE.captures(id).filter(|_| self.available) else {
            return Ok(node.clone());
        };
        let key = (
            captures[1].to_owned(),
            captures[2].to_owned(),
            captures[3].to_owned(),
        );
        if !self.cache.contains_key(&key) {
            let observed = self.observation(&key.0, &key.1, &key.2)?;
            self.cache.insert(key.clone(), observed);
        }
        let mut result = node.clone();
        for (name, value) in super::fields(&self.cache[&key]) {
            set(&mut result, name, value.clone());
        }
        Ok(result)
    }

    fn observation(&self, domain: &str, occurrence: &str, txid: &str) -> Result<Value, AppError> {
        let block = occurrence.split(':').next().unwrap();
        let records = rows(self.conn, "SELECT a.status_json,g.verified_tip_height,g.verified_tip_hash,g.verified_tip_at,g.last_code FROM chain_analysis_reference_assertions a JOIN chain_analysis_acquisition_grants g ON g.id=a.grant_id AND g.profile_id=a.profile_id WHERE a.profile_id=? AND a.active=1 AND a.txid=? AND a.domain_id=? AND a.occurrence_id=?", [&self.profile, txid, domain, occurrence])?;
        let mut observations = Vec::new();
        for row in records {
            let status = mapping(field(&row, "status_json"));
            let (Some(height), Some(tip), Some(tip_hash), Some(at)) = (
                nonnegative(field(&status, "block_height")),
                nonnegative(field(&row, "verified_tip_height")),
                field(&row, "verified_tip_hash").as_str(),
                field(&row, "verified_tip_at").as_str(),
            ) else {
                continue;
            };
            let Some(instant) = kassiber_pytext::iso_datetime(kassiber_pytext::strip(at)) else {
                continue;
            };
            if matches!(
                field(&row, "last_code").as_str(),
                Some("chain_analysis_stale" | "reorg_reconciling")
            ) || field(&status, "confirmed") != &Value::Bool(true)
                || field(&status, "removed") == &Value::Bool(true)
                || field(&status, "conflicted") == &Value::Bool(true)
                || field(&status, "block_hash") != &s(block)
                || tip < height
                || tip_hash.len() != 64
                || !tip_hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                || tip == height && tip_hash != block
            {
                continue;
            }
            observations.push((
                instant.utc_us(),
                height.clone(),
                tip - height + BigInt::from(1),
                at.to_owned(),
            ));
        }
        let Some(newest) = observations.iter().map(|v| v.0).max() else {
            return Ok(empty());
        };
        let latest: Vec<_> = observations.iter().filter(|v| v.0 == newest).collect();
        let selected = latest.iter().min_by_key(|v| &v.3).unwrap();
        if latest
            .iter()
            .any(|v| (v.1.clone(), v.2.clone()) != (selected.1.clone(), selected.2.clone()))
        {
            return Ok(empty());
        }
        Ok(dict([
            ("confirmations", Value::Int(selected.2.clone())),
            ("confirmed", Value::Bool(true)),
            ("block_height", Value::Int(selected.1.clone())),
            ("confirmation_observed_at", s(selected.3.as_str())),
        ]))
    }
}
