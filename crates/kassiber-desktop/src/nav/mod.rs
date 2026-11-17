//! Browser-style history input. The owner asked for it ("mouse forwards (same with touch
//! gestures on trackpads) should go back and forward like in browsers"); the Tauri app has no
//! counterpart, since WKWebView ignores the extra mouse buttons and the swipe.
//!
//! - The mouse's Back and Forward buttons (4 and 5) run [`App::go_back`] / [`App::go_forward`],
//!   as the title bar's arrows, ⌥←/⌥→ and ⌘[/⌘] do, anywhere in the window ([`history_buttons`]
//!   on the shell's root). An open dialog blocks them: it is modal, so the root behind it takes
//!   no input. A menu with a scrim (most menus) takes the press on its scrim and keeps it,
//!   which matches a native macOS menu. A non-modal popover (books, notifications, alpha,
//!   privacy) closes and the page goes back, as a press outside closes it.
//! - A two-finger horizontal trackpad swipe over the page does the same ("Swipe between pages"),
//!   unless something under the pointer scrolls or pans sideways ([`swipe_area`] on the page's
//!   scroll view, and [`swipe::Swipe`] for the decision). While the fingers move, a round arrow
//!   slides in from the page's edge and fills in once a release would navigate ([`affordance`]).
//!   macOS three-finger swipes aren't reported to the app (Rune's winit has no
//!   `swipeWithEvent:`), so only the two-finger setting works.

pub mod swipe;

use rune::{El, GesturePhase, PointerButton, Tx, WheelEvent, row};

use crate::app::App;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use swipe::{Dir, Phase, Swipe};

#[derive(Default)]
pub struct NavUi {
    pub swipe: Swipe,
}

fn go(app: &mut App, dir: Dir) {
    match dir {
        Dir::Back => app.go_back(),
        Dir::Forward => app.go_forward(),
    }
}

fn can_go(app: &App, dir: Dir) -> bool {
    match dir {
        Dir::Back => !app.back.is_empty(),
        Dir::Forward => !app.forward.is_empty(),
    }
}

/// The shell's root hears the mouse's Back and Forward buttons wherever nothing inside takes
/// them (nothing does). Rune hands a non-primary button only to an element with a press handler
/// that opted in to that button, so the root gets a background press: it stays out of the Tab
/// order and shows no pointer cursor (the shell names it or leaves it out of the accessibility
/// tree). A primary press on bare chrome now lands
/// on it too: `press_focuses_below` keeps such a press doing to focus what a press on nothing
/// did (it blurs, and focuses nothing). It closes a non-modal popover there (`close_popover`),
/// as the root's background press did before.
pub fn history_buttons(root: El, close_popover: bool) -> El {
    root.pointer_buttons([PointerButton::Back, PointerButton::Forward])
        .press_focuses_below()
        .on_background_press_tx(move |s: &mut App, tx: &mut Tx| {
            // Locked, the page behind the lock screen stays put (as ⌥← does).
            if s.locked {
                return;
            }
            let dir = match tx.button() {
                PointerButton::Back => Dir::Back,
                PointerButton::Forward => Dir::Forward,
                _ => {
                    if close_popover {
                        s.shell.popup = None;
                    }
                    return;
                }
            };
            // A press outside an open popover closes it; then the page moves.
            s.shell.close_popups();
            go(s, dir);
        })
}

fn input(e: &WheelEvent) -> swipe::Input {
    swipe::Input {
        dx: e.dx,
        dy: e.dy,
        phase: match e.phase {
            GesturePhase::Began => Phase::Began,
            GesturePhase::Ended => Phase::Ended,
            _ => Phase::Moved,
        },
        momentum: e.momentum,
        mods: e.mods.shift || e.mods.cmd || e.mods.ctrl || e.mods.alt,
    }
}

/// A scroll view whose horizontal swipes go back and forward. Rune offers a wheel delta to the
/// handlers under the pointer up to the nearest scroll view, so a table that scrolls sideways,
/// the chart (which pans) or a graph canvas keeps its own swipes. The handler claims only a
/// gesture whose first movement is clearly horizontal, and only on a trackpad (phases reported).
/// It then holds that gesture, so the page doesn't drift vertically under a swipe.
pub fn swipe_area(el: El) -> El {
    el.on_wheel(|s: &mut App, e: WheelEvent, _tx: &mut Tx| {
        if s.locked {
            return false;
        }
        let now = s.clock;
        let step = s.nav.swipe.step(input(&e), now);
        if let Some(dir) = step.go
            && can_go(s, dir)
        {
            s.shell.close_popups();
            go(s, dir);
        }
        step.take
    })
}

/// Every frame: a swipe whose end never came (the window lost the pointer) is dropped after
/// [`swipe::STALE`], with its arrow.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    if app.nav.swipe.expire(app.clock) {
        return true;
    }
    if !matches!(app.nav.swipe, Swipe::Idle) {
        tx.wake_after(std::time::Duration::from_secs_f64(swipe::STALE));
    }
    false
}

/// The swipe's arrow, for the page panel (which clips it): a `size-9 rounded-full border
/// bg-popover shadow-md` circle centred on the edge the page will come from, sliding in with the
/// fingers. It fills with the primary colour once releasing would navigate. It follows the
/// fingers rather than animating, so Reduce Motion leaves it as is. It is decorative and hidden
/// from assistive technology; the navigation itself moves focus and announces as any page change.
pub fn affordance(app: &App, tk: &Tk) -> Option<El> {
    let (dir, p) = app.nav.swipe.progress()?;
    if p < 0.05 || !can_go(app, dir) {
        return None;
    }
    let size = tk.r(2.25);
    let ready = p >= 1.0;
    // From just past the edge (hidden) to 1 rem inside it at the commit mark.
    let lead = -size + (size + tk.r(1.0)) * p;
    let glyph: &'static rune::Icon = if dir == Dir::Back {
        &I::ARROW_LEFT
    } else {
        &I::ARROW_RIGHT
    };
    let circle = row()
        .key("nav-swipe-arrow")
        .size(size, size)
        .center()
        .radius(size / 2.0)
        .fill(if ready { tk.primary } else { tk.popover })
        .border(1.0, if ready { tk.primary } else { tk.border })
        .shadow(4.0, 6.0, rune::Color::BLACK.alpha(0.1))
        .opacity((p * 2.0).min(1.0))
        .child(ic(
            glyph,
            tk.r(1.0),
            if ready { tk.primary_fg } else { tk.muted_fg },
        ));
    let (circle, wrap) = match dir {
        Dir::Back => (circle.ml(lead), row().justify_start()),
        Dir::Forward => (circle.mr(lead), row().justify_end()),
    };
    Some(
        wrap.key("nav-swipe")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center_y()
            .inert()
            .a11y_hidden()
            .child(circle),
    )
}

/// Headless scenes: Transactions with two pages behind it (Overview, Connections) and one ahead
/// (Reports), so a press that fired twice would show; a swipe frozen mid-way and at the mark.
fn with_history(app: &mut App) {
    app.navigate("/overview");
    app.navigate("/connections");
    app.navigate("/transactions");
    app.navigate("/reports");
    app.go_back();
    crate::transactions::prime(app, false);
}

/// Overview with Transactions behind it, for swipes over its chart (which pans).
fn on_overview(app: &mut App) {
    app.navigate("/transactions");
    app.navigate("/overview");
}

fn tracking(app: &mut App, dx: f32) {
    with_history(app);
    app.nav.swipe = Swipe::Tracking {
        dx,
        dy: 0.0,
        at: f64::MAX / 4.0,
    };
}

pub static SCENES: &[crate::app::Scene] = &[
    ("nav-history", with_history),
    ("nav-overview", on_overview),
    // The table expanded over the page: below 1080 pt wide it scrolls sideways.
    ("nav-history-table", |a| {
        with_history(a);
        a.txs.expanded = true;
    }),
    ("nav-swipe-back", |a| tracking(a, swipe::COMMIT * 0.5)),
    ("nav-swipe-back-ready", |a| tracking(a, swipe::COMMIT * 1.2)),
    ("nav-swipe-forward-ready", |a| {
        tracking(a, -swipe::COMMIT * 1.2)
    }),
    ("nav-swipe-forward-light", |a| {
        tracking(a, -swipe::COMMIT * 0.6);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
];
