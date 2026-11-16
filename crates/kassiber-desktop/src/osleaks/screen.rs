//! "Hide amounts while the screen is shared" (Settings → Privacy, on by default): while the
//! screen is shared or recorded, a Normal privacy mode is raised to Hide amounts, and lowered
//! again when the capture ends, unless the user changed the mode in between (then theirs
//! stands). The saved preference never records the raised mode (`saved`).
//!
//! macOS has no public "the screen is being captured" signal (`UIScreen.isCaptured` is iOS only;
//! `CGSessionCopyCurrentDictionary` has no capture key; `SCShareableContent` needs the Screen
//! Recording permission). The WindowServer's own `CGSIsScreenWatcherPresent` (SkyLight, private)
//! is true while any screen watcher is attached: ScreenCaptureKit recordings and shares (the
//! screenshot toolbar's recording, QuickTime, video calls), and Screen Sharing / remote
//! management sessions. It takes no permission and prompts for nothing. Being private, it is
//! resolved at run time with `dlsym`: if a macOS release drops it, the setting says the system
//! does not report sharing and nothing else changes. It cannot tell a capture of another
//! display from one of Kassiber's; amounts hide either way, which errs on the private side.
//!
//! Windows and Linux report no capture to apps (Windows only lets a window exclude itself,
//! `SetWindowDisplayAffinity`), so the setting is unavailable there.
//!
//! A background thread asks once a second while the setting is on (about 20 µs a call) and wakes
//! the event loop only when the answer changes. So that no frame shows amounts before the first
//! answer, the poll that turns the setting on (the first poll after launch included, which runs
//! before the first frame is drawn) asks once itself. Tooling runs never start it; scenes simulate.

use std::cell::{Cell, RefCell};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

use crate::app::{App, Privacy, ToastTone};
use crate::i18n::t;

const UNKNOWN: u8 = 0;
const IDLE: u8 = 1;
const CAPTURED: u8 = 2;

/// What the detector thread saw last.
static SEEN: AtomicU8 = AtomicU8::new(UNKNOWN);
/// Whether the setting is on (the thread does not ask while it is off).
static WANTED: AtomicBool = AtomicBool::new(false);
/// Whether a detector exists on this system (set once the thread starts).
static DETECTOR: AtomicBool = AtomicBool::new(false);
/// The system's "a screen watcher is attached" call, once `start` found it.
static PROBE: OnceLock<fn() -> bool> = OnceLock::new();
const INTERVAL: Duration = Duration::from_secs(1);

/// The privacy-mode side of a capture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Machine {
    /// The last observation: a capture is on, and the setting is on.
    pub captured: bool,
    pub enabled: bool,
    /// Kassiber raised Normal to Hide amounts and will lower it again.
    pub forced: bool,
}

impl Machine {
    /// One observation of the capture state, the setting and the current mode: the mode to
    /// switch to, if any. A capture (or the setting) starting raises Normal once; a mode the
    /// user picks while it lasts ends Kassiber's part.
    pub fn step(&mut self, enabled: bool, captured: bool, privacy: Privacy) -> Option<Privacy> {
        let begins = captured && enabled && !(self.captured && self.enabled);
        self.captured = captured;
        self.enabled = enabled;
        if self.forced {
            if privacy != Privacy::HideAmounts {
                self.forced = false;
                return None;
            }
            if !captured || !enabled {
                self.forced = false;
                return Some(Privacy::Normal);
            }
            return None;
        }
        if begins && privacy == Privacy::Normal {
            self.forced = true;
            return Some(Privacy::HideAmounts);
        }
        None
    }

    /// The mode the preferences file keeps: the user's own while Kassiber holds amounts hidden.
    pub fn saved(&self, privacy: Privacy) -> Privacy {
        if self.forced && privacy == Privacy::HideAmounts {
            Privacy::Normal
        } else {
            privacy
        }
    }
}

thread_local! {
    static MACHINE: RefCell<Machine> = RefCell::new(Machine::default());
    /// Scenes: a simulated capture (and a detector), instead of the system's.
    static SIMULATED: Cell<Option<bool>> = const { Cell::new(None) };
    /// Scenes: a system that reports no captures (Windows, Linux).
    static NO_DETECTOR: Cell<bool> = const { Cell::new(false) };
}

/// For `persist`: the privacy mode to save.
pub fn saved_privacy(privacy: Privacy) -> Privacy {
    MACHINE.with(|m| m.borrow().saved(privacy))
}

/// Whether Kassiber currently holds amounts hidden for a capture.
pub fn forced() -> bool {
    MACHINE.with(|m| m.borrow().forced)
}

/// Whether this system reports captures to Kassiber.
pub fn available() -> bool {
    if NO_DETECTOR.with(Cell::get) {
        return false;
    }
    SIMULATED.with(Cell::get).is_some()
        || DETECTOR.load(Ordering::Relaxed)
        || (cfg!(target_os = "macos") && crate::persist::file::tooling_run())
}

/// Scenes: pretend the screen is (or is not) being captured.
pub fn simulate(captured: bool) {
    SIMULATED.with(|s| s.set(Some(captured)));
}

/// Scenes: pretend this system reports no captures.
pub fn simulate_no_detector() {
    NO_DETECTOR.with(|s| s.set(true));
}

/// Every frame: feeds the machine and applies its decision. True when the mode changed.
pub fn poll(app: &mut App) -> bool {
    let enabled = app.settings.prefs.hide_on_screen_share;
    want(&WANTED, &SEEN, enabled, PROBE.get().copied());
    let captured = SIMULATED
        .with(Cell::get)
        .unwrap_or_else(|| SEEN.load(Ordering::Relaxed) == CAPTURED);
    let (next, was) = MACHINE.with(|m| {
        let mut m = m.borrow_mut();
        let was = m.forced;
        (m.step(enabled, captured, app.prefs.privacy), was)
    });
    match next {
        Some(p) => {
            app.prefs.privacy = p;
            if p == Privacy::HideAmounts {
                app.toast(
                    ToastTone::Info,
                    &t("settings:privacy.screenShareToastTitle"),
                    Some(t("settings:privacy.screenShareToastBody")),
                    Some("screen-share-hidden"),
                );
            }
            true
        }
        None => was != forced(),
    }
}

/// Records whether the setting is on; turning it on (the first poll included) asks the
/// detector now rather than at the thread's next round, so the poll's frame already knows.
fn want(wanted: &AtomicBool, seen: &AtomicU8, enabled: bool, probe: Option<fn() -> bool>) {
    if !wanted.swap(enabled, Ordering::Relaxed) && enabled {
        probe_into(seen, probe);
    }
}

/// One synchronous answer from the system's detector, when there is one.
fn probe_into(seen: &AtomicU8, probe: Option<fn() -> bool>) {
    if let Some(watching) = probe {
        seen.store(if watching() { CAPTURED } else { IDLE }, Ordering::Relaxed);
    }
}

/// Starts the detector thread (from `setup`, with the event loop's waker). Tooling runs and
/// systems without a detector start nothing. The thread asks only while the setting is on and
/// leaves the last answer alone while it is off (`poll` asks afresh when it turns on).
pub fn start(wake: rune::Waker) {
    if crate::persist::file::tooling_run() {
        return;
    }
    let Some(watching) = detector() else { return };
    let _ = PROBE.set(watching);
    DETECTOR.store(true, Ordering::Relaxed);
    // The setting may already be on (the first poll ran before `setup`): ask once now.
    if WANTED.load(Ordering::Relaxed) {
        probe_into(&SEEN, Some(watching));
    }
    let _ = std::thread::Builder::new()
        .name("kassiber-screen-share".into())
        .spawn(move || {
            loop {
                if WANTED.load(Ordering::Relaxed) {
                    let now = if watching() { CAPTURED } else { IDLE };
                    if SEEN.swap(now, Ordering::Relaxed) != now {
                        wake();
                    }
                }
                std::thread::sleep(INTERVAL);
            }
        });
}

/// `CGSIsScreenWatcherPresent`, if this macOS has it.
#[cfg(target_os = "macos")]
fn detector() -> Option<fn() -> bool> {
    use std::ffi::{c_char, c_void};
    unsafe extern "C" {
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }
    /// `RTLD_DEFAULT`.
    const DEFAULT: *mut c_void = -2isize as *mut c_void;
    static FN: OnceLock<usize> = OnceLock::new();
    let p = unsafe { dlsym(DEFAULT, c"CGSIsScreenWatcherPresent".as_ptr()) };
    if p.is_null() {
        return None;
    }
    let _ = FN.set(p as usize);
    fn call() -> bool {
        let Some(p) = FN.get().copied() else {
            return false;
        };
        // Declared `bool`; read as a byte, which is right for a C `bool` and for the low byte
        // of an `int` alike.
        let f = unsafe {
            std::mem::transmute::<*mut c_void, unsafe extern "C" fn() -> u8>(p as *mut c_void)
        };
        unsafe { f() != 0 }
    }
    Some(call)
}

#[cfg(not(target_os = "macos"))]
fn detector() -> Option<fn() -> bool> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use Privacy::{HideAll, HideAmounts, Normal};

    #[test]
    fn a_capture_hides_amounts_and_its_end_restores_normal() {
        let mut m = Machine::default();
        assert_eq!(m.step(true, false, Normal), None);
        assert_eq!(m.step(true, true, Normal), Some(HideAmounts));
        assert_eq!(
            m.saved(HideAmounts),
            Normal,
            "the raised mode is never saved"
        );
        assert_eq!(m.step(true, true, HideAmounts), None);
        assert_eq!(m.step(true, false, HideAmounts), Some(Normal));
        assert!(!m.forced);
        assert_eq!(m.step(true, false, Normal), None);
    }

    #[test]
    fn a_mode_already_hidden_is_left_alone() {
        let mut m = Machine::default();
        assert_eq!(m.step(true, true, HideAmounts), None);
        assert_eq!(m.step(true, false, HideAmounts), None);
        assert_eq!(m.step(true, true, HideAll), None);
        assert_eq!(m.step(true, false, HideAll), None);
        assert_eq!(m.saved(HideAmounts), HideAmounts);
    }

    #[test]
    fn the_users_choice_during_a_capture_stands() {
        let mut m = Machine::default();
        assert_eq!(m.step(true, true, Normal), Some(HideAmounts));
        // The user shows amounts on purpose while sharing: not hidden again this capture.
        assert_eq!(m.step(true, true, Normal), None);
        assert_eq!(m.step(true, true, Normal), None);
        assert_eq!(m.step(true, false, Normal), None);
        // A new capture hides again.
        assert_eq!(m.step(true, true, Normal), Some(HideAmounts));
        // Raising to Hide all is also the user's: kept when the capture ends.
        assert_eq!(m.step(true, true, HideAll), None);
        assert_eq!(m.step(true, false, HideAll), None);
    }

    #[test]
    fn turning_the_setting_on_asks_at_once() {
        fn sharing() -> bool {
            true
        }
        fn not_sharing() -> bool {
            false
        }
        let (wanted, seen) = (AtomicBool::new(false), AtomicU8::new(UNKNOWN));
        // The first poll after launch, with the setting on: answered before its frame.
        want(&wanted, &seen, true, Some(sharing));
        assert_eq!(seen.load(Ordering::Relaxed), CAPTURED);
        let mut m = Machine::default();
        assert_eq!(
            m.step(true, seen.load(Ordering::Relaxed) == CAPTURED, Normal),
            Some(HideAmounts),
            "hidden from the first frame"
        );
        // Still on: the thread answers from here; the poll does not ask again.
        want(&wanted, &seen, true, Some(not_sharing));
        assert_eq!(seen.load(Ordering::Relaxed), CAPTURED);
        // Off, then on again: asked afresh.
        want(&wanted, &seen, false, Some(not_sharing));
        want(&wanted, &seen, true, Some(not_sharing));
        assert_eq!(seen.load(Ordering::Relaxed), IDLE);
        // No detector on this system: nothing is recorded.
        let fresh = AtomicU8::new(UNKNOWN);
        want(&AtomicBool::new(false), &fresh, true, None);
        assert_eq!(fresh.load(Ordering::Relaxed), UNKNOWN);
    }

    #[test]
    fn the_setting_gates_hiding_and_turning_it_off_restores() {
        let mut m = Machine::default();
        assert_eq!(m.step(false, true, Normal), None);
        // Turned on mid-capture: hides at once.
        assert_eq!(m.step(true, true, Normal), Some(HideAmounts));
        // Turned off mid-capture: the user's mode returns.
        assert_eq!(m.step(false, true, HideAmounts), Some(Normal));
        assert_eq!(m.step(false, false, Normal), None);
    }
}
