//! Operating-system surfaces where book data could leave Kassiber's window
//! (docs/reference/privacy-and-security.md, "Operating-system surfaces"):
//!
//! - [`clipboard`]: every copy is kept on this device and out of clipboard histories, and is
//!   cleared after 30 s (with "Clear clipboard after copy" on), on lock and on quit, while the
//!   clipboard still holds it;
//! - OS notifications carry fixed, generic texts only (`native::notify`; guarded by the tests in
//!   `notify_guard`);
//! - [`backup`]: unencrypted books are kept out of Time Machine item by item, and the Data page
//!   warns while one sits in a synced folder;
//! - [`screen`]: amounts hide while the screen is shared or recorded.
//!
//! None of it makes a network request; all of it is local system calls. Tooling runs record
//! instead of touching the clipboard or file attributes.

pub mod backup;
pub mod clipboard;
pub mod screen;
pub mod ui;

#[cfg(test)]
mod notify_guard;

use std::cell::Cell;

use rune::Tx;
use serde_json::Value;

use crate::app::App;

thread_local! {
    static WAS_LOCKED: Cell<bool> = const { Cell::new(false) };
}

/// From `setup`: the event loop's waker, for the screen-share detector.
pub fn set_waker(wake: rune::Waker) {
    screen::start(wake);
}

/// Every frame (from `settings::effects::poll`). True when something visible changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = screen::poll(app);
    changed |= backup::poll(app, tx);
    let locked = app.locked;
    if locked && !WAS_LOCKED.with(|w| w.replace(locked)) {
        clipboard::clear_held(Some(tx));
    } else {
        WAS_LOCKED.with(|w| w.set(locked));
    }
    changed
}

/// `App::on_quit`: a copy still on the clipboard that was due to be cleared goes now.
pub fn on_quit() {
    clipboard::clear_held(None);
}

/// Opens a Settings section with the open book's `status` read in, and runs these effects once.
fn open(a: &mut App, section: usize) {
    a.shell.settings_open = true;
    a.shell.settings_section = section;
    crate::settings::prime(a);
    if a.read("status").is_none() {
        a.send("status", Value::Null, "osleaks.status");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while a.read("status").is_none() && std::time::Instant::now() < deadline {
            a.pump();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
    poll(a, &mut Tx::default());
}

fn set_encrypted(a: &mut App, encrypted: bool) {
    if let Some(r) = a.reads.get_mut("status") {
        r.data["database_encrypted"] = Value::Bool(encrypted);
    }
}

/// Named states for `--scene` (headless snapshots, `AREA=osleaks tools/pilot-eval.sh`).
pub const SCENES: &[crate::app::Scene] = &[
    ("osleaks-privacy", |a| open(a, crate::settings::PRIVACY)),
    ("osleaks-privacy-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, crate::settings::PRIVACY)
    }),
    // A capture is on: amounts hidden by Kassiber, the status line and the toast show.
    ("osleaks-privacy-sharing", |a| {
        screen::simulate(true);
        open(a, crate::settings::PRIVACY)
    }),
    // Windows / Linux: no capture signal, the switch is off and says why.
    ("osleaks-privacy-unavailable", |a| {
        screen::simulate_no_detector();
        open(a, crate::settings::PRIVACY)
    }),
    ("osleaks-privacy-clipboard-off", |a| {
        a.settings.prefs.clear_clipboard = false;
        a.settings.prefs.hide_on_screen_share = false;
        open(a, crate::settings::PRIVACY)
    }),
    // The demo book is unencrypted: its files are kept out of Time Machine.
    ("osleaks-data", |a| {
        open(a, crate::settings::DATA);
        set_encrypted(a, false);
        poll(a, &mut Tx::default());
    }),
    ("osleaks-data-encrypted", |a| {
        open(a, crate::settings::DATA);
        set_encrypted(a, true);
        poll(a, &mut Tx::default());
    }),
    ("osleaks-data-off", |a| {
        a.settings.prefs.exclude_unencrypted_backups = false;
        open(a, crate::settings::DATA);
        set_encrypted(a, false);
        poll(a, &mut Tx::default());
    }),
    ("osleaks-data-synced", |a| {
        backup::simulate_sync("iCloud Drive");
        open(a, crate::settings::DATA);
        set_encrypted(a, false);
        poll(a, &mut Tx::default());
    }),
    ("osleaks-data-synced-generic", |a| {
        backup::simulate_sync("");
        a.prefs.theme = crate::app::ThemeMode::Light;
        open(a, crate::settings::DATA);
        set_encrypted(a, false);
        poll(a, &mut Tx::default());
    }),
];
