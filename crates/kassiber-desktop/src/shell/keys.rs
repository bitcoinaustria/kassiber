//! Shell keyboard shortcuts ("mod" = ⌘ on macOS, Ctrl elsewhere), as AppShell.tsx,
//! appWorkflowHotkeys.ts, shellNavigation.ts and the native menu accelerators bind them.

use rune::{Key, KeyEvent, Tx};

use super::Popup;
use crate::app::{App, Privacy};

fn set_scale(app: &mut App, scale: f32) {
    app.prefs.scale = ((scale * 20.0).round() / 20.0).clamp(0.8, 1.2);
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    let m = ev.mods;
    // ⌘L locks from anywhere; locked, only the passphrase unlocks (the menu item is disabled).
    if ev.is_command('l') && !m.shift {
        super::overlays::lock(app);
        return true;
    }
    if app.locked {
        return false;
    }
    match &ev.key {
        Key::Escape if app.shell.settings_open => {
            app.shell.settings_open = false;
            true
        }
        Key::Escape if app.shell.popup.is_some() => {
            app.shell.popup = None;
            true
        }
        Key::Left if m.alt && !m.cmd => {
            app.go_back();
            true
        }
        Key::Right if m.alt && !m.cmd => {
            app.go_forward();
            true
        }
        Key::Command(c) => {
            let c = *c;
            match (c, m.shift) {
                ('k', false) => app.shell.toggle(Popup::Search),
                ('b', false) => super::toggle_nav(app, super::narrow_now(app)),
                ('r', false) | ('s', true) => app.refresh_book_from_title_bar(false),
                ('r', true) => app.refresh_book_from_title_bar(true),
                ('j', true) => app.process_journals(),
                ('h', true) => {
                    app.prefs.privacy = match app.prefs.privacy {
                        Privacy::Normal => Privacy::HideAmounts,
                        Privacy::HideAmounts => Privacy::HideAll,
                        Privacy::HideAll => Privacy::Normal,
                    }
                }
                ('[', _) => app.go_back(),
                (']', _) => app.go_forward(),
                (',', false) => {
                    app.shell.popup = None;
                    app.shell.settings_open = true;
                }
                ('-', _) => set_scale(app, app.prefs.scale - 0.05),
                ('=', _) | ('+', _) => set_scale(app, app.prefs.scale + 0.05),
                ('0', false) => set_scale(app, crate::app::DEFAULT_SCALE),
                // ⌘⇧A and ⌘1–⌘9 are menu accelerators: `native::on_key` fires the item, with the
                // menu's gates and the route guards (`dispatchMenuIntent`).
                _ => return false,
            }
            true
        }
        _ => false,
    }
}
