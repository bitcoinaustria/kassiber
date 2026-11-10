//! `core/book_reset.py`: keep a book's decisions across "Reset book data"
//! and bind them again as the resync brings their transactions back.
//!
//! The reset records each deleted transaction's stable identity and parks
//! every decision row naming one, verbatim. The resync gives a re-observed
//! row its old id back (exact fingerprint, else the only matching wallet,
//! external id, asset and direction), so every reference keyed by transaction
//! id lines up again. A decision whose transactions are missing or changed
//! stays parked and holds the returned rows (`book_reset_decision_held`).
//! Every stored text is byte-identical to Python's.
use super::*;
use kassiber_pyjson::{Key, Options};
use kassiber_store::rusqlite::types::Value as Sql;
use std::collections::{BTreeMap, BTreeSet};

pub const HOLD_REASON: &str = "book_reset_decision_held";

const TX: &str = "SELECT id FROM transactions WHERE profile_id = ?";

/// `PARKED_TABLES`: parents first, with the columns naming a transaction.
pub const PARKED_TABLES: &[(&str, &str, &[&str])] = &[
    ("transaction_tags", "transaction_id IN ({TX})", &["transaction_id"]),
    ("loan_legs", "transaction_id IN ({TX})", &["transaction_id"]),
    (
        "transaction_pairs",
        "out_transaction_id IN ({TX}) OR in_transaction_id IN ({TX})",
        &["out_transaction_id", "in_transaction_id"],
    ),
    (
        "transaction_pair_dismissals",
        "out_transaction_id IN ({TX}) OR in_transaction_id IN ({TX})",
        &["out_transaction_id", "in_transaction_id"],
    ),
    ("direct_swap_payouts", "out_transaction_id IN ({TX})", &["out_transaction_id"]),
    (
        "custody_component_transaction_memberships",
        "transaction_id IN ({TX})",
        &["transaction_id"],
    ),
    ("commercial_links", "transaction_id IN ({TX})", &["transaction_id"]),
    ("attachments", "transaction_id IN ({TX})", &["transaction_id"]),
    (
        "external_document_attachments",
        "attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({TX}))",
        &[],
    ),
    (
        "source_funds_links",
        "from_transaction_id IN ({TX}) OR to_transaction_id IN ({TX})",
        &["from_transaction_id", "to_transaction_id"],
    ),
    (
        "source_funds_link_attachments",
        "link_id IN (SELECT id FROM source_funds_links WHERE from_transaction_id IN ({TX}) OR to_transaction_id IN ({TX})) OR attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({TX}))",
        &[],
    ),
    (
        "source_funds_source_attachments",
        "attachment_id IN (SELECT id FROM attachments WHERE transaction_id IN ({TX}))",
        &[],
    ),
    ("source_funds_cases", "target_transaction_id IN ({TX})", &["target_transaction_id"]),
    (
        "source_funds_snapshots",
        "case_id IN (SELECT id FROM source_funds_cases WHERE target_transaction_id IN ({TX}))",
        &[],
    ),
    ("transaction_edit_events", "transaction_id IN ({TX})", &["transaction_id"]),
    (
        "transaction_edit_fields",
        "event_id IN (SELECT id FROM transaction_edit_events WHERE transaction_id IN ({TX}))",
        &[],
    ),
];
/// `_GROUPED_TABLES`: a component's memberships bind only all together.
const GROUPED: (&str, &str) = ("custody_component_transaction_memberships", "component_id");
pub const DERIVED_TRANSACTION_TABLES: &[&str] = &[
    "chain_observation_provenance",
    "import_batch_transactions",
    "journal_custody_decisions",
    "journal_custody_economic_relations",
    "journal_entries",
    "journal_quantity_postings",
    "journal_quarantines",
];
pub const REANCHORED_TRANSACTION_TABLES: &[&str] = &["custody_component_legs"];
const METADATA_COLUMNS: &[&str] = &[
    "note",
    "review_status",
    "taxability_override",
    "at_regime_override",
    "at_category_override",
    "kind_override",
    "excluded",
    "pricing_review_accepted_price",
    "commercial_applied_link_id",
];
const PRICING_COLUMNS: &[&str] = &[
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
];
const PRICING_EDIT_FIELDS: &[&str] = &[
    "fiat_currency",
    "fiat_rate",
    "fiat_value",
    "fiat_price_source",
    "pricing_source_kind",
    "pricing_provider",
    "pricing_pair",
    "pricing_timestamp",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_external_ref",
    "pricing_quality",
];
const FACT_COLUMNS: &[&str] = &["direction", "asset", "amount", "fee", "amount_includes_fee"];

fn predicate(template: &str) -> String {
    template.replace("{TX}", TX)
}
fn repeated(sql: &str, profile_id: &str) -> Vec<String> {
    vec![profile_id.to_owned(); sql.matches('?').count()]
}
fn dumps(value: &Value, sort_keys: bool) -> Result<String> {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(sort_keys))
        .map_err(|_| AppError::new("app_error", "Kept reset record could not be encoded"))
}
fn loads(text: &str) -> Result<Value> {
    kassiber_pytext::loads(text)
        .map_err(|_| AppError::new("app_error", "Kept reset record could not be decoded"))
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Option<Vec<u8>> {
    text.len()
        .is_multiple_of(2)
        .then(|| {
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
                .collect()
        })
        .flatten()
}
/// A stored SQL value as Python's sqlite3 returns it, BLOBs as `{"$hex"}`.
fn from_sql(value: &Sql) -> Value {
    match value {
        Sql::Null => Value::Null,
        Sql::Integer(n) => Value::Int((*n).into()),
        Sql::Real(n) => Value::Float(*n),
        Sql::Text(t) => s(t.as_str()),
        Sql::Blob(b) => dict([("$hex", s(hex(b)))]),
    }
}
fn to_sql(value: &Value) -> Result<Sql> {
    Ok(match value {
        Value::Null => Sql::Null,
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Int(n) => Sql::Integer(
            i64::try_from(n.clone())
                .map_err(|_| AppError::new("app_error", "Kept reset value is out of range"))?,
        ),
        Value::Float(n) => Sql::Real(*n),
        Value::Str(t) => Sql::Text(py_str(&Value::Str(t.clone()))),
        Value::Dict(_) => match get(value, "$hex") {
            Some(Value::Str(t)) => Sql::Blob(
                unhex(&py_str(&Value::Str(t.clone())))
                    .ok_or_else(|| AppError::new("app_error", "Kept reset blob is invalid"))?,
            ),
            _ => return Err(AppError::new("app_error", "Kept reset value is invalid")),
        },
        _ => return Err(AppError::new("app_error", "Kept reset value is invalid")),
    })
}
/// Python's `==` between two stored values (`1 == 1.0`).
fn same(a: &Sql, b: &Sql) -> bool {
    match (a, b) {
        (Sql::Integer(x), Sql::Real(y)) | (Sql::Real(y), Sql::Integer(x)) => (*x as f64) == *y,
        _ => a == b,
    }
}
/// `SELECT *` in order, as (columns, rows of raw SQL values).
fn raw_rows(
    conn: &Connection,
    sql: &str,
    params: impl rusqlite::Params,
) -> Result<(Vec<String>, Vec<Vec<Sql>>)> {
    let mut stmt = conn.prepare(sql).map_err(sql_error)?;
    let columns: Vec<String> = stmt
        .column_names()
        .iter()
        .map(|c| (*c).to_owned())
        .collect();
    let width = columns.len();
    let rows = stmt
        .query_map(params, |r| (0..width).map(|i| r.get::<_, Sql>(i)).collect())
        .map_err(sql_error)?
        .collect::<std::result::Result<Vec<Vec<Sql>>, _>>()
        .map_err(sql_error)?;
    Ok((columns, rows))
}
fn column<'a>(columns: &[String], row: &'a [Sql], name: &str) -> &'a Sql {
    columns
        .iter()
        .position(|c| c == name)
        .map(|i| &row[i])
        .unwrap_or(&Sql::Null)
}
fn sql_text(value: &Sql) -> String {
    match value {
        Sql::Text(t) => t.clone(),
        Sql::Integer(n) => n.to_string(),
        Sql::Real(n) => kassiber_pyjson::float_repr(*n),
        Sql::Null => "None".into(),
        Sql::Blob(_) => String::new(),
    }
}

fn metadata(conn: &Connection, columns: &[String], row: &[Sql]) -> Result<Value> {
    let mut kept: Vec<(String, Value)> = Vec::new();
    for name in METADATA_COLUMNS {
        let value = column(columns, row, name);
        let zero_excluded = *name == "excluded" && same(value, &Sql::Integer(0));
        if !matches!(value, Sql::Null) && !zero_excluded {
            kept.push(((*name).to_owned(), from_sql(value)));
        }
    }
    let field = |name: &str| column(columns, row, name);
    let edited = {
        let placeholders = vec!["?"; PRICING_EDIT_FIELDS.len()].join(",");
        let mut params = vec![Sql::Text(sql_text(field("id")))];
        params.extend(PRICING_EDIT_FIELDS.iter().map(|f| Sql::Text((*f).into())));
        conn.query_row(
            &format!("SELECT 1 FROM transaction_edit_fields f JOIN transaction_edit_events e ON e.id = f.event_id WHERE e.transaction_id = ? AND f.field IN ({placeholders}) LIMIT 1"),
            rusqlite::params_from_iter(params),
            |_| Ok(()),
        )
        .optional()
        .map_err(sql_error)?
        .is_some()
    };
    let priced_by_user = same(
        field("pricing_source_kind"),
        &Sql::Text("manual_override".into()),
    ) || same(field("fiat_price_source"), &Sql::Text("manual".into()))
        || !matches!(field("commercial_applied_link_id"), Sql::Null)
        || edited;
    if priced_by_user {
        for name in PRICING_COLUMNS {
            let value = from_sql(field(name));
            match kept.iter_mut().find(|(k, _)| k == name) {
                Some((_, slot)) => *slot = value,
                None => kept.push(((*name).to_owned(), value)),
            }
        }
    }
    Ok(dict(kept))
}

/// `park_profile`: runs inside the reset's write transaction.
pub fn park_profile(
    conn: &Connection,
    workspace_id: &str,
    profile_id: &str,
    timestamp: &str,
) -> Result<Vec<(String, i64)>> {
    let mut kept = Vec::new();
    for (table, template, transaction_columns) in PARKED_TABLES {
        let where_ = predicate(template);
        let (columns, rows) = raw_rows(
            conn,
            &format!("SELECT * FROM {table} WHERE {where_} ORDER BY rowid"),
            rusqlite::params_from_iter(repeated(&where_, profile_id)),
        )?;
        let mut groups: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let grouped = *table == GROUPED.0;
        let push = |ids: &mut Vec<String>, row: &[Sql]| {
            for name in *transaction_columns {
                let value = column(&columns, row, name);
                if !matches!(value, Sql::Null) && !ids.contains(&sql_text(value)) {
                    ids.push(sql_text(value));
                }
            }
        };
        if grouped {
            for row in &rows {
                let key = sql_text(column(&columns, row, GROUPED.1));
                push(groups.entry(key).or_default(), row);
            }
        }
        for row in &rows {
            let mut ids = if grouped {
                groups[&sql_text(column(&columns, row, GROUPED.1))].clone()
            } else {
                Vec::new()
            };
            push(&mut ids, row);
            let encoded = dict(
                columns
                    .iter()
                    .zip(row)
                    .map(|(c, v)| (c.as_str(), from_sql(v))),
            );
            let ids_json = dumps(
                &Value::List(ids.iter().map(|i| s(i.as_str())).collect()),
                false,
            )?;
            conn.execute(
                "INSERT INTO book_reset_parked_rows(workspace_id, profile_id, table_name, row_json, transaction_ids_json, created_at) VALUES(?, ?, ?, ?, ?, ?)",
                [workspace_id, profile_id, table, &dumps(&encoded, false)?, &ids_json, timestamp],
            )
            .map_err(sql_error)?;
        }
        kept.push(((*table).to_owned(), rows.len() as i64));
    }
    let (columns, rows) = raw_rows(
        conn,
        "SELECT * FROM transactions WHERE profile_id = ? ORDER BY rowid",
        [profile_id],
    )?;
    let mut with_metadata = 0;
    for row in &rows {
        let kept_fields = metadata(conn, &columns, row)?;
        if matches!(&kept_fields, Value::Dict(f) if !f.is_empty()) {
            with_metadata += 1;
        }
        let field = |name: &str| column(&columns, row, name).clone();
        conn.execute(
            "INSERT OR REPLACE INTO book_reset_transactions(transaction_id, workspace_id, profile_id, wallet_id, fingerprint, external_id, occurred_at, direction, asset, amount, fee, amount_includes_fee, metadata_json, state, changed_fields_json, created_at) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 'pending', '[]', ?)",
            rusqlite::params_from_iter([
                field("id"),
                field("workspace_id"),
                Sql::Text(profile_id.into()),
                field("wallet_id"),
                field("fingerprint"),
                field("external_id"),
                field("occurred_at"),
                field("direction"),
                field("asset"),
                field("amount"),
                field("fee"),
                field("amount_includes_fee"),
                Sql::Text(dumps(&kept_fields, true)?),
                Sql::Text(timestamp.into()),
            ]),
        )
        .map_err(sql_error)?;
    }
    kept.push(("transaction_metadata".into(), with_metadata));
    Ok(kept)
}

fn count(conn: &Connection, sql: &str, profile_id: &str) -> Result<i64> {
    conn.query_row(sql, [profile_id], |r| r.get(0))
        .map_err(sql_error)
}

/// Every book this version opens has both tables; partial test and
/// historical fixture schemas may not.
fn tables_present(conn: &Connection) -> Result<bool> {
    conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name IN ('book_reset_transactions', 'book_reset_parked_rows')",
        [],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n == 2)
    .map_err(sql_error)
}

pub fn has_state(conn: &Connection, profile_id: &str) -> Result<bool> {
    if !tables_present(conn)? {
        return Ok(false);
    }
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM book_reset_transactions WHERE profile_id = ?) OR EXISTS(SELECT 1 FROM book_reset_parked_rows WHERE profile_id = ?)",
        [profile_id, profile_id],
        |r| r.get::<_, i64>(0),
    )
    .map(|n| n == 1)
    .map_err(sql_error)
}

/// `discard_profile`: the wipe-everything reset.
pub fn discard_profile(conn: &Connection, profile_id: &str) -> Result<Vec<(String, i64)>> {
    let counts = vec![
        (
            "book_reset_transactions".to_owned(),
            count(
                conn,
                "SELECT COUNT(*) FROM book_reset_transactions WHERE profile_id = ?",
                profile_id,
            )?,
        ),
        (
            "book_reset_parked_rows".to_owned(),
            count(
                conn,
                "SELECT COUNT(*) FROM book_reset_parked_rows WHERE profile_id = ?",
                profile_id,
            )?,
        ),
    ];
    conn.execute(
        "DELETE FROM book_reset_parked_rows WHERE profile_id = ?",
        [profile_id],
    )
    .map_err(sql_error)?;
    conn.execute(
        "DELETE FROM book_reset_transactions WHERE profile_id = ?",
        [profile_id],
    )
    .map_err(sql_error)?;
    Ok(counts)
}

/// `claim_transaction_id`.
pub fn claim_transaction_id(
    conn: &Connection,
    profile_id: &str,
    wallet_id: &str,
    fingerprint: &str,
    external_id: Option<&str>,
    asset: &str,
    direction: &str,
) -> Result<Option<String>> {
    let exact: Option<Sql> = conn
        .query_row(
            "SELECT transaction_id FROM book_reset_transactions WHERE profile_id = ? AND wallet_id = ? AND fingerprint = ? AND state = 'pending' AND NOT EXISTS (SELECT 1 FROM transactions t WHERE t.id = transaction_id)",
            [profile_id, wallet_id, fingerprint],
            |r| r.get(0),
        )
        .optional()
        .map_err(sql_error)?;
    if let Some(id) = exact {
        return Ok(Some(sql_text(&id)));
    }
    let Some(external_id) = external_id.filter(|e| !e.is_empty()) else {
        return Ok(None);
    };
    let (_, rows) = raw_rows(
        conn,
        "SELECT transaction_id, state FROM book_reset_transactions WHERE profile_id = ? AND wallet_id = ? AND external_id = ? AND asset = ? AND direction = ?",
        [profile_id, wallet_id, external_id, asset, direction],
    )?;
    if rows.len() != 1 || !same(&rows[0][1], &Sql::Text("pending".into())) {
        return Ok(None);
    }
    let id = sql_text(&rows[0][0]);
    let present = conn
        .query_row("SELECT 1 FROM transactions WHERE id = ?", [&id], |_| Ok(()))
        .optional()
        .map_err(sql_error)?
        .is_some();
    Ok((!present).then_some(id))
}

#[derive(Debug, PartialEq, Eq)]
pub enum Settled {
    Bound,
    BoundWithMetadata,
    Changed,
}

/// `settle_claim`.
pub fn settle_claim(conn: &Connection, transaction_id: &str) -> Result<Settled> {
    let (r_columns, reservation) = raw_rows(
        conn,
        "SELECT * FROM book_reset_transactions WHERE transaction_id = ?",
        [transaction_id],
    )?;
    let (c_columns, current) = raw_rows(
        conn,
        &format!(
            "SELECT profile_id, {} FROM transactions WHERE id = ?",
            FACT_COLUMNS.join(", ")
        ),
        [transaction_id],
    )?;
    let (reservation, current) = (&reservation[0], &current[0]);
    let changed: Vec<&str> = FACT_COLUMNS
        .iter()
        .copied()
        .filter(|name| {
            !same(
                column(&r_columns, reservation, name),
                column(&c_columns, current, name),
            )
        })
        .collect();
    if !changed.is_empty() {
        let fields = dumps(&Value::List(changed.iter().map(|f| s(*f)).collect()), false)?;
        conn.execute(
            "UPDATE book_reset_transactions SET state = 'changed', changed_fields_json = ? WHERE transaction_id = ?",
            [&fields, transaction_id],
        )
        .map_err(sql_error)?;
        return Ok(Settled::Changed);
    }
    let metadata = loads(&sql_text(column(&r_columns, reservation, "metadata_json")))?;
    let mut fields: Vec<(String, Value)> = match &metadata {
        Value::Dict(items) => items
            .iter()
            .map(|(k, v)| {
                (
                    match k {
                        Key::Str(k) => py_str(&Value::Str(k.clone())),
                        _ => String::new(),
                    },
                    v.clone(),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    fields.sort_by(|a, b| a.0.cmp(&b.0));
    if !fields.is_empty() {
        let assignments = fields
            .iter()
            .map(|(k, _)| format!("{k} = ?"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut params = fields
            .iter()
            .map(|(_, v)| to_sql(v))
            .collect::<Result<Vec<_>>>()?;
        params.push(Sql::Text(transaction_id.into()));
        conn.execute(
            &format!("UPDATE transactions SET {assignments} WHERE id = ?"),
            rusqlite::params_from_iter(params),
        )
        .map_err(sql_error)?;
    }
    conn.execute(
        "UPDATE custody_component_legs SET transaction_id = anchor_transaction_id WHERE profile_id = ? AND transaction_id IS NULL AND anchor_transaction_id = ?",
        rusqlite::params_from_iter([column(&c_columns, current, "profile_id").clone(), Sql::Text(transaction_id.into())]),
    )
    .map_err(sql_error)?;
    conn.execute(
        "DELETE FROM book_reset_transactions WHERE transaction_id = ?",
        [transaction_id],
    )
    .map_err(sql_error)?;
    Ok(if fields.is_empty() {
        Settled::Bound
    } else {
        Settled::BoundWithMetadata
    })
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum State {
    Bound,
    Changed,
    Missing,
}

fn states(conn: &Connection, ids: &[String]) -> Result<Vec<(String, State)>> {
    let exists = |sql: &str, id: &str| -> Result<bool> {
        Ok(conn
            .query_row(sql, [id], |_| Ok(()))
            .optional()
            .map_err(sql_error)?
            .is_some())
    };
    let mut out: Vec<(String, State)> = Vec::new();
    for id in ids {
        let present = exists("SELECT 1 FROM transactions WHERE id = ?", id)?;
        let reserved = exists(
            "SELECT 1 FROM book_reset_transactions WHERE transaction_id = ?",
            id,
        )?;
        let state = if !present {
            State::Missing
        } else if reserved {
            State::Changed
        } else {
            State::Bound
        };
        // A dict keyed by id keeps the first position of a repeated id.
        match out.iter_mut().find(|(k, _)| k == id) {
            Some((_, slot)) => *slot = state,
            None => out.push((id.clone(), state)),
        }
    }
    Ok(out)
}

fn id_list(text: &str) -> Result<Vec<String>> {
    Ok(match loads(text)? {
        Value::List(items) => items.iter().map(py_str).collect(),
        _ => Vec::new(),
    })
}

type ForeignKeys = Vec<(String, Vec<(String, String)>)>;

fn foreign_keys(conn: &Connection, table: &str) -> Result<ForeignKeys> {
    let (columns, rows) = raw_rows(conn, &format!("PRAGMA foreign_key_list(\"{table}\")"), [])?;
    let mut groups: BTreeMap<i64, (String, Vec<(String, String)>)> = BTreeMap::new();
    for row in &rows {
        let id = match column(&columns, row, "id") {
            Sql::Integer(n) => *n,
            _ => 0,
        };
        let parent = sql_text(column(&columns, row, "table"));
        let to = match column(&columns, row, "to") {
            Sql::Null => "id".to_owned(),
            other => sql_text(other),
        };
        groups
            .entry(id)
            .or_insert_with(|| (parent, Vec::new()))
            .1
            .push((sql_text(column(&columns, row, "from")), to));
    }
    Ok(groups.into_values().collect())
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let (columns, rows) = raw_rows(conn, &format!("PRAGMA table_info(\"{table}\")"), [])?;
    Ok(rows
        .iter()
        .map(|row| sql_text(column(&columns, row, "name")))
        .collect())
}

fn parents_present(
    conn: &Connection,
    values: &[(String, Sql)],
    keys: &ForeignKeys,
) -> Result<bool> {
    let lookup = |name: &str| values.iter().find(|(k, _)| k == name).map(|(_, v)| v);
    for (parent, pairs) in keys {
        if pairs
            .iter()
            .any(|(child, _)| lookup(child).is_none_or(|v| matches!(v, Sql::Null)))
        {
            continue;
        }
        let where_ = pairs
            .iter()
            .map(|(_, c)| format!("\"{c}\" = ?"))
            .collect::<Vec<_>>()
            .join(" AND ");
        let params: Vec<Sql> = pairs
            .iter()
            .map(|(child, _)| lookup(child).unwrap().clone())
            .collect();
        let found = conn
            .query_row(
                &format!("SELECT 1 FROM \"{parent}\" WHERE {where_} LIMIT 1"),
                rusqlite::params_from_iter(params),
                |_| Ok(()),
            )
            .optional()
            .map_err(sql_error)?
            .is_some();
        if !found {
            return Ok(false);
        }
    }
    Ok(true)
}

/// `rebind_parked`: restore every parked row whose transactions and parents are back.
pub fn rebind_parked(conn: &Connection, profile_id: &str) -> Result<i64> {
    let mut restored = 0;
    let mut schema: BTreeMap<String, (Vec<String>, ForeignKeys)> = BTreeMap::new();
    let mut progress = true;
    while progress {
        progress = false;
        let (columns, parked) = raw_rows(
            conn,
            "SELECT seq, table_name, row_json, transaction_ids_json FROM book_reset_parked_rows WHERE profile_id = ? ORDER BY seq",
            [profile_id],
        )?;
        for row in &parked {
            let ids = id_list(&sql_text(column(&columns, row, "transaction_ids_json")))?;
            if states(conn, &ids)?
                .iter()
                .any(|(_, state)| *state != State::Bound)
            {
                continue;
            }
            let table = sql_text(column(&columns, row, "table_name"));
            if !schema.contains_key(&table) {
                schema.insert(
                    table.clone(),
                    (table_columns(conn, &table)?, foreign_keys(conn, &table)?),
                );
            }
            let (table_cols, keys) = &schema[&table];
            let values: Vec<(String, Sql)> =
                match loads(&sql_text(column(&columns, row, "row_json")))? {
                    Value::Dict(items) => items
                        .iter()
                        .map(|(k, v)| {
                            let key = match k {
                                Key::Str(k) => py_str(&Value::Str(k.clone())),
                                _ => String::new(),
                            };
                            Ok((key, to_sql(v)?))
                        })
                        .collect::<Result<_>>()?,
                    _ => Vec::new(),
                };
            if table_cols.is_empty() || !parents_present(conn, &values, keys)? {
                continue;
            }
            let insert: Vec<&String> = table_cols
                .iter()
                .filter(|c| values.iter().any(|(k, _)| k == *c))
                .collect();
            let params: Vec<Sql> = insert
                .iter()
                .map(|c| values.iter().find(|(k, _)| k == *c).unwrap().1.clone())
                .collect();
            kassiber_store::begin_write_savepoint(conn, "book_reset_rebind").map_err(sql_error)?;
            let outcome = conn.execute(
                &format!(
                    "INSERT INTO {table}({}) VALUES({})",
                    insert
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", "),
                    vec!["?"; insert.len()].join(", ")
                ),
                rusqlite::params_from_iter(params),
            );
            match outcome {
                Err(error) if integrity(&error) => {
                    // Something now occupies the decision's place: keep it held.
                    conn.execute_batch("ROLLBACK TO SAVEPOINT book_reset_rebind; RELEASE SAVEPOINT book_reset_rebind")
                        .map_err(sql_error)?;
                    continue;
                }
                Err(error) => {
                    let _ = conn.execute_batch("ROLLBACK TO SAVEPOINT book_reset_rebind; RELEASE SAVEPOINT book_reset_rebind");
                    return Err(sql_error(error));
                }
                Ok(_) => {}
            }
            conn.execute_batch("RELEASE SAVEPOINT book_reset_rebind")
                .map_err(sql_error)?;
            conn.execute(
                "DELETE FROM book_reset_parked_rows WHERE seq = ?",
                [column(&columns, row, "seq").clone()],
            )
            .map_err(sql_error)?;
            restored += 1;
            progress = true;
        }
    }
    Ok(restored)
}

#[derive(Default)]
struct Hold {
    decisions: BTreeMap<String, i64>,
    metadata_held: bool,
    missing: BTreeSet<String>,
    changed: BTreeSet<String>,
}

fn held(conn: &Connection, profile_id: &str) -> Result<(BTreeMap<String, Hold>, i64, i64)> {
    let mut holds: BTreeMap<String, Hold> = BTreeMap::new();
    let (mut held, mut waiting) = (0, 0);
    let (columns, parked) = raw_rows(
        conn,
        "SELECT table_name, transaction_ids_json FROM book_reset_parked_rows WHERE profile_id = ? ORDER BY seq",
        [profile_id],
    )?;
    for row in &parked {
        let states = states(
            conn,
            &id_list(&sql_text(column(&columns, row, "transaction_ids_json")))?,
        )?;
        let present: Vec<&String> = states
            .iter()
            .filter(|(_, s)| *s != State::Missing)
            .map(|(k, _)| k)
            .collect();
        if present.is_empty() {
            waiting += 1;
            continue;
        }
        held += 1;
        let table = sql_text(column(&columns, row, "table_name"));
        for id in present {
            let item = holds.entry(id.clone()).or_default();
            *item.decisions.entry(table.clone()).or_default() += 1;
            for (key, state) in &states {
                match state {
                    State::Missing => {
                        item.missing.insert(key.clone());
                    }
                    State::Changed => {
                        item.changed.insert(key.clone());
                    }
                    State::Bound => {}
                }
            }
        }
    }
    let (columns, reservations) = raw_rows(
        conn,
        "SELECT transaction_id, state FROM book_reset_transactions WHERE profile_id = ? AND metadata_json != '{}' ORDER BY transaction_id",
        [profile_id],
    )?;
    for row in &reservations {
        if same(column(&columns, row, "state"), &Sql::Text("changed".into())) {
            held += 1;
            let id = sql_text(column(&columns, row, "transaction_id"));
            let item = holds.entry(id.clone()).or_default();
            item.metadata_held = true;
            item.changed.insert(id);
        } else {
            waiting += 1;
        }
    }
    Ok((holds, held, waiting))
}

/// `counts`: kept decisions not yet restored.
pub fn counts(conn: &Connection, profile_id: &str) -> Result<Value> {
    let (_, held, waiting) = held(conn, profile_id)?;
    Ok(dict([
        ("held", Value::Int(held.into())),
        ("waiting", Value::Int(waiting.into())),
    ]))
}

fn identity(conn: &Connection, id: &str) -> Result<Value> {
    let (columns, rows) = raw_rows(
        conn,
        "SELECT wallet_id, external_id, asset, direction FROM book_reset_transactions WHERE transaction_id = ?",
        [id],
    )?;
    let mut fields = vec![("transaction_id".to_owned(), s(id))];
    if let Some(row) = rows.first() {
        for name in ["wallet_id", "external_id", "asset", "direction"] {
            fields.push((name.to_owned(), from_sql(column(&columns, row, name))));
        }
    }
    Ok(dict(fields))
}

/// `held_quarantines`: hold every returned transaction a kept decision could not bind to.
pub fn held_quarantines(conn: &Connection, profile_id: &str) -> Result<Vec<Value>> {
    if !has_state(conn, profile_id)? {
        return Ok(Vec::new());
    }
    let (holds, _, _) = held(conn, profile_id)?;
    let mut quarantines = Vec::new();
    for (id, item) in holds {
        let workspace: Option<Sql> = conn
            .query_row(
                "SELECT workspace_id FROM transactions WHERE id = ?",
                [&id],
                |r| r.get(0),
            )
            .optional()
            .map_err(sql_error)?;
        let mut changed = Vec::new();
        for changed_id in &item.changed {
            let fields: Option<String> = conn
                .query_row(
                    "SELECT changed_fields_json FROM book_reset_transactions WHERE transaction_id = ?",
                    [changed_id],
                    |r| r.get(0),
                )
                .optional()
                .map_err(sql_error)?;
            changed.push(dict([
                ("transaction_id", s(changed_id.as_str())),
                (
                    "fields",
                    match fields {
                        Some(f) => loads(&f)?,
                        None => Value::List(Vec::new()),
                    },
                ),
            ]));
        }
        let detail = dict([
            (
                "held_decisions",
                dict(
                    item.decisions
                        .iter()
                        .map(|(k, v)| (k.as_str(), Value::Int((*v).into()))),
                ),
            ),
            ("metadata_held", Value::Bool(item.metadata_held)),
            (
                "missing_transactions",
                Value::List(
                    item.missing
                        .iter()
                        .map(|k| identity(conn, k))
                        .collect::<Result<_>>()?,
                ),
            ),
            ("changed_transactions", Value::List(changed)),
        ]);
        quarantines.push(dict([
            ("transaction_id", s(id.as_str())),
            (
                "workspace_id",
                workspace.as_ref().map(from_sql).unwrap_or(Value::Null),
            ),
            ("profile_id", s(profile_id)),
            ("reason", s(HOLD_REASON)),
            ("detail_json", s(dumps(&detail, true)?)),
        ]));
    }
    Ok(quarantines)
}

/// `parked_attachment_relpaths`: managed files of parked attachments.
pub fn parked_attachment_relpaths(conn: &Connection) -> Result<BTreeSet<String>> {
    if !tables_present(conn)? {
        return Ok(BTreeSet::new());
    }
    let (_, rows) = raw_rows(
        conn,
        "SELECT row_json FROM book_reset_parked_rows WHERE table_name = 'attachments'",
        [],
    )?;
    let mut paths = BTreeSet::new();
    for row in &rows {
        if let Some(path) = get(&loads(&sql_text(&row[0]))?, "stored_relpath").filter(|v| truthy(v))
        {
            paths.insert(py_str(path));
        }
    }
    Ok(paths)
}
