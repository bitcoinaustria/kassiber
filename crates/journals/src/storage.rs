//! R4-12 storage primitives. The caller owns BEGIN IMMEDIATE and its savepoint.
//! No command registration, clock, entropy, connection opening or commit occurs
//! here. Values are bound through the book's existing SQLCipher driver.
use crate::{pyjson, Refusal};
use kassiber_store::rusqlite::{self, params_from_iter, types::Value as Sql, Connection};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

trait SqlText {
    fn as_str(&self) -> Option<&str>;
}
impl SqlText for Sql {
    fn as_str(&self) -> Option<&str> {
        if let Self::Text(s) = self {
            Some(s)
        } else {
            None
        }
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
                let spelling = n.to_string();
                out.push_str(if spelling == "-0" { "0" } else { &spelling });
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
    let text = compact_tuple(&json!([
        "kassiber-journal-entry-v1",
        profile,
        transaction,
        wallet,
        kind,
        asset,
        ordinal
    ]))
    .expect("entry identity tuple contains only strings and an ordinal");
    let mut hex = kassiber_custody::codec::sha256(&text).as_bytes()[..32].to_vec();
    hex[12] = b'8';
    let variant = (hex[16] as char).to_digit(16).expect("SHA-256 hex");
    hex[16] = b"0123456789abcdef"[((variant & 3) | 8) as usize];
    let h = std::str::from_utf8(&hex).expect("SHA-256 ASCII");
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    ))
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
        .is_some_and(|n| !n.to_string().contains(['.', 'e', 'E']))
}
fn exact_integer(v: &Value, exact: &Value) -> bool {
    let spelling = v.to_string();
    integer(v) && exact.as_str() == Some(spelling.as_str())
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

fn same_row<const N: usize>(
    old: &rusqlite::Row<'_>,
    new: &[Sql; N],
    reals: &[usize],
) -> rusqlite::Result<bool> {
    use rusqlite::types::ValueRef;
    for (index, value) in new[..N - 1].iter().enumerate() {
        let prior = old.get_ref(index)?;
        if prior != ValueRef::from(value) {
            return Ok(false);
        }
        if let (ValueRef::Real(a), Sql::Real(b)) = (prior, value) {
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
    fn wrote(&mut self, table: &'static str) -> rusqlite::Result<()>;
}
impl WriteObserver for () {
    fn wrote(&mut self, _: &'static str) -> rusqlite::Result<()> {
        Ok(())
    }
}

#[allow(clippy::too_many_arguments)] // Fixed SQL column layout mirrors Python's shared delta writer.
fn replace<const N: usize>(
    conn: &Connection,
    profile: &str,
    rows: &[[Sql; N]],
    table: &'static str,
    columns: &[&str; N],
    reals: &[usize],
    tie: Option<usize>,
    observer: &mut impl WriteObserver,
) -> rusqlite::Result<()> {
    let names = columns.join(", ");
    let mut select = conn.prepare(&format!("SELECT {names} FROM {table} WHERE profile_id = ?"))?;
    let identifier = |row: &[Sql; N]| match &row[0] {
        Sql::Text(s) => Ok(s.clone()),
        _ => Err(rusqlite::Error::InvalidQuery),
    };
    let wanted: HashMap<&str, &[Sql; N]> = rows
        .iter()
        .map(|row| {
            row[0]
                .as_str()
                .map(|id| (id, row))
                .ok_or(rusqlite::Error::InvalidQuery)
        })
        .collect::<rusqlite::Result<_>>()?;
    if wanted.len() != rows.len() {
        return Err(rusqlite::Error::InvalidQuery);
    }
    let mut changed: HashSet<String> = wanted.keys().map(|id| (*id).to_owned()).collect();
    let mut existing_order = Vec::new();
    let mut stamps = HashMap::<String, HashSet<String>>::new();
    {
        let mut existing = select.query([profile])?;
        while let Some(old) = existing.next()? {
            let id = old.get_ref(0)?.as_str()?.to_owned();
            if let Some(new) = wanted.get(id.as_str()) {
                if same_row(old, new, reals)? {
                    changed.remove(&id);
                }
                if let Some(tie) = tie {
                    let time = new[tie].as_str().ok_or(rusqlite::Error::InvalidQuery)?;
                    stamps
                        .entry(time.into())
                        .or_default()
                        .insert(old.get(N - 1)?);
                }
            }
            existing_order.push(id);
        }
    }
    if let Some(tie) = tie {
        let mut dirty_times = HashSet::new();
        for row in rows {
            let time = row[tie]
                .as_str()
                .ok_or(rusqlite::Error::InvalidQuery)?
                .to_owned();
            if changed.contains(row[0].as_str().expect("validated text id")) {
                dirty_times.insert(time.clone());
            }
        }
        dirty_times.extend(
            stamps
                .into_iter()
                .filter(|(_, s)| s.len() > 1)
                .map(|(t, _)| t),
        );
        changed = rows
            .iter()
            .filter(|row| dirty_times.contains(row[tie].as_str().expect("validated time")))
            .map(identifier)
            .collect::<rusqlite::Result<_>>()?;
    }
    let mut delete = conn.prepare(&format!(
        "DELETE FROM {table} WHERE profile_id = ? AND {} = ?",
        columns[0]
    ))?;
    // Python's old-row iteration order is SQLite's SELECT order. Retain it:
    // deletion order matters for trigger/fault-injection witnesses.
    for id in existing_order {
        if !wanted.contains_key(id.as_str()) || changed.contains(&id) {
            delete.execute([profile, &id])?;
            observer.wrote(table)?;
        }
    }
    let slots = vec!["?"; N].join(",");
    let mut insert = conn.prepare(&format!("INSERT INTO {table}({names}) VALUES({slots})"))?;
    for row in rows {
        if changed.contains(row[0].as_str().expect("validated text id")) {
            insert.execute(params_from_iter(row))?;
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

/// Connection-local rebuild tuning; hold this scope until after commit/rollback.
/// SQLCipher DELETE/FULL and explicit cache-spill settings remain unchanged.
pub struct RebuildCache<'a> {
    conn: &'a Connection,
    previous: i64,
}
impl<'a> RebuildCache<'a> {
    pub fn enter(conn: &'a Connection) -> rusqlite::Result<Self> {
        let previous = conn.pragma_query_value(None, "cache_size", |r| r.get(0))?;
        conn.pragma_update(None, "cache_size", -131072)?;
        Ok(Self { conn, previous })
    }
    pub fn restore(self) -> rusqlite::Result<()> {
        self.conn.pragma_update(None, "cache_size", self.previous)
    }
}
impl Drop for RebuildCache<'_> {
    fn drop(&mut self) {
        let _ = self.conn.pragma_update(None, "cache_size", self.previous);
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
