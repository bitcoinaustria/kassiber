//! File import-run listing and rollback, from `core/import_batches.py`.
//! Import parsers and observation insertion remain separate, unported services.
use std::collections::BTreeMap;

use kassiber_envelope::{dict, s, AppError};
use kassiber_pyjson::Value;
use kassiber_store::rusqlite::{self, Connection, OptionalExtension, Row};
use kassiber_wallets::{database_error, invalidate_journals};

/// Python's `max(1,min(int(limit or 50),200))`, including string zero (1)
/// versus numeric zero (50), Unicode decimal digits and arbitrary integers.
pub fn list_limit(value: Option<&Value>) -> Result<i64, AppError> {
    use kassiber_envelope::{truthy, type_name};
    let Some(value) = value.filter(|v| truthy(v)) else {
        return Ok(50);
    };
    let integer = match value {
        Value::Int(n) => n.clone(),
        Value::Bool(b) => i64::from(*b).into(),
        Value::Float(f) => {
            if !f.is_finite() {
                return Err(AppError::new(
                    "internal_error",
                    if f.is_nan() {
                        "cannot convert float NaN to integer"
                    } else {
                        "cannot convert float infinity to integer"
                    },
                ));
            }
            return Ok((*f as i64).clamp(1, 200));
        }
        Value::Str(text) => {
            let parsed = text
                .as_str()
                .map(kassiber_pytext::parse_int)
                .unwrap_or(Err(kassiber_pytext::NumberError::Invalid));
            parsed.map_err(|error| AppError::new("internal_error", match error {
                kassiber_pytext::NumberError::Invalid => format!("invalid literal for int() with base 10: {}",kassiber_pytext::repr_py(text)),
                kassiber_pytext::NumberError::TooManyDigits(n) => format!("Exceeds the limit (4300 digits) for integer string conversion: value has {n} digits; use sys.set_int_max_str_digits() to increase the limit"),
            }))?
        }
        other => {
            return Err(AppError::new(
                "internal_error",
                format!(
                "int() argument must be a string, a bytes-like object or a real number, not '{}'",
                type_name(other)
            ),
            ))
        }
    };
    Ok(if integer < 1.into() {
        1
    } else if integer > 200.into() {
        200
    } else {
        i64::try_from(integer).expect("bounded")
    })
}

/// Operations expose only run identities and redacted projections to transports.
pub enum Operation {
    List {
        profile_id: String,
        wallet: Option<String>,
        limit: Value,
    },
    Plan {
        profile_id: String,
        batch: String,
    },
    /// The plan with the book's `journal_input_version` appended, the facts
    /// `review_rollback` binds a consented rollback to (#682).
    Review {
        profile_id: String,
        batch: String,
    },
    /// `input_version(conn, profile_id)`: the profile's journal input
    /// version, which assistant plans and handoffs are bound to.
    InputVersion {
        profile_id: String,
    },
    Rollback {
        profile_id: String,
        batch: String,
    },
}

/// Execute against a connection admitted by the real store. Rollback has its own
/// savepoint, so a failed cascade or invalidation cannot leave partial writes.
pub fn execute(conn: &Connection, operation: Operation) -> Result<Value, AppError> {
    match operation {
        Operation::List {
            profile_id,
            wallet,
            limit,
        } => {
            let wallet_id = wallet
                .map(|w| kassiber_wallets::resolve(conn, &profile_id, &w).map(|w| w.id))
                .transpose()?;
            list(
                conn,
                &profile_id,
                wallet_id.as_deref(),
                list_limit(Some(&limit))?,
            )
        }
        Operation::Plan { profile_id, batch } => plan(conn, &profile_id, &batch),
        Operation::Review { profile_id, batch } => {
            let mut reviewed = plan(conn, &profile_id, &batch)?;
            kassiber_envelope::set(
                &mut reviewed,
                "input_version",
                Value::from(input_version(conn, &profile_id)?),
            );
            Ok(reviewed)
        }
        Operation::InputVersion { profile_id } => {
            Ok(Value::from(input_version(conn, &profile_id)?))
        }
        Operation::Rollback { profile_id, batch } => rollback(conn, &profile_id, &batch),
    }
}

/// `_input_version(conn, profile_id)`.
fn input_version(conn: &Connection, profile_id: &str) -> Result<i64, AppError> {
    let version: Option<Option<i64>> = conn
        .query_row(
            "SELECT journal_input_version FROM profiles WHERE id = ?",
            [profile_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(database_error)?;
    Ok(version.flatten().unwrap_or(0))
}

struct Batch {
    id: String,
    wallet_id: Option<String>,
    source_format: String,
    source_filename: Option<String>,
    column_map: Option<String>,
    imported_at: String,
    inserted: i64,
    updated: i64,
    skipped: i64,
}

impl Batch {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        let raw: Option<String> = row.get("column_map_json")?;
        Ok(Self {
            id: row.get("id")?,
            wallet_id: row.get("wallet_id")?,
            source_format: row.get("source_format")?,
            source_filename: row.get("source_filename")?,
            column_map: raw,
            imported_at: row.get("imported_at")?,
            inserted: row.get::<_, Option<i64>>("rows_inserted")?.unwrap_or(0),
            updated: row.get::<_, Option<i64>>("rows_updated")?.unwrap_or(0),
            skipped: row.get::<_, Option<i64>>("rows_skipped")?.unwrap_or(0),
        })
    }

    fn payload(&self, remaining: i64) -> Result<Value, AppError> {
        let column_map = match self.column_map.as_deref().map(kassiber_pytext::loads) {
            Some(Ok(value)) => value,
            Some(Err(error))
                if matches!(error.kind, kassiber_pytext::JsonErrorKind::TooDeep { .. }) =>
            {
                return Err(AppError::new("internal_error", error.to_string()));
            }
            _ => Value::Null,
        };
        Ok(dict([
            ("id", s(self.id.as_str())),
            (
                "wallet_id",
                self.wallet_id.as_deref().map_or(Value::Null, s),
            ),
            ("source_format", s(self.source_format.as_str())),
            (
                "source_filename",
                self.source_filename.as_deref().map_or(Value::Null, s),
            ),
            ("column_map", column_map),
            ("imported_at", s(self.imported_at.as_str())),
            ("rows_inserted", Value::from(self.inserted)),
            ("rows_updated", Value::from(self.updated)),
            ("rows_skipped", Value::from(self.skipped)),
            ("rows_present", Value::from(remaining)),
            ("rolled_back", Value::Bool(remaining == 0)),
        ]))
    }
}

fn batch(conn: &Connection, profile: &str, reference: &str) -> Result<Batch, AppError> {
    conn.query_row(
        "SELECT * FROM import_batches WHERE profile_id=? AND id=?",
        [profile, reference],
        Batch::read,
    )
    .optional()
    .map_err(database_error)?
    .ok_or_else(|| {
        AppError::new("not_found", format!("Import batch not found: {reference}"))
            .with_hint("Run `imports list` to see this book's import runs.")
    })
}

fn list(
    conn: &Connection,
    profile: &str,
    wallet: Option<&str>,
    limit: i64,
) -> Result<Value, AppError> {
    let limit = if limit == 0 { 50 } else { limit }.clamp(1, 200);
    let mut stmt = conn.prepare("SELECT b.*,(SELECT COUNT(*) FROM import_batch_transactions bt WHERE bt.batch_id=b.id) AS rows_present
        FROM import_batches b WHERE b.profile_id=?1 AND (?2 IS NULL OR b.wallet_id=?2) ORDER BY b.imported_at DESC,b.id DESC LIMIT ?3").map_err(database_error)?;
    let batches = stmt
        .query_map(rusqlite::params![profile, wallet, limit], |row| {
            Ok((Batch::read(row)?, row.get::<_, i64>("rows_present")?))
        })
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    let payloads = batches
        .into_iter()
        .map(|(batch, remaining)| batch.payload(remaining))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Value::List(payloads))
}

// Quote schema-owned identifiers, including unusual user-created table names.
fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn primary_key(conn: &Connection, table: &str) -> Result<Option<String>, AppError> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({})", quote(table)))
        .map_err(database_error)?;
    let keys = stmt
        .query_map([], |r| Ok((r.get::<_, String>(1)?, r.get::<_, i64>(5)?)))
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?
        .into_iter()
        .filter(|(_, pk)| *pk != 0)
        .map(|(name, _)| name)
        .collect::<Vec<_>>();
    Ok((keys.len() == 1).then(|| keys[0].clone()))
}

/// Match Python's insertion-ordered, bounded FK fixpoint. Each table has one OR
/// predicate so a pair reachable through both legs is counted only once.
fn cascade_predicates(conn: &Connection) -> Result<Vec<(String, String)>, AppError> {
    let tables = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'")
        .map_err(database_error)?
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(database_error)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(database_error)?;
    let mut edges = Vec::new();
    for table in tables {
        let mut stmt = conn
            .prepare(&format!("PRAGMA foreign_key_list({})", quote(&table)))
            .map_err(database_error)?;
        let foreign_keys = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?,
                    r.get::<_, String>(6)?,
                ))
            })
            .map_err(database_error)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(database_error)?;
        let mut links = Vec::new();
        for (parent, column, key, on_delete) in foreign_keys {
            if on_delete.to_uppercase() != "CASCADE" {
                continue;
            }
            let key = match key.filter(|s| !s.is_empty()) {
                Some(key) => Some(key),
                None => primary_key(conn, &parent)?,
            };
            if let Some(key) = key {
                links.push((column, parent, key));
            }
        }
        if !links.is_empty() {
            edges.push((table, links));
        }
    }
    let mut resolved = BTreeMap::from([(
        "transactions".to_owned(),
        "id IN (SELECT id FROM temp._import_rollback_ids)".to_owned(),
    )]);
    let mut order = Vec::new();
    for _ in 0..6 {
        let mut changed = false;
        for (table, links) in &edges {
            if matches!(table.as_str(), "transactions" | "import_batch_transactions") {
                continue;
            }
            let usable = links
                .iter()
                .filter(|(_, parent, _)| parent != table && resolved.contains_key(parent))
                .collect::<Vec<_>>();
            if usable.is_empty() {
                continue;
            }
            let predicate = usable
                .into_iter()
                .map(|(column, parent, key)| {
                    format!(
                        "{} IN (SELECT {} FROM {} WHERE {})",
                        quote(column),
                        quote(key),
                        quote(parent),
                        resolved[parent]
                    )
                })
                .collect::<Vec<_>>()
                .join(" OR ");
            if resolved.get(table) != Some(&predicate) {
                if !resolved.contains_key(table) {
                    order.push(table.clone());
                }
                resolved.insert(table.clone(), predicate);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    Ok(order
        .into_iter()
        .map(|table| {
            let predicate = resolved.remove(&table).expect("resolved path");
            (table, predicate)
        })
        .collect())
}

fn collateral(conn: &Connection, staged: i64) -> Result<Value, AppError> {
    let mut counts = Vec::new();
    if staged != 0 {
        for (table, predicate) in cascade_predicates(conn)? {
            let count: i64 = conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {} WHERE {predicate}", quote(&table)),
                    [],
                    |r| r.get(0),
                )
                .map_err(database_error)?;
            if count != 0 {
                counts.push((table, Value::from(count)));
            }
        }
    }
    Ok(dict(counts))
}

fn with_staged<T>(
    conn: &Connection,
    batch: &str,
    run: impl FnOnce(i64) -> Result<T, AppError>,
) -> Result<T, AppError> {
    conn.execute_batch("DROP TABLE IF EXISTS temp._import_rollback_ids;CREATE TEMP TABLE _import_rollback_ids(id TEXT PRIMARY KEY)").map_err(database_error)?;
    let result = (|| {
        conn.execute("INSERT OR IGNORE INTO temp._import_rollback_ids(id) SELECT transaction_id FROM import_batch_transactions WHERE batch_id=?",[batch]).map_err(database_error)?;
        let staged = conn
            .query_row("SELECT COUNT(*) FROM temp._import_rollback_ids", [], |r| {
                r.get(0)
            })
            .map_err(database_error)?;
        run(staged)
    })();
    let cleaned = conn
        .execute_batch("DROP TABLE IF EXISTS temp._import_rollback_ids")
        .map_err(database_error);
    let result = result?;
    cleaned?;
    Ok(result)
}

fn plan(conn: &Connection, profile: &str, reference: &str) -> Result<Value, AppError> {
    let row = batch(conn, profile, reference)?;
    with_staged(conn, &row.id, |staged| {
        Ok(dict([
            ("batch", row.payload(staged)?),
            ("transactions_to_delete", Value::from(staged)),
            ("also_removed", collateral(conn, staged)?),
            ("journals_invalidated", Value::Bool(staged != 0)),
        ]))
    })
}

fn rollback(conn: &Connection, profile: &str, reference: &str) -> Result<Value, AppError> {
    conn.execute_batch("SAVEPOINT import_run_rollback")
        .map_err(database_error)?;
    let result = (|| {
        let row = batch(conn, profile, reference)?;
        let (staged, counts) = with_staged(conn, &row.id, |staged| {
            let counts = collateral(conn, staged)?;
            conn.execute(
                "DELETE FROM transactions WHERE id IN (SELECT id FROM temp._import_rollback_ids)",
                [],
            )
            .map_err(database_error)?;
            Ok((staged, counts))
        })?;
        conn.execute("DELETE FROM import_batches WHERE id=?", [&row.id])
            .map_err(database_error)?;
        if staged != 0 {
            invalidate_journals(conn, profile)?;
        }
        Ok(dict([
            ("id", s(row.id)),
            ("source_format", s(row.source_format)),
            (
                "source_filename",
                row.source_filename.map_or(Value::Null, s),
            ),
            ("wallet_id", row.wallet_id.map_or(Value::Null, s)),
            ("rolled_back", Value::Bool(true)),
            ("transactions_deleted", Value::from(staged)),
            ("also_removed", counts),
            ("journals_invalidated", Value::Bool(staged != 0)),
        ]))
    })();
    let result = result.and_then(|payload| {
        conn.execute_batch("RELEASE import_run_rollback")
            .map_err(database_error)?;
        Ok(payload)
    });
    if result.is_err() {
        conn.execute_batch("ROLLBACK TO import_run_rollback;RELEASE import_run_rollback")
            .map_err(database_error)?;
    }
    result
}

#[cfg(test)]
mod tests;
