//! Journal reproducibility (docs/reference/replay-bundle.md): recompute a
//! book's journals in a private in-memory copy and compare them with the
//! stored ones in a bounded diff. Nothing here writes to the caller's book.
use crate::{pipeline, process, service, Refusal};
use kassiber_store::rusqlite::{types::Value as Sql, Connection};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Differences listed when the caller names no limit.
pub const DEFAULT_LIMIT: usize = 20;
/// The most differences one report lists; counts always cover every row.
pub const MAX_LIMIT: usize = 200;
/// Displayed text values longer than this are cut.
const DISPLAY_CHARS: usize = 160;

/// One compared value. REAL compares by bit pattern.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Cell {
    Null,
    Int(i64),
    Real(u64),
    Text(String),
    Blob(Vec<u8>),
}

impl Cell {
    pub fn real(value: f64) -> Self {
        Self::Real(value.to_bits())
    }

    pub fn text(value: impl Into<String>) -> Self {
        Self::Text(value.into())
    }

    fn from_sql(value: Sql) -> Self {
        match value {
            Sql::Null => Self::Null,
            Sql::Integer(n) => Self::Int(n),
            Sql::Real(n) => Self::real(n),
            Sql::Text(text) => Self::Text(text),
            Sql::Blob(bytes) => Self::Blob(bytes),
        }
    }

    /// The exact JSON encoding of a cell: REAL as its shortest repr, BLOB as
    /// hex.
    pub fn to_json(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Int(n) => json!(n),
            Self::Real(bits) => json!({"real": kassiber_pyjson::float_repr(f64::from_bits(*bits))}),
            Self::Text(text) => json!(text),
            Self::Blob(bytes) => json!({"blob": hex(bytes)}),
        }
    }

    /// Two differing values for a report: long texts are cut to a window
    /// around their first difference, so the change stays visible.
    fn display_pair(a: Option<&Self>, b: Option<&Self>) -> (Value, Value) {
        let show = |cell: Option<&Self>| cell.map_or(Value::Null, Self::display);
        let (Some(Self::Text(a)), Some(Self::Text(b))) = (a, b) else {
            return (show(a), show(b));
        };
        if a.chars().count() <= DISPLAY_CHARS && b.chars().count() <= DISPLAY_CHARS {
            return (json!(a), json!(b));
        }
        let first = a
            .chars()
            .zip(b.chars())
            .position(|(x, y)| x != y)
            .unwrap_or_else(|| a.chars().count().min(b.chars().count()));
        let start = first.saturating_sub(DISPLAY_CHARS / 4);
        let window = |text: &str| {
            let total = text.chars().count();
            let body: String = text.chars().skip(start).take(DISPLAY_CHARS).collect();
            let prefix = if start > 0 { "…" } else { "" };
            let suffix = if start + DISPLAY_CHARS < total {
                "…"
            } else {
                ""
            };
            json!(format!("{prefix}{body}{suffix}"))
        };
        (window(a), window(b))
    }

    /// A bounded, human-readable rendering for diff reports.
    fn display(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Int(n) => json!(n),
            Self::Real(bits) => {
                let value = f64::from_bits(*bits);
                if value.is_finite() {
                    json!(value)
                } else {
                    json!(kassiber_pyjson::float_repr(value))
                }
            }
            Self::Text(text) if text.chars().count() > DISPLAY_CHARS => {
                json!(format!(
                    "{}…",
                    text.chars().take(DISPLAY_CHARS).collect::<String>()
                ))
            }
            Self::Text(text) => json!(text),
            Self::Blob(bytes) => json!(format!("x'{}'", hex(&bytes[..bytes.len().min(80)]))),
        }
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// One compared row: its key within the table, the context a report groups
/// by, and the compared columns.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub key: String,
    pub transaction_id: Option<String>,
    pub wallet_id: Option<String>,
    pub year: Option<String>,
    pub columns: BTreeMap<String, Cell>,
}

/// One compared table. Rows are sorted by key; a key repeated inside one
/// table gets an occurrence suffix (`#2`, `#3`, …) in content order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub name: String,
    pub rows: Vec<Row>,
}

impl Table {
    pub fn new(name: impl Into<String>, mut rows: Vec<Row>) -> Self {
        rows.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.columns.cmp(&b.columns)));
        let mut seen = BTreeMap::<String, usize>::new();
        for row in &mut rows {
            let count = seen.entry(row.key.clone()).or_default();
            *count += 1;
            if *count > 1 {
                row.key = format!("{}#{count}", row.key);
            }
        }
        Self {
            name: name.into(),
            rows,
        }
    }
}

/// The bounded comparison of two sets of tables. `differing` and the
/// groupings count every difference; `differences` lists the first `limit`.
#[derive(Debug, Default)]
pub struct Diff {
    pub tables: Vec<Value>,
    pub differences: Vec<Value>,
    pub differing: usize,
    pub by_wallet: BTreeMap<String, usize>,
    pub by_year: BTreeMap<String, usize>,
    pub limit: usize,
}

impl Diff {
    pub fn identical(&self) -> bool {
        self.differing == 0
    }

    /// The report fields of a comparison.
    pub fn fields(&self) -> Map<String, Value> {
        let mut out = Map::new();
        out.insert("limit".into(), json!(self.limit));
        out.insert("differing".into(), json!(self.differing));
        out.insert(
            "truncated".into(),
            json!(self.differing > self.differences.len()),
        );
        out.insert("tables".into(), json!(self.tables));
        out.insert("by_wallet".into(), json!(self.by_wallet));
        out.insert("by_year".into(), json!(self.by_year));
        out.insert("differences".into(), json!(self.differences));
        out
    }
}

/// Clamp a requested difference limit to `1..=MAX_LIMIT`.
pub fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// Compare `stored` with `recomputed`, table by table and key by key.
pub fn diff(stored: &[Table], recomputed: &[Table], limit: usize) -> Diff {
    let mut report = Diff {
        limit,
        ..Default::default()
    };
    let mut names: Vec<&str> = stored.iter().map(|t| t.name.as_str()).collect();
    for table in recomputed {
        if !names.contains(&table.name.as_str()) {
            names.push(&table.name);
        }
    }
    let empty = Vec::new();
    for name in names {
        let rows = |tables: &'_ [Table]| -> BTreeMap<String, Row> {
            tables
                .iter()
                .find(|t| t.name == name)
                .map_or(&empty, |t| &t.rows)
                .iter()
                .map(|row| (row.key.clone(), row.clone()))
                .collect()
        };
        let (before, after) = (rows(stored), rows(recomputed));
        let keys: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
        let mut differing = 0usize;
        for key in keys {
            let (change, context, columns) = match (before.get(key), after.get(key)) {
                (Some(a), Some(b)) if a.columns == b.columns => continue,
                (Some(a), Some(b)) => {
                    let names: BTreeSet<&String> =
                        a.columns.keys().chain(b.columns.keys()).collect();
                    let columns: Vec<Value> = names
                        .into_iter()
                        .filter(|column| a.columns.get(*column) != b.columns.get(*column))
                        .map(|column| {
                            let (stored, recomputed) =
                                Cell::display_pair(a.columns.get(column), b.columns.get(column));
                            json!({"column": column, "stored": stored, "recomputed": recomputed})
                        })
                        .collect();
                    ("changed", a, Some(columns))
                }
                (Some(a), None) => ("missing", a, None),
                (None, Some(b)) => ("unexpected", b, None),
                (None, None) => unreachable!("key from one side"),
            };
            differing += 1;
            report.differing += 1;
            if let Some(wallet) = &context.wallet_id {
                *report.by_wallet.entry(wallet.clone()).or_default() += 1;
            }
            if let Some(year) = &context.year {
                *report.by_year.entry(year.clone()).or_default() += 1;
            }
            if report.differences.len() < limit {
                let mut item = Map::new();
                item.insert("table".into(), json!(name));
                item.insert("change".into(), json!(change));
                item.insert("key".into(), json!(key));
                for (field, value) in [
                    ("transaction_id", &context.transaction_id),
                    ("wallet_id", &context.wallet_id),
                    ("year", &context.year),
                ] {
                    if let Some(value) = value {
                        item.insert(field.into(), json!(value));
                    }
                }
                if let Some(columns) = columns {
                    item.insert("columns".into(), Value::Array(columns));
                }
                report.differences.push(Value::Object(item));
            }
        }
        report.tables.push(json!({
            "table": name,
            "stored": before.len(),
            "recomputed": after.len(),
            "differing": differing,
        }));
    }
    report
}

fn year_of(cell: Option<&Cell>) -> Option<String> {
    match cell? {
        Cell::Text(text) => {
            let year: String = text.chars().take(4).collect();
            (year.len() == 4 && year.chars().all(|c| c.is_ascii_digit())).then_some(year)
        }
        Cell::Int(n) => Some(n.to_string()),
        _ => None,
    }
}

fn text_of(cell: Option<&Cell>) -> Option<String> {
    match cell? {
        Cell::Text(text) => Some(text.clone()),
        Cell::Int(n) => Some(n.to_string()),
        _ => None,
    }
}

fn key_of(columns: &BTreeMap<String, Cell>, key: &[&str]) -> String {
    key.iter()
        .map(|column| match columns.get(*column) {
            None | Some(Cell::Null) => String::new(),
            Some(Cell::Text(text)) => text.clone(),
            Some(other) => other.display().to_string(),
        })
        .collect::<Vec<_>>()
        .join("|")
}

/// A stored journal table and how the check reads it.
struct Spec {
    table: &'static str,
    key: &'static [&'static str],
    /// Columns that carry no computed fact: random row ids, clocks.
    drop: &'static [&'static str],
    transaction: Option<&'static str>,
    wallet: Option<&'static str>,
    date: Option<&'static str>,
}

const SPECS: &[Spec] = &[
    Spec {
        table: "journal_entries",
        key: &["id"],
        drop: &[],
        transaction: Some("transaction_id"),
        wallet: Some("wallet_id"),
        date: Some("occurred_at"),
    },
    Spec {
        table: "journal_tax_summary",
        key: &["year", "asset", "transaction_type", "capital_gains_type"],
        drop: &["id"],
        transaction: None,
        wallet: None,
        date: Some("year"),
    },
    Spec {
        table: "journal_wallet_holdings",
        key: &["wallet_id", "account_code", "asset"],
        drop: &["id"],
        transaction: None,
        wallet: Some("wallet_id"),
        date: None,
    },
    Spec {
        table: "journal_account_holdings",
        key: &["account_id", "account_code", "asset"],
        drop: &["id"],
        transaction: None,
        wallet: None,
        date: None,
    },
    Spec {
        table: "journal_quantity_balances",
        key: &["location_kind", "location_id", "asset"],
        drop: &[],
        transaction: None,
        wallet: None,
        date: None,
    },
    Spec {
        table: "journal_quantity_postings",
        key: &["posting_id"],
        drop: &[],
        transaction: Some("transaction_id"),
        wallet: None,
        date: Some("occurred_at"),
    },
    Spec {
        table: "journal_quantity_issues",
        key: &["issue_id"],
        drop: &[],
        transaction: None,
        wallet: None,
        date: Some("occurred_at"),
    },
    Spec {
        table: "journal_custody_decisions",
        key: &["decision_id"],
        drop: &[],
        transaction: Some("source_transaction_id"),
        wallet: Some("source_wallet_id"),
        date: Some("occurred_at"),
    },
    Spec {
        table: "journal_custody_economic_relations",
        key: &["relation_id"],
        drop: &[],
        transaction: Some("source_transaction_id"),
        wallet: None,
        date: Some("occurred_at"),
    },
    Spec {
        table: "journal_custody_reconciliations",
        key: &["component_id"],
        drop: &[],
        transaction: None,
        wallet: None,
        date: None,
    },
    Spec {
        table: "journal_custody_gap_inputs",
        key: &[],
        drop: &[],
        transaction: None,
        wallet: None,
        date: None,
    },
    Spec {
        table: "journal_quarantines",
        key: &["transaction_id"],
        drop: &[],
        transaction: Some("transaction_id"),
        wallet: None,
        date: None,
    },
];

/// Columns no table compares: clocks and the constant book scope.
const ALWAYS_DROPPED: &[&str] = &["created_at", "workspace_id", "profile_id"];

/// The processing facts a run stamps on the profile row, without its clock.
const PROFILE_COLUMNS: &str = "last_processed_tx_count,last_processed_input_version,journal_semantics_version,required_journal_semantics_version,ownership_review_counts_json";

fn select(
    conn: &Connection,
    sql: &str,
    profile_id: &str,
) -> Result<Vec<BTreeMap<String, Cell>>, Refusal> {
    let mut statement = conn.prepare(sql)?;
    let names: Vec<String> = statement
        .column_names()
        .into_iter()
        .map(str::to_owned)
        .collect();
    let rows = statement.query_map([profile_id], |row| {
        (0..names.len())
            .map(|i| Ok((names[i].clone(), Cell::from_sql(row.get::<_, Sql>(i)?))))
            .collect::<Result<BTreeMap<_, _>, _>>()
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The stored journal outputs of one profile, as compared tables.
pub fn stored_tables(conn: &Connection, profile_id: &str) -> Result<Vec<Table>, Refusal> {
    let mut tables = Vec::with_capacity(SPECS.len() + 1);
    for spec in SPECS {
        let rows = select(
            conn,
            &format!("SELECT * FROM {} WHERE profile_id = ?", spec.table),
            profile_id,
        )?
        .into_iter()
        .map(|mut columns| {
            for column in ALWAYS_DROPPED.iter().chain(spec.drop) {
                columns.remove(*column);
            }
            let wallet = match spec.wallet {
                Some(column) => text_of(columns.get(column)),
                None if spec.table == "journal_quantity_balances"
                    && columns.get("location_kind") == Some(&Cell::text("wallet")) =>
                {
                    text_of(columns.get("location_id"))
                }
                None => None,
            };
            Row {
                key: key_of(&columns, spec.key),
                transaction_id: spec.transaction.and_then(|c| text_of(columns.get(c))),
                wallet_id: wallet,
                year: spec.date.and_then(|c| year_of(columns.get(c))),
                columns,
            }
        })
        .collect();
        tables.push(Table::new(spec.table, rows));
    }
    let profile = select(
        conn,
        &format!("SELECT {PROFILE_COLUMNS} FROM profiles WHERE id = ?"),
        profile_id,
    )?
    .into_iter()
    .map(|columns| Row {
        key: "profile".into(),
        transaction_id: None,
        wallet_id: None,
        year: None,
        columns,
    })
    .collect();
    tables.push(Table::new("profiles", profile));
    Ok(tables)
}

/// The processing payload fields that mean the run first changed inputs.
const INPUT_CHANGES: &[&str] = &[
    "auto_priced",
    "custody_pair_evidence_repairs",
    "source_overlap_repair",
];

/// The `INPUT_CHANGES` fields a processing payload shows as changed. The
/// overlap repair object also reports overlaps it left alone (skipped
/// records, unresolved address lists); only its removal and exclusion
/// counts mean the run changed inputs, or a book with an overlap no run
/// can repair would read stale forever.
fn input_changes(payload: &Value) -> Vec<&'static str> {
    let nonzero = |value: &Value| match value {
        Value::Null => false,
        Value::Number(n) => n.as_i64() != Some(0),
        _ => true,
    };
    INPUT_CHANGES
        .iter()
        .copied()
        .filter(|field| match (*field, &payload[*field]) {
            ("source_overlap_repair", Value::Object(repair)) => {
                ["addresses_removed", "duplicates_excluded"]
                    .iter()
                    .any(|count| repair.get(*count).is_some_and(nonzero))
            }
            (_, value) => nonzero(value),
        })
        .collect()
}

enum Recomputed {
    Tables(Vec<Table>),
    Stale(String),
    Refused(Refusal),
}

/// Process a fresh in-memory copy of `pristine` and read its tables.
fn recompute(
    pristine: &Connection,
    workspace_id: &str,
    profile_id: &str,
    sources: &impl kassiber_transfers::Sources,
) -> Result<Recomputed, Refusal> {
    let copy = pipeline::memory_snapshot(pristine, || random_key(pristine))?;
    let (workspace, profile) = (
        kassiber_envelope::s(workspace_id),
        kassiber_envelope::s(profile_id),
    );
    let result = process::process_native(
        &copy,
        |conn| service::profile(conn, Some(&workspace), Some(&profile)),
        kassiber_pydecimal::Context::RP2,
        sources,
        &mut |_| {},
        &mut (),
    );
    let committed = match result {
        Ok(committed) => committed,
        Err(refusal) => return Ok(Recomputed::Refused(refusal)),
    };
    let changes = input_changes(&committed.payload);
    committed.after_reply()?;
    if !changes.is_empty() {
        return Ok(Recomputed::Stale(format!(
            "processing would first update journal inputs ({})",
            changes.join(", ")
        )));
    }
    Ok(Recomputed::Tables(stored_tables(&copy, profile_id)?))
}

fn random_key(conn: &Connection) -> Result<String, Refusal> {
    conn.query_row("SELECT lower(hex(randomblob(32)))", [], |row| row.get(0))
        .map_err(Refusal::from)
}

/// Recompute the selected profile's journals from its current inputs in a
/// private in-memory copy and compare them with the stored journals. The
/// caller's book is only read, through SQLite's online backup.
pub fn check(
    conn: &Connection,
    workspace: Option<&kassiber_pyjson::Value>,
    profile: Option<&kassiber_pyjson::Value>,
    limit: usize,
    sources: &impl kassiber_transfers::Sources,
) -> Result<Value, Refusal> {
    let resolved = service::profile(conn, workspace, profile)?;
    let (Some(workspace_id), Some(profile_id)) =
        (resolved["workspace_id"].as_str(), resolved["id"].as_str())
    else {
        return Err(Refusal::new("KeyError", "id"));
    };
    let mut report = Map::new();
    report.insert("workspace_id".into(), json!(workspace_id));
    report.insert("profile_id".into(), json!(profile_id));
    report.insert("profile".into(), resolved["label"].clone());
    // The pristine copy is the one consistent read of the book: stored tables
    // and every recomputation start from it.
    let pristine = pipeline::memory_snapshot(conn, || random_key(conn))?;
    let versions = select(
        &pristine,
        "SELECT journal_input_version,last_processed_input_version,journal_semantics_version FROM profiles WHERE id = ?",
        profile_id,
    )?;
    if let Some(row) = versions.first() {
        for (name, value) in row {
            report.insert(name.clone(), value.to_json());
        }
    }
    let (status, reason) = crate::views::freshness_of(&pristine, profile_id)?
        .ok_or_else(|| Refusal::new("KeyError", "id"))?;
    if status != "current" {
        report.insert("status".into(), json!(status));
        report.insert("reason".into(), json!(reason));
        return Ok(Value::Object(report));
    }
    let stored = stored_tables(&pristine, profile_id)?;
    let recomputed = match recompute(&pristine, workspace_id, profile_id, sources)? {
        Recomputed::Tables(tables) => tables,
        Recomputed::Stale(reason) => {
            report.insert("status".into(), json!("stale"));
            report.insert("reason".into(), json!(reason));
            return Ok(Value::Object(report));
        }
        Recomputed::Refused(refusal) => {
            report.insert("status".into(), json!("refused"));
            report.insert(
                "error".into(),
                json!({"code": refusal.code.clone().unwrap_or_else(|| refusal.family.clone()), "message": refusal.message}),
            );
            return Ok(Value::Object(report));
        }
    };
    let compared = diff(&stored, &recomputed, limit);
    if compared.identical() {
        report.insert("status".into(), json!("identical"));
        report.insert(
            "reason".into(),
            json!("stored journals match a recomputation from the current inputs"),
        );
    } else {
        report.insert("status".into(), json!("different"));
        report.insert(
            "reason".into(),
            json!("stored journals differ from a recomputation from the current inputs"),
        );
        // A second recomputation separates nondeterminism from stored drift.
        let deterministic = match recompute(&pristine, workspace_id, profile_id, sources)? {
            Recomputed::Tables(again) => json!(diff(&recomputed, &again, 1).identical()),
            _ => Value::Null,
        };
        report.insert("deterministic".into(), deterministic);
    }
    report.extend(compared.fields());
    Ok(Value::Object(report))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, value: i64) -> Row {
        Row {
            key: key.into(),
            transaction_id: Some(format!("tx-{key}")),
            wallet_id: Some("w".into()),
            year: Some("2024".into()),
            columns: BTreeMap::from([("amount".into(), Cell::Int(value))]),
        }
    }

    #[test]
    fn an_overlap_left_unrepaired_is_not_an_input_change() {
        let unrepaired = json!({"source_overlap_repair": {
            "addresses_removed": 0, "duplicates_excluded": 0,
            "skipped_records": [{"id": "r"}], "unresolved_address_list_wallets": ["w"],
        }});
        assert!(input_changes(&unrepaired).is_empty());
        for count in ["addresses_removed", "duplicates_excluded"] {
            let mut repaired = unrepaired.clone();
            repaired["source_overlap_repair"][count] = json!(1);
            assert_eq!(
                input_changes(&repaired),
                ["source_overlap_repair"],
                "{count}"
            );
        }
        let other = json!({"auto_priced": 2, "custody_pair_evidence_repairs": 0});
        assert_eq!(input_changes(&other), ["auto_priced"]);
    }

    #[test]
    fn diff_reports_changed_missing_and_unexpected_rows_within_the_limit() {
        let stored = vec![Table::new("t", vec![row("a", 1), row("b", 2), row("c", 3)])];
        let recomputed = vec![Table::new("t", vec![row("a", 1), row("b", 5), row("d", 4)])];
        let report = diff(&stored, &recomputed, 2);
        assert_eq!(report.differing, 3);
        assert_eq!(report.differences.len(), 2);
        assert_eq!(report.by_wallet["w"], 3);
        assert_eq!(report.by_year["2024"], 3);
        assert_eq!(
            report.differences[0],
            json!({"table":"t","change":"changed","key":"b","transaction_id":"tx-b","wallet_id":"w","year":"2024",
                   "columns":[{"column":"amount","stored":2,"recomputed":5}]})
        );
        assert_eq!(report.differences[1]["change"], "missing");
        let fields = report.fields();
        assert_eq!(fields["truncated"], json!(true));
        assert_eq!(
            fields["tables"],
            json!([{"table":"t","stored":3,"recomputed":3,"differing":3}])
        );
        assert!(diff(&stored, &stored, 2).identical());
    }

    #[test]
    fn reals_compare_by_bits_and_repeated_keys_get_suffixes() {
        assert_ne!(
            Cell::real(0.1),
            Cell::real(0.1 + 1e-17_f64.max(f64::EPSILON / 8.0))
        );
        assert_ne!(Cell::real(0.0), Cell::real(-0.0));
        let table = Table::new("t", vec![row("a", 1), row("a", 2)]);
        assert_eq!(table.rows[1].key, "a#2");
    }
}
