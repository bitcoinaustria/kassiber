//! Wallets: the connections list (`/connections`), a connection's detail (`/connections/<id>`,
//! the node variant for Lightning nodes), imports (`/imports`) and the Add connection flow.
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies) through the
//! functions below. Submodules:
//! - `conn`: the connection record and lib/connectionDisplay.ts helpers.
//! - `common`: status pill, asset badge, logos, headless pump helpers.
//! - `list`: routes/Connections.tsx (+ components/kb/wallets/Wallets*).
//! - `imports`: routes/Imports.tsx (+ shadcnblocks/settings-integrations4).
//! - `detail`: routes/ConnectionDetail.tsx (wallet detail and its panels).
//! - `node`: routes/NodeConnectionDetail.tsx (+ ConnectionDetail's NodeConnectionContainer).
//! - `add`: components/kb/AddConnectionDialog.tsx (both steps, every setup form);
//!   `add::btcpay` and `add::exchange` the BTCPay / exchange setup panels and ExchangeSyncCard.
//! - `assets`: components/kb/assets/* (the asset scope notice of the exchange card and of an
//!   import result).
//! - `reimport`: an import's counts and kept values, the changed-rows refusal and its answers
//!   (Rust only).
//! - `asset_choice`: the asset question before a new provider wallet's import (Rust only).

pub mod add;
pub mod asset_choice;
pub mod assets;
pub mod common;
pub mod conn;
pub mod detail;
pub mod imports;
pub mod list;
pub mod node;
pub mod reimport;

use rune::{Cx, El, KeyEvent, Tx};

use crate::app::{App, Reply};
use crate::theme::Tk;

/// This area's UI state (`app.wallets`).
#[derive(Default)]
pub struct Ui {
    pub list: list::State,
    pub imports: imports::State,
    pub detail: detail::State,
    pub node: node::State,
    pub add: add::State,
    pub assets: assets::State,
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    path == "/connections" || path.starts_with("/connections/") || path == "/imports"
}

/// The connection id of a `/connections/<id>` route.
pub fn route_id(app: &App) -> Option<String> {
    app.route
        .path()
        .strip_prefix("/connections/")
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('/').to_owned())
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let path = app.route.path();
    let page = if path == "/imports" {
        imports::view(app, cx, tk, layer)
    } else if let Some(id) = route_id(app) {
        match conn::snapshot(app) {
            None => detail::skeleton(tk),
            Some(_) => match conn::find(app, &id) {
                None => detail::not_found(app, tk, &id),
                Some(c) if c.is_node() => node::view(app, cx, tk, layer, &c),
                Some(c) => detail::view(app, cx, tk, layer, &c),
            },
        }
    } else if app.wallets.add.exchange.card_scene {
        add::exchange::card_scene_view(app, cx, tk)
    } else {
        list::view(app, cx, tk, layer)
    };
    // The Add connection dialog mounts over every Wallets route (the Overview mounts its own).
    if app.ov.add_open {
        add::dialog(app, cx, tk, layer);
    }
    page
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = add::poll(app, tx);
    if !owns(app.route.path()) {
        return changed;
    }
    changed |= list::poll(app, tx);
    changed |= imports::poll(app, tx);
    changed |= detail::poll(app, tx);
    changed |= node::poll(app, tx);
    changed
}

/// Leaving one of this area's pages (`crate::shell::on_leave`): what the route component keeps
/// in `useState` starts over; the reads (React Query's cache), the remembered sort and the
/// Add connection dialog (mounted over every Wallets route, closed by its own controls) stay.
pub fn on_leave(app: &mut App, from: &str) {
    if from == "/imports" {
        // `selectedId`, the dialog's source.
        app.wallets.imports = imports::State::default();
    } else if from == "/connections" {
        // `kindFilter`, `statusFilter`, open menus.
        let l = &mut app.wallets.list;
        l.kind = None;
        l.status = None;
        l.popup = None;
    } else if from.starts_with("/connections/") {
        app.wallets.detail.leave();
        app.wallets.node.leave();
        // `useExchangeSync`'s `lastError`; its status read stays cached.
        app.wallets.add.exchange.sync.last_error = None;
    }
}

/// Every daemon reply; act only on this area's own request tags (`"wallets.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("wallets.") {
        return;
    }
    list::on_reply(app, tag, r);
    imports::on_reply(app, tag, r);
    detail::on_reply(app, tag, r);
    node::on_reply(app, tag, r);
    add::on_reply(app, tag, r);
    assets::on_reply(app, tag, r);
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // The dialog is the innermost overlay wherever it is open (the Overview routes its own
    // keys through `overview::add_connection::on_key`, which lands in `add::on_key` too).
    if app.ov.add_open && app.route.path() != "/overview" && app.route.path() != "/" {
        return add::on_key(app, ev, tx);
    }
    if !owns(app.route.path()) {
        return false;
    }
    detail::on_key(app, ev, tx)
        || node::on_key(app, ev, tx)
        || list::on_key(app, ev, tx)
        || imports::on_key(app, ev, tx)
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    common::settle(app);
    let path = app.route.path().to_owned();
    if path == "/imports" {
        imports::prime(app);
    } else if let Some(id) = route_id(app) {
        match conn::find(app, &id) {
            Some(c) if c.is_node() => node::prime(app, &c),
            Some(c) => detail::prime(app, &c),
            None => {}
        }
    } else {
        list::prime(app);
    }
}

use crate::app::Scene;

/// Named states for `--scene` (headless snapshots and the parity harness); prefix names with
/// `wallets-`. Each submodule lists its own; they are joined at compile time.
pub const SCENES: &[Scene] = &join(&[
    list::SCENES,
    imports::SCENES,
    detail::SCENES,
    node::SCENES,
    add::SCENES,
    add::btcpay::SCENES,
    add::exchange::SCENES,
]);

const N_SCENES: usize = list::SCENES.len()
    + imports::SCENES.len()
    + detail::SCENES.len()
    + node::SCENES.len()
    + add::SCENES.len()
    + add::btcpay::SCENES.len()
    + add::exchange::SCENES.len();

const fn join(parts: &[&[Scene]]) -> [Scene; N_SCENES] {
    fn noop(_: &mut App) {}
    let mut out: [Scene; N_SCENES] = [("", noop as fn(&mut App)); N_SCENES];
    let (mut i, mut n) = (0, 0);
    while i < parts.len() {
        let mut j = 0;
        while j < parts[i].len() {
            out[n] = parts[i][j];
            n += 1;
            j += 1;
        }
        i += 1;
    }
    out
}

/// Navigates and primes the route (scene helper).
pub fn scene_route(app: &mut App, route: &str) {
    app.navigate(route);
    prime(app);
}

/// The id of the snapshot connection labelled `label` (scenes pick by label: ids differ
/// between book copies).
pub fn scene_id_by_label(app: &mut App, label: &str) -> Option<String> {
    common::settle(app);
    conn::all(app)
        .into_iter()
        .find(|c| c.label == label)
        .map(|c| c.id)
}
