//! Edit mode, the home screen's "jiggle": tiles wiggle, a drag reorders them (the others slide
//! out of the way), each carries a remove badge and a size control, and the page header offers
//! Add widget, Reset and Done. It starts from a long press on a tile, the context menu or the
//! header's Edit button, and ends with Done or Escape. With the keyboard: arrows move the
//! focused tile, S changes its size, Delete removes it; each change is announced.

use rune::{Color, Drag, El, Key, KeyEvent, LiveRegion, Phase, Shadow, Tx, row, text};

use super::model::{Layout, Size, WidgetId, pack};
use super::{copy, gallery, title};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// Seconds a press rests on a tile before edit mode starts.
const HOLD: f64 = 0.5;
/// Seconds between the wiggle's turns.
pub const WIGGLE: f64 = 0.14;
/// Seconds a reorder settles before the drag reorders again.
const SETTLE: f64 = 0.18;

#[derive(Default)]
pub struct Edit {
    pub on: bool,
    pub drag: Option<DragState>,
    /// The press in progress on a tile.
    pub hold: Option<Hold>,
    pub gallery: Option<gallery::Gallery>,
    moved_at: f64,
    /// The wiggle's phase.
    pub flip: bool,
    flip_at: f64,
}

pub struct DragState {
    pub id: WidgetId,
    /// The order when the drag started (a cancelled drag restores it).
    start: Layout,
    /// The tile's top left in the grid when picked up, and the pointer's travel since.
    pub home: (f32, f32),
    pub off: (f32, f32),
    /// The tile it last traded places with, until its centre is back over its own slot.
    over: Option<WidgetId>,
}

pub struct Hold {
    at: f64,
    fired: bool,
}

impl Edit {
    /// A scene's drag in progress: `id` lifted by (`dx`, `dy`) from its place in `geo`.
    pub fn scene_drag(&mut self, layout: &Layout, geo: super::Geo, id: WidgetId, dx: f32, dy: f32) {
        self.on = true;
        if let Some(c) = pack(layout.tiles(), geo.cols).iter().find(|c| c.id == id) {
            self.drag = Some(DragState {
                id,
                start: layout.clone(),
                home: geo.origin(c.x, c.y),
                off: (dx, dy),
                over: None,
            });
        }
    }
}

pub fn enter(a: &mut App, tx: &mut Tx) {
    if !a.ov.edit.on {
        a.ov.edit.on = true;
        tx.animated();
        tx.announce(copy::editing(), LiveRegion::Polite);
    }
}

pub fn exit(a: &mut App, tx: &mut Tx) {
    let e = &mut a.ov.edit;
    if let Some(d) = e.drag.take() {
        a.ov.layout = d.start;
    }
    let e = &mut a.ov.edit;
    e.hold = None;
    if e.on {
        e.on = false;
        tx.animated();
        tx.announce(copy::edit_done(), LiveRegion::Polite);
    }
}

/// The tile's press: held still it starts edit mode; in edit mode it drags the tile through
/// the order.
pub fn drag(id: WidgetId) -> impl Fn(&mut App, &mut Drag, &mut Tx) + 'static {
    move |a: &mut App, d: &mut Drag, tx: &mut Tx| {
        let now = tx.clock();
        match d.phase {
            Phase::Down => {
                a.ov.edit.hold = Some(Hold {
                    at: now,
                    fired: a.ov.edit.on,
                });
                if !a.ov.edit.on {
                    tx.wake_at(now + HOLD);
                }
            }
            Phase::Move => {
                let travel = d.dx.hypot(d.dy);
                if !a.ov.edit.on {
                    // A moving press is not a long press.
                    if travel > 4.0 {
                        a.ov.edit.hold = None;
                    }
                    return;
                }
                let Some(geo) = a.ov.geo.get() else { return };
                if a.ov.edit.drag.is_none() {
                    if travel < 3.0 {
                        return;
                    }
                    a.ov.layout.settle(geo.cols);
                    let Some(c) = pack(a.ov.layout.tiles(), geo.cols)
                        .into_iter()
                        .find(|c| c.id == id)
                    else {
                        return;
                    };
                    a.ov.edit.drag = Some(DragState {
                        id,
                        start: a.ov.layout.clone(),
                        home: geo.origin(c.x, c.y),
                        off: (0.0, 0.0),
                        over: None,
                    });
                }
                let Some(c) = pack(a.ov.layout.tiles(), geo.cols)
                    .into_iter()
                    .find(|c| c.id == id)
                else {
                    return;
                };
                let (w, h) = geo.span_at(c.y, c.w, c.h);
                let (sx, sy) = geo.origin(c.x, c.y);
                let settled = now - a.ov.edit.moved_at > SETTLE;
                let Some(st) = a.ov.edit.drag.as_mut() else {
                    return;
                };
                st.off = (d.dx, d.dy);
                let (cx, cy) = (
                    st.home.0 + st.off.0 + w / 2.0,
                    st.home.1 + st.off.1 + h / 2.0,
                );
                if cx >= sx && cx < sx + w && cy >= sy && cy < sy + h {
                    st.over = None;
                    return;
                }
                if let Some((over, index)) = super::drop_target(&a.ov.layout, geo, id, cx, cy)
                    && st.over != Some(over)
                    && settled
                {
                    st.over = Some(over);
                    if a.ov.layout.move_to(id, index) {
                        a.ov.edit.moved_at = now;
                        tx.animated();
                    }
                }
            }
            Phase::Up => {
                a.ov.edit.hold = None;
                if let Some(st) = a.ov.edit.drag.take() {
                    if d.cancelled() {
                        a.ov.layout = st.start;
                    } else if a.ov.layout != st.start
                        && let Some(i) = a.ov.layout.index_of(id)
                    {
                        tx.announce(
                            copy::moved(&title(a, id), i + 1, a.ov.layout.len()),
                            LiveRegion::Polite,
                        );
                    }
                    tx.animated();
                }
            }
        }
    }
}

/// The focused tile's keys in edit mode.
pub fn key(id: WidgetId) -> impl Fn(&mut App, &KeyEvent, &mut Tx) -> bool + 'static {
    move |a: &mut App, ev: &KeyEvent, tx: &mut Tx| {
        let name = title(a, id);
        let by = match &ev.key {
            Key::Left | Key::Up => -1,
            Key::Right | Key::Down => 1,
            Key::Text(s) if s.eq_ignore_ascii_case("s") => {
                cycle(a, id, tx);
                return true;
            }
            Key::Delete | Key::Backspace => {
                remove(a, id, tx);
                return true;
            }
            _ => return false,
        };
        if let Some(geo) = a.ov.geo.get() {
            a.ov.layout.settle(geo.cols);
        }
        if a.ov.layout.shift(id, by) {
            let i = a.ov.layout.index_of(id).unwrap_or(0);
            tx.animated();
            tx.announce(
                copy::moved(&name, i + 1, a.ov.layout.len()),
                LiveRegion::Polite,
            );
        } else {
            tx.announce(copy::edge(&name), LiveRegion::Polite);
        }
        true
    }
}

fn cycle(a: &mut App, id: WidgetId, tx: &mut Tx) {
    let name = title(a, id);
    match a.ov.layout.cycle_size(id) {
        Some(z) => {
            tx.animated();
            tx.announce(copy::sized(&name, z), LiveRegion::Polite);
        }
        None => tx.announce(copy::one_size(&name), LiveRegion::Polite),
    }
}

fn remove(a: &mut App, id: WidgetId, tx: &mut Tx) {
    let name = title(a, id);
    let at = a.ov.layout.index_of(id);
    if a.ov.layout.remove(id) {
        tx.animated();
        tx.announce(copy::removed(&name), LiveRegion::Polite);
        // Focus goes to the tile that took its place (or the one before, or Add widget).
        let next = at
            .and_then(|i| {
                a.ov.layout
                    .tiles()
                    .get(i)
                    .or_else(|| a.ov.layout.tiles().last())
            })
            .map(|t| format!("ov-tile-{}", t.id.id()));
        tx.focus(next.unwrap_or_else(|| "ov-add".to_owned()), true);
    }
}

/// The remove badge on a tile's top left corner.
pub fn remove_badge(tk: &Tk, id: WidgetId, name: &str) -> El {
    let d = tk.r(1.625);
    let fill = if tk.dark {
        Color::WHITE.mix(tk.card, 0.22)
    } else {
        tk.fg.mix(tk.card, 0.18)
    };
    row()
        .key(format!("ov-remove-{}", id.id()))
        .absolute(-tk.r(0.4), -tk.r(0.4))
        .size(d, d)
        .center()
        .radius(d / 2.0)
        .fill(fill)
        .hover(fill.mix(tk.alert.solid, 0.55))
        .box_shadow(Shadow {
            y: 1.0,
            blur: 4.0,
            color: Color::BLACK.alpha(0.25),
            ..Shadow::default()
        })
        .role("button")
        .label(copy::remove_named(name))
        .focusable(true)
        .focus_ring(2.0, 1.0, tk.ring)
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |a: &mut App, tx: &mut Tx| remove(a, id, tx))
        .child(ic(
            &I::MINUS,
            tk.r(1.0),
            if tk.dark { tk.background } else { tk.card },
        ))
}

/// The size control on a tile's bottom right corner: its size, a press takes the next one.
pub fn size_pill(tk: &Tk, id: WidgetId, name: &str, size: Size, w: f32, h: f32) -> El {
    let ph = tk.r(1.5);
    let label = copy::size(size);
    row()
        .key(format!("ov-size-{}", id.id()))
        .pin(None, None, Some(tk.r(0.625)), Some(tk.r(0.625)))
        .max_w((w - tk.r(1.25)).max(0.0))
        .h(ph.min(h))
        .px(tk.r(0.5))
        .gap(tk.r(0.3))
        .center_y()
        .radius(ph / 2.0)
        .fill(tk.fg.mix(tk.card, 0.86))
        .hover(tk.fg.mix(tk.card, 0.78))
        .role("button")
        .label(copy::size_of(name, size))
        .focusable(true)
        .focus_ring(2.0, 1.0, tk.ring)
        .cursor(rune::Cursor::Pointer)
        .on_press_tx(move |a: &mut App, tx: &mut Tx| cycle(a, id, tx))
        .child(ic(&I::MAXIMIZE_2, tk.r(0.75), tk.fg))
        .child(text(label).font(tk.t2xs(600.0)).color(tk.fg))
}

/// The page header's buttons: Add connection and Edit; while editing, Add widget, Reset and Done.
pub fn header(app: &App, tk: &Tk, narrow: bool, add_connection: Option<El>) -> El {
    let r = row().gap(tk.r(0.5)).center_y();
    if !app.ov.edit.on {
        let edit =
            crate::ui::outline_button(tk, "overview-edit", None, &copy::edit()).on_press_tx(enter);
        return r.child(edit).children(add_connection);
    }
    let add = if narrow {
        crate::ui::icon_button(tk, "ov-add", &I::PLUS, &copy::add_widget())
    } else {
        crate::ui::outline_button(tk, "ov-add", Some(&I::PLUS), &copy::add_widget())
    };
    r.child(add.on_press(gallery::open))
        .child(
            crate::ui::icon_button(tk, "ov-reset", &I::ROTATE_CCW, &copy::reset()).on_press_tx(
                |a: &mut App, tx: &mut Tx| {
                    if !a.ov.layout.is_default_arrangement() {
                        a.ov.layout.reset();
                        tx.animated();
                    }
                    tx.announce(copy::reset_done(), LiveRegion::Polite);
                },
            ),
        )
        .child(
            crate::ui::primary_button(tk, "overview-done", None, &t("common:actions.done"))
                .on_press_tx(exit),
        )
}

/// The long press and the wiggle run off the clock. Leaving the Overview ends edit mode.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let on_route = matches!(app.route.path(), "/overview" | "/");
    let now = tx.clock();
    let reduced = app.ov.reduced.get();
    let e = &mut app.ov.edit;
    if !on_route {
        let was = e.on || e.hold.is_some() || e.drag.is_some() || e.gallery.is_some();
        e.on = false;
        e.hold = None;
        e.gallery = None;
        if let Some(d) = e.drag.take() {
            app.ov.layout = d.start;
        }
        return was;
    }
    let mut changed = false;
    if let Some(h) = e.hold.as_mut().filter(|h| !h.fired) {
        if now - h.at >= HOLD {
            h.fired = true;
            e.on = true;
            tx.animated();
            tx.announce(copy::editing(), LiveRegion::Polite);
            changed = true;
        } else {
            tx.wake_at(h.at + HOLD);
        }
    }
    if e.on && !reduced {
        if now >= e.flip_at {
            e.flip = !e.flip;
            e.flip_at = now + WIGGLE;
            tx.animated();
            changed = true;
        }
        tx.wake_at(e.flip_at);
    }
    changed
}
