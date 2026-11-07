//! `build_transactions_dashboard_snapshot`: flow totals, calendar buckets and
//! bounded transfer/swap candidates over stored rows. Candidates come from the
//! caller's suggestion service; they are presentation only and pair nothing.
use kassiber_pytext::zone::{civil_from_days, days_from_civil, Failure, Zone};

use super::*;

const MAX_CANDIDATES: usize = 256;
const AUTO_MIN_AMOUNT_MSAT: i64 = 1_000_000;
const AUTO_MIN_TRANSACTIONS: usize = 3;
const AUTO_MIN_BUCKETS: usize = 2;
const SEGMENTS: [&str; 4] = ["incoming", "outgoing", "transfers", "swaps"];
const DISPLAY_AMOUNT: &str = "CASE WHEN t.direction='outbound' AND COALESCE(t.amount,0)=0 AND COALESCE(t.fee,0)>0 THEN ABS(t.fee) ELSE ABS(COALESCE(t.amount,0)) END";
/// Microseconds from 0001-01-01 to 1970-01-01, the parser's and zone epochs.
const UNIX_US: i64 = 719_162 * 86_400_000_000;
const DAY: i64 = 86_400;

/// Zone lookups the caller binds: the display zone by IANA key and the book
/// calendar (Europe/Vienna for Austrian books, otherwise UTC).
pub struct Zones<'a> {
    pub display: &'a dyn Fn(&str) -> std::result::Result<Zone, Failure>,
    pub book: &'a dyn Fn(bool) -> Result<Zone>,
}

/// A wall-clock time in one zone: whole seconds since 1970 plus microseconds,
/// and the `fold` that `astimezone` and `replace` carry but `+ timedelta`
/// resets. Python compares datetimes sharing a tzinfo by wall time alone.
#[derive(Clone, Copy)]
struct Wall {
    seconds: i64,
    micros: i64,
    fold: bool,
}
impl Wall {
    fn key(&self) -> (i64, i64) {
        (self.seconds, self.micros)
    }
    /// `replace(...)` of the date and clock, keeping the fold.
    fn replace(&self, seconds: i64, micros: i64) -> Wall {
        Wall {
            seconds,
            micros,
            fold: self.fold,
        }
    }
}
impl PartialEq for Wall {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}
impl PartialOrd for Wall {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.key().cmp(&other.key()))
    }
}

fn parse(raw: &str, field: &str) -> Result<i64> {
    time::normalized(raw).ok_or_else(|| {
        invalid(format!("Invalid {field} timestamp '{raw}'"))
            .with_hint("Use RFC3339 UTC like 2025-01-01T00:00:00Z")
    })
}
fn wall(zone: &Zone, us: i64) -> Result<Wall> {
    let unix = us - UNIX_US;
    let seconds = unix.div_euclid(1_000_000);
    let offset = zone
        .utc_offset(seconds)
        .ok_or_else(|| AppError::new("internal_error", "zone has no offset"))?;
    Ok(Wall {
        seconds: seconds + offset,
        micros: unix.rem_euclid(1_000_000),
        fold: zone.utc_fold(seconds),
    })
}
fn civil(at: Wall) -> (i64, i64, i64) {
    civil_from_days(at.seconds.div_euclid(DAY))
}
fn midnight(at: Wall, year: i64, month: i64, day: i64) -> Wall {
    at.replace(days_from_civil(year, month, day) * DAY, 0)
}
/// "All" over a long book reads as a run of years: a decade of quarters is
/// already forty bars.
const YEARLY_BUCKET_MIN_YEARS: f64 = 10.0;

/// `_history_years`: years from `earliest` to `end_us`, never negative.
fn history_years(earliest: &str, end_us: i64) -> Result<f64> {
    let elapsed = end_us - parse(earliest, "earliest")?;
    let seconds = kassiber_pytext::parse_float(&format!("{elapsed}e-6")).unwrap();
    Ok((seconds / (365.2425_f64 * 24.0 * 60.0 * 60.0)).max(0.0))
}
/// `_dashboard_bucket_size`.
fn bucket_size(period: &str, history_years: f64) -> &'static str {
    match period {
        "30days" => "day",
        "3months" => "week",
        "all" if history_years >= YEARLY_BUCKET_MIN_YEARS => "year",
        "4years" | "5years" | "10years" | "15years" | "all" => "quarter",
        _ => "month",
    }
}
/// `_dashboard_bucket_key`.
fn bucket(at: Wall, size: &str) -> (String, Wall) {
    let (year, month, day) = civil(at);
    let days = at.seconds.div_euclid(DAY);
    let date = |days: i64| {
        let (y, m, d) = civil_from_days(days);
        format!("{y:04}-{m:02}-{d:02}")
    };
    match size {
        "day" => (date(days), midnight(at, year, month, day)),
        "week" => {
            // Subtracting a timedelta resets the fold before `replace`.
            let monday = days - (days + 3).rem_euclid(7);
            let (y, m, d) = civil_from_days(monday);
            let reset = Wall { fold: false, ..at };
            (date(monday), midnight(reset, y, m, d))
        }
        "year" => (year.to_string(), midnight(at, year, 1, 1)),
        "quarter" => {
            let first = (month - 1) / 3 * 3 + 1;
            (
                format!("{year}-Q{}", (first - 1) / 3 + 1),
                midnight(at, year, first, 1),
            )
        }
        _ => (format!("{year}-{month:02}"), midnight(at, year, month, 1)),
    }
}
/// `_dashboard_next_bucket`: wall-clock arithmetic, as aware datetimes add.
fn next(at: Wall, size: &str) -> Wall {
    let step = match size {
        "day" => 1,
        "week" => 7,
        "year" => {
            let (year, _, _) = civil(at);
            let clock = at.seconds.rem_euclid(DAY);
            return at.replace(days_from_civil(year + 1, 1, 1) * DAY + clock, at.micros);
        }
        _ => {
            let (year, month, _) = civil(at);
            let index = year * 12 + month - 1 + if size == "quarter" { 3 } else { 1 };
            let clock = at.seconds.rem_euclid(DAY);
            return at.replace(
                days_from_civil(index.div_euclid(12), index.rem_euclid(12) + 1, 1) * DAY + clock,
                at.micros,
            );
        }
    };
    Wall {
        seconds: at.seconds + step * DAY,
        micros: at.micros,
        fold: false,
    }
}
/// `datetime.isoformat()` of a zone-aware wall time (fold 0).
fn isoformat(zone: &Zone, at: Wall) -> String {
    let (year, month, day) = civil(at);
    let clock = at.seconds.rem_euclid(DAY);
    let mut text = format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    );
    if at.micros != 0 {
        text.push_str(&format!(".{:06}", at.micros));
    }
    if let Some(offset) = zone.local_offset_fold(at.seconds, at.fold) {
        let sign = if offset < 0 { '-' } else { '+' };
        let offset = offset.abs();
        text.push_str(&format!(
            "{sign}{:02}:{:02}",
            offset / 3600,
            offset / 60 % 60
        ));
        if offset % 60 != 0 {
            text.push_str(&format!(":{:02}", offset % 60));
        }
    }
    text
}
/// `_UI_TRANSACTION_PERIOD_BY_DAYS`: every accepted spelling of a period
/// sizes buckets like its canonical name.
fn canonical_period(period: &str) -> &str {
    match period {
        "30day" | "30d" => "30days",
        "3month" | "3m" => "3months",
        "6month" | "6m" => "6months",
        "1years" | "1y" => "1year",
        "4year" | "4y" => "4years",
        "5year" | "5y" => "5years",
        "10year" | "10y" => "10years",
        "15year" | "15y" => "15years",
        other => other,
    }
}
fn empty_stats() -> Value {
    dict([
        ("count", Value::Int(0.into())),
        ("btc", Value::Float(0.0)),
        ("eur", Value::Float(0.0)),
        ("missingPrice", Value::Int(0.into())),
        ("review", Value::Int(0.into())),
        ("failed", Value::Int(0.into())),
    ])
}
fn segments() -> Value {
    dict(SEGMENTS.map(|segment| (segment, empty_stats())))
}
fn float(value: &Value) -> f64 {
    match value {
        Value::Int(n) => kassiber_pytext::parse_float(&n.to_string()).unwrap_or_default(),
        Value::Float(n) => *n,
        _ => 0.0,
    }
}
/// `int(x)` of a rendered sat amount: floats truncate toward zero.
fn integer(value: &Value) -> i128 {
    match value {
        Value::Int(n) => i128::try_from(n.clone()).unwrap_or_default(),
        Value::Float(n) => n.trunc() as i128,
        _ => 0,
    }
}
fn bump(stats: &mut Value, key: &str, by: Value) {
    let total = match (val(stats, key), by) {
        (Value::Int(a), Value::Int(b)) => Value::Int(a + b),
        (Value::Float(a), Value::Float(b)) => Value::Float(a + b),
        (a, _) => a,
    };
    set(stats, key, total);
}
/// `_dashboard_status`: the row's queue status, which presentation always sets.
fn status(tx: &Value) -> String {
    py_str(&val(tx, "queueStatus"))
}
/// `_dashboard_add_stats`.
fn add(stats: &mut Value, tx: &Value) {
    let btc = float(&val(tx, "amountSat")).abs() / 100_000_000.0;
    let rate = val(tx, "rate");
    let eur = match val(tx, "eur") {
        Value::Null if rate == Value::Null => 0.0,
        Value::Null => btc * float(&rate),
        raw => float(&raw).abs(),
    };
    let status = status(tx);
    bump(stats, "count", Value::Int(1.into()));
    bump(stats, "btc", Value::Float(btc));
    bump(stats, "eur", Value::Float(eur));
    if !truthy(&rate) {
        bump(stats, "missingPrice", Value::Int(1.into()));
    }
    if status == "review" || status == "pending" {
        bump(stats, "review", Value::Int(1.into()));
    }
    if status == "failed" {
        bump(stats, "failed", Value::Int(1.into()));
    }
    let larger = match get(stats, "largest") {
        None => true,
        Some(largest) => btc > float(&val(largest, "btc")),
    };
    if larger {
        let label = ["counter", "account", "id"]
            .into_iter()
            .map(|key| val(tx, key))
            .find(truthy)
            .unwrap_or_else(|| val(tx, "id"));
        set(
            stats,
            "largest",
            dict([
                ("label", label),
                ("btc", Value::Float(btc)),
                ("eur", Value::Float(eur)),
            ]),
        );
    }
}
fn stats_of<'a>(container: &'a mut Value, segment: &str) -> &'a mut Value {
    let Value::Dict(pairs) = container else {
        unreachable!()
    };
    &mut pairs
        .iter_mut()
        .find(|(k, _)| *k == Key::from(segment))
        .expect("segment")
        .1
}
/// `_dashboard_flow`.
fn flow(tx: &Value) -> &'static str {
    let tag = lower(&py_str(&val(tx, "tag")));
    let kind = text(tx, "type");
    if tag.contains("swap") || ["Swap", "Mint", "Melt"].contains(&kind) {
        return "swaps";
    }
    if truthy(&val(tx, "internal")) || ["Transfer", "Consolidation", "Rebalance"].contains(&kind) {
        return "transfers";
    }
    if integer(&val(tx, "amountSat")) >= 0 {
        "incoming"
    } else {
        "outgoing"
    }
}
fn reference(candidate: &Value, key: &str) -> String {
    let value = val(candidate, key);
    if truthy(&value) {
        py_str(&value)
    } else {
        String::new()
    }
}
/// `_dashboard_candidate_type`.
fn candidate_type(candidate: &Value) -> &'static str {
    match crate::sql::string(&val(candidate, "candidate_type")) {
        Some("transfer") => return "transfer",
        Some("swap") => return "swap",
        _ => {}
    }
    let layers = [
        "chain-swap",
        "peg-in",
        "peg-out",
        "reverse-submarine-swap",
        "submarine-swap",
        "swap-refund",
    ];
    if crate::sql::string(&val(candidate, "default_kind")).is_some_and(|k| layers.contains(&k)) {
        return "transfer";
    }
    let input = upper(&reference(candidate, "in_asset"));
    let output = upper(&reference(candidate, "out_asset"));
    if !input.is_empty() && !output.is_empty() && input == output {
        "transfer"
    } else {
        "swap"
    }
}
/// `_dashboard_non_conflicted_candidates`.
fn non_conflicted(candidates: &[Value]) -> Vec<&Value> {
    let cluster = |index: usize, candidate: &Value| {
        let id = reference(candidate, "conflict_set_id");
        if id.is_empty() {
            format!("solo:{index}")
        } else {
            id
        }
    };
    let mut sizes: BTreeMap<String, i64> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        *sizes.entry(cluster(index, candidate)).or_default() += 1;
    }
    let mut used = BTreeSet::new();
    let mut output = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        let size = match val(candidate, "conflict_size") {
            Value::Int(n) => n,
            Value::Bool(b) => i64::from(b).into(),
            _ => sizes[&cluster(index, candidate)].into(),
        };
        let input = reference(candidate, "in_id");
        let output_id = reference(candidate, "out_id");
        if size > 1.into() || input.is_empty() || output_id.is_empty() {
            continue;
        }
        if used.contains(&input) || used.contains(&output_id) {
            continue;
        }
        used.insert(input);
        used.insert(output_id);
        output.push(candidate);
    }
    output
}

struct Auto<'a> {
    conn: &'a Connection,
    filters: &'a str,
    bind: &'a [SqlValue],
    earliest: Option<&'a str>,
    zone: &'a Zone,
    end: Wall,
    end_us: i64,
}
/// `_dashboard_auto_period`.
fn auto_period(auto: Auto<'_>) -> Result<String> {
    let Some(earliest) = auto.earliest else {
        return Ok("1year".into());
    };
    let mut bind = auto.bind.to_vec();
    bind.push(SqlValue::Integer(AUTO_MIN_AMOUNT_MSAT));
    let found = rows(
        auto.conn,
        &format!(
            "SELECT t.occurred_at FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {} AND {DISPLAY_AMOUNT}>=? ORDER BY t.occurred_at",
            auto.filters
        ),
        params_from_iter(bind),
    )?;
    if found.is_empty() {
        return Ok("1year".into());
    }
    let dates = found
        .iter()
        .map(|row| {
            wall(
                auto.zone,
                parse(&py_str(&val(row, "occurred_at")), "occurred_at")?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    let history_years = history_years(earliest, auto.end_us)?;
    // Auto settles on a range the page offers: twelve months, a halving
    // cycle, a decade, or everything.
    let mut candidates = vec!["1year"];
    for (period, years) in [("4years", 4.0), ("10years", 10.0)] {
        if history_years >= years * 0.8 {
            candidates.push(period);
        }
    }
    candidates.push("all");
    let target = AUTO_MIN_TRANSACTIONS.min(dates.len());
    let end = auto.end;
    let (year, month, day) = civil(end);
    let clock = end.seconds.rem_euclid(DAY);
    let start = |period: &str| -> Option<Wall> {
        match period {
            "all" => None,
            _ => {
                let years: i64 = period
                    .trim_end_matches("years")
                    .trim_end_matches("year")
                    .parse()
                    .unwrap();
                let day = if month == 2 && day == 29 { 28 } else { day };
                Some(end.replace(
                    days_from_civil(year - years, month, day) * DAY + clock,
                    end.micros,
                ))
            }
        }
    };
    let mut qualifying = Vec::new();
    for period in candidates {
        let start = start(period);
        let visible: Vec<_> = dates
            .iter()
            .filter(|date| start.is_none_or(|start| **date >= start) && **date <= end)
            .collect();
        if visible.len() < target {
            continue;
        }
        qualifying.push(period);
        let size = bucket_size(canonical_period(period), history_years);
        let buckets: BTreeSet<_> = visible.iter().map(|date| bucket(**date, size).0).collect();
        if buckets.len() >= AUTO_MIN_BUCKETS {
            return Ok(period.into());
        }
    }
    Ok(qualifying.first().copied().unwrap_or("all").into())
}

/// `screen_metrics_scope`.
fn metrics_scope(
    conn: &Connection,
    profile_id: &str,
    wallet: &str,
    grouped: bool,
) -> Result<Value> {
    let mut wallet_id = Value::Null;
    if !wallet.is_empty() {
        let matches = rows(
            conn,
            "SELECT id FROM wallets WHERE profile_id=? AND (id=? OR lower(label)=lower(?)) ORDER BY id",
            [profile_id, wallet, wallet],
        )?;
        if matches.len() != 1 {
            return Ok(Value::Null);
        }
        wallet_id = s(py_str(&val(&matches[0], "id")));
    }
    let profile = rows(
        conn,
        "SELECT workspace_id,journal_input_version FROM profiles WHERE id=?",
        [profile_id],
    )?;
    let Some(profile) = profile.first() else {
        return Ok(Value::Null);
    };
    let version = match val(profile, "journal_input_version") {
        Value::Int(n) => Value::Int(n),
        _ => Value::Int(0.into()),
    };
    Ok(dict([
        ("workspaceId", s(py_str(&val(profile, "workspace_id")))),
        ("profileId", s(profile_id)),
        ("inputVersion", version),
        ("wallet", wallet_id),
        ("pairsGrouped", Value::Bool(grouped)),
    ]))
}

/// `_accounting_calendar`: years newest first, in the book's own calendar.
fn calendar(
    conn: &Connection,
    profile_id: &str,
    zones: &Zones<'_>,
) -> Result<Vec<(&'static str, Value)>> {
    let profile = rows(
        conn,
        "SELECT tax_country FROM profiles WHERE id=?",
        [profile_id],
    )?;
    let Some(profile) = profile.first() else {
        return Ok(vec![
            ("accountingYears", Value::List(vec![])),
            ("accountingTimezone", s("UTC")),
        ]);
    };
    let country = val(profile, "tax_country");
    let austrian = truthy(&country) && lower(&py_str(&country)) == "at";
    let zone = (zones.book)(austrian)?;
    let bounds = rows(
        conn,
        "SELECT MIN(occurred_at) AS first,MAX(occurred_at) AS last FROM transactions WHERE profile_id=? AND occurred_at NOT IN ('',?)",
        [profile_id, "1970-01-01T00:00:00Z"],
    )?;
    let mut years = Vec::new();
    let (first, last) = (val(&bounds[0], "first"), val(&bounds[0], "last"));
    if truthy(&first) && truthy(&last) {
        let year = |raw: &Value| -> Result<i64> {
            let raw = py_str(raw);
            let us = time::normalized(&raw).ok_or_else(|| {
                AppError::new(
                    "internal_error",
                    format!("Invalid isoformat string: '{raw}'"),
                )
            })?;
            Ok(civil(wall(&zone, us)?).0)
        };
        let (first, last) = (year(&first)?, year(&last)?);
        years.extend((first..=last).rev().map(|y| Value::Int(y.into())));
    }
    Ok(vec![
        ("accountingYears", Value::List(years)),
        ("accountingTimezone", s(zone.key.as_str())),
    ])
}

/// The current UTC instant on the parser's microsecond scale.
fn now_us(conn: &Connection) -> Result<i64> {
    let now = rows(
        conn,
        "SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now') AS now",
        [],
    )?;
    parse(text(&now[0], "now"), "now")
}

/// `_iso_z` of a UTC instant in whole seconds since 1970.
fn utc_text(seconds: i64) -> String {
    let (year, month, day) = civil_from_days(seconds.div_euclid(DAY));
    let clock = seconds.rem_euclid(DAY);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        clock / 3600,
        clock / 60 % 60,
        clock % 60
    )
}

/// `_dashboard_years`: the calendar years holding dated activity, newest
/// first, counted in the display zone the buckets use. Undated rows
/// (stored at 1970-01-01) are not a year of their own.
fn years(conn: &Connection, base: &str, bind: &[SqlValue], zone: &Zone) -> Result<Vec<Value>> {
    let dated = format!("{base} AND t.occurred_at NOT IN ('',?)");
    let mut dated_bind = bind.to_vec();
    dated_bind.push(SqlValue::Text("1970-01-01T00:00:00Z".into()));
    let bounds = rows(
        conn,
        &format!("SELECT MIN(t.occurred_at) AS first,MAX(t.occurred_at) AS last FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {dated}"),
        params_from_iter(&dated_bind),
    )?;
    let (first, last) = (val(&bounds[0], "first"), val(&bounds[0], "last"));
    if !truthy(&first) || !truthy(&last) {
        return Ok(vec![]);
    }
    let first = civil(wall(zone, parse(&py_str(&first), "first")?)?).0;
    let last = civil(wall(zone, parse(&py_str(&last), "last")?)?).0;
    // `datetime(year, 1, 1, tzinfo=zone)` (fold 0) as a UTC instant.
    let new_year = |year: i64| -> Result<i64> {
        let local = days_from_civil(year, 1, 1) * DAY;
        let offset = zone
            .local_offset(local)
            .ok_or_else(|| AppError::new("internal_error", "zone has no offset"))?;
        Ok(local - offset)
    };
    let mut output = Vec::new();
    for year in (first..=last).rev() {
        let (start, end) = (new_year(year)?, new_year(year + 1)?);
        let mut count_bind = dated_bind.clone();
        count_bind.push(SqlValue::Text(utc_text(start)));
        count_bind.push(SqlValue::Text(utc_text(end)));
        let count = rows(
            conn,
            &format!("SELECT COUNT(*) AS count FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {dated} AND t.occurred_at >= ? AND t.occurred_at < ?"),
            params_from_iter(count_bind),
        )?;
        let count = num(&count[0], "count");
        if count != 0 {
            output.push(dict([
                ("year", Value::Int(year.into())),
                ("count", Value::Int(count.into())),
                ("since", s(utc_text(start))),
                // Inclusive, whole-second: bounds compare as text, and a
                // fractional until would sort before an exact timestamp.
                ("until", s(utc_text(end - 1))),
            ]));
        }
    }
    Ok(output)
}

fn empty(resolve_auto: bool) -> Value {
    let mut history = dict([
        ("count", Value::Int(0.into())),
        ("earliest", Value::Null),
        ("latest", Value::Null),
        ("accountingYears", Value::List(vec![])),
        ("accountingTimezone", s("UTC")),
        ("years", Value::List(vec![])),
        ("walletOptions", Value::List(vec![])),
    ]);
    if resolve_auto {
        set(&mut history, "autoPeriod", s("1year"));
    }
    let mut summary = segments();
    for key in [
        "review",
        "pending",
        "failed",
        "markedSwapCount",
        "swapCandidateCount",
    ] {
        set(&mut summary, key, Value::Int(0.into()));
    }
    dict([
        ("period", s("all")),
        (
            "window",
            dict([("since", Value::Null), ("until", Value::Null)]),
        ),
        ("history", history),
        (
            "counts",
            dict([
                ("all", Value::Int(0.into())),
                ("external", Value::Int(0.into())),
            ]),
        ),
        ("summary", summary),
        ("bucketSize", s("month")),
        ("buckets", Value::List(vec![])),
        ("candidates", Value::List(vec![])),
        ("swapCandidateTotal", Value::Int(0.into())),
    ])
}

fn bool_arg(args: &Value, key: &str) -> Result<bool> {
    let Some(raw) = get(args, key) else {
        return Ok(false);
    };
    match raw {
        Value::Bool(b) => return Ok(*b),
        Value::Int(n) if *n == 0.into() || *n == 1.into() => return Ok(*n == 1.into()),
        Value::Float(n) if *n == 0.0 || *n == 1.0 => return Ok(*n == 1.0),
        Value::Str(_) => match lower(strip(crate::sql::string(raw).unwrap_or_default())).as_str() {
            "1" | "true" | "yes" | "on" | "with-fees" => return Ok(true),
            "0" | "false" | "no" | "off" | "all" | "" => return Ok(false),
            _ => {}
        },
        _ => {}
    }
    Err(invalid(format!("{key} must be a boolean")).with_details(dict([(key, raw.clone())])))
}

pub(super) fn execute(
    conn: &Connection,
    scope: &Scope<'_>,
    args: &Value,
    candidates: &[Value],
    zones: &Zones<'_>,
) -> Result<Value> {
    let kind = "ui.transactions.dashboard";
    if !matches!(args, Value::Dict(_)) {
        return Err(invalid("ui snapshot args must be an object")
            .with_details(dict([("type", s(type_name(args)))])));
    }
    unknown(
        args,
        kind,
        &[
            "period",
            "since",
            "until",
            "wallet",
            "timezone",
            "resolveAuto",
        ],
    )?;
    let resolve_auto = bool_arg(args, "resolveAuto")?;
    if scope.profile_id.is_empty() {
        return Ok(empty(resolve_auto));
    }
    let profile_id = scope.profile_id;
    let period_raw = optional(args, "period")?.unwrap_or_else(|| "1year".into());
    let period = {
        let t = lower(&period_raw).replace(['_', '-', ' '], "");
        match t.as_str() {
            "ytd" => t,
            "all" | "max" => "all".into(),
            "30days" | "30day" | "30d" | "3months" | "3month" | "3m" | "6months" | "6month"
            | "6m" | "1year" | "1years" | "1y" | "4years" | "4year" | "4y" | "5years" | "5year"
            | "5y" | "10years"
            | "10year" | "10y" | "15years" | "15year" | "15y" => t,
            _ => {
                return Err(invalid("period must be one of: 30days, 3months, 6months, ytd, 1year, 4years, 5years, 10years, 15years, all").with_details(dict([("period", s(period_raw))])))
            }
        }
    };
    let since_raw = optional(args, "since")?;
    let until_raw = optional(args, "until")?;
    if period != "all" && (since_raw.is_none() || until_raw.is_none()) {
        return Err(
            invalid(format!("{kind} requires exact since and until bounds"))
                .with_details(dict([("period", s(period.as_str()))])),
        );
    }
    let since = time::timestamp(since_raw.as_deref(), "since")?;
    let until = time::timestamp(until_raw.as_deref(), "until")?;
    if !since.is_empty() && !until.is_empty() && parse(&since, "since")? > parse(&until, "until")? {
        return Err(invalid("since must not be after until"));
    }
    let timezone = optional(args, "timezone")?.unwrap_or_else(|| "UTC".into());
    let zone = match (zones.display)(&timezone) {
        Ok(zone) => zone,
        Err(Failure::NotFound) => {
            return Err(invalid("timezone must be an IANA timezone name")
                .with_details(dict([("timezone", s(timezone))])))
        }
        Err(Failure::Invalid(message)) => return Err(AppError::new("internal_error", message)),
    };
    let wallet = optional(args, "wallet")?.unwrap_or_default();
    let mut base = vec!["t.profile_id=?".to_owned()];
    let mut base_bind = vec![SqlValue::Text(profile_id.into())];
    if !wallet.is_empty() {
        base.push(wallet_sql(&wallet));
        wallet_args(&mut base_bind, &wallet);
    }
    let base = base.join(" AND ");
    let history = rows(
        conn,
        &format!("SELECT COUNT(*) AS count,MIN(t.occurred_at) AS earliest,MAX(t.occurred_at) AS latest FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {base}"),
        params_from_iter(&base_bind),
    )?
    .remove(0);
    let wallet_rows = rows(
        conn,
        &format!("SELECT DISTINCT w.label FROM transactions t JOIN wallets w ON w.id=t.wallet_id WHERE {base} ORDER BY lower(w.label),w.label"),
        params_from_iter(&base_bind),
    )?;
    let earliest = val(&history, "earliest");
    let latest = val(&history, "latest");
    let earliest_text = truthy(&earliest).then(|| py_str(&earliest));
    let auto = if resolve_auto {
        let end_us = if until.is_empty() {
            now_us(conn)?
        } else {
            parse(&until, "until")?
        };
        Some(auto_period(Auto {
            conn,
            filters: &base,
            bind: &base_bind,
            earliest: earliest_text.as_deref(),
            zone: &zone,
            end: wall(&zone, end_us)?,
            end_us,
        })?)
    } else {
        None
    };
    let mut conditions = vec![base.clone()];
    let mut bind = base_bind.clone();
    for (op, bound) in [(">=", &since), ("<=", &until)] {
        if !bound.is_empty() {
            conditions.push(time::predicate(conn, "t.occurred_at", op)?);
            bind.push(SqlValue::Text(bound.clone()));
        }
    }
    let raw = rows(
        conn,
        &format!(
            "SELECT {COLUMNS},{} AS payment_method {FROM} WHERE {} ORDER BY t.occurred_at ASC,t.created_at ASC,t.id ASC",
            sql::PAYMENT,
            conditions.join(" AND ")
        ),
        params_from_iter(bind),
    )?;
    let transactions = activity(conn, scope, &raw, &wallet, &since, &until)?;

    let all_candidates = non_conflicted(candidates);
    let by_id: BTreeMap<String, &Value> = transactions
        .iter()
        .map(|tx| (py_str(&val(tx, "id")), tx))
        .collect();
    let refs: Vec<&Value> = all_candidates
        .iter()
        .copied()
        .filter(|c| {
            by_id.contains_key(&reference(c, "in_id"))
                && by_id.contains_key(&reference(c, "out_id"))
        })
        .take(MAX_CANDIDATES)
        .collect();
    let mut candidate_flow: BTreeMap<String, &str> = BTreeMap::new();
    let mut candidate_pairs: [(&str, Vec<&Value>); 2] = [("transfers", vec![]), ("swaps", vec![])];
    for candidate in &refs {
        let flow = if candidate_type(candidate) == "transfer" {
            "transfers"
        } else {
            "swaps"
        };
        candidate_flow.insert(reference(candidate, "in_id"), flow);
        candidate_flow.insert(reference(candidate, "out_id"), flow);
        candidate_pairs[usize::from(flow == "swaps")]
            .1
            .push(candidate);
    }
    let mut summary = segments();
    for key in ["review", "pending", "failed"] {
        set(&mut summary, key, Value::Int(0.into()));
    }
    // Python sizes by the canonical period and measures "all" to `until`
    // when the request gives one.
    let size = match &earliest_text {
        Some(earliest) => {
            let end_us = if until.is_empty() {
                now_us(conn)?
            } else {
                parse(&until, "until")?
            };
            bucket_size(canonical_period(&period), history_years(earliest, end_us)?)
        }
        None => bucket_size(canonical_period(&period), 0.0),
    };
    let mut buckets: Vec<(String, Value)> = Vec::new();
    let new_bucket = |key: &str, start: Wall| {
        dict([
            ("bucketKey", s(key)),
            ("bucketStart", s(isoformat(&zone, start))),
            ("stats", segments()),
        ])
    };
    if !transactions.is_empty() {
        let start = wall(
            &zone,
            if since.is_empty() {
                parse(&py_str(&earliest), "earliest")?
            } else {
                parse(&since, "since")?
            },
        )?;
        let end = wall(
            &zone,
            if until.is_empty() {
                parse(&py_str(&latest), "latest")?
            } else {
                parse(&until, "until")?
            },
        )?;
        let mut cursor = bucket(start, size).1;
        while cursor <= end {
            let (key, start) = bucket(cursor, size);
            let entry = new_bucket(&key, start);
            match buckets.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = entry,
                None => buckets.push((key, entry)),
            }
            cursor = next(cursor, size);
        }
    }
    let mut external = 0;
    for tx in &transactions {
        let id = py_str(&val(tx, "id"));
        let segment = candidate_flow.get(&id).copied().unwrap_or_else(|| flow(tx));
        if !candidate_flow.contains_key(&id) {
            add(stats_of(&mut summary, segment), tx);
            if segment == "incoming" || segment == "outgoing" {
                external += 1;
            }
        }
        let occurred = val(tx, "occurredAt");
        let raw_date = if truthy(&occurred) {
            py_str(&occurred)
        } else {
            py_str(&val(tx, "date"))
        };
        let date = wall(&zone, parse(&raw_date, "occurredAt")?)?;
        let (key, start) = bucket(date, size);
        let index = match buckets.iter().position(|(k, _)| *k == key) {
            Some(index) => index,
            None => {
                buckets.push((key.clone(), new_bucket(&key, start)));
                buckets.len() - 1
            }
        };
        let Value::Dict(fields) = &mut buckets[index].1 else {
            unreachable!()
        };
        let stats = &mut fields
            .iter_mut()
            .find(|(k, _)| *k == Key::from("stats"))
            .unwrap()
            .1;
        add(stats_of(stats, segment), tx);
    }
    let current = current(conn, scope)?;
    let queue_sql = queue_items(conn, &wallet, &since, &until, current)?;
    let counts = rows(
        conn,
        &format!("SELECT queue_leg.item_rank AS rank,COUNT(DISTINCT queue_leg.item_key) AS count FROM ({queue_sql}) queue_leg GROUP BY queue_leg.item_rank"),
        params_from_iter(queue_args(scope, &wallet, &since, &until)),
    )?;
    for row in &counts {
        let status = match num(row, "rank") {
            3 => "review",
            2 => "failed",
            1 => "pending",
            _ => continue,
        };
        set(&mut summary, status, val(row, "count"));
    }
    let swaps = val(&summary, "swaps");
    set(&mut summary, "markedSwapCount", val(&swaps, "count"));
    set(
        &mut summary,
        "swapCandidateCount",
        Value::Int(candidate_pairs[1].1.len().into()),
    );
    for (segment, pairs) in &candidate_pairs {
        for candidate in pairs {
            let input = by_id[&reference(candidate, "in_id")];
            let output = by_id[&reference(candidate, "out_id")];
            let mut pair = input.clone();
            let amount = integer(&val(input, "amountSat"))
                .abs()
                .min(integer(&val(output, "amountSat")).abs());
            set(&mut pair, "amountSat", Value::Int(amount.into()));
            let eur = match (val(input, "eur"), val(output, "eur")) {
                (Value::Null, _) | (_, Value::Null) => Value::Null,
                (a, b) => {
                    let (a, b) = (float(&a).abs(), float(&b).abs());
                    Value::Float(if b < a { b } else { a })
                }
            };
            set(&mut pair, "eur", eur);
            add(stats_of(&mut summary, segment), &pair);
        }
    }
    let methods: BTreeSet<String> = transactions
        .iter()
        .map(|tx| {
            let method = val(tx, "paymentMethod");
            if truthy(&method) {
                py_str(&method)
            } else {
                "On-chain".into()
            }
        })
        .collect();
    let fields = [
        "in_id",
        "out_id",
        "in_asset",
        "out_asset",
        "default_kind",
        "candidate_type",
        "conflict_set_id",
        "conflict_size",
    ];
    let compact = refs
        .iter()
        .map(|candidate| match candidate {
            Value::Dict(pairs) => Value::Dict(
                pairs
                    .iter()
                    .filter(|(k, _)| fields.iter().any(|f| *k == Key::from(*f)))
                    .cloned()
                    .collect(),
            ),
            _ => dict::<&str>([]),
        })
        .collect();
    let mut history_payload = dict([
        ("count", Value::Int(num(&history, "count").into())),
        ("earliest", earliest),
        ("latest", latest),
    ]);
    for (key, value) in calendar(conn, profile_id, zones)? {
        set(&mut history_payload, key, value);
    }
    set(
        &mut history_payload,
        "years",
        Value::List(years(conn, &base, &base_bind, &zone)?),
    );
    set(
        &mut history_payload,
        "walletOptions",
        Value::List(
            wallet_rows
                .iter()
                .map(|row| s(py_str(&val(row, "label"))))
                .collect(),
        ),
    );
    set(
        &mut history_payload,
        "paymentMethods",
        Value::List(methods.into_iter().map(s).collect()),
    );
    if let Some(auto) = auto {
        set(&mut history_payload, "autoPeriod", s(auto));
    }
    let swap_total = all_candidates
        .iter()
        .filter(|c| candidate_type(c) == "swap")
        .count();
    Ok(dict([
        ("period", s(period.as_str())),
        (
            "window",
            dict([
                ("since", nullable(&since)),
                ("until", nullable(&until)),
                ("timezone", s(timezone)),
            ]),
        ),
        (
            "metricsScope",
            metrics_scope(conn, profile_id, &wallet, current)?,
        ),
        ("history", history_payload),
        (
            "counts",
            dict([
                ("all", Value::Int(transactions.len().into())),
                ("external", Value::Int(external.into())),
            ]),
        ),
        ("summary", summary),
        ("bucketSize", s(size)),
        (
            "buckets",
            Value::List(buckets.into_iter().map(|(_, b)| b).collect()),
        ),
        ("candidates", Value::List(compact)),
        ("swapCandidateTotal", Value::Int(swap_total.into())),
    ]))
}

/// `_activity_transaction_rows_to_ui`: one untagged row per reviewed pair,
/// rendered from the pair's last leg, with the filtered queue status.
fn activity(
    conn: &Connection,
    scope: &Scope<'_>,
    raw: &[Value],
    wallet: &str,
    since: &str,
    until: &str,
) -> Result<Vec<Value>> {
    let current = current(conn, scope)?;
    let mut pairs = BTreeMap::new();
    for chunk in raw.chunks(400) {
        pairs.extend(render::pairs(conn, chunk, current)?);
    }
    let mut last: BTreeMap<&str, &Value> = BTreeMap::new();
    for row in raw {
        if let Some(pair) = pairs.get(text(row, "id")) {
            last.insert(text(pair, "pair_id"), row);
        }
    }
    let mut rendered = BTreeSet::new();
    let mut output = Vec::new();
    let mut paired = Vec::new();
    for row in raw {
        let pair = pairs.get(text(row, "id"));
        let mut source = row;
        if let Some(pair) = pair {
            let id = text(pair, "pair_id");
            if !rendered.insert(id) {
                continue;
            }
            source = last.get(id).copied().unwrap_or(row);
            if text(pair, "kind") != "journal-derived" {
                paired.push(output.len());
            }
        }
        output.push(render::untagged(source, pair));
    }
    if !paired.is_empty() {
        let ranks = rows(
            conn,
            &queue_items(conn, wallet, since, until, current)?,
            params_from_iter(queue_args(scope, wallet, since, until)),
        )?;
        let ranks: BTreeMap<String, i64> = ranks
            .iter()
            .map(|r| (py_str(&val(r, "id")), num(r, "item_rank")))
            .collect();
        for index in paired {
            let id = py_str(&val(&output[index], "id"));
            if let Some(rank) = ranks.get(&id) {
                set(
                    &mut output[index],
                    "queueStatus",
                    s(match rank {
                        3 => "review",
                        2 => "failed",
                        1 => "pending",
                        _ => "completed",
                    }),
                );
            }
        }
    }
    Ok(output)
}

#[cfg(test)]
mod period_tests {
    /// tests/rust_port/repro_dashboard_bucket_aliases.py
    #[test]
    fn every_period_spelling_buckets_like_its_canonical_name() {
        for (canonical, aliases) in [
            ("30days", &["30days", "30day", "30d"][..]),
            ("3months", &["3months", "3month", "3m"]),
            ("6months", &["6months", "6month", "6m"]),
            ("1year", &["1year", "1years", "1y"]),
            ("4years", &["4years", "4year", "4y"]),
            ("5years", &["5years", "5year", "5y"]),
            ("10years", &["10years", "10year", "10y"]),
            ("15years", &["15years", "15year", "15y"]),
            ("ytd", &["ytd"]),
            ("all", &["all"]),
        ] {
            for alias in aliases {
                assert_eq!(super::canonical_period(alias), canonical, "{alias}");
            }
        }
    }
}
