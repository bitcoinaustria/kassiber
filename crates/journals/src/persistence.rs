//! Persist an already built journal. This service is deliberately unregistered.
//! Call under the caller's BEGIN IMMEDIATE and journal savepoint; the nested
//! savepoint also protects callers that catch a storage refusal.
use crate::{array, model::*, pyjson, render::msat, storage, string, truth, Refusal};
use kassiber_store::rusqlite::types::ValueRef;
use kassiber_store::rusqlite::{self, params, params_from_iter, types::Value as Sql, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
mod entries;
mod filed;
mod quantity;
mod quarantine;
pub use quarantine::{dedupe_quarantines, enrich_quarantine_detail};

impl From<rusqlite::Error> for Refusal {
    fn from(e: rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(code, Some(message)) = e {
            use rusqlite::ffi;
            let family = match code.extended_code & 0xff {
                ffi::SQLITE_CONSTRAINT | ffi::SQLITE_MISMATCH => "IntegrityError",
                ffi::SQLITE_TOOBIG => "DataError",
                ffi::SQLITE_INTERNAL | ffi::SQLITE_NOTFOUND => "InternalError",
                ffi::SQLITE_MISUSE | ffi::SQLITE_RANGE => "ProgrammingError",
                ffi::SQLITE_NOMEM => "MemoryError",
                ffi::SQLITE_CORRUPT | ffi::SQLITE_NOTADB | ffi::SQLITE_AUTH => "DatabaseError",
                _ => "OperationalError",
            };
            let mut refusal = Self::new(family, message);
            refusal.sqlite_code = Some(code.extended_code);
            return refusal;
        }
        Self::new("DatabaseError", e.to_string())
    }
}
pub(super) fn borrowed(v: &Value) -> Result<rusqlite::types::ValueRef<'_>, Refusal> {
    use rusqlite::types::ValueRef;
    Ok(match v {
        Value::Null => ValueRef::Null,
        Value::String(s) => ValueRef::Text(s.as_bytes()),
        Value::Bool(b) => ValueRef::Integer(i64::from(*b)),
        Value::Number(n) => {
            if let Some(n) = n.as_i64() {
                ValueRef::Integer(n)
            } else if !n.as_str().contains(['.', 'e', 'E']) {
                return Err(Refusal::new(
                    "OverflowError",
                    "Python int too large to convert to SQLite INTEGER",
                ));
            } else {
                ValueRef::Real(
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
pub(super) fn bind(v: &Value) -> Result<Sql, Refusal> {
    use rusqlite::types::ValueRef;
    if let Some(bytes) = kassiber_custody::codec::raw_bytes(v) {
        return Ok(Sql::Blob(bytes));
    }
    Ok(match borrowed(v)? {
        ValueRef::Null => Sql::Null,
        ValueRef::Integer(n) => Sql::Integer(n),
        ValueRef::Real(n) => Sql::Real(n),
        ValueRef::Text(s) => Sql::Text(std::str::from_utf8(s).expect("JSON text").into()),
        ValueRef::Blob(_) => unreachable!("JSON contains no blob scalars"),
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
fn amount(d: &JournalDecimal) -> Result<Sql, Refusal> {
    if let Some(n) =
        d.0.coefficient_u128()
            .filter(|n| *n <= u128::from(u64::MAX))
    {
        let shift = d.0.exponent() + 11;
        if (-19..=19).contains(&shift) {
            let magnitude = if shift >= 0 {
                n * 10u128.pow(shift as u32)
            } else {
                let divisor = 10u128.pow((-shift) as u32);
                n / divisor + u128::from(n % divisor >= divisor / 2)
            };
            if let Ok(n) = i64::try_from(magnitude) {
                return Ok(Sql::Integer(if d.0.is_sign_negative() { -n } else { n }));
            }
        }
    }
    bind(&msat(&d.0)?)
}

fn decimal_bindings(d: Option<&JournalDecimal>) -> Result<(Sql, Sql), Refusal> {
    let Some(d) = d else {
        return Ok((Sql::Null, Sql::Null));
    };
    let fixed = d.0.to_fixed_string()?;
    let float = if fixed.len() <= 64 {
        fixed.parse().expect("Decimal float spelling")
    } else {
        d.0.to_string().parse().expect("Decimal float spelling")
    };
    Ok((Sql::Real(float), t(fixed)))
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
    store_ledger_state_with_clock(
        conn,
        profile,
        state,
        created_at,
        &mut || Ok(created_at.to_owned()),
        ids,
        observer,
    )
}

/// Like `store_ledger_state`, with the separately sampled clocks used when
/// resolving filed-chain impacts. The caller supplies the stored-row clock.
pub fn store_ledger_state_with_clock(
    conn: &Connection,
    profile: &Value,
    state: &JournalState<'_>,
    created_at: &str,
    clock: &mut impl FnMut() -> Result<String, Refusal>,
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
        clock,
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
    clock: &mut impl FnMut() -> Result<String, Refusal>,
    ids: &mut impl FnMut() -> Result<String, Refusal>,
) -> Result<Stored, Refusal> {
    writer.observer.phase("setup");
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
    writer.observer.phase("quantity");
    quantity::reconciliations(writer, profile, state, at)?;
    let (observations, postings) = state.quantity_refs();
    let quantity_counts = quantity::store(
        writer,
        &wid,
        &pid,
        state.custody_quantity,
        &observations,
        &postings,
        at,
    )?;
    writer.observer.phase("pricing");
    let pricing: HashMap<String, (Sql, Sql)> = writer
        .conn
        .prepare(
            "SELECT id,pricing_source_kind,pricing_quality FROM transactions WHERE profile_id=?",
        )?
        .query_map([&pid], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
        .collect::<rusqlite::Result<_>>()?;
    writer.observer.phase("entries_prepare");
    let retained: bool = writer.conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM journal_entries WHERE profile_id=?)",
        [&pid],
        |r| r.get(0),
    )?;
    let mut ordinals = HashMap::new();
    let mut rows = Vec::with_capacity(state.entries.len());
    let calculations = std::cell::RefCell::new(storage::CalculationEncoder::default());
    let mut comparisons = storage::compare::Sources::default();
    for e in &state.entries {
        let key = (
            quantity::key(&e.transaction_id),
            quantity::key(e.wallet_id),
            e.entry_type,
            e.asset.as_ref(),
        );
        let ordinal = ordinals.entry(key.clone()).or_insert(0usize);
        let id = storage::journal_entry_id(&pid, &key.0, &key.1, key.2, key.3, *ordinal)?;
        *ordinal += 1;
        let calculation = if retained {
            comparisons.prepare(e.calculation.as_ref(), &mut calculations.borrow_mut())?
        } else if let Some(calculation) = &e.calculation {
            let (legacy, compact) = calculations.borrow_mut().encode(calculation)?;
            storage::compare::Prepared::Fallback(legacy, compact)
        } else {
            storage::compare::Prepared::Absent
        };
        let (source, quality) = pricing
            .get(key.0.as_ref())
            .unwrap_or(&(Sql::Null, Sql::Null));
        let metadata = [
            borrowed(e.workspace_id)?,
            borrowed(e.profile_id)?,
            borrowed(&e.transaction_id)?,
            borrowed(e.wallet_id)?,
            borrowed(e.account_id)?,
            borrowed(&e.occurred_at)?,
            ValueRef::Text(e.entry_type.as_bytes()),
            ValueRef::Text(e.asset.as_bytes()),
            source.into(),
            quality.into(),
            ValueRef::Text(e.description.as_bytes()),
            e.at_category
                .as_ref()
                .map_or(ValueRef::Null, |s| ValueRef::Text(s.as_bytes())),
            e.at_kennzahl
                .flatten()
                .map_or(ValueRef::Null, |n| ValueRef::Integer(n.into())),
            e.capital_gains_type
                .map_or(ValueRef::Null, |s| ValueRef::Text(s.as_bytes())),
            ValueRef::Text(at.as_bytes()),
        ];
        let quantity = match &e.quantity {
            JournalQuantity::Finite(d) => amount(d)?,
            other => bind(&other.msat()?)?,
        };
        for decimal in [
            Some(&e.fiat_value),
            Some(&e.unit_cost),
            e.cost_basis.as_ref(),
            e.proceeds.as_ref(),
            e.gain_loss.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            entries::validate(decimal)?;
        }
        rows.push(entries::Plan {
            id,
            metadata,
            quantity,
            entry: e,
            calculation,
        });
    }
    let rows: Vec<_> = rows
        .into_iter()
        .map(|plan| entries::Row {
            plan,
            sources: &comparisons,
            encoder: &calculations,
            bindings: std::cell::OnceCell::new(),
        })
        .collect();
    writer.observer.phase("entries_write");
    storage::replace_entry_bindings(writer.conn, &pid, &rows, writer.observer)?;
    writer.observer.phase("small_tables");
    let live: HashSet<&str> = pricing.keys().map(String::as_str).collect();
    let mut quarantines = dedupe_quarantines(&state.quarantines)?;
    quarantines.retain(|q| live.contains(quantity::key(&q.transaction_id).as_ref()));
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
    let semantics = kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION;
    writer.conn.prepare("UPDATE profiles SET journal_semantics_version=?,required_journal_semantics_version=MAX(required_journal_semantics_version,?) WHERE id=?")?
        .execute(params![semantics, semantics, pid])?;
    writer.observer.wrote("profiles")?;
    writer.observer.phase("filed");
    let filed_report_impacts_resolved = if !truth(state.custody_component_blockers)
        && quarantines.is_empty()
        && array(&state.custody_quantity["issues"]).is_empty()
    {
        filed::resolve(writer, &wid, &pid, at, clock)?
    } else {
        0
    };
    writer.observer.phase("complete");
    Ok(Stored {
        processed_at,
        processed_transactions: count,
        quarantines,
        quantity_counts,
        filed_report_impacts_resolved,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_writers_preserve_driver_and_observer_refusals_until_caller_rollback() {
        struct Observer {
            fail: bool,
            calls: usize,
        }
        impl storage::WriteObserver for Observer {
            fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
                self.calls += 1;
                if self.fail {
                    Err(rusqlite::Error::InvalidQuery)
                } else {
                    Ok(())
                }
            }
        }
        for mode in [
            "clear_prepare",
            "clear_execute",
            "clear_observer",
            "insert_prepare",
            "insert_execute",
            "insert_observer",
            "insert_width",
        ] {
            let conn = Connection::open_in_memory().unwrap();
            conn.execute_batch("CREATE TABLE projection(profile_id TEXT, value TEXT UNIQUE); INSERT INTO projection VALUES('p','before')").unwrap();
            if mode == "clear_execute" {
                conn.execute_batch("CREATE TEMP TRIGGER refuse_clear BEFORE DELETE ON projection BEGIN SELECT RAISE(ABORT,'clear refused'); END").unwrap();
            }
            conn.execute_batch("BEGIN IMMEDIATE").unwrap();
            let mut observer = Observer {
                fail: mode.ends_with("observer"),
                calls: 0,
            };
            let mut writer = Writer {
                conn: &conn,
                observer: &mut observer,
            };
            let result = if mode.starts_with("clear") {
                writer.clear(
                    if mode == "clear_prepare" {
                        "missing"
                    } else {
                        "projection"
                    },
                    "p",
                )
            } else {
                let mut rows = vec![vec![t("p"), t("after")]];
                if mode == "insert_execute" {
                    rows.push(vec![t("p"), t("before")]);
                }
                if mode == "insert_width" {
                    rows[0].pop();
                }
                writer.insert(
                    "projection",
                    if mode == "insert_prepare" {
                        &["profile_id", "missing"]
                    } else {
                        &["profile_id", "value"]
                    },
                    rows,
                )
            };
            let refusal = result.unwrap_err();
            // SQLite failures keep Python's sqlite3 subclass; other driver
            // and observer failures keep the generic DatabaseError family.
            let family = match mode {
                "clear_prepare" | "insert_prepare" => "OperationalError",
                "clear_execute" | "insert_execute" => "IntegrityError",
                _ => "DatabaseError",
            };
            assert_eq!(refusal.family, family, "{mode}: {refusal}");
            assert_eq!(
                observer.calls,
                usize::from(mode.ends_with("observer") || mode == "insert_execute"),
                "{mode}"
            );
            if mode == "clear_execute" {
                assert!(refusal.message.contains("clear refused"));
            }
            if mode.ends_with("observer") {
                assert_eq!(refusal.message, rusqlite::Error::InvalidQuery.to_string());
            }
            assert!(!conn.is_autocommit(), "{mode}");
            conn.execute_batch("ROLLBACK").unwrap();
            let rows: Vec<String> = conn
                .prepare("SELECT value FROM projection")
                .unwrap()
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<rusqlite::Result<_>>()
                .unwrap();
            assert_eq!(rows, ["before"], "{mode}");
        }
    }

    #[test]
    fn amount_bindings_match_python_rounding_fallbacks_and_sqlite_overflow() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/data/storage.json")).unwrap();
        for case in array(&vectors["amounts"]) {
            let decimal = JournalDecimal(case[0].as_str().unwrap().parse().unwrap());
            let actual = amount(&decimal);
            if let Some(family) = case[2].as_str() {
                assert_eq!(actual.unwrap_err().family, family, "{}", case[0]);
            } else {
                assert_eq!(
                    actual.unwrap(),
                    Sql::Integer(case[1].as_i64().unwrap()),
                    "{}",
                    case[0]
                );
            }
        }
        assert!(array(&vectors["amounts"]).len() > 200);
    }

    #[test]
    fn decimal_bindings_match_python_fixed_text_and_all_float_bits() {
        let vectors: Value =
            serde_json::from_str(include_str!("../tests/data/storage.json")).unwrap();
        for case in array(&vectors["decimals"]) {
            let decimal = JournalDecimal(case[0].as_str().unwrap().parse().unwrap());
            let (actual, exact) = decimal_bindings(Some(&decimal)).unwrap();
            let Sql::Real(actual) = actual else {
                panic!("REAL binding")
            };
            assert_eq!(actual.to_bits(), case[1].as_u64().unwrap(), "{}", case[0]);
            assert_eq!(exact, t(case[2].as_str().unwrap()), "{}", case[0]);
        }
        for spelling in ["1E100000001", "1E-100000002"] {
            let decimal = JournalDecimal(spelling.parse().unwrap());
            let expected = decimal.0.to_fixed_string().unwrap_err();
            let actual = decimal_bindings(Some(&decimal)).unwrap_err();
            assert_eq!(actual.family, expected.name());
            assert_eq!(actual.message, expected.to_string());
        }
        assert_eq!(decimal_bindings(None).unwrap(), (Sql::Null, Sql::Null));
    }

    #[test]
    fn shared_decimal_spelling_preserves_float_bits_and_exact_text() {
        for spelling in [
            "0",
            "-0.00",
            "123.4500",
            "1E-32",
            "1E+32",
            "1E-1000",
            "1E+1000",
            "9007199254740993",
            "1.00000000000000011102230246251565404236316680908203125",
        ] {
            let decimal = JournalDecimal(spelling.parse().unwrap());
            let (float, exact) = decimal_bindings(Some(&decimal)).unwrap();
            let Sql::Real(float) = float else {
                panic!("REAL binding")
            };
            let Sql::Real(reference) = real(&decimal) else {
                panic!("REAL reference")
            };
            assert_eq!(float.to_bits(), reference.to_bits(), "{spelling}");
            assert_eq!(exact, t(decimal.0.to_fixed_string().unwrap()));
        }
        assert_eq!(decimal_bindings(None).unwrap(), (Sql::Null, Sql::Null));
    }
    #[test]
    fn scalar_bindings_preserve_types_signs_and_refuse_wide_or_structured_values() {
        for (value, expected) in [
            (json!(null), Sql::Null),
            (json!(true), Sql::Integer(1)),
            (json!(false), Sql::Integer(0)),
            (json!("é🔥"), t("é🔥")),
            (json!("\0"), t("\0")),
            (json!(i64::MIN), Sql::Integer(i64::MIN)),
            (json!(i64::MAX), Sql::Integer(i64::MAX)),
            (json!(1.5), Sql::Real(1.5)),
        ] {
            assert_eq!(bind(&value).unwrap(), expected);
        }
        let Sql::Real(n) = bind(&json!(-0.0)).unwrap() else {
            panic!("REAL");
        };
        assert!(n.is_sign_negative());
        for value in [json!([]), json!({"n":1})] {
            assert_eq!(bind(&value).unwrap_err().family, "ProgrammingError");
        }
        for text in [
            "9223372036854775808",
            "-9223372036854775809",
            "999999999999999999999999999999",
        ] {
            let value = serde_json::from_str(text).unwrap();
            let error = bind(&value).unwrap_err();
            assert_eq!(error.family, "OverflowError");
            assert_eq!(
                error.message,
                "Python int too large to convert to SQLite INTEGER"
            );
        }
        for text in ["1e1000", "-1e1000"] {
            let value = serde_json::from_str(text).unwrap();
            let error = bind(&value).unwrap_err();
            assert_eq!(error.family, "ValueError");
            assert_eq!(error.message, "invalid SQLite REAL");
        }
    }
}
