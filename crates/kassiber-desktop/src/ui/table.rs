//! Wide tables at narrow widths. The web wraps every `<Table>` in shadcn's container
//! (`relative w-full overflow-x-auto`) and gives the wide ones a `min-w-[…]`: below that width
//! the table keeps its columns and the card scrolls sideways instead of squeezing or clipping
//! them. These helpers are that container in Rune: a horizontal scroll viewport that is
//! labelled for assistive technology, a Tab stop while it overflows, and scrolled by the arrow
//! keys like a focused overflowing box in a browser (Rune's own viewports take the wheel,
//! trackpad and Shift + wheel; they leave Left and Right to the app).

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use rune::{El, Key, KeyEvent, ScrollPos, Tx, col};

use crate::app::App;

/// How far one Left or Right press moves the view: a browser's 40 px line step.
const STEP: f32 = 40.0;

thread_local! {
    /// Each table viewport's horizontal offset and range, as it last drew: the arrow keys step
    /// from where the wheel or a fling left it, which the view never re-runs for.
    static AT: RefCell<HashMap<Rc<str>, [f32; 2]>> = RefCell::new(HashMap::new());
}

/// The width viewport `key` had at the last layout. A layout read like `Cx::layout_size`,
/// usable where a view builds without its `Cx` (a table inside a chat message).
fn measured(key: &str) -> Option<f32> {
    rune::memo::layout_size(key)
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
}

/// Whether a viewport keyed `key` is narrower than `content_w`, the table's width (its
/// `min-w-[…]`, or the widths of the columns it shows): the table then scrolls sideways.
/// Before the first layout it reads as not overflowing.
pub fn overflows(key: &str, content_w: f32) -> bool {
    measured(key).is_some_and(|w| w + 0.5 < content_w)
}

/// The width a table takes in viewport `key`: the viewport's width, but never less than `min`
/// (`w-full min-w-[min]`). `fallback` is the viewport's width before its first layout.
pub fn width(key: &str, min: f32, fallback: f32) -> f32 {
    measured(key).unwrap_or(fallback).max(min)
}

/// Makes an existing scroll viewport (`scroll_x` or `scroll_xy`) keyed `key` a table's
/// scroll region: `role="region"` named `label` (as the web's scroll regions are), focusable
/// while `overflow` says the content is wider, and stepped by Left and Right while it has
/// focus (Home and End too on a horizontal-only one; a viewport that also scrolls down
/// leaves them to Rune, which pages it vertically).
pub fn region(viewport: El, key: &str, label: String, overflow: bool, both_axes: bool) -> El {
    let name: Rc<str> = Rc::from(key);
    let record = name.clone();
    let viewport = viewport
        .key(key.to_owned())
        .role("region")
        .label(label)
        .on_scroll_offset(move |at: ScrollPos| {
            AT.with(|m| {
                let mut m = m.borrow_mut();
                let v = [at.offset[0], at.max[0]];
                match m.get_mut(&record) {
                    Some(slot) => *slot = v,
                    None => {
                        m.insert(record.clone(), v);
                    }
                }
            })
        });
    if !overflow {
        // Nothing to scroll sideways: a two-finger horizontal swipe over it goes back or
        // forward, as over the rest of the page (Rune stops a wheel's handlers at the nearest
        // scroll view, so the page's own handler can't hear it here).
        return crate::nav::swipe_area(viewport);
    }
    viewport
        .focusable(true)
        .on_key(move |_s: &mut App, ev: &KeyEvent, tx: &mut Tx| {
            if tx.focused() != Some(&*name) || ev.mods.cmd || ev.mods.ctrl || ev.mods.alt {
                return false;
            }
            let [x, max] = AT
                .with(|m| m.borrow().get(&name).copied())
                .unwrap_or([0.0, f32::MAX]);
            let to = match ev.key {
                Key::Left => x - STEP,
                Key::Right => x + STEP,
                Key::Home if !both_axes => 0.0,
                Key::End if !both_axes => max,
                _ => return false,
            };
            tx.scroll_to_x(name.to_string(), to.clamp(0.0, max));
            true
        })
}

/// The shadcn `Table` container around `table`, a table laid out `content_w` wide: a
/// horizontal scroll viewport keyed `key` that fills its parent's width ([`region`] says what
/// else it does). Give `table` its own width (`width(…)` above); padding the web puts inside
/// its `overflow-x-auto` box goes on `table` too, so it scrolls with the rows.
pub fn hscroll(key: &str, label: String, content_w: f32, table: El) -> El {
    hscroll_if(key, label, overflows(key, content_w), table)
}

/// [`hscroll`] for a view that already knows whether the table is wider than its card
/// (`overflow`), from widths it laid out itself.
pub fn hscroll_if(key: &str, label: String, overflow: bool, table: El) -> El {
    region(
        col().w_full().min_w(0.0).scroll_x().child(table),
        key,
        label,
        overflow,
        false,
    )
}

/// The width a table laid out from its own column widths takes: `avail` (the card's inner
/// width), or `least` (what its columns need) when that's more, and whether it overflows.
pub fn fit(avail: f32, least: f32) -> (f32, bool) {
    (avail.max(least), least > avail + 0.5)
}

// ---- The table kit ------------------------------------------------------------------------
//
// One look for every table and tabular list (the owner asked for it: "they all look very
// different"). The rules, in rem unless noted:
//
// - Header band: `HEAD_H` (2.5) tall, `bg-muted/50`, a 1 pt rule under it; labels `text-sm
//   font-medium text-muted-foreground`, sentence case, never uppercase. A sortable column's
//   label is a button with the lucide sort glyph after it (`ArrowUpDown` idle, `ArrowUp` /
//   `ArrowDown` active); numeric columns put their label on the right.
// - Cells: `PX` (0.5) each side, `PY` (0.5) above and below (`PY_COMPACT` 0.375 for one-line
//   lists); a table that runs to its card's edges insets its first and last cell by `EDGE`
//   (1.5), the card header's own inset, so header, rows and card title line up (a card with
//   a tighter header, as Wallet detail's 1 rem, uses that inset instead). A table inside a
//   small widget or picker with `text-xs` cells uses the compact header (`HEAD_H_COMPACT`,
//   [`th_text_compact`]).
// - Rows: a 1 pt rule under each (none under the last), hover `bg-muted/35`, selected
//   `bg-muted/60`, keyboard focus `bg-muted/35`; a row that opens something shows the pointer.
// - Leading visual: one 1.5 rem slot (`LEAD`). A row about a network or asset shows its rail
//   logo (`crate::logos::rail`, 20 px art in the 24 px slot); a row about a wallet or source
//   its `ASSET_BADGE_SM` badge; everything else a 1 rem lucide glyph centred in the slot
//   ([`lead_glyph`]). Direction (in, out, between own wallets) is that glyph on a tinted
//   tile ([`lead_tile`]), never a loose arrow or a word.
// - Numbers: right-aligned, tabular figures; the primary figure `text-sm font-medium`, its
//   other currency or fee under it `text-xs text-muted-foreground` ([`num`], [`num_sub`]).
// - Status: one chip ([`chip`]: `rounded-md px-1.5 py-0.5 text-xs font-medium`, tinted with
//   a 1 pt ring), drawn only when something needs attention; an OK state draws nothing.
// - Text that can run long truncates with an ellipsis and shows the whole text in a tooltip
//   while it is cut ([`cell_text`]); nothing clips mid-glyph or overlaps the next column.
// - Empty, loading and error bodies: one centred `text-sm text-muted-foreground` line in a
//   6 rem band ([`empty`]).
// - One visible action per row; the rest live in its "⋯" menu (a 1.75 rem ghost button).

use rune::{Color, El as Node, Icon, TextStyle, row as hrow, text};

use crate::theme::Tk;

/// Header band height.
pub const HEAD_H: f32 = 2.5;
/// The compact header (a table inside a small widget card, its cells `text-xs`).
pub const HEAD_H_COMPACT: f32 = 2.0;
/// A cell's horizontal padding.
pub const PX: f32 = 0.5;
/// A cell's vertical padding (two-line rows and most tables).
pub const PY: f32 = 0.5;
/// A one-line list's vertical padding.
pub const PY_COMPACT: f32 = 0.375;
/// The first and last cell's inset in a table that runs to its card's edges: the app's card
/// padding (`theme::CARD_PAD`), so a table's rows line up with every other card's content.
pub const EDGE: f32 = crate::theme::CARD_PAD;
/// The leading visual's slot (the rail logo's 24 px slot, `crate::logos::role::RAIL`).
pub const LEAD: f32 = 1.5;
/// The gap between the leading visual and the text beside it.
pub const LEAD_GAP: f32 = 0.5;

/// The header band: `bg-muted/50`, a rule under it.
pub fn head(tk: &Tk) -> Node {
    hrow()
        .w_full()
        .fill(tk.muted.alpha(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .role("row")
}

/// A header cell `w` wide (None: the flexible column), its label or control inside.
pub fn th(tk: &Tk, w: Option<f32>, right: bool, content: Node) -> Node {
    let c = hrow()
        .h(tk.r(HEAD_H))
        .px(tk.r(PX))
        .center_y()
        .min_w(0.0)
        .child(content);
    let c = match w {
        Some(w) => c.w(w),
        None => c.grow(1.0).w(0.0),
    };
    if right { c.justify_end() } else { c }
}

/// A header label: `text-sm font-medium text-muted-foreground`, cut with a tooltip.
pub fn th_text(tk: &Tk, label: impl Into<String>) -> Node {
    let label = label.into();
    text(label.clone())
        .font(tk.tsm(500.0))
        .color(tk.muted_fg)
        .truncate()
        .min_w(0.0)
        .help_when_truncated(label)
}

/// A compact header label (`text-xs font-medium text-muted-foreground`) over `text-xs` cells.
pub fn th_text_compact(tk: &Tk, label: impl Into<String>) -> Node {
    let label = label.into();
    text(label.clone())
        .font(tk.txs(500.0))
        .color(tk.muted_fg)
        .truncate()
        .min_w(0.0)
        .help_when_truncated(label)
}

/// Which way a sortable column sorts: None when it doesn't.
pub type SortState = Option<bool>;

/// A sortable header label: the label and the sort glyph (`h-7 rounded-md px-1.5
/// hover:bg-muted`), active in the foreground colour. `asc` is Some(true) ascending.
pub fn sort_label(
    tk: &Tk,
    key: impl Into<String>,
    label: &str,
    aria: String,
    asc: SortState,
    right: bool,
) -> Node {
    let color = if asc.is_some() { tk.fg } else { tk.muted_fg };
    let glyph: &'static Icon = match asc {
        None => &crate::icons::ARROW_UP_DOWN,
        Some(true) => &crate::icons::ARROW_UP,
        Some(false) => &crate::icons::ARROW_DOWN,
    };
    let lbl = text(label.to_owned())
        .font(tk.tsm(500.0))
        .color(color)
        .truncate()
        .min_w(0.0)
        .shrink();
    let g = crate::ui::ic(glyph, tk.r(0.875), color);
    let b = hrow()
        .key(key.into())
        .h(tk.r(1.75))
        .px(tk.r(0.375))
        .ml(-tk.r(0.375))
        .gap(tk.r(0.25))
        .center_y()
        .min_w(0.0)
        .shrink()
        .radius(tk.r(0.375))
        .hover(tk.muted)
        .role("button")
        .label(aria.clone())
        .a11y_value(
            match asc {
                None => "none",
                Some(true) => "ascending",
                Some(false) => "descending",
            }
            .to_owned(),
        )
        .help(aria)
        .focusable(true)
        .cursor(rune::Cursor::Pointer);
    if right {
        b.ml(0.0).mr(-tk.r(0.375)).child(lbl).child(g)
    } else {
        b.child(lbl).child(g)
    }
}

/// A body row: a rule under it (none under the last), `hover:bg-muted/35`.
pub fn tr(tk: &Tk, last: bool) -> Node {
    let r = hrow()
        .w_full()
        .center_y()
        .hover(tk.muted.alpha(0.35))
        .role("row");
    if last {
        r
    } else {
        r.border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
    }
}

/// A row that opens something: pointer, the focus fill, and `bg-muted/60` while `selected`.
pub fn interactive(r: Node, tk: &Tk, selected: bool) -> Node {
    let r = r
        .cursor(rune::Cursor::Pointer)
        .focus_highlight(tk.muted.alpha(0.35));
    if selected {
        r.fill(tk.muted.alpha(0.6))
            .hover(tk.muted.alpha(0.6))
            .checked(true)
    } else {
        r
    }
}

/// A body cell `w` wide (None: the flexible column) holding `content`, padded `PX`/`py`.
pub fn td(tk: &Tk, w: Option<f32>, right: bool, py: f32, content: Node) -> Node {
    let c = rune::col()
        .px(tk.r(PX))
        .py(tk.r(py))
        .justify_center()
        .min_w(0.0)
        .child(content);
    let c = match w {
        Some(w) => c.w(w),
        None => c.grow(1.0).w(0.0),
    };
    if right {
        c.items_end()
    } else {
        c.items_start()
    }
}

/// One line of cell text: cut with an ellipsis, the whole text on hover while it is cut.
pub fn cell_text(s: impl Into<String>, style: TextStyle, color: Color) -> Node {
    let s = s.into();
    text(s.clone())
        .font(style)
        .color(color)
        .single_line()
        .truncate()
        .min_w(0.0)
        .max_w(f32::INFINITY)
        .help_when_truncated(s)
}

/// The primary figure of a numeric cell: `text-sm font-medium tabular-nums`.
pub fn num_style(tk: &Tk) -> TextStyle {
    tk.tsm(500.0).features("'tnum' 1")
}

/// The line under a figure (its other currency, a fee): `text-xs text-muted-foreground`.
pub fn sub_style(tk: &Tk) -> TextStyle {
    tk.txs(400.0).features("'tnum' 1")
}

/// A primary figure, blurred while amounts are hidden.
pub fn num(tk: &Tk, s: impl Into<String>, color: Color, hidden: bool) -> Node {
    crate::ui::sensitive(s, num_style(tk), color, hidden).single_line()
}

/// The muted line under a figure.
pub fn num_sub(tk: &Tk, s: impl Into<String>, hidden: bool) -> Node {
    crate::ui::sensitive(s, sub_style(tk), tk.muted_fg, hidden).single_line()
}

/// The muted dash of an empty cell, hidden from assistive technology.
pub fn dash(tk: &Tk) -> Node {
    text("—")
        .font(tk.txs(400.0))
        .color(tk.muted_fg.alpha(0.6))
        .a11y_hidden()
}

/// A 1 rem lucide glyph centred in the leading slot.
pub fn lead_glyph(tk: &Tk, glyph: &'static Icon, color: Color) -> Node {
    hrow()
        .size(tk.r(LEAD), tk.r(LEAD))
        .center()
        .a11y_hidden()
        .child(crate::ui::ic(glyph, tk.r(1.0), color))
}

/// The leading slot holding `content` (a logo already sized to it, a badge).
pub fn lead(tk: &Tk, content: Node) -> Node {
    hrow().size(tk.r(LEAD), tk.r(LEAD)).center().child(content)
}

/// A rail's logo in the leading slot (20 px art in the 24 px slot, `logos::role::RAIL`), for
/// views that build without a `Cx` (virtual rows); `title` is the hover help.
pub fn rail_logo(
    app: &crate::app::App,
    tk: &Tk,
    rail: crate::logos::RailLogo,
    title: &str,
) -> Node {
    use crate::logos::{BITCOIN, LIGHTNING, LIQUID, Logo, RailLogo, fitted_at, role};
    let bytes = match rail {
        RailLogo::Onchain => BITCOIN,
        RailLogo::Lightning => LIGHTNING,
        RailLogo::Liquid => LIQUID,
    };
    let (art, slot) = role::RAIL;
    fitted_at(app, 2.0, Logo::svg(bytes), tk.r(art), tk.r(slot)).help(title.to_owned())
}

/// The rail a network label names ("On-chain", "Lightning", "Liquid", a wallet kind…), if any.
pub fn rail_of(label: &str) -> Option<crate::logos::RailLogo> {
    use crate::logos::RailLogo;
    let l = label.to_lowercase();
    if l.contains("lightning") || l.contains("phoenix") {
        Some(RailLogo::Lightning)
    } else if l.contains("liquid") || l.contains("lbtc") || l.contains("l-btc") {
        Some(RailLogo::Liquid)
    } else if l.contains("on-chain") || l.contains("onchain") || l.contains("bitcoin") || l == "btc"
    {
        Some(RailLogo::Onchain)
    } else {
        None
    }
}

/// A status tone: what a chip or direction tile is tinted with.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Needs a look (amber).
    Warning,
    /// Failed or blocking (red).
    Danger,
    /// Waiting on something (sky).
    Info,
    /// Plain (zinc).
    Neutral,
}

/// (fill, ink, ring) of a tone, `bg-{c}-50 text-{c}-700 ring-{c}-600/20` and dark
/// `bg-{c}-900/30 text-{c}-300 ring-{c}-400/20`.
pub fn tone(tk: &Tk, t: Tone) -> (Color, Color, Color) {
    let f = match t {
        Tone::Warning => "amber",
        Tone::Danger => "red",
        Tone::Info => "sky",
        Tone::Neutral => "zinc",
    };
    if tk.dark {
        let bg = if t == Tone::Neutral {
            tw(f, 800).alpha(0.7)
        } else {
            tw(f, 900).alpha(0.3)
        };
        (
            bg,
            tw(f, 300),
            tw(
                if t == Tone::Neutral { "zinc" } else { f },
                if t == Tone::Neutral { 500 } else { 400 },
            )
            .alpha(0.2),
        )
    } else {
        (
            tw(f, 50),
            tw(f, 700),
            tw(f, if t == Tone::Neutral { 500 } else { 600 }).alpha(0.2),
        )
    }
}

/// The status label: the tone's ink and an optional 0.75 rem glyph, no fill or ring (as
/// `crate::ui::tag`). Draw it only for a state that needs attention.
pub fn chip(tk: &Tk, t: Tone, glyph: Option<&'static Icon>, label: impl Into<String>) -> Node {
    let (_, fg, _) = tone(tk, t);
    let fg = if t == Tone::Neutral { tk.muted_fg } else { fg };
    let label = label.into();
    let mut r = hrow().gap(tk.r(0.25)).center_y().min_w(0.0);
    if let Some(g) = glyph {
        r = r.child(crate::ui::ic(g, tk.r(0.75), fg));
    }
    r.child(
        text(label.clone())
            .font(tk.txs(500.0))
            .color(fg)
            .single_line()
            .truncate()
            .min_w(0.0)
            .help_when_truncated(label),
    )
}

/// The direction tile in the leading slot: the glyph on a tinted `rounded-md` tile.
pub fn lead_tile(tk: &Tk, glyph: &'static Icon, t: Tone) -> Node {
    let (bg, fg, ring) = tone(tk, t);
    hrow()
        .size(tk.r(LEAD), tk.r(LEAD))
        .center()
        .radius(tk.r(0.375))
        .fill(bg)
        .border(1.0, ring)
        .border_layout()
        .a11y_hidden()
        .child(crate::ui::ic(glyph, tk.r(0.875), fg))
}

/// The Tailwind v4 shades a tone uses (oklch, as the web's utilities resolve).
fn tw(family: &str, shade: u16) -> Color {
    let (l, c, h) = match (family, shade) {
        ("amber", 50) => (0.987, 0.022, 95.277),
        ("amber", 300) => (0.879, 0.169, 91.605),
        ("amber", 400) => (0.828, 0.189, 84.429),
        ("amber", 600) => (0.666, 0.179, 58.318),
        ("amber", 700) => (0.555, 0.163, 48.998),
        ("amber", 900) => (0.414, 0.112, 45.904),
        ("red", 50) => (0.971, 0.013, 17.38),
        ("red", 300) => (0.808, 0.114, 19.571),
        ("red", 400) => (0.704, 0.191, 22.216),
        ("red", 600) => (0.577, 0.245, 27.325),
        ("red", 700) => (0.505, 0.213, 27.518),
        ("red", 900) => (0.396, 0.141, 25.723),
        ("sky", 50) => (0.977, 0.013, 236.62),
        ("sky", 300) => (0.828, 0.111, 230.318),
        ("sky", 400) => (0.746, 0.16, 232.661),
        ("sky", 600) => (0.588, 0.158, 241.966),
        ("sky", 700) => (0.5, 0.134, 242.749),
        ("sky", 900) => (0.391, 0.09, 240.876),
        ("emerald", 50) => (0.979, 0.021, 166.113),
        ("emerald", 300) => (0.845, 0.143, 164.978),
        ("emerald", 400) => (0.765, 0.177, 163.223),
        ("emerald", 600) => (0.596, 0.145, 163.225),
        ("emerald", 700) => (0.508, 0.118, 165.612),
        ("emerald", 900) => (0.378, 0.077, 168.94),
        ("zinc", 50) => (0.985, 0.0, 0.0),
        ("zinc", 300) => (0.871, 0.006, 286.286),
        ("zinc", 500) => (0.552, 0.016, 285.938),
        ("zinc", 700) => (0.37, 0.013, 285.805),
        ("zinc", 800) => (0.274, 0.006, 286.033),
        _ => (0.5, 0.0, 0.0),
    };
    rune::oklch(l, c, h)
}

/// An empty, loading or error body: one centred muted line in a 6 rem band.
pub fn empty(tk: &Tk, content: Node) -> Node {
    hrow()
        .w_full()
        .min_h(tk.r(6.0))
        .px(tk.r(EDGE))
        .py(tk.r(0.5))
        .center()
        .child(content)
}

/// [`empty`] with a plain message.
pub fn empty_text(tk: &Tk, msg: impl Into<String>) -> Node {
    empty(
        tk,
        text(msg.into())
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap()
            .align(rune::Align::Center),
    )
}

#[cfg(test)]
mod tests {
    use super::fit;

    #[test]
    fn fit_fills_the_card_until_the_columns_need_more() {
        assert_eq!(fit(900.0, 640.0), (900.0, false));
        assert_eq!(fit(600.0, 640.0), (640.0, true));
        // Sub-point rounding of measured widths is not an overflow.
        assert_eq!(fit(640.0, 640.3), (640.3, false));
    }
}
