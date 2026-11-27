//! App state shared by the shell and the screens: daemon plumbing, cached reads, preferences,
//! navigation and transient UI (toasts, busy flags).
//!
//! Requests are tagged with a `&'static str` naming who asked (`"app.overview"`, `"shell.books"`,
//! `"chart.resolve"` …). A reply is stored under its kind in `reads` and handed to every
//! module's `on_reply`; a module only acts on its own tags.
//!
//! The plumbing follows the Tauri shell's `daemon_invoke` and the web's daemon client:
//! - every kind passes the allowlist and the AI kill switch first (`crate::kinds`); a refusal,
//!   a failed spawn or a daemon that went away answers the request with an error reply, so no
//!   caller waits forever;
//! - `auth_required` is terminal: the database's own challenge locks the app (the lock screen asks
//!   for the passphrase and retries with `daemon.unlock` or `ui.projects.select`, as AppShell does),
//!   other challenges reach the caller as an error reply;
//! - a lock sends `daemon.lock`, drops every cached read and screen state, and holds back every
//!   request but the unlock's until the unlock; nothing of the book stays in memory behind it;
//! - `refresh_reads` runs a book refresh cycle (daemon/bookRefresh.ts): the book's revision before
//!   and after the re-reads, repeated when a write landed in between, and every re-read answer
//!   held back until the last one arrived, so all screens swap to the new state in one frame.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::daemon::{
    DaemonError, Incoming, Launch, Route as RecordRoute, Supervisor, Wake, route_record,
};
use crate::i18n::t;
use crate::kinds;
use crate::model::{Badges, Snapshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeMode {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Currency {
    Btc,
    Fiat,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privacy {
    Normal,
    HideAmounts,
    HideAll,
}

#[derive(Clone, Debug)]
pub struct Prefs {
    pub theme: ThemeMode,
    pub currency: Currency,
    pub privacy: Privacy,
    pub hide_on_unlock: bool,
    pub nav_collapsed: bool,
    pub developer_tools: bool,
    pub ai_features: bool,
    pub alpha_visible: bool,
    /// The web app's `--app-ui-scale` (0.8–1.2). The default is [`DEFAULT_SCALE`] (100 %), the
    /// owner's choice over the web's 0.9.
    pub scale: f32,
    pub right_panel_open: bool,
    pub lang: crate::i18n::Lang,
}

/// The UI scale a fresh install starts at and ⌘0 returns to: 100 % (owner feedback; the web's
/// `--app-ui-scale: 0.9` default is overridden on purpose).
pub const DEFAULT_SCALE: f32 = 1.0;

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: ThemeMode::Dark,
            currency: Currency::Btc,
            privacy: Privacy::Normal,
            hide_on_unlock: false,
            nav_collapsed: false,
            developer_tools: false,
            ai_features: true,
            alpha_visible: true,
            scale: DEFAULT_SCALE,
            right_panel_open: false,
            lang: crate::i18n::Lang::En,
        }
    }
}

impl Prefs {
    pub fn hide_sensitive(&self) -> bool {
        self.privacy != Privacy::Normal
    }
    pub fn hide_shares(&self) -> bool {
        self.privacy == Privacy::HideAll
    }
}

/// A route path such as `/overview` or `/transactions?tx=…`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route(pub String);

impl Route {
    pub fn path(&self) -> &str {
        self.0.split(['?', '#']).next().unwrap_or("/")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    Success,
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Toast {
    pub id: u64,
    pub key: Option<&'static str>,
    pub title: String,
    pub body: Option<String>,
    pub tone: ToastTone,
    pub repeat: u32,
    /// App-clock time it disappears.
    pub until: f64,
    /// Up to two text actions (`toastopen`: Open / Show in Finder for a saved file).
    pub actions: Vec<crate::toastopen::ToastAction>,
    /// The pointer is over it: `toastopen::poll` holds it.
    pub hovered: bool,
}

/// A caller's claim on the answer to one request ([`App::send_ticket`]).
///
/// The answer reaches the modules only while the caller still holds the ticket (or a clone).
#[derive(Clone, Debug)]
pub struct Ticket(Arc<()>);

impl Ticket {
    /// Whether the answer `on_reply` is handling now is this ticket's.
    pub fn answers(&self, app: &App) -> bool {
        app.delivering_ticket
            .as_ref()
            .is_some_and(|ticket| std::ptr::eq(ticket.as_ptr(), Arc::as_ptr(&self.0)))
    }
}

impl PartialEq for Ticket {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

/// A reply as the UI keeps it.
#[derive(Clone, Debug)]
pub struct Reply {
    pub kind: String,
    pub data: Value,
    pub error: Option<String>,
    /// The whole error object (`code`, `message`, `details`) when the request failed.
    pub error_detail: Value,
    pub elapsed: Duration,
}

/// Data loading state of one read.
#[derive(Clone, Debug, Default)]
pub enum Load<T> {
    #[default]
    Idle,
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn ready(&self) -> Option<&T> {
        match self {
            Load::Ready(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Busy {
    /// `ui.freshness.run` in flight (stream).
    pub syncing: bool,
    /// `ui.journals.process` in flight (stream).
    pub processing: bool,
    /// `ui.rates.latest` in flight.
    pub rate_refresh: bool,
    /// Foreground reads in flight (route progress line).
    pub reads: usize,
    /// The latest progress event of a running job, for the sync card / progress line.
    pub progress: Option<String>,
}

/// A named state for `--scene` (headless snapshots and the parity harness): the name and what
/// sets the state up. Each area lists its own in a `SCENES` table.
pub type Scene = (&'static str, fn(&mut App));

/// A shared press handler (a click target that more than one element may call).
pub type Handler = std::rc::Rc<dyn Fn(&mut App)>;

pub struct App {
    /// How the daemon runs now (binary, data root); a copy of `daemon.launch`.
    pub launch: Launch,
    /// The `kassiber daemon` supervisor (`daemon.is_some()`: requests can be served at all).
    pub daemon: Supervisor,
    pub daemon_error: Option<String>,
    pub daemon_ready: Option<Value>,
    /// request_id → (tag, kind, sent at).
    pub pending: HashMap<String, (&'static str, String, Instant)>,
    /// Latest successful reply per kind (`ui.review.badges`, `ui.profiles.snapshot`, …).
    pub reads: HashMap<String, Reply>,
    pub overview: Load<Snapshot>,
    pub badges: Option<Badges>,
    pub prefs: Prefs,
    pub route: Route,
    pub back: Vec<Route>,
    pub forward: Vec<Route>,
    pub busy: Busy,
    pub toasts: Vec<Toast>,
    pub next_toast: u64,
    /// App clock (seconds) as of the last poll.
    pub clock: f64,
    /// When the artwork variant on screen next changes (`crate::art::poll`).
    pub art_due: f64,
    /// Whether `cx.system_dark()` said dark at the last view (for ThemeMode::System).
    pub system_dark: bool,
    pub locked: bool,
    /// Request timings for the comparison write-up: (kind, elapsed).
    pub timings: VecDeque<(String, Duration)>,
    pub shell: crate::shell::ShellUi,
    pub ov: crate::overview::OverviewUi,
    pub chart: crate::chart::ChartUi,
    pub txs: crate::transactions::TxUi,
    pub detail: crate::txdetail::DetailUi,
    pub graph: crate::txdetail::graph::GraphUi,
    pub settings: crate::settings::Ui,
    pub wallets: crate::wallets::Ui,
    pub review: crate::review::Ui,
    pub reports: crate::reports::Ui,
    pub assistant: crate::assistant::Ui,
    pub custody: crate::custody::Ui,
    pub provenance: crate::provenance::Ui,
    /// Native shell features (`crate::native`): menu state, waiting links, update checks.
    pub native: crate::native::Ui,
    /// Touch ID and the OS credential store (`crate::secrets`).
    pub secrets: crate::secrets::Ui,
    /// Welcome and setup, and the identity that gates them (`crate::onboarding`).
    pub onboarding: crate::onboarding::Ui,
    /// Mouse Back/Forward buttons and the trackpad history swipe (`crate::nav`).
    pub nav: crate::nav::NavUi,
    /// The bundled demo book while it is open (`crate::demo`).
    pub demo: crate::demo::Demo,
    /// Preferences kept between launches (`crate::persist`).
    pub persist: crate::persist::Store,
    // ---- Request plumbing and the lock (this file) ----
    /// The lock screen's state (passphrase field, attempt, error, a project waiting to open).
    pub lock: crate::shell::overlays::LockUi,
    /// The open book is SQLCipher-encrypted (`status.database_encrypted`, or its challenge).
    pub encrypted: bool,
    /// The daemon asked for the passphrase again (`daemonAuthRequired`).
    pub daemon_auth_required: bool,
    /// Bumped by every lock, unlock and book switch (`daemonSession`): a challenge answering an
    /// older request no longer locks.
    pub session: u64,
    /// Bumped each time a book refresh cycle re-reads; screens that keep their own reads may
    /// watch it and re-read too (their answers are held with the cycle's).
    pub book_generation: u64,
    /// The running book refresh cycle.
    pub cycle: BookRefresh,
    /// What the last cycle changed, for 30 s (`lastChange`).
    pub last_change: Option<BookChange>,
    meta: HashMap<String, Meta>,
    #[cfg(test)]
    pub(crate) sent_args: HashMap<String, Value>,
    /// Requests whose answers are dropped (made before a lock, an unlock or a book switch):
    /// request_id → (kind, process generation).
    orphaned: HashMap<String, (String, u64)>,
    /// Terminal answers that overtook an earlier request with the same tag (the daemon answers
    /// slow requests out of order): request_id → (process generation, envelope), delivered once
    /// every earlier same-tag request has its answer, so screens see their tag's answers in the
    /// order they asked.
    parked: HashMap<String, (u64, Value)>,
    next_seq: u64,
    delivering_ticket: Option<Weak<()>>,
    tickets: HashMap<String, Weak<()>>,
    /// While true, error toasts are not shown: the answer is a refusal the app handles itself
    /// (a request refused behind the lock, a write refused because the open book changed).
    quiet_errors: bool,
    /// A data-root switch warming its replacement (`switch_root`).
    root_switch: Option<RootSwitch>,
    /// Answers the app gives itself (refusals, spawn failures), handled on the next pump.
    local: VecDeque<(u64, Incoming)>,
    ever_unlocked: bool,
    reload_after_respawn: bool,
    pumps: u64,
    lifecycle_seen: u64,
}

/// Bookkeeping beside `pending`.
#[derive(Clone, Debug, Default)]
struct Meta {
    pgen: u64,
    session: u64,
    streaming: bool,
    /// Sent during a cycle's re-read: its answer waits for the cycle.
    held: bool,
    /// Send order (per-tag delivery order).
    seq: u64,
    /// A read's arguments, kept to read again when the daemon answers `stale_context`.
    reread: Option<Value>,
    /// `stale_context` re-reads so far.
    rereads: u8,
}

/// How often a read answered with `stale_context` is read again before its caller hears it.
const MAX_REREADS: u8 = 3;

/// What runs when a data-root switch has its outcome.
pub type SwitchDone = Box<dyn FnOnce(&mut App, Result<(), String>)>;

/// A data-root switch in progress: its continuation, and the requests made while the
/// replacement warms (they go to whichever daemon serves once it is decided).
struct RootSwitch {
    done: Option<SwitchDone>,
    deferred: Vec<(String, String, Value)>,
}

/// Tags whose callers keep their state across a lock and a book switch: their requests are not
/// orphaned, so they must always hear an answer.
fn kept_tag(tag: &str) -> bool {
    matches!(tag, "app.lock" | "app.unlock")
        || tag.starts_with("settings.fx")
        || (tag.starts_with("reports.diag.")
            && !tag.starts_with("reports.diag.act.")
            && !tag.starts_with("reports.diag.chg."))
}

/// Every book-bound write carries the open book as `expected_scope` (docs/reference/daemon.md):
/// the daemon refuses it with `stale_context` instead of acting on another book. A caller that
/// bound its own scope keeps it; nothing is added while the open book is unknown, or when a
/// book-management request names another book (`ui.profiles.*` with a `profile_id`: the Books
/// page managing an inactive book, which the open book's scope would make the daemon refuse).
pub fn scope_args(kind: &str, args: Value, scope: &Value) -> Value {
    let Value::Object(mut map) = args else {
        return args;
    };
    if map.contains_key("expected_scope") || !kinds::is_book_mutation(kind) {
        return Value::Object(map);
    }
    // Book management (`ui.profiles.*`) addresses its book by `profile_id`; other kinds act on the
    // open book whatever they carry, so they stay bound.
    if kind.starts_with("ui.profiles.")
        && let Some(target) = map.get("profile_id").and_then(Value::as_str)
        && scope["profile_id"].as_str() != Some(target)
    {
        return Value::Object(map);
    }
    let id = |k: &str| scope[k].as_str().is_some_and(|s| !s.trim().is_empty());
    if id("workspace_id") && id("profile_id") {
        map.insert("expected_scope".into(), scope.clone());
    }
    Value::Object(map)
}

/// A book refresh cycle's step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Idle,
    /// Reading the revision before the re-reads.
    StartRev,
    /// The re-reads are out.
    Refetch,
    /// Reading the revision after them.
    EndRev,
}

/// `bookRefresh.ts` state.
#[derive(Default)]
pub struct BookRefresh {
    pub phase: Phase,
    /// Mounted reads keep showing their committed data while true (at most `HOLD_LIMIT`).
    pub holding: bool,
    attempt: u8,
    start_rev: Option<String>,
    started: Option<Instant>,
    refetch_pump: u64,
    held: Vec<(&'static str, String, Reply)>,
    /// A refresh asked for while this one runs: one follow-up cycle.
    queued: bool,
    before: Option<BookView>,
}

/// `MAX_ATTEMPTS`, `HOLD_LIMIT_MS`, `CHANGE_SUMMARY_MS`.
pub const MAX_ATTEMPTS: u8 = 3;
pub const HOLD_LIMIT: Duration = Duration::from_secs(15);
pub const CHANGE_SUMMARY_SECS: f64 = 30.0;

/// What a refresh can compare: the overview's figures.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BookView {
    pub transaction_count: Option<f64>,
    pub balance_sat: Option<i64>,
}

/// `BookChange`: what the last cycle changed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BookChange {
    pub transaction_delta: i64,
    pub balance_delta_sat: Option<i64>,
    pub at: f64,
}

/// `describeChange`.
pub fn describe_change(before: BookView, after: BookView, at: f64) -> Option<BookChange> {
    let transaction_delta = match (before.transaction_count, after.transaction_count) {
        (Some(a), Some(b)) => (b - a) as i64,
        _ => 0,
    };
    let balance = match (before.balance_sat, after.balance_sat) {
        (Some(a), Some(b)) => Some(b - a),
        _ => None,
    };
    if transaction_delta == 0 && balance.unwrap_or(0) == 0 {
        return None;
    }
    Some(BookChange {
        transaction_delta,
        balance_delta_sat: balance.filter(|d| *d != 0),
        at,
    })
}

/// `parseDaemonFreshnessEvent` (lib/daemonFreshnessEvent.ts) plus the watch-inbox push: the
/// unsolicited events that start a book refresh cycle. Progress and idle-worker records do not.
pub fn is_book_refresh_event(env: &Value) -> bool {
    if env.get("event").and_then(Value::as_bool) != Some(true)
        || env.get("request_id").is_some()
        || !env["schema_version"].is_number()
    {
        return false;
    }
    match env["kind"].as_str() {
        Some("ui.chain_analysis.watches.changed") => true,
        Some("ui.freshness.background") => env["data"].is_object(),
        Some("ui.freshness.worker") => matches!(
            env["data"]["status"].as_str(),
            Some("error" | "unavailable")
        ),
        _ => false,
    }
}

/// Why the lock screen is up (`LockReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LockReason {
    Encrypted,
    DaemonAuth,
    Session,
}

/// The lock screen's words for a refused unlock (`unlockFailureKey` + `formatDaemonEnvelopeError`).
pub fn unlock_failure(r: &Reply) -> String {
    match r.error_detail["code"].as_str() {
        Some("local_auth_denied") => t("chrome:lock.passphraseFailed"),
        Some("auth_required") => t("chrome:lock.passphraseRequiredError"),
        _ => {
            let e = &r.error_detail;
            let mut parts: Vec<String> = [e["message"].as_str().or(e["code"].as_str())]
                .into_iter()
                .flatten()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if let Some(h) = e["hint"].as_str().filter(|h| !h.is_empty()) {
                parts.push(h.to_owned());
            }
            if parts.is_empty() {
                r.error
                    .clone()
                    .unwrap_or_else(|| t("chrome:lock.unlockFailed"))
            } else {
                parts.join("\n\n")
            }
        }
    }
}

impl App {
    pub fn new() -> App {
        let args = rune::extra_args();
        // Nothing starts yet: the first request spawns the daemon, once the launch decided which
        // book it opens (Welcome reads nothing, and an imported book opens on its own root), so
        // the default state root gets no skeleton before Welcome.
        let daemon = Supervisor::new(Launch::from_args(args));
        let daemon_error = None;
        let mut app = App {
            launch: daemon.launch.clone(),
            daemon,
            daemon_error,
            daemon_ready: None,
            pending: HashMap::new(),
            reads: HashMap::new(),
            overview: Load::Idle,
            badges: None,
            prefs: Prefs::default(),
            route: Route("/overview".into()),
            back: vec![],
            forward: vec![],
            busy: Busy::default(),
            toasts: vec![],
            next_toast: 0,
            clock: 0.0,
            art_due: 0.0,
            system_dark: true,
            locked: false,
            timings: VecDeque::new(),
            shell: Default::default(),
            ov: Default::default(),
            chart: Default::default(),
            txs: Default::default(),
            detail: Default::default(),
            graph: Default::default(),
            settings: Default::default(),
            wallets: Default::default(),
            review: Default::default(),
            reports: Default::default(),
            assistant: Default::default(),
            custody: Default::default(),
            provenance: Default::default(),
            native: Default::default(),
            secrets: Default::default(),
            onboarding: Default::default(),
            nav: Default::default(),
            demo: Default::default(),
            persist: Default::default(),
            lock: Default::default(),
            encrypted: false,
            daemon_auth_required: false,
            session: 0,
            book_generation: 0,
            cycle: BookRefresh::default(),
            last_change: None,
            meta: HashMap::new(),
            #[cfg(test)]
            sent_args: HashMap::new(),
            orphaned: HashMap::new(),
            parked: HashMap::new(),
            next_seq: 0,
            delivering_ticket: None,
            tickets: HashMap::new(),
            quiet_errors: false,
            root_switch: None,
            local: VecDeque::new(),
            ever_unlocked: false,
            reload_after_respawn: false,
            pumps: 0,
            lifecycle_seen: 0,
        };
        // What the last launch left (preferences, page); launch flags and scenes override it.
        crate::persist::restore(&mut app);
        match flag(args, "--lang").as_deref() {
            Some("de") => app.prefs.lang = crate::i18n::Lang::De,
            Some("en") => app.prefs.lang = crate::i18n::Lang::En,
            _ => {}
        }
        if let Some(theme) = flag(args, "--theme") {
            app.prefs.theme = if theme == "light" {
                ThemeMode::Light
            } else {
                ThemeMode::Dark
            };
        }
        // Welcome (no identity yet) reads nothing from the book; an imported book opens first.
        if crate::onboarding::boot(&mut app) {
            app.load_shell_reads();
        }
        // Headless runs (snapshots, scripts) render on a virtual clock: wait for the first reads
        // so frame 0 already shows the book (or its lock screen).
        if rune::tooling::headless_run().is_some() {
            app.settle(Duration::from_secs(20));
        }
        app
    }

    /// Pumps until a warming root switch is decided and every request has its answer, or
    /// `limit` passes (headless runs and scenes only: the UI thread never waits on the daemon).
    pub fn settle(&mut self, limit: Duration) {
        let deadline = Instant::now() + limit;
        loop {
            self.pump();
            if (self.root_switch.is_none() && self.pending.is_empty()) || Instant::now() >= deadline
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// A data-root switch is warming its replacement.
    pub fn switching(&self) -> bool {
        self.root_switch.is_some()
    }

    pub fn dark(&self) -> bool {
        match self.prefs.theme {
            ThemeMode::Dark => true,
            ThemeMode::Light => false,
            ThemeMode::System => self.system_dark,
        }
    }

    pub fn tk(&self) -> crate::theme::Tk {
        crate::theme::Tk::new(self.dark(), self.prefs.scale)
    }

    pub fn set_waker(&self, wake: Wake) {
        self.daemon.set_waker(wake);
    }

    /// Local reads the shell and the Overview make on mount. None of them touches the network.
    pub fn load_shell_reads(&mut self) {
        self.overview = Load::Loading;
        self.send("ui.overview.snapshot", Value::Null, "app.overview");
        self.send("ui.review.badges", Value::Null, "app.badges");
        self.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        self.send("ui.projects.list", Value::Null, "app.projects");
        self.send("ui.freshness.status", Value::Null, "app.freshness");
        self.send("ui.network.offline", Value::Null, "app.offline");
        self.send("ui.backends.settings.list", Value::Null, "app.backends");
        // Sources behind for the previous year (`sources.behind`; a local read).
        crate::wallets::behind::read_behind(self);
    }

    /// Re-reads everything the screen shows (after a mutation, a sync, a journal run or a
    /// background freshness event) as one book refresh cycle; a request while one runs merges
    /// into one follow-up cycle.
    pub fn refresh_reads(&mut self) {
        if self.locked {
            return;
        }
        if self.cycle.phase != Phase::Idle {
            self.cycle.queued = true;
            return;
        }
        self.cycle = BookRefresh {
            phase: Phase::StartRev,
            holding: true,
            attempt: 1,
            started: Some(Instant::now()),
            before: Some(self.book_view()),
            ..Default::default()
        };
        self.send("status", json!({"revision": true}), "app.revision");
    }

    /// The overview's transaction count and balance (`captureBookView`).
    pub fn book_view(&self) -> BookView {
        let s = self.snapshot();
        BookView {
            transaction_count: s
                .and_then(|s| s.status.as_ref())
                .and_then(|s| s.transaction_count),
            balance_sat: s
                .and_then(|s| s.balance_summary.as_ref())
                .map(|b| (b.total_btc * 1e8).round() as i64),
        }
    }

    /// A cycle's re-reads: the shell's, and the transaction list's (its answer lands with them).
    fn refetch_book_reads(&mut self) {
        self.cycle.phase = Phase::Refetch;
        self.cycle.refetch_pump = self.pumps;
        self.book_generation += 1;
        self.send("ui.overview.snapshot", Value::Null, "app.overview");
        self.send("ui.review.badges", Value::Null, "app.badges");
        crate::review::refresh_commercial(self);
        self.send("ui.freshness.status", Value::Null, "app.freshness");
        self.send("ui.profiles.snapshot", Value::Null, "app.profiles");
        crate::wallets::behind::read_behind(self);
        // Transactions re-reads its list and dashboard on its next poll (held with these); rows
        // the refresh brings in are highlighted.
        self.txs.list_key.clear();
        self.txs.dash_key.clear();
        if !self.txs.rows.is_empty() {
            self.txs.highlight_until = -1.0;
        }
    }

    fn on_revision(&mut self, r: &Reply) {
        let rev = if r.error.is_none() {
            r.data["revision"].as_str().map(str::to_owned)
        } else {
            None
        };
        match self.cycle.phase {
            Phase::StartRev => {
                self.cycle.start_rev = rev;
                self.refetch_book_reads();
            }
            Phase::EndRev => {
                let moved = matches!((&self.cycle.start_rev, &rev), (Some(a), Some(b)) if a != b);
                if moved && self.cycle.attempt < MAX_ATTEMPTS {
                    // A write landed during the re-reads: read again.
                    self.cycle.attempt += 1;
                    self.cycle.start_rev = rev;
                    self.refetch_book_reads();
                } else {
                    self.release_cycle();
                }
            }
            _ => {}
        }
    }

    /// Every pump: the cycle's next step, and the hold limit.
    fn cycle_step(&mut self) -> bool {
        if self.cycle.phase == Phase::Idle {
            return false;
        }
        if self.cycle.holding && self.cycle.started.is_some_and(|s| s.elapsed() > HOLD_LIMIT) {
            // Never hold the screens longer than the limit, even if reads hang.
            self.cycle.holding = false;
            self.deliver_held();
            return true;
        }
        if self.cycle.phase == Phase::Refetch
            && self.pumps > self.cycle.refetch_pump
            && !self.meta.values().any(|m| m.held)
        {
            self.cycle.phase = Phase::EndRev;
            self.send("status", json!({"revision": true}), "app.revision");
        }
        false
    }

    fn deliver_held(&mut self) {
        for (tag, id, reply) in std::mem::take(&mut self.cycle.held) {
            self.busy.reads = self.busy.reads.saturating_sub(1);
            self.deliver_as(tag, &id, reply);
        }
    }

    /// Releases the hold: every screen swaps at once, then the change summary.
    fn release_cycle(&mut self) {
        let before = self.cycle.before.take();
        let queued = self.cycle.queued;
        self.deliver_held();
        self.cycle = BookRefresh::default();
        // The list already re-read with the cycle; its count watcher must not load it again.
        if let Some(c) = self.book_view().transaction_count {
            self.txs.seen_tx_count = Some(c);
        }
        if let Some(b) = before
            && let Some(change) = describe_change(b, self.book_view(), self.clock)
        {
            self.last_change = Some(change);
        }
        if queued {
            self.refresh_reads();
        }
    }

    /// Sends a request; its reply reaches every module's `on_reply` under `tag`. Always Some:
    /// a request the app cannot make (not allowed, AI off, no daemon, refused behind the lock)
    /// gets an error reply, so no caller waits forever. Book-bound writes carry the open book
    /// ([`scope_args`]).
    pub fn send(&mut self, kind: &str, args: Value, tag: &'static str) -> Option<String> {
        let id = self.daemon.alloc_id();
        let args = scope_args(kind, args, &self.expected_scope());
        if let Err(e) = kinds::check(kind, self.prefs.ai_features) {
            self.register(&id, kind, tag, &args);
            self.local.push_back((
                0,
                Incoming::Record {
                    request_id: id.clone(),
                    envelope: e.envelope(&id),
                },
            ));
            return Some(id);
        }
        if self.locked && !kinds::ALLOWED_WHILE_LOCKED.contains(&kind) {
            // Nothing reads the book behind the lock; the unlock starts every screen over. The
            // caller still hears a refusal (shown nowhere: see `quiet_errors`).
            self.register(&id, kind, tag, &args);
            let e = DaemonError::new(LOCKED_CODE, "Kassiber is locked.").retryable();
            self.local.push_back((
                0,
                Incoming::Record {
                    request_id: id.clone(),
                    envelope: e.envelope(&id),
                },
            ));
            return Some(id);
        }
        self.register(&id, kind, tag, &args);
        match &mut self.root_switch {
            // The daemon that answers is decided when the switch is: wait for it.
            Some(sw) => sw.deferred.push((id.clone(), kind.to_owned(), args)),
            None => self.write(&id, kind, &args),
        }
        Some(id)
    }

    /// [`send`](Self::send), keeping the answer for as long as the returned ticket lives.
    pub fn send_ticket(&mut self, kind: &str, args: Value, tag: &'static str) -> Ticket {
        self.send_ticket_id(kind, args, tag).1
    }

    /// [`send_ticket`](Self::send_ticket), also naming the request: for a caller that cancels it
    /// or matches its progress records by id. Its answer still reaches only the ticket's holder.
    pub fn send_ticket_id(
        &mut self,
        kind: &str,
        args: Value,
        tag: &'static str,
    ) -> (String, Ticket) {
        let id = self
            .send(kind, args, tag)
            .expect("send answers every request");
        let ticket = Ticket(Arc::new(()));
        self.tickets.insert(id.clone(), Arc::downgrade(&ticket.0));
        (id, ticket)
    }

    /// Whether the answer `on_reply` is handling now was sent with a ticket ([`Ticket::answers`]
    /// tells whose). An answer without one names no caller.
    pub fn delivering_ticketed(&self) -> bool {
        self.delivering_ticket.is_some()
    }

    /// Hands a registered request to the daemon.
    fn write(&mut self, id: &str, kind: &str, args: &Value) {
        match self.daemon.request(id, kind, args) {
            Ok(pgen) => {
                if let Some(m) = self.meta.get_mut(id) {
                    m.pgen = pgen;
                }
            }
            Err(e) => {
                self.daemon_error = Some(e.message.clone());
                self.local.push_back((
                    0,
                    Incoming::Record {
                        request_id: id.to_owned(),
                        envelope: e.envelope(id),
                    },
                ));
            }
        }
    }

    fn register(&mut self, id: &str, kind: &str, tag: &'static str, args: &Value) {
        let held = self.cycle.phase == Phase::Refetch
            && self.cycle.holding
            && kinds::is_read(kind)
            && !kinds::is_streaming(kind)
            && tag != "app.revision";
        self.next_seq += 1;
        let reread = (kinds::is_read(kind) && !kinds::is_streaming(kind)).then(|| args.clone());
        self.pending
            .insert(id.to_owned(), (tag, kind.to_owned(), Instant::now()));
        #[cfg(test)]
        self.sent_args.insert(id.to_owned(), args.clone());
        self.meta.insert(
            id.to_owned(),
            Meta {
                pgen: 0,
                session: self.session,
                streaming: kinds::is_streaming(kind),
                held,
                seq: self.next_seq,
                reread,
                rereads: 0,
            },
        );
        self.busy.reads += 1;
    }

    /// Drains daemon output; true when anything changed. Each record is handled on its own: a
    /// panic in one screen's handler leaves the rest of the batch queued for the next pump (the
    /// panic itself still reaches crash recovery), so every request keeps getting its answer.
    pub fn pump(&mut self) -> bool {
        self.pumps += 1;
        let mut msgs: VecDeque<(u64, Incoming)> = std::mem::take(&mut self.local);
        msgs.extend(self.daemon.drain());
        let mut changed = !msgs.is_empty();
        while let Some((pgen, m)) = msgs.pop_front() {
            if let Err(panic) =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.handle(pgen, m)))
            {
                self.delivering_ticket = None;
                self.quiet_errors = false;
                // Ahead of anything queued since: the order the records came in is kept.
                for item in msgs.into_iter().rev() {
                    self.local.push_front(item);
                }
                std::panic::resume_unwind(panic);
            }
        }
        changed |= self.step_switch();
        if !self.parked.is_empty() {
            self.sweep_parked();
        }
        changed |= self.cycle_step();
        changed |= self.fold_lifecycle();
        if self
            .last_change
            .is_some_and(|c| self.clock - c.at > CHANGE_SUMMARY_SECS)
        {
            self.last_change = None;
            changed = true;
        }
        changed
    }

    fn handle(&mut self, pgen: u64, m: Incoming) {
        match m {
            Incoming::Ready(env) => {
                self.daemon_ready = Some(env["data"].clone());
                self.daemon_error = None;
                // A respawn after a crash: what failed with the old process reads again (the
                // web's queries retry a retryable daemon error).
                if std::mem::take(&mut self.reload_after_respawn) {
                    self.refresh_reads();
                }
            }
            Incoming::Exited(err) => {
                self.daemon_error = Some(err.message.clone());
                self.reload_after_respawn = true;
                self.fail_gen(pgen, &err);
            }
            Incoming::Event(env) => {
                if is_book_refresh_event(&env) {
                    self.refresh_reads();
                }
                crate::shell::on_event(self, &env);
            }
            Incoming::Record {
                request_id,
                envelope,
            } => self.on_record(pgen, request_id, envelope),
        }
    }

    /// Every request a process had pending fails with `err`; the jobs it ran stop showing. An
    /// answer it already gave (parked behind an earlier same-tag request) stands.
    fn fail_gen(&mut self, pgen: u64, err: &DaemonError) {
        let mut ids: Vec<(u64, String)> = self
            .meta
            .iter()
            .filter(|(id, m)| m.pgen == pgen && pgen != 0 && !self.parked.contains_key(*id))
            .map(|(id, m)| (m.seq, id.clone()))
            .collect();
        ids.sort();
        let ids: Vec<String> = ids.into_iter().map(|(_, id)| id).collect();
        for id in ids {
            let env = err.envelope(&id);
            self.on_record(0, id, env);
        }
        self.orphaned.retain(|_, (_, g)| *g != pgen);
        self.busy.syncing = false;
        self.busy.processing = false;
        self.busy.rate_refresh = false;
        self.busy.progress = None;
    }

    fn on_record(&mut self, pgen: u64, request_id: String, mut envelope: Value) {
        let from_daemon = pgen != 0;
        let rec_kind = envelope
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if let Some((kind, _)) = self.orphaned.get(&request_id) {
            if route_record(kind, kinds::is_streaming(kind), &rec_kind) == RecordRoute::Terminal {
                self.orphaned.remove(&request_id);
            }
            return;
        }
        // A late record for a request nobody waits for any more is dropped, as Tauri does.
        let Some((tag, kind, sent)) = self.pending.get(&request_id) else {
            return;
        };
        let (tag, kind, sent) = (*tag, kind.clone(), *sent);
        let meta = self.meta.get(&request_id).cloned().unwrap_or_default();
        match route_record(&kind, meta.streaming, &rec_kind) {
            RecordRoute::Ignore => {}
            RecordRoute::Progress => {
                crate::shell::on_progress(self, tag, &envelope);
                crate::review::on_progress(self, tag, &envelope);
                crate::reports::books::on_progress(self, tag, &envelope);
                crate::assistant::on_progress(self, tag, &envelope);
                crate::wallets::on_progress(self, tag, &request_id, &envelope);
            }
            RecordRoute::Violation(err) => {
                // The daemon broke the protocol: it is killed, and everything it had fails.
                let err = self.daemon.break_process(pgen, err);
                self.daemon_error = Some(err.message.clone());
                self.fail_gen(pgen, &err);
            }
            RecordRoute::Terminal => {
                // An answer that overtook an earlier request with the same tag waits for it.
                if self.earlier_pending(tag, meta.seq, &request_id) {
                    self.parked.insert(request_id, (pgen, envelope));
                    return;
                }
                self.parked.remove(&request_id);
                self.pending.remove(&request_id);
                self.meta.remove(&request_id);
                if from_daemon {
                    self.daemon.note_answer(pgen);
                }
                if rec_kind == "error" && envelope["error"]["code"] == "internal_error" {
                    attach_stderr_tail(&mut envelope, self.daemon.stderr_tail(pgen));
                }
                // `note_project_data_root_from_response`: a respawn opens the selected project.
                if matches!(kind.as_str(), "ui.projects.select" | "ui.projects.create")
                    && rec_kind == kind
                    && let Some(root) = envelope["data"]["project"]["data_root"]
                        .as_str()
                        .filter(|r| !r.trim().is_empty())
                {
                    self.daemon.note_project_root(PathBuf::from(root));
                    self.launch = self.daemon.launch.clone();
                }
                let code = envelope["error"]["code"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let stale =
                    rec_kind == "error" && code == "stale_context" && meta.session == self.session;
                if stale && self.reread(&request_id, tag, &kind, &meta) {
                    // The read asked a book that is no longer open: read again, quietly.
                    self.release_parked(tag);
                    return;
                }
                let reply = make_reply(&kind, &rec_kind, &envelope, sent.elapsed());
                if rec_kind == "auth_required"
                    && self.on_auth_required(tag, &kind, &envelope, meta.session)
                    && self.locked
                    && !kept_tag(tag)
                {
                    // The challenge locked the app: every screen starts over behind the lock, so
                    // only a caller whose state survives a lock (Logs, Network monitor …) hears it.
                    self.busy.reads = self.busy.reads.saturating_sub(1);
                    self.release_parked(tag);
                    return;
                }
                if meta.held && self.cycle.holding {
                    self.cycle.held.push((tag, request_id, reply));
                    self.release_parked(tag);
                    return;
                }
                self.busy.reads = self.busy.reads.saturating_sub(1);
                // A write the daemon refused because another book is open (or a request refused
                // behind the lock) is not an error to show: the screens re-read instead.
                let quiet = code == LOCKED_CODE || (stale && rec_kind == "error");
                self.quiet_errors = quiet;
                self.deliver_as(tag, &request_id, reply);
                self.quiet_errors = false;
                if stale && !self.locked {
                    self.refresh_reads();
                }
                self.release_parked(tag);
            }
        }
    }

    /// A pending request with the same tag that was sent before `seq`.
    fn earlier_pending(&self, tag: &str, seq: u64, id: &str) -> bool {
        self.pending.iter().any(|(other, (t, _, _))| {
            *t == tag && other != id && self.meta.get(other).is_some_and(|m| m.seq < seq)
        })
    }

    /// Delivers the next parked answer of `tag`, once nothing sent before it is pending.
    fn release_parked(&mut self, tag: &str) {
        let next = self
            .parked
            .keys()
            .filter_map(|id| Some((self.meta.get(id)?.seq, id.clone())))
            .filter(|(_, id)| self.pending.get(id).is_some_and(|(t, _, _)| *t == tag))
            .min();
        if let Some((seq, id)) = next
            && !self.earlier_pending(tag, seq, &id)
            && let Some((pgen, env)) = self.parked.remove(&id)
        {
            self.on_record(pgen, id, env);
        }
    }

    /// Parked answers whose request was orphaned go; the rest that nothing holds back any more
    /// are delivered.
    fn sweep_parked(&mut self) {
        self.parked.retain(|id, _| self.pending.contains_key(id));
        let tags: Vec<&'static str> = self
            .parked
            .keys()
            .filter_map(|id| self.pending.get(id).map(|(t, _, _)| *t))
            .collect();
        for tag in tags {
            self.release_parked(tag);
        }
    }

    /// Re-sends a read the daemon refused with `stale_context`, keeping its place in its tag's
    /// order; false when it is not a read or has been read again often enough.
    fn reread(&mut self, old: &str, tag: &'static str, kind: &str, meta: &Meta) -> bool {
        let Some(args) = meta.reread.clone().filter(|_| meta.rereads < MAX_REREADS) else {
            return false;
        };
        self.busy.reads = self.busy.reads.saturating_sub(1);
        let id = self.daemon.alloc_id();
        self.register(&id, kind, tag, &args);
        if let Some(ticket) = self.tickets.remove(old) {
            self.tickets.insert(id.clone(), ticket);
        }
        if let Some(m) = self.meta.get_mut(&id) {
            m.seq = meta.seq;
            m.rereads = meta.rereads + 1;
        }
        match &mut self.root_switch {
            Some(sw) => sw.deferred.push((id, kind.to_owned(), args)),
            None => self.write(&id, kind, &args),
        }
        true
    }

    /// Hands a finished reply to the app and every module. An answer whose caller dropped
    /// every clone of its ticket is not delivered.
    pub(crate) fn deliver_as(&mut self, tag: &'static str, id: &str, reply: Reply) {
        let ticket = self.tickets.remove(id);
        if ticket
            .as_ref()
            .is_some_and(|ticket| ticket.strong_count() == 0)
        {
            return;
        }
        let before = std::mem::replace(&mut self.delivering_ticket, ticket);
        self.deliver(tag, reply);
        self.delivering_ticket = before;
    }

    /// Hands a finished reply to the app and every module.
    fn deliver(&mut self, tag: &'static str, reply: Reply) {
        let kind = reply.kind.clone();
        self.timings.push_back((kind.clone(), reply.elapsed));
        if self.timings.len() > 200 {
            self.timings.pop_front();
        }
        if kind == "status"
            && reply.error.is_none()
            && reply.data["database_encrypted"].is_boolean()
        {
            self.encrypted = reply.data["database_encrypted"] == true;
        }
        self.on_reply(tag, &reply);
        if reply.error.is_none() && tag != "app.revision" {
            if kind == "ui.profiles.snapshot" {
                // The open book, which Notifications file their entries under.
                self.shell.book = book_key(self.daemon.data_root(), &reply.data);
            }
            self.reads.insert(kind, reply.clone());
        }
        crate::shell::on_reply(self, tag, &reply);
        crate::overview::on_reply(self, tag, &reply);
        crate::chart::on_reply(self, tag, &reply);
        crate::transactions::on_reply(self, tag, &reply);
        crate::txdetail::on_reply(self, tag, &reply);
        crate::txdetail::graph::on_reply(self, tag, &reply);
        crate::settings::on_reply(self, tag, &reply);
        crate::wallets::on_reply(self, tag, &reply);
        crate::review::on_reply(self, tag, &reply);
        crate::reports::on_reply(self, tag, &reply);
        crate::assistant::on_reply(self, tag, &reply);
        crate::custody::on_reply(self, tag, &reply);
        crate::provenance::on_reply(self, tag, &reply);
        crate::secrets::on_reply(self, tag, &reply);
        crate::onboarding::on_reply(self, tag, &reply);
        // `mutationAdvancesDaemonSession`: another book is open, whoever switched (the title
        // bar's switcher, the Books page, a book set's overview). Welcome's own switch enters
        // the app afresh already.
        if reply.kind == "ui.profiles.switch"
            && reply.error.is_none()
            && tag != "onboarding.import_switch"
        {
            self.begin_book_session();
        }
    }

    /// An `auth_required` answer; true when it is handled here (not the caller's to show).
    fn on_auth_required(&mut self, tag: &str, kind: &str, env: &Value, session: u64) -> bool {
        if tag == "app.unlock" {
            return false;
        }
        let scope = env["data"]["scope"].as_str().unwrap_or_default();
        if kind == "ui.projects.select" && scope == "unlock_project" {
            // `switchProject`: the chosen book wants its passphrase before it opens.
            self.daemon_auth_required = true;
            self.encrypted = true;
            self.lock_session(false);
            self.lock.pending_project = Some(env["data"]["project"].clone());
            return true;
        }
        if scope == "unlock_database" {
            // The database's own challenge (a locked or restarted daemon): the lock screen asks.
            if session == self.session && !self.locked {
                self.encrypted = true;
                // At launch this is the encrypted book's first lock; later, a session that ended.
                self.daemon_auth_required = self.ever_unlocked;
                self.lock_session(false);
            }
            return true;
        }
        false
    }

    // ---- Lock ----

    /// The lock screen's reason and whether it asks for a passphrase (`lockScreenConfig`).
    pub fn lock_reason(&self) -> LockReason {
        if self.daemon_auth_required {
            LockReason::DaemonAuth
        } else if self.encrypted {
            LockReason::Encrypted
        } else {
            LockReason::Session
        }
    }

    pub fn passphrase_required(&self) -> bool {
        self.encrypted || self.daemon_auth_required
    }

    /// `applyLock`: drop every cached read and screen state, show the lock screen, and (when
    /// `send_lock`) close the daemon's database handle with `daemon.lock`.
    pub fn lock_session(&mut self, send_lock: bool) {
        if self
            .read("status")
            .is_some_and(|s| s["database_encrypted"] == true)
        {
            self.encrypted = true;
        }
        self.session += 1;
        self.locked = true;
        self.lock = Default::default();
        self.reset_book_state();
        self.toasts.clear();
        if send_lock {
            self.send("daemon.lock", Value::Null, "app.lock");
        }
    }

    /// The lock screen's submit (`unlockApp`): `daemon.unlock`, or `ui.projects.select` for a
    /// book that asked for its passphrase on opening, with the passphrase as `auth_response`.
    pub fn submit_unlock(&mut self) {
        if self.lock.submitting {
            return;
        }
        let passphrase = self.lock.passphrase.text.clone();
        if self.passphrase_required() && passphrase.is_empty() {
            // An empty field cannot open the books; asking the daemon would read as a refusal.
            self.lock.error = Some(t("chrome:lock.passphraseEmpty"));
            self.lock.passphrase.focused = true;
            return;
        }
        self.lock.error = None;
        self.lock.submitting = true;
        self.lock.attempt += 1;
        self.session += 1;
        let auth = json!({"passphrase_secret": passphrase});
        if let Some(project) = self.lock.pending_project.clone() {
            let args = json!({"project_id": project["id"], "require_existing_project": true, "auth_response": auth});
            self.send("ui.projects.select", args, "app.unlock");
        } else {
            let mut args = json!({"auth_response": auth});
            // A data root opened by path (not the default state root) must already hold a book.
            if self.daemon.launch.explicit_root {
                args["require_existing_project"] = json!(true);
            }
            self.send("daemon.unlock", args, "app.unlock");
        }
    }

    fn on_unlock_reply(&mut self, r: &Reply) {
        self.lock.submitting = false;
        if r.error.is_some() {
            if r.error_detail["code"] == "auth_required" {
                self.daemon_auth_required = true;
            }
            self.lock.error = Some(unlock_failure(r));
            self.lock.passphrase = rune::TextEdit::new("");
            self.lock.passphrase.focused = true;
            return;
        }
        let project = self.lock.pending_project.take();
        let status = r.data["status"].clone();
        if status.is_object() {
            self.encrypted = status["database_encrypted"] == true;
        }
        self.ever_unlocked = true;
        self.daemon_auth_required = false;
        self.locked = false;
        self.lock = Default::default();
        // Whatever screens did behind the lock starts over now.
        self.reset_book_state();
        if status.is_object() {
            self.reads.insert(
                "status".into(),
                Reply {
                    kind: "status".into(),
                    data: status,
                    error: None,
                    error_detail: Value::Null,
                    elapsed: r.elapsed,
                },
            );
        }
        self.load_shell_reads();
        if project.is_some() {
            self.navigate("/overview");
        }
        if self.prefs.hide_on_unlock && self.prefs.privacy == Privacy::Normal {
            self.prefs.privacy = Privacy::HideAmounts;
        }
    }

    /// Forgets every cached read and the state of every screen that shows book data (a lock, an
    /// unlock, a data-root switch). Preferences, the RAM logs and window layout stay. Requests in
    /// flight are orphaned: their answers are dropped when they come.
    /// Crash recovery's Reload (the web reloads its page): every screen's state starts afresh
    /// and the shell's reads load again. The daemon, its unlock, the preferences, the identity
    /// and the Logs ring stay; requests in flight finish unheard.
    pub fn reload_ui(&mut self) {
        self.reset_book_state();
        self.toasts.clear();
        // Holdings' view and the widget layout are preferences, not screen state.
        let (holdings, layout) = (
            std::mem::take(&mut self.ov.holdings),
            std::mem::take(&mut self.ov.layout),
        );
        self.ov = Default::default();
        self.ov.holdings = holdings;
        self.ov.layout = layout;
        self.settings.open = Default::default();
        self.shell.settings_open = false;
        self.shell.popup = None;
        if !self.locked && self.onboarding.identity.is_some() {
            self.load_shell_reads();
        }
    }

    pub(crate) fn reset_book_state(&mut self) {
        let ids: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, (tag, _, _))| !kept_tag(tag))
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            let (_, kind, _) = self.pending.remove(&id).expect("pending");
            let pgen = self.meta.remove(&id).map(|m| m.pgen).unwrap_or(0);
            self.parked.remove(&id);
            self.orphaned.insert(id, (kind, pgen));
        }
        self.tickets.retain(|id, _| self.pending.contains_key(id));
        self.cycle = BookRefresh::default();
        self.last_change = None;
        self.reads.clear();
        self.overview = Load::Idle;
        self.badges = None;
        self.busy = Busy {
            reads: self.pending.len(),
            ..Busy::default()
        };

        let sort = self.txs.filters.sort;
        self.txs = Default::default();
        self.txs.filters.sort = sort;
        let (width, maximized) = (self.detail.width, self.detail.maximized);
        self.detail = Default::default();
        self.detail.width = width;
        self.detail.maximized = maximized;
        self.graph = Default::default();
        self.wallets = Default::default();
        self.review = Default::default();
        self.custody = Default::default();
        // Registries outside the app state go with it: plans still running and revealed wallet
        // material waiting for the clipboard.
        crate::custody::unlink::common::forget_running();
        crate::wallets::detail::forget_pending_copy();
        self.provenance = Default::default();
        let diag = std::mem::take(&mut self.reports.diag);
        self.reports = Default::default();
        self.reports.diag = diag;
        self.reports.diag.act = Default::default();
        self.reports.diag.chg = Default::default();
        self.reports.diag.popup = None;
        // Replay consent and its passphrase belong to the book they were given for.
        self.reports.diag.replay = Default::default();
        let a = std::mem::take(&mut self.assistant);
        self.assistant.selection = a.selection;
        self.assistant.effort = a.effort;
        self.assistant.fast_mode = a.fast_mode;
        self.assistant.permission = a.permission;
        self.assistant.draft = a.draft;
        self.assistant.tab_open = a.tab_open;
        self.assistant.surfaces = a.surfaces;
        self.assistant.front = a.front;
        let (prefs, fx, open) = (
            std::mem::take(&mut self.settings.prefs),
            std::mem::take(&mut self.settings.fx),
            std::mem::take(&mut self.settings.open),
        );
        self.settings = Default::default();
        self.settings.prefs = prefs;
        self.settings.fx = fx;
        self.settings.open = open;
        let c = std::mem::take(&mut self.chart);
        self.chart.period = c.period;
        self.chart.vis = c.vis;
        self.chart.inc_min = c.inc_min;
        self.chart.out_min = c.out_min;
        self.chart.group = c.group;
        self.chart.log_y = c.log_y;
        self.chart.log_x = c.log_x;
        self.chart.fit = c.fit;
        self.chart.show_last = c.show_last;
        self.ov.add_open = false;
        let sh = &mut self.shell;
        sh.popup = None;
        sh.settings_open = false;
        sh.sync_card_open = false;
        sh.sync_failed = false;
        sh.maint = None;
        sh.run = Default::default();
        sh.journals_failed = None;
        sh.book = None;
        sh.resolved = None;
        sh.resolving = None;
        sh.switching_profile = None;
        sh.books_error = None;
        sh.probes.clear();
        sh.probing = false;
        sh.search = rune::TextEdit::new("");
    }

    /// A successful book switch (`ui.profiles.switch`, `ui.projects.select`): a new session, as
    /// the web's `daemonSession` bump. Every request the old book still had in flight is
    /// orphaned (its answer dropped when it comes), every screen's book state (the docked
    /// transaction detail and its actions, drafts, caches, page state, the journal-failure
    /// notice and its Retry) starts over, and the shell reads the new book.
    pub fn begin_book_session(&mut self) {
        self.session += 1;
        self.reset_book_state();
        if !self.locked {
            self.send("status", Value::Null, "app.status");
            self.load_shell_reads();
        }
    }

    /// Tauri's `current_data_root().unwrap_or_else(default_state_data_root)`: the data root the
    /// daemon serves (it follows a book switch), else the default project's. The managed-path
    /// checks (exports, attachments) resolve against it.
    pub fn data_root(&self) -> Option<std::path::PathBuf> {
        self.daemon
            .data_root()
            .map(std::path::Path::to_path_buf)
            .or_else(crate::daemon::default_state_data_root)
    }

    // ---- Diagnostics ----

    /// The supervisor's lifecycle ring after `after` (`daemon_lifecycle_snapshot`), for the Logs
    /// page and panel; `pump` already folds new records into the Logs ring.
    #[allow(dead_code)] // the Logs panel may read the ring directly
    pub fn daemon_lifecycle(&self, after: u64) -> (Vec<crate::daemon::LifecycleRecord>, u64) {
        self.daemon.lifecycle(after)
    }

    /// `pollSupervisorRing`: new lifecycle records join the Logs ring (RAM only).
    fn fold_lifecycle(&mut self) -> bool {
        use crate::reports::diag::logstore::{Record, iso_from_secs, text};
        let (records, last) = self.daemon.lifecycle(self.lifecycle_seen);
        if records.is_empty() {
            return false;
        }
        self.lifecycle_seen = last;
        for r in records {
            let mut fields = vec![("source".to_owned(), text(r.source))];
            if !r.stderr_tail.is_empty() {
                fields.push(("stderr_tail".to_owned(), text(&r.stderr_tail)));
            }
            self.reports.diag.logs.store.emit(Record {
                id: format!("super-{}", r.id),
                ts: iso_from_secs(r.ts_ms as f64 / 1000.0),
                level: r.level().into(),
                module: "supervisor".into(),
                file: "supervisor.rs".into(),
                line: 0,
                msg: if r.detail.is_empty() {
                    r.event.to_owned()
                } else {
                    format!("{}: {}", r.event, r.detail)
                },
                fields,
            });
        }
        true
    }

    fn on_reply(&mut self, tag: &str, r: &Reply) {
        match tag {
            "app.overview" => {
                self.overview = match (&r.error, r.data.is_null()) {
                    (Some(e), _) => Load::Failed(e.clone()),
                    (None, true) => Load::Failed(
                        "No real overview data is available for the current book.".into(),
                    ),
                    (None, false) => match Snapshot::parse(&r.data) {
                        Ok(s) => Load::Ready(s),
                        Err(e) => Load::Failed(format!(
                            "Kassiber could not read real overview data. ({e})"
                        )),
                    },
                };
            }
            "app.badges" if r.error.is_none() => {
                self.badges = serde_json::from_value(r.data.clone()).ok();
            }
            "app.revision" => self.on_revision(r),
            "app.unlock" => self.on_unlock_reply(r),
            #[cfg(test)]
            "test.panic" => panic!("synthetic reply handler panic"),
            "app.freshness_run" => {
                self.busy.syncing = false;
                self.busy.progress = None;
                // Toast, Recent entry, condition and the card's failed state (useWalletSyncAction).
                crate::shell::maint::on_refresh_reply(self, r);
                self.refresh_reads();
                if std::mem::take(&mut self.shell.run.exchanges) {
                    crate::wallets::add::exchange::refresh::sync_after_refresh(self);
                }
            }
            "app.journals_process" => {
                self.busy.processing = false;
                self.busy.progress = None;
                crate::shell::maint::on_journals_reply(self, r);
                self.refresh_reads();
            }
            _ => {}
        }
    }

    pub fn read(&self, kind: &str) -> Option<&Value> {
        self.reads.get(kind).map(|r| &r.data)
    }

    pub fn snapshot(&self) -> Option<&Snapshot> {
        self.overview.ready()
    }

    /// `expected_scope` for book-bound mutations.
    pub fn expected_scope(&self) -> Value {
        let p = self.read("ui.profiles.snapshot");
        json!({
            "workspace_id": p.and_then(|p| p.get("activeWorkspaceId")).cloned().unwrap_or(Value::Null),
            "profile_id": p.and_then(|p| p.get("activeProfileId")).cloned().unwrap_or(Value::Null),
        })
    }

    // ---- Navigation ----

    pub fn navigate(&mut self, to: &str) {
        if let Some(target) = crate::shell::redirect(self, to) {
            return self.navigate(&target);
        }
        if self.route.0 == to {
            return;
        }
        let prev = std::mem::replace(&mut self.route, Route(to.to_owned()));
        self.back.push(prev);
        self.forward.clear();
        self.page_changed();
        self.shell.close_popups();
    }

    /// A different page is now showing: reveal its sidebar entry, and bring a maximized right
    /// panel back to its docked width so the page can be seen. The panel stays open with its tab,
    /// scroll and draft. An owner request: the web keeps the panel maximized over the new page
    /// (`rightPanelMaximized` resets only when the panel closes), so a nav click looked like it
    /// did nothing.
    fn page_changed(&mut self) {
        crate::shell::on_leave(self);
        crate::shell::sidebar::reveal_route(self);
        self.detail.maximized = false;
    }

    /// Back (⌘[, the title bar, mouse buttons, the history swipe). An entry the route guard
    /// now refuses (a developer page with developer tools off, the assistant with AI off) lands
    /// where the guard sends it, as the web router's `beforeLoad` redirect does.
    pub fn go_back(&mut self) {
        if let Some(r) = self.back.pop() {
            let r = self.guarded(r);
            if r == self.route {
                return;
            }
            let cur = std::mem::replace(&mut self.route, r);
            self.forward.push(cur);
            self.page_changed();
            self.shell.close_popups();
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(r) = self.forward.pop() {
            let r = self.guarded(r);
            if r == self.route {
                return;
            }
            let cur = std::mem::replace(&mut self.route, r);
            self.back.push(cur);
            self.page_changed();
            self.shell.close_popups();
        }
    }

    fn guarded(&self, r: Route) -> Route {
        match crate::shell::route_guard(self, r.path()) {
            Some(target) => Route(target),
            None => r,
        }
    }

    // ---- Toasts ----

    pub fn toast(
        &mut self,
        tone: ToastTone,
        title: &str,
        body: Option<String>,
        key: Option<&'static str>,
    ) {
        self.toast_with(tone, title, body, key, vec![]);
    }

    /// [`Self::toast`] with up to two actions; a toast with actions stays longer. A repeat of a
    /// toast that has actions takes the newest body and actions (they name the newest file).
    pub fn toast_with(
        &mut self,
        tone: ToastTone,
        title: &str,
        body: Option<String>,
        key: Option<&'static str>,
        mut actions: Vec<crate::toastopen::ToastAction>,
    ) {
        if self.quiet_errors && tone == ToastTone::Error {
            return;
        }
        actions.truncate(2);
        let life = if actions.is_empty() {
            6.0
        } else {
            crate::toastopen::ACTION_LIFE
        };
        if let Some(k) = key {
            if let Some(t) = self
                .toasts
                .iter_mut()
                .find(|t| t.key == Some(k) && t.title == title)
            {
                t.repeat += 1;
                t.until = self.clock + life;
                if !actions.is_empty() || !t.actions.is_empty() {
                    t.body = body;
                    t.actions = actions;
                }
                return;
            }
            self.toasts.retain(|t| t.key != Some(k));
        }
        self.next_toast += 1;
        self.toasts.push(Toast {
            id: self.next_toast,
            key,
            title: title.to_owned(),
            body,
            tone,
            repeat: 1,
            until: self.clock + life,
            actions,
            hovered: false,
        });
    }

    // ---- Shared actions ----

    /// `syncAll()`: the header refresh, ⌘R, and the chart's empty-state Refresh. Network egress,
    /// so only ever called from a user action.
    pub fn refresh_book(&mut self, force_full: bool) {
        self.refresh_book_with(force_full, false);
    }

    /// The title-bar refresh (its button, menu and shortcut): the book, then every exchange API
    /// source with a key, one request each (`includeExchangeApis`). Other refreshes never contact
    /// an exchange.
    pub fn refresh_book_from_title_bar(&mut self, force_full: bool) {
        self.refresh_book_with(force_full, true);
    }

    fn refresh_book_with(&mut self, force_full: bool, exchanges: bool) {
        if self.busy.syncing || self.busy.processing {
            return;
        }
        let mut args = json!({"all": true, "journals": true, "auto_pair": true, "run": true, "force_full": force_full});
        args["expected_scope"] = self.expected_scope();
        if self
            .send("ui.freshness.run", args, "app.freshness_run")
            .is_some()
        {
            self.busy.syncing = true;
            crate::shell::maint::start_refresh(self, force_full);
            self.shell.run.exchanges = exchanges;
        }
    }

    /// `runJournalProcessing`. A second request while a run is in flight does nothing: the
    /// running job is already visible (the web shows no "already running" notice either).
    pub fn process_journals(&mut self) {
        self.process_journals_with(true);
    }

    /// `runJournalProcessingWith`: `auto_pair: false` recalculates without pairing exact
    /// matches (a Retry repeats the failed run's choice).
    pub fn process_journals_with(&mut self, auto_pair: bool) {
        if self.busy.processing || self.busy.syncing {
            return;
        }
        let mut args = json!({"expected_scope": self.expected_scope()});
        if !auto_pair {
            args["auto_pair"] = json!(false);
        }
        if self
            .send("ui.journals.process", args, "app.journals_process")
            .is_some()
        {
            self.busy.processing = true;
            crate::shell::maint::start_journals(self, auto_pair);
        }
    }
}

/// A terminal envelope as the UI keeps it. `auth_required` is an error reply to its caller
/// (`DaemonAuthRequiredError`), its challenge in `error_detail.details`.
fn make_reply(kind: &str, rec_kind: &str, env: &Value, elapsed: Duration) -> Reply {
    let data = env.get("data").cloned().unwrap_or(Value::Null);
    if rec_kind == "auth_required" {
        let label = data["label"]
            .as_str()
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| t("chrome:lock.passphraseRequiredError"));
        let detail = json!({"code": "auth_required", "message": label, "details": data});
        return Reply {
            kind: kind.to_owned(),
            data,
            error: Some(label),
            error_detail: detail,
            elapsed,
        };
    }
    let error = env.get("error").filter(|e| !e.is_null()).map(|e| {
        e.get("message")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| e.to_string())
    });
    let error_detail = env.get("error").cloned().unwrap_or(Value::Null);
    Reply {
        kind: kind.to_owned(),
        data,
        error,
        error_detail,
        elapsed,
    }
}

/// `attach_stderr_tail_to_internal_error`.
fn attach_stderr_tail(env: &mut Value, tail: String) {
    if tail.is_empty() {
        return;
    }
    let Some(error) = env.get_mut("error").and_then(Value::as_object_mut) else {
        return;
    };
    match error.get_mut("details") {
        Some(Value::Object(d)) => {
            d.insert("stderr_tail".into(), Value::String(tail));
        }
        Some(Value::Null) | None => {
            error.insert("details".into(), json!({"stderr_tail": tail}));
        }
        Some(other) => {
            let previous = other.take();
            *other = json!({"details": previous, "stderr_tail": tail});
        }
    }
}

/// `set_data_root` / `clear_data_root` for the app: restart the daemon against another data root
/// (None: the default state root) and preference environment (`env`: the demo's own files, or
/// empty for every real book), as the Tauri shell does when a book folder is opened.
///
/// It never blocks: the replacement warms in the background while the current daemon keeps
/// serving, and `done` runs from the pump with the outcome. From the start every screen starts
/// over (requests in flight are orphaned; those made meanwhile wait for the outcome and go to
/// whichever daemon serves then). On failure the current daemon stays. A switch away from the
/// demo's environment ends the demo (`crate::demo::ended`): the one way out, whatever the exit.
pub fn switch_root(
    app: &mut App,
    root: Option<PathBuf>,
    env: Vec<(String, String)>,
    done: impl FnOnce(&mut App, Result<(), String>) + 'static,
) {
    // A switch still warming is abandoned (its continuation never runs); what it deferred
    // waits for this one.
    let carried = match app.root_switch.take() {
        Some(prev) => {
            app.demo.switching = false;
            prev.deferred
        }
        None => Vec::new(),
    };
    match app.daemon.begin_switch(root, env) {
        Ok(true) => {
            app.session += 1;
            app.encrypted = false;
            app.daemon_auth_required = false;
            app.ever_unlocked = false;
            app.locked = false;
            app.lock = Default::default();
            app.reset_book_state();
            app.daemon_error = None;
            app.root_switch = Some(RootSwitch {
                done: Some(Box::new(done)),
                deferred: carried,
            });
        }
        outcome => {
            for (id, kind, args) in carried {
                app.write(&id, &kind, &args);
            }
            match outcome {
                Ok(_) => done(app, Ok(())),
                Err(e) => done(app, Err(e.message)),
            }
        }
    }
}

impl App {
    /// The pump's half of `switch_root`: once the replacement is ready (or failed), apply it
    /// and run the continuation.
    fn step_switch(&mut self) -> bool {
        if self.root_switch.is_none() {
            return false;
        }
        let Some(outcome) = self.daemon.poll_switch() else {
            return false;
        };
        let sw = self.root_switch.take().expect("switch");
        match outcome {
            Ok(old) => {
                self.launch = self.daemon.launch.clone();
                if let Some(old) = old {
                    self.fail_gen(
                        old,
                        &DaemonError::new(
                            "daemon_exited",
                            "The daemon was replaced: the data root changed",
                        )
                        .retryable(),
                    );
                }
                self.daemon_error = None;
                if self
                    .demo
                    .active
                    .as_ref()
                    .is_some_and(|d| d.env != self.daemon.launch.env)
                {
                    crate::demo::ended(self);
                }
                for (id, kind, args) in sw.deferred {
                    self.write(&id, &kind, &args);
                }
                if let Some(done) = sw.done {
                    done(self, Ok(()));
                }
            }
            Err(e) => {
                for (id, _, _) in sw.deferred {
                    self.local.push_back((
                        0,
                        Incoming::Record {
                            request_id: id.clone(),
                            envelope: e.envelope(&id),
                        },
                    ));
                }
                if let Some(done) = sw.done {
                    done(self, Err(e.message));
                }
            }
        }
        true
    }

    /// `status` and the shell's reads: a book just became the open one.
    pub fn load_book(&mut self) {
        self.send("status", Value::Null, "app.status");
        self.load_shell_reads();
    }
}

/// The key Notifications file entries under: the data root and the open book's ids (None while
/// the open book is unknown, so nothing is filed in, or acts on, the wrong book).
pub fn book_key(root: Option<&std::path::Path>, profiles: &Value) -> Option<String> {
    let id = |k: &str| {
        profiles[k]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    let (w, p) = (id("activeWorkspaceId")?, id("activeProfileId")?);
    Some(format!(
        "{}|{w}|{p}",
        root.map(|r| r.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

/// The refusal a request made behind the lock gets.
pub const LOCKED_CODE: &str = "app_locked";

/// `pagehide` → `lockApp` when "Lock when the window closes" is on for an encrypted book.
pub fn on_window_close(app: &mut App, _tx: &mut rune::Tx) {
    let encrypted = app.encrypted || crate::settings::effects::encrypted(app);
    if encrypted && app.settings.prefs.lock_on_close && !app.locked {
        app.lock_session(true);
    }
}

/// Headless states of the runtime (`--scene`).
pub static SCENES: &[crate::app::Scene] = &[
    // The daemon is killed mid-session while the Overview reads: the read fails with the
    // daemon's exit, the Logs ring records it, and the next request (any local read) respawns.
    ("runtime-daemon-killed", |a| {
        a.prefs.developer_tools = true;
        a.overview = Load::Loading;
        // A slow read in flight (the whole transaction list), then the kill.
        a.send("ui.transactions.list", json!({"limit": 5000}), "app.scene");
        a.send("ui.overview.snapshot", Value::Null, "app.overview");
        if let Some(pid) = a.daemon.pid() {
            let _ = std::process::Command::new("kill")
                .arg("-9")
                .arg(pid.to_string())
                .status();
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while !a.pending.is_empty() && Instant::now() < deadline {
            a.pump();
            std::thread::sleep(Duration::from_millis(5));
        }
        a.pump();
    }),
];

pub fn flag(args: &[String], name: &str) -> Option<String> {
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

impl Drop for App {
    /// `--i18n-report <file>`: the keys that fell back to English or to the key itself.
    fn drop(&mut self) {
        if let Some(path) = flag(rune::extra_args(), "--i18n-report") {
            let _ = std::fs::write(path, crate::i18n::missing().join("\n") + "\n");
        }
    }
}

#[cfg(test)]
mod delivery_model;

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn refresh_events_follow_parse_daemon_freshness_event() {
        let ev = |v: Value| is_book_refresh_event(&v);
        assert!(ev(
            json!({"kind": "ui.freshness.background", "schema_version": 1, "event": true, "data": {"completed": []}})
        ));
        assert!(ev(
            json!({"kind": "ui.freshness.worker", "schema_version": 1, "event": true, "data": {"status": "error"}})
        ));
        assert!(ev(
            json!({"kind": "ui.chain_analysis.watches.changed", "schema_version": 1, "event": true})
        ));
        // Progress, idle workers, request-bound or malformed records start nothing.
        assert!(!ev(
            json!({"kind": "ui.freshness.progress", "schema_version": 1, "event": true, "data": {}})
        ));
        assert!(!ev(
            json!({"kind": "ui.freshness.worker", "schema_version": 1, "event": true, "data": {"status": "idle"}})
        ));
        assert!(!ev(
            json!({"kind": "ui.freshness.background", "schema_version": 1, "event": true, "request_id": "x", "data": {}})
        ));
        assert!(!ev(
            json!({"kind": "ui.freshness.background", "event": true, "data": {}})
        ));
    }

    #[test]
    fn change_summary_follows_describe_change() {
        let v = |c: f64, b: i64| BookView {
            transaction_count: Some(c),
            balance_sat: Some(b),
        };
        assert_eq!(describe_change(v(10.0, 5), v(10.0, 5), 1.0), None);
        assert_eq!(
            describe_change(v(10.0, 5), v(12.0, 9), 1.0),
            Some(BookChange {
                transaction_delta: 2,
                balance_delta_sat: Some(4),
                at: 1.0
            })
        );
        assert_eq!(
            describe_change(v(10.0, 5), v(9.0, 5), 1.0),
            Some(BookChange {
                transaction_delta: -1,
                balance_delta_sat: None,
                at: 1.0
            })
        );
        assert_eq!(describe_change(BookView::default(), v(9.0, 5), 1.0), None);
    }

    #[test]
    fn auth_required_becomes_an_error_reply() {
        let env = json!({"kind": "auth_required", "request_id": "r", "data": {"scope": "reveal_token", "label": "Re-enter it"}});
        let r = make_reply(
            "wallets.reveal_descriptor",
            "auth_required",
            &env,
            Duration::ZERO,
        );
        assert_eq!(r.error.as_deref(), Some("Re-enter it"));
        assert_eq!(r.error_detail["code"], "auth_required");
        assert_eq!(r.error_detail["details"]["scope"], "reveal_token");
    }

    #[test]
    fn internal_errors_carry_the_stderr_tail() {
        let mut env = json!({"kind": "error", "error": {"code": "internal_error", "message": "x", "details": null}});
        attach_stderr_tail(&mut env, "trace".into());
        assert_eq!(env["error"]["details"]["stderr_tail"], "trace");
        let mut env = json!({"kind": "error", "error": {"code": "internal_error", "message": "x", "details": "d"}});
        attach_stderr_tail(&mut env, "trace".into());
        assert_eq!(
            env["error"]["details"],
            json!({"details": "d", "stderr_tail": "trace"})
        );
    }

    // ---- Session, ordering and refusal rules (review findings 2, 3, 6, 9) ----

    /// An app whose daemon can never start (unit tests have no data root): every write fails at
    /// once with a local error, and nothing of App::new's launch is pending.
    pub(crate) fn quiet_app() -> App {
        let mut app = App::new();
        app.daemon = Supervisor::new(Launch {
            data_root: None,
            explicit_root: false,
            bin_flag: None,
            binary: "kassiber".into(),
            env: Vec::new(),
        });
        app.pending.clear();
        app.meta.clear();
        app.parked.clear();
        app.local.clear();
        app.root_switch = None;
        app.busy = Busy::default();
        app.toasts.clear();
        app
    }

    /// A request as `send` registers it, without writing it anywhere.
    pub(crate) fn inject(app: &mut App, kind: &str, tag: &'static str) -> String {
        let id = app.daemon.alloc_id();
        app.register(&id, kind, tag, &Value::Null);
        id
    }

    pub(crate) fn inject_ticket(app: &mut App, kind: &str, tag: &'static str) -> (String, Ticket) {
        let id = inject(app, kind, tag);
        let ticket = Ticket(Arc::new(()));
        app.tickets.insert(id.clone(), Arc::downgrade(&ticket.0));
        (id, ticket)
    }

    pub(crate) fn answer(app: &mut App, id: &str, kind: &str, data: Value) {
        app.local.push_back((0, Incoming::Record { request_id: id.to_owned(), envelope: json!({"kind": kind, "schema_version": 1, "request_id": id, "data": data}) }));
    }

    fn refuse(app: &mut App, id: &str, code: &str) {
        let env = json!({"kind": "error", "schema_version": 1, "request_id": id, "error": {"code": code, "message": "The active book changed before the requested input was supplied", "retryable": true}});
        app.local.push_back((
            0,
            Incoming::Record {
                request_id: id.to_owned(),
                envelope: env,
            },
        ));
    }

    fn in_app(app: &mut App) {
        app.onboarding.identity = Some(crate::onboarding::model::Identity {
            name: "Book".into(),
            ..Default::default()
        });
        app.route = Route("/overview".into());
    }

    /// Exchange APIs are contacted after a title-bar refresh only (docs/reference/privacy-and-security.md):
    /// the chart's, the menu's and a notification's Retry refresh never ask for an exchange.
    #[test]
    fn only_the_title_bar_refresh_reaches_exchanges() {
        let asked = |title_bar: bool| {
            let mut app = quiet_app();
            in_app(&mut app);
            if title_bar {
                app.refresh_book_from_title_bar(false);
            } else {
                app.refresh_book(false);
            }
            for _ in 0..6 {
                app.pump();
            }
            app.timings
                .iter()
                .map(|(kind, _)| kind.clone())
                .collect::<Vec<_>>()
        };
        let plain = asked(false);
        assert!(plain.iter().any(|k| k == "ui.freshness.run"), "{plain:?}");
        assert!(
            !plain
                .iter()
                .any(|k| k.starts_with("ui.connections.exchange")),
            "{plain:?}"
        );
        let title_bar = asked(true);
        assert!(
            title_bar
                .iter()
                .any(|k| k == "ui.connections.exchange.status"),
            "{title_bar:?}"
        );
    }

    #[test]
    fn a_panic_in_one_reply_leaves_the_rest_of_the_batch_answered() {
        let mut app = quiet_app();
        let bad = inject(&mut app, "ui.review.badges", "test.panic");
        let good = inject(&mut app, "ui.review.badges", "app.badges");
        answer(&mut app, &bad, "ui.review.badges", json!({}));
        answer(
            &mut app,
            &good,
            "ui.review.badges",
            json!({"quarantine": 2}),
        );
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| app.pump()));
        assert!(caught.is_err(), "the panic still reaches crash recovery");
        // The record after the panicking one was not dropped: the next pump answers it.
        assert!(app.pending.contains_key(&good));
        app.pump();
        assert!(app.pending.is_empty());
        assert!(app.read("ui.review.badges").is_some());
    }

    #[test]
    fn answers_with_one_tag_reach_the_screen_in_the_order_it_asked() {
        let mut app = quiet_app();
        let first = inject(&mut app, "ui.review.badges", "app.badges");
        let second = inject(&mut app, "ui.review.badges", "app.badges");
        // The daemon answers the later request first.
        answer(
            &mut app,
            &second,
            "ui.review.badges",
            json!({"quarantine": 2}),
        );
        app.pump();
        assert!(
            app.read("ui.review.badges").is_none(),
            "the overtaking answer waits"
        );
        answer(
            &mut app,
            &first,
            "ui.review.badges",
            json!({"quarantine": 1}),
        );
        app.pump();
        // Both delivered, the later one last: the screen shows the newest answer.
        assert!(app.pending.is_empty() && app.parked.is_empty());
        assert_eq!(app.read("ui.review.badges").unwrap()["quarantine"], 2);
        // Another tag is never held back by it.
        let slow = inject(&mut app, "ui.overview.snapshot", "app.overview");
        let fast = inject(&mut app, "ui.review.badges", "app.badges");
        answer(
            &mut app,
            &fast,
            "ui.review.badges",
            json!({"quarantine": 5}),
        );
        app.pump();
        assert_eq!(app.read("ui.review.badges").unwrap()["quarantine"], 5);
        assert!(app.pending.contains_key(&slow));
    }

    #[test]
    fn a_book_switch_starts_a_new_session() {
        let mut app = quiet_app();
        in_app(&mut app);
        app.shell.journals_failed = Some("journal run failed".into());
        crate::txdetail::open(&mut app, "tx-a", "details");
        app.pump();
        assert!(app.detail.open.is_some());
        app.shell.book = Some("root|w1|p1".into());
        app.shell.note("Synced", None, ToastTone::Success, 0.0);
        let session = app.session;
        // Book A's read is still in flight when the switch answers.
        let old = inject(&mut app, "ui.overview.snapshot", "app.overview");
        let sw = inject(&mut app, "ui.profiles.switch", "shell.switch_profile");
        answer(
            &mut app,
            &sw,
            "ui.profiles.switch",
            json!({"profile_id": "p2"}),
        );
        app.pump();
        assert!(app.session > session);
        assert!(
            app.detail.open.is_none(),
            "the docked detail (and its actions) closes"
        );
        assert!(
            app.shell.journals_failed.is_none(),
            "Retry has nothing of book A to retry"
        );
        assert!(
            app.shell.recent_here().next().is_none(),
            "book A's Recent entries stay with book A"
        );
        // Book A's answer arrives after the switch: dropped, never shown as book B's.
        answer(
            &mut app,
            &old,
            "ui.overview.snapshot",
            json!({"status": {"transaction_count": 99}}),
        );
        app.pump();
        assert!(app.read("ui.overview.snapshot").is_none());
        assert!(!matches!(app.overview, Load::Ready(_)));
        // Book A's entries come back with book A.
        app.shell.book = Some("root|w1|p1".into());
        assert_eq!(app.shell.recent_here().count(), 1);
    }

    #[test]
    fn requests_behind_the_lock_are_refused_quietly() {
        let mut app = quiet_app();
        in_app(&mut app);
        app.locked = true;
        app.send(
            "ui.logs.snapshot",
            json!({"after_id": 0}),
            "reports.diag.logs.snapshot",
        );
        app.send("ui.overview.snapshot", Value::Null, "app.overview");
        app.pump();
        // Every request got its answer (nothing waits forever), and no error toast shows.
        assert!(app.pending.is_empty());
        assert!(app.toasts.iter().all(|t| t.tone != ToastTone::Error));
        assert!(!app.reports.diag.logs.store.in_flight);
    }

    #[test]
    fn a_challenge_that_locks_still_answers_callers_that_outlive_the_lock() {
        let mut app = quiet_app();
        in_app(&mut app);
        let logs = inject(&mut app, "ui.logs.snapshot", "reports.diag.logs.snapshot");
        app.reports.diag.logs.store.in_flight = true;
        let env = json!({"kind": "auth_required", "schema_version": 1, "request_id": logs, "data": {"scope": "unlock_database"}});
        app.local.push_back((
            0,
            Incoming::Record {
                request_id: logs.clone(),
                envelope: env,
            },
        ));
        app.pump();
        assert!(app.locked, "the database's challenge locks");
        assert!(
            !app.reports.diag.logs.store.in_flight,
            "the Logs poll heard its answer and can resume"
        );
    }

    #[test]
    fn stale_context_reads_are_read_again_and_writes_re_read_without_an_error_toast() {
        let mut app = quiet_app();
        in_app(&mut app);
        let read = inject(&mut app, "ui.review.badges", "app.badges");
        refuse(&mut app, &read, "stale_context");
        app.pump();
        // Read again under a new id (the write itself fails here: tests have no daemon).
        assert!(!app.pending.contains_key(&read));
        let write = inject(&mut app, "ui.loans.mark", "txs.loan_mark");
        refuse(&mut app, &write, "stale_context");
        app.pump();
        assert!(
            app.toasts.iter().all(|t| t.tone != ToastTone::Error),
            "{:?}",
            app.toasts.iter().map(|t| &t.title).collect::<Vec<_>>()
        );
        // A real failure still shows.
        let other = inject(&mut app, "ui.loans.mark", "txs.loan_mark");
        refuse(&mut app, &other, "validation");
        app.pump();
        assert!(app.toasts.iter().any(|t| t.tone == ToastTone::Error));
    }

    #[test]
    fn book_bound_writes_carry_the_open_book() {
        let scope = json!({"workspace_id": "w", "profile_id": "p"});
        let args = scope_args("ui.loans.mark", json!({"transaction_id": "t"}), &scope);
        assert_eq!(args["expected_scope"], scope);
        let args = scope_args("ui.attachments.add", json!({"transaction_id": "t"}), &scope);
        assert_eq!(args["expected_scope"], scope);
        // A caller's own binding wins; reads, the book switch and an unknown book get none.
        let own = json!({"workspace_id": "w0", "profile_id": "p0"});
        assert_eq!(
            scope_args(
                "ui.transfers.unpair",
                json!({"expected_scope": own}),
                &scope
            )["expected_scope"],
            own
        );
        assert!(
            scope_args("ui.transactions.list", json!({}), &scope)
                .get("expected_scope")
                .is_none()
        );
        assert!(
            scope_args("ui.profiles.switch", json!({"profile_id": "p"}), &scope)
                .get("expected_scope")
                .is_none()
        );
        // A request naming another book (the Books page editing an inactive book) is not bound to
        // the open one; naming the open book still is.
        assert!(
            scope_args("ui.profiles.update", json!({"profile_id": "p2"}), &scope)
                .get("expected_scope")
                .is_none()
        );
        assert_eq!(
            scope_args("ui.profiles.update", json!({"profile_id": "p"}), &scope)["expected_scope"],
            scope
        );
        let unknown = json!({"workspace_id": null, "profile_id": null});
        assert!(
            scope_args("ui.loans.mark", json!({}), &unknown)
                .get("expected_scope")
                .is_none()
        );
        // `send` applies it from the open book's profiles read.
        let mut app = quiet_app();
        app.reads.insert(
            "ui.profiles.snapshot".into(),
            Reply {
                kind: "ui.profiles.snapshot".into(),
                data: json!({"activeWorkspaceId": "w", "activeProfileId": "p"}),
                error: None,
                error_detail: Value::Null,
                elapsed: Duration::ZERO,
            },
        );
        assert_eq!(app.expected_scope(), scope);
    }

    #[test]
    fn history_never_reaches_a_guarded_page() {
        let mut app = quiet_app();
        in_app(&mut app);
        app.prefs.developer_tools = false;
        app.prefs.ai_features = false;
        for page in ["/logs", "/egress", "/activity", "/assistant"] {
            app.route = Route("/transactions".into());
            app.back = vec![Route(page.into())];
            app.go_back();
            assert_eq!(app.route.path(), "/overview", "Back to {page}");
            app.route = Route("/transactions".into());
            app.forward = vec![Route(page.into())];
            app.go_forward();
            assert_eq!(app.route.path(), "/overview", "Forward to {page}");
            app.route = Route("/transactions".into());
            app.navigate(page);
            assert_eq!(app.route.path(), "/overview", "navigate to {page}");
        }
        // With the switches on, the pages open.
        app.prefs.developer_tools = true;
        app.prefs.ai_features = true;
        app.navigate("/logs");
        assert_eq!(app.route.path(), "/logs");
        app.navigate("/assistant");
        app.go_back();
        assert_eq!(app.route.path(), "/logs");
    }
}
