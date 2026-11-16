//! The chart's data pipeline, ported from overview-dashboard/model.ts: period windows, sampling,
//! activity events and their flows, treasury points, marker anchoring, the marker view, brush
//! normalization and marker bucketing.

use crate::format::{self, Day};
use crate::model::{self, Snapshot, Tx, TxStatus};

pub const DAY_MS: f64 = 86_400_000.0;
pub const MS_PER_YEAR: f64 = 365.2425 * DAY_MS;
/// The shortest window the range navigator and the zoom allow (the web's brush keeps 7/3 days;
/// a week still reads as a range on a multi-year track and shows daily steps).
pub const BRUSH_MIN_WINDOW_MS: f64 = 7.0 * DAY_MS;
/// `Date.UTC(2009, 0, 3, 18, 15, 5)`.
pub const GENESIS_MS: f64 = 1_230_747_305_000.0;
pub const DEFAULT_INCOMING_MIN: f64 = 0.0025;
pub const DEFAULT_OUTGOING_MIN: f64 = 0.0;
pub const SLIDER_MARKS: [f64; 6] = [0.0, 0.0025, 0.01, 0.1, 0.5, 1.0];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Period {
    Auto,
    D30,
    /// Year to date: 1 January of the current year until now (`lib/period.ts` maps `ytd` to the
    /// current calendar year). An owner request; the web's chart offers no YTD chip.
    Ytd,
    Y1,
    Y4,
    Y10,
    All,
    Year(i32),
}

impl Period {
    pub const RANGES: [Period; 6] = [
        Period::D30,
        Period::Ytd,
        Period::Y1,
        Period::Y4,
        Period::Y10,
        Period::All,
    ];
    /// A language-independent id for element keys and scripts (`30D`, `1Y`, `All`, …).
    pub fn id(self) -> String {
        match self {
            Period::D30 => "30D".into(),
            Period::Ytd => "YTD".into(),
            Period::Y1 => "1Y".into(),
            Period::Y4 => "4Y".into(),
            Period::Y10 => "10Y".into(),
            Period::All => "All".into(),
            Period::Auto => "Auto".into(),
            Period::Year(y) => y.to_string(),
        }
    }
    fn label_key(self) -> Option<&'static str> {
        Some(match self {
            Period::D30 => "30days",
            Period::Ytd => return None,
            Period::Y1 => "1year",
            Period::Y4 => "4years",
            Period::Y10 => "10years",
            Period::All => "all",
            Period::Auto => "auto",
            Period::Year(_) => return None,
        })
    }
    /// The chip text (`overview:period.short.*`).
    pub fn short(self) -> String {
        match (self, self.label_key()) {
            (_, Some(k)) => crate::i18n::t(&format!("overview:period.short.{k}")),
            (Period::Year(y), None) => y.to_string(),
            (Period::Ytd, None) => crate::i18n::t("chrome:range.ytd"),
            _ => String::new(),
        }
    }
    /// The chip's title and name (`overview:period.*`).
    pub fn long(self) -> String {
        match (self, self.label_key()) {
            (_, Some(k)) => crate::i18n::t(&format!("overview:period.{k}")),
            (Period::Year(y), None) => y.to_string(),
            (Period::Ytd, None) => crate::i18n::t("assistant:visual.period.ytd"),
            _ => String::new(),
        }
    }
    pub fn long_range(self) -> bool {
        matches!(self, Period::All | Period::Y4 | Period::Y10)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AFlow {
    Incoming,
    Outgoing,
    Movement,
    Fee,
}

impl AFlow {
    /// `overview:activityFlow.*`: Received, Spent, Movement, Fee.
    pub fn label(self) -> String {
        crate::i18n::t(match self {
            AFlow::Incoming => "overview:activityFlow.incoming",
            AFlow::Outgoing => "overview:activityFlow.outgoing",
            AFlow::Movement => "overview:activityFlow.movement",
            AFlow::Fee => "overview:activityFlow.fee",
        })
    }
    const ORDER: [AFlow; 4] = [
        AFlow::Incoming,
        AFlow::Movement,
        AFlow::Outgoing,
        AFlow::Fee,
    ];
}

/// `activityFlowForTx`.
pub fn flow_for(tx: &Tx) -> AFlow {
    if tx.tx_type == "Fee" {
        return AFlow::Fee;
    }
    match model::flow(tx) {
        model::Flow::Incoming => AFlow::Incoming,
        model::Flow::Outgoing => AFlow::Outgoing,
        model::Flow::Transfer | model::Flow::Swap => AFlow::Movement,
    }
}

/// One activity row (`TreasuryActivityEvent`).
#[derive(Clone, Debug)]
pub struct Event {
    pub t_ms: f64,
    pub seq: usize,
    pub signed: f64,
    pub fee: f64,
    pub post_balance: Option<f64>,
    pub post_basis: Option<f64>,
    pub price: f64,
    pub value_eur: f64,
    pub flow: AFlow,
    pub volume: f64,
    pub tx_type: String,
    pub account: String,
    pub counter: String,
    pub tag: String,
    pub status: TxStatus,
    pub conf: f64,
    pub event_id: String,
    pub tx_id: String,
}

/// `parseOverviewTxDate`, in ms.
fn tx_time_ms(tx: &Tx) -> Option<f64> {
    let raw = tx
        .occurred_at
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&tx.date);
    if raw.is_empty() {
        return None;
    }
    format::parse_ts(raw).map(|s| s * 1000.0)
}

/// `activityTxs`.
pub fn events(s: &Snapshot) -> Vec<Event> {
    let src = if s.activity_txs.is_empty() {
        &s.txs
    } else {
        &s.activity_txs
    };
    let rate = s.rate();
    let mut out: Vec<Event> = src
        .iter()
        .enumerate()
        .filter_map(|(seq, tx)| {
            if tx.excluded {
                return None;
            }
            let t_ms = tx_time_ms(tx)?;
            let signed = tx.amount_sat / 1e8;
            let btc = signed.abs();
            let fee = tx.fee_sat.abs() / 1e8;
            let flow = flow_for(tx);
            let paired = tx
                .pair
                .as_ref()
                .map(|p| {
                    btc.max(p.out_amount_sat.abs() / 1e8)
                        .max(p.in_amount_sat.abs() / 1e8)
                })
                .unwrap_or(btc);
            let volume = match flow {
                AFlow::Fee => btc.max(fee),
                AFlow::Movement => paired,
                _ => btc,
            };
            if volume <= 0.0 && fee <= 0.0 {
                return None;
            }
            let value_eur = tx.eur.unwrap_or(0.0).abs();
            let price = if value_eur > 0.0 && btc > 0.0 {
                value_eur / btc
            } else {
                tx.rate.unwrap_or(rate)
            };
            Some(Event {
                t_ms,
                seq,
                signed,
                fee,
                post_balance: tx.balance_btc,
                post_basis: tx.cost_basis_eur,
                price,
                value_eur,
                flow,
                volume,
                tx_type: tx.tx_type.clone(),
                account: tx.account.clone(),
                counter: tx.counter.clone(),
                tag: tx.tag.clone(),
                status: model::tx_status(tx),
                conf: tx.conf,
                event_id: tx
                    .explorer_id
                    .clone()
                    .filter(|e| !e.is_empty())
                    .unwrap_or_else(|| {
                        if tx.external_id.is_empty() {
                            tx.id.clone()
                        } else {
                            tx.external_id.clone()
                        }
                    }),
                tx_id: tx.id.clone(),
            })
        })
        .collect();
    out.sort_by(|a, b| a.t_ms.total_cmp(&b.t_ms).then(a.seq.cmp(&b.seq)));
    out
}

/// A day string's midnight (`parseSeriesDate`).
pub fn day_ms(date: &str) -> f64 {
    Day::parse(date)
        .map(|d| d.epoch_days() as f64 * DAY_MS)
        .unwrap_or_else(|| format::now_secs() * 1000.0)
}

fn day_of_ms(ms: f64) -> Day {
    Day::from_epoch_days((ms / DAY_MS).floor() as i64)
}

fn year_of(ms: f64) -> i32 {
    day_of_ms(ms).y
}

/// `treasuryPeriodStart`: calendar arithmetic in UTC, keeping the time of day.
fn period_start(latest_ms: f64, p: Period) -> Option<f64> {
    let d = day_of_ms(latest_ms);
    let tod = latest_ms - d.epoch_days() as f64 * DAY_MS;
    let shifted = |years: i32| {
        (Day {
            y: d.y - years,
            m: d.m,
            d: d.d,
        }
        .epoch_days() as f64)
            * DAY_MS
            + tod
    };
    match p {
        Period::D30 => Some(latest_ms - 30.0 * DAY_MS),
        // From 1 January of the current calendar year (UTC, like the chart's other windows).
        Period::Ytd => Some(
            Day {
                y: year_of(format::now_secs() * 1000.0),
                m: 1,
                d: 1,
            }
            .epoch_days() as f64
                * DAY_MS,
        ),
        Period::Y1 => Some(shifted(1)),
        Period::Y4 => Some(shifted(4)),
        Period::Y10 => Some(shifted(10)),
        _ => None,
    }
}

/// `isTimeInTreasuryPeriod`.
pub fn in_period(t: f64, latest: f64, p: Period) -> bool {
    match p {
        Period::Auto | Period::All => true,
        Period::Year(y) => year_of(t) == y,
        _ => match period_start(latest, p) {
            Some(start) => t >= start && t <= latest,
            None => true,
        },
    }
}

/// `treasuryHistoryBounds`.
pub fn history_bounds(s: &Snapshot, evs: &[Event]) -> (f64, f64) {
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for p in &s.portfolio_series {
        if let Some(d) = Day::parse(&p.date) {
            let t = d.epoch_days() as f64 * DAY_MS;
            lo = lo.min(t);
            hi = hi.max(t);
        }
    }
    for e in evs {
        lo = lo.min(e.t_ms);
        hi = hi.max(e.t_ms);
    }
    if !hi.is_finite() {
        let now = format::now_secs() * 1000.0;
        return (now, now);
    }
    (lo, hi)
}

pub fn history_years(lo: f64, hi: f64) -> f64 {
    ((hi - lo) / MS_PER_YEAR).max(0.0)
}

/// `selectablePeriods`.
pub fn selectable(years: f64) -> Vec<Period> {
    let mut v = vec![Period::D30, Period::Ytd, Period::Y1];
    if years >= 4.0 {
        v.push(Period::Y4);
    }
    if years >= 10.0 {
        v.push(Period::Y10);
    }
    v.push(Period::All);
    v
}

fn auto_candidates(years: f64) -> Vec<Period> {
    let mut v = vec![Period::Y1];
    if years >= 4.0 * 0.8 {
        v.push(Period::Y4);
    }
    if years >= 10.0 * 0.8 {
        v.push(Period::Y10);
    }
    v.push(Period::All);
    v
}

/// `treasuryYears`: years with activity, newest first, with counts.
pub fn years(evs: &[Event]) -> Vec<(i32, usize)> {
    let mut m: std::collections::BTreeMap<i32, usize> = Default::default();
    for e in evs {
        *m.entry(year_of(e.t_ms)).or_default() += 1;
    }
    m.into_iter().rev().collect()
}

/// `resolveAutoTimePeriod`.
pub fn resolve(s: &Snapshot, evs: &[Event], p: Period) -> Period {
    if p != Period::Auto {
        return p;
    }
    if evs.is_empty() {
        return Period::Y1;
    }
    let (lo, latest) = history_bounds(s, evs);
    let meaningful: Vec<&Event> = evs.iter().filter(|e| e.volume >= 0.00001).collect();
    if meaningful.is_empty() {
        return Period::Y1;
    }
    let target = meaningful.len().min(3);
    for cand in auto_candidates(history_years(lo, latest)) {
        let visible = meaningful
            .iter()
            .filter(|e| in_period(e.t_ms, latest, cand))
            .count();
        if visible < target {
            continue;
        }
        let mut values: Vec<f64> = s
            .portfolio_series
            .iter()
            .filter(|p| in_period(day_ms(&p.date), latest, cand))
            .map(|p| p.balance_btc)
            .collect();
        values.extend(
            evs.iter()
                .filter(|e| in_period(e.t_ms, latest, cand))
                .filter_map(|e| e.post_balance),
        );
        if meaningful_range(&values) {
            return cand;
        }
    }
    Period::All
}

fn meaningful_range(v: &[f64]) -> bool {
    if v.len() < 2 {
        return true;
    }
    let min = v.iter().copied().fold(f64::INFINITY, f64::min);
    let max = v.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let latest = v.last().copied().unwrap_or(max).abs();
    max - min >= 0.001f64.max(latest * 0.01)
}

/// Completeness: `basisIncompleteFromMs` (None = complete, -inf = everywhere).
pub fn basis_from_ms(s: &Snapshot) -> Option<f64> {
    let c = &s.fiat.completeness;
    if c.cost_basis_complete {
        return None;
    }
    Some(
        c.earliest_incomplete_at
            .as_deref()
            .and_then(format::parse_ts)
            .map(|t| t * 1000.0)
            .unwrap_or(f64::NEG_INFINITY),
    )
}

fn incomplete_on_day(from: Option<f64>, date: &str) -> bool {
    match from {
        None => false,
        Some(f) if f == f64::NEG_INFINITY => true,
        Some(f) => match Day::parse(date) {
            Some(d) => d.epoch_days() as f64 * DAY_MS + DAY_MS > f,
            None => true,
        },
    }
}

fn incomplete_at(from: Option<f64>, t: f64) -> bool {
    match from {
        None => false,
        Some(f) => !t.is_finite() || t >= f,
    }
}

/// Event-only details of a point.
#[derive(Clone, Debug)]
pub struct Ev {
    pub flow: AFlow,
    pub size: f64,
    pub activity_btc: f64,
    pub signed: f64,
    pub fee: f64,
    pub fiat: f64,
    pub marker_bal: f64,
    pub event_price: f64,
    pub tx_type: Option<String>,
    pub account: Option<String>,
    pub counter: Option<String>,
    pub tag: Option<String>,
    pub status: Option<TxStatus>,
    pub conf: Option<f64>,
    pub id: Option<String>,
    pub tx_id: Option<String>,
    pub count: usize,
    pub mixed: bool,
    /// The events a merged dot stands for.
    pub members: Vec<Pt>,
}

/// `TreasuryChartPoint`.
#[derive(Clone, Debug)]
pub struct Pt {
    pub date: String,
    pub month: String,
    pub detail: String,
    pub balance: f64,
    pub value_eur: f64,
    pub cost_basis: f64,
    pub unrealized: f64,
    pub basis_incomplete: bool,
    pub price: f64,
    pub avg_cost: Option<f64>,
    pub l_bal: Option<f64>,
    pub l_val: Option<f64>,
    pub l_price: Option<f64>,
    pub l_avg: Option<f64>,
    pub brush_bal: f64,
    pub sort_ms: f64,
    pub power_days: f64,
    pub ev: Option<Box<Ev>>,
}

impl Pt {
    pub fn is_event(&self) -> bool {
        self.ev.is_some()
    }
}

pub fn power_days(ms: f64) -> f64 {
    if !ms.is_finite() {
        return 1.0;
    }
    ((ms - GENESIS_MS) / DAY_MS).max(1.0)
}

/// `samplePortfolioPoints`'s cap for one period and density.
pub fn sample_max(p: Period, detailed: bool) -> usize {
    if detailed {
        if p.long_range() { 720 } else { 420 }
    } else if p == Period::D30 {
        90
    } else {
        180
    }
}

/// `samplePortfolioPoints` over the display points in `r`, as indices: the balance points are
/// thinned to at most `max` (keeping the first and the last), activity points (the 30-day
/// view's line stops) are all kept.
pub fn sample_indices(display: &[Pt], r: Range, max: usize) -> Vec<usize> {
    if display.is_empty() {
        return vec![];
    }
    let end = r.end.min(display.len() - 1);
    let start = r.start.min(end);
    let nbase = display[start..=end]
        .iter()
        .filter(|p| !p.is_event())
        .count();
    if nbase <= max {
        return (start..=end).collect();
    }
    let step = (nbase - 2).div_ceil((max - 2).max(1));
    let mut out = Vec::with_capacity(max + 8);
    let mut ord = 0usize;
    for (i, p) in display.iter().enumerate().take(end + 1).skip(start) {
        let keep = if p.is_event() {
            true
        } else {
            let k = ord == 0 || ord == nbase - 1 || ord.is_multiple_of(step);
            ord += 1;
            k
        };
        if keep || i == start || i == end {
            out.push(i);
        }
    }
    out
}

/// Everything the chart derives from one snapshot and one period: the whole history.
pub struct Series {
    pub resolved: Period,
    /// `enrichTreasuryChartData`, sorted.
    pub points: Vec<Pt>,
}

/// `getDataForPeriod` + `enrichTreasuryChartData` over the whole history: the period picks
/// the window on it (`chip_win`), the formats and whether the line runs through activity.
/// The plot samples its window (`sample_indices`); nothing is dropped here, so the range
/// navigator and every zoom work on the full-resolution history.
pub fn build(s: &Snapshot, evs: &[Event], period: Period) -> Series {
    let resolved = resolve(s, evs, period);
    let from = basis_from_ms(s);
    let rate = s.rate();
    // Base points.
    let mut series: Vec<&model::PortfolioPoint> = s.portfolio_series.iter().collect();
    series.sort_by(|a, b| a.date.cmp(&b.date));
    let sampled = series;
    let base: Vec<Pt> = sampled
        .iter()
        .map(|p| {
            let day = Day::parse(&p.date).unwrap_or(Day {
                y: 1970,
                m: 1,
                d: 1,
            });
            let month = if resolved == Period::D30 {
                day.month_day_ui()
            } else {
                day.month_year_ui()
            };
            let incomplete = incomplete_on_day(from, &p.date);
            let price = match p.price_eur {
                Some(v) if v > 0.0 => v,
                _ if p.balance_btc > 0.0 => p.value_eur / p.balance_btc,
                _ => rate,
            };
            let avg = (!incomplete && p.balance_btc > 0.0 && p.cost_basis_eur > 0.0)
                .then(|| p.cost_basis_eur / p.balance_btc);
            let sort_ms = day.epoch_days() as f64 * DAY_MS + DAY_MS - 1.0;
            Pt {
                date: p.date.get(0..10).unwrap_or(&p.date).to_owned(),
                month,
                detail: day.long_ui(),
                balance: p.balance_btc,
                value_eur: p.value_eur,
                cost_basis: p.cost_basis_eur,
                unrealized: p.value_eur - p.cost_basis_eur,
                basis_incomplete: incomplete,
                price,
                avg_cost: avg,
                l_bal: Some(p.balance_btc),
                l_val: Some(p.value_eur),
                l_price: Some(price),
                l_avg: avg,
                brush_bal: p.balance_btc,
                sort_ms,
                power_days: power_days(sort_ms),
                ev: None,
            }
        })
        .collect();

    let draw_lines = resolved == Period::D30;
    let latest_balance = s.latest_balance_btc();
    let mut by_day: std::collections::HashMap<&str, usize> = Default::default();
    for (i, p) in base.iter().enumerate() {
        by_day.insert(p.date.as_str(), i);
    }
    let nearest = |t: f64| -> Option<usize> {
        // Last base point at or before t, else the first.
        let idx = base.partition_point(|p| p.sort_ms <= t);
        if idx > 0 {
            Some(idx - 1)
        } else if base.is_empty() {
            None
        } else {
            Some(0)
        }
    };
    let mut event_points: Vec<Pt> = Vec::new();
    for e in evs {
        let anchor = nearest(e.t_ms).map(|i| &base[i]);
        let marker_anchor = if draw_lines {
            None
        } else {
            let day = day_of_ms(e.t_ms).iso();
            by_day
                .get(day.as_str())
                .copied()
                .or_else(|| {
                    let i = base.partition_point(|p| p.sort_ms < e.t_ms);
                    (i < base.len()).then_some(i)
                })
                .or_else(|| nearest(e.t_ms))
                .map(|i| &base[i])
        };
        let balance = e
            .post_balance
            .or(anchor.map(|a| a.balance))
            .unwrap_or(latest_balance);
        let basis = e
            .post_basis
            .or(anchor.map(|a| a.cost_basis))
            .unwrap_or(s.fiat.eur_cost_basis);
        let value = if balance > 0.0 {
            balance * e.price
        } else {
            anchor.map(|a| a.value_eur).unwrap_or(s.fiat.eur_balance)
        };
        let incomplete = incomplete_at(from, e.t_ms);
        let avg = (!incomplete && balance > 0.0 && basis > 0.0).then(|| basis / balance);
        let eday = day_of_ms(e.t_ms);
        let event_key = format!(
            "{}#{}",
            iso_ms(e.t_ms),
            if e.tx_id.is_empty() {
                e.seq.to_string()
            } else {
                e.tx_id.clone()
            }
        );
        let date = if draw_lines {
            event_key.clone()
        } else {
            marker_anchor
                .map(|a| a.date.clone())
                .unwrap_or(event_key.clone())
        };
        let month_day = Day::parse(&date).unwrap_or(eday);
        event_points.push(Pt {
            month: month_day.month_year_ui(),
            date,
            detail: eday.long_ui(),
            balance,
            value_eur: value,
            cost_basis: basis,
            unrealized: value - basis,
            basis_incomplete: incomplete,
            price: e.price,
            avg_cost: avg,
            l_bal: draw_lines.then_some(balance),
            l_val: draw_lines.then_some(value),
            l_price: None,
            l_avg: if draw_lines { avg } else { None },
            brush_bal: balance,
            sort_ms: e.t_ms + e.seq as f64 / 1000.0,
            power_days: power_days(e.t_ms),
            ev: Some(Box::new(Ev {
                flow: e.flow,
                size: e.volume.max(e.fee),
                activity_btc: e.volume,
                signed: e.signed,
                fee: e.fee,
                fiat: e.value_eur,
                marker_bal: marker_anchor.map(|a| a.balance).unwrap_or(balance),
                event_price: e.price,
                tx_type: Some(e.tx_type.clone()),
                account: Some(e.account.clone()).filter(|v| !v.is_empty()),
                counter: Some(e.counter.clone()).filter(|v| !v.is_empty()),
                tag: Some(e.tag.clone()).filter(|v| !v.is_empty()),
                status: Some(e.status),
                conf: Some(e.conf),
                id: Some(e.event_id.clone()),
                tx_id: Some(e.tx_id.clone()),
                count: 1,
                mixed: false,
                members: vec![],
            })),
        });
    }
    let mut points = base;
    points.extend(event_points);
    points.sort_by(|a, b| {
        a.sort_ms
            .total_cmp(&b.sort_ms)
            .then_with(|| match (a.is_event(), b.is_event()) {
                (true, false) => std::cmp::Ordering::Less,
                (false, true) => std::cmp::Ordering::Greater,
                _ => a.date.cmp(&b.date),
            })
    });
    Series { resolved, points }
}

/// `toISOString()` of an ms timestamp.
fn iso_ms(ms: f64) -> String {
    let d = day_of_ms(ms);
    let rem = ms - d.epoch_days() as f64 * DAY_MS;
    let secs = (rem / 1000.0).floor() as i64;
    let millis = (rem - secs as f64 * 1000.0).round() as i64;
    format!(
        "{}T{:02}:{:02}:{:02}.{:03}Z",
        d.iso(),
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        millis.min(999)
    )
}

// ---- Windows ----------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Range {
    pub start: usize,
    pub end: usize,
}

/// The navigator's mini balance line: the whole history in at most `buckets` equal time
/// slices, each the last balance in it (a step series, like the plot's line).
pub fn mini_series(times: &[f64], values: &[f64], buckets: usize) -> Vec<(f64, f64)> {
    let n = times.len().min(values.len());
    if n == 0 {
        return vec![];
    }
    let (t0, t1) = (times[0], times[n - 1]);
    if n <= buckets || t1 <= t0 {
        return (0..n).map(|i| (times[i], values[i])).collect();
    }
    let width = (t1 - t0) / buckets as f64;
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(buckets + 1);
    let mut cur = usize::MAX;
    for i in 0..n {
        let b = (((times[i] - t0) / width) as usize).min(buckets - 1);
        if b != cur {
            out.push((times[i], values[i]));
            cur = b;
        } else if let Some(last) = out.last_mut() {
            last.1 = values[i];
        }
    }
    if out.last().is_some_and(|l| l.0 < t1) {
        out.push((t1, values[n - 1]));
    }
    out
}

/// Index of the time nearest `t` in a sorted list.
pub fn nearest_time(times: &[f64], t: f64) -> usize {
    let i = times.partition_point(|x| *x < t);
    if i == 0 {
        0
    } else if i >= times.len() {
        times.len().saturating_sub(1)
    } else if (times[i] - t).abs() < (t - times[i - 1]).abs() {
        i
    } else {
        i - 1
    }
}

/// A time window on the history, in ms (the moments of `Pt::sort_ms`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Win {
    pub lo: f64,
    pub hi: f64,
}

impl Win {
    pub fn span(&self) -> f64 {
        self.hi - self.lo
    }
    /// The same window, give or take a millisecond.
    pub fn same(&self, o: &Win) -> bool {
        (self.lo - o.lo).abs() < 1.0 && (self.hi - o.hi).abs() < 1.0
    }
}

/// `w` ordered, at least the minimum window long (the whole history when that is shorter),
/// and moved inside `[first, last]` without changing its length.
pub fn clamp_win(w: Win, first: f64, last: f64) -> Win {
    let total = last - first;
    if total <= 0.0 || total.is_nan() {
        return Win {
            lo: first,
            hi: last.max(first),
        };
    }
    let min = BRUSH_MIN_WINDOW_MS.min(total);
    let (mut lo, mut hi) = (w.lo.min(w.hi), w.lo.max(w.hi));
    if hi - lo < min {
        let mid = (lo + hi) / 2.0;
        (lo, hi) = (mid - min / 2.0, mid + min / 2.0);
    }
    let span = (hi - lo).min(total);
    if lo < first {
        (lo, hi) = (first, first + span);
    }
    if hi > last {
        (lo, hi) = (last - span, last);
    }
    Win { lo, hi }
}

/// The window a range chip puts on the history (`getDataForPeriod`'s window): from the
/// period's start (against the last balance day, as the web filters the series) to the end of
/// the history; a calendar year as itself; `All` the whole history.
pub fn chip_win(display: &[Pt], p: Period) -> Win {
    let (Some(first), Some(last)) = (
        display.first().map(|q| q.sort_ms),
        display.last().map(|q| q.sort_ms),
    ) else {
        return Win { lo: 0.0, hi: 0.0 };
    };
    let latest_day = display
        .iter()
        .rev()
        .find(|q| !q.is_event())
        .map(|q| day_ms(&q.date))
        .unwrap_or(last);
    let w = match p {
        Period::All | Period::Auto => Win {
            lo: first,
            hi: last,
        },
        Period::Year(y) => Win {
            lo: Day { y, m: 1, d: 1 }.epoch_days() as f64 * DAY_MS,
            hi: Day {
                y: y + 1,
                m: 1,
                d: 1,
            }
            .epoch_days() as f64
                * DAY_MS
                - 1.0,
        },
        _ => Win {
            lo: period_start(latest_day, p).unwrap_or(first),
            hi: last,
        },
    };
    clamp_win(
        Win {
            lo: w.lo.max(first),
            hi: w.hi.min(last),
        },
        first,
        last,
    )
}

/// The display points a window shows: the last point before it (its balance carries into the
/// window's left edge, as a step line holds its value) and the points inside.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub carry: Option<usize>,
    /// `first..end` are inside the window (empty in a stretch without points).
    pub first: usize,
    pub end: usize,
    /// A point follows the window, so the line runs on past its right edge.
    pub more: bool,
}

pub fn window_span(times: &[f64], w: Win) -> Span {
    let first = times.partition_point(|t| *t < w.lo);
    let end = times.partition_point(|t| *t <= w.hi);
    let carry = (first > 0 && (first == end || times[first] > w.lo)).then(|| first - 1);
    Span {
        carry,
        first,
        end,
        more: end < times.len(),
    }
}

// ---- Marker bucketing -----------------------------------------------------------------------

fn dominant(group: &[&Pt]) -> AFlow {
    let mut totals = [0.0f64; 4];
    for p in group {
        if let Some(e) = &p.ev {
            let i = AFlow::ORDER.iter().position(|f| *f == e.flow).unwrap();
            totals[i] += if e.size > 0.0 { e.size } else { e.activity_btc };
        }
    }
    // Ties keep "movement" (strict >), starting from movement at 0.
    let mut winner = (AFlow::Movement, 0.0);
    for f in [
        AFlow::Incoming,
        AFlow::Outgoing,
        AFlow::Movement,
        AFlow::Fee,
    ] {
        let v = totals[AFlow::ORDER.iter().position(|x| *x == f).unwrap()];
        if v > winner.1 {
            winner = (f, v);
        }
    }
    winner.0
}

fn merged(group: &[&Pt]) -> Pt {
    let anchor = group[0];
    let last = group[group.len() - 1];
    let ev = |p: &Pt| p.ev.as_deref().unwrap().clone();
    let sum = |f: &dyn Fn(&Ev) -> f64| {
        group
            .iter()
            .map(|p| f(p.ev.as_deref().unwrap()))
            .sum::<f64>()
    };
    let flows: std::collections::HashSet<AFlow> =
        group.iter().map(|p| p.ev.as_ref().unwrap().flow).collect();
    let mut out = last.clone();
    out.date = anchor.date.clone();
    out.month = anchor.month.clone();
    out.detail = if anchor.detail == last.detail {
        anchor.detail.clone()
    } else {
        format!("{} – {}", anchor.detail, last.detail)
    };
    out.sort_ms = anchor.sort_ms;
    out.power_days = anchor.power_days;
    let a = ev(anchor);
    out.ev = Some(Box::new(Ev {
        flow: dominant(group),
        size: group
            .iter()
            .map(|p| {
                p.ev.as_ref()
                    .map(|e| if e.size > 0.0 { e.size } else { e.activity_btc })
                    .unwrap_or(0.0)
            })
            .fold(0.0, f64::max),
        activity_btc: sum(&|e| e.activity_btc),
        signed: sum(&|e| e.signed),
        fee: sum(&|e| e.fee),
        fiat: sum(&|e| e.fiat),
        marker_bal: a.marker_bal,
        event_price: last.ev.as_ref().unwrap().event_price,
        tx_type: None,
        account: None,
        counter: None,
        tag: None,
        status: None,
        conf: None,
        id: None,
        tx_id: None,
        count: group.len(),
        mixed: flows.len() > 1,
        members: group.iter().map(|p| (*p).clone()).collect(),
    }));
    out
}

/// `bucketActivityMarkers`: merges markers closer than `len / target` columns.
pub fn bucket(markers: Vec<Pt>, display: &[Pt], target: usize) -> Vec<Pt> {
    if markers.len() < 2 {
        return markers;
    }
    let target = target.max(1);
    let mut index: std::collections::HashMap<&str, usize> = Default::default();
    for (i, p) in display.iter().enumerate() {
        index.entry(p.date.as_str()).or_insert(i);
    }
    let col = |p: &Pt| index.get(p.date.as_str()).copied().unwrap_or(0);
    let gap = display.len().div_ceil(target).max(1);
    let mut ordered: Vec<&Pt> = markers.iter().collect();
    ordered.sort_by(|a, b| col(a).cmp(&col(b)).then(a.sort_ms.total_cmp(&b.sort_ms)));
    let mut groups: Vec<Vec<&Pt>> = Vec::new();
    let mut anchor_col: i64 = i64::MIN / 2;
    for m in ordered {
        let c = col(m) as i64;
        if c - anchor_col >= gap as i64 {
            groups.push(vec![m]);
            anchor_col = c;
        } else if let Some(g) = groups.last_mut() {
            g.push(m);
        }
    }
    groups
        .into_iter()
        .map(|g| {
            if g.len() == 1 {
                g[0].clone()
            } else {
                merged(&g)
            }
        })
        .collect()
}

/// `activityFlowShares`: slices of a merged dot in a stable order.
pub fn flow_shares(members: &[Pt]) -> Vec<(AFlow, f64)> {
    let mut totals = [0.0f64; 4];
    let mut seen = [false; 4];
    for p in members {
        if let Some(e) = &p.ev {
            let i = AFlow::ORDER.iter().position(|f| *f == e.flow).unwrap();
            totals[i] += (if e.size > 0.0 { e.size } else { e.activity_btc }).max(1e-9);
            seen[i] = true;
        }
    }
    let total: f64 = totals.iter().sum();
    if total <= 0.0 {
        return vec![];
    }
    AFlow::ORDER
        .iter()
        .enumerate()
        .filter(|(i, _)| seen[*i])
        .map(|(i, f)| (*f, totals[i] / total))
        .collect()
}

/// `balanceRangeChange` in integer msat: (delta msat, percent).
pub fn range_change(points: &[&Pt]) -> Option<(i128, Option<f64>)> {
    if points.len() < 2 {
        return None;
    }
    let msat = |btc: f64| -> Option<i128> { btc.is_finite().then(|| btc_to_msat(btc)) };
    let start = msat(points[0].balance)?;
    let end = msat(points[points.len() - 1].balance)?;
    let delta = end - start;
    let pct = (start > 0).then(|| delta as f64 / start as f64 * 100.0);
    Some((delta, pct))
}

/// The shortest decimal of a JSON number, parsed exactly into msat (as the web's
/// `decimalBtcToMsat(String(abs))`).
fn btc_to_msat(btc: f64) -> i128 {
    let s = format!("{}", btc.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut frac = frac.to_owned();
    frac.truncate(11);
    while frac.len() < 11 {
        frac.push('0');
    }
    let v = int.parse::<i128>().unwrap_or(0) * 100_000_000_000 + frac.parse::<i128>().unwrap_or(0);
    if btc < 0.0 { -v } else { v }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn msat_is_exact() {
        assert_eq!(btc_to_msat(98.58445804), 9_858_445_804_000);
        assert_eq!(btc_to_msat(-0.00000001), -1000);
    }

    fn days(n: usize) -> Vec<f64> {
        (0..n).map(|i| i as f64 * DAY_MS).collect()
    }

    #[test]
    fn windows_clamp_into_the_history() {
        let (a, b) = (0.0, 100.0 * DAY_MS);
        // Too short grows to a week around its middle.
        let w = clamp_win(
            Win {
                lo: 50.0 * DAY_MS,
                hi: 51.0 * DAY_MS,
            },
            a,
            b,
        );
        assert!(
            (w.span() - BRUSH_MIN_WINDOW_MS).abs() < 1.0 && (w.lo + w.hi) / 2.0 == 50.5 * DAY_MS
        );
        // Past an end slides back in, keeping its length; reversed edges are ordered.
        assert_eq!(
            clamp_win(
                Win {
                    lo: 90.0 * DAY_MS,
                    hi: 120.0 * DAY_MS
                },
                a,
                b
            ),
            Win {
                lo: 70.0 * DAY_MS,
                hi: b
            }
        );
        assert_eq!(
            clamp_win(
                Win {
                    lo: 20.0 * DAY_MS,
                    hi: -10.0 * DAY_MS
                },
                a,
                b
            ),
            Win {
                lo: 0.0,
                hi: 30.0 * DAY_MS
            }
        );
        // Longer than the history is the history; a history shorter than a week is all of it.
        assert_eq!(
            clamp_win(
                Win {
                    lo: -5.0,
                    hi: b + 5.0
                },
                a,
                b
            ),
            Win { lo: a, hi: b }
        );
        assert_eq!(
            clamp_win(
                Win {
                    lo: 0.0,
                    hi: DAY_MS
                },
                0.0,
                3.0 * DAY_MS
            ),
            Win {
                lo: 0.0,
                hi: 3.0 * DAY_MS
            }
        );
    }

    #[test]
    fn a_window_shows_its_points_and_carries_the_one_before() {
        let mut t = days(100);
        t.extend((300..400).map(|i| i as f64 * DAY_MS));
        let s = window_span(
            &t,
            Win {
                lo: 10.5 * DAY_MS,
                hi: 20.0 * DAY_MS,
            },
        );
        assert_eq!(
            s,
            Span {
                carry: Some(10),
                first: 11,
                end: 21,
                more: true
            }
        );
        // Starting on a point needs no carry; the history's end has nothing after it.
        assert_eq!(
            window_span(
                &t,
                Win {
                    lo: 0.0,
                    hi: 399.0 * DAY_MS
                }
            ),
            Span {
                carry: None,
                first: 0,
                end: 200,
                more: false
            }
        );
        // Inside the gap: only the carried balance.
        let g = window_span(
            &t,
            Win {
                lo: 150.0 * DAY_MS,
                hi: 160.0 * DAY_MS,
            },
        );
        assert_eq!((g.carry, g.first == g.end, g.more), (Some(99), true, true));
    }

    #[test]
    fn iso_and_power_days() {
        assert_eq!(iso_ms(0.0), "1970-01-01T00:00:00.000Z");
        assert!((power_days(GENESIS_MS + DAY_MS) - 1.0).abs() < 1e-9);
    }
}
