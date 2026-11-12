//! Visual projections for the assistant (#676, `core/chat_visuals.py`).
//!
//! The assistant chooses *what* to show (a source, a measure, filters, a
//! title); every figure comes from the same local reads the rest of the app
//! uses. These functions turn those payloads into a typed visual
//! specification, long-format `rows` and source-specific extras. A visual is
//! display only: it never grants ownership, carries basis or decides tax
//! facts, and it states the gaps it knows about as `caveats` instead of
//! plotting them as zero. `model_digest` is what the provider receives
//! instead of the full payload.

use kassiber_envelope::{dict, get, s, AppError};
use kassiber_pyjson::{Key, Value};

use crate::executor::Book;
use crate::ToolError;

const VISUAL_VERSION: i64 = 1;

/// `VISUAL_SOURCES`.
pub const VISUAL_SOURCES: [&str; 8] = [
    "overview",
    "balance_history",
    "portfolio",
    "tax_summary",
    "cash_flow",
    "transactions",
    "transaction_flow",
    "readiness",
];

/// `VISUAL_MEASURES`: the first measure is each source's default.
fn measures(source: &str) -> &'static [&'static str] {
    match source {
        "overview" => &["market_value"],
        "balance_history" => &["quantity", "market_value", "cost_basis"],
        "portfolio" => &["market_value", "quantity", "cost_basis", "unrealized_pnl"],
        "tax_summary" => &["gain_loss", "proceeds", "cost_basis"],
        "cash_flow" => &["quantity", "fiat_value"],
        "transactions" | "transaction_flow" => &["amount"],
        _ => &["status"],
    }
}

/// `_SOURCE_ARGUMENTS`: the filters each source accepts besides `source`,
/// `title` and `measure`.
fn source_arguments(source: &str) -> &'static [&'static str] {
    match source {
        "balance_history" => &["interval", "start", "end", "wallet", "asset", "limit"],
        "portfolio" => &["group_by", "asset"],
        "cash_flow" => &["period", "wallet"],
        "transactions" => &[
            "query",
            "direction",
            "wallet",
            "period",
            "status",
            "sort",
            "limit",
        ],
        "transaction_flow" => &["transaction"],
        _ => &[],
    }
}

const BALANCE_HISTORY_INTERVALS: [&str; 4] = ["hour", "day", "week", "month"];
const DEFAULT_BALANCE_HISTORY_INTERVAL: &str = "month";
const DEFAULT_BALANCE_HISTORY_LIMIT: i64 = 60;
const MAX_BALANCE_HISTORY_LIMIT: i64 = 240;
const BALANCE_HISTORY_REPORT_ROWS: i64 = 500;
const PORTFOLIO_GROUPS: [&str; 2] = ["wallet", "asset"];
const MAX_PORTFOLIO_ROWS: usize = 60;
const CASH_FLOW_PERIODS: [&str; 7] = [
    "30days", "3months", "6months", "ytd", "1year", "5years", "all",
];
const DEFAULT_CASH_FLOW_PERIOD: &str = "1year";
const TRANSACTION_DIRECTIONS: [&str; 2] = ["inbound", "outbound"];
const TRANSACTION_STATUSES: [&str; 4] = ["completed", "pending", "failed", "review"];
const TRANSACTION_SORTS: [&str; 4] = ["recent", "oldest", "largest", "smallest"];
const DEFAULT_TRANSACTION_LIMIT: i64 = 10;
const MAX_TRANSACTION_LIMIT: i64 = 25;
const MAX_OVERVIEW_SERIES_POINTS: usize = 160;
const MAX_OVERVIEW_WALLETS: usize = 6;
const MAX_TAX_DETAILS: usize = 200;
const MAX_NOTE_CHARS: usize = 80;
const MAX_TITLE_CHARS: usize = 80;
const MAX_FILTER_CHARS: usize = 200;
/// The model sees every row up to this many; above it, only the digest.
const MODEL_ROW_LIMIT: usize = 30;
const FIAT_MEASURES: [&str; 6] = [
    "market_value",
    "cost_basis",
    "unrealized_pnl",
    "gain_loss",
    "proceeds",
    "fiat_value",
];
const BTC_ASSETS: [&str; 4] = ["BTC", "LBTC", "L-BTC", "LNBTC"];
/// `JOURNAL_SOURCES`: report sources reading journal-derived state.
pub const JOURNAL_SOURCES: [&str; 4] = ["overview", "balance_history", "portfolio", "tax_summary"];

fn validation(message: impl Into<String>, details: Vec<(&str, Value)>) -> AppError {
    let error = AppError::new("validation", message.into());
    if details.is_empty() {
        error
    } else {
        error.with_details(dict(details))
    }
}

fn strs(items: &[&str]) -> Value {
    Value::List(items.iter().map(|item| s(*item)).collect())
}

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    get(value, key).unwrap_or(&Value::Null)
}

fn text_of(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        other => kassiber_envelope::py_str(other),
    }
}

/// `str(value or "")`.
fn or_text(value: &Value) -> String {
    if kassiber_envelope::truthy(value) {
        text_of(value)
    } else {
        String::new()
    }
}

/// `value or default` for a value kept as it is.
fn or_value(value: &Value, default: Value) -> Value {
    if kassiber_envelope::truthy(value) {
        value.clone()
    } else {
        default
    }
}

/// `x in choices` for a JSON value.
fn one_of(value: &Value, choices: &[&str]) -> Option<String> {
    match value {
        Value::Str(text) => choices
            .iter()
            .find(|choice| text.is(choice))
            .map(|choice| (*choice).to_owned()),
        _ => None,
    }
}

/// `_clean_text(value, field, limit=...)`: control and format characters
/// have no place in a chart label the model wrote.
fn clean_text(value: &Value, name: &str, limit: usize) -> Result<Option<String>, AppError> {
    let text = match value {
        Value::Null => return Ok(None),
        Value::Str(text) => crate::pyval::lossy(text),
        _ => {
            return Err(validation(
                format!("ui.visuals.render {name} must be a string"),
                vec![("field", s(name))],
            ))
        }
    };
    let kept: String = text
        .chars()
        .filter_map(|c| {
            let point = u32::from(c);
            if kassiber_pytext::is_space(point) {
                Some(' ')
            } else if !is_other_category(c) {
                Some(c)
            } else {
                None
            }
        })
        .collect();
    let collapsed = kept
        .split(|c: char| kassiber_pytext::is_space(u32::from(c)))
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    if collapsed.chars().count() > limit {
        return Err(validation(
            format!("ui.visuals.render {name} is too long"),
            vec![("field", s(name)), ("max_length", Value::from(limit))],
        ));
    }
    Ok((!collapsed.is_empty()).then_some(collapsed))
}

/// `unicodedata.category(char)[0] == "C"`: a printable character is never
/// of category C; of the non-printable ones only the separators (all of
/// them whitespace, kept above) are not.
fn is_other_category(c: char) -> bool {
    !kassiber_pytext::is_printable(u32::from(c))
}

/// `_enum(args, key, choices, default)`.
fn enumerated(
    args: &Value,
    key: &str,
    choices: &[&str],
    default: Option<&str>,
) -> Result<Option<String>, AppError> {
    let value = match get(args, key) {
        Some(value) => value.clone(),
        None => default.map_or(Value::Null, s),
    };
    if value == Value::Null {
        return Ok(None);
    }
    one_of(&value, choices).map(Some).ok_or_else(|| {
        validation(
            format!("ui.visuals.render {key} is unsupported"),
            vec![(key, value.clone()), ("supported", strs(choices))],
        )
    })
}

/// `_bounded_int(args, key, default, maximum)`.
fn bounded_int(args: &Value, key: &str, default: i64, maximum: i64) -> Result<i64, AppError> {
    let value = get(args, key).cloned().unwrap_or(Value::from(default));
    if let Value::Int(number) = &value {
        if let Ok(number) = i64::try_from(number.clone()) {
            if (1..=maximum).contains(&number) {
                return Ok(number);
            }
        }
    }
    Err(validation(
        format!("ui.visuals.render {key} must be an integer between 1 and {maximum}"),
        vec![(key, value)],
    ))
}

/// `normalize_visual_args(raw_args)`: validate the model's request and split
/// it into chart and read arguments.
pub fn normalize_visual_args(raw: &Value) -> Result<Value, AppError> {
    let empty = Value::Dict(Vec::new());
    let args = if matches!(raw, Value::Dict(_)) {
        raw
    } else {
        &empty
    };
    let source_value = field(args, "source").clone();
    let Some(source) = one_of(&source_value, &VISUAL_SOURCES) else {
        return Err(validation(
            "ui.visuals.render source is unsupported",
            vec![
                ("source", source_value),
                ("supported", strs(&VISUAL_SOURCES)),
            ],
        ));
    };
    let allowed: Vec<&str> = ["source", "title", "measure"]
        .into_iter()
        .chain(source_arguments(&source).iter().copied())
        .collect();
    let mut unknown: Vec<String> = match args {
        Value::Dict(pairs) => pairs
            .iter()
            .filter_map(|(key, _)| match key {
                Key::Str(name) if !allowed.iter().any(|known| name.is(known)) => {
                    Some(crate::pyval::lossy(name))
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    unknown.sort();
    if !unknown.is_empty() {
        return Err(validation(
            format!("ui.visuals.render does not accept these fields for {source}"),
            vec![
                ("unknown", Value::List(unknown.into_iter().map(s).collect())),
                ("source", s(source.as_str())),
            ],
        ));
    }
    let choices = measures(&source);
    let measure_value = get(args, "measure")
        .cloned()
        .unwrap_or_else(|| s(choices[0]));
    let Some(measure) = one_of(&measure_value, choices) else {
        return Err(validation(
            format!("ui.visuals.render measure is unsupported for {source}"),
            vec![("measure", measure_value), ("supported", strs(choices))],
        ));
    };
    let mut spec = vec![
        ("source", s(source.as_str())),
        ("measure", s(measure)),
        (
            "title",
            clean_text(field(args, "title"), "title", MAX_TITLE_CHARS)?.map_or(Value::Null, s),
        ),
    ];
    let text = |key: &str| clean_text(field(args, key), key, MAX_FILTER_CHARS);
    let mut report_args: Vec<(&str, Value)> = Vec::new();
    match source.as_str() {
        "balance_history" => {
            let interval = enumerated(
                args,
                "interval",
                &BALANCE_HISTORY_INTERVALS,
                Some(DEFAULT_BALANCE_HISTORY_INTERVAL),
            )?;
            let limit = bounded_int(
                args,
                "limit",
                DEFAULT_BALANCE_HISTORY_LIMIT,
                MAX_BALANCE_HISTORY_LIMIT,
            )?;
            let interval = interval.map_or(Value::Null, s);
            // The report truncates by row (one per asset and bucket); read as
            // much as it allows and keep whole buckets here instead.
            report_args.push(("interval", interval.clone()));
            report_args.push(("limit", Value::from(BALANCE_HISTORY_REPORT_ROWS)));
            for key in ["start", "end", "wallet", "asset"] {
                if let Some(value) = text(key)? {
                    report_args.push((key, s(value)));
                }
            }
            spec.push(("interval", interval));
            spec.push(("limit", Value::from(limit)));
        }
        "portfolio" => {
            let group = enumerated(args, "group_by", &PORTFOLIO_GROUPS, Some("wallet"))?;
            spec.push(("group_by", group.map_or(Value::Null, s)));
            if let Some(asset) = text("asset")? {
                spec.push(("asset", s(asset)));
            }
        }
        "cash_flow" => {
            let period = enumerated(
                args,
                "period",
                &CASH_FLOW_PERIODS,
                Some(DEFAULT_CASH_FLOW_PERIOD),
            )?
            .map_or(Value::Null, s);
            spec.push(("period", period.clone()));
            report_args.push(("period", period));
            if let Some(wallet) = text("wallet")? {
                report_args.push(("wallet", s(wallet)));
            }
        }
        "transactions" => {
            let sort = enumerated(args, "sort", &TRANSACTION_SORTS, Some("recent"))?;
            spec.push(("sort", sort.map_or(Value::Null, s)));
            let limit = bounded_int(
                args,
                "limit",
                DEFAULT_TRANSACTION_LIMIT,
                MAX_TRANSACTION_LIMIT,
            )?;
            spec.push(("limit", Value::from(limit)));
            for key in ["query", "wallet"] {
                if let Some(value) = text(key)? {
                    report_args.push((key, s(value)));
                }
            }
            if let Some(direction) = enumerated(args, "direction", &TRANSACTION_DIRECTIONS, None)? {
                report_args.push(("direction", s(direction)));
            }
            if let Some(status) = enumerated(args, "status", &TRANSACTION_STATUSES, None)? {
                report_args.push(("status", s(status)));
            }
            if let Some(period) = enumerated(args, "period", &CASH_FLOW_PERIODS, None)? {
                if period != "all" {
                    report_args.push(("period", s(period)));
                }
            }
        }
        "transaction_flow" => {
            let Some(transaction) = text("transaction")? else {
                return Err(validation(
                    "ui.visuals.render transaction_flow requires a transaction",
                    vec![],
                ));
            };
            report_args.push(("transaction", s(transaction)));
        }
        _ => {}
    }
    spec.push(("report_args", dict(report_args)));
    Ok(dict(spec))
}

// ---------------------------------------------------------------------------
// Shared helpers

fn unit(measure: &str, fiat_currency: &str) -> Value {
    if FIAT_MEASURES.contains(&measure) {
        return dict([("kind", s("fiat")), ("code", s(fiat_currency))]);
    }
    match measure {
        "amount" => dict([("kind", s("sats")), ("code", s("sat"))]),
        "status" => dict([("kind", s("count")), ("code", Value::Null)]),
        // Quantities stay in each row's own asset; the series key names it.
        _ => dict([("kind", s("asset")), ("code", Value::Null)]),
    }
}

/// `_number(value)`: a finite float, or `None`.
fn number(value: &Value) -> Option<f64> {
    let parsed = match value {
        Value::Null | Value::Bool(_) => return None,
        Value::Float(number) => *number,
        Value::Int(number) => number.to_string().parse::<f64>().ok()?,
        Value::Str(text) => kassiber_pytext::parse_float(&crate::pyval::lossy(text)).ok()?,
        _ => return None,
    };
    parsed.is_finite().then_some(parsed)
}

fn number_value(value: &Value) -> Value {
    number(value).map_or(Value::Null, Value::Float)
}

/// `_int(value)`: `int(value)`, or `None`.
fn int(value: &Value) -> Option<i64> {
    match value {
        Value::Null | Value::Bool(_) => None,
        Value::Int(number) => i64::try_from(number.clone()).ok(),
        Value::Float(number) if number.is_finite() => Some(number.trunc() as i64),
        Value::Str(text) => kassiber_pytext::parse_int(&crate::pyval::lossy(text))
            .ok()
            .and_then(|number| i64::try_from(number).ok()),
        _ => None,
    }
}

/// `int(value or 0)`.
fn count(value: &Value) -> i64 {
    if kassiber_envelope::truthy(value) {
        int(value).unwrap_or(0)
    } else {
        0
    }
}

/// `readiness_from_blockers(payload)`: what the report blockers say about
/// the figures a report visual draws.
pub fn readiness_from_blockers(payload: &Value) -> Value {
    let ids: Vec<String> = match get(payload, "blockers") {
        Some(Value::List(blockers)) => blockers
            .iter()
            .filter(|blocker| matches!(blocker, Value::Dict(_)))
            .map(|blocker| text_of(field(blocker, "id")))
            .collect(),
        _ => Vec::new(),
    };
    let has = |id: &str| ids.iter().any(|known| known == id);
    let health = field(payload, "health");
    let journals = field(health, "journals");
    let rates = field(field(payload, "rates_coverage"), "summary");
    dict([
        (
            "journals_stale",
            Value::Bool(
                has("journals_stale")
                    || kassiber_envelope::truthy(field(journals, "needs_processing")),
            ),
        ),
        (
            "quarantine",
            Value::from(count(field(journals, "quarantine_count"))),
        ),
        (
            "custody_unresolved",
            Value::Bool(has("custody_quantity_unresolved")),
        ),
        (
            "unpriced_transactions",
            Value::from(count(field(rates, "missing_price_transactions"))),
        ),
        ("sync_failed", Value::Bool(has("sync_failed"))),
    ])
}

fn caveat(code: &str, count: Option<i64>) -> Value {
    match count {
        Some(count) => dict([("code", s(code)), ("count", Value::from(count))]),
        None => dict([("code", s(code))]),
    }
}

/// `_readiness_caveats(readiness)`.
fn readiness_caveats(readiness: Option<&Value>) -> Vec<Value> {
    let Some(readiness) = readiness.filter(|value| kassiber_envelope::truthy(value)) else {
        return Vec::new();
    };
    let flag = |key: &str| kassiber_envelope::truthy(field(readiness, key));
    let mut caveats = Vec::new();
    if flag("journals_stale") {
        caveats.push(caveat("journals_stale", None));
    }
    let quarantines = count(field(readiness, "quarantine"));
    if quarantines != 0 {
        caveats.push(caveat("quarantines", Some(quarantines)));
    }
    if flag("custody_unresolved") {
        caveats.push(caveat("custody_unresolved", None));
    }
    let unpriced = count(field(readiness, "unpriced_transactions"));
    if unpriced != 0 {
        caveats.push(caveat("unpriced_transactions", Some(unpriced)));
    }
    if flag("sync_failed") {
        caveats.push(caveat("sync_failed", None));
    }
    caveats
}

/// `_header(spec, chart=..., fiat_currency=..., **extra)`.
fn header(
    spec: &Value,
    chart: &str,
    fiat_currency: &str,
    extra: Vec<(&str, Value)>,
) -> Vec<(Key, Value)> {
    let source = text_of(field(spec, "source"));
    let measure = text_of(field(spec, "measure"));
    let mut visual = vec![
        (Key::from("version"), Value::from(VISUAL_VERSION)),
        (Key::from("source"), s(source.as_str())),
        (Key::from("chart"), s(chart)),
        (Key::from("measure"), s(measure.as_str())),
        (Key::from("measures"), strs(measures(&source))),
        (Key::from("title"), field(spec, "title").clone()),
        (Key::from("unit"), unit(&measure, fiat_currency)),
        (Key::from("fiat_currency"), s(fiat_currency)),
    ];
    visual.extend(
        extra
            .into_iter()
            .map(|(key, value)| (Key::from(key), value)),
    );
    visual
}

/// `_downsample(rows, limit)`: `limit` evenly spaced points, always the
/// first and the last.
fn downsample(rows: Vec<Value>, limit: usize) -> Vec<Value> {
    if rows.len() <= limit {
        return rows;
    }
    let step = (rows.len() - 1) as f64 / (limit - 1) as f64;
    (0..limit)
        .map(|index| rows[(index as f64 * step).round_ties_even() as usize].clone())
        .collect()
}

/// What a source builder makes: the visual header, rows, caveats, extras.
type Built = (
    Vec<(Key, Value)>,
    Vec<Value>,
    Vec<Value>,
    Vec<(&'static str, Value)>,
);

fn overview_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let fiat = field(payload, "fiat");
    let completeness = field(fiat, "completeness");
    let balance = field(payload, "balanceSummary");
    let market_missing = kassiber_envelope::truthy(field(completeness, "marketRateMissing"));
    let basis_complete = kassiber_envelope::truthy(field(completeness, "costBasisComplete"));
    let price = field(payload, "marketRate");
    let unless_missing = |value: &Value| {
        if market_missing {
            Value::Null
        } else {
            number_value(value)
        }
    };
    let stats = vec![
        ("portfolio_value", unless_missing(field(fiat, "eurBalance"))),
        ("btc_balance", number_value(field(balance, "totalBtc"))),
        ("cost_basis", number_value(field(fiat, "eurCostBasis"))),
        ("unrealized", unless_missing(field(fiat, "eurUnrealized"))),
        ("realized_ytd", number_value(field(fiat, "eurRealizedYTD"))),
        ("btc_price", unless_missing(field(price, "rate"))),
        ("price_at", field(price, "timestamp").clone()),
        ("basis_complete", Value::Bool(basis_complete)),
        (
            "transactions",
            int(field(field(payload, "status"), "transactionCount"))
                .map_or(Value::Null, Value::from),
        ),
    ];
    let series: Vec<Value> = match get(payload, "portfolioSeries") {
        Some(Value::List(points)) => points
            .iter()
            .filter(|point| {
                matches!(point, Value::Dict(_)) && kassiber_envelope::truthy(field(point, "date"))
            })
            .map(|point| {
                dict([
                    ("date", field(point, "date").clone()),
                    ("balance_btc", number_value(field(point, "balanceBtc"))),
                    ("market_value", number_value(field(point, "valueEur"))),
                    ("cost_basis", number_value(field(point, "costBasisEur"))),
                ])
            })
            .collect(),
        _ => Vec::new(),
    };
    let series = downsample(series, MAX_OVERVIEW_SERIES_POINTS);
    let mut connections: Vec<(String, f64)> = match get(payload, "connections") {
        Some(Value::List(items)) => items
            .iter()
            .filter(|item| matches!(item, Value::Dict(_)))
            .map(|item| {
                let balance = number(field(item, "balance"))
                    .filter(|n| *n != 0.0)
                    .unwrap_or(0.0);
                (or_text(field(item, "label")), balance)
            })
            .collect(),
        _ => Vec::new(),
    };
    connections.sort_by(|a, b| {
        (-a.1)
            .partial_cmp(&-b.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.cmp(&b.0))
    });
    let total: f64 = connections
        .iter()
        .map(|(_, balance)| balance.max(0.0))
        .sum();
    let share = |balance: f64| {
        if total != 0.0 {
            balance.max(0.0) / total
        } else {
            0.0
        }
    };
    let mut wallets: Vec<Value> = connections
        .iter()
        .take(MAX_OVERVIEW_WALLETS)
        .map(|(label, balance)| {
            dict([
                ("label", s(label.as_str())),
                ("balance_btc", Value::Float(*balance)),
                ("share", Value::Float(share(*balance))),
            ])
        })
        .collect();
    let rest = &connections[connections.len().min(MAX_OVERVIEW_WALLETS)..];
    if !rest.is_empty() {
        let rest_total: f64 = rest.iter().map(|(_, balance)| balance.max(0.0)).sum();
        wallets.push(dict([
            ("label", Value::Null),
            ("other_count", Value::from(rest.len())),
            ("balance_btc", Value::Float(rest_total)),
            (
                "share",
                Value::Float(if total != 0.0 {
                    rest_total / total
                } else {
                    0.0
                }),
            ),
        ]));
    }
    let reasons: Vec<String> = match get(completeness, "reasons") {
        Some(Value::List(items)) => items.iter().map(text_of).collect(),
        _ => Vec::new(),
    };
    let mut caveats = Vec::new();
    if reasons.iter().any(|reason| reason == "journals_stale") {
        caveats.push(caveat("journals_stale", None));
    }
    if kassiber_envelope::truthy(field(completeness, "quarantineCount")) {
        caveats.push(caveat(
            "quarantines",
            int(field(completeness, "quarantineCount")),
        ));
    }
    if reasons.iter().any(|reason| reason == "custody_unresolved") {
        caveats.push(caveat("custody_unresolved", None));
    }
    if kassiber_envelope::truthy(field(completeness, "missingPriceCount")) {
        caveats.push(caveat(
            "missing_prices",
            int(field(completeness, "missingPriceCount")),
        ));
    }
    if market_missing {
        caveats.push(caveat("market_rate_missing", None));
    }
    if !basis_complete && caveats.is_empty() {
        caveats.push(caveat("basis_incomplete", None));
    }
    let rows = stats
        .iter()
        .filter(|(key, _)| !["basis_complete", "price_at"].contains(key))
        .map(|(key, value)| dict([("metric", s(*key)), ("value", value.clone())]))
        .collect();
    let visual = header(spec, "dashboard", fiat_currency, vec![]);
    let extras = vec![
        ("stats", dict(stats)),
        ("series", Value::List(series)),
        ("wallets", Value::List(wallets)),
        (
            "completeness",
            dict([
                ("state", field(completeness, "state").clone()),
                ("basis_complete", Value::Bool(basis_complete)),
            ]),
        ),
    ];
    (visual, rows, caveats, extras)
}

// --- Calendar: Python's aware datetimes, floored in their own offset.

const DAY_US: i64 = 86_400_000_000;
const HOUR_US: i64 = 3_600_000_000;

/// An aware moment: microseconds since 0001-01-01 in its own local calendar,
/// and its UTC offset.
#[derive(Clone, Copy)]
struct Moment {
    local_us: i64,
    offset_us: i64,
}

impl Moment {
    fn utc_us(self) -> i64 {
        self.local_us - self.offset_us
    }
}

/// `_parse_period(value)`: naive times are UTC.
fn parse_period(value: &Value) -> Option<Moment> {
    let Value::Str(text) = value else {
        return None;
    };
    let text = crate::pyval::lossy(text);
    if text.is_empty() {
        return None;
    }
    let parsed = kassiber_pytext::iso_datetime(&text.replace('Z', "+00:00"))?;
    Some(Moment {
        local_us: parsed.local_us,
        offset_us: parsed.offset_us.unwrap_or(0),
    })
}

fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn month_days(year: i64, month: i64) -> i64 {
    match month {
        2 if leap(year) => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days from 0001-01-01 to `year-month-day`.
fn ordinal(year: i64, month: i64, day: i64) -> i64 {
    let previous = year - 1;
    let mut days = previous * 365 + previous / 4 - previous / 100 + previous / 400;
    for earlier in 1..month {
        days += month_days(year, earlier);
    }
    days + day - 1
}

/// `(year, month, day)` of a day count from 0001-01-01.
fn civil(days: i64) -> (i64, i64, i64) {
    let mut year = (days * 400 / 146_097) + 1;
    while ordinal(year, 1, 1) > days {
        year -= 1;
    }
    while ordinal(year + 1, 1, 1) <= days {
        year += 1;
    }
    let mut month = 1;
    while month < 12 && ordinal(year, month + 1, 1) <= days {
        month += 1;
    }
    (year, month, days - ordinal(year, month, 1) + 1)
}

/// `core_reports._floor_to_interval(dt, interval)`.
fn floor_to_interval(moment: Moment, interval: &str) -> Moment {
    let day = moment.local_us.div_euclid(DAY_US);
    let local_us = match interval {
        "hour" => moment.local_us.div_euclid(HOUR_US) * HOUR_US,
        "day" => day * DAY_US,
        // 0001-01-01 is a Monday, Python's weekday 0.
        "week" => (day - day.rem_euclid(7)) * DAY_US,
        _ => {
            let (year, month, _) = civil(day);
            ordinal(year, month, 1) * DAY_US
        }
    };
    Moment {
        local_us,
        offset_us: moment.offset_us,
    }
}

/// `core_reports._next_interval(dt, interval)`.
fn next_interval(moment: Moment, interval: &str) -> Moment {
    let local_us = match interval {
        "hour" => moment.local_us + HOUR_US,
        "day" => moment.local_us + DAY_US,
        "week" => moment.local_us + 7 * DAY_US,
        _ => {
            let day = moment.local_us.div_euclid(DAY_US);
            let within = moment.local_us.rem_euclid(DAY_US);
            let (year, month, date) = civil(day);
            let (year, month) = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
            ordinal(year, month, date) * DAY_US + within
        }
    };
    Moment {
        local_us,
        offset_us: moment.offset_us,
    }
}

/// `_period_text(value)`: `%Y-%m-%dT%H:%M:%SZ` in UTC.
fn period_text(moment: Moment) -> String {
    let utc = moment.utc_us();
    let (year, month, day) = civil(utc.div_euclid(DAY_US));
    let seconds = utc.rem_euclid(DAY_US) / 1_000_000;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// `_calendar_periods(first, last, interval)`: every bucket start from
/// `first` to `last`, the way the report steps.
fn calendar_periods(first: Moment, last: Moment, interval: &str) -> Vec<String> {
    let mut periods = Vec::new();
    let mut cursor = floor_to_interval(first, interval);
    let end = floor_to_interval(last, interval);
    while cursor.utc_us() <= end.utc_us() && periods.len() as i64 <= MAX_BALANCE_HISTORY_LIMIT * 40
    {
        periods.push(period_text(cursor));
        cursor = next_interval(cursor, interval);
    }
    periods
}

fn balance_history_visual(
    spec: &Value,
    payload: &Value,
    fiat_currency: &str,
    now: Moment,
) -> Built {
    let interval_value = field(spec, "interval");
    let interval = if kassiber_envelope::truthy(interval_value) {
        text_of(interval_value)
    } else {
        DEFAULT_BALANCE_HISTORY_INTERVAL.to_owned()
    };
    let source_rows: Vec<&Value> = match get(payload, "rows") {
        Some(Value::List(rows)) => rows
            .iter()
            .filter(|row| parse_period(field(row, "period_start")).is_some())
            .collect(),
        _ => Vec::new(),
    };
    let mut by_key: Vec<((String, String), &Value)> = Vec::new();
    let mut assets: Vec<String> = Vec::new();
    for row in source_rows {
        let period = period_text(parse_period(field(row, "period_start")).expect("filtered"));
        let asset = or_text(field(row, "asset"));
        if !assets.contains(&asset) {
            assets.push(asset.clone());
        }
        let key = (period, asset);
        match by_key.iter_mut().find(|(existing, _)| *existing == key) {
            Some((_, slot)) => *slot = row,
            None => by_key.push((key, row)),
        }
    }
    let summary = field(payload, "summary");
    let filters = field(payload, "filters");
    let mut truncated = kassiber_envelope::truthy(field(summary, "truncated"));
    let mut present: Vec<String> = by_key
        .iter()
        .map(|((period, _), _)| period.clone())
        .collect();
    present.sort();
    present.dedup();
    let mut periods: Vec<String> = Vec::new();
    if !present.is_empty() {
        let mut first = parse_period(&s(present[0].as_str())).expect("written above");
        if truncated && present.len() > 1 {
            // The report kept its latest rows, so its oldest bucket may have
            // lost some assets. Start after it rather than plot a partial one.
            first = parse_period(&s(present[1].as_str())).expect("written above");
        } else if !truncated {
            // A start before the first movement shows the empty months too.
            if let Some(start) = parse_period(field(filters, "start")) {
                if start.utc_us() < first.utc_us() {
                    first = start;
                }
            }
        }
        // The calendar comes from the range, not from the rows: a full exit
        // must fall to zero rather than end on the last held balance.
        let last = parse_period(field(filters, "end")).unwrap_or(now);
        let latest = parse_period(&s(present[present.len() - 1].as_str())).expect("written above");
        let last = if latest.utc_us() > last.utc_us() {
            latest
        } else {
            last
        };
        periods = calendar_periods(first, last, &interval);
    }
    let limit = int(field(spec, "limit"))
        .filter(|limit| *limit != 0)
        .unwrap_or(DEFAULT_BALANCE_HISTORY_LIMIT) as usize;
    if periods.len() > limit {
        truncated = true;
        periods = periods.split_off(periods.len() - limit);
    }
    // Series in the order they appear within the buckets shown.
    let mut shown: Vec<String> = Vec::new();
    for ((period, asset), _) in &by_key {
        if periods.contains(period) && !shown.contains(asset) {
            shown.push(asset.clone());
        }
    }
    if !shown.is_empty() {
        assets = shown;
    }
    let mut rows = Vec::new();
    let mut missing_prices = 0;
    for period in &periods {
        for asset in &assets {
            let found = by_key
                .iter()
                .find(|((p, a), _)| p == period && a == asset)
                .map(|(_, row)| *row);
            let Some(row) = found else {
                // No row is a zero balance: every measure of it is zero.
                rows.push(dict([
                    ("period", s(period.as_str())),
                    ("asset", s(asset.as_str())),
                    ("quantity", Value::Float(0.0)),
                    ("market_value", Value::Float(0.0)),
                    ("cost_basis", Value::Float(0.0)),
                ]));
                continue;
            };
            let quantity = number(field(row, "quantity"));
            let mut market_value = number(field(row, "market_value"));
            // A held balance worth exactly zero is a missing price, not zero.
            if quantity.is_some_and(|q| q != 0.0) && market_value.is_none_or(|v| v == 0.0) {
                market_value = None;
                missing_prices += 1;
            }
            rows.push(dict([
                ("period", s(period.as_str())),
                ("asset", s(asset.as_str())),
                ("quantity", quantity.map_or(Value::Null, Value::Float)),
                (
                    "market_value",
                    market_value.map_or(Value::Null, Value::Float),
                ),
                (
                    "cost_basis",
                    number_value(field(row, "cumulative_cost_basis")),
                ),
            ]));
        }
    }
    let mut caveats = Vec::new();
    if missing_prices != 0 {
        caveats.push(caveat("missing_prices", Some(missing_prices)));
    }
    if truncated {
        caveats.push(caveat("truncated", None));
    }
    let shown_filters: Vec<(&str, Value)> = ["start", "end", "wallet", "asset"]
        .into_iter()
        .filter(|key| kassiber_envelope::truthy(field(filters, key)))
        .map(|key| (key, field(filters, key).clone()))
        .collect();
    let visual = header(
        spec,
        "line",
        fiat_currency,
        vec![
            ("x_key", s("period")),
            ("series_key", s("asset")),
            ("interval", s(interval)),
            ("filters", dict(shown_filters)),
        ],
    );
    (visual, rows, caveats, vec![])
}

fn sort_key_abs(value: &Value) -> f64 {
    match value {
        Value::Float(number) => number.abs(),
        _ => 0.0,
    }
}

fn portfolio_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let filter = match get(spec, "asset") {
        Some(Value::Str(asset)) if asset.code_points().next().is_some() => {
            Some(crate::pyval::lossy(asset))
        }
        _ => None,
    };
    let mut rows: Vec<Value> = Vec::new();
    let mut missing_prices = 0;
    if let Some(Value::List(items)) = get(payload, "rows") {
        for row in items {
            let asset = or_text(field(row, "asset"));
            if let Some(filter) = &filter {
                if asset.to_uppercase() != filter.to_uppercase() {
                    continue;
                }
            }
            let quantity = number(field(row, "quantity"));
            let mut market_value = number(field(row, "market_value"));
            let mut unrealized = number(field(row, "unrealized_pnl"));
            if quantity.is_some_and(|q| q != 0.0) && market_value.is_none_or(|v| v == 0.0) {
                // No price: neither the value nor the unrealized result is known.
                market_value = None;
                unrealized = None;
                missing_prices += 1;
            }
            rows.push(dict([
                ("wallet", s(or_text(field(row, "wallet")))),
                ("asset", s(asset)),
                ("quantity", quantity.map_or(Value::Null, Value::Float)),
                (
                    "market_value",
                    market_value.map_or(Value::Null, Value::Float),
                ),
                ("cost_basis", number_value(field(row, "cost_basis"))),
                (
                    "unrealized_pnl",
                    unrealized.map_or(Value::Null, Value::Float),
                ),
            ]));
        }
    }
    rows.sort_by(|a, b| {
        sort_key_abs(field(b, "market_value"))
            .partial_cmp(&sort_key_abs(field(a, "market_value")))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| text_of(field(a, "wallet")).cmp(&text_of(field(b, "wallet"))))
            .then_with(|| text_of(field(a, "asset")).cmp(&text_of(field(b, "asset"))))
    });
    let mut caveats = Vec::new();
    if missing_prices != 0 {
        caveats.push(caveat("missing_prices", Some(missing_prices)));
    }
    if rows.len() > MAX_PORTFOLIO_ROWS {
        caveats.push(caveat(
            "truncated",
            Some((rows.len() - MAX_PORTFOLIO_ROWS) as i64),
        ));
        rows.truncate(MAX_PORTFOLIO_ROWS);
    }
    let visual = header(
        spec,
        "allocation",
        fiat_currency,
        vec![
            ("group_by", field(spec, "group_by").clone()),
            (
                "filters",
                match filter {
                    Some(asset) => dict([("asset", s(asset))]),
                    None => Value::Dict(Vec::new()),
                },
            ),
        ],
    );
    (visual, rows, caveats, vec![])
}

fn tax_summary_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let items: Vec<&Value> = match get(payload, "rows") {
        Some(Value::List(items)) => items.iter().collect(),
        _ => Vec::new(),
    };
    let of_type = |kind: &'static str| {
        items.iter().copied().filter(move |row| {
            field(row, "row_type") == &s(kind) && int(field(row, "year")).is_some()
        })
    };
    let rows: Vec<Value> = of_type("year_total")
        .map(|row| {
            dict([
                (
                    "year",
                    Value::from(int(field(row, "year")).expect("filtered")),
                ),
                ("proceeds", number_value(field(row, "proceeds"))),
                ("cost_basis", number_value(field(row, "cost_basis"))),
                ("gain_loss", number_value(field(row, "gain_loss"))),
            ])
        })
        .collect();
    let mut details: Vec<Value> = of_type("detail")
        .map(|row| {
            dict([
                (
                    "year",
                    Value::from(int(field(row, "year")).expect("filtered")),
                ),
                ("asset", or_value(field(row, "asset"), s(""))),
                (
                    "transaction_type",
                    or_value(field(row, "transaction_type"), s("")),
                ),
                (
                    "capital_gains_type",
                    or_value(field(row, "capital_gains_type"), s("")),
                ),
                ("quantity", number_value(field(row, "quantity"))),
                ("proceeds", number_value(field(row, "proceeds"))),
                ("cost_basis", number_value(field(row, "cost_basis"))),
                ("gain_loss", number_value(field(row, "gain_loss"))),
            ])
        })
        .collect();
    let mut caveats = Vec::new();
    if details.len() > MAX_TAX_DETAILS {
        caveats.push(caveat(
            "truncated",
            Some((details.len() - MAX_TAX_DETAILS) as i64),
        ));
        details = details.split_off(details.len() - MAX_TAX_DETAILS);
    }
    let visual = header(spec, "years", fiat_currency, vec![("x_key", s("year"))]);
    (
        visual,
        rows,
        caveats,
        vec![("details", Value::List(details))],
    )
}

/// `_flow_stats(stats)`.
fn flow_stats(stats: &Value) -> Value {
    let largest = match get(stats, "largest") {
        Some(largest @ Value::Dict(_)) => dict([
            ("label", s(or_text(field(largest, "label")))),
            ("btc", number_value(field(largest, "btc"))),
            ("fiat", number_value(field(largest, "eur"))),
        ]),
        _ => Value::Null,
    };
    dict([
        ("count", Value::from(count(field(stats, "count")))),
        (
            "btc",
            Value::Float(
                number(field(stats, "btc"))
                    .filter(|n| *n != 0.0)
                    .unwrap_or(0.0),
            ),
        ),
        (
            "fiat",
            Value::Float(
                number(field(stats, "eur"))
                    .filter(|n| *n != 0.0)
                    .unwrap_or(0.0),
            ),
        ),
        (
            "missing_prices",
            Value::from(count(field(stats, "missingPrice"))),
        ),
        ("largest", largest),
    ])
}

fn cash_flow_bucket(period: &Value) -> &'static str {
    match text_of(period).as_str() {
        "30days" => "day",
        "3months" => "week",
        "5years" | "all" => "quarter",
        _ => "month",
    }
}

fn cash_flow_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let mut rows = Vec::new();
    let mut missing_prices = 0;
    if let Some(Value::List(buckets)) = get(payload, "buckets") {
        for bucket in buckets {
            let stats = field(bucket, "stats");
            let incoming = flow_stats(field(stats, "incoming"));
            let outgoing = flow_stats(field(stats, "outgoing"));
            let missing = count(field(&incoming, "missing_prices"))
                + count(field(&outgoing, "missing_prices"));
            missing_prices += missing;
            rows.push(dict([
                ("period", field(bucket, "bucketStart").clone()),
                ("bucket", field(bucket, "bucketKey").clone()),
                ("in_btc", field(&incoming, "btc").clone()),
                ("out_btc", field(&outgoing, "btc").clone()),
                ("in_fiat", field(&incoming, "fiat").clone()),
                ("out_fiat", field(&outgoing, "fiat").clone()),
                ("in_count", field(&incoming, "count").clone()),
                ("out_count", field(&outgoing, "count").clone()),
                (
                    "transfer_count",
                    Value::from(count(field(field(stats, "transfers"), "count"))),
                ),
                (
                    "swap_count",
                    Value::from(count(field(field(stats, "swaps"), "count"))),
                ),
                ("missing_prices", Value::from(missing)),
            ]));
        }
    }
    let summary = field(payload, "summary");
    let totals = dict([
        ("incoming", flow_stats(field(summary, "incoming"))),
        ("outgoing", flow_stats(field(summary, "outgoing"))),
        ("transfers", flow_stats(field(summary, "transfers"))),
        ("swaps", flow_stats(field(summary, "swaps"))),
        ("review", Value::from(count(field(summary, "review")))),
        ("pending", Value::from(count(field(summary, "pending")))),
    ]);
    let mut caveats = Vec::new();
    if missing_prices != 0 {
        // The page counts an unpriced movement as zero fiat; say so.
        caveats.push(caveat("missing_prices", Some(missing_prices)));
    }
    let window = field(payload, "window");
    let wallet = field(field(spec, "report_args"), "wallet");
    let visual = header(
        spec,
        "cash_flow",
        fiat_currency,
        vec![
            ("x_key", s("period")),
            ("period", field(spec, "period").clone()),
            ("bucket", s(cash_flow_bucket(field(spec, "period")))),
            (
                "window",
                dict([
                    ("since", field(window, "since").clone()),
                    ("until", field(window, "until").clone()),
                ]),
            ),
            (
                "filters",
                if kassiber_envelope::truthy(wallet) {
                    dict([("wallet", wallet.clone())])
                } else {
                    Value::Dict(Vec::new())
                },
            ),
        ],
    );
    (visual, rows, caveats, vec![("totals", totals)])
}

/// `_transaction_row(tx)`.
fn transaction_row(tx: &Value) -> Value {
    // Lightning amounts can be fractions of a sat; the sign must survive.
    let amount = number(field(tx, "amountSat"))
        .filter(|n| *n != 0.0)
        .unwrap_or(0.0);
    let note: String = or_text(field(tx, "note"))
        .chars()
        .take(MAX_NOTE_CHARS)
        .collect();
    let date = if kassiber_envelope::truthy(field(tx, "occurredAt")) {
        field(tx, "occurredAt").clone()
    } else {
        field(tx, "date").clone()
    };
    let tags: Vec<Value> = match get(tx, "tags") {
        Some(Value::List(tags)) => tags
            .iter()
            .take(4)
            .map(|tag| s(kassiber_envelope::py_str(tag)))
            .collect(),
        _ => Vec::new(),
    };
    dict([
        ("id", s(or_text(field(tx, "id")))),
        ("date", date),
        (
            "direction",
            s(if amount > 0.0 {
                "in"
            } else if amount < 0.0 {
                "out"
            } else {
                "none"
            }),
        ),
        ("type", or_value(field(tx, "type"), s(""))),
        ("asset", or_value(field(tx, "asset"), s(""))),
        ("amount_sat", Value::Float(amount)),
        (
            "fee_sat",
            Value::Float(
                number(field(tx, "feeSat"))
                    .filter(|n| *n != 0.0)
                    .unwrap_or(0.0),
            ),
        ),
        ("fiat_value", number_value(field(tx, "eur"))),
        ("wallet", or_value(field(tx, "account"), s(""))),
        ("counterparty", or_value(field(tx, "counter"), s(""))),
        ("note", s(note)),
        ("status", or_value(field(tx, "queueStatus"), s(""))),
        ("tags", Value::List(tags)),
    ])
}

fn transactions_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let rows: Vec<Value> = match get(payload, "txs") {
        Some(Value::List(txs)) => txs
            .iter()
            .filter(|tx| matches!(tx, Value::Dict(_)))
            .map(transaction_row)
            .collect(),
        _ => Vec::new(),
    };
    let total = int(field(payload, "total"));
    let mut caveats = Vec::new();
    if let Some(total) = total.filter(|total| *total > rows.len() as i64) {
        caveats.push(caveat("more_transactions", Some(total - rows.len() as i64)));
    }
    // Amounts of different assets do not add up; totals only within BTC.
    let btc_only = rows
        .iter()
        .all(|row| BTC_ASSETS.contains(&text_of(field(row, "asset")).to_uppercase().as_str()));
    let amounts: Vec<f64> = rows
        .iter()
        .filter_map(|row| number(field(row, "amount_sat")))
        .collect();
    let in_sat: f64 = amounts.iter().filter(|a| **a > 0.0).sum();
    let out_sat: f64 = amounts.iter().filter(|a| **a < 0.0).sum();
    let summary = dict([
        ("shown", Value::from(rows.len())),
        ("total", Value::from(total.unwrap_or(rows.len() as i64))),
        (
            "in_sat",
            if btc_only {
                sum_value(in_sat, amounts.iter().any(|a| *a > 0.0))
            } else {
                Value::Null
            },
        ),
        (
            "out_sat",
            if btc_only {
                sum_value(-out_sat, amounts.iter().any(|a| *a < 0.0))
            } else {
                Value::Null
            },
        ),
    ]);
    let visual = header(
        spec,
        "list",
        fiat_currency,
        vec![
            ("sort", field(spec, "sort").clone()),
            ("filters", field(spec, "report_args").clone()),
        ],
    );
    (visual, rows, caveats, vec![("summary", summary)])
}

/// Python's `sum()` of floats is the int `0` over no items.
fn sum_value(total: f64, any: bool) -> Value {
    if any {
        Value::Float(total)
    } else {
        Value::from(0)
    }
}

/// `_flow_row(node, side, position)`.
fn flow_row(node: &Value, side: &str, position: i64) -> Value {
    let overflow = kassiber_envelope::truthy(field(node, "overflow"));
    let (label_kind, label) = if overflow {
        ("overflow", or_text(field(node, "label")))
    } else if side == "fee" {
        ("fee", "Miner fee".to_owned())
    } else {
        // Only the user's own wallets are named; other parties' outpoints and
        // addresses stay out of the visual even where the graph read has them.
        let ownership = text_of(field(node, "ownership"));
        match field(node, "wallet") {
            Value::Str(wallet)
                if ["owned", "ambiguous"].contains(&ownership.as_str())
                    && !kassiber_pytext::strip(&crate::pyval::lossy(wallet)).is_empty() =>
            {
                (
                    "wallet",
                    kassiber_pytext::strip(&crate::pyval::lossy(wallet)).to_owned(),
                )
            }
            _ => (
                "generic",
                format!(
                    "{} {}",
                    if side == "input" { "Input" } else { "Output" },
                    position + 1
                ),
            ),
        }
    };
    let value = field(node, "valueSats");
    let known = matches!(value, Value::Int(_));
    // `isinstance(value, int)` holds for a bool too: its state reads known.
    let known_state = known || matches!(value, Value::Bool(_));
    let mut row = vec![
        ("side", s(side)),
        ("label", s(label)),
        ("label_kind", s(label_kind)),
        ("position", Value::from(position)),
        (
            "ownership",
            or_value(field(node, "ownership"), s("unknown")),
        ),
        (
            "value_sats",
            if known { value.clone() } else { Value::Null },
        ),
        (
            "value_state",
            or_value(
                field(node, "valueState"),
                s(if known_state { "known" } else { "missing" }),
            ),
        ),
    ];
    if overflow {
        row.push((
            "overflow_count",
            Value::from(count(field(node, "overflowCount"))),
        ));
    }
    dict(row)
}

fn transaction_flow_visual(
    spec: &Value,
    payload: &Value,
    fiat_currency: &str,
    context: &Value,
) -> Built {
    let mut rows = Vec::new();
    for (side, key) in [("input", "inputs"), ("output", "outputs")] {
        if let Some(Value::List(nodes)) = get(payload, key) {
            for (position, node) in nodes.iter().enumerate() {
                if matches!(node, Value::Dict(_)) {
                    rows.push(flow_row(node, side, position as i64));
                }
            }
        }
    }
    if let Some(fee @ Value::Dict(_)) = get(payload, "fee") {
        rows.push(flow_row(fee, "fee", 0));
    }
    let mut caveats = Vec::new();
    let support = text_of(field(payload, "supportLevel"));
    if rows.is_empty() || ["graphless", "unsupported"].contains(&support.as_str()) {
        caveats.push(caveat("graph_unavailable", None));
    }
    let hidden = rows
        .iter()
        .filter(|row| field(row, "value_state") != &s("known"))
        .count();
    if hidden != 0 {
        caveats.push(caveat("amounts_hidden", Some(hidden as i64)));
    }
    if kassiber_envelope::truthy(field(field(payload, "accounting"), "quarantine")) {
        caveats.push(caveat("transaction_quarantined", None));
    }
    let transaction = match get(context, "transaction") {
        Some(resolved @ Value::Dict(_)) => transaction_row(resolved),
        _ => Value::Null,
    };
    let visual = header(spec, "flow", fiat_currency, vec![]);
    (visual, rows, caveats, vec![("transaction", transaction)])
}

fn check(
    name: &str,
    status: &str,
    count: Option<i64>,
    detail: Value,
    action: Option<&str>,
) -> Value {
    dict([
        ("check", s(name)),
        ("status", s(status)),
        ("count", count.map_or(Value::Null, Value::from)),
        ("detail", detail),
        ("action", action.map_or(Value::Null, s)),
        ("title", Value::Null),
    ])
}

fn readiness_visual(spec: &Value, payload: &Value, fiat_currency: &str) -> Built {
    let health = field(payload, "health");
    let journals = field(health, "journals");
    let queue = field(health, "review_queue");
    let rates = field(field(payload, "rates_coverage"), "summary");
    let custody = field(payload, "custody_quantity");
    let counts = field(health, "counts");
    let mut checks = Vec::new();
    let has_transactions = kassiber_envelope::truthy(field(counts, "active_transactions"))
        || kassiber_envelope::truthy(field(counts, "transactions"));
    checks.push(check(
        "transactions",
        if has_transactions { "ok" } else { "warning" },
        Some(count(field(counts, "active_transactions"))),
        Value::Null,
        (!has_transactions).then_some("import"),
    ));
    let stale = kassiber_envelope::truthy(field(journals, "needs_processing"));
    checks.push(check(
        "journals",
        if stale { "warning" } else { "ok" },
        None,
        field(journals, "last_processed_at").clone(),
        stale.then_some("process_journals"),
    ));
    let quarantine_count = field(journals, "quarantine_count");
    let quarantines = if kassiber_envelope::truthy(quarantine_count) {
        count(quarantine_count)
    } else {
        count(field(counts, "quarantines"))
    };
    checks.push(check(
        "quarantine",
        if quarantines != 0 { "blocked" } else { "ok" },
        Some(quarantines),
        Value::Null,
        (quarantines != 0).then_some("review_quarantine"),
    ));
    let missing = count(field(rates, "missing_price_transactions"));
    checks.push(check(
        "prices",
        if missing != 0 { "warning" } else { "ok" },
        Some(missing),
        if missing != 0 {
            s(count(field(rates, "cache_coverable_missing")).to_string())
        } else {
            Value::Null
        },
        (missing != 0).then_some("fill_prices"),
    ));
    let custody_issues = count(field(custody, "issue_count"));
    checks.push(check(
        "custody",
        if custody_issues != 0 { "blocked" } else { "ok" },
        Some(custody_issues),
        Value::Null,
        (custody_issues != 0).then_some("review_custody"),
    ));
    let review = count(field(queue, "review"));
    checks.push(check(
        "review_queue",
        if review != 0 { "warning" } else { "ok" },
        Some(review),
        Value::Null,
        (review != 0).then_some("open_review"),
    ));
    // These blockers already have their own check above.
    const COVERED: [&str; 5] = [
        "journals_stale",
        "journal_quarantine",
        "custody_quantity_unresolved",
        "missing_prices",
        "no_transactions",
    ];
    if let Some(Value::List(blockers)) = get(payload, "blockers") {
        for blocker in blockers {
            if !matches!(blocker, Value::Dict(_))
                || COVERED.iter().any(|id| field(blocker, "id") == &s(*id))
            {
                continue;
            }
            let detail = or_text(field(blocker, "detail"));
            let mut item = check(
                "blocker",
                if field(blocker, "severity") == &s("blocking") {
                    "blocked"
                } else {
                    "warning"
                },
                None,
                if detail.is_empty() {
                    Value::Null
                } else {
                    s(detail)
                },
                None,
            );
            let title = if kassiber_envelope::truthy(field(blocker, "title")) {
                text_of(field(blocker, "title"))
            } else {
                or_text(field(blocker, "id"))
            };
            kassiber_envelope::set(&mut item, "title", s(title));
            checks.push(item);
        }
    }
    let ready = kassiber_envelope::truthy(field(payload, "ready"));
    let passed = checks
        .iter()
        .filter(|item| field(item, "status") == &s("ok"))
        .count();
    let total = checks.len();
    let visual = header(spec, "checklist", fiat_currency, vec![]);
    (
        visual,
        checks,
        Vec::new(),
        vec![(
            "summary",
            dict([
                ("ready", Value::Bool(ready)),
                ("passed", Value::from(passed)),
                ("total", Value::from(total)),
            ]),
        )],
    )
}

/// `build_chat_visual(spec, payload, fiat_currency=..., readiness=...,
/// local_reference=..., context=..., now=...)`: project a source payload
/// into the visual envelope data. `now_utc_us` is the moment in microseconds
/// since 0001-01-01 UTC.
pub fn build_chat_visual(
    spec: &Value,
    payload: &Value,
    fiat_currency: &str,
    readiness: Option<&Value>,
    local_reference: Option<&Value>,
    context: &Value,
    now_utc_us: i64,
) -> Value {
    let currency = fiat_currency.to_uppercase();
    let currency = if currency.is_empty() {
        "EUR".to_owned()
    } else {
        currency
    };
    let now = Moment {
        local_us: now_utc_us,
        offset_us: 0,
    };
    let source = text_of(field(spec, "source"));
    let (mut visual, rows, mut caveats, extras) = match source.as_str() {
        "overview" => overview_visual(spec, payload, &currency),
        "balance_history" => balance_history_visual(spec, payload, &currency, now),
        "portfolio" => portfolio_visual(spec, payload, &currency),
        "tax_summary" => tax_summary_visual(spec, payload, &currency),
        "cash_flow" => cash_flow_visual(spec, payload, &currency),
        "transactions" => transactions_visual(spec, payload, &currency),
        "transaction_flow" => transaction_flow_visual(spec, payload, &currency, context),
        _ => readiness_visual(spec, payload, &currency),
    };
    if JOURNAL_SOURCES.contains(&source.as_str()) && source != "overview" {
        // The overview carries its own completeness block.
        let mut combined = readiness_caveats(readiness);
        combined.append(&mut caveats);
        caveats = combined;
    }
    visual.push((Key::from("as_of"), s(period_text(now))));
    let mut result = vec![
        (Key::from("visual"), Value::Dict(visual)),
        (Key::from("rows"), Value::List(rows)),
        (Key::from("caveats"), Value::List(caveats)),
    ];
    result.extend(
        extras
            .into_iter()
            .map(|(key, value)| (Key::from(key), value)),
    );
    if let Some(reference) = local_reference.filter(|value| kassiber_envelope::truthy(value)) {
        result.push((Key::from("local_reference"), reference.clone()));
    }
    let mut result = Value::Dict(result);
    let digest = digest(&source, &result);
    kassiber_envelope::set(&mut result, "digest", digest);
    result
}

/// One asset's series per measure, in the order measures first appear.
type MeasureSeries = Vec<(&'static str, Vec<f64>)>;

/// `_series_digest(values)`.
fn series_digest(values: &[f64]) -> Value {
    let (Some(first), Some(last)) = (values.first(), values.last()) else {
        return Value::Dict(Vec::new());
    };
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    dict([
        ("first", Value::Float(*first)),
        ("last", Value::Float(*last)),
        ("min", Value::Float(min)),
        ("max", Value::Float(max)),
        ("change", Value::Float(last - first)),
    ])
}

/// `_digest(source, result)`.
fn digest(source: &str, result: &Value) -> Value {
    let rows: Vec<Value> = match get(result, "rows") {
        Some(Value::List(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    let take = |key: &str| field(result, key).clone();
    match source {
        "overview" => dict([("stats", take("stats")), ("wallets", take("wallets"))]),
        "balance_history" => {
            let mut by_asset: Vec<(String, MeasureSeries)> = Vec::new();
            for row in &rows {
                let asset = text_of(field(row, "asset"));
                let position = match by_asset.iter().position(|(name, _)| *name == asset) {
                    Some(position) => position,
                    None => {
                        by_asset.push((asset, Vec::new()));
                        by_asset.len() - 1
                    }
                };
                for measure in ["quantity", "market_value", "cost_basis"] {
                    if let Some(value) = number(field(row, measure)) {
                        let series = &mut by_asset[position].1;
                        match series.iter_mut().find(|(name, _)| *name == measure) {
                            Some((_, values)) => values.push(value),
                            None => series.push((measure, vec![value])),
                        }
                    }
                }
            }
            let mut periods: Vec<String> = rows
                .iter()
                .map(|row| text_of(field(row, "period")))
                .collect();
            periods.sort();
            periods.dedup();
            dict([
                ("periods", Value::from(periods.len())),
                (
                    "first_period",
                    rows.first()
                        .map_or(Value::Null, |row| field(row, "period").clone()),
                ),
                (
                    "last_period",
                    rows.last()
                        .map_or(Value::Null, |row| field(row, "period").clone()),
                ),
                (
                    "assets",
                    Value::Dict(
                        by_asset
                            .into_iter()
                            .map(|(asset, measures)| {
                                (
                                    Key::from(asset.as_str()),
                                    Value::Dict(
                                        measures
                                            .into_iter()
                                            .map(|(measure, values)| {
                                                (Key::from(measure), series_digest(&values))
                                            })
                                            .collect(),
                                    ),
                                )
                            })
                            .collect(),
                    ),
                ),
            ])
        }
        "cash_flow" => dict([
            ("buckets", Value::from(rows.len())),
            ("totals", take("totals")),
        ]),
        "tax_summary" => dict([("years", Value::List(rows))]),
        "transactions" | "readiness" => dict([("summary", take("summary"))]),
        "transaction_flow" => dict([
            ("transaction", take("transaction")),
            (
                "inputs",
                Value::from(
                    rows.iter()
                        .filter(|row| field(row, "side") == &s("input"))
                        .count(),
                ),
            ),
            (
                "outputs",
                Value::from(
                    rows.iter()
                        .filter(|row| field(row, "side") == &s("output"))
                        .count(),
                ),
            ),
        ]),
        _ => Value::Dict(Vec::new()),
    }
}

/// `model_digest(data)`: what the provider needs to describe a visual; the
/// rows only while they are few.
pub fn model_digest(data: &Value) -> Value {
    let rows: Vec<Value> = match get(data, "rows") {
        Some(Value::List(rows)) => rows.clone(),
        _ => Vec::new(),
    };
    let caveats = or_value(field(data, "caveats"), Value::List(Vec::new()));
    let mut summary = match get(data, "digest") {
        Some(digest @ Value::Dict(_)) => digest.clone(),
        _ => Value::Dict(Vec::new()),
    };
    kassiber_envelope::set(&mut summary, "caveats", caveats.clone());
    kassiber_envelope::set(&mut summary, "shown_to_user", Value::Bool(true));
    let mut digest = dict([
        ("shown_to_user", Value::Bool(true)),
        ("visual", field(data, "visual").clone()),
        ("caveats", caveats),
        // The auto-read context keeps only `summary` when an entry is too
        // large, so the caveats and the shown-to-user note travel inside it.
        ("summary", summary),
        ("row_count", Value::from(rows.len())),
    ]);
    if rows.len() <= MODEL_ROW_LIMIT {
        kassiber_envelope::set(&mut digest, "rows", Value::List(rows));
    } else {
        kassiber_envelope::set(&mut digest, "rows_omitted", Value::from(rows.len()));
    }
    digest
}

/// `_tool_result_for_model(result)`: a visual's full rows are for the
/// screen; the model reads its digest.
pub fn tool_result_for_model(result: &Value) -> Value {
    let Some(envelope @ Value::Dict(_)) = get(result, "envelope") else {
        return result.clone();
    };
    if field(envelope, "kind") != &s("ui.visuals.render")
        || !matches!(get(envelope, "data"), Some(Value::Dict(_)))
    {
        return result.clone();
    }
    let mut envelope = envelope.clone();
    let digest = model_digest(field(&envelope, "data"));
    kassiber_envelope::set(&mut envelope, "data", digest);
    let mut result = result.clone();
    kassiber_envelope::set(&mut result, "envelope", envelope);
    result
}

/// `_chat_visual_transactions_payload(conn, spec)`: the list read, and for a
/// size order each direction read from the end that matches, merged.
fn transactions_payload(book: &mut dyn Book, spec: &Value) -> Result<Value, ToolError> {
    let filters = field(spec, "report_args").clone();
    let limit = field(spec, "limit").clone();
    let sort = text_of(field(spec, "sort"));
    let with = |extra: Vec<(&str, Value)>| {
        let mut args = filters.clone();
        for (key, value) in extra {
            kassiber_envelope::set(&mut args, key, value);
        }
        args
    };
    if sort == "recent" || sort == "oldest" {
        let args = with(vec![
            ("limit", limit),
            ("sort", s("occurred-at")),
            ("order", s(if sort == "recent" { "desc" } else { "asc" })),
        ]);
        return book.read("ui.transactions.list", &args, "gui");
    }
    // The amount sort is signed: receipts positive, payments negative. Per
    // direction the size order is one end of it.
    let largest = sort == "largest";
    let order = |direction: &str| match (direction, largest) {
        ("inbound", true) | ("outbound", false) => "desc",
        _ => "asc",
    };
    let directions: Vec<String> = match get(&filters, "direction") {
        Some(direction) if kassiber_envelope::truthy(direction) => vec![text_of(direction)],
        _ => vec!["inbound".into(), "outbound".into()],
    };
    let mut candidates: Vec<(String, Value)> = Vec::new();
    for direction in &directions {
        let page = book.read(
            "ui.transactions.list",
            &with(vec![
                ("direction", s(direction.as_str())),
                ("limit", limit.clone()),
                ("sort", s("amount")),
                ("order", s(order(direction))),
            ]),
            "gui",
        )?;
        if let Some(Value::List(txs)) = get(&page, "txs") {
            for tx in txs {
                // `{str(tx.get("id")): tx ...}`: a later copy replaces an
                // earlier one in its place.
                let id = kassiber_envelope::py_str(field(tx, "id"));
                match candidates.iter_mut().find(|(known, _)| *known == id) {
                    Some((_, slot)) => *slot = tx.clone(),
                    None => candidates.push((id, tx.clone())),
                }
            }
        }
    }
    let size = |tx: &Value| {
        let amount = field(tx, "amountSat");
        if kassiber_envelope::truthy(amount) {
            number(amount).unwrap_or(0.0).abs()
        } else {
            0.0
        }
    };
    let mut ordered: Vec<Value> = candidates.into_iter().map(|(_, tx)| tx).collect();
    ordered.sort_by(|a, b| {
        let (a, b) = if largest {
            (size(b), size(a))
        } else {
            (size(a), size(b))
        };
        a.partial_cmp(&b).unwrap_or(std::cmp::Ordering::Equal)
    });
    let keep = int(&limit).unwrap_or(0).max(0) as usize;
    ordered.truncate(keep);
    let total = book.read(
        "ui.transactions.list",
        &with(vec![("limit", Value::from(1))]),
        "gui",
    )?;
    Ok(dict([
        ("txs", Value::List(ordered)),
        ("total", field(&total, "total").clone()),
    ]))
}

/// `_chat_visual_payload(conn, runtime, raw_args)`: an assistant visual from
/// the same reads its pages and tools use. The model picks the source and
/// filters; every figure comes from the local payload, and a visual never
/// contacts a backend.
pub fn render(book: &mut dyn Book, arguments: &Value) -> Result<Value, ToolError> {
    let spec = normalize_visual_args(arguments)?;
    let source = text_of(field(&spec, "source"));
    let report_args = field(&spec, "report_args").clone();
    let empty = Value::Dict(Vec::new());
    let mut context = Value::Dict(Vec::new());
    let mut local_reference = None;
    let payload = match source.as_str() {
        "overview" => book.read("ui.overview.snapshot", &empty, "gui")?,
        "balance_history" => book.read("ui.reports.balance_history", &report_args, "gui")?,
        "portfolio" => book.read("ui.reports.portfolio_summary", &empty, "gui")?,
        "tax_summary" => book.read("ui.reports.tax_summary", &empty, "gui")?,
        "cash_flow" => {
            // The Transactions page's dashboard read, with its own pairing.
            let period = field(&report_args, "period").clone();
            let mut args = vec![("period", period.clone())];
            if period != s("all") {
                args.push(("until", s(crate::clock::now_iso())));
            }
            let wallet = field(&report_args, "wallet");
            if kassiber_envelope::truthy(wallet) {
                args.push(("wallet", wallet.clone()));
            }
            book.read("ui.transactions.dashboard", &dict(args), "gui")?
        }
        "transactions" => transactions_payload(book, &spec)?,
        "readiness" => book.report_blockers(None)?,
        _ => {
            let reference = field(&report_args, "transaction").clone();
            let payload = book.read(
                "ui.transactions.graph",
                &dict([("transaction", reference.clone())]),
                "gui",
            )?;
            let resolved = book.read(
                "ui.transactions.resolve",
                &dict([("query", reference)]),
                "gui",
            )?;
            let transaction = field(&resolved, "transaction").clone();
            let id = [
                field(field(&payload, "transaction"), "id"),
                field(&transaction, "id"),
            ]
            .into_iter()
            .find(|id| kassiber_envelope::truthy(id))
            .cloned();
            kassiber_envelope::set(&mut context, "transaction", transaction);
            if let Some(Value::Str(id)) = id {
                local_reference = Some(dict([
                    ("route", s("/transactions")),
                    ("transaction", Value::Str(id)),
                ]));
            }
            payload
        }
    };
    let fiat_currency = book.profile_fiat_currency()?;
    let readiness = if JOURNAL_SOURCES.contains(&source.as_str()) && source != "overview" {
        // The same blockers the readiness checklist reads.
        Some(readiness_from_blockers(&book.report_blockers(None)?))
    } else {
        None
    };
    Ok(build_chat_visual(
        &spec,
        &payload,
        &fiat_currency,
        readiness.as_ref(),
        local_reference.as_ref(),
        &context,
        crate::clock::now_utc_us(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calendars_step_like_python() {
        let at = |text: &str| parse_period(&s(text)).unwrap();
        assert_eq!(
            period_text(at("2025-03-15T10:30:00Z")),
            "2025-03-15T10:30:00Z"
        );
        assert_eq!(
            period_text(floor_to_interval(at("2025-03-15T10:30:00Z"), "week")),
            "2025-03-10T00:00:00Z"
        );
        // Floored in its own offset, written in UTC.
        assert_eq!(
            period_text(floor_to_interval(at("2025-03-01T01:00:00+02:00"), "month")),
            "2025-02-28T22:00:00Z"
        );
        assert_eq!(
            calendar_periods(
                at("2024-11-20T00:00:00Z"),
                at("2025-02-01T00:00:00Z"),
                "month"
            ),
            [
                "2024-11-01T00:00:00Z",
                "2024-12-01T00:00:00Z",
                "2025-01-01T00:00:00Z",
                "2025-02-01T00:00:00Z"
            ]
        );
        assert_eq!(civil(ordinal(2024, 2, 29)), (2024, 2, 29));
        assert_eq!(civil(0), (1, 1, 1));
    }
}
