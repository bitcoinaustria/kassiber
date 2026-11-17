//! Activity (routes/Activity.tsx): the book's metadata provenance. Filters map onto
//! `ui.activity.history` args (paged by cursor, "Load more"); `ui.activity.stale` counts edits
//! after the last journal run, with "Process" (`ui.journals.process`, local). The timeline is
//! `TransactionHistoryTimeline` (TransactionEditHistoryPanel.tsx) with `showTransaction` and
//! revert (`ui.transactions.history.revert` after a confirm, then a notification).

use std::collections::{HashSet, VecDeque};

use rune::{Color, Cx, El, KeyEvent, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::surfaces::hex;
use super::widgets;
use super::{Popup, time};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::sens;
use crate::txdetail::widgets::{Size, Variant, button};
use crate::ui::ic;

const DATES: [(&str, &str); 4] = [
    ("7", "review:activity.dateLast7"),
    ("30", "review:activity.dateLast30"),
    ("365", "review:activity.dateLastYear"),
    ("all", "review:activity.dateAll"),
];
const SOURCES: [(&str, &str); 4] = [
    ("all", "review:activity.sourceAll"),
    ("gui", "review:activity.sourceDesktop"),
    ("cli", "review:activity.sourceCli"),
    ("ai_tool", "review:activity.sourceAssistant"),
];
const FAMILIES: [(&str, &str); 4] = [
    ("all", "review:activity.fieldAll"),
    ("metadata", "review:activity.fieldMetadata"),
    ("pricing", "review:activity.fieldPricing"),
    ("tax", "review:activity.fieldTax"),
];

pub struct State {
    pub date: &'static str,
    pub source: &'static str,
    pub family: &'static str,
    pub wallet: TextEdit,
    pub transaction: TextEdit,
    pub pricing_only: bool,
    pub ai_only: bool,
    pub stale_only: bool,
    /// `start` for the date filter, fixed when the filter changes (the web's memo).
    start: Option<String>,
    start_for: &'static str,
    pub events: Vec<Value>,
    pub next_cursor: Option<String>,
    /// First page in flight with nothing loaded yet (`isLoading`).
    pub loading: bool,
    /// Any page in flight (`isFetching`).
    pub fetching: bool,
    pub fetching_next: bool,
    pub args_key: String,
    queue: VecDeque<(String, bool)>,
    pub stale: Option<Value>,
    stale_sent: bool,
    pub expanded: HashSet<String>,
    /// (event, field) awaiting the revert confirm.
    pub confirm: Option<(Value, Option<Value>)>,
    pub reverting: bool,
    /// The field diff under the pointer (its revert button shows).
    pub hover_field: Option<String>,
    was_processing: bool,
    /// `App::book_generation` of the history read; a book refresh (an edit, an exclusion)
    /// reads it again.
    generation: u64,
    /// The Filters menu's open submenu ("source", "family", "wallet").
    pub sub: Option<&'static str>,
    /// The edit whose "⋯" menu is open.
    pub row_menu: Option<String>,
}

impl State {
    /// Read the history again (the panel's revert invalidates it).
    pub fn args_key_clear(&mut self) {
        self.args_key.clear();
        self.stale_sent = false;
    }
}

impl State {
    /// Leaving `/activity`: the filters, open rows and the revert prompt are the route's
    /// `useState`s (default again next visit); the history read and its paging stay cached.
    pub fn leave(&mut self) {
        let d = State::default();
        self.date = d.date;
        self.source = d.source;
        self.family = d.family;
        self.wallet = d.wallet;
        self.transaction = d.transaction;
        self.pricing_only = false;
        self.ai_only = false;
        self.stale_only = false;
        self.expanded.clear();
        if !self.reverting {
            self.confirm = None;
        }
        self.hover_field = None;
        self.sub = None;
        self.row_menu = None;
    }
}

impl Default for State {
    fn default() -> Self {
        State {
            date: "30",
            source: "all",
            family: "all",
            wallet: TextEdit::new(""),
            transaction: TextEdit::new(""),
            pricing_only: false,
            ai_only: false,
            stale_only: false,
            start: None,
            start_for: "",
            events: vec![],
            next_cursor: None,
            loading: false,
            fetching: false,
            fetching_next: false,
            args_key: String::new(),
            queue: VecDeque::new(),
            stale: None,
            stale_sent: false,
            expanded: HashSet::new(),
            confirm: None,
            reverting: false,
            hover_field: None,
            was_processing: false,
            generation: 0,
            sub: None,
            row_menu: None,
        }
    }
}

fn args(s: &mut State) -> Value {
    if s.start_for != s.date {
        s.start_for = s.date;
        s.start = s.date.parse::<f64>().ok().map(|days| {
            super::logstore::iso_from_secs(crate::format::now_secs() - days * 86_400.0)
        });
    }
    let mut a = json!({"limit": 50, "include_stale": false});
    if let Some(st) = &s.start {
        a["start"] = json!(st);
    }
    if s.source != "all" {
        a["source"] = json!(s.source);
    }
    if s.family != "all" {
        a["field_family"] = json!(s.family);
    }
    if !s.wallet.text.trim().is_empty() {
        a["wallet"] = json!(s.wallet.text.trim());
    }
    if !s.transaction.text.trim().is_empty() {
        a["transaction"] = json!(s.transaction.text.trim());
    }
    if s.pricing_only {
        a["pricing_only"] = json!(true);
    }
    if s.ai_only {
        a["ai_only"] = json!(true);
    }
    if s.stale_only {
        a["stale_only"] = json!(true);
    }
    a
}

/// The key ignoring `start` (it is recomputed only when the date filter changes).
fn key(s: &mut State) -> String {
    args(s).to_string()
}

fn reload(app: &mut App) {
    app.reports.diag.act.args_key.clear();
    app.reports.diag.act.stale_sent = false;
}

pub fn poll(app: &mut App, _tx: &mut Tx) -> bool {
    step(app)
}

fn step(app: &mut App) -> bool {
    let mut changed = false;
    let processing = app.busy.processing;
    let generation = app.book_generation;
    let s = &mut app.reports.diag.act;
    if s.generation != generation {
        s.generation = generation;
        s.args_key.clear();
        s.stale_sent = false;
    }
    if s.was_processing && !processing {
        // The web invalidates every daemon query once journals are processed.
        s.args_key.clear();
        s.stale_sent = false;
    }
    s.was_processing = processing;
    let k = key(s);
    if k != s.args_key {
        s.args_key = k.clone();
        s.queue.push_back((k.clone(), false));
        s.fetching = true;
        s.loading = s.events.is_empty();
        let a: Value = serde_json::from_str(&k).unwrap_or(Value::Null);
        app.send("ui.activity.history", a, "reports.diag.act.history");
        changed = true;
    }
    let s = &mut app.reports.diag.act;
    // `ui.activity.stale` waits for the first history page (`enabled: !isLoading`).
    if !s.stale_sent && !s.loading {
        s.stale_sent = true;
        app.send("ui.activity.stale", Value::Null, "reports.diag.act.stale");
    }
    changed
}

fn load_more(app: &mut App) {
    let s = &mut app.reports.diag.act;
    let Some(cursor) = s.next_cursor.clone() else {
        return;
    };
    if s.fetching_next {
        return;
    }
    let mut a: Value = serde_json::from_str(&s.args_key).unwrap_or(json!({}));
    a["cursor"] = json!(cursor);
    s.fetching_next = true;
    s.fetching = true;
    s.queue.push_back((s.args_key.clone(), true));
    app.send("ui.activity.history", a, "reports.diag.act.history");
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "reports.diag.act.history" => {
            let s = &mut app.reports.diag.act;
            let (k, more) = s.queue.pop_front().unwrap_or_default();
            if k != s.args_key {
                return;
            }
            s.fetching = !s.queue.is_empty();
            s.loading = false;
            if more {
                s.fetching_next = false;
            }
            if r.error.is_some() {
                return;
            }
            let evs = r.data["events"].as_array().cloned().unwrap_or_default();
            if more {
                s.events.extend(evs);
            } else {
                s.events = evs;
            }
            s.next_cursor = r.data["next_cursor"]
                .as_str()
                .map(str::to_owned)
                .filter(|c| !c.is_empty());
            if s.stale.is_none() && r.data["stale"].is_object() {
                s.stale = Some(r.data["stale"].clone());
            }
        }
        "reports.diag.act.stale" if r.error.is_none() => {
            app.reports.diag.act.stale = Some(r.data.clone())
        }
        "reports.diag.act.revert" => {
            app.reports.diag.act.reverting = false;
            match &r.error {
                Some(e) => app.toast(
                    ToastTone::Error,
                    &t("review:activity.revert.failedTitle"),
                    Some(if e.is_empty() {
                        t("review:activity.revert.failedBody")
                    } else {
                        e.clone()
                    }),
                    None,
                ),
                None => app.toast(
                    ToastTone::Success,
                    &t("review:activity.revert.title"),
                    Some(t("review:activity.revert.body")),
                    None,
                ),
            }
            reload(app);
        }
        _ => {}
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if matches!(ev.key, rune::Key::Escape) && app.reports.diag.act.confirm.is_some() {
        app.reports.diag.act.confirm = None;
        return true;
    }
    false
}

pub fn prime(app: &mut App) {
    step(app);
    super::super::pump_until(app, |a| a.reports.diag.act.queue.is_empty());
    step(app);
    super::super::pump_until(app, |a| {
        a.reports.diag.act.stale.is_some() || a.pending.is_empty()
    });
}

fn revert(app: &mut App, event: Value, field: Option<Value>) {
    app.reports.diag.act.reverting = true;
    super::changes::revert(app, &event, field.as_ref(), "reports.diag.act.revert");
}

// ---- View ----------------------------------------------------------------------------------
//
// An owner request over the web's layout ("too many buttons weirdly placed around"): one
// toolbar (the transaction search, a Filters menu holding every other filter, the period) with
// the stale-edits signal and Process on the right only while there are stale edits; the edits as
// a table on the kit grouped by day, each row unfolding into its field diffs, its actions (open
// the transaction, undo) in its "⋯" menu; Load more in the card's footer.

fn select_value(list: &[(&str, &str)], v: &str) -> String {
    list.iter()
        .find(|(id, _)| *id == v)
        .map(|(_, k)| t(k))
        .unwrap_or_default()
}

fn reset_filters(a: &mut App) {
    let s = &mut a.reports.diag.act;
    s.date = "30";
    s.source = "all";
    s.family = "all";
    s.wallet = TextEdit::new("");
    s.transaction = TextEdit::new("");
    s.pricing_only = false;
    s.ai_only = false;
    s.stale_only = false;
}

/// The filters the Filters menu holds that differ from their default (its button's count).
fn menu_filter_count(s: &State) -> usize {
    [
        s.source != "all",
        s.family != "all",
        !s.wallet.text.trim().is_empty(),
        s.pricing_only,
        s.ai_only,
        s.stale_only,
    ]
    .iter()
    .filter(|x| **x)
    .count()
}

fn any_filter(s: &State) -> bool {
    menu_filter_count(s) > 0 || s.date != "30" || !s.transaction.text.trim().is_empty()
}

fn close_menu(a: &mut App) {
    a.reports.diag.popup = None;
    a.reports.diag.act.sub = None;
    a.reports.diag.act.row_menu = None;
}

/// The Filters menu: Source, Field family and Wallet as submenus of choices, the three
/// switches as check items (the menu stays open while they change), and Clear filters.
fn filters_menu(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    use crate::ui::kit_menu::{Entry, Item};
    let st = &app.reports.diag.act;
    let sub = st.sub;
    let choices = |list: &[(&'static str, &'static str)],
                   cur: &str,
                   set: fn(&mut App, &'static str)|
     -> Vec<Entry> {
        list.iter()
            .map(|(v, k)| {
                let v: &'static str = v;
                Entry::Item(
                    Item::new(t(k))
                        .checked(v == cur)
                        .on_select(move |a: &mut App| {
                            set(a, v);
                            a.reports.diag.act.sub = None;
                        }),
                )
            })
            .collect()
    };
    let open_sub = |name: &'static str| move |a: &mut App| a.reports.diag.act.sub = Some(name);
    let close_sub = |a: &mut App| a.reports.diag.act.sub = None;
    // Wallets: the book's wallets from the shell's overview read (no read of its own); the
    // daemon takes a wallet's label or id, as the web's free-text field sends it.
    let current = st.wallet.text.trim().to_owned();
    let mut wallets = vec![Entry::Item(
        Item::new(t("transactions:table.filter.allWallets"))
            .checked(current.is_empty())
            .on_select(|a: &mut App| {
                a.reports.diag.act.wallet = TextEdit::new("");
                a.reports.diag.act.sub = None;
            }),
    )];
    let mut listed = false;
    for c in app
        .snapshot()
        .map(|s| s.connections.as_slice())
        .unwrap_or(&[])
    {
        if c.label.is_empty() {
            continue;
        }
        let on = current == c.label || current == c.id;
        listed |= on;
        let label = c.label.clone();
        wallets.push(Entry::Item(
            Item::new(c.label.clone())
                .checked(on)
                .on_select(move |a: &mut App| {
                    a.reports.diag.act.wallet = TextEdit::new(label.as_str());
                    a.reports.diag.act.sub = None;
                }),
        ));
    }
    if !current.is_empty() && !listed {
        wallets.push(Entry::Item(Item::new(current.clone()).checked(true)));
    }
    let wallet_value = if current.is_empty() {
        t("transactions:table.filter.allWallets")
    } else {
        current.clone()
    };
    let check =
        |label: String, glyph: Option<&'static rune::Icon>, on: bool, flip: fn(&mut App)| {
            let mut it = Item::new(label)
                .checked(on)
                .on_select(move |a: &mut App| flip(a));
            if let Some(g) = glyph {
                it = it.icon(g);
            }
            Entry::Item(it)
        };
    let entries = vec![
        Entry::Item(
            Item::new(t("review:activity.source"))
                .description(select_value(&SOURCES, st.source))
                .submenu(
                    choices(&SOURCES, st.source, |a, v| a.reports.diag.act.source = v),
                    sub == Some("source"),
                    open_sub("source"),
                    close_sub,
                ),
        ),
        Entry::Item(
            Item::new(t("review:activity.fieldFamily"))
                .description(select_value(&FAMILIES, st.family))
                .submenu(
                    choices(&FAMILIES, st.family, |a, v| a.reports.diag.act.family = v),
                    sub == Some("family"),
                    open_sub("family"),
                    close_sub,
                ),
        ),
        Entry::Item(
            Item::new(t("review:activity.wallet"))
                .description(wallet_value)
                .submenu(
                    wallets,
                    sub == Some("wallet"),
                    open_sub("wallet"),
                    close_sub,
                ),
        ),
        Entry::Sep,
        check(
            t("review:activity.pricingChanges"),
            None,
            st.pricing_only,
            |a| a.reports.diag.act.pricing_only ^= true,
        ),
        check(
            t("review:activity.assistantChanges"),
            Some(&I::BOT),
            st.ai_only,
            |a| a.reports.diag.act.ai_only ^= true,
        ),
        check(
            t("review:activity.staleReports"),
            None,
            st.stale_only,
            |a| a.reports.diag.act.stale_only ^= true,
        ),
        Entry::Sep,
        Entry::Item(
            Item::new(t("review:logsControls.clearFilters"))
                .icon(&I::ROTATE_CCW)
                .disabled(!any_filter(st))
                .on_select(|a: &mut App| {
                    reset_filters(a);
                    close_menu(a);
                }),
        ),
    ];
    crate::ui::kit_menu::mount(
        tk,
        layer,
        "act-filters-menu",
        "act-filters",
        rune_kit::Placement::BottomStart,
        4.0,
        tk.r(15.0),
        entries,
        std::rc::Rc::new(close_menu),
    );
}

/// The Filters button: outline `h-9`, its count of set filters beside the label.
fn filters_button(tk: &Tk, open: bool, count: usize) -> El {
    let label = t("review:activity.filters");
    let mut b = crate::txdetail::widgets::button_surface(
        tk,
        row()
            .key("act-filters")
            .h(tk.r(2.25))
            .px(tk.r(0.75))
            .gap(tk.r(0.5))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(if count > 0 {
        format!("{label} ({count})")
    } else {
        label.clone()
    })
    .focusable(true)
    .child(ic(&I::SLIDERS_HORIZONTAL, tk.r(1.0), tk.fg))
    .child(text(label).font(tk.tsm(500.0)).color(tk.fg).single_line());
    if count > 0 {
        b = b.child(
            row()
                .min_w(tk.r(1.25))
                .h(tk.r(1.25))
                .px(tk.r(0.375))
                .radius(999.0)
                .center()
                .fill(tk.primary)
                .a11y_hidden()
                .child(
                    text(count.to_string())
                        .font(tk.txs(600.0))
                        .color(tk.primary_fg),
                ),
        );
    }
    crate::ui::popup::trigger(b, crate::ui::popup::Kind::Menu, open, |a: &mut App| {
        let open = a.reports.diag.popup == Some(Popup::ActFilters);
        a.reports.diag.popup = if open { None } else { Some(Popup::ActFilters) };
        a.reports.diag.act.sub = None;
    })
}

/// The toolbar: the transaction search, Filters, the period; on the right the stale-edits
/// signal with Process, only while edits wait for a journal run.
fn toolbar(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let st = &app.reports.diag.act;
    let popup = app.reports.diag.popup;
    if popup == Some(Popup::ActDate) {
        let opts = DATES.iter().map(|(v, k)| (*v, t(k))).collect::<Vec<_>>();
        widgets::select_menu(tk, layer, "act-date", opts, st.date, |a, v| {
            a.reports.diag.act.date = v
        });
    }
    if popup == Some(Popup::ActFilters) {
        filters_menu(app, tk, layer);
    }
    let h9 = tk.r(2.25);
    let k = crate::txdetail::widgets::form_kit(tk);
    let mut search = rune_kit::TextInput::new(&st.transaction)
        .label(t("review:activity.transaction"))
        .placeholder(t("review:activity.transactionPlaceholder"))
        .height(h9)
        .text_style(tk.tsm(400.0))
        .leading_icon(&I::SEARCH);
    if !st.transaction.text.is_empty() {
        search = search.clear_button();
    }
    let search = search
        .on_edit(|a: &mut App| &mut a.reports.diag.act.transaction)
        .build(&k)
        .key("act-transaction".to_owned())
        .radius(tk.r(0.375))
        .border(1.0, tk.input);
    let left = row()
        .min_w(0.0)
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .role("toolbar")
        .label(t("review:activity.filters"))
        .child(search.w(tk.r(20.0)).min_w(tk.r(9.0)).shrink())
        .child(filters_button(
            tk,
            popup == Some(Popup::ActFilters),
            menu_filter_count(st),
        ))
        .child(widgets::select(
            tk,
            "act-date",
            t("review:activity.date"),
            select_value(&DATES, st.date),
            Popup::ActDate,
            popup == Some(Popup::ActDate),
        ));
    let stale = st
        .stale
        .as_ref()
        .and_then(|v| v["edit_count"].as_u64())
        .unwrap_or(0) as usize;
    // The stale-edits line beside Process where the toolbar leaves room for it; in a narrow
    // page its glyph alone carries it (the line as its tooltip and accessible name).
    let win = rune::memo::layout_size("sh-root")
        .map(|[w, _]| w)
        .filter(|w| *w > 0.0)
        .unwrap_or(if app.shell.window_w > 0.0 {
            app.shell.window_w
        } else {
            1440.0
        });
    let roomy = win >= 1280.0;
    let right = (stale > 0).then(|| {
        let ink = hex(if tk.dark { "#fcd34d" } else { "#b45309" });
        let busy = app.busy.processing;
        let line = tc(
            "review:activity.staleEdits",
            stale,
            &[("count", &stale.to_string())],
        );
        let process = crate::reports::ledger::common::action(
            tk,
            "act-process",
            Variant::Default,
            &I::REFRESH_CW,
            &t("review:activity.process"),
            busy,
            true,
        );
        let process = if busy {
            process
        } else {
            process.on_press(|a: &mut App| a.process_journals())
        };
        let mut signal = row()
            .gap(tk.r(0.375))
            .center_y()
            .role("status")
            .label(line.clone())
            .help(line.clone())
            .child(ic(&I::CALENDAR_CLOCK, tk.r(1.0), ink).a11y_hidden());
        if roomy {
            signal = signal.child(
                text(line)
                    .font(tk.tsm(500.0))
                    .color(ink)
                    .single_line()
                    .a11y_hidden(),
            );
        }
        row()
            .gap(tk.r(0.75))
            .center_y()
            .child(signal)
            .child(process)
    });
    crate::reports::ledger::common::page_header(tk, left, right)
}

fn chip(tk: &Tk, label: String, ink: Color, fill: Color) -> El {
    row()
        .px(tk.r(0.375))
        .h(tk.r(1.25))
        .center_y()
        .radius(tk.r(0.25))
        .fill(fill)
        .child(text(label).font(tk.txs(400.0)).color(ink).single_line())
}

/// A row's "⋯" menu: open the transaction, undo the whole edit.
fn row_menu(app: &App, tk: &Tk, layer: &mut Vec<El>, ev: &Value, anchor: &str) {
    use crate::ui::kit_menu::{Entry, Item};
    let txid = ev["transaction_id"].as_str().unwrap_or("").to_owned();
    let e2 = ev.clone();
    let entries = vec![
        Entry::Item(
            Item::new(t("chrome:notifications.action.openTransaction"))
                .icon(&I::EXTERNAL_LINK)
                .disabled(txid.is_empty())
                .on_select(move |a: &mut App| {
                    close_menu(a);
                    crate::txdetail::open(a, &txid, "details");
                }),
        ),
        Entry::Sep,
        Entry::Item(
            Item::new(t("assistant:panel.changes.undo"))
                .icon(&I::ROTATE_CCW)
                .disabled(app.reports.diag.act.reverting)
                .on_select(move |a: &mut App| {
                    close_menu(a);
                    a.reports.diag.act.confirm = Some((e2.clone(), None));
                }),
        ),
    ];
    crate::ui::kit_menu::mount(
        tk,
        layer,
        &format!("{anchor}-menu"),
        anchor,
        rune_kit::Placement::BottomEnd,
        4.0,
        tk.r(13.0),
        entries,
        std::rc::Rc::new(close_menu),
    );
}

/// One edit: the time, its source, the summary over the transaction and wallet, its field
/// families (and "stale reports" as a signal), the "⋯" menu; pressing it unfolds the diffs.
fn edit_row(app: &App, tk: &Tk, layer: &mut Vec<El>, ev: &Value, last: bool) -> El {
    use crate::ui::table as tb;
    let st = &app.reports.diag.act;
    let hide = app.prefs.hide_sensitive();
    let s = |k: &str| ev[k].as_str().unwrap_or("").to_owned();
    let id = s("id");
    let expanded = st.expanded.contains(&id);
    let (glyph, ink) = super::changes::source_glyph(tk, &s("source"));
    let when = ev["changed_at"].as_str();
    let mut meta = vec![super::changes::short_id(&super::changes::tx_label(ev))];
    if !s("wallet_label").is_empty() {
        meta.push(s("wallet_label"));
    }
    let tog = id.clone();
    let main = row()
        .key(format!("act-ev-{id}"))
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .py(tk.r(0.625))
        .gap(tk.r(0.75))
        .center_y()
        .role("button")
        .expanded(expanded)
        .label(format!("{} · {}", s("summary"), time::history_date(when)))
        .focusable(true)
        .cursor(rune::Cursor::Pointer)
        .on_press(move |a: &mut App| {
            let e = &mut a.reports.diag.act.expanded;
            if !e.remove(&tog) {
                e.insert(tog.clone());
            }
        })
        .child(
            ic(
                if expanded {
                    &I::CHEVRON_DOWN
                } else {
                    &I::CHEVRON_RIGHT
                },
                tk.r(0.875),
                tk.muted_fg,
            )
            .a11y_hidden(),
        )
        .child(
            text(time::short_time(when))
                .font(tk.txs(400.0).features("'tnum' 1"))
                .color(tk.muted_fg)
                .w(tk.r(4.25))
                .single_line()
                .help(time::history_date(when)),
        )
        .child(
            row()
                .size(tk.r(1.75), tk.r(1.75))
                .radius(tk.r(0.375))
                .center()
                .fill(ink.alpha(0.12))
                .help(s("source_label"))
                .label(s("source_label"))
                .child(ic(glyph, tk.r(0.875), ink)),
        )
        .child(
            col()
                .grow(1.0)
                .w(0.0)
                .min_w(0.0)
                .gap(tk.r(0.125))
                .child(
                    sens(tk, s("summary"), tk.tsm(500.0), tk.fg, hide)
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                )
                .child(
                    sens(tk, meta.join(" · "), tk.txs(400.0), tk.muted_fg, hide)
                        .single_line()
                        .truncate()
                        .min_w(0.0),
                ),
        );
    let mut chips = row().gap(tk.r(0.25)).center_y().justify_end();
    if ev["report_anchor"]["stale_for_reports"].as_bool() == Some(true) {
        chips = chips.child(chip(
            tk,
            t("transactions:history.staleReports"),
            hex(if tk.dark { "#fcd34d" } else { "#b45309" }),
            hex("#f59e0b").alpha(0.12),
        ));
    }
    for fam in ev["families"].as_array().cloned().unwrap_or_default() {
        chips = chips.child(chip(
            tk,
            fam.as_str().unwrap_or("").to_owned(),
            tk.muted_fg,
            tk.muted,
        ));
    }
    let anchor = format!("act-more-{id}");
    let open = st.row_menu.as_deref() == Some(id.as_str())
        && app.reports.diag.popup == Some(Popup::ActRow);
    if open {
        row_menu(app, tk, layer, ev, &anchor);
    }
    let mid = id.clone();
    let more = crate::txdetail::widgets::icon_button(
        tk,
        anchor.clone(),
        Variant::Ghost,
        tk.r(2.0),
        &I::ELLIPSIS,
        tk.r(1.0),
        tr(
            "chrome:notifications.moreActions",
            &[("title", &s("summary"))],
        ),
    );
    let more = crate::ui::popup::trigger(
        more,
        crate::ui::popup::Kind::Menu,
        open,
        move |a: &mut App| {
            let same = a.reports.diag.act.row_menu.as_deref() == Some(mid.as_str())
                && a.reports.diag.popup == Some(Popup::ActRow);
            a.reports.diag.act.row_menu = if same { None } else { Some(mid.clone()) };
            a.reports.diag.popup = if same { None } else { Some(Popup::ActRow) };
        },
    );
    let line = tb::tr(tk, true)
        .px(tk.r(tb::EDGE - 0.5))
        .gap(tk.r(0.75))
        .child(main)
        .child(chips)
        .child(more);
    let mut item = col()
        .key(format!("act-row-{id}"))
        .w_full()
        .role("listitem")
        .child(line);
    if expanded {
        let ctx = super::changes::RowCtx {
            key: "act",
            expanded: true,
            hide,
            reverting: st.reverting,
            hover_field: st.hover_field.clone(),
            toggle: |_, _| {},
            confirm: |a, ev, f| a.reports.diag.act.confirm = Some((ev, f)),
            hover: |a, f| a.reports.diag.act.hover_field = f,
        };
        let mut body = col()
            .w_full()
            .pl(tk.r(tb::EDGE + 6.0))
            .pr(tk.r(tb::EDGE))
            .pb(tk.r(0.875))
            .gap(tk.r(0.5));
        if !s("reason").is_empty() {
            body = body.child(
                sens(tk, s("reason"), tk.txs(400.0), tk.muted_fg, hide)
                    .wrap()
                    .w_full(),
            );
        }
        for f in ev["fields"].as_array().cloned().unwrap_or_default() {
            body = body.child(super::changes::field_diff(tk, ev, &f, &ctx));
        }
        item = item.fill(tk.muted.alpha(0.25)).child(body);
    }
    if !last {
        item = item
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    item
}

/// The empty state, Review's centred one: a glyph disc and the web's line, with Clear filters
/// when filters hide the edits.
fn empty(tk: &Tk, filtered: bool) -> El {
    let mut c = col()
        .key("act-empty")
        .w_full()
        .py(tk.r(4.0))
        .items_center()
        .gap(tk.r(0.75))
        .child(
            row()
                .size(tk.r(3.0), tk.r(3.0))
                .radius(tk.r(1.5))
                .center()
                .fill(tk.muted)
                .a11y_hidden()
                .child(ic(&I::HISTORY, tk.r(1.5), tk.muted_fg)),
        )
        .child(
            text(t("review:activity.emptyLabel"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .align(rune::Align::Center)
                .role("status"),
        );
    if filtered {
        c = c.child(
            button(
                tk,
                "act-reset",
                Variant::Outline,
                Size::Sm,
                Some(&I::ROTATE_CCW),
                &t("review:logsControls.clearFilters"),
                true,
            )
            .on_press(reset_filters),
        );
    }
    c
}

/// Skeleton rows while the first page loads.
fn loading(tk: &Tk) -> El {
    let bar = |w: f32| {
        row()
            .w(w)
            .h(tk.r(0.75))
            .radius(tk.r(0.25))
            .fill(tk.muted.alpha(0.7))
    };
    let mut c = col()
        .w_full()
        .busy(true)
        .role("status")
        .label(t("transactions:history.loading"));
    for i in 0..5 {
        c = c.child(
            row()
                .w_full()
                .px(tk.r(crate::ui::table::EDGE))
                .py(tk.r(0.875))
                .gap(tk.r(0.75))
                .center_y()
                .border_sides([0.0, 0.0, if i < 4 { 1.0 } else { 0.0 }, 0.0], tk.border)
                .child(bar(tk.r(3.5)))
                .child(
                    row()
                        .size(tk.r(1.75), tk.r(1.75))
                        .radius(tk.r(0.375))
                        .fill(tk.muted.alpha(0.7)),
                )
                .child(
                    col()
                        .gap(tk.r(0.375))
                        .child(bar(tk.r(14.0 - i as f32)))
                        .child(bar(tk.r(9.0))),
                ),
        );
    }
    c
}

pub fn view(app: &App, _cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    use crate::ui::table as tb;
    let st = &app.reports.diag.act;
    let head = toolbar(app, tk, layer);
    let n = st.events.len();
    let mut status = row().gap(tk.r(0.5)).center_y();
    if st.fetching && !st.loading {
        status = status.child(crate::reports::ledger::common::inline_status(
            tk,
            t("review:activity.refreshing"),
        ));
    }
    let desc = (!st.loading && n > 0).then(|| {
        format!(
            "{} {}",
            crate::format::count(n),
            tc(
                "review:activity.loadedEdits",
                n,
                &[("count", &n.to_string())]
            )
        )
    });
    let mut card = widgets::surface(tk)
        .key("act-card")
        .w_full()
        .clip_rounded(tk.r(0.75))
        .role("region")
        .label(t("review:activity.provenance"))
        .child(crate::reports::ledger::common::card_head(
            tk,
            t("review:activity.provenance"),
            desc,
            Some(status),
        ));
    let body = if st.loading && st.events.is_empty() {
        loading(tk)
    } else if st.events.is_empty() {
        empty(tk, any_filter(st))
    } else {
        // Grouped by local day, newest first as the daemon pages them.
        let now = crate::format::now_secs();
        let mut list = col().w_full().role("list");
        let mut day: Option<Option<i64>> = None;
        for (i, ev) in st.events.iter().enumerate() {
            let d = time::local_day(ev["changed_at"].as_str());
            if day != Some(d) {
                day = Some(d);
                let label = d
                    .map(|d| time::day_heading(d, now))
                    .unwrap_or_else(|| "—".into());
                list = list.child(
                    row()
                        .key(format!("act-day-{}", d.unwrap_or(0)))
                        .w_full()
                        .h(tk.r(2.0))
                        .px(tk.r(tb::EDGE))
                        .center_y()
                        .fill(tk.muted.alpha(0.5))
                        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                        .border_layout()
                        .child(
                            text(label)
                                .font(tk.txs(500.0))
                                .color(tk.muted_fg)
                                .role("heading")
                                .level(3),
                        ),
                );
            }
            let next_same = st
                .events
                .get(i + 1)
                .map(|e| time::local_day(e["changed_at"].as_str()) == d)
                .unwrap_or(false);
            list = list.child(edit_row(app, tk, layer, ev, !next_same && i + 1 == n));
        }
        list.border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .border_layout()
    };
    card = card.child(body);
    if st.next_cursor.is_some() {
        let busy = st.fetching_next;
        let label = if busy {
            t("review:activity.loadingMore")
        } else {
            t("review:activity.loadMore")
        };
        let mut b = button(
            tk,
            "act-more",
            Variant::Outline,
            Size::Sm,
            None,
            &label,
            !busy,
        );
        if !busy {
            b = b.on_press(load_more);
        }
        card = card.child(
            row()
                .w_full()
                .py(tk.r(0.75))
                .justify_center()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(b),
        );
    }
    super::changes::mount_confirm(
        tk,
        layer,
        "act-confirm",
        &st.confirm,
        |a, ev, f| {
            a.reports.diag.act.confirm = None;
            revert(a, ev, f);
        },
        |a| a.reports.diag.act.confirm = None,
    );
    widgets::shell(tk).child(head).child(card)
}

/// SYNTHETIC events for the `reports-activity-sample` scene (the demo book has none).
pub fn inject_sample(app: &mut App) {
    let s = &mut app.reports.diag.act;
    s.events = super::changes::sample_events();
    s.stale = Some(json!({"edit_count": 1}));
    s.expanded.insert("ev-1".into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_count_covers_the_menu_and_any_filter_the_toolbar() {
        let mut s = State::default();
        assert_eq!(menu_filter_count(&s), 0);
        assert!(!any_filter(&s));
        s.date = "7";
        assert!(
            any_filter(&s),
            "the period is a filter Clear filters resets"
        );
        assert_eq!(
            menu_filter_count(&s),
            0,
            "but it has its own select, not the menu's count"
        );
        s.source = "cli";
        s.ai_only = true;
        s.wallet = TextEdit::new("Cold");
        assert_eq!(menu_filter_count(&s), 3);
        // The daemon args are unchanged by the redesign.
        let a = args(&mut s);
        assert_eq!(
            (
                a["source"].as_str(),
                a["ai_only"].as_bool(),
                a["wallet"].as_str(),
                a["limit"].as_u64()
            ),
            (Some("cli"), Some(true), Some("Cold"), Some(50))
        );
    }
}
