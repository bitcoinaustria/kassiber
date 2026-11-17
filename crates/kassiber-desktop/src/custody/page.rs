//! `TransfersPage` / `TransfersOverview` (`routes/transfers/TransfersPage.tsx`): has every move
//! between the owner's wallets been accounted for? The status tabs (needs you, linked, unlinked
//! by you, set aside) with their counts, the stat strip, then one card per status: what is held
//! and the suggestions, the moves unlinked recently, set-aside suggestions, and every linked
//! move with how it was linked and the way to unlink it. "Link manually" and a move's details
//! open in a panel beside the lists when the page is wide, as dialogs otherwise. What an action
//! did is said in a strip at its row or in a toast, with the way back. The developer-gated
//! advanced tools are the one secondary section.

use std::collections::HashSet;

use rune::{Color, Cx, El, Tx, col, el, row, text};
use rune_kit::Placement;
use serde_json::{Value, json};

use super::kit::{self, Entry, Var};
use super::link::{self, LinkStart, LinkedResult, Preset};
use super::link_model::{self, Parts, date_part, date_time, has_open_part, kind_key, time_part};
use super::model::{self, Dismissal, Filter, Lineage, LinkedMove, Pair, Source, Unlinked, Verdict};
use super::route::{self, LINK_NEW, Search, Tab, View};
use super::unlink::{self, Intent, UnlinkResult};
use super::{components, gaps, rules, suggestions};
use crate::app::{App, Currency, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::popup::TriggerExt;
use crate::ui::{ic, sensitive};

const LINKED_PAGE: usize = 25;
const TOAST_SECS: f64 = 10.0;
const DISMISSALS_LIMIT: u32 = 500;
const HELD_PAGE: u32 = 100;
/// Content this wide or wider keeps a panel beside the lists.
const PANEL_MIN_WIDTH: f32 = 1100.0;

/// One read's state, as react-query keeps it: data stays through a refetch; `failed` is the
/// last fetch's outcome.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rd {
    #[default]
    Idle,
    Loading,
    Ready,
}

#[derive(Clone, Debug, Default)]
pub struct Read {
    pub st: Rd,
    pub failed: bool,
    pub fetching: bool,
}

impl Read {
    fn start(&mut self) {
        if self.st == Rd::Idle {
            self.st = Rd::Loading;
        }
        self.fetching = true;
    }
    fn done(&mut self, ok: bool) {
        self.fetching = false;
        self.failed = !ok;
        if ok || self.st == Rd::Loading {
            self.st = Rd::Ready;
        }
    }
    /// `isLoading`: the first fetch is still out.
    pub fn loading(&self) -> bool {
        self.st == Rd::Loading
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusTab {
    Needs,
    Linked,
    Unlinked,
    SetAside,
}

impl StatusTab {
    const ALL: [StatusTab; 4] = [
        StatusTab::Needs,
        StatusTab::Linked,
        StatusTab::Unlinked,
        StatusTab::SetAside,
    ];
    fn key(self) -> &'static str {
        match self {
            StatusTab::Needs => "needs",
            StatusTab::Linked => "linked",
            StatusTab::Unlinked => "unlinked",
            StatusTab::SetAside => "setAside",
        }
    }
}

/// The transfer a note is about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Spot {
    pub out_id: String,
    pub in_id: String,
}

/// A note after an action, with a way back (`Toast`).
// The page holds one note at a time, so the size gap between variants costs nothing worth a box.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum NoteKind {
    Unlinked {
        dismissal_id: Option<String>,
        out_id: String,
        in_id: String,
    },
    /// `portion`: only part of the send was swapped, so linking it again is checked first.
    Unpaired {
        pair: Option<Pair>,
        mv: LinkedMove,
        portion: bool,
        listed: Option<Pair>,
    },
    LinkedBack,
    Linked {
        pair_id: Option<String>,
        send_id: String,
    },
    LinkUndone,
}

impl NoteKind {
    fn key(&self) -> &'static str {
        match self {
            NoteKind::Unlinked { .. } => "unlinked",
            NoteKind::Unpaired { .. } => "unpaired",
            NoteKind::LinkedBack => "linkedBack",
            NoteKind::Linked { .. } => "linked",
            NoteKind::LinkUndone => "linkUndone",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Note {
    pub spot: Option<Spot>,
    pub kind: NoteKind,
    /// App clock when it was shown (the toast's timer runs from then, or from when it may).
    pub since: f64,
}

/// An open menu of the page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Popup {
    Menu,
    Filter,
    Row(String),
    Held(String),
}

#[derive(Default)]
pub struct State {
    // Reads.
    pub badges: Option<Value>,
    pub badges_rd: Read,
    pub suggest: Option<Vec<Value>>,
    pub suggest_rd: Read,
    /// `useSettledSuggestions`: the list as read before the journals started recalculating.
    pub frozen: Option<Vec<Value>>,
    was_updating: bool,
    pub pairs: Option<Vec<Pair>>,
    pub unlinked_pairs: Vec<Pair>,
    pub pairs_rd: Read,
    pub parts: Option<Parts>,
    pub dismissals: Option<Vec<Dismissal>>,
    pub dismissals_more: bool,
    pub dismissals_rd: Read,
    pub held_items: Vec<Value>,
    held_buf: Vec<Value>,
    pub held_rd: Read,
    /// More pages of holds to read (`hasNextPage`).
    pub held_more: bool,
    pub lineage: Vec<Lineage>,
    lineage_buf: Vec<Lineage>,
    pub lineage_total: usize,
    pub lineage_cursor: Option<String>,
    pub lineage_rd: Read,
    pub lineage_more_loading: bool,
    /// Pages loaded, so a refetch reads as many again.
    lineage_pages: usize,
    lineage_refetch_left: usize,
    pub views: Vec<Value>,
    views_for: Option<String>,
    // Derived.
    pub moves: Vec<LinkedMove>,
    pub held: Vec<Dismissal>,
    pub gap_holds: Vec<Value>,
    pub unlinked: Vec<Unlinked>,
    pub set_aside: Vec<Dismissal>,
    // UI.
    pub filter: Option<Filter>,
    pub year: Option<i32>,
    pub shown: usize,
    pub tab: Option<StatusTab>,
    pub note: Option<Note>,
    pub error: Option<String>,
    pub popup: Option<Popup>,
    pub last_place: Option<usize>,
    /// Mutations in flight: undismiss, repair, unpair.
    pub undismissing: bool,
    pub repairing: bool,
    pub unpairing: bool,
    /// The note a pending mutation turns into, once it succeeds.
    pending_note: Option<Note>,
    /// Suggestions-set-aside restore: refetch on success.
    restoring: bool,
    /// The saved view the page last opened for (an effect on `savedViewId`).
    seen_view: Option<String>,
    opened_tab_for_unreadable: bool,
}

fn st(app: &App) -> &State {
    &app.custody.page
}

fn stm(app: &mut App) -> &mut State {
    &mut app.custody.page
}

// ---- Reads ------------------------------------------------------------------------------------

/// The route mounted (`first`) or its search changed.
pub fn mount(app: &mut App, first: bool) {
    match super::current_view(app) {
        View::Advanced { tab, .. } => {
            match tab {
                Tab::Gaps => gaps::mount_gaps(app),
                Tab::Components => components::mount(app),
                Tab::Coverage => gaps::mount_coverage(app),
            }
            if first {
                read_all(app);
            }
        }
        View::Overview { link, saved_view } => {
            if first || st(app).pairs_rd.st == Rd::Idle {
                read_all(app);
            }
            if let Some(id) = link {
                // Another screen asked for "Link manually" on a transaction.
                unlink::close(app);
                link::open(
                    app,
                    if id == LINK_NEW {
                        LinkStart::default()
                    } else {
                        LinkStart {
                            transaction_id: Some(id),
                            ..Default::default()
                        }
                    },
                );
                // `onConsumeLink`: the one-shot `link` leaves the URL (replace).
                let next = Search {
                    saved_view: saved_view.clone(),
                    ..Search::default()
                };
                app.route.0 = route::href(&next);
                app.custody.seen = app.route.0.clone();
            }
            if saved_view != st(app).seen_view {
                stm(app).seen_view = saved_view.clone();
                if saved_view.is_some() {
                    stm(app).tab = Some(StatusTab::Needs);
                    unlink::close(app);
                    if st(app).views_for.is_none() {
                        read_views(app);
                    }
                }
            }
        }
    }
    // The gated section never shows without developer tools (`beforeLoad` + the effect).
    if !app.prefs.developer_tools
        && route::params(&app.route.0[app.route.path().len()..])
            .iter()
            .any(|(k, v)| k == "section" && v == "advanced")
    {
        app.route.0 = "/transfers".into();
        app.custody.seen = app.route.0.clone();
    }
}

/// The screen was left: menus close, the panels' content stays for the next visit.
/// Leaving `/transfers`: `TransfersPage`'s `useState`s start over (filter, year, tab, paging,
/// the undo toast and one still waiting on its mutation, the error, open menus). The reads stay.
pub fn leave(app: &mut App) {
    let s = stm(app);
    s.popup = None;
    s.filter = None;
    s.year = None;
    s.shown = 0;
    s.tab = None;
    s.note = None;
    s.pending_note = None;
    s.error = None;
    s.last_place = None;
    s.seen_view = None;
}

fn read_views(app: &mut App) {
    stm(app).views_for = Some(String::new());
    stm(app).views.clear();
    app.send(
        "ui.saved_views.list",
        json!({"surface": "transfer_candidates"}),
        "custody.page.views",
    );
    app.send(
        "ui.saved_views.list",
        json!({"surface": "swap_candidates"}),
        "custody.page.views",
    );
}

fn read_all(app: &mut App) {
    read_badges(app);
    read_suggest(app);
    read_pairs(app);
    read_dismissals(app);
    read_held(app);
    read_lineage(app, true);
}

fn read_badges(app: &mut App) {
    stm(app).badges_rd.start();
    app.send("ui.review.badges", Value::Null, "custody.page.badges");
}
fn read_suggest(app: &mut App) {
    stm(app).suggest_rd.start();
    app.send("ui.transfers.suggest", json!({}), "custody.page.suggest");
}
fn read_pairs(app: &mut App) {
    stm(app).pairs_rd.start();
    app.send(
        "ui.transfers.list",
        json!({"include_unlinked": true, "priced": true}),
        "custody.page.pairs",
    );
}
fn read_dismissals(app: &mut App) {
    stm(app).dismissals_rd.start();
    app.send(
        "ui.transfers.dismissals.list",
        json!({"limit": DISMISSALS_LIMIT}),
        "custody.page.dismissals",
    );
}
fn read_held(app: &mut App) {
    let s = stm(app);
    // `hasNextPage` comes from the data on screen: a refetch of read holds is not "loading".
    if s.held_rd.st == Rd::Idle {
        s.held_more = true;
    }
    s.held_rd.start();
    s.held_buf.clear();
    app.send(
        "ui.journals.quarantine",
        json!({"limit": HELD_PAGE, "offset": 0, "scope": "attention"}),
        "custody.page.held",
    );
}
/// The lineage from its first page; a refetch reads as many pages as were loaded.
fn read_lineage(app: &mut App, reset: bool) {
    let s = stm(app);
    s.lineage_rd.start();
    s.lineage_buf.clear();
    s.lineage_refetch_left = if reset {
        0
    } else {
        s.lineage_pages.saturating_sub(1)
    };
    app.send(
        "ui.custody.lineage.snapshot",
        json!({"limit": 100, "priced": true}),
        "custody.page.lineage",
    );
}

/// `fetchNextPage` of the lineage ("Show older transfers").
fn load_more_lineage(app: &mut App) {
    let Some(cursor) = st(app).lineage_cursor.clone() else {
        return;
    };
    stm(app).lineage_more_loading = true;
    app.send(
        "ui.custody.lineage.snapshot",
        json!({"limit": 100, "priced": true, "cursor": cursor}),
        "custody.page.lineage_more",
    );
}

/// Every read again (the journals were recalculated, or a mutation refreshed the book).
pub fn reload(app: &mut App) {
    match super::current_view(app) {
        View::Advanced { .. } => {}
        View::Overview { .. } => {}
    }
    read_badges(app);
    read_suggest(app);
    read_pairs(app);
    read_dismissals(app);
    read_held(app);
    read_lineage(app, false);
    if st(app).views_for.is_some() {
        read_views(app);
    }
}

/// Nothing to chain outside replies; kept for `settle`.
pub fn continue_reads(_app: &mut App) {}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let ok = r.error.is_none();
    match tag {
        "custody.page.badges" => {
            let s = stm(app);
            s.badges_rd.done(ok);
            if ok {
                s.badges = Some(r.data.clone());
            }
        }
        "custody.page.suggest" => {
            let s = stm(app);
            s.suggest_rd.done(ok);
            if ok {
                s.suggest = Some(r.data["candidates"].as_array().cloned().unwrap_or_default());
                // `useSettledSuggestions`: the run is over and the list is read again.
                if !super::updating(app) {
                    stm(app).frozen = None;
                }
            }
        }
        "custody.page.pairs" => {
            let s = stm(app);
            s.pairs_rd.done(ok);
            if ok {
                let pairs = Pair::list(&r.data["pairs"]);
                s.parts = Some(link_model::open_parts(&pairs));
                s.pairs = Some(pairs);
                s.unlinked_pairs = Pair::list(&r.data["unlinked"]);
            }
        }
        "custody.page.dismissals" => {
            let s = stm(app);
            s.dismissals_rd.done(ok);
            if ok {
                s.dismissals = Some(Dismissal::list(&r.data["dismissals"]));
                s.dismissals_more = r.data["has_more"].as_bool().unwrap_or(false);
            }
        }
        "custody.page.held" => {
            if !ok {
                let s = stm(app);
                s.held_rd.done(false);
                s.held_more = false;
            } else {
                let items = r.data["items"].as_array().cloned().unwrap_or_default();
                let summary = &r.data["summary"];
                let offset = summary["offset"].as_u64().unwrap_or(0) as usize;
                let total = summary["scope_count"]
                    .as_u64()
                    .or_else(|| summary["count"].as_u64())
                    .unwrap_or(0) as usize;
                let next = (!items.is_empty() && offset + items.len() < total)
                    .then_some(offset + items.len());
                let s = stm(app);
                s.held_buf.extend(items);
                match next {
                    Some(n) => {
                        app.send(
                            "ui.journals.quarantine",
                            json!({"limit": HELD_PAGE, "offset": n, "scope": "attention"}),
                            "custody.page.held",
                        );
                    }
                    None => {
                        s.held_items = std::mem::take(&mut s.held_buf);
                        s.held_more = false;
                        s.held_rd.done(true);
                    }
                }
            }
        }
        "custody.page.lineage" | "custody.page.lineage_more" => {
            let more = tag == "custody.page.lineage_more";
            let s = stm(app);
            if more {
                s.lineage_more_loading = false;
            }
            if !ok {
                if !more {
                    s.lineage_rd.done(false);
                }
                return;
            }
            let items: Vec<Lineage> = r.data["items"]
                .as_array()
                .map(|a| a.iter().map(Lineage::parse).collect())
                .unwrap_or_default();
            let cursor = r.data["next_cursor"]
                .as_str()
                .map(str::to_owned)
                .filter(|c| !c.is_empty());
            let total = r.data["summary"]["total_count"].as_u64().unwrap_or(0) as usize;
            if more {
                s.lineage.extend(items);
                s.lineage_cursor = cursor;
                s.lineage_pages += 1;
            } else {
                if s.lineage_buf.is_empty() {
                    s.lineage_total = total;
                }
                s.lineage_buf.extend(items);
                if s.lineage_refetch_left > 0
                    && let Some(c) = cursor
                {
                    s.lineage_refetch_left -= 1;
                    app.send(
                        "ui.custody.lineage.snapshot",
                        json!({"limit": 100, "priced": true, "cursor": c}),
                        "custody.page.lineage",
                    );
                    return;
                }
                let pages = s.lineage_pages.max(1);
                s.lineage = std::mem::take(&mut s.lineage_buf);
                s.lineage_cursor = cursor;
                s.lineage_pages = pages;
                s.lineage_rd.done(true);
            }
        }
        "custody.page.views" => {
            if ok {
                let views = r.data["views"].as_array().cloned().unwrap_or_default();
                stm(app).views.extend(views);
            }
        }
        "custody.page.undismiss" | "custody.page.repair" | "custody.page.unpair" => {
            let s = stm(app);
            s.undismissing = false;
            s.repairing = false;
            s.unpairing = false;
            let note = s.pending_note.take();
            let restoring = std::mem::take(&mut s.restoring);
            match &r.error {
                Some(e) => {
                    stm(app).error = Some(if e.is_empty() {
                        t(if tag == "custody.page.unpair" {
                            "review:transfersPage.link.failed"
                        } else {
                            "review:transfersPage.toast.linkBackFailed"
                        })
                    } else {
                        e.clone()
                    });
                }
                None => {
                    if restoring {
                        // A suggestion set aside comes back to the list; nothing is linked.
                        read_suggest(app);
                        read_dismissals(app);
                    } else {
                        if let Some(n) = note {
                            set_note(app, n);
                        }
                        super::request_refresh(app);
                    }
                    // A mutation without a narrower target refreshes every read.
                    app.refresh_reads();
                    super::invalidate(app);
                }
            }
        }
        _ => {}
    }
    derive(app);
}

/// The merged lists, recomputed after every read.
fn derive(app: &mut App) {
    let s = &mut app.custody.page;
    let dismissals = s.dismissals.clone().unwrap_or_default();
    let held_ids: HashSet<String> = s
        .held_items
        .iter()
        .filter_map(|i| i["transaction_id"].as_str().map(str::to_owned))
        .collect();
    s.held = model::held_undone(&dismissals, &held_ids);
    let pairs = s.pairs.clone().unwrap_or_default();
    let parts = s.parts.clone();
    s.gap_holds = s
        .held_items
        .iter()
        .filter(|item| {
            item["actions"].as_array().is_some_and(|acts| {
                acts.iter().any(|a| {
                    a["kind"] == "review_custody_gap"
                        || (a["kind"] == "review_pair"
                            && rest_hold_pair(item, &pairs, parts.as_ref()).is_some())
                })
            })
        })
        .cloned()
        .collect();
    s.moves = model::without_held(
        model::build_linked_moves(&s.lineage, &pairs, &dismissals),
        &s.held,
    );
    s.unlinked = model::recently_unlinked(&s.unlinked_pairs, &dismissals, &s.held, &s.moves);
    s.set_aside = model::set_aside(&dismissals);
}

/// What a pair leaves of its send for a further link (`restOfPair`).
fn rest_of_pair(pairs: &[Pair], parts: Option<&Parts>, pair_id: Option<&str>) -> Option<i128> {
    let pair = pairs.iter().find(|p| Some(p.id.as_str()) == pair_id)?;
    let open = parts?.send.get(&pair.out_transaction_id)?;
    has_open_part(true, Some(open)).then_some(open.unlinked_msat)
}

/// `restHoldPair`: a hold that is the rest of a send the owner linked in part.
fn rest_hold_pair(item: &Value, pairs: &[Pair], parts: Option<&Parts>) -> Option<(Pair, i128)> {
    let ev = &item["evidence"];
    if let Some(blocker) = ev["blocker_code"].as_str()
        && blocker != "reviewed_residual_suspense"
    {
        return None;
    }
    if ev["review_codes"].as_array().is_some_and(|c| {
        c.iter()
            .any(|x| x == "legacy_pair_fee_allocation_review_required")
    }) {
        return None;
    }
    let pair_id = item["actions"]
        .as_array()?
        .iter()
        .find(|a| a["kind"] == "review_pair")?["pair_id"]
        .as_str()?
        .to_owned();
    let pair = pairs.iter().find(|p| p.id == pair_id)?;
    let rest = rest_of_pair(pairs, parts, Some(&pair.id))?;
    Some((pair.clone(), rest))
}

fn gap_of(item: &Value) -> Option<String> {
    item["actions"]
        .as_array()?
        .iter()
        .find(|a| a["kind"] == "review_custody_gap")
        .map(|a| model::s(a, "gap_id"))
}

// ---- Derived state ----------------------------------------------------------------------------

/// The suggestions shown (`useSettledSuggestions`).
pub fn candidates(app: &App) -> Vec<Value> {
    let s = st(app);
    s.frozen
        .clone()
        .or_else(|| s.suggest.clone())
        .unwrap_or_default()
}

fn linked_total(app: &App) -> usize {
    let c = model::count_moves(&st(app).moves);
    (c.automatic + c.reviewed).max(st(app).lineage_total)
}

fn waiting(app: &App) -> usize {
    st(app).held.len() + st(app).gap_holds.len()
}

fn dismissals_incomplete(app: &App) -> bool {
    st(app).dismissals_more
}

fn held_unavailable(app: &App) -> bool {
    let s = st(app);
    s.held_rd.failed || s.dismissals_rd.failed || s.dismissals_more
}

fn linked_partial(app: &App) -> bool {
    st(app).lineage_rd.failed || st(app).pairs_rd.failed
}

fn verdict(app: &App) -> Verdict {
    let s = st(app);
    let loading = s.badges_rd.loading()
        || s.suggest_rd.loading()
        || s.lineage_rd.loading()
        || s.dismissals_rd.loading()
        || s.held_rd.loading()
        || (s.held_more && !s.held_rd.failed);
    let unavailable =
        held_unavailable(app) || s.badges_rd.failed || s.suggest_rd.failed || linked_partial(app);
    let stale = s
        .badges
        .as_ref()
        .is_some_and(|b| b["journals_needs_processing"].as_bool().unwrap_or(false))
        && !super::updating(app);
    model::verdict(
        loading,
        unavailable,
        stale,
        waiting(app),
        candidates(app).len(),
        linked_total(app),
    )
}

fn tab_count(app: &App, tab: StatusTab) -> usize {
    match tab {
        StatusTab::Needs => waiting(app) + candidates(app).len(),
        StatusTab::Linked => linked_total(app),
        StatusTab::Unlinked => st(app).unlinked.len(),
        StatusTab::SetAside => st(app).set_aside.len(),
    }
}

/// The owner's choice holds until that list empties; otherwise what needs them comes first.
fn active_tab(app: &App) -> StatusTab {
    match st(app).tab {
        Some(t) if t == StatusTab::Linked || t == StatusTab::Needs || tab_count(app, t) > 0 => t,
        _ if tab_count(app, StatusTab::Needs) > 0 => StatusTab::Needs,
        _ => StatusTab::Linked,
    }
}

fn saved_view(app: &App) -> Option<&Value> {
    let View::Overview {
        saved_view: Some(id),
        ..
    } = super::current_view(app)
    else {
        return None;
    };
    st(app)
        .views
        .iter()
        .find(|v| v["id"].as_str() == Some(id.as_str()))
}

/// `viewFilter`: the parts of a saved view's filter the suggestions understand.
fn view_filter(view: &Value) -> suggestions::Filter {
    let get = |k: &str| {
        view["filter"][k]
            .as_str()
            .filter(|v| !v.is_empty() && *v != "all")
            .map(str::to_owned)
    };
    let mut f = suggestions::Filter {
        confidence: get("confidence"),
        method: get("method"),
        asset_pair: get("asset_pair"),
        route_pair: get("route_pair"),
        candidate_type: get("candidate_type"),
    };
    if f.candidate_type.is_none() {
        match view["surface"].as_str() {
            Some("swap_candidates") => f.candidate_type = Some("swap".into()),
            Some("transfer_candidates") => f.candidate_type = Some("transfer".into()),
            _ => {}
        }
    }
    f
}

/// Whether the page leaves room for a panel beside its lists.
fn wide(app: &App, cx: &Cx<App>, tk: &Tk) -> bool {
    let w = cx
        .layout_size("tp-list")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or_else(|| kit::page_w(app, cx, tk));
    w >= PANEL_MIN_WIDTH
}

/// The list container's width (`@container/list`): the page less a panel beside it.
fn list_w(app: &App, cx: &Cx<App>, tk: &Tk, panel: Option<f32>) -> f32 {
    let page = cx
        .layout_size("tp-list")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or_else(|| kit::page_w(app, cx, tk));
    match panel {
        Some(p) => page - p - tk.card_gap(),
        None => page,
    }
}

// ---- Poll -------------------------------------------------------------------------------------

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    // `useSettledSuggestions`: freeze while the journals recalculate.
    let updating = super::updating(app);
    if updating && !st(app).was_updating {
        let cur = st(app).suggest.clone().unwrap_or_default();
        stm(app).frozen.get_or_insert(cur);
        changed = true;
    }
    stm(app).was_updating = updating;
    // What could not be read about what waits is said under Needs you, so the page opens there.
    if !st(app).opened_tab_for_unreadable
        && super::owns(app.route.path())
        && verdict(app) == Verdict::Unavailable
    {
        let s = stm(app);
        s.opened_tab_for_unreadable = true;
        if s.tab.is_none() {
            s.tab = Some(StatusTab::Needs);
            changed = true;
        }
    }
    // A toast goes away on its own after 10 s, once the journals are done; a strip stays.
    let strip = st(app).note.as_ref().is_some_and(|n| n.spot.is_some())
        && active_tab(app) == StatusTab::Linked;
    if let Some(n) = &st(app).note {
        if strip || updating {
            let now = app.clock;
            stm(app).note.as_mut().unwrap().since = now;
        } else {
            let until = n.since + TOAST_SECS;
            if app.clock >= until {
                stm(app).note = None;
                changed = true;
            } else {
                tx.wake_at(until);
            }
        }
    }
    changed
}

fn set_note(app: &mut App, kind_spot: Note) {
    let now = app.clock;
    stm(app).note = Some(Note {
        since: now,
        ..kind_spot
    });
}

fn note(kind: NoteKind, spot: Option<Spot>) -> Note {
    Note {
        spot,
        kind,
        since: 0.0,
    }
}

fn spot_of(out_id: &str, in_id: &str) -> Spot {
    Spot {
        out_id: out_id.to_owned(),
        in_id: in_id.to_owned(),
    }
}

// ---- Actions ----------------------------------------------------------------------------------

fn choose_tab(app: &mut App, tab: StatusTab) {
    stm(app).tab = Some(tab);
    unlink::close(app);
}

/// `openLink`: "Link manually" on a transaction, or with nothing picked. One panel at a time:
/// starting a link closes the transfer that was open.
pub fn open_link(app: &mut App, transaction_id: Option<String>) {
    unlink::close(app);
    link::open(
        app,
        LinkStart {
            transaction_id,
            ..Default::default()
        },
    );
}

fn open_link_start(app: &mut App, start: LinkStart) {
    unlink::close(app);
    link::open(app, start);
}

/// `openMove`: a row opens the move's details; its menu goes straight to unlinking.
fn open_move(app: &mut App, mv: LinkedMove, intent: Intent) {
    unlink::open(app, mv, intent);
}

fn mutate(app: &mut App, kind: &str, args: Value, tag: &'static str, note: Option<Note>) {
    stm(app).error = None;
    let mut args = args;
    args["expected_scope"] = app.expected_scope();
    if app.send(kind, args, tag).is_some() {
        let s = stm(app);
        match tag {
            "custody.page.undismiss" => s.undismissing = true,
            "custody.page.repair" => s.repairing = true,
            _ => s.unpairing = true,
        }
        s.pending_note = note;
    }
}

/// `linkBack`: remove an automatic pairing's undo.
fn link_back(app: &mut App, dismissal_id: Option<String>, mv: Option<(String, String)>) {
    let args = match (&dismissal_id, &mv) {
        (Some(id), _) => json!({"dismissal_id": id}),
        (None, Some((o, i))) => json!({"tx_out": o, "tx_in": i}),
        _ => return,
    };
    let spot = mv.map(|(o, i)| spot_of(&o, &i));
    mutate(
        app,
        "ui.transfers.dismissals.delete",
        args,
        "custody.page.undismiss",
        Some(note(NoteKind::LinkedBack, spot)),
    );
}

/// A suggestion set aside comes back to the list; nothing is linked.
fn restore_suggestion(app: &mut App, d: &Dismissal) {
    mutate(
        app,
        "ui.transfers.dismissals.delete",
        json!({"dismissal_id": d.id}),
        "custody.page.undismiss",
        None,
    );
    stm(app).restoring = true;
}

/// "Link again" with the terms the pair had, after the usual preview.
fn link_again(app: &mut App, row: &Unlinked) {
    if let Some(d) = &row.dismissal {
        link_back(
            app,
            Some(d.id.clone()),
            Some((
                row.out_transaction_id.clone(),
                row.in_transaction_id.clone(),
            )),
        );
        return;
    }
    let preset = row.pair.as_ref().map(|p| Preset {
        kind: Some(p.kind.clone()),
        policy: Some(p.policy.clone()),
        terms: Some(link_model::relink_terms(&p.raw)),
    });
    open_link_start(
        app,
        LinkStart {
            send_id: Some(row.out_transaction_id.clone()),
            receipt_id: Some(row.in_transaction_id.clone()),
            relink: true,
            preset,
            ..Default::default()
        },
    );
}

/// `swappedPortion`: whether a pair swapped only part of its send.
fn swapped_portion(listed: Option<&Pair>, removed: Option<&Pair>) -> bool {
    match listed {
        Some(l) => match l.out.unlinked_msat {
            None => removed.is_some_and(|r| r.out_amount.is_some_and(|a| a != 0)),
            Some(left) => l.out.asset != l.inn.asset && left > 0,
        },
        None => removed.is_some_and(|r| r.out_amount.is_some_and(|a| a != 0)),
    }
}

/// The unlink panel or dialog finished (`onUnlinked`).
pub fn on_unlinked(app: &mut App, result: UnlinkResult, differently: bool) {
    unlink::close(app);
    let keep = active_tab(app);
    stm(app).tab = Some(keep);
    let mv = result.mv().clone();
    let spot = Some(spot_of(&mv.out_transaction_id, &mv.in_transaction_id));
    match result {
        UnlinkResult::Automatic { dismissal_id, mv } => {
            set_note(
                app,
                note(
                    NoteKind::Unlinked {
                        dismissal_id,
                        out_id: mv.out_transaction_id.clone(),
                        in_id: mv.in_transaction_id.clone(),
                    },
                    spot,
                ),
            );
            // The dismissal is stored: journals and readiness follow, as for a manual pair.
            super::request_refresh(app);
        }
        UnlinkResult::Pair { pair, mv } => {
            let listed = st(app).pairs.as_ref().and_then(|ps| {
                ps.iter()
                    .find(|p| Some(&p.id) == mv.pair_id.as_ref())
                    .cloned()
            });
            let portion = swapped_portion(listed.as_ref(), pair.as_ref());
            set_note(
                app,
                note(
                    NoteKind::Unpaired {
                        pair,
                        mv,
                        portion,
                        listed,
                    },
                    spot,
                ),
            );
            super::request_refresh(app);
        }
    }
    if differently {
        open_link(app, Some(mv.out_transaction_id.clone()));
    }
}

/// "Link manually" made its link (`onLinked`).
pub fn on_linked(app: &mut App, r: LinkedResult) {
    let keep = active_tab(app);
    stm(app).tab = Some(keep);
    link::close(app);
    let spot = Some(spot_of(&r.send_id, &r.receipt_id));
    set_note(
        app,
        note(
            NoteKind::Linked {
                pair_id: r.pair_id,
                send_id: r.send_id,
            },
            spot,
        ),
    );
    super::request_refresh(app);
}

/// A suggestion was decided on the suggestions card (`onDecided`).
pub fn on_suggestions_decided(app: &mut App) {
    stm(app).tab = Some(StatusTab::Needs);
    super::request_refresh(app);
    read_pairs(app);
    read_dismissals(app);
}

/// The rules sheet opened a saved view (`onOpenView`).
pub fn open_saved_view(app: &mut App, view_id: &str) {
    rules::close(app);
    choose_tab(app, StatusTab::Needs);
    super::select(
        app,
        Search {
            saved_view: Some(view_id.to_owned()),
            ..Search::default()
        },
    );
}

/// The saved view's chip was removed (`onClearFilter`).
pub fn clear_saved_view(app: &mut App) {
    super::select(app, Search::default());
}

fn undo_link(app: &mut App, pair_id: &str, spot: Option<Spot>) {
    mutate(
        app,
        "ui.transfers.unpair",
        json!({"pair_id": pair_id}),
        "custody.page.unpair",
        Some(note(NoteKind::LinkUndone, spot)),
    );
}

/// `undoUnpair`: Undo right after unlinking restores the pair with its terms; a swapped
/// portion goes through Link again's preview instead.
fn undo_unpair(app: &mut App, n: &NoteKind) {
    let NoteKind::Unpaired {
        pair,
        mv,
        portion,
        listed,
    } = n
    else {
        return;
    };
    if let (Some(p), false) = (pair, portion) {
        let mut args = json!({
            "tx_out": mv.out_transaction_id,
            "tx_in": mv.in_transaction_id,
            "kind": p.kind,
            "policy": p.policy,
            "pair_source": p.pair_source.clone().unwrap_or_else(|| "manual".into()),
        });
        if let Some(notes) = &p.notes {
            args["notes"] = json!(notes);
        }
        let spot = Some(spot_of(&mv.out_transaction_id, &mv.in_transaction_id));
        mutate(
            app,
            "ui.transfers.pair",
            args,
            "custody.page.repair",
            Some(note(NoteKind::LinkedBack, spot)),
        );
        return;
    }
    stm(app).note = None;
    let mut raw = listed.as_ref().map(|l| l.raw.clone()).unwrap_or(json!({}));
    if let Some(p) = pair {
        raw["out_amount"] = p.raw["out_amount"].clone();
        raw["swap_fee_msat"] = p.raw["swap_fee_msat"].clone();
    }
    let preset = Preset {
        kind: pair
            .as_ref()
            .map(|p| p.kind.clone())
            .or_else(|| mv.pair_kind.clone()),
        policy: pair
            .as_ref()
            .map(|p| p.policy.clone())
            .or_else(|| mv.pair_policy.clone()),
        terms: Some(link_model::relink_terms(&raw)),
    };
    open_link_start(
        app,
        LinkStart {
            send_id: Some(mv.out_transaction_id.clone()),
            receipt_id: Some(mv.in_transaction_id.clone()),
            relink: true,
            preset: Some(preset),
            ..Default::default()
        },
    );
}

fn note_pending(app: &App) -> bool {
    let s = st(app);
    s.undismissing || s.repairing || s.unpairing || super::processing(app)
}

/// Escape closes the page's open menu; true when it did.
pub fn escape_menu(app: &mut App) -> bool {
    stm(app).popup.take().is_some()
}

fn close_popup(a: &mut App) {
    a.custody.page.popup = None;
}

fn toggle(app: &mut App, p: Popup) {
    let s = stm(app);
    s.popup = if s.popup.as_ref() == Some(&p) {
        None
    } else {
        Some(p)
    };
}

// ---- View -------------------------------------------------------------------------------------

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let shell = col()
        .key("transfers")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    match super::current_view(app) {
        View::Advanced { tab, gap } => {
            shell.child(advanced(app, cx, tk, layer, tab, gap.as_deref()))
        }
        View::Overview { .. } => shell.child(overview(app, cx, tk, layer)),
    }
}

/// `SecondarySection` + `AdvancedCustodyTools`.
fn advanced(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    tab: Tab,
    gap: Option<&str>,
) -> El {
    let back = kit::btn35(
        tk,
        "tp-back",
        Var::Ghost,
        Some(&I::ARROW_LEFT),
        &t("review:transfersPage.back"),
        true,
    )
    .h(tk.r(1.75))
    .px(tk.r(0.5))
    .gap(tk.r(0.25))
    .ml(-tk.r(0.5))
    .on_press(|a: &mut App| super::select(a, Search::default()));
    let head = col()
        .w_full()
        .gap(tk.r(0.375))
        .child(row().child(back))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(t("review:transfersPage.advanced.title"))
                        .font(tk.tlg(600.0).lh(tk.r(1.4)))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(kit::dev_badge(tk)),
        )
        .child(
            text(t("review:transfersPage.advanced.description"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap()
                .max_w(tk.r(48.0)),
        );
    // shadcn `TabsList`: `bg-muted h-9 rounded-lg p-[3px]`, the active trigger `bg-background`.
    let mut tabs = row()
        .h(tk.r(2.25))
        .padding(3.0)
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("tablist");
    for (tb, key) in [
        (Tab::Gaps, "gaps"),
        (Tab::Components, "components"),
        (Tab::Coverage, "coverage"),
    ] {
        let active = tb == tab;
        let label = t(&format!("review:transfersPage.advanced.tabs.{key}"));
        let mut b = row()
            .key(format!("tp-adv-{key}"))
            .h_full()
            .px(tk.r(0.5))
            .center()
            .radius(tk.r(0.375))
            .role("tab")
            .checked(active)
            .label(label.clone())
            .focusable(true)
            .on_press(move |a: &mut App| {
                super::select(
                    a,
                    Search {
                        advanced: true,
                        tab: Some(tb),
                        ..Search::default()
                    },
                )
            })
            .child(
                text(label)
                    .font(tk.tsm(500.0))
                    .color(if active { tk.fg } else { tk.muted_fg }),
            );
        if active {
            b = b
                .fill(if tk.dark {
                    tk.input.alpha(tk.input.a * 0.3)
                } else {
                    tk.background
                })
                .border(
                    1.0,
                    if tk.dark {
                        tk.input
                    } else {
                        Color::TRANSPARENT
                    },
                )
                .box_shadow(rune::Shadow {
                    y: 1.0,
                    blur: 3.0,
                    color: Color::BLACK.alpha(0.1),
                    ..Default::default()
                });
        }
        tabs = tabs.child(b);
    }
    let body = match tab {
        Tab::Components => components::resolver(app, cx, tk, layer),
        Tab::Coverage => gaps::coverage(app, cx, tk, layer),
        Tab::Gaps => gaps::content(app, cx, tk, layer, gap),
    };
    col().w_full().gap(tk.r(1.0)).child(head).child(
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(row().child(tabs))
            .child(body),
    )
}

fn overview(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let v = verdict(app);
    let tab = active_tab(app);
    let is_wide = wide(app, cx, tk);
    let link_panel = is_wide && link::is_open(app);
    let panel_move = is_wide && !link_panel && unlink::current(app).is_some();
    let panel_w = if link_panel {
        Some(tk.r(27.0))
    } else if panel_move {
        Some(tk.r(22.0))
    } else {
        None
    };
    let lw = list_w(app, cx, tk, panel_w);

    let mut page = col().w_full().gap(tk.section_gap());
    page = page.child(header(app, tk, layer, v, tab));
    page = page.child(stats(app, cx, tk, v == Verdict::Loading));
    if let Some(e) = &st(app).error {
        page = page.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }

    let mut lists = col().min_w(0.0).grow(1.0).gap(tk.card_gap());
    match tab {
        StatusTab::Needs => {
            let unavailable = held_unavailable(app);
            if unavailable {
                let s = st(app);
                lists = lists.child(held_unavailable_card(
                    tk,
                    dismissals_incomplete(app) && !s.held_rd.failed && !s.dismissals_rd.failed,
                ));
            }
            if waiting(app) > 0 {
                lists = lists.child(sort_out(app, cx, tk, layer));
            }
            let s = st(app);
            let no_suggestions =
                !s.suggest_rd.loading() && !s.suggest_rd.failed && candidates(app).is_empty();
            let view = saved_view(app);
            let showing_view = matches!(
                super::current_view(app),
                View::Overview {
                    saved_view: Some(_),
                    ..
                }
            );
            if no_suggestions && !showing_view && !suggestions::undo_showing(app) {
                if waiting(app) == 0 && !unavailable {
                    lists = lists.child(suggestions_empty(app, tk));
                }
            } else {
                let filter = view.map(view_filter);
                let label = view.and_then(|v| v["name"].as_str()).map(str::to_owned);
                lists = lists.child(suggestions::card(
                    app,
                    cx,
                    tk,
                    layer,
                    filter.as_ref(),
                    label.as_deref(),
                ));
            }
        }
        StatusTab::Unlinked => lists = lists.child(unlinked_card(app, cx, tk, layer, lw)),
        StatusTab::SetAside => lists = lists.child(set_aside_card(app, cx, tk, layer, lw)),
        StatusTab::Linked => lists = lists.child(linked_card(app, cx, tk, layer, lw, panel_move)),
    }
    let mut content = row()
        .key("tp-list")
        .w_full()
        .gap(tk.card_gap())
        .items_start()
        .child(lists);
    if panel_move {
        content = content.child(
            col()
                .w(tk.r(22.0))
                .min_w(tk.r(22.0))
                .sticky_top(tk.r(1.0))
                .child(unlink::panel(app, cx, tk, layer)),
        );
    } else if link_panel {
        content = content.child(
            col()
                .w(tk.r(27.0))
                .min_w(tk.r(27.0))
                .sticky_top(tk.r(1.0))
                .child(link::panel(app, cx, tk, layer)),
        );
    }
    page = page.child(content);

    // Dialogs: a move opened while a link is in progress in the panel opens over it.
    if unlink::current(app).is_some() && !panel_move {
        unlink::dialog(app, cx, tk, layer, !link::is_open(app));
    }
    if link::is_open(app) && !link_panel {
        link::dialog(app, cx, tk, layer);
    }
    rules::sheet(app, cx, tk, layer);
    // A note the Linked list shows at its row is not repeated as a toast.
    let strip = st(app).note.as_ref().is_some_and(|n| n.spot.is_some()) && tab == StatusTab::Linked;
    if let Some(n) = &st(app).note
        && !strip
    {
        layer.push(
            row()
                .key("tp-toast-layer")
                .absolute(0.0, 0.0)
                .w_full()
                .h_full()
                .px(tk.r(1.0))
                .pb(tk.r(1.0))
                .justify_center()
                .items_end()
                .role("status")
                .child(
                    row()
                        .key("transfers-toast")
                        .max_w(tk.r(42.0))
                        .px(tk.r(1.0))
                        .py(tk.r(0.625))
                        .radius(tk.r(0.5))
                        .fill(tk.popover)
                        .border(1.0, tk.border)
                        .box_shadow(rune::Shadow {
                            y: 10.0,
                            blur: 15.0,
                            spread: -3.0,
                            color: Color::BLACK.alpha(0.1),
                            ..Default::default()
                        })
                        .child(note_view(app, tk, n, false)),
                ),
        );
    }
    page
}

/// The page's first row: the status selector, the verdict for screen readers, the stale pill,
/// "updating" or the caveat; Link manually and the menu on the right (`TransfersHeader`).
fn header(app: &App, tk: &Tk, layer: &mut Vec<El>, v: Verdict, tab: StatusTab) -> El {
    let updating = super::updating(app);
    let summary = match v {
        Verdict::Loading => t("review:transfersPage.verdict.loading"),
        Verdict::Stale => t("review:transfersPage.verdict.staleBody"),
        Verdict::Unavailable => t("review:transfersPage.verdict.unavailableBody"),
        Verdict::SortOut { total } => tc("review:transfersPage.verdict.sortOutTitle", total, &[]),
        Verdict::Settled => t("review:transfersPage.verdict.settledTitle"),
        Verdict::Empty => t("review:transfersPage.verdict.emptyTitle"),
    };
    let mut left = row()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.75))
        .center_y()
        .flex_wrap();
    left = left.child(if v == Verdict::Loading {
        kit::skeleton(tk, tk.r(18.0), tk.r(2.0)).radius(tk.r(0.5))
    } else {
        status_tabs(app, tk, tab)
    });
    left = left.child(
        el().size(0.0, 0.0)
            .role("status")
            .label(summary)
            .live_region(rune::LiveRegion::Polite),
    );
    left = left.child(match v {
        Verdict::Stale => kit::readiness_pill(
            tk,
            "tp-recalc",
            kit::PillTone::Warning,
            &I::REFRESH_CW,
            &t("review:transfersPage.verdict.recalculate"),
            &t("review:transfersPage.verdict.staleBody"),
            updating,
            Some(&t("review:transfersPage.verdict.updating")),
        )
        .on_press(super::request_refresh),
        Verdict::Unavailable => kit::readiness_pill(
            tk,
            "tp-retry-pill",
            kit::PillTone::Warning,
            &I::TRIANGLE_ALERT,
            &t("review:transfersPage.verdict.unavailableTitle"),
            &t("review:transfersPage.verdict.unavailableBody"),
            false,
            None,
        )
        .on_press(retry_reads),
        _ if updating => row()
            .gap(tk.r(0.375))
            .center_y()
            .role("status")
            .child(kit::spinner(tk, tk.r(0.75), tk.muted_fg))
            .child(
                text(t("review:transfersPage.verdict.updating"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        _ => row()
            .min_w(0.0)
            .gap(tk.r(0.25))
            .center_y()
            .child(
                text(t("review:transfersPage.verdict.caveat"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            )
            .child(
                row()
                    .key("tp-caveat-why")
                    .size(tk.r(1.0), tk.r(1.0))
                    .center()
                    .radius(tk.r(0.5))
                    .role("button")
                    .label(t("review:transfersPage.verdict.caveatWhyLabel"))
                    .help(t("review:transfersPage.verdict.caveatWhy"))
                    .focusable(true)
                    .hover_group()
                    .on_press(|_: &mut App| {})
                    .child(ic(&I::CIRCLE_HELP, tk.r(0.875), tk.muted_fg).group_color(tk.fg)),
            ),
    });
    let link_btn = kit::btn(
        tk,
        "tp-link-manually",
        Var::Outline,
        Some(&I::LINK_2),
        &t("review:transfersPage.linkManually"),
        true,
    )
    .gap(tk.r(0.5))
    .on_press(|a: &mut App| open_link(a, None));
    let menu_btn = kit::icon_btn(
        tk,
        "tp-menu",
        Var::Outline,
        tk.r(2.0),
        &I::ELLIPSIS,
        tk.r(1.0),
        t("review:transfersPage.menu.label"),
    )
    .menu_trigger(st(app).popup == Some(Popup::Menu), |a: &mut App| {
        toggle(a, Popup::Menu)
    });
    if st(app).popup == Some(Popup::Menu) {
        let dev = app.prefs.developer_tools;
        let entries = vec![
            Entry::Item {
                key: "tp-menu-recheck".into(),
                glyph: Some(&I::REFRESH_CW),
                label: t("review:transfersPage.menu.recheck"),
                hint: Some(t("review:transfersPage.menu.recheckHint")),
                badge: false,
                disabled: updating,
                title: None,
                on: std::rc::Rc::new(super::request_refresh),
            },
            Entry::Item {
                key: "tp-menu-rules".into(),
                glyph: Some(&I::SETTINGS_2),
                label: t("review:transfersPage.menu.rules"),
                hint: Some(t("review:transfersPage.menu.rulesHint")),
                badge: false,
                disabled: false,
                title: None,
                on: std::rc::Rc::new(rules::open),
            },
            Entry::Sep,
            Entry::Item {
                key: "tp-menu-advanced".into(),
                glyph: Some(&I::WRENCH),
                label: t("review:transfersPage.menu.advanced"),
                hint: Some(t(if dev {
                    "review:transfersPage.menu.advancedHint"
                } else {
                    "review:transfersPage.menu.advancedLocked"
                })),
                badge: true,
                disabled: !dev,
                title: (!dev).then(|| t("nav:devLocked")),
                on: std::rc::Rc::new(|a: &mut App| {
                    super::select(
                        a,
                        Search {
                            advanced: true,
                            ..Search::default()
                        },
                    )
                }),
            },
        ];
        kit::menu(
            tk,
            layer,
            "tp-menu-content",
            "tp-menu",
            Placement::BottomEnd,
            Some(tk.r(18.0)),
            entries,
            close_popup,
        );
    }
    row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(left)
        .child(
            row()
                .gap(tk.r(0.75))
                .center_y()
                .child(link_btn)
                .child(menu_btn),
        )
}

fn retry_reads(app: &mut App) {
    let s = st(app).clone_flags();
    if s.0 {
        read_badges(app);
    }
    if s.1 {
        read_held(app);
    }
    if s.2 {
        read_dismissals(app);
    }
    if s.3 {
        read_suggest(app);
    }
    if s.4 {
        read_lineage(app, false);
    }
    if s.5 {
        read_pairs(app);
    }
}

impl State {
    fn clone_flags(&self) -> (bool, bool, bool, bool, bool, bool) {
        (
            self.badges_rd.failed,
            self.held_rd.failed,
            self.dismissals_rd.failed,
            self.suggest_rd.failed,
            self.lineage_rd.failed,
            self.pairs_rd.failed,
        )
    }
}

/// `StatusTabs`: one track, the chosen segment raised; a list with nothing in it keeps no tab
/// (except the first two).
fn status_tabs(app: &App, tk: &Tk, tab: StatusTab) -> El {
    let mut track = row()
        .key("transfers-tabs")
        .h(tk.r(2.0))
        .padding(tk.r(0.125))
        .gap(tk.r(0.25))
        .center_y()
        .radius(tk.r(0.5))
        .fill(tk.muted)
        .role("group")
        .label(t("review:transfersPage.tabs.label"));
    for key in StatusTab::ALL {
        let count = tab_count(app, key);
        if !(key == StatusTab::Needs || key == StatusTab::Linked || count > 0) {
            continue;
        }
        let active = tab == key;
        let attention = key == StatusTab::Needs && count > 0;
        let label = t(&format!("review:transfersPage.tabs.{}", key.key()));
        let mut chip = row().px(tk.r(0.375)).radius(999.0).child(
            text(crate::format::count(count))
                .font(tk.t2xs(400.0).features("'tnum' 1"))
                .color(if attention {
                    kit::tw(if tk.dark { "amber-200" } else { "amber-800" })
                } else if active {
                    tk.fg
                } else {
                    tk.muted_fg
                }),
        );
        if attention {
            chip = chip.fill(kit::tw("amber-500").alpha(0.2));
        } else if active {
            chip = chip.fill(tk.muted);
        }
        let mut b = row()
            .key(format!("tp-tab-{}", key.key()))
            .h(tk.r(1.75))
            .px(tk.r(0.75))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .role("button")
            .checked(active)
            .label(format!("{label} {}", crate::format::count(count)))
            .focusable(true)
            .on_press(move |a: &mut App| choose_tab(a, key))
            .child(
                text(label)
                    .font(tk.tsm(500.0))
                    .color(if active { tk.fg } else { tk.muted_fg }),
            )
            .child(chip);
        if active {
            b = b.fill(tk.card).box_shadow(rune::Shadow {
                y: 1.0,
                blur: 2.0,
                color: Color::BLACK.alpha(0.05),
                ..Default::default()
            });
        } else {
            b = b.hover_group();
        }
        track = track.child(b);
    }
    track
}

/// `movedTotals`: bitcoin on both sides, the moves whose amounts add up to "moved".
fn moved_totals(moves: &[LinkedMove]) -> (i128, f64, usize, usize) {
    let linked: Vec<&LinkedMove> = moves
        .iter()
        .filter(|m| m.source != Source::Undone && !m.different_asset && model::is_bitcoin(&m.asset))
        .collect();
    let priced: Vec<&&LinkedMove> = linked.iter().filter(|m| m.amount_fiat.is_some()).collect();
    let amount = linked.iter().map(|m| m.amount_msat).sum();
    let fiat = priced.iter().map(|m| m.amount_fiat.unwrap_or(0.0)).sum();
    (amount, fiat, linked.len() - priced.len(), linked.len())
}

/// Per quarter, from the first linked move's to the last's (at most 32), `f` of the moves in it:
/// the trend under a stat tile.
fn move_trend(moves: &[LinkedMove], f: impl Fn(&LinkedMove) -> f64) -> Vec<f64> {
    let quarter = |m: &LinkedMove| -> Option<i64> {
        let d = crate::format::Day::parse(m.occurred_at.as_deref()?)?;
        Some(i64::from(d.y) * 4 + i64::from((d.m - 1) / 3))
    };
    let keyed: Vec<(i64, f64)> = moves
        .iter()
        .filter(|m| m.source != Source::Undone)
        .filter_map(|m| Some((quarter(m)?, f(m))))
        .collect();
    let (Some(lo), Some(hi)) = (
        keyed.iter().map(|k| k.0).min(),
        keyed.iter().map(|k| k.0).max(),
    ) else {
        return Vec::new();
    };
    let lo = lo.max(hi - 31);
    let mut out = vec![0.0; (hi - lo + 1) as usize];
    for (q, v) in keyed.into_iter().filter(|k| k.0 >= lo) {
        out[(q - lo) as usize] += v;
    }
    out
}

/// The stat strip (`TransfersStats`): linked transfers and what moved, as the home screen's stat
/// tiles (an owner request), each with its trend by quarter.
fn stats(app: &App, cx: &Cx<App>, tk: &Tk, loading: bool) -> El {
    use crate::overview::grid::stat::{Stat, row_of};
    let label = t("review:transfersPage.stats.label");
    if loading {
        let stats = vec![
            Stat::loading(
                "tp-stat-linked",
                &I::ARROW_LEFT_RIGHT,
                t("review:transfersPage.stats.linked"),
            ),
            Stat::loading(
                "tp-stat-moved",
                &I::BITCOIN,
                t("review:transfersPage.stats.moved"),
            ),
        ];
        return row_of(app, cx, tk, "tp-stats", label, stats);
    }
    let moves = &st(app).moves;
    let counts = model::count_moves(moves);
    let (amount, fiat, unpriced, count) = moved_totals(moves);
    let btc = app.prefs.currency == Currency::Btc;
    let a = counts.automatic.to_string();
    let r = counts.reviewed.to_string();
    let linked = Stat::new(
        "tp-stat-linked",
        &I::ARROW_LEFT_RIGHT,
        t("review:transfersPage.stats.linked"),
        crate::format::count(linked_total(app)),
    )
    .line(tr(
        "review:transfersPage.stats.linkedMeta",
        &[("automatic", &a), ("reviewed", &r)],
    ))
    .spark(move_trend(moves, |_| 1.0), tk.muted_fg)
    .button(t("review:transfersPage.stats.linkedAria"), |a: &mut App| {
        a.custody.page.filter = Some(Filter::All);
        a.custody.page.shown = LINKED_PAGE;
        choose_tab(a, StatusTab::Linked);
    });
    let value = if btc {
        model::msat_btc(amount)
    } else {
        kit::money(app, fiat)
    };
    let meta = if !btc && unpriced > 0 {
        let u = unpriced.to_string();
        tc(
            "review:transfersPage.stats.movedMetaUnpriced",
            count,
            &[("unpriced", &u)],
        )
    } else {
        tc("review:transfersPage.stats.movedMeta", count, &[])
    };
    let moved_trend = move_trend(moves, |m| {
        if m.different_asset || !model::is_bitcoin(&m.asset) {
            0.0
        } else if btc {
            m.amount_msat as f64 / 1e11
        } else {
            m.amount_fiat.unwrap_or(0.0)
        }
    });
    let moved = Stat::new(
        "tp-stat-moved",
        &I::BITCOIN,
        t("review:transfersPage.stats.moved"),
        value,
    )
    .sensitive(true)
    .line(meta)
    .spark(moved_trend, kit::tw("orange-500"));
    row_of(app, cx, tk, "tp-stats", label, vec![linked, moved])
}

/// The holds could not be read: what waits on the owner is unknown, not nothing.
fn held_unavailable_card(tk: &Tk, incomplete: bool) -> El {
    let mut r = row()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.625))
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), kit::amber_icon(tk)))
        .child(
            text(t(if incomplete {
                "review:transfersPage.sortOut.incomplete"
            } else {
                "review:transfersPage.sortOut.unavailable"
            }))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap()
            .min_w(0.0)
            .grow(1.0),
        );
    if !incomplete {
        r = r.child(
            kit::btn_xs(
                tk,
                "tp-held-retry",
                Var::Ghost,
                Some(&I::REFRESH_CW),
                &t("review:transfersPage.verdict.retry"),
                true,
            )
            .on_press(retry_reads),
        );
    }
    kit::surface(tk)
        .key("transfers-held-unavailable")
        .w_full()
        .role("alert")
        .child(r)
}

/// Nothing to confirm, in one line (`SuggestionsEmpty`).
fn suggestions_empty(app: &App, tk: &Tk) -> El {
    let updating = super::updating(app);
    let mut glyph = ic(&I::REFRESH_CW, tk.r(0.875), tk.muted_fg);
    if updating {
        glyph = glyph.looping(rune::LoopSpec::rotation(0.9));
    }
    let check = row()
        .key("tp-check-again")
        .h(tk.r(1.75))
        .px(tk.r(0.625))
        .gap(tk.r(0.375))
        .center_y()
        .radius(tk.r(0.375))
        .role("button")
        .label(t("review:transfersPage.empty.checkAgain"))
        .focusable(!updating)
        .disabled(updating)
        .opacity(if updating { 0.5 } else { 1.0 })
        .hover(if tk.dark {
            tk.accent.alpha(0.5)
        } else {
            tk.accent
        })
        .on_press(super::request_refresh)
        .child(glyph)
        .child(
            text(t("review:transfersPage.empty.checkAgain"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        );
    kit::surface(tk)
        .key("transfers-suggestions-empty")
        .w_full()
        .role("region")
        .label(t("review:inbox.transfers.title"))
        .child(
            row()
                .w_full()
                .px(tk.card_pad())
                .py(tk.r(0.625))
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(&I::CIRCLE_CHECK, tk.r(1.0), kit::emerald_icon(tk)))
                .child(
                    text(t("review:transfersPage.empty.suggestionsTitle"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(t("review:transfersPage.empty.suggestionsMissed"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .truncate()
                        .min_w(0.0)
                        .shrink()
                        .help_when_truncated(t("review:transfersPage.empty.suggestionsMissed")),
                )
                .child(kit::row_hint(
                    tk,
                    "tp-empty-checks",
                    t("review:transfersPage.empty.suggestionsChecks"),
                    &I::INFO,
                    tk.muted_fg,
                ))
                .child(el().grow(1.0))
                .child(check),
        )
}

/// A row's "more" menu: the ways to its transactions (`RowMenu` / `TransactionsMenuItems`).
fn transactions_entries(out_id: &str, in_id: &str, say_what: bool) -> Vec<Entry> {
    let (o, i) = (out_id.to_owned(), in_id.to_owned());
    let (o2, i2) = (o.clone(), i.clone());
    let mut e = vec![
        Entry::item(
            format!("tp-rm-sent-{o}"),
            Some(&I::ARROW_UP_RIGHT),
            t("review:transfersPage.rowMenu.showSent"),
            move |a: &mut App| super::show_transaction(a, &o, None),
        ),
        Entry::item(
            format!("tp-rm-recv-{i}"),
            Some(&I::ARROW_UP_RIGHT),
            t("review:transfersPage.rowMenu.showReceived"),
            move |a: &mut App| super::show_transaction(a, &i, None),
        ),
    ];
    if say_what {
        e.push(Entry::Sep);
        e.push(Entry::item(
            format!("tp-rm-saysent-{o2}"),
            Some(&I::TAG),
            t("review:transfersPage.rowMenu.saySent"),
            move |a: &mut App| super::show_transaction(a, &o2, Some("tax")),
        ));
        e.push(Entry::item(
            format!("tp-rm-sayrecv-{i2}"),
            Some(&I::TAG),
            t("review:transfersPage.rowMenu.sayReceived"),
            move |a: &mut App| super::show_transaction(a, &i2, Some("tax")),
        ));
    }
    e
}

#[allow(clippy::too_many_arguments)]
fn row_menu(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: String,
    route_text: &str,
    out_id: &str,
    in_id: &str,
    say_what: bool,
) -> El {
    row_menu_with(
        app,
        tk,
        layer,
        key,
        route_text,
        out_id,
        in_id,
        say_what,
        Vec::new(),
    )
}

/// [`row_menu`] with the row's own secondary actions first (one visible action per row; the
/// rest live here).
#[allow(clippy::too_many_arguments)]
fn row_menu_with(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: String,
    route_text: &str,
    out_id: &str,
    in_id: &str,
    say_what: bool,
    mut lead: Vec<Entry>,
) -> El {
    let anchor = format!("tp-rowmenu-{key}");
    let open = st(app).popup == Some(Popup::Held(key.clone()));
    if open {
        if !lead.is_empty() {
            lead.push(Entry::Sep);
        }
        lead.extend(transactions_entries(out_id, in_id, say_what));
        kit::menu(
            tk,
            layer,
            &format!("{anchor}-content"),
            &anchor,
            Placement::BottomEnd,
            None,
            lead,
            close_popup,
        );
    }
    let k = key.clone();
    kit::icon_btn(
        tk,
        anchor,
        Var::Ghost,
        tk.r(1.75),
        &I::ELLIPSIS,
        tk.r(1.0),
        tr(
            "review:transfersPage.rowMenu.label",
            &[("route", route_text)],
        ),
    )
    .menu_trigger(open, move |a: &mut App| toggle(a, Popup::Held(k.clone())))
}

/// What waits on the owner, asked once (`SortOutCard`).
fn sort_out(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let s = st(app);
    let count = s.held.len() + s.gap_holds.len();
    let head = row()
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.875))
        .gap(tk.r(1.0))
        .center_y()
        .child(
            row()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(t("review:transfersPage.sortOut.title"))
                        .font(tk.tbase(500.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(kit::count_chip(tk, count.to_string())),
        )
        .child(
            text(t("review:transfersPage.sortOut.reports"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    let mut list = col().w_full();
    let add = |list: El, item: El| list.child(kit::rule(tk)).child(item);
    for d in &s.held {
        list = add(list, held_question(app, tk, layer, d));
    }
    let pairs = s.pairs.clone().unwrap_or_default();
    let mut gap_only = vec![];
    for item in &s.gap_holds {
        match rest_hold_pair(item, &pairs, s.parts.as_ref()) {
            Some((pair, rest)) => {
                let mv = s
                    .moves
                    .iter()
                    .find(|m| m.pair_id.as_deref() == Some(pair.id.as_str()))
                    .cloned();
                list = add(
                    list,
                    rest_hold(app, tk, layer, &pair, rest, mv, gap_of(item)),
                );
            }
            None => gap_only.push(item),
        }
    }
    for item in gap_only {
        list = add(list, gap_hold(app, tk, item));
    }
    kit::surface(tk)
        .key("transfers-sort-out")
        .w_full()
        .role("region")
        .label(t("review:transfersPage.sortOut.title"))
        .child(head)
        .child(list)
}

fn held_question(app: &App, tk: &Tk, layer: &mut Vec<El>, d: &Dismissal) -> El {
    let hidden = app.prefs.hide_sensitive();
    let pending = st(app).undismissing || super::updating(app);
    let amount = model::msat_in(d.out.amount_msat, &d.out.asset);
    let q = tr(
        "review:transfersPage.held.question",
        &[("amount", &amount), ("from", &d.out.wallet)],
    );
    let why = tr("review:transfersPage.held.why", &[("to", &d.inn.wallet)]);
    let id = d.id.clone();
    let id2 = d.out_transaction_id.clone();
    let mut answers = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .role("group")
        .label(t("review:transfersPage.held.answers"))
        .child(
            kit::btn(
                tk,
                format!("tp-held-back-{}", d.id),
                Var::Default,
                Some(&I::LINK_2),
                &tr(
                    "review:transfersPage.held.linkBack",
                    &[("to", &d.inn.wallet)],
                ),
                !pending,
            )
            .gap(tk.r(0.5))
            .on_press(move |a: &mut App| link_back(a, Some(id.clone()), None)),
        )
        .child(
            kit::btn(
                tk,
                format!("tp-held-diff-{}", d.id),
                Var::Outline,
                Some(&I::ARROW_LEFT_RIGHT),
                &t("review:transfersPage.held.linkDifferently"),
                !pending,
            )
            .gap(tk.r(0.5))
            .on_press(move |a: &mut App| open_link(a, Some(id2.clone()))),
        );
    if app.prefs.developer_tools {
        answers = answers.child(
            kit::btn(
                tk,
                format!("tp-held-desc-{}", d.id),
                Var::Ghost,
                Some(&I::WRENCH),
                &t("review:transfersPage.held.describe"),
                true,
            )
            .gap(tk.r(0.5))
            .child(kit::dev_badge(tk))
            .on_press(|a: &mut App| {
                super::select(
                    a,
                    Search {
                        advanced: true,
                        tab: Some(Tab::Components),
                        ..Search::default()
                    },
                )
            }),
        );
    }
    let route_text = tr(
        "review:inbox.transfers.route",
        &[("from", &d.out.wallet), ("to", &d.inn.wallet)],
    );
    question_row(
        tk,
        format!("transfers-held-move-{}", d.id),
        kit::amber_box(tk, &I::CIRCLE_HELP),
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(sensitive(q, tk.tsm(500.0), tk.fg, hidden).wrap())
            .child(
                row()
                    .w_full()
                    .flex_wrap()
                    .gap(tk.r(0.25))
                    .child(sensitive(why, tk.tsm(400.0), tk.muted_fg, hidden).wrap())
                    .child(
                        text(t("review:transfersPage.held.ask"))
                            .font(tk.tsm(400.0))
                            .color(tk.fg),
                    ),
            )
            .child(
                row()
                    .gap(tk.r(0.375))
                    .center_y()
                    .child(
                        text(tr(
                            "review:transfersPage.held.sent",
                            &[("date", &date_part(d.out.occurred_at.as_deref()))],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    )
                    .child(
                        text("·")
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .a11y_hidden(),
                    )
                    .child(
                        text(tr(
                            "review:transfersPage.held.unlinkedOn",
                            &[("date", &date_part(Some(&d.created_at)))],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    ),
            ),
        answers,
        None,
        Some(row_menu(
            app,
            tk,
            layer,
            format!("held-{}", d.id),
            &route_text,
            &d.out_transaction_id,
            &d.in_transaction_id,
            false,
        )),
    )
}

/// One question row: the icon box, the text, the answers, an optional note and the menu.
fn question_row(
    tk: &Tk,
    key: String,
    glyph: El,
    body: El,
    answers: El,
    foot: Option<El>,
    menu: Option<El>,
) -> El {
    let mut main = col()
        .min_w(0.0)
        .grow(1.0)
        .gap(tk.r(0.75))
        .child(body)
        .child(answers);
    if let Some(f) = foot {
        main = main.child(f);
    }
    let mut r = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .child(glyph)
        .child(main);
    if let Some(m) = menu {
        r = r.child(m);
    }
    col()
        .key(key)
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(1.0))
        .child(r)
}

fn rest_hold(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    pair: &Pair,
    rest: i128,
    mv: Option<LinkedMove>,
    gap: Option<String>,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let dev = app.prefs.developer_tools;
    let send = pair.out_transaction_id.clone();
    let mut answers = row()
        .gap(tk.r(0.5))
        .flex_wrap()
        .role("group")
        .label(t("review:transfersPage.held.answers"))
        .child(
            kit::btn(
                tk,
                format!("tp-rest-link-{}", pair.id),
                Var::Default,
                Some(&I::LINK_2),
                &t("review:transfersPage.restHold.linkRest"),
                true,
            )
            .gap(tk.r(0.5))
            .on_press(move |a: &mut App| open_link(a, Some(send.clone()))),
        );
    if let Some(m) = mv {
        answers = answers.child(
            kit::btn(
                tk,
                format!("tp-rest-show-{}", pair.id),
                Var::Outline,
                Some(&I::INFO),
                &t("review:transfersPage.restHold.show"),
                true,
            )
            .gap(tk.r(0.5))
            .on_press(move |a: &mut App| open_move(a, m.clone(), Intent::Details)),
        );
    }
    if dev && let Some(g) = gap {
        answers = answers.child(
            kit::btn(
                tk,
                format!("tp-rest-review-{}", pair.id),
                Var::Ghost,
                Some(&I::WRENCH),
                &t("review:transfersPage.gapHold.review"),
                true,
            )
            .gap(tk.r(0.5))
            .child(kit::dev_badge(tk))
            .on_press(move |a: &mut App| {
                super::select(
                    a,
                    Search {
                        advanced: true,
                        tab: Some(Tab::Gaps),
                        gap: Some(g.clone()).filter(|x| !x.is_empty()),
                        ..Search::default()
                    },
                )
            }),
        );
    }
    let foot = (!dev).then(|| {
        text(t("review:transfersPage.restHold.locked"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap()
    });
    let route_text = tr(
        "review:inbox.transfers.route",
        &[("from", &pair.out.wallet), ("to", &pair.inn.wallet)],
    );
    let title = tr(
        "review:transfersPage.restHold.title",
        &[("rest", &model::msat_btc(rest)), ("from", &pair.out.wallet)],
    );
    question_row(
        tk,
        format!("transfers-rest-hold-{}", pair.id),
        kit::amber_box(tk, &I::SPLIT),
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(sensitive(title, tk.tsm(500.0), tk.fg, hidden).wrap())
            .child(
                row()
                    .w_full()
                    .flex_wrap()
                    .gap(tk.r(0.25))
                    .child(
                        text(t("review:transfersPage.restHold.body"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    )
                    .child(
                        text(t("review:transfersPage.restHold.ask"))
                            .font(tk.tsm(400.0))
                            .color(tk.fg),
                    ),
            )
            .child(
                text(tr(
                    "review:transfersPage.held.sent",
                    &[("date", &date_part(Some(&pair.out.occurred_at)))],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            ),
        answers,
        foot,
        Some(row_menu(
            app,
            tk,
            layer,
            format!("rest-{}", pair.id),
            &route_text,
            &pair.out_transaction_id,
            &pair.in_transaction_id,
            false,
        )),
    )
}

fn gap_hold(app: &App, tk: &Tk, item: &Value) -> El {
    let hidden = app.prefs.hide_sensitive();
    let amount = model::exact_of(item, "amount_msat")
        .map(model::msat_btc)
        .unwrap_or_else(|| "—".into());
    let wallet = model::s(item, "wallet");
    let title = tr(
        "review:transfersPage.gapHold.title",
        &[("amount", &amount), ("wallet", &wallet)],
    );
    let gap = gap_of(item);
    let action = if app.prefs.developer_tools {
        kit::btn(
            tk,
            format!("tp-gap-review-{}", model::s(item, "transaction_id")),
            Var::Outline,
            Some(&I::WRENCH),
            &t("review:transfersPage.gapHold.review"),
            true,
        )
        .gap(tk.r(0.5))
        .child(kit::dev_badge(tk))
        .on_press(move |a: &mut App| {
            super::select(
                a,
                Search {
                    advanced: true,
                    tab: Some(Tab::Gaps),
                    gap: gap.clone().filter(|x| !x.is_empty()),
                    ..Search::default()
                },
            )
        })
    } else {
        text(t("review:transfersPage.gapHold.locked"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap()
    };
    question_row(
        tk,
        format!("transfers-gap-hold-{}", model::s(item, "transaction_id")),
        kit::amber_box(tk, &I::CIRCLE_HELP),
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(sensitive(title, tk.tsm(500.0), tk.fg, hidden).wrap())
            .child(
                text(t("review:transfersPage.gapHold.body"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            )
            .child(
                text(tr(
                    "review:transfersPage.held.sent",
                    &[("date", &date_part(item["occurred_at"].as_str()))],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            ),
        row().child(action),
        None,
        None,
    )
}

/// The route of an unlinked or set-aside row (with its rail icons).
fn unlinked_card(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, lw: f32) -> El {
    let rows = &st(app).unlinked;
    if rows.is_empty() {
        return el();
    }
    let wide_list = lw >= tk.r(72.0);
    let pending = st(app).undismissing;
    let hidden = app.prefs.hide_sensitive();
    let header = kit::card_header(
        tk,
        t("review:transfersPage.unlinked.title"),
        Some(rows.len().to_string()),
        vec![
            text(t("review:transfersPage.unlinked.hint"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        ],
        None,
    );
    let mut list = col().w_full();
    for r in rows {
        let was = match &r.pair {
            Some(p) => tr(
                "review:transfersPage.unlinked.wasPair",
                &[
                    (
                        "kind",
                        &t(&format!(
                            "review:transfersPage.kind.{}",
                            kind_key(Some(&p.kind))
                        )),
                    ),
                    (
                        "policy",
                        &t(&format!(
                            "review:transfersPage.policyShort.{}",
                            if p.policy == "taxable" {
                                "taxable"
                            } else {
                                "carrying"
                            }
                        )),
                    ),
                ],
            ),
            None => t("review:transfersPage.unlinked.wasAutomatic"),
        };
        let route_text = tr(
            "review:inbox.transfers.route",
            &[("from", &r.from_label), ("to", &r.to_label)],
        );
        let rr = r.clone();
        let out = r.out_transaction_id.clone();
        let actions = row()
            .gap(tk.r(0.375))
            .center_y()
            .justify_end()
            .child(
                kit::btn35(
                    tk,
                    format!("tp-unl-again-{}", r.key.replace('\u{0}', "-")),
                    Var::Default,
                    Some(&I::LINK_2),
                    &t(if r.pair.is_some() {
                        "review:transfersPage.unlinked.linkAgain"
                    } else {
                        "review:transfersPage.unlinked.linkBack"
                    }),
                    !pending,
                )
                .label(tr(
                    "review:transfersPage.unlinked.linkAgainLabel",
                    &[("route", &route_text)],
                ))
                .on_press(move |a: &mut App| link_again(a, &rr)),
            )
            .child(row_menu_with(
                app,
                tk,
                layer,
                format!("unl-{}", r.key.replace('\u{0}', "-")),
                &route_text,
                &r.out_transaction_id,
                &r.in_transaction_id,
                true,
                vec![Entry::Item {
                    key: format!("tp-unl-diff-{}", r.key.replace('\u{0}', "-")),
                    glyph: Some(&I::LINK_2),
                    label: t("review:transfersPage.unlinked.linkDifferently"),
                    hint: None,
                    badge: false,
                    disabled: pending,
                    title: None,
                    on: std::rc::Rc::new(move |a: &mut App| {
                        a.custody.page.popup = None;
                        open_link(a, Some(out.clone()));
                    }),
                }],
            ));
        let route_el = col()
            .min_w(0.0)
            .grow(1.0)
            .gap(tk.r(0.125))
            .child(kit::route_stack(
                app,
                cx,
                tk,
                &r.from_label,
                &r.to_label,
                r.from_rail,
                r.to_rail,
                true,
                route_cell_w(tk, lw, wide_list, tk.r(9.0)),
            ))
            .child(
                row()
                    .gap(tk.r(0.375))
                    .center_y()
                    .flex_wrap()
                    .child(ic(&I::UNLINK, tk.r(0.75), tk.muted_fg))
                    .child(
                        text(tr(
                            "review:transfersPage.unlinked.on",
                            &[("date", &date_part(r.unlinked_at.as_deref()))],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    )
                    .child(
                        text("·")
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .a11y_hidden(),
                    )
                    .child(text(was).font(tk.txs(400.0)).color(tk.muted_fg)),
            );
        let date = text(date_time(r.occurred_at.as_deref()))
            .font(tk.tsm(400.0).features("'tnum' 1"))
            .color(tk.muted_fg)
            .single_line();
        let amount =
            crate::ui::table::num(tk, model::msat_in(r.amount_msat, &r.asset), tk.fg, hidden);
        list = list.child(kit::rule(tk)).child(list_row(
            tk,
            format!("transfers-unlinked-row-{}", r.key.replace('\u{0}', "-")),
            wide_list,
            date,
            route_el,
            amount,
            actions,
            tk.r(crate::ui::table::PY),
        ));
    }
    kit::surface(tk)
        .key("transfers-recently-unlinked")
        .w_full()
        .role("region")
        .label(t("review:transfersPage.unlinked.title"))
        .child(header)
        .child(list)
}

/// The route cell's width in an unlinked or set-aside row: the list less its insets, and on
/// one line the date, the amount, the actions (`actions_w`, about) and the gaps.
fn route_cell_w(tk: &Tk, lw: f32, wide_list: bool, actions_w: f32) -> f32 {
    use crate::ui::table::EDGE;
    let inner = lw - 2.0 * tk.r(EDGE);
    if wide_list {
        inner - tk.r(7.5) - tk.r(9.0) - actions_w - 3.0 * tk.r(1.0)
    } else {
        inner
    }
}

/// The rows of the unlinked and set-aside cards, on the table kit's insets and rules: one line
/// `7.5rem 1fr 9rem auto` at `@6xl/list`, otherwise date + amount, the route under them, the
/// actions last.
#[allow(clippy::too_many_arguments)]
fn list_row(
    tk: &Tk,
    key: String,
    wide_list: bool,
    date: El,
    route_el: El,
    amount: El,
    actions: El,
    py: f32,
) -> El {
    use crate::ui::table::EDGE;
    let r = if wide_list {
        row()
            .key(key)
            .w_full()
            .px(tk.r(EDGE))
            .py(py)
            .gap(tk.r(1.0))
            .center_y()
            .child(col().w(tk.r(7.5)).child(date))
            .child(route_el)
            .child(row().w(tk.r(9.0)).justify_end().child(amount))
            .child(actions)
    } else {
        col()
            .key(key)
            .w_full()
            .px(tk.r(EDGE))
            .py(py)
            .gap(tk.r(0.375))
            .child(
                row()
                    .w_full()
                    .gap(tk.r(1.0))
                    .center_y()
                    .justify_between()
                    .child(date)
                    .child(amount),
            )
            .child(route_el)
            .child(row().w_full().justify_end().child(actions))
    };
    r.hover(tk.muted.alpha(0.35)).role("row")
}

/// Suggestions the owner set aside, each with Restore (`SetAsideCard`, unfolded on its tab).
fn set_aside_card(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, lw: f32) -> El {
    let rows = &st(app).set_aside;
    if rows.is_empty() {
        return el();
    }
    let wide_list = lw >= tk.r(72.0);
    let pending = st(app).undismissing;
    let hidden = app.prefs.hide_sensitive();
    let head = row()
        .key("tp-setaside-toggle")
        .w_full()
        .px(tk.card_pad())
        .py(tk.r(0.625))
        .gap(tk.r(0.5))
        .center_y()
        .role("button")
        .expanded(true)
        .label(tc("review:transfersPage.setAside.title", rows.len(), &[]))
        .focusable(true)
        .hover(tk.muted.alpha(0.35))
        .child(
            text(tc("review:transfersPage.setAside.title", rows.len(), &[]))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            text(t("review:transfersPage.setAside.hint"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .min_w(0.0)
                .shrink(),
        )
        .child(el().grow(1.0))
        .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg).rotate(180.0));
    let mut list = col().w_full();
    for d in rows {
        let route_text = tr(
            "review:inbox.transfers.route",
            &[("from", &d.out.wallet), ("to", &d.inn.wallet)],
        );
        let dd = d.clone();
        let actions = row()
            .gap(tk.r(0.25))
            .center_y()
            .justify_end()
            .child(
                kit::btn35(
                    tk,
                    format!("tp-restore-{}", d.id),
                    Var::Ghost,
                    Some(&I::ROTATE_CCW),
                    &t("review:transfersPage.setAside.restore"),
                    !pending,
                )
                .label(tr(
                    "review:transfersPage.setAside.restoreLabel",
                    &[("route", &route_text)],
                ))
                .on_press(move |a: &mut App| restore_suggestion(a, &dd)),
            )
            .child(row_menu(
                app,
                tk,
                layer,
                format!("aside-{}", d.id),
                &route_text,
                &d.out_transaction_id,
                &d.in_transaction_id,
                true,
            ));
        let route_el = row().min_w(0.0).grow(1.0).child(kit::route_stack(
            app,
            cx,
            tk,
            &d.out.wallet,
            &d.inn.wallet,
            model::rail_for_leg(Some(&d.out.asset), None),
            model::rail_for_leg(Some(&d.inn.asset), None),
            false,
            route_cell_w(tk, lw, wide_list, tk.r(8.5)),
        ));
        let date = text(date_time(
            d.out
                .occurred_at
                .as_deref()
                .or(d.inn.occurred_at.as_deref()),
        ))
        .font(tk.tsm(400.0).features("'tnum' 1"))
        .color(tk.muted_fg)
        .single_line();
        let amount = crate::ui::table::num(
            tk,
            model::msat_in(d.out.amount_msat, &d.out.asset),
            tk.fg,
            hidden,
        );
        list = list.child(kit::rule(tk)).child(list_row(
            tk,
            format!("transfers-set-aside-row-{}", d.id),
            wide_list,
            date,
            route_el,
            amount,
            actions,
            tk.r(crate::ui::table::PY),
        ));
    }
    kit::surface(tk)
        .key("transfers-set-aside")
        .w_full()
        .role("region")
        .label(tc("review:transfersPage.setAside.title", rows.len(), &[]))
        .child(head)
        .child(list)
}

// ---- Linked transfers -------------------------------------------------------------------------

/// Reasons that only repeat the "Linked by you" chip.
const PLAIN_PAIR_EVIDENCE: [&str; 5] = [
    "manual",
    "reviewed_transfer_pair",
    "bulk_exact",
    "bulk_selected",
    "rule_auto",
];

/// The linked table's columns (the table kit's widths): date, the stacked route (the rest),
/// amount with its fee under it, type with its chip under it, how it was linked, the menu.
struct Cols {
    date: f32,
    amount: f32,
    ty: f32,
    how: f32,
    actions: f32,
    /// The route cell's inner width.
    route: f32,
    /// A narrow list (a panel beside it): type and how share one two-line column, so every
    /// column stays in view without scrolling sideways.
    merged: bool,
}

/// The linked table's least width with its own Type column.
const LINKED_MIN: f32 = 820.0;
/// The least width with Type and Linked merged; narrower still, it scrolls sideways.
const LINKED_MIN_MERGED: f32 = 600.0;

fn cols(tk: &Tk, table_w: f32, merged: bool) -> Cols {
    use crate::ui::table::{EDGE, PX};
    let date = tk.r(EDGE) + tk.r(PX) + tk.r(5.5);
    let (amount, ty, how) = (tk.r(8.5), if merged { 0.0 } else { tk.r(7.5) }, tk.r(10.5));
    let actions = tk.r(PX) + tk.r(1.75) + tk.r(EDGE);
    let route = (table_w - date - amount - ty - how - actions - 2.0 * tk.r(PX)).max(tk.r(8.0));
    Cols {
        date,
        amount,
        ty,
        how,
        actions,
        route,
        merged,
    }
}

fn linked_card(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    lw: f32,
    panel_move: bool,
) -> El {
    let s = st(app);
    let filter = s.filter.unwrap_or(Filter::All);
    let counts = model::count_moves(&s.moves);
    let all_linked = model::filter_moves(&s.moves, Filter::All);
    let years = model::move_years(&all_linked);
    let visible: Vec<&LinkedMove> = model::filter_moves(&s.moves, filter)
        .into_iter()
        .filter(|m| s.year.is_none() || model::move_year(m.occurred_at.as_deref()) == s.year)
        .collect();
    let shown = if s.shown == 0 { LINKED_PAGE } else { s.shown };
    let page: Vec<&LinkedMove> = visible.iter().take(shown).copied().collect();
    let active_filters = (filter != Filter::All) as usize + s.year.is_some() as usize;
    let filtered = active_filters > 0;
    let merged = lw < LINKED_MIN;
    let least = if merged {
        LINKED_MIN_MERGED
    } else {
        LINKED_MIN
    };
    let table_w = crate::ui::table::width("tp-linked-scroll", least, lw);
    let c = cols(tk, table_w, merged);
    let updating = super::updating(app);
    let loading = s.lineage_rd.loading() || s.pairs_rd.loading();
    let failed = s.lineage_rd.failed && s.pairs_rd.failed;
    let partial = linked_partial(app);
    let total = linked_total(app);
    let selected = if panel_move {
        unlink::current(app).map(|m| m.key.clone())
    } else {
        None
    };

    // The filter menu.
    let filter_open = s.popup == Some(Popup::Filter);
    let mut fbtn = kit::btn35(
        tk,
        "tp-filter",
        Var::Outline,
        Some(&I::SLIDERS_HORIZONTAL),
        &t("review:transfersPage.linked.filter.label"),
        true,
    )
    .gap(tk.r(0.5))
    .menu_trigger(filter_open, |a: &mut App| toggle(a, Popup::Filter));
    if active_filters > 0 {
        fbtn = fbtn.border(1.0, tk.primary).child(
            row()
                .min_w(tk.r(1.0))
                .px(tk.r(0.25))
                .center()
                .radius(999.0)
                .fill(tk.primary)
                .child(
                    text(active_filters.to_string())
                        .font(tk.t2xs(600.0).lh(tk.r(1.0)))
                        .color(tk.primary_fg),
                ),
        );
    }
    if filter_open {
        let mut entries = vec![Entry::Label(t("review:transfersPage.linked.filter.how"))];
        for f in Filter::ALL {
            let n = counts.of(f);
            entries.push(Entry::Radio {
                key: format!("tp-filter-{}", f.key()),
                label: t(&format!("review:transfersPage.linked.filter.{}", f.key())),
                trailing: Some(n.to_string()),
                checked: filter == f,
                disabled: f != Filter::All && n == 0,
                on: std::rc::Rc::new(move |a: &mut App| {
                    a.custody.page.filter = Some(f);
                    a.custody.page.shown = LINKED_PAGE;
                }),
            });
        }
        if years.len() > 1 {
            entries.push(Entry::Sep);
            entries.push(Entry::Label(t("review:transfersPage.linked.filter.year")));
            entries.push(Entry::Radio {
                key: "tp-year-all".into(),
                label: t("review:transfersPage.linked.filter.allYears"),
                trailing: None,
                checked: s.year.is_none(),
                disabled: false,
                on: std::rc::Rc::new(|a: &mut App| {
                    a.custody.page.year = None;
                    a.custody.page.shown = LINKED_PAGE;
                }),
            });
            for y in &years {
                let y = *y;
                entries.push(Entry::Radio {
                    key: format!("tp-year-{y}"),
                    label: y.to_string(),
                    trailing: None,
                    checked: s.year == Some(y),
                    disabled: false,
                    on: std::rc::Rc::new(move |a: &mut App| {
                        a.custody.page.year = Some(y);
                        a.custody.page.shown = LINKED_PAGE;
                    }),
                });
            }
        }
        kit::menu(
            tk,
            layer,
            "tp-filter-content",
            "tp-filter",
            Placement::BottomEnd,
            Some(240.0),
            entries,
            close_popup,
        );
    }
    let mut card = kit::surface(tk)
        .key("tp-linked")
        .w_full()
        .role("region")
        .label(t("review:transfersPage.linked.title"));
    card = card.child(kit::card_header(
        tk,
        t("review:transfersPage.linked.title"),
        filtered.then(|| visible.len().to_string()),
        vec![],
        Some(fbtn),
    ));
    if filtered {
        let mut chips = row()
            .w_full()
            .px(tk.card_pad())
            .pb(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .flex_wrap()
            .child(
                text(t("review:transfersPage.linked.filter.active"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        if filter != Filter::All {
            let name = t(&format!(
                "review:transfersPage.linked.filter.{}",
                filter.key()
            ));
            chips = chips.child(
                kit::filter_chip(
                    tk,
                    "tp-chip-filter",
                    &name,
                    tr(
                        "review:transfersPage.linked.filter.clear",
                        &[("name", &name)],
                    ),
                )
                .on_press(|a: &mut App| {
                    a.custody.page.filter = Some(Filter::All);
                    a.custody.page.shown = LINKED_PAGE;
                }),
            );
        }
        if let Some(y) = s.year {
            chips = chips.child(
                kit::filter_chip(
                    tk,
                    "tp-chip-year",
                    &y.to_string(),
                    tr(
                        "review:transfersPage.linked.filter.clear",
                        &[("name", &y.to_string())],
                    ),
                )
                .on_press(|a: &mut App| {
                    a.custody.page.year = None;
                    a.custody.page.shown = LINKED_PAGE;
                }),
            );
        }
        card = card.child(chips);
    }
    if partial && !failed && !loading {
        card = card.child(kit::rule(tk)).child(
            row()
                .key("transfers-linked-partial")
                .w_full()
                .px(tk.card_pad())
                .py(tk.r(0.625))
                .gap(tk.r(0.5))
                .center_y()
                .role("alert")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), kit::amber_icon(tk)))
                .child(
                    text(t("review:transfersPage.linked.partial"))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .grow(1.0)
                        .wrap(),
                )
                .child(
                    kit::btn_xs(
                        tk,
                        "tp-linked-retry",
                        Var::Ghost,
                        Some(&I::REFRESH_CW),
                        &t("review:transfersPage.verdict.retry"),
                        true,
                    )
                    .on_press(retry_reads),
                ),
        );
    }
    // The table: header band, rows, the strip at its transfer's row.
    use crate::ui::table as tb;
    let head = tb::head(tk)
        .child(
            tb::th(
                tk,
                Some(c.date),
                false,
                tb::th_text(tk, t("review:transfersPage.linked.column.date")),
            )
            .pl(tk.r(tb::EDGE)),
        )
        .child(tb::th(
            tk,
            None,
            false,
            tb::th_text(tk, t("review:transfersPage.linked.column.route")),
        ))
        .child(tb::th(
            tk,
            Some(c.amount),
            true,
            tb::th_text(tk, t("review:transfersPage.linked.column.amount")),
        ));
    let head = if c.merged {
        head.child(tb::th(
            tk,
            Some(c.how),
            false,
            tb::th_text(
                tk,
                format!(
                    "{} · {}",
                    t("review:transfersPage.linked.column.type"),
                    t("review:transfersPage.linked.column.how")
                ),
            ),
        ))
    } else {
        head.child(tb::th(
            tk,
            Some(c.ty),
            false,
            tb::th_text(tk, t("review:transfersPage.linked.column.type")),
        ))
        .child(tb::th(
            tk,
            Some(c.how),
            false,
            tb::th_text(tk, t("review:transfersPage.linked.column.how")),
        ))
    };
    let head = head.child(
        row()
            .w(c.actions)
            .h(tk.r(tb::HEAD_H))
            .label(t("review:transfersPage.linked.column.actions")),
    );
    let mut body = col().key("transfers-linked-list").w_full();
    let center_cell = |content: El| {
        row()
            .w_full()
            .h(tk.r(6.0))
            .px(tk.card_pad())
            .center()
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(content)
    };
    let strip_note = s
        .note
        .as_ref()
        .filter(|n| n.spot.is_some() && active_tab(app) == StatusTab::Linked);
    let strip_line = strip_note.map(|n| {
        row()
            .key("transfers-strip")
            .w_full()
            .px(tk.card_pad())
            .py(tk.r(0.375))
            .fill(tk.muted.alpha(0.4))
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(note_view(app, tk, n, true))
    });
    let strip_row = strip_note.and_then(|n| {
        let sp = n.spot.as_ref().unwrap();
        page.iter()
            .position(|m| m.out_transaction_id == sp.out_id && m.in_transaction_id == sp.in_id)
    });
    let strip_at = if strip_line.is_none() {
        None
    } else {
        Some(match strip_row {
            Some(i) => i + 1,
            None => s.last_place.unwrap_or(0).min(page.len()),
        })
    };
    if loading {
        body = body.child(center_cell(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .role("status")
                .child(kit::spinner(tk, tk.r(1.0), tk.muted_fg))
                .child(
                    text(t("review:transfersPage.linked.loading"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        ));
    } else if failed {
        body = body.child(center_cell(
            text(t("review:transfersPage.linked.unavailable"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .role("alert"),
        ));
    } else if page.is_empty()
        && let Some(line) = strip_line
    {
        body = body.child(line);
    } else if page.is_empty() {
        let content = if filtered {
            text(t("review:transfersPage.linked.emptyFilter"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
        } else {
            col()
                .key("transfers-linked-empty")
                .max_w(tk.r(36.0))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .items_center()
                .child(
                    text(t("review:transfersPage.linked.empty"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(t("review:transfersPage.empty.linkedBody"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
                .child(
                    kit::btn35(
                        tk,
                        "tp-empty-link",
                        Var::Outline,
                        Some(&I::LINK_2),
                        &t("review:transfersPage.linkManually"),
                        true,
                    )
                    .on_press(|a: &mut App| open_link(a, None)),
                )
        };
        body = body.child(
            row()
                .w_full()
                .min_h(tk.r(6.0))
                .px(tk.card_pad())
                .py(tk.r(0.5))
                .center()
                .child(content),
        );
    } else {
        let mut strip_line = strip_line;
        for (i, m) in page.iter().enumerate() {
            if strip_at == Some(i)
                && let Some(sl) = strip_line.take()
            {
                body = body.child(sl);
            }
            body = body.child(linked_row(
                app,
                cx,
                tk,
                layer,
                m,
                &c,
                updating,
                selected.as_deref() == Some(m.key.as_str()),
            ));
        }
        if let Some(sl) = strip_line.take() {
            body = body.child(sl);
        }
    }
    // `overflow-x-auto` around the `min-w-[640px] table-fixed` table: a narrower list keeps the
    // columns and scrolls sideways.
    let grid = col().w(table_w).fill(tk.card).child(head).child(body);
    card = card.child(crate::ui::table::hscroll(
        "tp-linked-scroll",
        t("review:transfersPage.linked.title"),
        least,
        grid,
    ));
    // Filters apply to the transfers loaded so far, so older ones stay a click away.
    let has_more = s.lineage_cursor.is_some();
    if visible.len() > shown || has_more {
        let loaded = page.len().to_string();
        let tot = (if filtered { visible.len() } else { total })
            .max(visible.len())
            .to_string();
        let msg = if filtered && visible.len() <= shown {
            t("review:transfersPage.linked.olderMayMatch")
        } else {
            tr(
                "review:transfersPage.linked.loaded",
                &[("loaded", &loaded), ("total", &tot)],
            )
        };
        let more = visible.len() > shown;
        let loading_more = s.lineage_more_loading;
        card = card.child(
            row()
                .w_full()
                .pt(tk.r(0.75))
                .pb(tk.r(0.75))
                .gap(tk.r(0.75))
                .center()
                .child(text(msg).font(tk.txs(400.0)).color(tk.muted_fg))
                .child(
                    kit::btn(
                        tk,
                        "tp-load-more",
                        Var::Outline,
                        None,
                        &t(if loading_more {
                            "review:transfersPage.linked.loadingMore"
                        } else {
                            "review:transfersPage.linked.loadMore"
                        }),
                        !loading_more,
                    )
                    .on_press(move |a: &mut App| {
                        if more {
                            let s = &mut a.custody.page;
                            s.shown =
                                (if s.shown == 0 { LINKED_PAGE } else { s.shown }) + LINKED_PAGE;
                        } else {
                            load_more_lineage(a);
                        }
                    }),
                ),
        );
    }
    card
}

/// What a move the owner linked leaves of its send (`restOfMove`).
fn rest_of_move(m: &LinkedMove, parts: Option<&Parts>) -> Option<i128> {
    m.pair_id.as_ref()?;
    let open = parts?.send.get(&m.out_transaction_id)?;
    has_open_part(true, Some(open)).then_some(open.unlinked_msat)
}

#[allow(clippy::too_many_arguments)]
fn linked_row(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    m: &LinkedMove,
    c: &Cols,
    updating: bool,
    selected: bool,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let btc = app.prefs.currency == Currency::Btc;
    let rest = rest_of_move(m, st(app).parts.as_ref());
    let route_text = tr(
        "review:inbox.transfers.route",
        &[("from", &m.from_label), ("to", &m.to_label)],
    );
    let unlinkable = model::can_unlink(m);
    let evidence = if m.source == Source::Undone {
        Some(t("review:transfersPage.linked.undoneReason"))
    } else if !unlinkable {
        Some(t(&format!(
            "review:transfersPage.linked.fixed.{}",
            model::fixed_link_kind(m.evidence_reason.as_deref())
        )))
    } else if let Some(l) = m
        .lineage
        .as_ref()
        .filter(|l| !PLAIN_PAIR_EVIDENCE.contains(&l.evidence_reason.as_str()))
    {
        Some(t(model::lineage_evidence_key(
            &l.evidence_reason,
            &l.custody_state,
        )))
    } else if m.source == Source::Reviewed && !m.by_rule {
        Some(t("review:transfersPage.linked.linkedByYouReason"))
    } else {
        None
    };
    let rule_reason = (m.by_rule && m.source != Source::Undone)
        .then(|| t("review:transfersPage.linked.byRuleReason"));
    let automatic =
        (m.source == Source::Automatic).then(|| t("review:transfersPage.linked.automaticReason"));
    let rest_reason = rest.filter(|_| !hidden).map(|r| {
        tr(
            "review:transfersPage.linked.rest",
            &[("left", &model::msat_in(r, &m.asset))],
        )
    });
    let reason: Vec<String> = [
        rest_reason
            .clone()
            .map(|r| r.trim_end_matches('.').to_owned()),
        automatic,
        evidence,
        rule_reason,
    ]
    .into_iter()
    .flatten()
    .collect();
    let reason = (!reason.is_empty()).then(|| reason.join(". "));
    let ty = model::move_type(m.pair_kind.as_deref(), m.from_rail, m.to_rail);
    let taxable = model::move_is_taxable(m);
    let time = time_part(m.occurred_at.as_deref());
    let has_fee = m.fee_msat.is_some_and(|f| f != 0);
    let fiat = |v: Option<f64>| {
        v.map(|v| kit::money(app, v))
            .unwrap_or_else(|| t("review:transfersPage.linked.unpriced"))
    };
    let fee = if btc {
        model::msat_sats(m.fee_msat.unwrap_or(0))
    } else {
        fiat(m.fee_fiat)
    };
    let rail_title = if m.from_rail == m.to_rail {
        t(&format!("review:transfersPage.rail.{}", m.from_rail.key()))
    } else {
        format!(
            "{} → {}",
            t(&format!("review:transfersPage.rail.{}", m.from_rail.key())),
            t(&format!("review:transfersPage.rail.{}", m.to_rail.key()))
        )
    };
    let key = m.key.replace('\u{0}', "-");
    use crate::ui::table as tb;
    let py = tb::PY;
    // Date, the time under it.
    let mut date = col().child(
        text(date_part(m.occurred_at.as_deref()))
            .font(tk.tsm(400.0).features("'tnum' 1"))
            .color(tk.fg)
            .single_line(),
    );
    if let Some(tm) = time {
        date = date.child(tb::num_sub(tk, tm, false));
    }
    let date = tb::td(tk, Some(c.date), false, py, date).pl(tk.r(tb::EDGE));
    // From over to.
    let route_el = tb::td(
        tk,
        None,
        false,
        py,
        kit::route_stack(
            app,
            cx,
            tk,
            &m.from_label,
            &m.to_label,
            m.from_rail,
            m.to_rail,
            true,
            c.route,
        ),
    )
    .help(rail_title);
    // Amount, the network fee under it.
    let amount_style = if m.source == Source::Undone {
        tk.tsm(400.0)
    } else {
        tk.tsm(500.0)
    }
    .features("'tnum' 1");
    let amount_color = if m.source == Source::Undone {
        tk.muted_fg
    } else {
        tk.fg
    };
    let (amount_el, amount_name) = if btc {
        (
            kit::asset_figure(
                tk,
                m.amount_msat,
                &m.asset,
                amount_style,
                amount_color,
                hidden,
            ),
            model::msat_in(m.amount_msat, &m.asset),
        )
    } else {
        let s = fiat(m.amount_fiat);
        (sensitive(s.clone(), amount_style, amount_color, hidden), s)
    };
    let mut amount = col().items_end().child(kit::currency_toggle(
        app,
        tk,
        format!("tp-amt-{key}"),
        amount_name,
        amount_el,
    ));
    if has_fee {
        let mut f = tb::num_sub(
            tk,
            format!("{} {fee}", t("review:transfersPage.linked.column.fee")),
            hidden,
        );
        if !hidden {
            f = f.help(tr("review:transfersPage.linked.fee", &[("amount", &fee)]));
        }
        amount = amount.child(f);
    }
    let amount = tb::td(tk, Some(c.amount), true, py, amount);
    // Type, the chip that decides taxes under it.
    // Merged with the Linked column, the type and its chip share one line.
    let ty_base = if c.merged {
        row().center_y().gap(tk.r(0.375))
    } else {
        col().gap(tk.r(0.125)).items_start()
    };
    let mut ty_cell = ty_base.w_full().min_w(0.0).child(tb::cell_text(
        t(&format!("review:transfersPage.type.{ty}")),
        tk.txs(400.0),
        tk.fg,
    ));
    if m.different_asset {
        ty_cell = ty_cell.child(tb::chip(
            tk,
            tb::Tone::Warning,
            None,
            t("review:swap.table.differentAsset"),
        ));
    } else if taxable {
        ty_cell = ty_cell.child(tb::chip(
            tk,
            tb::Tone::Warning,
            None,
            t("review:transfersPage.linked.taxable"),
        ));
    }
    // How it was linked: the source glyph in the leading slot and its name; why, on hover.
    let glyph = match m.source {
        Source::Automatic => &I::SPARKLES,
        Source::Reviewed => &I::USER_CHECK,
        Source::Undone => &I::UNLINK,
    };
    let chip_label = if rest.is_some() {
        t("review:transfersPage.linked.chip.part")
    } else if m.source == Source::Automatic {
        t(&format!(
            "review:transfersPage.linked.how.{}",
            model::automatic_link_kind(m.evidence_reason.as_deref())
        ))
    } else if m.by_rule && m.source == Source::Reviewed {
        t("review:transfersPage.linked.chip.rule")
    } else {
        t(&format!(
            "review:transfersPage.linked.chip.{}",
            m.source.key()
        ))
    };
    let chip_color = if rest.is_some() {
        kit::amber_text(tk)
    } else if m.source == Source::Automatic {
        tk.muted_fg
    } else {
        tk.fg
    };
    let why = reason.clone().unwrap_or_else(|| chip_label.clone());
    let label_el = text(chip_label.clone())
        .font(tk.txs(400.0))
        .color(chip_color)
        .single_line()
        .truncate()
        .min_w(0.0)
        .shrink()
        .label(match &reason {
            Some(r) => format!("{chip_label}. {r}"),
            None => chip_label.clone(),
        })
        .help(why);
    let mut how = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(tb::LEAD_GAP))
        .center_y()
        .child(tb::lead_glyph(
            tk,
            glyph,
            if m.source == Source::Automatic {
                tk.muted_fg
            } else {
                tk.fg
            },
        ))
        .child(label_el);
    if m.basis_blocked {
        how = how.child(el().grow(1.0)).child(kit::row_hint(
            tk,
            format!("tp-basis-{key}"),
            format!(
                "{}. {}",
                t("review:transfersPage.linked.basisBlocked"),
                t("review:transfersPage.linked.basisBlockedHint")
            ),
            &I::TRIANGLE_ALERT,
            kit::amber_text(tk),
        ));
    }
    // Actions.
    let menu_key = format!("tp-row-menu-{key}");
    let open = st(app).popup == Some(Popup::Row(m.key.clone()));
    if open {
        let mv = m.clone();
        let mv2 = m.clone();
        let mut entries = vec![Entry::item(
            format!("tp-row-details-{key}"),
            Some(&I::INFO),
            t("review:transfersPage.details.show"),
            move |a: &mut App| open_move(a, mv.clone(), Intent::Details),
        )];
        if unlinkable {
            entries.push(Entry::Item {
                key: format!("tp-row-unlink-{key}"),
                glyph: Some(&I::UNLINK),
                label: t("review:transfersPage.linked.actions.unlink"),
                hint: None,
                badge: false,
                disabled: updating,
                title: None,
                on: std::rc::Rc::new(move |a: &mut App| open_move(a, mv2.clone(), Intent::Unlink)),
            });
        }
        entries.push(Entry::Sep);
        entries.extend(transactions_entries(
            &m.out_transaction_id,
            &m.in_transaction_id,
            false,
        ));
        kit::menu(
            tk,
            layer,
            &format!("{menu_key}-content"),
            &menu_key,
            Placement::BottomEnd,
            None,
            entries,
            close_popup,
        );
    }
    let mk = m.key.clone();
    let actions = tb::td(
        tk,
        Some(c.actions),
        true,
        tb::PY_COMPACT,
        kit::icon_btn(
            tk,
            menu_key,
            Var::Ghost,
            tk.r(1.75),
            &I::ELLIPSIS,
            tk.r(1.0),
            tr(
                "review:transfersPage.linked.actions.label",
                &[("route", &route_text)],
            ),
        )
        .menu_trigger(open, move |a: &mut App| toggle(a, Popup::Row(mk.clone()))),
    )
    .pr(tk.r(tb::EDGE));
    let r = tb::tr(tk, false)
        .key(format!("tp-row-{key}"))
        .role("button")
        .label(tr(
            "review:transfersPage.details.open",
            &[("route", &route_text)],
        ))
        .focusable(true);
    let mv = m.clone();
    let r = tb::interactive(r, tk, selected)
        .on_press(move |a: &mut App| open_move(a, mv.clone(), Intent::Details))
        .child(date)
        .child(route_el)
        .child(amount);
    let r = if c.merged {
        r.child(tb::td(
            tk,
            Some(c.how),
            false,
            py,
            col()
                .w_full()
                .min_w(0.0)
                .gap(tk.r(0.25))
                .child(ty_cell)
                .child(how),
        ))
    } else {
        r.child(tb::td(tk, Some(c.ty), false, py, ty_cell))
            .child(tb::td(tk, Some(c.how), false, py, how))
    };
    r.child(actions)
}

// ---- Notes ------------------------------------------------------------------------------------

/// A note's words; in its row it is short (`noteText`).
fn note_text(app: &App, n: &Note, compact: bool) -> String {
    if let NoteKind::Linked { pair_id, .. } = &n.kind {
        let pairs = st(app).pairs.clone().unwrap_or_default();
        if let Some(rest) = rest_of_pair(&pairs, st(app).parts.as_ref(), pair_id.as_deref()) {
            return tr(
                "review:transfersPage.strip.linkedRest",
                &[("left", &model::msat_btc(rest))],
            );
        }
    }
    t(&format!(
        "review:transfersPage.{}.{}",
        if compact { "strip" } else { "toast" },
        n.kind.key()
    ))
}

/// `TransfersNote`: the note with its way back, as a toast or (`compact`) a strip at the row.
fn note_view(app: &App, tk: &Tk, n: &Note, compact: bool) -> El {
    let pending = note_pending(app);
    let hidden = app.prefs.hide_sensitive();
    let pairs = st(app).pairs.clone().unwrap_or_default();
    let rest = match &n.kind {
        NoteKind::Linked { pair_id, .. } => {
            rest_of_pair(&pairs, st(app).parts.as_ref(), pair_id.as_deref())
        }
        _ => None,
    };
    let unlinked = matches!(
        n.kind,
        NoteKind::Unlinked { .. } | NoteKind::Unpaired { .. }
    );
    let glyph = if unlinked || matches!(n.kind, NoteKind::LinkUndone) {
        ic(&I::UNLINK, tk.r(1.0), tk.muted_fg)
    } else {
        ic(&I::CHECK, tk.r(1.0), kit::emerald_icon(tk))
    };
    let words = note_text(app, n, compact);
    let size_btn = |b: El| {
        if compact {
            b.h(tk.r(1.5)).px(tk.r(0.5))
        } else {
            b.h(tk.r(1.75))
        }
    };
    let mut r = row()
        .min_w(0.0)
        .gap(if compact { tk.r(0.5) } else { tk.r(0.75) })
        .center_y()
        .flex_wrap()
        .child(glyph.a11y_hidden());
    r = r.child(if rest.is_some() {
        sensitive(words, tk.tsm(400.0), tk.fg, hidden)
            .wrap()
            .grow(1.0)
            .min_w(0.0)
    } else {
        text(words)
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap()
            .grow(1.0)
            .min_w(0.0)
    });
    let k = if compact { "strip" } else { "toast" };
    let size = if compact {
        crate::txdetail::widgets::Size::Xs
    } else {
        crate::txdetail::widgets::Size::Sm
    };
    let mk = |key: String, v: Var, label: String, enabled: bool| {
        size_btn(crate::txdetail::widgets::button(
            tk, key, v, size, None, &label, enabled,
        ))
    };
    match &n.kind {
        NoteKind::Linked { pair_id, send_id } => {
            if rest.is_some() {
                let s = send_id.clone();
                r = r.child(
                    mk(
                        format!("tp-{k}-linkrest"),
                        Var::Default,
                        t("review:transfersPage.strip.linkRest"),
                        true,
                    )
                    .on_press(move |a: &mut App| open_link(a, Some(s.clone()))),
                );
            }
            if let Some(pid) = pair_id.clone() {
                let spot = n.spot.clone();
                r = r.child(
                    mk(
                        format!("tp-{k}-undo"),
                        Var::Outline,
                        t("review:transfersPage.toast.undo"),
                        !pending,
                    )
                    .on_press(move |a: &mut App| undo_link(a, &pid, spot.clone())),
                );
            }
        }
        NoteKind::Unlinked {
            dismissal_id,
            out_id,
            in_id,
        } => {
            let (d, o, i) = (dismissal_id.clone(), out_id.clone(), in_id.clone());
            r = r.child(
                mk(
                    format!("tp-{k}-undo"),
                    Var::Outline,
                    t("review:transfersPage.toast.undo"),
                    !pending,
                )
                .on_press(move |a: &mut App| link_back(a, d.clone(), Some((o.clone(), i.clone())))),
            );
        }
        NoteKind::Unpaired { pair, portion, .. } => {
            let label = if pair.is_some() && !portion {
                t("review:transfersPage.toast.undo")
            } else {
                t("review:transfersPage.toast.linkAgain")
            };
            let kind = n.kind.clone();
            r = r.child(
                mk(format!("tp-{k}-undo"), Var::Outline, label, !pending)
                    .on_press(move |a: &mut App| undo_unpair(a, &kind)),
            );
        }
        _ => {}
    }
    if unlinked {
        let out = match &n.kind {
            NoteKind::Unlinked { out_id, .. } => out_id.clone(),
            NoteKind::Unpaired { mv, .. } => mv.out_transaction_id.clone(),
            _ => String::new(),
        };
        r = r.child(
            mk(
                format!("tp-{k}-diff"),
                Var::Ghost,
                t("review:transfersPage.toast.linkDifferently"),
                true,
            )
            .on_press(move |a: &mut App| {
                a.custody.page.note = None;
                open_link(a, Some(out.clone()));
            }),
        );
    }
    r.child(
        kit::icon_btn(
            tk,
            format!("tp-{k}-close"),
            Var::Ghost,
            if compact { tk.r(1.5) } else { tk.r(1.75) },
            &I::X,
            tk.r(0.875),
            t("review:transfersPage.toast.close"),
        )
        .on_press(|a: &mut App| a.custody.page.note = None),
    )
}

// ---- Headless states the demo book cannot reach ---------------------------------------------

/// Scene-only: what waits on the owner, from synthetic rows over the real book's moves — a
/// held undone pairing, the unlinked rest of a send linked in part, and a missing-wallet hold.
/// Nothing is written; the next read replaces it.
pub fn inject_synthetic_holds(app: &mut App) {
    let s = stm(app);
    let taken: HashSet<String> = s
        .dismissals
        .iter()
        .flatten()
        .map(|d| model::move_key(&d.out_transaction_id, &d.in_transaction_id))
        .collect();
    let Some(first) = s
        .lineage
        .iter()
        .find(|l| {
            !taken.contains(&model::move_key(
                &l.out_transaction_id,
                &l.in_transaction_id,
            ))
        })
        .cloned()
    else {
        return;
    };
    let d = Dismissal::parse(&json!({
        "id": "synthetic-held", "out_transaction_id": first.out_transaction_id, "in_transaction_id": first.in_transaction_id,
        "reason": model::AUTOMATIC_UNDO_REASON, "created_at": "2026-11-14T10:00:00Z", "active": true, "permanent": false,
        "out": {"transaction_id": first.out_transaction_id, "wallet": first.from_wallet_label, "asset": first.asset, "amount_msat": first.amount_msat.to_string(), "occurred_at": first.occurred_at},
        "in": {"transaction_id": first.in_transaction_id, "wallet": first.to_wallet_label, "asset": first.asset, "amount_msat": first.amount_msat.to_string(), "occurred_at": first.occurred_at},
    }));
    s.dismissals.get_or_insert_with(Vec::new).push(d);
    s.held_items
        .push(json!({"transaction_id": first.out_transaction_id, "actions": []}));
    if let Some(p) = s.pairs.as_mut().and_then(|ps| {
        ps.iter_mut()
            .find(|p| p.kind == "chain-swap" || p.kind == "manual")
    }) {
        p.shares_legs = Some(true);
        p.out.unlinked_msat = Some(p.out.amount_msat);
        let id = p.id.clone();
        let out = p.out_transaction_id.clone();
        s.held_items.push(json!({"transaction_id": format!("{out}-rest"), "evidence": {"blocker_code": "reviewed_residual_suspense"}, "actions": [{"kind": "review_pair", "pair_id": id}]}));
    }
    if let Some(ps) = s.pairs.clone() {
        s.parts = Some(link_model::open_parts(&ps));
    }
    s.held_items.push(json!({"transaction_id": "synthetic-gap", "wallet": "Regtest Cold Storage", "amount_msat": 1_250_000_000_i64, "occurred_at": "2023-03-02T08:00:00Z", "actions": [{"kind": "review_custody_gap", "gap_id": "synthetic-gap"}]}));
    s.tab = Some(StatusTab::Needs);
    derive(app);
}

/// Scene-only: the note a link leaves at its row (`linked`) or as a toast.
pub fn inject_note(app: &mut App, linked_tab: bool) {
    let Some(m) = st(app).moves.iter().find(|m| m.pair_id.is_some()).cloned() else {
        return;
    };
    let spot = Some(spot_of(&m.out_transaction_id, &m.in_transaction_id));
    set_note(
        app,
        note(
            NoteKind::Linked {
                pair_id: m.pair_id.clone(),
                send_id: m.out_transaction_id.clone(),
            },
            spot,
        ),
    );
    stm(app).tab = Some(if linked_tab {
        StatusTab::Linked
    } else {
        StatusTab::Needs
    });
}

/// Scene-only: open a row's menu.
pub fn open_first_row_menu(app: &mut App) {
    if let Some(m) = st(app).moves.iter().find(|m| m.pair_id.is_some()) {
        stm(app).popup = Some(Popup::Row(m.key.clone()));
    }
}
