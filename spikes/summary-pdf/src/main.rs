//! Rendering spike. All financial decisions remain in the captured Python input.
use rust_decimal::{Decimal as D, RoundingStrategy, prelude::ToPrimitive};
use serde_json::{Value as J, json};
use std::{collections::HashMap, error::Error, path::PathBuf, str::FromStr, time::Instant};
use typst::{
    Library, LibraryExt, World,
    diag::{FileError, FileResult},
    foundations::{Bytes, Datetime, Dict, Duration, Value},
    syntax::{FileId, RootedPath, Source, VirtualPath, VirtualRoot},
    text::{Font, FontBook},
    utils::LazyHash,
};
use typst_layout::PagedDocument;

const MM: f64 = 72.0 / 25.4;
const RED: &str = "#e3000f";
const GREEN: &str = "#2ca02c";
const BLUE: &str = "#1f77b4";
const GRAY: &str = "#7f7f7f";
const INK: &str = "#222222";
const MUTED: &str = "#666666";
const GRID: &str = "#d9d9d9";
const PALETTE: [&str; 7] = [RED, BLUE, GREEN, "#9467bd", "#ff7f0e", "#17becf", GRAY];

fn dec(v: &J) -> D {
    let s = v
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| v.to_string());
    D::from_str(&s)
        .or_else(|_| D::from_scientific(&s))
        .unwrap_or(D::ZERO)
}
fn d(v: f64) -> D {
    D::from_str(&v.to_string()).unwrap()
}
fn num(v: &J) -> f64 {
    dec(v).to_f64().unwrap()
}
fn s(v: &J) -> &str {
    v.as_str().unwrap_or("")
}
fn rows(v: &J) -> &[J] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}
fn fixed(v: D, places: u32, comma: bool) -> String {
    let rounded = v.round_dp_with_strategy(places, RoundingStrategy::MidpointNearestEven);
    let text = format!("{rounded:.precision$}", precision = places as usize);
    if !comma {
        return text;
    }
    let (whole, fractional) = text.split_once('.').unwrap_or((&text, ""));
    let (sign, digits) = whole
        .strip_prefix('-')
        .map(|s| ("-", s))
        .unwrap_or(("", whole));
    let mut out = sign.to_owned();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if places > 0 {
        out.push('.');
        out.push_str(fractional);
    }
    out
}
fn money(c: &str, v: D) -> String {
    format!("{c} {}", fixed(v, 2, true))
}
fn signed(c: &str, v: D) -> String {
    format!(
        "{}{c} {}",
        if v > D::ZERO { "+" } else { "" },
        fixed(v, 2, true)
    )
}
fn btc(v: D) -> String {
    format!("{} BTC", fixed(v, 8, true))
}
fn compact(c: &str, v: D) -> String {
    let sign = if v < D::ZERO { "-" } else { "" };
    let a = v.abs();
    let value = if a >= d(1e6) {
        format!("{}m", fixed(a / d(1e6), 1, false))
    } else if a >= d(1e3) {
        format!("{}k", fixed(a / d(1e3), 1, false))
    } else {
        fixed(a, if a > D::ZERO && a < D::ONE { 2 } else { 0 }, false)
    };
    format!("{sign}{c} {value}")
}
fn compact_number(v: D) -> String {
    if v.abs() >= d(1e6) {
        format!("{}m", fixed(v / d(1e6), 1, false))
    } else if v.abs() >= d(1e3) {
        format!("{}k", fixed(v / d(1e3), 1, false))
    } else {
        fixed(v, 0, false)
    }
}
fn quantity(row: &J) -> String {
    let parts: Vec<_> = rows(&row["asset_quantities"])
        .iter()
        .map(|a| {
            format!(
                "{} {}",
                fixed(dec(&a["quantity"]), 8, true),
                s(&a["asset"]).to_uppercase()
            )
        })
        .collect();
    if !parts.is_empty() {
        return parts.join(", ");
    }
    let assets = rows(&row["assets"]);
    let suffix = if assets.len() == 1 {
        s(&assets[0])
    } else {
        "units"
    };
    let value = ["quantity", "end_quantity", "total_quantity"]
        .iter()
        .find_map(|k| (!row[k].is_null()).then_some(&row[k]))
        .unwrap_or(&J::Null);
    format!("{} {suffix}", fixed(dec(value), 8, true))
}
fn short_date(v: &J) -> String {
    s(v).chars().take(10).collect()
}

struct Chart {
    height: f64,
    labels: Vec<J>,
    lines: Vec<J>,
    rects: Vec<J>,
    circles: Vec<J>,
}
impl Chart {
    fn new(title: &str, height: f64) -> Self {
        let mut c = Self {
            height,
            labels: vec![],
            lines: vec![],
            rects: vec![],
            circles: vec![],
        };
        c.label(0.0, height - 10.0, title, 9.0, INK, "left", true);
        c
    }
    fn label(
        &mut self,
        x: f64,
        y: f64,
        text: impl Into<String>,
        size: f64,
        color: &str,
        align: &str,
        bold: bool,
    ) {
        let width = if align == "left" { 510.0 - x } else { 100.0 };
        let x = match align {
            "center" => x - 50.0,
            "right" => x - 100.0,
            _ => x,
        };
        self.labels.push(json!({"x":x,"y":self.height-y-size*0.8,"width":width,"text":text.into(),"size":size,"color":color,"align":align,"bold":bold}));
    }
    fn line(&mut self, x1: f64, y1: f64, x2: f64, y2: f64, color: &str, width: f64) {
        self.lines.push(json!({"x1":x1,"y1":self.height-y1,"x2":x2,"y2":self.height-y2,"color":color,"width":width}));
    }
    fn rect(&mut self, x: f64, y: f64, w: f64, h: f64, color: &str) {
        self.rects
            .push(json!({"x":x,"y":self.height-y-h,"w":w,"h":h.max(0.001),"color":color}));
    }
    fn circle(&mut self, x: f64, y: f64, r: f64, color: &str) {
        self.circles
            .push(json!({"x":x-r,"y":self.height-y-r,"r":r,"color":color}));
    }
    fn finish(self) -> J {
        json!({"height":self.height,"labels":self.labels,"lines":self.lines,"rects":self.rects,"circles":self.circles})
    }
}
fn bounds(values: &[f64]) -> (f64, f64) {
    let low = values.iter().copied().reduce(f64::min).unwrap_or(0.0);
    let high = values.iter().copied().reduce(f64::max).unwrap_or(1.0);
    let pad = if low == high {
        if high == 0.0 { 1.0 } else { high.abs() * 0.1 }
    } else {
        (high - low) * 0.06
    };
    (
        if low >= 0.0 {
            (low - pad).max(0.0)
        } else {
            low - pad
        },
        high + pad,
    )
}
fn scale(v: f64, low: f64, high: f64, size: f64) -> f64 {
    if high == low {
        size / 2.0
    } else {
        (v - low) / (high - low) * size
    }
}
fn label_axis(c: &mut Chart, rs: &[J], idx: usize, x: f64, bottom: f64, base: f64) {
    let n = rs.len();
    let step = (n as f64 / 10.0).round_ties_even().max(1.0) as usize;
    if n > 12 && !idx.is_multiple_of(step) && idx != n - 1 {
        return;
    }
    let period = s(&rs[idx]["period"]);
    let label: String = if n > 12 {
        period.chars().skip(2).take(5).collect()
    } else {
        period.chars().take(7).collect()
    };
    c.label(
        x,
        bottom - if n > 12 && idx % 2 == 1 { 13.0 } else { 8.0 },
        label,
        if n > 18 {
            3.6
        } else if n > 12 {
            4.1
        } else {
            base
        },
        MUTED,
        "center",
        false,
    );
}
fn line_chart(rs: &[J], currency: &str) -> J {
    let mut c = Chart::new("Total balance over time", 60.0 * MM);
    let (left, bottom, w, h) = (48.0, 25.0, 180.0 * MM - 96.0, 60.0 * MM - 53.0);
    if rs.is_empty() {
        c.label(
            left,
            bottom + h / 2.0,
            "No balance history in scope.",
            8.0,
            MUTED,
            "left",
            false,
        );
        return c.finish();
    }
    let vals: Vec<_> = rs
        .iter()
        .flat_map(|r| [num(&r["market_value"]), num(&r["cumulative_cost_basis"])])
        .collect();
    let (lo, hi) = bounds(&vals);
    let quantities: Vec<_> = rs.iter().map(|r| num(&r["quantity"])).collect();
    let (ql, qh) = bounds(&quantities);
    for i in 0..5 {
        let t = i as f64 / 4.0;
        let y = bottom + h * t;
        c.line(left, y, left + w, y, GRID, 0.3);
        c.label(
            left - 4.0,
            y - 2.0,
            compact(currency, d(lo + (hi - lo) * t)),
            5.8,
            MUTED,
            "right",
            false,
        );
        let q = ql + (qh - ql) * t;
        let places = if q.abs() >= 1.0 {
            4
        } else if q.abs() >= 0.01 {
            3
        } else {
            4
        };
        c.label(
            left + w + 4.0,
            y - 2.0,
            fixed(d(q), places, false),
            5.8,
            MUTED,
            "left",
            false,
        );
    }
    for (field, color, width, low, high) in [
        ("cumulative_cost_basis", GRAY, 0.9, lo, hi),
        ("market_value", RED, 1.4, lo, hi),
        ("quantity", BLUE, 1.1, ql, qh),
    ] {
        let points: Vec<_> = rs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                (
                    left + w * i as f64 / (rs.len() - 1).max(1) as f64,
                    bottom + scale(num(&r[field]), low, high, h),
                )
            })
            .collect();
        for p in points.windows(2) {
            c.line(p[0].0, p[0].1, p[1].0, p[1].1, color, width);
        }
        if field != "cumulative_cost_basis" {
            for p in [points[0], *points.last().unwrap()] {
                c.circle(p.0, p.1, 1.7, color);
            }
        }
    }
    for i in 0..rs.len() {
        label_axis(
            &mut c,
            rs,
            i,
            left + w * i as f64 / (rs.len() - 1).max(1) as f64,
            bottom,
            5.2,
        );
    }
    for (i, (text, color)) in [("Market value", RED), ("Cost basis", GRAY), ("Units", BLUE)]
        .iter()
        .enumerate()
    {
        c.label(
            350.0 + i as f64 * 57.0,
            c.height - 14.0,
            *text,
            6.4,
            color,
            "left",
            false,
        );
    }
    c.label(
        left + 3.0,
        bottom + h + 5.0,
        format!("Start {}", compact(currency, dec(&rs[0]["market_value"]))),
        6.0,
        MUTED,
        "left",
        false,
    );
    c.label(
        left + w - 3.0,
        bottom + h + 5.0,
        format!(
            "End {}",
            compact(currency, dec(&rs.last().unwrap()["market_value"]))
        ),
        6.0,
        INK,
        "right",
        true,
    );
    c.label(
        left,
        5.0,
        "Left axis market value · right axis total units",
        6.3,
        MUTED,
        "left",
        false,
    );
    if rs.last().unwrap()["period_partial"] == true {
        c.label(
            left + 142.0,
            5.0,
            format!(
                "Final period capped at {}",
                short_date(&rs.last().unwrap()["period_end"])
            ),
            6.3,
            MUTED,
            "left",
            false,
        );
    }
    c.finish()
}
fn bars(rs: &[J], currency: &str, paired: bool) -> J {
    let title = if paired {
        "Inflows vs outflows volume"
    } else {
        "Realized PnL per period"
    };
    let mut c = Chart::new(title, 48.0 * MM);
    let (left, bottom, w, h) = (42.0, 25.0, 180.0 * MM - 52.0, 48.0 * MM - 45.0);
    if rs.is_empty() {
        c.label(
            left,
            bottom + h / 2.0,
            "No rows in scope.",
            8.0,
            MUTED,
            "left",
            false,
        );
        return c.finish();
    }
    let mut values: Vec<_> = vec![0.0];
    for r in rs {
        if paired {
            values.extend([num(&r["inflow_volume"]), num(&r["outflow_volume"])]);
        } else {
            values.push(num(&r["realized_pnl"]));
        }
    }
    let raw_lo = values.iter().copied().reduce(f64::min).unwrap();
    let raw_hi = values.iter().copied().reduce(f64::max).unwrap();
    let (lo, hi) = if paired {
        (0.0, if raw_hi == 0.0 { 1.0 } else { raw_hi * 1.1 })
    } else if raw_lo == raw_hi {
        (raw_lo, 1.0)
    } else {
        let pad = (raw_hi - raw_lo) * 0.1;
        (raw_lo - pad, raw_hi + pad)
    };
    let baseline = bottom + scale(0.0, lo, hi, h);
    let ticks = if paired { 4 } else { 5 };
    for i in 0..ticks {
        let value = lo + (hi - lo) * i as f64 / (ticks - 1) as f64;
        let y = bottom + scale(value, lo, hi, h);
        c.line(left, y, left + w, y, GRID, 0.25);
        c.label(
            left - 4.0,
            y - 2.0,
            compact(currency, d(value)),
            5.8,
            MUTED,
            "right",
            false,
        );
    }
    c.line(left, baseline, left + w, baseline, GRID, 0.5);
    let slot = w / rs.len() as f64;
    for (i, r) in rs.iter().enumerate() {
        let x = left + i as f64 * slot + 2.0;
        label_axis(&mut c, rs, i, x + slot / 2.0, bottom, 5.0);
        if paired {
            for (j, (field, color)) in [("inflow_volume", GREEN), ("outflow_volume", RED)]
                .iter()
                .enumerate()
            {
                let value = num(&r[*field]);
                let height = scale(value, 0.0, hi, h);
                c.rect(
                    x + j as f64 * slot / 2.0,
                    bottom,
                    (slot / 2.0 - 2.0).max(1.0),
                    height,
                    color,
                );
                if value != 0.0 && rs.len() <= 12 {
                    c.label(
                        x + slot * (0.25 + 0.5 * j as f64),
                        (bottom + height + 3.0).min(bottom + h - 5.0),
                        compact_number(dec(&r[*field])),
                        5.2,
                        INK,
                        "center",
                        false,
                    );
                }
            }
        } else {
            let value = num(&r["realized_pnl"]);
            let y = bottom + scale(value, lo, hi, h);
            let height = (y - baseline).abs();
            let min = y.min(baseline);
            c.rect(
                x,
                min,
                (slot - 4.0).max(1.0),
                height,
                if value >= 0.0 { GREEN } else { RED },
            );
            if value != 0.0 && rs.len() <= 18 {
                let inside = value < 0.0 && height >= 9.0;
                let ly = if inside {
                    min + height - 7.0
                } else {
                    min + height + 3.0
                };
                c.label(
                    x + slot / 2.0,
                    ly.max(bottom + 3.0).min(bottom + h - 6.0),
                    compact(currency, dec(&r["realized_pnl"])),
                    5.2,
                    if inside { "#ffffff" } else { INK },
                    "center",
                    true,
                );
            }
        }
    }
    let (a, b) = if paired {
        ("Inflow", "Outflow")
    } else {
        ("Gain", "Loss")
    };
    c.label(407.0, c.height - 14.0, a, 6.5, GREEN, "left", false);
    c.label(460.0, c.height - 14.0, b, 6.5, RED, "left", false);
    let footer = if paired {
        let inflow: D = rs.iter().map(|r| dec(&r["inflow_volume"])).sum();
        let outflow: D = rs.iter().map(|r| dec(&r["outflow_volume"])).sum();
        format!(
            "Max period {} · Total in {} · Total out {}",
            money(currency, d(raw_hi)),
            money(currency, inflow),
            money(currency, outflow)
        )
    } else {
        format!(
            "Range {} to {}",
            signed(currency, d(raw_lo)),
            signed(currency, d(raw_hi))
        )
    };
    c.label(left, 4.0, footer, 6.5, MUTED, "left", false);
    c.finish()
}
fn composition(rs: &[J], currency: &str) -> (J, String) {
    let mut c = Chart::new("Holdings by wallet at period end", 58.0 * MM);
    let visible: Vec<_> = rs
        .iter()
        .filter(|r| dec(&r["market_value"]) > D::ZERO)
        .collect();
    let total: D = visible.iter().map(|r| dec(&r["market_value"])).sum();
    let (cx, cy, r) = (31.0 * MM, 27.0 * MM, 19.0 * MM);
    let mut angle = -std::f64::consts::FRAC_PI_2;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">",
        180.0 * MM,
        c.height,
        180.0 * MM,
        c.height
    );
    for (i, row) in visible.iter().enumerate() {
        let sweep = (dec(&row["market_value"]) / total).to_f64().unwrap() * std::f64::consts::TAU;
        let color = PALETTE[i % 7];
        // Two arcs also handle a 100% holding.
        for part in 0..2 {
            let a = angle + sweep * part as f64 / 2.0;
            let b = a + sweep / 2.0;
            let (x1, y1) = (cx + r * a.cos(), c.height - cy + r * a.sin());
            let (x2, y2) = (cx + r * b.cos(), c.height - cy + r * b.sin());
            svg.push_str(&format!("<path d=\"M {cx} {} L {x1} {y1} A {r} {r} 0 0 1 {x2} {y2} Z\" fill=\"{color}\" stroke=\"white\" stroke-width=\"0.4\"/>",c.height-cy));
        }
        angle += sweep;
    }
    svg.push_str(&format!(
        "<circle cx=\"{cx}\" cy=\"{}\" r=\"{}\" fill=\"white\"/></svg>",
        c.height - cy,
        r * 0.52
    ));
    if total > D::ZERO {
        c.label(cx, cy + 3.0, "Period end", 6.2, MUTED, "center", false);
        c.label(
            cx,
            cy - 7.0,
            compact(currency, total),
            8.0,
            INK,
            "center",
            true,
        );
        for (i, row) in visible.iter().take(6).enumerate() {
            let y = c.height - 22.0 - 17.0 * i as f64;
            c.rect(75.0 * MM, y - 6.0, 5.0, 5.0, PALETTE[i % 7]);
            c.label(
                75.0 * MM + 8.0,
                y - 4.0,
                s(&row["wallet"]).chars().take(24).collect::<String>(),
                6.7,
                INK,
                "left",
                true,
            );
            c.label(
                75.0 * MM + 8.0,
                y - 12.0,
                format!(
                    "{} · {}%",
                    money(currency, dec(&row["market_value"])),
                    fixed(dec(&row["market_value"]) / total * d(100.0), 1, false)
                ),
                6.2,
                MUTED,
                "left",
                false,
            );
        }
        if visible.len() > 6 {
            let value: D = visible[6..].iter().map(|r| dec(&r["market_value"])).sum();
            c.label(
                75.0 * MM + 8.0,
                c.height - 28.0 - 17.0 * 6.0,
                format!(
                    "+{} more · {} · {}%",
                    visible.len() - 6,
                    money(currency, value),
                    fixed(value / total * d(100.0), 1, false)
                ),
                6.2,
                MUTED,
                "left",
                false,
            );
        }
    } else {
        c.label(8.0, 9.0, "No holdings in scope.", 8.0, MUTED, "left", false);
    }
    (c.finish(), svg)
}

fn presentation(r: &J) -> (J, String) {
    let c = s(&r["fiat_currency"]);
    let m = &r["metrics"];
    let di = &r["data_integrity"];
    let mut integrity = vec![
        vec!["Signal".into(), "Status".into()],
        vec![
            "Priced transactions".into(),
            format!(
                "{} / {} ({}%)",
                di["priced_transactions"],
                di["total_transactions"],
                fixed(dec(&di["priced_percentage"]), 1, false)
            ),
        ],
        vec![
            "Journals".into(),
            s(&di["journals"]["status"])
                .replace('_', " ")
                .split_whitespace()
                .map(|w| {
                    let mut chars = w.chars();
                    format!("{}{}", chars.next().unwrap().to_uppercase(), chars.as_str())
                })
                .collect::<Vec<_>>()
                .join(" "),
        ],
        vec!["Quarantines".into(), di["quarantine_count"].to_string()],
    ];
    if num(&di["internal_transfers"]["count"]) > 0.0 {
        integrity.push(vec![
            "Internal transfers (excluded from flow)".into(),
            format!(
                "{} tx · {}",
                di["internal_transfers"]["count"],
                money(c, dec(&di["internal_transfers"]["fiat_volume"]))
            ),
        ]);
    }
    if rows(&di["quarantine_reasons"]).is_empty() {
        integrity.push(vec!["Quarantine reasons".into(), "None in scope".into()]);
    } else {
        for reason in rows(&di["quarantine_reasons"]) {
            integrity.push(vec![
                format!("Quarantine: {}", s(&reason["reason"])),
                reason["count"].to_string(),
            ]);
        }
    }
    let start = dec(&m["period_start_value"]);
    let end = dec(&m["period_end_value"]);
    let qs = dec(&m["btc_stack_start"]);
    let qe = dec(&m["btc_stack_end"]);
    let plus = |v: D| {
        format!(
            "{}{}",
            if v >= D::ZERO { "+" } else { "" },
            fixed(v, 1, false)
        )
    };
    let mut performance = if start > D::ZERO {
        format!("{}% vs start", plus((end - start) / start * d(100.0)))
    } else {
        format!("{} vs start", signed(c, end - start))
    };
    if !r["benchmark"]["change_pct"].is_null() {
        performance.push_str(&format!(
            " (BTC spot {}%)",
            plus(dec(&r["benchmark"]["change_pct"]))
        ));
    }
    let change = if qs > D::ZERO {
        format!("{}%", plus((qe - qs) / qs * d(100.0)))
    } else {
        format!(
            "{}{} units",
            if qe > qs { "+" } else { "" },
            fixed(qe - qs, 4, false)
        )
    };
    let perf = vec![
        format!(
            "Period performance: {performance} · Unrealized at close: {}",
            signed(c, dec(&m["unrealized_pnl"]))
        ),
        format!(
            "Total quantity: {} → {} units ({change}) · Network + venue fees: {} · {}",
            fixed(qs, 4, false),
            fixed(qe, 4, false),
            btc(dec(&m["fees_btc"])),
            money(c, dec(&m["fees_fiat"]))
        ),
    ];
    let kpis:Vec<_>=[("Start value","period_start_value",false),("End value","period_end_value",false),("Net flow","net_flow",true),("Realized PnL","realized_pnl",true),("Fees","fees_fiat",false)].iter().map(|(label,key,sign)|json!({"label":label,"value":if *sign {signed(c,dec(&m[*key]))}else{money(c,dec(&m[*key]))}})).collect();
    let mut disposals = vec![vec![
        "Date".to_string(),
        "Wallet".into(),
        "Quantity".into(),
        "Proceeds".into(),
        "Cost basis".into(),
        "Gain/Loss".into(),
    ]];
    for row in rows(&r["top_disposals"]) {
        disposals.push(vec![
            short_date(&row["occurred_at"]),
            s(&row["wallet"]).into(),
            btc(-dec(&row["quantity"])),
            money(c, dec(&row["proceeds"])),
            money(c, dec(&row["cost_basis"])),
            signed(c, dec(&row["gain_loss"])),
        ]);
    }
    let mut movements = vec![vec![
        "Date".to_string(),
        "Wallet".into(),
        "Dir".into(),
        "Asset".into(),
        "Amount".into(),
        "Value".into(),
        "Counterparty".into(),
    ]];
    for row in rows(&r["top_movements"]) {
        movements.push(vec![
            short_date(&row["occurred_at"]),
            s(&row["wallet"]).into(),
            match s(&row["direction"]) {
                "inbound" => "In",
                "outbound" => "Out",
                _ => "—",
            }
            .into(),
            s(&row["asset"]).into(),
            btc(dec(&row["quantity"])),
            money(c, dec(&row["fiat_value"])),
            s(&row["counterparty"]).into(),
        ]);
    }
    let mut appendix = vec![vec![
        "Wallet".to_string(),
        "Scope".into(),
        "Tx count".into(),
        "End balance".into(),
        "End value".into(),
    ]];
    for row in rows(&r["wallet_appendix"]) {
        appendix.push(vec![
            s(&row["wallet"]).into(),
            s(&row["scope"]).into(),
            row["tx_count"].to_string(),
            quantity(row),
            money(c, dec(&row["end_market_value"])),
        ]);
    }
    let mut snapshot = vec![vec![
        "Wallet".to_string(),
        "Assets".into(),
        "Balance".into(),
        "Market value".into(),
    ]];
    for row in rows(&r["snapshot"]["wallets"]) {
        snapshot.push(vec![
            s(&row["wallet"]).into(),
            rows(&row["assets"])
                .iter()
                .map(|a| s(a).to_owned())
                .collect::<Vec<_>>()
                .join(", "),
            quantity(row),
            money(c, dec(&row["market_value"])),
        ]);
    }
    let age = &r["holding_age"];
    let mut age_parts = vec![];
    if num(&age["acquisition_count"]) == 0.0 {
        age_parts.push("No acquisitions recorded in scope.".into());
    } else {
        if !age["weighted_days"].is_null() {
            let days = dec(&age["weighted_days"]);
            let formatted = if days >= d(365.0) {
                format!("{} years", fixed(days / d(365.25), 1, false))
            } else if days >= d(60.0) {
                format!("{} months", fixed(days / d(30.44), 1, false))
            } else {
                format!("{} days", fixed(days, 0, false))
            };
            age_parts.push(format!("Weighted-avg acquisition age: {formatted}"));
        }
        if !s(&age["oldest_acquisition"]).is_empty() {
            age_parts.push(format!(
                "Oldest acquisition: {}",
                short_date(&age["oldest_acquisition"])
            ));
        }
        age_parts.push(format!(
            "{} acquisition tx in scope",
            age["acquisition_count"]
        ));
    }
    let (composition, svg) = composition(rows(&r["wallet_holdings"]), c);
    (
        json!({"title":r["title"],"book":format!("{} / {}",s(&r["workspace"]),s(&r["profile"])),"timeframe":r["timeframe"]["label"],"generated":s(&r["generated_at"]).replace('T'," ").chars().take(16).collect::<String>(),"snapshot":snapshot,"snapshot_heading":format!("Current snapshot: {} · {}",money(c,dec(&r["snapshot"]["total_market_value"])),quantity(&r["snapshot"])),"kpis":kpis,"performance":perf,"integrity":integrity,"movement_chart":line_chart(rows(&r["balance_history"]),c),"composition_chart":composition,"age":age_parts.join(" · "),"pnl_chart":bars(rows(&r["realized_pnl_periods"]),c,false),"flow_chart":bars(rows(&r["flow_periods"]),c,true),"disposals":disposals,"movements":movements,"appendix":appendix}),
        svg,
    )
}

struct EmbeddedWorld {
    library: LazyHash<Library>,
    book: LazyHash<FontBook>,
    fonts: Vec<Font>,
    source: Source,
    files: HashMap<FileId, Bytes>,
}
impl World for EmbeddedWorld {
    fn library(&self) -> &LazyHash<Library> {
        &self.library
    }
    fn book(&self) -> &LazyHash<FontBook> {
        &self.book
    }
    fn main(&self) -> FileId {
        self.source.id()
    }
    fn source(&self, id: FileId) -> FileResult<Source> {
        if id == self.source.id() {
            Ok(self.source.clone())
        } else {
            Err(FileError::NotFound(PathBuf::from(
                id.vpath().get_with_slash(),
            )))
        }
    }
    fn file(&self, id: FileId) -> FileResult<Bytes> {
        self.files
            .get(&id)
            .cloned()
            .or_else(|| {
                (id == self.source.id()).then(|| Bytes::from_string(self.source.text().to_string()))
            })
            .ok_or_else(|| FileError::NotFound(PathBuf::from(id.vpath().get_with_slash())))
    }
    fn font(&self, index: usize) -> Option<Font> {
        self.fonts.get(index).cloned()
    }
    fn today(&self, _: Option<Duration>) -> Option<Datetime> {
        Datetime::from_ymd(2026, 5, 1)
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 3 {
        return Err("input.json output.pdf".into());
    }
    let start = Instant::now();
    let raw: J = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let (data, svg) = presentation(&raw);
    let mut inputs = Dict::new();
    inputs.insert("data".into(), Value::Str(data.to_string().into()));
    let fonts: Vec<_> = [
        include_bytes!("../fonts/Vera.ttf").as_slice(),
        include_bytes!("../fonts/VeraBd.ttf").as_slice(),
        include_bytes!("../fonts/LiberationSans-Regular.ttf").as_slice(),
    ]
    .iter()
    .map(|bytes| Font::new(Bytes::new(bytes.to_vec()), 0).unwrap())
    .collect();
    let file_id = |name| {
        FileId::new(RootedPath::new(
            VirtualRoot::Project,
            VirtualPath::new(name).unwrap(),
        ))
    };
    let source = Source::new(
        file_id("main.typ"),
        include_str!("../summary.typ").to_string(),
    );
    let mut files = HashMap::new();
    files.insert(file_id("ring.svg"), Bytes::new(svg.into_bytes()));
    let world = EmbeddedWorld {
        library: LazyHash::new(Library::builder().with_inputs(inputs).build()),
        book: LazyHash::new(FontBook::from_fonts(&fonts)),
        fonts,
        source,
        files,
    };
    let result = typst::compile::<PagedDocument>(&world);
    let doc = result.output.map_err(|errors| format!("{errors:?}"))?;
    if !result.warnings.is_empty() {
        eprintln!("Typst warnings: {:?}", result.warnings);
    }
    let bytes = typst_pdf::pdf(&doc, &typst_pdf::PdfOptions::default())
        .map_err(|errors| format!("{errors:?}"))?;
    std::fs::write(&args[2], &bytes)?;
    println!(
        "{}",
        json!({"pages":doc.pages().len(),"bytes":bytes.len(),"render_ms":start.elapsed().as_millis(),"font":world.fonts[0].info().family,"typst":"0.15.1"})
    );
    Ok(())
}
