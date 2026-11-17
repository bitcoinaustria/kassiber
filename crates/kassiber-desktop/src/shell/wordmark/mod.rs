//! The pixel "Kassiber" wordmark (ui-tauri/src/components/kb/KassiberWordmark.tsx) and its K as
//! a busy indicator (`KassiberWorkingMark`). Each `#` of the grid is one filled cell; a frame is
//! drawn as one canvas of horizontal runs, settled cells in the mark's colour and cells still
//! moving or decoding in Bitcoin orange.
//!
//! The animated marks sit at fixed sites, each mounted when the web mounts its component:
//! - the nav's wordmark (AppShell's `SidebarBrand`): decrypts in each time the shell opens or
//!   unlocks, and assembles the first time it opens on books setup just created;
//! - the lock screen's (LockScreen.tsx): decrypts in as the screen appears and again on each
//!   unlock attempt (`decryptKey`), while the books decrypt;
//! - Welcome's start page (StartChoicePanel.tsx): rains in on mount;
//! - the sync card's K (FirstSyncCard.tsx): loops while the card shows.
//!
//! The poll steps them on the frame clock and asks for a frame only when one changes, so a
//! resting mark costs nothing; the view draws what the poll last stepped to.

mod grid;
mod play;

use std::cell::Cell;

use rune::{Color, Cx, El, Shape, Tx, canvas, row};

use crate::app::App;
use grid::{Frame, Motion, Reveal};
use play::{Env, Slot, Want};

/// Bitcoin orange, as in the app icon's wedge, for cells still in motion (`NOISE_FILL`).
fn noise() -> Color {
    rune::color::hex("#f7931a").expect("hex")
}

/// Where an animated mark sits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Site {
    Shell,
    Lock,
    Start,
    Working,
}

const SITES: [Site; 4] = [Site::Shell, Site::Lock, Site::Start, Site::Working];

/// The animated marks' state (in `ShellUi`).
#[derive(Default)]
pub struct Marks {
    slots: [Slot; 4],
    /// The OS's (or Rune's override's) reduced motion, as the view last saw it: `Cx` only
    /// reaches the view, and the poll needs it too.
    reduced: Cell<bool>,
    /// The first poll ran.
    launched: bool,
}

impl Marks {
    /// Reduced motion as the view last saw it, for loops drawn outside a `Cx`.
    pub fn reduced_motion(&self) -> bool {
        self.reduced.get()
    }
}

/// What each site shows now, as the view decides it (None: not on screen).
fn wants(app: &App) -> [Option<Want>; 4] {
    let welcome = crate::onboarding::shows(app);
    let shell = !welcome;
    let reveal = |key: u64, r: Reveal| {
        Some(Want {
            key,
            animate: true,
            motion: Motion::Reveal(r),
        })
    };
    let o = &app.onboarding;
    // The sync card shows while a refresh runs or failed; the K, only while it runs.
    let card = shell && !app.locked && app.shell.sync_card_open && app.busy.syncing;
    [
        if shell && !app.locked {
            reveal(
                0,
                if o.celebrate {
                    Reveal::Assemble
                } else {
                    Reveal::Decrypt
                },
            )
        } else {
            None
        },
        if shell && app.locked {
            reveal(u64::from(app.lock.attempt), Reveal::Decrypt)
        } else {
            None
        },
        if welcome && o.flow != crate::onboarding::Flow::Setup && o.import_selection.is_none() {
            reveal(0, Reveal::Rain)
        } else {
            None
        },
        card.then_some(Want {
            key: 0,
            animate: true,
            motion: Motion::Working,
        }),
    ]
}

/// A headless `--snap`/`--tree` capture shows the marks its scene mounts at rest, as the web's
/// captures (taken once the page settled) do; script, frame and video runs play them.
fn settles_launch() -> bool {
    rune::tooling::headless_run().is_some_and(|r| r.frames() == 0 && r.script().is_none())
}

/// Called by the root view each run: the reduced-motion preference the poll steps under.
pub fn observe(app: &App, cx: &Cx<App>) {
    app.shell.marks.reduced.set(cx.is_reduced_motion());
}

/// Steps every site to the clock; true when a drawn frame changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let now = app.clock;
    let wants = wants(app);
    let marks = &mut app.shell.marks;
    let mut reduced = marks.reduced.get();
    if !marks.launched {
        marks.launched = true;
        // Treated as played: the reveals end at rest, the working K loops on.
        reduced |= settles_launch();
    }
    let busy = marks
        .slots
        .iter()
        .zip(&wants)
        .any(|(s, w)| s.playing() || w.is_some_and(|w| s.pending(&w, Env::default())));
    let env = Env {
        hidden: busy && window_hidden(),
        reduced,
    };
    let (mut changed, mut wake): (bool, Option<f64>) = (false, None);
    let mut shell_ended = false;
    for (i, site) in SITES.into_iter().enumerate() {
        let step = marks.slots[i].step(wants[i], now, env);
        changed |= step.changed;
        shell_ended |= step.ended && site == Site::Shell;
        wake = match (wake, step.wake) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
    }
    if let Some(at) = wake {
        tx.wake_at(at);
    }
    // `shellRevealPlayed`: the assemble cue is spent once its reveal has ended.
    if shell_ended {
        app.onboarding.celebrate = false;
    }
    changed
}

/// The frame a site draws now: the poll's, or, for a mount the poll has not stepped yet, the
/// play's first frame (the web starts it before paint, so the mark never flashes at rest).
fn frame(app: &App, site: Site) -> (Option<Frame>, Option<Want>) {
    let marks = &app.shell.marks;
    let i = site as usize;
    let want = wants(app)[i];
    let slot = &marks.slots[i];
    let env = Env {
        hidden: false,
        reduced: marks.reduced.get(),
    };
    let starting = marks.launched || !settles_launch();
    match want {
        Some(w) if starting && slot.pending(&w, env) => (Some(w.motion.frame(0.0)), want),
        _ => (slot.frame().cloned(), want),
    }
}

/// `document.visibilityState === "hidden"`: the window is minimized, or on macOS not visible at
/// all (covered, on another Space, the app hidden), as WebKit reports a page hidden.
fn window_hidden() -> bool {
    let Some(h) = crate::persist::window::handle() else {
        return false;
    };
    #[cfg(target_os = "macos")]
    if let Some(n) = h.window_number() {
        return !macos_visible(n);
    }
    h.placement().is_some_and(|p| p.minimized)
}

/// `NSWindow.occlusionState` has `NSWindowOcclusionStateVisible` (`1 << 1`).
#[cfg(target_os = "macos")]
fn macos_visible(number: u32) -> bool {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    // SAFETY: NSApplication's shared instance lives for the process; `windowWithWindowNumber:`
    // returns a window or nil, checked before `occlusionState` (an NSUInteger). Called from the
    // poll, on the UI thread.
    unsafe {
        let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
        if app.is_null() {
            return true;
        }
        let window: *mut AnyObject = msg_send![app, windowWithWindowNumber: number as isize];
        if window.is_null() {
            return true;
        }
        let state: usize = msg_send![window, occlusionState];
        state & (1 << 1) != 0
    }
}

/// The cells of a frame at `cell` points each.
fn shape(frame: &Frame, cell: f32, color: Color) -> Shape {
    let mut s = Shape::new();
    for (runs, c) in [(&frame.solid, color), (&frame.noise, noise())] {
        for r in runs {
            let (x0, x1) = (r.x as f32 * cell, (r.x + r.w) as f32 * cell);
            let (y0, y1) = (r.y as f32 * cell, (r.y + 1) as f32 * cell);
            s = s.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], c);
        }
    }
    s
}

/// The word in its clip: folded, only the K shows (the clip falls mid-gap after it); open, a
/// point of slack keeps the clip off the R. Rain enters from above the box, so the clip trims it.
fn word_el(frame: &Frame, cell: f32, color: Color, folded: bool) -> El {
    let w = if folded {
        (grid::K_COLUMNS as f32 + 0.5) * cell
    } else {
        grid::COLUMNS as f32 * cell + 1.0
    };
    let (cw, ch) = (grid::COLUMNS as f32 * cell, grid::ROWS as f32 * cell);
    row()
        .w(w)
        .h(ch)
        .clip_rounded(0.0)
        .role("img")
        .label("Kassiber")
        .child(canvas(shape(frame, cell, color)).size(cw, ch).a11y_hidden())
}

/// "ALPHA" in the wordmark's pixel language (an owner request: the pre-alpha tag reads as part of
/// the logo, Omarchy-style, instead of a separate pill): 6×9 letters with the wordmark's heavy,
/// two-cell strokes, two cells apart. Decorative here; the caller names the control.
const ALPHA_GLYPHS: [[&str; 9]; 5] = [
    [
        ".####.", "######", "##..##", "##..##", "######", "######", "##..##", "##..##", "##..##",
    ],
    [
        "##....", "##....", "##....", "##....", "##....", "##....", "##....", "######", "######",
    ],
    [
        "#####.", "######", "##..##", "##..##", "######", "#####.", "##....", "##....", "##....",
    ],
    [
        "##..##", "##..##", "##..##", "##..##", "######", "######", "##..##", "##..##", "##..##",
    ],
    [
        ".####.", "######", "##..##", "##..##", "######", "######", "##..##", "##..##", "##..##",
    ],
];
const ALPHA_GLYPH_W: usize = 6;
const ALPHA_GAP: usize = 2;
pub const ALPHA_COLUMNS: usize = 5 * ALPHA_GLYPH_W + 4 * ALPHA_GAP;
pub const ALPHA_ROWS: usize = 9;

pub fn alpha_mark(cell: f32, color: Color) -> El {
    let mut s = Shape::new();
    for (i, glyph) in ALPHA_GLYPHS.iter().enumerate() {
        let x_off = i * (ALPHA_GLYPH_W + ALPHA_GAP);
        for (y, row) in glyph.iter().enumerate() {
            // One rectangle per run of cells, as the wordmark draws its rows.
            let cells: Vec<char> = row.chars().collect();
            let mut x = 0;
            while x < cells.len() {
                if cells[x] != '#' {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < cells.len() && cells[x] == '#' {
                    x += 1;
                }
                let (x0, x1) = ((x_off + start) as f32 * cell, (x_off + x) as f32 * cell);
                let (y0, y1) = (y as f32 * cell, (y + 1) as f32 * cell);
                s = s.fill(vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], color);
            }
        }
    }
    canvas(s)
        .size(ALPHA_COLUMNS as f32 * cell, ALPHA_ROWS as f32 * cell)
        .a11y_hidden()
}

/// The wordmark at rest, at `cell` points per cell (Welcome's setup and open-books pages).
pub fn wordmark(cell: f32, color: Color, folded: bool) -> El {
    word_el(&grid::word(), cell, color, folded).key("sh-wordmark")
}

/// When the mark comes on screen, ask the poll for the next frame, which starts its play: a
/// mount from a handler (an unlock attempt, the shell opening) otherwise waits for the next
/// wake to begin.
fn kick(el: El) -> El {
    el.on_visible(|_: &mut App, on: bool, tx: &mut Tx| {
        if on {
            tx.wake_after(std::time::Duration::ZERO);
        }
        tx.live_only();
    })
}

/// The wordmark at a site that animates it (the nav, the lock screen, Welcome's start page).
pub fn animated(app: &App, site: Site, cell: f32, color: Color, folded: bool) -> El {
    let (frame, want) = frame(app, site);
    let frame = frame.unwrap_or_else(grid::word);
    // A new key is a remount, as React's `key={decryptKey}`.
    let key = format!("sh-wordmark-{site:?}-{}", want.map_or(0, |w| w.key)).to_lowercase();
    kick(word_el(&frame, cell, color, folded).key(key))
}

/// The K as a busy indicator (`KassiberWorkingMark`): decrypts in, holds, scrambles back out
/// and repeats while it shows; at rest under reduced motion. Decorative: hidden from assistive
/// technology, as the web's `aria-hidden` SVG is.
pub fn working_mark(app: &App, cell: f32, color: Color) -> El {
    let frame = frame(app, Site::Working).0.unwrap_or_else(grid::k);
    let (w, h) = (grid::K_COLUMNS as f32 * cell, grid::K_ROWS as f32 * cell);
    kick(
        canvas(shape(&frame, cell, color))
            .size(w, h)
            .key("sh-working-mark")
            .a11y_hidden(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime(scene: fn(&mut App)) -> rune::Runtime<App> {
        let app = rune::App::new(rune::AppSpec::<App> {
            fonts: crate::FONTS,
            families: (crate::sans_stack(), crate::MONO),
            setup: Some(crate::setup),
            poll: Some(crate::poll),
            on_key: Some(crate::on_key),
            ..rune::AppSpec::new("Kassiber", (1440.0, 900.0), App::new, crate::view)
        });
        let mut rt = app.headless_runtime((1440.0, 900.0), rune::Env::new(true, 1.0));
        // The launch reads land first, so their answers (status, lock) can't undo the scene.
        crate::wallets::common::pump_until(&mut rt.state, |a| a.pending.is_empty());
        // A book is open (tests restore no preferences, so the launch would be Welcome).
        rt.state.onboarding.identity = Some(crate::onboarding::model::Identity {
            name: "Test".into(),
            ..Default::default()
        });
        rt.state.navigate("/overview");
        scene(&mut rt.state);
        rt
    }

    fn commits_over(rt: &mut rune::Runtime<App>, seconds: f64) -> u64 {
        let before = rt.stats.commits;
        for _ in 0..(seconds * 60.0).round() as usize {
            rt.frame(1.0 / 60.0);
        }
        rt.stats.commits - before
    }

    /// The lock screen's decrypt plays from the first frame, then the mark rests: no commit and
    /// no wake of its own once it has landed.
    #[test]
    fn the_lock_reveal_plays_then_costs_nothing() {
        let mut rt = runtime(|a| a.locked = true);
        rt.frame(1.0 / 60.0);
        let m = &rt.state.shell.marks;
        assert!(
            m.slots[Site::Lock as usize].playing(),
            "decrypting on appear: locked {} welcome {} reduced {} launched {} slot {:?} wants {:?}",
            rt.state.locked,
            crate::onboarding::shows(&rt.state),
            m.reduced.get(),
            m.launched,
            m.slots[1],
            wants(&rt.state)
        );
        assert!(
            commits_over(&mut rt, 0.4) > 3,
            "the opening flicker commits every 70 ms"
        );
        commits_over(&mut rt, 1.4);
        assert!(!rt.state.shell.marks.slots[Site::Lock as usize].playing());
        assert_eq!(
            rt.state.shell.marks.slots[Site::Lock as usize].frame(),
            None,
            "at rest"
        );
        assert_eq!(
            commits_over(&mut rt, 1.0),
            0,
            "a resting mark asks for nothing"
        );
        // An unlock attempt remounts it and it decrypts again.
        rt.update(|a, _| a.lock.attempt += 1);
        rt.frame(1.0 / 60.0);
        assert!(
            rt.state.shell.marks.slots[Site::Lock as usize].playing(),
            "replays on the attempt"
        );
    }

    /// The working K loops while the sync card shows, sleeps through its hold, and stops with the
    /// card.
    #[test]
    fn the_working_k_loops_only_while_the_card_shows() {
        let mut rt = runtime(|a| {
            a.locked = false;
            a.busy.syncing = true;
            a.shell.sync_card_open = true;
        });
        commits_over(&mut rt, 1.5);
        let (locked, welcome) = (rt.state.locked, crate::onboarding::shows(&rt.state));
        assert!(
            rt.state.shell.marks.slots[Site::Working as usize].playing(),
            "locked {locked}, welcome {welcome}"
        );
        // 1.5 s into the loop the K holds whole until 1.9 s.
        assert_eq!(commits_over(&mut rt, 0.35), 0, "the hold draws nothing");
        assert!(commits_over(&mut rt, 1.0) > 5, "scrambling out");
        rt.update(|a, _| a.busy.syncing = false);
        commits_over(&mut rt, 0.1);
        assert!(
            !rt.state.shell.marks.slots[Site::Working as usize].playing(),
            "unmounted with the card"
        );
    }

    /// Right after setup created the books the nav's word assembles, and the cue is spent once
    /// that reveal has played (`celebrateBooksCreated` / `shellRevealPlayed`).
    #[test]
    fn the_first_shell_after_setup_assembles_once() {
        let mut rt = runtime(|a| {
            a.locked = false;
            a.onboarding.celebrate = true;
        });
        rt.frame(1.0 / 60.0);
        assert_eq!(
            wants(&rt.state)[Site::Shell as usize].map(|w| w.motion),
            Some(Motion::Reveal(Reveal::Assemble))
        );
        commits_over(&mut rt, 1.0);
        assert!(rt.state.onboarding.celebrate, "still assembling");
        commits_over(&mut rt, 1.0);
        assert!(!rt.state.onboarding.celebrate, "spent");
        assert_eq!(
            wants(&rt.state)[Site::Shell as usize].map(|w| w.motion),
            Some(Motion::Reveal(Reveal::Decrypt))
        );
        assert!(
            !rt.state.shell.marks.slots[Site::Shell as usize].playing(),
            "and it does not replay"
        );
    }
}
