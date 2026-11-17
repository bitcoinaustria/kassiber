//! components/kb/btcpay/BtcpaySetupPanel.tsx + btcpaySetupModel.ts: the BTCPay setup panel the
//! Add connection dialog shows for the `btcpay` source (`setupKind === "btcpay"`).
//!
//! Contract with the dialog (`wallets::add`):
//! - `reset` when the BTCPay setup step opens (a fresh draft, errors hidden).
//! - `panel` draws it; `label_field` / `footer` are the dialog's `renderConnectionLabelField()`
//!   and `renderSyncAfterCreate(t("add.btcpay.refreshAfter"))`, placed where the web places them.
//!   `savedInstances` = `app.read("ui.backends.options")["backends"]` filtered to kind "btcpay";
//!   `wallets` = `app.read("ui.wallets.list")["wallets"]` (the dialog loads both on open).
//! - `submit` on the dialog's primary button: shows errors (`showErrors`) and returns the
//!   `ui.connections.btcpay.create` args (`buildCreateArgs`, `connectionLabel` = `label`), or the
//!   message for the dialog's error line (`add.btcpay.issue.discover_first` when undiscovered).
//! - `created` with the create reply's `data`: the labels to sync (`walletsToRefresh`, wallet
//!   sources then mapped wallets) and the feedback toast.
//! - `poll` every frame (from `add::poll`): opens the authorize page a key-guide reply named
//!   (`openExternalUrl` needs a `Tx`).
//!
//! Discover / key guide (`ui.connections.btcpay.discover`, `.key_guide`) contact the BTCPay
//! server: egress, sent only from their buttons. Tags `"wallets.btcpay.…"`.

pub mod form;
pub mod model;

use std::collections::HashMap;

use rune::{Color, Cx, El, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply, ToastTone};
use crate::i18n::{has, t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use form::{
    Variant, badge, badge_secondary, button, error_line, input, notice, notice_with, select_menu,
    select_trigger, setup_field, sm_medium, xs,
};
use model::{
    ACTIONS, Choice, DEFAULT_PAYMENT_METHOD_ID, Instance, ManualRoute, Method, Plan, Store, Tone,
    WalletOption,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Saved,
    New,
}

pub struct State {
    pub mode: Mode,
    /// The user picked Saved / New (`userChoseInstanceMode`): saved instances that load later
    /// no longer switch the mode.
    pub user_chose_mode: bool,
    pub saved_instance: String,
    pub instance_label: TextEdit,
    pub server_url: TextEdit,
    pub api_key: TextEdit,
    /// "read_only" | "wallet_history".
    pub preset: &'static str,
    /// "all" | "single".
    pub store_scope: &'static str,
    pub plan: Option<Plan>,
    pub plan_error: Option<String>,
    pub authorize_error: Option<String>,
    pub choices: HashMap<String, Choice>,
    pub manual_store: TextEdit,
    pub manual_method: TextEdit,
    pub manual_action: String,
    pub manual_wallet: String,
    pub show_errors: bool,
    pub discover_pending: bool,
    /// The instance (its request arguments) each discovery in flight was for, oldest first.
    /// Kept across a fresh draft, so an earlier draft's answer is recognized.
    pub discovering: std::collections::VecDeque<String>,
    pub key_guide_pending: bool,
    /// An authorize URL to open on the next poll.
    pub open_url: Option<String>,
    /// The open native select (its trigger key).
    pub select: Option<String>,
    /// `<details>` "Create the key manually".
    pub manual_open: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            mode: Mode::New,
            user_chose_mode: false,
            saved_instance: String::new(),
            instance_label: TextEdit::new("btcpay"),
            server_url: TextEdit::new(""),
            api_key: TextEdit::new(""),
            preset: "read_only",
            store_scope: "all",
            plan: None,
            plan_error: None,
            authorize_error: None,
            choices: HashMap::new(),
            manual_store: TextEdit::new(""),
            manual_method: TextEdit::new(DEFAULT_PAYMENT_METHOD_ID),
            manual_action: "provenance_only".into(),
            manual_wallet: String::new(),
            show_errors: false,
            discover_pending: false,
            discovering: Default::default(),
            key_guide_pending: false,
            open_url: None,
            select: None,
            manual_open: false,
        }
    }
}

/// What the dialog does after `ui.connections.btcpay.create` succeeded.
pub struct Created {
    /// Wallet labels to `ui.wallets.sync` when "sync after create" is on (in this order).
    pub sync: Vec<String>,
    pub title: String,
    pub body: String,
    pub tone: ToastTone,
}

fn st(app: &App) -> &State {
    &app.wallets.add.btcpay
}

fn stm(app: &mut App) -> &mut State {
    &mut app.wallets.add.btcpay
}

/// A saved BTCPay instance (`BtcpaySavedInstance`).
pub struct Saved {
    pub name: String,
    pub display_name: String,
    pub is_default: bool,
}

/// `savedInstances`: the book's `btcpay` backends.
pub fn saved_instances(app: &App) -> Vec<Saved> {
    app.read("ui.backends.options")
        .and_then(|d| d.get("backends"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|b| b["kind"] == "btcpay")
                .map(|b| Saved {
                    name: b["name"].as_str().unwrap_or("").to_owned(),
                    display_name: b["display_name"].as_str().unwrap_or("").to_owned(),
                    is_default: b["is_default"].as_bool().unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// `wallets`: the book's wallets (`ui.wallets.list`).
pub fn wallet_options(app: &App) -> Vec<WalletOption> {
    app.read("ui.wallets.list")
        .and_then(|d| d.get("wallets"))
        .and_then(Value::as_array)
        .map(|a| a.iter().map(WalletOption::from_value).collect())
        .unwrap_or_default()
}

fn instance_display_name(s: &Saved) -> String {
    let label = if s.display_name.trim().is_empty() {
        s.name.clone()
    } else {
        s.display_name.trim().to_owned()
    };
    if label == s.name {
        label
    } else {
        format!("{label} ({})", s.name)
    }
}

fn default_saved(saved: &[Saved]) -> String {
    saved
        .iter()
        .find(|s| s.is_default)
        .or(saved.first())
        .map(|s| s.name.clone())
        .unwrap_or_default()
}

/// The mode shown: saved instances load asynchronously; prefer them once they appear unless
/// the user already chose (the web's effect on `savedInstances`).
pub fn mode(app: &App) -> Mode {
    let s = st(app);
    if s.user_chose_mode {
        return s.mode;
    }
    let saved = saved_instances(app);
    if !saved.is_empty() && s.server_url.text.is_empty() {
        Mode::Saved
    } else if saved.is_empty() {
        Mode::New
    } else {
        s.mode
    }
}

fn saved_choice(app: &App) -> String {
    let s = st(app);
    if s.saved_instance.is_empty() {
        default_saved(&saved_instances(app))
    } else {
        s.saved_instance.clone()
    }
}

/// `instanceIssue`.
fn instance_issue(app: &App) -> Option<&'static str> {
    let s = st(app);
    match mode(app) {
        Mode::Saved => saved_choice(app)
            .trim()
            .is_empty()
            .then_some("choose_instance"),
        Mode::New => {
            if s.instance_label.text.trim().is_empty() {
                Some("instance_name")
            } else if !model::looks_like_server_url(&s.server_url.text) {
                Some("server_url")
            } else if s.api_key.text.trim().is_empty() {
                Some("api_key")
            } else {
                None
            }
        }
    }
}

fn instance(app: &App) -> Option<Instance> {
    if instance_issue(app).is_some() {
        return None;
    }
    let s = st(app);
    Some(match mode(app) {
        Mode::Saved => Instance::Saved {
            backend: saved_choice(app).trim().to_owned(),
        },
        Mode::New => Instance::New {
            backend_label: s.instance_label.text.trim().to_owned(),
            server_url: s.server_url.text.trim().to_owned(),
            api_key: s.api_key.text.trim().to_owned(),
        },
    })
}

fn manual_route(s: &State) -> ManualRoute {
    ManualRoute {
        store_id: s.manual_store.text.clone(),
        payment_method_id: s.manual_method.text.clone(),
        action: s.manual_action.clone(),
        wallet: s.manual_wallet.clone(),
    }
}

/// (issue, create routes) of the current draft.
fn draft(app: &App, label: &str) -> (Option<&'static str>, Vec<Value>) {
    let s = st(app);
    let wallets = wallet_options(app);
    let Some(plan) = &s.plan else {
        return (Some("discover_first"), vec![]);
    };
    let entries = model::plan_routes(plan, &s.choices, &wallets);
    let manual = manual_route(s);
    let manual = plan.payment_methods.is_empty().then_some(&manual);
    (
        model::validate(Some(plan), &entries, manual, &wallets),
        model::build_create_routes(plan, &entries, manual, label),
    )
}

/// A fresh draft (the setup step opened).
pub fn reset(app: &mut App) {
    let discovering = std::mem::take(&mut app.wallets.add.btcpay.discovering);
    app.wallets.add.btcpay = State {
        discovering,
        ..State::default()
    };
}

/// The instance a discovery is for, as its request arguments.
fn instance_key(a: &App) -> Option<String> {
    instance(a).map(|i| Value::Object(i.args()).to_string())
}

fn reset_plan(a: &mut App) {
    let s = stm(a);
    s.plan = None;
    s.plan_error = None;
    s.choices.clear();
}

pub fn submit(app: &mut App, label: &str) -> Result<Value, String> {
    stm(app).show_errors = true;
    let issue_text = t("connections:add.btcpay.issue.discover_first");
    let Some(inst) = instance(app) else {
        return Err(issue_text);
    };
    let (issue, routes) = draft(app, label);
    if issue.is_some() {
        return Err(issue_text);
    }
    let Some(plan) = st(app).plan.as_ref() else {
        return Err(issue_text);
    };
    Ok(model::build_create_args(
        &inst,
        mode(app) == Mode::New,
        plan,
        &routes,
        label,
    ))
}

pub fn created(app: &mut App, data: &Value) -> Created {
    let label = data["wallet"]["label"].as_str().unwrap_or("").to_owned();
    let (_, routes) = draft(app, &label);
    let created: Vec<String> = ["wallet_sources", "payment_ledgers"]
        .iter()
        .flat_map(|k| data[*k].as_array().cloned().unwrap_or_default())
        .filter_map(|w| w["label"].as_str().map(str::to_owned))
        .collect();
    let (sources, mapped) = model::wallets_to_refresh(&routes, &created);
    let failure = data["provenance"]
        .as_array()
        .and_then(|a| a.iter().find(|r| r["status"] == "error"))
        .cloned();
    let active = routes.iter().filter(|r| r["action"] != "skip").count();
    let (body, tone) = match &failure {
        Some(f) => (
            tr(
                "connections:add.btcpay.provenancePartial",
                &[("message", f["message"].as_str().unwrap_or(""))],
            ),
            ToastTone::Warning,
        ),
        None => (
            tr(
                "connections:add.btcpay.accountSavedBody",
                &[
                    ("count", &active.to_string()),
                    ("wallets", &created.len().to_string()),
                ],
            ),
            ToastTone::Success,
        ),
    };
    if let (Some(f), Some(_)) = (&failure, data["wallet"]["id"].as_str()) {
        // `recordBtcpayProvenanceIncomplete`: waits in the notification centre.
        let clock = app.clock;
        app.shell.note(
            &label,
            f["message"].as_str().map(str::to_owned),
            ToastTone::Warning,
            clock,
        );
    }
    let mut sync = sources;
    sync.extend(mapped);
    Created {
        sync,
        title: t("connections:add.btcpay.accountSavedTitle"),
        body,
        tone,
    }
}

fn run_discovery(a: &mut App) {
    let Some(inst) = instance(a) else { return };
    if st(a).discover_pending {
        return;
    }
    stm(a).plan_error = None;
    let args = Value::Object(inst.args());
    let key = args.to_string();
    if a.send(
        "ui.connections.btcpay.discover",
        args,
        "wallets.btcpay.discover",
    )
    .is_some()
    {
        let s = stm(a);
        s.discover_pending = true;
        s.discovering.push_back(key);
    }
}

fn open_authorize(a: &mut App) {
    if st(a).key_guide_pending {
        return;
    }
    stm(a).authorize_error = None;
    let s = st(a);
    let args = match mode(a) {
        Mode::Saved => {
            json!({"backend": saved_choice(a), "preset": s.preset, "store_scope": s.store_scope})
        }
        Mode::New => {
            json!({"server_url": s.server_url.text.trim(), "preset": s.preset, "store_scope": s.store_scope})
        }
    };
    if a.send(
        "ui.connections.btcpay.key_guide",
        args,
        "wallets.btcpay.key_guide",
    )
    .is_some()
    {
        stm(a).key_guide_pending = true;
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "wallets.btcpay.discover" => {
            let asked = stm(app).discovering.pop_front();
            let pending = !st(app).discovering.is_empty();
            stm(app).discover_pending = pending;
            // The server, key or saved instance changed since it was asked (or the draft
            // started over): this plan is not the current instance's.
            if asked.is_none() || asked != instance_key(app) {
                return;
            }
            let parsed: Option<Plan> = if r.error.is_none() && !r.data.is_null() {
                serde_json::from_value(r.data.clone()).ok()
            } else {
                None
            };
            match (&r.error, parsed) {
                (None, Some(plan)) => {
                    let wallets = wallet_options(app);
                    let choices = model::default_route_choices(&plan, &wallets);
                    let s = stm(app);
                    s.choices = choices;
                    s.plan = Some(plan);
                }
                (None, None) if r.data.is_null() => {
                    let s = stm(app);
                    s.plan = None;
                    s.choices.clear();
                }
                (e, _) => {
                    let s = stm(app);
                    s.plan = None;
                    s.plan_error = Some(
                        e.clone()
                            .filter(|m| !m.is_empty())
                            .unwrap_or_else(|| t("connections:add.btcpay.discoveryFailed")),
                    );
                }
            }
        }
        "wallets.btcpay.key_guide" => {
            let s = stm(app);
            s.key_guide_pending = false;
            match (
                &r.error,
                r.data["authorize_url"].as_str().filter(|u| !u.is_empty()),
            ) {
                (None, Some(url)) => s.open_url = Some(url.to_owned()),
                (Some(e), _) if !e.is_empty() => s.authorize_error = Some(e.clone()),
                _ => s.authorize_error = Some(t("connections:add.btcpay.authorizeFailed")),
            }
        }
        _ => {}
    }
}

/// Opens a pending authorize URL (the system browser; not a request Kassiber makes).
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    match stm(app).open_url.take() {
        Some(url) => {
            if tx.open_url(&url).is_err() {
                stm(app).authorize_error = Some(t("connections:add.btcpay.authorizeFailed"));
            }
            true
        }
        None => false,
    }
}

// ---- view ---------------------------------------------------------------------------------

fn close_select(a: &mut App) {
    stm(a).select = None;
}

fn toggle_select(a: &mut App, key: &str) {
    let s = stm(a);
    s.select = if s.select.as_deref() == Some(key) {
        None
    } else {
        Some(key.to_owned())
    };
}

/// A select trigger and, when open, its menu.
#[allow(clippy::too_many_arguments)]
fn select(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    name: &str,
    current: &str,
    placeholder: Option<&str>,
    options: Vec<form::Opt>,
    enabled: bool,
    choose: impl Fn(&mut App, &str) + Clone + 'static,
) -> El {
    let shown = options.iter().find(|o| o.0 == current).map(|o| o.1.clone());
    let muted = shown.is_none();
    let shown = shown.or(placeholder.map(str::to_owned)).unwrap_or_default();
    let k = key.to_owned();
    let mut trig = select_trigger(tk, key, name, &shown, enabled, muted);
    if enabled {
        let open = st(app).select.as_deref() == Some(key);
        trig = crate::ui::popup::trigger(
            trig,
            crate::ui::popup::Kind::Listbox,
            open,
            move |a: &mut App| toggle_select(a, &k),
        );
    }
    if enabled && st(app).select.as_deref() == Some(key) {
        let w = cx.layout_size(key).map(|[w, _]| w).unwrap_or(tk.r(16.0));
        let mut opts = options;
        if let Some(p) = placeholder
            && !opts.iter().any(|o| o.0.is_empty())
        {
            opts.insert(0, (String::new(), p.to_owned(), false, None));
        }
        select_menu(tk, layer, key, w, opts, current, close_select, choose);
    }
    trig
}

/// `t(key, { defaultValue })`: the bundle's text, else the daemon's own.
fn tdef(key: &str, args: &[(&str, &str)], default: &str) -> String {
    if has(key) || has(&format!("{key}_one")) {
        tr(key, args)
    } else {
        default.to_owned()
    }
}

/// `Intl.DateTimeFormat({dateStyle: "medium", timeStyle: "short"})`, in UTC.
pub fn format_medium(ts: &str) -> String {
    let Some(secs) = crate::format::parse_ts(ts) else {
        return ts.to_owned();
    };
    let day = crate::format::Day::from_epoch_days((secs / 86_400.0).floor() as i64);
    let tod = secs.rem_euclid(86_400.0) as i64;
    let (h, m) = (tod / 3600, (tod % 3600) / 60);
    match crate::i18n::lang() {
        crate::i18n::Lang::De => format!("{:02}.{:02}.{}, {:02}:{:02}", day.d, day.m, day.y, h, m),
        crate::i18n::Lang::En => {
            let (h12, ap) = match h {
                0 => (12, "AM"),
                1..=11 => (h, "AM"),
                12 => (12, "PM"),
                _ => (h - 12, "PM"),
            };
            format!("{}, {}:{:02} {ap}", day.long(), h12, m)
        }
    }
}

fn store_freshness_line(tk: &Tk, store: &Store) -> Option<El> {
    let f = model::store_freshness(store.sync_state.as_ref())?;
    let synced_at = store
        .sync_state
        .as_ref()
        .and_then(|s| s.last_success_at.clone());
    let txt = match f {
        "never" => t("connections:add.btcpay.storeSync.never"),
        "failed" => t("connections:add.btcpay.storeSync.failed"),
        "stale" => t("connections:add.btcpay.storeSync.stale"),
        _ => tr(
            "connections:add.btcpay.storeSync.at",
            &[(
                "when",
                &synced_at.as_deref().map(format_medium).unwrap_or_default(),
            )],
        ),
    };
    let ink = if f == "fresh" {
        tk.muted_fg
    } else {
        form::amber_ink(tk)
    };
    let mut e = text(txt)
        .font(tk.px(11.0 * tk.rem / 16.0, 400.0))
        .color(ink);
    if let Some(s) = synced_at {
        e = e.help(format_medium(&s));
    }
    Some(e)
}

const STORE_CAPABILITIES: [&str; 4] = ["invoices", "payouts", "payment_requests", "wallet_history"];

fn preset_permissions(p: &str) -> &'static str {
    if p == "read_only" {
        "btcpay.store.canviewstoresettings, btcpay.store.canviewwallet"
    } else {
        "btcpay.store.canmodifystoresettings"
    }
}

/// A selectable card (`role="radio"`): `rounded-md border p-3 text-left`, chosen
/// `border-primary bg-primary/5`.
fn radio_card(
    tk: &Tk,
    key: &str,
    name: &str,
    checked: bool,
    pad: (f32, f32),
    children: Vec<El>,
) -> El {
    let mut c = col()
        .key(key.to_owned())
        .w_full()
        .px(pad.0)
        .py(pad.1)
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(
            1.0,
            if checked {
                tk.primary
            } else {
                tk.border.alpha(tk.border.a * 0.7)
            },
        )
        .border_layout()
        .fill(if checked {
            tk.primary.alpha(0.05)
        } else {
            Color::TRANSPARENT
        })
        .role("radio")
        .checked(checked)
        .label(name.to_owned())
        .focusable(true);
    if !checked {
        c = c.hover(tk.muted.alpha(0.4));
    }
    for ch in children {
        c = c.child(ch);
    }
    c
}

/// A bordered box (`rounded-md border border-border/70 p-3`), `gap` between children.
fn boxed(tk: &Tk, gap: f32, muted_fill: bool) -> El {
    let b = col()
        .w_full()
        .min_w(0.0)
        .padding(tk.r(0.75))
        .gap(gap)
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout();
    if muted_fill {
        b.fill(tk.muted.alpha(0.2))
    } else {
        b
    }
}

fn mono11(tk: &Tk, s: impl Into<String>, color: Color) -> El {
    text(s.into())
        .font(tk.px(11.0 * tk.rem / 16.0, 400.0).mono())
        .color(color)
}

#[allow(clippy::too_many_arguments)]
fn render_method(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    plan: &Plan,
    entries: &[model::Entry],
    idx: usize,
    m: &Method,
    wide: bool,
) -> El {
    let key = model::route_key(&m.store_id, &m.payment_method_id);
    let wallets = wallet_options(app);
    let entry = entries.iter().find(|e| e.key == key);
    let choice = entry
        .map(|e| e.choice.clone())
        .unwrap_or_else(|| Choice::new("skip", ""));
    let store = model::store_name(Some(plan), &m.store_id);
    let wallet_opts =
        model::wallet_options_for_method(m.chain.as_deref(), &m.payment_method_id, &wallets);
    let rail = model::rail_kind(&m.rail, m.chain.as_deref(), m.bitcoin_asset);
    let mut shared: Vec<String> = Vec::new();
    for s in &m.shared_with {
        let n = model::store_name(Some(plan), &s.store_id);
        if !shared.contains(&n) {
            shared.push(n);
        }
    }
    let rec = m.recommendation.as_ref();
    let reason_text = rec
        .and_then(|r| r.reason.as_deref())
        .filter(|r| *r != "already_configured")
        .map(|r| {
            tdef(
                &format!("connections:add.btcpay.reason.{r}"),
                &[],
                rec.and_then(|x| x.reason_text.as_deref()).unwrap_or(""),
            )
        });

    // Left: label, badges, settlement, recommendation, notices.
    let mut head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(text(m.label.clone()).font(tk.tsm(500.0)).color(tk.fg))
        .child(badge(
            tk,
            &t(&format!("connections:add.btcpay.rail.{rail}")),
            None,
            false,
            false,
        ))
        .child(badge(tk, &m.payment_method_id, None, true, false));
    if !m.existing_routes.is_empty() {
        head = head.child(badge_secondary(
            tk,
            &t("connections:add.btcpay.notice.alreadyConfigured"),
        ));
    }
    let mut left = col().min_w(0.0).gap(tk.r(0.375)).child(head);
    if let Some(sett) = &m.settlement {
        left = left.child(xs(
            tk,
            tdef(
                &format!("connections:add.btcpay.settlement.{sett}"),
                &[],
                m.settlement_hint.as_deref().unwrap_or(""),
            ),
            tk.muted_fg,
        ));
    }
    if let Some(rt) = reason_text {
        let lead = format!("{}:", t("connections:add.btcpay.recommended"));
        let full = format!("{lead} {rt}");
        left = left.child(
            rune::rich(full, vec![rune::Span::new(0, lead.len()).weight(500.0)])
                .font(tk.txs(400.0))
                .color(tk.fg),
        );
    }
    if let Some(o) = &m.owned_by {
        left = left.child(notice(
            tk,
            Tone::Good,
            tr(
                "connections:add.btcpay.notice.recognised",
                &[("wallet", &o.wallet)],
            ),
        ));
    }
    if !shared.is_empty() {
        left = left.child(notice(
            tk,
            Tone::Neutral,
            tr(
                "connections:add.btcpay.notice.sharedWith",
                &[("stores", &shared.join(", "))],
            ),
        ));
    }
    if !m.configured_via.is_empty() {
        left = left.child(notice(
            tk,
            Tone::Neutral,
            tr(
                "connections:add.btcpay.notice.configuredVia",
                &[("names", &m.configured_via.join(", "))],
            ),
        ));
    }
    if !m.lightning_wallets.is_empty() {
        let names = m
            .lightning_wallets
            .iter()
            .map(|w| w.wallet.clone())
            .collect::<Vec<_>>()
            .join(", ");
        left = left.child(notice(
            tk,
            Tone::Neutral,
            tr(
                "connections:add.btcpay.notice.lightningLinks",
                &[("wallets", &names)],
            ),
        ));
    }

    // Right: the action select, ledger notes, the wallet select.
    let action_opts: Vec<form::Opt> = ACTIONS
        .iter()
        .map(|a| {
            let available = model::action_available(m, a);
            let title = if available {
                None
            } else {
                m.actions.as_ref().and_then(|x| x.get(*a)).and_then(|x| {
                    x.reason.as_ref().map(|r| {
                        tdef(
                            &format!("connections:add.btcpay.reason.{r}"),
                            &[],
                            x.reason_text.as_deref().unwrap_or(""),
                        )
                    })
                })
            };
            (
                a.to_string(),
                t(&format!("connections:add.btcpay.action.{a}")),
                available,
                title,
            )
        })
        .collect();
    let k2 = key.clone();
    let fallback = choice.clone();
    let action_name = tr(
        "connections:add.btcpay.actionLabel",
        &[("method", &m.payment_method_id), ("store", &store)],
    );
    let mut right = col().min_w(0.0).gap(tk.r(0.5)).child(select(
        app,
        cx,
        tk,
        layer,
        &format!("btcpay-action-{idx}"),
        &action_name,
        &choice.action,
        None,
        action_opts,
        true,
        move |a: &mut App, v: &str| {
            let s = stm(a);
            let mut c = s
                .choices
                .get(&k2)
                .cloned()
                .unwrap_or_else(|| fallback.clone());
            c.action = v.to_owned();
            s.choices.insert(k2.clone(), c);
        },
    ));
    // Lightning and LNURL of one store (or one plugin rail) share a ledger: explain it once.
    if choice.action == "payment_ledger" {
        let group = |e: &model::Entry| {
            format!(
                "{}\u{0}{}",
                e.method.store_id,
                e.method
                    .ledger_group
                    .clone()
                    .unwrap_or_else(|| e.method.payment_method_id.clone())
            )
        };
        let mine = format!(
            "{}\u{0}{}",
            m.store_id,
            m.ledger_group
                .clone()
                .unwrap_or_else(|| m.payment_method_id.clone())
        );
        let members: Vec<&model::Entry> = entries
            .iter()
            .filter(|e| e.choice.action == "payment_ledger" && group(e) == mine)
            .collect();
        if let Some(first) = members.first() {
            if first.key == key {
                let labels: Vec<String> = members.iter().map(|e| e.method.label.clone()).collect();
                let lead = if labels.len() > 1 {
                    format!(
                        "{} ",
                        tr(
                            "connections:add.btcpay.notice.ledgerShared",
                            &[("methods", &list_format(&labels))]
                        )
                    )
                } else {
                    String::new()
                };
                right = right.child(xs(
                    tk,
                    format!("{lead}{}", t("connections:add.btcpay.notice.ledgerScope")),
                    tk.muted_fg,
                ));
            } else {
                right = right.child(xs(
                    tk,
                    tr(
                        "connections:add.btcpay.notice.ledgerSameAs",
                        &[("method", &first.method.label)],
                    ),
                    tk.muted_fg,
                ));
            }
        }
    }
    if choice.action == "existing_wallet" {
        let opts: Vec<form::Opt> = wallet_opts
            .iter()
            .map(|w| {
                (
                    w.label.clone(),
                    match &w.chain {
                        Some(c) => format!("{} ({c})", w.label),
                        None => w.label.clone(),
                    },
                    true,
                    None,
                )
            })
            .collect();
        let ph = if wallet_opts.is_empty() {
            t("connections:add.btcpay.noWallets")
        } else {
            t("connections:add.btcpay.selectWallet")
        };
        let k3 = key.clone();
        let fallback = choice.clone();
        let name = tr(
            "connections:add.btcpay.walletLabel",
            &[("method", &m.payment_method_id), ("store", &store)],
        );
        right = right.child(select(
            app,
            cx,
            tk,
            layer,
            &format!("btcpay-wallet-{idx}"),
            &name,
            &choice.wallet,
            Some(&ph),
            opts,
            !wallet_opts.is_empty(),
            move |a: &mut App, v: &str| {
                let s = stm(a);
                let mut c = s
                    .choices
                    .get(&k3)
                    .cloned()
                    .unwrap_or_else(|| fallback.clone());
                c.wallet = v.to_owned();
                s.choices.insert(k3.clone(), c);
            },
        ));
    }
    let body: El = if wide {
        row()
            .w_full()
            .gap(tk.r(0.75))
            .items_start()
            .child(left.grow(1.0))
            .child(right.grow(0.75).min_w(220.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(left.w_full())
            .child(right.w_full())
    };
    col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border.alpha(tk.border.a * 0.6))
        .border_layout()
        .fill(tk.background.alpha(0.7))
        .opacity(if choice.action == "skip" { 0.7 } else { 1.0 })
        .child(body)
}

/// `Intl.ListFormat(conjunction)`.
fn list_format(items: &[String]) -> String {
    let de = crate::i18n::lang() == crate::i18n::Lang::De;
    let and = if de { "und" } else { "and" };
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        2 => format!("{} {and} {}", items[0], items[1]),
        n => format!(
            "{}{} {and} {}",
            items[..n - 1].join(", "),
            if de { "" } else { "," },
            items[n - 1]
        ),
    }
}

/// `BtcpaySetupPanel`. "Use CSV instead" calls `wallets::add::select_source_for_setup(app,
/// "btcpay-csv")`.
pub fn panel(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    label_field: El,
    footer: El,
) -> El {
    let s = st(app);
    let saved = saved_instances(app);
    let mode = mode(app);
    let issue_inst = instance_issue(app);
    let inst = instance(app);
    let (issue, _) = draft(app, "");
    let wallets = wallet_options(app);
    let inst_err = |target: &str| -> Option<String> {
        (s.show_errors && issue_inst == Some(target)).then(|| {
            t(match target {
                "choose_instance" => "connections:add.btcpay.errorChooseInstance",
                "instance_name" => "connections:add.btcpay.errorInstanceName",
                "server_url" => "connections:add.btcpay.errorServerUrl",
                _ => "connections:add.btcpay.errorApiKey",
            })
        })
    };

    let mut out = col().key("btcpay-setup").w_full().min_w(0.0).gap(tk.r(1.0));
    out = out.child(
        boxed(tk, tk.r(0.25), true)
            .child(sm_medium(tk, t("connections:add.btcpay.introTitle")))
            .child(xs(tk, t("connections:add.btcpay.introBody"), tk.muted_fg)),
    );
    out = out.child(label_field);

    let saved_btn = button(
        tk,
        "btcpay-mode-saved",
        if mode == Mode::Saved {
            Variant::Secondary
        } else {
            Variant::Outline
        },
        false,
        None,
        &t("connections:add.btcpay.savedInstance"),
        !saved.is_empty(),
    );
    let saved_btn = if saved.is_empty() {
        saved_btn
    } else {
        saved_btn.on_press(|a: &mut App| {
            let def = default_saved(&saved_instances(a));
            let s = stm(a);
            s.user_chose_mode = true;
            s.mode = Mode::Saved;
            if s.saved_instance.is_empty() {
                s.saved_instance = def;
            }
            reset_plan(a);
        })
    };
    let new_btn = button(
        tk,
        "btcpay-mode-new",
        if mode == Mode::New {
            Variant::Secondary
        } else {
            Variant::Outline
        },
        false,
        None,
        &t("connections:add.btcpay.newInstance"),
        true,
    )
    .on_press(|a: &mut App| {
        let s = stm(a);
        s.user_chose_mode = true;
        s.mode = Mode::New;
        reset_plan(a);
    });
    out = out.child(form::grid2(
        tk.r(0.5),
        vec![saved_btn.w_full(), new_btn.w_full()],
    ));

    if mode == Mode::Saved {
        let opts: Vec<form::Opt> = saved
            .iter()
            .map(|x| (x.name.clone(), instance_display_name(x), true, None))
            .collect();
        let sel = select(
            app,
            cx,
            tk,
            layer,
            "btcpay-instance",
            &t("connections:add.btcpay.instance"),
            &saved_choice(app),
            Some(&t("connections:add.btcpay.selectInstance")),
            opts,
            true,
            |a: &mut App, v: &str| {
                stm(a).saved_instance = v.to_owned();
                reset_plan(a);
            },
        );
        out = out.child(setup_field(
            tk,
            &t("connections:add.btcpay.instance"),
            sel,
            inst_err("choose_instance"),
            None,
        ));
    } else {
        out = out.child(setup_field(
            tk,
            &t("connections:add.btcpay.instanceName"),
            input(
                tk,
                "btcpay-instance-label",
                &s.instance_label,
                |a: &mut App| &mut a.wallets.add.btcpay.instance_label,
                &t("connections:add.btcpay.instanceName"),
                None,
                false,
                reset_plan,
            ),
            inst_err("instance_name"),
            Some(t("connections:add.btcpay.instanceNameHelper")),
        ));
        out = out.child(setup_field(
            tk,
            &t("connections:add.btcpay.serverUrl"),
            input(
                tk,
                "btcpay-url",
                &s.server_url,
                |a: &mut App| &mut a.wallets.add.btcpay.server_url,
                &t("connections:add.btcpay.serverUrl"),
                Some(&t("connections:add.btcpay.serverUrlPlaceholder")),
                false,
                reset_plan,
            ),
            inst_err("server_url"),
            Some(t("connections:add.btcpay.serverUrlHelper")),
        ));
        out = out.child(key_step(app, tk));
        out = out.child(setup_field(
            tk,
            &t("connections:add.btcpay.apiKey"),
            input(
                tk,
                "btcpay-api-key",
                &s.api_key,
                |a: &mut App| &mut a.wallets.add.btcpay.api_key,
                &t("connections:add.btcpay.apiKey"),
                None,
                true,
                reset_plan,
            ),
            inst_err("api_key"),
            Some(t("connections:add.btcpay.apiKeyHelper")),
        ));
    }

    // 2 · Check the key.
    let pending = s.discover_pending;
    let check_label = if pending {
        t("connections:add.btcpay.checking")
    } else {
        t("connections:add.btcpay.checkKey")
    };
    let mut check = button(
        tk,
        "btcpay-discover",
        if s.plan.is_some() {
            Variant::Outline
        } else {
            Variant::Secondary
        },
        true,
        pending.then_some((&I::LOADER_CIRCLE, tk.r(0.875), true)),
        &check_label,
        inst.is_some() && !pending,
    );
    if inst.is_some() && !pending {
        check = check.on_press(run_discovery);
    }
    let mut check_row = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(check);
    if let Some(p) = &s.plan {
        check_row = check_row.child(xs(
            tk,
            tc(
                "connections:add.btcpay.storesFound",
                p.stores.len(),
                &[("methods", &p.payment_methods.len().to_string())],
            ),
            tk.muted_fg,
        ));
    }
    let mut discover = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(sm_medium(tk, t("connections:add.btcpay.discoverStepTitle")))
        .child(check_row);
    if let Some(e) = &s.plan_error {
        discover = discover.child(error_line(tk, e.clone(), true));
    }
    if s.show_errors && issue == Some("discover_first") && s.plan_error.is_none() {
        discover = discover.child(error_line(
            tk,
            t("connections:add.btcpay.issue.discover_first"),
            false,
        ));
    }
    out = out.child(discover);

    if let Some(plan) = &s.plan {
        out = out.child(plan_view(app, cx, tk, layer, plan, issue, &wallets));
    }

    out = out.child(
        row().w_full().justify_end().child(
            button(
                tk,
                "btcpay-use-csv",
                Variant::Ghost,
                true,
                None,
                &t("connections:add.btcpay.manualCsvAlternative"),
                true,
            )
            .on_press(|a: &mut App| crate::wallets::add::select_source_for_setup(a, "btcpay-csv")),
        ),
    );
    out.child(footer)
}

/// "1 · Create an API key": presets, store scope, the authorize link and the manual steps.
fn key_step(app: &App, tk: &Tk) -> El {
    let s = st(app);
    let can_link = match mode(app) {
        Mode::Saved => !saved_choice(app).is_empty(),
        Mode::New => model::looks_like_server_url(&s.server_url.text),
    };
    let title = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(&I::KEY_ROUND, tk.r(1.0), tk.fg))
        .child(sm_medium(tk, t("connections:add.btcpay.keyStepTitle")));
    let head = col().w_full().gap(tk.r(0.25)).child(title).child(xs(
        tk,
        t("connections:add.btcpay.keyStepBody"),
        tk.muted_fg,
    ));
    let h = |s: &str| rune::color::hex(s).expect("hex");
    let presets: Vec<El> = ["read_only", "wallet_history"]
        .into_iter()
        .map(|p: &'static str| {
            let glyph = if p == "read_only" {
                ic(&I::SHIELD_CHECK, tk.r(1.0), h("#059669"))
            } else {
                ic(&I::SHIELD_ALERT, tk.r(1.0), h("#d97706"))
            };
            let title = t(&format!("connections:add.btcpay.presets.{p}.title"));
            radio_card(
                tk,
                &format!("btcpay-preset-{p}"),
                &title,
                s.preset == p,
                (tk.r(0.75), tk.r(0.75)),
                vec![
                    row()
                        .gap(tk.r(0.375))
                        .center_y()
                        .child(glyph)
                        .child(sm_medium(tk, title.clone())),
                    xs(
                        tk,
                        t(&format!("connections:add.btcpay.presets.{p}.body")),
                        tk.muted_fg,
                    ),
                    mono11(tk, preset_permissions(p), tk.muted_fg).wrap(),
                ],
            )
            .on_press(move |a: &mut App| {
                let s = stm(a);
                s.preset = p;
                s.store_scope = if p == "read_only" { "all" } else { "single" };
            })
        })
        .collect();
    let scopes: Vec<El> = ["all", "single"]
        .into_iter()
        .map(|sc: &'static str| {
            let title = t(&format!("connections:add.btcpay.scope.{sc}.title"));
            radio_card(
                tk,
                &format!("btcpay-scope-{sc}"),
                &title,
                s.store_scope == sc,
                (tk.r(0.75), tk.r(0.5)),
                vec![
                    text(title.clone()).font(tk.txs(500.0)).color(tk.fg).wrap(),
                    xs(
                        tk,
                        t(&format!("connections:add.btcpay.scope.{sc}.body")),
                        tk.muted_fg,
                    ),
                ],
            )
            .on_press(move |a: &mut App| stm(a).store_scope = sc)
        })
        .collect();
    let link_pending = s.key_guide_pending;
    let mut open = button(
        tk,
        "btcpay-authorize",
        Variant::Outline,
        true,
        Some(if link_pending {
            (&I::LOADER_CIRCLE, tk.r(0.875), true)
        } else {
            (&I::EXTERNAL_LINK, tk.r(0.875), false)
        }),
        &if link_pending {
            t("connections:add.btcpay.openingAuthorize")
        } else {
            t("connections:add.btcpay.openAuthorize")
        },
        can_link && !link_pending,
    );
    if can_link && !link_pending {
        open = open.on_press(open_authorize);
    }
    let hint = if can_link {
        if s.store_scope == "all" {
            t("connections:add.btcpay.authorizeHintAll")
        } else {
            t("connections:add.btcpay.authorizeHintSingle")
        }
    } else {
        t("connections:add.btcpay.authorizeNeedsUrl")
    };
    let mut b = boxed(tk, tk.r(0.75), false)
        .child(head)
        .child(
            form::grid2(tk.r(0.5), presets)
                .role("radiogroup")
                .label(t("connections:add.btcpay.keyStepTitle")),
        )
        .child(
            col()
                .w_full()
                .gap(tk.r(0.375))
                .child(
                    text(t("connections:add.btcpay.scope.label"))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(
                    form::grid2(tk.r(0.5), scopes)
                        .role("radiogroup")
                        .label(t("connections:add.btcpay.scope.label")),
                ),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .flex_wrap()
                .child(open)
                .child(xs(tk, hint, tk.muted_fg)),
        );
    if let Some(e) = &s.authorize_error {
        b = b.child(error_line(tk, e.clone(), false));
    }
    // `<details>`: the summary with the browser's disclosure triangle, then the steps.
    let open_d = s.manual_open;
    let summary = row()
        .key("btcpay-manual-toggle")
        .gap(tk.r(0.375))
        .center_y()
        .role("button")
        .expanded(open_d)
        .label(t("connections:add.btcpay.manualToggle"))
        .focusable(true)
        .on_press(|a: &mut App| {
            let s = stm(a);
            s.manual_open = !s.manual_open;
        })
        .child(ic(
            if open_d {
                &I::CHEVRON_DOWN
            } else {
                &I::CHEVRON_RIGHT
            },
            tk.r(0.625),
            tk.muted_fg,
        ))
        .child(
            text(t("connections:add.btcpay.manualToggle"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    let mut details = col().w_full().child(summary);
    if open_d {
        let steps = [
            t("connections:add.btcpay.manualSteps.signIn"),
            t("connections:add.btcpay.manualSteps.openKeys"),
            tr(
                "connections:add.btcpay.manualSteps.permissions",
                &[("permissions", preset_permissions(s.preset))],
            ),
            if s.store_scope == "all" {
                t("connections:add.btcpay.manualSteps.storesAll")
            } else {
                t("connections:add.btcpay.manualSteps.storesSingle")
            },
            t("connections:add.btcpay.manualSteps.copy"),
        ];
        let mut ol = col()
            .w_full()
            .mt(tk.r(0.5))
            .pl(tk.r(0.25))
            .gap(tk.r(0.25))
            .role("list");
        for (i, step) in steps.into_iter().enumerate() {
            ol = ol.child(form::list_item(
                tk,
                &format!("{}.", i + 1),
                xs(tk, step, tk.muted_fg),
            ));
        }
        details = details.child(ol);
    }
    b.child(details)
}

#[allow(clippy::too_many_arguments)]
fn plan_view(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    plan: &Plan,
    issue: Option<&'static str>,
    wallets: &[WalletOption],
) -> El {
    let s = st(app);
    let key_info = plan.api_key.clone().unwrap_or_default();
    let risk = if key_info.risk.is_empty() {
        "unknown".to_owned()
    } else {
        key_info.risk.clone()
    };
    let transport = plan.server.as_ref().map(|sv| {
        if sv.tor {
            "tor"
        } else if sv.loopback {
            "loopback"
        } else if sv.transport.as_deref() == Some("https") {
            "https"
        } else {
            "http"
        }
    });
    let mut out = col().w_full().gap(tk.r(0.75));

    // Server and key cards.
    let version = plan
        .server
        .as_ref()
        .and_then(|x| x.version.clone())
        .filter(|v| !v.is_empty());
    let mut server_badges = row().gap(tk.r(0.375)).flex_wrap();
    match plan.server.as_ref().and_then(|x| x.fully_synced) {
        Some(false) => {
            server_badges = server_badges.child(badge(
                tk,
                &t("connections:add.btcpay.server.notSynced"),
                Some(Tone::Warning),
                false,
                false,
            ))
        }
        Some(true) => {
            server_badges = server_badges.child(badge(
                tk,
                &t("connections:add.btcpay.server.synced"),
                None,
                false,
                false,
            ))
        }
        None => {}
    }
    if let Some(tp) = transport {
        server_badges = server_badges.child(badge(
            tk,
            &t(&format!("connections:add.btcpay.server.transport.{tp}")),
            (tp == "http").then_some(Tone::Danger),
            false,
            false,
        ));
    }
    let server_card = boxed(tk, tk.r(0.25), false)
        .child(sm_medium(
            tk,
            match &version {
                Some(v) => tr("connections:add.btcpay.server.version", &[("version", v)]),
                None => t("connections:add.btcpay.server.versionUnknown"),
            },
        ))
        .child(server_badges);
    let scope_text = if key_info.scope == "selected_stores" {
        tc(
            "connections:add.btcpay.key.scope.selected_stores",
            key_info.store_ids.as_ref().map_or(0, Vec::len),
            &[],
        )
    } else {
        let sc = if key_info.scope.is_empty() {
            "unknown"
        } else {
            key_info.scope.as_str()
        };
        tdef(
            &format!("connections:add.btcpay.key.scope.{sc}"),
            &[],
            &key_info.scope,
        )
    };
    let mut key_card = boxed(tk, tk.r(0.25), false)
        .child(sm_medium(
            tk,
            match key_info.label.as_deref().filter(|l| !l.is_empty()) {
                Some(l) => tr("connections:add.btcpay.key.labelled", &[("label", l)]),
                None => t("connections:add.btcpay.key.title"),
            },
        ))
        .child(
            row()
                .gap(tk.r(0.375))
                .flex_wrap()
                .child(badge(
                    tk,
                    &tdef(
                        &format!("connections:add.btcpay.key.risk.{risk}"),
                        &[],
                        &risk,
                    ),
                    Some(model::key_risk_tone(Some(&risk))),
                    false,
                    false,
                ))
                .child(badge(tk, &scope_text, None, false, false)),
        );
    if let Some(ex) = key_info
        .excess_permissions
        .as_ref()
        .filter(|e| !e.is_empty())
    {
        key_card = key_card.child(xs(
            tk,
            tr(
                "connections:add.btcpay.key.excess",
                &[("permissions", &ex.join(", "))],
            ),
            tk.muted_fg,
        ));
    }
    out = out.child(form::grid2(
        tk.r(0.5),
        vec![server_card.h_full(), key_card.h_full()],
    ));

    if key_info.known && risk == "read_only" {
        let ink = form::tone_colors(tk, Tone::Neutral).2;
        let mut parts = vec![xs(
            tk,
            t("connections:add.btcpay.key.readOnlyExplainer"),
            ink,
        )];
        if let Some(url) = plan
            .key_upgrade
            .as_ref()
            .and_then(|k| k.authorize_url.clone())
        {
            parts.push(
                form::link_button(
                    tk,
                    "btcpay-upgrade",
                    &I::EXTERNAL_LINK,
                    &t("connections:add.btcpay.key.upgrade"),
                    ink,
                )
                .on_press(move |a: &mut App| stm(a).open_url = Some(url.clone())),
            );
        }
        out = out.child(notice_with(tk, Tone::Neutral, parts));
    }
    if !plan.sibling_backends.is_empty() {
        out = out.child(notice(
            tk,
            Tone::Neutral,
            tr(
                "connections:add.btcpay.key.siblings",
                &[("names", &plan.sibling_backends.join(", "))],
            ),
        ));
    }
    for w in &plan.warnings {
        let store = w
            .store_id
            .as_deref()
            .map(|id| model::store_name(Some(plan), id))
            .unwrap_or_default();
        let count = w.store_ids.as_ref().map_or(1, Vec::len);
        let key = format!("connections:add.btcpay.warning.{}", w.code);
        let msg = if has(&key) || has(&format!("{key}_one")) {
            tc(&key, count, &[("store", &store)])
        } else {
            w.message.clone()
        };
        out = out.child(notice(tk, model::warning_tone(&w.code), msg));
    }
    out = out.child(
        col()
            .w_full()
            .gap(tk.r(0.25))
            .child(sm_medium(tk, t("connections:add.btcpay.storesTitle")))
            .child(xs(tk, t("connections:add.btcpay.storesBody"), tk.muted_fg)),
    );
    out = out.child(notice(
        tk,
        Tone::Neutral,
        t("connections:add.btcpay.staleNote"),
    ));

    // Stores, each with its payment methods (`@container`: two columns from 36 rem).
    let wide = cx
        .layout_size("btcpay-stores")
        .map(|[w, _]| w >= tk.r(36.0))
        .unwrap_or(true);
    let entries = model::plan_routes(plan, &s.choices, wallets);
    let mut stores = col().key("btcpay-stores").w_full().gap(tk.r(0.75));
    for store in &plan.stores {
        let methods: Vec<(usize, &Method)> = plan
            .payment_methods
            .iter()
            .enumerate()
            .filter(|(_, m)| m.store_id == store.id)
            .collect();
        let missing = STORE_CAPABILITIES
            .iter()
            .filter(|c| store.capability(c) == Some(false))
            .count();
        let mut ident = col()
            .min_w(0.0)
            .grow(1.0)
            .child(
                text(store.name.clone())
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .truncate(),
            )
            .child(mono11(tk, store.id.clone(), tk.muted_fg).truncate());
        if let Some(f) = store_freshness_line(tk, store) {
            ident = ident.child(f);
        }
        let mut caps = row().gap(tk.r(0.25)).flex_wrap();
        for c in STORE_CAPABILITIES {
            if let Some(v) = store.capability(c) {
                let mut b = badge(
                    tk,
                    &t(&format!("connections:add.btcpay.capability.{c}")),
                    None,
                    false,
                    !v,
                );
                if !v {
                    b = b.help(t("connections:add.btcpay.capabilityMissing"));
                }
                caps = caps.child(b);
            }
        }
        let mut card = boxed(tk, tk.r(0.5), false).child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .center_y()
                .justify_between()
                .flex_wrap()
                .child(ident)
                .child(caps),
        );
        if missing > 0 && store.capability("invoices") == Some(false) {
            card = card.child(notice(
                tk,
                Tone::Warning,
                t("connections:add.btcpay.reason.invoices_permission_missing"),
            ));
        }
        if methods.is_empty() {
            card = card.child(xs(tk, t("connections:add.btcpay.noMethods"), tk.muted_fg));
        } else {
            let mut list = col().w_full().gap(tk.r(0.5));
            for (i, m) in methods {
                list = list.child(render_method(
                    app, cx, tk, layer, plan, &entries, i, m, wide,
                ));
            }
            card = card.child(list);
        }
        stores = stores.child(card);
    }
    out = out.child(stores);

    if plan.payment_methods.is_empty() {
        out = out.child(manual_box(app, cx, tk, layer, plan, issue, wallets));
    } else {
        let counts = model::summarize_routes(&entries);
        let mut ul = col().w_full().gap(tk.r(0.125)).role("list");
        for (i, a) in ACTIONS.iter().enumerate() {
            if counts[i] > 0 {
                ul = ul.child(
                    xs(
                        tk,
                        tc(
                            &format!("connections:add.btcpay.summary.{a}"),
                            counts[i],
                            &[],
                        ),
                        tk.muted_fg,
                    )
                    .role("listitem"),
                );
            }
        }
        out = out.child(
            boxed(tk, tk.r(0.25), true)
                .child(sm_medium(tk, t("connections:add.btcpay.summary.title")))
                .child(ul),
        );
    }
    if s.show_errors
        && let Some(i) = issue.filter(|i| *i != "discover_first" && *i != "manual_store_id")
    {
        out = out.child(error_line(
            tk,
            t(&format!("connections:add.btcpay.issue.{i}")),
            true,
        ));
    }
    out
}

/// "Enter the store manually": the key cannot list payment methods.
#[allow(clippy::too_many_arguments)]
fn manual_box(
    app: &App,
    cx: &Cx<App>,
    tk: &Tk,
    layer: &mut Vec<El>,
    plan: &Plan,
    issue: Option<&'static str>,
    wallets: &[WalletOption],
) -> El {
    let s = st(app);
    let store_err = (s.show_errors && issue == Some("manual_store_id"))
        .then(|| t("connections:add.btcpay.issue.manual_store_id"));
    // The `<datalist>` of known store ids has no Rune equivalent; the store names show as the
    // field's tooltip instead.
    let mut store_input = input(
        tk,
        "btcpay-manual-store",
        &s.manual_store,
        |a: &mut App| &mut a.wallets.add.btcpay.manual_store,
        &t("connections:add.btcpay.storeId"),
        None,
        false,
        |_| {},
    );
    if !plan.stores.is_empty() {
        store_input = store_input.help(
            plan.stores
                .iter()
                .map(|x| format!("{} · {}", x.id, x.name))
                .collect::<Vec<_>>()
                .join("\n"),
        );
    }
    let pm_input = input(
        tk,
        "btcpay-manual-method",
        &s.manual_method,
        |a: &mut App| &mut a.wallets.add.btcpay.manual_method,
        &t("connections:add.btcpay.paymentMethodId"),
        Some(DEFAULT_PAYMENT_METHOD_ID),
        false,
        |_| {},
    );
    let method_name = if s.manual_method.text.is_empty() {
        DEFAULT_PAYMENT_METHOD_ID.to_owned()
    } else {
        s.manual_method.text.clone()
    };
    let store_name = if s.manual_store.text.is_empty() {
        t("connections:add.btcpay.storeId")
    } else {
        s.manual_store.text.clone()
    };
    let action_opts: Vec<form::Opt> = ACTIONS
        .iter()
        .filter(|a| **a != "skip")
        .map(|a| {
            (
                a.to_string(),
                t(&format!("connections:add.btcpay.action.{a}")),
                true,
                None,
            )
        })
        .collect();
    let action_sel = select(
        app,
        cx,
        tk,
        layer,
        "btcpay-manual-action",
        &tr(
            "connections:add.btcpay.actionLabel",
            &[("method", &method_name), ("store", &store_name)],
        ),
        &s.manual_action,
        None,
        action_opts,
        true,
        |a: &mut App, v: &str| {
            stm(a).manual_action = v.to_owned();
        },
    );
    let mut second = vec![action_sel];
    if s.manual_action == "existing_wallet" {
        let settle = model::settlement_wallet_options(wallets);
        let opts: Vec<form::Opt> = settle
            .iter()
            .map(|w| (w.label.clone(), w.label.clone(), true, None))
            .collect();
        let ph = if settle.is_empty() {
            t("connections:add.btcpay.noWallets")
        } else {
            t("connections:add.btcpay.selectWallet")
        };
        second.push(select(
            app,
            cx,
            tk,
            layer,
            "btcpay-manual-wallet",
            &tr(
                "connections:add.btcpay.walletLabel",
                &[("method", &method_name), ("store", &store_name)],
            ),
            &s.manual_wallet,
            Some(&ph),
            opts,
            !settle.is_empty(),
            |a: &mut App, v: &str| {
                stm(a).manual_wallet = v.to_owned();
            },
        ));
    }
    col()
        .w_full()
        .padding(tk.r(0.75))
        .gap(tk.r(0.75))
        .radius(tk.r(0.375))
        // `border-dashed`: Rune draws solid borders only.
        .border(1.0, tk.border.alpha(tk.border.a * 0.7))
        .border_layout()
        .child(
            col()
                .w_full()
                .gap(tk.r(0.25))
                .child(sm_medium(tk, t("connections:add.btcpay.manualTitle")))
                .child(xs(tk, t("connections:add.btcpay.manualBody"), tk.muted_fg)),
        )
        .child(form::grid2(
            tk.r(0.75),
            vec![
                setup_field(
                    tk,
                    &t("connections:add.btcpay.storeId"),
                    store_input,
                    store_err,
                    None,
                ),
                setup_field(
                    tk,
                    &t("connections:add.btcpay.paymentMethodId"),
                    pm_input,
                    None,
                    None,
                ),
            ],
        ))
        .child(form::grid2(tk.r(0.75), second))
}

// ---- scenes -------------------------------------------------------------------------------

/// A plan shaped like the model tests' live BTCPay 2.3.9 one (SYNTHETIC; no server contacted).
fn synthetic_plan(read_only: bool) -> Plan {
    let v = json!({
        "backend": "shop-btcpay",
        "server": {"version": "2.3.9", "fully_synced": true, "transport": "https"},
        "api_key": {"known": true, "scope": "all_stores", "risk": if read_only { "read_only" } else { "can_modify_store" }, "label": "kassiber-ro-all"},
        "detected_network": "regtest",
        "key_upgrade": {"preset": "wallet_history", "authorize_url": "https://btcpay.example.invalid/api-keys/authorize"},
        "warnings": if read_only { json!([{"code": "shared_store_wallet", "message": "Some stores pay into the same wallet."}]) } else { json!([{"code": "key_can_modify_store", "message": ""}]) },
        "stores": [
            {"id": "store-popup", "name": "Pop-up Stand", "capabilities": {"invoices": true, "payouts": true, "payment_requests": true, "wallet_history": false},
             "sync_state": {"last_attempt_at": null, "last_success_at": null, "last_error_code": null, "age_seconds": null, "stale": true, "never_synced": true}},
            {"id": "store-cafe", "name": "Café", "capabilities": {"invoices": true, "payouts": false, "payment_requests": true, "wallet_history": false}}
        ],
        "payment_methods": [
            {"store_id": "store-popup", "payment_method_id": "BTC-CHAIN", "label": "Bitcoin on-chain", "enabled": true, "sync_supported": true, "rail": "onchain", "chain": "bitcoin", "bitcoin_asset": true,
             "settlement": "btc_onchain", "network": "regtest", "wallet_fingerprint": "btcpay-wallet:shared",
             "owned_by": {"wallet": "Regtest Spending Wallet", "wallet_id": "w-1"}, "shared_with": [{"store_id": "store-cafe", "payment_method_id": "BTC-CHAIN"}],
             "actions": {"wallet_source": {"available": false, "reason": "wallet_history_permission_missing"}, "existing_wallet": {"available": true, "reason": null}, "provenance_only": {"available": true, "reason": null}, "skip": {"available": true, "reason": null}},
             "recommendation": {"action": "existing_wallet", "wallet": "Regtest Spending Wallet", "reason": "wallet_recognised"}},
            {"store_id": "store-cafe", "payment_method_id": "BTC-LN", "label": "Lightning", "enabled": true, "sync_supported": false, "rail": "lightning", "chain": "bitcoin", "bitcoin_asset": true,
             "settlement": "lightning", "ledger_group": "lightning",
             "actions": {"wallet_source": {"available": false, "reason": "not_on_chain"}, "existing_wallet": {"available": false, "reason": "not_on_chain"}, "payment_ledger": {"available": true, "reason": null}, "provenance_only": {"available": true, "reason": null}, "skip": {"available": true, "reason": null}},
             "recommendation": {"action": "payment_ledger", "wallet": null, "reason": "lightning_payment_ledger"}},
            {"store_id": "store-cafe", "payment_method_id": "BTC-CHAIN", "label": "Bitcoin on-chain", "enabled": true, "sync_supported": true, "rail": "onchain", "chain": "bitcoin", "bitcoin_asset": true,
             "settlement": "btc_onchain", "network": "regtest", "wallet_fingerprint": "btcpay-wallet:shared",
             "existing_routes": [{"action": "provenance_only", "store_id": "store-cafe", "payment_method_id": "BTC-CHAIN"}],
             "recommendation": {"action": "provenance_only", "wallet": null, "reason": "already_configured"}}
        ]
    });
    serde_json::from_value(v).expect("synthetic plan")
}

fn scene_open(a: &mut App) {
    crate::wallets::add::open(a, Some("btcpay"));
    crate::wallets::common::settle(a);
}

fn fill_new(a: &mut App) {
    let s = stm(a);
    s.user_chose_mode = true;
    s.mode = Mode::New;
    s.server_url = TextEdit::new("https://btcpay.example.invalid");
    s.api_key = TextEdit::new("synthetic-not-a-key");
}

fn with_plan(a: &mut App, read_only: bool) {
    fill_new(a);
    let plan = synthetic_plan(read_only);
    let wallets = wallet_options(a);
    let s = stm(a);
    s.choices = model::default_route_choices(&plan, &wallets);
    s.plan = Some(plan);
}

pub const SCENES: &[crate::app::Scene] = &[
    // The panel as it opens (no saved BTCPay instance in the demo book → New instance).
    ("wallets-add-btcpay", scene_open),
    // Submitted as it opens: the server URL error and the discover-first line (validation
    // only; nothing is sent).
    ("wallets-add-btcpay-errors", |a| {
        scene_open(a);
        stm(a).show_errors = true;
    }),
    // The manual key steps disclosed and the wallet-history preset chosen.
    ("wallets-add-btcpay-manual-key", |a| {
        scene_open(a);
        let s = stm(a);
        s.preset = "wallet_history";
        s.store_scope = "single";
        s.manual_open = true;
    }),
    // SYNTHETIC: a discovered plan (read-only key, two stores, three methods) as Check key and
    // stores would show it; no server is contacted.
    ("wallets-add-btcpay-plan", |a| {
        scene_open(a);
        with_plan(a, true);
    }),
    ("wallets-add-btcpay-plan-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_open(a);
        with_plan(a, true);
    }),
    ("wallets-add-btcpay-plan-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_open(a);
        with_plan(a, false);
    }),
    // SYNTHETIC: a key that cannot list payment methods: the manual store route.
    ("wallets-add-btcpay-manual-store", |a| {
        scene_open(a);
        fill_new(a);
        let mut plan = synthetic_plan(true);
        plan.payment_methods.clear();
        let s = stm(a);
        s.plan = Some(plan);
        s.manual_action = "existing_wallet".into();
        s.show_errors = true;
    }),
];

#[cfg(test)]
mod discovery_tests {
    use super::*;

    /// Discovery of instance A answering after the server changed: A's plan is not installed
    /// for the current instance.
    #[test]
    fn an_earlier_instances_plan_is_not_installed() {
        let mut app = crate::app::tests::quiet_app();
        stm(&mut app)
            .discovering
            .push_back("{\"server_url\":\"https://a.example\"}".into());
        stm(&mut app).discover_pending = true;
        let plan = Reply {
            kind: "ui.connections.btcpay.discover".into(),
            data: json!({"stores": []}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.btcpay.discover", &plan);
        assert!(st(&app).plan.is_none());
        assert!(!st(&app).discover_pending);
    }
}
