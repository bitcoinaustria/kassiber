//! The native menu bar (`build_app_menu`, `handle_app_menu_event`, `set_menu_state` in
//! src-tauri lib.rs): every item with its id, accelerator and enabled state, kept in step with
//! the app as AppShell's `set_menu_state` effect keeps Tauri's.
//!
//! Rune draws native menus on macOS only (`AppHandle::set_menu` is `Unsupported` on Windows and
//! Linux). Rune's own items stay: the app menu (About, Services, Hide…, Quit), the Edit menu
//! (Undo … Select All, as Tauri's) and the Window menu's Minimize/Close, which this menu joins.
//! Items whose key the window also handles let the window see it first (`keys_first`), as a web
//! page sees a key before WKWebView hands it to the menu; [`on_key`] fires every item by its
//! accelerator when nothing else took the key, so the accelerators (and their gates) work the
//! same where Rune has no menu bar.

use rune::{Key, KeyEvent, MenuItem, Shortcut, Tx};

use super::intent::{Intent, dispatch, has_workspace};
use crate::app::App;
use crate::i18n::{Lang, t};

const DOCS_URL: &str = "https://github.com/bitcoinaustria/kassiber#readme";
const ISSUES_URL: &str = "https://github.com/bitcoinaustria/kassiber/issues";

/// How an item's label is translated: a web key (whose English is Tauri's label), or Tauri's
/// English with a German of its own where the web has no string (the Tauri menu is English-only).
#[derive(Clone, Copy, Debug)]
pub enum Text {
    Key(&'static str),
    Own(&'static str, &'static str),
}

impl Text {
    pub fn get(self, lang: Lang) -> String {
        match self {
            Text::Key(k) => t(k),
            Text::Own(en, de) => if lang == Lang::De { de } else { en }.to_owned(),
        }
    }
}

/// `set_menu_state`'s groups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Always,
    /// `workspace_gated`: a book is open and not locked.
    Workspace,
    /// The View menu's Assistant: AI features on, and the workspace gate.
    Assistant,
    /// Help › Logs: developer tools on, and the workspace gate.
    Logs,
}

/// Where an item goes: Rune's app menu, or a named top-level menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // macOS-only path (and its tests)
    App,
    Menu(&'static str),
}

#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// Tauri's menu id (`kassiber:…`); empty for a separator.
    pub id: &'static str,
    pub place: Place,
    pub text: Text,
    /// Tauri's accelerator as Rune parses it (`cmd` is ⌘ on macOS, Ctrl elsewhere).
    pub accel: Option<&'static str>,
    pub gate: Gate,
    /// Placed right after this item (a Rune item's id, or a standard item's AppKit action).
    pub after: Option<&'static str>,
}

const fn item(
    id: &'static str,
    place: Place,
    text: Text,
    accel: Option<&'static str>,
    gate: Gate,
) -> Entry {
    Entry {
        id,
        place,
        text,
        accel,
        gate,
        after: None,
    }
}

const fn sep(place: Place) -> Entry {
    Entry {
        id: "",
        place,
        text: Text::Own("", ""),
        accel: None,
        gate: Gate::Always,
        after: None,
    }
}

pub const CHECK_UPDATES: &str = "kassiber:check-updates";
pub const OPEN_SETTINGS: &str = "kassiber:settings";
pub const LOCK: &str = "kassiber:lock";
pub const FULLSCREEN: &str = "kassiber:window:toggle-fullscreen";
pub const CLOSE: &str = "kassiber:window:close";
pub const MINIMIZE: &str = "kassiber:window:minimize";
pub const ZOOM: &str = "kassiber:window:zoom";
pub const FOCUS: &str = "kassiber:window:focus";
pub const QUIT: &str = "kassiber:quit";
pub const HELP_DOCS: &str = "kassiber:help:docs";
pub const HELP_ISSUES: &str = "kassiber:help:issues";

const FILE: Place = Place::Menu("File");
const VIEW: Place = Place::Menu("View");
const FLOWS: Place = Place::Menu("Workflows");
const SETTINGS: Place = Place::Menu("Settings");
const WINDOW: Place = Place::Menu("Window");
const HELP: Place = Place::Menu("Help");

use Gate::{Always, Assistant, Logs, Workspace};

/// The macOS layout, in menu order (`build_app_menu`). Rune makes the menus Rune already has
/// (the app menu, Edit, Window's Minimize and Close) and places these into them.
#[cfg(target_os = "macos")]
pub static ENTRIES: &[Entry] = &[
    // Kassiber: About · Check for Updates… · — · Settings... · — · Services … Quit (Rune's).
    item(
        CHECK_UPDATES,
        Place::App,
        Text::Own("Check for Updates…", "Nach Updates suchen…"),
        None,
        Always,
    ),
    sep(Place::App),
    item(
        OPEN_SETTINGS,
        Place::App,
        Text::Own("Settings...", "Einstellungen..."),
        Some("cmd+,"),
        Always,
    ),
    // File
    item(
        LOCK,
        FILE,
        Text::Key("chrome:shell.lockKassiber"),
        Some("cmd+l"),
        Workspace,
    ),
    sep(FILE),
    item(
        CLOSE,
        FILE,
        Text::Own("Close Window", "Fenster schließen"),
        Some("cmd+w"),
        Always,
    ),
    // View
    item(
        "kassiber:navigate:overview",
        VIEW,
        Text::Key("nav:book.overview"),
        Some("cmd+1"),
        Workspace,
    ),
    item(
        "kassiber:navigate:transactions",
        VIEW,
        Text::Key("nav:book.transactions"),
        Some("cmd+2"),
        Workspace,
    ),
    item(
        "kassiber:navigate:connections",
        VIEW,
        Text::Own("Connections", "Verbindungen"),
        Some("cmd+3"),
        Workspace,
    ),
    item(
        "kassiber:navigate:books",
        VIEW,
        Text::Key("chrome:routeMeta.books.title"),
        Some("cmd+4"),
        Workspace,
    ),
    item(
        "kassiber:navigate:reports",
        VIEW,
        Text::Key("nav:book.reports"),
        Some("cmd+5"),
        Workspace,
    ),
    item(
        "kassiber:navigate:source-funds",
        VIEW,
        Text::Key("chrome:routeMeta.sourceOfFunds.title"),
        Some("cmd+6"),
        Workspace,
    ),
    sep(VIEW),
    item(
        "kassiber:navigate:journals",
        VIEW,
        Text::Key("nav:book.ledger"),
        Some("cmd+7"),
        Workspace,
    ),
    item(
        "kassiber:navigate:review",
        VIEW,
        Text::Key("nav:book.review"),
        Some("cmd+8"),
        Workspace,
    ),
    item(
        "kassiber:navigate:assistant",
        VIEW,
        Text::Key("nav:book.assistant"),
        Some("cmd+9"),
        Assistant,
    ),
    sep(VIEW),
    item(
        "kassiber:toggle-sensitive",
        VIEW,
        Text::Own("Cycle Privacy Mode", "Privatsphäre-Modus wechseln"),
        Some("cmd+shift+h"),
        Always,
    ),
    sep(VIEW),
    item(
        "kassiber:ui-scale:decrease",
        VIEW,
        Text::Own("Smaller UI", "Kleinere Oberfläche"),
        Some("cmd+-"),
        Always,
    ),
    item(
        "kassiber:ui-scale:increase",
        VIEW,
        Text::Own("Larger UI", "Größere Oberfläche"),
        Some("cmd+="),
        Always,
    ),
    item(
        "kassiber:ui-scale:reset",
        VIEW,
        Text::Own("Default UI Scale", "Standardgröße der Oberfläche"),
        Some("cmd+0"),
        Always,
    ),
    sep(VIEW),
    item(
        FULLSCREEN,
        VIEW,
        Text::Own("Toggle Full Screen", "Vollbild ein/aus"),
        Some("f11"),
        Always,
    ),
    // Workflows
    item(
        "kassiber:workflow:add-wallet",
        FLOWS,
        Text::Own(
            "Add Wallet Connection...",
            "Wallet-Verbindung hinzufügen...",
        ),
        Some("cmd+shift+a"),
        Always,
    ),
    sep(FLOWS),
    item(
        "kassiber:workflow:sync-all",
        FLOWS,
        Text::Own("Sync All Wallets", "Alle Wallets synchronisieren"),
        Some("cmd+r"),
        Workspace,
    ),
    item(
        "kassiber:workflow:process-journals",
        FLOWS,
        Text::Own("Process Journals", "Journale verarbeiten"),
        Some("cmd+shift+j"),
        Workspace,
    ),
    sep(FLOWS),
    item(
        "kassiber:workflow:open-reports",
        FLOWS,
        Text::Own("Reports & Export...", "Berichte & Export..."),
        Some("cmd+shift+e"),
        Workspace,
    ),
    item(
        "kassiber:workflow:connections-imports",
        FLOWS,
        Text::Own("Connections & Imports...", "Verbindungen & Importe..."),
        None,
        Workspace,
    ),
    item(
        "kassiber:workflow:data-backup",
        FLOWS,
        Text::Own("Local Data & Backup...", "Lokale Daten & Backup..."),
        None,
        Workspace,
    ),
    // Settings
    item(
        "kassiber:settings:general",
        SETTINGS,
        Text::Key("settings:nav.group.general"),
        None,
        Always,
    ),
    sep(SETTINGS),
    item(
        "kassiber:settings:privacy",
        SETTINGS,
        Text::Key("settings:nav.section.privacy.label"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:display",
        SETTINGS,
        Text::Key("chrome:header.display"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:security",
        SETTINGS,
        Text::Key("onboarding:steps.security"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:backends",
        SETTINGS,
        Text::Own("Sync Backends", "Sync-Backends"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:ai",
        SETTINGS,
        Text::Key("nav:book.assistant"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:data",
        SETTINGS,
        Text::Own("Local Data", "Lokale Daten"),
        None,
        Always,
    ),
    // Window: Rune's Minimize (⌘M), then Zoom · — · Bring Main Window to Front. Rune's Close
    // stays below them (Rune can't drop a standard item); File › Close Window takes its ⌘W.
    Entry {
        after: Some("performMiniaturize:"),
        ..item(ZOOM, WINDOW, Text::Own("Zoom", "Zoomen"), None, Always)
    },
    Entry {
        after: Some(ZOOM),
        ..sep(WINDOW)
    },
    Entry {
        after: Some(ZOOM),
        ..item(
            FOCUS,
            WINDOW,
            Text::Own(
                "Bring Main Window to Front",
                "Hauptfenster nach vorne bringen",
            ),
            None,
            Always,
        )
    },
    // Help
    item(
        HELP_DOCS,
        HELP,
        Text::Own("Kassiber Documentation", "Kassiber-Dokumentation"),
        None,
        Always,
    ),
    item(
        "kassiber:navigate:logs",
        HELP,
        Text::Key("nav:book.logs"),
        None,
        Logs,
    ),
    sep(HELP),
    item(
        HELP_ISSUES,
        HELP,
        Text::Own("Report an Issue", "Problem melden"),
        None,
        Always,
    ),
];

/// Windows and Linux: Tauri's layout there (Settings, Lock, Close and Quit in File; Minimize in
/// Window). Rune shows no menu bar on these platforms; the table still drives [`on_key`] and
/// headless menu scripts.
#[cfg(not(target_os = "macos"))]
pub static ENTRIES: &[Entry] = &[
    item(
        OPEN_SETTINGS,
        FILE,
        Text::Own("Settings...", "Einstellungen..."),
        Some("cmd+,"),
        Always,
    ),
    sep(FILE),
    item(
        LOCK,
        FILE,
        Text::Key("chrome:shell.lockKassiber"),
        Some("cmd+l"),
        Workspace,
    ),
    sep(FILE),
    item(
        CLOSE,
        FILE,
        Text::Own("Close Window", "Fenster schließen"),
        Some("cmd+w"),
        Always,
    ),
    sep(FILE),
    item(
        QUIT,
        FILE,
        Text::Own("Quit Kassiber", "Kassiber beenden"),
        Some("cmd+q"),
        Always,
    ),
    item(
        "kassiber:navigate:overview",
        VIEW,
        Text::Key("nav:book.overview"),
        Some("cmd+1"),
        Workspace,
    ),
    item(
        "kassiber:navigate:transactions",
        VIEW,
        Text::Key("nav:book.transactions"),
        Some("cmd+2"),
        Workspace,
    ),
    item(
        "kassiber:navigate:connections",
        VIEW,
        Text::Own("Connections", "Verbindungen"),
        Some("cmd+3"),
        Workspace,
    ),
    item(
        "kassiber:navigate:books",
        VIEW,
        Text::Key("chrome:routeMeta.books.title"),
        Some("cmd+4"),
        Workspace,
    ),
    item(
        "kassiber:navigate:reports",
        VIEW,
        Text::Key("nav:book.reports"),
        Some("cmd+5"),
        Workspace,
    ),
    item(
        "kassiber:navigate:source-funds",
        VIEW,
        Text::Key("chrome:routeMeta.sourceOfFunds.title"),
        Some("cmd+6"),
        Workspace,
    ),
    sep(VIEW),
    item(
        "kassiber:navigate:journals",
        VIEW,
        Text::Key("nav:book.ledger"),
        Some("cmd+7"),
        Workspace,
    ),
    item(
        "kassiber:navigate:review",
        VIEW,
        Text::Key("nav:book.review"),
        Some("cmd+8"),
        Workspace,
    ),
    item(
        "kassiber:navigate:assistant",
        VIEW,
        Text::Key("nav:book.assistant"),
        Some("cmd+9"),
        Assistant,
    ),
    sep(VIEW),
    item(
        "kassiber:toggle-sensitive",
        VIEW,
        Text::Own("Cycle Privacy Mode", "Privatsphäre-Modus wechseln"),
        Some("cmd+shift+h"),
        Always,
    ),
    sep(VIEW),
    item(
        "kassiber:ui-scale:decrease",
        VIEW,
        Text::Own("Smaller UI", "Kleinere Oberfläche"),
        Some("cmd+-"),
        Always,
    ),
    item(
        "kassiber:ui-scale:increase",
        VIEW,
        Text::Own("Larger UI", "Größere Oberfläche"),
        Some("cmd+="),
        Always,
    ),
    item(
        "kassiber:ui-scale:reset",
        VIEW,
        Text::Own("Default UI Scale", "Standardgröße der Oberfläche"),
        Some("cmd+0"),
        Always,
    ),
    sep(VIEW),
    item(
        FULLSCREEN,
        VIEW,
        Text::Own("Toggle Full Screen", "Vollbild ein/aus"),
        Some("f11"),
        Always,
    ),
    item(
        "kassiber:workflow:add-wallet",
        FLOWS,
        Text::Own(
            "Add Wallet Connection...",
            "Wallet-Verbindung hinzufügen...",
        ),
        Some("cmd+shift+a"),
        Always,
    ),
    sep(FLOWS),
    item(
        "kassiber:workflow:sync-all",
        FLOWS,
        Text::Own("Sync All Wallets", "Alle Wallets synchronisieren"),
        Some("cmd+r"),
        Workspace,
    ),
    item(
        "kassiber:workflow:process-journals",
        FLOWS,
        Text::Own("Process Journals", "Journale verarbeiten"),
        Some("cmd+shift+j"),
        Workspace,
    ),
    sep(FLOWS),
    item(
        "kassiber:workflow:open-reports",
        FLOWS,
        Text::Own("Reports & Export...", "Berichte & Export..."),
        Some("cmd+shift+e"),
        Workspace,
    ),
    item(
        "kassiber:workflow:connections-imports",
        FLOWS,
        Text::Own("Connections & Imports...", "Verbindungen & Importe..."),
        None,
        Workspace,
    ),
    item(
        "kassiber:workflow:data-backup",
        FLOWS,
        Text::Own("Local Data & Backup...", "Lokale Daten & Backup..."),
        None,
        Workspace,
    ),
    item(
        "kassiber:settings:general",
        SETTINGS,
        Text::Key("settings:nav.group.general"),
        None,
        Always,
    ),
    sep(SETTINGS),
    item(
        "kassiber:settings:privacy",
        SETTINGS,
        Text::Key("settings:nav.section.privacy.label"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:display",
        SETTINGS,
        Text::Key("chrome:header.display"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:security",
        SETTINGS,
        Text::Key("onboarding:steps.security"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:backends",
        SETTINGS,
        Text::Own("Sync Backends", "Sync-Backends"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:ai",
        SETTINGS,
        Text::Key("nav:book.assistant"),
        None,
        Always,
    ),
    item(
        "kassiber:settings:data",
        SETTINGS,
        Text::Own("Local Data", "Lokale Daten"),
        None,
        Always,
    ),
    item(
        MINIMIZE,
        WINDOW,
        Text::Own("Minimize", "Minimieren"),
        Some("cmd+m"),
        Always,
    ),
    item(ZOOM, WINDOW, Text::Own("Zoom", "Zoomen"), None, Always),
    sep(WINDOW),
    item(
        FOCUS,
        WINDOW,
        Text::Own(
            "Bring Main Window to Front",
            "Hauptfenster nach vorne bringen",
        ),
        None,
        Always,
    ),
    item(
        HELP_DOCS,
        HELP,
        Text::Own("Kassiber Documentation", "Kassiber-Dokumentation"),
        None,
        Always,
    ),
    item(
        "kassiber:navigate:logs",
        HELP,
        Text::Key("nav:book.logs"),
        None,
        Logs,
    ),
    sep(HELP),
    item(
        HELP_ISSUES,
        HELP,
        Text::Own("Report an Issue", "Problem melden"),
        None,
        Always,
    ),
];

/// The names of the top-level menus and of Rune's own items, in the app's language
/// (`rune::set_labels`). English keeps Rune's (and AppKit's) defaults.
pub fn rune_label(label: rune::Label<'_>, lang: Lang) -> Option<String> {
    use rune::Label as L;
    if lang != Lang::De {
        return None;
    }
    let s = match label {
        L::Menu("File") => "Ablage",
        L::Menu("Edit") => "Bearbeiten",
        L::Menu("View") => "Darstellung",
        L::Menu("Workflows") => "Abläufe",
        L::Menu("Settings") => "Einstellungen",
        L::Menu("Window") => "Fenster",
        L::Menu("Help") => "Hilfe",
        L::Undo => "Widerrufen",
        L::Redo => "Wiederholen",
        L::Cut => "Ausschneiden",
        L::Copy => "Kopieren",
        L::Paste => "Einsetzen",
        L::SelectAll => "Alles auswählen",
        L::Minimize => "Im Dock ablegen",
        L::Close => "Schließen",
        L::About(name) => return Some(format!("Über {name}")),
        L::Services => "Dienste",
        L::Hide(name) => return Some(format!("{name} ausblenden")),
        L::HideOthers => "Andere ausblenden",
        L::ShowAll => "Alle einblenden",
        L::Quit(name) => return Some(format!("{name} beenden")),
        _ => return None,
    };
    Some(s.to_owned())
}

/// What `set_menu_state` is computed from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub ai_features: bool,
    pub developer_tools: bool,
    pub has_workspace: bool,
    pub locked: bool,
    pub lang: Lang,
}

impl MenuState {
    pub fn of(app: &App) -> MenuState {
        MenuState {
            ai_features: app.prefs.ai_features,
            developer_tools: app.prefs.developer_tools,
            has_workspace: has_workspace(app),
            locked: app.locked,
            lang: app.prefs.lang,
        }
    }

    /// `set_menu_state`: while locked the daemon refuses mutating calls, so the workflow items
    /// go grey with the no-workspace case.
    pub fn enabled(&self, gate: Gate) -> bool {
        let workflows = self.has_workspace && !self.locked;
        match gate {
            Gate::Always => true,
            Gate::Workspace => workflows,
            Gate::Assistant => self.ai_features && workflows,
            Gate::Logs => self.developer_tools && workflows,
        }
    }
}

/// `menu_action_for_id`: the items that become an intent for the app.
pub fn intent_for_id(id: &str) -> Option<Intent> {
    use Intent::*;
    Some(match id {
        CHECK_UPDATES => Action("check-for-updates"),
        OPEN_SETTINGS | "kassiber:settings:general" => OpenSettings(None),
        "kassiber:settings:privacy" => OpenSettings(Some("privacy")),
        "kassiber:settings:display" => OpenSettings(Some("display")),
        "kassiber:settings:security" => OpenSettings(Some("security")),
        "kassiber:settings:backends" => OpenSettings(Some("backends")),
        "kassiber:settings:ai" => OpenSettings(Some("ai")),
        "kassiber:settings:data" => OpenSettings(Some("data")),
        LOCK => Action("lock-app"),
        "kassiber:toggle-sensitive" => Action("toggle-sensitive"),
        "kassiber:ui-scale:decrease" => Action("ui-scale-decrease"),
        "kassiber:ui-scale:increase" => Action("ui-scale-increase"),
        "kassiber:ui-scale:reset" => Action("ui-scale-reset"),
        "kassiber:workflow:add-wallet" => Action("add-wallet"),
        "kassiber:workflow:sync-all" => Action("sync-all-wallets"),
        "kassiber:workflow:process-journals" => Action("process-journals"),
        "kassiber:workflow:open-reports" => Navigate("/reports"),
        "kassiber:workflow:connections-imports" => Navigate("/connections"),
        "kassiber:workflow:data-backup" => OpenSettings(Some("data")),
        "kassiber:navigate:overview" => Navigate("/overview"),
        "kassiber:navigate:transactions" => Navigate("/transactions"),
        "kassiber:navigate:connections" => Navigate("/connections"),
        "kassiber:navigate:books" => Navigate("/books"),
        "kassiber:navigate:reports" => Navigate("/reports"),
        "kassiber:navigate:source-funds" => Navigate("/source-of-funds"),
        "kassiber:navigate:journals" => Navigate("/journals"),
        "kassiber:navigate:review" => Navigate("/review"),
        "kassiber:navigate:assistant" => Navigate("/assistant"),
        "kassiber:navigate:logs" => Navigate("/logs"),
        _ => return None,
    })
}

/// `handle_app_menu_event`: an item was picked (or its accelerator pressed).
pub fn fire(app: &mut App, tx: &mut Tx, id: &str) {
    if let Some(intent) = intent_for_id(id) {
        dispatch(app, &intent);
        return;
    }
    match id {
        FULLSCREEN => tx.toggle_full_screen(),
        CLOSE => tx.close_window(),
        MINIMIZE => tx.minimize(),
        ZOOM => tx.toggle_maximized(),
        FOCUS => super::bring_to_front(),
        QUIT => tx.quit(),
        // The user picked a help link: the browser opens the fixed GitHub page.
        HELP_DOCS => drop(tx.open_url(DOCS_URL)),
        HELP_ISSUES => drop(tx.open_url(ISSUES_URL)),
        _ => {}
    }
}

/// One action per item: Rune menu actions are plain functions, so each index gets its own.
fn action<const I: usize>(app: &mut App, tx: &mut Tx) {
    fire(app, tx, ENTRIES[I].id);
}

macro_rules! actions {
    ($($i:literal)*) => { &[$(action::<$i>),*] };
}
static ACTIONS: &[fn(&mut App, &mut Tx)] = actions!(
    0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20 21 22 23 24 25 26 27 28 29 30 31 32 33 34 35 36
    37 38 39 40 41 42 43 44 45 46 47 48 49 50 51 52 53 54 55 56 57 58 59
);

/// The items for the current state.
pub fn items(state: &MenuState) -> Vec<MenuItem<App>> {
    crate::i18n::set_lang(state.lang);
    ENTRIES
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let mut m = if e.id.is_empty() {
                MenuItem::separator()
            } else {
                MenuItem::new(e.text.get(state.lang), ACTIONS[i]).id(e.id)
            };
            if let Place::Menu(name) = e.place {
                m = m.menu(name);
            }
            if let Some(a) = e.after {
                m = m.after(a);
            }
            if let Some(s) = e.accel.and_then(|a| a.parse::<Shortcut>().ok()) {
                // The window sees the key first; the item runs when nothing took it.
                m = m.key_equivalent(s).keys_first();
            }
            if !e.id.is_empty() {
                m = m.enabled(state.enabled(e.gate));
            }
            m
        })
        .collect()
}

/// The accelerator a key press is, in `Entry::accel` form.
fn accel_of(ev: &KeyEvent) -> Option<String> {
    let m = ev.mods;
    match &ev.key {
        Key::Command(c) if !m.alt => {
            let c = match c {
                '+' => '=',
                '_' => '-',
                c => *c,
            };
            Some(if m.shift {
                format!("cmd+shift+{c}")
            } else {
                format!("cmd+{c}")
            })
        }
        Key::Function(n) if !m.cmd && !m.alt && !m.shift && !m.ctrl => Some(format!("f{n}")),
        _ => None,
    }
}

/// A menu accelerator no handler before took (the end of the app's key chain): the item fires
/// if it's enabled now, as its menu item would. On macOS this is also what runs for a
/// `keys_first` item when the window didn't take its key, so it never runs twice.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(accel) = accel_of(ev) else {
        return false;
    };
    let state = MenuState::of(app);
    let Some(e) = ENTRIES
        .iter()
        .find(|e| !e.id.is_empty() && e.accel == Some(accel.as_str()))
    else {
        return false;
    };
    if state.enabled(e.gate) {
        fire(app, tx, e.id);
    }
    // A disabled item still owns its accelerator: nothing else runs for it.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_has_an_action_and_parses() {
        assert!(ENTRIES.len() <= ACTIONS.len());
        let mut ids = std::collections::HashSet::new();
        for e in ENTRIES.iter().filter(|e| !e.id.is_empty()) {
            assert!(ids.insert(e.id), "duplicate {}", e.id);
            if let Some(a) = e.accel {
                assert!(a.parse::<Shortcut>().is_ok(), "{a}");
            }
            // Every item either becomes an intent or is a window/help action.
            assert!(
                intent_for_id(e.id).is_some()
                    || [
                        FULLSCREEN,
                        CLOSE,
                        MINIMIZE,
                        ZOOM,
                        FOCUS,
                        QUIT,
                        HELP_DOCS,
                        HELP_ISSUES
                    ]
                    .contains(&e.id),
                "{}",
                e.id
            );
        }
    }

    #[test]
    fn labels_resolve_in_both_languages() {
        for lang in [Lang::En, Lang::De] {
            crate::i18n::set_lang(lang);
            for e in ENTRIES.iter().filter(|e| !e.id.is_empty()) {
                if let Text::Key(k) = e.text {
                    assert!(crate::i18n::has(k), "{k}");
                }
                assert!(!e.text.get(lang).is_empty());
            }
        }
        // English is Tauri's exactly.
        crate::i18n::set_lang(Lang::En);
        let en: Vec<String> = ENTRIES
            .iter()
            .filter(|e| !e.id.is_empty())
            .map(|e| e.text.get(Lang::En))
            .collect();
        for want in [
            "Lock Kassiber",
            "Overview",
            "Transactions",
            "Connections",
            "Books",
            "Reports",
            "Source of Funds",
            "Journals",
            "Review",
            "Assistant",
            "Logs",
            "General",
            "Privacy",
            "Display",
            "Security",
            "Sync Backends",
            "Local Data",
        ] {
            assert!(en.iter().any(|l| l == want), "{want}");
        }
    }

    /// `menu_action_for_id`, as src-tauri's own test pins it.
    #[test]
    fn menu_ids_map_to_tauri_actions() {
        assert_eq!(
            intent_for_id("kassiber:workflow:open-reports"),
            Some(Intent::Navigate("/reports"))
        );
        assert_eq!(
            intent_for_id("kassiber:workflow:connections-imports"),
            Some(Intent::Navigate("/connections"))
        );
        assert_eq!(
            intent_for_id("kassiber:workflow:data-backup"),
            Some(Intent::OpenSettings(Some("data")))
        );
        assert_eq!(
            intent_for_id(OPEN_SETTINGS),
            Some(Intent::OpenSettings(None))
        );
        assert_eq!(intent_for_id(FULLSCREEN), None);
        assert_eq!(intent_for_id(HELP_DOCS), None);
    }

    #[test]
    fn menu_state_matches_set_menu_state() {
        let s = |ai, dev, ws, locked| MenuState {
            ai_features: ai,
            developer_tools: dev,
            has_workspace: ws,
            locked,
            lang: Lang::En,
        };
        assert!(s(false, false, false, false).enabled(Gate::Always));
        assert!(!s(true, true, false, false).enabled(Gate::Workspace));
        assert!(!s(true, true, true, true).enabled(Gate::Workspace));
        assert!(s(false, false, true, false).enabled(Gate::Workspace));
        assert!(!s(false, true, true, false).enabled(Gate::Assistant));
        assert!(s(true, false, true, false).enabled(Gate::Assistant));
        assert!(!s(true, false, true, false).enabled(Gate::Logs));
        assert!(s(false, true, true, false).enabled(Gate::Logs));
        assert!(!s(true, true, true, true).enabled(Gate::Logs));
    }

    #[test]
    fn accelerators_map_from_key_events() {
        let ev = |key: Key, shift: bool| KeyEvent {
            key,
            mods: rune::Mods {
                cmd: true,
                shift,
                ..Default::default()
            },
        };
        assert_eq!(
            accel_of(&ev(Key::Command('6'), false)).as_deref(),
            Some("cmd+6")
        );
        assert_eq!(
            accel_of(&ev(Key::Command('e'), true)).as_deref(),
            Some("cmd+shift+e")
        );
        assert_eq!(
            accel_of(&ev(Key::Command('+'), false)).as_deref(),
            Some("cmd+=")
        );
        let f11 = KeyEvent {
            key: Key::Function(11),
            mods: rune::Mods::default(),
        };
        assert_eq!(accel_of(&f11).as_deref(), Some("f11"));
    }
}
