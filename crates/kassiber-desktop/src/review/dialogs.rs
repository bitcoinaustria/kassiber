//! The Review screen's reviewed steps: unpairing picked pairs (`QuarantineFix.tsx`), a proposal
//! decided on a card (`ReviewProposalDialog.tsx` for accepted and entered prices), and pairing
//! exact matches in bulk (`ReviewTransfers`' confirmation). Every change is previewed by the
//! daemon on a copy of the book and applied, exactly as previewed, once confirmed.

use rune::{El, col, row, text};
use serde_json::{Value, json};

use super::kit::{self, Var};
use super::model::{self, Snap, s};
use super::state::{self, Phase, PropKind};
use crate::app::{App, Reply, ToastTone};
use crate::i18n::{t, tc, tr};
use crate::theme::{MASK, Tk};

const LISTED: usize = 5;

/// Escape closes the open dialog (not while it applies).
pub fn escape(app: &mut App) -> bool {
    if app.review.rv.fix.is_some() {
        fix_close(app);
        return true;
    }
    if app.review.rv.prop.is_some() {
        prop_close(app);
        return true;
    }
    if app.review.rv.bulk.is_some() {
        state::close_bulk(app);
        return true;
    }
    false
}

fn msg_of(r: &Reply) -> String {
    r.error.clone().unwrap_or_default()
}

// ---- Unpair picked pairs ---------------------------------------------------------------------------

/// `check`: re-read the book, then preview the picks still held as picked.
pub fn fix_check(app: &mut App) {
    let Some(f) = app.review.rv.fix.as_mut() else {
        return;
    };
    f.phase = Phase::Checking;
    if f.scope.is_none() {
        f.phase = Phase::Failed(t("journals:quarantine.fix.noScope"));
        return;
    }
    state::new_fix_token(app);
    state::refetch(app, Some(state::Waiter::Fix));
}

/// The re-read is back (`ok` false when it failed).
pub fn fix_refreshed(app: &mut App, ok: bool) {
    if app
        .review
        .rv
        .fix
        .as_ref()
        .is_none_or(|f| !matches!(f.phase, Phase::Checking))
    {
        return;
    }
    let fresh: Option<Snap> = if ok {
        app.review.rv.q.attention()
    } else {
        None
    };
    let f = app.review.rv.fix.as_mut().unwrap();
    let Some(fresh) = fresh else {
        f.phase = Phase::Failed(t("journals:quarantine.fix.recheckFailed"));
        return;
    };
    if !state::scopes_match(fresh.book_scope().as_ref(), f.scope.as_ref()) {
        f.phase = Phase::Failed(t("journals:quarantine.fix.bookChanged"));
        return;
    }
    let (current, cleared, changed) = model::reconcile_picks(&f.items, &fresh.items);
    f.left = cleared.len();
    f.targets = current.clone();
    if !changed.is_empty() {
        f.phase = Phase::Changed(changed.len());
        return;
    }
    if current.is_empty() {
        f.phase = Phase::Nothing;
        return;
    }
    let scope = f.scope.clone().unwrap();
    let token = state::fix_token(app).unwrap();
    state::send_t(
        app,
        "ui.review.cases",
        json!({ "limit": 1, "expected_scope": scope }),
        "review.fix.cases",
        token,
    );
}

pub fn on_fix_reply(app: &mut App, tag: &str, r: &Reply) {
    let tag: &'static str = match tag {
        "review.fix.cases" => "review.fix.cases",
        "review.fix.plan" => "review.fix.plan",
        _ => "review.fix.apply",
    };
    let token = state::take_t(app, tag);
    if state::fix_token(app) != Some(token) {
        // The dialog was closed (or replaced) meanwhile: an applied fix still changed the book.
        if tag == "review.fix.apply" && r.error.is_none() {
            app.refresh_reads();
            state::refetch_all(app);
        }
        return;
    }
    let stale = state::stale(r);
    let f = app.review.rv.fix.as_mut().unwrap();
    match tag {
        "review.fix.cases" => {
            if r.error.is_some() {
                f.phase = if stale {
                    Phase::Stale
                } else {
                    Phase::Failed(msg_of(r))
                };
                return;
            }
            let ops = model::fix_operations(&f.targets);
            let scope = f.scope.clone();
            let v = r.data["input_version"].as_u64().unwrap_or(0);
            state::send_t(
                app,
                "ui.review.plan",
                json!({ "operations": ops, "expected_input_version": v, "expected_scope": scope }),
                "review.fix.plan",
                token,
            );
        }
        "review.fix.plan" => {
            f.phase = if r.error.is_some() {
                if stale {
                    Phase::Stale
                } else {
                    Phase::Failed(msg_of(r))
                }
            } else if !model::artifact_ok(&r.data) {
                Phase::Failed("invalid review preview".into())
            } else {
                Phase::Ready(r.data.clone(), model::uuid())
            };
        }
        _ => {
            let Phase::Applying(artifact, key) = f.phase.clone() else {
                return;
            };
            if r.error.is_some() {
                f.phase = if stale {
                    Phase::Stale
                } else {
                    Phase::Uncertain(artifact, key, msg_of(r))
                };
                return;
            }
            let n = artifact["operations"].as_array().map(Vec::len).unwrap_or(0);
            let remaining = artifact["after"]["quarantine_count"].to_string();
            app.review.rv.fix = None;
            app.toast(
                ToastTone::Success,
                &tc("journals:quarantine.fix.doneTitle", n, &[]),
                Some(tr(
                    "journals:quarantine.fix.doneBody",
                    &[("remaining", &remaining)],
                )),
                Some("quarantine-fix"),
            );
            app.refresh_reads();
            state::refetch_all(app);
        }
    }
}

pub fn fix_confirm(app: &mut App) {
    let Some(f) = app.review.rv.fix.as_mut() else {
        return;
    };
    let (artifact, key) = match &f.phase {
        Phase::Ready(a, k) | Phase::Uncertain(a, k, _) => (a.clone(), k.clone()),
        _ => return,
    };
    let art_scope =
        json!({ "workspace_id": artifact["workspace_id"], "profile_id": artifact["profile_id"] });
    if !state::scopes_match(Some(&art_scope), f.scope.as_ref()) {
        f.phase = Phase::Failed(t("journals:quarantine.fix.bookChanged"));
        return;
    }
    f.phase = Phase::Applying(artifact.clone(), key.clone());
    let scope = f.scope.clone();
    let token = state::fix_token(app).unwrap();
    state::send_t(
        app,
        "ui.review.apply",
        json!({ "expected_scope": scope, "artifact": artifact, "idempotency_key": key }),
        "review.fix.apply",
        token,
    );
}

pub fn fix_retry(app: &mut App) {
    match app.review.rv.fix.as_ref().map(|f| f.phase.clone()) {
        Some(Phase::Uncertain(..)) => fix_confirm(app),
        Some(Phase::Stale | Phase::Failed(_)) => fix_check(app),
        _ => {}
    }
}

pub fn fix_close(app: &mut App) {
    if app
        .review
        .rv
        .fix
        .as_ref()
        .is_some_and(|f| matches!(f.phase, Phase::Applying(..)))
    {
        return;
    }
    app.review.rv.fix = None;
}

// ---- Review proposals (accept / entered prices) -----------------------------------------------------

pub fn prop_check(app: &mut App) {
    let Some(p) = app.review.rv.prop.as_mut() else {
        return;
    };
    p.phase = Phase::Checking;
    let scope = p.scope.clone();
    let token = state::new_prop_token(app);
    state::send_t(
        app,
        "ui.review.cases",
        json!({ "limit": 1, "expected_scope": scope }),
        "review.prop.cases",
        token,
    );
}

pub fn on_prop_reply(app: &mut App, tag: &str, r: &Reply) {
    let tag: &'static str = match tag {
        "review.prop.cases" => "review.prop.cases",
        "review.prop.plan" => "review.prop.plan",
        _ => "review.prop.apply",
    };
    let token = state::take_t(app, tag);
    if state::prop_token(app) != Some(token) {
        // The dialog was closed (or replaced) meanwhile: an applied proposal still changed the
        // book.
        if tag == "review.prop.apply" && r.error.is_none() {
            app.refresh_reads();
            state::refetch_all(app);
        }
        return;
    }
    let stale = state::stale(r);
    let p = app.review.rv.prop.as_mut().unwrap();
    match tag {
        "review.prop.cases" => {
            if r.error.is_some() {
                p.phase = if stale {
                    Phase::Stale
                } else {
                    Phase::Failed(msg_of(r))
                };
                return;
            }
            let v = r.data["input_version"].as_u64().unwrap_or(0);
            let args = json!({ "operations": p.ops, "expected_input_version": v, "expected_scope": p.scope });
            state::send_t(app, "ui.review.plan", args, "review.prop.plan", token);
        }
        "review.prop.plan" => {
            p.phase = if r.error.is_some() {
                if stale {
                    Phase::Stale
                } else {
                    Phase::Failed(msg_of(r))
                }
            } else if !model::artifact_ok(&r.data) {
                Phase::Failed("invalid review preview".into())
            } else {
                Phase::Ready(r.data.clone(), model::uuid())
            };
        }
        _ => {
            let Phase::Applying(artifact, key) = p.phase.clone() else {
                return;
            };
            if r.error.is_some() {
                p.phase = if stale {
                    Phase::Stale
                } else {
                    Phase::Uncertain(artifact, key, msg_of(r))
                };
                return;
            }
            let kind = p.kind.clone();
            app.review.rv.prop = None;
            let remaining = artifact["after"]["quarantine_count"].to_string();
            let (title, key) = match &kind {
                PropKind::Accept => (
                    tc(
                        "journals:quarantine.accept.doneTitle",
                        accepted_count(&artifact),
                        &[],
                    ),
                    "quarantine-accept-prices",
                ),
                PropKind::Prices { lines, .. } => (
                    tc("journals:quarantine.price.doneTitle", lines.len(), &[]),
                    "quarantine-enter-prices",
                ),
            };
            app.toast(
                ToastTone::Success,
                &title,
                Some(tr(
                    "journals:quarantine.fix.doneBody",
                    &[("remaining", &remaining)],
                )),
                Some(key),
            );
            if let PropKind::Prices { card, .. } = &kind {
                let c = state::card_mut(app, card);
                c.entering = false;
                c.entries.clear();
                c.checked = false;
            }
            app.refresh_reads();
            state::refetch_all(app);
        }
    }
}

fn accepted_count(a: &Value) -> usize {
    a["operations"][0]["transaction_ids"]
        .as_array()
        .map(Vec::len)
        .unwrap_or(0)
}

pub fn prop_confirm(app: &mut App) {
    let Some(p) = app.review.rv.prop.as_mut() else {
        return;
    };
    let (artifact, key) = match &p.phase {
        Phase::Ready(a, k) | Phase::Uncertain(a, k, _) => (a.clone(), k.clone()),
        _ => return,
    };
    p.phase = Phase::Applying(artifact.clone(), key.clone());
    let scope = p.scope.clone();
    let token = state::prop_token(app).unwrap();
    state::send_t(
        app,
        "ui.review.apply",
        json!({ "expected_scope": scope, "artifact": artifact, "idempotency_key": key }),
        "review.prop.apply",
        token,
    );
}

pub fn prop_retry(app: &mut App) {
    match app.review.rv.prop.as_ref().map(|p| p.phase.clone()) {
        Some(Phase::Uncertain(..)) => prop_confirm(app),
        Some(Phase::Stale | Phase::Failed(_)) => prop_check(app),
        _ => {}
    }
}

pub fn prop_close(app: &mut App) {
    if app
        .review
        .rv
        .prop
        .as_ref()
        .is_some_and(|p| matches!(p.phase, Phase::Applying(..)))
    {
        return;
    }
    app.review.rv.prop = None;
}

// ---- Views -----------------------------------------------------------------------------------------

/// `ReviewTaxDeltaView`: the previewed tax change by year.
pub fn tax_delta(tk: &Tk, artifact: &Value, hidden: bool) -> Option<El> {
    let (tax, years) = model::tax_delta(artifact)?;
    let cur = s(&tax, "fiat_currency");
    let money = |v: f64| {
        if hidden {
            MASK.to_owned()
        } else {
            format!(
                "{}{}",
                if v > 0.0 { "+" } else { "" },
                crate::format::fiat(v, &cur)
            )
        }
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(0.25))
        .label(t("assistant:review.tax.title"))
        .child(
            text(t("assistant:review.tax.title"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        );
    if years.is_empty() {
        return Some(
            c.child(
                text(t("assistant:review.tax.unchanged"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    let at = s(&tax, "jurisdiction") == "AT";
    for y in &years {
        let num = |k: &str| s(y, k).parse::<f64>().unwrap_or(0.0);
        let mut parts = vec![
            format!(
                "{} {}",
                t("assistant:review.tax.realized"),
                money(num("realized_gain_loss"))
            ),
            format!(
                "{} {}",
                t("assistant:review.tax.income"),
                money(num("income"))
            ),
        ];
        if at {
            let year = y["year"].as_i64().unwrap_or(0);
            parts.push(if let Some(c) = y["estimated_tax_eur_cents"].as_f64() {
                format!(
                    "{} {}",
                    t("assistant:review.tax.indicative"),
                    money(c / 100.0)
                )
            } else if let Some(after) = tax["years"]
                .as_array()
                .and_then(|a| a.iter().find(|r| r["year"].as_i64() == Some(year)))
                .and_then(|r| r["indicative_tax"]["estimated_tax_eur_cents"].as_f64())
            {
                let amount = if hidden {
                    MASK.to_owned()
                } else {
                    crate::format::fiat(after / 100.0, &cur)
                };
                tr(
                    "assistant:review.tax.indicativeAfter",
                    &[("amount", &amount)],
                )
            } else {
                t("assistant:review.tax.withheld")
            });
        }
        c = c.child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .items_end()
                .flex_wrap()
                .child(
                    text(y["year"].to_string())
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                )
                .child(
                    text(parts.join(" · "))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.fg)
                        .wrap(),
                ),
        );
    }
    Some(c)
}

/// The effect box both dialogs share (`kb-surface-inset p-3 text-sm`, a live status region).
fn effect_box(
    tk: &Tk,
    key: &str,
    phase: &Phase,
    extra: Option<El>,
    fix_text: Option<String>,
) -> El {
    let mut b = kit::inset(tk)
        .key(key.to_owned())
        .padding(tk.r(0.75))
        .gap(tk.r(0.25))
        .role("status")
        .live_region(rune::LiveRegion::Polite);
    match phase {
        Phase::Ready(a, _) | Phase::Applying(a, _) => {
            let before = a["before"]["quarantine_count"].to_string();
            let after = a["after"]["quarantine_count"].to_string();
            b = b.child(
                text(tr(
                    "journals:quarantine.fix.effectQuarantine",
                    &[("before", &before), ("after", &after)],
                ))
                .font(tk.tsm(500.0).features("'tnum' 1"))
                .color(tk.fg),
            );
            let ready = a["after"]["report_ready"].as_bool() == Some(true);
            b = b.child(
                text(if ready {
                    t("journals:quarantine.fix.reportsReady")
                } else {
                    t("journals:quarantine.fix.reportsBlocked")
                })
                .font(tk.tsm(400.0))
                .color(if ready {
                    kit::emerald_text(tk)
                } else {
                    tk.muted_fg
                }),
            );
            if let Some(x) = fix_text {
                b = b.child(text(x).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
            }
            if let Some(e) = extra {
                b = b.gap(tk.r(0.75)).child(e);
            }
        }
        Phase::Checking => {
            b = b.child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(kit::spinner(tk, tk.r(1.0)))
                    .child(
                        text(t("journals:quarantine.fix.checking"))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        Phase::Stale => {
            b = b.child(
                text(t("journals:quarantine.fix.stale"))
                    .font(tk.tsm(400.0))
                    .color(tk.fg),
            )
        }
        Phase::Changed(n) => {
            b = b.child(
                text(tc("journals:quarantine.fix.changed", *n, &[]))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .role("alert")
                    .wrap(),
            )
        }
        Phase::Nothing => {}
        Phase::Failed(m) | Phase::Uncertain(_, _, m) => {
            b = b.child(
                text(m.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.destructive)
                    .role("alert")
                    .wrap(),
            )
        }
    }
    b
}

pub fn mount(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    if app.review.rv.fix.is_some() {
        fix_view(app, tk, layer);
    }
    if app.review.rv.prop.is_some() {
        prop_view(app, tk, layer);
    }
    if app.review.rv.bulk.is_some() {
        bulk_view(app, tk, layer);
    }
}

fn fix_view(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let f = app.review.rv.fix.as_ref().unwrap();
    let hidden = app.prefs.hide_sensitive();
    let picks = &f.items;
    let single = if picks.len() == 1 {
        picks[0].pair_legs()
    } else {
        None
    };
    let title = if single.is_some() {
        t("journals:quarantine.pair.confirmTitle")
    } else {
        tc("journals:quarantine.fix.title", picks.len(), &[])
    };
    let desc = match single {
        Some((o, i)) => crate::ui::sensitive(
            tr(
                "journals:quarantine.pair.confirmBody",
                &[
                    ("outWallet", &s(o, "wallet")),
                    ("inWallet", &s(i, "wallet")),
                ],
            ),
            tk.tsm(400.0),
            tk.muted_fg,
            hidden,
        )
        .wrap(),
        None => kit::dialog_desc(tk, tc("journals:quarantine.fix.body", picks.len(), &[])),
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(title.clone())
        .child(kit::dialog_header(tk, title, Some(desc)));
    let nothing = matches!(f.phase, Phase::Nothing)
        .then(|| tc("journals:quarantine.fix.nothing", picks.len(), &[]));
    let left = (f.left > 0 && f.phase.artifact().is_some())
        .then(|| tc("journals:quarantine.fix.leftOut", f.left, &[]));
    let mut effect = effect_box(tk, "rv-fix-effect", &f.phase, None, left);
    if let Some(n) = nothing {
        effect = effect.child(text(n).font(tk.tsm(400.0)).color(tk.fg).wrap());
    }
    c = c.child(effect);
    let shown = if f.targets.is_empty() {
        picks
    } else {
        &f.targets
    };
    let mut list = col()
        .w_full()
        .gap(tk.r(0.5))
        .max_h(tk.r(16.0))
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    for it in shown.iter().take(LISTED) {
        let Some((o, i)) = it.pair_legs() else {
            continue;
        };
        let mut li = col().w_full().gap(0.0).child(
            row()
                .w_full()
                .gap(tk.r(1.0))
                .items_end()
                .child(
                    crate::ui::sensitive(
                        format!(
                            "{} · {} → {}",
                            crate::format::short_date(o["occurred_at"].as_str()),
                            s(o, "wallet"),
                            s(i, "wallet")
                        ),
                        tk.tsm(400.0),
                        tk.fg,
                        hidden,
                    )
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0),
                )
                .child(crate::ui::sensitive(
                    model::sats_of((model::num(o, "amount_msat") / 1000.0).round()),
                    tk.tsm(400.0).features("'tnum' 1"),
                    tk.muted_fg,
                    hidden,
                )),
        );
        let review = &it.evidence()["pair_review"];
        if review.is_object() {
            let kind = review["kind"].as_str().unwrap_or("—").to_owned();
            let sent = model::sats_of((model::num(review, "out_amount_msat") / 1000.0).round());
            let recv = model::sats_of((model::num(review, "in_amount_msat") / 1000.0).round());
            li = li.child(
                crate::ui::sensitive(
                    tr(
                        "journals:quarantine.fix.reviewLine",
                        &[("kind", &kind), ("sent", &sent), ("received", &recv)],
                    ),
                    tk.txs(400.0),
                    tk.muted_fg,
                    hidden,
                )
                .wrap(),
            );
        }
        list = list.child(li);
    }
    c = c.child(list);
    if shown.len() > LISTED {
        c = c.child(
            text(tc(
                "journals:quarantine.pair.andMore",
                shown.len() - LISTED,
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    c = c.child(
        text(t("journals:quarantine.pair.confirmUndo"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
    );
    let applying = matches!(f.phase, Phase::Applying(..));
    let closing = matches!(f.phase, Phase::Changed(_) | Phase::Nothing);
    let mut footer = kit::dialog_footer(tk).child(
        kit::btn_md(
            tk,
            "rv-fix-cancel",
            Var::Outline,
            None,
            &if closing {
                t("journals:quarantine.fix.close")
            } else {
                t("journals:quarantine.pair.cancel")
            },
            !applying,
        )
        .on_press(fix_close),
    );
    let retry = matches!(
        f.phase,
        Phase::Uncertain(..) | Phase::Stale | Phase::Failed(_)
    );
    if retry {
        footer = footer.child(
            kit::btn_md(
                tk,
                "rv-fix-retry",
                Var::Default,
                None,
                &t("journals:quarantine.fix.retry"),
                true,
            )
            .on_press(fix_retry),
        );
    } else if !closing {
        let ready = matches!(f.phase, Phase::Ready(..));
        let label = if applying {
            t("journals:quarantine.fix.applying")
        } else if single.is_some() {
            t("journals:quarantine.pair.confirm")
        } else {
            tc(
                "journals:quarantine.fix.apply",
                if f.targets.is_empty() {
                    picks.len()
                } else {
                    f.targets.len()
                },
                &[],
            )
        };
        let mut b = busy_md(tk, "rv-fix-confirm", applying, &label, ready);
        if ready {
            b = b.on_press(fix_confirm);
        }
        footer = footer.child(b);
    }
    c = c.child(footer);
    kit::dialog(app, tk, layer, "rv-fix-dialog", 42.0, c, fix_close);
}

/// A default-size button with the spinner while busy.
fn busy_md(tk: &Tk, key: &str, busy: bool, label: &str, enabled: bool) -> El {
    if busy {
        kit::btn_busy(tk, key, Var::Default, true, None, label, false)
            .h(tk.r(2.25))
            .px(tk.r(0.75))
    } else {
        kit::btn_md(tk, key, Var::Default, None, label, enabled)
    }
}

fn prop_view(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let p = app.review.rv.prop.as_ref().unwrap();
    let hidden = app.prefs.hide_sensitive();
    let artifact = p.phase.artifact();
    let (title, body, foot, confirm, applying_label) = match &p.kind {
        PropKind::Accept => (
            artifact
                .map(|a| tc("journals:quarantine.accept.title", accepted_count(a), &[]))
                .unwrap_or_else(|| t("journals:quarantine.accept.titlePending")),
            t("journals:quarantine.accept.body"),
            t("journals:quarantine.accept.undo"),
            artifact
                .map(|a| tc("journals:quarantine.accept.confirm", accepted_count(a), &[]))
                .unwrap_or_else(|| t("journals:quarantine.accept.confirmPending")),
            t("journals:quarantine.accept.applying"),
        ),
        PropKind::Prices { lines, .. } => (
            tc("journals:quarantine.price.title", lines.len(), &[]),
            t("journals:quarantine.price.body"),
            t("journals:quarantine.price.undo"),
            tc("journals:quarantine.price.confirm", lines.len(), &[]),
            t("journals:quarantine.price.applying"),
        ),
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(title.clone())
        .child(kit::dialog_header(
            tk,
            title,
            Some(kit::dialog_desc(tk, body)),
        ));
    if let PropKind::Prices {
        lines,
        rate,
        currency,
        ..
    } = &p.kind
    {
        let mut list = col()
            .w_full()
            .max_h(tk.r(12.0))
            .role("list")
            .label(t("journals:quarantine.price.listLabel"))
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            });
        for (i, (label, value, asset)) in lines.iter().enumerate() {
            let v = tr(
                if *rate {
                    "journals:quarantine.price.lineRate"
                } else {
                    "journals:quarantine.price.lineValue"
                },
                &[
                    ("value", &model::format_exact_decimal(value)),
                    ("currency", currency.as_deref().unwrap_or("")),
                    ("asset", asset),
                ],
            );
            let mut li = row()
                .w_full()
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .items_end()
                .justify_between()
                .child(
                    crate::ui::sensitive(label.clone(), tk.tsm(400.0), tk.muted_fg, hidden)
                        .truncate()
                        .min_w(0.0)
                        .grow(1.0),
                )
                .child(crate::ui::sensitive(
                    v.trim().to_owned(),
                    tk.tsm(500.0).features("'tnum' 1"),
                    tk.fg,
                    hidden,
                ));
            if i > 0 {
                li = li.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
            }
            list = list.child(li);
        }
        c = c.child(list);
    }
    let delta = artifact.and_then(|a| tax_delta(tk, a, hidden));
    c = c.child(effect_box(tk, "rv-prop-effect", &p.phase, delta, None).gap(tk.r(0.75)));
    c = c.child(text(foot).font(tk.txs(400.0)).color(tk.muted_fg).wrap());
    let applying = matches!(p.phase, Phase::Applying(..));
    let mut footer = kit::dialog_footer(tk).child(
        kit::btn_md(
            tk,
            "rv-prop-cancel",
            Var::Outline,
            None,
            &t("journals:quarantine.pair.cancel"),
            !applying,
        )
        .on_press(prop_close),
    );
    if matches!(
        p.phase,
        Phase::Uncertain(..) | Phase::Stale | Phase::Failed(_)
    ) {
        footer = footer.child(
            kit::btn_md(
                tk,
                "rv-prop-retry",
                Var::Default,
                None,
                &t("journals:quarantine.fix.retry"),
                true,
            )
            .on_press(prop_retry),
        );
    } else {
        let ready = matches!(p.phase, Phase::Ready(..));
        let mut b = busy_md(
            tk,
            "rv-prop-confirm",
            applying,
            if applying { &applying_label } else { &confirm },
            ready,
        );
        if ready {
            b = b.on_press(prop_confirm);
        }
        footer = footer.child(b);
    }
    c = c.child(footer);
    kit::dialog(app, tk, layer, "rv-prop-dialog", 36.0, c, prop_close);
}

fn bulk_view(app: &App, tk: &Tk, layer: &mut Vec<El>) {
    let b = app.review.rv.bulk.as_ref().unwrap();
    let hidden = app.prefs.hide_sensitive();
    let count = b
        .preview
        .as_ref()
        .and_then(|(a, _)| a["operations"].as_array().map(Vec::len))
        .unwrap_or(b.count);
    let title = tc("review:inbox.transfers.bulk.title", count, &[]);
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .label(title.clone())
        .child(kit::dialog_header(
            tk,
            title,
            Some(kit::dialog_desc(tk, t("review:inbox.transfers.bulk.body"))),
        ));
    if let Some(e) = &app.review.rv.tr.error {
        c = c.child(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .role("alert")
                .wrap(),
        );
    }
    match &b.preview {
        Some((a, _)) => {
            if let Some(d) = tax_delta(tk, a, hidden) {
                c = c.child(d);
            }
        }
        None => {
            c = c.child(
                text(t("review:inbox.transfers.saving"))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .role("status"),
            )
        }
    }
    let enabled = !b.busy && b.preview.is_some();
    let mut confirm = if b.applying {
        kit::btn_busy(
            tk,
            "rv-bulk-confirm",
            Var::Default,
            true,
            None,
            &tc("review:inbox.transfers.bulk.confirm", count, &[]),
            false,
        )
        .h(tk.r(2.25))
        .px(tk.r(0.75))
    } else {
        kit::btn_md(
            tk,
            "rv-bulk-confirm",
            Var::Default,
            None,
            &tc("review:inbox.transfers.bulk.confirm", count, &[]),
            enabled,
        )
    };
    if enabled {
        confirm = confirm.on_press(state::pair_all_exact);
    }
    c = c.child(
        kit::dialog_footer(tk)
            .child(
                kit::btn_md(
                    tk,
                    "rv-bulk-cancel",
                    Var::Outline,
                    None,
                    &t("review:inbox.transfers.bulk.cancel"),
                    !b.busy,
                )
                .on_press(state::close_bulk),
            )
            .child(confirm),
    );
    kit::dialog(app, tk, layer, "rv-bulk-dialog", 32.0, c, state::close_bulk);
}
