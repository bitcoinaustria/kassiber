use super::ingestion::ObservationBuilder;
use super::value::{json, Value};
use crate::{database_error, publication::row::rows};
use kassiber_envelope::AppError;
use kassiber_pyjson::Options;
use kassiber_store::rusqlite::{params, params_from_iter, types::Value as Sql, Connection, Params};
use std::collections::{BTreeMap, BTreeSet};

#[path = "durable_schema.rs"]
mod schema;

pub const VERSION: i64 = 4;
pub const PHYSICAL: [&str; 5] = [
    "transactions",
    "transaction_graph_cache",
    "wallet_utxos",
    "chain_analysis_observations",
    "chain_analysis_reference_assertions",
];
pub const OVERLAYS: [&str; 9] = [
    "wallets",
    "profiles",
    "journal_custody_decisions",
    "journal_custody_economic_relations",
    "chain_analysis_labels",
    "chain_analysis_datasets",
    "chain_analysis_dataset_claims",
    "book_network_bindings",
    "chain_analysis_acquisition_grants",
];

fn write(
    conn: &Connection,
    sql: &str,
    params: impl Params,
) -> kassiber_store::rusqlite::Result<usize> {
    let result = conn.execute(sql, params)?;
    #[cfg(any(test, feature = "test-harness"))]
    fixture::after_write(sql);
    Ok(result)
}

fn read(conn: &Connection, sql: &str, params: impl Params) -> Result<Vec<Value>, AppError> {
    Ok(rows(conn, sql, params)?
        .iter()
        .map(Value::from_py)
        .collect())
}

fn names(conn: &Connection, sql: &str) -> Result<BTreeSet<String>, AppError> {
    conn.prepare(sql)
        .map_err(database_error)?
        .query_map([], |row| row.get(0))
        .map_err(database_error)?
        .collect::<Result<_, _>>()
        .map_err(database_error)
}

fn encoded(value: &Value) -> Result<String, AppError> {
    kassiber_pyjson::dumps(
        &value.py(),
        &Options::new().sort_keys(true).compact().allow_nan(false),
    )
    .map_err(|_| super::invalid())
}

fn parse(value: &Value) -> Result<Value, AppError> {
    kassiber_pytext::loads(value.as_str().ok_or_else(super::invalid)?)
        .map(|v| Value::from_py(&v))
        .map_err(|_| super::invalid())
}

pub fn install(conn: &Connection) -> Result<BTreeSet<String>, AppError> {
    let objects = names(
        conn,
        "SELECT name FROM sqlite_master WHERE type IN ('table','trigger')",
    )?;
    let required: BTreeSet<_> = schema::DDL
        .iter()
        .filter(|sql| sql.starts_with("CREATE TABLE"))
        .map(|sql| {
            sql.split_whitespace()
                .nth(5)
                .unwrap()
                .split('(')
                .next()
                .unwrap()
                .to_owned()
        })
        .collect();
    if !required.is_subset(&objects) {
        for sql in schema::DDL {
            let sql = sql
                .replace(
                    "profile_id TEXT NOT NULL",
                    "profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE",
                )
                .replace(
                    "profile_id TEXT PRIMARY KEY",
                    "profile_id TEXT PRIMARY KEY REFERENCES profiles(id) ON DELETE CASCADE",
                );
            write(conn, &sql, []).map_err(database_error)?;
        }
        write(conn, "UPDATE chain_index_state SET version=0", []).map_err(database_error)?;
    }
    write(conn, "CREATE INDEX IF NOT EXISTS idx_chain_index_label_subject ON chain_index_labels(profile_id,json_extract(payload_json,'$.subject'))", []).map_err(database_error)?;
    if objects.contains("chain_analysis_dataset_claims") {
        for field in ["valid_from", "valid_until"] {
            write(conn, &format!("CREATE INDEX IF NOT EXISTS idx_chain_index_claim_{field} ON chain_analysis_dataset_claims({field},dataset_id) WHERE {field} IS NOT NULL"), []).map_err(database_error)?;
        }
    }
    let mut tables = BTreeSet::new();
    for table in PHYSICAL.into_iter().chain(OVERLAYS) {
        if !objects.contains(table) {
            continue;
        }
        tables.insert(table.to_owned());
        let mut added = false;
        for (operation, prefix) in [("INSERT", "NEW"), ("UPDATE", "NEW"), ("DELETE", "OLD")] {
            let name = format!("chain_index_track_{table}_{}", operation.to_lowercase());
            if objects.contains(&name) {
                continue;
            }
            let keys = if PHYSICAL.contains(&table) {
                if operation == "UPDATE" {
                    vec!["OLD.rowid".to_owned(), "NEW.rowid".to_owned()]
                } else {
                    vec![format!("{prefix}.rowid")]
                }
            } else {
                vec!["-1".to_owned()]
            };
            let writes = keys.into_iter().map(|key| format!("INSERT INTO chain_index_dirty VALUES('{table}',{key},(SELECT revision FROM chain_index_clock WHERE id=1)) ON CONFLICT(source_table,source_key) DO UPDATE SET revision=excluded.revision;")).collect::<String>();
            let when = if table == "chain_analysis_acquisition_grants" && operation == "UPDATE" {
                let columns: BTreeSet<_> = read(conn, &format!("PRAGMA table_info({table})"), [])?
                    .into_iter()
                    .map(|row| text(&row["name"]))
                    .collect();
                let fields = [
                    "verified_tip_height",
                    "verified_tip_hash",
                    "verified_tip_at",
                    "last_code",
                    "status",
                ]
                .into_iter()
                .filter(|field| columns.contains(*field))
                .map(|field| format!("OLD.{field} IS NOT NEW.{field}"))
                .collect::<Vec<_>>();
                format!(" WHEN {}", fields.join(" OR "))
            } else {
                String::new()
            };
            write(conn, &format!("CREATE TRIGGER {name} AFTER {operation} ON {table}{when} BEGIN UPDATE chain_index_clock SET revision=revision+1 WHERE id=1; {writes} END"), []).map_err(database_error)?;
            added = true;
        }
        if added {
            write(
                conn,
                "UPDATE chain_index_clock SET revision=revision+1 WHERE id=1",
                [],
            )
            .map_err(database_error)?;
            write(conn, "INSERT OR REPLACE INTO chain_index_dirty SELECT ?,-1,revision FROM chain_index_clock WHERE id=1", [table]).map_err(database_error)?;
        }
    }
    Ok(tables)
}

fn integer(value: &Value) -> Result<i64, AppError> {
    if let Value::Int(v) = value {
        i64::try_from(v).map_err(|_| super::invalid())
    } else {
        Err(super::invalid())
    }
}

fn text(value: &Value) -> String {
    kassiber_envelope::py_str(&value.py())
}

fn apply(builder: &mut ObservationBuilder<'_>, table: &str, row: &Value) -> Result<(), AppError> {
    match table {
        "transactions" => builder.transaction(row, false),
        "wallet_utxos" => builder.inventory(row),
        _ => builder.reference(row, table != "transaction_graph_cache"),
    }
}

pub fn replace_source(
    conn: &Connection,
    profile: &str,
    table: &str,
    key: i64,
) -> Result<BTreeSet<String>, AppError> {
    if !PHYSICAL.contains(&table) {
        return Err(super::invalid());
    }
    let rows = read(
        conn,
        &format!("SELECT rowid AS _source_key,* FROM {table} WHERE rowid=?"),
        [key],
    )?;
    contribute(conn, profile, table, key, rows.into_iter().next())
}

pub fn contributing_sources(
    conn: &Connection,
    profile: &str,
    nodes: Option<&BTreeSet<String>>,
) -> Result<kassiber_pyjson::Value, AppError> {
    Ok(kassiber_pyjson::Value::List(
        contributors(conn, profile, nodes)?
            .iter()
            .map(|((table, key), row)| {
                kassiber_pyjson::Value::List(vec![
                    PHYSICAL[*table].into(),
                    kassiber_pyjson::Value::Int((*key).into()),
                    row.py(),
                ])
            })
            .collect(),
    ))
}

fn contribute(
    conn: &Connection,
    profile: &str,
    table: &str,
    key: i64,
    row: Option<Value>,
) -> Result<BTreeSet<String>, AppError> {
    let mut old: BTreeSet<String> = read(conn, "SELECT node_id FROM chain_index_source_nodes WHERE profile_id=? AND source_table=? AND source_key=?", params![profile,table,key])?.iter().map(|row| text(&row["node_id"])).collect();
    write(conn, "DELETE FROM chain_index_source_nodes WHERE profile_id=? AND source_table=? AND source_key=?", params![profile,table,key]).map_err(database_error)?;
    write(
        conn,
        "DELETE FROM chain_index_sources WHERE profile_id=? AND source_table=? AND source_key=?",
        params![profile, table, key],
    )
    .map_err(database_error)?;
    let Some(mut row) = row.filter(|row| {
        (table == "transaction_graph_cache" || row["profile_id"] == profile)
            && (table != "chain_analysis_reference_assertions" || row["active"] == 1)
    }) else {
        return Ok(old);
    };
    if table == "transactions" {
        let wallets = read(
            conn,
            "SELECT * FROM wallets WHERE id=?",
            [text(&row["wallet_id"])],
        )?;
        let wallet = wallets.first().unwrap_or(&Value::Null);
        row["wallet_kind"] = wallet["kind"].clone();
        row["wallet_config_json"] = wallet["config_json"].clone();
    }
    let mut builder = ObservationBuilder::read(conn, profile)?;
    apply(&mut builder, table, &row)?;
    write(
        conn,
        "INSERT INTO chain_index_sources VALUES(?,?,?,?,?,?)",
        params![
            profile,
            table,
            key,
            encoded(&row)?,
            integer(&builder.coverage["invalid_observations"])?,
            integer(&builder.coverage["cache_rejected"])?
        ],
    )
    .map_err(database_error)?;
    for node in builder.nodes.keys() {
        write(
            conn,
            "INSERT INTO chain_index_source_nodes VALUES(?,?,?,?)",
            params![profile, table, key, node],
        )
        .map_err(database_error)?;
        old.insert(node.clone());
    }
    Ok(old)
}

type Contributions = BTreeMap<(usize, i64), Value>;

fn contributors(
    conn: &Connection,
    profile: &str,
    nodes: Option<&BTreeSet<String>>,
) -> Result<Contributions, AppError> {
    let mut result = BTreeMap::new();
    let mut extend = |rows: Vec<Value>| -> Result<(), AppError> {
        for row in rows {
            let table = text(&row["source_table"]);
            let position = PHYSICAL
                .iter()
                .position(|name| *name == table)
                .ok_or_else(super::invalid)?;
            result.insert(
                (position, integer(&row["source_key"])?),
                parse(&row["payload_json"])?,
            );
        }
        Ok(())
    };
    if let Some(nodes) = nodes {
        for chunk in nodes.iter().collect::<Vec<_>>().chunks(200) {
            let marks = vec!["?"; chunk.len()].join(",");
            let params = std::iter::once(Sql::Text(profile.to_owned()))
                .chain(chunk.iter().map(|v| Sql::Text((*v).clone())));
            extend(read(conn, &format!("SELECT s.source_table,s.source_key,s.payload_json FROM chain_index_source_nodes n JOIN chain_index_sources s USING(profile_id,source_table,source_key) WHERE n.profile_id=? AND n.node_id IN ({marks})"), params_from_iter(params))?)?;
        }
    } else {
        extend(read(conn, "SELECT source_table,source_key,payload_json FROM chain_index_sources WHERE profile_id=?", [profile])?)?;
    }
    Ok(result)
}

#[cfg(any(test, feature = "test-harness"))]
pub mod fixture;

mod synchronize;
pub use synchronize::{synchronize, synchronize_with_sources};
