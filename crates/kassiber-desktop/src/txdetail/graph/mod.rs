//! The transaction graph in the detail panel (TransactionGraphTab.tsx, TransactionGraphModel.ts,
//! TransactionGraphGeometry.ts, graph3d/*): the 2.5D glass view (primary) and the flat bowtie,
//! the inputs/outputs list, swap route strip, annotations, warnings, technical details, graph
//! focus and the on-chain lookup consent.
//!
//! Graph focus (an owner-requested change from the web's expanded dialog): Expand maximizes the
//! side panel exactly as its own Maximize does (same state change, so the same layout spring and
//! geometry) and the panel body becomes the graph: a header with Restore, the drawing filling
//! the body, legend, and inputs/outputs. Restore, Escape or the panel's restore control bring
//! back the docked body at its scroll position. Switching tabs, closing or navigating leaves it.
//!
//! Data: `ui.transactions.graph` is a local read. `allowPublicLookup` goes out only after the
//! "Look up on-chain" click, approved for that transaction id alone (the web's
//! `publicGraphLookupApproval`), so one lookup never enables the next. The transaction open in
//! the panel also asks its wallet's own backend for missing amounts (`lookup: "wallet_backend"`,
//! see `auto`), which the daemon confines to that backend and refuses while offline.

pub mod auto;
mod data;
mod dev;
mod flat;
mod geometry;
mod glass;
pub mod gpu;
mod list;
mod model;
pub mod route;
mod route_strings;
pub mod sources;
mod strip;
mod trade;
mod view;
mod warnings;

use std::collections::HashMap;
use std::rc::Rc;

use rune::{Cx, El, Key, KeyEvent, Tx, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::surface;

pub use data::{LegKey, Payload, Warning};
pub use dev::{dev_page, scene_open};

#[derive(Clone, Debug)]
pub enum Entry {
    Loading,
    Ready(Rc<Payload>),
    Failed(String),
}

/// Compact (inline) or expanded (graph focus): each has its own hover and list folding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Compact = 0,
    Expanded = 1,
}

#[derive(Clone, Debug, Default)]
pub struct ViewState {
    /// The leg under the pointer in the drawing (shows the hover card).
    pub hover: Option<String>,
    /// Columns opened past the first eight rows: [inputs, outputs].
    pub open: [bool; 2],
    /// Keyboard walk through the legs of the 2D drawing.
    pub focus: Option<usize>,
}

/// The panel state graph focus was entered from.
#[derive(Clone, Debug, Default)]
pub struct Focus {
    /// The detail tab it belongs to (another tab in front ends it).
    pub tab: usize,
    /// The page it was entered on (navigating ends it).
    pub route: String,
    /// The panel was docked: leaving focus restores the docked width.
    pub restore: bool,
    /// The docked body's scroll offset, given back on the way out.
    pub scroll: f32,
    /// The panel maximize (true) or restore (false) to apply in the next frame's commit.
    pub resize: Option<bool>,
}

pub struct GraphUi {
    cache: HashMap<String, Entry>,
    tags: HashMap<String, &'static str>,
    /// The one transaction id whose public lookup the user approved.
    approved: Option<String>,
    /// The swap leg picked for a transaction (record id, leg).
    selected_leg: Option<(String, LegKey)>,
    pub views: [ViewState; 2],
    /// Graph focus is on (the dev harness page shows it in the page; the panel only while
    /// `focus_shown` holds).
    pub expanded: bool,
    /// Where graph focus was entered, and what to give back when it ends.
    pub focus: Focus,
    /// Show the flat bowtie instead of the glass view.
    pub flat: bool,
    /// One lit leg for drawing and list.
    pub active: Option<String>,
    tech_open: bool,
    /// "Network & data sources" unfolded.
    pub sources_open: bool,
    fee_absolute: bool,
    /// The route card's ⋯ menu is open.
    route_menu: bool,
    /// The dev harness's transaction.
    pub dev_record: Option<String>,
    /// What the GPU compositor draws at each graph slot this frame (filled by the views).
    pub gpu_jobs: std::cell::RefCell<HashMap<u32, gpu::Job>>,
    /// "Fill in transaction details from the wallet's backend" (Settings → Privacy), as the
    /// preference file the daemon reads says.
    pub auto_enabled: bool,
    /// The launch's override the preference was last read for (None: not read since a reset).
    pub auto_source: Option<Option<String>>,
    /// `App::book_generation` the local reads belong to; a book refresh (an unlink, reprocessed
    /// journals) re-reads them. Lookups that asked a server are not repeated.
    generation: u64,
    /// The book generation each graph read was sent in, by cache key.
    asked: HashMap<String, u64>,
}

impl Default for GraphUi {
    fn default() -> Self {
        GraphUi {
            cache: HashMap::new(),
            tags: HashMap::new(),
            approved: None,
            selected_leg: None,
            views: Default::default(),
            expanded: false,
            flat: false,
            active: None,
            tech_open: false,
            sources_open: false,
            fee_absolute: false,
            route_menu: false,
            dev_record: None,
            gpu_jobs: Default::default(),
            focus: Default::default(),
            // Read by `auto::sync` on the next poll; nothing is asked before that.
            auto_enabled: true,
            auto_source: None,
            generation: 0,
            asked: HashMap::new(),
        }
    }
}

impl GraphUi {
    fn key(reference: &str, public: bool) -> String {
        format!("{reference}|{}", if public { "public" } else { "local" })
    }
    pub fn entry(&self, reference: &str, public: bool) -> Option<&Entry> {
        self.cache.get(&Self::key(reference, public))
    }
    fn public_for(&self, id: &str) -> bool {
        self.approved.as_deref() == Some(id)
    }
    /// The automatic wallet-backend lookup's answer for `reference`.
    pub fn entry_auto(&self, reference: &str) -> Option<&Entry> {
        self.cache.get(&format!("{reference}|wallet"))
    }
    fn set(&mut self, reference: &str, mode: &str, entry: Entry) {
        self.cache.insert(format!("{reference}|{mode}"), entry);
    }
    #[cfg(test)]
    fn approve(&mut self, id: &str) {
        self.approved = Some(id.to_owned());
    }
    /// Marks the automatic lookup for `reference` as running and returns its tag, once.
    fn begin_auto(&mut self, reference: &str) -> Option<&'static str> {
        let key = format!("{reference}|wallet");
        if self.cache.contains_key(&key) {
            return None;
        }
        let tag: &'static str = self
            .tags
            .entry(key.clone())
            .or_insert_with(|| Box::leak(format!("graph:{key}").into_boxed_str()));
        self.cache.insert(key, Entry::Loading);
        Some(tag)
    }
    pub fn view(&self, v: View) -> &ViewState {
        &self.views[v as usize]
    }
    pub fn view_mut(&mut self, v: View) -> &mut ViewState {
        &mut self.views[v as usize]
    }
}

fn request(app: &mut App, reference: &str, public: bool) {
    if reference.is_empty() {
        return;
    }
    let key = GraphUi::key(reference, public);
    if matches!(
        app.graph.cache.get(&key),
        Some(Entry::Loading | Entry::Ready(_))
    ) {
        return;
    }
    let tag: &'static str = app
        .graph
        .tags
        .entry(key.clone())
        .or_insert_with(|| Box::leak(format!("graph:{key}").into_boxed_str()));
    app.graph.cache.insert(key.clone(), Entry::Loading);
    app.graph.asked.insert(key.clone(), app.book_generation);
    if app
        .send(
            "ui.transactions.graph",
            json!({"transaction": reference, "allowPublicLookup": public}),
            tag,
        )
        .is_none()
    {
        app.graph
            .cache
            .insert(key, Entry::Failed(t("transactions:graph.error")));
    }
}

/// Starts (or reuses) the local `ui.transactions.graph` read for a transaction id. The detail
/// panel calls this whenever it shows a transaction. Never enables the explicit public lookup by
/// itself; the wallet-backend lookup follows once the local read shows something is missing.
pub fn ensure(app: &mut App, transaction_id: &str) {
    if app.graph.generation != app.book_generation {
        // The book changed: its local projection (the swap route, own wallets) is read again.
        app.graph.generation = app.book_generation;
        app.graph.cache.retain(|k, _| !k.ends_with("|local"));
    }
    let public = app.graph.public_for(transaction_id);
    request(app, transaction_id, public);
    maybe_auto(app, transaction_id);
}

/// The transaction open in the panel.
fn open_id(app: &App) -> Option<&str> {
    app.detail
        .open
        .as_ref()
        .and_then(|o| o.record.as_ref())
        .and_then(|r| r["id"].as_str())
}

/// Asks the wallet's own backend for `reference`'s missing amounts when `auto::wanted` says so.
fn maybe_auto(app: &mut App, reference: &str) {
    if !auto::wanted(&app.graph, reference, open_id(app), auto::allowed_here()) {
        return;
    }
    let Some(tag) = app.graph.begin_auto(reference) else {
        return;
    };
    if app
        .send(
            "ui.transactions.graph",
            json!({"transaction": reference, "lookup": "wallet_backend"}),
            tag,
        )
        .is_none()
    {
        app.graph.set(
            reference,
            "wallet",
            Entry::Failed(t("transactions:graph.error")),
        );
    }
}

/// The "Look up on-chain" click: the consent for this transaction id only.
fn approve_lookup(app: &mut App, id: &str) {
    app.graph.approved = Some(id.to_owned());
    request(app, id, true);
}

/// Scenes only: the user approved the lookup for `id` and its reply is `entry` (nothing is sent).
#[allow(dead_code)]
pub fn scene_lookup(app: &mut App, id: &str, entry: Entry) {
    app.graph.approved = Some(id.to_owned());
    app.graph.cache.insert(GraphUi::key(id, true), entry);
}

/// Scenes only: opens the route card's ⋯ menu.
#[allow(dead_code)]
pub fn scene_route_menu(app: &mut App) {
    app.graph.route_menu = true;
}

/// Scenes only: the automatic wallet-backend lookup's state for `id` (nothing is sent).
#[allow(dead_code)]
pub fn scene_auto(app: &mut App, id: &str, entry: Entry) {
    app.graph.set(id, "wallet", entry);
}

/// Scenes only: replaces the local read's reply for `id`.
#[allow(dead_code)]
pub fn scene_local(app: &mut App, id: &str, entry: Entry) {
    app.graph.cache.insert(GraphUi::key(id, false), entry);
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    // Offline mode turned off: ask again where it refused the wallet-backend lookup.
    if r.kind == "ui.network.offline"
        && r.error.is_none()
        && r.data["offline"] != true
        && r.data["environment_blocked"] != true
    {
        app.graph
            .cache
            .retain(|k, e| !(k.ends_with("|wallet") && auto::offline_refusal(e)));
        if let Some(id) = open_id(app).map(str::to_owned) {
            maybe_auto(app, &id);
        }
    }
    let Some(key) = tag.strip_prefix("graph:") else {
        return;
    };
    let Some((reference, mode)) = key.rsplit_once('|') else {
        return;
    };
    if mode == "wallet" {
        let entry = match (&r.error, Payload::parse(&r.data)) {
            (Some(e), _) => Entry::Failed(e.clone()),
            (None, Ok(p)) => Entry::Ready(Rc::new(p)),
            (None, Err(e)) => Entry::Failed(e),
        };
        app.graph.set(reference, "wallet", entry);
        return;
    }
    let public = mode == "public";
    let generation = app.graph.asked.remove(key);
    if !public && generation != Some(app.book_generation) {
        // Read before the latest book refresh: dropped; the next show reads it again.
        app.graph.cache.remove(key);
        return;
    }
    let entry = match (&r.error, Payload::parse(&r.data)) {
        (Some(e), _) => Entry::Failed(e.clone()),
        (None, Ok(p)) => Entry::Ready(Rc::new(p)),
        (None, Err(e)) => Entry::Failed(e),
    };
    // Preload both legs of a swap (the web's swap-leg queries), under the same opt-in.
    let mut legs: Vec<(String, bool)> = Vec::new();
    if let Entry::Ready(p) = &entry
        && let Some(route) = &p.swap_route
    {
        let current = current_refs(reference, p);
        for leg in [LegKey::Out, LegKey::In] {
            if let Some(r) = model::preload_reference(route, leg, &current) {
                let local_row = route.leg(leg).id.as_deref() == Some(r.as_str());
                legs.push((r, public && local_row));
            }
        }
    }
    // The route is the book's (its custody projection), even in an answer that asked a server:
    // a fresh local read's route replaces the one in the lookups already shown.
    if let (false, Entry::Ready(local)) = (public, &entry) {
        for mode in ["public", "wallet"] {
            let k = format!("{reference}|{mode}");
            if let Some(Entry::Ready(p)) = app.graph.cache.get(&k)
                && p.swap_route != local.swap_route
            {
                let mut q = (**p).clone();
                q.swap_route = local.swap_route.clone();
                app.graph.cache.insert(k, Entry::Ready(Rc::new(q)));
            }
        }
    }
    app.graph
        .cache
        .insert(GraphUi::key(reference, public), entry);
    for (reference, public) in legs {
        request(app, &reference, public);
    }
    if !public {
        let reference = reference.to_owned();
        maybe_auto(app, &reference);
    }
}

fn current_refs(reference: &str, p: &Payload) -> Vec<String> {
    let mut current = vec![reference.to_owned()];
    if let Some(tx) = &p.transaction {
        current.extend(
            [tx.id.clone(), tx.txid.clone(), tx.external_id.clone()]
                .into_iter()
                .flatten(),
        );
    }
    current
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    // Escape leaves graph focus (before the panel's own Escape, which closes the detail).
    if ev.key == Key::Escape && app.graph.expanded && (focus_shown(app) || dev_page_route(app)) {
        collapse(app, tx, false);
        return true;
    }
    false
}

fn dev_page_route(app: &App) -> bool {
    app.route.path() == "/dev/graph"
}

/// Whether the side panel shows graph focus now: on, for the detail tab and page it was entered
/// on, the detail in front, and the panel maximized (or about to be, or a sheet, which has no
/// maximize).
pub fn focus_shown(app: &App) -> bool {
    let g = &app.graph;
    g.expanded
        && app.detail.open.is_some()
        && crate::assistant::detail_in_view(app)
        && g.focus.tab == app.detail.active_tab
        && g.focus.route == app.route.path()
        && (crate::txdetail::view::sheet_mode(app)
            || app.detail.maximized
            || g.focus.resize == Some(true))
}

/// Expand: graph focus. The body swaps at once at the panel's current width; a docked panel
/// then maximizes in the next frame's own commit (`poll`), the same state change the panel's
/// Maximize makes on its default spring, so geometry and motion are the panel's own and the
/// graph grows with it. Maximized or as a sheet, only the body changes.
pub fn expand(app: &mut App, tx: &mut Tx) {
    app.graph.views[View::Expanded as usize] = ViewState::default();
    app.graph.expanded = true;
    if dev_page_route(app) || app.detail.open.is_none() {
        return;
    }
    let sheet = crate::txdetail::view::sheet_mode(app);
    let restore = !sheet && !app.detail.maximized;
    app.graph.focus = Focus {
        tab: app.detail.active_tab,
        route: app.route.path().to_owned(),
        restore,
        scroll: crate::txdetail::view::scroll_y(),
        resize: restore.then_some(true),
    };
    tx.instant();
}

/// Scenes: graph focus already entered (the panel maximized without the motion).
#[allow(dead_code)]
pub fn scene_expand(app: &mut App) {
    expand(app, &mut Tx::default());
    if app.graph.focus.resize.take() == Some(true) {
        app.detail.maximized = true;
    }
}

/// Leaves graph focus: the docked body comes back at once at the current width, at the scroll
/// offset it had; then, when focus maximized the panel (always for the panel's restore control,
/// `restore_panel`), the panel restores in the next frame's commit as its own Restore does.
pub fn collapse(app: &mut App, tx: &mut Tx, restore_panel: bool) {
    let shown = focus_shown(app);
    app.graph.expanded = false;
    app.graph.views[View::Expanded as usize].hover = None;
    let grow_pending = app.graph.focus.resize.take() == Some(true);
    if !shown {
        return;
    }
    tx.instant();
    tx.scroll_to_y("txd-scroll", app.graph.focus.scroll);
    if (restore_panel || app.graph.focus.restore) && !grow_pending && app.detail.maximized {
        app.graph.focus.resize = Some(false);
    }
}

/// Each frame: applies the panel resize graph focus asked for (an animated commit of its own),
/// and ends graph focus once the panel stops showing it (another tab in front, the detail closed,
/// the page changed, the panel restored or hidden), so it doesn't come back later.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    // The wallet-backend lookup preference follows the daemon's launch (the demo has its own).
    auto::sync(app) | poll_focus(app, tx)
}

fn poll_focus(app: &mut App, tx: &mut Tx) -> bool {
    if let Some(max) = app.graph.focus.resize.take() {
        // A pending grow applies only while focus is still on; a pending restore only while it
        // stayed off.
        if max == app.graph.expanded {
            app.detail.maximized = max;
            tx.animated();
            if !max {
                tx.scroll_to_y("txd-scroll", app.graph.focus.scroll);
            }
            return true;
        }
    }
    if !app.graph.expanded || dev_page_route(app) || focus_shown(app) {
        return false;
    }
    app.graph.expanded = false;
    true
}

/// What the panel shows for a record: the record's own graph or the picked swap leg's, plus the
/// local read and the approved lookup of that reference apart (so a lookup in flight or failed
/// keeps the stored graph on screen, and the sources panel can say what the lookup added).
pub(crate) struct Shown {
    pub entry: Option<Entry>,
    pub own: bool,
    pub leg: Option<LegKey>,
    pub local: Option<Entry>,
    /// The approved lookup's reply (None without an approval for this reference).
    pub public: Option<Entry>,
    /// What the automatic wallet-backend lookup did for the record's own graph.
    pub auto: auto::AutoState,
    auto_payload: Option<Rc<Payload>>,
}

/// One reference's entries: the lookup's reply once it has a graph, else the local read while
/// the lookup runs or after it failed, else whatever there is.
fn pick(
    g: &GraphUi,
    reference: &str,
    public: bool,
) -> (Option<Entry>, Option<Entry>, Option<Entry>) {
    let local = g.entry(reference, false).cloned();
    if !public {
        return (local.clone(), local, None);
    }
    let lookup = g.entry(reference, true).cloned();
    let shown = match (&lookup, &local) {
        (Some(Entry::Ready(_)), _) => lookup.clone(),
        (None | Some(Entry::Loading | Entry::Failed(_)), Some(Entry::Ready(_))) => local.clone(),
        _ => lookup.clone().or(local.clone()),
    };
    (shown, local, lookup)
}

impl Shown {
    /// The automatic lookup's answer when it is the record's own graph's (for its stop reason).
    pub fn entry_auto_payload(&self) -> Option<Rc<Payload>> {
        self.auto_payload.clone()
    }
}

pub(crate) fn shown(app: &App, record_id: &str) -> Shown {
    let g = &app.graph;
    let public = g.public_for(record_id);
    let (mut own, local, lookup) = pick(g, record_id, public);
    // The wallet's backend filled the graph in: shown unless the explicit lookup has a graph.
    let auto = if public {
        auto::AutoState::NotAsked
    } else {
        auto::classify(g.entry_auto(record_id))
    };
    if auto == auto::AutoState::Filled {
        own = g.entry_auto(record_id).cloned();
    }
    let auto_payload = match g.entry_auto(record_id) {
        Some(Entry::Ready(p)) if !public => Some(p.clone()),
        _ => None,
    };
    let whole = |leg| Shown {
        entry: own.clone(),
        own: true,
        leg,
        local: local.clone(),
        public: lookup.clone(),
        auto: auto.clone(),
        auto_payload: auto_payload.clone(),
    };
    let Some(Entry::Ready(p)) = &own else {
        return whole(None);
    };
    let Some(route) = &p.swap_route else {
        return whole(None);
    };
    let picked = g
        .selected_leg
        .as_ref()
        .filter(|(id, _)| id == record_id)
        .map(|(_, l)| *l);
    let default = model::exchange_transfer(route)
        .map(|(_, _, chain)| chain)
        .or(match route.current_leg.as_deref() {
            Some("out") => Some(LegKey::Out),
            Some("in") => Some(LegKey::In),
            _ => None,
        });
    let leg = picked.or(default);
    let leg_ref = leg.and_then(|l| model::preload_reference(route, l, &current_refs(record_id, p)));
    match (leg, leg_ref) {
        (Some(l), Some(r)) => {
            let local_row = route.leg(l).id.as_deref() == Some(r.as_str());
            let (entry, local, public) = pick(g, &r, public && local_row);
            Shown {
                entry,
                own: false,
                leg: Some(l),
                local,
                public,
                auto: auto::AutoState::NotAsked,
                auto_payload: None,
            }
        }
        _ => whole(leg),
    }
}

fn select_leg(app: &mut App, record_id: &str, leg: LegKey) {
    app.graph.selected_leg = Some((record_id.to_owned(), leg));
    app.graph.active = None;
}

/// Context every part of the section reads.
#[derive(Clone, Copy)]
pub(crate) struct Ctx<'a> {
    /// Key suffix: "" inline, "-x" in the expanded dialog (keys are one namespace).
    pub sfx: &'static str,
    pub app: &'a App,
    pub tk: &'a Tk,
    pub hide: bool,
    /// The record's own EUR price when the app shows fiat and the graph is the record's own.
    pub fiat: Option<f64>,
    /// The record's own price at all (the amount click toggles the currency only then).
    pub priced: bool,
}

/// The graph area for the open transaction (`record` is the full transaction row JSON as the
/// list/resolve kinds return it).
fn ctx<'a>(app: &'a App, tk: &'a Tk, record: &Value, sh: &Shown, sfx: &'static str) -> Ctx<'a> {
    let hide = app.prefs.hide_sensitive();
    let euro = record["fiatCurrency"]
        .as_str()
        .is_some_and(|c| c.eq_ignore_ascii_case("EUR"));
    let rate = record["rate"]
        .as_f64()
        .filter(|r| r.is_finite() && *r > 0.0);
    let priced = sh.own && euro && rate.is_some();
    let fiat = if priced && app.prefs.currency == crate::app::Currency::Fiat {
        rate
    } else {
        None
    };
    Ctx {
        sfx,
        app,
        tk,
        hide,
        fiat,
        priced,
    }
}

/// "Look up on-chain" (`Button variant="outline" size="sm"`: h-8, px-3, text-xs); disabled while
/// offline mode is on. The click is the consent, for this transaction id alone.
fn lookup_button(tk: &Tk, key: &str, record_id: String, off: bool) -> El {
    let b = crate::ui::button::button(
        tk,
        key,
        crate::ui::button::Variant::Outline,
        crate::ui::button::Size::Sm,
        None,
        &t("transactions:graph.lookupOnChain"),
        !off,
    );
    if off {
        b.help(t("chrome:network.indicator.offlineMode"))
    } else {
        b.on_press(move |a: &mut App| {
            // Offline mode can have turned on since this frame.
            if !sources::offline(a) {
                approve_lookup(a, &record_id);
            }
        })
    }
}

pub fn section(app: &App, cx: &mut Cx<App>, tk: &Tk, record: &Value, layer: &mut Vec<El>) -> El {
    let record_id = record["id"].as_str().unwrap_or("").to_owned();
    let sh = shown(app, &record_id);
    let c = ctx(app, tk, record, &sh, "");
    let payload = match &sh.entry {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    let loading = matches!(sh.entry, None | Some(Entry::Loading));
    let error = match &sh.entry {
        Some(Entry::Failed(e)) => Some(e.clone()),
        _ => None,
    };
    let drawn = payload.as_ref().is_some_and(|p| p.drawable());
    let state = sources::lookup(app, record, &sh);
    // Offered (and, as an improvement, shown disabled while offline mode switches it off).
    let can_lookup = matches!(
        state,
        sources::Lookup::Offer { .. } | sources::Lookup::Offline { .. }
    );
    let off = matches!(state, sources::Lookup::Offline { .. });
    let lookup = || lookup_button(tk, "graph-lookup", record_id.clone(), off);
    // `KB_TXD_PLAIN=1` leaves out this round's additions (the notice and the sources fold), only
    // to measure the tabs' parity against the web on the same layout.
    let plain = std::env::var_os("KB_TXD_PLAIN").is_some();
    let notice = if plain {
        None
    } else {
        sources::notice(&c, record, &sh, &state).or_else(|| sources::quiet(&c, &state))
    };

    let mut panel = col().key("graph-panel").w_full().gap(tk.r(1.0));
    if let Some(route) = payload.as_ref().and_then(|p| p.swap_route.as_ref()) {
        let id = record_id.clone();
        panel = panel.child(strip::route(
            &c,
            record,
            route,
            sh.leg,
            move |a: &mut App, leg| select_leg(a, &id, leg),
            layer,
            cx.layout_size("graph-route").map(|[w, _]| w).unwrap_or(0.0),
        ));
    }
    let mut card = surface(tk)
        .key("graph-card")
        .w_full()
        .padding(tk.r(1.25))
        .gap(tk.r(0.75));
    match &payload {
        Some(p) if drawn => {
            // `flex flex-wrap items-center justify-between gap-2`: the title block takes its
            // text's width, so a long caption pushes the actions onto the next line (left).
            let mut actions = row().gap(tk.r(0.5)).center_y();
            if can_lookup {
                actions = actions.child(lookup());
            }
            actions = actions.child(view::expand_button(tk));
            let mut heading = col().child(
                text(t("transactions:graph.title"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            );
            if let Some(caption) = warnings::support_caption(p) {
                heading = heading.child(text(caption).font(tk.txs(400.0)).color(tk.muted_fg));
            }
            panel = panel
                .child(
                    row()
                        .key("graph-header")
                        .w_full()
                        .flex_wrap()
                        .gaps(tk.r(0.5), tk.r(0.5))
                        .center_y()
                        .justify_between()
                        .child(heading)
                        .child(actions),
                )
                .children(notice)
                .children(strip::annotations(&c, p))
                .child(flow_layout(&c, cx, p, View::Compact))
                .children(warnings::warnings(&c, Some(p)));
        }
        _ => {
            if can_lookup {
                card = card.child(row().w_full().justify_end().child(lookup()));
            }
            // A buy or sell from an exchange import with no chain transaction: the trade receipt
            // in place of the empty graph (`graphlessContent`), the record's own graph only.
            let trade = payload
                .as_ref()
                .filter(|_| sh.own && !loading && error.is_none())
                .and_then(|p| trade::kind(record, p).map(|k| (p.clone(), k)));
            panel = match trade {
                Some((p, k)) => panel
                    .children(notice)
                    .child(trade::flow(&c, record, k))
                    .children(warnings::warnings(&c, Some(&p))),
                None => panel.children(notice).child(warnings::empty_state(
                    &c,
                    payload.as_deref(),
                    loading,
                    error.as_deref(),
                )),
            };
        }
    }
    let mut out = col()
        .key("graph-section")
        .w_full()
        .gap(tk.r(0.75))
        .child(card.child(panel));
    if let Some(tech) = payload.as_ref().and_then(|p| warnings::technical(&c, p)) {
        out = out.child(tech);
    }
    if plain {
        return out;
    }
    out.child(sources::panel(&c, record, &sh, &state))
}

/// The drawing with its inputs and outputs listed below it, as mempool shows a transaction.
fn flow_layout(c: &Ctx, cx: &mut Cx<App>, p: &Rc<Payload>, v: View) -> El {
    let drawing = if c.app.graph.flat {
        view::flat_view(c, cx, p, v)
    } else {
        view::glass_view(c, cx, p, v)
    };
    col()
        .key(format!("graph-flow-{}", v as usize))
        .w_full()
        .gap(c.tk.r(0.75))
        .child(drawing)
        .children(list::panel(c, cx, p, v))
}

/// Graph focus: the panel body becomes the graph (the web's expanded dialog content, in place):
/// a header with Restore, the title and the lookup, then the drawing sized to fill the body
/// with the legend and the inputs/outputs below it. `subtitle` names the transaction.
pub fn focus_view(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    record: &Value,
    subtitle: Option<String>,
) -> El {
    let record_id = record["id"].as_str().unwrap_or("").to_owned();
    let sh = shown(app, &record_id);
    let c = ctx(app, tk, record, &sh, "-x");
    let payload = match &sh.entry {
        Some(Entry::Ready(p)) => Some(p.clone()),
        _ => None,
    };
    let loading = matches!(sh.entry, None | Some(Entry::Loading));
    let error = match &sh.entry {
        Some(Entry::Failed(e)) => Some(e.clone()),
        _ => None,
    };
    let state = sources::lookup(app, record, &sh);
    let can_lookup = matches!(
        state,
        sources::Lookup::Offer { .. } | sources::Lookup::Offline { .. }
    );
    let off = matches!(state, sources::Lookup::Offline { .. });
    // `p-4 sm:p-6`, as the panel body.
    let pad = if app.shell.window_w <= 0.0 || app.shell.window_w >= 640.0 {
        tk.r(1.5)
    } else {
        tk.r(1.0)
    };

    // Restore (outline `size="sm"`, Minimize2 + text, the table's Expand/Restore pair).
    let restore = t("transactions:table.expand.collapse");
    let back = crate::ui::button::button(
        tk,
        "graph-focus-restore",
        crate::ui::button::Variant::Outline,
        crate::ui::button::Size::Sm,
        Some(&I::MINIMIZE_2),
        &restore,
        true,
    )
    .help(format!("{restore} (Esc)"))
    .on_press_tx(|a: &mut App, tx: &mut Tx| collapse(a, tx, false));
    let mut title_line = row().min_w(0.0).gap(tk.r(0.5)).center_y().child(
        text(t("transactions:graph.title"))
            .font(tk.tsm(600.0))
            .color(tk.fg)
            .role("heading"),
    );
    if let Some(sub) = subtitle.filter(|s| !s.is_empty() && !c.hide) {
        title_line = title_line
            .child(text("·").font(tk.tsm(400.0)).color(tk.muted_fg))
            .child(
                text(sub.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .truncate()
                    .help_when_truncated(sub),
            );
    }
    let mut heading = col().grow(1.0).min_w(0.0).child(title_line);
    if let Some(caption) = payload.as_deref().and_then(warnings::support_caption) {
        heading = heading.child(
            text(caption)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate(),
        );
    }
    let mut header = row()
        .key("graph-focus-header")
        .w_full()
        .px(pad)
        .py(tk.r(0.75))
        .gap(tk.r(0.75))
        .center_y()
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(back)
        .child(heading);
    if can_lookup {
        header = header.child(lookup_button(tk, "graph-lookup-x", record_id.clone(), off));
    }

    let content = match &payload {
        Some(p) if p.drawable() => flow_layout(&c, cx, p, View::Expanded),
        _ => {
            let trade = payload
                .as_ref()
                .filter(|_| sh.own && !loading && error.is_none())
                .and_then(|p| trade::kind(record, p));
            match trade {
                Some(k) => trade::flow(&c, record, k),
                None => warnings::empty_state(&c, payload.as_deref(), loading, error.as_deref()),
            }
        }
    };
    let body = col()
        .key(FOCUS_SCROLL)
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(col().w_full().padding(pad).child(content));
    col()
        .key("graph-focus")
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .role("region")
        .label(t("transactions:graph.expandedTitle"))
        .child(header)
        .child(body)
}

/// The graph focus body's scroll viewport (sizes the drawing and clips the GPU view).
pub(crate) const FOCUS_SCROLL: &str = "graph-focus-scroll";

#[cfg(test)]
mod refresh_tests {
    use super::*;

    /// A book refresh re-reads a transaction's local graph (its swap route comes from the
    /// custody projection) but keeps a lookup that asked a server; a pre-refresh local answer
    /// does not land.
    #[test]
    fn a_book_refresh_rereads_the_local_graph_only() {
        let mut app = crate::app::tests::quiet_app();
        app.graph.generation = app.book_generation;
        app.graph.cache.insert(
            GraphUi::key("t1", false),
            Entry::Ready(Rc::new(Payload::default())),
        );
        app.graph.cache.insert(
            GraphUi::key("t1", true),
            Entry::Ready(Rc::new(Payload::default())),
        );
        app.book_generation += 1;
        ensure(&mut app, "t1");
        assert!(matches!(app.graph.entry("t1", false), Some(Entry::Loading)));
        assert!(matches!(app.graph.entry("t1", true), Some(Entry::Ready(_))));
        // An answer sent before another refresh is dropped.
        app.book_generation += 1;
        let reply = Reply {
            kind: "ui.transactions.graph".into(),
            data: json!({}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "graph:t1|local", &reply);
        assert!(app.graph.entry("t1", false).is_none());
    }
}
