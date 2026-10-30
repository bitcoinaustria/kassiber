//! Transitional graph facts for the journal review join; no ownership authority.
use crate::graph::{self, ownership::Index};
use serde_json::Value;

pub type Scope = (String, String, String, String);

pub fn onchain_scope(row: &Value) -> Option<Scope> {
    graph::onchain_scope(row)
}
pub fn parsed(row: &Value) -> Result<Option<Value>, String> {
    graph::parse(&row["raw_json"])
}
pub fn ownership_scope(
    row: &Value,
    parsed: &Value,
    index: &Value,
) -> Result<Option<Scope>, String> {
    graph::ownership::scope(row, Some(parsed), Index(Some(index)))
}
pub fn script_owners(
    script: &Value,
    scope: &(String, String),
    index: &Value,
) -> Result<std::collections::BTreeSet<String>, String> {
    Index(Some(index)).script_owners(script, scope)
}
pub fn clear_memos() {
    crate::codec::clear_memos();
}
