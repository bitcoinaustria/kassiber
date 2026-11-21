//! Journal reproducibility (docs/reference/replay-bundle.md): recompute a
//! book's journals in a private in-memory copy and compare them with the
//! stored ones, and the canonical tables and bounded diff the replay bundle
//! shares. Nothing here writes to the caller's book.
use crate::{pipeline, process, service, JournalState, Refusal};
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
            // SQLite stores a REAL zero as +0.0: -0.0 reads back positive, so
            // the in-memory build carries the stored form (the exact text
            // column keeps the sign).
            Sql::Real(n) => Self::real(if n == 0.0 { 0.0 } else { n }),
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

    pub fn from_json(value: &Value) -> Result<Self, String> {
        Ok(match value {
            Value::Null => Self::Null,
            Value::String(text) => Self::Text(text.clone()),
            Value::Number(n) => Self::Int(
                n.as_i64()
                    .ok_or_else(|| "replay cell integers must fit 64 bits".to_owned())?,
            ),
            Value::Object(map) if map.len() == 1 => {
                if let Some(Value::String(text)) = map.get("real") {
                    Self::real(parse_float(text)?)
                } else if let Some(Value::String(text)) = map.get("blob") {
                    Self::Blob(unhex(text)?)
                } else {
                    return Err("unknown replay cell".into());
                }
            }
            _ => return Err("unknown replay cell".into()),
        })
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

fn parse_float(text: &str) -> Result<f64, String> {
    match text {
        "inf" => Ok(f64::INFINITY),
        "-inf" => Ok(f64::NEG_INFINITY),
        "nan" => Ok(f64::NAN),
        _ => text.parse().map_err(|_| "invalid replay real".to_owned()),
    }
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Result<Vec<u8>, String> {
    // Byte pairs of ASCII hex digits only: a multi-byte character must be
    // refused as corruption, never sliced.
    let digit = |byte: u8| (byte as char).to_digit(16).filter(|_| byte.is_ascii());
    let bytes = text.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err("invalid replay blob".into());
    }
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| match (digit(pair[0]), digit(pair[1])) {
            (Some(high), Some(low)) => Ok((high * 16 + low) as u8),
            _ => Err("invalid replay blob".to_owned()),
        })
        .collect()
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

    pub fn to_json(&self) -> Value {
        json!({
            "table": self.name,
            "rows": self.rows.iter().map(|row| {
                let mut out = Map::new();
                out.insert("key".into(), json!(row.key));
                for (name, value) in [("transaction_id", &row.transaction_id), ("wallet_id", &row.wallet_id), ("year", &row.year)] {
                    if let Some(value) = value {
                        out.insert(name.into(), json!(value));
                    }
                }
                out.insert("columns".into(), Value::Object(row.columns.iter().map(|(k, v)| (k.clone(), v.to_json())).collect()));
                Value::Object(out)
            }).collect::<Vec<_>>(),
        })
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let name = value["table"]
            .as_str()
            .ok_or("replay table without a name")?;
        let rows = value["rows"]
            .as_array()
            .ok_or("replay table without rows")?
            .iter()
            .map(|row| {
                let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_owned);
                Ok(Row {
                    key: text("key").ok_or("replay row without a key")?,
                    transaction_id: text("transaction_id"),
                    wallet_id: text("wallet_id"),
                    year: text("year"),
                    columns: row["columns"]
                        .as_object()
                        .ok_or("replay row without columns")?
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), Cell::from_json(v)?)))
                        .collect::<Result<_, String>>()?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        // Keys arrive already suffixed; keep them as recorded.
        Ok(Self {
            name: name.to_owned(),
            rows,
        })
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

/// The replay bundle's canonical outputs: the journal figures exactly as
/// persistence stores them (quantities as integer msat, exact decimal text,
/// REAL holdings and tax-summary values), so the same tables come from an
/// in-memory build (`state_tables`) and from a book's stored journals
/// (`stored_outputs`). Presentation text, calculation encodings and row ids
/// are left out.
const ENTRY_COLUMNS: [&str; 20] = [
    "transaction_id",
    "wallet_id",
    "account_id",
    "occurred_at",
    "entry_type",
    "asset",
    "quantity",
    "fiat_value",
    "fiat_value_exact",
    "unit_cost",
    "unit_cost_exact",
    "cost_basis",
    "cost_basis_exact",
    "proceeds",
    "proceeds_exact",
    "gain_loss",
    "gain_loss_exact",
    "at_category",
    "at_kennzahl",
    "capital_gains_type",
];

/// An entry's calculation in its decoded form, whichever encoding stored
/// it, as the gain explanation reads it; sorted-key JSON text.
fn calculation_cell(json: Option<&str>, compact: Option<&str>) -> Result<Cell, Refusal> {
    if json.is_none() && compact.is_none() {
        return Ok(Cell::Null);
    }
    Ok(match kassiber_reports::journal_calculation(json, compact) {
        Some(value) => Cell::Text(
            kassiber_pyjson::dumps(
                &value,
                &kassiber_pyjson::Options {
                    sort_keys: true,
                    ..Default::default()
                },
            )
            .map_err(|error| Refusal::new("ValueError", error.to_string()))?,
        ),
        None => Cell::text("undecodable"),
    })
}

fn entry_row(columns: BTreeMap<String, Cell>) -> Row {
    let text = |name: &str| text_of(columns.get(name)).unwrap_or_default();
    Row {
        key: format!(
            "{}|{}|{}|{}",
            text("transaction_id"),
            text("wallet_id"),
            text("entry_type"),
            text("asset")
        ),
        transaction_id: text_of(columns.get("transaction_id")),
        wallet_id: text_of(columns.get("wallet_id")),
        year: year_of(columns.get("occurred_at")),
        columns,
    }
}

fn keyed(
    key: &[&str],
    columns: BTreeMap<String, Cell>,
    wallet: Option<&str>,
    year: Option<&str>,
) -> Row {
    Row {
        key: key_of(&columns, key),
        transaction_id: None,
        wallet_id: wallet.and_then(|c| text_of(columns.get(c))),
        year: year.and_then(|c| year_of(columns.get(c))),
        columns,
    }
}

const SUMMARY_KEY: [&str; 4] = ["year", "asset", "transaction_type", "capital_gains_type"];
const WALLET_KEY: [&str; 3] = ["wallet_id", "account_code", "asset"];
const ACCOUNT_KEY: [&str; 3] = ["account_id", "account_code", "asset"];
const BALANCE_KEY: [&str; 3] = ["location_kind", "location_id", "asset"];

/// The canonical outputs of one in-memory journal build, converted the way
/// persistence converts them before it stores a row.
pub fn state_tables(state: &JournalState<'_>) -> Result<Vec<Table>, Refusal> {
    use crate::persistence::{amount, bind, real};
    let cell = |value: &Value| -> Result<Cell, Refusal> { Ok(Cell::from_sql(bind(value)?)) };
    // Both stored representations, REAL and exact text, as persistence
    // binds them: reports read either.
    let both = |columns: &mut BTreeMap<String, Cell>,
                name: &str,
                value: Option<&crate::JournalDecimal>|
     -> Result<(), Refusal> {
        let (real, exact) = crate::persistence::decimal_bindings(value)?;
        columns.insert(name.to_owned(), Cell::from_sql(real));
        columns.insert(format!("{name}_exact"), Cell::from_sql(exact));
        Ok(())
    };
    let mut entries = Vec::with_capacity(state.entries.len());
    for e in &state.entries {
        let quantity = match &e.quantity {
            crate::JournalQuantity::Finite(d) => amount(d)?,
            other => bind(&other.msat()?)?,
        };
        let mut columns = BTreeMap::new();
        columns.insert("transaction_id".into(), cell(&e.transaction_id)?);
        columns.insert("wallet_id".into(), cell(e.wallet_id)?);
        columns.insert("account_id".into(), cell(e.account_id)?);
        columns.insert("occurred_at".into(), cell(&e.occurred_at)?);
        columns.insert("entry_type".into(), Cell::text(e.entry_type));
        columns.insert("asset".into(), Cell::text(e.asset.as_ref()));
        columns.insert("quantity".into(), Cell::from_sql(quantity));
        both(&mut columns, "fiat_value", Some(&e.fiat_value))?;
        both(&mut columns, "unit_cost", Some(&e.unit_cost))?;
        both(&mut columns, "cost_basis", e.cost_basis.as_ref())?;
        both(&mut columns, "proceeds", e.proceeds.as_ref())?;
        both(&mut columns, "gain_loss", e.gain_loss.as_ref())?;
        let calculation = match &e.calculation {
            Some(calculation) => {
                let value = serde_json::to_value(calculation)
                    .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
                let (json, compact) = crate::storage::encode_calculation(&value)?;
                calculation_cell(json.as_deref(), compact.as_deref())?
            }
            None => Cell::Null,
        };
        columns.insert("calculation".into(), calculation);
        columns.insert(
            "at_category".into(),
            e.at_category.as_deref().map_or(Cell::Null, Cell::text),
        );
        columns.insert(
            "at_kennzahl".into(),
            e.at_kennzahl
                .flatten()
                .map_or(Cell::Null, |n| Cell::Int(n.into())),
        );
        columns.insert(
            "capital_gains_type".into(),
            e.capital_gains_type.map_or(Cell::Null, Cell::text),
        );
        entries.push(entry_row(columns));
    }
    let mut tables = vec![Table::new("journal_entries", entries)];
    let mut summary = Vec::with_capacity(state.tax_summary.len());
    for s in &state.tax_summary {
        let quantity = if crate::truth(&s.quantity_msat) {
            bind(&s.quantity_msat)?
        } else {
            amount(&crate::JournalDecimal(s.quantity.to_string().parse()?))?
        };
        let columns = BTreeMap::from([
            ("year".into(), Cell::Int(s.year.into())),
            ("asset".into(), Cell::text(s.asset.as_str())),
            ("transaction_type".into(), Cell::text(s.transaction_type)),
            (
                "capital_gains_type".into(),
                Cell::text(s.capital_gains_type),
            ),
            ("quantity".into(), Cell::from_sql(quantity)),
            ("proceeds".into(), Cell::real(s.proceeds)),
            ("cost_basis".into(), Cell::real(s.cost_basis)),
            ("gain_loss".into(), Cell::real(s.gain_loss)),
        ]);
        summary.push(keyed(&SUMMARY_KEY, columns, None, Some("year")));
    }
    tables.push(Table::new("journal_tax_summary", summary));
    for (name, holdings, wallet) in [
        ("journal_wallet_holdings", &state.wallet_holdings, true),
        ("journal_account_holdings", &state.account_holdings, false),
    ] {
        let mut rows = Vec::with_capacity(holdings.pairs.len());
        for (key, holding) in &holdings.pairs {
            if key.len() != 4 {
                return Err(Refusal::new("ValueError", "holding key width"));
            }
            // Wallet: id, label, account code, asset; account: id, code, label, asset.
            let (identity, names) = if wallet {
                ([&key[0], &key[2], &key[3]], WALLET_KEY)
            } else {
                ([&key[0], &key[1], &key[3]], ACCOUNT_KEY)
            };
            let mut columns = BTreeMap::new();
            for (name, value) in names.iter().zip(identity) {
                columns.insert((*name).to_owned(), cell(value)?);
            }
            columns.insert(
                "quantity".into(),
                Cell::from_sql(amount(&holding.quantity)?),
            );
            columns.insert(
                "cost_basis".into(),
                Cell::from_sql(real(&holding.cost_basis)),
            );
            rows.push(keyed(&names, columns, wallet.then_some("wallet_id"), None));
        }
        tables.push(Table::new(name, rows));
    }
    let (_, postings) = state.quantity_refs();
    let mut balances = BTreeMap::<(String, String, String), i128>::new();
    for posting in postings {
        let amount = posting.at("amount_msat").as_i64().ok_or_else(|| {
            Refusal::new("OverflowError", "quantity posting does not fit 64 bits")
        })?;
        *balances
            .entry((
                crate::string(posting.at("location_kind")),
                crate::string(posting.at("location_id")),
                crate::string(posting.at("asset")),
            ))
            .or_default() += i128::from(amount);
    }
    let mut rows = Vec::new();
    for ((kind, location, asset), amount) in balances {
        if amount == 0 {
            continue;
        }
        let amount = i64::try_from(amount)
            .map_err(|_| Refusal::new("OverflowError", "quantity balance does not fit 64 bits"))?;
        let wallet = kind == "wallet";
        let columns = BTreeMap::from([
            ("location_kind".into(), Cell::Text(kind)),
            ("location_id".into(), Cell::Text(location)),
            ("asset".into(), Cell::Text(asset)),
            ("amount_msat".into(), Cell::Int(amount)),
        ]);
        rows.push(keyed(
            &BALANCE_KEY,
            columns,
            wallet.then_some("location_id"),
            None,
        ));
    }
    tables.push(Table::new("journal_quantity_balances", rows));
    let quarantines = crate::persistence::dedupe_quarantines(&state.quarantines)?
        .into_iter()
        .map(|q| {
            let id = crate::string(&q.transaction_id);
            Row {
                key: id.clone(),
                transaction_id: Some(id),
                wallet_id: None,
                year: None,
                columns: BTreeMap::from([("reason".into(), Cell::text(q.reason))]),
            }
        })
        .collect();
    tables.push(Table::new("journal_quarantines", quarantines));
    Ok(tables)
}

/// The same canonical tables from a book's stored journals: the figures the
/// user saw, which a replay bundle carries next to its recomputation.
pub fn stored_outputs(conn: &Connection, profile_id: &str) -> Result<Vec<Table>, Refusal> {
    let read = |sql: &str| select(conn, sql, profile_id);
    let entries = read(&format!(
        "SELECT {},calculation_json,calculation_compact FROM journal_entries WHERE profile_id = ?",
        ENTRY_COLUMNS.join(",")
    ))?
    .into_iter()
    .map(|mut columns| {
        let text = |cell: Option<Cell>| match cell {
            Some(Cell::Text(text)) => Some(text),
            _ => None,
        };
        let json = text(columns.remove("calculation_json"));
        let compact = text(columns.remove("calculation_compact"));
        columns.insert(
            "calculation".into(),
            calculation_cell(json.as_deref(), compact.as_deref())?,
        );
        Ok(entry_row(columns))
    })
    .collect::<Result<_, Refusal>>()?;
    let mut tables = vec![Table::new("journal_entries", entries)];
    let summary = read("SELECT year,asset,transaction_type,capital_gains_type,quantity,proceeds,cost_basis,gain_loss FROM journal_tax_summary WHERE profile_id = ?")?
        .into_iter()
        .map(|columns| keyed(&SUMMARY_KEY, columns, None, Some("year")))
        .collect();
    tables.push(Table::new("journal_tax_summary", summary));
    let wallets = read("SELECT wallet_id,account_code,asset,quantity,cost_basis FROM journal_wallet_holdings WHERE profile_id = ?")?
        .into_iter()
        .map(|columns| keyed(&WALLET_KEY, columns, Some("wallet_id"), None))
        .collect();
    tables.push(Table::new("journal_wallet_holdings", wallets));
    let accounts = read("SELECT account_id,account_code,asset,quantity,cost_basis FROM journal_account_holdings WHERE profile_id = ?")?
        .into_iter()
        .map(|columns| keyed(&ACCOUNT_KEY, columns, None, None))
        .collect();
    tables.push(Table::new("journal_account_holdings", accounts));
    let balances = read("SELECT location_kind,location_id,asset,amount_msat FROM journal_quantity_balances WHERE profile_id = ?")?
        .into_iter()
        .map(|columns| {
            let wallet = columns.get("location_kind") == Some(&Cell::text("wallet"));
            keyed(&BALANCE_KEY, columns, wallet.then_some("location_id"), None)
        })
        .collect();
    tables.push(Table::new("journal_quantity_balances", balances));
    let quarantines =
        read("SELECT transaction_id,reason FROM journal_quarantines WHERE profile_id = ?")?
            .into_iter()
            .map(|mut columns| {
                let id = text_of(columns.get("transaction_id")).unwrap_or_default();
                columns.remove("transaction_id");
                Row {
                    key: id.clone(),
                    transaction_id: Some(id),
                    wallet_id: None,
                    year: None,
                    columns,
                }
            })
            .collect();
    tables.push(Table::new("journal_quarantines", quarantines));
    Ok(tables)
}

/// Build the canonical outputs of a loaded custody snapshot in memory.
pub fn snapshot_tables(
    snapshot: kassiber_custody::snapshot::Snapshot,
) -> Result<Vec<Table>, Refusal> {
    snapshot
        .validate_loaded()
        .map_err(|message| Refusal::new("ValueError", message))?;
    let profile = serde_json::to_value(&snapshot.profile)
        .map_err(|error| Refusal::new("ValueError", error.to_string()))?;
    // Persistence keeps only quarantines of the profile's own transactions
    // (a synthetic or foreign id is dropped before storage); so does this.
    let live: std::collections::HashSet<String> = snapshot
        .transactions
        .iter()
        .filter_map(|t| t.get("id").map(crate::string))
        .chain(snapshot.anchors.keys().cloned())
        .collect();
    // The same custody, fusion, engine and render path `journals process`
    // stores from.
    process::render_loaded(&profile, &snapshot, |state| {
        let mut tables = state_tables(state)?;
        live_quarantines(&mut tables, &live);
        Ok(tables)
    })
}

/// Drops quarantine rows whose transaction is not one of `live`.
fn live_quarantines(tables: &mut [Table], live: &std::collections::HashSet<String>) {
    for table in tables
        .iter_mut()
        .filter(|t| t.name == "journal_quarantines")
    {
        table.rows.retain(|row| {
            row.transaction_id
                .as_ref()
                .is_some_and(|id| live.contains(id))
        });
    }
}

/// What a replay bundle captures of one profile: its settings, journal
/// freshness, and the custody snapshot the journal pipeline loads, read in
/// one read transaction as of the last processing time.
pub struct BundleSource {
    pub profile: Value,
    pub status: &'static str,
    pub reason: &'static str,
    pub snapshot: kassiber_custody::snapshot::Snapshot,
}

/// Load the replay bundle's source for the selected profile. Read-only.
pub fn bundle_source(
    conn: &Connection,
    workspace: Option<&kassiber_pyjson::Value>,
    profile: Option<&kassiber_pyjson::Value>,
) -> Result<BundleSource, Refusal> {
    let read = if conn.is_autocommit() {
        Some(conn.unchecked_transaction()?)
    } else {
        None
    };
    let resolved = service::profile(conn, workspace, profile)?;
    let profile_id = resolved["id"]
        .as_str()
        .ok_or_else(|| Refusal::new("KeyError", "id"))?
        .to_owned();
    let (status, reason) = crate::views::freshness_of(conn, &profile_id)?
        .ok_or_else(|| Refusal::new("KeyError", "id"))?;
    let as_of = resolved["last_processed_at"]
        .as_str()
        .filter(|at| !at.is_empty())
        .unwrap_or("1970-01-01T00:00:00+00:00")
        .to_owned();
    let loaded = crate::loader::load_read_snapshot(
        conn,
        kassiber_custody::codec::from_json(&resolved)?,
        &as_of,
    )
    .map_err(|error| match error {
        crate::loader::Error::Reference(refusal) => refusal,
        other => Refusal::new("RuntimeError", other.to_string()),
    })?;
    let snapshot = loaded
        .into_custody()
        .map_err(|error| Refusal::new("RuntimeError", error.to_string()))?;
    drop(read);
    Ok(BundleSource {
        profile: resolved,
        status,
        reason,
        snapshot,
    })
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
    fn malformed_blobs_are_corruption_not_a_panic() {
        for text in ["\u{20ac}x", "zz", "abc", "0g", "\u{e9}\u{e9}"] {
            assert!(Cell::from_json(&json!({"blob": text})).is_err(), "{text:?}");
        }
        assert_eq!(
            Cell::from_json(&json!({"blob": "00ff"})).unwrap(),
            Cell::Blob(vec![0, 255])
        );
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

    /// The in-memory build reads a REAL as SQLite stores it: a negative
    /// zero comes back positive, so a correctly stored signed-zero entry is
    /// not a replay difference.
    #[test]
    fn a_real_zero_reads_as_sqlite_stores_it() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE t(x REAL); INSERT INTO t VALUES(-0.0)")
            .unwrap();
        let stored: Sql = conn.query_row("SELECT x FROM t", [], |r| r.get(0)).unwrap();
        assert_eq!(Cell::from_sql(Sql::Real(-0.0)), Cell::from_sql(stored));
        assert_eq!(Cell::from_sql(Sql::Real(-0.0)), Cell::real(0.0));
        assert_eq!(Cell::from_sql(Sql::Real(-1.5)), Cell::real(-1.5));
    }

    #[test]
    fn quarantines_of_transactions_outside_the_profile_are_dropped() {
        let quarantine = |id: &str| Row {
            key: id.into(),
            transaction_id: Some(id.into()),
            wallet_id: None,
            year: None,
            columns: BTreeMap::from([("reason".into(), Cell::text("x"))]),
        };
        let mut tables = vec![Table::new(
            "journal_quarantines",
            vec![quarantine("tx-1"), quarantine("synthetic:unknown")],
        )];
        live_quarantines(&mut tables, &["tx-1".to_owned()].into_iter().collect());
        let kept: Vec<_> = tables[0].rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(kept, ["tx-1"]);
    }

    #[test]
    fn cells_and_tables_round_trip_through_the_bundle_encoding() {
        for cell in [
            Cell::Null,
            Cell::Int(-7),
            Cell::real(0.1),
            Cell::real(f64::INFINITY),
            Cell::text("x"),
            Cell::Blob(vec![0, 255]),
        ] {
            assert_eq!(Cell::from_json(&cell.to_json()).unwrap(), cell);
        }
        let table = Table::new("t", vec![row("a", 1), row("a", 2)]);
        assert_eq!(Table::from_json(&table.to_json()).unwrap(), table);
    }
}
