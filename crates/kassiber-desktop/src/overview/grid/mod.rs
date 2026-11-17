//! The Overview's widgets laid out like a phone's home screen (Rune only; the Tauri Overview
//! keeps its fixed layout): a uniform grid of square tiles with equal gutters, each widget at
//! one of a few fixed sizes ([`model::Size`]), packed in reading order. A click on a tile opens
//! the widget's destination; a secondary click opens its menu (size, remove, edit). A long
//! press, the menu or the page's Edit button starts edit mode ([`edit`]): tiles wiggle, drag to
//! reorder, carry a remove badge and a size control, and the gallery ([`gallery`]) adds more.
//! A widget that is not placed is not mounted.

pub mod copy;
mod edit;
pub mod frame;
mod gallery;
pub mod model;
pub mod stat;
#[cfg(test)]
mod tests;

use std::rc::Rc;

use rune::{Anim, Color, Cx, El, Icon, KeyEvent, Shadow, Spring, Tx, col, text};
use rune_kit::Placement;

pub use edit::{Edit, header, poll};
use model::{Cell, Layout, Size, WidgetId, pack, rows};

use super::menu::{self, Entry};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::model::Snapshot;
use crate::theme::Tk;

/// What the open context menu is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuTarget {
    Widget(WidgetId),
    Page,
}

impl Default for MenuTarget {
    fn default() -> Self {
        MenuTarget::Widget(WidgetId::Holdings)
    }
}

/// The tile grid at a page width: columns from the width (four at the 1440 pt window, six from
/// the 1920 pt one, two at the 980 pt minimum), one gutter, and a height per row. A row is as
/// tall as the tallest content of the one-row tiles that start in it (each widget says what it
/// needs, [`natural_h`]), never taller than the square-ish base row; a row only taller tiles
/// cross keeps the base height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Geo {
    pub cols: u8,
    pub col_w: f32,
    /// The base (and tallest) row height.
    pub row_h: f32,
    pub gap: f32,
    rows: [f32; MAX_ROWS],
}

/// Rows with their own height; any further ones take the base height.
const MAX_ROWS: usize = 48;

/// The narrowest a grid column gets, in rem.
pub const MIN_COL: f32 = 15.0;

impl Geo {
    pub fn new(tk: &Tk, width: f32) -> Geo {
        // The app's card gap (`Tk::card_gap`) between tiles, across and down.
        let gap = tk.card_gap();
        // An even count, so two Medium tiles always fill a row and nothing strands a column.
        // Six columns from a 1 510 pt page (the 1 920 pt window, or 1 728 pt with the sidebar
        // collapsed), so the default's right column of Recent transactions over Holdings by
        // Source holds wherever a tile can stay at least 15 rem wide.
        let fit = ((width + gap) / (tk.r(MIN_COL) + gap)).floor() as i32;
        let cols = (fit / 2 * 2).clamp(2, 6) as u8;
        let col_w = ((width - f32::from(cols - 1) * gap) / f32::from(cols)).max(1.0);
        let row_h = (col_w * 0.8).min(tk.r(15.0));
        Geo {
            cols,
            col_w,
            row_h,
            gap,
            rows: [row_h; MAX_ROWS],
        }
    }

    /// The same grid with these row heights (from the first row; each at most the base).
    pub fn with_rows(mut self, heights: &[f32]) -> Geo {
        for (i, h) in heights.iter().take(MAX_ROWS).enumerate() {
            self.rows[i] = h.clamp(1.0, self.row_h);
        }
        self
    }

    /// Row `y`'s height.
    pub fn row(&self, y: u16) -> f32 {
        self.rows.get(usize::from(y)).copied().unwrap_or(self.row_h)
    }

    /// The size of a `w` × `h` tile span at base row heights (the gallery's preview).
    pub fn span(&self, w: u8, h: u8) -> (f32, f32) {
        (
            f32::from(w) * self.col_w + f32::from(w.saturating_sub(1)) * self.gap,
            f32::from(h) * self.row_h + f32::from(h.saturating_sub(1)) * self.gap,
        )
    }

    /// The size of a `w` × `h` tile span starting in row `y`.
    pub fn span_at(&self, y: u16, w: u8, h: u8) -> (f32, f32) {
        let tall: f32 = (0..u16::from(h)).map(|i| self.row(y + i)).sum::<f32>()
            + f32::from(h.saturating_sub(1)) * self.gap;
        (
            f32::from(w) * self.col_w + f32::from(w.saturating_sub(1)) * self.gap,
            tall,
        )
    }

    /// The top left of the tile at column `x`, row `y`.
    pub fn origin(&self, x: u8, y: u16) -> (f32, f32) {
        (
            f32::from(x) * (self.col_w + self.gap),
            (0..y).map(|i| self.row(i) + self.gap).sum(),
        )
    }

    /// The height of `n` rows with their gutters.
    pub fn total(&self, n: u16) -> f32 {
        if n == 0 {
            0.0
        } else {
            (0..n).map(|i| self.row(i)).sum::<f32>() + f32::from(n - 1) * self.gap
        }
    }

    fn rect(&self, c: &Cell) -> (f32, f32, f32, f32) {
        let (x, y) = self.origin(c.x, c.y);
        let (w, h) = self.span_at(c.y, c.w, c.h);
        (x, y, w, h)
    }
}

/// The row heights of a packed arrangement: per row, the tallest [`natural_h`] of the one-row
/// tiles that sit in it; the base height where none does.
pub fn row_heights(geo: &Geo, cells: &[Cell], natural: impl Fn(&Cell, f32) -> f32) -> Vec<f32> {
    let n = usize::from(rows(cells));
    let mut out = vec![0.0f32; n];
    for c in cells.iter().filter(|c| c.h == 1) {
        let (w, _) = geo.span(c.w, 1);
        let want = natural(c, w).clamp(1.0, geo.row_h);
        let r = &mut out[usize::from(c.y)];
        *r = r.max(want);
    }
    out.into_iter()
        .map(|h| if h <= 0.0 { geo.row_h } else { h })
        .collect()
}

/// Where a dragged widget goes when its centre is at (`x`, `y`) in the grid: the tile under the
/// point and that tile's place in the order. `None` over its own tile or a gutter; below the
/// last row it is the end of the list.
pub fn drop_target(
    layout: &Layout,
    geo: Geo,
    dragged: WidgetId,
    x: f32,
    y: f32,
) -> Option<(WidgetId, usize)> {
    let cells = pack(layout.tiles(), geo.cols);
    let total = geo.total(rows(&cells));
    if y > total {
        let last = layout.tiles().last()?;
        return (last.id != dragged).then(|| (last.id, layout.len() - 1));
    }
    let hit = cells.iter().find(|c| {
        let (cx, cy, w, h) = geo.rect(c);
        x >= cx && x < cx + w && y >= cy && y < cy + h
    })?;
    if hit.id == dragged {
        return None;
    }
    Some((hit.id, layout.index_of(hit.id)?))
}

/// A widget's name.
pub fn title(app: &App, id: WidgetId) -> String {
    match id {
        WidgetId::Stats => copy::stats_title(),
        WidgetId::Chart => t("overview:treasury.title"),
        WidgetId::Recent => t("overview:recentTx.title"),
        WidgetId::Holdings => t("overview:holdings.title"),
        WidgetId::TaxYear => super::widgets::tax_title(app),
        WidgetId::Flows => t("overview:widgets.flows.title"),
        WidgetId::Assets => super::asset_card::widget_title(),
        WidgetId::Attention => super::attention::title(),
        WidgetId::Age => super::age::title(),
        WidgetId::Goal => super::goal::title(),
    }
}

pub fn icon(id: WidgetId) -> &'static Icon {
    match id {
        WidgetId::Stats => &I::BITCOIN,
        WidgetId::Chart => &I::CHART_LINE,
        WidgetId::Recent => &I::HISTORY,
        WidgetId::Holdings => &I::WALLET,
        WidgetId::TaxYear => &I::RECEIPT,
        WidgetId::Flows => &I::ARROW_DOWN_UP,
        WidgetId::Assets => &I::COINS,
        WidgetId::Attention => &I::BELL,
        WidgetId::Age => &I::HOURGLASS,
        WidgetId::Goal => &I::TARGET,
    }
}

/// Where a click on the widget goes, and what that is called. The chart opens nothing: its
/// tile is the chart. Needs attention goes to its most serious item's screen (its rows each
/// go to their own); Stack goal opens its sheet (see [`action`]).
pub fn dest(app: &App, id: WidgetId) -> Option<(&'static str, String)> {
    match id {
        WidgetId::Stats | WidgetId::Holdings | WidgetId::Assets => {
            Some(("/connections", t("overview:holdings.viewConnections")))
        }
        WidgetId::Recent | WidgetId::Flows => {
            Some(("/transactions", t("overview:recentTx.showAll")))
        }
        WidgetId::TaxYear | WidgetId::Age => {
            Some(("/reports", t("overview:widgets.tax.openReports")))
        }
        WidgetId::Attention => super::attention::first(app),
        WidgetId::Chart | WidgetId::Goal => None,
    }
}

/// What a click on the tile does, and what that is called: its destination, or Stack goal's sheet.
pub fn action(app: &App, id: WidgetId) -> Option<(String, crate::app::Handler)> {
    if id == WidgetId::Goal {
        let label = if app.ov.layout.goal_sats().is_some() {
            super::goal::edit_goal()
        } else {
            super::goal::set_goal()
        };
        return Some((label, Rc::new(super::goal::open_sheet)));
    }
    let (href, label) = dest(app, id)?;
    Some((label, Rc::new(move |a: &mut App| a.navigate(href))))
}

/// The placed tiles the grid shows: all of them while editing; otherwise Needs attention only
/// while something needs attention (signal, never a standing "all good").
pub fn shown_tiles(app: &App, loaded: bool) -> Vec<model::Tile> {
    let tiles = app.ov.layout.tiles();
    if app.ov.edit.on
        || !loaded
        || !app.ov.layout.contains(WidgetId::Attention)
        || super::attention::has_items(app)
    {
        return tiles.to_vec();
    }
    tiles
        .iter()
        .copied()
        .filter(|t| t.id != WidgetId::Attention)
        .collect()
}

/// The tile's key (`pv-` prefixed in a gallery preview).
pub fn frame_key(id: WidgetId, pv: bool) -> String {
    let k = match id {
        WidgetId::Stats => "stats",
        WidgetId::Chart => "btc-activity-card",
        WidgetId::Recent => "recent-transactions",
        WidgetId::Holdings => "holdings",
        WidgetId::TaxYear => "tax-year-widget",
        WidgetId::Flows => "flows-widget",
        WidgetId::Assets => "holdings-by-asset",
        WidgetId::Attention => "attention-widget",
        WidgetId::Age => "holding-age-widget",
        WidgetId::Goal => "stack-goal-widget",
    };
    if pv { format!("pv-{k}") } else { k.to_owned() }
}

fn caption_key(id: WidgetId, pv: bool) -> String {
    format!("{}ov-cap-{}", if pv { "pv-" } else { "" }, id.id())
}

/// A control inside a widget: `f` on a click outside edit mode, and the tile's long press and
/// drag (a press goes to one element, so each control carries the tile's gesture too).
pub fn press(el: El, id: WidgetId, pv: bool, f: impl Fn(&mut App) + 'static) -> El {
    if pv {
        return el;
    }
    el.on_press(move |a: &mut App| {
        if !a.ov.edit.on {
            f(a);
        }
    })
    .on_drag(edit::drag(id))
}

/// One widget in its tile, `w` × `h`. `s` is `None` while the book loads.
#[allow(clippy::too_many_arguments)]
pub fn tile(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    s: Option<&Snapshot>,
    id: WidgetId,
    size: Size,
    w: f32,
    h: f32,
    pv: bool,
    layer: &mut Vec<El>,
) -> El {
    let name = title(app, id);
    let slot = frame::slot(tk, size, w, h, pv);
    let shell = frame::shell(tk, frame_key(id, pv), name.clone(), w, h);
    let caption = frame::caption(tk, icon(id), &name).key(caption_key(id, pv));
    let Some(s) = s else {
        return shell.child(caption).child(frame::skeleton(tk, slot.h));
    };
    if id == WidgetId::Chart {
        if pv {
            let ink = if tk.dark {
                Color::WHITE.alpha(0.85)
            } else {
                tk.fg.alpha(0.8)
            };
            return shell.child(caption).child(frame::spark(
                &s.balance_series,
                slot.w,
                slot.h,
                ink,
            ));
        }
        // The chart brings its own header (title, figures, expand); its plot takes what is left.
        app.ov.chart_fit.set(chart_fit(
            app.ov.chart_fit.get(),
            h - frame::pad_top(tk) - frame::pad_bottom(tk),
            cx.layout_size("btc-activity").map(|m| m[1]),
        ));
        return shell.child(crate::chart::view(app, cx, tk, s, layer));
    }
    let x = super::holdings_model::Extras::parse(app.read("ui.overview.snapshot"));
    let body = match id {
        WidgetId::Stats => super::stats::body(app, tk, s, &x, &slot),
        WidgetId::Recent => super::recent::body(app, tk, s, &slot),
        WidgetId::Holdings => super::holdings::body(app, cx, tk, s, &x, &slot),
        WidgetId::TaxYear => super::widgets::tax_year(app, tk, s, &x, &slot),
        WidgetId::Flows => super::widgets::flows(app, tk, s, &slot),
        WidgetId::Assets => super::asset_card::body(app, cx, tk, s, &slot),
        WidgetId::Attention => super::attention::body(app, tk, s, &slot),
        WidgetId::Age => super::age::body(app, tk, s, &x, &slot),
        WidgetId::Goal => super::goal::body(app, tk, s, &x, &slot),
        WidgetId::Chart => unreachable!(),
    };
    shell
        .child(caption)
        .child(col().w_full().h(slot.h).min_h(0.0).child(body))
}

/// The chart's plot block height and what it was fitted to: (block, tile inner height, the
/// chart's measured height the block was last corrected for).
pub type ChartFit = (f32, f32, f32);

/// Fits the chart's block so the whole chart (header, legend, plot) is as tall as the tile's
/// inside: the measured height says how much the header and legend take at this width. One
/// correction per measurement, so a measurement a frame behind never corrects twice.
fn chart_fit(prev: ChartFit, inner_h: f32, measured: Option<f32>) -> ChartFit {
    let (block, fit_h, seen) = prev;
    if block <= 0.0 {
        return ((inner_h - 70.0).max(160.0), inner_h, -1.0);
    }
    let Some(m) = measured else {
        return (block, inner_h, seen);
    };
    if (m - seen).abs() < 0.05 && (fit_h - inner_h).abs() < 0.05 {
        return prev;
    }
    if (m - inner_h).abs() < 0.5 {
        return (block, inner_h, m);
    }
    ((block + inner_h - m).max(160.0), inner_h, m)
}

/// The chart's plot block height in its tile (`src/chart` reads it).
pub fn chart_block_h(app: &App, tk: &Tk) -> f32 {
    let (block, ..) = app.ov.chart_fit.get();
    if block > 0.0 { block } else { tk.r(24.0) }
}

/// The chart header's actions in its tile: expand.
pub fn chart_actions(_app: &App, _tk: &Tk, expand: El) -> El {
    expand
}

pub fn open_menu(a: &mut App, target: MenuTarget, at: Option<(f32, f32)>) {
    a.ov.menu_target = target;
    a.ov.menu_at = at;
    a.ov.menu_sub = None;
    a.ov.menu_open = true;
}

/// The tiles. `width` is the page's content width.
pub fn view(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    s: Option<&Snapshot>,
    width: f32,
    layer: &mut Vec<El>,
) -> El {
    let base = Geo::new(tk, width);
    app.ov.reduced.set(cx.is_reduced_motion());
    if app.ov.edit.gallery.is_some() {
        gallery::mount(app, cx, tk, s, layer);
    }
    let shown = shown_tiles(app, s.is_some());
    let tiles = shown.as_slice();
    if app.ov.layout.tiles().is_empty() {
        return col()
            .key("ov-grid")
            .w_full()
            .h(tk.r(16.0))
            .center()
            .gap(tk.r(0.75))
            .on_secondary_press(|a: &mut App, p: rune::PointerPress, _: &mut Tx| {
                open_menu(a, MenuTarget::Page, Some((p.x, p.y)))
            })
            .child(
                text(copy::empty_home())
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            )
            .child(
                crate::ui::outline_button(tk, "ov-add-empty", Some(&I::PLUS), &copy::add_widget())
                    .on_press(gallery::open),
            );
    }
    // In reading order, so focus, the wiggle and "position 2 of 5" follow what is seen.
    let mut cells = pack(tiles, base.cols);
    cells.sort_by_key(|c| (c.y, c.x));
    let heights = row_heights(&base, &cells, |c, w| {
        natural_h(app, tk, s, c.id, c.size, w, base.row_h)
    });
    let geo = base.with_rows(&heights);
    app.ov.geo.set(Some(geo));
    let n = cells.len();
    let total = geo.total(rows(&cells));
    let mut grid = col()
        .key("ov-grid")
        .role("group")
        .label(copy::grid_aria())
        .w_full()
        .h(total)
        .on_secondary_press(|a: &mut App, p: rune::PointerPress, _: &mut Tx| {
            open_menu(a, MenuTarget::Page, Some((p.x, p.y)))
        });
    for (i, c) in cells.iter().enumerate() {
        grid = grid.child(cell(app, cx, tk, s, geo, c, i, n, layer));
    }
    grid
}

/// The height a one-row tile's content needs at width `w` (at most `base`); widgets that fill
/// whatever they get (lists, bars, the chart) take the base height.
pub fn natural_h(
    app: &App,
    tk: &Tk,
    s: Option<&Snapshot>,
    id: WidgetId,
    size: Size,
    _w: f32,
    base: f32,
) -> f32 {
    let Some(s) = s else { return base };
    let content = match id {
        WidgetId::Stats => Some(super::stats::natural(tk)),
        WidgetId::TaxYear => Some(super::widgets::tax_natural(app, tk, s, size)),
        WidgetId::Attention => Some(super::attention::natural(app, tk, s, size)),
        WidgetId::Age => Some(super::age::natural(app, tk, size)),
        WidgetId::Goal => Some(super::goal::natural(tk, size)),
        _ => None,
    };
    content.map_or(base, |c| (c + frame::chrome_h(tk)).ceil().min(base))
}

/// The move of the other tiles when one is added, removed, resized or dragged past.
fn slide() -> Spring {
    Spring::new(0.42, 0.16)
}

#[allow(clippy::too_many_arguments)]
fn cell(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    s: Option<&Snapshot>,
    geo: Geo,
    c: &Cell,
    i: usize,
    n: usize,
    layer: &mut Vec<El>,
) -> El {
    let id = c.id;
    let (x, y, w, h) = geo.rect(c);
    let e = &app.ov.edit;
    let editing = e.on;
    let drag = e.drag.as_ref().filter(|d| d.id == id);
    let name = title(app, id);
    let mut content = tile(app, cx, tk, s, id, c.size, w, h, false, layer);
    // While editing the widget's own controls rest (the tile is the handle). A long press that
    // just started edit mode keeps them alive until release, so the same press goes on to drag.
    if editing && e.hold.is_none() {
        content = content.inert();
    }
    let r = frame::radius(tk);
    let mut handle = col()
        .key(format!("ov-tile-{}", id.id()))
        .size(w, h)
        .radius(r)
        .continuous_corners()
        .focus_ring(2.0, 2.0, tk.ring)
        .on_drag(edit::drag(id))
        .on_secondary_press(move |a: &mut App, p: rune::PointerPress, _: &mut Tx| {
            open_menu(a, MenuTarget::Widget(id), Some((p.x, p.y)))
        })
        .child(content);
    if editing {
        handle = handle
            .role("button")
            .label(copy::tile(&name, c.size, i + 1, n))
            .description(copy::keys())
            .focusable(true)
            .cursor(if drag.is_some() {
                rune::Cursor::Grabbing
            } else {
                rune::Cursor::Grab
            })
            .on_key(edit::key(id));
    } else if let Some((label, f)) = action(app, id) {
        handle = handle
            .role(if id == WidgetId::Goal {
                "button"
            } else {
                "link"
            })
            .label(copy::open_named(&name, &label))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .press_scale(0.985)
            .on_press(move |a: &mut App| {
                if !a.ov.edit.on {
                    f(a);
                }
            });
    }
    if drag.is_some() {
        handle = handle.box_shadow(Shadow {
            y: 18.0,
            blur: 44.0,
            spread: -8.0,
            color: Color::BLACK.alpha(if tk.dark { 0.6 } else { 0.28 }),
            ..Shadow::default()
        });
    }
    // The wiggle: a small turn about the centre, alternating; neighbours turn opposite ways.
    let angle = if editing && drag.is_none() && !app.ov.reduced.get() {
        let a = 1.6 / (w.max(h) / 2.0);
        if i.is_multiple_of(2) == e.flip { a } else { -a }
    } else {
        0.0
    };
    let mut wig = col()
        .size(w, h)
        .rotate(angle)
        .rotate_motion(Spring::new(edit::WIGGLE as f32, 0.0))
        .scale(if drag.is_some() { 1.04 } else { 1.0 })
        .scale_motion(Spring::SNAPPY)
        .child(handle);
    if editing {
        wig = wig
            .child(edit::remove_badge(tk, id, &name))
            .child(edit::size_pill(tk, id, &name, c.size, w, h));
    }
    let cell = col()
        .key(format!("ov-cell-{}", id.id()))
        .absolute(x, y)
        .size(w, h)
        .child(wig);
    match drag {
        // The dragged tile follows the pointer from where it was picked up, wherever its slot
        // in the order now is.
        Some(d) => cell
            .motion(Spring::NONE)
            .offset(d.home.0 + d.off.0 - x, d.home.1 + d.off.1 - y)
            .offset_motion(Anim::INSTANT)
            .raise(),
        None => cell.motion(slide()).offset(0.0, 0.0).offset_motion(slide()),
    }
}

/// Mounts the open context menu: at the pointer for a secondary click, under the widget's
/// caption otherwise.
pub fn mount_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if !app.ov.menu_open {
        return;
    }
    let target = app.ov.menu_target;
    let entries = match target {
        MenuTarget::Widget(id) if app.ov.layout.contains(id) => widget_entries(app, tk, id),
        _ => page_entries(),
    };
    let (anchor, placement) = match (app.ov.menu_at, target) {
        (Some((x, y)), _) => {
            // The pointer's place anchors the menu and names it (the widget, or the grid).
            let name = match target {
                MenuTarget::Widget(id) if app.ov.layout.contains(id) => title(app, id),
                _ => copy::grid_aria(),
            };
            layer.push(
                rune::el()
                    .key("ov-ctx")
                    .absolute(x, y)
                    .size(1.0, 1.0)
                    .label(name),
            );
            ("ov-ctx".to_owned(), Placement::BottomStart)
        }
        (None, MenuTarget::Widget(id)) if app.ov.layout.contains(id) => (
            if id == WidgetId::Chart {
                frame_key(id, false)
            } else {
                caption_key(id, false)
            },
            Placement::BottomStart,
        ),
        (None, _) => (
            (if app.ov.edit.on {
                "overview-done"
            } else {
                "overview-edit"
            })
            .to_owned(),
            Placement::BottomEnd,
        ),
    };
    menu::mount(
        tk,
        layer,
        "ov-menu",
        &anchor,
        tk.r(15.0),
        app.ov.menu_sub,
        entries,
        placement,
    );
}

fn item(key: &str, label: String, on: impl Fn(&mut App) + 'static) -> Entry {
    Entry::Item {
        key: key.to_owned(),
        label,
        on: Rc::new(on),
    }
}

fn widget_entries(app: &App, tk: &Tk, id: WidgetId) -> Vec<Entry> {
    let mut out = Vec::new();
    if id == WidgetId::Holdings {
        out.extend(super::holdings::menu_entries(app, tk));
        out.push(Entry::Sep);
    }
    match action(app, id) {
        Some((label, f)) => out.push(item("ov-menu-open", label, move |a| f(a))),
        None if id == WidgetId::Chart => out.push(item(
            "ov-menu-open",
            t("overview:treasury.expandChart"),
            |a| {
                a.chart.expanded = true;
                a.chart.inspect_col = None;
            },
        )),
        // Needs attention with nothing in it (seen only while editing): nowhere to go.
        None => {}
    }
    if id == WidgetId::Goal && app.ov.layout.goal_sats().is_some() {
        out.push(item("ov-menu-goal-clear", copy::clear_goal(), |a| {
            a.ov.layout.set_goal_sats(None);
        }));
    }
    if !out.is_empty() {
        out.push(Entry::Sep);
    }
    out.push(Entry::Label(copy::size_label()));
    let cur = app.ov.layout.size_of(id);
    for z in id.sizes() {
        let z = *z;
        out.push(Entry::Radio {
            key: format!("ov-menu-size-{}", z.id()),
            label: copy::size(z),
            checked: cur == Some(z),
            on: Rc::new(move |a: &mut App| {
                a.ov.layout.set_size(id, z);
            }),
        });
    }
    out.push(Entry::Sep);
    if !app.ov.edit.on {
        out.push(item("ov-menu-edit", copy::edit_home(), |a| {
            a.ov.edit.on = true
        }));
    }
    out.push(item("ov-menu-remove", copy::remove(), move |a| {
        a.ov.layout.remove(id);
    }));
    out
}

fn page_entries() -> Vec<Entry> {
    vec![
        item("ov-menu-edit", copy::edit_home(), |a| a.ov.edit.on = true),
        item("ov-menu-add", copy::add_widget_more(), gallery::open),
        Entry::Sep,
        item("ov-menu-reset", copy::reset(), |a| a.ov.layout.reset()),
    ]
}

/// Escape closes the gallery, then leaves edit mode.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if ev.key != rune::Key::Escape {
        return false;
    }
    if app.ov.edit.gallery.is_some() {
        app.ov.edit.gallery = None;
        return true;
    }
    if app.ov.edit.on {
        edit::exit(app, tx);
        return true;
    }
    false
}

/// A scene's open gallery on `id` at `size`.
pub fn scene_gallery(a: &mut App, id: WidgetId, size: Size) {
    a.ov.edit.gallery = Some(gallery::Gallery { sel: id, size });
}
