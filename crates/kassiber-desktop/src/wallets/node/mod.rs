//! routes/NodeConnectionDetail.tsx and ConnectionDetail.tsx's `NodeConnectionContainer`: the
//! detail page of a Lightning node connection (`isNodeConnection`: the connection declares the
//! `nodeSnapshot` capability).
//!
//! The node snapshot (`ui.connections.node.snapshot {connection}`) is fetched from the node
//! itself, so, as on the web (`useDaemon(..., { enabled: false })`), it is requested only when
//! the user presses Refresh. Nothing on mount: the page shows the connection's own `node`
//! (when the overview snapshot carries one) or the "No node snapshot yet" card.
//!
//! `model` parses the snapshot; `view` draws the page, its tabs and the channel sheet.

mod model;
mod view;

use rune::{Cx, El, Key, KeyEvent, Tx};
use serde_json::{Value, json};

use super::conn::Conn;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::tc;
use crate::theme::Tk;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Overview,
    Channels,
    Activity,
    Profitability,
    Accounting,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Overview,
        Tab::Channels,
        Tab::Activity,
        Tab::Profitability,
        Tab::Accounting,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Tab::Overview => "overview",
            Tab::Channels => "channels",
            Tab::Activity => "activity",
            Tab::Profitability => "profitability",
            Tab::Accounting => "accounting",
        }
    }
}

#[derive(Default)]
pub struct State {
    /// The connection this state belongs to; another connection starts fresh.
    pub conn_id: Option<String>,
    /// The latest snapshot a Refresh returned (`nodeSnapshotQuery.data`).
    pub live: Option<Value>,
    /// A Refresh is in flight (`isFetching`).
    pub fetching: bool,
    /// The last Refresh's error (`nodeSnapshotQuery.error.message`).
    pub error: Option<String>,
    pub tab: Tab,
    /// "Show closed" (None: the default, on when only closed channels are left).
    pub show_closed: Option<bool>,
    /// The channel whose detail sheet is open.
    pub open_channel: Option<String>,
    /// The copy button showing "Copied" (key, until app clock).
    pub copied: Option<(String, f64)>,
}

/// The connection with its live snapshot merged in (`resolvedConnection.node`).
fn node_value<'a>(app: &'a App, c: &'a Conn) -> Option<&'a Value> {
    app.wallets.node.live.as_ref().or(c.node.as_ref())
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, c: &Conn) -> El {
    view::page(
        app,
        cx,
        tk,
        layer,
        c,
        node_value(app, c).map(model::Node::parse),
    )
}

impl State {
    /// Leaving the node's page: the tab, "Show closed", the open channel sheet and the copy
    /// check are `useState`s; the last Refresh's snapshot or error is the query's (kept).
    pub fn leave(&mut self) {
        let k = std::mem::take(self);
        *self = State {
            conn_id: k.conn_id,
            live: k.live,
            fetching: k.fetching,
            error: k.error,
            ..State::default()
        };
    }
}

/// Keeps the state bound to the shown connection; expires the "Copied" label.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(id) = super::route_id(app) else {
        return false;
    };
    let mut changed = false;
    if app.wallets.node.conn_id.as_deref() != Some(id.as_str()) {
        app.wallets.node = State {
            conn_id: Some(id),
            ..State::default()
        };
        changed = true;
    }
    if let Some((_, until)) = app.wallets.node.copied {
        if app.clock >= until {
            app.wallets.node.copied = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (until - app.clock).max(0.05),
            ));
        }
    }
    changed
}

/// `onSync`: the page's Refresh buttons. Contacts the Lightning node (egress): only ever called
/// from a press. A running refresh already shows on the page, so a second press is ignored.
pub fn sync(app: &mut App, id: &str) {
    if app.wallets.node.fetching {
        return;
    }
    if app
        .send(
            "ui.connections.node.snapshot",
            json!({ "connection": id }),
            "wallets.node.snapshot",
        )
        .is_some()
    {
        app.wallets.node.fetching = true;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if tag != "wallets.node.snapshot" {
        return;
    }
    app.wallets.node.fetching = false;
    let label = app
        .wallets
        .node
        .conn_id
        .clone()
        .and_then(|id| super::conn::find(app, &id))
        .map(|c| c.label)
        .unwrap_or_default();
    match &r.error {
        Some(e) => {
            app.wallets.node.error = Some(e.clone());
            // `recordConnectionSyncFailed`: a condition in the centre and an error toast.
            let title = tc(
                "chrome:notifications.condition.connectionSyncFailed.title",
                1,
                &[],
            );
            let at = app.clock;
            app.shell
                .note(&title, Some(label.clone()), ToastTone::Error, at);
            app.toast(
                ToastTone::Error,
                &title,
                Some(label),
                Some("wallets-node-sync"),
            );
        }
        None => {
            app.wallets.node.error = None;
            if r.data.is_object() {
                app.wallets.node.live = Some(r.data.clone());
            }
            // `recordConnectionSynced`.
            let title = crate::i18n::t("chrome:notifications.result.connectionSynced.title");
            let at = app.clock;
            app.shell
                .note(&title, Some(label.clone()), ToastTone::Success, at);
            app.toast(
                ToastTone::Success,
                &title,
                Some(label),
                Some("wallets-node-sync"),
            );
        }
    }
}

/// Escape closes the channel sheet (the innermost overlay).
pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if app.wallets.node.open_channel.is_some() && matches!(ev.key, Key::Escape) {
        app.wallets.node.open_channel = None;
        return true;
    }
    false
}

/// Headless: nothing to load (the snapshot waits for a press); bind the state.
pub fn prime(app: &mut App, c: &Conn) {
    if app.wallets.node.conn_id.as_deref() != Some(c.id.as_str()) {
        app.wallets.node = State {
            conn_id: Some(c.id.clone()),
            ..State::default()
        };
    }
}

// ---- Scenes ----------------------------------------------------------------------------------

/// The seeded "Regtest Lightning Node" (forks/node/seed-node.sh) on the current book.
fn open_node(a: &mut App) {
    if let Some(id) = super::scene_id_by_label(a, "Regtest Lightning Node") {
        super::scene_route(a, &format!("/connections/{id}"));
    }
}

/// SYNTHETIC: a populated snapshot (model::synthetic) as if a Refresh had returned it.
fn open_populated(a: &mut App, tab: Tab) {
    open_node(a);
    a.wallets.node.live = Some(model::synthetic());
    a.wallets.node.tab = tab;
}

pub const SCENES: &[crate::app::Scene] = &[
    // The node as the book has it: never refreshed, no snapshot.
    ("wallets-node", |a| open_node(a)),
    ("wallets-node-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_node(a);
    }),
    ("wallets-node-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_node(a);
    }),
    // A Refresh in flight on a node without a snapshot (state only: nothing is sent).
    ("wallets-node-loading", |a| {
        open_node(a);
        a.wallets.node.fetching = true;
    }),
    // SYNTHETIC: the last Refresh failed.
    ("wallets-node-error", |a| {
        open_node(a);
        a.wallets.node.error = Some("lnd backend is not configured for this wallet".into());
    }),
    ("wallets-node-populated", |a| {
        open_populated(a, Tab::Overview)
    }),
    ("wallets-node-channels", |a| {
        open_populated(a, Tab::Channels)
    }),
    ("wallets-node-channels-closed", |a| {
        open_populated(a, Tab::Channels);
        a.wallets.node.show_closed = Some(true);
    }),
    ("wallets-node-sheet", |a| {
        open_populated(a, Tab::Channels);
        a.wallets.node.open_channel = Some("ch-alpha".into());
    }),
    ("wallets-node-sheet-private", |a| {
        open_populated(a, Tab::Channels);
        a.wallets.node.open_channel = Some("ch-gamma".into());
    }),
    ("wallets-node-activity", |a| {
        open_populated(a, Tab::Activity)
    }),
    ("wallets-node-profitability", |a| {
        open_populated(a, Tab::Profitability)
    }),
    ("wallets-node-accounting", |a| {
        open_populated(a, Tab::Accounting)
    }),
    ("wallets-node-populated-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_populated(a, Tab::Overview);
    }),
    ("wallets-node-populated-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_populated(a, Tab::Overview);
    }),
    ("wallets-node-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        open_populated(a, Tab::Overview);
    }),
];
