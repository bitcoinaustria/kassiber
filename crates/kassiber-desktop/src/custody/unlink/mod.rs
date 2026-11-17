//! A move's details and unlinking it (`routes/transfers/UnlinkDialog.tsx`, `MoveEffects.tsx`,
//! `MoveLeg.tsx`, `reviewPreview.ts`): both legs to the sat, the fee, how and when it was
//! linked, what changes in the book (the review core's dry run where it has one) and how to
//! undo it. A dialog over the page, or a panel beside the table when the page is wide.
//!
//! An automatic link (exchange or Bark evidence) is undone through the review core: its dry run
//! is shown and exactly that run is applied. The core cannot dry-run removing an owner's pair,
//! so the dialog says plainly what unpairing does today. A link derived from one transaction
//! seen by two wallets cannot be undone; the dialog explains it instead.

pub mod common;

use rune::{Cx, El, Tx, col, row, text};
use serde_json::{Value, json};

use self::common::{Applied, Fallback, Legs, Owner, PStatus, Preview};
use super::kit::{self, Var};
use super::link_model::{self, date_time, kind_key};
use super::model::{self, AUTOMATIC_UNDO_DAYS, AUTOMATIC_UNDO_REASON, LinkedMove, Pair};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;

/// "details" shows the move with an Unlink button; "unlink" asks to confirm it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Intent {
    #[default]
    Details,
    Unlink,
}

/// `UnlinkResult`.
// One result per finished unlink, moved straight into the page's note: boxing the larger
// variant would only add an allocation.
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum UnlinkResult {
    Pair {
        pair: Option<Pair>,
        mv: LinkedMove,
    },
    Automatic {
        dismissal_id: Option<String>,
        mv: LinkedMove,
    },
}

impl UnlinkResult {
    pub fn mv(&self) -> &LinkedMove {
        match self {
            UnlinkResult::Pair { mv, .. } | UnlinkResult::Automatic { mv, .. } => mv,
        }
    }
}

#[derive(Default)]
pub struct State {
    pub mv: Option<LinkedMove>,
    pub intent: Intent,
    pub legs: Legs,
    pub preview: Preview,
    pub error: Option<String>,
    /// `ui.transfers.unpair` in flight, and whether "differently" asked for it.
    pub unpairing: bool,
    differently: bool,
}

/// How the move can be unlinked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Automatic,
    Pair,
    Fixed,
}

pub fn mode_of(mv: &LinkedMove) -> Mode {
    if mv.pair_id.is_some() {
        Mode::Pair
    } else if mv.undoable && mv.lineage.is_some() {
        Mode::Automatic
    } else {
        Mode::Fixed
    }
}

/// Opens a move (`openMove`).
pub fn open(app: &mut App, mv: LinkedMove, intent: Intent) {
    let same = app
        .custody
        .unlink
        .mv
        .as_ref()
        .is_some_and(|m| m.key == mv.key);
    let legs = std::mem::take(&mut app.custody.unlink.legs);
    let mut preview = std::mem::take(&mut app.custody.unlink.preview);
    if !same {
        preview.reset();
    }
    app.custody.unlink = State {
        mv: Some(mv.clone()),
        intent,
        legs,
        preview,
        ..State::default()
    };
    common::ensure_leg(app, Owner::Unlink, &mv.out_transaction_id);
    common::ensure_leg(app, Owner::Unlink, &mv.in_transaction_id);
}

pub fn close(app: &mut App) {
    let legs = std::mem::take(&mut app.custody.unlink.legs);
    let mut preview = std::mem::take(&mut app.custody.unlink.preview);
    preview.reset();
    app.custody.unlink = State {
        legs,
        preview,
        ..State::default()
    };
}

pub fn current(app: &App) -> Option<&LinkedMove> {
    app.custody.unlink.mv.as_ref()
}

fn pending(app: &App) -> bool {
    app.custody.unlink.unpairing || app.custody.unlink.preview.applying
}

/// The close the frames use: not while a change is applying.
fn request_close(app: &mut App) {
    if !pending(app) {
        close(app);
    }
}

fn request_unlink(app: &mut App) {
    app.custody.unlink.intent = Intent::Unlink;
}

/// The operations the dry run plans: only asking to unlink an automatic link runs it.
fn operations(app: &App) -> Option<Value> {
    let s = &app.custody.unlink;
    let mv = s.mv.as_ref()?;
    (mode_of(mv) == Mode::Automatic && s.intent == Intent::Unlink).then(|| {
        json!([{
            "type": "dismiss_pair",
            "out_transaction_id": mv.out_transaction_id,
            "in_transaction_id": mv.in_transaction_id,
            "reason": AUTOMATIC_UNDO_REASON,
            "expires_in_days": AUTOMATIC_UNDO_DAYS,
        }])
    })
}

pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let ops = operations(app);
    common::sync_preview(app, Owner::Unlink, ops);
    common::poll_preview(app, Owner::Unlink, tx)
}

pub fn on_reply(app: &mut App, tag: &str, r: &Reply) {
    match tag {
        "custody.unlink.leg" => common::on_leg(app, Owner::Unlink, r),
        "custody.unlink.unpair" => {
            let s = &mut app.custody.unlink;
            s.unpairing = false;
            let differently = s.differently;
            let Some(mv) = s.mv.clone() else { return };
            match &r.error {
                Some(e) => {
                    s.error = Some(if e.is_empty() {
                        t("review:transfersPage.undo.failed")
                    } else {
                        e.clone()
                    })
                }
                None => {
                    let pair = r
                        .data
                        .get("pair")
                        .filter(|p| p.is_object())
                        .map(Pair::parse);
                    super::page::on_unlinked(app, UnlinkResult::Pair { pair, mv }, differently);
                }
            }
        }
        _ => {
            let Some(stage) = tag.strip_prefix("custody.unlink.pv.") else {
                return;
            };
            let failed = t("review:transfersPage.effects.unavailable");
            if let Some(applied) = common::on_preview_reply(app, Owner::Unlink, stage, r, &failed) {
                let differently = app.custody.unlink.differently;
                let Some(mv) = app.custody.unlink.mv.clone() else {
                    return;
                };
                match applied {
                    Applied::Ok(result) => {
                        let dismissal_id = common::applied_id(&result, "dismissal_id");
                        super::page::on_unlinked(
                            app,
                            UnlinkResult::Automatic { dismissal_id, mv },
                            differently,
                        );
                    }
                    Applied::Err { message, code } => {
                        if common::is_stale(code.as_deref()) {
                            // The book changed since the dry run: run it again and ask again.
                            app.custody.unlink.error = Some(t("review:transfersPage.unlink.stale"));
                            common::retry_preview(app, Owner::Unlink);
                        } else {
                            app.custody.unlink.error = Some(if message.is_empty() {
                                t("review:transfersPage.undo.failed")
                            } else {
                                message
                            });
                        }
                    }
                }
            }
        }
    }
}

/// Escape closes the dialog or panel (not while it applies; the press is still taken).
pub fn escape(app: &mut App) -> bool {
    if app.custody.unlink.mv.is_none() {
        return false;
    }
    request_close(app);
    true
}

pub fn reload(app: &mut App) {
    if app.custody.unlink.mv.is_some() {
        common::reload_legs(app, Owner::Unlink);
    } else {
        app.custody.unlink.legs = Legs::default();
    }
}

fn confirm(app: &mut App, differently: bool) {
    let Some(mv) = app.custody.unlink.mv.clone() else {
        return;
    };
    app.custody.unlink.error = None;
    app.custody.unlink.differently = differently;
    match mode_of(&mv) {
        Mode::Pair => {
            if app.custody.unlink.unpairing {
                return;
            }
            let args = json!({"pair_id": mv.pair_id, "expected_scope": app.expected_scope()});
            if app
                .send("ui.transfers.unpair", args, "custody.unlink.unpair")
                .is_some()
            {
                app.custody.unlink.unpairing = true;
            }
        }
        Mode::Automatic => {
            common::apply_preview(app, Owner::Unlink);
        }
        Mode::Fixed => {}
    }
}

/// The content as a panel beside the table (22 rem wide).
pub fn panel(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let _ = layer;
    let Some(mv) = current(app) else { return col() };
    let (title, desc) = title_desc(app, mv);
    let content = body(app, cx, tk, mv, true, true);
    common::panel_shell(
        app,
        cx,
        tk,
        "unlink-dialog",
        title,
        desc_el(app, tk, desc),
        t("review:transfersPage.unlink.close"),
        true,
        request_close,
        content,
    )
}

/// The content as a dialog. `link_rest`: whether "Link the rest" may start a link.
pub fn dialog(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, link_rest: bool) {
    let Some(mv) = current(app) else { return };
    let (title, desc) = title_desc(app, mv);
    let content = body(app, cx, tk, mv, false, link_rest);
    common::dialog_shell(
        app,
        tk,
        layer,
        "unlink-dialog",
        42.0,
        title,
        desc_el(app, tk, desc),
        request_close,
        content,
    );
}

fn title_desc(app: &App, mv: &LinkedMove) -> (String, String) {
    let s = &app.custody.unlink;
    let title = if s.intent == Intent::Details {
        t("review:transfersPage.details.title")
    } else if mode_of(mv) == Mode::Fixed {
        t("review:transfersPage.unlink.titleFixed")
    } else {
        t("review:transfersPage.unlink.title")
    };
    (
        title,
        tr(
            "review:transfersPage.unlink.subtitle",
            &[("from", &mv.from_label), ("to", &mv.to_label)],
        ),
    )
}

fn desc_el(app: &App, tk: &Tk, desc: String) -> El {
    crate::ui::sensitive(desc, tk.tsm(400.0), tk.muted_fg, app.prefs.hide_sensitive()).wrap()
}

fn body(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    mv: &LinkedMove,
    panel: bool,
    link_rest: bool,
) -> Vec<El> {
    let s = &app.custody.unlink;
    let mode = mode_of(mv);
    let details = s.intent == Intent::Details;
    let busy = pending(app);
    // Only a link the owner made can leave a part of its send for another link.
    let parts = if mv.pair_id.is_some() {
        super::open_parts(app)
    } else {
        None
    };
    let (send, send_loading, _) = s.legs.leg(Some(&mv.out_transaction_id), parts);
    let (receipt, receipt_loading, _) = s.legs.leg(Some(&mv.in_transaction_id), parts);
    let rest = send
        .as_ref()
        .filter(|l| link_model::leg_has_open_part(l))
        .cloned();
    let send_msat = send
        .as_ref()
        .map(|l| l.amount_msat)
        .or(Some(mv.amount_msat));
    let receipt_msat = receipt
        .as_ref()
        .map(|l| l.amount_msat)
        .or(mv.in_amount_msat);
    let network_fee = send.as_ref().and_then(|l| l.fee_msat).unwrap_or(0);
    let alloc_out = mv.pair_id.as_ref().map(|_| mv.amount_msat);
    let alloc_in = if mv.pair_id.is_some() {
        mv.in_amount_msat
    } else {
        None
    };
    let shares_legs = matches!((alloc_out, &send), (Some(a), Some(l)) if a != l.amount_msat)
        || matches!((alloc_in, &receipt), (Some(a), Some(l)) if a != l.amount_msat);
    let same_asset = receipt
        .as_ref()
        .map(|r| r.asset.as_str())
        .unwrap_or(&mv.asset)
        == mv.asset;
    let fee = if rest.is_some() {
        None
    } else {
        mv.fee_msat
            .or_else(|| match (shares_legs, send_msat, receipt_msat) {
                (false, Some(s), Some(r)) if same_asset => Some(s + network_fee - r),
                _ => None,
            })
    };
    let how = match mode {
        Mode::Pair => {
            let date = date_time(mv.linked_at.as_deref());
            if mv.by_rule {
                tr("review:transfersPage.unlink.howRule", &[("date", &date)])
            } else if matches!(
                mv.pair_source.as_deref(),
                Some("bulk_exact" | "bulk_selected")
            ) {
                tr("review:transfersPage.unlink.howBulk", &[("date", &date)])
            } else {
                tr("review:transfersPage.unlink.howYou", &[("date", &date)])
            }
        }
        _ => match &mv.lineage {
            Some(l) => tr(
                "review:transfersPage.unlink.howAutomatic",
                &[(
                    "reason",
                    &t(model::lineage_evidence_key(
                        &l.evidence_reason,
                        &l.custody_state,
                    )),
                )],
            ),
            None => t("review:transfersPage.unlink.howAutomaticPlain"),
        },
    };
    let recorded = (mode == Mode::Pair && mv.pair_kind.is_some()).then(|| {
        tr(
            "review:transfersPage.unlink.recordedAs",
            &[
                (
                    "kind",
                    &t(&format!(
                        "review:transfersPage.kind.{}",
                        kind_key(mv.pair_kind.as_deref())
                    )),
                ),
                (
                    "policy",
                    &t(&format!(
                        "review:transfersPage.policy.{}",
                        if mv.pair_policy.as_deref() == Some("taxable") {
                            "taxable"
                        } else {
                            "carrying"
                        }
                    )),
                ),
            ],
        )
    });
    let outcome = match &s.preview.status {
        PStatus::Ready { artifact, .. } => Some(common::move_outcome(
            artifact,
            &mv.out_transaction_id,
            &mv.in_transaction_id,
        )),
        _ => None,
    };
    let mut auto_lines = vec![];
    if let Some(o) = outcome {
        auto_lines.push(if o.send_held || o.receipt_held {
            tr(
                "review:transfersPage.unlink.effectHeld",
                &[("from", &mv.from_label)],
            )
        } else {
            tr(
                "review:transfersPage.unlink.effectBooked",
                &[("from", &mv.from_label), ("to", &mv.to_label)],
            )
        });
    }

    let mut out = vec![];
    // The legs: stacked in a panel, side by side in a dialog.
    let fb_send = Fallback {
        wallet: mv.from_label.clone(),
        rail: mv.from_rail,
        amount_msat: Some(mv.amount_msat),
        occurred_at: mv.occurred_at.clone(),
    };
    let fb_recv = Fallback {
        wallet: mv.to_label.clone(),
        rail: mv.to_rail,
        amount_msat: mv.in_amount_msat,
        occurred_at: mv.in_occurred_at.clone(),
    };
    let leg_send = common::move_leg(
        app,
        cx,
        tk,
        "unlink-leg-send",
        t("review:transfersPage.leg.sent"),
        send.as_ref(),
        send_loading,
        Some(&fb_send),
        false,
        None,
        close,
    );
    let leg_recv = common::move_leg(
        app,
        cx,
        tk,
        "unlink-leg-recv",
        t("review:transfersPage.leg.received"),
        receipt.as_ref(),
        receipt_loading,
        Some(&fb_recv),
        false,
        None,
        close,
    );
    out.push(if panel {
        col()
            .w_full()
            .gap(tk.r(0.5))
            .child(leg_send)
            .child(leg_recv)
    } else {
        row()
            .w_full()
            .gap(tk.r(0.5))
            .items_stretch()
            .child(leg_send.grow(1.0).w(0.0))
            .child(
                row()
                    .center()
                    .child(crate::ui::ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg).a11y_hidden()),
            )
            .child(leg_recv.grow(1.0).w(0.0))
    });

    // The fee (or the rest left), and how it was linked.
    let first = match &rest {
        Some(r) => {
            let left = model::msat_btc(r.open.map(|o| o.unlinked_msat).unwrap_or(0));
            let mut c = col().w_full().gap(tk.r(0.375)).child(
                common::para_s(
                    app,
                    tk,
                    tr("review:transfersPage.linked.rest", &[("left", &left)]),
                    tk.fg,
                )
                .key("unlink-rest"),
            );
            if link_rest {
                let id = r.id.clone();
                c = c.child(
                    row().child(
                        kit::btn_xs(
                            tk,
                            "unlink-link-rest",
                            Var::Default,
                            Some(&I::LINK_2),
                            &t("review:transfersPage.strip.linkRest"),
                            !busy,
                        )
                        .on_press(move |a: &mut App| super::page::open_link(a, Some(id.clone()))),
                    ),
                );
            }
            common::section(tk, t("review:transfersPage.unlink.restTitle"), c)
        }
        None => common::section(
            tk,
            t("review:transfersPage.unlink.feeTitle"),
            common::fee_line(app, tk, send_msat, fee),
        ),
    };
    let mut how_c = col()
        .w_full()
        .gap(tk.r(0.125))
        .child(common::para(tk, how, tk.fg));
    if let Some(r) = recorded {
        how_c = how_c.child(common::para(tk, r, tk.muted_fg));
    }
    let second = common::section(tk, t("review:transfersPage.unlink.howTitle"), how_c);
    out.push(if panel {
        col().w_full().gap(tk.r(1.0)).child(first).child(second)
    } else {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(first.grow(1.0).w(0.0))
            .child(second.grow(1.0).w(0.0))
    });

    // What changes, or why it can't be unlinked.
    if !(details && mode != Mode::Fixed) {
        let title = t(if mode == Mode::Fixed {
            "review:transfersPage.unlink.fixedTitle"
        } else {
            "review:transfersPage.unlink.changesTitle"
        });
        let content = match mode {
            Mode::Automatic => common::outcome_view(
                app,
                tk,
                "unlink-preview",
                &s.preview,
                outcome,
                &auto_lines,
                false,
                None,
                None,
                Some(|a: &mut App| common::retry_preview(a, Owner::Unlink)),
            ),
            Mode::Pair => {
                let kind = t(&format!(
                    "review:transfersPage.kind.{}",
                    kind_key(mv.pair_kind.as_deref())
                ));
                let mut c = col().key("unlink-plain-effects").w_full().gap(tk.r(0.375));
                if mv.pair_policy.as_deref() == Some("taxable") {
                    c = c.child(common::para(
                        tk,
                        tr(
                            "review:transfersPage.unlink.effectPairTaxable",
                            &[("kind", &kind)],
                        ),
                        tk.fg,
                    ));
                } else {
                    c = c
                        .child(common::para_s(
                            app,
                            tk,
                            tr(
                                "review:transfersPage.unlink.effectPair",
                                &[("from", &mv.from_label), ("to", &mv.to_label)],
                            ),
                            tk.fg,
                        ))
                        .child(common::para(
                            tk,
                            t("review:transfersPage.unlink.effectPairWhy"),
                            tk.muted_fg,
                        ));
                }
                c.child(
                    text(t("review:transfersPage.unlink.noDryRun"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                )
            }
            Mode::Fixed => common::para(
                tk,
                format!(
                    "{} {}",
                    t(&format!(
                        "review:transfersPage.linked.fixed.{}",
                        model::fixed_link_kind(mv.evidence_reason.as_deref())
                    )),
                    t("review:transfersPage.unlink.fixedFix")
                ),
                tk.muted_fg,
            ),
        };
        out.push(common::section(tk, title, content));
    }
    if mode != Mode::Fixed && !details {
        out.push(common::section(
            tk,
            t("review:transfersPage.unlink.undoTitle"),
            common::para(
                tk,
                t(if mode == Mode::Automatic {
                    "review:transfersPage.unlink.undoAutomatic"
                } else {
                    "review:transfersPage.unlink.undoPair"
                }),
                tk.muted_fg,
            ),
        ));
    }
    if let Some(e) = &s.error {
        out.push(
            text(e.clone())
                .font(tk.tsm(400.0))
                .color(tk.destructive)
                .wrap()
                .role("alert"),
        );
    }
    // The footer.
    let ready = mode == Mode::Pair
        || (mode == Mode::Automatic && matches!(s.preview.status, PStatus::Ready { .. }));
    let mut foot = row().w_full().gap(tk.r(0.5)).center_y();
    match mode {
        Mode::Fixed => {
            if !panel {
                foot = foot.justify_end().child(
                    kit::btn_md(
                        tk,
                        "unlink-close-btn",
                        Var::Outline,
                        None,
                        &t("review:transfersPage.unlink.close"),
                        true,
                    )
                    .on_press(close),
                );
            }
        }
        _ if details => {
            foot = foot.justify_between().child(
                kit::btn_md(
                    tk,
                    "unlink-request",
                    if panel { Var::Outline } else { Var::Ghost },
                    Some(&I::UNLINK),
                    &t("review:transfersPage.details.unlink"),
                    true,
                )
                .on_press(request_unlink),
            );
            if !panel {
                foot = foot.child(
                    kit::btn_md(
                        tk,
                        "unlink-close-btn",
                        Var::Outline,
                        None,
                        &t("review:transfersPage.unlink.close"),
                        true,
                    )
                    .on_press(close),
                );
            }
        }
        _ => {
            let differently = kit::btn_md(
                tk,
                "unlink-differently",
                Var::Ghost,
                None,
                &t("review:transfersPage.unlink.differently"),
                !busy && ready,
            )
            .on_press(|a: &mut App| confirm(a, true));
            let keep = kit::btn_md(
                tk,
                "unlink-cancel",
                Var::Outline,
                None,
                &t("review:transfersPage.undo.cancel"),
                !busy,
            )
            .on_press(close);
            let go = common::destructive_btn(
                tk,
                "unlink-confirm",
                &t(if busy {
                    "review:transfersPage.undo.undoing"
                } else {
                    "review:transfersPage.undo.confirm"
                }),
                !busy && ready,
                busy,
            )
            .on_press(|a: &mut App| confirm(a, false));
            if panel {
                foot = col()
                    .w_full()
                    .gap(tk.r(0.5))
                    .child(
                        row()
                            .w_full()
                            .justify_end()
                            .gap(tk.r(0.5))
                            .child(keep)
                            .child(go),
                    )
                    .child(row().w_full().justify_start().child(differently));
            } else {
                foot = foot
                    .justify_between()
                    .child(differently)
                    .child(row().gap(tk.r(0.5)).child(keep).child(go));
            }
        }
    }
    out.push(foot);
    out
}

/// Named states for `--scene`.
pub const SCENES: &[crate::app::Scene] = &[
    ("custody-unlink-details", |a| {
        open_first(a, None, Intent::Details, false)
    }),
    ("custody-unlink-details-dialog", |a| {
        open_first(a, None, Intent::Details, true)
    }),
    ("custody-unlink-pair", |a| {
        open_first(a, Some("pair"), Intent::Unlink, false)
    }),
    ("custody-unlink-pair-dialog", |a| {
        open_first(a, Some("pair"), Intent::Unlink, true)
    }),
    ("custody-unlink-fixed", |a| {
        open_first(a, Some("fixed"), Intent::Unlink, false)
    }),
    ("custody-unlink-light", |a| {
        a.prefs.theme = crate::app::ThemeMode::Light;
        open_first(a, Some("pair"), Intent::Unlink, false)
    }),
    ("custody-unlink-de", |a| {
        a.prefs.lang = crate::i18n::Lang::De;
        open_first(a, Some("pair"), Intent::Unlink, false)
    }),
];

/// Opens the first linked move of a kind (`pair`, `fixed`, any) on the Transfers page.
fn open_first(a: &mut App, kind: Option<&str>, intent: Intent, narrow: bool) {
    let _ = narrow;
    a.navigate("/transfers");
    super::prime(a);
    let mv = a.custody.page.moves.iter().find(|m| match kind {
        Some("pair") => m.pair_id.is_some(),
        Some("fixed") => mode_of(m) == Mode::Fixed,
        _ => true,
    });
    if let Some(m) = mv.cloned() {
        open(a, m, intent);
        super::settle(a);
        let ops = operations(a);
        common::sync_preview(a, Owner::Unlink, ops);
        common::start_now(a, Owner::Unlink);
        super::settle(a);
    }
}
