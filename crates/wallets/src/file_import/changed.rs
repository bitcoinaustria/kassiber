//! Re-importing a newer export (docs/reference/imports.md#re-importing-a-newer-export).
//!
//! A row that matches no stored row is normally new. Two kinds of unmatched
//! rows look like a stored row instead, and neither is merged by guessing:
//!
//! - **re-identified**: the same date, direction, asset, amount and fee as a
//!   row an earlier run of the same file format stored in this wallet, under
//!   another id. An optional id column that appeared or disappeared, or a
//!   changed date format behind a synthetic id, re-identifies every row.
//! - **corrected**: the id, direction and asset of a stored row, with another
//!   amount or fee: the provider corrected the row, or it is a second row
//!   under that id.
//!
//! Without the user's choice (`changed_rows`) the import refuses and writes
//! nothing. `skip` keeps what the book has and imports the other rows;
//! `import` stores them as separate transactions. A stored row answers for
//! at most one file row, and only rows stored before this import count.
//!
//! Every identical copy of a row in the export gets the same answer. A
//! `skip` is remembered for that stored row and that exact incoming row (the
//! wallet's `changed_rows_kept`), so the same difference does not ask again
//! on the next import or sync; a new difference does, and so does the same
//! one once the import run that recorded the answer is rolled back.
//!
//! A matched row whose provider values differ where the import never
//! overwrites them (a fee an exchange price relaxed, a date, a kind) is
//! reported in `corrections_ignored`, as is the amount and fee of a corrected
//! row the user chose to skip.
use super::*;
use std::collections::BTreeSet;

/// The user's answer to rows that match no stored row but look like one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangedRows {
    /// Keep the stored rows; the export's copies are not imported.
    Skip,
    /// The rows are separate transactions; import them.
    Import,
}

impl ChangedRows {
    pub fn parse(value: &str) -> Result<Self, AppError> {
        match kassiber_pytext::lower(kassiber_pytext::strip(value)).as_str() {
            "skip" => Ok(Self::Skip),
            "import" => Ok(Self::Import),
            _ => Err(AppError::new(
                "validation",
                "changed_rows must be 'skip' or 'import'",
            )),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Import => "import",
        }
    }
}

/// What a re-import screens: the format whose earlier runs a re-identified
/// row is compared with (`None` for an API sync, whose ids are the
/// exchange's own), and the user's answer, if any.
pub(super) struct Screen<'a> {
    pub format: Option<&'a str>,
    pub changed_rows: Option<ChangedRows>,
    /// An exchange API sync rather than a file import (the refusal's advice).
    pub sync: bool,
}

#[derive(Clone)]
pub(super) enum Conflict {
    Reidentified(Fields),
    Corrected(Fields),
}

impl Conflict {
    pub fn stored(&self) -> &Fields {
        match self {
            Conflict::Reidentified(stored) | Conflict::Corrected(stored) => stored,
        }
    }
}

/// A conflict and whether an earlier `skip` already answered it.
#[derive(Clone)]
pub(super) struct Found {
    pub conflict: Conflict,
    pub remembered: bool,
    /// The incoming row's identity and values (`incoming_key`).
    pub key: String,
}

/// The wallet config key holding remembered `skip` answers:
/// `{stored transaction id: [{"key": incoming_key, "batch": import run}, ...]}`.
/// An answer stands while its run is in the book (`batch` null: the run
/// stored no row, so there is nothing to roll back).
pub const KEPT: &str = "changed_rows_kept";
/// Remembered answers per stored row; older ones are dropped first.
const KEPT_PER_ROW: usize = 16;

/// The incoming row's identity and values, as one digest.
pub(super) fn incoming_key(r: &Record) -> String {
    let value = json!([
        string(&r.fields, "external_id"),
        string(&r.fields, "occurred_at"),
        string(&r.fields, "direction"),
        string(&r.fields, "asset"),
        integer_or_zero(r.fields.get("amount")),
        integer_or_zero(r.fields.get("fee")),
    ]);
    Sha256::digest(value.to_string().as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn wallet_config(conn: &Connection, wallet: &str) -> Result<Json, AppError> {
    let stored: Option<String> = conn
        .query_row(
            "SELECT config_json FROM wallets WHERE id=?",
            [wallet],
            |r| r.get(0),
        )
        .map_err(database_error)?;
    Ok(stored
        .and_then(|text| serde_json::from_str(&text).ok())
        .filter(Json::is_object)
        .unwrap_or_else(|| json!({})))
}

/// Remembers `skip` answers (stored transaction id, incoming key) on the
/// wallet, each bound to `batch`, the import run that recorded it (`None`
/// when the run stored no row and left no run to roll back); the caller's
/// write transaction holds it. Answers whose run was rolled back go.
pub(super) fn remember(
    conn: &Connection,
    wallet: &str,
    answers: &[(String, String)],
    batch: Option<&str>,
) -> Result<(), AppError> {
    if answers.is_empty() {
        return Ok(());
    }
    let mut config = wallet_config(conn, wallet)?;
    let live = live_batches(conn, config.get(KEPT))?;
    let kept = config
        .as_object_mut()
        .expect("object config")
        .entry(KEPT)
        .or_insert_with(|| json!({}));
    if !kept.is_object() {
        *kept = json!({});
    }
    for (id, key) in answers {
        let keys = kept
            .as_object_mut()
            .expect("object")
            .entry(id.clone())
            .or_insert_with(|| json!([]));
        if !keys.is_array() {
            *keys = json!([]);
        }
        let list = keys.as_array_mut().expect("array");
        list.retain(|k| kept_key(k, &live).is_some_and(|k| k != key));
        list.push(json!({"key": key, "batch": batch}));
        let excess = list.len().saturating_sub(KEPT_PER_ROW);
        list.drain(..excess);
    }
    conn.execute(
        "UPDATE wallets SET config_json=? WHERE id=?",
        params![config::stored_json(&config), wallet],
    )
    .map_err(database_error)?;
    Ok(())
}

/// The import runs that remembered answers name and the book still has.
fn live_batches(conn: &Connection, kept: Option<&Json>) -> Result<BTreeSet<String>, AppError> {
    let named: BTreeSet<&str> = kept
        .and_then(Json::as_object)
        .into_iter()
        .flat_map(|rows| rows.values())
        .filter_map(Json::as_array)
        .flatten()
        .filter_map(|k| k.get("batch").and_then(Json::as_str))
        .collect();
    let mut live = BTreeSet::new();
    for batch in named {
        let found: Option<i64> = conn
            .query_row("SELECT 1 FROM import_batches WHERE id=?", [batch], |r| {
                r.get(0)
            })
            .optional()
            .map_err(database_error)?;
        if found.is_some() {
            live.insert(batch.to_owned());
        }
    }
    Ok(live)
}

/// A remembered answer's key while it stands: its run is still in the book,
/// or it was recorded by a run that stored nothing.
fn kept_key<'k>(entry: &'k Json, live: &BTreeSet<String>) -> Option<&'k str> {
    let key = entry.get("key")?.as_str()?;
    match entry.get("batch")? {
        Json::Null => Some(key),
        Json::String(batch) if live.contains(batch) => Some(key),
        _ => None,
    }
}

/// The refusal's hint, shared by the CLI and the desktop.
pub const HINT: &str = "Nothing was imported. Run the import again with --changed-rows skip to keep what the book has and import only the other rows, or with --changed-rows import if they are separate transactions. To take the export's values instead, roll back the earlier import (kassiber imports list, then imports rollback) and import this file again.";
/// [`HINT`] for an exchange API sync (`wallets sync-<kind>`).
pub const SYNC_HINT: &str = "Nothing was imported. Run the sync again with --changed-rows skip to keep what the book has and import only the other rows, or with --changed-rows import if they are separate transactions. To take the exchange's values instead, roll back the earlier sync (kassiber imports list, then imports rollback) and sync again.";

fn rows(
    conn: &Connection,
    sql: &str,
    params: impl kassiber_store::rusqlite::Params,
) -> Result<Vec<Fields>, AppError> {
    let mut statement = conn.prepare(sql).map_err(database_error)?;
    let columns = statement
        .column_names()
        .iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>();
    let found = statement
        .query_map(params, |row| {
            columns
                .iter()
                .enumerate()
                .map(|(i, k)| Ok((k.clone(), row.get(i)?)))
                .collect::<Result<Fields, _>>()
        })
        .map_err(database_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(database_error)?;
    Ok(found)
}

/// Stored rows of this wallet under the row's id, direction and asset.
fn same_id(conn: &Connection, wallet: &str, r: &Record) -> Result<Vec<Fields>, AppError> {
    rows(
        conn,
        "SELECT * FROM transactions WHERE wallet_id=? AND external_id=? AND direction=? AND asset=? ORDER BY created_at, id",
        params![
            wallet,
            string(&r.fields, "external_id"),
            r.fields["direction"],
            r.fields["asset"],
        ],
    )
}

/// Rows an earlier run of `format` stored in this wallet with the row's
/// date, direction, asset, amount and fee under another id.
fn same_economics(
    conn: &Connection,
    wallet: &str,
    format: &str,
    r: &Record,
) -> Result<Vec<Fields>, AppError> {
    rows(
        conn,
        "SELECT DISTINCT t.* FROM transactions t \
         JOIN import_batch_transactions bt ON bt.transaction_id=t.id \
         JOIN import_batches b ON b.id=bt.batch_id \
         WHERE t.wallet_id=? AND b.source_format=? AND t.occurred_at=? AND t.direction=? \
           AND t.asset=? AND t.amount=? AND t.fee=? AND COALESCE(t.external_id,'')!=? \
         ORDER BY t.created_at, t.id",
        params![
            wallet,
            format,
            r.fields["occurred_at"],
            r.fields["direction"],
            r.fields["asset"],
            r.fields["amount"],
            r.fields["fee"],
            string(&r.fields, "external_id"),
        ],
    )
}

/// The rows of this import that look like an unclaimed stored row, keyed by
/// their index. `matched` holds, per record, the stored row its ordinary
/// lookup matched (`Some(None)` for a row matched without a stored id at
/// hand), or `None` for an unmatched row.
pub(super) fn conflicts(
    conn: &Connection,
    wallet: &str,
    records: &[Record],
    matched: &[Option<Option<String>>],
    screen: &Screen<'_>,
) -> Result<BTreeMap<usize, Found>, AppError> {
    let mut claimed: BTreeSet<String> = matched.iter().flatten().flatten().cloned().collect();
    let kept = wallet_config(conn, wallet)?
        .get(KEPT)
        .cloned()
        .unwrap_or(Json::Null);
    // An answer whose import run was rolled back asks again.
    let live = live_batches(conn, Some(&kept))?;
    let remembered = |conflict: &Conflict, key: &str| {
        kept.get(string(conflict.stored(), "id"))
            .and_then(Json::as_array)
            .is_some_and(|keys| keys.iter().any(|k| kept_key(k, &live) == Some(key)))
    };
    let mut found = BTreeMap::new();
    // Every identical copy of an incoming row gets the first copy's answer.
    let mut by_key: BTreeMap<String, Found> = BTreeMap::new();
    for (index, r) in records.iter().enumerate() {
        if matched[index].is_some() {
            continue;
        }
        let key = incoming_key(r);
        if let Some(first) = by_key.get(&key) {
            found.insert(index, first.clone());
            continue;
        }
        let mut record = |index: usize, conflict: Conflict| {
            let item = Found {
                remembered: remembered(&conflict, &key),
                conflict,
                key: key.clone(),
            };
            by_key.insert(key.clone(), item.clone());
            found.insert(index, item);
        };
        let mut free = |candidates: Vec<Fields>| {
            let row = candidates
                .into_iter()
                .find(|row| !claimed.contains(&string(row, "id")));
            if let Some(row) = &row {
                claimed.insert(string(row, "id"));
            }
            row
        };
        if !string(&r.fields, "external_id").is_empty() {
            if let Some(row) = free(same_id(conn, wallet, r)?) {
                record(index, Conflict::Corrected(row));
                continue;
            }
        }
        if let Some(format) = screen.format {
            if let Some(row) = free(same_economics(conn, wallet, format, r)?) {
                record(index, Conflict::Reidentified(row));
            }
        }
    }
    Ok(found)
}

fn btc(value: Option<&Sql>) -> String {
    msat_to_btc(integer_or_zero(value))
}

/// One `corrections_ignored` entry.
fn entry(stored: &Fields, field: &str, kept: String, file: String) -> Py {
    dict([
        ("transaction_id", s(string(stored, "id"))),
        ("external_id", s(string(stored, "external_id"))),
        ("field", s(field)),
        ("stored", s(kept)),
        ("file", s(file)),
    ])
}

/// The provider values of `r` the stored row keeps: the amount and fee of a
/// skipped correction, and for a matched row under the same id any fee,
/// date or kind the import did not write (`changes`).
pub(super) fn ignored(stored: &Fields, r: &Record, changes: Option<&Fields>) -> Vec<Py> {
    let incoming = &r.fields;
    let written = |key: &str| changes.is_some_and(|c| c.contains_key(key));
    let external = string(incoming, "external_id");
    if changes.is_some() && (external.is_empty() || external != string(stored, "external_id")) {
        return vec![];
    }
    let mut out = vec![];
    for key in ["amount", "fee"] {
        if changes.is_none() || key == "fee" {
            let (old, new) = (
                integer_or_zero(stored.get(key)),
                integer_or_zero(incoming.get(key)),
            );
            if old != new && !written(key) {
                out.push(entry(
                    stored,
                    key,
                    btc(stored.get(key)),
                    btc(incoming.get(key)),
                ));
            }
        }
    }
    let (old, new) = (
        string(stored, "occurred_at"),
        string(incoming, "occurred_at"),
    );
    if !new.is_empty() && old != new && !written("occurred_at") {
        out.push(entry(stored, "occurred_at", old, new));
    }
    let (old, new) = (string(stored, "kind"), string(incoming, "kind"));
    if !old.is_empty() && !new.is_empty() && old != new && !written("kind") {
        out.push(entry(stored, "kind", old, new));
    }
    out
}

/// The refusal for rows the user has not decided on.
pub(super) fn refusal(records: &[Record], found: &BTreeMap<usize, Found>, sync: bool) -> AppError {
    // Rows an earlier answer covers do not ask again.
    let found: BTreeMap<usize, Conflict> = found
        .iter()
        .filter(|(_, f)| !f.remembered)
        .map(|(i, f)| (*i, f.conflict.clone()))
        .collect();
    let found = &found;
    let reidentified = found
        .values()
        .filter(|c| matches!(c, Conflict::Reidentified(_)))
        .count();
    let corrected = found.len() - reidentified;
    let plural = |n: usize, one: &str, many: &str| {
        if n == 1 {
            format!("1 {one}")
        } else {
            format!("{n} {many}")
        }
    };
    let mut parts = vec![];
    if reidentified > 0 {
        parts.push(format!(
            "this export identifies {} differently from the earlier import (same date, direction, asset, amount and fee as a stored row under another id)",
            plural(reidentified, "row", "rows")
        ));
    }
    if corrected > 0 {
        let first = found.iter().find_map(|(index, c)| match c {
            Conflict::Corrected(stored) => Some((&records[*index], stored)),
            Conflict::Reidentified(_) => None,
        });
        let example = first
            .map(|(r, stored)| {
                let field = if integer_or_zero(stored.get("amount"))
                    != integer_or_zero(r.fields.get("amount"))
                {
                    "amount"
                } else {
                    "fee"
                };
                format!(
                    "; first '{}': {} {} in the book, {} in the file",
                    string(stored, "external_id"),
                    field,
                    btc(stored.get(field)),
                    btc(r.fields.get(field)),
                )
            })
            .unwrap_or_default();
        parts.push(format!(
            "{} the id of a stored row with another amount or fee{example}",
            plural(corrected, "row has", "rows have")
        ));
    }
    let mut message = parts.join("; ");
    if let Some(first) = message.get(..1) {
        message = first.to_uppercase() + &message[1..];
    }
    let shown = found
        .iter()
        .take(20)
        .map(|(index, c)| {
            let (kind, stored) = match c {
                Conflict::Reidentified(stored) => ("reidentified", stored),
                Conflict::Corrected(stored) => ("corrected", stored),
            };
            let r = &records[*index];
            dict([
                ("row", Py::from(*index as i64 + 1)),
                ("kind", s(kind)),
                ("external_id", s(string(&r.fields, "external_id"))),
                ("transaction_id", s(string(stored, "id"))),
                ("stored_external_id", s(string(stored, "external_id"))),
                ("occurred_at", s(string(&r.fields, "occurred_at"))),
                ("stored_amount", s(btc(stored.get("amount")))),
                ("file_amount", s(btc(r.fields.get("amount")))),
                ("stored_fee", s(btc(stored.get("fee")))),
                ("file_fee", s(btc(r.fields.get("fee")))),
            ])
        })
        .collect();
    AppError::new("import_changed_rows", message)
        .with_hint(if sync { SYNC_HINT } else { HINT })
        .with_details(dict([
            ("reidentified", Py::from(reidentified as i64)),
            ("corrected", Py::from(corrected as i64)),
            ("rows", Py::List(shown)),
        ]))
}

/// The result's `changed_rows` block, when the import met any.
pub(super) fn summary(
    choice: Option<ChangedRows>,
    reidentified: i64,
    corrected: i64,
) -> Option<Py> {
    let choice = choice.filter(|_| reidentified + corrected > 0)?;
    Some(dict([
        ("action", s(choice.as_str())),
        ("reidentified", Py::from(reidentified)),
        ("corrected", Py::from(corrected)),
    ]))
}

/// Numbers identical ID-less rows of a generic file (docs/reference/imports.md#generic-transaction-imports):
/// the first occurrence keeps the identity earlier builds gave every copy,
/// so re-importing an already imported file adds only the copies they
/// merged; later ones become `occurrence:<n>`.
pub(super) fn number_occurrences(records: &mut [Record]) {
    let mut seen: BTreeMap<String, i64> = BTreeMap::new();
    for r in records.iter_mut() {
        if r.deferred.is_some()
            || r.fingerprint_identity.is_some()
            || !string(&r.fields, "external_id").is_empty()
        {
            continue;
        }
        let n = seen.entry(fingerprint("", r)).or_default();
        *n += 1;
        if *n > 1 {
            r.fingerprint_identity = Some(format!("occurrence:{n}"));
        }
    }
}
