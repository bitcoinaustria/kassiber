//! The Journals route (routes/Journals.tsx): the State / Ledger entries tabs, the page actions,
//! the four status metrics, the entry mix (a filter over the latest rows) and the latest rows
//! table, whose rows open the transaction.

use rune::{Color, Cx, El, col, el, row, text};
use serde_json::Value;
use std::collections::VecDeque;

use super::common::{self as c, Tone, sens, tnum};
use super::reportable::{self, TableUi};
use crate::app::{App, Load, Reply};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::Variant;
use crate::ui::surface;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    State,
    Reportable,
}

#[derive(Default)]
pub struct JournalsUi {
    pub tab: Tab,
    pub snapshot: Load<Snapshot>,
    /// The entry-mix filter (a display type: `transfer` groups both directions).
    pub filter: Option<String>,
    pub events: Load<Value>,
    pub table: TableUi,
    /// The reproducibility check dialog.
    pub verify: super::verify::Verify,
    /// The page's "⋯" menu (Reports, Check reproducibility) is open.
    pub menu: bool,
    /// The snapshot / entry reads in flight, in send order (the daemon answers in order): true
    /// while current, false once a refresh or book switch made its answer obsolete (dropped).
    snapshot_asked: VecDeque<bool>,
    stale_snapshot: bool,
    events_asked: VecDeque<bool>,
    stale_events: bool,
}

impl JournalsUi {
    /// Another book: the view starts over, but the reads still out keep their place, so their
    /// answers are not shown as the new book's.
    pub fn switch_book(&mut self) {
        let snapshot_asked = self.snapshot_asked.iter().map(|_| false).collect();
        let events_asked = self.events_asked.iter().map(|_| false).collect();
        *self = JournalsUi {
            snapshot_asked,
            events_asked,
            ..JournalsUi::default()
        };
    }

    fn snapshot_inflight(&self) -> bool {
        self.snapshot_asked.contains(&true)
    }

    fn events_inflight(&self) -> bool {
        self.events_asked.contains(&true)
    }

    /// A refresh: what is shown stays up while it is re-read; the reads already out answer for
    /// the book before it, so their answers are dropped and the reads asked again.
    pub fn invalidate(&mut self) {
        self.snapshot_asked.iter_mut().for_each(|c| *c = false);
        self.events_asked.iter_mut().for_each(|c| *c = false);
        // Keep showing what we have while the re-read runs, as React Query does.
        if !matches!(self.snapshot, Load::Ready(_)) {
            self.snapshot = Load::Idle;
        } else {
            self.stale_snapshot = true;
        }
        if !matches!(self.events, Load::Ready(_)) {
            self.events = Load::Idle;
        } else {
            self.stale_events = true;
        }
    }
}

// Re-read flags live beside the loads (kept out of `Load` so the shared type stays as is).
impl JournalsUi {
    fn wants_snapshot(&self) -> bool {
        matches!(self.snapshot, Load::Idle) || self.stale_snapshot
    }
}

#[derive(Clone, Debug, Default)]
pub struct Status {
    pub transaction_count: f64,
    pub journal_entry_count: f64,
    pub needs_journals: bool,
    pub quarantines: f64,
    pub freshness_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct EntryType {
    pub kind: String,
    pub count: f64,
    pub gain_loss: f64,
}

#[derive(Clone, Debug)]
pub struct Recent {
    pub date: String,
    pub kind: String,
    pub transaction_id: Option<String>,
    pub wallet: String,
    pub asset: String,
    pub quantity: f64,
    pub fiat: f64,
    pub gain_loss: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub status: Status,
    pub entry_types: Vec<EntryType>,
    pub recent: Vec<Recent>,
    pub recent_by_type: Option<Vec<(String, Vec<Recent>)>>,
}

fn num(v: &Value) -> f64 {
    v.as_f64().unwrap_or(0.0)
}

fn s(v: &Value) -> String {
    v.as_str().unwrap_or("").to_owned()
}

fn parse_recent(v: &Value) -> Recent {
    Recent {
        date: s(&v["date"]),
        kind: s(&v["type"]),
        transaction_id: v["transactionId"]
            .as_str()
            .filter(|x| !x.is_empty())
            .map(str::to_owned),
        wallet: s(&v["wallet"]),
        asset: s(&v["asset"]),
        quantity: num(&v["quantity"]),
        fiat: num(&v["fiatValueEur"]),
        gain_loss: num(&v["gainLossEur"]),
    }
}

impl Snapshot {
    pub fn parse(d: &Value) -> Snapshot {
        let st = &d["status"];
        Snapshot {
            status: Status {
                transaction_count: num(&st["transactionCount"]),
                journal_entry_count: num(&st["journalEntryCount"]),
                needs_journals: st["needsJournals"].as_bool().unwrap_or(false),
                quarantines: num(&st["quarantines"]),
                freshness_reason: st["freshnessReason"].as_str().map(str::to_owned),
            },
            entry_types: d["entryTypes"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|e| EntryType {
                            kind: s(&e["type"]),
                            count: num(&e["count"]),
                            gain_loss: num(&e["gainLossEur"]),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            recent: d["recent"]
                .as_array()
                .map(|a| a.iter().map(parse_recent).collect())
                .unwrap_or_default(),
            recent_by_type: d["recentByType"].as_object().map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        (
                            k.clone(),
                            v.as_array()
                                .map(|a| a.iter().map(parse_recent).collect())
                                .unwrap_or_default(),
                        )
                    })
                    .collect()
            }),
        }
    }

    fn by_type(&self, kind: &str) -> Option<&Vec<Recent>> {
        self.recent_by_type
            .as_ref()
            .and_then(|m| m.iter().find(|(k, _)| k == kind).map(|(_, v)| v))
    }
}

// ---- Model (the route's helpers) ----------------------------------------------------------

pub fn is_transfer_direction(kind: &str) -> bool {
    kind == "transfer_in" || kind == "transfer_out"
}

pub fn display_type(kind: &str) -> &str {
    if is_transfer_direction(kind) {
        "transfer"
    } else {
        kind
    }
}

/// `humanizeEntryCode`: `transfer_fee` → `Transfer Fee`.
pub fn humanize(code: &str) -> String {
    let spaced = code.replace('_', " ");
    let mut out = String::with_capacity(spaced.len());
    let mut prev_word = false;
    for ch in spaced.chars() {
        let word = ch.is_alphanumeric() || ch == '_';
        if word && !prev_word {
            out.extend(ch.to_uppercase())
        } else {
            out.push(ch)
        }
        prev_word = word;
    }
    out
}

/// `localizedEntryType`.
pub fn entry_type_label(kind: &str) -> String {
    let key = match kind {
        "acquisition" => "acquisition",
        "disposal" => "disposal",
        "income" => "income",
        "fee" => "fee",
        "transfer_fee" => "transferFee",
        "transfer" => "transfer",
        "transfer_in" => "transferIn",
        "transfer_out" => "transferOut",
        "neutral_swap" => "neutralSwap",
        _ => return humanize(kind),
    };
    t(&format!("journals:ledger.entryType.{key}"))
}

/// `groupEntryTypesForDisplay`: transfer in/out fold into one `transfer` row, first-seen order.
pub fn group_entry_types(types: &[EntryType]) -> Vec<EntryType> {
    let mut out: Vec<EntryType> = Vec::new();
    for e in types {
        let d = display_type(&e.kind);
        if let Some(x) = out.iter_mut().find(|x| x.kind == d) {
            x.count += e.count;
            x.gain_loss += e.gain_loss;
        } else {
            out.push(EntryType {
                kind: d.to_owned(),
                count: e.count,
                gain_loss: e.gain_loss,
            });
        }
    }
    out
}

fn sort_recent(mut rows: Vec<Recent>) -> Vec<Recent> {
    // `localeCompare` descending on the `YYYY-MM-DD HH:MM` strings; stable like Array.sort.
    rows.sort_by(|a, b| b.date.cmp(&a.date));
    rows
}

/// `recentRowsForDisplayType`.
pub fn rows_for_type(snap: &Snapshot, display: &str) -> Vec<Recent> {
    if display != "transfer" {
        let rows = snap.by_type(display).cloned().unwrap_or_else(|| {
            snap.recent
                .iter()
                .filter(|e| e.kind == display)
                .cloned()
                .collect()
        });
        return sort_recent(rows);
    }
    let mut both: Vec<Recent> = Vec::new();
    for k in ["transfer_out", "transfer_in"] {
        if let Some(v) = snap.by_type(k) {
            both.extend(v.iter().cloned());
        }
    }
    if !both.is_empty() {
        return sort_recent(both);
    }
    sort_recent(
        snap.recent
            .iter()
            .filter(|e| is_transfer_direction(&e.kind))
            .cloned()
            .collect(),
    )
}

fn type_description(kind: &str) -> String {
    if kind == "transfer" {
        t("journals:ledger.entries.labelTransferRows")
    } else {
        tr(
            "journals:ledger.entries.labelTypedRows",
            &[("type", &entry_type_label(kind))],
        )
    }
}

/// `journalReadiness`: (label, detail, tone).
pub fn readiness(st: &Status) -> (String, String, Tone) {
    if st.quarantines > 0.0 {
        (
            t("journals:ledger.readiness.blocked"),
            t("journals:ledger.readiness.blockedDetail"),
            Tone::Alert,
        )
    } else if st.needs_journals {
        (
            t("journals:ledger.readiness.stale"),
            st.freshness_reason
                .clone()
                .unwrap_or_else(|| t("journals:ledger.readiness.staleDetail")),
            Tone::Warning,
        )
    } else if st.journal_entry_count > 0.0 {
        (
            t("journals:ledger.readiness.current"),
            st.freshness_reason
                .clone()
                .unwrap_or_else(|| t("journals:ledger.readiness.currentDetail")),
            Tone::Good,
        )
    } else {
        (
            t("journals:ledger.readiness.empty"),
            t("journals:ledger.readiness.emptyDetail"),
            Tone::Neutral,
        )
    }
}

fn count(n: f64) -> String {
    crate::format::count(n.max(0.0).round() as usize)
}

// ---- Loading ------------------------------------------------------------------------------

/// Starts the reads the visible tab needs. True when anything changed.
pub fn step(app: &mut App) -> bool {
    let mut changed = false;
    let ui = &app.reports.ledger.journals;
    if ui.wants_snapshot() && !ui.snapshot_inflight() {
        app.reports.ledger.journals.snapshot_asked.push_back(true);
        if matches!(app.reports.ledger.journals.snapshot, Load::Idle) {
            app.reports.ledger.journals.snapshot = Load::Loading;
        }
        app.send(
            "ui.journals.snapshot",
            Value::Null,
            "reports.ledger.journals",
        );
        changed = true;
    }
    // The Ledger entries tab mounts (and reads) only while selected, as Radix unmounts it.
    let ui = &app.reports.ledger.journals;
    if ui.tab == Tab::Reportable
        && (matches!(ui.events, Load::Idle) || ui.stale_events)
        && !ui.events_inflight()
    {
        app.reports.ledger.journals.events_asked.push_back(true);
        if matches!(app.reports.ledger.journals.events, Load::Idle) {
            app.reports.ledger.journals.events = Load::Loading;
        }
        app.send(
            "ui.journals.events.list",
            serde_json::json!({"limit": reportable::LIMIT}),
            "reports.ledger.events",
        );
        changed = true;
    }
    changed
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    let ui = &mut app.reports.ledger.journals;
    match tag {
        "reports.ledger.journals" => {
            if ui.snapshot_asked.pop_front() != Some(true) {
                return;
            }
            ui.stale_snapshot = false;
            ui.snapshot = match (&r.error, r.data.is_null() || r.data.get("status").is_none()) {
                (Some(e), _) => Load::Failed(e.clone()),
                (None, true) => Load::Failed(String::new()),
                (None, false) => Load::Ready(Snapshot::parse(&r.data)),
            };
        }
        "reports.ledger.events" => {
            if ui.events_asked.pop_front() != Some(true) {
                return;
            }
            ui.stale_events = false;
            ui.events = match (
                &r.error,
                r.data.is_null() || r.data.get("summary").is_none(),
            ) {
                (Some(e), _) => Load::Failed(e.clone()),
                (None, true) => Load::Failed(String::new()),
                (None, false) => Load::Ready(r.data.clone()),
            };
        }
        _ => {}
    }
}

/// Headless: load the snapshot (and the entries when that tab is selected).
pub fn prime(app: &mut App) {
    step(app);
    crate::reports::pump_until(app, |a| {
        let j = &a.reports.ledger.journals;
        j.snapshot_asked.is_empty() && j.events_asked.is_empty()
    });
}

/// Scene: the first entry-mix row selected.
pub fn scene_filter_first(app: &mut App) {
    if let Load::Ready(snap) = &app.reports.ledger.journals.snapshot {
        app.reports.ledger.journals.filter = group_entry_types(&snap.entry_types)
            .first()
            .map(|e| e.kind.clone());
    }
}

/// Scene: the loaded status with `quarantines` held rows and the stale flag set (synthetic).
pub fn scene_status(app: &mut App, quarantines: f64, stale: bool) {
    if let Load::Ready(snap) = &mut app.reports.ledger.journals.snapshot {
        snap.status.quarantines = quarantines;
        snap.status.needs_journals = stale;
        if stale {
            snap.status.freshness_reason = None;
        }
    }
}

// ---- View ---------------------------------------------------------------------------------

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let ui = &app.reports.ledger.journals;
    let snap = match &ui.snapshot {
        Load::Idle | Load::Loading => {
            return c::screen_skeleton(tk, tk.r(8.0), c::win_w(app, cx) >= 1536.0);
        }
        Load::Failed(e) => {
            let msg = if e.is_empty() {
                t("journals:ledger.unavailable.fallback")
            } else {
                e.clone()
            };
            return c::shell(tk).key("lg-page").child(
                c::centered(
                    tk,
                    "lg-unavailable",
                    &I::TRIANGLE_ALERT,
                    tk.alert.solid,
                    t("journals:ledger.unavailable.title"),
                    msg,
                    None,
                    c::free_h(app, tk),
                )
                .role("alert"),
            );
        }
        Load::Ready(s) => s,
    };
    let header = page_header(app, tk, layer, snap);
    let panel = match ui.tab {
        Tab::State => col()
            .w_full()
            .gap(tk.card_gap())
            .child(metrics(app, cx, tk, snap))
            .child(body(app, cx, tk, snap)),
        Tab::Reportable => reportable::view(app, cx, tk, layer),
    };
    super::verify::mount(app, tk, layer);
    c::shell(tk)
        .key("lg-page")
        .child(header)
        .child(panel.role("tabpanel"))
}

/// The tabs (and the running job beside them), then Process journals and the "⋯" menu with
/// Reports and Check reproducibility, on both tabs.
fn page_header(app: &App, tk: &Tk, layer: &mut Vec<El>, snap: &Snapshot) -> El {
    let ui = &app.reports.ledger.journals;
    let tabs = c::seg_tabs(
        tk,
        "lg-tabs",
        // The tab list is named by its page (Radix leaves it unnamed; screen readers then
        // announce a bare "tab list").
        t("nav:book.ledger"),
        true,
        vec![
            c::Seg::new(
                "lg-tabs-0",
                t("journals:ledger.tabs.state"),
                ui.tab == Tab::State,
                |a: &mut App| a.reports.ledger.journals.tab = Tab::State,
            ),
            c::Seg::new(
                "lg-tabs-1",
                t("journals:ledger.tabs.reportable"),
                ui.tab == Tab::Reportable,
                |a: &mut App| a.reports.ledger.journals.tab = Tab::Reportable,
            ),
        ],
    );
    let busy = app.busy.processing;
    let mut left = row().min_w(0.0).gap(tk.r(0.75)).center_y().child(tabs);
    if busy {
        // The run's phase, as the title bar and notifications show it.
        let label = match &app.busy.progress {
            Some(p) if p != "Processing journals" => p.clone(),
            _ => t("journals:processing.phase.fallback"),
        };
        left = left.child(c::inline_status(tk, label));
    }
    // The primary look only while the journals need a run (signal, not decoration).
    let variant = if snap.status.needs_journals && snap.status.quarantines <= 0.0 {
        Variant::Default
    } else {
        Variant::Outline
    };
    let process = c::action(
        tk,
        "lg-process",
        variant,
        &I::REFRESH_CW,
        &t("journals:ledger.actions.processJournals"),
        busy,
        !app.busy.syncing,
    )
    .on_press(|a: &mut App| a.process_journals());
    let open = ui.menu;
    let more = c::more_button(tk, "lg-more", open, |a: &mut App| {
        a.reports.ledger.journals.menu = !a.reports.ledger.journals.menu
    });
    if open {
        use crate::ui::kit_menu::{Entry, Item};
        let entries = vec![
            Entry::Item(
                Item::new(t("nav:book.reports"))
                    .icon(&I::FILE_TEXT)
                    .on_select(|a: &mut App| {
                        a.reports.ledger.journals.menu = false;
                        crate::shell::go(a, "/reports");
                    }),
            ),
            // The reproducibility check: local only, from this press.
            Entry::Item(
                Item::new(t("journals:ledger.actions.verifyJournals"))
                    .icon(&I::LIST_CHECKS)
                    .disabled(busy)
                    .on_select(|a: &mut App| {
                        a.reports.ledger.journals.menu = false;
                        super::verify::open(a);
                    }),
            ),
        ];
        let w = tk.r(if crate::i18n::lang() == crate::i18n::Lang::De {
            17.0
        } else {
            14.0
        });
        crate::ui::kit_menu::mount(
            tk,
            layer,
            "lg-more-menu",
            "lg-more",
            rune_kit::Placement::BottomEnd,
            4.0,
            w,
            entries,
            std::rc::Rc::new(|a: &mut App| a.reports.ledger.journals.menu = false),
        );
    }
    c::page_header(
        tk,
        left,
        Some(row().gap(tk.r(0.5)).center_y().child(process).child(more)),
    )
}

/// The status tiles (`JournalMetric`) as the home screen's stat tiles (an owner request). The
/// state tile and the quarantine tile show only when there is something to do (a stale,
/// blocked or empty state; held rows): "Current" and "0 clear" say nothing, and the current
/// state stays on the entries tile's tooltip and the Process button's quiet look.
fn metrics(app: &App, cx: &Cx<App>, tk: &Tk, snap: &Snapshot) -> El {
    use crate::overview::grid::stat::{Stat, row_of};
    let st = &snap.status;
    let (label, detail, tone) = readiness(st);
    let held = st.quarantines > 0.0;
    let mut stats = Vec::new();
    let signal = match tone {
        Tone::Alert => Some((&I::SHIELD_ALERT, tk.alert.solid)),
        Tone::Warning => Some((&I::TRIANGLE_ALERT, tk.warn.solid)),
        Tone::Neutral => Some((&I::CIRCLE_DASHED, tk.fg)),
        Tone::Good => None,
    };
    if let Some((glyph, ink)) = signal {
        let mut s = Stat::new(
            "lg-metric-0",
            glyph,
            t("journals:ledger.metric.status"),
            label.clone(),
        )
        .line(detail.clone());
        if tone != Tone::Neutral {
            s = s.ink(ink).glyph_ink(ink);
        }
        stats.push(s);
    }
    stats.push(
        Stat::new(
            "lg-metric-1",
            &I::LIST,
            t("journals:ledger.metric.transactions"),
            count(st.transaction_count),
        )
        .line(t("journals:ledger.metric.transactionsSub")),
    );
    let mut entries = Stat::new(
        "lg-metric-2",
        &I::BOOK_OPEN,
        t("journals:ledger.metric.journalEntries"),
        count(st.journal_entry_count),
    )
    .line(t("journals:ledger.metric.journalEntriesSub"));
    if tone == Tone::Good {
        entries = entries.help(format!("{label} · {detail}"));
    }
    stats.push(entries);
    if held {
        // The way to the held rows (the web's header "Review" link, shown only with quarantines).
        stats.push(
            Stat::new(
                "lg-review",
                &I::SHIELD_ALERT,
                t("journals:ledger.metric.quarantine"),
                count(st.quarantines),
            )
            .ink(tk.alert.solid)
            .glyph_ink(tk.alert.solid)
            .line(t("journals:ledger.metric.quarantineHeld"))
            .link(
                format!(
                    "{} · {} {}",
                    t("nav:book.review"),
                    count(st.quarantines),
                    t("journals:ledger.metric.quarantineHeld")
                ),
                |a: &mut App| crate::shell::go(a, "/review"),
            ),
        );
    }
    row_of(
        app,
        cx,
        tk,
        "lg-metrics",
        t("journals:ledger.tabs.state"),
        stats,
    )
}

/// The entry mix and the latest rows: side by side from `xl` (`grid-cols-[360px_1fr]`); below
/// that the mix lays its rows out in columns above the table.
fn body(app: &App, cx: &mut Cx<App>, tk: &Tk, snap: &Snapshot) -> El {
    // Side by side while the page itself has room (the side panel narrows it as a smaller
    // window does): the mix's column and a table that needs no sideways scroll.
    let page = cx
        .layout_size("lg-page")
        .map(|[w, _]| w - 2.0 * tk.page_pad())
        .filter(|w| *w > 0.0)
        .unwrap_or_else(|| c::page_w(app, cx, tk));
    let wide = page >= 1000.0;
    let mix_w = if wide { 340.0 } else { page };
    let mix = entry_mix(app, tk, snap, mix_w, wide);
    let rows = entries(
        app,
        cx,
        tk,
        snap,
        if wide {
            page - mix_w - tk.card_gap() - 2.0
        } else {
            page - 2.0
        },
    );
    if wide {
        row()
            .w_full()
            .gap(tk.card_gap())
            .items_start()
            .child(mix.w(mix_w))
            .child(rows.grow(1.0).w(0.0).min_w(0.0))
    } else {
        col()
            .w_full()
            .gap(tk.card_gap())
            .child(rows.w_full())
            .child(mix.w_full())
    }
}

fn entry_mix(app: &App, tk: &Tk, snap: &Snapshot, card_w: f32, beside: bool) -> El {
    let hidden = app.prefs.hide_sensitive();
    // The type totals are journal `gain_loss` sums, in the book's currency.
    let cur = crate::format::book_currency(app);
    let groups = group_entry_types(&snap.entry_types);
    let max = groups.iter().map(|e| e.count).fold(1.0, f64::max);
    // One column beside the table; two or three across the page when stacked.
    let inner = card_w - 2.0 - 2.0 * tk.r(0.75);
    let cols = if beside {
        1
    } else if inner >= 900.0 {
        3
    } else {
        2
    };
    let gap = tk.r(0.5);
    let item_w = ((inner - gap * (cols as f32 - 1.0)) / cols as f32).floor();
    let mut list = row()
        .w_full()
        .px(tk.r(0.75))
        .pb(tk.r(0.75))
        .gap(gap)
        .flex_wrap();
    if groups.is_empty() {
        list = list.child(crate::ui::table::empty_text(
            tk,
            t("journals:ledger.entryMix.empty"),
        ));
    }
    for e in groups {
        let active = app.reports.ledger.journals.filter.as_deref() == Some(e.kind.as_str());
        let rows_label = if e.kind == "transfer" {
            tc(
                "journals:ledger.entryMix.transferRows",
                e.count as usize,
                &[],
            )
        } else {
            tc("journals:ledger.entryMix.rows", e.count as usize, &[])
        };
        let label = entry_type_label(&e.kind);
        let frac = ((e.count / max) * 100.0).max(4.0) as f32 / 100.0;
        let kind = e.kind.clone();
        let mut b = col()
            .key(format!("lg-mix-{}", e.kind))
            .w(item_w)
            .px(tk.r(0.75))
            .py(tk.r(0.625))
            .gap(tk.r(0.5))
            .radius(tk.r(0.5))
            .border(
                1.0,
                if active {
                    tk.primary.alpha(0.45)
                } else {
                    Color::TRANSPARENT
                },
            )
            .border_layout()
            .role("button")
            .checked(active)
            .label(format!("{label} {rows_label}"))
            .focusable(true)
            .cursor(rune::Cursor::Pointer)
            .on_press(move |a: &mut App| {
                let f = &mut a.reports.ledger.journals.filter;
                *f = if f.as_deref() == Some(kind.as_str()) {
                    None
                } else {
                    Some(kind.clone())
                };
            });
        b = if active {
            b.fill(tk.muted.alpha(0.6))
        } else {
            b.hover(tk.muted.alpha(0.35))
        };
        let bar = row()
            .w_full()
            .h(tk.r(0.25))
            .radius(tk.r(0.125))
            .fill(tk.muted)
            .a11y_hidden()
            .child(
                row()
                    .w(0.0)
                    .grow(frac)
                    .h_full()
                    .radius(tk.r(0.125))
                    .fill(if active {
                        tk.primary
                    } else {
                        tk.muted_fg.alpha(0.7)
                    }),
            )
            .child(el().w(0.0).grow(1.0 - frac));
        b = b
            .child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .justify_between()
                    .items_start()
                    .child(
                        col()
                            .min_w(0.0)
                            .grow(1.0)
                            .w(0.0)
                            .child(crate::ui::table::cell_text(label, tk.tsm(500.0), tk.fg))
                            .child(
                                text(rows_label)
                                    .font(tnum(tk.txs(400.0)))
                                    .color(tk.muted_fg),
                            ),
                    )
                    .child(crate::ui::table::num(
                        tk,
                        crate::format::fiat(e.gain_loss, &cur),
                        tk.fg,
                        hidden,
                    )),
            )
            .child(bar);
        list = list.child(b);
    }
    surface(tk)
        .key("lg-mix")
        .min_w(0.0)
        .role("region")
        .label(t("journals:ledger.entryMix.title"))
        .clip_rounded(tk.r(0.75))
        .child(c::card_head(
            tk,
            t("journals:ledger.entryMix.title"),
            Some(t("journals:ledger.entryMix.description")),
            None,
        ))
        .child(list)
}

/// The table's box: its laid-out width while it scrolls, else the card's whole width (the
/// flexible columns grow into whatever the first layout's width guess left over).
pub(super) fn fill_table(table_w: f32, overflow: bool) -> El {
    if overflow {
        col().w(table_w)
    } else {
        col().w_full().min_w(table_w)
    }
}

/// The latest rows on the table kit: date, type, the wallet (the flexible column, cut with a
/// tooltip), then the figures right-aligned.
/// One recent-entries row: date, type, wallet, quantity, value and gain/loss. The daemon's
/// `fiatValueEur`/`gainLossEur` hold the stored journal amounts, in the book's currency `cur`.
fn entry_cells(e: &Recent, cur: &str) -> [String; 6] {
    [
        e.date.clone(),
        entry_type_label(display_type(&e.kind)),
        e.wallet.clone(),
        format!("{:.8} {}", e.quantity, e.asset),
        crate::format::fiat(e.fiat, cur),
        crate::format::fiat(e.gain_loss, cur),
    ]
}

fn entries(app: &App, cx: &mut Cx<App>, tk: &Tk, snap: &Snapshot, fallback_w: f32) -> El {
    use crate::ui::table as tb;
    let ui = &app.reports.ledger.journals;
    let hidden = app.prefs.hide_sensitive();
    let filtered: Vec<Recent> = match &ui.filter {
        Some(f) => rows_for_type(snap, f),
        None => snap.recent.clone(),
    };
    let filtered_total = ui
        .filter
        .as_ref()
        .and_then(|f| {
            group_entry_types(&snap.entry_types)
                .into_iter()
                .find(|e| &e.kind == f)
                .map(|e| e.count)
        })
        .unwrap_or(snap.status.journal_entry_count);
    let label = match &ui.filter {
        Some(f) => type_description(f),
        None => t("journals:ledger.entries.labelProcessedRows"),
    };
    let desc = tr(
        "journals:ledger.entries.rowsDescription",
        &[
            ("rows", &crate::format::count(filtered.len())),
            ("total", &count(filtered_total)),
            ("label", &label),
        ],
    );
    let more = (snap.status.journal_entry_count > snap.recent.len() as f64).then(|| {
        crate::txdetail::widgets::button(
            tk,
            "lg-view-entries",
            Variant::Outline,
            crate::txdetail::widgets::Size::Sm,
            None,
            &t("journals:ledger.actions.viewLedgerEntries"),
            true,
        )
        .on_press(|a: &mut App| a.reports.ledger.journals.tab = Tab::Reportable)
    });
    let head = c::card_head(tk, t("journals:ledger.entries.title"), Some(desc), more);
    let labels = [
        t("common:field.date"),
        t("common:field.type"),
        t("journals:ledger.entries.column.wallet"),
        t("journals:ledger.entries.column.quantity"),
        t("journals:ledger.entries.column.fiat"),
        t("journals:ledger.entries.column.gainLoss"),
    ];
    let (sm, xs, nums) = (tk.tsm(400.0), tk.txs(400.0), tb::num_style(tk));
    let side = |k: &str| {
        t(if k == "transfer_out" {
            "journals:ledger.entries.outgoingSide"
        } else {
            "journals:ledger.entries.incomingSide"
        })
    };
    let split_date = |d: &str| match d.split_once(' ') {
        Some((day, time)) => (day.to_owned(), time.to_owned()),
        None => (d.to_owned(), String::new()),
    };
    let cur = crate::format::book_currency(app);
    let cells_text: Vec<[String; 6]> = filtered.iter().map(|e| entry_cells(e, &cur)).collect();
    // Every column but the wallet takes its widest cell; the wallet takes the rest.
    let mut content: [f32; 6] = std::array::from_fn(|k| {
        if k == 2 {
            0.0
        } else {
            c::tw(&labels[k], tk.tsm(500.0))
        }
    });
    for (e, cells) in filtered.iter().zip(&cells_text) {
        let (day, _) = split_date(&cells[0]);
        let side_w = if is_transfer_direction(&e.kind) {
            c::tw(&side(&e.kind), xs)
        } else {
            0.0
        };
        let w = [
            c::tw(&day, tnum(sm)),
            c::tw(&cells[1], sm).max(side_w),
            0.0,
            c::tw(&cells[3], nums),
            c::tw(&cells[4], nums),
            c::tw(&cells[5], nums),
        ];
        for k in 0..6 {
            content[k] = content[k].max(w[k]);
        }
    }
    let px = 2.0 * tk.r(tb::PX);
    let edge = tk.r(tb::EDGE) - tk.r(tb::PX);
    let mut widths: [f32; 6] = std::array::from_fn(|k| (content[k] + px).ceil());
    widths[0] += edge;
    widths[5] += edge;
    let wallet_min = tk.r(9.0);
    let fixed: f32 = widths.iter().sum();
    let avail = c::avail(cx, "lg-entries-table", fallback_w);
    let (table_w, overflow) = tb::fit(avail, fixed + wallet_min);
    widths[2] = table_w - fixed;
    let right = [false, false, false, true, true, true];
    let th = |k: usize| {
        let cell = tb::th(
            tk,
            Some(widths[k]),
            right[k],
            tb::th_text(tk, labels[k].clone()),
        );
        let cell = if k == 2 { cell.grow(1.0) } else { cell };
        match k {
            0 => cell.pl(tk.r(tb::EDGE)),
            5 => cell.pr(tk.r(tb::EDGE)),
            _ => cell,
        }
    };
    let td = |k: usize, content: El| {
        let cell = tb::td(tk, Some(widths[k]), right[k], tb::PY, content);
        let cell = if k == 2 { cell.grow(1.0) } else { cell };
        match k {
            0 => cell.pl(tk.r(tb::EDGE)),
            5 => cell.pr(tk.r(tb::EDGE)),
            _ => cell,
        }
    };
    let header = tb::head(tk).children((0..6).map(th));
    let n = filtered.len();
    let mut rows: Vec<El> = Vec::new();
    for (i, (e, cells)) in filtered.iter().zip(cells_text).enumerate() {
        let [date, type_label, wallet, qty, fiat, gain] = cells;
        let (day, time) = split_date(&date);
        let mut date_cell = col().child(text(day).font(tnum(sm)).color(tk.fg).single_line());
        if !time.is_empty() {
            date_cell = date_cell.child(text(time).font(tnum(xs)).color(tk.muted_fg).single_line());
        }
        let mut kind_cell = col()
            .min_w(0.0)
            .child(tb::cell_text(type_label.clone(), sm, tk.fg));
        if is_transfer_direction(&e.kind) {
            kind_cell = kind_cell.child(tb::cell_text(side(&e.kind), xs, tk.muted_fg));
        }
        let wallet_cell = if hidden {
            sens(wallet, sm, tk.fg, true)
                .single_line()
                .truncate()
                .min_w(0.0)
        } else {
            tb::cell_text(wallet, sm, tk.fg)
        };
        let cells = vec![
            td(0, date_cell),
            td(1, kind_cell),
            td(2, wallet_cell),
            td(3, tb::num(tk, qty, tk.fg, hidden)),
            td(4, tb::num(tk, fiat, tk.fg, hidden)),
            td(
                5,
                tb::num(
                    tk,
                    gain,
                    if e.gain_loss < 0.0 {
                        tk.alert.solid
                    } else {
                        tk.fg
                    },
                    hidden,
                ),
            ),
        ];
        let mut r = tb::tr(tk, i + 1 == n)
            .items_stretch()
            .children(cells)
            .key(format!("lg-row-{i}"));
        if let Some(id) = e.transaction_id.clone() {
            r = tb::interactive(r, tk, false)
                .role("button")
                .label(tr(
                    "journals:ledger.entries.openTransactionAria",
                    &[("type", &type_label)],
                ))
                .focusable(true)
                .on_press(move |a: &mut App| {
                    crate::shell::go(a, &format!("/transactions?tx={id}"))
                });
        }
        rows.push(r);
    }
    if rows.is_empty() {
        let msg = match &ui.filter {
            Some(f) => tr(
                "journals:ledger.entries.emptyFiltered",
                &[("label", &type_description(f))],
            ),
            None => t("journals:ledger.entries.empty"),
        };
        rows.push(tb::empty_text(tk, msg));
    }
    let table = tb::hscroll_if(
        "lg-entries-table",
        t("journals:ledger.entries.title"),
        overflow,
        fill_table(table_w, overflow).child(header).children(rows),
    );
    surface(tk)
        .key("lg-entries")
        .min_w(0.0)
        .role("region")
        .label(t("journals:ledger.entries.title"))
        .clip_rounded(tk.r(0.75))
        .child(head)
        .child(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A book refresh while the snapshot and entries are out (held by the refresh cycle, say):
    /// those answers predate it, so they are dropped, both reads are asked again, and only the
    /// fresh answers land.
    #[test]
    fn a_refresh_drops_the_journal_reads_asked_before_it() {
        let mut app = crate::app::tests::quiet_app();
        let reply = |data: Value| Reply {
            kind: String::new(),
            data,
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        let snap = |n: f64| reply(json!({"status": {"journalEntryCount": n}}));
        let events = |n: &str| reply(json!({"summary": {}, "n": n}));
        // Shown and stale, so the refresh keeps them on screen while they are re-read.
        app.reports.ledger.journals.snapshot = Load::Ready(Snapshot::parse(&json!({"status": {}})));
        app.reports.ledger.journals.events = Load::Ready(json!({"summary": {}}));
        app.reports.ledger.journals.tab = Tab::Reportable;
        app.reports.ledger.journals.invalidate();
        assert!(step(&mut app));
        // The refresh lands while both are out.
        app.reports.ledger.journals.invalidate();
        assert!(step(&mut app), "asked again");
        assert!(!step(&mut app), "once");
        on_reply(&mut app, "reports.ledger.journals", &snap(1.0));
        on_reply(&mut app, "reports.ledger.events", &events("before"));
        let j = &app.reports.ledger.journals;
        assert!(
            j.stale_snapshot && j.stale_events,
            "still waiting for the fresh reads"
        );
        on_reply(&mut app, "reports.ledger.journals", &snap(2.0));
        on_reply(&mut app, "reports.ledger.events", &events("after"));
        let j = &app.reports.ledger.journals;
        match (&j.snapshot, &j.events) {
            (Load::Ready(s), Load::Ready(e)) => {
                assert_eq!(s.status.journal_entry_count, 2.0);
                assert_eq!(e["n"], "after");
            }
            _ => panic!("the fresh answers land"),
        }
        assert!(!j.stale_snapshot && !j.stale_events);
    }

    /// A USD book's journal amounts render in dollars: the daemon's `*Eur` names are historical.
    #[test]
    fn recent_entries_render_in_the_book_currency() {
        let snap = Snapshot::parse(&json!({
            "status": {"journalEntryCount": 1},
            "entryTypes": [{"type": "sell", "count": 1, "gainLossEur": 210.25}],
            "recent": [{
                "date": "2025-03-01 10:00", "type": "sell", "wallet": "Cold",
                "asset": "BTC", "quantity": -0.01, "fiatValueEur": 950.5, "gainLossEur": 210.25,
            }],
        }));
        let cells = entry_cells(&snap.recent[0], "USD");
        assert_eq!(cells[4], "$950.50");
        assert_eq!(cells[5], "$210.25");
        assert!(cells.iter().all(|c| !c.contains('€')), "{cells:?}");
        assert_eq!(
            crate::format::fiat(group_entry_types(&snap.entry_types)[0].gain_loss, "USD"),
            "$210.25"
        );
        let eur = entry_cells(&snap.recent[0], "EUR");
        assert_eq!(eur[4], "€\u{a0}950,50");
    }
}
