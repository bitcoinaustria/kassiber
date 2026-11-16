//! Source of funds (`/source-of-funds`), Privacy mirror (`/privacy-mirror`) and Chain analysis (`/chain-analysis`).
//!
//! Registered in `main.rs` (view, poll, keys, scenes) and `app.rs` (state, replies) through the
//! functions below; everything else for this area lives under `src/provenance/`.
#![allow(unused_variables)]

pub mod chain;
pub mod mirror;

use rune::{Cx, El, KeyEvent, Tx};

use crate::app::{App, Reply};
use crate::theme::Tk;

pub mod sof;

/// This area's UI state (`app.provenance`).
#[derive(Default)]
pub struct Ui {
    pub chain: chain::State,
    pub mirror: mirror::State,
    /// Source of funds (`/source-of-funds`).
    pub sof: sof::Sof,
}

/// Whether this area renders `path` (the route without its query).
pub fn owns(path: &str) -> bool {
    matches!(
        path,
        "/source-of-funds" | "/privacy-mirror" | "/chain-analysis"
    )
}

/// The page for a route this area owns. Overlays (menus, dialogs, sheets) go into `layer`.
pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    match app.route.path() {
        "/chain-analysis" => chain::page::view(app, cx, tk, layer),
        mirror::ROUTE => mirror::view(app, cx, tk, layer),
        p if p == sof::ROUTE => sof::page::view(app, cx, tk, layer),
        _ => crate::ui::not_found_page(app, tk),
    }
}

/// Every frame: start the reads the current route needs (local reads only), advance timers.
/// True when anything changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = chain::poll(app, tx);
    changed |= mirror::poll(app, tx);
    changed |= sof::poll(app, tx);
    changed
}

/// Every daemon reply; act only on this area's own request tags (`"provenance.…"`).
pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    chain::on_reply(app, tag, r);
    mirror::on_reply(app, tag, r);
    sof::on_reply(app, tag, r);
}

/// Key presses before the shell sees them; true when handled.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    chain::on_key(app, ev, tx) || mirror::on_key(app, ev, tx) || sof::on_key(app, ev, tx)
}

/// Headless runs: load what the current route needs before the first frame (block on the
/// daemon, as `transactions::prime` does). Called by the `route` scene after navigating.
pub fn prime(app: &mut App) {
    match app.route.path() {
        "/chain-analysis" => chain::prime(app),
        mirror::ROUTE => mirror::prime(app),
        p if p == sof::ROUTE => sof::prime(app),
        _ => {}
    }
}

/// Named states for `--scene` (headless snapshots and the parity harness); prefix names with
/// `provenance-`.
pub const SCENES: &[crate::app::Scene] = &[
    ("provenance-chain-default", |a| {
        chain::scenes::open(a, "/chain-analysis")
    }),
    ("provenance-chain-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        chain::scenes::open(a, "/chain-analysis");
    }),
    ("provenance-chain-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        chain::scenes::open(a, "/chain-analysis");
    }),
    ("provenance-chain-tools", chain::scenes::tools),
    ("provenance-chain-node", chain::scenes::node),
    ("provenance-chain-filters", chain::scenes::filters),
    ("provenance-chain-table", chain::scenes::table),
    ("provenance-chain-coverage", |a| {
        chain::scenes::tab(a, "coverage")
    }),
    ("provenance-chain-entropy", |a| {
        chain::scenes::tab(a, "entropy")
    }),
    ("provenance-chain-labels", |a| {
        chain::scenes::tab(a, "labels")
    }),
    ("provenance-chain-saved", chain::scenes::saved),
    ("provenance-chain-acquire", chain::scenes::acquire),
    ("provenance-chain-watch", chain::scenes::watch),
    ("provenance-chain-inbox", chain::scenes::inbox),
    ("provenance-chain-psbt", |a| {
        chain::scenes::open(a, "/chain-analysis?workspace=psbt")
    }),
    ("provenance-chain-datasets", |a| {
        chain::scenes::open(a, "/chain-analysis?workspace=datasets")
    }),
    ("provenance-chain-trace", |a| {
        chain::scenes::open(
            a,
            &format!("/chain-analysis?subject={}", chain::scenes::ANCHOR_TXID),
        )
    }),
    ("provenance-chain-empty", |a| {
        chain::scenes::open(a, "/chain-analysis?mode=trace")
    }),
    // A saved export's notice with its × (synthetic: nothing is written).
    ("provenance-chain-notice", |a| {
        chain::scenes::open(a, "/chain-analysis");
        a.provenance.chain.notice = Some(crate::i18n::t("chainAnalysis:exportSaved"));
    }),
    // Privacy Mirror (src/provenance/mirror/scenes.rs).
    ("provenance-mirror-default", mirror::scenes::open),
    ("provenance-mirror-light", mirror::scenes::light),
    ("provenance-mirror-de", mirror::scenes::german),
    ("provenance-mirror-narrow", mirror::scenes::open),
    ("provenance-mirror-attention", mirror::scenes::attention),
    ("provenance-mirror-context", |a| {
        mirror::scenes::with_open(a, &["context"])
    }),
    ("provenance-mirror-basis", mirror::scenes::basis),
    ("provenance-mirror-coverage", |a| {
        mirror::scenes::with_open(a, &["coverage"])
    }),
    ("provenance-mirror-entropy", |a| {
        mirror::scenes::with_open(a, &["coverage", "entropy", "assumptions"]);
        a.provenance.mirror.reveal = Some("pm-assumptions-summary".into());
    }),
    ("provenance-mirror-all", |a| {
        mirror::scenes::attention(a);
        for k in ["context", "coverage", "entropy", "assumptions"] {
            a.provenance.mirror.open.insert(k.into());
        }
        a.provenance.mirror.reveal = Some("pm-assumptions-summary".into());
    }),
    ("provenance-mirror-loading", mirror::scenes::loading),
    ("provenance-mirror-error", mirror::scenes::error),
    ("provenance-mirror-refreshing", mirror::scenes::refreshing),
    (
        "provenance-mirror-refresh-error",
        mirror::scenes::refresh_error,
    ),
    ("provenance-mirror-empty", mirror::scenes::empty),
    ("provenance-mirror-nothing", mirror::scenes::nothing_found),
    ("provenance-mirror-drill", mirror::scenes::drill),
];

/// Source of funds' scenes (registered beside `SCENES` in main.rs).
pub const SOF_SCENES: &[crate::app::Scene] = sof::scenes::SCENES;
