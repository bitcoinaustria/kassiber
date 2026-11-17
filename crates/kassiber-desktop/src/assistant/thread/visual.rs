//! Assistant visuals (components/ai/visuals/**): a `ui.visuals.render` result drawn as a card —
//! icon, title and scope, the table / CSV / expand / open-in-app actions, the source's body,
//! caveats, follow-up questions and the "drawn by Kassiber" note — plus the skeleton shown while
//! one is being drawn. Bodies: overview, balance history, holdings, tax years, cash flow,
//! transactions, transaction flow and report readiness. Charts are drawn with Rune's canvas
//! (lines and areas) and plain bars; the recharts tooltips and mount animations are not ported.
//!
//! A payload that does not match the daemon's shape is not drawn (`parse` returns `None`), so
//! it falls back to the ordinary tool card, as `chatVisualFromToolCall` does.

use std::collections::HashMap;

use rune::{Color, El, Shape, Tx, canvas, col, el, row, text};
use serde_json::Value;

use super::{is_open, toggle};
use crate::app::App;
use crate::assistant::model::{ToolCall, ToolStatus};
use crate::i18n::{Lang, lang, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

pub const TOOL: &str = "ui.visuals.render";
const SOURCES: [&str; 8] = [
    "overview",
    "balance_history",
    "portfolio",
    "tax_summary",
    "cash_flow",
    "transactions",
    "transaction_flow",
    "readiness",
];

/// A drawable visual: the tool's `visual` header and `data`, validated.
pub struct Visual {
    pub call_id: String,
    pub source: &'static str,
    pub visual: Value,
    pub data: Value,
}

impl Visual {
    fn fiat(&self) -> String {
        self.visual["fiat_currency"]
            .as_str()
            .filter(|c| c.len() == 3 && c.chars().all(|c| c.is_ascii_uppercase()))
            .unwrap_or("EUR")
            .to_owned()
    }
    fn rows(&self) -> &[Value] {
        self.data["rows"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }
}

fn opt_num(v: &Value) -> Result<Option<f64>, ()> {
    match v {
        Value::Null => Ok(None),
        Value::Number(n) => n.as_f64().filter(|f| f.is_finite()).map(Some).ok_or(()),
        _ if v.is_null() => Ok(None),
        _ => Err(()),
    }
}

/// `chatVisualFromToolCall`: the parts of the contract the bodies rely on.
pub fn parse(c: &ToolCall) -> Option<Visual> {
    if c.name != TOOL || c.status != ToolStatus::Done {
        return None;
    }
    let env = c.result.as_ref()?;
    if env["kind"].as_str() != Some(TOOL) {
        return None;
    }
    let data = env["data"].clone();
    let visual = data["visual"].clone();
    if visual["version"].as_u64() != Some(1)
        || visual["measure"]
            .as_str()
            .is_none_or(|m| m.trim().is_empty())
    {
        return None;
    }
    let unit = &visual["unit"];
    let unit_ok = match unit["kind"].as_str() {
        Some("fiat") => unit["code"].as_str().is_some_and(|c| c.len() == 3),
        Some("asset" | "sats" | "count") => true,
        _ => false,
    };
    if !unit_ok {
        return None;
    }
    let source = SOURCES
        .iter()
        .find(|s| visual["source"].as_str() == Some(**s))?;
    let rows_ok = |max: usize| {
        data["rows"]
            .as_array()
            .is_some_and(|r| r.len() <= max && r.iter().all(Value::is_object))
    };
    let nums_ok = |rows: &Value, fields: &[&str]| {
        rows.as_array().is_some_and(|r| {
            r.iter().all(|row| {
                fields
                    .iter()
                    .all(|f| opt_num(row.get(*f).unwrap_or(&Value::Null)).is_ok())
            })
        })
    };
    let ok = match *source {
        "overview" => {
            data["stats"].is_object() && data["series"].is_array() && data["wallets"].is_array()
        }
        "balance_history" => {
            rows_ok(5000)
                && nums_ok(&data["rows"], &["quantity", "market_value", "cost_basis"])
                && data["rows"]
                    .as_array()
                    .is_some_and(|r| r.iter().all(|x| x["period"].is_string()))
        }
        "portfolio" => {
            rows_ok(5000)
                && nums_ok(
                    &data["rows"],
                    &["quantity", "market_value", "cost_basis", "unrealized_pnl"],
                )
        }
        "tax_summary" => {
            rows_ok(5000)
                && data["rows"]
                    .as_array()
                    .is_some_and(|r| r.iter().all(|x| x["year"].is_number()))
        }
        "cash_flow" => rows_ok(5000) && data["totals"].is_object(),
        "transactions" => {
            rows_ok(50)
                && data["summary"].is_object()
                && data["rows"]
                    .as_array()
                    .is_some_and(|r| r.iter().all(|x| x["id"].is_string()))
        }
        "transaction_flow" => {
            rows_ok(5000)
                && data["rows"].as_array().is_some_and(|r| {
                    r.iter().all(|x| {
                        ["input", "output", "fee"].contains(&x["side"].as_str().unwrap_or(""))
                            && ["wallet", "generic", "fee", "overflow"]
                                .contains(&x["label_kind"].as_str().unwrap_or(""))
                            && x["label"].is_string()
                    })
                })
        }
        "readiness" => {
            rows_ok(50)
                && data["summary"].is_object()
                && data["rows"].as_array().is_some_and(|r| {
                    r.iter().all(|x| {
                        ["ok", "warning", "blocked"].contains(&x["status"].as_str().unwrap_or(""))
                    })
                })
        }
        _ => false,
    };
    ok.then(|| Visual {
        call_id: c.call_id.clone(),
        source,
        visual,
        data,
    })
}

/// `pendingVisualSources`: visuals still being drawn.
pub fn pending(calls: &[ToolCall]) -> Vec<(String, Option<String>)> {
    calls
        .iter()
        .filter(|c| c.name == TOOL && matches!(c.status, ToolStatus::Pending | ToolStatus::Running))
        .map(|c| {
            (
                c.call_id.clone(),
                c.arguments["source"].as_str().map(str::to_owned),
            )
        })
        .collect()
}

/// `visualPalette`: the categorical series and the gain/loss pair for this mode.
pub struct Palette {
    pub series: [Color; 4],
    pub positive: Color,
    pub negative: Color,
    pub muted: Color,
}

pub fn palette(tk: &Tk) -> Palette {
    let h = |s: &str| rune::color::hex(s).unwrap_or(Color::WHITE);
    if tk.dark {
        Palette {
            series: [h("#d27a0b"), h("#1d9fd8"), h("#34ac7e"), h("#8f84e8")],
            positive: h("#1d9fd8"),
            negative: h("#e66767"),
            muted: h("#5c5c5c"),
        }
    } else {
        Palette {
            series: [h("#e8820c"), h("#0284c7"), h("#1f9d6a"), h("#6250d6")],
            positive: h("#0284c7"),
            negative: h("#e0453f"),
            muted: h("#a3a3a3"),
        }
    }
}

fn source_icon(s: &str) -> &'static rune::Icon {
    match s {
        "overview" => &I::LAYOUT_DASHBOARD,
        "balance_history" => &I::CHART_LINE,
        "portfolio" => &I::CHART_PIE,
        "tax_summary" => &I::RECEIPT,
        "cash_flow" => &I::ARROW_DOWN_UP,
        "transactions" => &I::LIST,
        "transaction_flow" => &I::WORKFLOW,
        "readiness" => &I::LIST_CHECKS,
        _ => &I::CHART_LINE,
    }
}

fn source_route(s: &str) -> &'static str {
    match s {
        "overview" => "/overview",
        "cash_flow" | "transactions" | "transaction_flow" => "/transactions",
        _ => "/reports",
    }
}

fn accent(s: &str, p: &Palette) -> Color {
    match s {
        "cash_flow" | "tax_summary" => p.positive,
        "readiness" => p.series[2],
        "transactions" | "transaction_flow" => p.series[3],
        _ => p.series[0],
    }
}

// ---- Formatting (visuals/format.ts) ------------------------------------------------------------

fn de() -> bool {
    lang() == Lang::De
}

/// Groups and localises a fixed-point number: `1,234.5` / `1.234,5`.
pub fn number(v: f64, max_digits: usize, min_digits: usize) -> String {
    let s = format!("{:.*}", max_digits, v.abs());
    let (int, frac) = s.split_once('.').unwrap_or((&s, ""));
    let mut frac = frac.to_owned();
    while frac.len() > min_digits && frac.ends_with('0') {
        frac.pop();
    }
    let (g, d) = if de() { ('.', ',') } else { (',', '.') };
    let mut grouped = String::new();
    for (i, ch) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(g);
        }
        grouped.push(ch);
    }
    let sign = if v < 0.0 && (int != "0" || frac.chars().any(|c| c != '0')) {
        "-"
    } else {
        ""
    };
    if frac.is_empty() {
        format!("{sign}{grouped}")
    } else {
        format!("{sign}{grouped}{d}{frac}")
    }
}

fn btc(v: f64) -> String {
    format!("₿ {}", number(v, 8, 0))
}

fn btc_compact(v: f64) -> String {
    let m = v.abs();
    let d = if m >= 10.0 {
        1
    } else if m >= 1.0 {
        2
    } else if m >= 0.01 {
        4
    } else {
        6
    };
    format!("₿ {}", number(v, d, 0))
}

fn asset_amount(v: f64, asset: &str) -> String {
    let up = asset.to_uppercase();
    if asset.is_empty() || ["BTC", "LBTC", "L-BTC", "LNBTC"].contains(&up.as_str()) {
        if up == "BTC" || asset.is_empty() {
            btc(v)
        } else {
            format!("{} {asset}", btc(v))
        }
    } else {
        format!("{} {asset}", number(v, 8, 0))
    }
}

fn signed(formatted: String, v: f64) -> String {
    if v > 0.0 {
        format!("+{formatted}")
    } else if v < 0.0 && !formatted.contains('-') && !formatted.contains('−') {
        format!("−{formatted}")
    } else {
        formatted
    }
}

fn percent(v: f64) -> String {
    format!(
        "{}{}%",
        number(v * 100.0, 1, 0),
        if de() { "\u{a0}" } else { "" }
    )
}

const MONTHS_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const MONTHS_DE: [&str; 12] = [
    "Jän.", "Feb.", "März", "Apr.", "Mai", "Juni", "Juli", "Aug.", "Sep.", "Okt.", "Nov.", "Dez.",
];

/// `fmt.date(iso, style)` in UTC: `short` (`Jan 5, 2024`), `month`, `day`, `quarter`, `year`.
pub fn date(iso: &str, style: &str) -> String {
    if iso.len() == 4 && iso.chars().all(|c| c.is_ascii_digit()) {
        return iso.to_owned();
    }
    let Some(d) = crate::format::Day::parse(iso) else {
        return iso.to_owned();
    };
    let m = if de() {
        MONTHS_DE[(d.m as usize).clamp(1, 12) - 1]
    } else {
        MONTHS_EN[(d.m as usize).clamp(1, 12) - 1]
    };
    match style {
        "year" => d.y.to_string(),
        "quarter" => format!("Q{} {}", (d.m - 1) / 3 + 1, d.y),
        "month" => format!("{m} {}", d.y),
        "hour" => {
            let hm = iso
                .get(11..16)
                .filter(|t| t.as_bytes().get(2) == Some(&b':'))
                .unwrap_or("00:00");
            if de() {
                format!("{}. {m}, {hm}", d.d)
            } else {
                format!("{m} {}, {hm}", d.d)
            }
        }
        "day" | "week" => {
            if de() {
                format!("{}. {m}", d.d)
            } else {
                format!("{m} {}", d.d)
            }
        }
        _ => {
            if de() {
                format!("{}. {m} {}", d.d, d.y)
            } else {
                format!("{m} {}, {}", d.d, d.y)
            }
        }
    }
}

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|f| f.is_finite())
}

// ---- Shared pieces (visuals/primitives.tsx) -----------------------------------------------------

/// A figure that the privacy modes blur.
fn figure(app: &App, s: String, style: rune::TextStyle, color: Color) -> El {
    crate::ui::sensitive(
        s,
        style.features("'tnum' 1"),
        color,
        app.prefs.hide_sensitive(),
    )
}

/// `StatTile`: label, figure and an optional hint or incomplete note.
fn stat_tile(
    app: &App,
    tk: &Tk,
    label: String,
    value: Option<String>,
    hint: Option<String>,
    incomplete: Option<String>,
) -> El {
    stat_tile_delta(app, tk, label, value, None, hint, incomplete)
}

/// `DeltaPill`: an arrow and a signed change, green up, red down, muted flat.
fn delta_pill(app: &App, tk: &Tk, label: String, value: f64) -> El {
    let h = |s: &str| rune::color::hex(s).unwrap_or(tk.fg);
    let (fg, bg) = if value > 0.0 {
        (
            if tk.dark { h("#6ee7b7") } else { h("#047857") },
            h("#10b981").alpha(0.1),
        )
    } else if value < 0.0 {
        (
            if tk.dark { h("#fca5a5") } else { h("#b91c1c") },
            h("#ef4444").alpha(0.1),
        )
    } else {
        (tk.muted_fg, tk.muted)
    };
    row()
        .gap(2.0)
        .center_y()
        .radius(999.0)
        .fill(bg)
        .px(tk.r(0.375))
        .py(2.0)
        .child(
            ic(
                if value >= 0.0 {
                    &I::ARROW_UP_RIGHT
                } else {
                    &I::ARROW_DOWN_RIGHT
                },
                tk.r(0.75),
                fg,
            )
            .a11y_hidden(),
        )
        .child(crate::ui::sensitive(
            label,
            tk.t2xs(500.0),
            fg,
            app.prefs.hide_sensitive(),
        ))
}

/// `StatTile` with its `delta` pill beside the figure.
fn stat_tile_delta(
    app: &App,
    tk: &Tk,
    label: String,
    value: Option<String>,
    delta: Option<(String, f64)>,
    hint: Option<String>,
    incomplete: Option<String>,
) -> El {
    let fig = figure(
        app,
        value.unwrap_or_else(|| "—".into()),
        tk.tbase(600.0),
        if incomplete.is_some() {
            tk.muted_fg
        } else {
            tk.fg
        },
    );
    let fig_row = match delta {
        Some((l, v)) => row()
            .w_full()
            .flex_wrap()
            .center_y()
            .gap(tk.r(0.375))
            .mt(2.0)
            .child(fig)
            .child(delta_pill(app, tk, l, v)),
        None => row().mt(2.0).child(fig),
    };
    let mut c = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .fill(tk.background.alpha(0.4))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .child(
            text(label)
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .truncate(),
        )
        .child(fig_row);
    if let Some(n) = incomplete {
        c = c.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .mt(2.0)
                .child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), tk.warn.text).a11y_hidden())
                .child(text(n).font(tk.t2xs(400.0)).color(tk.warn.text)),
        );
    } else if let Some(h) = hint {
        c = c.child(
            text(h)
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .mt(2.0),
        );
    }
    c
}

/// `Segmented`: a radio group of small pills.
fn segmented(
    tk: &Tk,
    key: String,
    label: String,
    current: &str,
    options: Vec<(String, String)>,
) -> El {
    // `if (options.length < 2) return null`.
    if options.len() < 2 {
        return el();
    }
    let mut r = row()
        .gap(2.0)
        .center_y()
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .fill(tk.background.alpha(0.4))
        .padding(2.0)
        .role("radiogroup")
        .label(label);
    for (value, text_label) in options {
        let on = value == current;
        let k = key.clone();
        let v = value.clone();
        r = r.child(
            row()
                .key(format!("{key}-{value}"))
                .h(tk.r(1.5))
                .px(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.25))
                .fill(if on {
                    tk.fg.alpha(0.1)
                } else {
                    Color::TRANSPARENT
                })
                .role("radio")
                .checked(on)
                .label(text_label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    a.assistant.thread.choices.insert(k.clone(), v.clone());
                })
                .child(text(text_label).font(tk.t2xs(500.0)).color(if on {
                    tk.fg
                } else {
                    tk.muted_fg
                })),
        );
    }
    r
}

fn choice(app: &App, key: &str, default: &str) -> String {
    app.assistant
        .thread
        .choices
        .get(key)
        .cloned()
        .unwrap_or_else(|| default.to_owned())
}

/// A horizontal bar (`GrowBar`) in a muted track.
fn bar(tk: &Tk, fraction: f32, color: Color, height: f32) -> El {
    let f = fraction.clamp(0.0, 1.0);
    row()
        .grow(1.0)
        .min_w(0.0)
        .h(height)
        .radius(height / 2.0)
        .fill(tk.muted.alpha(0.5))
        .child(el().h_full().w_frac(f).radius(height / 2.0).fill(color))
}

fn legend(tk: &Tk, items: &[(String, Color)]) -> El {
    let mut r = row().flex_wrap().gap(tk.r(0.75)).center_y();
    for (label, c) in items {
        r = r.child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(el().w(tk.r(0.75)).h(2.0).radius(1.0).fill(*c).a11y_hidden())
                .child(text(label.clone()).font(tk.t2xs(400.0)).color(tk.muted_fg)),
        );
    }
    r
}

/// A charted series: values on the shared x points, `None` breaks the line (`connectNulls`).
struct Series {
    values: Vec<Option<f64>>,
    color: Color,
    area: bool,
    /// `type="stepAfter"`.
    step: bool,
    width: f32,
}

/// recharts-scale's `getFormatStep`: the rough step rounded up to a 0.05 (0.1 for one-digit
/// steps) share of its power of ten, `correction` notches further.
fn format_step(rough: f64, correction: f64) -> f64 {
    if rough <= 0.0 || !rough.is_finite() {
        return 1.0;
    }
    let digits = rough.log10().floor() + 1.0;
    let unit = 10f64.powf(digits);
    let ratio = rough / unit;
    // Steps of `unit / 20` (`/ 10` for one-digit steps), counted whole so ticks stay exact.
    let (scale, notch) = if digits != 1.0 {
        (0.05, unit / 20.0)
    } else {
        (0.1, unit / 10.0)
    };
    ((ratio / scale - 1e-9).ceil() + correction) * notch
}

/// recharts-scale's `getNiceTickValues` (`tickCount` 5): evenly stepped ticks that cover
/// `[min, max]`, zero among them when the range spans it.
fn nice_ticks(min: f64, max: f64, count: usize) -> Vec<f64> {
    if !(min.is_finite() && max.is_finite()) || count < 2 {
        return vec![0.0];
    }
    if (max - min).abs() < 1e-12 {
        // `getTickOfSingleValue`: steps of 1 (or the value's own power of ten) around it.
        let step = if min == 0.0 {
            1.0
        } else {
            10f64.powf(min.abs().log10().floor())
        };
        let start = if min > 0.0 {
            0.0
        } else {
            min - step * (count as f64 - 1.0) / 2.0
        };
        return (0..count).map(|i| start + step * i as f64).collect();
    }
    let mut correction = 0.0;
    loop {
        let step = format_step((max - min) / (count as f64 - 1.0), correction);
        let middle = if min <= 0.0 && max >= 0.0 {
            0.0
        } else {
            let m = (min + max) / 2.0;
            m - m.rem_euclid(step)
        };
        let mut below = ((middle - min) / step - 1e-9).ceil().max(0.0) as usize;
        let mut up = ((max - middle) / step - 1e-9).ceil().max(0.0) as usize;
        let scale = below + up + 1;
        if scale > count {
            correction += 1.0;
            continue;
        }
        if scale < count {
            if max > 0.0 {
                up += count - scale
            } else {
                below += count - scale
            }
        }
        let _ = up;
        let start = middle - below as f64 * step;
        return (0..count).map(|i| start + step * i as f64).collect();
    }
}

/// recharts' `getTicksEnd` (`interval="preserveEnd"`): from the last label back, each label that
/// fits before the previous one (with `gap`) shows; the last is pulled inside the axis.
fn end_ticks(widths: &[f32], coords: &[f32], start: f32, end: f32, gap: f32) -> Vec<(usize, f32)> {
    let mut shown = Vec::new();
    let mut limit = end;
    for i in (0..coords.len()).rev() {
        let size = widths[i];
        let mut c = coords[i];
        if i + 1 == coords.len() {
            let over = c + size / 2.0 - limit;
            if over > 0.0 {
                c -= over;
            }
        }
        if c - size / 2.0 >= start - 0.01 && c + size / 2.0 <= limit + 0.01 {
            limit = c - size / 2.0 - gap;
            shown.push((i, c));
        }
    }
    shown.reverse();
    shown
}

// ---- Tooltips (recharts `Tooltip` + primitives.tsx `TooltipFrame`) ------------------------------

/// A tooltip row: the series swatch, the value (blurred by the privacy modes), its label.
struct TipRow {
    color: Option<Color>,
    label: String,
    value: String,
}

fn tip_row(color: Option<Color>, label: String, value: String) -> TipRow {
    TipRow {
        color,
        label,
        value,
    }
}

/// The point of chart `key` under the pointer.
fn hovered(app: &App, key: &str) -> Option<usize> {
    app.assistant
        .thread
        .chart_hover
        .as_ref()
        .filter(|(k, _)| k == key)
        .map(|(_, i)| *i)
}

fn set_hover(s: &mut App, key: &str, i: Option<usize>, tx: &mut Tx) {
    let next = i.map(|i| (key.to_owned(), i));
    let cur = &s.assistant.thread.chart_hover;
    if *cur == next || (i.is_none() && cur.as_ref().is_none_or(|(k, _)| k != key)) {
        tx.live_only();
        return;
    }
    s.assistant.thread.chart_hover = next;
}

/// `TooltipFrame`: `min-w-36 rounded-md border bg-popover/95 px-2.5 py-2 text-xs shadow-lg`, the
/// title in `text-2xs` muted, then a row per series. Returns the frame and its width.
fn tooltip_frame(app: &App, tk: &Tk, title: String, rows: &[TipRow]) -> (El, f32) {
    let hide = app.prefs.hide_sensitive();
    let value_style = tk.txs(500.0).features("'tnum' 1");
    let label_style = tk.txs(400.0);
    let swatch = tk.r(0.75);
    let gap = tk.r(0.5);
    let widest = rows
        .iter()
        .map(|r| {
            (if r.color.is_some() { swatch + gap } else { 0.0 })
                + rune::text::measure(&r.value, &[], value_style, None).0
                + gap
                + rune::text::measure(&r.label, &[], label_style, None).0
        })
        .fold(
            rune::text::measure(&title, &[], tk.t2xs(400.0), None).0,
            f32::max,
        );
    let w = (widest + tk.r(1.25) + 2.0).max(tk.r(9.0));
    let mut list = col().w_full().gap(tk.r(0.125));
    for r in rows {
        let mut line = row().w_full().gap(gap).center_y();
        if let Some(c) = r.color {
            line = line.child(el().w(swatch).h(2.0).radius(1.0).fill(c));
        }
        line = line
            .child(crate::ui::sensitive(
                r.value.clone(),
                value_style,
                tk.fg,
                hide,
            ))
            .child(
                text(r.label.clone())
                    .font(label_style)
                    .color(tk.muted_fg)
                    .truncate()
                    .min_w(0.0),
            );
        list = list.child(line);
    }
    let frame = col()
        .w(w)
        .px(tk.r(0.625))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .fill(tk.popover.alpha(0.95))
        .border(1.0, tk.border)
        .border_layout()
        .box_shadow(rune::Shadow {
            y: 10.0,
            blur: 15.0,
            color: Color::BLACK.alpha(0.1),
            ..rune::Shadow::default()
        })
        .box_shadow(rune::Shadow {
            y: 4.0,
            blur: 6.0,
            color: Color::BLACK.alpha(0.1),
            ..rune::Shadow::default()
        })
        .child(
            text(title)
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .mb(tk.r(0.25)),
        )
        .child(list);
    (frame, w)
}

/// The tooltip beside x `at` (recharts' offset 10, flipped left when it would leave the chart),
/// near the plot's top.
fn place_tooltip(frame: El, w: f32, at: f32, chart_w: f32, top: f32) -> El {
    let x = if at + 10.0 + w <= chart_w {
        at + 10.0
    } else {
        (at - 10.0 - w).max(0.0)
    };
    frame.absolute(x, top).inert()
}

/// A recharts `AreaChart` as the visuals draw it: a 58 pt value axis, a 30 pt category axis
/// (labels that fit, `minTickGap` apart), horizontal grid rules at the ticks, the series as
/// lines (areas fill to the axis floor). `margin` is recharts' (top, right).
#[allow(clippy::too_many_arguments)]
fn axis_chart(
    app: &App,
    tk: &Tk,
    key: String,
    series: &[Series],
    x_labels: &[String],
    min_gap: f32,
    height: f32,
    margin: (f32, f32),
    y_fmt: &dyn Fn(f64) -> String,
    tip: &dyn Fn(usize) -> (String, Vec<TipRow>),
) -> El {
    let width = app
        .assistant
        .thread
        .chart_w
        .get(&key)
        .copied()
        .unwrap_or(560.0);
    let (y_w, x_h) = (58.0_f32, 30.0_f32);
    let pw = (width - y_w - margin.1).max(10.0);
    let ph = (height - margin.0 - x_h).max(10.0);
    let all: Vec<f64> = series
        .iter()
        .flat_map(|s| s.values.iter().flatten().copied())
        .collect();
    let (dmin, dmax) = all
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), v| (a.min(*v), b.max(*v)));
    let (dmin, dmax) = if all.is_empty() {
        (0.0, 1.0)
    } else {
        (
            dmin.min(0.0),
            dmax.max(if dmin < 0.0 && dmax < 0.0 { 0.0 } else { dmax }),
        )
    };
    let ticks = nice_ticks(dmin, dmax, 5);
    let (lo, hi) = (ticks[0], ticks[ticks.len() - 1].max(ticks[0] + 1e-12));
    let y_of = |v: f64| ph - (((v - lo) / (hi - lo)) as f32) * ph;
    let n = series
        .iter()
        .map(|s| s.values.len())
        .max()
        .unwrap_or(0)
        .max(x_labels.len());
    let x_of = |i: usize| {
        if n <= 1 {
            pw / 2.0
        } else {
            pw * i as f32 / (n - 1) as f32
        }
    };
    let grid = tk.border.alpha(tk.border.a * 0.5);
    let mut shape = Shape::new().view_box(pw, ph);
    for v in &ticks {
        let y = y_of(*v).clamp(0.5, ph - 0.5);
        shape = shape.line([0.0, y], [pw, y], 1.0, rune::LineCap::Butt, grid);
    }
    let floor = y_of(0.0f64.clamp(lo, hi));
    for s in series {
        // Runs of known values; a gap starts a new run.
        let mut runs: Vec<Vec<[f32; 2]>> = vec![vec![]];
        let mut prev_y: Option<f32> = None;
        for (i, v) in s.values.iter().enumerate() {
            match v {
                Some(v) => {
                    let (x, y) = (x_of(i), y_of(*v));
                    let run = runs.last_mut().unwrap();
                    if s.step
                        && let Some(py) = prev_y
                    {
                        run.push([x, py]);
                    }
                    run.push([x, y]);
                    prev_y = Some(y);
                }
                None => {
                    if !runs.last().unwrap().is_empty() {
                        runs.push(vec![]);
                    }
                    prev_y = None;
                }
            }
        }
        for pts in runs.into_iter().filter(|r| r.len() >= 2) {
            if s.area {
                let mut poly = pts.clone();
                poly.push([pts[pts.len() - 1][0], floor]);
                poly.push([pts[0][0], floor]);
                shape = shape.fill(poly, s.color.alpha(0.12));
            }
            shape = shape.stroke(pts, s.width, s.color);
        }
    }
    let tick_style = rune::TextStyle::new(tk.r(0.625), 400.0);
    let lh = rune::text::measure("0", &[], tick_style, None).1;
    let k = key.clone();
    let mut c = col()
        .key(key.clone())
        .w_full()
        .h(height)
        .a11y_hidden()
        .on_layout(move |s: &mut App, r: [f32; 4]| {
            if s.assistant
                .thread
                .chart_w
                .get(&k)
                .is_none_or(|w| (w - r[2]).abs() > 0.5)
            {
                s.assistant.thread.chart_w.insert(k.clone(), r[2]);
            }
        })
        .child(canvas(shape).absolute(y_w, margin.0).size(pw, ph));
    // The value axis: labels end 8 pt left of the plot (tick size 6 + margin 2).
    for v in &ticks {
        let label = y_fmt(*v);
        if label.is_empty() {
            continue;
        }
        c = c.child(
            text(label)
                .font(tick_style)
                .color(tk.muted_fg)
                .w(y_w - 8.0)
                .align(rune::Align::End)
                .absolute(0.0, margin.0 + y_of(*v) - lh / 2.0),
        );
    }
    // The category axis.
    let mut cache: HashMap<&str, f32> = HashMap::new();
    let widths: Vec<f32> = x_labels
        .iter()
        .map(|l| {
            *cache
                .entry(l.as_str())
                .or_insert_with(|| rune::text::measure(l, &[], tick_style, None).0)
        })
        .collect();
    let coords: Vec<f32> = (0..x_labels.len()).map(|i| y_w + x_of(i)).collect();
    for (i, cx) in end_ticks(&widths, &coords, y_w, y_w + pw, min_gap) {
        c = c.child(
            text(x_labels[i].clone())
                .font(tick_style)
                .color(tk.muted_fg)
                .absolute(cx - widths[i] / 2.0, margin.0 + ph + 8.0),
        );
    }
    // The hovered point: the cursor rule, each series' active dot (`r: 4`, a 2 pt card ring),
    // and the tooltip.
    if let Some(i) = hovered(app, &key).filter(|i| *i < n) {
        let x = y_w + x_of(i);
        c = c.child(
            el().absolute(x - 0.5, margin.0)
                .size(1.0, ph)
                .fill(tk.border)
                .inert(),
        );
        for s in series {
            if let Some(v) = s.values.get(i).copied().flatten() {
                let y = margin.0 + y_of(v);
                c = c.child(
                    el().absolute(x - 5.0, y - 5.0)
                        .size(10.0, 10.0)
                        .radius(5.0)
                        .fill(s.color)
                        .border(2.0, tk.card)
                        .inert(),
                );
            }
        }
        let (title, rows) = tip(i);
        let (frame, w) = tooltip_frame(app, tk, title, &rows);
        c = c.child(place_tooltip(frame, w, x, width, margin.0));
    }
    // The pointer picks the nearest point (`Tooltip` follows the mouse over the plot).
    let hk = key.clone();
    let hk2 = key.clone();
    c.child(
        el().absolute(y_w, margin.0)
            .size(pw, ph)
            .on_pointer_move_local(move |s: &mut App, e, tx: &mut Tx| {
                let i = if n <= 1 {
                    0
                } else {
                    ((e.x / pw) * (n - 1) as f32)
                        .round()
                        .clamp(0.0, (n - 1) as f32) as usize
                };
                set_hover(s, &hk, (n > 0).then_some(i), tx);
            })
            .on_hover_change(move |s: &mut App, on, tx: &mut Tx| {
                if !on {
                    set_hover(s, &hk2, None, tx);
                }
            }),
    )
}

/// The days between two ISO dates (`Date.parse` difference), when both parse.
fn span_days(a: &str, b: &str) -> f64 {
    match (crate::format::Day::parse(a), crate::format::Day::parse(b)) {
        (Some(x), Some(y)) => (y.epoch_days() - x.epoch_days()) as f64,
        _ => 0.0,
    }
}

/// The ranges a series long enough offers (`["all", …["1y", "3m"] longer than 1.2× theirs]`).
fn ranges_for(span: f64) -> Vec<&'static str> {
    let mut v = vec!["all"];
    for (r, days) in [("1y", 366.0), ("3m", 92.0)] {
        if span > days * 1.2 {
            v.push(r);
        }
    }
    v
}

fn range_days(range: &str) -> Option<f64> {
    match range {
        "1y" => Some(366.0),
        "3m" => Some(92.0),
        _ => None,
    }
}

/// `onAsk`: a follow-up question sent (or queued) as the user's own.
fn ask(app: &mut App, prompt: &str) {
    crate::assistant::chat::send_prompt(app, prompt, None);
}

// ---- The card ---------------------------------------------------------------------------------

/// `ChatVisual`: the figure card.
pub fn card(app: &App, tk: &Tk, v: &Visual, wide: bool) -> El {
    let p = palette(tk);
    let glow = accent(v.source, &p);
    let key = format!("asst-visual-{}", v.call_id);
    let title = v.visual["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t(&format!("assistant:visual.source.{}", v.source)));
    let scope = subtitle(v);
    let table_key = format!("{key}-table");
    let show_table = is_open(app, &table_key, false);
    let copy_key = format!("{key}-csv");
    let copied = app
        .assistant
        .thread
        .copied
        .as_ref()
        .is_some_and(|(k, until)| *k == copy_key && *until > app.clock);
    let action = |k: String, glyph: &'static rune::Icon, label: String, pressed: bool| {
        row()
            .key(k)
            .size(tk.r(1.75), tk.r(1.75))
            .center()
            .radius(tk.r(0.375))
            .fill(if pressed {
                tk.muted
            } else {
                Color::TRANSPARENT
            })
            .hover(tk.muted)
            .role("button")
            .label(label.clone())
            .help(label)
            .checked(pressed)
            .focusable(true)
            .child(
                ic(
                    glyph,
                    tk.r(0.875),
                    if pressed { tk.fg } else { tk.muted_fg },
                )
                .a11y_hidden(),
            )
    };
    let csv = table_csv(&table(v));
    let tk_ = table_key.clone();
    let ck = copy_key.clone();
    let route = source_route(v.source);
    let local_tx = if v.source == "transaction_flow" {
        v.data["local_reference"]["transaction"]
            .as_str()
            .map(str::to_owned)
            .filter(|_| v.data["local_reference"]["route"].as_str() == Some("/transactions"))
    } else {
        None
    };
    let expand_key = key.clone();
    let actions = row()
        .gap(2.0)
        .center_y()
        .child(
            action(
                format!("{key}-a-table"),
                &I::TABLE_2,
                t(if show_table {
                    "assistant:visual.actions.hideTable"
                } else {
                    "assistant:visual.actions.showTable"
                }),
                show_table,
            )
            .on_press(move |a: &mut App| toggle(a, &tk_, false)),
        )
        .child(
            action(
                format!("{key}-a-csv"),
                if copied { &I::CHECK } else { &I::COPY },
                t(if copied {
                    "assistant:visual.actions.copied"
                } else {
                    "assistant:visual.actions.copyCsv"
                }),
                false,
            )
            .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                crate::osleaks::clipboard::copy(a, tx, csv.clone());
                super::mark_copied(a, tx, &ck);
            }),
        )
        .child(
            action(
                format!("{key}-a-expand"),
                &I::MAXIMIZE_2,
                t("assistant:visual.actions.expand"),
                false,
            )
            .on_press(move |a: &mut App| {
                a.assistant.thread.expanded_visual = Some(expand_key.clone())
            }),
        )
        .child(
            action(
                format!("{key}-a-open"),
                &I::ARROW_UP_RIGHT,
                t("assistant:visual.actions.openInApp"),
                false,
            )
            .on_press(move |a: &mut App| match &local_tx {
                Some(id) => super::open_transaction(a, id),
                None => a.navigate(route),
            }),
        );
    let head = row()
        .w_full()
        .gap(tk.r(0.75))
        .mb(tk.r(0.75))
        .child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(tk.r(0.625))
                .child(
                    row()
                        .size(tk.r(1.75), tk.r(1.75))
                        .mt(2.0)
                        .center()
                        .radius(tk.r(0.375))
                        .fill(glow.alpha(0.16))
                        .child(ic(source_icon(v.source), tk.r(1.0), glow))
                        .a11y_hidden(),
                )
                .child({
                    let mut c = col().min_w(0.0).child(
                        text(title.clone())
                            .font(tk.tsm(600.0))
                            .color(tk.fg)
                            .truncate(),
                    );
                    if !scope.is_empty() {
                        c = c.child(
                            text(scope.clone())
                                .font(tk.t2xs(400.0))
                                .color(tk.muted_fg)
                                .truncate(),
                        );
                    }
                    c
                }),
        )
        .child(actions);
    let body = if show_table {
        table_view(app, tk, v)
    } else {
        body(app, tk, v, &key, wide, false)
    };
    let mut c = col()
        .key(key.clone())
        .w_full()
        .min_w(0.0)
        .my(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .clip_rounded(tk.r(0.5))
        .role("figure")
        .label(title.clone())
        // `kb-visual-rise`: a new figure fades in from 6 px below (Rune lands it at once on the
        // first frame and shortens it to a fade under Reduce Motion).
        .transition(rune::Transition::offset(0.0, 6.0).combined(rune::Transition::opacity()))
        // The soft glow of the source's colour in the top-left corner.
        .child(
            el().absolute(-64.0, -96.0)
                .size(256.0, 256.0)
                .radius(128.0)
                .fill(glow.alpha(if tk.dark { 0.16 } else { 0.10 }))
                .blur(48.0)
                .a11y_hidden()
                .inert(),
        )
        .child(head)
        .child(body);
    let caveats = v.data["caveats"].as_array().cloned().unwrap_or_default();
    if !caveats.is_empty() {
        let mut ul = col().w_full().mt(tk.r(0.75)).gap(tk.r(0.25));
        for cv in caveats {
            let Some(code) = cv["code"].as_str().filter(|c| !c.trim().is_empty()) else {
                continue;
            };
            let k = format!("assistant:visual.caveat.{code}");
            let fallback = tr("assistant:visual.caveat.other", &[("code", code)]);
            let s = match num(&cv["count"]) {
                Some(n)
                    if crate::i18n::has(&format!("{k}_one"))
                        || crate::i18n::has(&format!("{k}_other")) =>
                {
                    tc(&k, n as usize, &[])
                }
                _ if crate::i18n::has(&k) => t(&k),
                _ => fallback,
            };
            ul = ul.child(
                row()
                    .w_full()
                    .gap(tk.r(0.375))
                    .role("note")
                    .child(
                        ic(
                            &I::TRIANGLE_ALERT,
                            tk.r(0.875),
                            rune::color::hex("#f59e0b").unwrap_or(tk.warn.solid),
                        )
                        .mt(2.0)
                        .a11y_hidden(),
                    )
                    .child(
                        text(s)
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        c = c.child(ul);
    }
    // Two follow-up questions per visual (`FOLLOW_UPS`), asked through the session (`onAsk`).
    let follow: &[&str] = match v.source {
        "overview" => &["balanceOverTime", "whatNeedsAttention"],
        "balance_history" => &["holdings", "cashFlow"],
        "portfolio" => &["balanceOverTime", "largestTransactions"],
        "tax_summary" => &["whatNeedsAttention", "disposals"],
        "cash_flow" => &["largestOutgoing", "balanceOverTime"],
        "transactions" => &["cashFlow", "overview"],
        "transaction_flow" => &["recentTransactions", "overview"],
        _ => &[],
    };
    let mut foot = row()
        .w_full()
        .flex_wrap()
        .gap(tk.r(0.375))
        .center_y()
        .mt(tk.r(0.75));
    for f in follow {
        let q = t(&format!("assistant:visual.followUp.{f}"));
        let prompt = q.clone();
        foot = foot.child(
            row()
                .key(format!("{key}-follow-{f}"))
                .gap(tk.r(0.25))
                .center_y()
                .radius(999.0)
                .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                .border_layout()
                .fill(tk.background.alpha(0.5))
                .px(tk.r(0.625))
                .py(tk.r(0.25))
                .role("button")
                .label(q.clone())
                .focusable(true)
                .on_press(move |a: &mut App| ask(a, &prompt))
                .child(ic(&I::MESSAGE_SQUARE_PLUS, tk.r(0.75), tk.muted_fg).a11y_hidden())
                .child(text(q).font(tk.t2xs(400.0)).color(tk.muted_fg)),
        );
    }
    // `ml-auto`: beside the chips when it fits, else on its own line, right-aligned.
    // Unwrapped, so its whole width decides the line: beside the chips when it fits, else on
    // its own line, right-aligned (`ml-auto`).
    foot = foot.child(
        row().grow(1.0).justify_end().child(
            text(t("assistant:visual.provenance"))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg.alpha(0.8)),
        ),
    );
    c.child(foot)
}

/// The "Open larger" dialog for the visual whose key is `asst-visual-<call id>`.
pub fn expanded_dialog(app: &App, tk: &Tk, v: &Visual, layer: &mut Vec<El>) {
    let key = format!("asst-visual-{}", v.call_id);
    let p = palette(tk);
    let title = v.visual["title"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t(&format!("assistant:visual.source.{}", v.source)));
    let close = |a: &mut App| a.assistant.thread.expanded_visual = None;
    let dialog = col()
        .key(format!("{key}-dialog"))
        .w_full()
        .max_w(tk.r(56.0))
        .gap(tk.r(0.5))
        .padding(tk.r(1.5))
        .radius(tk.r(0.5))
        .fill(tk.background)
        .border(1.0, tk.border)
        .border_layout()
        .role("dialog")
        .label(title.clone())
        .modal(true, close)
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(source_icon(v.source), tk.r(1.0), accent(v.source, &p)).a11y_hidden())
                .child(text(title).font(tk.tbase(600.0)).color(tk.fg).grow(1.0))
                .child(
                    crate::txdetail::widgets::icon_button(
                        tk,
                        format!("{key}-dialog-close"),
                        crate::txdetail::widgets::Variant::Ghost,
                        tk.r(1.75),
                        &I::X,
                        tk.r(1.0),
                        t("common:actions.close"),
                    )
                    .on_press(close),
                ),
        )
        .child(text(subtitle(v)).font(tk.txs(400.0)).color(tk.muted_fg))
        .child(
            col()
                .w_full()
                .max_h(app.shell.window_h * 0.75)
                .scroll(rune::ScrollSpec {
                    stick_bottom: false,
                })
                .child(body(app, tk, v, &format!("{key}-x"), true, true)),
        );
    layer.push(
        crate::shell::window_backdrop(format!("{key}-scrim"), Color::BLACK.alpha(0.5))
            .on_press(close)
            .role("none"),
    );
    layer.push(
        row()
            .key(format!("{key}-dialog-layer"))
            .absolute(0.0, 0.0)
            .w_full()
            .h_full()
            .center()
            .padding(tk.r(1.0))
            .child(dialog),
    );
}

fn subtitle(v: &Visual) -> String {
    let mut parts = vec![];
    let vis = &v.visual;
    let tkey = |prefix: &str, val: &str| {
        let k = format!("assistant:visual.{prefix}.{val}");
        if crate::i18n::has(&k) {
            t(&k)
        } else {
            val.to_owned()
        }
    };
    if v.source == "cash_flow"
        && let Some(p) = vis["period"].as_str().filter(|s| !s.is_empty())
    {
        parts.push(tkey("period", p));
    }
    if v.source == "balance_history" {
        parts.push(tkey(
            "interval",
            vis["interval"].as_str().unwrap_or("month"),
        ));
    }
    if v.source == "transactions" {
        parts.push(tkey("sort", vis["sort"].as_str().unwrap_or("recent")));
    }
    for (k, val) in vis["filters"].as_object().into_iter().flatten() {
        let Some(val) = val.as_str().filter(|s| !s.is_empty()) else {
            continue;
        };
        parts.push(match k.as_str() {
            "period" => tkey("period", val),
            "direction" => t(&format!(
                "assistant:visual.transactions.direction.{}",
                if val == "inbound" { "in" } else { "out" }
            )),
            "query" => format!("“{val}”"),
            _ => val.to_owned(),
        });
    }
    if let Some(at) = vis["as_of"].as_str().filter(|s| !s.trim().is_empty()) {
        parts.push(tr("assistant:visual.asOf", &[("date", &date(at, "short"))]));
    }
    parts.join(" · ")
}

/// `ChatVisualSkeleton`.
pub fn skeleton(tk: &Tk, call_id: &str, source: Option<&str>) -> El {
    let known = source.filter(|s| SOURCES.contains(s));
    let label = tr(
        "assistant:visual.drawing",
        &[(
            "source",
            &t(&format!(
                "assistant:visual.source.{}",
                known.unwrap_or("generic")
            )),
        )],
    );
    col()
        .key(format!("asst-visual-pending-{call_id}"))
        .w_full()
        .my(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .role("status")
        .label(label.clone())
        .child(
            row()
                .gap(tk.r(0.625))
                .center_y()
                .mb(tk.r(0.75))
                .child(
                    row()
                        .size(tk.r(1.75), tk.r(1.75))
                        .center()
                        .radius(tk.r(0.375))
                        .fill(tk.muted)
                        .child(ic(
                            known.map_or(&I::CHART_LINE, source_icon),
                            tk.r(1.0),
                            tk.muted_fg,
                        ))
                        .a11y_hidden(),
                )
                .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg)),
        )
        .child(
            el().w_frac(0.33)
                .h(tk.r(1.5))
                .radius(tk.r(0.375))
                .fill(tk.muted.alpha(0.5)),
        )
        .child(
            el().w_full()
                .h(tk.r(8.0))
                .mt(tk.r(0.5))
                .radius(tk.r(0.375))
                .fill(tk.muted.alpha(0.4)),
        )
}

// ---- Bodies ----------------------------------------------------------------------------------

fn body(app: &App, tk: &Tk, v: &Visual, key: &str, wide: bool, expanded: bool) -> El {
    match v.source {
        "overview" => overview(app, tk, v, key, wide, expanded),
        "balance_history" => balance(app, tk, v, key, expanded),
        "portfolio" => allocation(app, tk, v, key, wide),
        "tax_summary" => tax_years(app, tk, v, key, expanded),
        "cash_flow" => cash_flow(app, tk, v, expanded),
        "transactions" => transactions(app, tk, v),
        "transaction_flow" => flow(app, tk, v),
        _ => readiness(app, tk, v),
    }
}

fn tiles(row_or_grid: Vec<El>, tk: &Tk, per_row: usize) -> El {
    let mut c = col().w_full().gap(tk.r(0.5));
    let mut it = row_or_grid.into_iter().peekable();
    while it.peek().is_some() {
        let mut r = row().w_full().gap(tk.r(0.5));
        for _ in 0..per_row {
            match it.next() {
                Some(e) => r = r.child(e),
                None => r = r.child(el().grow(1.0).w(0.0)),
            }
        }
        c = c.child(r);
    }
    c
}

fn overview(app: &App, tk: &Tk, v: &Visual, key: &str, wide: bool, expanded: bool) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let fiat = |x: f64| crate::format::fiat(x, &cur);
    let s = &v.data["stats"];
    let hide = app.prefs.hide_sensitive();
    let hide_shares = app.prefs.hide_shares();
    let incomplete = s["basis_complete"].as_bool() != Some(true);
    let mut sub = row().flex_wrap().child(figure(
        app,
        num(&s["btc_balance"])
            .map(btc)
            .unwrap_or_else(|| "—".into()),
        tk.txs(400.0),
        tk.muted_fg,
    ));
    if let Some(price) = num(&s["btc_price"]) {
        let mut tail = format!(
            " · {}",
            tr(
                "assistant:visual.overview.price",
                &[("price", &fiat(price))]
            )
        );
        if let Some(at) = s["price_at"].as_str() {
            tail.push_str(&format!(" · {}", date(at, "short")));
        }
        sub = sub.child(text(tail).font(tk.txs(400.0)).color(tk.muted_fg));
    }
    let (tone, glyph, label) = if incomplete {
        (
            tk.warn,
            &I::TRIANGLE_ALERT,
            t("assistant:visual.overview.incomplete"),
        )
    } else {
        (
            tk.good,
            &I::CIRCLE_CHECK,
            t("assistant:visual.overview.complete"),
        )
    };
    let head = row()
        .w_full()
        .flex_wrap()
        .items_end()
        .justify_between()
        .gap(tk.r(0.75))
        .child(
            col()
                .min_w(0.0)
                .child(
                    text(t("assistant:visual.overview.portfolioValue"))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg),
                )
                // `text-3xl sm:text-4xl`: the window is always past `sm`.
                .child(figure(
                    app,
                    num(&s["portfolio_value"])
                        .map(fiat)
                        .unwrap_or_else(|| "—".into()),
                    rune::TextStyle::new(tk.r(if wide { 2.25 } else { 1.875 }), 600.0)
                        .lh(tk.r(if wide { 2.5 } else { 2.25 }))
                        .tracking(-0.025 * tk.r(if wide { 2.25 } else { 1.875 })),
                    tk.fg,
                ))
                .child(sub.mt(2.0)),
        )
        .child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(glyph, tk.r(0.75), tone.text).a11y_hidden())
                .child(text(label).font(tk.t2xs(500.0)).color(tone.text)),
        );
    let inc = incomplete.then(|| t("assistant:visual.overview.incompleteShort"));
    let signed_fiat = |x: f64| signed(fiat(x.abs()), x);
    let unrealized = num(&s["unrealized"]);
    let pct = unrealized
        .zip(num(&s["cost_basis"]).filter(|c| *c != 0.0))
        .map(|(u, c)| u / c.abs());
    let delta = (!incomplete)
        .then_some(())
        .and(pct.zip(unrealized))
        .map(|(pc, u)| {
            (
                if hide_shares {
                    crate::theme::MASK.to_owned()
                } else {
                    percent(pc)
                },
                u,
            )
        });
    let cards = vec![
        stat_tile(
            app,
            tk,
            t("assistant:visual.overview.costBasis"),
            num(&s["cost_basis"]).map(fiat),
            None,
            inc.clone(),
        ),
        stat_tile_delta(
            app,
            tk,
            t("assistant:visual.overview.unrealized"),
            unrealized.map(signed_fiat),
            delta,
            None,
            inc.clone(),
        ),
        stat_tile(
            app,
            tk,
            t("assistant:visual.overview.realizedYtd"),
            num(&s["realized_ytd"]).map(signed_fiat),
            None,
            inc,
        ),
        stat_tile(
            app,
            tk,
            t("assistant:visual.overview.transactions"),
            num(&s["transactions"]).map(|n| crate::format::count(n as usize)),
            None,
            None,
        ),
    ];
    let mut c = col().w_full().gap(tk.r(0.75)).child(head).child(tiles(
        cards,
        tk,
        if wide { 4 } else { 2 },
    ));
    let all = v.data["series"].as_array().cloned().unwrap_or_default();
    let first = all.first().and_then(|x| x["date"].as_str()).unwrap_or("");
    let last = all.last().and_then(|x| x["date"].as_str()).unwrap_or("");
    let span = if all.len() > 1 {
        span_days(first, last)
    } else {
        0.0
    };
    let ranges = ranges_for(span);
    let range_key = format!("{key}-range");
    let range = choice(app, &range_key, "all");
    let series: Vec<Value> = match range_days(&range) {
        Some(days) if !all.is_empty() => all
            .iter()
            .filter(|x| span_days(x["date"].as_str().unwrap_or(""), last) <= days)
            .cloned()
            .collect(),
        _ => all,
    };
    if series.len() > 1 {
        let values: Vec<Option<f64>> = series.iter().map(|x| num(&x["market_value"])).collect();
        let basis: Vec<Option<f64>> = series.iter().map(|x| num(&x["cost_basis"])).collect();
        let labels: Vec<String> = series
            .iter()
            .map(|x| date(x["date"].as_str().unwrap_or(""), "month"))
            .collect();
        let lines = [
            Series {
                values,
                color: p.series[0],
                area: true,
                step: false,
                width: 2.0,
            },
            Series {
                values: basis,
                color: p.muted,
                area: false,
                step: true,
                width: 1.5,
            },
        ];
        let y_fmt = |x: f64| {
            if hide {
                String::new()
            } else {
                crate::format::fiat_compact(x, &cur)
            }
        };
        let options = ranges
            .iter()
            .map(|r| (r.to_string(), t(&format!("assistant:visual.range.{r}"))))
            .collect();
        let tip = |i: usize| {
            let x = &series[i];
            let or_unknown =
                |v: Option<String>| v.unwrap_or_else(|| t("assistant:visual.unknownValue"));
            (
                date(x["date"].as_str().unwrap_or(""), "short"),
                vec![
                    tip_row(
                        Some(p.series[0]),
                        t("assistant:visual.measure.market_value"),
                        or_unknown(num(&x["market_value"]).map(fiat)),
                    ),
                    tip_row(
                        Some(p.muted),
                        t("assistant:visual.measure.cost_basis"),
                        or_unknown(num(&x["cost_basis"]).map(fiat)),
                    ),
                    tip_row(
                        None,
                        t("assistant:visual.measure.quantity"),
                        or_unknown(num(&x["balance_btc"]).map(btc)),
                    ),
                ],
            )
        };
        c = c.child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(
                    row()
                        .w_full()
                        .justify_between()
                        .center_y()
                        .gap(tk.r(0.5))
                        .child(legend(
                            tk,
                            &[
                                (t("assistant:visual.measure.market_value"), p.series[0]),
                                (t("assistant:visual.measure.cost_basis"), p.muted),
                            ],
                        ))
                        .child(segmented(
                            tk,
                            range_key,
                            t("assistant:visual.controls.range"),
                            &range,
                            options,
                        )),
                )
                .child(axis_chart(
                    app,
                    tk,
                    format!("{key}-chart{}", if expanded { "-x" } else { "" }),
                    &lines,
                    &labels,
                    28.0,
                    if expanded { tk.r(18.0) } else { tk.r(10.0) },
                    (6.0, 8.0),
                    &y_fmt,
                    &tip,
                )),
        );
    }
    let wallets = v.data["wallets"].as_array().cloned().unwrap_or_default();
    if !wallets.is_empty() {
        let colors: Vec<Color> = p
            .series
            .iter()
            .copied()
            .chain(std::iter::once(p.muted))
            .collect();
        let color = |i: usize| colors[i.min(colors.len() - 1)];
        let mut alloc = col().w_full().gap(tk.r(0.375)).child(
            text(t("assistant:visual.overview.allocation"))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg),
        );
        if !hide_shares {
            let mut strip = row()
                .w_full()
                .h(tk.r(0.625))
                .gap(2.0)
                .radius(999.0)
                .clip_rounded(tk.r(0.3125))
                .role("img")
                .label(t("assistant:visual.overview.allocation"));
            for (i, w) in wallets.iter().enumerate() {
                let share = num(&w["share"]).unwrap_or(0.0);
                if share > 0.0 {
                    strip = strip.child(el().h_full().grow(share as f32).w(0.0).fill(color(i)));
                }
            }
            alloc = alloc.child(strip);
        }
        // `grid-cols-1 sm:grid-cols-2 gap-x-4 gap-y-1`.
        let cell = |i: usize, w: &Value| {
            let label = w["label"].as_str().map(str::to_owned).unwrap_or_else(|| {
                tc(
                    "assistant:visual.overview.otherWallets",
                    num(&w["other_count"]).unwrap_or(0.0) as usize,
                    &[],
                )
            });
            let share = num(&w["share"]).unwrap_or(0.0);
            row()
                .key(format!("{key}-w-{i}"))
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .gap(tk.r(0.5))
                .center_y()
                .child(el().size(8.0, 8.0).radius(2.0).fill(color(i)).a11y_hidden())
                .child(
                    text(label)
                        .font(tk.txs(400.0))
                        .color(tk.fg)
                        .truncate()
                        .grow(1.0)
                        .min_w(0.0),
                )
                .child(figure(
                    app,
                    btc_compact(num(&w["balance_btc"]).unwrap_or(0.0)),
                    tk.txs(400.0),
                    tk.muted_fg,
                ))
                .child(
                    text(if hide_shares {
                        crate::theme::MASK.to_owned()
                    } else {
                        percent(share)
                    })
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.fg)
                    .w(tk.r(3.0))
                    .align(rune::Align::End),
                )
        };
        let per = if wide { 2 } else { 1 };
        let mut grid = col().w_full().gap(tk.r(0.25)).role("list");
        for (r, chunk) in wallets.chunks(per).enumerate() {
            let mut line = row().w_full().gap(tk.r(1.0));
            for (j, w) in chunk.iter().enumerate() {
                line = line.child(cell(r * per + j, w));
            }
            if chunk.len() < per {
                line = line.child(el().grow(1.0).w(0.0));
            }
            grid = grid.child(line);
        }
        c = c.child(alloc.child(grid));
    }
    c
}

fn balance(app: &App, tk: &Tk, v: &Visual, key: &str, expanded: bool) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let rows = v.rows();
    let hide = app.prefs.hide_sensitive();
    let mut assets: Vec<String> = vec![];
    for r in rows {
        let a = r["asset"].as_str().unwrap_or("").to_owned();
        if !assets.contains(&a) {
            assets.push(a);
        }
    }
    let single = assets.len() == 1;
    let measure_key = format!("{key}-measure");
    let given = v.visual["measure"].as_str().unwrap_or("quantity");
    let initial = if given == "market_value" && single {
        "compare"
    } else {
        given
    };
    let allowed: [&str; 3] = [
        "quantity",
        if single { "compare" } else { "market_value" },
        "cost_basis",
    ];
    let mut measure = choice(
        app,
        &measure_key,
        if allowed.contains(&initial) {
            initial
        } else {
            "quantity"
        },
    );
    if !allowed.contains(&measure.as_str()) {
        measure = "quantity".into();
    }
    let mut periods: Vec<String> = vec![];
    for r in rows {
        let pd = r["period"].as_str().unwrap_or("").to_owned();
        if !periods.contains(&pd) {
            periods.push(pd);
        }
    }
    let span = if periods.len() > 1 {
        span_days(&periods[0], &periods[periods.len() - 1])
    } else {
        0.0
    };
    let ranges = ranges_for(span);
    let range_key = format!("{key}-range");
    let range = choice(app, &range_key, "all");
    let end = periods.last().cloned().unwrap_or_default();
    let visible: Vec<String> = match range_days(&range) {
        Some(days) if !periods.is_empty() => periods
            .iter()
            .filter(|pd| span_days(pd, &end) <= days)
            .cloned()
            .collect(),
        _ => periods.clone(),
    };
    let field = match measure.as_str() {
        "quantity" => "quantity",
        "cost_basis" => "cost_basis",
        _ => "market_value",
    };
    let is_fiat = measure != "quantity";
    let at = |pd: &str, asset: &str, f: &str| {
        rows.iter()
            .find(|r| {
                r["period"].as_str() == Some(pd) && r["asset"].as_str().unwrap_or("") == asset
            })
            .and_then(|r| num(&r[f]))
    };
    let colors: HashMap<String, Color> = {
        // `seriesColorMap`: BTC first slot, Liquid second, Lightning fourth, others in order.
        let mut m = HashMap::new();
        let mut used = [false; 4];
        for a in &assets {
            let slot = match a.to_uppercase().as_str() {
                "BTC" => Some(0),
                "LBTC" | "L-BTC" => Some(1),
                "LNBTC" => Some(3),
                _ => None,
            };
            if let Some(sl) = slot.filter(|sl| !used[*sl]) {
                m.insert(a.clone(), p.series[sl]);
                used[sl] = true;
            }
        }
        for a in &assets {
            if m.contains_key(a) {
                continue;
            }
            match (0..4).find(|i| !used[*i]) {
                Some(sl) => {
                    m.insert(a.clone(), p.series[sl]);
                    used[sl] = true;
                }
                None => {
                    m.insert(a.clone(), p.muted);
                }
            }
        }
        m
    };
    let mut lines: Vec<Series> = assets
        .iter()
        .map(|a| Series {
            values: visible.iter().map(|pd| at(pd, a, field)).collect(),
            color: colors.get(a).copied().unwrap_or(p.muted),
            area: true,
            step: true,
            width: 2.0,
        })
        .collect();
    let mut legend_items: Vec<(String, Color)> = assets
        .iter()
        .map(|a| {
            (
                if measure == "compare" {
                    t("assistant:visual.measure.market_value")
                } else {
                    a.clone()
                },
                colors.get(a).copied().unwrap_or(p.muted),
            )
        })
        .collect();
    if measure == "compare" {
        // One asset: its cost basis as the step line beside the value.
        let a = &assets[0];
        lines.push(Series {
            values: visible.iter().map(|pd| at(pd, a, "cost_basis")).collect(),
            color: p.muted,
            area: false,
            step: true,
            width: 1.5,
        });
        legend_items.push((t("assistant:visual.measure.cost_basis"), p.muted));
    }
    // The headline: one asset's own figure; fiat measures add up across assets.
    let total = |pd: Option<&String>| -> Option<f64> {
        let pd = pd?;
        if single {
            return at(pd, &assets[0], field);
        }
        if !is_fiat {
            return None;
        }
        assets.iter().map(|a| at(pd, a, field)).sum::<Option<f64>>()
    };
    let first_v = total(visible.first());
    let last_v = total(visible.last());
    let first_asset = assets.first().cloned().unwrap_or_else(|| "BTC".into());
    let fmt_v = |x: f64| {
        if is_fiat {
            crate::format::fiat(x, &cur)
        } else {
            asset_amount(x, &first_asset)
        }
    };
    let style = match v.visual["interval"].as_str() {
        Some("month") => "month",
        Some("hour") => "hour",
        _ => "day",
    };
    let mut figure_row = row().flex_wrap().center_y().gap(tk.r(0.5));
    if single || is_fiat {
        figure_row = figure_row.child(figure(
            app,
            last_v.map(fmt_v).unwrap_or_else(|| "—".into()),
            rune::TextStyle::new(tk.r(1.5), 600.0)
                .lh(tk.r(2.0))
                .tracking(-0.025 * tk.r(1.5)),
            tk.fg,
        ));
    } else {
        figure_row = figure_row.child(
            text(t("assistant:visual.allocation.mixedAssets"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    }
    if let (Some(a), Some(b)) = (first_v, last_v) {
        let change = b - a;
        let label = tr(
            "assistant:visual.balance.change",
            &[
                ("change", &signed(fmt_v(change.abs()), change)),
                (
                    "date",
                    &date(visible.first().map(String::as_str).unwrap_or(""), style),
                ),
            ],
        );
        figure_row = figure_row.child(delta_pill(app, tk, label, change));
    }
    let head = col()
        .min_w(0.0)
        .child(
            text(tr(
                "assistant:visual.balance.latest",
                &[(
                    "date",
                    &date(visible.last().map(String::as_str).unwrap_or(""), style),
                )],
            ))
            .font(tk.t2xs(400.0))
            .color(tk.muted_fg),
        )
        .child(figure_row);
    let measure_options: Vec<(String, String)> = allowed
        .iter()
        .map(|m| (m.to_string(), t(&format!("assistant:visual.measure.{m}"))))
        .collect();
    let range_options = ranges
        .iter()
        .map(|r| (r.to_string(), t(&format!("assistant:visual.range.{r}"))))
        .collect();
    let controls = row()
        .flex_wrap()
        .center_y()
        .gap(tk.r(0.5))
        .child(segmented(
            tk,
            measure_key,
            t("assistant:visual.controls.measure"),
            &measure,
            measure_options,
        ))
        .child(segmented(
            tk,
            range_key,
            t("assistant:visual.controls.range"),
            &range,
            range_options,
        ));
    let labels: Vec<String> = visible.iter().map(|pd| date(pd, style)).collect();
    let y_fmt = |x: f64| {
        if hide {
            String::new()
        } else if is_fiat {
            crate::format::fiat_compact(x, &cur)
        } else {
            btc_compact(x)
        }
    };
    // One row per charted series, as recharts' payload lists them.
    let tip = |i: usize| {
        let pd = visible.get(i).cloned().unwrap_or_default();
        let fmt = |x: f64, asset: &str| {
            if is_fiat {
                crate::format::fiat(x, &cur)
            } else {
                asset_amount(x, asset)
            }
        };
        let mut rows: Vec<TipRow> = assets
            .iter()
            .map(|a| {
                let label = if measure == "compare" {
                    t("assistant:visual.measure.market_value")
                } else {
                    a.clone()
                };
                let value = at(&pd, a, field)
                    .map(|x| fmt(x, a))
                    .unwrap_or_else(|| t("assistant:visual.unknownValue"));
                tip_row(colors.get(a).copied().or(Some(p.muted)), label, value)
            })
            .collect();
        if measure == "compare" {
            let a = &assets[0];
            rows.push(tip_row(
                Some(p.muted),
                t("assistant:visual.measure.cost_basis"),
                at(&pd, a, "cost_basis")
                    .map(|x| crate::format::fiat(x, &cur))
                    .unwrap_or_else(|| t("assistant:visual.unknownValue")),
            ));
        }
        (date(&pd, style), rows)
    };
    col()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            row()
                .w_full()
                .flex_wrap()
                .items_end()
                .justify_between()
                .gap(tk.r(0.75))
                .child(head)
                .child(controls),
        )
        .child(axis_chart(
            app,
            tk,
            format!("{key}-chart{}", if expanded { "-x" } else { "" }),
            &lines,
            &labels,
            24.0,
            if expanded { tk.r(20.0) } else { tk.r(12.0) },
            (8.0, 12.0),
            &y_fmt,
            &tip,
        ))
        .child(if legend_items.len() >= 2 {
            legend(tk, &legend_items)
        } else {
            el()
        })
}

fn allocation(app: &App, tk: &Tk, v: &Visual, key: &str, wide: bool) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let mkey = format!("{key}-measure");
    let gkey = format!("{key}-group");
    let dm = v.visual["measure"].as_str().unwrap_or("market_value");
    let measure = choice(
        app,
        &mkey,
        if ["market_value", "quantity", "cost_basis", "unrealized_pnl"].contains(&dm) {
            dm
        } else {
            "market_value"
        },
    );
    let group = choice(
        app,
        &gkey,
        if v.visual["group_by"].as_str() == Some("asset") {
            "asset"
        } else {
            "wallet"
        },
    );
    // groupRows: fiat measures add up across assets; amounts keep one row per asset.
    let mut groups: Vec<(String, String, Option<String>, Option<f64>)> = vec![];
    for r in v.rows() {
        let wallet = r["wallet"].as_str().unwrap_or("").to_owned();
        let asset = r["asset"].as_str().unwrap_or("").to_owned();
        let split = measure == "quantity" || group == "asset";
        let gk = if group == "asset" {
            asset.clone()
        } else if split {
            format!("{wallet}\u{0}{asset}")
        } else {
            wallet.clone()
        };
        let part = num(&r[measure.as_str()]);
        match groups.iter_mut().find(|g| g.0 == gk) {
            Some(g) => g.3 = g.3.zip(part).map(|(a, b)| a + b),
            None => groups.push((
                gk,
                if group == "asset" {
                    asset.clone()
                } else {
                    wallet.clone()
                },
                split.then_some(asset.clone()),
                part,
            )),
        }
    }
    groups.sort_by(|a, b| {
        b.3.unwrap_or(0.0)
            .abs()
            .partial_cmp(&a.3.unwrap_or(0.0).abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });
    let known: Vec<_> = groups.iter().filter(|g| g.3.is_some()).collect();
    let total: f64 = known.iter().map(|g| g.3.unwrap_or(0.0)).sum();
    let positive: f64 = known.iter().map(|g| g.3.unwrap_or(0.0).max(0.0)).sum();
    let max_abs = known
        .iter()
        .map(|g| g.3.unwrap_or(0.0).abs())
        .fold(1e-12, f64::max);
    let is_fiat = measure != "quantity";
    let fmt_v = |x: f64, asset: Option<&str>| {
        if is_fiat {
            crate::format::fiat(x, &cur)
        } else {
            asset_amount(x, asset.unwrap_or("BTC"))
        }
    };
    let signed_m = measure == "unrealized_pnl";
    let mixed = !is_fiat
        && known
            .iter()
            .map(|g| g.2.clone())
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            > 1;
    let color_for = |i: usize, g: &(String, String, Option<String>, Option<f64>)| {
        let asset = g.2.as_deref().unwrap_or(&g.1).to_uppercase();
        if group == "asset" || g.2.is_some() {
            match asset.as_str() {
                "BTC" => p.series[0],
                "LBTC" | "L-BTC" => p.series[1],
                "LNBTC" => p.series[3],
                _ => p.series[2],
            }
        } else if i < 4 {
            p.series[i]
        } else {
            p.muted
        }
    };
    let mut total_col = col().grow(1.0).min_w(0.0).child(
        text(format!(
            "{}{}",
            t(&format!("assistant:visual.allocation.total.{measure}")),
            if known.len() < groups.len() {
                format!(" · {}", t("assistant:visual.allocation.pricedOnly"))
            } else {
                String::new()
            }
        ))
        .font(tk.t2xs(400.0))
        .color(tk.muted_fg),
    );
    if mixed {
        total_col = total_col.child(
            text(t("assistant:visual.allocation.mixedAssets"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    } else {
        let first_asset = known.first().and_then(|g| g.2.clone());
        let s = if known.is_empty() {
            "—".into()
        } else if signed_m {
            signed(fmt_v(total.abs(), first_asset.as_deref()), total)
        } else {
            fmt_v(total, first_asset.as_deref())
        };
        total_col = total_col.child(figure(
            app,
            s,
            rune::TextStyle::new(tk.r(1.5), 600.0).lh(tk.r(2.0)),
            tk.fg,
        ));
    }
    let controls = row()
        .flex_wrap()
        .gap(tk.r(0.5))
        .child(segmented(
            tk,
            gkey,
            t("assistant:visual.controls.groupBy"),
            &group,
            vec![
                ("wallet".into(), t("assistant:visual.allocation.byWallet")),
                ("asset".into(), t("assistant:visual.allocation.byAsset")),
            ],
        ))
        .child(segmented(
            tk,
            mkey,
            t("assistant:visual.controls.measure"),
            &measure,
            ["market_value", "quantity", "cost_basis", "unrealized_pnl"]
                .iter()
                .map(|m| (m.to_string(), t(&format!("assistant:visual.measure.{m}"))))
                .collect(),
        ));
    let mut c = col().w_full().gap(tk.r(0.75)).child(if wide {
        row()
            .w_full()
            .items_end()
            .gap(tk.r(0.75))
            .child(total_col)
            .child(controls)
    } else {
        col()
            .w_full()
            .gap(tk.r(0.5))
            .child(total_col)
            .child(controls)
    });
    let hide_shares = app.prefs.hide_shares();
    if !signed_m && positive > 0.0 && !hide_shares {
        let mut strip = row()
            .w_full()
            .h(12.0)
            .gap(2.0)
            .radius(6.0)
            .clip_rounded(6.0)
            .role("img")
            .label(t("assistant:visual.allocation.share"));
        for (i, g) in groups.iter().enumerate() {
            let val = g.3.unwrap_or(0.0);
            if val > 0.0 {
                strip = strip.child(
                    el().h_full()
                        .grow((val / positive) as f32)
                        .w(0.0)
                        .fill(color_for(i, g))
                        .help(g.1.clone()),
                );
            }
        }
        c = c.child(strip);
    }
    let mut list = col().w_full().gap(tk.r(0.375));
    for (i, g) in groups.iter().enumerate() {
        let val = g.3;
        let s = match val {
            None => t("assistant:visual.unknownValue"),
            Some(x) if signed_m => signed(fmt_v(x.abs(), g.2.as_deref()), x),
            Some(x) => fmt_v(x, g.2.as_deref()),
        };
        let mut name = col().w(tk.r(10.0)).min_w(0.0).child(
            text(g.1.clone())
                .font(tk.txs(500.0))
                .color(tk.fg)
                .truncate(),
        );
        if group == "wallet"
            && let Some(a) = &g.2
        {
            name = name.child(text(a.clone()).font(tk.t2xs(400.0)).color(tk.muted_fg));
        }
        let color = if signed_m {
            if val.unwrap_or(0.0) < 0.0 {
                p.negative
            } else {
                p.positive
            }
        } else {
            color_for(i, g)
        };
        list = list.child(
            row()
                .key(format!("{key}-row-{i}"))
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .child(el().size(8.0, 8.0).radius(2.0).fill(color).a11y_hidden())
                .child(name)
                .child(bar(
                    tk,
                    (val.unwrap_or(0.0).abs() / max_abs) as f32,
                    color,
                    6.0,
                ))
                .child(figure(app, s, tk.txs(400.0), tk.fg))
                .child(
                    text(if hide_shares || signed_m || positive <= 0.0 {
                        String::new()
                    } else {
                        percent(val.unwrap_or(0.0).max(0.0) / positive)
                    })
                    .font(tk.t2xs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg)
                    .w(tk.r(3.0))
                    .align(rune::Align::End),
                ),
        );
    }
    c.child(list)
}

fn tax_years(app: &App, tk: &Tk, v: &Visual, key: &str, expanded: bool) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let fiat = |x: f64| crate::format::fiat(x, &cur);
    let mkey = format!("{key}-measure");
    let ykey = format!("{key}-year");
    let dm = v.visual["measure"].as_str().unwrap_or("gain_loss");
    let measure = choice(
        app,
        &mkey,
        if ["gain_loss", "proceeds", "cost_basis"].contains(&dm) {
            dm
        } else {
            "gain_loss"
        },
    );
    let years = v.rows();
    let last_active = years
        .iter()
        .rev()
        .find(|y| {
            num(&y["gain_loss"]).unwrap_or(0.0) != 0.0 || num(&y["proceeds"]).unwrap_or(0.0) != 0.0
        })
        .or(years.last());
    let default_year = last_active
        .and_then(|y| num(&y["year"]))
        .map(|y| (y as i64).to_string())
        .unwrap_or_default();
    let selected = choice(app, &ykey, &default_year);
    let color_for = |x: f64| {
        if measure != "gain_loss" {
            p.series[1]
        } else if x < 0.0 {
            p.negative
        } else {
            p.positive
        }
    };
    let vals: Vec<f64> = years
        .iter()
        .map(|y| num(&y[measure.as_str()]).unwrap_or(0.0))
        .collect();
    let max = vals.iter().fold(0.0_f64, |a, b| a.max(*b));
    let min = vals.iter().fold(0.0_f64, |a, b| a.min(*b));
    let span = (max - min).max(1e-9);
    let height = if expanded { tk.r(16.0) } else { tk.r(10.0) };
    let zero = (max / span) as f32 * height;
    let mut bars = row()
        .w_full()
        .h(height + tk.r(1.25))
        .gap(tk.r(0.5))
        .border_sides([0.0, 0.0, 0.0, 0.0], Color::TRANSPARENT);
    let chart_key = format!("{key}-bars{}", if expanded { "-x" } else { "" });
    let hover = hovered(app, &chart_key);
    let signed_fiat = |x: f64| signed(fiat(x.abs()), x);
    let chart_w = app
        .assistant
        .thread
        .chart_w
        .get(&chart_key)
        .copied()
        .unwrap_or(560.0);
    let mut tooltip = None;
    for (i, y) in years.iter().enumerate() {
        let yr = num(&y["year"])
            .map(|y| (y as i64).to_string())
            .unwrap_or_default();
        let val = vals[i];
        let hpx = ((val.abs() / span) as f32 * height).max(if val != 0.0 { 1.0 } else { 0.0 });
        let top = if val >= 0.0 { zero - hpx } else { zero };
        let on = selected.is_empty() || selected == yr;
        let yk = ykey.clone();
        let yv = yr.clone();
        let (hk, hk2) = (chart_key.clone(), chart_key.clone());
        if hover == Some(i) {
            // The tooltip's rows (TaxYearsVisual): gain/loss in its colour, proceeds, cost basis.
            let or_unknown =
                |v: Option<String>| v.unwrap_or_else(|| t("assistant:visual.unknownValue"));
            let gain = num(&y["gain_loss"]);
            let rows = vec![
                tip_row(
                    Some(if gain.unwrap_or(0.0) < 0.0 {
                        p.negative
                    } else {
                        p.positive
                    }),
                    t("assistant:visual.measure.gain_loss"),
                    or_unknown(gain.map(signed_fiat)),
                ),
                tip_row(
                    None,
                    t("assistant:visual.measure.proceeds"),
                    or_unknown(num(&y["proceeds"]).map(fiat)),
                ),
                tip_row(
                    None,
                    t("assistant:visual.measure.cost_basis"),
                    or_unknown(num(&y["cost_basis"]).map(fiat)),
                ),
            ];
            let (frame, w) = tooltip_frame(app, tk, yr.clone(), &rows);
            let n = years.len().max(1) as f32;
            let gap = tk.r(0.5);
            let at = (chart_w - gap * (n - 1.0)) / n * (i as f32 + 0.5) + gap * i as f32;
            tooltip = Some(place_tooltip(frame, w, at, chart_w, 0.0));
        }
        bars = bars.child(
            col()
                .key(format!("{key}-bar-{yr}"))
                // `cursor={{ fill: "var(--muted)", opacity: 0.4 }}`: the hovered band.
                .fill(if hover == Some(i) {
                    tk.muted.alpha(tk.muted.a * 0.4)
                } else {
                    Color::TRANSPARENT
                })
                .on_hover_change(move |s: &mut App, on, tx: &mut Tx| {
                    set_hover(s, if on { &hk } else { &hk2 }, on.then_some(i), tx)
                })
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .h_full()
                .items_center()
                .role("button")
                .label(yr.clone())
                .checked(selected == yr)
                .focusable(true)
                .on_press(move |a: &mut App| {
                    a.assistant.thread.choices.insert(yk.clone(), yv.clone());
                })
                .child(
                    col().w_full().h(height).items_center().child(
                        el().w(24.0_f32.min(tk.r(1.5)))
                            .h(hpx)
                            .mt(top)
                            .radius(4.0)
                            .fill(color_for(val).alpha(if on { 1.0 } else { 0.35 })),
                    ),
                )
                .child(
                    text(yr)
                        .font(tk.px(10.0, 400.0))
                        .color(tk.muted_fg)
                        .mt(tk.r(0.25)),
                ),
        );
    }
    let ck = chart_key.clone();
    let bars = bars.on_layout(move |s: &mut App, r: [f32; 4]| {
        if s.assistant
            .thread
            .chart_w
            .get(&ck)
            .is_none_or(|w| (w - r[2]).abs() > 0.5)
        {
            s.assistant.thread.chart_w.insert(ck.clone(), r[2]);
        }
    });
    // One structure either way, so the bars don't remount when the tooltip comes and goes.
    let bars = col().w_full().child(bars).children(tooltip);
    let mut c = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            row()
                .w_full()
                .flex_wrap()
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(t("assistant:visual.tax.pickYear"))
                        .font(tk.t2xs(400.0))
                        .color(tk.muted_fg)
                        .grow(1.0),
                )
                .child(segmented(
                    tk,
                    mkey,
                    t("assistant:visual.controls.measure"),
                    &measure,
                    ["gain_loss", "proceeds", "cost_basis"]
                        .iter()
                        .map(|m| (m.to_string(), t(&format!("assistant:visual.measure.{m}"))))
                        .collect(),
                )),
        )
        .child(bars);
    if let Some(y) = years.iter().find(|y| {
        num(&y["year"]).map(|n| (n as i64).to_string()).as_deref() == Some(selected.as_str())
    }) {
        c = c
            .child(
                text(tr(
                    "assistant:visual.tax.yearDetail",
                    &[("year", &selected)],
                ))
                .font(tk.txs(500.0))
                .color(tk.fg),
            )
            .child(tiles(
                vec![
                    stat_tile(
                        app,
                        tk,
                        t("assistant:visual.measure.proceeds"),
                        num(&y["proceeds"]).map(fiat),
                        None,
                        None,
                    ),
                    stat_tile(
                        app,
                        tk,
                        t("assistant:visual.measure.cost_basis"),
                        num(&y["cost_basis"]).map(fiat),
                        None,
                        None,
                    ),
                    stat_tile(
                        app,
                        tk,
                        t("assistant:visual.measure.gain_loss"),
                        num(&y["gain_loss"]).map(signed_fiat),
                        None,
                        None,
                    ),
                ],
                tk,
                3,
            ));
        let mut groups: Vec<(String, String, f64)> = vec![];
        for d in v.data["details"].as_array().into_iter().flatten() {
            if num(&d["year"]).map(|n| (n as i64).to_string()).as_deref() != Some(selected.as_str())
            {
                continue;
            }
            let ty = d["capital_gains_type"]
                .as_str()
                .filter(|s| !s.is_empty())
                .or(d["transaction_type"].as_str())
                .unwrap_or("")
                .to_owned();
            let asset = d["asset"].as_str().unwrap_or("—").to_owned();
            let val = num(&d[measure.as_str()]).unwrap_or(0.0);
            match groups.iter_mut().find(|g| g.0 == asset && g.1 == ty) {
                Some(g) => g.2 += val,
                None => groups.push((asset, ty, val)),
            }
        }
        groups.sort_by(|a, b| {
            b.2.abs()
                .partial_cmp(&a.2.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let max_abs = groups.iter().map(|g| g.2.abs()).fold(1e-9, f64::max);
        let mut list = col().w_full().gap(tk.r(0.25));
        for (asset, ty, val) in groups {
            let tkey = format!("assistant:visual.tax.type.{ty}");
            let tlabel = if crate::i18n::has(&tkey) {
                t(&tkey)
            } else {
                ty.clone()
            };
            list = list.child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        row()
                            .w(tk.r(10.0))
                            .min_w(0.0)
                            .child(text(asset).font(tk.txs(500.0)).color(tk.fg))
                            .child(
                                text(if tlabel.is_empty() {
                                    String::new()
                                } else {
                                    format!(" · {tlabel}")
                                })
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .truncate(),
                            ),
                    )
                    .child(bar(tk, (val.abs() / max_abs) as f32, color_for(val), 6.0))
                    .child(figure(
                        app,
                        if measure == "gain_loss" {
                            signed(fiat(val.abs()), val)
                        } else {
                            fiat(val)
                        },
                        tk.txs(400.0),
                        tk.fg,
                    )),
            );
        }
        c = c.child(list);
    }
    c
}

fn cash_flow(app: &App, tk: &Tk, v: &Visual, expanded: bool) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let tot = &v.data["totals"];
    let stat = |k: &str| {
        (
            num(&tot[k]["btc"]).unwrap_or(0.0),
            num(&tot[k]["fiat"]).unwrap_or(0.0),
            num(&tot[k]["count"]).unwrap_or(0.0) as usize,
        )
    };
    let (in_btc, in_fiat, in_n) = stat("incoming");
    let (out_btc, out_fiat, out_n) = stat("outgoing");
    let net_btc = in_btc - out_btc;
    let mut c = col().w_full().gap(tk.r(0.75)).child(tiles(
        vec![
            stat_tile(
                app,
                tk,
                t("assistant:visual.cashFlow.received"),
                Some(btc(in_btc)),
                Some(format!(
                    "{} · {}",
                    crate::format::fiat(in_fiat, &cur),
                    tc("assistant:visual.cashFlow.count", in_n, &[])
                )),
                None,
            ),
            stat_tile(
                app,
                tk,
                t("assistant:visual.cashFlow.sent"),
                Some(btc(out_btc)),
                Some(format!(
                    "{} · {}",
                    crate::format::fiat(out_fiat, &cur),
                    tc("assistant:visual.cashFlow.count", out_n, &[])
                )),
                None,
            ),
            stat_tile(
                app,
                tk,
                t("assistant:visual.cashFlow.net"),
                Some(signed(btc(net_btc.abs()), net_btc)),
                Some(signed(
                    crate::format::fiat((in_fiat - out_fiat).abs(), &cur),
                    in_fiat - out_fiat,
                )),
                None,
            ),
        ],
        tk,
        3,
    ));
    let rows = v.rows();
    if !rows.is_empty() {
        let max = rows
            .iter()
            .map(|r| {
                num(&r["in_btc"])
                    .unwrap_or(0.0)
                    .max(num(&r["out_btc"]).unwrap_or(0.0))
            })
            .fold(1e-12, f64::max);
        let h = tk.r(7.0);
        let bucket = v.visual["bucket"].as_str().unwrap_or("month");
        let mut chart = row().w_full().h(h * 2.0 + tk.r(1.25)).gap(tk.r(0.25));
        let chart_key = format!(
            "asst-visual-{}-flow{}",
            v.call_id,
            if expanded { "-x" } else { "" }
        );
        let hover = hovered(app, &chart_key);
        let chart_w = app
            .assistant
            .thread
            .chart_w
            .get(&chart_key)
            .copied()
            .unwrap_or(560.0);
        let mut tooltip = None;
        let n = rows.len();
        for (idx, r) in rows.iter().enumerate() {
            let (hk, hk2) = (chart_key.clone(), chart_key.clone());
            if hover == Some(idx) {
                // CashFlowVisual's rows: received and sent with their counts, net, then the
                // transfers and unpriced rows when there are any.
                let (inv, outv) = (
                    num(&r["in_btc"]).unwrap_or(0.0),
                    num(&r["out_btc"]).unwrap_or(0.0),
                );
                let count = |k: &str| num(&r[k]).unwrap_or(0.0).max(0.0) as usize;
                let mut tip_rows = vec![
                    tip_row(
                        Some(p.positive),
                        tc(
                            "assistant:visual.cashFlow.receivedCount",
                            count("in_count"),
                            &[],
                        ),
                        btc(inv),
                    ),
                    tip_row(
                        Some(p.negative),
                        tc(
                            "assistant:visual.cashFlow.sentCount",
                            count("out_count"),
                            &[],
                        ),
                        btc(-outv),
                    ),
                    tip_row(
                        None,
                        t("assistant:visual.cashFlow.net"),
                        signed(btc((inv - outv).abs()), inv - outv),
                    ),
                ];
                if count("transfer_count") > 0 {
                    tip_rows.push(tip_row(
                        None,
                        tc(
                            "assistant:visual.cashFlow.transfers",
                            count("transfer_count"),
                            &[],
                        ),
                        count("transfer_count").to_string(),
                    ));
                }
                let (frame, w) = tooltip_frame(
                    app,
                    tk,
                    date(r["period"].as_str().unwrap_or(""), bucket),
                    &tip_rows,
                );
                let gap = tk.r(0.25);
                let at = (chart_w - gap * (n as f32 - 1.0)) / n.max(1) as f32 * (idx as f32 + 0.5)
                    + gap * idx as f32;
                tooltip = Some(place_tooltip(frame, w, at, chart_w, 0.0));
            }
            let i = (num(&r["in_btc"]).unwrap_or(0.0) / max) as f32 * h;
            let o = (num(&r["out_btc"]).unwrap_or(0.0) / max) as f32 * h;
            chart = chart.child(
                col()
                    .grow(1.0)
                    .w(0.0)
                    .min_w(0.0)
                    .items_center()
                    .fill(if hover == Some(idx) {
                        tk.muted.alpha(tk.muted.a * 0.4)
                    } else {
                        Color::TRANSPARENT
                    })
                    .on_hover_change(move |s: &mut App, on, tx: &mut Tx| {
                        set_hover(s, if on { &hk } else { &hk2 }, on.then_some(idx), tx)
                    })
                    .child(
                        col()
                            .w_full()
                            .h(h)
                            .justify_end()
                            .items_center()
                            .child(el().w(24.0).h(i).radius(2.0).fill(p.positive)),
                    )
                    .child(el().w_full().h(1.0).fill(tk.border))
                    .child(
                        col()
                            .w_full()
                            .h(h)
                            .items_center()
                            .child(el().w(24.0).h(o).radius(2.0).fill(p.negative)),
                    )
                    .child(
                        text(date(r["period"].as_str().unwrap_or(""), bucket))
                            .font(tk.px(9.0, 400.0))
                            .color(tk.muted_fg)
                            .truncate(),
                    ),
            );
        }
        let ck = chart_key.clone();
        let chart = chart.on_layout(move |s: &mut App, r: [f32; 4]| {
            if s.assistant
                .thread
                .chart_w
                .get(&ck)
                .is_none_or(|w| (w - r[2]).abs() > 0.5)
            {
                s.assistant.thread.chart_w.insert(ck.clone(), r[2]);
            }
        });
        // One structure either way, so the bars don't remount when the tooltip comes and goes.
        let chart = col().w_full().child(chart).children(tooltip);
        c = c
            .child(legend(
                tk,
                &[
                    (t("assistant:visual.cashFlow.received"), p.positive),
                    (t("assistant:visual.cashFlow.sent"), p.negative),
                ],
            ))
            .child(chart);
    }
    let transfers = num(&tot["transfers"]["count"]).unwrap_or(0.0) as usize;
    let swaps = num(&tot["swaps"]["count"]).unwrap_or(0.0) as usize;
    let mut notes = vec![];
    if transfers > 0 {
        notes.push(tc(
            "assistant:visual.cashFlow.transfersExcluded",
            transfers,
            &[],
        ));
    }
    if swaps > 0 {
        notes.push(tc("assistant:visual.cashFlow.swapsExcluded", swaps, &[]));
    }
    if !notes.is_empty() {
        c = c.child(
            text(notes.join(" · "))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    c
}

fn transactions(app: &App, tk: &Tk, v: &Visual) -> El {
    let p = palette(tk);
    let cur = v.fiat();
    let s = &v.data["summary"];
    let mut head = row().flex_wrap().gap(tk.r(0.75)).center_y().child(
        text(tc(
            "assistant:visual.transactions.shown",
            num(&s["shown"]).unwrap_or(0.0) as usize,
            &[(
                "total",
                &crate::format::count(num(&s["total"]).unwrap_or(0.0) as usize),
            )],
        ))
        .font(tk.txs(400.0))
        .color(tk.muted_fg),
    );
    if let Some(i) = num(&s["in_sat"]).filter(|x| *x != 0.0) {
        head = head.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(&I::ARROW_DOWN_LEFT, tk.r(0.75), p.positive).a11y_hidden())
                .child(figure(app, btc(i / 1e8), tk.txs(400.0), tk.muted_fg)),
        );
    }
    if let Some(o) = num(&s["out_sat"]).filter(|x| *x != 0.0) {
        head = head.child(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .child(ic(&I::ARROW_UP_RIGHT, tk.r(0.75), p.negative).a11y_hidden())
                .child(figure(app, btc(o / 1e8), tk.txs(400.0), tk.muted_fg)),
        );
    }
    let rows = v.rows();
    let max = rows
        .iter()
        .map(|r| num(&r["amount_sat"]).unwrap_or(0.0).abs())
        .fold(1.0, f64::max);
    let mut list = col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .clip_rounded(tk.r(0.375));
    for (i, r) in rows.iter().enumerate() {
        let dir = r["direction"].as_str().unwrap_or("none");
        let dir = if dir == "in" || dir == "out" {
            dir
        } else {
            "none"
        };
        let color = match dir {
            "in" => p.positive,
            "out" => p.negative,
            _ => p.muted,
        };
        let glyph: &'static rune::Icon = match dir {
            "in" => &I::ARROW_DOWN_LEFT,
            "out" => &I::ARROW_UP_RIGHT,
            _ => &I::MINUS,
        };
        let title = [
            r["counterparty"].as_str(),
            r["note"].as_str(),
            r["type"].as_str(),
        ]
        .into_iter()
        .flatten()
        .find(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t(&format!("assistant:visual.transactions.direction.{dir}")));
        let amount_sat = num(&r["amount_sat"]).unwrap_or(0.0);
        let amount = signed(
            asset_amount(amount_sat.abs() / 1e8, r["asset"].as_str().unwrap_or("")),
            amount_sat,
        );
        let status = r["status"]
            .as_str()
            .filter(|s| !s.is_empty() && *s != "completed");
        let mut meta: Vec<String> = vec![];
        if let Some(w) = r["wallet"].as_str().filter(|s| !s.is_empty()) {
            meta.push(w.to_owned());
        }
        if let Some(d) = r["date"].as_str() {
            meta.push(date(d, "short"));
        }
        meta.extend(
            r["tags"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .take(2)
                .map(str::to_owned),
        );
        let mut name = row().gap(tk.r(0.375)).center_y().child(
            text(title.clone())
                .font(tk.txs(500.0))
                .color(tk.fg)
                .truncate(),
        );
        if let Some(st) = status {
            let k = format!("assistant:visual.transactions.status.{st}");
            name = name.child(
                row().radius(999.0).fill(tk.warn.bg).px(tk.r(0.375)).child(
                    text(if crate::i18n::has(&k) {
                        t(&k)
                    } else {
                        st.to_owned()
                    })
                    .font(tk.t2xs(400.0))
                    .color(tk.warn.text),
                ),
            );
        }
        let mut amt = col()
            .items_end()
            .child(figure(app, amount, tk.txs(500.0), tk.fg));
        if let Some(f) = num(&r["fiat_value"]) {
            amt = amt.child(figure(
                app,
                crate::format::fiat(f.abs(), &cur),
                tk.t2xs(400.0),
                tk.muted_fg,
            ));
        }
        let id = r["id"].as_str().unwrap_or("").to_owned();
        list = list.child(
            col()
                .key(format!("asst-vtx-{}-{i}", v.call_id))
                .w_full()
                .border_sides(
                    [if i > 0 { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0],
                    tk.border.alpha(tk.border.a * 0.5),
                )
                .hover(tk.muted.alpha(0.4))
                .role("button")
                .label(tr(
                    "assistant:visual.transactions.open",
                    &[("title", &title)],
                ))
                .focusable(true)
                .on_press(move |a: &mut App| super::open_transaction(a, &id))
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.75))
                        .center_y()
                        .px(tk.r(0.75))
                        .py(tk.r(0.5))
                        .child(
                            row()
                                .size(tk.r(1.75), tk.r(1.75))
                                .center()
                                .radius(999.0)
                                .fill(color.alpha(0.16))
                                .child(ic(glyph, tk.r(0.875), color))
                                .a11y_hidden(),
                        )
                        .child(
                            col().grow(1.0).min_w(0.0).child(name).child(
                                text(meta.join(" · "))
                                    .font(tk.t2xs(400.0))
                                    .color(tk.muted_fg)
                                    .truncate(),
                            ),
                        )
                        .child(amt),
                )
                .child(
                    row().w_full().px(tk.r(0.75)).h(2.0).child(
                        el().h_full()
                            .w_frac(((amount_sat.abs() / max) as f32).clamp(0.0, 1.0))
                            .radius(1.0)
                            .fill(color.alpha(0.5)),
                    ),
                ),
        );
    }
    let mut c = col().w_full().gap(tk.r(0.5)).child(head);
    if !rows.is_empty() {
        c = c.child(list);
    }
    c
}

fn flow(app: &App, tk: &Tk, v: &Visual) -> El {
    let p = palette(tk);
    let rows = v.rows();
    let label_of = |n: &Value| -> String {
        let side = n["side"].as_str().unwrap_or("");
        let pos = num(&n["position"]).unwrap_or(0.0) as usize + 1;
        match n["label_kind"].as_str().unwrap_or("") {
            "wallet" => n["label"].as_str().unwrap_or("").to_owned(),
            "fee" => t("assistant:visual.flow.fee"),
            "overflow" => tc(
                "assistant:visual.flow.overflow",
                num(&n["overflow_count"]).unwrap_or(0.0) as usize,
                &[],
            ),
            _ => tr(
                if side == "input" {
                    "assistant:visual.flow.input"
                } else {
                    "assistant:visual.flow.output"
                },
                &[("position", &pos.to_string())],
            ),
        }
    };
    let total_in: f64 = rows
        .iter()
        .filter(|n| n["side"].as_str() == Some("input"))
        .filter_map(|n| num(&n["value_sats"]))
        .sum();
    let column = |side: &str| {
        let mut c = col().grow(1.0).w(0.0).min_w(0.0).gap(tk.r(0.375));
        for n in rows.iter().filter(|n| {
            n["side"].as_str() == Some(side)
                || (side == "output" && n["side"].as_str() == Some("fee"))
        }) {
            let yours = n["ownership"].as_str() == Some("owned")
                || n["label_kind"].as_str() == Some("wallet");
            let fee = n["side"].as_str() == Some("fee");
            let color = if fee {
                p.negative
            } else if yours {
                p.series[0]
            } else {
                p.muted
            };
            let sats = num(&n["value_sats"]);
            c = c.child(
                col()
                    .w_full()
                    .radius(tk.r(0.375))
                    .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                    .border_layout()
                    .fill(tk.background.alpha(0.4))
                    .px(tk.r(0.625))
                    .py(tk.r(0.375))
                    .gap(2.0)
                    .child(
                        row()
                            .gap(tk.r(0.375))
                            .center_y()
                            .child(el().size(8.0, 8.0).radius(4.0).fill(color).a11y_hidden())
                            .child(
                                text(label_of(n))
                                    .font(tk.txs(500.0))
                                    .color(tk.fg)
                                    .truncate(),
                            ),
                    )
                    .child(figure(
                        app,
                        sats.map(|s| format!("{} sat", crate::format::count(s as usize)))
                            .unwrap_or_else(|| t("assistant:visual.unknownValue")),
                        tk.t2xs(400.0),
                        tk.muted_fg,
                    ))
                    .child(
                        row().w_full().h(3.0).child(
                            el().h_full()
                                .w_frac(if total_in > 0.0 {
                                    (sats.unwrap_or(0.0) / total_in) as f32
                                } else {
                                    0.0
                                })
                                .radius(1.5)
                                .fill(color),
                        ),
                    ),
            );
        }
        c
    };
    let n_in = rows
        .iter()
        .filter(|n| n["side"].as_str() == Some("input"))
        .count();
    let n_out = rows
        .iter()
        .filter(|n| n["side"].as_str() == Some("output"))
        .count();
    let mut c = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(legend(
            tk,
            &[
                (t("assistant:visual.flow.yours"), p.series[0]),
                (t("assistant:visual.flow.others"), p.muted),
                (t("assistant:visual.flow.fee"), p.negative),
            ],
        ))
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .center_y()
                .role("img")
                .label(tr(
                    "assistant:visual.flow.aria",
                    &[
                        ("inputs", &n_in.to_string()),
                        ("outputs", &n_out.to_string()),
                    ],
                ))
                .child(column("input"))
                .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg).a11y_hidden())
                .child(column("output")),
        );
    let tx = &v.data["transaction"];
    if tx.is_object() {
        let mut facts = vec![];
        if let Some(f) = num(&tx["fiat_value"]) {
            facts.push(stat_tile(
                app,
                tk,
                t("assistant:visual.flow.fiatValue"),
                Some(crate::format::fiat(f.abs(), &v.fiat())),
                None,
                None,
            ));
        }
        if let Some(cp) = tx["counterparty"].as_str().filter(|s| !s.is_empty()) {
            facts.push(stat_tile(
                app,
                tk,
                t("assistant:visual.flow.counterparty"),
                Some(cp.to_owned()),
                None,
                None,
            ));
        }
        if !facts.is_empty() {
            let n = facts.len();
            c = c.child(tiles(facts, tk, n));
        }
    }
    c
}

fn readiness(app: &App, tk: &Tk, v: &Visual) -> El {
    let s = &v.data["summary"];
    let passed = num(&s["passed"]).unwrap_or(0.0);
    let total = num(&s["total"]).unwrap_or(0.0);
    let frac = if total > 0.0 {
        (passed / total) as f32
    } else {
        0.0
    };
    let checks = v.rows();
    let open = checks
        .iter()
        .filter(|c| c["status"].as_str() != Some("ok"))
        .count();
    let blocked = checks
        .iter()
        .any(|c| c["status"].as_str() == Some("blocked"));
    let ready = s["ready"].as_bool() == Some(true) && open == 0;
    let ok_c = if tk.dark {
        rune::color::hex("#34d399")
    } else {
        rune::color::hex("#059669")
    }
    .unwrap_or(tk.good.solid);
    let warn_c = if tk.dark {
        rune::color::hex("#fbbf24")
    } else {
        rune::color::hex("#d97706")
    }
    .unwrap_or(tk.warn.solid);
    let bad_c = if tk.dark {
        rune::color::hex("#f87171")
    } else {
        rune::color::hex("#dc2626")
    }
    .unwrap_or(tk.alert.solid);
    let ring_c = if ready {
        ok_c
    } else if blocked {
        bad_c
    } else {
        warn_c
    };
    // The ring: a muted track and the passed share, from 12 o'clock.
    let arc = |from: f32, to: f32| -> Vec<[f32; 2]> {
        let steps = 64;
        (0..=steps)
            .map(|i| {
                let a = (from + (to - from) * i as f32 / steps as f32) * std::f32::consts::TAU
                    - std::f32::consts::FRAC_PI_2;
                [32.0 + 26.0 * a.cos(), 32.0 + 26.0 * a.sin()]
            })
            .collect()
    };
    let mut shape = Shape::new()
        .view_box(64.0, 64.0)
        .stroke(arc(0.0, 1.0), 6.0, tk.muted);
    if frac > 0.0 {
        shape = shape.stroke(arc(0.0, frac), 6.0, ring_c);
    }
    let progress = tr(
        "assistant:visual.readiness.progress",
        &[
            ("passed", &(passed as i64).to_string()),
            ("total", &(total as i64).to_string()),
        ],
    );
    let head = row()
        .w_full()
        .gap(tk.r(1.0))
        .center_y()
        .child(
            canvas(shape)
                .size(tk.r(4.0), tk.r(4.0))
                .role("img")
                .label(progress.clone()),
        )
        .child(
            col()
                .min_w(0.0)
                .child(
                    text(if ready {
                        t("assistant:visual.readiness.ready")
                    } else {
                        tc("assistant:visual.readiness.open", open, &[])
                    })
                    .font(tk.tbase(600.0))
                    .color(tk.fg),
                )
                .child(text(progress).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
    let mut list = col().w_full().gap(tk.r(0.375));
    for c in checks {
        let status = c["status"].as_str().unwrap_or("ok");
        let check = c["check"].as_str().unwrap_or("");
        let count = num(&c["count"]).unwrap_or(0.0) as usize;
        let (glyph, color, border, fill): (&'static rune::Icon, Color, Color, Color) = match status
        {
            "ok" => (
                &I::CIRCLE_CHECK,
                ok_c,
                tk.border.alpha(tk.border.a * 0.5),
                Color::TRANSPARENT,
            ),
            "warning" => (
                &I::CIRCLE_ALERT,
                warn_c,
                tk.warn.border.alpha(0.3),
                tk.warn.bg.alpha(0.05),
            ),
            _ => (
                &I::OCTAGON_X,
                bad_c,
                tk.alert.border.alpha(0.3),
                tk.alert.bg.alpha(0.05),
            ),
        };
        let detail_s = c["detail"].as_str();
        let detail = match check {
            "blocker" => detail_s.map(str::to_owned),
            "journals" => match (detail_s, status) {
                (None, "ok") => None,
                (None, _) => Some(t("assistant:visual.readiness.detail.journals.never")),
                (Some(d), "ok") => Some(tr(
                    "assistant:visual.readiness.detail.journals.ok",
                    &[("date", &date(d, "short"))],
                )),
                (Some(d), _) => Some(tr(
                    "assistant:visual.readiness.detail.journals.warning",
                    &[("date", &date(d, "short"))],
                )),
            },
            "prices" if status == "ok" => Some(t("assistant:visual.readiness.detail.pricesOk")),
            "prices" => Some(tc(
                "assistant:visual.readiness.detail.prices",
                count,
                &[("coverable", detail_s.unwrap_or("0"))],
            )),
            _ => {
                let k = format!("assistant:visual.readiness.detail.{check}.{status}");
                (crate::i18n::has(&k) || crate::i18n::has(&format!("{k}_one")))
                    .then(|| tc(&k, count, &[]))
            }
        };
        let title = if check == "blocker" {
            c["title"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:visual.readiness.check.blocker"))
        } else {
            t(&format!("assistant:visual.readiness.check.{check}"))
        };
        let mut info = col()
            .grow(1.0)
            .min_w(0.0)
            .child(text(title).font(tk.txs(500.0)).color(tk.fg));
        if let Some(d) = detail.filter(|d| !d.is_empty()) {
            info = info.child(text(d).font(tk.t2xs(400.0)).color(tk.muted_fg).wrap());
        }
        let mut li = row()
            .w_full()
            .gap(tk.r(0.625))
            .radius(tk.r(0.375))
            .border(1.0, border)
            .border_layout()
            .fill(fill)
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .child(ic(glyph, tk.r(1.0), color).mt(2.0).a11y_hidden())
            .child(info);
        // A check's action chip asks its follow-up through the session (`onAsk`).
        if let Some(action) = c["action"].as_str().filter(|a| !a.is_empty()) {
            let label = t(&format!("assistant:visual.readiness.action.{action}"));
            let prompt = t(&format!("assistant:visual.readiness.prompt.{action}"));
            li = li.child(
                row()
                    .key(format!("asst-visual-{}-check-{check}-{action}", v.call_id))
                    .gap(tk.r(0.25))
                    .center_y()
                    .radius(999.0)
                    .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                    .border_layout()
                    .fill(tk.background.alpha(0.6))
                    .hover(tk.muted)
                    .px(tk.r(0.5))
                    .py(2.0)
                    .role("button")
                    .label(label.clone())
                    .focusable(true)
                    .on_press(move |a: &mut App| ask(a, &prompt))
                    .child(ic(&I::SPARKLES, tk.r(0.75), tk.fg).a11y_hidden())
                    .child(text(label).font(tk.t2xs(500.0)).color(tk.fg)),
            );
        }
        let _ = app;
        list = list.child(li);
    }
    col().w_full().gap(tk.r(0.75)).child(head).child(list)
}

// ---- The table twin (visuals/table.ts) ----------------------------------------------------------

pub struct Table {
    pub columns: Vec<String>,
    /// (display, raw for CSV, numeric).
    pub rows: Vec<Vec<(String, Option<String>, bool)>>,
}

pub fn table(v: &Visual) -> Table {
    let unknown = t("assistant:visual.unknownValue");
    let cur = v.fiat();
    let fcell = |x: Option<f64>| {
        (
            x.map(|x| crate::format::fiat(x, &cur))
                .unwrap_or_else(|| unknown.clone()),
            x.map(|x| x.to_string()),
            true,
        )
    };
    let ncell = |x: Option<f64>, f: &dyn Fn(f64) -> String, unk: &str| {
        (
            x.map(f).unwrap_or_else(|| unk.to_owned()),
            x.map(|x| x.to_string()),
            true,
        )
    };
    let tcell = |s: Option<&str>| {
        (
            s.filter(|s| !s.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| "—".into()),
            s.map(str::to_owned),
            false,
        )
    };
    let col_t = |k: &str| t(&format!("assistant:visual.{k}"));
    let rows = v.rows();
    match v.source {
        "overview" => {
            let s = &v.data["stats"];
            let mut out = vec![
                vec![
                    tcell(Some(&col_t("overview.portfolioValue"))),
                    fcell(num(&s["portfolio_value"])),
                ],
                vec![
                    tcell(Some(&col_t("measure.quantity"))),
                    ncell(num(&s["btc_balance"]), &btc, &unknown),
                ],
                vec![
                    tcell(Some(&col_t("overview.costBasis"))),
                    fcell(num(&s["cost_basis"])),
                ],
                vec![
                    tcell(Some(&col_t("overview.unrealized"))),
                    fcell(num(&s["unrealized"])),
                ],
                vec![
                    tcell(Some(&col_t("overview.realizedYtd"))),
                    fcell(num(&s["realized_ytd"])),
                ],
                vec![
                    tcell(Some(&col_t("overview.btcPrice"))),
                    fcell(num(&s["btc_price"])),
                ],
            ];
            for w in v.data["wallets"].as_array().into_iter().flatten() {
                let label = w["label"].as_str().map(str::to_owned).unwrap_or_else(|| {
                    tc(
                        "assistant:visual.overview.otherWallets",
                        num(&w["other_count"]).unwrap_or(0.0) as usize,
                        &[],
                    )
                });
                out.push(vec![
                    tcell(Some(&label)),
                    ncell(num(&w["balance_btc"]), &btc, &unknown),
                ]);
            }
            Table {
                columns: vec![col_t("table.metric"), col_t("table.value")],
                rows: out,
            }
        }
        "balance_history" => Table {
            columns: vec![
                col_t("table.period"),
                col_t("table.asset"),
                col_t("measure.quantity"),
                col_t("measure.market_value"),
                col_t("measure.cost_basis"),
            ],
            rows: rows
                .iter()
                .map(|r| {
                    let asset = r["asset"].as_str().unwrap_or("").to_owned();
                    let a2 = asset.clone();
                    vec![
                        (
                            date(
                                r["period"].as_str().unwrap_or(""),
                                if v.visual["interval"].as_str() == Some("month") {
                                    "month"
                                } else {
                                    "day"
                                },
                            ),
                            r["period"].as_str().map(str::to_owned),
                            false,
                        ),
                        tcell(Some(&asset)),
                        ncell(
                            num(&r["quantity"]),
                            &move |x| asset_amount(x, &a2),
                            &unknown,
                        ),
                        fcell(num(&r["market_value"])),
                        fcell(num(&r["cost_basis"])),
                    ]
                })
                .collect(),
        },
        "portfolio" => Table {
            columns: vec![
                col_t("table.wallet"),
                col_t("table.asset"),
                col_t("measure.quantity"),
                col_t("measure.market_value"),
                col_t("measure.cost_basis"),
                col_t("measure.unrealized_pnl"),
            ],
            rows: rows
                .iter()
                .map(|r| {
                    let a2 = r["asset"].as_str().unwrap_or("").to_owned();
                    vec![
                        tcell(r["wallet"].as_str()),
                        tcell(r["asset"].as_str()),
                        ncell(
                            num(&r["quantity"]),
                            &move |x| asset_amount(x, &a2),
                            &unknown,
                        ),
                        fcell(num(&r["market_value"])),
                        fcell(num(&r["cost_basis"])),
                        fcell(num(&r["unrealized_pnl"])),
                    ]
                })
                .collect(),
        },
        "tax_summary" => Table {
            columns: vec![
                col_t("table.year"),
                col_t("measure.proceeds"),
                col_t("measure.cost_basis"),
                col_t("measure.gain_loss"),
            ],
            rows: rows
                .iter()
                .map(|r| {
                    let y = num(&r["year"])
                        .map(|y| (y as i64).to_string())
                        .unwrap_or_default();
                    vec![
                        (y.clone(), Some(y), false),
                        fcell(num(&r["proceeds"])),
                        fcell(num(&r["cost_basis"])),
                        fcell(num(&r["gain_loss"])),
                    ]
                })
                .collect(),
        },
        "cash_flow" => Table {
            columns: vec![
                col_t("table.period"),
                col_t("cashFlow.received"),
                col_t("cashFlow.sent"),
                format!("{} ({cur})", col_t("cashFlow.received")),
                format!("{} ({cur})", col_t("cashFlow.sent")),
                col_t("table.count"),
            ],
            rows: rows
                .iter()
                .map(|r| {
                    let count =
                        num(&r["in_count"]).unwrap_or(0.0) + num(&r["out_count"]).unwrap_or(0.0);
                    vec![
                        (
                            date(
                                r["period"].as_str().unwrap_or(""),
                                v.visual["bucket"].as_str().unwrap_or("month"),
                            ),
                            r["period"].as_str().map(str::to_owned),
                            false,
                        ),
                        ncell(num(&r["in_btc"]), &btc, &unknown),
                        ncell(num(&r["out_btc"]), &btc, &unknown),
                        fcell(num(&r["in_fiat"])),
                        fcell(num(&r["out_fiat"])),
                        ncell(Some(count), &|x| crate::format::count(x as usize), &unknown),
                    ]
                })
                .collect(),
        },
        "transactions" => Table {
            columns: vec![
                col_t("table.date"),
                col_t("table.description"),
                col_t("table.wallet"),
                col_t("table.amount"),
                col_t("table.fiat"),
                col_t("table.status"),
            ],
            rows: rows
                .iter()
                .map(|r| {
                    let a2 = r["asset"].as_str().unwrap_or("").to_owned();
                    let desc = [
                        r["counterparty"].as_str(),
                        r["note"].as_str(),
                        r["type"].as_str(),
                    ]
                    .into_iter()
                    .flatten()
                    .find(|s| !s.is_empty());
                    vec![
                        (
                            r["date"]
                                .as_str()
                                .map(|d| date(d, "short"))
                                .unwrap_or_else(|| "—".into()),
                            r["date"].as_str().map(str::to_owned),
                            false,
                        ),
                        tcell(desc),
                        tcell(r["wallet"].as_str()),
                        ncell(
                            num(&r["amount_sat"]).map(|x| x / 1e8),
                            &move |x| asset_amount(x, &a2),
                            &unknown,
                        ),
                        fcell(num(&r["fiat_value"])),
                        tcell(r["status"].as_str()),
                    ]
                })
                .collect(),
        },
        "transaction_flow" => Table {
            columns: vec![
                col_t("table.side"),
                col_t("table.label"),
                col_t("table.amount"),
            ],
            rows: rows
                .iter()
                .map(|n| {
                    let side = n["side"].as_str().unwrap_or("");
                    let pos = num(&n["position"]).unwrap_or(0.0) as usize + 1;
                    let label = match n["label_kind"].as_str().unwrap_or("") {
                        "wallet" => n["label"].as_str().unwrap_or("").to_owned(),
                        "fee" => t("assistant:visual.flow.fee"),
                        "overflow" => tc(
                            "assistant:visual.flow.overflow",
                            num(&n["overflow_count"]).unwrap_or(0.0) as usize,
                            &[],
                        ),
                        _ => tr(
                            if side == "input" {
                                "assistant:visual.flow.input"
                            } else {
                                "assistant:visual.flow.output"
                            },
                            &[("position", &pos.to_string())],
                        ),
                    };
                    vec![
                        tcell(Some(&t(&format!("assistant:visual.flow.side.{side}")))),
                        tcell(Some(&label)),
                        ncell(
                            num(&n["value_sats"]),
                            &|x| format!("{} sat", crate::format::count(x as usize)),
                            &unknown,
                        ),
                    ]
                })
                .collect(),
        },
        _ => Table {
            columns: vec![
                col_t("table.check"),
                col_t("table.status"),
                col_t("table.count"),
            ],
            rows: rows
                .iter()
                .map(|c| {
                    let check = c["check"].as_str().unwrap_or("");
                    let name = if check == "blocker" {
                        c["title"]
                            .as_str()
                            .or(c["detail"].as_str())
                            .unwrap_or("")
                            .to_owned()
                    } else {
                        t(&format!("assistant:visual.readiness.check.{check}"))
                    };
                    vec![
                        tcell(Some(&name)),
                        tcell(Some(&t(&format!(
                            "assistant:visual.readiness.status.{}",
                            c["status"].as_str().unwrap_or("ok")
                        )))),
                        ncell(num(&c["count"]), &|x| crate::format::count(x as usize), "—"),
                    ]
                })
                .collect(),
        },
    }
}

/// `csvCell`: quoted when needed; a leading formula character is neutralised.
pub fn csv_cell(v: &str) -> String {
    let safe = if v.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{v}")
    } else {
        v.to_owned()
    };
    if safe.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", safe.replace('"', "\"\""))
    } else {
        safe
    }
}

pub fn table_csv(t: &Table) -> String {
    let mut lines = vec![
        t.columns
            .iter()
            .map(|c| csv_cell(c))
            .collect::<Vec<_>>()
            .join(","),
    ];
    for r in &t.rows {
        lines.push(
            r.iter()
                .map(|(_, raw, numeric)| match raw {
                    None => String::new(),
                    Some(s) if *numeric => s.clone(),
                    Some(s) => csv_cell(s),
                })
                .collect::<Vec<_>>()
                .join(","),
        );
    }
    lines.join("\n")
}

fn table_view(app: &App, tk: &Tk, v: &Visual) -> El {
    let tb = table(v);
    let hide = app.prefs.hide_sensitive();
    // `overflow-auto` around a `w-full` auto table: each column is at least its widest cell, and
    // past the message's width the table scrolls sideways (and down, past `max-h-72`).
    let measure = |s: &str, style: rune::TextStyle| rune::text::measure(s, &[], style, None).0;
    let least: Vec<f32> = (0..tb.columns.len())
        .map(|i| {
            let body = tb
                .rows
                .iter()
                .filter_map(|r| r.get(i))
                .map(|(d, _, _)| measure(d, tk.txs(400.0)))
                .fold(0.0, f32::max);
            measure(&tb.columns[i], tk.txs(500.0)).max(body) + tk.r(1.0)
        })
        .collect();
    let total: f32 = least.iter().sum();
    let scroll_key = format!("visual-table-{}", v.call_id);
    let table_w = crate::ui::table::width(&scroll_key, total, total);
    let cell = |s: String, i: usize, head: bool, numeric: bool| {
        let style = if head { tk.txs(500.0) } else { tk.txs(400.0) };
        let mut c = row()
            .grow(if i == 0 { 1.5 } else { 1.0 })
            .w(0.0)
            .min_w(least.get(i).copied().unwrap_or(0.0))
            .px(tk.r(0.5))
            .py(if head { tk.r(0.375) } else { tk.r(0.25) });
        if i > 0 {
            c = c.justify_end();
        }
        let e = if numeric && !head {
            crate::ui::sensitive(s, style.features("'tnum' 1"), tk.fg, hide)
        } else {
            text(s)
                .font(style)
                .color(if head { tk.muted_fg } else { tk.fg })
        };
        c.child(e.truncate())
    };
    let mut head = row()
        .w_full()
        .fill(tk.card)
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
    for (i, c) in tb.columns.iter().enumerate() {
        head = head.child(cell(c.clone(), i, true, false));
    }
    let mut body = col().w_full();
    let n = tb.rows.len();
    for (ri, r) in tb.rows.into_iter().enumerate() {
        let mut line = row().w_full();
        if ri + 1 < n {
            line = line.border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
        }
        for (i, (display, _, numeric)) in r.into_iter().enumerate() {
            line = line.child(cell(display, i, false, numeric));
        }
        body = body.child(line);
    }
    let grid = col()
        .w(table_w)
        .role("table")
        .child(head)
        .child(super::nested_scroll(col().w_full().max_h(tk.r(18.0))).child(body));
    let label = tb.columns.join(", ");
    col()
        .w_full()
        .min_w(0.0)
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .clip_rounded(tk.r(0.375))
        .child(crate::ui::table::hscroll(&scroll_key, label, total, grid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn value_axis_ticks_match_recharts() {
        assert_eq!(
            nice_ticks(0.0, 11_500_000.0, 5),
            vec![0.0, 3_000_000.0, 6_000_000.0, 9_000_000.0, 12_000_000.0]
        );
        assert_eq!(nice_ticks(0.0, 98.6, 5), vec![0.0, 25.0, 50.0, 75.0, 100.0]);
        // Spanning zero, zero is a tick.
        assert!(nice_ticks(-30.0, 70.0, 5).contains(&0.0));
    }

    #[test]
    fn category_labels_keep_the_end_and_their_gap() {
        // Five 30 pt labels 40 pt apart on a 160 pt axis, 10 pt gap: the last pulled inside the
        // axis, then each one that clears the previous by the gap.
        let shown = end_ticks(
            &[30.0; 5],
            &[0.0, 40.0, 80.0, 120.0, 160.0],
            0.0,
            160.0,
            10.0,
        );
        assert_eq!(
            shown.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![1, 2, 4]
        );
        assert_eq!(shown.last().unwrap().1, 145.0);
    }

    #[test]
    fn ranges_need_a_fifth_more_than_their_span() {
        assert_eq!(ranges_for(100.0), vec!["all"]);
        assert_eq!(ranges_for(120.0), vec!["all", "3m"]);
        assert_eq!(ranges_for(500.0), vec!["all", "1y", "3m"]);
    }

    #[test]
    fn csv_is_spreadsheet_safe() {
        assert_eq!(csv_cell("=SUM(A1)"), "'=SUM(A1)");
        assert_eq!(csv_cell("a,b"), "\"a,b\"");
        assert_eq!(csv_cell("say \"hi\""), "\"say \"\"hi\"\"\"");
        assert_eq!(csv_cell("plain"), "plain");
    }

    #[test]
    fn numbers_and_dates_follow_the_language() {
        crate::i18n::set_lang(Lang::En);
        assert_eq!(number(1234567.5, 2, 0), "1,234,567.5");
        assert_eq!(btc(0.5), "₿ 0.5");
        assert_eq!(date("2025-03-05T10:00:00Z", "short"), "Mar 5, 2025");
        assert_eq!(date("2025-03-05", "month"), "Mar 2025");
        assert_eq!(date("2025-08-05", "quarter"), "Q3 2025");
        crate::i18n::set_lang(Lang::De);
        assert_eq!(number(1234567.5, 2, 0), "1.234.567,5");
        assert_eq!(date("2025-03-05", "short"), "5. März 2025");
        crate::i18n::set_lang(Lang::En);
    }
}
