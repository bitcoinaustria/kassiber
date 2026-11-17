//! routes/ConnectionDetail.tsx: a wallet connection's detail (`ConnectionDetailView`).
//!
//! - `mod` (here): state, the screen's local daemon reads (the same kinds and args the web
//!   reads on mount), replies, keys, the headless prime and scenes.
//! - `actions`: Refresh / Full rescan (egress, press only), edit, remove, reveal, rollback.
//! - `hero`: the page header, the balance hero and its fact list.
//! - `history`: `WalletBalanceHistory` (+ `useWalletBalanceHistory`).
//! - `notices`: backend required, sync error, reconciliation and attention notices.
//! - `txs`: the tabs and `WalletTransactionsTable`.
//! - `coins`: `WalletCoinsBreakdown` and `UtxosInventoryPanel`.
//! - `info`: the Details tab (connection details, `ImportRunsPanel`, Samourai, BTCPay).
//! - `dialogs`: the More actions menu and the edit / reveal / remove / rollback dialogs.
//! - `ui`: shadcn pieces drawn locally.

mod actions;
mod coins;
mod dialogs;
mod hero;
mod history;
mod info;
mod notices;
mod txs;
pub mod ui;

use rune::{Cx, El, Key, KeyEvent, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::conn::Conn;
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;

/// Rows of `ui.transactions.list` the page loads (`RECENT_TRANSACTION_PREVIEW_LIMIT`).
pub const RECENT_LIMIT: usize = 100;
/// The daemon's balance-history row cap (`HISTORY_ROW_LIMIT`).
pub const HISTORY_LIMIT: usize = 500;
pub const MAX_GAP_LIMIT: i64 = 5000;
pub const CHANGE_ACK: &str = "CHANGE LOCAL DATA";
pub const DELETE_ACK: &str = "DELETE LOCAL DATA";
pub const REVEAL_ACK: &str = "COPY LOCAL SECRET";
pub const CLEAR_BACKEND: &str = "__kassiber_clear_backend__";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Transactions,
    Coins,
    Details,
}

impl Tab {
    pub const ALL: [Tab; 3] = [Tab::Transactions, Tab::Coins, Tab::Details];
    pub fn key(self) -> &'static str {
        match self {
            Tab::Transactions => "transactions",
            Tab::Coins => "coins",
            Tab::Details => "details",
        }
    }
}

/// `BalanceRange`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Range {
    Y1,
    Y3,
    #[default]
    All,
}

impl Range {
    pub const ALL: [Range; 3] = [Range::Y1, Range::Y3, Range::All];
    pub fn months(self) -> Option<usize> {
        match self {
            Range::Y1 => Some(12),
            Range::Y3 => Some(36),
            Range::All => None,
        }
    }
    pub fn key(self) -> &'static str {
        match self {
            Range::Y1 => "1y",
            Range::Y3 => "3y",
            Range::All => "all",
        }
    }
}

/// `UtxoSortValue`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Default,
    SizeDesc,
    SizeAsc,
    DateDesc,
    DateAsc,
    OutpointAsc,
    OutpointDesc,
}

/// A daemon read: in flight, answered, or failed.
#[derive(Clone, Debug, Default)]
pub enum Read<T> {
    #[default]
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Read<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Read::Ready(v) => Some(v),
            _ => None,
        }
    }
    pub fn loading(&self) -> bool {
        matches!(self, Read::Loading)
    }
}

/// Which select of the edit dialog is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    Backend,
    BtcpayInstance,
}

#[derive(Default)]
pub struct EditForm {
    pub label: TextEdit,
    pub passphrase: TextEdit,
    pub ack: TextEdit,
    pub material: TextEdit,
    pub script_types: Vec<String>,
    pub gap: TextEdit,
    pub birthday: TextEdit,
    pub store_id: TextEdit,
    pub payment_method: TextEdit,
    pub backend: String,
    pub source_file: TextEdit,
    pub deprecated: bool,
    pub clear_provenance: bool,
    pub removed_routes: Vec<String>,
    pub error: Option<String>,
    pub pending: bool,
}

#[derive(Default)]
pub struct DeleteForm {
    pub passphrase: TextEdit,
    pub ack: TextEdit,
    pub confirm: TextEdit,
    pub error: Option<String>,
    pub pending: bool,
}

#[derive(Default)]
pub struct RevealForm {
    pub passphrase: TextEdit,
    pub ack: TextEdit,
    pub error: Option<String>,
    pub pending: bool,
    pub copied: bool,
    /// App-clock time the dialog closes itself after a copy.
    pub close_at: Option<f64>,
}

/// The rollback dialog of `ImportRunsPanel`.
pub struct Rollback {
    pub batch: Value,
    pub plan: Option<Value>,
    pub plan_pending: bool,
    pub pending: bool,
    pub error: Option<String>,
}

/// The certificate trust dialog beside a sync error.
pub static CERT_HOST: crate::settings::certtrust::Host = crate::settings::certtrust::Host {
    key: "wd-cert",
    get: |a| &a.wallets.detail.cert,
    get_mut: |a| &mut a.wallets.detail.cert,
    fetch_tag: "wallets.detail.cert_fetch",
    trust_tag: "wallets.detail.cert_trust",
    trigger: "wd-cert-check",
    on_trusted: |a, _| {
        a.wallets.detail.cert_problem = None;
        a.wallets.detail.sync_error = None;
        // `ui.backends.certificate.trust` invalidates the backend lists and status.
        crate::settings::refresh_backends(a);
        a.send(
            "ui.backends.options",
            Value::Null,
            "wallets.detail.backends",
        );
    },
};

#[derive(Default)]
pub struct State {
    /// The wallet the reads below belong to; another id resets the page (the web keys the
    /// view by connection).
    pub wallet: String,
    pub tab: Tab,
    pub txs: Read<Vec<Value>>,
    pub dash: Read<Value>,
    pub utxos: Read<Value>,
    pub history: Read<Value>,
    pub imports: Vec<Value>,
    pub wallets: Read<Vec<Value>>,
    pub backends: Vec<Value>,
    pub encrypted: bool,
    pub tx_visible: usize,
    pub utxo_visible: usize,
    pub utxo_sort: Sort,
    pub range: Range,
    pub hist_hover: Option<usize>,
    pub menu: bool,
    pub select: Option<Select>,
    pub edit: Option<EditForm>,
    pub delete: Option<DeleteForm>,
    pub reveal: Option<RevealForm>,
    pub rollback: Option<Rollback>,
    /// `ui.wallets.sync` in flight (a user's Refresh / Full rescan).
    pub syncing: bool,
    /// The pre-rescan Bitcoin Core coverage probe in flight.
    pub core_testing: bool,
    /// The wallet that probe is for: its reply continues a rescan only for that wallet.
    pub core_test_wallet: Option<String>,
    pub sync_error: Option<String>,
    /// An untrusted or changed backend certificate behind the last sync error: offers the
    /// explicit trust dialog for the wallet's backend.
    pub cert_problem: Option<crate::settings::certtrust::Problem>,
    pub cert: crate::settings::certtrust::Trust,
    /// A UTXO's transaction being resolved before it opens in the detail panel.
    pub pending_utxo: Option<String>,
    /// The copy button showing its check, and until when.
    pub copied: Option<(String, f64)>,
}

impl State {
    pub fn is_copied(&self, key: &str, clock: f64) -> bool {
        self.copied
            .as_ref()
            .is_some_and(|(k, until)| k == key && clock < *until)
    }
}

// ---- Shared derived data --------------------------------------------------------------------

/// The `ui.wallets.list` row of this connection (`walletDetail`): by id, else by label.
pub fn wallet_detail<'a>(app: &'a App, c: &Conn) -> Option<&'a Value> {
    app.wallets.detail.wallets.ready()?.iter().find(|w| {
        w.get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| !id.is_empty() && id == c.id)
            || w.get("label").and_then(Value::as_str) == Some(c.label.as_str())
    })
}

pub fn s<'a>(v: &'a Value, k: &str) -> Option<&'a str> {
    v.get(k).and_then(Value::as_str).filter(|x| !x.is_empty())
}

pub fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(|x| {
        x.as_f64()
            .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
    })
}

/// The wallet's chain (`walletDetail.chain || connection.chain || "bitcoin"`), lower case.
pub fn wallet_chain(app: &App, c: &Conn) -> String {
    wallet_detail(app, c)
        .and_then(|w| s(w, "chain"))
        .map(str::to_owned)
        .or_else(|| c.chain.clone().filter(|x| !x.is_empty()))
        .unwrap_or_else(|| "bitcoin".into())
        .to_lowercase()
}

/// `walletDetail.sync_mode || connection.syncMode`.
pub fn sync_mode(app: &App, c: &Conn) -> String {
    wallet_detail(app, c)
        .and_then(|w| s(w, "sync_mode"))
        .map(str::to_owned)
        .or_else(|| c.sync_mode.clone())
        .unwrap_or_default()
}

/// `isExchangeKind(walletDetail?.kind)`: the exchange kind of an exchange API wallet.
pub fn exchange_kind(app: &App, c: &Conn) -> Option<String> {
    wallet_detail(app, c)
        .and_then(|w| s(w, "kind"))
        .filter(|k| crate::wallets::add::exchange::is_exchange_kind(k))
        .map(str::to_owned)
}

/// `walletSyncNeedsBackend` (once the wallets list answered; never for exchange wallets).
pub fn backend_required(app: &App, c: &Conn) -> bool {
    if exchange_kind(app, c).is_some() || app.wallets.detail.wallets.ready().is_none() {
        return false;
    }
    let backend = wallet_detail(app, c)
        .and_then(|w| w.get("backend"))
        .and_then(|b| s(b, "name"));
    sync_mode(app, c).starts_with("backend_") && backend.is_none()
}

/// `isWalletSyncRunning`: this page's sync, a book refresh touching it, or an exchange sync.
pub fn sync_running(app: &App) -> bool {
    app.wallets.detail.syncing
        || app.busy.syncing
        || crate::wallets::add::exchange::sync_pending(app)
}

/// `encryptedWorkspace`: `status.database_encrypted`.
pub fn encrypted(app: &App) -> bool {
    app.wallets.detail.encrypted
}

/// The fiat currency and rate the page values balances at (`activeMarketFiatCurrency/Rate`).
pub fn fiat(app: &App) -> (String, f64) {
    match app.snapshot() {
        Some(s) => (s.fiat_currency(), s.rate()),
        None => ("EUR".into(), 0.0),
    }
}

/// `eurRateMissing`.
pub fn rate_missing(app: &App) -> bool {
    app.snapshot()
        .map(|s| {
            let book = s
                .market_rate
                .as_ref()
                .and_then(|m| m.fiat_currency.clone())
                .unwrap_or_else(|| "EUR".into())
                .to_uppercase();
            s.price_eur <= 0.0
                || s.price_eur.is_nan()
                || (book == "EUR" && s.fiat.completeness.market_rate_missing)
        })
        .unwrap_or(true)
}

/// `formatBtc`: `₿ X.XXXXXXXX`, optionally signed with `+ ` / `− `.
pub fn btc(v: f64, precision: usize, sign: bool) -> String {
    let abs = format!("{:.*}", precision, v.abs());
    let prefix = if sign {
        if v >= 0.0 { "+ " } else { "− " }
    } else {
        ""
    };
    format!("{prefix}₿ {abs}")
}

/// `formatCount` in the UI locale.
pub fn count(n: f64) -> String {
    crate::format::count(n.max(0.0).round() as usize)
}

impl State {
    /// Leaving the wallet's page: `ConnectionDetail`'s `useState`s (tab, paging, sort, range,
    /// menus, the edit / delete / reveal / rollback dialogs, the last sync error and its
    /// certificate offer) start over. The reads stay (React Query shows them on the next visit
    /// while refetching), as do the flags of a sync or probe still in flight.
    pub fn leave(&mut self) {
        let k = std::mem::take(self);
        *self = State {
            wallet: k.wallet,
            txs: k.txs,
            dash: k.dash,
            utxos: k.utxos,
            history: k.history,
            imports: k.imports,
            wallets: k.wallets,
            backends: k.backends,
            encrypted: k.encrypted,
            syncing: k.syncing,
            core_testing: k.core_testing,
            core_test_wallet: k.core_test_wallet,
            tx_visible: txs::PAGE,
            utxo_visible: coins::PAGE,
            ..State::default()
        };
    }
}

// ---- Reads -----------------------------------------------------------------------------------

fn send_reads(app: &mut App, id: &str) {
    app.send("ui.wallets.list", Value::Null, "wallets.detail.wallets");
    app.send(
        "ui.transactions.list",
        json!({"wallet": id, "limit": RECENT_LIMIT}),
        "wallets.detail.txs",
    );
    app.send(
        "ui.transactions.dashboard",
        json!({"wallet": id, "period": "all", "timezone": "UTC"}),
        "wallets.detail.dash",
    );
    app.send(
        "ui.reports.balance_history",
        json!({"wallet": id, "interval": "month", "limit": HISTORY_LIMIT}),
        "wallets.detail.history",
    );
    app.send(
        "ui.wallets.utxos",
        json!({"wallet": id}),
        "wallets.detail.utxos",
    );
    app.send(
        "ui.imports.list",
        json!({"wallet": id}),
        "wallets.detail.imports",
    );
}

/// Starts every read the page makes on mount for `c` (all local).
fn load(app: &mut App, c: &Conn) {
    let id = c.id.clone();
    app.wallets.detail = State {
        wallet: id.clone(),
        tx_visible: txs::PAGE,
        utxo_visible: coins::PAGE,
        ..State::default()
    };
    app.send("status", Value::Null, "wallets.detail.status");
    app.send(
        "ui.backends.options",
        Value::Null,
        "wallets.detail.backends",
    );
    send_reads(app, &id);
}

/// Re-reads after a mutation (the web invalidates every daemon query), keeping the UI state.
pub fn reload(app: &mut App) {
    let id = app.wallets.detail.wallet.clone();
    if id.is_empty() {
        return;
    }
    send_reads(app, &id);
    app.refresh_reads();
}

/// The connection the current route shows, when it is a wallet (not a node).
fn current(app: &App) -> Option<Conn> {
    let id = super::route_id(app)?;
    super::conn::find(app, &id).filter(|c| !c.is_node())
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let Some(c) = current(app) else { return false };
    actions::take_pending_copy(app, tx);
    let mut changed = false;
    if app.wallets.detail.wallet != c.id {
        load(app, &c);
        changed = true;
    }
    if let Some((_, until)) = app.wallets.detail.copied
        && app.clock >= until
    {
        app.wallets.detail.copied = None;
        changed = true;
    }
    if let Some(at) = app.wallets.detail.reveal.as_ref().and_then(|r| r.close_at) {
        if app.clock >= at {
            app.wallets.detail.reveal = None;
            changed = true;
        } else {
            tx.wake_after(std::time::Duration::from_secs_f64(
                (at - app.clock).max(0.02),
            ));
        }
    }
    if exchange_kind(app, &c).is_some() {
        changed |= crate::wallets::add::exchange::sync_poll(app, tx, &c);
    }
    changed
}

fn read_of<T>(r: &Reply, ok: impl FnOnce(&Value) -> T) -> Read<T> {
    match &r.error {
        Some(e) => Read::Failed(e.clone()),
        None => Read::Ready(ok(&r.data)),
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    if !tag.starts_with("wallets.detail.") {
        return;
    }
    let d = &mut app.wallets.detail;
    match tag {
        "wallets.detail.status" => {
            d.encrypted = r
                .data
                .get("database_encrypted")
                .and_then(Value::as_bool)
                .unwrap_or(false)
        }
        "wallets.detail.backends" => {
            d.backends = r
                .data
                .get("backends")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        }
        "wallets.detail.wallets" => {
            d.wallets = read_of(r, |v| {
                v.get("wallets")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
        }
        "wallets.detail.txs" => {
            d.txs = read_of(r, |v| {
                v.get("txs")
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            })
        }
        "wallets.detail.dash" => d.dash = read_of(r, Value::clone),
        "wallets.detail.history" => d.history = read_of(r, Value::clone),
        "wallets.detail.utxos" => d.utxos = read_of(r, Value::clone),
        "wallets.detail.imports" if r.error.is_none() => {
            d.imports = r
                .data
                .get("batches")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
        }
        _ => actions::on_reply(app, tag, r),
    }
}

pub fn on_key(app: &mut App, ev: &KeyEvent, _tx: &mut Tx) -> bool {
    if current(app).is_none() {
        return false;
    }
    if matches!(ev.key, Key::Escape) && app.wallets.detail.cert.open {
        // The trust dialog is the innermost overlay (it stays while a request runs).
        crate::settings::certtrust::close(app, &CERT_HOST);
        return true;
    }
    let d = &mut app.wallets.detail;
    if matches!(ev.key, Key::Escape) {
        if d.select.is_some() {
            d.select = None;
        } else if d.menu {
            d.menu = false;
        } else if d.rollback.is_some() {
            d.rollback = None;
        } else if d.edit.is_some() {
            d.edit = None;
        } else if d.delete.is_some() {
            d.delete = None;
        } else if d.reveal.is_some() {
            d.reveal = None;
        } else {
            return false;
        }
        return true;
    }
    false
}

/// Headless: block until the page's reads answered.
pub fn prime(app: &mut App, c: &Conn) {
    load(app, c);
    super::common::settle(app);
}

// ---- View ------------------------------------------------------------------------------------

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, c: &Conn) -> El {
    // The page lays out by its own width (`@container/wallet`).
    let width = cx
        .layout_size("wallet-detail")
        .map(|[w, _]| w - 2.0 * tk.page_pad())
        .unwrap_or(1160.0);
    let mut page = col()
        .key("wallet-detail")
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .fill(tk.background);
    page = page
        .child(hero::header(app, cx, tk, layer, c))
        .child(hero::hero(app, tk, c, width));
    if exchange_kind(app, c).is_some() {
        page = page.child(crate::wallets::add::exchange::sync_card(
            app,
            cx,
            tk,
            c,
            sync_running(app) && !crate::wallets::add::exchange::sync_pending(app),
        ));
    }
    for n in notices::all(app, tk, c) {
        page = page.child(n);
    }
    page = page.child(txs::tabs(app, tk, c));
    page = match app.wallets.detail.tab {
        Tab::Transactions => page.child(txs::panel(app, tk, c, width)),
        Tab::Coins => page.children(coins::panel(app, tk, width)),
        Tab::Details => page.children(info::panel(app, tk, c, width)),
    };
    dialogs::mount(app, tk, layer, c);
    page
}

/// `ScreenSkeleton titleWidth="w-44"` while the snapshot loads.
pub fn skeleton(tk: &Tk) -> El {
    let bar = |w: f32, h: f32| ui::skeleton(tk, Some(w), h);
    let mut metrics = row().w_full();
    for i in 0..4 {
        let mut cell = col()
            .grow(1.0)
            .padding(tk.r(1.0))
            .gap(tk.r(0.5))
            .child(bar(tk.r(6.0), tk.r(0.75)))
            .child(bar(tk.r(5.0), tk.r(1.5)))
            .child(bar(tk.r(8.0), tk.r(0.75)));
        if i < 3 {
            cell = cell
                .border_sides([0.0, 1.0, 0.0, 0.0], tk.border)
                .border_layout();
        }
        metrics = metrics.child(cell);
    }
    let mut list = col().w_full();
    for _ in 0..8 {
        list = list.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.75))
                .gap(tk.r(0.75))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .border_layout()
                .child(
                    col()
                        .grow(1.0)
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(11.0), tk.r(1.0)))
                        .child(bar(tk.r(16.0), tk.r(0.75))),
                )
                .child(
                    col()
                        .w(96.0)
                        .items_end()
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(5.0), tk.r(1.0)))
                        .child(bar(tk.r(3.5), tk.r(0.75))),
                ),
        );
    }
    col()
        .w_full()
        .padding(tk.page_pad())
        .gap(tk.section_gap())
        .busy(true)
        .child(
            row()
                .w_full()
                .justify_between()
                .center_y()
                .child(
                    col()
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(7.0), tk.r(0.75)))
                        .child(bar(tk.r(11.0), tk.r(1.25))),
                )
                .child(
                    row()
                        .gap(tk.r(0.75))
                        .child(bar(tk.r(6.0), tk.r(2.0)))
                        .child(bar(tk.r(7.0), tk.r(2.0))),
                ),
        )
        .child(ui::card(tk).child(metrics))
        .child(
            ui::card(tk)
                .child(
                    col()
                        .w_full()
                        .px(tk.r(1.0))
                        .py(tk.r(0.75))
                        .gap(tk.r(0.5))
                        .child(bar(tk.r(9.0), tk.r(1.0)))
                        .child(bar(tk.r(14.0), tk.r(0.75))),
                )
                .child(list),
        )
}

/// The "not found" state: `flex flex-1 flex-col items-center justify-center gap-4 p-12
/// text-muted-foreground`, the message and a Back to wallets outline button.
pub fn not_found(app: &App, tk: &Tk, id: &str) -> El {
    // `flex flex-1 … justify-center`: the page's scroll area gives it no height, so it sits at
    // the top, centred across.
    let _ = app;
    col()
        .w_full()
        .padding(tk.r(3.0))
        .gap(tk.r(1.0))
        .items_center()
        .child(
            text(tr("connections:detail.notFound", &[("id", id)]))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        )
        .child(
            crate::txdetail::widgets::button(
                tk,
                "wd-notfound-back",
                crate::txdetail::widgets::Variant::Outline,
                crate::txdetail::widgets::Size::Sm,
                Some(&I::ARROW_LEFT),
                &t("connections:detail.backToWallets"),
                true,
            )
            .role("link")
            .on_press(|a: &mut App| a.navigate("/connections")),
        )
}

// ---- Scenes ----------------------------------------------------------------------------------

fn open_by_label(a: &mut App, label: &str) {
    if let Some(id) = super::scene_id_by_label(a, label) {
        super::scene_route(a, &format!("/connections/{id}"));
    }
}

const SPENDING: &str = "Regtest Spending Wallet";
const LIQUID: &str = "Regtest Liquid Treasury 2024 Rotation";
const LIQUID_DEPRECATED: &str = "Regtest Liquid Treasury";
const SILENT: &str = "Regtest Silent Payments Wallet";

fn with_tab(a: &mut App, label: &str, tab: Tab) {
    open_by_label(a, label);
    a.wallets.detail.tab = tab;
}

/// A sync that failed on the wallet's backend certificate (synthetic: the daemon's fixed text,
/// nothing was contacted).
fn cert_failure(a: &mut App, problem: crate::settings::certtrust::Problem) {
    open_by_label(a, SPENDING);
    let line = match problem {
        crate::settings::certtrust::Problem::Changed => {
            "Regtest Spending Wallet refresh failed: This server's certificate changed: it no longer matches the certificate trusted for this backend"
        }
        crate::settings::certtrust::Problem::Untrusted => {
            "Regtest Spending Wallet refresh failed: The server's TLS certificate is not signed by an authority Kassiber trusts"
        }
    };
    a.wallets.detail.sync_error = Some(line.into());
    a.wallets.detail.cert_problem = Some(problem);
    // The demo book's `ui.wallets.list` reports this wallet's backend kind as "" (so the web
    // offers nothing there); the scene names a pinnable kind, as a TLS Electrum server would.
    if let Read::Ready(rows) = &mut a.wallets.detail.wallets {
        for w in rows
            .iter_mut()
            .filter(|w| w.get("label").and_then(Value::as_str) == Some(SPENDING))
        {
            if let Some(b) = w.get_mut("backend").and_then(Value::as_object_mut) {
                b.insert("kind".into(), json!("electrum"));
            }
        }
    }
}

pub const SCENES: &[crate::app::Scene] = &[
    // The real flow's starting point: a watch-only address wallet on `local-tls` (a local
    // self-signed TLS server); nothing is contacted until a script presses Refresh.
    ("certdelta-real-wallet", |a| {
        open_by_label(a, "Local TLS Watch")
    }),
    ("certdelta-wallet-offer", |a| {
        cert_failure(a, crate::settings::certtrust::Problem::Untrusted)
    }),
    ("certdelta-wallet-offer-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        cert_failure(a, crate::settings::certtrust::Problem::Untrusted)
    }),
    ("certdelta-wallet-offer-changed-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        cert_failure(a, crate::settings::certtrust::Problem::Changed)
    }),
    ("certdelta-wallet-offer-dialog", |a| {
        cert_failure(a, crate::settings::certtrust::Problem::Changed);
        let row = a.wallets.detail.wallets.ready().and_then(|r| {
            r.iter()
                .find(|w| w.get("label").and_then(Value::as_str) == Some(SPENDING))
                .cloned()
        });
        let name = row
            .as_ref()
            .and_then(|w| w.get("backend"))
            .and_then(|b| s(b, "name"))
            .unwrap_or("")
            .to_owned();
        crate::settings::certtrust::open(
            a,
            &CERT_HOST,
            &name,
            &name,
            Some(crate::settings::certtrust::Problem::Changed),
        );
    }),
    ("wallets-detail-spending", |a| open_by_label(a, SPENDING)),
    ("wallets-detail-spending-coins", |a| {
        with_tab(a, SPENDING, Tab::Coins)
    }),
    ("wallets-detail-spending-details", |a| {
        with_tab(a, SPENDING, Tab::Details)
    }),
    ("wallets-detail-liquid", |a| open_by_label(a, LIQUID)),
    ("wallets-detail-liquid-coins", |a| {
        with_tab(a, LIQUID, Tab::Coins)
    }),
    ("wallets-detail-liquid-details", |a| {
        with_tab(a, LIQUID, Tab::Details)
    }),
    ("wallets-detail-liquid-deprecated", |a| {
        open_by_label(a, LIQUID_DEPRECATED)
    }),
    ("wallets-detail-silent", |a| open_by_label(a, SILENT)),
    ("wallets-detail-menu", |a| {
        open_by_label(a, SPENDING);
        a.wallets.detail.menu = true;
    }),
    ("wallets-detail-liquid-menu", |a| {
        open_by_label(a, LIQUID);
        a.wallets.detail.menu = true;
    }),
    ("wallets-detail-edit", |a| {
        open_by_label(a, SPENDING);
        actions::open_edit(a);
    }),
    ("wallets-detail-liquid-edit", |a| {
        open_by_label(a, LIQUID);
        actions::open_edit(a);
    }),
    ("wallets-detail-edit-backend", |a| {
        open_by_label(a, SPENDING);
        actions::open_edit(a);
        a.wallets.detail.select = Some(Select::Backend);
    }),
    ("wallets-detail-remove", |a| {
        open_by_label(a, SPENDING);
        actions::open_delete(a);
    }),
    ("wallets-detail-reveal", |a| {
        open_by_label(a, LIQUID);
        actions::open_reveal(a);
    }),
    ("wallets-detail-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_by_label(a, SPENDING);
    }),
    ("wallets-detail-light-coins", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        with_tab(a, SPENDING, Tab::Coins);
    }),
    ("wallets-detail-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_by_label(a, SPENDING);
    }),
    ("wallets-detail-de-coins", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        with_tab(a, SPENDING, Tab::Coins);
    }),
    ("wallets-detail-hidden", |a| {
        a.prefs.privacy = crate::app::Privacy::HideAmounts;
        with_tab(a, SPENDING, Tab::Coins);
    }),
    ("wallets-detail-history-hover", |a| {
        open_by_label(a, SPENDING);
        a.wallets.detail.hist_hover = Some(40);
    }),
    ("wallets-detail-utxo-sorted", |a| {
        with_tab(a, SPENDING, Tab::Coins);
        a.wallets.detail.utxo_sort = Sort::SizeDesc;
    }),
    ("wallets-detail-notfound", |a| {
        super::scene_route(a, "/connections/does-not-exist")
    }),
];

/// URL-encodes a query value (the transactions route's codec).
pub fn notices_enc(s: &str) -> String {
    s.bytes()
        .map(|c| match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (c as char).to_string()
            }
            _ => format!("%{c:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod transient_tests {
    use super::*;

    #[test]
    fn leaving_the_wallet_page_keeps_reads_and_drops_page_state() {
        let mut s = State {
            wallet: "w1".into(),
            txs: Read::Ready(vec![Value::Null]),
            tab: Tab::Details,
            tx_visible: 99,
            menu: true,
            syncing: true,
            ..State::default()
        };
        s.sync_error = Some("backend unreachable".into());
        s.delete = Some(DeleteForm::default());
        s.leave();
        assert_eq!(s.wallet, "w1");
        assert!(matches!(s.txs, Read::Ready(ref v) if v.len() == 1));
        assert!(s.syncing, "a sync in flight keeps its flag");
        assert_eq!(s.tab, Tab::default());
        assert_eq!(s.tx_visible, txs::PAGE);
        assert!(!s.menu && s.sync_error.is_none() && s.delete.is_none());
    }
}
