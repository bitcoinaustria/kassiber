//! The assistant's most common changes as the app's own cards ("answers are actions, not text"):
//! linking two transactions as one transfer (`ui.transfers.pair`), no longer suggesting a link
//! (`ui.transfers.dismiss`) and editing a transaction (`ui.transactions.metadata.update`).
//!
//! A waiting call shows what changes in plain terms: the transactions as the open book has them
//! (`ui.transactions.resolve`, a local read that starts no sync and contacts nothing) and each
//! field it sets, current value first. Apply and Decline answer the daemon's consent
//! (`ai.tool_call.consent`) exactly as the approval row does; none of these tools is plan-bound,
//! so the daemon validates and computes everything when the approved call runs. Arguments that do
//! not parse as the tool's documented shape keep the generic row with the raw arguments.
//!
//! A finished change whose result names an undo (`data.undo`) offers Undo when the hinted tool is
//! a book mutation the desktop may send itself (`ALLOWED_DAEMON_KINDS`): the desktop sends it as
//! the user's own action, with the hint's arguments and the open book's `expected_scope`, then
//! re-reads the screen. A plan-bound inverse (a plan the model must show first) or a kind outside
//! the allowlist gets no button; the hint still says the assistant can take it back.

use std::collections::HashMap;

use rune::{El, col, row, text};
use serde_json::{Value, json};

use crate::app::{App, Reply};
use crate::assistant::chat::{Book, book_now};
use crate::assistant::model::{ToolCall, ToolStatus};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

const PAIR_KINDS: [&str; 9] = [
    "manual",
    "coinjoin",
    "whirlpool",
    "chain-swap",
    "peg-in",
    "peg-out",
    "reverse-submarine-swap",
    "submarine-swap",
    "swap-refund",
];

const POLICIES: [&str; 2] = ["carrying-value", "taxable"];

/// The fields `ui.transactions.metadata.update` sets, in the order the card lists them, with the
/// key the resolved transaction keeps the current value under.
const EDIT_FIELDS: [(&str, Option<&str>); 14] = [
    ("note", Some("note")),
    ("tags", Some("tags")),
    ("kind", Some("kindOverride")),
    ("excluded", Some("excluded")),
    ("taxable", Some("taxable")),
    ("review_status", Some("reviewStatus")),
    ("at_regime", Some("atRegime")),
    ("at_category", Some("atCategory")),
    ("fiat_rate", Some("rate")),
    ("fiat_value", None),
    ("fiat_currency", Some("fiatCurrency")),
    ("pricing_source_kind", Some("pricingSourceKind")),
    ("pricing_quality", Some("pricingQuality")),
    ("pricing_external_ref", Some("pricingExternalRef")),
];

/// Amounts shown behind the privacy blur.
const SENSITIVE_FIELDS: [&str; 2] = ["fiat_rate", "fiat_value"];

/// A change the assistant proposes, as its tool's arguments describe it.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    Pair {
        out: String,
        inn: String,
        kind: Option<String>,
        policy: Option<String>,
        out_amount: Option<String>,
        notes: Option<String>,
    },
    Dismiss {
        out: String,
        inn: String,
        reason: Option<String>,
        days: Option<u64>,
    },
    Edit {
        transaction: String,
        fields: Vec<(&'static str, Value)>,
        reason: Option<String>,
    },
}

impl Action {
    /// The transactions the card names, in order.
    pub fn tx_ids(&self) -> Vec<&str> {
        match self {
            Action::Pair { out, inn, .. } | Action::Dismiss { out, inn, .. } => vec![out, inn],
            Action::Edit { transaction, .. } => vec![transaction],
        }
    }

    pub fn title(&self) -> String {
        t(match self {
            Action::Pair { .. } => "assistant:action.pair.title",
            Action::Dismiss { .. } => "assistant:action.dismiss.title",
            Action::Edit { .. } => "assistant:action.edit.title",
        })
    }

    pub fn glyph(&self) -> &'static rune::Icon {
        match self {
            Action::Pair { .. } => &I::LINK_2,
            Action::Dismiss { .. } => &I::EYE_OFF,
            Action::Edit { .. } => &I::SQUARE_PEN,
        }
    }
}

fn id_of(v: &Value) -> Option<String> {
    v.as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// An optional string argument: `Some(None)` absent or null, `None` the wrong type.
fn opt_str(v: &Value) -> Option<Option<String>> {
    match v {
        Value::Null => Some(None),
        Value::String(s) => Some(Some(s.clone())),
        _ => None,
    }
}

/// A positive decimal BTC string (`out_amount`), kept exactly as written.
fn btc_decimal(s: &str) -> bool {
    let mut parts = s.split('.');
    let whole = parts.next().unwrap_or("");
    let frac = parts.next();
    parts.next().is_none()
        && whole.chars().all(|c| c.is_ascii_digit())
        && frac
            .is_none_or(|f| !f.is_empty() && f.len() <= 11 && f.chars().all(|c| c.is_ascii_digit()))
        && (!whole.is_empty() || frac.is_some())
        && s.chars().any(|c| matches!(c, '1'..='9'))
}

fn edit_value_ok(field: &str, v: &Value) -> bool {
    match field {
        "tags" => v
            .as_array()
            .is_some_and(|a| a.len() <= 50 && a.iter().all(Value::is_string)),
        "excluded" => v.is_boolean(),
        "taxable" => v.is_boolean() || v.is_null(),
        "fiat_rate" | "fiat_value" => v.is_string() || v.is_number() || v.is_null(),
        _ => v.is_string() || v.is_null(),
    }
}

/// The tool call as a native card, or `None` (an unknown tool or arguments outside its schema:
/// the generic row shows them raw).
pub fn parse(name: &str, args: &Value) -> Option<Action> {
    let o = args.as_object()?;
    let only = |keys: &[&str]| o.keys().all(|k| keys.contains(&k.as_str()));
    match name {
        "ui.transfers.pair" => {
            if !only(&["tx_out", "tx_in", "kind", "policy", "notes", "out_amount"]) {
                return None;
            }
            let kind = opt_str(&args["kind"])?;
            let policy = opt_str(&args["policy"])?;
            let out_amount = opt_str(&args["out_amount"])?;
            if kind.as_deref().is_some_and(|k| !PAIR_KINDS.contains(&k))
                || policy.as_deref().is_some_and(|p| !POLICIES.contains(&p))
                || out_amount.as_deref().is_some_and(|a| !btc_decimal(a))
            {
                return None;
            }
            Some(Action::Pair {
                out: id_of(&args["tx_out"])?,
                inn: id_of(&args["tx_in"])?,
                kind,
                policy,
                out_amount,
                notes: opt_str(&args["notes"])?.filter(|n| !n.trim().is_empty()),
            })
        }
        "ui.transfers.dismiss" => {
            if !only(&["tx_out", "tx_in", "reason", "expires_in_days"]) {
                return None;
            }
            let days = match &args["expires_in_days"] {
                Value::Null => None,
                v => Some(v.as_u64()?),
            };
            Some(Action::Dismiss {
                out: id_of(&args["tx_out"])?,
                inn: id_of(&args["tx_in"])?,
                reason: opt_str(&args["reason"])?.filter(|r| !r.trim().is_empty()),
                days,
            })
        }
        "ui.transactions.metadata.update" => {
            let mut keys: Vec<&str> = EDIT_FIELDS.iter().map(|(f, _)| *f).collect();
            keys.extend(["transaction", "reason"]);
            if !only(&keys) {
                return None;
            }
            let mut fields = vec![];
            for (field, _) in EDIT_FIELDS {
                if let Some(v) = o.get(field) {
                    if !edit_value_ok(field, v) {
                        return None;
                    }
                    fields.push((field, v.clone()));
                }
            }
            if fields.is_empty() {
                return None;
            }
            Some(Action::Edit {
                transaction: id_of(&args["transaction"])?,
                fields,
                reason: opt_str(&args["reason"])?.filter(|r| !r.trim().is_empty()),
            })
        }
        _ => None,
    }
}

// ---- Undo ------------------------------------------------------------------------------------

/// The daemon kind an undo hint names (its wire name, `ui_transfers_unpair`, or the dotted kind),
/// when the desktop may send it itself: allowed, a book mutation, and not a plan to show first.
pub fn undo_kind(tool: &str) -> Option<&'static str> {
    if tool.is_empty() {
        return None;
    }
    let mut found = crate::kinds::ALLOWED_DAEMON_KINDS
        .iter()
        .copied()
        .filter(|k| *k == tool || k.replace('.', "_") == tool);
    let kind = found.next()?;
    // Two kinds behind one wire name: refuse rather than guess.
    if found.next().is_some() {
        return None;
    }
    (crate::kinds::is_book_mutation(kind) && !kind.ends_with(".plan")).then_some(kind)
}

/// The undo a finished call's result names, as the desktop would send it: (kind, arguments).
pub fn desktop_undo(c: &ToolCall) -> Option<(&'static str, Value)> {
    if c.status != ToolStatus::Done {
        return None;
    }
    let hint = &c.result.as_ref()?["data"]["undo"];
    let kind = undo_kind(hint["tool"].as_str()?)?;
    let args = hint["arguments"].as_object()?;
    // The binding is the open book's, never one the hint carries.
    if args.contains_key("expected_scope") {
        return None;
    }
    Some((kind, Value::Object(args.clone())))
}

/// The request Undo sends: the hinted kind and arguments, bound to the open book. `None` while
/// the open book's ids are unknown (an unbound write is never sent).
pub fn undo_request(app: &App, c: &ToolCall) -> Option<(&'static str, Value)> {
    let (kind, args) = desktop_undo(c)?;
    let book = book_now(app)?;
    let scope = crate::assistant::chat::scope_json(&book);
    Some((kind, crate::app::scope_args(kind, args, &scope)))
}

#[derive(Clone, Debug, PartialEq)]
pub enum UndoState {
    Running,
    Done,
    Failed(String),
}

#[derive(Clone, Debug, Default)]
pub struct Leg {
    pub loading: bool,
    pub failed: bool,
    pub tx: Option<Value>,
}

/// The cards' reads and undos.
#[derive(Default)]
pub struct State {
    /// Transactions read for the cards, by the id the tool named.
    pub legs: HashMap<String, Leg>,
    /// The book `legs` were read in.
    pub legs_book: Option<Book>,
    /// Leg reads in flight: request id → transaction id.
    pub leg_ids: Vec<(String, String)>,
    /// Undo by call id.
    pub undo: HashMap<String, UndoState>,
    /// Undos in flight: request id → call id.
    pub undo_ids: Vec<(String, String)>,
}

/// Undo pressed on the card of `call_id`.
pub fn start_undo(app: &mut App, call_id: &str) {
    if matches!(
        app.assistant.thread.actions.undo.get(call_id),
        Some(UndoState::Running | UndoState::Done)
    ) {
        return;
    }
    let Some(call) = app
        .assistant
        .messages
        .iter()
        .flat_map(|m| &m.tool_calls)
        .find(|c| c.call_id == call_id)
    else {
        return;
    };
    let Some((kind, args)) = undo_request(app, call) else {
        return;
    };
    // A SYNTHETIC scene has no book behind its calls: it shows the outcome, nothing is sent.
    if app.assistant.synthetic && app.assistant.turn.is_none() {
        app.assistant
            .thread
            .actions
            .undo
            .insert(call_id.to_owned(), UndoState::Done);
        return;
    }
    app.assistant
        .thread
        .actions
        .undo
        .insert(call_id.to_owned(), UndoState::Running);
    if let Some(id) = app.send(kind, args, "assistant.action.undo") {
        app.assistant
            .thread
            .actions
            .undo_ids
            .push((id, call_id.to_owned()));
    }
}

/// The one of `list` whose request is no longer pending: the reply being handled now.
fn finished(app: &App, list: &mut Vec<(String, String)>) -> Option<String> {
    let i = list
        .iter()
        .position(|(id, _)| !app.pending.contains_key(id))?;
    Some(list.remove(i).1)
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "assistant.action.undo" => {
            let mut list = std::mem::take(&mut app.assistant.thread.actions.undo_ids);
            let call = finished(app, &mut list);
            app.assistant.thread.actions.undo_ids = list;
            let Some(call) = call else { return };
            let state = match &r.error {
                Some(e) => UndoState::Failed(e.clone()),
                None => UndoState::Done,
            };
            let done = state == UndoState::Done;
            app.assistant.thread.actions.undo.insert(call, state);
            if done {
                // The book changed: the cards' transactions and every screen read again.
                app.assistant.thread.actions.legs.clear();
                app.refresh_reads();
            }
        }
        "assistant.action.leg" => {
            let mut list = std::mem::take(&mut app.assistant.thread.actions.leg_ids);
            let id = finished(app, &mut list);
            app.assistant.thread.actions.leg_ids = list;
            let Some(id) = id else { return };
            let tx = r.data.get("transaction").filter(|v| v.is_object()).cloned();
            app.assistant.thread.actions.legs.insert(
                id,
                Leg {
                    loading: false,
                    failed: r.error.is_some() || tx.is_none(),
                    tx,
                },
            );
        }
        _ => {}
    }
}

/// Every frame: reads the transactions the cards name, once per book.
pub fn poll(app: &mut App) -> bool {
    let book = book_now(app);
    let st = &mut app.assistant.thread.actions;
    let mut changed = false;
    // A SYNTHETIC scene seeds its transactions; they belong to no book.
    if st.legs_book != book && !app.assistant.synthetic {
        st.legs.clear();
        st.leg_ids.clear();
        st.legs_book = book.clone();
        changed = true;
    }
    if book.is_none() {
        return changed;
    }
    let mut want: Vec<String> = vec![];
    let consents = app
        .assistant
        .pending_consents
        .iter()
        .filter_map(|r| parse(&r.name, &r.arguments_preview));
    let applied = app
        .assistant
        .messages
        .iter()
        .flat_map(|m| &m.tool_calls)
        .filter(|c| c.status == ToolStatus::Done)
        .filter_map(|c| parse(&c.name, &c.arguments));
    for a in consents.chain(applied) {
        for id in a.tx_ids() {
            if !app.assistant.thread.actions.legs.contains_key(id) && !want.iter().any(|w| w == id)
            {
                want.push(id.to_owned());
            }
        }
    }
    let synthetic = app.assistant.synthetic;
    for id in want {
        changed = true;
        // A SYNTHETIC scene's ids are not in the book: the card names them as given.
        let leg = Leg {
            loading: !synthetic,
            ..Leg::default()
        };
        app.assistant.thread.actions.legs.insert(id.clone(), leg);
        if !synthetic
            && let Some(req) = app.send(
                "ui.transactions.resolve",
                json!({"query": id}),
                "assistant.action.leg",
            )
        {
            app.assistant.thread.actions.leg_ids.push((req, id));
        }
    }
    changed
}

// ---- Views -----------------------------------------------------------------------------------

/// A label/value row (the proposal's `<dl>` row).
fn fact(tk: &Tk, label: String, value: El) -> El {
    row()
        .w_full()
        .flex_wrap()
        .gap(tk.r(0.75))
        .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg).grow(1.0))
        .child(value)
}

fn value_text(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into())
        .font(tk.tsm(500.0).features("'tnum' 1"))
        .color(tk.fg)
        .wrap()
}

fn leg_view(app: &App, tk: &Tk, label: String, id: &str) -> El {
    let hide = app.prefs.hide_sensitive();
    let leg = app.assistant.thread.actions.legs.get(id);
    let found = leg
        .and_then(|l| l.tx.as_ref())
        .and_then(|tx| crate::custody::link_model::leg_from_tx(tx, None).map(|l| (tx, l)));
    let mut c = col()
        .w_full()
        .gap(2.0)
        .role("group")
        .label(label.clone())
        .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg));
    match found {
        Some((tx, l)) => {
            let sign = match l.side {
                crate::custody::link_model::LegSide::Send => "−",
                _ => "+",
            };
            let amount = super::approval::custody_msat(&json!(l.amount_msat.to_string()), &l.asset);
            let date = tx["date"].as_str().unwrap_or("").to_owned();
            let wallet = if l.wallet.is_empty() {
                t("assistant:action.leg.noWallet")
            } else {
                l.wallet.clone()
            };
            c = c.child(
                row()
                    .w_full()
                    .flex_wrap()
                    .gap(tk.r(0.5))
                    .child(
                        text(
                            [date.as_str(), wallet.as_str()]
                                .iter()
                                .filter(|s| !s.is_empty())
                                .copied()
                                .collect::<Vec<_>>()
                                .join(" · "),
                        )
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .grow(1.0)
                        .min_w(0.0)
                        .wrap(),
                    )
                    .child(crate::ui::sensitive(
                        format!("{sign}{amount}"),
                        tk.tsm(500.0).features("'tnum' 1"),
                        tk.fg,
                        hide,
                    )),
            );
        }
        None => {
            let (key, alert) = match leg {
                Some(l) if l.loading => ("assistant:action.leg.loading", false),
                Some(l) if l.failed => ("assistant:action.leg.missing", true),
                _ => ("", false),
            };
            if !key.is_empty() {
                let mut note = text(t(key)).font(tk.txs(400.0)).color(tk.fg).wrap();
                if alert {
                    note = note.role("alert");
                }
                c = c.child(note);
            }
        }
    }
    let shown = leg
        .and_then(|l| l.tx.as_ref())
        .and_then(|tx| tx["externalId"].as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(id);
    c.child(
        text(shown.to_owned())
            .font(tk.t2xs(400.0).mono())
            .color(tk.muted_fg)
            .wrap_anywhere(),
    )
}

/// A field's value in words.
fn field_text(field: &str, v: &Value, before: bool) -> String {
    match v {
        Value::Null => t(if before {
            "assistant:action.edit.unset"
        } else {
            "assistant:action.edit.cleared"
        }),
        Value::Bool(b) => t(if *b {
            "assistant:action.yes"
        } else {
            "assistant:action.no"
        }),
        Value::Array(a) if a.is_empty() => t("assistant:action.edit.noTags"),
        Value::Array(a) => a
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(", "),
        Value::String(s) if s.is_empty() => field_text(field, &Value::Null, before),
        Value::String(s) if field == "kind" => crate::txdetail::draft::kind_label(s),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Object(_) => String::new(),
    }
}

fn edit_row(app: &App, tk: &Tk, field: &str, after: &Value, current: Option<&Value>) -> El {
    let hide = app.prefs.hide_sensitive() && SENSITIVE_FIELDS.contains(&field);
    let shown =
        |s: String| crate::ui::sensitive(s, tk.tsm(500.0).features("'tnum' 1"), tk.fg, hide).wrap();
    let mut v = row().flex_wrap().gap(tk.r(0.5)).center_y();
    if let Some(b) = current {
        v = v.child(shown(field_text(field, b, true))).child(
            ic(&I::ARROW_RIGHT, tk.r(0.875), tk.muted_fg)
                .label(t("assistant:review.effects.becomes")),
        );
    }
    fact(
        tk,
        t(&format!("assistant:action.edit.field.{field}")),
        v.child(shown(field_text(field, after, false))),
    )
}

/// What the change does, in plain terms. `pending`: before it ran (current values shown).
pub fn body(app: &App, tk: &Tk, a: &Action, pending: bool) -> El {
    let hide = app.prefs.hide_sensitive();
    let mut c = col().w_full().gap(tk.r(0.625));
    let reason = match a {
        Action::Pair {
            out,
            inn,
            kind,
            policy,
            out_amount,
            notes,
        } => {
            c = c
                .child(leg_view(app, tk, t("assistant:action.leg.from"), out))
                .child(leg_view(app, tk, t("assistant:action.leg.to"), inn));
            let kind_key = crate::custody::link_model::kind_key(kind.as_deref());
            c = c.child(fact(
                tk,
                t("assistant:action.pair.kind"),
                value_text(tk, t(&format!("review:transfersPage.kind.{kind_key}"))),
            ));
            let policy = match policy.as_deref() {
                Some("carrying-value") => t("review:transfersPage.policy.carrying"),
                Some(_) => t("review:transfersPage.policy.taxable"),
                None => t("assistant:action.pair.policyDefault"),
            };
            c = c.child(fact(
                tk,
                t("assistant:action.pair.policy"),
                value_text(tk, policy),
            ));
            if let Some(amount) = out_amount {
                c = c.child(fact(
                    tk,
                    t("assistant:action.pair.partial"),
                    crate::ui::sensitive(
                        format!("{amount} BTC"),
                        tk.tsm(500.0).features("'tnum' 1"),
                        tk.fg,
                        hide,
                    ),
                ));
            }
            if let Some(n) = notes {
                c = c.child(fact(
                    tk,
                    t("assistant:action.pair.notes"),
                    value_text(tk, n),
                ));
            }
            None
        }
        Action::Dismiss {
            out,
            inn,
            reason,
            days,
        } => {
            c = c
                .child(leg_view(app, tk, t("assistant:action.leg.from"), out))
                .child(leg_view(app, tk, t("assistant:action.leg.to"), inn));
            let until = match days {
                None => t("assistant:action.dismiss.defaultDays"),
                Some(0) => t("assistant:action.dismiss.never"),
                Some(n) => tc("assistant:action.dismiss.days", *n as usize, &[]),
            };
            c = c.child(fact(
                tk,
                t("assistant:action.dismiss.until"),
                value_text(tk, until),
            ));
            reason.as_deref()
        }
        Action::Edit {
            transaction,
            fields,
            reason,
        } => {
            c = c.child(leg_view(
                app,
                tk,
                t("assistant:action.edit.transaction"),
                transaction,
            ));
            let tx = app
                .assistant
                .thread
                .actions
                .legs
                .get(transaction.as_str())
                .and_then(|l| l.tx.as_ref());
            for (field, after) in fields {
                let current = EDIT_FIELDS
                    .iter()
                    .find(|(f, _)| f == field)
                    .and_then(|(_, k)| *k)
                    .filter(|_| pending)
                    .and_then(|k| tx.map(|tx| &tx[k]));
                c = c.child(edit_row(app, tk, field, after, current));
            }
            reason.as_deref()
        }
    };
    if let Some(r) = reason {
        c = c.child(
            text(tr("assistant:action.reason", &[("reason", r)]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    c
}

/// The Undo control of an applied change: the button, its progress, or its outcome.
pub fn undo_control(app: &App, tk: &Tk, key: &str, c: &ToolCall) -> Option<El> {
    desktop_undo(c)?;
    let state = app.assistant.thread.actions.undo.get(&c.call_id);
    if state == Some(&UndoState::Done) {
        let s = t("assistant:action.undo.done");
        return Some(
            row()
                .gap(tk.r(0.25))
                .center_y()
                .role("status")
                .label(s.clone())
                .child(ic(&I::UNDO_2, tk.r(0.75), tk.muted_fg).a11y_hidden())
                .child(text(s).font(tk.txs(400.0)).color(tk.muted_fg).wrap()),
        );
    }
    let running = state == Some(&UndoState::Running);
    let enabled = !running && book_now(app).is_some();
    let mut b = crate::ui::button::button_busy(
        tk,
        format!("{key}-undo"),
        crate::txdetail::widgets::Variant::Outline,
        crate::txdetail::widgets::Size::Sm,
        running,
        Some(&I::UNDO_2),
        &t(if running {
            "assistant:action.undo.running"
        } else {
            "assistant:action.undo.action"
        }),
        enabled,
    )
    .label(t("assistant:action.undo.label"));
    if enabled {
        let id = c.call_id.clone();
        b = b.on_press(move |a: &mut App| start_undo(a, &id));
    }
    let mut r = row().flex_wrap().gap(tk.r(0.5)).center_y().child(b);
    if let Some(UndoState::Failed(e)) = state {
        r = r.child(
            text(tr("assistant:action.undo.failed", &[("message", e)]))
                .font(tk.txs(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    Some(r)
}

/// "Applied with the assistant", with Undo where the desktop can send it.
pub fn applied_line(app: &App, tk: &Tk, key: &str, c: &ToolCall) -> El {
    let mut line = row().w_full().flex_wrap().gap(tk.r(0.5)).center_y().child(
        row()
            .gap(tk.r(0.25))
            .center_y()
            .child(ic(&I::SPARKLES, tk.r(0.75), tk.muted_fg).a11y_hidden())
            .child(
                text(t("assistant:action.applied"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
    );
    if let Some(u) = undo_control(app, tk, key, c) {
        line = line.child(u);
    }
    line
}

/// Whether a finished call shows as its own card (a native change, or one the desktop can undo)
/// rather than inside the collapsed work log.
pub fn shows_applied(c: &ToolCall) -> bool {
    c.status == ToolStatus::Done
        && (parse(&c.name, &c.arguments).is_some() || desktop_undo(c).is_some())
}

/// A finished native change: what it did, "Applied with the assistant" and Undo.
pub fn applied_card(app: &App, tk: &Tk, c: &ToolCall) -> Option<El> {
    if c.status != ToolStatus::Done {
        return None;
    }
    let a = parse(&c.name, &c.arguments)?;
    let key = format!("asst-action-{}", c.call_id);
    let title = a.title();
    Some(
        col()
            .key(key.clone())
            .w_full()
            .min_w(0.0)
            .mb(tk.r(0.5))
            .gap(tk.r(0.75))
            .padding(tk.r(1.0))
            .radius(tk.r(0.5))
            .fill(tk.card)
            .border(1.0, tk.border.alpha(tk.border.a * 0.7))
            .border_layout()
            .role("region")
            .label(format!("{title}: {}", t("assistant:action.applied")))
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(ic(a.glyph(), tk.r(1.0), tk.muted_fg).a11y_hidden())
                    .child(
                        text(title)
                            .font(tk.tsm(500.0))
                            .color(tk.fg)
                            .role("heading")
                            .grow(1.0)
                            .min_w(0.0)
                            .wrap(),
                    ),
            )
            .child(body(app, tk, &a, false))
            .child(applied_line(app, tk, &key, c)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tests::quiet_app;

    fn done(name: &str, args: Value, data: Value) -> ToolCall {
        ToolCall {
            call_id: "call-1".into(),
            name: name.into(),
            arguments: args,
            kind_class: "mutating".into(),
            needs_consent: true,
            status: ToolStatus::Done,
            approval: Some("required".into()),
            result: Some(json!({"kind": name, "data": data})),
            reason: None,
            review_preview: None,
            segment_id: None,
        }
    }

    fn with_book(app: &mut App) {
        app.reads.insert(
            "ui.profiles.snapshot".into(),
            Reply {
                kind: "ui.profiles.snapshot".into(),
                data: json!({"activeWorkspaceId": "w", "activeProfileId": "p"}),
                error: None,
                error_detail: Value::Null,
                elapsed: Default::default(),
            },
        );
    }

    #[test]
    fn pair_arguments_parse_and_fail_closed() {
        let a = parse(
            "ui.transfers.pair",
            &json!({"tx_out": "t-out", "tx_in": "t-in", "kind": "peg-in", "policy": "carrying-value", "out_amount": "0.05"}),
        )
        .expect("a documented pair");
        assert_eq!(a.tx_ids(), vec!["t-out", "t-in"]);
        assert!(matches!(a, Action::Pair { ref kind, .. } if kind.as_deref() == Some("peg-in")));
        for bad in [
            json!({"tx_out": "t-out"}),
            json!({"tx_out": "t-out", "tx_in": " "}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "policy": "guess"}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "kind": "sell"}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "out_amount": "-0.1"}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "out_amount": "0"}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "out_amount": "0.000000000001"}),
            json!({"tx_out": "t-out", "tx_in": "t-in", "extra": true}),
        ] {
            assert!(parse("ui.transfers.pair", &bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn dismiss_and_edit_arguments_parse() {
        let d = parse(
            "ui.transfers.dismiss",
            &json!({"tx_out": "a", "tx_in": "b", "expires_in_days": 0}),
        );
        assert!(matches!(d, Some(Action::Dismiss { days: Some(0), .. })));
        assert!(
            parse(
                "ui.transfers.dismiss",
                &json!({"tx_out": "a", "tx_in": "b", "expires_in_days": -1})
            )
            .is_none()
        );
        let e = parse(
            "ui.transactions.metadata.update",
            &json!({"transaction": "t", "note": "Cold storage", "kind": "income", "taxable": null, "reason": "Owner said so"}),
        )
        .expect("an edit");
        let Action::Edit { fields, reason, .. } = e else {
            panic!("an edit")
        };
        assert_eq!(
            fields.iter().map(|(f, _)| *f).collect::<Vec<_>>(),
            vec!["note", "kind", "taxable"],
            "fields in the card's order"
        );
        assert_eq!(reason.as_deref(), Some("Owner said so"));
        // Nothing to change, a wrong type, or an unknown field: the generic row.
        for bad in [
            json!({"transaction": "t"}),
            json!({"transaction": "t", "excluded": "yes"}),
            json!({"transaction": "t", "tags": "a"}),
            json!({"transaction": "t", "note": "x", "debug": true}),
        ] {
            assert!(
                parse("ui.transactions.metadata.update", &bad).is_none(),
                "{bad}"
            );
        }
        assert!(parse("ui.transfers.rules.create", &json!({})).is_none());
    }

    #[test]
    fn field_values_read_in_words() {
        assert_eq!(
            field_text("note", &json!(null), false),
            t("assistant:action.edit.cleared")
        );
        assert_eq!(
            field_text("note", &json!(""), true),
            t("assistant:action.edit.unset")
        );
        assert_eq!(field_text("tags", &json!(["a", "b"]), false), "a, b");
        assert_eq!(
            field_text("excluded", &json!(true), false),
            t("assistant:action.yes")
        );
        assert_eq!(
            field_text("fiat_rate", &json!("58210.40"), false),
            "58210.40"
        );
    }

    #[test]
    fn undo_hints_map_to_allowed_book_mutations_only() {
        assert_eq!(
            undo_kind("ui_transfers_unpair"),
            Some("ui.transfers.unpair")
        );
        assert_eq!(
            undo_kind("ui_transactions_history_revert"),
            Some("ui.transactions.history.revert")
        );
        assert_eq!(
            undo_kind("ui_transfers_dismissals_delete"),
            Some("ui.transfers.dismissals.delete")
        );
        assert_eq!(undo_kind("ui.transfers.pair"), Some("ui.transfers.pair"));
        // Plan-bound inverses and reads: the model shows the plan first.
        for plan in [
            "ui_transfers_components_plan",
            "ui_custody_review_plan",
            "ui_imports_rollback_plan",
            "ui_transactions_list",
        ] {
            assert_eq!(undo_kind(plan), None, "{plan}");
        }
        // Every inverse `crates/ai/src/undo.rs` names: the direct ones get Undo.
        for (hint, direct) in [
            ("ui_transfers_unpair", true),
            ("ui_transfers_pair", true),
            ("ui_transfers_dismissals_delete", true),
            ("ui_transfers_dismiss", true),
            ("ui_transfers_rules_delete", true),
            ("ui_transfers_rules_create", true),
            ("ui_transfers_rules_set_enabled", true),
            ("ui_transfers_payouts_delete", true),
            ("ui_saved_views_delete", true),
            ("ui_loans_unmark", true),
            ("ui_transactions_history_revert", true),
            ("ui_chain_analysis_cases_delete", true),
            ("ui_chain_analysis_watches_delete", true),
        ] {
            assert_eq!(undo_kind(hint).is_some(), direct, "{hint}");
        }
        // Not in the desktop's allowlist.
        assert_eq!(undo_kind("ui_shell_exec"), None);
        assert_eq!(undo_kind(""), None);
    }

    #[test]
    fn undo_sends_the_hinted_kind_bound_to_the_open_book() {
        let mut app = quiet_app();
        let call = done(
            "ui.transfers.pair",
            json!({"tx_out": "a", "tx_in": "b"}),
            json!({"id": "pair-9", "undo": {"tool": "ui_transfers_unpair", "arguments": {"pair_id": "pair-9"}}}),
        );
        // No open book known: nothing is sent unbound.
        assert!(undo_request(&app, &call).is_none());
        with_book(&mut app);
        let (kind, args) = undo_request(&app, &call).expect("an undo");
        assert_eq!(kind, "ui.transfers.unpair");
        assert_eq!(
            args,
            json!({"pair_id": "pair-9", "expected_scope": {"workspace_id": "w", "profile_id": "p"}})
        );
        let mut m =
            crate::assistant::model::Msg::new("a1", crate::assistant::model::Role::Assistant, "");
        m.tool_calls = vec![call];
        app.assistant.messages = vec![m];
        start_undo(&mut app, "call-1");
        assert_eq!(
            app.assistant.thread.actions.undo.get("call-1"),
            Some(&UndoState::Running)
        );
        assert_eq!(
            app.pending
                .values()
                .filter(|(tag, k, _)| k == "ui.transfers.unpair" && *tag == "assistant.action.undo")
                .count(),
            1
        );
        // A second press while it runs sends nothing more.
        start_undo(&mut app, "call-1");
        assert_eq!(app.assistant.thread.actions.undo_ids.len(), 1);
    }

    #[test]
    fn no_undo_for_a_kind_the_desktop_may_not_send() {
        let plan = done(
            "ui.transfers.components.apply",
            json!({}),
            json!({"undo": {"tool": "ui_transfers_components_plan", "arguments": {"action": "supersede", "component_id": "c1"}}}),
        );
        assert!(desktop_undo(&plan).is_none());
        let foreign = done(
            "ui.transfers.pair",
            json!({"tx_out": "a", "tx_in": "b"}),
            json!({"undo": {"tool": "ui_attachments_add_from_chat", "arguments": {"attachment": "a"}}}),
        );
        assert!(desktop_undo(&foreign).is_none());
        // A hint cannot pick the book it runs in.
        let scoped = done(
            "ui.transfers.pair",
            json!({"tx_out": "a", "tx_in": "b"}),
            json!({"undo": {"tool": "ui_transfers_unpair", "arguments": {"pair_id": "x", "expected_scope": {"workspace_id": "o", "profile_id": "o"}}}}),
        );
        assert!(desktop_undo(&scoped).is_none());
        let mut running = scoped.clone();
        running.status = ToolStatus::Running;
        assert!(!shows_applied(&running));
        // A native change shows as its own card even without an undo.
        assert!(shows_applied(&foreign));
    }

    #[test]
    fn undo_replies_settle_the_card() {
        let mut app = quiet_app();
        app.assistant.thread.actions.undo_ids = vec![("r-gone".into(), "call-1".into())];
        app.assistant
            .thread
            .actions
            .undo
            .insert("call-1".into(), UndoState::Running);
        let reply = |error: Option<&str>| Reply {
            kind: "ui.transfers.unpair".into(),
            data: json!({}),
            error: error.map(str::to_owned),
            error_detail: Value::Null,
            elapsed: Default::default(),
        };
        on_reply(&mut app, "assistant.action.undo", &reply(Some("not found")));
        assert_eq!(
            app.assistant.thread.actions.undo.get("call-1"),
            Some(&UndoState::Failed("not found".into()))
        );
        app.assistant.thread.actions.undo_ids = vec![("r-gone".into(), "call-1".into())];
        on_reply(&mut app, "assistant.action.undo", &reply(None));
        assert_eq!(
            app.assistant.thread.actions.undo.get("call-1"),
            Some(&UndoState::Done)
        );
    }

    /// The actions scene's cards alone, headless: what the thread draws for them.
    fn cards_view(app: &App, _cx: &mut rune::Cx<App>) -> El {
        crate::i18n::set_lang(app.prefs.lang);
        let tk = app.tk();
        let mut layer = vec![];
        let mut c = col().w_full().padding(16.0);
        for r in &app.assistant.pending_consents {
            c = c.child(super::super::approval::pending_row(app, &tk, r, &mut layer));
        }
        for call in app.assistant.messages.iter().flat_map(|m| &m.tool_calls) {
            if call.status == ToolStatus::Done {
                c = c.child(super::super::tool::card(app, &tk, call));
            }
        }
        rune_kit::root(c, layer)
    }

    fn cards_app() -> App {
        let mut app = quiet_app();
        with_book(&mut app);
        app.prefs.ai_features = true;
        crate::assistant::synthetic::load_actions(&mut app);
        app
    }

    fn cards(lang: crate::i18n::Lang) -> rune::Runtime<App> {
        let mut rt = rune::App::new(rune::AppSpec::new(
            "cards",
            (900.0, 3200.0),
            cards_app,
            cards_view,
        ))
        .headless_runtime((900.0, 3200.0), rune::Env::new(true, 1.0));
        rt.state.prefs.lang = lang;
        for _ in 0..3 {
            rt.frame(1.0 / 60.0);
        }
        rt
    }

    fn press(rt: &mut rune::Runtime<App>, key: &str) {
        let (x, y) = rt.find(key).unwrap_or_else(|| panic!("{key} is drawn"));
        rt.pointer_move(x, y);
        rt.pointer_down(x, y);
        rt.pointer_up(x, y);
        rt.frame(1.0 / 60.0);
    }

    #[test]
    fn the_cards_are_labelled_and_translated() {
        for lang in [crate::i18n::Lang::En, crate::i18n::Lang::De] {
            let rt = cards(lang);
            let found = crate::checks::a11y_findings(&rt.accessibility_json(), &rt.elements());
            assert!(found.is_empty(), "{lang:?}: {found:?}");
            let missing: Vec<String> = crate::i18n::missing()
                .into_iter()
                .filter(|k| k.contains("assistant:action."))
                .collect();
            assert!(missing.is_empty(), "{lang:?}: {missing:?}");
            // Plain terms, not raw arguments: the legs' wallets and the native titles show.
            let tree = rt.accessibility_json().to_string();
            for words in ["Cold wallet", "Kraken"] {
                assert!(tree.contains(words), "{lang:?}: {words}");
            }
        }
    }

    #[test]
    fn apply_answers_the_consent_for_its_call() {
        let mut rt = cards(crate::i18n::Lang::En);
        // A live answer waiting on the approval (not the scene's local stand-in).
        rt.state.assistant.synthetic = false;
        rt.state.assistant.turn = Some(crate::assistant::chat::Turn {
            request_id: "req-1".into(),
            assistant_id: "a7".into(),
            user_id: "u7".into(),
            parser: Default::default(),
            observed: Default::default(),
            no_persist: false,
            provider: "ollama".into(),
            model: "m".into(),
            started: true,
            prompt: String::new(),
            generated: false,
            new_files: vec![],
        });
        rt.frame(1.0 / 60.0);
        press(&mut rt, "asst-approval-x1-approve");
        let st = &rt.state;
        assert_eq!(
            st.assistant
                .in_flight
                .consents
                .iter()
                .map(|(_, call, decision, target)| (
                    call.as_str(),
                    decision.as_str(),
                    target.as_str()
                ))
                .collect::<Vec<_>>(),
            vec![("x1", "allow_once", "req-1")]
        );
        assert!(
            st.pending
                .values()
                .any(|(tag, k, _)| k == "ai.tool_call.consent" && *tag == "assistant.consent")
        );
        assert!(
            st.assistant
                .pending_consents
                .iter()
                .all(|c| c.call_id != "x1")
        );
        // Undo on the applied link sends the hinted kind; the plan-bound one has no button.
        assert!(rt.find("asst-tool-y3-undo").is_none());
        press(&mut rt, "asst-action-y1-undo");
        assert!(
            rt.state
                .pending
                .values()
                .any(|(tag, k, _)| k == "ui.transfers.unpair" && *tag == "assistant.action.undo")
        );
        assert_eq!(
            rt.state.assistant.thread.actions.undo.get("y1"),
            Some(&UndoState::Running)
        );
    }

    #[test]
    fn cards_read_their_transactions_once_per_book() {
        let mut app = quiet_app();
        with_book(&mut app);
        app.assistant.pending_consents = vec![crate::assistant::model::Consent {
            call_id: "c1".into(),
            name: "ui.transfers.pair".into(),
            summary: String::new(),
            arguments_preview: json!({"tx_out": "a", "tx_in": "b"}),
            review_preview: None,
            reversible: true,
            egresses: false,
            can_allow_session: true,
        }];
        assert!(poll(&mut app));
        let reads = |app: &App| {
            app.pending
                .values()
                .filter(|(tag, k, _)| {
                    k == "ui.transactions.resolve" && *tag == "assistant.action.leg"
                })
                .count()
        };
        assert_eq!(reads(&app), 2);
        assert!(!poll(&mut app));
        assert_eq!(reads(&app), 2, "each transaction is read once");
    }
}
