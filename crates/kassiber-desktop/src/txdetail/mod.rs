//! The transaction detail panel, docked beside Overview and Transactions
//! (ui-tauri/src/components/transactions/TransactionDetailSheet*.tsx + TransactionDetailController).
//!
//! State lives in `DetailUi`. Opening resolves the row with `ui.transactions.resolve` and reads
//! the side panels; edits go to a draft compared with the last-saved baseline; Save sends
//! `ui.transactions.metadata.update` with the web's argument rules (`draft::save_args`).

use std::collections::HashMap;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx};
use serde_json::{Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc};
use crate::theme::Tk;

pub mod actions;
pub mod dialogs;
pub mod draft;
pub mod graph;
pub mod header;
pub mod linked;
pub mod rail;
pub mod scenes;
pub mod tabs;
pub mod tax;
pub mod view;
pub mod widgets;

use draft::Draft;
pub use scenes::SCENES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Details,
    Classify,
    Pricing,
    Tax,
    Linked,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Details,
        Tab::Classify,
        Tab::Pricing,
        Tab::Tax,
        Tab::Linked,
    ];
    pub fn parse(s: &str) -> Tab {
        match s {
            "classify" => Tab::Classify,
            "pricing" => Tab::Pricing,
            "tax" => Tab::Tax,
            "linked" => Tab::Linked,
            // `graph` and `ledger` links land on Details, like anything unknown.
            _ => Tab::Details,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Tab::Details => "details",
            Tab::Classify => "classify",
            Tab::Pricing => "pricing",
            Tab::Tax => "tax",
            Tab::Linked => "linked",
        }
    }
    pub fn label(self) -> String {
        t(&format!("transactions:sheet.tab.{}", self.code()))
    }
}

/// Which select menu is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    Label,
    Review,
    Category,
    Kind,
    AcqKind,
    ReuseSource,
    SplitAsset,
    SplitPolicy,
}

#[derive(Default)]
pub struct Busy {
    pub attach: bool,
    pub links: usize,
    pub loan: bool,
    pub unpair: bool,
    pub link: Option<String>,
    pub process: bool,
    pub commercial: bool,
    pub acq: bool,
    pub split: bool,
    pub reuse: bool,
}

/// The daemon's reading of a quarantined row as the Review page hands it to its sheet
/// (`QuarantineDetailContext`).
#[derive(Clone, Debug, PartialEq)]
pub struct QContext {
    pub reason: String,
    pub category: String,
    pub evidence: Value,
    pub root_label: Option<String>,
}

/// Which Review list "Save & next" walks (`DetailSource`): the waiting list, nothing, or the
/// loaded rows of one cause.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReviewSource {
    List,
    None,
    Cause(String),
}

/// What the Review page (QuarantineDashboard) gives the sheet it opens: the reading the click
/// came with (for a root on another page) and the queue "Save & next" steps through.
#[derive(Clone, Debug)]
pub struct ReviewHost {
    pub opened: Option<QContext>,
    pub source: ReviewSource,
    pub ids: Vec<String>,
}

pub struct Reuse {
    pub source: Option<String>,
    pub list: Option<Vec<Value>>,
    pub selected: Vec<String>,
}

pub struct Open {
    /// What the caller asked for (id, txid or explorer id).
    pub query: String,
    pub tab: Tab,
    pub record: Option<Value>,
    pub loading: bool,
    pub error: Option<String>,
    /// Coin trail: ids to go back to.
    pub trail: Vec<String>,
    /// While following a coin: the id to push when the target arrives.
    pub follow_from: Option<String>,
    pub draft: Draft,
    pub base: Draft,
    pub saving: bool,
    pub save_error: Option<String>,
    pub close_after_save: bool,
    pub next_after_save: bool,
    pub attachments: Option<Vec<Value>>,
    pub history: Option<Value>,
    pub events: Option<Value>,
    pub commercial: Option<Value>,
    pub select: Option<Select>,
    pub rename: Option<(String, Option<String>)>,
    pub links_open: bool,
    pub links_error: Option<String>,
    pub explorer_open: bool,
    pub explorer_error: Option<String>,
    pub reuse: Option<Reuse>,
    pub history_open: bool,
    pub expanded: Vec<String>,
    pub revert_confirm: Option<(String, Option<String>, String)>,
    pub busy: Busy,
    pub attach_error: Option<String>,
    pub commercial_kind: Option<String>,
    pub commercial_error: Option<String>,
    pub acq_kind: Option<String>,
    pub acq_preview: Option<Value>,
    pub acq_scope: Option<Value>,
    pub acq_error: Option<String>,
    pub split_asset: String,
    pub split_policy: String,
    pub split_error: Option<String>,
    pub split_msg: Option<String>,
    /// `t` pressed: focus the tag input on the next view.
    pub focus_tag: bool,
    /// Opened by the Review page: its reading and "Save & next" queue (None elsewhere).
    pub review: Option<ReviewHost>,
    /// The book the shown reading came from (`workspaceId`/`profileId` of the resolve reply).
    pub scope: Option<Value>,
    /// The open "Explain" dialog of a Tax calculation row (`CapitalGainExplanation`): its
    /// reference and `ui.reports.explain_capital_gain`'s reply.
    pub explain: Option<(Value, crate::app::Load<Value>)>,
}

impl Open {
    fn new(query: &str, tab: Tab, trail: Vec<String>) -> Open {
        let empty = draft::baseline(&Value::Null);
        Open {
            query: query.to_owned(),
            tab,
            record: None,
            loading: true,
            error: None,
            trail,
            follow_from: None,
            draft: empty.clone(),
            base: empty,
            saving: false,
            save_error: None,
            close_after_save: false,
            next_after_save: false,
            attachments: None,
            history: None,
            events: None,
            commercial: None,
            select: None,
            rename: None,
            links_open: false,
            links_error: None,
            explorer_open: false,
            explorer_error: None,
            reuse: None,
            history_open: false,
            expanded: vec![],
            revert_confirm: None,
            busy: Busy::default(),
            attach_error: None,
            commercial_kind: None,
            commercial_error: None,
            acq_kind: None,
            acq_preview: None,
            acq_scope: None,
            acq_error: None,
            split_asset: "BTC".into(),
            split_policy: "carrying-value".into(),
            split_error: None,
            split_msg: None,
            focus_tag: false,
            review: None,
            scope: None,
            explain: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.record.as_ref().map(|r| draft::s(r, "id"))
    }

    pub fn dirty(&self) -> draft::Dirty {
        draft::diff(&self.draft, &self.base)
    }
}

/// One tab of the side panel (`RightPanel` details): the active tab's content lives in
/// `DetailUi::open`; the others are parked here with the label they had.
pub struct TabSlot {
    pub parked: Option<Box<Open>>,
    /// Kept open (`pinDetail`); a page's preview (false) is replaced by the next row it opens.
    pub pinned: bool,
    pub label: String,
}

pub struct DetailUi {
    pub open: Option<Box<Open>>,
    /// The panel's detail tabs, in the order they opened; `active_tab` is the one shown.
    pub tabs: Vec<TabSlot>,
    pub active_tab: usize,
    /// The tab whose icon shows the close mark (pointer over the tab chip).
    pub tab_hover: Option<usize>,
    /// The `+` (Open in side panel) menu.
    pub add_open: bool,
    /// `sidePanelWidth`: default 600, 480–1680.
    pub width: f32,
    pub maximized: bool,
    /// Last saved draft per transaction (the host's `drafts` map).
    pub drafts: HashMap<String, Draft>,
    /// The transactions of the saves in flight, in the order they were sent (metadata writes run
    /// one at a time and reply in order), so a reply finds its panel after a tab switch.
    pub saving_ids: std::collections::VecDeque<String>,
    /// `ui.loans.list` marks.
    pub loans: Option<Vec<Value>>,
    /// The host's row order for "Save & next" (`set_nav_list`).
    pub nav: Vec<String>,
    pub drag_from: Option<(f32, f32)>,
    // Text fields (bound with `on_edit`, so they live outside the Option).
    pub note: TextEdit,
    pub tag: TextEdit,
    pub cur: TextEdit,
    pub price: TextEdit,
    pub value: TextEdit,
    pub source: TextEdit,
    pub rename: TextEdit,
    pub links: TextEdit,
    pub split_out: TextEdit,
    pub split_payout: TextEdit,
    pub split_counterparty: TextEdit,
    pub split_notes: TextEdit,
}

impl Default for DetailUi {
    fn default() -> Self {
        DetailUi {
            open: None,
            tabs: vec![],
            active_tab: 0,
            tab_hover: None,
            add_open: false,
            width: DEFAULT_WIDTH,
            maximized: false,
            saving_ids: Default::default(),
            drafts: HashMap::new(),
            loans: None,
            nav: vec![],
            drag_from: None,
            note: TextEdit::new(""),
            tag: TextEdit::new(""),
            cur: TextEdit::new(""),
            price: TextEdit::new(""),
            value: TextEdit::new(""),
            source: TextEdit::new(""),
            rename: TextEdit::new(""),
            links: TextEdit::new(""),
            split_out: TextEdit::new(""),
            split_payout: TextEdit::new(""),
            split_counterparty: TextEdit::new(""),
            split_notes: TextEdit::new(""),
        }
    }
}

pub const MIN_WIDTH: f32 = 480.0;
pub const MAX_WIDTH: f32 = 1680.0;
pub const DEFAULT_WIDTH: f32 = 600.0;
/// Below this window width the panel is a sheet over the page (`RIGHT_PANEL_SHEET_QUERY`).
pub const SHEET_BELOW: f32 = 1023.0;

// ---- Opening ----------------------------------------------------------------------------------

/// Opens the panel for a transaction id / txid / explorer id, on `tab` (`details`, `classify`,
/// `pricing`, `tax`, `linked`). Opening from anywhere but the coin trail clears the trail.
pub fn open(app: &mut App, id: &str, tab: &str) {
    // A page's preview takes the next row it opens; a kept tab stays and a new preview opens
    // (`RightPanel`: details with `owner === null` vs pinned ones).
    if app.detail.open.is_some()
        && app
            .detail
            .tabs
            .get(app.detail.active_tab)
            .is_some_and(|t| t.pinned)
    {
        if let Some(i) = app.detail.tabs.iter().position(|t| !t.pinned) {
            activate(app, i);
        } else {
            park_active(app);
            app.detail.tabs.push(TabSlot {
                parked: None,
                pinned: false,
                label: String::new(),
            });
            app.detail.active_tab = app.detail.tabs.len() - 1;
        }
    }
    open_inner(app, id, Tab::parse(tab), vec![], None);
}

/// Opens a row from the Review page (`openDetail`): as `open`, plus the reading the click came
/// with and the list "Save & next" walks.
pub fn open_for_review(
    app: &mut App,
    id: &str,
    tab: &str,
    opened: Option<QContext>,
    source: ReviewSource,
    ids: Vec<String>,
) {
    open(app, id, tab);
    if let Some(o) = app.detail.open.as_mut() {
        o.review = Some(ReviewHost {
            opened,
            source,
            ids,
        });
    }
}

/// The Review page's hosting of the open sheet, while that page is the one on screen.
pub fn review_host(app: &App) -> Option<&ReviewHost> {
    if app.route.path() != "/review" {
        return None;
    }
    app.detail.open.as_ref().and_then(|o| o.review.as_ref())
}

/// Moves the active content into its slot (with its tab label).
fn park_active(app: &mut App) {
    let label = app
        .detail
        .open
        .as_deref()
        .map(|o| view::tab_label(app, o))
        .unwrap_or_default();
    let cur = app.detail.open.take();
    if let Some(slot) = app.detail.tabs.get_mut(app.detail.active_tab) {
        slot.label = label;
        slot.parked = cur;
    }
}

/// Brings tab `i` to the front.
pub fn activate(app: &mut App, i: usize) {
    if i == app.detail.active_tab || i >= app.detail.tabs.len() {
        return;
    }
    park_active(app);
    app.detail.active_tab = i;
    app.detail.open = app.detail.tabs[i].parked.take();
    if let Some(d) = app.detail.open.as_ref().map(|o| o.draft.clone()) {
        sync_edits(app, &d);
    }
}

/// "Keep open": the preview keeps its own tab.
pub fn pin(app: &mut App) {
    if let Some(t) = app.detail.tabs.get_mut(app.detail.active_tab) {
        t.pinned = true;
    }
}

/// Closes tab `i` (the active one shows the last remaining tab, or the panel closes).
pub fn close_tab(app: &mut App, i: usize) {
    if i != app.detail.active_tab {
        if i < app.detail.tabs.len() {
            app.detail.tabs.remove(i);
            if i < app.detail.active_tab {
                app.detail.active_tab -= 1;
            }
        }
        return;
    }
    close(app);
}

/// The host's row order, for "Save & next" (the Transactions table sets it; empty elsewhere).
#[allow(dead_code)]
pub fn set_nav_list(app: &mut App, ids: Vec<String>) {
    app.detail.nav = ids;
}

fn open_inner(app: &mut App, id: &str, tab: Tab, trail: Vec<String>, follow_from: Option<String>) {
    if app.detail.tabs.is_empty()
        || app.detail.open.is_none() && app.detail.active_tab >= app.detail.tabs.len()
    {
        app.detail.tabs = vec![TabSlot {
            parked: None,
            pinned: false,
            label: String::new(),
        }];
        app.detail.active_tab = 0;
    }
    let mut o = Open::new(id, tab, trail);
    o.follow_from = follow_from;
    app.detail.open = Some(Box::new(o));
    app.send(
        "ui.transactions.resolve",
        json!({ "query": id }),
        "txd.resolve",
    );
    if app.detail.loans.is_none() {
        app.send("ui.loans.list", Value::Null, "txd.loans");
    }
}

/// `trail.follow(id)`: open another transaction and remember this one for Back.
#[allow(dead_code)]
pub fn follow(app: &mut App, id: &str) {
    let Some(o) = &app.detail.open else {
        return open(app, id, "details");
    };
    let from = o.id().map(str::to_owned);
    let trail = o.trail.clone();
    open_inner(app, id, Tab::Details, trail, from);
}

/// `trail.back()`.
pub fn back(app: &mut App) {
    let Some(o) = &mut app.detail.open else {
        return;
    };
    let Some(prev) = o.trail.pop() else { return };
    let trail = o.trail.clone();
    open_inner(app, &prev, Tab::Details, trail, None);
}

/// Whether the host wires loan marking (`onMarkLoan`/`onLinkLoan`): the Transactions page does,
/// the Overview's detail does not.
pub fn host_loans(app: &App) -> bool {
    app.route.path() == "/transactions"
}

/// Closes the active detail tab; another open tab comes to the front, else the panel closes.
pub fn close(app: &mut App) {
    let i = app.detail.active_tab;
    if i < app.detail.tabs.len() {
        app.detail.tabs.remove(i);
    }
    app.detail.open = None;
    app.detail.tab_hover = None;
    app.detail.add_open = false;
    if app.detail.tabs.is_empty() {
        app.detail.maximized = false;
        app.detail.active_tab = 0;
        return;
    }
    let next = app.detail.tabs.len() - 1;
    app.detail.active_tab = next;
    app.detail.open = app.detail.tabs[next].parked.take();
    if let Some(d) = app.detail.open.as_ref().map(|o| o.draft.clone()) {
        sync_edits(app, &d);
    }
}

pub fn retry(app: &mut App) {
    if let Some(o) = &app.detail.open {
        let (q, tab, trail, host) = (o.query.clone(), o.tab, o.trail.clone(), o.review.clone());
        open_inner(app, &q, tab, trail, None);
        if let Some(o) = app.detail.open.as_mut() {
            o.review = host;
        }
    }
}

fn matches_query(r: &Value, q: &str) -> bool {
    let q = q.to_lowercase();
    ["id", "externalId", "explorerId", "txnId"]
        .iter()
        .any(|k| draft::s(r, k).to_lowercase() == q)
        || q.contains(':')
}

/// Loads a resolved record into the panel: baseline, text fields, side reads, the graph.
fn load_record(app: &mut App, record: Value, keep_draft: bool) {
    let id = draft::s(&record, "id").to_owned();
    let base = app
        .detail
        .drafts
        .get(&id)
        .cloned()
        .unwrap_or_else(|| draft::baseline(&record));
    let Some(o) = app.detail.open.as_mut() else {
        return;
    };
    let same_row = o.id() == Some(id.as_str());
    let prev_base = o.base.clone();
    // A refetched row with identical metadata keeps in-progress edits (§3.2).
    let reset = !(keep_draft && same_row) || draft::diff(&prev_base, &base).count > 0;
    if let Some(from) = o.follow_from.take().filter(|f| *f != id) {
        o.trail.push(from);
    }
    o.record = Some(record);
    o.loading = false;
    o.error = None;
    if reset {
        o.base = base.clone();
        o.draft = base;
        let d = o.draft.clone();
        sync_edits(app, &d);
    }
    if !same_row || !keep_draft {
        side_reads(app, &id);
        graph::ensure(app, &id);
    }
}

pub fn sync_edits(app: &mut App, d: &Draft) {
    let ui = &mut app.detail;
    ui.note = TextEdit::new(d.note.clone());
    ui.tag = TextEdit::new("");
    ui.cur = TextEdit::new(d.manual_currency.clone());
    ui.price = TextEdit::new(d.manual_price.clone());
    ui.value = TextEdit::new(d.manual_value.clone());
    ui.source = TextEdit::new(d.manual_source.clone());
}

pub fn side_reads(app: &mut App, id: &str) {
    app.send(
        "ui.attachments.list",
        json!({ "transaction": id }),
        "txd.attachments",
    );
    app.send(
        "ui.transactions.history",
        json!({ "transaction": id, "limit": 25 }),
        "txd.history",
    );
    app.send(
        "ui.journals.events.list",
        json!({ "transaction": id, "limit": 20 }),
        "txd.events",
    );
    app.send(
        "ui.transactions.commercial_context",
        json!({ "transaction": id }),
        "txd.commercial",
    );
}

// ---- Replies ----------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("txd.") {
        return;
    }
    if tag == "txd.loans" {
        if r.error.is_none() {
            app.detail.loans = r.data["marks"].as_array().cloned();
        }
        return;
    }
    if app.detail.open.is_none() {
        return;
    }
    match tag {
        "txd.resolve" | "txd.reload" => {
            let keep = tag == "txd.reload";
            let record = r.data.get("transaction").filter(|v| v.is_object()).cloned();
            let o = app.detail.open.as_mut().unwrap();
            let fits = record
                .as_ref()
                .is_some_and(|rec| keep || matches_query(rec, &o.query));
            if r.error.is_none() && fits {
                // `shownEnvelope.workspaceId/profileId`: the book this reading came from.
                let (w, p) = (&r.data["workspaceId"], &r.data["profileId"]);
                o.scope = (w.is_string() && p.is_string())
                    .then(|| json!({"workspace_id": w, "profile_id": p}));
            }
            // The Review page drops a row it cannot read, with a notice (QuarantineDashboard).
            if let (Some(e), false, Some(_)) = (&r.error, keep, review_host(app)) {
                let title = t("journals:quarantine.detail.resolveError");
                let body = if e.is_empty() {
                    title.clone()
                } else {
                    e.clone()
                };
                app.toast(
                    ToastTone::Error,
                    &title,
                    Some(body),
                    Some("quarantine-resolve"),
                );
                close(app);
                return;
            }
            match (&r.error, record) {
                (None, Some(rec)) if fits => load_record(app, rec, keep),
                (None, Some(_)) => {}
                (Some(e), _) if !keep => {
                    let o = app.detail.open.as_mut().unwrap();
                    o.loading = false;
                    o.error = Some(e.clone());
                }
                (None, None) if !keep => {
                    let o = app.detail.open.as_mut().unwrap();
                    o.loading = false;
                    o.error = Some(t("transactions:route.resolveError.body"));
                }
                _ => {}
            }
        }
        "txd.attachments" if r.error.is_none() => {
            if let Some(o) = app.detail.open.as_mut() {
                o.attachments = Some(
                    r.data["attachments"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
        "txd.history" if r.error.is_none() => set(app, |o| o.history = Some(r.data.clone())),
        "txd.events" if r.error.is_none() => set(app, |o| o.events = Some(r.data.clone())),
        "txd.commercial" if r.error.is_none() => set(app, |o| o.commercial = Some(r.data.clone())),
        "txd.save" => actions::on_saved(app, r),
        "txd.explain" => {
            if let Some((_, load)) = app.detail.open.as_mut().and_then(|o| o.explain.as_mut()) {
                *load = match &r.error {
                    Some(e) => crate::app::Load::Failed(e.clone()),
                    None => crate::app::Load::Ready(r.data.clone()),
                };
            }
        }
        _ => actions::on_reply(app, tag, r),
    }
}

fn set(app: &mut App, f: impl FnOnce(&mut Open)) {
    if let Some(o) = app.detail.open.as_mut() {
        f(o);
    }
}

/// Re-reads the open row after a write (keeps unrelated edits) and the side panels.
pub fn reload(app: &mut App) {
    let Some(id) = app
        .detail
        .open
        .as_ref()
        .and_then(|o| o.id().map(str::to_owned))
    else {
        return;
    };
    app.send(
        "ui.transactions.resolve",
        json!({ "query": id }),
        "txd.reload",
    );
    side_reads(app, &id);
}

// ---- Keys (TransactionDetailSheet.tsx:324-369) -----------------------------------------------

fn typing(app: &App) -> bool {
    let d = &app.detail;
    [
        &d.note,
        &d.tag,
        &d.cur,
        &d.price,
        &d.value,
        &d.source,
        &d.rename,
        &d.links,
        &d.split_out,
        &d.split_payout,
        &d.split_counterparty,
        &d.split_notes,
    ]
    .iter()
    .any(|e| e.focused)
}

pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let Some(o) = app.detail.open.as_ref() else {
        return false;
    };
    // Nested dialogs take their own keys (Escape closes them via `.modal`).
    if o.links_open
        || o.explorer_open
        || o.explain.is_some()
        || o.reuse.is_some()
        || o.revert_confirm.is_some()
        || o.select.is_some()
    {
        return false;
    }
    if o.record.is_none() {
        if matches!(ev.key, Key::Escape) {
            close(app);
            return true;
        }
        return false;
    }
    if ev.is_command('s') {
        // The Review page's `onSave` closes its sheet; the other hosts keep it open.
        let close_after = review_host(app).is_some();
        actions::save(app, close_after, false);
        return true;
    }
    if typing(app) {
        return false;
    }
    match &ev.key {
        Key::Escape => {
            close(app);
            true
        }
        Key::Text(s) if !ev.mods.cmd && !ev.mods.ctrl && !ev.mods.alt => {
            let o = app.detail.open.as_mut().unwrap();
            match s.as_str() {
                "1" | "2" | "3" | "4" | "5" => {
                    o.tab = Tab::ALL[s.parse::<usize>().unwrap() - 1];
                    true
                }
                "e" => {
                    o.draft.excluded = !o.draft.excluded;
                    true
                }
                "t" => {
                    o.tab = Tab::Classify;
                    o.focus_tag = true;
                    app.detail.tag.focused = true;
                    let _ = tx;
                    true
                }
                _ => false,
            }
        }
        _ => false,
    }
}

// ---- Dock ---------------------------------------------------------------------------------------

/// The docked panel, or None when closed. Below 1023 pt the panel is a sheet over the page; it
/// then mounts in `layer` and the dock slot stays empty.
pub fn dock(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    view::dock(app, cx, tk, layer)
}

/// Width the open panel takes from the window (for the nav's auto-fold rule); 0 when closed,
/// maximized or shown as a sheet.
pub fn docked_width(app: &App) -> f32 {
    match &app.detail.open {
        Some(_) if !app.detail.maximized && !view::sheet_mode(app) => app.detail.width,
        _ => 0.0,
    }
}

/// Opens an external target (attachment URL or file) with the system handler. Headless runs
/// never open anything.
pub fn open_external(target: &str) -> bool {
    if rune::tooling::headless_run().is_some() {
        return true;
    }
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let cmd = "xdg-open";
    std::process::Command::new(cmd).arg(target).spawn().is_ok()
}

pub fn toast_ok(app: &mut App, title_key: &str, body: Option<String>) {
    app.toast(ToastTone::Success, &t(title_key), body, Some("txd"));
}

pub fn toast_err(app: &mut App, title_key: &str, body: Option<String>) {
    app.toast(ToastTone::Error, &t(title_key), body, Some("txd-err"));
}

/// `{{count}} unsaved change(s)`.
pub fn unsaved_label(n: usize) -> String {
    tc("transactions:sheet.footer.unsavedChanges", n, &[])
}

/// Headless scenes: waits (up to 5 s) until the open row and its side reads have arrived, so the
/// first frame shows them.
#[allow(dead_code)]
pub fn settle(app: &mut App) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        app.pump();
        let done = app.detail.open.as_ref().is_some_and(|o| !o.loading && (o.record.is_none() || (o.attachments.is_some() && o.history.is_some() && o.events.is_some() && o.commercial.is_some())))
            && app.detail.loans.is_some()
            // The graph area's local read, started once the row resolves.
            && !app.pending.values().any(|(_, kind, _)| kind == "ui.transactions.graph");
        if done || std::time::Instant::now() >= deadline {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

/// Opens a row for a scene: `pick` chooses from the snapshot's recent and activity rows.
#[allow(dead_code)]
pub fn scene_open(app: &mut App, tab: &str, pick: fn(&crate::model::Tx) -> bool) {
    let id = app.snapshot().and_then(|s| {
        s.txs
            .iter()
            .chain(s.activity_txs.iter())
            .find(|t| pick(t))
            .or(s.txs.first())
            .map(|t| t.id.clone())
    });
    if let Some(id) = id {
        open(app, &id, tab);
        settle(app);
    }
}

/// Headless scenes: pumps the daemon until `done` holds (up to 20 s; a whole-book review plan
/// takes about 4 s on the demo book).
#[allow(dead_code)]
pub fn pump_until(app: &mut App, done: fn(&App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while !done(app) && std::time::Instant::now() < deadline {
        app.pump();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}
