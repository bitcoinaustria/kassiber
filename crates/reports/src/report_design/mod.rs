//! The shared report design (`docs/reference/report-design.md`): one
//! document of typed sections, rendered as PDF, XLSX and a CSV bundle with
//! the same sections, columns, labels and number formats.
//!
//! Builders turn a finished report model into a [`Document`]; nothing here
//! computes a tax, journal or custody figure. Every value keeps its exact
//! decimal text, so PDF display rounding, XLSX numbers and CSV strings all
//! derive from the same value.
use super::*;
use sha2::{Digest, Sha256};

mod austrian;
mod ch;
pub mod countries;
mod cross_check;
mod de;
mod es;
mod exit;
mod full;
mod gb;
mod ie;
mod jp;
mod locale;
mod source_funds;
mod strings;
mod strings_es;
mod strings_ja;
mod summary;
mod us;
mod year_handoff;
pub use austrian::austrian;
pub use cross_check::cross_check;
pub use exit::exit_tax;
pub use full::full;
pub use locale::Lang;
pub use source_funds::source_funds;
pub(crate) use summary::prepare as prepare_summary;
pub use summary::{summary, summary_model};
pub use year_handoff::year_handoff;

/// The CSV bundle's schema identifier, versioned with the layout.
pub const CSV_SCHEMA: &str = "kassiber.report/1";

#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Empty,
    Text(String),
    /// ISO date (`2025-03-15`) or month (`2025-03`).
    Date(String),
    /// A fiat amount, exact.
    Fiat(Decimal),
    /// A BTC-denominated amount, exact.
    Btc(Decimal),
    /// Integer millisatoshi beside a BTC amount.
    Msat(kassiber_pyjson::BigInt),
    Count(i64),
    /// A percentage (12.3 means 12.3 %).
    Percent(Decimal),
    /// A live spreadsheet check: `expression` names the row's other columns
    /// as `{key}`; PDF and CSV show the computed value `cached`.
    Formula {
        expression: String,
        cached: Box<Cell>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Text,
    /// Identifiers, set in the mono face.
    Code,
    Date,
    Fiat,
    Btc,
    Msat,
    Count,
    Percent,
}

impl Kind {
    fn right(self) -> bool {
        matches!(
            self,
            Kind::Fiat | Kind::Btc | Kind::Msat | Kind::Count | Kind::Percent
        )
    }
    /// Only identifiers use the mono face; amounts use DM Sans's tabular
    /// figures.
    fn mono(self) -> bool {
        matches!(self, Kind::Code)
    }
}

#[derive(Clone, Debug)]
pub struct Column {
    /// The CSV header and XLSX key: stable snake_case.
    pub key: &'static str,
    pub label: String,
    pub kind: Kind,
    /// Relative PDF width.
    pub width: f64,
    /// A mostly-zero column: zero shows as `–` in the PDF so the few
    /// non-zero values stand out. XLSX and CSV keep the zero.
    pub sparse: bool,
}

impl Column {
    pub fn new(key: &'static str, label: impl Into<String>, kind: Kind, width: f64) -> Column {
        Column {
            key,
            label: label.into(),
            kind,
            width,
            sparse: false,
        }
    }
    pub fn sparse(self) -> Column {
        Column {
            sparse: true,
            ..self
        }
    }
}

#[derive(Clone, Debug)]
pub struct Table {
    /// Sheet and CSV file name.
    pub id: &'static str,
    /// A caption above the table; empty for none.
    pub title: String,
    pub columns: Vec<Column>,
    pub rows: Vec<Vec<Cell>>,
    pub total: Option<Vec<Cell>>,
    pub note: String,
}

impl Table {
    /// The columns on paper: every column but the msat companions.
    fn shown(&self) -> Vec<usize> {
        (0..self.columns.len())
            .filter(|i| self.columns[*i].kind != Kind::Msat)
            .collect()
    }
    /// A row as the PDF prints it.
    pub fn display_row(&self, cells: &[Cell], lang: Lang) -> Vec<String> {
        self.shown()
            .into_iter()
            .map(|i| {
                if self.columns[i].sparse && cells[i].is_zero() {
                    "\u{2013}".to_owned()
                } else {
                    cells[i].display(lang)
                }
            })
            .collect()
    }
}

#[derive(Clone, Debug)]
pub struct Fact {
    pub key: &'static str,
    pub label: String,
    pub value: Cell,
    /// Rendered text when it is not just the value (for example a list).
    pub display: Option<String>,
    pub accent: bool,
}

#[derive(Clone, Debug)]
pub struct Kpi {
    pub key: &'static str,
    pub label: String,
    pub value: Cell,
    pub detail: String,
    pub accent: bool,
}

/// A legend entry: its label and how the series is drawn.
#[derive(Clone, Debug)]
pub struct Legend {
    pub label: String,
    /// `line`, `dashed`, `thin`, `filled` or `hollow`.
    pub mark: &'static str,
    pub tone: &'static str,
}

/// A chart in unit coordinates (x and y in 0..1, y up).
#[derive(Clone, Debug, Default)]
pub struct Chart {
    pub title: String,
    pub height: f64,
    pub y_labels: Vec<(f64, String)>,
    pub y2_labels: Vec<(f64, String)>,
    pub x_labels: Vec<(f64, String)>,
    pub series: Vec<Series>,
    pub legend: Vec<Legend>,
}

#[derive(Clone, Debug)]
pub enum Series {
    Line {
        points: Vec<(f64, f64)>,
        weight: f64,
        dashed: bool,
        tone: &'static str,
    },
    /// Bars as `(x0, x1, y0, y1)`.
    Bars {
        bars: Vec<(f64, f64, f64, f64)>,
        hollow: bool,
        tone: &'static str,
    },
}

#[derive(Clone, Debug)]
pub enum Block {
    Facts(Vec<Fact>),
    Kpis(Vec<Kpi>),
    Table(Table),
    Chart(Chart),
    Flow(Flow),
    Note { text: String, accent: bool },
}

/// A flow diagram: columns of boxes left to right and directed links
/// between them. Only the PDF draws it; its data is in the report's tables.
#[derive(Clone, Debug, Default)]
pub struct Flow {
    pub columns: Vec<FlowColumn>,
    pub links: Vec<FlowLink>,
    pub legend: Vec<Legend>,
}

#[derive(Clone, Debug)]
pub struct FlowColumn {
    pub title: String,
    pub nodes: Vec<FlowNode>,
}

#[derive(Clone, Debug)]
pub struct FlowNode {
    pub id: String,
    pub label: String,
    pub detail: String,
    pub amount: String,
    /// `target`, `source`, `attested`, `hop`, `privacy` or `omitted`.
    pub style: &'static str,
}

#[derive(Clone, Debug)]
pub struct FlowLink {
    pub from: String,
    pub to: String,
    pub label: String,
    /// `plain`, `swap` or `privacy` (dashed).
    pub style: &'static str,
    /// Stroke width in points.
    pub weight: f64,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub id: &'static str,
    pub title: String,
    pub lead: String,
    pub page_break: bool,
    /// Wide tables: the section gets landscape pages of its own.
    pub landscape: bool,
    pub blocks: Vec<Block>,
}

#[derive(Clone, Debug)]
pub struct Document {
    /// Report kind, for the manifest (`summary`, …).
    pub kind: &'static str,
    pub title: String,
    pub kicker: String,
    pub book: String,
    pub period: String,
    pub period_start: String,
    pub period_end: String,
    pub generated_at: String,
    /// The journal input version; `None` when the report has none.
    pub input_version: Option<i64>,
    pub fiat: String,
    pub country: String,
    pub lang: Lang,
    pub sections: Vec<Section>,
}

impl Document {
    /// The notice of a preview country (`kassiber_tax_countries`), which
    /// every rendering carries: the PDF's first page and footers, the XLSX
    /// summary sheet, the CSV summary and manifest.
    pub fn preview_notice(&self) -> Option<&'static str> {
        self.lang.preview_notice(&self.country)
    }
    /// The input version as the footer prints it; unknown is `—`.
    fn version_text(&self) -> String {
        self.input_version
            .map(|v| v.to_string())
            .unwrap_or_else(|| "\u{2014}".into())
    }
}

// ---------------------------------------------------------------- formatting

impl Cell {
    /// PDF text in the report's language. The same rounding rule applies to
    /// every format's display; `Empty` (unknown or not applicable) is `—`.
    pub fn display(&self, lang: Lang) -> String {
        match self {
            Cell::Empty => "\u{2014}".into(),
            Cell::Text(t) => t.clone(),
            Cell::Date(t) if t.len() == 7 => lang.month(t),
            Cell::Date(t) => lang.date(t),
            Cell::Fiat(d) => lang.number(d, 2),
            Cell::Btc(d) => lang.number(d, 8),
            Cell::Msat(n) => n
                .to_string()
                .parse::<i128>()
                .map(|v| lang.integer(v))
                .unwrap_or_else(|_| n.to_string()),
            Cell::Count(n) => lang.integer((*n).into()),
            Cell::Percent(d) => lang.percent(d),
            Cell::Formula { cached, .. } => cached.display(lang),
        }
    }
    /// A zero amount; sparse columns show it as `–`.
    fn is_zero(&self) -> bool {
        match self {
            Cell::Fiat(d) | Cell::Btc(d) | Cell::Percent(d) => d.is_zero(),
            Cell::Count(n) => *n == 0,
            Cell::Msat(n) => n.to_string() == "0",
            Cell::Formula { cached, .. } => cached.is_zero(),
            _ => false,
        }
    }
    /// CSV text: exact, plain, machine-readable.
    pub fn csv(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Text(t) | Cell::Date(t) => t.clone(),
            Cell::Fiat(d) | Cell::Percent(d) => plain(d),
            Cell::Btc(d) => btc_text(d),
            Cell::Msat(n) => n.to_string(),
            Cell::Count(n) => n.to_string(),
            Cell::Formula { cached, .. } => cached.csv(),
        }
    }
}

/// Whether the stored `tax_country` (compared exactly) follows Austria's
/// rules: its country layer (E 1kv Kennzahlen, regimes, review gate).
pub(crate) fn austrian_rules(country: &str) -> bool {
    kassiber_tax_countries::lookup(country)
        .is_some_and(|entry| entry.rules == kassiber_tax_countries::Rules::Austria)
}

/// A decimal without exponent or grouping.
pub fn plain(d: &Decimal) -> String {
    d.to_fixed_string().unwrap_or_else(|_| d.to_canonical())
}

/// A BTC amount as CSV text: eight decimal places (`0.05000000`), more only
/// for the sub-satoshi digits an exact amount carries (`0.00000000001` is one
/// msat); the value is never rounded.
pub fn btc_text(d: &Decimal) -> String {
    let text = plain(d);
    let (whole, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let fraction = fraction.trim_end_matches('0');
    format!("{whole}.{fraction:0<8}")
}

/// The exact decimal behind a Python float (its shortest `repr`).
pub fn from_float(v: &Value) -> Decimal {
    let text = match v {
        Value::Float(f) => kassiber_pyjson::float_repr(*f),
        Value::Int(n) => n.to_string(),
        Value::Null => return Decimal::zero(),
        other => py_str(other),
    };
    text.parse().unwrap_or_else(|_| Decimal::zero())
}

/// Integer msat of an exact BTC amount, when it is whole.
pub fn msat_of(btc: &Decimal) -> Cell {
    let scaled = btc
        .mul(&Decimal::from(100_000_000_000_i64), &DECIMAL)
        .ok()
        .and_then(|d| {
            d.to_fixed_places(0, Rounding::HalfEven)
                .ok()
                .map(|t| (d, t))
        });
    match scaled {
        Some((exact, text))
            if exact == text.parse::<Decimal>().unwrap_or_else(|_| Decimal::zero()) =>
        {
            kassiber_pytext::parse_int(&text)
                .map(Cell::Msat)
                .unwrap_or(Cell::Empty)
        }
        _ => Cell::Empty,
    }
}

// ----------------------------------------------------------------- charts

/// A chart axis on round values: its bounds and labelled ticks.
pub struct Axis {
    pub low: f64,
    pub high: f64,
    pub ticks: Vec<(f64, String)>,
}

impl Axis {
    /// The axis over `values` and zero, about `target` steps of 1, 2, 2.5
    /// or 5 times a power of ten. `compact` writes thousands and millions
    /// short (`20k`, `1.5M`; `20 Tsd.`, `1,5 Mio.`).
    pub fn new(values: &[f64], target: usize, lang: Lang, compact: bool) -> Axis {
        let finite = values.iter().copied().filter(|v| v.is_finite());
        let low = finite.clone().fold(0.0, f64::min);
        let high = finite.fold(0.0, f64::max);
        let span = if high > low { high - low } else { 1.0 };
        let raw = span / target.max(1) as f64;
        let magnitude = 10f64.powf(raw.log10().floor());
        let step = [1.0, 2.0, 2.5, 5.0, 10.0]
            .into_iter()
            .map(|m| m * magnitude)
            .find(|s| *s >= raw * (1.0 - 1e-9))
            .unwrap_or(10.0 * magnitude);
        // The tolerance keeps a bound that is already a whole step from
        // gaining an empty step.
        let first = (low / step + 1e-9).floor() as i64;
        let last = ((if high > low { high } else { low + 1.0 }) / step - 1e-9).ceil() as i64;
        let (axis_low, axis_high) = (first as f64 * step, last as f64 * step);
        let biggest = axis_low.abs().max(axis_high.abs());
        let (scale, unit) = match (compact, lang) {
            (true, _) if biggest >= 1e6 => (
                1e6,
                match lang {
                    _ if lang.german() => "\u{a0}Mio.",
                    Lang::Es => "\u{a0}mill.",
                    _ => "M",
                },
            ),
            (true, _) if biggest >= 1e3 => (
                1e3,
                match lang {
                    _ if lang.german() => "\u{a0}Tsd.",
                    Lang::Es => "\u{a0}mil",
                    _ => "k",
                },
            ),
            _ => (1.0, ""),
        };
        let places = (0..4u32)
            .find(|p| {
                let shifted = step / scale * 10f64.powi(*p as i32);
                (shifted - shifted.round()).abs() < 1e-6
            })
            .unwrap_or(4);
        let ticks = (first..=last)
            .map(|i| {
                let v = i as f64 * step;
                let text = format!("{:.*}", places as usize, v / scale);
                let d: Decimal = text.parse().unwrap_or_else(|_| Decimal::zero());
                let label = if d.is_zero() {
                    "0".to_owned()
                } else {
                    format!("{}{unit}", lang.number(&d, places))
                };
                ((v - axis_low) / (axis_high - axis_low), label)
            })
            .collect();
        Axis {
            low: axis_low,
            high: axis_high,
            ticks,
        }
    }
    /// A value's position on the axis, 0..1.
    pub fn at(&self, v: f64) -> f64 {
        (v - self.low) / (self.high - self.low)
    }
}

/// A `&'static str` for a key read from data (sheet metadata, statuses).
/// The keys are a small fixed set, so interning them is bounded.
fn intern(key: String) -> &'static str {
    static KEYS: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());
    let mut keys = KEYS.lock().unwrap();
    if let Some(found) = keys.iter().find(|k| **k == key) {
        return found;
    }
    let interned: &'static str = Box::leak(key.into_boxed_str());
    keys.push(interned);
    interned
}

// --------------------------------------------------------------- builders
//
// Shared by every report builder: cells from model values (`None` is
// `Empty`, never zero), columns, facts and the balance chart.

fn list<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    artifact_csv::list(value(v, key))
}
fn fiat(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Fiat(from_float(raw)),
    }
}
fn btc(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Btc(from_float(raw)),
    }
}
fn msat(cell: &Cell) -> Cell {
    match cell {
        Cell::Btc(d) => msat_of(d),
        _ => Cell::Empty,
    }
}
fn txt(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Text(py_str(raw)),
    }
}
fn day(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Null => Cell::Empty,
        raw => Cell::Date(py_str(raw).chars().take(10).collect()),
    }
}
fn count(v: &Value, key: &str) -> Cell {
    match value(v, key) {
        Value::Int(n) => Cell::Count(i64::try_from(n.clone()).unwrap_or(0)),
        _ => Cell::Empty,
    }
}
fn col(key: &'static str, label: impl Into<String>, kind: Kind, width: f64) -> Column {
    Column::new(key, label, kind, width)
}
fn fact(key: &'static str, label: &str, value: Cell) -> Fact {
    Fact {
        key,
        label: label.into(),
        value,
        display: None,
        accent: false,
    }
}
fn method_name(lang: Lang, algorithm: &str) -> String {
    lang.get(&format!("method.{algorithm}"))
        .map(str::to_owned)
        .unwrap_or_else(|| algorithm.to_owned())
}
fn status_name(lang: Lang, status: &str) -> String {
    lang.get(&format!("status.{status}"))
        .map(str::to_owned)
        .unwrap_or_else(|| status.replace('_', " "))
}

/// At most about eight month labels; the last month is always labelled and
/// replaces a regular label too close to it.
fn month_labels(lang: Lang, months: &[String]) -> Vec<(f64, String)> {
    let total = months.len();
    let step = total.div_ceil(8).max(1);
    months
        .iter()
        .enumerate()
        .filter(|(i, _)| (i % step == 0 && *i + step < total) || *i + 1 == total)
        .map(|(i, m)| ((i as f64 + 0.5) / total as f64, lang.month_short(m)))
        .collect()
}
fn float(row: &Value, key: &str) -> f64 {
    from_float(value(row, key))
        .to_canonical()
        .parse()
        .unwrap_or(0.0)
}

/// Market value (solid, red), cost basis (dashed) and, when the quantity is
/// one asset's, the quantity on a second axis, by month.
fn balance_chart(
    lang: Lang,
    title: &str,
    months: &[String],
    market: &[f64],
    basis: &[f64],
    quantity: Option<&[f64]>,
    currency: &str,
) -> Chart {
    let axis = Axis::new(&[market, basis].concat(), 4, lang, true);
    let total = months.len();
    let x = |i: usize| (i as f64 + 0.5) / total as f64;
    let line = |values: &[f64], axis: &Axis| {
        values
            .iter()
            .enumerate()
            .map(|(i, v)| (x(i), axis.at(*v)))
            .collect()
    };
    let fiat = [("fiat", currency)];
    let mut series = Vec::new();
    let mut legend = vec![
        Legend {
            label: lang.tf("legend.market_value", &fiat),
            mark: "line",
            tone: "accent",
        },
        Legend {
            label: lang.tf("legend.cost_basis", &fiat),
            mark: "dashed",
            tone: "ink",
        },
    ];
    let mut y2_labels = Vec::new();
    if let Some(quantity) = quantity {
        let qaxis = Axis::new(quantity, 4, lang, false);
        series.push(Series::Line {
            points: line(quantity, &qaxis),
            weight: 0.6,
            dashed: false,
            tone: "muted",
        });
        legend.push(Legend {
            label: lang.t("legend.quantity").into(),
            mark: "thin",
            tone: "muted",
        });
        y2_labels = qaxis.ticks;
    }
    series.push(Series::Line {
        points: line(basis, &axis),
        weight: 0.8,
        dashed: true,
        tone: "ink",
    });
    series.push(Series::Line {
        points: line(market, &axis),
        weight: 1.4,
        dashed: false,
        tone: "accent",
    });
    Chart {
        title: title.into(),
        height: 120.0,
        x_labels: month_labels(lang, months),
        series,
        legend,
        y_labels: axis.ticks,
        y2_labels,
    }
}

// ------------------------------------------------------------------- PDF

fn s(text: impl Into<String>) -> Value {
    kassiber_envelope::s(text.into())
}
fn n(v: f64) -> Value {
    Value::Float(v)
}
fn obj<const N: usize>(pairs: [(&str, Value); N]) -> Value {
    Value::Dict(pairs.into_iter().map(|(k, v)| (k.into(), v)).collect())
}

fn pdf_block(block: &Block, lang: Lang) -> Value {
    match block {
        // One typeface per key/value block: values are DM Sans throughout.
        Block::Facts(rows) => obj([
            ("t", s("facts")),
            (
                "rows",
                Value::List(
                    rows.iter()
                        .map(|f| {
                            obj([
                                ("label", s(f.label.as_str())),
                                (
                                    "value",
                                    s(f.display.clone().unwrap_or_else(|| f.value.display(lang))),
                                ),
                                ("tone", s(if f.accent { "accent" } else { "ink" })),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        Block::Kpis(items) => obj([
            ("t", s("kpis")),
            (
                "items",
                Value::List(
                    items
                        .iter()
                        .map(|k| {
                            obj([
                                ("label", s(k.label.as_str())),
                                ("value", s(k.value.display(lang))),
                                ("detail", s(k.detail.as_str())),
                                ("tone", s(if k.accent { "accent" } else { "ink" })),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        Block::Table(t) => {
            let shown = t.shown();
            let row = |cells: &Vec<Cell>| {
                Value::List(t.display_row(cells, lang).into_iter().map(s).collect())
            };
            obj([
                ("t", s("table")),
                ("title", s(t.title.as_str())),
                (
                    "columns",
                    Value::List(
                        shown
                            .iter()
                            .map(|i| {
                                let c = &t.columns[*i];
                                obj([
                                    ("label", s(c.label.as_str())),
                                    ("align", s(if c.kind.right() { "right" } else { "left" })),
                                    ("mono", Value::Bool(c.kind.mono())),
                                    ("width", n(c.width)),
                                ])
                            })
                            .collect(),
                    ),
                ),
                ("rows", Value::List(t.rows.iter().map(row).collect())),
                ("total", t.total.as_ref().map(row).unwrap_or(Value::Null)),
                ("note", s(t.note.as_str())),
            ])
        }
        Block::Chart(c) => {
            let ticks = |v: &Vec<(f64, String)>| {
                Value::List(
                    v.iter()
                        .map(|(at, t)| obj([("pos", n(*at)), ("text", s(t.as_str()))]))
                        .collect(),
                )
            };
            obj([
                ("t", s("chart")),
                ("title", s(c.title.as_str())),
                ("height", n(c.height)),
                ("y_labels", ticks(&c.y_labels)),
                ("y2_labels", ticks(&c.y2_labels)),
                ("x_labels", ticks(&c.x_labels)),
                (
                    "series",
                    Value::List(
                        c.series
                            .iter()
                            .map(|series| match series {
                                Series::Line {
                                    points,
                                    weight,
                                    dashed,
                                    tone,
                                } => obj([
                                    ("kind", s("line")),
                                    (
                                        "points",
                                        Value::List(
                                            points
                                                .iter()
                                                .map(|(x, y)| Value::List(vec![n(*x), n(*y)]))
                                                .collect(),
                                        ),
                                    ),
                                    ("weight", n(*weight)),
                                    ("dashed", Value::Bool(*dashed)),
                                    ("tone", s(*tone)),
                                ]),
                                Series::Bars { bars, hollow, tone } => obj([
                                    ("kind", s("bars")),
                                    (
                                        "bars",
                                        Value::List(
                                            bars.iter()
                                                .map(|(a, b, c, d)| {
                                                    Value::List(vec![n(*a), n(*b), n(*c), n(*d)])
                                                })
                                                .collect(),
                                        ),
                                    ),
                                    ("hollow", Value::Bool(*hollow)),
                                    ("tone", s(*tone)),
                                ]),
                            })
                            .collect(),
                    ),
                ),
                (
                    "legend",
                    Value::List(
                        c.legend
                            .iter()
                            .map(|l| {
                                obj([
                                    ("label", s(l.label.as_str())),
                                    ("mark", s(l.mark)),
                                    ("tone", s(l.tone)),
                                ])
                            })
                            .collect(),
                    ),
                ),
            ])
        }
        Block::Flow(f) => obj([
            ("t", s("flow")),
            (
                "columns",
                Value::List(
                    f.columns
                        .iter()
                        .map(|c| {
                            obj([
                                ("title", s(c.title.as_str())),
                                (
                                    "nodes",
                                    Value::List(
                                        c.nodes
                                            .iter()
                                            .map(|nd| {
                                                obj([
                                                    ("id", s(nd.id.as_str())),
                                                    ("label", s(nd.label.as_str())),
                                                    ("detail", s(nd.detail.as_str())),
                                                    ("amount", s(nd.amount.as_str())),
                                                    ("style", s(nd.style)),
                                                ])
                                            })
                                            .collect(),
                                    ),
                                ),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "links",
                Value::List(
                    f.links
                        .iter()
                        .map(|l| {
                            obj([
                                ("from", s(l.from.as_str())),
                                ("to", s(l.to.as_str())),
                                ("label", s(l.label.as_str())),
                                ("style", s(l.style)),
                                ("weight", n(l.weight)),
                            ])
                        })
                        .collect(),
                ),
            ),
            (
                "legend",
                Value::List(
                    f.legend
                        .iter()
                        .map(|l| {
                            obj([
                                ("label", s(l.label.as_str())),
                                ("mark", s(l.mark)),
                                ("tone", s(l.tone)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ]),
        Block::Note { text, accent } => obj([
            ("t", s("note")),
            ("text", s(text.as_str())),
            ("tone", s(if *accent { "accent" } else { "muted" })),
        ]),
    }
}

fn pdf_data(doc: &Document, stamp: kassiber_pdf::engine::Stamp) -> Value {
    let lang = doc.lang;
    let mut meta = obj([
        ("title", s(doc.title.as_str())),
        ("kicker", s(doc.kicker.as_str())),
        ("running_title", s(format!("Kassiber  ·  {}", doc.title))),
        ("book", s(doc.book.as_str())),
        ("period", s(doc.period.as_str())),
        (
            "footer",
            s({
                let generated = lang.tf(
                    "doc.generated",
                    &[("at", &lang.date_time(&doc.generated_at))],
                );
                // A report without a journal version names none.
                match doc.input_version {
                    Some(v) => format!(
                        "{generated}  ·  {}",
                        lang.tf("doc.input_version", &[("version", &v.to_string())])
                    ),
                    None => generated,
                }
            }),
        ),
        ("page_format", s(lang.t("doc.page"))),
        ("lang", s(lang.code())),
        (
            "keywords",
            Value::List(vec![s("kassiber"), s(doc.kind), s(doc.country.as_str())]),
        ),
        (
            "date",
            Value::List(
                [
                    i64::from(stamp.year),
                    stamp.month.into(),
                    stamp.day.into(),
                    stamp.hour.into(),
                    stamp.minute.into(),
                    stamp.second.into(),
                ]
                .into_iter()
                .map(|v| Value::Int(v.into()))
                .collect(),
            ),
        ),
    ]);
    // Only a preview country's report has the key, so every other PDF keeps
    // its bytes.
    if let Some(notice) = doc.preview_notice() {
        kassiber_envelope::set(&mut meta, "notice", s(notice));
    }
    obj([
        ("meta", meta),
        (
            "sections",
            Value::List(
                doc.sections
                    .iter()
                    .map(|section| {
                        obj([
                            ("id", s(section.id)),
                            ("title", s(section.title.as_str())),
                            ("lead", s(section.lead.as_str())),
                            ("page_break", Value::Bool(section.page_break)),
                            ("landscape", Value::Bool(section.landscape)),
                            (
                                "blocks",
                                Value::List(
                                    section.blocks.iter().map(|b| pdf_block(b, lang)).collect(),
                                ),
                            ),
                        ])
                    })
                    .collect(),
            ),
        ),
    ])
}

/// The PDF and its page count.
pub fn pdf(doc: &Document) -> Result<(Vec<u8>, usize)> {
    let rendered = rendered_pdf(doc)?;
    Ok((rendered.bytes, rendered.pages.len()))
}

/// The rendered PDF with its page geometry and any text past a page edge.
pub fn rendered_pdf(doc: &Document) -> Result<kassiber_pdf::engine::Rendered> {
    let stamp = kassiber_pdf::engine::Stamp::parse(&doc.generated_at)
        .ok_or_else(|| AppError::new("internal_error", "invalid report timestamp"))?;
    kassiber_pdf::engine::render(kassiber_pdf::engine::Document {
        template: kassiber_pdf::engine::REPORT,
        data: pdf_data(doc, stamp),
        files: Vec::new(),
        created: stamp,
    })
    .map_err(|e| AppError::new("internal_error", format!("PDF rendering failed: {e}")))
}

// ------------------------------------------------------------------ XLSX

use kassiber_xlsx::{Cell as XCell, Operation, Sheet, Style};

fn styles(lang: Lang) -> Vec<Style> {
    let (date, month) = match lang {
        Lang::En => ("yyyy-mm-dd", "yyyy-mm"),
        Lang::De | Lang::DeDe | Lang::DeCh => ("dd.mm.yyyy", "mm.yyyy"),
        Lang::Es => ("dd/mm/yyyy", "mm/yyyy"),
        Lang::Ja => ("yyyy/mm/dd", "yyyy/mm"),
    };
    let make = |format: Option<&str>, bold: bool, bottom: bool, color: Option<&str>| Style {
        bold,
        font_color: color.map(str::to_owned),
        num_format: format.map(str::to_owned),
        bottom: u8::from(bottom),
        valign: Some("top".into()),
        ..Default::default()
    };
    vec![
        make(None, false, false, None),
        make(None, true, true, Some("#5B6169")),
        make(None, true, false, None),
        make(Some("#,##0.00"), false, false, None),
        make(Some("#,##0.00000000"), false, false, None),
        make(Some("#,##0"), false, false, None),
        make(Some("0.0\"%\""), false, false, None),
        make(Some(date), false, false, None),
        make(None, false, false, Some("#5B6169")),
        make(Some("#,##0.00"), true, false, None),
        make(Some("#,##0.00000000"), true, false, None),
        make(Some(month), false, false, None),
    ]
}
const TEXT: usize = 0;
const HEADER: usize = 1;
const TITLE: usize = 2;
const FIAT: usize = 3;
const BTC: usize = 4;
const COUNT: usize = 5;
const PERCENT: usize = 6;
const DATE: usize = 7;
const MUTED: usize = 8;
const FIAT_TOTAL: usize = 9;
const BTC_TOTAL: usize = 10;
const MONTH: usize = 11;

fn number(d: &Decimal) -> f64 {
    d.to_canonical().parse().unwrap_or(0.0)
}

/// Days since 1899-12-30 for an ISO date or month.
fn excel_date(text: &str) -> Option<f64> {
    let iso = if text.len() == 7 {
        format!("{text}-01")
    } else {
        text.get(..10)?.to_owned()
    };
    let day = kassiber_rates::time::parse(&iso)?;
    let base = kassiber_rates::time::parse("1899-12-30")?;
    Some(((day - base) / 86_400_000_000) as f64)
}

/// `A1` notation of a zero-based column and row.
fn address(col: usize, row: usize) -> String {
    let mut index = col + 1;
    let mut name = String::new();
    while index > 0 {
        name.insert(0, (b'A' + ((index - 1) % 26) as u8) as char);
        index = (index - 1) / 26;
    }
    format!("{name}{}", row + 1)
}

fn xlsx_cell(cell: &Cell, total: bool) -> (XCell, usize) {
    match cell {
        Cell::Empty => (XCell::Blank, TEXT),
        Cell::Text(t) => (XCell::String(t.clone()), if total { TITLE } else { TEXT }),
        Cell::Date(t) => match excel_date(t) {
            Some(serial) => (
                XCell::Number(serial),
                if t.len() == 7 { MONTH } else { DATE },
            ),
            None => (XCell::String(t.clone()), TEXT),
        },
        Cell::Fiat(d) => (
            XCell::Number(number(d)),
            if total { FIAT_TOTAL } else { FIAT },
        ),
        Cell::Btc(d) => (
            XCell::Number(number(d)),
            if total { BTC_TOTAL } else { BTC },
        ),
        // Above 2^53 a double cannot hold the integer; keep the exact text.
        Cell::Msat(v) => match i64::try_from(v.clone()) {
            Ok(i) if i.unsigned_abs() < (1 << 53) => (XCell::Number(i as f64), COUNT),
            _ => (XCell::String(v.to_string()), TEXT),
        },
        Cell::Count(i) => (XCell::Number(*i as f64), COUNT),
        Cell::Percent(d) => (XCell::Number(number(d)), PERCENT),
        // Formulas are written with their sheet context in `table_sheet`.
        Cell::Formula { cached, .. } => xlsx_cell(cached, total),
    }
}

fn width(kind: Kind) -> f64 {
    match kind {
        Kind::Text => 22.0,
        Kind::Code => 18.0,
        Kind::Date => 12.0,
        Kind::Fiat => 15.0,
        Kind::Btc => 15.0,
        Kind::Msat => 18.0,
        Kind::Count => 10.0,
        Kind::Percent => 9.0,
    }
}

fn sheet_name(id: &str) -> String {
    id.chars().take(31).collect()
}

/// The workbook: a Summary sheet of facts, then one sheet per table.
pub fn xlsx(doc: &Document) -> Result<Vec<u8>> {
    kassiber_xlsx::render(&xlsx_document(doc))
        .map_err(|e| AppError::new("internal_error", e.to_string()))
}

/// The workbook before rendering, for exports that append sheets of their
/// own (the full report's verification appendix).
pub fn xlsx_document(doc: &Document) -> kassiber_xlsx::Document {
    let mut summary = vec![
        Operation::Column {
            first: 0,
            last: 0,
            width: 34.0,
        },
        Operation::Column {
            first: 1,
            last: 1,
            width: 40.0,
        },
        Operation::Cell {
            row: 0,
            col: 0,
            value: XCell::String(doc.title.clone()),
            style: TITLE,
        },
        Operation::Cell {
            row: 1,
            col: 0,
            value: XCell::String(format!("{}  ·  {}", doc.book, doc.period)),
            style: MUTED,
        },
    ];
    if let Some(notice) = doc.preview_notice() {
        summary.push(Operation::Cell {
            row: 2,
            col: 0,
            value: XCell::String(notice.into()),
            style: HEADER,
        });
    }
    let mut row = 3u32;
    let mut sheets = Vec::new();
    for section in &doc.sections {
        let mut facts = Vec::new();
        for block in &section.blocks {
            match block {
                Block::Facts(rows) => facts.extend(
                    rows.iter()
                        .map(|f| (f.label.clone(), f.value.clone(), f.display.clone())),
                ),
                Block::Kpis(items) => facts.extend(
                    items
                        .iter()
                        .map(|k| (k.label.clone(), k.value.clone(), None)),
                ),
                Block::Table(table) => sheets.push(table_sheet(table)),
                Block::Note { text, .. } => {
                    facts.push((String::new(), Cell::Text(text.clone()), None))
                }
                Block::Chart(_) | Block::Flow(_) => (),
            }
        }
        if facts.is_empty() {
            continue;
        }
        summary.push(Operation::Cell {
            row,
            col: 0,
            value: XCell::String(section.title.clone()),
            style: HEADER,
        });
        summary.push(Operation::Cell {
            row,
            col: 1,
            value: XCell::Blank,
            style: HEADER,
        });
        row += 1;
        for (label, value, display) in facts {
            summary.push(Operation::Cell {
                row,
                col: 0,
                value: XCell::String(label),
                style: MUTED,
            });
            let (cell, style) = match display {
                Some(text) => (XCell::String(text), TEXT),
                None => xlsx_cell(&value, false),
            };
            summary.push(Operation::Cell {
                row,
                col: 1,
                value: cell,
                style,
            });
            row += 1;
        }
        row += 1;
    }
    let mut all = vec![Sheet {
        name: "summary".into(),
        operations: summary,
    }];
    all.extend(sheets);
    kassiber_xlsx::Document {
        title: doc.title.clone(),
        subject: format!("{} · {}", doc.book, doc.period),
        author: "Kassiber".into(),
        comments: doc
            .lang
            .tf("doc.input_version", &[("version", &doc.version_text())]),
        created: doc.generated_at.clone(),
        formats: styles(doc.lang),
        sheets: all,
        names: vec![],
    }
}

fn table_sheet(table: &Table) -> Sheet {
    let mut ops = Vec::new();
    for (col, column) in table.columns.iter().enumerate() {
        let col = col as u16;
        ops.push(Operation::Column {
            first: col,
            last: col,
            width: width(column.kind).max(column.label.chars().count() as f64 + 2.0),
        });
        ops.push(Operation::Cell {
            row: 0,
            col,
            value: XCell::String(column.label.clone()),
            style: HEADER,
        });
    }
    for (r, cells) in table.rows.iter().enumerate() {
        for (col, cell) in cells.iter().enumerate() {
            let (value, style) = match cell {
                Cell::Formula { expression, cached } => {
                    // Columns are named by key; the row is this sheet row.
                    let mut formula = expression.clone();
                    for (j, column) in table.columns.iter().enumerate() {
                        formula =
                            formula.replace(&format!("{{{}}}", column.key), &address(j, r + 1));
                    }
                    let (result, style) = xlsx_cell(cached, false);
                    let result = match result {
                        XCell::Number(n) => kassiber_pyjson::float_repr(n),
                        XCell::String(t) => t,
                        _ => String::new(),
                    };
                    (
                        XCell::Formula {
                            expression: formula,
                            result,
                        },
                        style,
                    )
                }
                _ => xlsx_cell(cell, false),
            };
            ops.push(Operation::Cell {
                row: r as u32 + 1,
                col: col as u16,
                value,
                style,
            });
        }
    }
    if let Some(total) = &table.total {
        let r = table.rows.len() as u32 + 1;
        for (col, cell) in total.iter().enumerate() {
            let (value, style) = xlsx_cell(cell, true);
            ops.push(Operation::Cell {
                row: r,
                col: col as u16,
                value,
                style,
            });
        }
    }
    ops.push(Operation::Freeze { row: 1, col: 0 });
    if !table.rows.is_empty() {
        ops.push(Operation::Autofilter {
            first_row: 0,
            first_col: 0,
            last_row: table.rows.len() as u32,
            last_col: table.columns.len() as u16 - 1,
        });
    }
    Sheet {
        name: sheet_name(table.id),
        operations: ops,
    }
}

// ------------------------------------------------------------------- CSV

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// One CSV file: a header of keys, then rows of machine text.
pub fn csv_table(header: &[&str], rows: impl Iterator<Item = Vec<String>>) -> Vec<u8> {
    let mut out = header
        .iter()
        .map(|h| csv_field(h))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for row in rows {
        out.push_str(
            &row.iter()
                .map(|c| csv_field(c))
                .collect::<Vec<_>>()
                .join(","),
        );
        out.push('\n');
    }
    out.into_bytes()
}

/// The CSV bundle: `summary.csv` for facts, one file per table, then
/// `manifest.json` describing and hashing them. CSV is a machine format in
/// every language: snake_case keys, plain decimals, ISO dates, no labels.
pub fn csv(doc: &Document) -> Result<Vec<(String, Vec<u8>)>> {
    let mut facts = Vec::new();
    if let Some(notice) = doc.preview_notice() {
        facts.push(("preview", "notice", Cell::Text(notice.into())));
    }
    let mut files: Vec<(String, Vec<u8>, Vec<&str>, usize)> = Vec::new();
    for section in &doc.sections {
        for block in &section.blocks {
            match block {
                Block::Facts(rows) => {
                    facts.extend(rows.iter().map(|f| (section.id, f.key, f.value.clone())))
                }
                Block::Kpis(items) => {
                    facts.extend(items.iter().map(|k| (section.id, k.key, k.value.clone())))
                }
                Block::Table(table) => {
                    let header: Vec<&str> = table.columns.iter().map(|c| c.key).collect();
                    let bytes = csv_table(
                        &header,
                        table.rows.iter().map(|r| r.iter().map(Cell::csv).collect()),
                    );
                    files.push((format!("{}.csv", table.id), bytes, header, table.rows.len()));
                }
                _ => (),
            }
        }
    }
    let unit = |c: &Cell| match c {
        Cell::Fiat(_) => doc.fiat.clone(),
        Cell::Btc(_) => "BTC".into(),
        Cell::Msat(_) => "msat".into(),
        Cell::Percent(_) => "percent".into(),
        _ => String::new(),
    };
    let summary_header = ["section", "key", "value", "unit"];
    let summary = csv_table(
        &summary_header,
        facts.iter().map(|(section, key, value)| {
            vec![
                section.to_string(),
                key.to_string(),
                value.csv(),
                unit(value),
            ]
        }),
    );
    files.insert(
        0,
        (
            "summary.csv".into(),
            summary,
            summary_header.to_vec(),
            facts.len(),
        ),
    );
    let listing = Value::List(
        files
            .iter()
            .map(|(name, bytes, header, rows)| {
                obj([
                    ("name", s(name.as_str())),
                    (
                        "columns",
                        Value::List(header.iter().map(|h| s(*h)).collect()),
                    ),
                    ("rows", Value::Int((*rows).into())),
                    ("sha256", s(format!("{:x}", Sha256::digest(bytes)))),
                ])
            })
            .collect(),
    );
    let mut manifest = obj([
        ("schema", s(CSV_SCHEMA)),
        ("report", s(doc.kind)),
        ("book", s(doc.book.as_str())),
        (
            "period",
            obj([
                ("start", s(doc.period_start.as_str())),
                ("end", s(doc.period_end.as_str())),
            ]),
        ),
        ("fiat_currency", s(doc.fiat.as_str())),
        ("country", s(doc.country.as_str())),
        ("generated_at", s(doc.generated_at.as_str())),
        (
            "journal_input_version",
            doc.input_version
                .map(|v| Value::Int(v.into()))
                .unwrap_or(Value::Null),
        ),
        ("files", listing),
    ]);
    if let Some(notice) = doc.preview_notice() {
        kassiber_envelope::set(&mut manifest, "preview_notice", s(notice));
    }
    let mut text = kassiber_pyjson::dumps(
        &manifest,
        &kassiber_pyjson::Options {
            ensure_ascii: false,
            indent: Some(kassiber_pyjson::Indent::Spaces(2)),
            ..Default::default()
        },
    )
    .map_err(|e| AppError::new("internal_error", e.to_string()))?;
    text.push('\n');
    let mut out: Vec<(String, Vec<u8>)> = files
        .into_iter()
        .map(|(name, bytes, _, _)| (name, bytes))
        .collect();
    out.push(("manifest.json".into(), text.into_bytes()));
    Ok(out)
}

/// The CSV bundle as one ZIP, for single-file CSV destinations.
pub fn csv_zip(doc: &Document) -> Result<Vec<u8>> {
    kassiber_xlsx::archive(&csv(doc)?).map_err(|e| AppError::new("internal_error", e.to_string()))
}
