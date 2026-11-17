//! `HoldingsLayouts.tsx`: the four ways the Holdings card lists its sources — a list, tiles, a
//! table with exact bitcoin, and a list with each source's last twelve months. Every row opens
//! its wallet (the folded "Other sources" row opens none) and shows a dot while its sync runs or
//! after it fails.

use rune::{Color, Cx, El, LineCap, Shape, canvas, col, el, row, text};

use super::holdings::{Ctx, rail_icon, two_columns};
use super::holdings_model::Item;
use super::l10n;
use super::stats::mask;
use crate::app::App;
use crate::format;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::model::Rail;
use crate::theme::Tk;
use crate::ui::{hairline, ic, sensitive};

fn row_key(c: &Ctx, item: &Item) -> String {
    match &item.connection_id {
        Some(id) => c.key(format!("holding-{id}")),
        None => c.key("holding-other"),
    }
}

/// `shareLabel`: "—" for an emptied source, else the share (masked under "Hide all").
fn share_label(item: &Item, hidden: bool) -> String {
    if item.empty {
        "—".into()
    } else {
        mask(
            if item.percent == 0.0 && item.btc > 0.0 {
                "<1%".into()
            } else {
                format!("{}%", item.percent)
            },
            hidden,
        )
    }
}

fn date10(item: &Item) -> String {
    item.last_transaction_at
        .as_deref()
        .map(|d| d.chars().take(10).collect())
        .unwrap_or_default()
}

/// `SourceLink`: the row opens the source's wallet page; the folded row is plain.
fn source_link(c: &Ctx, item: &Item, body: El) -> El {
    let tk = c.tk;
    let Some(id) = item.connection_id.clone() else {
        return body;
    };
    let href = format!("/connections/{id}");
    let body = body
        .role("link")
        .label(tr("overview:holdings.openSource", &[("name", &item.name)]))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .hover(tk.muted.alpha(0.45))
        .focus_ring(2.0, 0.0, tk.ring);
    super::grid::press(
        body,
        super::grid::model::WidgetId::Holdings,
        c.pv,
        move |a| a.navigate(&href),
    )
}

/// `SourceIcon`: the rail's logo, a bank for an exchange or file, nothing for "Other sources".
fn source_icon(c: &Ctx, cx: &Cx<App>, item: &Item) -> Option<El> {
    let tk = c.tk;
    if item.rail != Rail::None {
        Some(rail_icon(c.app, cx, tk, item.rail))
    } else if item.connection_id.is_some() {
        Some(
            row()
                .size(tk.r(1.25), tk.r(1.25))
                .center()
                .radius(tk.r(0.625))
                .fill(tk.muted)
                .child(ic(&I::LANDMARK, tk.r(0.75), tk.muted_fg)),
        )
    } else {
        None
    }
}

/// `SourceName`: the name (muted for "Other sources" and emptied ones), the sync dot, and with
/// `show_sync` when it last synced (read from the snapshot; it never starts a sync).
fn source_name(c: &Ctx, item: &Item, show_sync: bool) -> El {
    let tk = c.tk;
    let name = if item.other {
        t("overview:holdings.otherSources")
    } else {
        item.name.clone()
    };
    let mut n = text(name)
        .font(tk.txs(400.0))
        .color(if item.other || item.empty {
            tk.muted_fg
        } else {
            tk.fg
        })
        .truncate()
        .min_w(0.0);
    if !item.other {
        n = n.help(item.name.clone());
    }
    let mut r = row().min_w(0.0).gap(tk.r(0.375)).center_y().child(n);
    if let Some(state) = item.sync_state {
        let label = t(&format!("overview:holdings.syncState.{state}"));
        let color = if state == "error" {
            rune::color::hex("#ef4444").unwrap_or(tk.alert.solid)
        } else {
            rune::color::hex("#f59e0b").unwrap_or(tk.warn.solid)
        };
        r = r.child(
            crate::ui::dot(tk.r(0.375), color)
                .role("img")
                .label(label.clone())
                .help(label),
        );
    }
    if let (true, Some(at)) = (show_sync, &item.last_sync_at) {
        let when =
            l10n::relative(at, format::now_secs()).unwrap_or_else(|| format::short_date(Some(at)));
        r = r.child(
            text(tr("overview:holdings.lastSynced", &[("time", &when)]))
                .font(tk.t2xs(400.0))
                .color(tk.muted_fg)
                .help(tr(
                    "overview:holdings.lastSyncedAt",
                    &[("date", &format::short_date(Some(at)))],
                )),
        );
    }
    r
}

/// The 1.5 rem icon column of the list and trend rows.
fn icon_cell(c: &Ctx, cx: &Cx<App>, item: &Item) -> El {
    let cell = row().w(c.tk.r(1.5)).justify_center();
    match source_icon(c, cx, item) {
        Some(i) => cell.child(i),
        None => cell,
    }
}

fn value_cell(c: &Ctx, item: &Item, w: f32) -> El {
    let tk = c.tk;
    let (weight, color) = if item.empty {
        (400.0, tk.muted_fg)
    } else {
        (500.0, tk.fg)
    };
    sensitive(
        c.value_label(item.btc, item.value),
        tk.txs(weight).features("'tnum' 1"),
        color,
        c.hide,
    )
    .w(w)
    .align(rune::Align::End)
    .truncate()
}

fn share_cell(c: &Ctx, item: &Item) -> El {
    let tk = c.tk;
    sensitive(
        share_label(item, c.hide_shares),
        tk.txs(400.0).features("'tnum' 1"),
        tk.muted_fg,
        c.hide,
    )
    .w(tk.r(2.5))
    .align(rune::Align::End)
}

/// `HoldingsListLayout`: icon, name, latest activity (from 56 rem), value, share. No "last
/// synced" note: the widget is a glance, the wallet page has it.
pub fn list(c: &Ctx, cx: &Cx<App>, rows: &[Item]) -> El {
    let tk = c.tk;
    let wide = c.inner >= tk.r(56.0);
    let els = rows
        .iter()
        .map(|item| {
            let mut r = row()
                .key(row_key(c, item))
                .w_full()
                .min_w(0.0)
                .h(c.row_h)
                .px(tk.r(0.25))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .child(icon_cell(c, cx, item))
                .child(source_name(c, item, false).grow(1.0).w(0.0));
            if wide {
                r = r.child(
                    text(date10(item))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.muted_fg)
                        .w(tk.r(5.5))
                        .align(rune::Align::End)
                        .truncate(),
                );
            }
            source_link(
                c,
                item,
                r.child(value_cell(c, item, tk.r(6.5)))
                    .child(share_cell(c, item)),
            )
        })
        .collect();
    two_columns(tk, els, c.inner >= tk.r(44.0))
}

/// `HoldingsTilesLayout`: a tile per source, the figure large, then its other unit, share,
/// latest activity and transaction count. A few sources fill the row; many wrap
/// (`repeat(auto-fit, minmax(13.5rem, 1fr))`).
pub fn tiles(c: &Ctx, cx: &Cx<App>, rows: &[Item]) -> El {
    let tk = c.tk;
    let gap = tk.r(0.5);
    let fit = (((c.inner + gap) / (tk.r(13.5) + gap)).floor() as usize).max(1);
    let cols = fit.min(rows.len()).max(1);
    let tile_w = (c.inner - gap * (cols as f32 - 1.0)) / cols as f32;
    let mut grid = col().w_full().gap(gap).role("list");
    for chunk in rows.chunks(cols) {
        let mut line = row().w_full().gap(gap).items_stretch();
        for item in chunk {
            line = line.child(tile(c, cx, item).w(tile_w));
        }
        grid = grid.child(line);
    }
    grid
}

/// A source tile's height in rem.
pub const TILE_H: f32 = 8.25;

fn tile(c: &Ctx, cx: &Cx<App>, item: &Item) -> El {
    let tk = c.tk;
    // The other unit underneath: bitcoin under fiat, fiat under bitcoin.
    let secondary = if c.btc_mode {
        c.fiat_label(item.value).unwrap_or_default()
    } else {
        mask(format::btc(item.btc, 8, false), c.hide)
    };
    let mut top = row().w_full().min_w(0.0).gap(tk.r(0.5)).center_y();
    if let Some(i) = source_icon(c, cx, item) {
        top = top.child(i);
    }
    top = top
        .child(source_name(c, item, false).grow(1.0).w(0.0))
        .child(sensitive(
            share_label(item, c.hide_shares),
            tk.txs(400.0).features("'tnum' 1"),
            tk.muted_fg,
            c.hide,
        ));
    let mut body = col()
        .key(row_key(c, item))
        .min_w(0.0)
        .gap(tk.r(0.375))
        .h(TILE_H * tk.rem)
        .padding(tk.r(0.75))
        .radius(tk.r(0.75))
        .continuous_corners()
        .fill(tk.fg.alpha(0.05))
        .child(top)
        .child(sensitive(
            c.value_label(item.btc, item.value),
            tk.tlg(600.0).features("'tnum' 1"),
            tk.fg,
            c.hide,
        ))
        .child(sensitive(
            secondary,
            tk.txs(400.0).features("'tnum' 1"),
            tk.muted_fg,
            c.hide,
        ));
    if item.empty {
        body = body.opacity(0.6);
    }
    if item.connection_id.is_some() {
        let date = date10(item);
        let foot = tk.t2xs(400.0).features("'tnum' 1");
        // `mt-auto`: the footer sits at the tile's bottom when a row of tiles stretches it.
        body = body.child(
            col()
                .w_full()
                .grow(1.0)
                .justify_end()
                .gap(tk.r(0.375))
                .child(
                    row()
                        .w_full()
                        .gap(tk.r(0.5))
                        .justify_between()
                        .child(
                            text(if date.is_empty() { "—".into() } else { date })
                                .font(foot)
                                .color(tk.muted_fg),
                        )
                        .child(
                            text(tc(
                                "overview:holdings.transactions",
                                item.transaction_count as usize,
                                &[],
                            ))
                            .font(foot)
                            .color(tk.muted_fg),
                        ),
                ),
        );
    }
    source_link(c, item, body)
}

/// `HoldingsTableLayout`: every figure in columns, for the reader who wants exact numbers.
/// HTML's automatic table layout: the source takes 24 %, the other columns share the rest in
/// proportion to their widest content.
pub fn table(c: &Ctx, cx: &Cx<App>, rows: &[Item]) -> El {
    let tk = c.tk;
    let xs = tk.txs(400.0).features("'tnum' 1");
    let head_style = tk.txs(500.0);
    use crate::ui::table as tb;
    let heads = [
        "network",
        "bitcoin",
        "value",
        "share",
        "transactions",
        "lastActivity",
    ]
    .map(|k| t(&format!("overview:holdings.table.{k}")));
    let cells: Vec<[String; 6]> = rows
        .iter()
        .map(|item| {
            let linked = item.connection_id.is_some();
            [
                if linked {
                    t(&format!("overview:holdings.rail.{}", item.group.id()))
                } else {
                    String::new()
                },
                mask(format::btc(item.btc, 8, false), c.hide),
                c.fiat_label(item.value).unwrap_or_else(|| "—".into()),
                share_label(item, c.hide_shares),
                if linked {
                    format!("{}", item.transaction_count as i64)
                } else {
                    String::new()
                },
                date10(item),
            ]
        })
        .collect();
    let measure = |s: &str, style: rune::TextStyle| rune::text::measure(s, &[], style, None).0;
    let mut max_content = [0.0f32; 6];
    for (i, h) in heads.iter().enumerate() {
        max_content[i] = measure(h, head_style);
    }
    for r in &cells {
        for (i, v) in r.iter().enumerate() {
            let style = if i == 2 {
                tk.txs(500.0).features("'tnum' 1")
            } else {
                xs
            };
            max_content[i] = max_content[i].max(measure(v, style));
        }
    }
    let pad = tk.r(0.5) * 2.0;
    let width = c.inner.max(tk.r(44.0));
    let source_w = width * 0.24;
    let sum: f32 = max_content.iter().map(|w| w + pad).sum();
    let widths: Vec<f32> = max_content
        .iter()
        .map(|w| (w + pad) * (width - source_w) / sum.max(1.0))
        .collect();
    let cell = |w: f32, end: bool, child: El| {
        let r = row().w(w).px(tk.r(0.5)).py(tk.r(0.375)).center_y();
        if end {
            r.justify_end().child(child)
        } else {
            r.child(child)
        }
    };
    // The table kit's compact header band (this table sits in a small widget card).
    let hcell = |w: f32, end: bool, child: El| {
        let r = row()
            .w(w)
            .h(tk.r(tb::HEAD_H_COMPACT))
            .px(tk.r(tb::PX))
            .center_y()
            .min_w(0.0);
        if end {
            r.justify_end().child(child)
        } else {
            r.child(child)
        }
    };
    let mut head = tb::head(tk).w(width).child(hcell(
        source_w,
        false,
        tb::th_text_compact(tk, t("overview:holdings.table.source")),
    ));
    for (i, h) in heads.iter().enumerate() {
        head = head.child(hcell(widths[i], i > 0, tb::th_text_compact(tk, h.clone())));
    }
    let mut table = col().w(width).role("table").child(head);
    let n = rows.len();
    for (ri, (item, vals)) in rows.iter().zip(cells).enumerate() {
        let mut name = row()
            .min_w(0.0)
            .gap(tk.r(0.5))
            .center_y()
            .px(tk.r(0.25))
            .py(tk.r(0.25))
            .radius(tk.r(0.375));
        // `w-5` around the 1.5 rem logo: it overhangs its column a little either side.
        let icon = match source_icon(c, cx, item) {
            Some(i) => row()
                .w(tk.r(1.25))
                .justify_center()
                .child(i.mx(-tk.r(0.125))),
            None => row().w(tk.r(1.25)).h(tk.r(1.5)),
        };
        name = name.child(icon).child(source_name(c, item, false).shrink());
        let name = source_link(c, item, name.key(format!("{}-link", row_key(c, item))));
        let mut r = row()
            .key(row_key(c, item))
            .w(width)
            .center_y()
            .role("row")
            .hover(tk.muted.alpha(0.35))
            .child(
                row()
                    .w(source_w)
                    .px(tk.r(0.25))
                    .py(tk.r(0.125))
                    .min_w(0.0)
                    .child(name.max_w(source_w - tk.r(0.5))),
            );
        for (i, v) in vals.into_iter().enumerate() {
            let (style, color, hidden) = match i {
                0 => (xs, tk.muted_fg, false),
                1 => (xs, if item.empty { tk.muted_fg } else { tk.fg }, c.hide),
                2 => (
                    if item.empty {
                        xs
                    } else {
                        tk.txs(500.0).features("'tnum' 1")
                    },
                    if item.empty { tk.muted_fg } else { tk.fg },
                    c.hide,
                ),
                3 => (xs, tk.muted_fg, c.hide),
                _ => (xs, tk.muted_fg, false),
            };
            r = r.child(cell(widths[i], i > 0, sensitive(v, style, color, hidden)));
        }
        table = table.child(r);
        if ri + 1 < n {
            table = table.child(hairline(tk.border));
        }
    }
    let wrap = col().w_full().child(table);
    if width > c.inner + 0.5 {
        wrap.scroll_x()
    } else {
        wrap
    }
}

/// `HoldingsTrendLayout`: the list with each source's last twelve months beside it — which
/// wallets fill up, which drain, and by how much. Two columns only from 72 rem.
pub fn trend(c: &Ctx, cx: &Cx<App>, rows: &[Item]) -> El {
    let tk = c.tk;
    let els = rows
        .iter()
        .map(|item| {
            let trend = item.trend.clone().unwrap_or_default();
            let change = if trend.len() > 1 {
                trend[trend.len() - 1] - trend[0]
            } else {
                0.0
            };
            let direction = if change.abs() < 1e-8 {
                0
            } else if change > 0.0 {
                1
            } else {
                -1
            };
            let spark = if trend.len() > 1 {
                sparkline(tk, &trend, direction)
            } else {
                el()
            };
            let spark_cell =
                row()
                    .w(tk.r(6.0))
                    .center_y()
                    .child(if c.hide { spark.blur(3.5) } else { spark });
            let change_text = if trend.len() < 2 {
                String::new()
            } else if direction == 0 {
                "—".into()
            } else {
                mask(format::btc(change, 3, true), c.hide)
            };
            let color = match direction {
                1 => tk.good.solid,
                -1 => tk.alert.solid,
                _ => tk.muted_fg,
            };
            let mut change_el = sensitive(
                change_text,
                tk.txs(400.0).features("'tnum' 1"),
                color,
                c.hide,
            )
            .w(tk.r(5.0))
            .align(rune::Align::End)
            .truncate();
            if trend.len() > 1 && !c.hide {
                change_el = change_el.help(tr(
                    "overview:holdings.trendChange",
                    &[("change", &format::btc(change, 4, true))],
                ));
            }
            let r = row()
                .key(row_key(c, item))
                .w_full()
                .min_w(0.0)
                .h(tk.r(2.0))
                .px(tk.r(0.25))
                .gap(tk.r(0.5))
                .center_y()
                .radius(tk.r(0.375))
                .child(icon_cell(c, cx, item))
                .child(source_name(c, item, false).grow(1.0).w(0.0))
                .child(spark_cell)
                .child(change_el)
                .child(value_cell(c, item, tk.r(6.5)))
                .child(share_cell(c, item));
            source_link(c, item, r)
        })
        .collect();
    two_columns(tk, els, c.inner >= tk.r(72.0))
}

/// A line of month-end balances scaled to its own range: the shape, not the magnitude. The web
/// draws it 96 × 20 CSS px whatever the UI scale.
fn sparkline(tk: &Tk, values: &[f64], direction: i32) -> El {
    let (w, h) = (96.0f32, 20.0f32);
    let min = values.iter().copied().fold(f64::INFINITY, f64::min);
    let max = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let span = if max - min == 0.0 { 1.0 } else { max - min };
    let pts: Vec<[f32; 2]> = values
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let x = i as f32 / (values.len() - 1) as f32 * w;
            let y = h - 1.0 - ((v - min) / span) as f32 * (h - 2.0);
            [x, y]
        })
        .collect();
    // One run per straight stretch: the stroke's caps would otherwise overlap (and show) at
    // every month of a flat line.
    let mut pts: Vec<[f32; 2]> = pts.into_iter().fold(Vec::new(), |mut acc, p| {
        if acc.len() >= 2 {
            let [a, b] = [acc[acc.len() - 2], acc[acc.len() - 1]];
            if ((b[0] - a[0]) * (p[1] - a[1]) - (b[1] - a[1]) * (p[0] - a[0])).abs() < 1e-3 {
                acc.pop();
            }
        }
        acc.push(p);
        acc
    });
    pts.dedup();
    let stroke = match direction {
        1 => hex_c("#10b981"),
        -1 => hex_c("#ef4444"),
        _ => tk.muted_fg,
    };
    canvas(Shape::new().stroke_with(pts, false, 1.5, LineCap::Round, stroke))
        .size(w, h)
        .inert()
}

fn hex_c(s: &str) -> Color {
    rune::color::hex(s).unwrap_or(Color::TRANSPARENT)
}
