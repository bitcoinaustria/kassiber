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
    /// `App::book_generation` this panel's record was read for (None before its first read): a
    /// book refresh since (an exclusion in the table, reprocessed journals) re-reads it. The
    /// first generation is 0 too, so 0 cannot also mean unread.
    pub generation: Option<u64>,
    /// This panel's identity for the requests it sends: unique per `Open`, so a reply finds the
    /// panel (active or parked) that asked, and a replaced panel (coin trail, retry) hears none.
    pub serial: u64,
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
    /// The classification picked in the acquisition card: `None` until the user picks,
    /// `Some(None)` for an explicit reset to the imported classification.
    pub acq_kind: Option<Option<String>>,
    pub acq_preview: Option<Value>,
    pub acq_scope: Option<Value>,
    pub acq_error: Option<String>,
    pub split_asset: String,
    pub split_policy: String,
    pub split_error: Option<String>,
    pub split_msg: Option<String>,
    /// The split card's text fields while this panel is parked (out amount, payout,
    /// counterparty, notes); the shown panel's live in `DetailUi`.
    pub split_text: [String; 4],
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

static NEXT_SERIAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Open {
    pub(crate) fn new(query: &str, tab: Tab, trail: Vec<String>) -> Open {
        let empty = draft::baseline(&Value::Null);
        Open {
            generation: None,
            serial: NEXT_SERIAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
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
            split_text: Default::default(),
            focus_tag: false,
            review: None,
            scope: None,
            explain: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.record.as_ref().map(|r| draft::s(r, "id"))
    }

    /// A request target for this panel and the transaction it shows; `key` is what else the
    /// reply must still match.
    pub fn target(&self, key: Value) -> Target {
        Target {
            panel: self.serial,
            tx: self.id().unwrap_or_default().to_owned(),
            key,
        }
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

/// Which panel a request was sent for: its `Open::serial`, the transaction it showed (empty
/// while it resolves) and what else the reply must still match (the reuse source, the explained
/// reference, …; `Null` when nothing).
#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub panel: u64,
    pub tx: String,
    pub key: Value,
}

impl Target {
    /// A target no panel matches.
    pub fn nowhere() -> Target {
        Target {
            panel: 0,
            tx: String::new(),
            key: Value::Null,
        }
    }
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
    /// The transactions of the saves in flight, in the order they were sent (metadata writes run
    /// one at a time and reply in order), so a reply finds its panel after a tab switch.
    /// With each, the draft that was submitted: only that becomes the saved baseline, so edits
    /// made while it saves stay dirty.
    pub saving_ids: std::collections::VecDeque<(u64, String, Draft)>,
    /// The panel each request in flight was sent for, per reply tag (replies with one tag arrive
    /// in the order they were sent, a stale-context re-read keeping its place), so a reply after a
    /// tab switch lands on its own panel, active or parked, or is dropped with it. Saves keep
    /// `saving_ids`; the shared loan list and attachment opening need none.
    pub targets: HashMap<&'static str, std::collections::VecDeque<Target>>,
    /// `ui.loans.list` marks.
    pub loans: Option<Vec<Value>>,
    /// `App::book_generation` the marks were read in, and of each read in flight: a book
    /// refresh (a loan marked from the table) re-reads them, and an answer asked before it is
    /// dropped.
    pub loans_generation: u64,
    pub loans_asked: std::collections::VecDeque<u64>,
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
            targets: HashMap::new(),
            loans: None,
            loans_generation: 0,
            loans_asked: Default::default(),
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
    let mut cur = app.detail.open.take();
    if let Some(o) = cur.as_mut() {
        let d = &app.detail;
        o.split_text = [
            d.split_out.text.clone(),
            d.split_payout.text.clone(),
            d.split_counterparty.text.clone(),
            d.split_notes.text.clone(),
        ];
    }
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
    restore_split(app);
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
    restore_split(app);
    // The reply goes to this panel only: a query with `:` resolves to a record that need not
    // carry it, so the record alone cannot tell which panel asked.
    send_for(
        app,
        "ui.transactions.resolve",
        json!({ "query": id }),
        "txd.resolve",
        Value::Null,
    );
    want_loans(app);
}

/// Reads the loan marks unless current ones are here or on their way.
fn want_loans(app: &mut App) {
    let generation = app.book_generation;
    let d = &app.detail;
    let current = d.loans.is_some() && d.loans_generation == generation;
    if current || d.loans_asked.back() == Some(&generation) {
        return;
    }
    if app
        .send("ui.loans.list", Value::Null, "txd.loans")
        .is_some()
    {
        app.detail.loans_asked.push_back(generation);
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
    restore_split(app);
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

/// Loads a resolved record into panel `panel` (active or parked): baseline, text fields (the
/// active panel's), side reads, the graph.
fn load_record(app: &mut App, panel: u64, record: Value, keep_draft: bool) {
    let id = draft::s(&record, "id").to_owned();
    // The record is the saved state: the daemon answers in order, so a read sent after a save
    // includes it, and metadata changed elsewhere (an exclusion from the table, the assistant)
    // must not give way to an older save's draft.
    let base = draft::baseline(&record);
    let generation = app.book_generation;
    let active = app.detail.open.as_ref().is_some_and(|o| o.serial == panel);
    let any_tx = Target {
        panel,
        ..Target::nowhere()
    };
    let Some(o) = panel_mut(app, &any_tx) else {
        return;
    };
    let same_row = o.id() == Some(id.as_str());
    let prev_base = o.base.clone();
    // A refetched row keeps in-progress edits (§3.2); fields the user left alone take the read.
    let reset = !(keep_draft && same_row);
    let moved = !reset && draft::diff(&prev_base, &base).count > 0;
    if let Some(from) = o.follow_from.take().filter(|f| *f != id) {
        o.trail.push(from);
    }
    o.record = Some(record);
    if o.generation.is_none() {
        o.generation = Some(generation);
    }
    o.loading = false;
    o.error = None;
    if reset {
        o.base = base.clone();
        o.draft = base;
        // A parked panel's text fields are rebound from its draft when it comes to the front.
        if active {
            let d = o.draft.clone();
            sync_edits(app, &d);
        }
    } else if moved {
        o.draft = draft::rebase(&prev_base, &base, &o.draft);
        o.base = base;
        if active {
            let d = o.draft.clone();
            rebind_changed(app, &d);
        }
    }
    if !same_row || !keep_draft {
        let to = Target {
            panel,
            tx: id.clone(),
            key: Value::Null,
        };
        side_reads(app, &to);
    }
    // Every load (a reload after a book refresh too): the graph re-reads its local projection
    // when the book changed since; otherwise the cached graph is reused.
    graph::ensure(app, &id);
}

/// Rebinds only the text fields whose draft value differs from what they show, so a field being
/// typed in keeps its cursor.
fn rebind_changed(app: &mut App, d: &Draft) {
    let ui = &mut app.detail;
    for (edit, value) in [
        (&mut ui.note, &d.note),
        (&mut ui.cur, &d.manual_currency),
        (&mut ui.price, &d.manual_price),
        (&mut ui.value, &d.manual_value),
        (&mut ui.source, &d.manual_source),
    ] {
        if edit.text != *value {
            *edit = TextEdit::new(value.clone());
        }
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

/// Binds the split card's text fields to the shown panel's (empty for a new panel).
fn restore_split(app: &mut App) {
    let [out, payout, counterparty, notes] = app
        .detail
        .open
        .as_mut()
        .map(|o| std::mem::take(&mut o.split_text))
        .unwrap_or_default();
    let ui = &mut app.detail;
    ui.split_out = TextEdit::new(out);
    ui.split_payout = TextEdit::new(payout);
    ui.split_counterparty = TextEdit::new(counterparty);
    ui.split_notes = TextEdit::new(notes);
}

/// The side reads of the panel and transaction `to` names.
pub fn side_reads(app: &mut App, to: &Target) {
    let id = to.tx.clone();
    let reads = [
        (
            "ui.attachments.list",
            json!({ "transaction": id }),
            "txd.attachments",
        ),
        (
            "ui.transactions.history",
            json!({ "transaction": id, "limit": 25 }),
            "txd.history",
        ),
        (
            "ui.journals.events.list",
            json!({ "transaction": id, "limit": 20 }),
            "txd.events",
        ),
        (
            "ui.transactions.commercial_context",
            json!({ "transaction": id }),
            "txd.commercial",
        ),
    ];
    for (kind, args, tag) in reads {
        send_to(app, to.clone(), kind, args, tag);
    }
}

// ---- Request targets ---------------------------------------------------------------------------

/// Sends a request whose reply belongs to the panel `to` names, remembering it under `tag`.
pub(super) fn send_to(app: &mut App, to: Target, kind: &str, args: Value, tag: &'static str) {
    app.detail.targets.entry(tag).or_default().push_back(to);
    app.send(kind, args, tag);
}

/// `send_to` for the panel shown now.
pub(super) fn send_for(app: &mut App, kind: &str, args: Value, tag: &'static str, key: Value) {
    let to = active_target(app, key);
    send_to(app, to, kind, args, tag);
}

/// The shown panel as a request target (one no panel matches when none is shown).
pub(super) fn active_target(app: &App, key: Value) -> Target {
    match &app.detail.open {
        Some(o) => o.target(key),
        None => Target {
            key,
            ..Target::nowhere()
        },
    }
}

/// The panel a reply of `tag` was sent for (always drains its entry; None for untracked tags).
fn take_target(app: &mut App, tag: &str) -> Option<Target> {
    app.detail.targets.get_mut(tag)?.pop_front()
}

/// The panel `to` names, shown or parked, while it still shows `to`'s transaction (any, when
/// it was sent while the panel resolved).
pub(super) fn panel_mut<'a>(app: &'a mut App, to: &Target) -> Option<&'a mut Open> {
    let ui = &mut app.detail;
    std::iter::once(ui.open.as_deref_mut())
        .chain(ui.tabs.iter_mut().map(|t| t.parked.as_deref_mut()))
        .flatten()
        .find(|o| o.serial == to.panel && (to.tx.is_empty() || o.id() == Some(to.tx.as_str())))
}

/// Whether `to` names the panel shown now.
pub(super) fn is_active(app: &App, to: &Target) -> bool {
    app.detail
        .open
        .as_ref()
        .is_some_and(|o| o.serial == to.panel)
}

// ---- Replies ----------------------------------------------------------------------------------

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("txd.") {
        return;
    }
    if tag == "txd.loans" {
        let asked = app.detail.loans_asked.pop_front();
        if r.error.is_none() && asked == Some(app.book_generation) {
            app.detail.loans = r.data["marks"].as_array().cloned();
            app.detail.loans_generation = app.book_generation;
        }
        return;
    }
    // Every save reply drains its queued entry, even after the last tab closed.
    if tag == "txd.save" {
        actions::on_saved(app, r);
        return;
    }
    // Every other reply drains its panel's entry (even with no panel left) and lands only on
    // the panel it was sent for, shown or parked.
    let target = take_target(app, tag);
    match tag {
        "txd.resolve" | "txd.reload" => {
            let Some(to) = target else { return };
            on_resolved(app, &to, tag == "txd.reload", r);
        }
        "txd.attachments" | "txd.history" | "txd.events" | "txd.commercial"
            if r.error.is_none() =>
        {
            let Some(o) = target.as_ref().and_then(|to| panel_mut(app, to)) else {
                return;
            };
            match tag {
                "txd.attachments" => {
                    o.attachments = Some(
                        r.data["attachments"]
                            .as_array()
                            .cloned()
                            .unwrap_or_default(),
                    )
                }
                "txd.history" => o.history = Some(r.data.clone()),
                "txd.events" => o.events = Some(r.data.clone()),
                _ => o.commercial = Some(r.data.clone()),
            }
        }
        "txd.attachments" | "txd.history" | "txd.events" | "txd.commercial" => {}
        "txd.explain" => {
            let Some(to) = target else { return };
            // Only the dialog still explaining the reference that was asked.
            if let Some((reference, load)) = panel_mut(app, &to).and_then(|o| o.explain.as_mut())
                && *reference == to.key
            {
                *load = match &r.error {
                    Some(e) => crate::app::Load::Failed(e.clone()),
                    None => crate::app::Load::Ready(r.data.clone()),
                };
            }
        }
        _ => actions::on_reply(app, tag, target, r),
    }
}

/// A `ui.transactions.resolve` reply for the panel `to` names: the first reading (`resolve`) or
/// a re-read of the transaction it shows (`reload`, which keeps unrelated edits).
fn on_resolved(app: &mut App, to: &Target, keep: bool, r: &Reply) {
    let active = is_active(app, to);
    let record = r.data.get("transaction").filter(|v| v.is_object()).cloned();
    let Some(o) = panel_mut(app, to) else {
        return;
    };
    // A re-read must still be of the transaction the panel shows.
    let fits = record
        .as_ref()
        .is_some_and(|rec| !keep || o.id() == Some(draft::s(rec, "id")));
    if r.error.is_none() && fits {
        // `shownEnvelope.workspaceId/profileId`: the book this reading came from.
        let (w, p) = (&r.data["workspaceId"], &r.data["profileId"]);
        o.scope =
            (w.is_string() && p.is_string()).then(|| json!({"workspace_id": w, "profile_id": p}));
    }
    // The Review page drops the row it shows when it cannot read it, with a notice
    // (QuarantineDashboard).
    if let (Some(e), false, true, Some(_)) = (&r.error, keep, active, review_host(app)) {
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
    let failed = match (&r.error, record) {
        (None, Some(rec)) if fits => {
            load_record(app, to.panel, rec, keep);
            None
        }
        (Some(e), _) if !keep => Some(e.clone()),
        (None, None) if !keep => Some(t("transactions:route.resolveError.body")),
        _ => None,
    };
    if let Some(e) = failed
        && let Some(o) = panel_mut(app, to)
    {
        o.loading = false;
        o.error = Some(e);
    }
}

/// Re-reads the open row after a write (keeps unrelated edits) and the side panels.
pub fn reload(app: &mut App) {
    let to = active_target(app, Value::Null);
    reload_to(app, to);
}

/// The shown panel follows book refreshes: when the book changed since its record was read, it
/// reads the record (and its journal events) again, keeping unsaved edits. A parked panel
/// catches up when it comes to the front. Only this panel's reads; no book refresh.
pub fn follow_generation(app: &mut App) -> bool {
    let generation = app.book_generation;
    if app.detail.open.is_none() {
        return false;
    }
    want_loans(app);
    let Some(o) = app.detail.open.as_mut() else {
        return false;
    };
    if o.generation.is_none_or(|g| g == generation) || o.loading || o.record.is_none() {
        return false;
    }
    o.generation = Some(generation);
    reload(app);
    true
}

/// `reload` for the panel `to` names, shown or parked.
pub(super) fn reload_to(app: &mut App, to: Target) {
    if to.tx.is_empty() {
        return;
    }
    let query = to.tx.clone();
    send_to(
        app,
        to.clone(),
        "ui.transactions.resolve",
        json!({ "query": query }),
        "txd.reload",
    );
    side_reads(app, &to);
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

#[cfg(test)]
mod split_tests {
    use super::*;

    /// The split card's inputs belong to their panel: another pinned transaction starts empty,
    /// and switching back brings each panel's own amounts (a save never pairs one panel's
    /// amounts with another's asset and policy).
    #[test]
    fn split_inputs_stay_with_their_panel() {
        let mut app = crate::app::tests::quiet_app();
        open(&mut app, "a", "tax");
        pin(&mut app);
        app.detail.split_out = TextEdit::new("0.5");
        app.detail.split_notes = TextEdit::new("A's note");
        open(&mut app, "b", "tax");
        assert_eq!(app.detail.active_tab, 1);
        assert_eq!(app.detail.split_out.text, "");
        assert_eq!(app.detail.split_notes.text, "");
        app.detail.split_out = TextEdit::new("0.2");
        activate(&mut app, 0);
        assert_eq!(app.detail.split_out.text, "0.5");
        assert_eq!(app.detail.split_notes.text, "A's note");
        activate(&mut app, 1);
        assert_eq!(app.detail.split_out.text, "0.2");
        close(&mut app);
        assert_eq!(app.detail.split_out.text, "0.5");
    }
}

#[cfg(test)]
mod loan_tests {
    use super::*;

    fn marks(ids: &[&str]) -> Reply {
        Reply {
            kind: "ui.loans.list".into(),
            data: json!({"marks": ids.iter().map(|i| json!({"transaction_id": i})).collect::<Vec<_>>()}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// A book refresh (a loan marked from the table) re-reads the detail panel's loan marks,
    /// and an answer asked for before it is dropped.
    #[test]
    fn a_book_refresh_rereads_the_loan_marks() {
        let mut app = crate::app::tests::quiet_app();
        let reads = |a: &App| -> usize {
            a.pending
                .values()
                .filter(|(tag, _, _)| *tag == "txd.loans")
                .count()
        };
        want_loans(&mut app);
        want_loans(&mut app);
        assert_eq!(reads(&app), 1, "one read in flight");
        app.book_generation += 1;
        want_loans(&mut app);
        assert_eq!(reads(&app), 2, "the refresh asks again");
        on_reply(&mut app, "txd.loans", &marks(&[]));
        assert!(
            app.detail.loans.is_none(),
            "the pre-refresh answer is dropped"
        );
        on_reply(&mut app, "txd.loans", &marks(&["a"]));
        assert_eq!(app.detail.loans.as_ref().map(Vec::len), Some(1));
        let before = reads(&app);
        want_loans(&mut app);
        assert_eq!(reads(&app), before, "current marks are kept");
    }
}
