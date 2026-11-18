//! The Overview screen (ui-tauri/src/routes/Overview.tsx + components/overview-dashboard).
//!
//! The welcome row, then the widgets as a home screen (`grid`, Rune only: the owner asked for
//! widgets in the manner of a phone's home screen; the Tauri Overview keeps its fixed layout):
//! borderless tiles in fixed sizes on a uniform grid, packed in order, each laid out for its
//! size. A tile opens its destination; its settings are in its context menu; "Edit" (or a long
//! press) starts the edit mode, where tiles are dragged into order, removed and added from the
//! gallery. The default home is the key figures, the BTC activity chart across, Recent
//! Transactions and Holdings by Source side by side, then Tax year; Money in and out and
//! Holdings by Asset (an owner-requested widget with no web counterpart yet) are in the gallery.

pub(crate) mod add_connection;
mod age;
mod asset_card;
pub(crate) mod assets;
mod attention;
pub(crate) mod catalog;
mod goal;
pub(crate) mod grid;
mod holdings;
mod holdings_layouts;
pub(crate) mod holdings_model;
pub(crate) mod l10n;
mod menu;
mod recent;
mod stats;
mod welcome;
mod widgets;

use std::cell::OnceCell;
use std::sync::Arc;

use std::cell::Cell;

use rune::{Cx, El, ImageCache, KeyEvent, TextEdit, Tx, col, row, text};

use crate::app::{App, Load, Reply};
use crate::theme::Tk;
use crate::ui::surface;

#[derive(Default)]
pub struct OverviewUi {
    /// The Add connection dialog.
    pub add_open: bool,
    pub add_search: TextEdit,
    pub add_category: Option<&'static str>,
    /// The source selected in the picker (the catalog's first by default).
    pub add_selected: Option<&'static str>,
    /// SYNTHETIC rows a scene adds to Holdings by Asset (never the book's own data).
    pub synthetic_assets: Vec<assets::AssetRow>,
    /// Holdings by Source layout, order, filter and folding (`holdingsView`, persisted).
    pub holdings: holdings_model::HoldingsView,
    /// The open context menu (a widget's, with its settings, or the page's), its open submenu,
    /// and where the pointer opened it (window points; none from the keyboard or a button).
    pub menu_open: bool,
    pub menu_target: grid::MenuTarget,
    pub menu_sub: Option<&'static str>,
    pub menu_at: Option<(f32, f32)>,
    /// The folded emptied sources, listed.
    pub empty_open: bool,
    /// A scene's pinned clock (epoch seconds) for the trend's and the widgets' months.
    pub now_override: Option<f64>,
    /// Which widgets are placed, in which order and size (`overviewLayout`, persisted for the
    /// whole app).
    pub layout: grid::model::Layout,
    /// The edit mode, its drag and the gallery.
    pub edit: grid::Edit,
    /// What the last view laid out, for the handlers: the grid's geometry, whether the system
    /// asks for reduced motion, and the chart's fitted plot height.
    pub geo: Cell<Option<grid::Geo>>,
    pub reduced: Cell<bool>,
    pub chart_fit: Cell<grid::ChartFit>,
    /// The Stack goal sheet: open, its field, and why the typed target was refused.
    pub goal_open: bool,
    pub goal_edit: TextEdit,
    pub goal_error: Option<goal::GoalError>,
    /// Rail logos (SVG) rasterized on the cache's worker; built on first use.
    images: OnceCell<ImageCache>,
}

impl OverviewUi {
    pub(crate) fn images(&self) -> &ImageCache {
        self.images.get_or_init(|| ImageCache::new(Arc::new(|| {})))
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("ov.") {
        return;
    }
    stats::on_reply(app, tag, r);
}

/// The edit mode's clock: the long press and the wiggle. True when the view changed.
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    grid::poll(app, tx)
}

/// Keys while the Overview is shown (Escape closes its sheets and dialogs); true consumes.
pub fn on_key(app: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    if app.ov.menu_open && ev.key == rune::Key::Escape {
        // Escape closes the innermost level first, as Radix does.
        if app.ov.menu_sub.take().is_none() {
            app.ov.menu_open = false;
            app.ov.menu_at = None;
        }
        return true;
    }
    if app.ov.add_open && add_connection::on_key(app, ev, tx) {
        return true;
    }
    if app.ov.goal_open {
        // The sheet is modal: Escape closes it, Enter saves.
        match ev.key {
            rune::Key::Escape => app.ov.goal_open = false,
            rune::Key::Enter => goal::save(app, tx),
            _ => return false,
        }
        return true;
    }
    grid::on_key(app, ev, tx)
}

/// Opens the transaction detail sheet for a transaction id / txid / explorer id
/// (`useOverviewTransactionDetail.openTransactionDetail`). Called by single activity dots.
pub fn open_transaction_detail(app: &mut App, id: &str) {
    // The shared docked panel (txdetail), as the web's Overview now uses.
    crate::txdetail::open(app, id, "details");
}

/// Opens the Add connection dialog (WelcomeSection button, chart empty state).
pub fn open_add_connection(app: &mut App) {
    app.ov.add_open = true;
    app.ov.add_search = TextEdit::new("");
    app.ov.add_search.focused = true;
    // The dialog opens on the Wallets category with its first source selected.
    app.ov.add_category = Some("wallets");
    app.ov.add_selected = None;
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    // The app's spacing scale: the page inset, and the section gap under the welcome row (the
    // tiles keep the card gap between them, `grid::Geo`).
    let gutter = tk.page_pad();
    let gap = tk.section_gap();
    let page = col()
        .key("overview")
        .w_full()
        .padding(gutter)
        .gap(gap)
        .fill(tk.background)
        .busy(app.busy.syncing || app.busy.processing || app.busy.rate_refresh);
    // The grid's width: the page's, less its gutters. Layout reads settle within the first
    // frame; the constant only shapes the view's first, unseen pass.
    let width = cx
        .layout_size("overview")
        .map(|[w, _]| w - 2.0 * gutter)
        .unwrap_or(1100.0);
    let page = match &app.overview {
        // Each shown widget's frame with the shared loading state, where it will be.
        Load::Idle | Load::Loading => page
            .child(skeleton_head(tk))
            .child(grid::view(app, cx, tk, None, width, layer)),
        Load::Failed(msg) => page.child(notice(tk, msg)),
        Load::Ready(s) => page
            .child(welcome::view(app, tk, s, width))
            .child(grid::view(app, cx, tk, Some(s), width, layer)),
    };
    grid::mount_menu(app, tk, layer);
    goal::mount_sheet(app, tk, layer);
    if app.ov.add_open {
        add_connection::dialog(app, cx, tk, layer);
    }
    page
}

/// The chart's plot block height at its widget's rows (`src/chart` reads it).
pub fn chart_block_h(app: &App, tk: &Tk) -> f32 {
    grid::chart_block_h(app, tk)
}

/// The chart header's actions in its tile: expand.
pub fn chart_actions(app: &App, tk: &Tk, expand: El) -> El {
    grid::chart_actions(app, tk, expand)
}

/// `ScreenSkeleton`'s header bars; the widgets below show their own loading state.
fn skeleton_head(tk: &Tk) -> El {
    let bar = |w: f32, h: f32| {
        row()
            .w(w)
            .h(h)
            .radius(tk.r(0.375))
            .fill(tk.muted.alpha(0.7))
    };
    row()
        .key("overview-skeleton")
        .w_full()
        .justify_between()
        .center_y()
        .label(crate::i18n::t("common:state.loading"))
        .busy(true)
        .child(
            col()
                .gap(tk.r(0.375))
                .child(bar(tk.r(8.0), tk.r(1.25)))
                .child(bar(tk.r(14.0), tk.r(0.75))),
        )
        .child(
            row()
                .gap(tk.r(0.5))
                .child(bar(96.0, tk.r(2.0)))
                .child(bar(112.0, tk.r(2.0))),
        )
}

/// `ScreenNotice`.
fn notice(tk: &Tk, msg: &str) -> El {
    surface(tk)
        .border_layout()
        .key("overview-notice")
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.75))
        .gap(tk.r(0.25))
        .role("alert")
        .child(
            text(crate::i18n::t("overview:screen.unavailableTitle"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
        .child(
            text(localized_notice(msg))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
}

/// The notice body: app.rs words the two fixed messages in English; daemon errors are data.
fn localized_notice(msg: &str) -> String {
    const NO_DATA: &str = "No real overview data is available for the current book.";
    const UNREADABLE: &str = "Kassiber could not read real overview data.";
    if msg == NO_DATA {
        crate::i18n::t("overview:screen.noRealData")
    } else if let Some(rest) = msg.strip_prefix(UNREADABLE) {
        format!(
            "{}{rest}",
            crate::i18n::t("overview:screen.unavailableBody")
        )
    } else {
        msg.to_owned()
    }
}

/// A relative time in the UI language (`just now`, `5m ago`, `vor 5 Min.` …), for the chart's
/// "priced …" note.
pub fn relative_time(ts: &str) -> Option<String> {
    l10n::relative(ts, crate::format::now_secs())
}

/// The completeness copy and hint in the UI language (`portfolioCompletenessDetail`).
pub fn completeness_text(s: &crate::model::Snapshot) -> Option<(String, String)> {
    l10n::completeness(s)
}

/// `transactionTypeLabel`: the daemon's English type labels in the UI language; tags verbatim.
pub fn transaction_type_label(label: &str) -> String {
    l10n::transaction_type(label)
}

// ---- Scenes ----------------------------------------------------------------------------------

use grid::model::{Size, WidgetId as W};

/// The SYNTHETIC EUR cash row the future-asset scenes add (EUR 1,250.00). Bitcoin-only book data
/// stays as it is; this row exists only in these scenes and is badged on screen.
fn synthetic_eur(a: &mut App) {
    show_assets(a);
    let cur = a
        .snapshot()
        .map(|s| s.fiat_currency())
        .unwrap_or_else(|| "EUR".into());
    a.ov.synthetic_assets = vec![assets::fiat_cash(
        "EUR",
        "Euro cash (SYNTHETIC)",
        125_000,
        &cur,
        true,
    )];
}

/// SYNTHETIC: edits the raw overview snapshot (never the book) and re-reads it, for states the
/// demo book does not have.
fn edit_raw(a: &mut App, f: impl FnOnce(&mut serde_json::Value)) {
    if let Some(r) = a.reads.get_mut("ui.overview.snapshot") {
        f(&mut r.data);
        if let Ok(s) = crate::model::Snapshot::parse(&r.data) {
            a.overview = Load::Ready(s);
        }
    }
}

fn conn_mut<'a>(v: &'a mut serde_json::Value, label: &str) -> Option<&'a mut serde_json::Value> {
    v["connections"]
        .as_array_mut()?
        .iter_mut()
        .find(|c| c["label"].as_str() == Some(label))
}

fn layout(a: &mut App, l: holdings_model::Layout) {
    a.ov.holdings.layout = l;
}

/// SYNTHETIC: two sources emptied (balance zero, history kept).
fn emptied(a: &mut App) {
    edit_raw(a, |v| {
        for label in ["Regtest Mining Rig", "Regtest Liquid Live Sync"] {
            if let Some(c) = conn_mut(v, label) {
                c["balance"] = 0.into();
            }
        }
    });
}

/// SYNTHETIC: a clock pinned to the fixtures' `now` (15 Nov 2026, just after the demo book's last
/// row), so its last twelve months have flows (and the Money in and out widget placed; it starts
/// in the gallery).
fn flows_clock(a: &mut App) {
    a.ov.now_override = Some(crate::fixtures::now());
    a.ov.layout.add(W::Flows, Size::Medium);
}

fn show_assets(a: &mut App) {
    a.ov.layout.add(W::Assets, Size::Medium);
}

/// A home of exactly these tiles, in this order.
fn home(a: &mut App, tiles: &[(W, Size)]) {
    a.ov.now_override = Some(crate::fixtures::now());
    a.ov.layout = grid::model::Layout::empty();
    for (id, size) in tiles {
        a.ov.layout.add(*id, *size);
    }
}

/// Tax year lifted out of the default home in edit mode and carried down over Recent
/// transactions, at the 1440 pt window's grid.
fn scene_drag(a: &mut App) {
    a.ov.edit.on = true;
    let tk = Tk::new(a.prefs.theme != crate::app::ThemeMode::Light, 1.0);
    let geo = grid::Geo::new(&tk, 1144.0);
    let layout = a.ov.layout.clone();
    a.ov.edit
        .scene_drag(&layout, geo, W::TaxYear, -420.0, 640.0);
}

fn context_menu(a: &mut App, id: W, at: (f32, f32)) {
    a.ov.menu_open = true;
    a.ov.menu_target = grid::MenuTarget::Widget(id);
    a.ov.menu_at = Some(at);
}

/// SYNTHETIC: an Austrian tax-free split with the given status.
fn tax_split(a: &mut App, status: &str) {
    let status = status.to_owned();
    edit_raw(a, move |v| {
        v["taxFreeBalance"] = serde_json::json!({
            "rule": "austrian_altbestand", "jurisdictionCode": "AT", "fiatCurrency": "EUR", "status": status,
            "taxFreeQuantitySats": 1_200_000_000i64, "taxableQuantitySats": 8_658_445_804i64, "totalQuantitySats": 9_858_445_804i64,
            "needsJournals": status == "needs_journals", "quarantines": if status == "quarantines" { 2 } else { 0 },
        });
    });
}

// ---- The owner-requested widgets' scenes (Rune only) -----------------------------------------------

/// SYNTHETIC: things that need attention: a review queue with quarantines, journals waiting,
/// one source failed and one stale (the badges and the freshness read, never the book).
fn attention_synthetic(a: &mut App) {
    a.badges = Some(crate::model::Badges {
        quarantine: Some(2.0),
        quarantine_attention: Some(2.0),
        journals_needs_processing: true,
        swaps: Some(1.0),
        review: Some(3.0),
    });
    if let Some(r) = a.reads.get_mut("ui.freshness.status")
        && let Some(src) = r.data["sources"].as_array_mut()
    {
        if let Some(x) = src.get_mut(0) {
            x["status"] = "failed".into();
            x["last_error_message"] = "connection refused".into();
        }
        if let Some(x) = src.get_mut(1) {
            x["state"] = "stale".into();
        }
    }
}

/// The three new widgets at one size, with Key figures or Tax year beside them, a goal set and
/// a clock pinned to 20 Nov 2025 (so the last twelve months have acquisitions for the pace).
fn new_widgets(a: &mut App, size: Size) {
    let fourth = if size == Size::Small {
        (W::Stats, Size::Small)
    } else {
        (W::TaxYear, Size::Medium)
    };
    home(
        a,
        &[
            (W::Attention, size),
            (W::Age, size),
            (W::Goal, size),
            fourth,
        ],
    );
    a.ov.layout.set_goal_sats(Some(100 * 100_000_000));
    attention_synthetic(a);
}

/// SYNTHETIC: the active book is Austrian, with a current tax-free split.
fn austrian(a: &mut App) {
    if let Some(r) = a.reads.get_mut("ui.profiles.snapshot") {
        let active = r.data["activeProfileId"].clone();
        for w in r.data["workspaces"].as_array_mut().into_iter().flatten() {
            for p in w["profiles"].as_array_mut().into_iter().flatten() {
                if p["id"] == active {
                    p["taxCountry"] = "at".into();
                }
            }
        }
    }
    tax_split(a, "current");
}

fn goal_sheet(a: &mut App, text: &str, error: Option<goal::GoalError>) {
    goal::open_sheet(a);
    a.ov.goal_edit = TextEdit::new(text);
    a.ov.goal_edit.focused = true;
    a.ov.goal_error = error;
}

/// Holdings by Source, the widgets and Holdings by Asset states. The `overview-holdings-*` and
/// `overview-widgets-*` names without "synthetic" have Tauri parity scenes
/// (parity/scenes.d/overview.json); snapshot them tall (`--size 1440x1140` and up).
pub static SCENES: &[crate::app::Scene] = &[
    ("overview-holdings-list", |_| {}),
    ("overview-holdings-tiles", |a| {
        layout(a, holdings_model::Layout::Tiles)
    }),
    ("overview-holdings-table", |a| {
        layout(a, holdings_model::Layout::Table)
    }),
    ("overview-holdings-trend", |a| {
        layout(a, holdings_model::Layout::Trend)
    }),
    ("overview-holdings-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light
    }),
    ("overview-holdings-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De
    }),
    ("overview-holdings-tiles-light", |a| {
        layout(a, holdings_model::Layout::Tiles);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-holdings-table-de", |a| {
        layout(a, holdings_model::Layout::Table);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-holdings-trend-light", |a| {
        layout(a, holdings_model::Layout::Trend);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-holdings-fiat", |a| {
        a.prefs.currency = crate::app::Currency::Fiat
    }),
    ("overview-holdings-tiles-fiat", |a| {
        layout(a, holdings_model::Layout::Tiles);
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    ("overview-holdings-table-fiat", |a| {
        layout(a, holdings_model::Layout::Table);
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    ("overview-holdings-sort-name", |a| {
        a.ov.holdings.sort = holdings_model::Sort::Name
    }),
    ("overview-holdings-sort-activity", |a| {
        a.ov.holdings.sort = holdings_model::Sort::Activity
    }),
    ("overview-holdings-filtered", |a| {
        a.ov.holdings.rails = vec![holdings_model::Group::Liquid]
    }),
    ("overview-holdings-limit-all", |a| a.ov.holdings.limit = 0),
    ("overview-holdings-hide-amounts", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts
    }),
    ("overview-holdings-hide-all", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAll
    }),
    ("overview-holdings-tiles-hide-all", |a| {
        layout(a, holdings_model::Layout::Tiles);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-holdings-table-hide-all", |a| {
        layout(a, holdings_model::Layout::Table);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-holdings-trend-hide-all", |a| {
        layout(a, holdings_model::Layout::Trend);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-holdings-menu", |a| a.ov.menu_open = true),
    ("overview-holdings-menu-rails", |a| {
        a.ov.menu_open = true;
        a.ov.menu_sub = Some("rails");
    }),
    ("overview-holdings-menu-empty", |a| {
        a.ov.menu_open = true;
        a.ov.menu_sub = Some("empty");
    }),
    ("overview-holdings-menu-limit", |a| {
        a.ov.menu_open = true;
        a.ov.menu_sub = Some("limit");
    }),
    ("overview-holdings-menu-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        a.ov.menu_open = true;
    }),
    ("overview-holdings-narrow", |_| {}),
    ("overview-holdings-trend-narrow", |a| {
        layout(a, holdings_model::Layout::Trend)
    }),
    ("overview-widgets-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    // ---- Rune-only states (SYNTHETIC data or clock; no Tauri counterpart in the demo book) ----
    ("overview-holdings-synthetic-emptied", emptied),
    ("overview-holdings-synthetic-emptied-open", |a| {
        emptied(a);
        a.ov.empty_open = true;
    }),
    ("overview-holdings-synthetic-emptied-show", |a| {
        emptied(a);
        a.ov.holdings.empty = holdings_model::Empty::Show;
        layout(a, holdings_model::Layout::Tiles);
    }),
    ("overview-holdings-synthetic-rails-one", |a| {
        a.ov.holdings.rails = vec![holdings_model::Group::Lightning];
        a.ov.menu_open = true;
        a.ov.menu_sub = Some("rails");
    }),
    ("overview-holdings-synthetic-sync-states", |a| {
        edit_raw(a, |v| {
            if let Some(c) = conn_mut(v, "Regtest Spending Wallet") {
                c["status"] = "syncing".into();
            }
            if let Some(c) = conn_mut(v, "Regtest Mining Rig") {
                c["status"] = "error".into();
            }
            if let Some(c) = conn_mut(v, "Regtest Liquid Operations") {
                c["kind"] = "kraken".into();
                c["chain"] = "".into();
            }
        });
        layout(a, holdings_model::Layout::Table);
    }),
    // A daemon without the month-end balances: the trend falls back to the list and leaves
    // the menu; the Bitcoin balance tile falls back to the history's end without `totalBtc`.
    ("overview-holdings-synthetic-old-daemon", |a| {
        edit_raw(a, |v| {
            for c in v["connections"].as_array_mut().into_iter().flatten() {
                c.as_object_mut().map(|o| o.remove("monthBalances"));
            }
        });
        layout(a, holdings_model::Layout::Trend);
        a.ov.menu_open = true;
    }),
    ("overview-holdings-synthetic-trend-moving", |a| {
        flows_clock(a);
        layout(a, holdings_model::Layout::Trend);
    }),
    ("overview-widgets-synthetic-flows", flows_clock),
    ("overview-widgets-synthetic-flows-fiat", |a| {
        flows_clock(a);
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    ("overview-widgets-synthetic-flows-light-de", |a| {
        flows_clock(a);
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-widgets-synthetic-flows-hide-amounts", |a| {
        flows_clock(a);
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
    }),
    ("overview-widgets-synthetic-flows-hide-all", |a| {
        flows_clock(a);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-widgets-synthetic-tax-split", |a| {
        tax_split(a, "current")
    }),
    ("overview-widgets-synthetic-tax-split-journals", |a| {
        tax_split(a, "needs_journals")
    }),
    ("overview-widgets-synthetic-tax-split-quarantines", |a| {
        tax_split(a, "quarantines");
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-widgets-synthetic-basis-incomplete", |a| {
        edit_raw(a, |v| {
            v["fiat"]["completeness"]["costBasisComplete"] = false.into();
            v["fiat"]["completeness"]["state"] = "incomplete".into();
        });
    }),
    // ---- The home screen (Rune only; `grid`) ----
    ("overview-layout", |_| {}),
    ("overview-layout-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light
    }),
    ("overview-layout-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De
    }),
    ("overview-layout-light-de", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-layout-fiat", |a| {
        a.prefs.currency = crate::app::Currency::Fiat
    }),
    ("overview-layout-hide-amounts", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts
    }),
    ("overview-layout-hide-all", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAll
    }),
    ("overview-layout-loading", |a| a.overview = Load::Loading),
    // SYNTHETIC: a book without connections, balances, history or transactions.
    ("overview-layout-empty-book", |a| {
        edit_raw(a, |v| {
            for k in [
                "connections",
                "txs",
                "balanceSeries",
                "portfolioSeries",
                "activity",
            ] {
                if v[k].is_array() {
                    v[k] = serde_json::json!([]);
                }
            }
            v["totalBtc"] = 0.into();
            for k in [
                "eurBalance",
                "eurCostBasis",
                "eurUnrealized",
                "eurRealizedYTD",
            ] {
                if !v["fiat"][k].is_null() {
                    v["fiat"][k] = 0.into();
                }
            }
        });
        a.ov.layout.add(W::Flows, Size::Medium);
        a.ov.layout.add(W::Assets, Size::Small);
    }),
    // The sizes: every widget at every size it comes in, across these three homes.
    ("overview-sizes-small", |a| {
        home(
            a,
            &[
                (W::Stats, Size::Small),
                (W::TaxYear, Size::Small),
                (W::Flows, Size::Medium),
                (W::Recent, Size::Medium),
                (W::Holdings, Size::Medium),
                (W::Assets, Size::Small),
            ],
        )
    }),
    ("overview-sizes-medium", |a| {
        home(
            a,
            &[
                (W::Stats, Size::Medium),
                (W::TaxYear, Size::Medium),
                (W::Chart, Size::Large),
                (W::Flows, Size::Large),
                (W::Assets, Size::Medium),
            ],
        )
    }),
    ("overview-sizes-large", |a| {
        home(
            a,
            &[
                (W::Stats, Size::Large),
                (W::Recent, Size::Large),
                (W::Chart, Size::XLarge),
                (W::Holdings, Size::Large),
            ],
        )
    }),
    ("overview-sizes-small-light-de", |a| {
        home(
            a,
            &[
                (W::Stats, Size::Small),
                (W::TaxYear, Size::Small),
                (W::Flows, Size::Medium),
                (W::Recent, Size::Medium),
                (W::Holdings, Size::Medium),
                (W::Assets, Size::Small),
            ],
        );
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-layout-context-menu", |a| {
        context_menu(a, W::Recent, (300.0, 640.0))
    }),
    ("overview-layout-context-menu-holdings", |a| {
        context_menu(a, W::Holdings, (900.0, 600.0))
    }),
    ("overview-layout-context-menu-size", |a| {
        context_menu(a, W::Stats, (240.0, 190.0));
        a.ov.menu_sub = Some("size");
    }),
    ("overview-layout-page-menu", |a| {
        a.ov.menu_open = true;
        a.ov.menu_target = grid::MenuTarget::Page;
        a.ov.menu_at = Some((700.0, 760.0));
    }),
    ("overview-layout-edit", |a| a.ov.edit.on = true),
    ("overview-layout-edit-light-de", |a| {
        a.ov.edit.on = true;
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-layout-dragging", scene_drag),
    ("overview-layout-gallery", |a| {
        a.ov.now_override = Some(crate::fixtures::now());
        a.ov.edit.on = true;
        grid::scene_gallery(a, W::Flows, Size::Medium);
    }),
    ("overview-layout-gallery-chart", |a| {
        a.ov.edit.on = true;
        grid::scene_gallery(a, W::Chart, Size::Large);
    }),
    ("overview-layout-gallery-light-de", |a| {
        a.ov.edit.on = true;
        grid::scene_gallery(a, W::Assets, Size::Small);
        a.prefs.theme = crate::app::ThemeMode::Light;
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-layout-all", |a| {
        flows_clock(a);
        show_assets(a);
    }),
    ("overview-layout-none", |a| {
        a.ov.layout = grid::model::Layout::empty()
    }),
    // The owner's default at the window widths (snapshot with `--size 1440x1300`, 1728, 1920, 980).
    ("overview-default", |_| {}),
    ("overview-default-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light
    }),
    // The new widgets, each at each size (SYNTHETIC attention items; the goal set to ₿100).
    ("overview-new-small", |a| new_widgets(a, Size::Small)),
    ("overview-new-small-light", |a| {
        new_widgets(a, Size::Small);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-new-small-de", |a| {
        new_widgets(a, Size::Small);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-new-small-hide-amounts", |a| {
        new_widgets(a, Size::Small);
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
    }),
    ("overview-new-small-hide-all", |a| {
        new_widgets(a, Size::Small);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-new-medium", |a| new_widgets(a, Size::Medium)),
    ("overview-new-medium-light", |a| {
        new_widgets(a, Size::Medium);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-new-medium-de", |a| {
        new_widgets(a, Size::Medium);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-new-medium-hide-amounts", |a| {
        new_widgets(a, Size::Medium);
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
    }),
    ("overview-new-medium-hide-all", |a| {
        new_widgets(a, Size::Medium);
        a.prefs.privacy = crate::app::Privacy::HideAll;
    }),
    ("overview-new-loading", |a| {
        new_widgets(a, Size::Medium);
        a.overview = Load::Loading;
    }),
    // Nothing needs attention: the widget leaves the home screen; editing shows its calm line.
    ("overview-attention-calm", |a| {
        home(
            a,
            &[
                (W::Attention, Size::Medium),
                (W::Stats, Size::Medium),
                (W::Age, Size::Small),
                (W::Goal, Size::Small),
            ],
        )
    }),
    ("overview-attention-calm-edit", |a| {
        home(
            a,
            &[
                (W::Attention, Size::Medium),
                (W::Stats, Size::Medium),
                (W::Age, Size::Small),
                (W::Goal, Size::Small),
            ],
        );
        a.ov.edit.on = true;
    }),
    // A goal ahead of the stack (₿150), Small and Medium side by side, for the layout variants.
    ("overview-goal-progress", |a| {
        home(a, &[(W::Goal, Size::Medium), (W::Stats, Size::Medium)]);
        a.ov.layout.set_goal_sats(Some(150 * 100_000_000));
    }),
    ("overview-goal-progress-small", |a| {
        home(
            a,
            &[
                (W::Goal, Size::Small),
                (W::Age, Size::Small),
                (W::Stats, Size::Medium),
            ],
        );
        a.ov.layout.set_goal_sats(Some(150 * 100_000_000));
    }),
    ("overview-goal-empty", |a| {
        home(
            a,
            &[
                (W::Goal, Size::Small),
                (W::Age, Size::Small),
                (W::Stats, Size::Medium),
            ],
        )
    }),
    ("overview-goal-empty-medium", |a| {
        home(a, &[(W::Goal, Size::Medium), (W::Stats, Size::Medium)])
    }),
    ("overview-goal-reached", |a| {
        home(
            a,
            &[
                (W::Goal, Size::Small),
                (W::Stats, Size::Small),
                (W::Age, Size::Medium),
            ],
        );
        a.ov.layout.set_goal_sats(Some(50 * 100_000_000));
    }),
    ("overview-goal-reached-medium", |a| {
        home(a, &[(W::Goal, Size::Medium), (W::Stats, Size::Medium)]);
        a.ov.layout.set_goal_sats(Some(50 * 100_000_000));
    }),
    ("overview-goal-sheet", |a| {
        new_widgets(a, Size::Medium);
        goal_sheet(a, "100", None);
    }),
    ("overview-goal-sheet-error", |a| {
        new_widgets(a, Size::Medium);
        goal_sheet(a, "0.000000001", Some(goal::GoalError::TooPrecise));
    }),
    ("overview-goal-sheet-de", |a| {
        new_widgets(a, Size::Medium);
        goal_sheet(a, "1,5", None);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-goal-sheet-light", |a| {
        home(a, &[(W::Goal, Size::Medium)]);
        goal_sheet(a, "abc", Some(goal::GoalError::NotANumber));
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-goal-menu", |a| {
        new_widgets(a, Size::Medium);
        context_menu(a, W::Goal, (1100.0, 200.0));
    }),
    ("overview-age-austria", |a| {
        new_widgets(a, Size::Medium);
        austrian(a);
    }),
    ("overview-age-austria-de", |a| {
        new_widgets(a, Size::Medium);
        austrian(a);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-new-empty-book", |a| {
        new_widgets(a, Size::Medium);
        a.badges = None;
        edit_raw(a, |v| {
            for k in [
                "connections",
                "txs",
                "balanceSeries",
                "portfolioSeries",
                "activity",
            ] {
                if v[k].is_array() {
                    v[k] = serde_json::json!([]);
                }
            }
            v["totalBtc"] = 0.into();
            v["balanceSummary"]["totalBtc"] = 0.into();
        });
        a.ov.edit.on = true;
    }),
    ("overview-layout-gallery-attention", |a| {
        a.ov.edit.on = true;
        attention_synthetic(a);
        grid::scene_gallery(a, W::Attention, Size::Medium);
    }),
    ("overview-layout-gallery-age", |a| {
        a.ov.edit.on = true;
        grid::scene_gallery(a, W::Age, Size::Medium);
    }),
    ("overview-layout-gallery-goal", |a| {
        a.ov.edit.on = true;
        grid::scene_gallery(a, W::Goal, Size::Small);
    }),
    ("overview-layout-gallery-goal-de", |a| {
        a.ov.edit.on = true;
        a.prefs.lang = crate::i18n::Lang::De;
        grid::scene_gallery(a, W::Goal, Size::Medium);
    }),
    // Holdings by Asset states (the widget placed; it starts in the gallery).
    // Snapshot these tall (e.g. `--size 1440x1700`): the tile sits below the fold.
    ("overview-assets", |a| show_assets(a)),
    ("overview-assets-fiat", |a| {
        show_assets(a);
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    ("overview-assets-synthetic", |a| {
        synthetic_eur(a);
    }),
    ("overview-assets-synthetic-fiat", |a| {
        synthetic_eur(a);
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    ("overview-assets-synthetic-de", |a| {
        synthetic_eur(a);
        a.prefs.lang = crate::i18n::Lang::De;
    }),
    ("overview-assets-synthetic-light", |a| {
        synthetic_eur(a);
        a.prefs.theme = crate::app::ThemeMode::Light;
    }),
    ("overview-assets-synthetic-hidden", |a| {
        synthetic_eur(a);
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
    }),
    // SYNTHETIC: no market rate, so bitcoin has no value in fiat mode ("No price").
    ("overview-assets-no-price", |a| {
        synthetic_eur(a);
        if let Load::Ready(s) = &mut a.overview {
            s.fiat.completeness.market_rate_missing = true;
        }
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    // SYNTHETIC: other crypto assets a `crypto`-scoped wallet keeps (`otherAssets`, which the
    // demo book has none of), one priced and one without a price, in fiat mode.
    ("overview-assets-other-crypto", |a| {
        show_assets(a);
        edit_raw(a, |r| {
            r["otherAssets"] = serde_json::json!([
                {"asset": "ETH", "quantity": "3", "quantityUnits": 300_000_000_000_i64,
                 "walletCount": 2, "source": "books", "marketValue": 6000.0},
                {"asset": "SOL", "quantity": "12.5", "quantityUnits": 1_250_000_000_000_i64,
                 "walletCount": 1, "source": "transactions", "marketValue": null}
            ]);
        });
        a.prefs.currency = crate::app::Currency::Fiat;
    }),
    // SYNTHETIC: every balance zeroed, the empty state.
    ("overview-assets-empty", |a| {
        show_assets(a);
        if let Load::Ready(s) = &mut a.overview {
            for c in &mut s.connections {
                c.balance = 0.0;
            }
        }
    }),
];
