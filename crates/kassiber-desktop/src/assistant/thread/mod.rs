//! The conversation (ChatThread.tsx, ChatMessage.tsx and the message parts): every message,
//! its reasoning, tool calls, approvals, plans, citations and errors.
//!
//! Scrolling (chatThreadScroll.ts): the thread is a scroll viewport that opens pinned to its
//! newest message and follows the end while it is there (`stick_bottom`). Any scroll back up
//! leaves it free: a wheel up marks the thread free (the web's `wheel` opt-out) and shows the
//! "Scroll to latest" pill until the thread is back at its end, where following resumes. A
//! scroll box inside a message (a JSON body, a plan, a table) passes a wheel it has no room for
//! on to the thread, as a browser chains scrolling; Rune scrolls only the innermost viewport, so
//! without this a reply's tool output under the pointer swallowed the wheel. A just-sent turn
//! anchors: its question sits 16 pt under the thread's top with blank room reserved below the
//! reply (`anchoredEndSpace`), the room shrinking as the reply grows, until the turn fills the
//! view and the thread follows its end again; a scroll back up drops the room.

pub mod action;
pub mod approval;
pub mod decision;
pub mod md;
pub mod message;
pub mod tool;
pub mod visual;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text};

use crate::app::{App, Reply};
use crate::assistant::model::Role;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

/// Per-thread UI state (expanded reasoning and tool cards, copy feedback, inline edits).
#[derive(Default)]
pub struct State {
    /// Collapsibles the user toggled away from their default (key → open).
    pub toggles: HashMap<String, bool>,
    /// The control whose "Copied" feedback shows, until this app-clock time.
    pub copied: Option<(String, f64)>,
    /// The user message being edited inline (id, text).
    pub editing: Option<(String, TextEdit)>,
    /// Visual controls (measure, group, year …) the user picked, by key.
    pub choices: HashMap<String, String>,
    /// The visual open in its "Open larger" dialog (`asst-visual-<call id>`).
    pub expanded_visual: Option<String>,
    /// The thread scrolled far from its end: [page, panel].
    pub far: [bool; 2],
    /// The user scrolled the thread back (`mode === "free"`): [page, panel].
    pub free: [bool; 2],
    /// The thread does not show its end: [page, panel].
    pub off_bottom: [bool; 2],
    /// The thread column's width as laid out: [page, panel].
    pub width: [f32; 2],
    /// The approval row whose More menu is open (its call id).
    pub consent_menu: Option<String>,
    /// A turn was just sent: scroll both threads to their end on the next poll.
    pub anchor_new_turn: bool,
    /// The question of the turn being anchored near the thread's top (`anchorIdRef`).
    pub anchor: Option<String>,
    /// As laid out, in window points: the anchored question's top, the content's bottom and the
    /// thread's height, [page, panel].
    pub anchor_top: [f32; 2],
    pub content_bottom: [f32; 2],
    pub view_h: [f32; 2],
    /// Each visual chart's laid-out width (its key), for the axis ticks that fit.
    pub chart_w: HashMap<String, f32>,
    /// The visual chart under the pointer and its hovered point (chart key, index): the
    /// recharts tooltip's state.
    pub chart_hover: Option<(String, usize)>,
    /// Reasoning panes seen being written (`wasStreamingRef`): key → (start, whole seconds so
    /// far), for "Thought for 3s".
    pub thought: HashMap<String, (f64, u64)>,
    /// The native change cards' transaction reads and undos.
    pub actions: action::State,
}

/// The thread column's laid-out width; before its first layout, the page's or panel's usual one.
pub(crate) fn width(app: &App, page: bool) -> f32 {
    let w = app.assistant.thread.width[if page { 0 } else { 1 }];
    if w > 0.0 {
        w
    } else if page {
        728.0
    } else {
        360.0
    }
}

pub(crate) fn is_open(app: &App, key: &str, default: bool) -> bool {
    app.assistant
        .thread
        .toggles
        .get(key)
        .copied()
        .unwrap_or(default)
}

pub(crate) fn toggle(app: &mut App, key: &str, default: bool) {
    let next = !is_open(app, key, default);
    app.assistant.thread.toggles.insert(key.to_owned(), next);
}

/// "Copied" for 1.5 s, then back (the wake re-renders it).
pub(crate) fn mark_copied(app: &mut App, tx: &mut Tx, key: &str) {
    app.assistant.thread.copied = Some((key.to_owned(), app.clock + 1.5));
    tx.wake_after(std::time::Duration::from_millis(1600));
}

/// A thread's key and index, and where it is scrolled to.
type Host = (&'static str, usize, Rc<Cell<[f32; 2]>>);

thread_local! {
    /// The thread being built, for the scroll boxes inside it (`nested_scroll`).
    static HOST: RefCell<Option<Host>> = const { RefCell::new(None) };
}

/// The jump-to-latest pill shows (`resolveScrollAffordances`): the thread is free and not at
/// its end, or it is far from its end however it got there (a scroll-bar drag, Page Up).
/// Where the anchored question sits below the thread's top (T3Code's 16).
const ANCHOR_OFFSET: f32 = 16.0;

/// The blank room reserved under an anchored turn so its question can sit `ANCHOR_OFFSET`
/// under the top: none once the turn fills the view (then the thread just follows its end).
pub(crate) fn anchor_room(view_h: f32, anchor_top: f32, content_bottom: f32) -> Option<f32> {
    if view_h <= 0.0 || content_bottom < anchor_top {
        return None;
    }
    let usable = view_h - ANCHOR_OFFSET;
    let turn = content_bottom - anchor_top;
    (turn < usable).then_some(usable - turn)
}

pub(crate) fn shows_jump(far: bool, free: bool, off_bottom: bool) -> bool {
    far || (free && off_bottom)
}

/// Where a wheel delta `dy` (positive moves content down, toward the top) goes from a scroll box
/// at `offset` of `max`: `None` while the box has room that way (it scrolls itself), else the
/// thread's new offset from `host` (its offset and max).
pub(crate) fn chained_offset(offset: f32, max: f32, dy: f32, host: [f32; 2]) -> Option<f32> {
    if max.is_nan() || host[1].is_nan() || dy == 0.0 {
        return None;
    }
    let room = if dy > 0.0 {
        offset > 0.5
    } else {
        offset < max - 0.5
    };
    (!room).then(|| (host[0] - dy).clamp(0.0, host[1].max(0.0)))
}

/// A vertical scroll box inside the conversation: a wheel it has no room for scrolls the thread.
pub(crate) fn nested_scroll(el: El) -> El {
    let el = el.scroll(rune::ScrollSpec {
        stick_bottom: false,
    });
    let Some((key, idx, host)) = HOST.with(|h| h.borrow().clone()) else {
        return el;
    };
    let own = Rc::new(Cell::new([f32::NAN; 2]));
    let seen = own.clone();
    el.on_scroll_offset(move |p: rune::ScrollPos| seen.set([p.offset[1], p.max[1]]))
        .on_wheel(move |s: &mut App, ev: rune::WheelEvent, tx: &mut Tx| {
            let [offset, max] = own.get();
            let Some(to) = chained_offset(offset, max, ev.dy, host.get()) else {
                return false;
            };
            host.set([to, host.get()[1]]);
            if ev.dy > 0.0 {
                s.assistant.thread.free[idx] = true;
                s.assistant.thread.anchor = None;
            }
            tx.scroll_to_instant(key, 0.0, to);
            true
        })
}

/// Opens a transaction a result points at (`openLocalReference`): the Transactions page with it.
pub(crate) fn open_transaction(app: &mut App, id: &str) {
    app.navigate("/transactions");
    crate::txdetail::open(app, id, "details");
}

/// The thread for `app.assistant.messages`. `page`: the full page's centered `max-w-3xl px-5`
/// column; otherwise the panel's `px-3 py-3`.
pub fn view(app: &App, _cx: &mut Cx<App>, tk: &Tk, page: bool, layer: &mut Vec<El>) -> El {
    let idx = if page { 0 } else { 1 };
    let key = if page {
        "asst-thread-page"
    } else {
        "asst-thread-panel"
    };
    let host = Rc::new(Cell::new([f32::NAN; 2]));
    HOST.with(|h| *h.borrow_mut() = Some((key, idx, host.clone())));
    let mut content = col()
        .w_full()
        .gap(tk.r(1.25))
        .on_layout(move |s: &mut App, r: [f32; 4]| s.assistant.thread.width[idx] = r[2]);
    content = if page {
        content.max_w(tk.r(48.0)).px(tk.r(1.25))
    } else {
        content.px(tk.r(0.75)).py(tk.r(0.75))
    };
    let th = &app.assistant.thread;
    for m in &app.assistant.messages {
        content = content.child(match m.role {
            Role::User if th.anchor.as_deref() == Some(m.id.as_str()) => {
                message::user(app, tk, m, page).on_layout(move |s: &mut App, r: [f32; 4]| {
                    s.assistant.thread.anchor_top[idx] = r[1]
                })
            }
            Role::User => message::user(app, tk, m, page),
            _ => message::assistant(app, tk, m, page, layer),
        });
    }
    let room = th
        .anchor
        .as_ref()
        .and_then(|_| anchor_room(th.view_h[idx], th.anchor_top[idx], th.content_bottom[idx]));
    if th.anchor.is_some() {
        content = content.on_layout(move |s: &mut App, r: [f32; 4]| {
            s.assistant.thread.width[idx] = r[2];
            s.assistant.thread.content_bottom[idx] = r[1] + r[3];
        });
    }
    HOST.with(|h| *h.borrow_mut() = None);
    let scroll = col()
        .key(key)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("log")
        .label(t("assistant:dock.conversation"))
        .scroll(rune::ScrollSpec { stick_bottom: true })
        .on_scroll_offset(move |p: rune::ScrollPos| host.set([p.offset[1], p.max[1]]))
        .on_far_from_bottom(120.0, move |s: &mut App, far: bool| {
            s.assistant.thread.far[idx] = far
        })
        .on_at_bottom(move |s: &mut App, at: bool| {
            s.assistant.thread.off_bottom[idx] = !at;
            // Back at the end: following resumes.
            if at {
                s.assistant.thread.free[idx] = false;
            }
        })
        .child(
            row()
                .w_full()
                .justify_center()
                // A wheel back up opts out of following before the scroll lands (it still scrolls).
                .on_wheel(move |s: &mut App, ev: rune::WheelEvent, _tx: &mut Tx| {
                    if ev.dy > 0.0 && !s.assistant.thread.free[idx] {
                        s.assistant.thread.free[idx] = true;
                        // Free scrolling drops the anchored turn's room.
                        s.assistant.thread.anchor = None;
                    }
                    false
                })
                .child(content),
        )
        .children(room.map(|h| {
            rune::el()
                .key(format!("{key}-room"))
                .w_full()
                .h(h)
                .a11y_hidden()
        }));
    let frame = col()
        .key(format!("{key}-frame"))
        .w_full()
        .h_full()
        .min_h(0.0)
        .on_layout(move |s: &mut App, r: [f32; 4]| s.assistant.thread.view_h[idx] = r[3])
        .child(scroll);
    if shows_jump(th.far[idx], th.free[idx], th.off_bottom[idx]) {
        let label = t("assistant:thread.scrollToLatest");
        layer.push(
            row()
                .key(format!("{key}-latest"))
                .anchored(
                    format!("{key}-frame"),
                    rune::Side::Bottom,
                    rune::Align::Center,
                    -tk.r(2.75),
                )
                .gap(tk.r(0.375))
                .center_y()
                .radius(999.0)
                .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                .border_layout()
                .fill(tk.card)
                .px(tk.r(0.75))
                .py(tk.r(0.25))
                .box_shadow(rune::Shadow {
                    y: 1.0,
                    blur: 2.0,
                    color: rune::Color::BLACK.alpha(0.05),
                    ..Default::default()
                })
                .role("button")
                .label(label.clone())
                .help(label)
                .focusable(true)
                .on_press_tx(move |s: &mut App, tx: &mut Tx| {
                    s.assistant.thread.far[idx] = false;
                    s.assistant.thread.free[idx] = false;
                    tx.scroll_to_bottom(key);
                })
                .child(ic(&I::CHEVRON_DOWN, tk.r(0.875), tk.muted_fg).a11y_hidden())
                .child(
                    text(t("assistant:thread.scrollToLatestShort"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if let Some(open) = &app.assistant.thread.expanded_visual {
        for m in &app.assistant.messages {
            for v in m.tool_calls.iter().filter_map(visual::parse) {
                if &format!("asst-visual-{}", v.call_id) == open {
                    visual::expanded_dialog(app, tk, &v, layer);
                }
            }
        }
    }
    frame
}

/// `ChatReasoning`'s timer: the pane being written counts whole seconds from when it started;
/// once it stops (a later round, the answer, the end) its count stays.
pub fn poll_thought(app: &mut App, tx: &mut Tx) -> bool {
    if app.assistant.messages.is_empty() {
        let had = !app.assistant.thread.thought.is_empty();
        app.assistant.thread.thought.clear();
        return had;
    }
    let Some(key) = app
        .assistant
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .and_then(message::active_thinking)
    else {
        return false;
    };
    let now = app.clock;
    let (start, shown) = *app
        .assistant
        .thread
        .thought
        .entry(key.clone())
        .or_insert((now, 0));
    let secs = (now - start).max(0.0).floor() as u64;
    tx.wake_after(std::time::Duration::from_secs_f64(
        (start + secs as f64 + 1.0 - now).max(0.05),
    ));
    if secs != shown {
        app.assistant.thread.thought.insert(key, (start, secs));
        return true;
    }
    false
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    action::on_reply(app, tag, r);
}

/// Escape closes the visual dialog, then an inline edit.
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if !matches!(ev.key, Key::Escape) {
        return false;
    }
    if app.assistant.thread.consent_menu.take().is_some() {
        return true;
    }
    if app.assistant.thread.expanded_visual.take().is_some() {
        return true;
    }
    app.assistant.thread.editing.take().is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_with_room_scrolls_itself() {
        assert_eq!(chained_offset(50.0, 100.0, 20.0, [400.0, 900.0]), None);
        assert_eq!(chained_offset(50.0, 100.0, -20.0, [400.0, 900.0]), None);
    }

    #[test]
    fn a_box_at_its_edge_or_without_room_scrolls_the_thread() {
        // At its top, a wheel up moves the thread up.
        assert_eq!(
            chained_offset(0.0, 100.0, 300.0, [900.0, 900.0]),
            Some(600.0)
        );
        // At its end, a wheel down moves the thread down, clamped to its end.
        assert_eq!(
            chained_offset(100.0, 100.0, -300.0, [800.0, 900.0]),
            Some(900.0)
        );
        // Nothing to scroll in the box.
        assert_eq!(chained_offset(0.0, 0.0, 300.0, [200.0, 900.0]), Some(0.0));
        // Not laid out yet: the box keeps the wheel.
        assert_eq!(
            chained_offset(f32::NAN, f32::NAN, 300.0, [200.0, 900.0]),
            None
        );
    }

    #[test]
    fn an_anchored_turn_keeps_room_until_it_fills_the_view() {
        // A 600 pt thread, the question at 100, the reply ending at 300: 600 - 16 - 200 room.
        assert_eq!(anchor_room(600.0, 100.0, 300.0), Some(384.0));
        // The turn outgrew the view: no room, the thread follows its end.
        assert_eq!(anchor_room(600.0, 0.0, 700.0), None);
        // Not laid out yet.
        assert_eq!(anchor_room(0.0, 0.0, 10.0), None);
    }

    #[test]
    fn the_pill_shows_once_free_and_off_the_end() {
        assert!(
            !shows_jump(false, false, true),
            "following, not yet at the end"
        );
        assert!(shows_jump(false, true, true));
        assert!(!shows_jump(false, true, false), "back at the end");
        assert!(shows_jump(true, false, true), "far however it got there");
    }
}
