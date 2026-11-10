//! R4-12 storage primitives. The caller owns BEGIN IMMEDIATE and its savepoint.
//! No command registration, clock, entropy, connection opening or commit occurs
//! here. Values are bound through the book's existing SQLCipher driver.
use crate::{pyjson, Refusal};
#[cfg(test)]
use kassiber_store::rusqlite::params_from_iter;
use kassiber_store::rusqlite::{
    self,
    types::{Value as Sql, ValueRef},
    Connection, ToSql,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

mod calculation;
pub(crate) use calculation::compare;
#[cfg(test)]
pub(crate) use calculation::encode as encode_typed_calculation;
pub(crate) use calculation::Encoder as CalculationEncoder;

trait Cell {
    fn value_ref(&self) -> ValueRef<'_>;
}
impl Cell for Sql {
    fn value_ref(&self) -> ValueRef<'_> {
        self.into()
    }
}
impl Cell for ValueRef<'_> {
    fn value_ref(&self) -> ValueRef<'_> {
        *self
    }
}

struct CellParam<'a>(ValueRef<'a>);
impl ToSql for CellParam<'_> {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        Ok(rusqlite::types::ToSqlOutput::Borrowed(self.0))
    }
}

pub(crate) trait BindingRow<const N: usize> {
    type Error: From<rusqlite::Error>;
    fn cell(&self, index: usize) -> ValueRef<'_>;
    fn matches(&self, old: &rusqlite::Row<'_>, reals: &[usize]) -> Result<bool, Self::Error> {
        Ok(same_row(old, self, reals)?)
    }
    fn prepare(&self) -> Result<(), Self::Error> {
        Ok(())
    }
}
impl<const N: usize, T: Cell> BindingRow<N> for [T; N] {
    type Error = rusqlite::Error;
    fn cell(&self, index: usize) -> ValueRef<'_> {
        self[index].value_ref()
    }
}

pub const ENTRY_COLUMNS: [&str; 29] = [
    "id",
    "workspace_id",
    "profile_id",
    "transaction_id",
    "wallet_id",
    "account_id",
    "occurred_at",
    "entry_type",
    "asset",
    "quantity",
    "fiat_value",
    "unit_cost",
    "cost_basis",
    "proceeds",
    "gain_loss",
    "fiat_value_exact",
    "unit_cost_exact",
    "cost_basis_exact",
    "proceeds_exact",
    "gain_loss_exact",
    "calculation_json",
    "calculation_compact",
    "pricing_source_kind",
    "pricing_quality",
    "description",
    "at_category",
    "at_kennzahl",
    "capital_gains_type",
    "created_at",
];
pub const POSTING_COLUMNS: [&str; 12] = [
    "posting_id",
    "workspace_id",
    "profile_id",
    "transaction_id",
    "observation_hash",
    "occurred_at",
    "asset",
    "location_kind",
    "location_id",
    "amount_msat",
    "state",
    "created_at",
];

/// Storage's validated compact tuples contain only arrays, strings, integers
/// and null. Write these directly, retaining the shared Python string escaper,
/// instead of allocating a second lossless JSON tree for every fragment.
fn compact_tuple(value: &Value) -> Option<String> {
    fn write(value: &Value, out: &mut String, options: &kassiber_pyjson::Options) -> Option<()> {
        match value {
            Value::Null => out.push_str("null"),
            Value::String(s) => kassiber_pyjson::dumps_str_into(s, options, out),
            Value::Number(n) if integer(value) => {
                let spelling = n.as_str();
                out.push_str(if spelling == "-0" { "0" } else { spelling });
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out, options)?;
                }
                out.push(']');
            }
            _ => return None,
        }
        Some(())
    }
    let mut out = String::new();
    write(value, &mut out, &kassiber_pyjson::Options::new().compact())?;
    Some(out)
}

/// UUIDv8, using precisely Python's compact ASCII identity preimage.
pub fn journal_entry_id(
    profile: &str,
    transaction: &str,
    wallet: &str,
    kind: &str,
    asset: &str,
    ordinal: usize,
) -> Result<String, Refusal> {
    use bitcoin_hashes::{sha256, Hash};
    use std::fmt::Write;
    let options = kassiber_pyjson::Options::new();
    let mut text = String::with_capacity(192);
    text.push('[');
    for value in [
        "kassiber-journal-entry-v1",
        profile,
        transaction,
        wallet,
        kind,
        asset,
    ] {
        if text.len() > 1 {
            text.push(',');
        }
        kassiber_pyjson::dumps_str_into(value, &options, &mut text);
    }
    write!(text, ",{ordinal}]").expect("String formatting");
    let mut digest = sha256::Hash::hash(text.as_bytes()).to_byte_array();
    digest[6] = (digest[6] & 15) | 128;
    digest[8] = (digest[8] & 63) | 128;
    let mut out = String::with_capacity(36);
    for (index, byte) in digest[..16].iter().copied().enumerate() {
        if [4, 6, 8, 10].contains(&index) {
            out.push('-');
        }
        out.push(b"0123456789abcdef"[usize::from(byte >> 4)] as char);
        out.push(b"0123456789abcdef"[usize::from(byte & 15)] as char);
    }
    Ok(out)
}

/// Rebuild references expire even when a clock repeats or moves backwards.
/// Row timestamps remain the unmodified injected clock.
pub fn journal_processed_at(candidate: &str, previous: Option<&str>) -> Result<String, Refusal> {
    use kassiber_rates::time::{format, parse};
    if let (Some(current), Some(prior)) = (parse(candidate), previous.and_then(parse)) {
        if current <= prior {
            let next = format(prior + 1)
                .ok_or_else(|| Refusal::new("OverflowError", "date value out of range"))?;
            return Ok(if next.contains('.') {
                next
            } else {
                next.replace('Z', ".000000Z")
            });
        }
    }
    Ok(candidate.into())
}

const PRICING: [&str; 12] = [
    "fiat_currency",
    "fiat_price_source",
    "fiat_rate_exact",
    "fiat_value_exact",
    "pricing_fetched_at",
    "pricing_granularity",
    "pricing_method",
    "pricing_pair",
    "pricing_provider",
    "pricing_quality",
    "pricing_source_kind",
    "pricing_timestamp",
];
fn keys(v: &Value, expected: &[&str]) -> bool {
    v.as_object()
        .is_some_and(|m| m.len() == expected.len() && expected.iter().all(|k| m.contains_key(*k)))
}
fn text(v: &Value) -> bool {
    v.is_null() || v.is_string()
}
fn integer(v: &Value) -> bool {
    v.as_number()
        .is_some_and(|n| !n.as_str().contains(['.', 'e', 'E']))
}
fn exact_integer(v: &Value, exact: &Value) -> bool {
    exact_integer_text(v, exact.as_str())
}
fn exact_integer_text(v: &Value, exact: Option<&str>) -> bool {
    v.as_number()
        .is_some_and(|n| !n.as_str().contains(['.', 'e', 'E']) && exact == Some(n.as_str()))
}
fn source(v: &Value) -> Option<Value> {
    if !keys(
        v,
        &[
            "asset",
            "crypto_fee_msat",
            "crypto_fee_msat_exact",
            "engine_event_id",
            "fiat_fee_exact",
            "occurred_at",
            "pricing",
            "spot_price_exact",
            "transaction_id",
        ],
    ) || !keys(&v["pricing"], &PRICING)
        || !exact_integer(&v["crypto_fee_msat"], &v["crypto_fee_msat_exact"])
    {
        return None;
    }
    let mut row = Vec::with_capacity(19);
    for key in [
        "transaction_id",
        "engine_event_id",
        "occurred_at",
        "asset",
        "spot_price_exact",
        "fiat_fee_exact",
    ] {
        if !text(&v[key]) {
            return None;
        }
        row.push(v[key].clone());
    }
    row.push(v["crypto_fee_msat"].clone());
    for key in PRICING {
        if !text(&v["pricing"][key]) {
            return None;
        }
        row.push(v["pricing"][key].clone());
    }
    Some(Value::Array(row))
}
fn compact(v: &Value) -> Option<Value> {
    if !keys(v, &["engine", "fragments", "method", "schema_version"])
        || !integer(&v["schema_version"])
        || v["schema_version"] != 1
        || !v["engine"].is_string()
        || !v["method"].is_string()
    {
        return None;
    }
    let mut sources = Vec::<Value>::new();
    let mut positions = HashMap::new();
    let mut fragments = Vec::new();
    for f in v["fragments"].as_array()? {
        if !keys(
            f,
            &[
                "acquisition_basis_exact",
                "acquisition_quantity_msat",
                "acquisition_quantity_msat_exact",
                "basis_authority",
                "cost_basis_exact",
                "event",
                "gain_loss_exact",
                "lot",
                "proceeds_exact",
                "quantity_msat",
                "quantity_msat_exact",
                "unit_basis_override_exact",
            ],
        ) || !exact_integer(&f["quantity_msat"], &f["quantity_msat_exact"])
        {
            return None;
        }
        if f["acquisition_quantity_msat"].is_null() {
            if !f["acquisition_quantity_msat_exact"].is_null() {
                return None;
            }
        } else if !exact_integer(
            &f["acquisition_quantity_msat"],
            &f["acquisition_quantity_msat_exact"],
        ) {
            return None;
        }
        let mut row = vec![f["quantity_msat"].clone()];
        for key in [
            "cost_basis_exact",
            "proceeds_exact",
            "gain_loss_exact",
            "unit_basis_override_exact",
            "basis_authority",
            "acquisition_basis_exact",
        ] {
            if !text(&f[key]) {
                return None;
            }
            row.push(f[key].clone());
        }
        row.push(f["acquisition_quantity_msat"].clone());
        for key in ["event", "lot"] {
            if f[key].is_null() {
                row.push(Value::Null);
                continue;
            }
            let encoded = source(&f[key])?;
            let next = sources.len();
            let index = *positions.entry(encoded.to_string()).or_insert_with(|| {
                sources.push(encoded);
                next
            });
            row.push(json!(index));
        }
        fragments.push(Value::Array(row));
    }
    Some(json!([2, v["engine"], v["method"], sources, fragments]))
}

/// `(calculation_json, calculation_compact)`, exactly as journal_calculation.py.
/// Transitional or future shapes retain the sorted legacy bytes.
pub fn encode_calculation(value: &Value) -> Result<(Option<String>, Option<String>), Refusal> {
    if !crate::truth(value) {
        return Ok((None, None));
    }
    match compact(value) {
        Some(v) => Ok((
            None,
            Some(compact_tuple(&v).expect("validated calculation tuple")),
        )),
        None => Ok((Some(pyjson(value, false)?), None)),
    }
}

fn same_row<const N: usize, T: BindingRow<N> + ?Sized>(
    old: &rusqlite::Row<'_>,
    new: &T,
    reals: &[usize],
) -> rusqlite::Result<bool> {
    for index in 0..N - 1 {
        let index = if N == 12 {
            [9, 10, 4, 3, 8, 7, 6, 5, 1, 2, 0][index]
        } else {
            index
        };
        let prior = old.get_ref(index)?;
        let value = new.cell(index);
        if prior != value {
            return Ok(false);
        }
        if let (ValueRef::Real(a), ValueRef::Real(b)) = (prior, value) {
            if a == 0.0 && reals.contains(&index) && a.is_sign_negative() != b.is_sign_negative() {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

/// Called after each successful DML statement, including deletes matching zero
/// rows. Returning an error exercises the surrounding caller's rollback path.
pub trait WriteObserver {
    fn check(&mut self) -> Result<(), Refusal> {
        Ok(())
    }
    fn phase(&mut self, _phase: &'static str) {}
    fn wrote(&mut self, table: &'static str) -> rusqlite::Result<()>;
}
impl WriteObserver for () {
    fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // Fixed SQL column layout mirrors Python's shared delta writer.
fn replace<const N: usize, T: BindingRow<N>>(
    conn: &Connection,
    profile: &str,
    rows: &[T],
    table: &'static str,
    columns: &[&str; N],
    reals: &[usize],
    tie: Option<usize>,
    observer: &mut impl WriteObserver,
) -> Result<(), T::Error> {
    let names = columns.join(", ");
    let wanted: HashMap<&str, usize> = rows
        .iter()
        .enumerate()
        .map(|(index, row)| {
            row.cell(0)
                .as_str()
                .ok()
                .map(|id| (id, index))
                .ok_or(rusqlite::Error::InvalidQuery)
        })
        .collect::<rusqlite::Result<_>>()?;
    if wanted.len() != rows.len() {
        return Err(rusqlite::Error::InvalidQuery.into());
    }
    let retained: bool = conn.query_row(
        &format!("SELECT EXISTS(SELECT 1 FROM {table} WHERE profile_id = ?)"),
        [profile],
        |row| row.get(0),
    )?;
    let time_column = if N == 29 { 6 } else { 5 };
    let mut ordered = retained
        .then(|| {
            rows.iter()
                .enumerate()
                .map(|(index, row)| {
                    Some((
                        if N == 12 {
                            ""
                        } else {
                            row.cell(time_column).as_str().ok()?
                        },
                        row.cell(0).as_str().ok()?,
                        index,
                    ))
                })
                .collect::<Option<Vec<_>>>()
        })
        .flatten();
    let mut shared_times = if tie.is_some() {
        vec![false; rows.len()]
    } else {
        Vec::new()
    };
    if let Some(ordered) = &mut ordered {
        ordered.sort_unstable_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        if tie.is_some() {
            for pair in ordered.windows(2) {
                if pair[0].0 == pair[1].0 {
                    shared_times[pair[0].2] = true;
                    shared_times[pair[1].2] = true;
                }
            }
        }
    } else {
        shared_times.fill(true);
    }
    let order = if ordered.is_some() {
        if N == 12 {
            format!(" ORDER BY {}", columns[0])
        } else {
            format!(
                " ORDER BY {}, {}, {}",
                columns[time_column],
                columns[N - 1],
                columns[0]
            )
        }
    } else {
        String::new()
    };
    let mut select = conn.prepare(&format!(
        "SELECT {names} FROM {table} WHERE profile_id = ?{order}"
    ))?;
    let mut position = 0;
    let mut changed = vec![true; rows.len()];
    let mut existing_order = Vec::new();
    let mut stamps = HashMap::<&str, (String, bool)>::new();
    {
        let mut existing = select.query([profile])?;
        while let Some(old) = existing.next()? {
            let id = old.get_ref(0)?.as_str().map_err(rusqlite::Error::from)?;
            let matched = if let (Some(ordered), Ok(time)) = (
                &ordered,
                if N == 12 {
                    Ok("")
                } else {
                    old.get_ref(time_column)?.as_str()
                },
            ) {
                while position < ordered.len()
                    && (ordered[position].0, ordered[position].1) < (time, id)
                {
                    position += 1;
                }
                if position < ordered.len()
                    && (ordered[position].0, ordered[position].1) == (time, id)
                {
                    let index = ordered[position].2;
                    position += 1;
                    Some(index)
                } else {
                    wanted.get(id).copied()
                }
            } else {
                wanted.get(id).copied()
            };
            if let Some(index) = matched {
                let new = &rows[index];
                changed[index] = !new.matches(old, reals)?;
                if let Some(tie) = tie {
                    let time = new
                        .cell(tie)
                        .as_str()
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    let created = old
                        .get_ref(N - 1)?
                        .as_str()
                        .map_err(rusqlite::Error::from)?;
                    if shared_times[index] {
                        match stamps.entry(time) {
                            std::collections::hash_map::Entry::Vacant(entry) => {
                                entry.insert((created.into(), false));
                            }
                            std::collections::hash_map::Entry::Occupied(mut entry) => {
                                if entry.get().0 != created {
                                    entry.get_mut().1 = true;
                                }
                            }
                        }
                    }
                }
                existing_order.push((Some(index), String::new()));
            } else {
                existing_order.push((None, id.to_owned()));
            }
        }
    }
    if let Some(tie) = tie.filter(|_| !retained) {
        for row in rows {
            row.cell(tie)
                .as_str()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
        }
    }
    if let Some(tie) = tie.filter(|_| retained).filter(|_| {
        changed.iter().any(|&changed| changed) || stamps.values().any(|(_, mixed)| *mixed)
    }) {
        let mut dirty_times = HashSet::new();
        for (index, row) in rows.iter().enumerate() {
            let time = row
                .cell(tie)
                .as_str()
                .map_err(|_| rusqlite::Error::InvalidQuery)?;
            if changed[index] {
                dirty_times.insert(time);
            }
        }
        dirty_times.extend(
            stamps
                .into_iter()
                .filter(|(_, (_, mixed))| *mixed)
                .map(|(time, _)| time),
        );
        for (index, row) in rows.iter().enumerate() {
            changed[index] = dirty_times.contains(row.cell(tie).as_str().expect("validated time"));
        }
    }
    for (index, row) in rows.iter().enumerate() {
        if changed[index] {
            row.prepare()?;
        }
    }
    let mut delete = conn.prepare(&format!(
        "DELETE FROM {table} WHERE profile_id = ? AND {} = ?",
        columns[0]
    ))?;
    // Python's old-row iteration order is SQLite's SELECT order. Retain it:
    // deletion order matters for trigger/fault-injection witnesses.
    for (index, removed) in existing_order {
        if index.is_none_or(|index| changed[index]) {
            let id = index.map_or(removed.as_str(), |index| {
                rows[index].cell(0).as_str().expect("validated text id")
            });
            delete.execute([profile, id])?;
            observer.wrote(table)?;
        }
    }
    let slots = vec!["?"; N].join(",");
    let mut insert = conn.prepare(&format!("INSERT INTO {table}({names}) VALUES({slots})"))?;
    let mut bound = [None; N];
    for (index, row) in rows.iter().enumerate() {
        if changed[index] {
            for (index, prior) in bound.iter_mut().enumerate() {
                let value = row.cell(index);
                let same = prior.is_some_and(|prior| match (prior, value) {
                    (ValueRef::Real(a), ValueRef::Real(b)) => a.to_bits() == b.to_bits(),
                    _ => prior == value,
                });
                if !same {
                    insert.raw_bind_parameter(index + 1, CellParam(value))?;
                    *prior = Some(value);
                }
            }
            insert.raw_execute()?;
            observer.wrote(table)?;
        }
    }
    Ok(())
}

pub fn replace_journal_entries(
    conn: &Connection,
    profile: &str,
    rows: &[[Sql; 29]],
    observer: &mut impl WriteObserver,
) -> rusqlite::Result<()> {
    replace_entry_bindings(conn, profile, rows, observer)
}

pub(crate) fn replace_entry_bindings<T: BindingRow<29>>(
    conn: &Connection,
    profile: &str,
    rows: &[T],
    observer: &mut impl WriteObserver,
) -> Result<(), T::Error> {
    replace(
        conn,
        profile,
        rows,
        "journal_entries",
        &ENTRY_COLUMNS,
        &[10, 11, 12, 13, 14],
        Some(6),
        observer,
    )
}
pub fn replace_quantity_postings(
    conn: &Connection,
    profile: &str,
    rows: &[[Sql; 12]],
    observer: &mut impl WriteObserver,
) -> rusqlite::Result<()> {
    replace(
        conn,
        profile,
        rows,
        "journal_quantity_postings",
        &POSTING_COLUMNS,
        &[],
        None,
        observer,
    )
}

pub(crate) fn replace_borrowed_postings(
    conn: &Connection,
    profile: &str,
    rows: &[[ValueRef<'_>; 12]],
    observer: &mut impl WriteObserver,
) -> rusqlite::Result<()> {
    replace(
        conn,
        profile,
        rows,
        "journal_quantity_postings",
        &POSTING_COLUMNS,
        &[],
        None,
        observer,
    )
}

/// Connection-local rebuild tuning; hold this scope until after commit/rollback.
/// SQLCipher DELETE/FULL and explicit cache-spill settings remain unchanged.
pub struct RebuildCache<'a> {
    conn: &'a Connection,
    previous: Option<i64>,
}
impl<'a> RebuildCache<'a> {
    pub fn enter(conn: &'a Connection) -> rusqlite::Result<Self> {
        let previous = conn.pragma_query_value(None, "cache_size", |r| r.get(0))?;
        conn.pragma_update(None, "cache_size", -131072)?;
        Ok(Self {
            conn,
            previous: Some(previous),
        })
    }
    pub fn restore(mut self) -> rusqlite::Result<()> {
        self.conn.pragma_update(
            None,
            "cache_size",
            self.previous.expect("active cache scope"),
        )?;
        self.previous = None;
        Ok(())
    }
}
impl Drop for RebuildCache<'_> {
    fn drop(&mut self) {
        if let Some(previous) = self.previous {
            let _ = self.conn.pragma_update(None, "cache_size", previous);
        }
    }
}

/// Disable automatic checkpoints before the caller begins its journal write.
/// Invoke `after_reply` only after COMMIT and after publishing the response.
pub struct DeferredCheckpoint<'a> {
    conn: &'a Connection,
    previous: i64,
    wal: bool,
}
impl<'a> DeferredCheckpoint<'a> {
    pub fn enter(conn: &'a Connection) -> rusqlite::Result<Self> {
        if !conn.is_autocommit() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        let wal = mode.eq_ignore_ascii_case("wal");
        let previous = conn.pragma_query_value(None, "wal_autocheckpoint", |r| r.get(0))?;
        if wal {
            conn.pragma_update(None, "wal_autocheckpoint", 0)?;
        }
        Ok(Self {
            conn,
            previous,
            wal,
        })
    }
    pub fn after_reply(self) -> rusqlite::Result<()> {
        if !self.conn.is_autocommit() {
            return Err(rusqlite::Error::InvalidQuery);
        }
        if self.wal {
            self.conn
                .query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))?;
        }
        Ok(())
    }
}
impl Drop for DeferredCheckpoint<'_> {
    fn drop(&mut self) {
        if self.wal {
            let _ = self
                .conn
                .pragma_update(None, "wal_autocheckpoint", self.previous);
        }
    }
}

#[cfg(test)]
mod tests;
