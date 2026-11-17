//! What a native menu item or a deep link asks for (`MenuActionPayload`), and what the app does
//! with it: the web's `dispatchMenuIntent` (components/kb/menuIntent.ts) with the two listeners
//! it runs under (`RootIntentListener`'s global scope, AppShell's workspace scope) folded into
//! one. Route and developer-mode guards apply here, not only to the visible navigation: a hidden
//! link is not an access boundary (ui-tauri/AGENTS.md).

use crate::app::{App, Privacy};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// `lock-app`, `toggle-sensitive`, `check-for-updates`, `ui-scale-{decrease,increase,reset}`,
    /// `add-wallet`, `sync-all-wallets`, `process-journals`.
    Action(&'static str),
    /// `open-settings` with an allowlisted section slug (or none).
    OpenSettings(Option<&'static str>),
    /// `navigate` to an app route.
    Navigate(&'static str),
}

/// `APP_ROUTE_PATHS`.
pub const APP_ROUTE_PATHS: &[&str] = &[
    "/overview",
    "/transactions",
    "/activity",
    "/reports",
    "/privacy-mirror",
    "/chain-analysis",
    "/exit-tax",
    "/source-of-funds",
    "/egress",
    "/connections",
    "/books",
    "/journals",
    "/transfers",
    "/swaps",
    "/custody-gaps",
    "/review",
    "/quarantine",
    "/reconcile",
    "/logs",
    "/settings",
    "/assistant",
];

/// `GLOBAL_ACTIONS`: reachable before a workspace exists (RootIntentListener).
fn global(intent: &Intent) -> bool {
    match intent {
        Intent::Navigate(_) | Intent::OpenSettings(_) => true,
        Intent::Action(a) => matches!(
            *a,
            "toggle-sensitive"
                | "ui-scale-decrease"
                | "ui-scale-increase"
                | "ui-scale-reset"
                | "check-for-updates"
        ),
    }
}

/// `identity !== null`: a book is open (the profiles read names its active profile).
pub fn has_workspace(app: &App) -> bool {
    app.read("ui.profiles.snapshot")
        .and_then(|p| p.get("activeProfileId"))
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty())
}

/// `isDevOnlyRoute`: `DEV_LOCKED_ROUTES` + `DEV_HIDDEN_ROUTES`.
pub fn dev_only_route(route: &str) -> bool {
    crate::shell::DEV_LOCKED.contains(&route) || crate::shell::DEV_HIDDEN.contains(&route)
}

/// `isDevOnlySettingsSlug`: the slug resolved through the canonical section map.
pub fn dev_only_settings_slug(slug: &str) -> bool {
    crate::settings::section_for_slug(slug).is_some_and(crate::settings::dev_only)
}

/// `cyclePrivacyMode`: Normal → Hide amounts → Hide all → Normal.
pub fn cycle_privacy(app: &mut App) {
    app.prefs.privacy = match app.prefs.privacy {
        Privacy::Normal => Privacy::HideAmounts,
        Privacy::HideAmounts => Privacy::HideAll,
        Privacy::HideAll => Privacy::Normal,
    };
}

/// The store's scale steps (5 %, 80–120 %), as the shell's ⌘-/⌘=/⌘0 apply them.
fn set_scale(app: &mut App, scale: f32) {
    app.prefs.scale = ((scale * 20.0).round() / 20.0).clamp(0.8, 1.2);
}

/// `openAddWalletConnection`: remember the descriptor setup for the Connections screen (its
/// poll reopens it there) and go to it. The caller has checked the workspace.
pub fn add_wallet(app: &mut App) {
    app.wallets.add.deferred = Some("descriptor");
    app.navigate("/connections");
}

/// `runMenuWalletSync`: already running brings the running job back into view (the sync card);
/// otherwise the book refresh starts. Network egress: only ever from the user's menu press.
fn wallet_sync(app: &mut App) {
    if app.busy.syncing {
        app.shell.sync_card_open = true;
        return;
    }
    app.refresh_book(false);
}

/// `revealSettingAnchor(anchor)` + `navigate({to: "/settings", hash: section})`.
fn reveal(app: &mut App, section: &str, anchor: &str) {
    crate::settings::open_section(app, &format!("{section}#{anchor}"));
}

/// `dispatchMenuIntent` over both scopes.
pub fn dispatch(app: &mut App, intent: &Intent) {
    let has_workspace = has_workspace(app);
    // AppShell's listener (workspace actions) only acts once a book is open: its runners call
    // `ensureWorkspaceForMenuAction`, which sends a book-less app back to the welcome screen.
    if !global(intent) && !has_workspace {
        return;
    }
    match intent {
        Intent::Action("lock-app") => crate::shell::overlays::lock(app),
        Intent::Action("toggle-sensitive") => cycle_privacy(app),
        Intent::Action("ui-scale-decrease") => set_scale(app, app.prefs.scale - 0.05),
        Intent::Action("ui-scale-increase") => set_scale(app, app.prefs.scale + 0.05),
        Intent::Action("ui-scale-reset") => set_scale(app, crate::app::DEFAULT_SCALE),
        Intent::Action("check-for-updates") => super::update::run_manual(app),
        Intent::Action("add-wallet") => add_wallet(app),
        Intent::Action("sync-all-wallets") => wallet_sync(app),
        Intent::Action("process-journals") => app.process_journals(),
        Intent::Action(_) => {}
        Intent::OpenSettings(section) => {
            // `lightning` resolves to a section hidden while early-stage features are off: point
            // at the switch instead of bouncing to the Overview.
            if !app.prefs.developer_tools && section.is_some_and(dev_only_settings_slug) {
                reveal(app, "developer", "earlyFeatures");
                return;
            }
            crate::settings::open_section(app, section.unwrap_or(""));
        }
        Intent::Navigate(route) => {
            if !APP_ROUTE_PATHS.contains(route) {
                return;
            }
            if *route == "/assistant" && !app.prefs.ai_features {
                reveal(app, "ai", "assistantFeatures");
                return;
            }
            // Book-less: nowhere to land (the identity guard would bounce it straight back).
            if !has_workspace {
                return;
            }
            if !app.prefs.developer_tools && dev_only_route(route) {
                reveal(app, "developer", "earlyFeatures");
                return;
            }
            app.navigate(route);
        }
    }
}
