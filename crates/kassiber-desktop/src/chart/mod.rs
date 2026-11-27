//! The BTC activity chart (overview-dashboard/BtcActivityChart.tsx and model.ts).
//!
//! `data` ports the model pipeline, `axes` the scales, `plot` draws, `tooltip` holds the hover
//! cards and the basis-gap popover, `toolbar` the range/scale footer and gear menu, `interact`
//! the zoom / pan / measure gestures and keys, and `expanded` the full-window dialog.
//!
//! The web keeps the period per book in the persisted UI store and mirrors chart settings in
//! the URL; here they live in `ChartUi` for the session.

mod axes;
mod brush;
mod controls;
mod data;
mod expanded;
mod interact;
mod metrics;
mod plot;
mod strings;
mod tip_place;
mod toolbar;
mod tooltip;

use std::cell::RefCell;
use std::rc::Rc;

use rune::{Color, Cx, El, Live, Shape, Tx, canvas, col, el, row, text};

use crate::app::{App, Reply};
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::model::{self, Snapshot};
use crate::theme::Tk;
use crate::ui::{ic, outline_button, primary_button};

pub use data::{AFlow, Period};
use data::{Event, Pt, Range, Win};
pub use interact::{GestureKind, PlotGesture};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SeriesKey {
    Primary,
    PortfolioValue,
    Events,
    Basis,
    Price,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Visible {
    pub primary: bool,
    pub portfolio_value: bool,
    pub price: bool,
    pub basis: bool,
    pub events: bool,
}

impl Visible {
    pub fn get(&self, k: SeriesKey) -> bool {
        match k {
            SeriesKey::Primary => self.primary,
            SeriesKey::PortfolioValue => self.portfolio_value,
            SeriesKey::Price => self.price,
            SeriesKey::Basis => self.basis,
            SeriesKey::Events => self.events,
        }
    }
    fn toggle(&mut self, k: SeriesKey) {
        let v = match k {
            SeriesKey::Primary => &mut self.primary,
            SeriesKey::PortfolioValue => &mut self.portfolio_value,
            SeriesKey::Price => &mut self.price,
            SeriesKey::Basis => &mut self.basis,
            SeriesKey::Events => &mut self.events,
        };
        *v = !*v;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum View {
    Compact = 0,
    Expanded = 1,
}

/// A navigator gesture: which part is held and the window at the press.
#[derive(Clone, Copy, Debug)]
pub struct BrushDrag {
    pub part: BrushPart,
    pub at_press: Win,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BrushPart {
    Start,
    End,
    Window,
    /// A press on the history outside the window, which brings the window there.
    Track,
}

/// Which popover/menu of the chart is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChartPopup {
    Gear(View),
    GearIncoming(View),
    GearOutgoing(View),
    Year(View),
    /// The basis-gap popover, from the header note (0) or the region label (1).
    BasisGap(View, u8),
}

/// A plot's cached geometry and the inputs it was built for.
type GeoSlot = Option<(GeoKey, Rc<Geo>)>;

pub struct ChartUi {
    pub period: Period,
    pub vis: Visible,
    pub active: Option<SeriesKey>,
    pub inc_min: f64,
    pub out_min: f64,
    pub group: bool,
    pub log_y: bool,
    pub log_x: bool,
    pub fit: bool,
    pub show_last: bool,
    /// The visible time window per view; `None` is the range chip's.
    pub brush: [Option<Win>; 2],
    pub drag: [Option<BrushDrag>; 2],
    pub brush_hover: [bool; 2],
    pub hover: [Option<usize>; 2],
    pub hover_dot: [Option<usize>; 2],
    /// Pointer in plot points while hovering, moved without re-running the view.
    pub tip_pos: Live<[f32; 2]>,
    /// The hover card's offset from the pointer per view, re-placed as the pointer moves.
    pub tip_delta: [Live<[f32; 2]>; 2],
    pub expanded: bool,
    pub inspect_col: Option<usize>,
    pub popup: Option<ChartPopup>,
    /// A press on the plot that may become a zoom, measure or pan drag.
    pub gesture: [Option<PlotGesture>; 2],
    /// A kept measurement, as two indices into `Geo::display`.
    pub measure: [Option<(usize, usize)>; 2],
    /// The next drag on the plot measures (the gear menu's "Measure").
    pub measure_armed: bool,
    /// Horizontal crosshair line with value labels on the axes.
    pub crosshair: bool,
    /// Pointer y in the plot, for the crosshair.
    pub cross_y: [Option<f32>; 2],
    /// Percent change from the start of the visible range instead of absolute values.
    pub pct: bool,
    /// Left-over wheel travel: horizontal, vertical, pinch.
    pub wheel_acc: [[f32; 3]; 2],
    cache: RefCell<Option<Cache>>,
    geo: RefCell<[GeoSlot; 2]>,
}

impl Default for ChartUi {
    fn default() -> Self {
        ChartUi {
            period: Period::Auto,
            vis: Visible {
                primary: true,
                portfolio_value: false,
                price: true,
                basis: true,
                events: true,
            },
            active: None,
            inc_min: data::DEFAULT_INCOMING_MIN,
            out_min: data::DEFAULT_OUTGOING_MIN,
            group: true,
            log_y: false,
            log_x: false,
            fit: false,
            show_last: true,
            brush: [None; 2],
            drag: [None; 2],
            brush_hover: [false; 2],
            hover: [None; 2],
            hover_dot: [None; 2],
            tip_pos: Live::new([0.0, 0.0]),
            tip_delta: [Live::new([0.0, 0.0]), Live::new([0.0, 0.0])],
            expanded: false,
            inspect_col: None,
            popup: None,
            gesture: [None; 2],
            measure: [None; 2],
            measure_armed: false,
            crosshair: true,
            cross_y: [None; 2],
            pct: false,
            wheel_acc: [[0.0; 3]; 2],
            cache: RefCell::new(None),
            geo: RefCell::new([None, None]),
        }
    }
}

impl ChartUi {
    pub fn set_period(&mut self, p: Period) {
        self.period = p;
        self.brush = [None; 2];
        self.hover = [None; 2];
        self.hover_dot = [None; 2];
        self.inspect_col = None;
        self.clear_plot_tools();
    }
    pub fn enable_log_time(&mut self, on: bool) {
        self.log_x = on;
        if on && self.period != Period::All {
            self.set_period(Period::All);
        }
    }
    pub fn set_inc_min(&mut self, v: f64) {
        self.inc_min = controls::clamp_min(v);
    }
    pub fn set_out_min(&mut self, v: f64) {
        self.out_min = controls::clamp_min(v);
    }
    pub fn mins_default(&self) -> bool {
        self.inc_min == data::DEFAULT_INCOMING_MIN && self.out_min == data::DEFAULT_OUTGOING_MIN
    }
    fn min_for(&self, flow: AFlow) -> f64 {
        match flow {
            AFlow::Incoming => self.inc_min,
            AFlow::Outgoing | AFlow::Fee => self.out_min,
            AFlow::Movement => self.inc_min.min(self.out_min),
        }
    }
}

type SnapKey = (usize, usize, usize, usize, usize, Period, crate::i18n::Lang);

/// Per-snapshot work: events, the whole history's points, the navigator's mini line, years
/// and ranges. Built once per data (or period) change, never per frame.
struct Cache {
    key: SnapKey,
    events: Vec<Event>,
    series: data::Series,
    /// `chartDisplayData` over the whole history (activity points only in the 30-day view).
    display: Rc<Vec<Pt>>,
    display_times: Rc<Vec<f64>>,
    /// The range chip's window.
    chip: Win,
    /// The navigator's downsampled balance line, (ms, BTC).
    mini: Rc<Vec<(f64, f64)>>,
    years: Vec<(i32, usize)>,
    selectable: Vec<Period>,
}

#[derive(Clone, Debug, PartialEq)]
struct GeoKey {
    snap: SnapKey,
    vis: Visible,
    mins: (u64, u64),
    group: bool,
    log_y: bool,
    log_x: bool,
    fit: bool,
    pct: bool,
    brush: Option<(u64, u64)>,
}

/// One view's plotted geometry (everything but pixel positions).
pub struct Geo {
    pub resolved: Period,
    /// All plotted points (`plottedData`).
    pub all_len: usize,
    pub last_date: Option<String>,
    /// `chartDisplayData` over the whole history; ranges index it.
    pub display: Rc<Vec<Pt>>,
    pub display_times: Rc<Vec<f64>>,
    /// The visible time window.
    pub win: Win,
    /// The range chip's window, which Reset zoom returns to.
    pub chip: Win,
    /// The display points the window shows (the carried point before it, then those inside).
    pub range: Range,
    /// The window was moved off the chip's range (zoom, pan, navigator).
    pub zoomed: bool,
    /// The display index of each plot column (the window, sampled; a carried balance at
    /// either edge repeats its point's index).
    pub plot_idx: Vec<usize>,
    /// The navigator's mini balance line over the whole history.
    pub mini: Rc<Vec<(f64, f64)>>,
    /// Lines show percent change from the first visible value.
    pub pct: bool,
    pub plot: Vec<Pt>,
    pub markers: Vec<Pt>,
    pub max_size: f64,
    pub xaxis: axes::XAxis,
    pub btc_axis: axes::YAxis,
    pub price_axis: axes::YAxis,
    pub fiat_axis: bool,
    /// Whether the history has any fiat figure at all. Without a rate every price, value and
    /// basis point is 0, and their lines, tags and axis would draw a made-up 0–1 scale.
    pub has_fiat: bool,
    pub btc_tag_domain: Option<(f64, f64)>,
    pub last_bal: Option<f64>,
    pub last_price: Option<f64>,
    pub last_value: Option<f64>,
    pub last_avg: Option<f64>,
    pub basis_trusted: bool,
    pub region: Option<f64>,
    pub counts: Counts,
    pub sums: Sums,
    pub range_change: Option<(i128, Option<f64>)>,
    pub years: Vec<(i32, usize)>,
    pub selectable: Vec<Period>,
}

impl Geo {
    /// The display index a plot column stands for.
    pub fn display_of(&self, col: usize) -> usize {
        self.plot_idx
            .get(col)
            .or(self.plot_idx.last())
            .copied()
            .unwrap_or(0)
    }
    /// The navigator shows (and the zoom works) once the history has a few points.
    pub fn navigable(&self) -> bool {
        self.display.len() > 3
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Counts {
    pub visible: usize,
    pub inc_total: usize,
    pub inc_visible: usize,
    pub out_total: usize,
    pub out_visible: usize,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Sums {
    pub received: f64,
    pub spent: f64,
    pub movement: f64,
    pub fee: f64,
}

fn snap_key(s: &Snapshot, p: Period) -> SnapKey {
    // The language is part of the key: the pipeline bakes tick and detail dates into its points.
    // The snapshot's revision: a refresh that changes values but not counts (a corrected
    // price, reprocessed journals) is a new snapshot at the same address.
    (
        s as *const Snapshot as usize,
        s.rev as usize,
        s.portfolio_series.len(),
        s.activity_txs.len(),
        s.txs.len(),
        p,
        crate::i18n::lang(),
    )
}

fn with_cache<R>(ui: &ChartUi, s: &Snapshot, f: impl FnOnce(&Cache) -> R) -> R {
    let key = snap_key(s, ui.period);
    let mut c = ui.cache.borrow_mut();
    if c.as_ref().map(|c| c.key) != Some(key) {
        let events = data::events(s);
        let series = data::build(s, &events, ui.period);
        let include_events = ui.period == Period::D30;
        let display: Vec<Pt> = series
            .points
            .iter()
            .filter(|p| !p.is_event() || include_events)
            .cloned()
            .collect();
        let display_times: Vec<f64> = display.iter().map(|p| p.sort_ms).collect();
        let chip = data::chip_win(&display, series.resolved);
        let bal: Vec<f64> = display.iter().map(|p| p.brush_bal).collect();
        let mini = data::mini_series(&display_times, &bal, brush::MINI_BUCKETS);
        let (lo, hi) = data::history_bounds(s, &events);
        let selectable = data::selectable(data::history_years(lo, hi));
        let years = data::years(&events);
        *c = Some(Cache {
            key,
            events,
            series,
            display: Rc::new(display),
            display_times: Rc::new(display_times),
            chip,
            mini: Rc::new(mini),
            years,
            selectable,
        });
    }
    f(c.as_ref().unwrap())
}

fn geo(ui: &ChartUi, s: &Snapshot, view: View) -> Rc<Geo> {
    let key = GeoKey {
        snap: snap_key(s, ui.period),
        vis: ui.vis,
        mins: (ui.inc_min.to_bits(), ui.out_min.to_bits()),
        group: ui.group,
        log_y: ui.log_y,
        log_x: ui.log_x,
        fit: ui.fit,
        pct: ui.pct,
        brush: ui.brush[view as usize].map(|w| (w.lo.to_bits(), w.hi.to_bits())),
    };
    if let Some((k, g)) = &ui.geo.borrow()[view as usize]
        && *k == key
    {
        return g.clone();
    }
    let g = Rc::new(with_cache(ui, s, |c| build_geo(ui, s, c, view)));
    ui.geo.borrow_mut()[view as usize] = Some((key, g.clone()));
    g
}

fn build_geo(ui: &ChartUi, s: &Snapshot, c: &Cache, view: View) -> Geo {
    let expanded = view == View::Expanded;
    let series = &c.series;
    let plotted = &series.points;
    let display = c.display.clone();
    let display_times = c.display_times.clone();
    let chip = c.chip;
    let (t_first, t_last) = (
        display_times.first().copied().unwrap_or(0.0),
        display_times.last().copied().unwrap_or(0.0),
    );
    let win = ui.brush[view as usize]
        .map(|w| data::clamp_win(w, t_first, t_last))
        .unwrap_or(chip);
    let span = data::window_span(&display_times, win);
    let range = Range {
        start: span.carry.unwrap_or(span.first),
        end: span
            .end
            .saturating_sub(1)
            .max(span.carry.unwrap_or(span.first)),
    };
    // The points inside the window (the carried one before it only feeds the line's start).
    let selected: &[Pt] = &display[span.first..span.end];
    let dates: std::collections::HashSet<&str> = selected.iter().map(|p| p.date.as_str()).collect();
    // `activityMarkerView`, over the activity in the window (a merged dot or a 30-day line
    // stop carries its anchor day's date).
    let activity: Vec<&Pt> = plotted
        .iter()
        .filter(|p| p.is_event() && dates.contains(p.date.as_str()))
        .collect();
    let visible: Vec<&Pt> = activity
        .iter()
        .copied()
        .filter(|p| {
            let e = p.ev.as_ref().unwrap();
            ui.vis.events
                && e.flow != AFlow::Fee
                && (if e.size > 0.0 { e.size } else { e.activity_btc }) >= ui.min_for(e.flow)
        })
        .collect();
    let sel_markers: Vec<Pt> = visible.iter().map(|p| (*p).clone()).collect();
    let cols = plot_columns(
        &display,
        &display_times,
        win,
        data::sample_max(series.resolved, expanded),
    );
    let plot_idx: Vec<usize> = cols.iter().map(|c| c.0).collect();
    let pos = |v: Option<f64>| v.filter(|v| v.is_finite() && *v > 0.0);
    // Over the whole history, so zooming into a flat stretch doesn't flip the axis.
    let has_fiat = display.iter().any(|p| {
        [p.l_price, p.l_val, p.l_avg]
            .into_iter()
            .any(|v| pos(v).is_some())
    });
    let mut plot: Vec<Pt> = if ui.log_y {
        cols.iter()
            .map(|&(i, at)| {
                let mut p = column_point(&display[i], i, at, &display_times);
                p.l_bal = pos(p.l_bal);
                p.l_val = pos(p.l_val);
                p.l_price = pos(p.l_price);
                p.l_avg = pos(p.l_avg);
                p
            })
            .collect()
    } else {
        cols.iter()
            .map(|&(i, at)| column_point(&display[i], i, at, &display_times))
            .collect()
    };
    let drawable: Vec<Pt> = if ui.log_y {
        sel_markers
            .into_iter()
            .filter(|p| p.ev.as_ref().unwrap().marker_bal > 0.0)
            .collect()
    } else {
        sel_markers
    };
    let target = if ui.group {
        if expanded { 56 } else { 32 }
    } else {
        selected.len()
    };
    let mut markers = data::bucket(drawable, selected, target);
    let pct = ui.pct && !ui.log_y;
    if pct {
        rebase_percent(&mut plot, &mut markers);
    }
    // Axes.
    let mut btc_values: Vec<Option<f64>> = Vec::new();
    if ui.vis.primary {
        btc_values.extend(plot.iter().map(|p| p.l_bal));
    }
    if ui.vis.events {
        btc_values.extend(markers.iter().map(|p| p.ev.as_ref().map(|e| e.marker_bal)));
    }
    let mut price_values: Vec<Option<f64>> = Vec::new();
    if ui.vis.portfolio_value {
        price_values.extend(plot.iter().map(|p| p.l_val));
    }
    if ui.vis.price {
        price_values.extend(plot.iter().map(|p| p.l_price));
    }
    if ui.vis.basis {
        price_values.extend(plot.iter().map(|p| p.l_avg));
    }
    let (btc_axis, price_axis) = if pct {
        // One percent scale for every line, so their slopes compare.
        let all: Vec<Option<f64>> = btc_values
            .iter()
            .chain(price_values.iter())
            .copied()
            .collect();
        let a = axes::YAxis::percent(&all, if expanded { 6 } else { 5 });
        (a.clone(), a)
    } else {
        (
            axes::YAxis::build(&btc_values, ui.log_y, ui.fit, expanded),
            axes::YAxis::build(&price_values, ui.log_y, ui.fit, expanded),
        )
    };
    let btc_tag_domain = btc_axis.fit.or_else(|| axes::auto_fit_domain(&btc_values));
    let xaxis = match (ui.log_x, axes::power_law_domain(&plot)) {
        (true, Some((lo, hi))) => axes::XAxis::PowerLaw { lo, hi },
        _ => axes::XAxis::Time {
            lo: win.lo,
            hi: win.hi,
        },
    };
    // The change in range runs from the balance at the window's start (the carried one).
    let with_carry: &[Pt] = &display[range.start..span.end.max(range.start + 1).min(display.len())];
    let balance_points: Vec<&Pt> = with_carry.iter().filter(|p| !p.is_event()).collect();
    let last = |pick: &dyn Fn(&Pt) -> Option<f64>| {
        plot.iter()
            .rev()
            .find_map(|p| pick(p).filter(|v| v.is_finite()))
    };
    let completeness = &s.fiat.completeness;
    // Summary over every activity point in the period.
    let mut sums = Sums::default();
    for p in &activity {
        let e = p.ev.as_ref().unwrap();
        match e.flow {
            AFlow::Incoming => sums.received += e.activity_btc,
            AFlow::Outgoing => sums.spent += e.activity_btc,
            AFlow::Movement => sums.movement += e.activity_btc,
            AFlow::Fee => {}
        }
        if e.fee > 0.0 {
            sums.fee += e.fee;
        } else if e.flow == AFlow::Fee {
            sums.fee += e.activity_btc;
        }
    }
    let flow_is = |p: &&Pt, inc: bool| {
        let f = p.ev.as_ref().unwrap().flow;
        if inc {
            f == AFlow::Incoming
        } else {
            matches!(f, AFlow::Outgoing | AFlow::Fee)
        }
    };
    let counts = Counts {
        visible: visible.len(),
        inc_total: activity.iter().filter(|p| flow_is(p, true)).count(),
        inc_visible: visible.iter().filter(|p| flow_is(p, true)).count(),
        out_total: activity.iter().filter(|p| flow_is(p, false)).count(),
        out_visible: visible.iter().filter(|p| flow_is(p, false)).count(),
    };
    let rc_src: Vec<&Pt> = if balance_points.len() >= 2 {
        balance_points
    } else {
        with_carry.iter().collect()
    };
    let range_change = data::range_change(&rc_src);
    // `basisIncompleteFromMs`, shown when the first plotted day reaches past it.
    let from = data::basis_from_ms(s);
    let region = from.filter(|f| {
        ui.vis.basis
            && plot
                .iter()
                .any(|q| *f == f64::NEG_INFINITY || data::day_ms(&q.date) + data::DAY_MS > *f)
    });
    let max_size = markers
        .iter()
        .map(|m| m.ev.as_ref().unwrap().size)
        .fold(0.0, f64::max);
    Geo {
        resolved: series.resolved,
        all_len: plotted.len(),
        last_date: plotted.last().map(|p| p.date.clone()),
        zoomed: display.len() > 3 && !win.same(&chip),
        display,
        display_times,
        win,
        chip,
        range,
        plot_idx,
        mini: c.mini.clone(),
        pct,
        markers,
        max_size,
        xaxis,
        btc_axis,
        price_axis,
        fiat_axis: !pct && has_fiat,
        has_fiat,
        btc_tag_domain,
        last_bal: last(&|p| p.l_bal),
        last_price: last(&|p| p.l_price),
        last_value: last(&|p| p.l_val),
        last_avg: last(&|p| p.l_avg),
        basis_trusted: completeness.cost_basis_complete,
        region,
        counts,
        sums,
        range_change,
        plot,
        years: c.years.clone(),
        selectable: c.selectable.clone(),
    }
}

/// The plot's columns for a window: (display index, moment drawn at). The balance before the
/// window starts the line at its left edge, the points inside are sampled to `max`, and where
/// more history follows, the last balance runs on to the right edge (a step line holds its
/// value until the next point).
pub(super) fn plot_columns(display: &[Pt], times: &[f64], w: Win, max: usize) -> Vec<(usize, f64)> {
    let span = data::window_span(times, w);
    let mut out: Vec<(usize, f64)> = Vec::new();
    if let Some(c) = span.carry {
        out.push((c, w.lo));
    }
    if span.end > span.first {
        out.extend(
            data::sample_indices(
                display,
                Range {
                    start: span.first,
                    end: span.end - 1,
                },
                max,
            )
            .into_iter()
            .map(|i| (i, times[i])),
        );
    }
    if let Some(&(last, at)) = out.last()
        && span.more
        && at < w.hi - 1.0
    {
        out.push((last, w.hi));
    }
    out
}

/// A plot column's point: the display point itself, or a copy standing at a window edge for a
/// carried balance. The copy holds the balance and the basis; price and value are left out
/// (they are not known there).
fn column_point(p: &Pt, i: usize, at: f64, times: &[f64]) -> Pt {
    let mut q = p.clone();
    if (times[i] - at).abs() >= 1.0 {
        q.sort_ms = at;
        q.power_days = data::power_days(at);
        q.l_price = None;
        q.l_val = None;
        q.ev = None;
    }
    q
}

/// Rebases each line to percent change from its first positive visible value (the dots ride
/// the balance line, so they share its base).
fn rebase_percent(plot: &mut [Pt], markers: &mut [Pt]) {
    let base = |pick: &dyn Fn(&Pt) -> Option<f64>| {
        plot.iter()
            .find_map(|p| pick(p).filter(|v| v.is_finite() && *v > 0.0))
    };
    let (b_bal, b_val, b_price, b_avg) = (
        base(&|p| p.l_bal),
        base(&|p| p.l_val),
        base(&|p| p.l_price),
        base(&|p| p.l_avg),
    );
    let to = |v: Option<f64>, b: Option<f64>| match (v, b) {
        (Some(v), Some(b)) if v.is_finite() => Some((v / b - 1.0) * 100.0),
        _ => None,
    };
    for p in plot.iter_mut() {
        p.l_bal = to(p.l_bal, b_bal);
        p.l_val = to(p.l_val, b_val);
        p.l_price = to(p.l_price, b_price);
        p.l_avg = to(p.l_avg, b_avg);
    }
    for m in markers.iter_mut() {
        if let Some(e) = m.ev.as_mut() {
            e.marker_bal = to(Some(e.marker_bal), b_bal).unwrap_or(f64::NAN);
        }
    }
}

pub fn flow_color(tk: &Tk, f: AFlow) -> Color {
    match f {
        AFlow::Incoming => tk.flow_in,
        AFlow::Outgoing => tk.flow_out,
        AFlow::Movement => tk.flow_move,
        AFlow::Fee => tk.flow_fee,
    }
}

/// `transactionSetHref`: the Transactions screen filtered to a merged dot's events.
pub fn transaction_set_href(app: &App, ids: &[String]) -> String {
    let period = match app.chart.period {
        Period::Auto => String::new(),
        p => format!("period={}&", period_param(p)),
    };
    format!(
        "/transactions?{period}txids={}#transactions-table",
        ids.join(",")
    )
}

fn period_param(p: Period) -> String {
    match p {
        Period::D30 => "30days".into(),
        Period::Ytd => "ytd".into(),
        Period::Y1 => "1year".into(),
        Period::Y4 => "4years".into(),
        Period::Y10 => "10years".into(),
        Period::All => "all".into(),
        Period::Auto => "auto".into(),
        Period::Year(y) => y.to_string(),
    }
}

pub fn on_reply(_app: &mut App, _tag: &str, _r: &Reply) {}

/// Escape closes the innermost chart surface: a menu, a drag in progress, a measurement, the
/// dialog. The full-window chart also takes the plot's zoom and pan keys wherever focus is.
pub fn on_key(app: &mut App, ev: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if !matches!(ev.key, rune::Key::Escape) {
        return app.chart.expanded
            && app.chart.popup.is_none()
            && interact::key(app, View::Expanded, ev, tx);
    }
    let c = &mut app.chart;
    if c.popup.take().is_some() {
        return true;
    }
    if c.gesture.iter().any(|g| g.is_some()) {
        // The drag's own release (cancelled) restores the range.
        return true;
    }
    if c.measure_armed || c.measure.iter().any(|m| m.is_some()) {
        c.measure_armed = false;
        c.measure = [None; 2];
        return true;
    }
    if c.expanded {
        c.expanded = false;
        c.hover[1] = None;
        return true;
    }
    false
}

/// The compact card on the Overview; the expanded dialog mounts into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, s: &Snapshot, layer: &mut Vec<El>) -> El {
    let has_data =
        !s.portfolio_series.is_empty() || with_cache(&app.chart, s, |c| !c.events.is_empty());
    let card = card(app, cx, tk, s, View::Compact, has_data, layer);
    if app.chart.expanded {
        let mut inner: Vec<El> = Vec::new();
        let dialog = expanded::dialog(app, cx, tk, s, has_data, &mut inner);
        layer.push(dialog);
        layer.extend(inner);
    }
    card
}

pub(crate) fn card(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    s: &Snapshot,
    view: View,
    has_data: bool,
    layer: &mut Vec<El>,
) -> El {
    let expanded = view == View::Expanded;
    let card = if expanded {
        col()
            .key("btc-activity-x")
            .w_full()
            .h_full()
            .fill(tk.card)
            .padding(tk.r(0.75))
            .gap(tk.r(0.75))
    } else {
        // The Overview widget frame (`overview::grid::frame`) draws the card around it.
        col()
            .key("btc-activity")
            .w_full()
            .min_w(0.0)
            .gap(tk.r(0.75))
    };
    // A range change lands at once, as the web's chart does: the header's figures re-wrap and
    // change its height, so eased, the legend and the plot under it slid with every switch.
    // Colours (hover, active chips) keep their springs; the legend opts back in for its fade.
    let mut card = card.instant_changes();
    if !has_data {
        return card
            .child(header(app, tk, s, None, view, false))
            .child(empty(tk, view));
    }
    let g = geo(&app.chart, s, view);
    card = card
        .child(header(app, tk, s, Some(&g), view, true))
        .child(legend(app, tk, view));
    let w_key = if expanded {
        "chart-plot-x"
    } else {
        "chart-plot"
    };
    // The web's block is `h-[420px]` and holds the plot (flex-1), the range navigator and the
    // toolbar (`mt-2` + 1 pt rule + `pt-2` + an 18 pt chip row = 33.4 pt at the 0.9 scale).
    let toolbar_h = tk.r(0.5) + 1.0 + tk.r(0.5) + tk.r(1.25);
    let brush_h = if g.navigable() { brush::H } else { 0.0 };
    let plot_h = if expanded {
        let win = cx
            .layout_size("btc-activity-x")
            .map(|s| s[1])
            .unwrap_or(800.0);
        // 250 pt of chrome around the plot, with a 60 pt brush; the navigator is lower.
        (win - 250.0 + 60.0 - brush::H).max(260.0)
    } else {
        crate::overview::chart_block_h(app, tk) - toolbar_h - brush_h
    };
    let w = cx.layout_size(w_key).map(|s| s[0]).unwrap_or(900.0);
    if g.all_len == 0 {
        return card.child(
            row()
                .key(w_key)
                .w_full()
                .h(plot_h)
                .center()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .child(
                    text(crate::i18n::t("overview:balanceChart.noHistory"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let brush_on = g.navigable();
    let frame = plot::Frame::new(w, plot_h, expanded, g.region.is_some(), brush_on);
    let hide = app.prefs.hide_sensitive();
    let cur = s.fiat_currency();
    let ctx = plot::Ctx {
        app,
        tk,
        geo: &g,
        view,
        frame,
        hide,
        vis: plot::drawn_series(app.chart.vis, g.has_fiat),
        active: app.chart.active,
        cur: &cur,
    };
    let hover = app.chart.hover[view as usize];
    // The hover card lives in the window layer, beside the pointer: what it needs measured.
    let v = view as usize;
    let tip_env = if app.chart.hover[v].is_some() || app.chart.hover_dot[v].is_some() {
        tooltip::TipEnv {
            card: cx.layout_size(&format!("chart-tip-card-{v}")),
            list_h: cx.layout_size(&format!("chart-tip-list-{v}")).map(|s| s[1]),
            origin: cx.window_layout_with(w_key, |r| [r[0], r[1]]),
            window: cx
                .layout_size("sh-root")
                .filter(|s| s[0] > 0.0 && s[1] > 0.0)
                .unwrap_or([app.shell.window_w, app.shell.window_h]),
        }
    } else {
        tooltip::TipEnv::default()
    };
    let (tip_anchor, tip_card) = match tooltip::tooltip(&ctx, &tip_env) {
        Some(t) => (Some((t.anchor, t.plan)), Some(t.card)),
        None => (None, None),
    };
    let plot_el = plot::plot(&ctx, w_key, hover, tip_anchor);
    if let Some(card) = tip_card {
        layer.push(card);
    }
    if let Some(ChartPopup::BasisGap(v, which)) = app.chart.popup
        && v == view
    {
        tooltip::basis_gap_popover(
            tk,
            s,
            &format!("basis-label-{}-{which}", view as usize),
            layer,
        );
    }
    let mut block = col().w_full().min_w(0.0).child(plot_el);
    if brush_on {
        block = block.child(brush::brush(app, cx, tk, &g, view));
    }
    let toolbar = toolbar::toolbar(app, tk, &g, view, layer);
    let body = if expanded {
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(
                row()
                    .w_full()
                    .grow(1.0)
                    .min_h(0.0)
                    .child(rotated(
                        tk,
                        &t("overview:treasury.series.bitcoinBalance"),
                        false,
                    ))
                    .child(block.grow(1.0))
                    .child(rotated(
                        tk,
                        &tr("overview:treasury.btcPriceAxis", &[("currency", &cur)]),
                        true,
                    )),
            )
            .child(toolbar)
    } else {
        block.child(toolbar)
    };
    card.child(body)
}

/// A rotated axis title (the expanded dialog's 18 / 20 pt gutters).
fn rotated(tk: &Tk, label: &str, right: bool) -> El {
    let st = tk.t2xs(600.0);
    let w = label.chars().count() as f32 * st.size * 0.6 + 8.0;
    let angle = if right {
        std::f32::consts::FRAC_PI_2
    } else {
        -std::f32::consts::FRAC_PI_2
    };
    col()
        .w(if right { 20.0 } else { 18.0 })
        .h_full()
        .center()
        .child(
            row()
                .w(w)
                .h(14.0)
                .justify_center()
                .rotate(angle)
                .inert()
                .child(text(label.to_owned()).font(st).color(tk.muted_fg)),
        )
}

fn empty(tk: &Tk, view: View) -> El {
    // The full-window chart mounts its own copy over the card's: keys stay unique per view
    // (the compact card keeps the script-stable names).
    let sfx = if view == View::Expanded { "-x" } else { "" };
    col()
        .key(format!("chart-empty{sfx}"))
        .w_full()
        .h(388.0)
        .center()
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_dash(4.0, 3.0)
        .child(ic(&I::CHART_LINE, tk.r(2.0), tk.muted_fg.alpha(0.6)))
        .child(
            text(t("overview:treasury.empty.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
        .child(
            text(t("overview:treasury.empty.body"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .max_w(tk.r(24.0))
                .align(rune::Align::Center),
        )
        .child(
            row()
                .gap(tk.r(0.5))
                .child(
                    primary_button(
                        tk,
                        &format!("chart-add-connection{sfx}"),
                        Some(&I::PLUS),
                        &t("overview:welcome.addConnection"),
                    )
                    .on_press(|s: &mut App| crate::overview::open_add_connection(s)),
                )
                .child(
                    outline_button(
                        tk,
                        &format!("chart-refresh{sfx}"),
                        Some(&I::REFRESH_CW),
                        &t("overview:welcome.refresh"),
                    )
                    .on_press(|s: &mut App| s.refresh_book(false)),
                ),
        )
}

fn header(app: &App, tk: &Tk, s: &Snapshot, g: Option<&Geo>, view: View, has_data: bool) -> El {
    let expanded = view == View::Expanded;
    let hide = app.prefs.hide_sensitive();
    let as_of = g
        .and_then(|g| g.last_date.as_deref())
        .and_then(format::Day::parse)
        .map(|d| tr("overview:treasury.asOf", &[("date", &d.long_ui())]));
    let mut title = row()
        .gap(tk.r(if expanded { 0.5 } else { 0.375 }))
        .center_y()
        .min_h(if expanded { 0.0 } else { tk.r(1.25) });
    // On the Overview the name is the tile's caption (`overview::grid::frame::caption`).
    title = if expanded {
        title.child(
            text(t("overview:treasury.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
    } else {
        title
            .child(crate::ui::ic(&I::CHART_LINE, tk.r(0.875), tk.muted_fg))
            .child(
                text(t("overview:treasury.title"))
                    .font(tk.txs(600.0))
                    .color(tk.muted_fg)
                    .role("heading"),
            )
    };
    if has_data {
        title = title.child(
            text(as_of.unwrap_or_else(|| t("overview:treasury.currentSnapshot")))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
        );
    }
    if expanded {
        let m = s.market_rate.as_ref();
        if let Some(when) = m
            .and_then(|m| m.fetched_at.clone().or_else(|| m.timestamp.clone()))
            .and_then(|w| crate::overview::relative_time(&w))
        {
            title = title.child(
                text(tr("overview:treasury.priced", &[("time", &when)]))
                    .font(tk.t2xs(400.0))
                    .color(tk.muted_fg)
                    .help(model::market_rate_detail(s)),
            );
        }
    }
    let mut left = col().min_w(220.0).grow(1.0).child(title);
    if let (true, Some(g)) = (has_data, g) {
        let good = tk.good.solid;
        let bad = tk.kb_accent;
        let small = tk.txs(400.0);
        let strong = tk.txs(600.0).features("'tnum' 1");
        let item = |label: &str, value: El| {
            row()
                .gap(4.0)
                .center_y()
                .child(text(label.to_owned()).font(small).color(tk.muted_fg))
                .child(value)
        };
        // `{label}{" "}<span class="font-semibold …">{value}</span>`: one text run, so the label
        // and its value wrap and read as one. Hidden amounts keep the value in its own blurred
        // element (a rich-text range can't blur).
        let run = |label: &str, value: String, color: Color| -> El {
            if hide {
                return item(label, crate::ui::sensitive(value, strong, color, true));
            }
            let content = format!("{label} {value}");
            let from = label.len() + 1;
            rune::rich(
                content.clone(),
                vec![
                    rune::Span::new(from, content.len())
                        .weight(600.0)
                        .color(color),
                ],
            )
            .font(small.features("'tnum' 1"))
            .color(tk.muted_fg)
        };
        let net_btc = g.sums.received - g.sums.spent - g.sums.fee;
        // The range's figures swap at once with the range, as the web's text does: eased, the
        // row's items slide as each figure changes width.
        let mut sum = row()
            .mt(tk.r(0.375))
            .gap(tk.r(0.625))
            .flex_wrap()
            .center_y()
            .instant_changes();
        sum = sum.child(run(
            &t("overview:treasury.net"),
            format::btc(net_btc, 4, true),
            if net_btc >= 0.0 { good } else { bad },
        ));
        if let Some((delta, pct)) = g.range_change {
            let color = if delta > 0 {
                good
            } else if delta < 0 {
                bad
            } else {
                tk.fg
            };
            let mut v = format::signed_msat_btc(delta);
            if let Some(p) = pct {
                let pct = format!("{}{:.2}", if p >= 0.0 { "+" } else { "\u{2212}" }, p.abs());
                v.push_str(&format!(
                    " ({})",
                    tr("overview:treasury.rangeChangePercent", &[("percent", &pct)])
                ));
            }
            let mut e = run(&t("overview:treasury.rangeChange"), v, color);
            if !hide {
                e = e.help(format::signed_msat_sats(delta));
            }
            sum = sum.child(e);
        }
        sum = sum.child(
            text(tc("overview:treasury.events", g.counts.visible, &[]))
                .font(small)
                .color(tk.muted_fg),
        );
        if expanded {
            sum = sum.child(run(
                &t("overview:treasury.in"),
                format::btc(g.sums.received, 4, false),
                tk.fg,
            ));
            sum = sum.child(run(
                &t("overview:treasury.out"),
                format::btc(g.sums.spent, 4, false),
                tk.fg,
            ));
            if g.sums.movement > 0.0 {
                sum = sum.child(run(
                    &t("overview:treasury.swap"),
                    format::btc(g.sums.movement, 4, false),
                    tk.fg,
                ));
            }
        }
        let detail = model::completeness_issue(s);
        let detail_text = crate::overview::completeness_text(s);
        if let Some(d) = &detail {
            let (copy, hint) = detail_text.clone().unwrap_or_default();
            let amber = tk.warn.solid;
            if d.href.is_some() {
                let key = format!("basis-label-{}-0", view as usize);
                let amount = s
                    .fiat
                    .completeness
                    .basis_uncovered_msat
                    .is_some_and(|m| m > 0.0);
                sum = sum.child(
                    row()
                        .key(key)
                        .role("button")
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(move |a: &mut App| {
                            a.chart.popup = Some(ChartPopup::BasisGap(view, 0))
                        })
                        .child(
                            col()
                                .child(crate::ui::sensitive(
                                    copy.clone(),
                                    tk.txs(600.0),
                                    amber,
                                    hide && amount,
                                ))
                                .child(dotted_underline(
                                    amber,
                                    copy.chars().count() as f32 * tk.txs(600.0).size * 0.55,
                                )),
                        ),
                );
            } else {
                sum = sum.child(text(copy).font(tk.txs(600.0)).color(amber).help(hint));
            }
        }
        let f = &s.fiat;
        if f.eur_cost_basis != 0.0 && detail.is_none() && !f.completeness.market_rate_missing {
            let gain = f.eur_balance - f.eur_cost_basis;
            let pct = gain / f.eur_cost_basis.abs() * 100.0;
            let pct_text = format!("{:.2}", pct.abs());
            sum = sum.child(crate::ui::sensitive(
                format!(
                    "{}{}",
                    if gain >= 0.0 { "+ " } else { "- " },
                    tr(
                        "overview:treasury.unrealized",
                        &[("percent", &pct_text), ("currency", &s.fiat_currency())]
                    )
                ),
                tk.txs(600.0),
                if gain >= 0.0 { good } else { bad },
                hide,
            ));
        }
        left = left.child(sum);
    }
    let right = if expanded {
        row()
            .gap(tk.r(0.5))
            .items_start()
            .justify_end()
            .min_w(0.0)
            .children(g.map(|g| expanded::inspector(app, tk, s, g)))
            .child(
                crate::ui::icon_button(
                    tk,
                    "chart-close",
                    &I::X,
                    &t("overview:treasury.closeChart"),
                )
                .on_press(|a: &mut App| {
                    a.chart.expanded = false;
                    a.chart.hover[1] = None;
                }),
            )
    } else {
        crate::overview::chart_actions(
            app,
            tk,
            crate::ui::icon_button(
                tk,
                "chart-expand",
                &I::MAXIMIZE_2,
                &t("overview:treasury.expandChart"),
            )
            .on_press(|a: &mut App| {
                a.chart.expanded = true;
                a.chart.inspect_col = None;
            }),
        )
    };
    row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .justify_between()
        .child(left)
        .child(right)
}

/// A dotted underline (`underline decoration-dotted`): the renderer has no text decorations
/// with dash styles, so draw the dots.
fn dotted_underline(color: Color, w: f32) -> El {
    let mut s = Shape::new();
    let mut x = 0.0;
    while x < w {
        s = s.line([x, 1.0], [x + 1.0, 1.0], 1.0, rune::LineCap::Butt, color);
        x += 3.0;
    }
    canvas(s).size(w, 2.0).mt(1.0)
}

fn legend(app: &App, tk: &Tk, view: View) -> El {
    // Keys stay script-stable on the compact card (`legend-Price`); the expanded dialog's copy
    // takes a `-x` suffix so keys stay unique while both are mounted.
    let sfx = if view == View::Expanded { "-x" } else { "" };
    let ui = &app.chart;
    let items = [
        (
            SeriesKey::Primary,
            t("overview:treasury.series.bitcoinBalance"),
            tk.chart_btc,
            false,
        ),
        (
            SeriesKey::PortfolioValue,
            t("overview:treasury.series.portfolioValue"),
            tk.chart_value,
            false,
        ),
        (
            SeriesKey::Events,
            t("overview:treasury.series.activity"),
            tk.flow_in,
            false,
        ),
        (
            SeriesKey::Basis,
            t("overview:treasury.series.avgBasis"),
            tk.chart_basis,
            true,
        ),
        (
            SeriesKey::Price,
            t("overview:treasury.series.btcPrice"),
            tk.chart_price,
            true,
        ),
    ];
    let mut r = row()
        .key(format!("chart-legend{sfx}"))
        .gap(tk.r(0.5))
        .flex_wrap()
        .center_y();
    for (k, label, color, dashed) in items {
        let on = ui.vis.get(k);
        let swatch: El = if k == SeriesKey::Events {
            // ActivityLegendSwatch: `flex w-11 gap-0.5` holding three `size-1.5` dots.
            row().w(tk.r(2.75)).gap(tk.r(0.125)).center_y().children(
                [tk.flow_in, tk.flow_out, tk.flow_move].map(|c| crate::ui::dot(tk.r(0.375), c)),
            )
        } else if dashed {
            let mut s = Shape::new();
            for d in plot::dashed(&[[0.0, 1.0], [20.0, 1.0]], 3.0, 2.0) {
                s = s.stroke_with(d, false, 1.0, rune::LineCap::Butt, color);
            }
            canvas(s).size(20.0, 2.0)
        } else {
            el().size(20.0, 2.0).radius(1.0).fill(color)
        };
        let mut opacity = if on { 1.0 } else { 0.3 };
        if ui.active.is_some() && ui.active != Some(k) {
            opacity = 0.4;
        }
        r = r.child(
            row()
                .key(format!("legend-{k:?}{sfx}"))
                .gap(tk.r(0.375))
                .px(tk.r(0.375))
                .py(tk.r(0.125))
                .center_y()
                .radius(tk.r(0.375))
                .hover(tk.muted.alpha(0.45))
                .opacity(opacity)
                // `transition-[background-color,opacity] duration-200`: only the fade eases; its
                // place lands with the card's.
                .animated_changes()
                .motion(rune::Spring::NONE)
                .opacity_motion(rune::Curve::tween(0.2, rune::Easing::EASE))
                .role("button")
                .checked(on)
                .label(tr(
                    "overview:treasury.seriesToggle",
                    &[
                        (
                            "action",
                            &t(if on {
                                "common:actions.hide"
                            } else {
                                "common:actions.show"
                            }),
                        ),
                        ("label", &label),
                    ],
                ))
                .focusable(true)
                .on_press(move |s: &mut App| s.chart.vis.toggle(k))
                .on_hover_change(move |s: &mut App, h, _tx: &mut Tx| {
                    if h {
                        s.chart.active = Some(k);
                    } else if s.chart.active == Some(k) {
                        s.chart.active = None;
                    }
                })
                .child(swatch)
                .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
    }
    r
}

/// Parity scenes: hover the compact plot at a point in its own coordinates, as a pointer at
/// that spot would (the nearest column; the tooltip follows the point). Uses the 1440 × 900
/// window's plot width.
pub fn scene_hover_at(a: &mut App, x: f32, y: f32) {
    let Some(s) = a.snapshot() else { return };
    let g = geo(&a.chart, s, View::Compact);
    let frame = plot::Frame::new(
        782.0,
        420.0 - (7.2 + 1.0 + 7.2 + 18.0) - brush::H,
        false,
        g.region.is_some(),
        g.navigable(),
    );
    let cols: Vec<f32> = g
        .plot
        .iter()
        .map(|p| g.xaxis.x_pt(p, frame.left as f64, frame.right as f64) as f32)
        .collect();
    let i = cols
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (*a - x).abs().total_cmp(&(*b - x).abs()))
        .map(|(i, _)| i);
    a.chart.hover[0] = i;
    a.chart.tip_pos.set([x, y]);
}

/// Scene helper: hover either view's plot at a point in its own coordinates, with the
/// crosshair at that height (the 1440 × 900 window's plot sizes).
fn scene_hover_view(a: &mut App, view: View, x: f32, y: f32) {
    let Some(s) = a.snapshot() else { return };
    let g = geo(&a.chart, s, view);
    let frame = match view {
        View::Compact => plot::Frame::new(
            757.2,
            420.0 - (7.2 + 1.0 + 7.2 + 18.0) - brush::H,
            false,
            g.region.is_some(),
            g.navigable(),
        ),
        View::Expanded => plot::Frame::new(1378.0, 610.0, true, g.region.is_some(), g.navigable()),
    };
    let cols: Vec<f32> = g
        .plot
        .iter()
        .map(|p| g.xaxis.x_pt(p, frame.left as f64, frame.right as f64) as f32)
        .collect();
    let v = view as usize;
    a.chart.hover[v] = Some(interact::nearest(&cols, x));
    a.chart.cross_y[v] = Some(y.min(frame.bottom));
    a.chart.tip_pos.set([x, y]);
    if view == View::Expanded {
        a.chart.inspect_col = a.chart.hover[v];
    }
}

/// SYNTHETIC: a book with no balance history and no activity (the chart's empty state).
fn scene_no_data(a: &mut App) {
    if let crate::app::Load::Ready(s) = &mut a.overview {
        s.portfolio_series.clear();
        s.activity_txs.clear();
        s.txs.clear();
    }
}

/// Scene helper: the window at the given fractions of the range chip's window.
fn scene_win(a: &mut App, view: View, from: f64, to: f64) -> Option<Win> {
    let s = a.snapshot()?;
    let g = geo(&a.chart, s, view);
    let c = g.chip;
    Some(Win {
        lo: c.lo + c.span() * from,
        hi: c.lo + c.span() * to,
    })
}

/// Scene helper: the display points nearest the given fractions of the chip's window.
fn scene_zoom(a: &mut App, view: View, from: f64, to: f64) -> Option<(usize, usize)> {
    let w = scene_win(a, view, from, to)?;
    let s = a.snapshot()?;
    let g = geo(&a.chart, s, view);
    Some((
        data::nearest_time(&g.display_times, w.lo),
        data::nearest_time(&g.display_times, w.hi),
    ))
}

/// The chart's own scenes for the states this module adds (zoom, selection, measure, crosshair,
/// percent scale). The older chart scenes live in `main.rs`.
pub const SCENES: &[crate::app::Scene] = &[
    ("chart-zoomed", |a| {
        a.chart.brush[0] = scene_win(a, View::Compact, 0.45, 0.8)
    }),
    ("chart-selecting", |a| {
        let full = scene_win(a, View::Compact, 0.0, 1.0);
        if let (Some((s, e)), Some(full)) = (scene_zoom(a, View::Compact, 0.3, 0.62), full) {
            a.chart.gesture[0] = Some(PlotGesture {
                kind: GestureKind::Zoom,
                from: s,
                to: e,
                moved: true,
                brush_at_press: None,
                range_at_press: full,
            });
        }
    }),
    ("chart-measure", |a| {
        a.chart.measure[0] = scene_zoom(a, View::Compact, 0.35, 0.85);
    }),
    ("chart-measure-armed", |a| a.chart.measure_armed = true),
    ("chart-crosshair", |a| {
        scene_hover_at(a, 430.0, 150.0);
        a.chart.cross_y[0] = Some(150.0);
    }),
    ("chart-pct", |a| a.chart.set_pct(true)),
    ("chart-pct-zoomed", |a| {
        a.chart.set_pct(true);
        a.chart.brush[0] = scene_win(a, View::Compact, 0.45, 0.8);
    }),
    // The hover card over the lowest plot rows, where it overlaps the brush.
    ("chart-hover-low", |a| {
        scene_hover_view(a, View::Compact, 300.0, 286.0)
    }),
    ("chart-expanded-hover-low", |a| {
        a.chart.expanded = true;
        scene_hover_view(a, View::Expanded, 640.0, 560.0);
    }),
    // The navigator hovered, and mid-drag on the end limiter after a zoom.
    ("chart-brush-hover", |a| a.chart.brush_hover[0] = true),
    ("chart-brush-drag", |a| {
        if let Some(w) = scene_win(a, View::Compact, 0.3, 0.7) {
            a.chart.brush[0] = Some(w);
            a.chart.drag[0] = Some(BrushDrag {
                part: BrushPart::End,
                at_press: w,
            });
        }
    }),
    // The navigator at the other ranges, and after a manual window (no chip active).
    ("chart-nav-30d", |a| a.chart.set_period(Period::D30)),
    ("chart-nav-ytd", |a| a.chart.set_period(Period::Ytd)),
    ("chart-nav-all", |a| a.chart.set_period(Period::All)),
    ("chart-nav-custom", |a| {
        a.chart.set_period(Period::All);
        a.chart.brush[0] = scene_win(a, View::Compact, 0.35, 0.62);
    }),
    ("chart-nav-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.chart.set_period(Period::All);
        a.chart.brush[0] = scene_win(a, View::Compact, 0.35, 0.62);
        a.chart.brush_hover[0] = true;
    }),
    ("chart-nav-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        a.chart.set_period(Period::Y1);
        a.chart.brush_hover[0] = true;
    }),
    ("chart-nav-expanded-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.chart.set_period(Period::D30);
        a.chart.expanded = true;
    }),
    ("chart-nav-expanded", |a| {
        a.chart.set_period(Period::Y4);
        a.chart.expanded = true;
    }),
    ("chart-empty", scene_no_data),
    ("chart-expanded-empty", |a| {
        scene_no_data(a);
        a.chart.expanded = true;
    }),
    ("chart-expanded-zoomed", |a| {
        a.chart.expanded = true;
        a.chart.brush[1] = scene_win(a, View::Expanded, 0.45, 0.8);
    }),
];
