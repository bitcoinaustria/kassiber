//! components/kb/exchange/*: `ExchangeApiSetupPanel` (+ exchangeApiModel, ExchangeErrorNotice)
//! for the Add connection dialog, and `ExchangeSyncCard` (+ useExchangeSync) for the wallet
//! detail.
//!
//! Contract with the dialog (`wallets::add`), for sources with `exchange` set and
//! `setupKind === "exchange-api"`:
//! - `reset(app, exchange)` when the setup step opens; `panel` draws it with the dialog's
//!   `label_field` and `footer` (`renderSyncAfterCreate(t("exchange.syncAfter", {provider}))`).
//! - `submit(app, label)`: shows errors and returns the `ui.connections.exchange.create` args
//!   (`credentialArgs(draft.credentials)` + `label`), or the dialog's error line
//!   (`exchange.issue.test_first` / `exchange.issue.<issue>`).
//!
//! The panel's Test connection (`ui.connections.exchange.test`) contacts the exchange: egress,
//! only from that button. Tags `"wallets.exchange.…"`.
//!
//! Contract with the wallet detail (`wallets::detail`), for wallets whose kind is an exchange:
//! `sync_poll` every frame while it is shown (starts the local `ui.connections.exchange.status`
//! read once per wallet), `sync_card` draws `ExchangeSyncCard`, `sync_pending` says whether
//! its "Sync now" (`ui.connections.exchange.sync`, egress, button only) is in flight.

pub mod model;

use rune::{Cx, El, TextEdit, Tx, col, row, text};
use serde_json::{Value, json};

use super::btcpay::form::{self, Variant, button, error_line, input, setup_field, xs};
use super::btcpay::model::Tone;
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;
use crate::wallets::conn::Conn;
use model::{Credentials, ErrorInfo};

/// A Test connection result, for the credentials it was run with.
#[derive(Clone, Debug)]
pub struct Outcome {
    pub credentials: Credentials,
    /// `ExchangeTestResult`: (ok, read_only).
    pub result: Option<(bool, Option<bool>)>,
    pub error: Option<ErrorInfo>,
}

#[derive(Default)]
pub struct State {
    pub exchange: String,
    pub api_key: TextEdit,
    pub api_secret: TextEdit,
    pub outcome: Option<Outcome>,
    pub link_error: Option<String>,
    pub test_pending: bool,
    /// Each test in flight, oldest first: the form it was sent from (`form`) and the
    /// credentials it sent. Kept across a reset, so a closed form's answer is recognized.
    pub tests: std::collections::VecDeque<(u64, Credentials)>,
    /// Counts form resets (each open of Add connection starts a new form).
    pub form: u64,
    /// Each exchange sync in flight, oldest first: (wallet, label, exchange). Kept outside
    /// the page's `SyncState`, which another wallet's page replaces.
    pub syncs: std::collections::VecDeque<(String, String, String)>,
    /// The wallet of each status read in flight, oldest first.
    pub statuses: std::collections::VecDeque<String>,
    pub show_errors: bool,
    pub sync: SyncState,
    /// Scene harness: `wallets-detail-exchange-card` mounts `card_scene_view` full-page.
    pub card_scene: bool,
    /// A save refused because the same exchange's CSV wallet holds this label: (id, label) of
    /// that wallet, offered to take the key instead (one hybrid source, no double count).
    pub attach_offer: Option<(String, String)>,
    /// The wallet the next save adds the key to (the offer's button).
    pub attach_wallet: Option<String>,
}

/// `useExchangeSync` for the wallet on screen.
#[derive(Default)]
pub struct SyncState {
    /// The wallet the status below belongs to (the read starts once per wallet).
    pub wallet: Option<String>,
    pub row: Option<Value>,
    pub status_error: Option<ErrorInfo>,
    pub last_error: Option<ErrorInfo>,
    pub label: String,
    pub exchange: String,
}

fn st(app: &App) -> &State {
    &app.wallets.add.exchange
}

fn stm(app: &mut App) -> &mut State {
    &mut app.wallets.add.exchange
}

/// `isExchangeKind`.
pub fn is_exchange_kind(kind: &str) -> bool {
    model::is_kind(kind)
}

/// `EXCHANGE_PROVIDER_NAMES`.
pub fn provider_name(exchange: &str) -> &'static str {
    model::provider_name(exchange)
}

fn credentials(s: &State) -> Credentials {
    Credentials {
        exchange: s.exchange.clone(),
        api_key: s.api_key.text.clone(),
        api_secret: s.api_secret.text.clone(),
    }
}

/// `tested`: the current credentials passed Test connection.
fn tested(s: &State) -> bool {
    let c = credentials(s);
    s.outcome.as_ref().is_some_and(|o| {
        o.result.is_some_and(|r| r.0) && model::same_credentials(&c, Some(&o.credentials))
    })
}

pub fn reset(app: &mut App, exchange: &str) {
    let s = stm(app);
    let sync = std::mem::take(&mut s.sync);
    let tests = std::mem::take(&mut s.tests);
    let syncs = std::mem::take(&mut s.syncs);
    let statuses = std::mem::take(&mut s.statuses);
    let form = s.form + 1;
    app.wallets.add.exchange = State {
        exchange: exchange.to_owned(),
        sync,
        tests,
        syncs,
        statuses,
        form,
        ..State::default()
    };
}

pub fn submit(app: &mut App, label: &str) -> Result<Value, String> {
    stm(app).show_errors = true;
    let s = st(app);
    let c = credentials(s);
    if let Some(i) = model::credential_issue(&c) {
        return Err(t(&format!("connections:exchange.issue.{i}")));
    }
    if !tested(s) {
        return Err(t("connections:exchange.issue.test_first"));
    }
    let mut args = model::credential_args(&c);
    args.insert("label".into(), json!(label));
    if let Some(wallet) = stm(app).attach_wallet.take() {
        args.insert("wallet".into(), json!(wallet));
    }
    Ok(Value::Object(args))
}

/// The offer ends with a save, a new choice or another source.
pub fn clear_attach(app: &mut App) {
    stm(app).attach_offer = None;
    stm(app).attach_wallet = None;
}

/// A refused save: the CSV wallet the daemon names (`details.attachable_wallet_id`) can take
/// the key instead.
pub fn on_create_error(app: &mut App, detail: &Value) {
    let d = &detail["details"];
    stm(app).attach_offer = d["attachable_wallet_id"].as_str().map(|id| {
        (
            id.to_owned(),
            d["attachable_wallet"].as_str().unwrap_or(id).to_owned(),
        )
    });
}

/// The offer under the panel: add the key to the existing CSV wallet.
pub fn attach_offer(app: &App, tk: &Tk, provider: &str) -> Option<El> {
    let (id, label) = st(app).attach_offer.clone()?;
    let (fill, border, ink) = crate::wallets::detail::ui::amber(tk);
    let button_label = tr("connections:coverage.attach.button", &[("wallet", &label)]);
    Some(
        col()
            .key("connection-exchange-attach")
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .radius(tk.r(0.375))
            .fill(fill)
            .border(1.0, border)
            .border_layout()
            .role("status")
            .child(
                text(tr(
                    "connections:coverage.attach.body",
                    &[("provider", provider), ("wallet", &label)],
                ))
                .font(tk.tsm(400.0))
                .color(ink)
                .wrap()
                .w_full(),
            )
            .child(
                crate::txdetail::widgets::button(
                    tk,
                    "connection-exchange-attach-submit",
                    crate::txdetail::widgets::Variant::Outline,
                    crate::txdetail::widgets::Size::Sm,
                    None,
                    &button_label,
                    true,
                )
                .on_press(move |a: &mut App| {
                    stm(a).attach_wallet = Some(id.clone());
                    crate::wallets::add::submit_setup(a);
                }),
            ),
    )
}

fn run_test(a: &mut App) {
    let s = st(a);
    let c = credentials(s);
    if model::credential_issue(&c).is_some() || s.test_pending {
        return;
    }
    if a.send(
        "ui.connections.exchange.test",
        Value::Object(model::credential_args(&c)),
        "wallets.exchange.test",
    )
    .is_some()
    {
        let s = stm(a);
        s.test_pending = true;
        let form = s.form;
        s.tests.push_back((form, c));
    }
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "wallets.exchange.test" => {
            let s = stm(app);
            let Some((form, sent)) = s.tests.pop_front() else {
                return;
            };
            // A test from a form since closed: not this form's result.
            if form != s.form {
                return;
            }
            s.test_pending = false;
            s.outcome = Some(match &r.error {
                None => Outcome {
                    credentials: sent,
                    result: Some((
                        r.data["ok"].as_bool().unwrap_or(false),
                        r.data["read_only"].as_bool(),
                    )),
                    error: None,
                },
                Some(m) => Outcome {
                    credentials: sent,
                    result: None,
                    error: Some(ErrorInfo::from_reply(&r.error_detail, Some(m))),
                },
            });
        }
        "wallets.exchange.status" => {
            let asked = stm(app).statuses.pop_front();
            let s = &mut stm(app).sync;
            // Another wallet's page since: not this page's status.
            if asked.is_none() || asked != s.wallet {
                return;
            }
            match &r.error {
                None => {
                    s.row = r.data["connections"]
                        .as_array()
                        .and_then(|a| a.first())
                        .cloned();
                    s.status_error = None;
                }
                Some(m) => s.status_error = Some(ErrorInfo::from_reply(&r.error_detail, Some(m))),
            }
        }
        "wallets.exchange.sync" => {
            // The wallet this sync was for, whichever wallet's page is open now.
            let Some((wallet, label, exchange)) = stm(app).syncs.pop_front() else {
                return;
            };
            match &r.error {
                None => {
                    let imported = r.data["imported"].as_u64().unwrap_or(0) as usize;
                    let title = t("chrome:notifications.result.connectionSynced.title");
                    let clock = app.clock;
                    app.shell.note(
                        &title,
                        Some(format!(
                            "{label} · {}",
                            tc(
                                "chrome:notifications.result.connectionSynced.imported",
                                imported,
                                &[]
                            )
                        )),
                        ToastTone::Success,
                        clock,
                    );
                    app.toast(
                        ToastTone::Success,
                        &title,
                        Some(label),
                        Some("exchange-sync"),
                    );
                }
                Some(m) => {
                    let info = ErrorInfo::from_reply(&r.error_detail, Some(m));
                    let provider = provider_name(&exchange);
                    let msg = info.message.clone().unwrap_or_else(|| {
                        tr(
                            "connections:exchange.error.unknown",
                            &[("provider", provider)],
                        )
                    });
                    let title = tr(
                        "chrome:notifications.condition.connectionSyncFailed.titleNamed",
                        &[("name", &label)],
                    );
                    let clock = app.clock;
                    app.shell
                        .note(&title, Some(msg.clone()), ToastTone::Error, clock);
                    app.toast(ToastTone::Error, &title, Some(msg), Some("exchange-sync"));
                    if stm(app).sync.wallet.as_deref() == Some(wallet.as_str()) {
                        stm(app).sync.last_error = Some(info);
                    }
                }
            }
            // `status.refetch()`, and the mutation invalidates every read.
            stm(app).statuses.push_back(wallet.clone());
            app.send(
                "ui.connections.exchange.status",
                json!({"wallet": wallet}),
                "wallets.exchange.status",
            );
            app.refresh_reads();
        }
        _ => {}
    }
}

/// The exchange of wallet `c` (`isExchangeKind(walletDetail.kind)`): the wallets list's kind,
/// else the snapshot's.
pub fn exchange_of(app: &App, c: &Conn) -> Option<String> {
    let listed = app
        .read("ui.wallets.list")
        .and_then(|d| d["wallets"].as_array())
        .and_then(|a| a.iter().find(|w| w["id"].as_str() == Some(c.id.as_str())))
        .and_then(|w| w["kind"].as_str())
        .map(str::to_owned);
    listed
        .filter(|k| model::is_kind(k))
        .or_else(|| model::is_kind(&c.kind).then(|| c.kind.clone()))
        .map(|k| k.trim().to_lowercase())
}

/// Starts the local status read once per exchange wallet.
pub fn sync_poll(app: &mut App, _tx: &mut Tx, c: &Conn) -> bool {
    sync_prime(app, c)
}

/// `sync_poll` without a `Tx` (headless scenes).
pub fn sync_prime(app: &mut App, c: &Conn) -> bool {
    let Some(ex) = exchange_of(app, c) else {
        return false;
    };
    if st(app).sync.wallet.as_deref() == Some(c.id.as_str()) {
        return false;
    }
    stm(app).sync = SyncState {
        wallet: Some(c.id.clone()),
        label: c.label.clone(),
        exchange: ex,
        ..SyncState::default()
    };
    stm(app).statuses.push_back(c.id.clone());
    app.send(
        "ui.connections.exchange.status",
        json!({"wallet": c.id}),
        "wallets.exchange.status",
    );
    true
}

/// Whether the shown wallet has a sync in flight.
pub fn sync_pending(app: &App) -> bool {
    let s = st(app);
    s.sync
        .wallet
        .as_ref()
        .is_some_and(|w| s.syncs.iter().any(|(x, _, _)| x == w))
}

fn run_sync(a: &mut App) {
    let s = &st(a).sync;
    let Some(wallet) = s.wallet.clone() else {
        return;
    };
    if sync_pending(a) {
        return;
    }
    let (label, exchange) = (s.label.clone(), s.exchange.clone());
    stm(a).sync.last_error = None;
    if a.send(
        "ui.connections.exchange.sync",
        json!({"wallet": wallet}),
        "wallets.exchange.sync",
    )
    .is_some()
    {
        stm(a).syncs.push_back((wallet, label, exchange));
    }
}

// ---- view ---------------------------------------------------------------------------------

/// `ExchangeErrorNotice`: the code's meaning, permissions still on, the daemon's message and
/// hint (`role="alert"`, destructive tint).
pub fn error_notice(tk: &Tk, provider: &str, e: &ErrorInfo) -> El {
    let kind = model::error_kind(e.code.as_deref());
    let perms = model::refused_permissions(e);
    let ink = tk.destructive;
    let mut c = col()
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .fill(tk.destructive.alpha(0.1))
        .border(1.0, tk.destructive.alpha(0.5))
        .border_layout()
        .role("alert")
        .child(
            text(tr(
                &format!("connections:exchange.error.{kind}"),
                &[("provider", provider)],
            ))
            .font(tk.txs(500.0))
            .color(ink)
            .wrap(),
        );
    if !perms.is_empty() {
        c = c.child(xs(
            tk,
            tr(
                "connections:exchange.error.permissions",
                &[("list", &perms.join(", "))],
            ),
            ink,
        ));
    }
    if kind != "needsRust" {
        if let Some(m) = e.message.as_deref().filter(|m| !m.is_empty()) {
            c = c.child(xs(
                tk,
                tr("connections:exchange.error.serverSaid", &[("message", m)]),
                ink,
            ));
        }
        if let Some(h) = &e.hint {
            c = c.child(xs(tk, h.clone(), ink));
        }
    }
    c
}

/// `<code>` inside the guide's lists.
fn code(tk: &Tk, s: &str) -> El {
    text(s.to_owned()).font(tk.txs(400.0).mono()).color(tk.fg)
}

/// `ExchangeApiSetupPanel`.
pub fn panel(
    app: &App,
    _cx: &mut Cx<App>,
    tk: &Tk,
    _layer: &mut Vec<El>,
    exchange: &str,
    label_field: El,
    footer: El,
) -> El {
    let s = st(app);
    let provider = provider_name(exchange);
    let guide = model::key_guide(exchange);
    let need_secret = model::needs_secret(exchange);
    let creds = Credentials {
        exchange: exchange.to_owned(),
        ..credentials(s)
    };
    let issue = model::credential_issue(&creds);
    let ok_tested = s.outcome.as_ref().is_some_and(|o| {
        o.result.is_some_and(|r| r.0) && model::same_credentials(&creds, Some(&o.credentials))
    });
    let stale = s
        .outcome
        .as_ref()
        .is_some_and(|o| o.result.is_some_and(|r| r.0))
        && !ok_tested;
    let field_err = |f: &str| {
        (s.show_errors && issue == Some(f)).then(|| t(&format!("connections:exchange.issue.{f}")))
    };

    let list = |title: String, items: &'static [&'static str]| -> El {
        let mut ul = col()
            .w_full()
            .mt(tk.r(0.25))
            .role("list")
            .label(title.clone());
        for it in items {
            ul = ul.child(form::list_item(tk, "•", code(tk, it)));
        }
        col()
            .min_w(0.0)
            .child(text(title).font(tk.txs(500.0)).color(tk.fg))
            .child(ul)
    };
    let url = guide.key_page_url;
    let guide_key = if model::is_kind(exchange) {
        exchange
    } else {
        "kraken"
    };
    let mut guide_box = col()
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.75))
        .gap(tk.r(0.5))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.3))
        .role("region")
        .label(t("connections:exchange.guide.title"))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(ic(&I::KEY_ROUND, tk.r(1.0), tk.fg))
                .child(
                    text(t("connections:exchange.guide.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .role("heading"),
                ),
        )
        .child(xs(
            tk,
            t(&format!("connections:exchange.guide.{guide_key}")),
            tk.muted_fg,
        ))
        .child(form::grid2(
            tk.r(0.5),
            vec![
                list(t("connections:exchange.guide.grant"), guide.grant),
                list(t("connections:exchange.guide.deny"), guide.deny),
            ],
        ))
        .child(
            row().child(
                form::link_button(
                    tk,
                    "exchange-key-page",
                    &I::EXTERNAL_LINK,
                    &tr("connections:exchange.guide.open", &[("provider", provider)]),
                    tk.fg,
                )
                .on_press_tx(move |a: &mut App, tx: &mut Tx| {
                    stm(a).link_error = None;
                    if let Err(e) = tx.open_url(url) {
                        stm(a).link_error = Some(format!("{e:?}"));
                    }
                }),
            ),
        );
    if let Some(e) = &s.link_error {
        guide_box = guide_box.child(error_line(tk, e.clone(), false));
    }

    let mut out = col()
        .key("exchange-setup")
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.0))
        .child(
            text(tr("connections:exchange.intro", &[("provider", provider)]))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(label_field)
        .child(guide_box)
        .child(setup_field(
            tk,
            &t("connections:exchange.field.apiKey"),
            input(
                tk,
                "exchange-api-key",
                &s.api_key,
                |a: &mut App| &mut a.wallets.add.exchange.api_key,
                &t("connections:exchange.field.apiKey"),
                None,
                true,
                |_| {},
            ),
            field_err("api_key"),
            None,
        ));
    if need_secret {
        let lbl = t(model::secret_label_key(exchange));
        out = out.child(setup_field(
            tk,
            &lbl,
            input(
                tk,
                "exchange-api-secret",
                &s.api_secret,
                |a: &mut App| &mut a.wallets.add.exchange.api_secret,
                &lbl,
                None,
                true,
                |_| {},
            ),
            field_err("api_secret"),
            None,
        ));
    }
    out = out.child(xs(tk, t("connections:exchange.field.stored"), tk.muted_fg));

    let pending = s.test_pending;
    let mut test_btn = button(
        tk,
        "exchange-test",
        Variant::Outline,
        true,
        pending.then_some((&I::LOADER_CIRCLE, tk.r(1.0), true)),
        &if pending {
            t("connections:exchange.test.pending")
        } else {
            t("connections:exchange.test.button")
        },
        issue.is_none() && !pending,
    );
    if issue.is_none() && !pending {
        test_btn = test_btn.on_press(run_test);
    }
    let mut test = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(xs(
            tk,
            tr(
                "connections:exchange.test.explain",
                &[("provider", provider)],
            ),
            tk.muted_fg,
        ))
        .child(row().child(test_btn));
    if ok_tested && let Some((_, read_only)) = s.outcome.as_ref().and_then(|o| o.result) {
        let ro = read_only == Some(true);
        let tone = if ro { Tone::Good } else { Tone::Warning };
        let (b, f, ink) = form::tone_colors(tk, tone);
        test = test.child(
            row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .items_start()
                .radius(tk.r(0.375))
                .fill(f)
                .border(1.0, b)
                .border_layout()
                .role("status")
                .child(
                    ic(
                        if ro {
                            &I::SHIELD_CHECK
                        } else {
                            &I::SHIELD_ALERT
                        },
                        tk.r(1.0),
                        ink,
                    )
                    .mt(tk.r(0.125)),
                )
                .child(
                    xs(
                        tk,
                        tr(
                            if ro {
                                "connections:exchange.test.okReadOnly"
                            } else {
                                "connections:exchange.test.okUnknown"
                            },
                            &[("provider", provider)],
                        ),
                        ink,
                    )
                    .grow(1.0)
                    .min_w(0.0),
                ),
        );
    }
    if stale {
        test = test.child(
            xs(
                tk,
                t("connections:exchange.test.changed"),
                form::amber_ink(tk),
            )
            .role("status"),
        );
    }
    if let Some(o) = &s.outcome
        && let Some(e) = o
            .error
            .as_ref()
            .filter(|_| model::same_credentials(&creds, Some(&o.credentials)))
    {
        test = test.child(error_notice(tk, provider, e));
    }
    if s.show_errors && issue.is_none() && !ok_tested {
        test = test.child(error_line(
            tk,
            t("connections:exchange.issue.test_first"),
            false,
        ));
    }
    out.child(test).child(footer)
}

/// `ExchangeSyncCard` for wallet `c` (nothing when it isn't an exchange wallet).
pub fn sync_card(app: &App, _cx: &mut Cx<App>, tk: &Tk, c: &Conn, disabled: bool) -> El {
    let Some(ex) = exchange_of(app, c) else {
        return col();
    };
    let s = &st(app).sync;
    let provider = provider_name(&ex);
    let mine = s.wallet.as_deref() == Some(c.id.as_str());
    let row_v = if mine { s.row.as_ref() } else { None };
    let status_error = if mine { s.status_error.as_ref() } else { None };
    let needs_rust =
        status_error.is_some_and(|e| model::error_kind(e.code.as_deref()) == "needsRust");
    let last = row_v
        .and_then(|r| r.get("last_sync"))
        .filter(|l| l.is_object());
    let last_err = if mine { s.last_error.clone() } else { None };
    let error = last_err.or_else(|| {
        last.filter(|l| l["status"] == "error")
            .and_then(|l| l.get("error"))
            .filter(|e| e.is_object())
            .map(|e| ErrorInfo::from_reply(e, None))
    });
    let key_stored: Option<bool> = row_v.map(|r| {
        r["backend_configured"].as_bool().unwrap_or(false)
            && r["has_api_key"].as_bool().unwrap_or(false)
    });
    let skipped: u64 = last
        .and_then(|l| l["skipped"].as_object())
        .map(|m| m.values().filter_map(Value::as_u64).sum())
        .unwrap_or(0);
    let pending = mine && sync_pending(app);
    let enabled = !(disabled || pending || needs_rust || key_stored == Some(false));

    let mut sync_btn = button(
        tk,
        "exchange-sync-now",
        Variant::Outline,
        true,
        Some((&I::REFRESH_CW, tk.r(1.0), pending)),
        &if pending {
            t("connections:exchange.sync.running")
        } else {
            t("connections:exchange.sync.button")
        },
        enabled,
    );
    if enabled {
        sync_btn = sync_btn.on_press(run_sync);
    }
    let title = tr("connections:exchange.sync.title", &[("provider", provider)]);
    let head = row()
        .w_full()
        .gap(tk.r(0.75))
        .items_start()
        .justify_between()
        .child(
            col()
                .min_w(0.0)
                .grow(1.0)
                .gap(tk.r(0.25))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::KEY_ROUND, tk.r(1.0), tk.fg))
                        .child(
                            text(title.clone())
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .role("heading"),
                        ),
                )
                .child(xs(
                    tk,
                    tr("connections:exchange.sync.body", &[("provider", provider)]),
                    tk.muted_fg,
                )),
        )
        .child(sync_btn);
    let mut card = crate::ui::surface(tk)
        .key("exchange-sync-card")
        .w_full()
        .padding(tk.r(1.0))
        .gap(tk.r(0.75))
        .role("region")
        .label(title)
        .child(head);

    if let (true, Some(e)) = (needs_rust, status_error) {
        card = card.child(error_notice(tk, provider, e));
    } else {
        let key_text = match key_stored {
            None => "…".to_owned(),
            Some(true) => tr(
                "connections:exchange.sync.keyStored",
                &[(
                    "backend",
                    row_v.and_then(|r| r["backend"].as_str()).unwrap_or(""),
                )],
            ),
            Some(false) => t("connections:exchange.sync.keyMissing"),
        };
        let last_text = match last {
            None => t("connections:exchange.sync.never"),
            Some(l) if l["status"] == "ok" => tr(
                "connections:exchange.sync.ok",
                &[
                    (
                        "time",
                        &super::btcpay::format_medium(l["at"].as_str().unwrap_or("")),
                    ),
                    (
                        "imported",
                        &crate::format::count(l["imported"].as_u64().unwrap_or(0) as usize),
                    ),
                    (
                        "fetched",
                        &crate::format::count(l["fetched"].as_u64().unwrap_or(0) as usize),
                    ),
                ],
            ),
            Some(l) => tr(
                "connections:exchange.sync.failed",
                &[(
                    "time",
                    &super::btcpay::format_medium(l["at"].as_str().unwrap_or("")),
                )],
            ),
        };
        let pair = |dt: String, dd: String| -> El {
            row()
                .w_full()
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .justify_between()
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border.alpha(tk.border.a * 0.6))
                .border_layout()
                .child(text(dt).font(tk.txs(400.0)).color(tk.muted_fg))
                .child(text(dd).font(tk.txs(500.0)).color(tk.fg).wrap().min_w(0.0))
        };
        card = card.child(form::grid2(
            tk.r(1.5),
            vec![
                pair(t("connections:exchange.sync.key"), key_text),
                pair(t("connections:exchange.sync.lastSync"), last_text),
            ],
        ));
    }
    if last.is_some_and(|l| l["status"] == "ok") && skipped > 0 {
        card = card.child(xs(
            tk,
            tc("connections:exchange.sync.skipped", skipped as usize, &[]),
            tk.muted_fg,
        ));
    }
    if let (Some(e), false) = (&error, needs_rust) {
        card = card.child(error_notice(tk, provider, e));
    }
    // The last sync's outcome is stored with the book now (no "kept until restart" note).
    card
}

// ---- scenes -------------------------------------------------------------------------------

/// The catalog source of an exchange's API setup (`binance-api`; the others share the name).
pub fn source_id(ex: &str) -> &str {
    if ex == "binance" { "binance-api" } else { ex }
}

fn scene_open(a: &mut App, ex: &str) {
    crate::wallets::add::open(a, Some(source_id(ex)));
    crate::wallets::common::settle(a);
    if st(a).exchange != ex {
        reset(a, ex);
    }
}

/// SYNTHETIC: a Test connection outcome as the daemon would report it (nothing is contacted).
fn fake_test(a: &mut App, ok: bool, read_only: Option<bool>, error: Option<ErrorInfo>) {
    let s = stm(a);
    s.api_key = TextEdit::new("synthetic-not-a-key");
    s.api_secret = TextEdit::new("synthetic-not-a-secret");
    s.outcome = Some(Outcome {
        credentials: credentials(s),
        result: ok.then_some((true, read_only)),
        error,
    });
}

/// SYNTHETIC: an exchange wallet for the sync card (the demo book has none).
pub fn synthetic_conn() -> Conn {
    Conn::parse(
        &json!({"id": "synthetic-kraken", "kind": "kraken", "label": "Kraken", "status": "idle", "balance": 0}),
    )
}

/// SYNTHETIC: the sync card's status as the daemon reports a wallet synced this session.
pub fn synthetic_sync(a: &mut App) {
    let c = synthetic_conn();
    a.wallets.add.exchange.sync = SyncState {
        wallet: Some(c.id.clone()),
        label: c.label.clone(),
        exchange: "kraken".into(),
        row: Some(
            json!({"wallet_id": c.id, "wallet": "Kraken", "exchange": "kraken", "provider": "Kraken", "backend": "kraken-api", "backend_configured": true, "has_api_key": true, "has_api_secret": true,
            "last_sync": {"status": "ok", "at": "2026-11-13T12:34:00Z", "imported": 12, "fetched": 40, "skipped": {"fiat_only": 3}}}),
        ),
        ..SyncState::default()
    };
}

/// The `wallets-detail-exchange-card` harness page: the card for the synthetic wallet, as the
/// wallet detail places it (`kb-surface` in the page column).
pub fn card_scene_view(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    col()
        .w_full()
        .padding(tk.r(1.25))
        .child(sync_card(app, cx, tk, &synthetic_conn(), false))
}

pub const SCENES: &[crate::app::Scene] = &[
    ("wallets-add-exchange-kraken", |a| scene_open(a, "kraken")),
    ("wallets-add-exchange-coinbase", |a| {
        scene_open(a, "coinbase")
    }),
    ("wallets-add-exchange-binance", |a| scene_open(a, "binance")),
    ("wallets-add-exchange-bitpanda", |a| {
        scene_open(a, "bitpanda")
    }),
    // Submitted with nothing pasted: the key error.
    ("wallets-add-exchange-kraken-errors", |a| {
        scene_open(a, "kraken");
        stm(a).show_errors = true;
    }),
    // SYNTHETIC: a passing test that confirms the key is read-only.
    ("wallets-add-exchange-binance-ok", |a| {
        scene_open(a, "binance");
        fake_test(a, true, Some(true), None);
    }),
    // SYNTHETIC: a refused key with the permissions it still holds.
    ("wallets-add-exchange-binance-refused", |a| {
        scene_open(a, "binance");
        let e = ErrorInfo {
            code: Some("validation".into()),
            message: Some("API key has trading permissions".into()),
            hint: Some("Edit the key's restrictions in Binance.".into()),
            details: Some(json!({"permissions": ["enableSpotAndMarginTrading"]})),
        };
        fake_test(a, false, None, Some(e));
    }),
    ("wallets-add-exchange-kraken-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        scene_open(a, "kraken");
        fake_test(a, true, None, None);
    }),
    ("wallets-add-exchange-kraken-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        scene_open(a, "kraken");
    }),
    // SYNTHETIC: `ExchangeSyncCard` for a Kraken wallet synced this session (the demo book has
    // no exchange wallet); the harness mounts it full-page.
    ("wallets-detail-exchange-card", |a| {
        a.navigate("/connections");
        a.wallets.add.exchange.card_scene = true;
        synthetic_sync(a);
    }),
];

#[cfg(test)]
mod test_reply_tests {
    use super::*;

    fn ok() -> Reply {
        Reply {
            kind: "ui.connections.exchange.test".into(),
            data: json!({"ok": true, "read_only": true}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        }
    }

    /// Wallet A's sync still running when wallet B's page opens: A's failure is not B's
    /// error, B's own sync stays pending, and A's status read does not land on B's card.
    #[test]
    fn an_exchange_sync_answer_stays_with_its_wallet() {
        let mut app = crate::app::tests::quiet_app();
        stm(&mut app)
            .syncs
            .push_back(("a".into(), "A".into(), "kraken".into()));
        stm(&mut app)
            .syncs
            .push_back(("b".into(), "B".into(), "kraken".into()));
        stm(&mut app).sync = SyncState {
            wallet: Some("b".into()),
            label: "B".into(),
            exchange: "kraken".into(),
            ..SyncState::default()
        };
        let failed = Reply {
            kind: "ui.connections.exchange.sync".into(),
            data: Value::Null,
            error: Some("refused".into()),
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.exchange.sync", &failed);
        assert!(st(&app).sync.last_error.is_none(), "A's failure is not B's");
        assert!(sync_pending(&app), "B's sync still runs");
        // A's status read (queued after its sync) does not land on B's card.
        let status = Reply {
            kind: "ui.connections.exchange.status".into(),
            data: json!({"connections": [{"id": "a"}]}),
            error: None,
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "wallets.exchange.status", &status);
        assert!(st(&app).sync.row.is_none());
    }

    /// A test for credentials A still out when Add connection closes and reopens: its answer
    /// is not the new form's result; the new form's own test is.
    #[test]
    fn a_closed_forms_test_answer_is_not_the_new_forms_result() {
        let mut app = crate::app::tests::quiet_app();
        reset(&mut app, "kraken");
        let a = Credentials {
            exchange: "kraken".into(),
            api_key: "A".into(),
            api_secret: "a".into(),
        };
        let form = stm(&mut app).form;
        stm(&mut app).tests.push_back((form, a));
        reset(&mut app, "kraken");
        let b = Credentials {
            exchange: "kraken".into(),
            api_key: "B".into(),
            api_secret: "b".into(),
        };
        let form = stm(&mut app).form;
        let s = stm(&mut app);
        s.test_pending = true;
        s.tests.push_back((form, b));
        on_reply(&mut app, "wallets.exchange.test", &ok());
        assert!(st(&app).outcome.is_none(), "A's answer dropped");
        assert!(st(&app).test_pending);
        on_reply(&mut app, "wallets.exchange.test", &ok());
        let o = st(&app).outcome.as_ref().unwrap();
        assert_eq!(o.credentials.api_key, "B");
        assert!(!st(&app).test_pending);
    }
}
