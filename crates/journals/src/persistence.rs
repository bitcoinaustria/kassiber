//! Persist an already built journal. This service is deliberately unregistered.
//! Call under the caller's BEGIN IMMEDIATE and journal savepoint; the nested
//! savepoint also protects callers that catch a storage refusal.
use crate::{array, model::*, pyjson, render::msat, storage, string, truth, Refusal};
use kassiber_store::rusqlite::{self, params, params_from_iter, types::Value as Sql, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
mod filed;
mod quantity;
mod quarantine;
pub use quarantine::{dedupe_quarantines, enrich_quarantine_detail};

impl From<rusqlite::Error> for Refusal {
    fn from(e: rusqlite::Error) -> Self {
        Self::new("DatabaseError", e.to_string())
    }
}
pub(super) fn bind(v: &Value) -> Result<Sql, Refusal> {
    Ok(match v {
        Value::Null => Sql::Null,
        Value::String(s) => Sql::Text(s.clone()),
        Value::Bool(b) => Sql::Integer(i64::from(*b)),
        Value::Number(n) => {
            if !n.to_string().contains(['.', 'e', 'E']) {
                Sql::Integer(n.as_i64().ok_or_else(|| {
                    Refusal::new(
                        "OverflowError",
                        "Python int too large to convert to SQLite INTEGER",
                    )
                })?)
            } else {
                Sql::Real(
                    n.as_f64()
                        .ok_or_else(|| Refusal::new("ValueError", "invalid SQLite REAL"))?,
                )
            }
        }
        _ => {
            return Err(Refusal::new(
                "ProgrammingError",
                "unsupported SQLite binding",
            ))
        }
    })
}
fn t(s: impl Into<String>) -> Sql {
    Sql::Text(s.into())
}
fn opt(s: Option<String>) -> Sql {
    s.map_or(Sql::Null, t)
}
fn real(d: &JournalDecimal) -> Sql {
    Sql::Real(d.0.to_string().parse().expect("Decimal float spelling"))
}
fn optional_real(d: &Option<JournalDecimal>) -> Sql {
    d.as_ref().map_or(Sql::Null, real)
}
fn exact(d: &JournalDecimal) -> Result<Sql, Refusal> {
    Ok(t(d.0.to_fixed_string()?))
}
fn optional_exact(d: &Option<JournalDecimal>) -> Result<Sql, Refusal> {
    d.as_ref().map_or(Ok(Sql::Null), exact)
}
fn amount(d: &JournalDecimal) -> Result<Sql, Refusal> {
    bind(&msat(&d.0)?)
}

pub(super) struct Writer<'a, O> {
    conn: &'a Connection,
    observer: &'a mut O,
}
impl<O: storage::WriteObserver> Writer<'_, O> {
    pub(super) fn clear(&mut self, table: &'static str, profile: &str) -> Result<(), Refusal> {
        self.conn
            .prepare(&format!("DELETE FROM {table} WHERE profile_id = ?"))?
            .execute([profile])?;
        self.observer.wrote(table)?;
        Ok(())
    }
    pub(super) fn insert(
        &mut self,
        table: &'static str,
        columns: &[&str],
        rows: impl IntoIterator<Item = Vec<Sql>>,
    ) -> Result<(), Refusal> {
        let query = format!(
            "INSERT INTO {table}({}) VALUES({})",
            columns.join(","),
            vec!["?"; columns.len()].join(",")
        );
        let mut statement = self.conn.prepare(&query)?;
        for row in rows {
            statement.execute(params_from_iter(row))?;
            self.observer.wrote(table)?;
        }
        Ok(())
    }
}

struct Savepoint<'a>(Option<&'a Connection>);
impl Drop for Savepoint<'_> {
    fn drop(&mut self) {
        if let Some(conn) = self.0 {
            let _ = conn.execute_batch("ROLLBACK TO r4_journal_store; RELEASE r4_journal_store");
        }
    }
}

#[derive(Debug, serde::Serialize)]
pub struct Stored {
    pub processed_at: String,
    pub processed_transactions: i64,
    pub quarantines: Vec<QuarantineRow>,
    pub quantity_counts: HashMap<&'static str, usize>,
    pub filed_report_impacts_resolved: usize,
}

/// No clock or entropy is read here: timestamps and replacement-row UUIDs come
/// from the caller. Entry ids are derived from identities in engine order.
pub fn store_ledger_state(
    conn: &Connection,
    profile: &Value,
    state: &JournalState<'_>,
    created_at: &str,
    ids: &mut impl FnMut() -> Result<String, Refusal>,
    observer: &mut impl storage::WriteObserver,
) -> Result<Stored, Refusal> {
    if conn.is_autocommit() {
        return Err(Refusal::new(
            "RuntimeError",
            "journal persistence requires BEGIN IMMEDIATE",
        ));
    }
    conn.execute_batch("SAVEPOINT r4_journal_store")?;
    let mut savepoint = Savepoint(Some(conn));
    let result = store(
        &mut Writer { conn, observer },
        profile,
        state,
        created_at,
        ids,
    )?;
    conn.execute_batch("RELEASE r4_journal_store")?;
    savepoint.0 = None;
    Ok(result)
}

fn store<O: storage::WriteObserver>(
    writer: &mut Writer<'_, O>,
    profile: &Value,
    state: &JournalState<'_>,
    at: &str,
    ids: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Stored, Refusal> {
    let pid = string(&profile["id"]);
    let wid = string(&profile["workspace_id"]);
    let previous: Option<String> = writer.conn.query_row(
        "SELECT last_processed_at FROM profiles WHERE id=?",
        [&pid],
        |r| r.get(0),
    )?;
    let processed_at = storage::journal_processed_at(at, previous.as_deref())?;
    writer.conn.prepare("INSERT INTO journal_custody_gap_inputs(workspace_id,profile_id,input_version,ignored_ids_json,accounting_ignored_ids_json,created_at) VALUES(?,?,?,?,?,?) ON CONFLICT(profile_id) DO UPDATE SET workspace_id=excluded.workspace_id,input_version=excluded.input_version,ignored_ids_json=excluded.ignored_ids_json,accounting_ignored_ids_json=excluded.accounting_ignored_ids_json,created_at=excluded.created_at")?
        .execute(params![wid,pid,profile["journal_input_version"].as_i64().unwrap_or(0),pyjson(state.custody_gap_ignored_transaction_ids,true)?,pyjson(state.custody_gap_accounting_ignored_transaction_ids,true)?,at])?;
    writer.observer.wrote("journal_custody_gap_inputs")?;
    for table in [
        "journal_quarantines",
        "journal_tax_summary",
        "journal_account_holdings",
        "journal_wallet_holdings",
    ] {
        writer.clear(table, &pid)?;
    }
    quantity::reconciliations(writer, profile, state, at)?;
    let quantity_counts = quantity::store(writer, &wid, &pid, state.custody_quantity, at)?;
    let pricing: HashMap<String, (Sql, Sql)> = writer
        .conn
        .prepare(
            "SELECT id,pricing_source_kind,pricing_quality FROM transactions WHERE profile_id=?",
        )?
        .query_map([&pid], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<rusqlite::Result<_>>()?;
    let mut ordinals = HashMap::new();
    let mut rows = Vec::with_capacity(state.entries.len());
    for e in &state.entries {
        let key = (
            string(&e.transaction_id),
            string(&e.wallet_id),
            e.entry_type,
            e.asset.as_str(),
        );
        let ordinal = ordinals.entry(key.clone()).or_insert(0usize);
        let id = storage::journal_entry_id(&pid, &key.0, &key.1, key.2, key.3, *ordinal)?;
        *ordinal += 1;
        let calculation = e
            .calculation
            .as_ref()
            .map(serde_json::to_value)
            .transpose()
            .map_err(|e| Refusal::new("ValueError", e.to_string()))?
            .unwrap_or(Value::Null);
        let (legacy, compact) = storage::encode_calculation(&calculation)?;
        let (source, quality) = pricing
            .get(&key.0)
            .cloned()
            .unwrap_or((Sql::Null, Sql::Null));
        rows.push([
            t(id),
            bind(&e.workspace_id)?,
            bind(&e.profile_id)?,
            bind(&e.transaction_id)?,
            bind(&e.wallet_id)?,
            bind(&e.account_id)?,
            bind(&e.occurred_at)?,
            t(e.entry_type),
            t(&e.asset),
            amount(&e.quantity)?,
            real(&e.fiat_value),
            real(&e.unit_cost),
            optional_real(&e.cost_basis),
            optional_real(&e.proceeds),
            optional_real(&e.gain_loss),
            exact(&e.fiat_value)?,
            exact(&e.unit_cost)?,
            optional_exact(&e.cost_basis)?,
            optional_exact(&e.proceeds)?,
            optional_exact(&e.gain_loss)?,
            opt(legacy),
            opt(compact),
            source,
            quality,
            t(&e.description),
            opt(e.at_category.clone()),
            e.at_kennzahl
                .flatten()
                .map_or(Sql::Null, |n| Sql::Integer(n.into())),
            opt(e.capital_gains_type.map(str::to_owned)),
            t(at),
        ]);
    }
    storage::replace_journal_entries(writer.conn, &pid, &rows, writer.observer)?;
    let live: HashSet<String> = pricing.into_keys().collect();
    let mut quarantines = dedupe_quarantines(&state.quarantines)?;
    quarantines.retain(|q| live.contains(&string(&q.transaction_id)));
    let mut candidates = HashMap::<String, Vec<Value>>::new();
    for c in array(&state.ownership_review_candidates) {
        if truth(&c["out_id"]) {
            candidates
                .entry(string(&c["out_id"]))
                .or_default()
                .push(c.clone());
        }
    }
    let rows = quarantines
        .iter()
        .map(|q| {
            Ok(vec![
                bind(&q.transaction_id)?,
                bind(&q.workspace_id)?,
                bind(&q.profile_id)?,
                t(&q.reason),
                t(enrich_quarantine_detail(
                    &q.detail_json,
                    candidates
                        .get(&string(&q.transaction_id))
                        .map(Vec::as_slice)
                        .unwrap_or(&[]),
                )?),
                t(at),
            ])
        })
        .collect::<Result<Vec<_>, Refusal>>()?;
    writer.insert(
        "journal_quarantines",
        &[
            "transaction_id",
            "workspace_id",
            "profile_id",
            "reason",
            "detail_json",
            "created_at",
        ],
        rows,
    )?;
    let mut rows = Vec::new();
    for s in &state.tax_summary {
        let quantity = if truth(&s.quantity_msat) {
            bind(&s.quantity_msat)?
        } else {
            let d = JournalDecimal(s.quantity.to_string().parse()?);
            amount(&d)?
        };
        rows.push(vec![
            t(ids()?),
            t(&wid),
            t(&pid),
            Sql::Integer(s.year.into()),
            t(&s.asset),
            t(s.transaction_type),
            t(s.capital_gains_type),
            quantity,
            Sql::Real(s.proceeds),
            Sql::Real(s.cost_basis),
            Sql::Real(s.gain_loss),
            t(at),
        ]);
    }
    writer.insert(
        "journal_tax_summary",
        &[
            "id",
            "workspace_id",
            "profile_id",
            "year",
            "asset",
            "transaction_type",
            "capital_gains_type",
            "quantity",
            "proceeds",
            "cost_basis",
            "gain_loss",
            "created_at",
        ],
        rows,
    )?;
    for (table, columns, holdings) in [
        (
            "journal_account_holdings",
            [
                "id",
                "workspace_id",
                "profile_id",
                "account_id",
                "account_code",
                "account_label",
                "asset",
                "quantity",
                "cost_basis",
                "created_at",
            ],
            &state.account_holdings,
        ),
        (
            "journal_wallet_holdings",
            [
                "id",
                "workspace_id",
                "profile_id",
                "wallet_id",
                "wallet_label",
                "account_code",
                "asset",
                "quantity",
                "cost_basis",
                "created_at",
            ],
            &state.wallet_holdings,
        ),
    ] {
        let mut rows = Vec::new();
        for (key, h) in &holdings.pairs {
            if key.len() != 4 {
                return Err(Refusal::new("ValueError", "holding key width"));
            }
            rows.push(vec![
                t(ids()?),
                t(&wid),
                t(&pid),
                bind(&key[0])?,
                bind(&key[1])?,
                bind(&key[2])?,
                bind(&key[3])?,
                amount(&h.quantity)?,
                real(&h.cost_basis),
                t(at),
            ]);
        }
        writer.insert(table, &columns, rows)?;
    }
    let count = writer.conn.query_row(
        "SELECT COUNT(*) FROM transactions WHERE profile_id=? AND excluded=0",
        [&pid],
        |r| r.get(0),
    )?;
    writer.conn.prepare("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=?,last_processed_input_version=journal_input_version,ownership_review_counts_json=? WHERE id=?")?
        .execute(params![processed_at,count,pyjson(&state.ownership_review_counts,false)?,pid])?;
    writer.observer.wrote("profiles")?;
    let filed_report_impacts_resolved = if !truth(state.custody_component_blockers)
        && quarantines.is_empty()
        && array(&state.custody_quantity["issues"]).is_empty()
    {
        filed::resolve(writer, &wid, &pid, at)?
    } else {
        0
    };
    Ok(Stored {
        processed_at,
        processed_transactions: count,
        quarantines,
        quantity_counts,
        filed_report_impacts_resolved,
    })
}
