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

mod locale;
mod strings;
mod summary;
pub use locale::Lang;
pub(crate) use summary::prepare as prepare_summary;
pub use summary::{summary, summary_model};

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
    Note { text: String, accent: bool },
}

#[derive(Clone, Debug)]
pub struct Section {
    pub id: &'static str,
    pub title: String,
    pub lead: String,
    pub page_break: bool,
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
    pub input_version: i64,
    pub fiat: String,
    pub country: String,
    pub lang: Lang,
    pub sections: Vec<Section>,
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
        }
    }
    /// A zero amount; sparse columns show it as `–`.
    fn is_zero(&self) -> bool {
        match self {
            Cell::Fiat(d) | Cell::Btc(d) | Cell::Percent(d) => d.is_zero(),
            Cell::Count(n) => *n == 0,
            Cell::Msat(n) => n.to_string() == "0",
            _ => false,
        }
    }
    /// CSV text: exact, plain, machine-readable.
    pub fn csv(&self) -> String {
        match self {
            Cell::Empty => String::new(),
            Cell::Text(t) | Cell::Date(t) => t.clone(),
            Cell::Fiat(d) | Cell::Btc(d) | Cell::Percent(d) => plain(d),
            Cell::Msat(n) => n.to_string(),
            Cell::Count(n) => n.to_string(),
        }
    }
}

/// A decimal without exponent or grouping.
pub fn plain(d: &Decimal) -> String {
    d.to_fixed_string().unwrap_or_else(|_| d.to_canonical())
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
            (true, _) if biggest >= 1e6 => (1e6, if lang == Lang::De { "\u{a0}Mio." } else { "M" }),
            (true, _) if biggest >= 1e3 => (1e3, if lang == Lang::De { "\u{a0}Tsd." } else { "k" }),
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
        Block::Note { text, accent } => obj([
            ("t", s("note")),
            ("text", s(text.as_str())),
            ("tone", s(if *accent { "accent" } else { "muted" })),
        ]),
    }
}

fn pdf_data(doc: &Document, stamp: kassiber_pdf::engine::Stamp) -> Value {
    let lang = doc.lang;
    obj([
        (
            "meta",
            obj([
                ("title", s(doc.title.as_str())),
                ("kicker", s(doc.kicker.as_str())),
                ("running_title", s(format!("Kassiber  ·  {}", doc.title))),
                ("book", s(doc.book.as_str())),
                ("period", s(doc.period.as_str())),
                (
                    "footer",
                    s(format!(
                        "{}  ·  {}",
                        lang.tf(
                            "doc.generated",
                            &[("at", &lang.date_time(&doc.generated_at))]
                        ),
                        lang.tf(
                            "doc.input_version",
                            &[("version", &doc.input_version.to_string())]
                        )
                    )),
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
            ]),
        ),
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
    let stamp = kassiber_pdf::engine::Stamp::parse(&doc.generated_at)
        .ok_or_else(|| AppError::new("internal_error", "invalid report timestamp"))?;
    let rendered = kassiber_pdf::engine::render(kassiber_pdf::engine::Document {
        template: kassiber_pdf::engine::REPORT,
        data: pdf_data(doc, stamp),
        files: Vec::new(),
        created: stamp,
    })
    .map_err(|e| AppError::new("internal_error", format!("PDF rendering failed: {e}")))?;
    Ok((rendered.bytes, rendered.pages.len()))
}

// ------------------------------------------------------------------ XLSX

use kassiber_xlsx::{Cell as XCell, Operation, Sheet, Style};

fn styles(lang: Lang) -> Vec<Style> {
    let (date, month) = match lang {
        Lang::En => ("yyyy-mm-dd", "yyyy-mm"),
        Lang::De => ("dd.mm.yyyy", "mm.yyyy"),
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
                Block::Chart(_) => (),
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
        name: "Summary".into(),
        operations: summary,
    }];
    all.extend(sheets);
    kassiber_xlsx::render(&kassiber_xlsx::Document {
        title: doc.title.clone(),
        subject: format!("{} · {}", doc.book, doc.period),
        author: "Kassiber".into(),
        comments: doc.lang.tf(
            "doc.input_version",
            &[("version", &doc.input_version.to_string())],
        ),
        created: doc.generated_at.clone(),
        formats: styles(doc.lang),
        sheets: all,
        names: vec![],
    })
    .map_err(|e| AppError::new("internal_error", e.to_string()))
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
            let (value, style) = xlsx_cell(cell, false);
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

fn csv_table(header: &[&str], rows: impl Iterator<Item = Vec<String>>) -> Vec<u8> {
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
    let manifest = obj([
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
            Value::Int(doc.input_version.into()),
        ),
        ("files", listing),
    ]);
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
