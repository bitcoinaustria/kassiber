//! Read-only custody facts inside the caller's journal transaction.
//!
//! This private pipeline seam is deliberately separate from product routing.
//! The ordered Python model retains SQLite storage classes and insertion order
//! for the v1 byte gate. `into_custody` is the temporary, text-free bridge to
//! today's custody rows; typed BookSnapshot can replace that one method.
mod ownership;
mod receipts;
#[cfg(test)]
mod tests;
mod worklist;

use kassiber_pyjson::{Key, Options, Value};
use kassiber_store::rusqlite::{self, types::ValueRef, Connection, Params};
use std::collections::{BTreeSet, HashMap};

#[derive(Debug)]
pub enum Error {
    /// Never start, commit, or repair a transaction on the caller's behalf.
    TransactionRequired,
    Database(rusqlite::Error),
    Unsupported(&'static str),
    Invalid(String),
    Json(kassiber_pyjson::Error),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // SQL, bound values and wallet material are private, even on failure.
        f.write_str(match self {
            Self::TransactionRequired => "custody loading requires the caller's write transaction",
            Self::Database(_) => "custody snapshot database read failed",
            Self::Unsupported(message) => message,
            Self::Invalid(_) => "custody snapshot contains unsupported facts",
            Self::Json(_) => "custody snapshot v1 serialization failed",
        })
    }
}
impl std::error::Error for Error {}
impl From<rusqlite::Error> for Error {
    fn from(error: rusqlite::Error) -> Self {
        Self::Database(error)
    }
}
type Result<T> = std::result::Result<T, Error>;

/// Raw facts only: no accounting admission, clocks, repairs or writes.
pub struct LoadedSnapshot {
    facts: Value,
}
impl LoadedSnapshot {
    /// Python json.dumps(..., ensure_ascii=True, allow_nan=True), including
    /// default separators and dict order. This is a verification interface.
    pub fn v1(&self) -> Result<String> {
        kassiber_pyjson::dumps(
            &self.facts,
            &Options::new().ensure_ascii(true).allow_nan(true),
        )
        .map_err(Error::Json)
    }
    pub fn facts(&self) -> &Value {
        &self.facts
    }
    /// No JSON text crosses the native pipeline boundary. Refuse values that
    /// today's serde adapter cannot represent rather than silently losing them.
    pub fn into_custody(self) -> Result<kassiber_custody::snapshot::Snapshot> {
        fn convert(value: Value) -> Result<serde_json::Value> {
            use serde_json::Value as J;
            Ok(match value {
                Value::Null => J::Null,
                Value::Bool(value) => J::Bool(value),
                Value::Int(value) => J::Number(
                    value
                        .to_string()
                        .parse()
                        .map_err(|_| Error::Invalid("integer".into()))?,
                ),
                Value::Float(value) => {
                    if !value.is_finite() {
                        return Err(Error::Unsupported(
                            "custody adapter cannot represent nonfinite SQLite REAL",
                        ));
                    }
                    // The existing adapter keeps arbitrary-precision number
                    // spellings, including Python's exponent padding.
                    J::Number(
                        kassiber_pyjson::float_repr(value)
                            .parse()
                            .map_err(|_| Error::Invalid("float".into()))?,
                    )
                }
                Value::Str(value) => J::String(
                    value
                        .as_str()
                        .ok_or(Error::Unsupported(
                            "custody adapter cannot represent surrogate strings",
                        ))?
                        .into(),
                ),
                Value::List(values) => {
                    J::Array(values.into_iter().map(convert).collect::<Result<_>>()?)
                }
                Value::Dict(values) => J::Object(
                    values
                        .into_iter()
                        .map(|(key, value)| {
                            let Key::Str(key) = key else {
                                return Err(Error::Invalid("key".into()));
                            };
                            Ok((
                                key.as_str()
                                    .ok_or(Error::Unsupported(
                                        "custody adapter cannot represent surrogate keys",
                                    ))?
                                    .into(),
                                convert(value)?,
                            ))
                        })
                        .collect::<Result<_>>()?,
                ),
                Value::Decimal(_) => return Err(Error::Invalid("unexpected Decimal".into())),
            })
        }
        let snapshot: kassiber_custody::snapshot::Snapshot =
            serde_json::from_value(convert(self.facts)?)
                .map_err(|error| Error::Invalid(error.to_string()))?;
        snapshot.validate().map_err(Error::Invalid)?;
        Ok(snapshot)
    }
}

fn dict<const N: usize>(values: [(&str, Value); N]) -> Value {
    Value::Dict(
        values
            .into_iter()
            .map(|(key, value)| (Key::from(key), value))
            .collect(),
    )
}
fn list(values: Vec<Value>) -> Value {
    Value::List(values)
}
fn get<'a>(value: &'a Value, key: &str) -> &'a Value {
    if let Value::Dict(values) = value {
        if let Some((_, value)) = values
            .iter()
            .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
        {
            return value;
        }
    }
    &Value::Null
}
fn set(value: &mut Value, key: &str, item: Value) {
    let Value::Dict(values) = value else {
        unreachable!("loader record");
    };
    if let Some((_, value)) = values
        .iter_mut()
        .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
    {
        *value = item;
    } else {
        values.push((Key::from(key), item));
    }
}
fn text(value: &Value) -> String {
    kassiber_envelope::py_str(value)
}
fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(value) => *value,
        Value::Int(value) => *value != 0.into(),
        Value::Float(value) => *value != 0.0,
        Value::Str(value) => !value.is(""),
        Value::List(value) => !value.is_empty(),
        Value::Dict(value) => !value.is_empty(),
        Value::Decimal(value) => !value.is_zero(),
    }
}
fn parsed(value: &Value) -> Value {
    match value {
        Value::Str(value) => value
            .as_str()
            .and_then(|s| kassiber_pytext::loads(s).ok())
            .unwrap_or(Value::Null),
        _ => Value::Null,
    }
}
fn sql_value(value: ValueRef<'_>) -> Result<Value> {
    Ok(match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(value) => Value::from(value),
        ValueRef::Real(value) => Value::Float(value),
        ValueRef::Text(value) => Value::from(
            std::str::from_utf8(value)
                .map_err(|_| Error::Unsupported("SQLite TEXT is not UTF-8"))?,
        ),
        // Python's v1 encoder also refuses bytes. Do not coerce a legacy BLOB
        // into a string, an amount, or an authority fact.
        ValueRef::Blob(_) => {
            return Err(Error::Unsupported(
                "custody snapshot v1 cannot serialize SQLite BLOB",
            ))
        }
    })
}
fn query(conn: &Connection, sql: &str, parameters: impl Params) -> Result<Vec<Value>> {
    let mut statement = conn.prepare_cached(sql)?;
    let mut seen = BTreeSet::new();
    let columns: Vec<_> = statement
        .column_names()
        .into_iter()
        .enumerate()
        .filter(|(_, name)| seen.insert(*name))
        .map(|(index, name)| (index, Key::from(name)))
        .collect();
    let mut cursor = statement.query(parameters)?;
    let mut values = vec![];
    while let Some(row) = cursor.next()? {
        // sqlite3.Row's name lookup uses the first duplicate column. Keep its
        // insertion position, just as dict(row) does.
        let mut record = Vec::with_capacity(columns.len());
        for (index, key) in &columns {
            record.push((key.clone(), sql_value(row.get_ref(*index)?)?));
        }
        values.push(Value::Dict(record));
    }
    Ok(values)
}
fn table_exists(conn: &Connection, name: &str) -> Result<bool> {
    Ok(!query(
        conn,
        "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?",
        [name],
    )?
    .is_empty())
}
fn operational(error: &Error) -> bool {
    matches!(error, Error::Database(rusqlite::Error::SqliteFailure(code,_) | rusqlite::Error::SqlInputError{error:code,..}) if code.code == rusqlite::ErrorCode::Unknown)
}

fn safe_row(mut row: Value, configs: &mut HashMap<String, (String, bool)>) -> Result<Value> {
    for key in ["config_json", "wallet_config_json"] {
        if matches!(&row,Value::Dict(values) if values.iter().any(|(k,_)| matches!(k,Key::Str(k) if k.is(key))))
        {
            let raw = get(&row, key);
            let cache_key = kassiber_pyjson::dumps(raw, &Options::new()).map_err(Error::Json)?;
            let (normalized, mentions) = if let Some(value) = configs.get(&cache_key) {
                value.clone()
            } else {
                let value = match raw {
                    Value::Str(_) => parsed(raw),
                    _ => raw.clone(),
                };
                let mapping = matches!(&value, Value::Dict(_));
                let mut facts = if let Value::Dict(values) = value {
                    Value::Dict(values.into_iter().filter(|(key,_)| matches!(key,Key::Str(key) if ["chain","network","samourai","chain_instance_id","bitcoin_network","chain_network"].iter().any(|name|key.is(name)))).collect())
                } else {
                    dict([])
                };
                let block = get(&facts, "samourai");
                if !matches!(block, Value::Null)
                    || matches!(&facts,Value::Dict(v) if v.iter().any(|(k,_)|matches!(k,Key::Str(k) if k.is("samourai"))))
                {
                    let sanitized = if let Value::Dict(_) = block {
                        Value::Dict(
                            ["role", "group_id", "section"]
                                .into_iter()
                                .filter_map(|key| {
                                    let Value::Dict(values) = block else {
                                        unreachable!()
                                    };
                                    values
                                        .iter()
                                        .find(|(k, _)| matches!(k,Key::Str(k) if k.is(key)))
                                        .map(|(_, value)| (Key::from(key), value.clone()))
                                })
                                .collect(),
                        )
                    } else {
                        Value::Null
                    };
                    set(&mut facts, "samourai", sanitized);
                }
                let normalized =
                    kassiber_pyjson::dumps(&facts, &Options::new().ensure_ascii(false))
                        .map_err(Error::Json)?;
                let result = (
                    normalized,
                    mapping && text(raw).to_lowercase().contains("samourai"),
                );
                configs.insert(cache_key, result.clone());
                result
            };
            set(&mut row, key, Value::from(normalized));
            if key == "config_json" {
                set(
                    &mut row,
                    "config_text_mentions_samourai",
                    Value::Bool(mentions),
                );
            }
        }
    }
    Ok(row)
}
fn safe_rows(
    rows: Vec<Value>,
    configs: &mut HashMap<String, (String, bool)>,
) -> Result<Vec<Value>> {
    rows.into_iter().map(|row| safe_row(row, configs)).collect()
}

const TRANSACTIONS: &str = "SELECT t.*,w.label AS wallet_label,w.kind AS wallet_kind,w.account_id AS wallet_account_id,w.config_json AS config_json,observation.authority_version AS observation_authority_version,observation.observer_kinds_json AS observation_observer_kinds_json,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision,observation.observed_at AS observation_observed_at,COALESCE(a.code,'treasury') AS account_code,COALESCE(a.label,'Treasury') AS account_label FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN accounts a ON a.id=w.account_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.profile_id=? AND t.excluded=0 ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC";

/// Load using the connection of an already opened `kassiber_store::Book`.
/// The caller supplies the exact profile mapping (including its key order),
/// and the journal's injected `as_of`. It retains rollback/commit ownership.
pub fn load(conn: &Connection, profile: Value, as_of: &str) -> Result<LoadedSnapshot> {
    struct Memos;
    impl Drop for Memos {
        fn drop(&mut self) {
            kassiber_custody::journal_support::clear_memos();
        }
    }
    let _memos = Memos;
    // sqlite3_txn_state distinguishes BEGIN DEFERRED/read from a write lock.
    if conn.is_autocommit()
        || conn.transaction_state(Some("main"))? != rusqlite::TransactionState::Write
    {
        return Err(Error::TransactionRequired);
    }
    let profile_id = text(get(&profile, "id"));
    let mut configs = HashMap::new();
    let network = network_facts(conn, &profile_id, &mut configs)?;
    let rows = safe_rows(query(conn, TRANSACTIONS, [&profile_id])?, &mut configs)?;
    let wallets=safe_rows(query(conn,"SELECT w.id,w.label,w.kind,w.config_json,w.account_id AS wallet_account_id,COALESCE(a.code,'treasury') AS account_code,COALESCE(a.label,'Treasury') AS account_label FROM wallets w LEFT JOIN accounts a ON a.id=w.account_id WHERE w.profile_id=?",[&profile_id])?,&mut configs)?;
    let (owned_index, warnings) = ownership::load(conn, &profile_id, &rows)?;
    let mut components = vec![];
    let mut anchor_ids = BTreeSet::new();
    let mut wallet_ids = BTreeSet::new();
    let mut revision_ids = BTreeSet::new();
    for header in query(conn,"SELECT * FROM custody_components WHERE profile_id=? AND state='active' ORDER BY created_at,revision,id",[&profile_id])? {
        let component_id=text(get(&header,"id"));
        let mut component=dict([("header",header.clone())]);
        for (key,table) in [("legs","custody_component_legs"),("allocations","custody_component_allocations"),("economic_terms","custody_component_economic_terms"),("commitments","custody_component_evidence_commitments")] {
            let mut sql=format!("SELECT * FROM {table} WHERE component_id=?");
            let mut parameters=vec![component_id.as_str()];
            if key == "commitments" { sql.push_str(" AND profile_id=?"); parameters.push(&profile_id); }
            sql.push_str(" ORDER BY ordinal,id");
            let values=query(conn,&sql,rusqlite::params_from_iter(parameters))?;
            if key == "legs" {
                for leg in &values {
                    for key in ["transaction_id","anchor_transaction_id"] { if get(leg,key) != &Value::Null { anchor_ids.insert(text(get(leg,key))); } }
                    if get(leg,"wallet_id") != &Value::Null { wallet_ids.insert(text(get(leg,"wallet_id"))); }
                }
            }
            set(&mut component,key,list(values));
        }
        for key in ["supersedes_component_id","superseded_by_component_id"] { if get(&header,key) != &Value::Null { revision_ids.insert(text(get(&header,key))); } }
        components.push(component);
    }
    let mut anchors = dict([]);
    for id in anchor_ids {
        let row=query(conn,"SELECT t.*,w.kind AS wallet_kind,w.config_json,observation.authority_version AS observation_authority_version,observation.graph_hash AS observation_graph_hash,observation.quantity_hash AS observation_quantity_hash,observation.fee_attribution AS observation_fee_attribution,observation.application_revision AS observation_application_revision FROM transactions t JOIN wallets w ON w.id=t.wallet_id LEFT JOIN chain_observation_provenance observation ON observation.transaction_id=t.id WHERE t.id=?",[&id])?.into_iter().next().map(|row|safe_row(row,&mut configs)).transpose()?.unwrap_or(Value::Null);
        set(&mut anchors, &id, row);
    }
    let mut anchor_wallets = vec![];
    for id in wallet_ids {
        anchor_wallets.extend(safe_rows(
            query(
                conn,
                "SELECT id,kind,config_json FROM wallets WHERE id=?",
                [&id],
            )?,
            &mut configs,
        )?);
    }
    let mut revision_headers = vec![];
    for id in revision_ids {
        revision_headers.extend(query(
            conn,
            "SELECT * FROM custody_components WHERE id=?",
            [&id],
        )?);
    }
    let channel_records=safe_rows(query(conn,"SELECT r.txid,r.outpoint,r.tag,r.wallet_id,r.channel_id,r.amount_msat,r.raw_json,w.config_json,b.chain,b.network FROM lightning_node_records r JOIN wallets w ON w.id=r.wallet_id LEFT JOIN backends b ON b.name=r.backend_name WHERE r.profile_id=? AND r.record_type='channel'",[&profile_id])?,&mut configs)?;
    let worklist = worklist::load(conn, &profile_id, &mut configs)?;
    let rates=query(conn,"SELECT asset,fiat_rate,fiat_value,fiat_rate_exact,fiat_value_exact,amount FROM transactions WHERE profile_id=? AND excluded=0 ORDER BY occurred_at DESC,created_at DESC,id DESC",[&profile_id]).or_else(|error|if operational(&error) { query(conn,"SELECT asset,fiat_rate,fiat_value,amount FROM transactions WHERE profile_id=? AND excluded=0 ORDER BY occurred_at DESC,created_at DESC,id DESC",[&profile_id]) } else { Err(error) })?;
    let reviews=query(conn,"SELECT r.* FROM custody_gap_reviews r JOIN (SELECT gap_id,MAX(revision) AS revision FROM custody_gap_reviews WHERE profile_id=? GROUP BY gap_id) latest ON latest.gap_id=r.gap_id AND latest.revision=r.revision WHERE r.profile_id=? ORDER BY r.gap_id,r.created_at,r.id",[&profile_id,&profile_id]).or_else(|error|if operational(&error) { Ok(vec![]) } else { Err(error) })?;
    let sources = receipts::load(conn, &profile_id, &rows)?;
    Ok(LoadedSnapshot{facts:dict([
        ("schema_version",Value::from(1)),("operation",Value::from("custody")),("as_of",Value::from(as_of)),
        ("profile",profile),("book_network",network),("wallets",list(wallets)),("transactions",list(rows)),("rate_rows",list(rates)),("owned_index",owned_index),("ownership_warnings",list(warnings)),("components",list(components)),("anchors",anchors),("anchor_wallets",list(anchor_wallets)),("revision_headers",list(revision_headers)),("receipt_sources",list(sources)),("channel_records",list(channel_records)),
        ("loan_legs",list(query(conn,"SELECT transaction_id,role FROM loan_legs WHERE profile_id=? AND deleted_at IS NULL",[&profile_id])?)),
        ("evidence_snapshots",list(query(conn,"SELECT subject_id,detail_hash,quantity_hash,payload_json,created_at FROM custody_authored_evidence_snapshots WHERE profile_id=? AND subject_kind='custody_component' ORDER BY subject_id,quantity_hash,detail_hash",[&profile_id])?)),
        ("migration_issues",list(query(conn,"SELECT * FROM custody_authored_migration_issues WHERE profile_id=? AND resolved_at IS NULL ORDER BY created_at,id",[&profile_id])?)),
        ("gap_reviews",list(reviews)),("pair_dismissals",list(query(conn,"SELECT * FROM transaction_pair_dismissals WHERE profile_id=?",[&profile_id])?)),("gap_worklist",worklist),
    ])})
}

fn network_facts(
    conn: &Connection,
    profile_id: &str,
    configs: &mut HashMap<String, (String, bool)>,
) -> Result<Value> {
    if !table_exists(conn, "book_network_bindings")? {
        return Ok(Value::Null);
    }
    let binding = query(
        conn,
        "SELECT * FROM book_network_bindings WHERE profile_id=?",
        [profile_id],
    )?
    .into_iter()
    .next()
    .unwrap_or(Value::Null);
    let observations=safe_rows(query(conn,"SELECT t.id,t.wallet_id,t.asset,t.raw_json,w.kind AS wallet_kind,w.config_json AS wallet_config_json FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE t.profile_id=? ORDER BY t.id",[profile_id])?,configs)?;
    let mut references = vec![];
    for table in [
        "chain_analysis_observations",
        "chain_observer_instances",
        "chain_observation_provenance",
        "wallet_policy_epochs",
        "wallet_utxos",
    ] {
        if !table_exists(conn, table)? {
            continue;
        }
        let columns = query(conn, &format!("PRAGMA table_info(\"{table}\")"), [])?;
        if !["profile_id", "chain", "network"]
            .iter()
            .all(|name| columns.iter().any(|row| text(get(row, "name")) == *name))
        {
            continue;
        }
        for source in query(
            conn,
            &format!("SELECT * FROM \"{table}\" WHERE profile_id=?"),
            [profile_id],
        )? {
            let mut item = dict([
                ("table", Value::from(table)),
                ("chain", get(&source, "chain").clone()),
                ("network", get(&source, "network").clone()),
                ("payload_json", get(&source, "payload_json").clone()),
            ]);
            if table == "wallet_policy_epochs" {
                // book_network._json keeps only mappings, including {}.
                let private = parsed(get(&source, "private_material_json"));
                set(
                    &mut item,
                    "private_material_present",
                    Value::Bool(matches!(&private, Value::Dict(_)) && truth(&private)),
                );
                set(
                    &mut item,
                    "source_present",
                    Value::Bool(
                        !query(
                            conn,
                            "SELECT 1 FROM wallet_policy_sources WHERE epoch_id=? LIMIT 1",
                            [text(get(&source, "id"))],
                        )?
                        .is_empty(),
                    ),
                );
            }
            references.push(item);
        }
    }
    Ok(dict([
        ("binding", binding),
        ("observations", list(observations)),
        ("references", list(references)),
    ]))
}
