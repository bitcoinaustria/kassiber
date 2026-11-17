//! What cannot be booked yet (`QuarantineSection` → `QuarantineDashboard` →
//! `QuarantineCausePanel` + `QuarantineQueue`): whether the list is current, how many need the
//! owner and how many only wait, one card per cause with what was seen, what to do and its
//! transactions or pairs, and on request the rows that only wait on a cause.

use rune::{Color, Cx, El, col, el, row, text};
use rune_kit::{Placement, TextInput};
use serde_json::Value;

use super::kit::{self, Var};
use super::model::{self, Group, Item, Snap, s};
use super::state::{self, PAGE, PropKind};
use super::{Popup, toggle};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::{ic, sensitive, surface};

/// Actions whose button already says what to do.
const SELF_DESCRIBING: [&str; 6] = [
    "sync_wallet",
    "connect_wallet",
    "import_history",
    "set_price",
    "pair_transfer",
    "restore_pairing",
];
/// Rows a cause shows before the rest fold away.
const SHOWN_ROWS: usize = 3;
/// Cause cards shown before the rest fold away.
const SHOWN_CAUSES: usize = 4;
const COARSE_PRICE_REASON: &str = "pricing_review_required";

fn info_toggle(a: &mut App, key: &str) {
    toggle(a, Popup::Info(key.to_owned()));
}

fn info_open(app: &App, key: &str) -> bool {
    app.review.popup == Some(Popup::Info(key.to_owned()))
}

/// The section: loading, unavailable, or the dashboard.
pub fn section(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let q = &app.review.rv.q;
    if q.pages.is_empty() && q.error.is_none() {
        // `isLoading`: the skeleton (`space-y-2`, `h-4 w-48`, `h-28 w-full rounded-lg`).
        return col()
            .w_full()
            .gap(tk.r(0.5))
            .busy(true)
            .child(kit::skeleton(tk, Some(tk.r(12.0)), tk.r(1.0)))
            .child(kit::skeleton(tk, None, tk.r(7.0)).radius(tk.r(0.5)));
    }
    let Some(att) = q.attention() else {
        return unavailable(tk, q.error.clone());
    };
    let mut c = col().w_full().gap(tk.r(0.75));
    if let Some(p) = cause_panel(app, cx, tk, &att, layer) {
        c = c.child(p);
    }
    if att.count() > 0 && q.waiting_shown {
        c = c.child(queue(app, tk, &att));
    }
    c
}

/// `QuarantineUnavailable`.
fn unavailable(tk: &Tk, message: Option<String>) -> El {
    surface(tk)
        .w_full()
        .padding(tk.card_pad())
        .role("alert")
        .child(
            text(t("journals:quarantine.unavailable.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg),
        )
        .child(
            text(
                message
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| t("journals:quarantine.unavailable.fallback")),
            )
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .mt(tk.r(0.25))
            .wrap(),
        )
}

/// The amber freshness box's colours (`border-amber-300 bg-amber-50 text-amber-900`, dark
/// `border-amber-900/60 bg-amber-950/40 text-amber-100`).
fn amber_box(tk: &Tk) -> (Color, Color, Color) {
    let h = |x: &str| rune::color::hex(x).unwrap();
    if tk.dark {
        (
            h("#78350f").alpha(0.6),
            h("#451a03").alpha(0.4),
            h("#fef3c7"),
        )
    } else {
        (h("#fcd34d"), h("#fffbeb"), h("#78350f"))
    }
}

fn cause_panel(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    snap: &Snap,
    layer: &mut Vec<El>,
) -> Option<El> {
    let rv = &app.review.rv;
    let groups = model::with_loaded_causes(snap.groups(), &snap.items);
    let unlisted = snap
        .group_count()
        .unwrap_or(groups.len())
        .saturating_sub(groups.len());
    let count = snap.count();
    let last_error = snap.last_error();
    let warning = snap.needs_processing() || last_error.is_some();
    if count == 0 && !warning {
        return None;
    }
    let calculated = model::opt(snap.freshness(), "last_processed_at").or_else(|| {
        snap.items
            .iter()
            .map(|i| i.created_at.clone())
            .max()
            .filter(|x| !x.is_empty())
    });
    let mut sec = col()
        .key("rv-quarantine")
        .w_full()
        .gap(tk.section_gap())
        .role("region")
        .label(t("journals:quarantine.section.title"));
    if warning {
        let (border, bg, ink) = amber_box(tk);
        let body = match (&last_error, &calculated) {
            (Some(m), _) => tr(
                "journals:quarantine.freshness.failedBody",
                &[("message", m)],
            ),
            (None, Some(w)) => tr(
                "journals:quarantine.freshness.staleBody",
                &[("when", &w.chars().take(10).collect::<String>())],
            ),
            _ => t("journals:quarantine.freshness.neverBody"),
        };
        let title = if last_error.is_some() {
            t("journals:quarantine.freshness.failedTitle")
        } else {
            t("journals:quarantine.freshness.staleTitle")
        };
        let processing = app.busy.processing;
        let mut b = kit::btn_busy(
            tk,
            "rv-fresh-process",
            Var::Default,
            processing,
            Some(&I::REFRESH_CW),
            &t("journals:quarantine.cta.processJournals"),
            !processing,
        );
        if !processing {
            b = b.on_press(|a: &mut App| a.process_journals());
        }
        sec = sec.child(
            row()
                .key("rv-freshness")
                .w_full()
                .padding(tk.r(1.0))
                .gap(tk.r(0.75))
                .center_y()
                .justify_between()
                .radius(tk.r(0.5))
                .border(1.0, border)
                .fill(bg)
                .role("status")
                .child(
                    row()
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(tk.r(0.75))
                        .items_start()
                        .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), ink).mt(tk.r(0.125)))
                        .child(
                            col()
                                .min_w(0.0)
                                .grow(1.0)
                                .gap(tk.r(0.25))
                                .child(text(title).font(tk.tsm(500.0)).color(ink))
                                .child(text(body).font(tk.tsm(400.0)).color(ink).wrap()),
                        ),
                )
                .child(b),
        );
    }
    if count > 0 {
        let att_count = snap.attention_count();
        let waiting = snap.waiting_count();
        let mut left = col().min_w(0.0).grow(1.0).gap(tk.r(0.125)).child(
            row()
                .gap(tk.r(0.5))
                .items_end()
                .flex_wrap()
                .role("heading")
                .level(2)
                .child(
                    text(t("journals:quarantine.section.title"))
                        .font(tk.tsm(600.0))
                        .color(tk.fg),
                )
                .child(
                    text(tc("journals:quarantine.summary.needsYou", att_count, &[]))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
        if waiting > 0 {
            let mut w = row().gap(tk.r(0.25)).items_end().flex_wrap().child(
                text(tc("journals:quarantine.summary.waiting", waiting, &[]))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
            if !app.review.rv.q.waiting_shown {
                w = w.child(
                    kit::link(
                        tk,
                        "rv-show-waiting",
                        &t("journals:quarantine.summary.showWaiting"),
                        true,
                    )
                    .on_press(|a: &mut App| state::set_waiting_shown(a, true)),
                );
            }
            left = left.child(w);
        }
        if snap.summary["reports_blocked"].as_bool() == Some(true) {
            left = left.child(
                text(t("journals:quarantine.summary.reportsBlocked"))
                    .font(tk.txs(400.0))
                    .color(kit::red_text(tk)),
            );
        }
        let mut summary = row()
            .key("rv-quarantine-summary")
            .w_full()
            .gap(tk.r(0.5))
            .items_start()
            .justify_between()
            .child(left);
        if app.prefs.ai_features && att_count > 0 {
            // `FixWithAssistant`: disabled while an answer streams.
            let idle = !app.assistant.streaming;
            let mut b = kit::btn_md(
                tk,
                "rv-fix-assistant",
                Var::Outline,
                Some(&I::SPARKLES),
                &t("journals:quarantine.actions.fixWithAssistant"),
                idle,
            );
            if idle {
                b = b.on_press(move |a: &mut App| crate::assistant::fix_quarantine(a, att_count));
            }
            summary = summary.child(b);
        }
        sec = sec.child(summary);
    }
    let shown: Vec<&Group> = if rv.all_causes {
        groups.iter().collect()
    } else {
        groups.iter().take(SHOWN_CAUSES).collect()
    };
    let card_offers_more = shown
        .iter()
        .any(|g| model::cause_offers_more(g, &snap.items));
    if !groups.is_empty() {
        let mut ol = col().w_full().gap(tk.card_gap()).role("list");
        for g in &shown {
            ol = ol.child(card(app, cx, tk, snap, g, layer));
        }
        let mut block = col().w_full().gap(tk.r(0.5)).child(ol);
        if groups.len() > SHOWN_CAUSES {
            let label = if rv.all_causes {
                t("journals:quarantine.causes.showFewer")
            } else {
                tc(
                    "journals:quarantine.causes.showMore",
                    groups.len() - SHOWN_CAUSES,
                    &[],
                )
            };
            block = block.child(
                kit::btn(tk, "rv-causes-more", Var::Ghost, None, &label, true)
                    .on_press(|a: &mut App| a.review.rv.all_causes = !a.review.rv.all_causes),
            );
        }
        sec = sec.child(block);
    }
    let q = &rv.q;
    if count > 0
        && (unlisted > 0 || (!card_offers_more && (q.has_more() || q.more_error.is_some())))
    {
        let msg = if unlisted > 0 {
            tc("journals:quarantine.causes.more", unlisted, &[])
        } else {
            t("journals:quarantine.causes.pageMore")
        };
        sec = sec.child(not_loaded(app, tk, msg).key("rv-load-more"));
    }
    Some(sec)
}

/// `NotLoadedYet`.
fn not_loaded(app: &App, tk: &Tk, msg: String) -> El {
    let q = &app.review.rv.q;
    let mut r = row()
        .gap(tk.r(0.5))
        .items_end()
        .flex_wrap()
        .child(text(msg).font(tk.txs(400.0)).color(tk.muted_fg));
    if q.has_more() {
        let mut l = kit::link_named(
            tk,
            "rv-load-more-btn",
            &t("journals:quarantine.causes.loadMore"),
            t("journals:quarantine.causes.loadMore"),
            !q.more_loading,
            q.more_loading,
        );
        if !q.more_loading {
            l = l.on_press(state::load_more);
        }
        r = r.child(l);
    }
    if let Some(e) = &q.more_error {
        r = r.child(
            text(tr(
                "journals:quarantine.causes.loadMoreFailed",
                &[("message", e)],
            ))
            .font(tk.txs(400.0))
            .color(tk.destructive)
            .role("alert"),
        );
    }
    r
}

fn direction_tone(tk: &Tk, direction: &str, hidden: bool) -> Color {
    if hidden {
        tk.muted_fg
    } else if direction == "outbound" {
        kit::red_text(tk)
    } else {
        kit::emerald_text(tk)
    }
}

/// Opens a cause's row in the detail panel, on the tab that holds its evidence.
fn open_root(app: &mut App, g: &Group, id: &str, tab: Option<&str>) {
    super::links::open_cause_row(app, g, id, tab);
}

/// `runAction`.
fn run_action(app: &mut App, g: &Group, a: &model::Action) {
    let root = g.root_transaction_id.clone();
    match a.kind.as_str() {
        "sync_wallet" => {
            if let Some(w) = &a.wallet_id {
                let label = a.wallet_label.clone().unwrap_or_else(|| w.clone());
                state::run_sync(app, w, &label);
            }
        }
        // `setDialog({ mode: "connect" })`: straight to the descriptor setup.
        "connect_wallet" => crate::wallets::add::open(app, Some("descriptor")),
        // `setDialog({ mode: "import", walletId })`: the history import into that wallet.
        "import_history" => crate::wallets::add::open_history(app, a.wallet_id.clone()),
        "review_custody_gap" => {
            // Decided at click time; the /transfers route guard enforces the same gate.
            if app.prefs.developer_tools && a.gap_id.is_some() {
                app.navigate(&format!(
                    "/transfers?section=advanced&tab=gaps&gap={}",
                    a.gap_id.clone().unwrap_or_default()
                ));
            } else {
                app.review.rv.locked_gap = Some(g.key.clone());
            }
        }
        "process_journals" => app.process_journals(),
        "wait_for_confirmation" => {}
        "resolve_root" => {
            if let Some(id) = &a.transaction_id {
                super::links::open_row(
                    app,
                    id,
                    "details",
                    None,
                    crate::txdetail::ReviewSource::None,
                );
            }
        }
        "review_pair" => {
            if let Some(id) = a.transaction_id.clone().or(root) {
                open_root(app, g, &id, Some("linked"));
            }
        }
        // Linking is decided in "Link manually" on Transfers, on this send.
        "pair_transfer" => {
            let link = a
                .transaction_id
                .clone()
                .or(root)
                .unwrap_or_else(|| "new".into());
            app.navigate(&format!("/transfers?link={link}"));
        }
        // The link the owner removed, back as it was (Transfers' "link it back"), then the
        // journals recalculate.
        "restore_pairing" => {
            if let Some(id) = &a.dismissal_id {
                state::link_back(app, id);
            }
        }
        _ => {
            if let Some(id) = a.transaction_id.clone().or(root) {
                open_root(app, g, &id, None);
            }
        }
    }
}

fn card(app: &App, cx: &mut Cx<App>, tk: &Tk, snap: &Snap, g: &Group, layer: &mut Vec<El>) -> El {
    let _ = cx;
    let hidden = app.prefs.hide_sensitive();
    let st = app.review.rv.card(&g.key);
    let unlinked = model::unlinked_automatic_pair(&g.reason, &g.actions);
    let copy = model::cause_copy_full(
        &g.reason,
        &g.category,
        &g.evidence,
        g.root_wallet.as_deref(),
        g.root_asset.as_deref(),
        None,
        &g.actions,
    );
    let facts = model::cause_facts(&g.evidence);
    let rows = model::cause_rows(g, &snap.items);
    let scope = snap.book_scope();
    let currency = snap.fiat_currency();
    let deprecated: Vec<String> = g.evidence["missing_source_wallets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|w| w["deprecated"].as_bool() == Some(true))
        .map(|w| s(w, "label"))
        .collect();
    let price_rows: Vec<&Item> = if g.category == "missing_price" && scope.is_some() {
        rows.members
            .iter()
            .copied()
            .filter(|i| !i.is_downstream)
            .collect()
    } else {
        vec![]
    };
    let entering = st.is_some_and(|c| c.entering);
    let accept = g.reason == COARSE_PRICE_REASON && scope.is_some();
    let actions: Vec<&model::Action> = g
        .actions
        .iter()
        .filter(|a| !(!rows.pairs.is_empty() && a.kind == "review_pair"))
        .filter(|a| !(!price_rows.is_empty() && a.kind == "set_price"))
        // A removed link is answered by linking, not by syncing: the sync the reason offers in
        // general cannot bring that link back.
        .filter(|a| !(unlinked && a.kind == "sync_wallet"))
        .collect();
    let primary_taken = accept || (!price_rows.is_empty() && !entering);
    let provide_inline = !rows.pairs.is_empty()
        || (!accept
            && price_rows.is_empty()
            && !(!actions.is_empty()
                && actions
                    .iter()
                    .all(|a| SELF_DESCRIBING.contains(&a.kind.as_str()))));
    let gkey = g.key.clone();
    let key_id = format!("rv-cause-{}", sanitize(&g.key));

    // Header: title, Blocks reports, counts, the instruction behind an info button.
    let mut head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .flex_wrap()
        .child(text(copy.title.clone()).font(tk.tsm(600.0)).color(tk.fg));
    if g.blocks_reports {
        let (bg, fg) = if tk.dark {
            (kit::tw("red-950").alpha(0.5), kit::tw("red-200"))
        } else {
            (kit::tw("red-100"), kit::tw("red-800"))
        };
        head = head.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.125))
                .radius(999.0)
                .fill(bg)
                .child(
                    text(t("journals:quarantine.panel.blocksReports"))
                        .font(tk.txs(500.0))
                        .color(fg),
                ),
        );
    }
    if rows.root_count > 0 {
        head = head.child(
            text(tc("journals:quarantine.causes.count", rows.root_count, &[]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    if g.downstream_count > 0 {
        head = head.child(
            text(format!(
                "· {}",
                tc(
                    "journals:quarantine.causes.waitingShort",
                    g.downstream_count,
                    &[]
                )
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    if !provide_inline {
        let ik = format!("{key_id}-about");
        let note = format!(
            "{}: {}",
            t("journals:quarantine.causes.whatToDo"),
            copy.provide
        );
        head = head.child(kit::info_note(
            tk,
            &ik,
            tr(
                "journals:quarantine.causes.aboutLabel",
                &[("cause", &copy.title)],
            ),
            &[note],
            info_open(app, &ik),
            layer,
            info_toggle,
        ));
    }
    let mut li = surface(tk)
        .key(key_id.clone())
        .w_full()
        .padding(tk.card_pad())
        .role("listitem")
        .child(head);
    if g.blocks_reports {
        let red = if tk.dark {
            kit::tw("red-900").alpha(0.6)
        } else {
            kit::tw("red-300")
        };
        li = li.border(1.0, red).border_layout();
    }
    li = li.child(
        sensitive(
            copy.why.clone(),
            tk.tsm(400.0),
            tk.muted_fg,
            hidden && copy.why_quotes_amounts,
        )
        .wrap()
        .max_w(tk.r(48.0))
        .mt(tk.r(0.125)),
    );
    if !facts.is_empty() && rows.pairs.is_empty() {
        let mut ul = col()
            .mt(tk.r(0.25))
            .gap(tk.r(0.125))
            .pl(tk.r(1.25))
            .role("list");
        for f in &facts {
            ul = ul.child(
                row()
                    .gap(tk.r(0.5))
                    .items_start()
                    .role("listitem")
                    .child(text("•").font(tk.tsm(400.0)).color(tk.muted_fg))
                    .child(
                        text(f.clone())
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
            );
        }
        li = li.child(ul);
    }
    if provide_inline {
        li = li.child(
            text(format!(
                "{}: {}",
                t("journals:quarantine.causes.whatToDo"),
                copy.provide
            ))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap()
            .max_w(tk.r(48.0))
            .mt(tk.r(0.25)),
        );
    }
    for w in &deprecated {
        li = li.child(
            text(tr(
                "journals:quarantine.panel.deprecatedWallet",
                &[("wallet", w)],
            ))
            .font(tk.txs(400.0))
            .color(kit::amber_text(tk))
            .mt(tk.r(0.25))
            .wrap(),
        );
    }
    if app.review.rv.locked_gap.as_deref() == Some(g.key.as_str()) {
        li = li.child(
            text(t("journals:quarantine.panel.gapReviewLocked"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .mt(tk.r(0.5))
                .wrap(),
        );
    }
    if !rows.pairs.is_empty() {
        li = li.child(pair_list(
            app,
            tk,
            g,
            &copy.title,
            &rows.pairs,
            rows.root_count,
            scope.clone(),
        ));
    } else if !rows.members.is_empty() {
        let price_ids: Vec<String> = price_rows.iter().map(|i| i.id.clone()).collect();
        li = li.child(transaction_list(
            app,
            tk,
            g,
            &copy.title,
            &rows.members,
            rows.root_count,
            entering.then_some(&price_ids),
            currency.as_deref(),
            layer,
        ));
    }
    if entering {
        let c = st.unwrap();
        let order: Vec<String> = price_rows.iter().map(|i| i.id.clone()).collect();
        let check =
            model::price_entry_operations(&order, &c.typed(), c.rate, model::decimal_separator());
        let filled = price_rows
            .iter()
            .filter(|i| {
                c.entries
                    .get(&i.id)
                    .is_some_and(|e| !e.text.trim().is_empty())
            })
            .count();
        let k1 = gkey.clone();
        let mut preview = kit::btn(
            tk,
            format!("{key_id}-preview"),
            Var::Default,
            None,
            &tc("journals:quarantine.price.preview", filled, &[]),
            filled > 0,
        );
        if filled > 0 {
            let labels: Vec<(String, String, String)> = price_rows
                .iter()
                .map(|i| (i.id.clone(), model::row_meta(i), i.asset.clone()))
                .collect();
            let (scope, currency, order) = (scope.clone(), currency.clone(), order.clone());
            preview = preview.on_press(move |a: &mut App| {
                preview_prices(a, &k1, &order, &labels, scope.clone(), currency.clone())
            });
        }
        let k2 = gkey.clone();
        let mut r = row()
            .key(format!("{key_id}-price-actions"))
            .mt(tk.r(0.75))
            .gap(tk.r(0.5))
            .center_y()
            .flex_wrap()
            .child(preview)
            .child(
                kit::btn(
                    tk,
                    format!("{key_id}-price-cancel"),
                    Var::Ghost,
                    None,
                    &t("journals:quarantine.pair.cancel"),
                    true,
                )
                .on_press(move |a: &mut App| stop_entering(a, &k2)),
            );
        if c.checked
            && let model::PriceCheck::Bad { invalid, too_many } = &check
        {
            if !invalid.is_empty() {
                r = r.child(
                    text(tc("journals:quarantine.price.invalid", invalid.len(), &[]))
                        .font(tk.txs(400.0))
                        .color(tk.destructive)
                        .role("alert"),
                );
            } else if *too_many {
                r = r.child(
                    text(tc(
                        "journals:quarantine.price.tooMany",
                        model::MAX_FIX_OPERATIONS,
                        &[],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.destructive)
                    .role("alert"),
                );
            }
        }
        li = li.child(r);
    } else if !actions.is_empty()
        || (rows.members.is_empty() && !rows.root_ids.is_empty())
        || app.prefs.ai_features
        || accept
        || !price_rows.is_empty()
    {
        let mut r = row().mt(tk.r(0.75)).gap(tk.r(0.5)).flex_wrap();
        if accept {
            let scope = scope.clone().unwrap();
            r = r.child(
                kit::btn(tk, format!("{key_id}-accept"), Var::Default, None, &tc("journals:quarantine.accept.action", rows.root_count, &[]), true).on_press(move |a: &mut App| {
                    state::start_proposal(a, PropKind::Accept, scope.clone(), vec![serde_json::json!({ "type": "accept_coarse_prices", "reason": model::ACCEPT_PRICES_REASON })]);
                }),
            );
        }
        if !price_rows.is_empty() {
            let k = gkey.clone();
            let label = if accept {
                t("journals:quarantine.price.enterExact")
            } else {
                t("journals:quarantine.price.enter")
            };
            r = r.child(
                kit::btn(
                    tk,
                    format!("{key_id}-enter"),
                    if accept { Var::Outline } else { Var::Default },
                    None,
                    &label,
                    true,
                )
                .on_press(move |a: &mut App| state::card_mut(a, &k).entering = true),
            );
        }
        for (i, a) in actions.iter().enumerate() {
            let sync = a.kind == "sync_wallet";
            let busy = sync && app.review.rv.sync_pending;
            let disabled = a.kind == "wait_for_confirmation" || busy;
            let variant = if i == 0 && !primary_taken {
                Var::Default
            } else {
                Var::Outline
            };
            let label = if unlinked && a.kind == "pair_transfer" {
                t("journals:quarantine.cta.linkDifferently")
            } else {
                model::action_label(a)
            };
            let mut b = kit::btn_busy(
                tk,
                format!("{key_id}-act-{i}"),
                variant,
                busy,
                None,
                &label,
                !disabled,
            );
            if !disabled {
                let (g2, a2) = ((*g).clone(), (*a).clone());
                b = b.on_press(move |app: &mut App| run_action(app, &g2, &a2));
            }
            r = r.child(b);
        }
        if g.actions.is_empty()
            && rows.members.is_empty()
            && let Some(first) = rows.root_ids.first()
        {
            let (g2, id) = (g.clone(), first.clone());
            r = r.child(
                kit::btn(
                    tk,
                    format!("{key_id}-open"),
                    Var::Outline,
                    None,
                    &t("journals:quarantine.cta.openTransaction"),
                    true,
                )
                .on_press(move |a: &mut App| open_root(a, &g2, &id, None)),
            );
        }
        // `askAboutCause`: the cause's root pinned as typed context, the question prefilled.
        if let Some(root) = g
            .root_transaction_id
            .clone()
            .or_else(|| g.root_transaction_ids.first().cloned())
            .filter(|_| app.prefs.ai_features)
        {
            let date = g
                .root_occurred_at
                .clone()
                .or_else(|| g.earliest_occurred_at.clone());
            r = r.child(
                kit::btn(
                    tk,
                    format!("{key_id}-ask"),
                    Var::Ghost,
                    Some(&I::SPARKLES),
                    &t("assistant:panel.askAbout"),
                    true,
                )
                .on_press(move |a: &mut App| {
                    crate::assistant::ask_about(a, &root, date.as_deref(), true)
                }),
            );
        }
        li = li.child(r);
    }
    li
}

fn sanitize(k: &str) -> String {
    k.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

fn stop_entering(app: &mut App, key: &str) {
    let c = state::card_mut(app, key);
    c.entering = false;
    c.entries.clear();
    c.checked = false;
}

/// `previewPrices`: check what was typed, then preview exactly those prices.
fn preview_prices(
    app: &mut App,
    key: &str,
    order: &[String],
    labels: &[(String, String, String)],
    scope: Option<Value>,
    currency: Option<String>,
) {
    let c = state::card_mut(app, key);
    c.checked = true;
    let rate = c.rate;
    let check = model::price_entry_operations(order, &c.typed(), rate, model::decimal_separator());
    let (model::PriceCheck::Ok(ops), Some(scope)) = (check, scope) else {
        return;
    };
    let lines = ops
        .iter()
        .map(|op| {
            let id = s(op, "transaction_id");
            let (label, asset) = labels
                .iter()
                .find(|(i, _, _)| *i == id)
                .map(|(_, l, a)| (l.clone(), a.clone()))
                .unwrap_or_default();
            let value = if rate {
                s(op, "fiat_rate")
            } else {
                s(op, "fiat_value")
            };
            (label, value, asset)
        })
        .collect();
    state::start_proposal(
        app,
        PropKind::Prices {
            card: key.to_owned(),
            lines,
            rate,
            currency,
        },
        scope,
        ops,
    );
}

/// `CauseTransactionList`.
#[allow(clippy::too_many_arguments)]
fn transaction_list(
    app: &App,
    tk: &Tk,
    g: &Group,
    cause: &str,
    items: &[&Item],
    total: usize,
    price_rows: Option<&Vec<String>>,
    currency: Option<&str>,
    layer: &mut Vec<El>,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let st = app.review.rv.card(&g.key);
    let expanded = st.is_some_and(|c| c.expanded);
    let rate = st.is_some_and(|c| c.rate);
    let typed = |id: &str| {
        st.and_then(|c| c.entries.get(id))
            .is_some_and(|e| !e.text.trim().is_empty())
    };
    let shown: Vec<&Item> = if expanded {
        items.to_vec()
    } else {
        items
            .iter()
            .enumerate()
            .filter(|(i, it)| *i < SHOWN_ROWS || typed(&it.id))
            .map(|(_, it)| *it)
            .collect()
    };
    let key_id = format!("rv-cause-{}", sanitize(&g.key));
    let mut c = col()
        .key(format!("{key_id}-txs"))
        .w_full()
        .mt(tk.r(0.75))
        .pt(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
    let cur = currency.unwrap_or("");
    if let Some(rows) = price_rows {
        let assets: std::collections::BTreeSet<String> = items
            .iter()
            .filter(|i| rows.contains(&i.id))
            .map(|i| i.asset.clone())
            .collect();
        let rate_label = if assets.len() == 1 {
            tr(
                "journals:quarantine.price.unitRate",
                &[("asset", assets.iter().next().unwrap())],
            )
        } else {
            t("journals:quarantine.price.unitRateGeneric")
        };
        let hint = if cur.is_empty() {
            t("journals:quarantine.price.hintNoCurrency")
        } else {
            tr("journals:quarantine.price.hint", &[("currency", cur)])
        };
        let sel_key = format!("{key_id}-unit");
        let value = if rate {
            rate_label.clone()
        } else {
            t("journals:quarantine.price.unitValue")
        };
        let gk = g.key.clone();
        let trigger = row()
            .key(sel_key.clone())
            .h(tk.r(1.75))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.375))
            .border(1.0, tk.input)
            .fill(if tk.dark {
                tk.input.alpha(tk.input.a * 0.3)
            } else {
                Color::TRANSPARENT
            })
            .role("combobox")
            .label(t("journals:quarantine.price.unitLabel"))
            .a11y_value(value.clone())
            .focusable(true)
            .child(text(value).font(tk.txs(400.0)).color(tk.fg))
            .child(ic(&I::CHEVRON_DOWN, tk.r(1.0), tk.muted_fg.alpha(0.5)));
        let trigger = crate::ui::popup::trigger(
            trigger,
            crate::ui::popup::Kind::Listbox,
            app.review.popup == Some(Popup::Unit(g.key.clone())),
            move |a: &mut App| toggle(a, Popup::Unit(gk.clone())),
        );
        if app.review.popup == Some(Popup::Unit(g.key.clone())) {
            unit_menu(tk, layer, &sel_key, &g.key, rate, rate_label);
        }
        c = c.child(
            row()
                .w_full()
                .pb(tk.r(0.25))
                .gap(tk.r(0.5))
                .center_y()
                .justify_between()
                .flex_wrap()
                .child(text(hint).font(tk.txs(400.0)).color(tk.muted_fg))
                .child(trigger),
        );
    }
    let k = tk.kit();
    let mut form = crate::txdetail::widgets::form_kit(tk);
    form.ty.ui = tk.tsm(400.0).features("'tnum' 1");
    let _ = k;
    let mut ul = col().w_full().role("list");
    for (i, it) in shown.iter().enumerate() {
        let tone = direction_tone(tk, &it.direction, hidden);
        let (gid, gg) = (it.id.clone(), g.clone());
        let open = row()
            .key(format!("{key_id}-row-{}", it.id))
            .min_w(0.0)
            .grow(1.0)
            .py(tk.r(0.5))
            .gap(tk.r(0.75))
            .center_y()
            .radius(tk.r(0.125))
            .hover(tk.muted.alpha(0.4))
            .role("button")
            .label(format!("{} {}", model::row_meta(it), model::row_amount(it)))
            .focusable(true)
            .on_press(move |a: &mut App| open_root(a, &gg, &gid, None))
            .child(if hidden {
                ic(
                    if it.direction == "outbound" {
                        &I::ARROW_UP_RIGHT
                    } else {
                        &I::ARROW_DOWN_LEFT
                    },
                    tk.r(0.875),
                    tone,
                )
                .blur(3.5)
            } else {
                ic(
                    if it.direction == "outbound" {
                        &I::ARROW_UP_RIGHT
                    } else {
                        &I::ARROW_DOWN_LEFT
                    },
                    tk.r(0.875),
                    tone,
                )
            })
            .child(
                sensitive(model::row_meta(it), tk.tsm(400.0), tk.muted_fg, hidden)
                    .truncate()
                    .min_w(0.0)
                    .grow(1.0),
            )
            .child(sensitive(
                model::row_amount(it),
                tk.tsm(400.0).features("'tnum' 1"),
                tone,
                hidden,
            ))
            .child(ic(&I::ARROW_RIGHT, tk.r(0.875), tk.muted_fg));
        let mut li = row()
            .w_full()
            .gap(tk.r(0.75))
            .center_y()
            .role("listitem")
            .child(open);
        if i > 0 {
            li = li.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        if price_rows.is_some_and(|r| r.contains(&it.id)) {
            let unit = if rate {
                tr(
                    "journals:quarantine.price.unitRate",
                    &[("asset", &it.asset)],
                )
            } else {
                t("journals:quarantine.price.unitValue")
            };
            let label = if cur.is_empty() {
                tr(
                    "journals:quarantine.price.inputLabelNoCurrency",
                    &[("unit", &unit), ("row", &model::row_meta(it))],
                )
            } else {
                tr(
                    "journals:quarantine.price.inputLabel",
                    &[
                        ("unit", &unit),
                        ("row", &model::row_meta(it)),
                        ("currency", cur),
                    ],
                )
            };
            let invalid = st.is_some_and(|c| c.checked)
                && matches!(model::price_entry_operations(std::slice::from_ref(&it.id), &st.map(|c| c.typed()).unwrap_or_default(), rate, model::decimal_separator()), model::PriceCheck::Bad { ref invalid, .. } if !invalid.is_empty());
            let empty = rune::TextEdit::default();
            let edit = st.and_then(|c| c.entries.get(&it.id)).unwrap_or(&empty);
            let (gk, id) = (g.key.clone(), it.id.clone());
            let input = TextInput::new(edit)
                .label(label)
                .placeholder(cur.to_owned())
                .height(tk.r(2.0))
                .width(Some(tk.r(9.0)))
                .text_style(tk.tsm(400.0).features("'tnum' 1"))
                .invalid(invalid)
                .on_edit(move |a: &mut App| {
                    state::card_mut(a, &gk)
                        .entries
                        .entry(id.clone())
                        .or_default()
                })
                .build(&form)
                .key(format!("{key_id}-price-{}", it.id))
                .blur(if hidden { 3.5 } else { 0.0 });
            let mut cell = col().py(tk.r(0.25)).gap(tk.r(0.125)).items_end().child(
                crate::txdetail::widgets::field(tk, input)
                    .border(1.0, if invalid { tk.destructive } else { tk.input }),
            );
            if rate {
                cell = cell.child(text(unit).font(tk.t2xs(400.0)).color(tk.muted_fg));
            }
            li = li.child(cell);
        }
        ul = ul.child(li);
    }
    c = c.child(ul);
    if let Some(f) = list_footer(
        app,
        tk,
        &g.key,
        cause,
        shown.len(),
        items.len(),
        total,
        expanded,
    ) {
        c = c.child(f);
    }
    c
}

fn unit_menu(
    tk: &Tk,
    layer: &mut Vec<El>,
    anchor: &str,
    card: &str,
    rate: bool,
    rate_label: String,
) {
    let opt = |key: &str, label: String, checked: bool, pick: bool| {
        let card = card.to_owned();
        row()
            .key(key.to_owned())
            .w_full()
            .min_h(tk.r(2.0))
            .pr(tk.r(0.5))
            .gap(tk.r(0.375))
            .center_y()
            .radius(tk.r(0.25))
            .hover(tk.accent)
            .focus_highlight(tk.accent)
            .role("option")
            .checked(checked)
            .label(label.clone())
            .focusable(true)
            .focus_selected(checked)
            .on_press(move |a: &mut App| {
                state::card_mut(a, &card).rate = pick;
                a.review.popup = None;
            })
            .child(row().w(tk.r(1.5)).center().child(if checked {
                ic(&I::CHECK, tk.r(0.875), tk.fg)
            } else {
                el().size(1.0, 1.0)
            }))
            .child(text(label).font(tk.tsm(400.0)).color(tk.fg))
    };
    let mut p = crate::ui::popup::Popup::listbox(anchor, |a: &mut App| a.review.popup = None);
    let value_l = t("journals:quarantine.price.unitValue");
    p.item(
        opt(&format!("{anchor}-value"), value_l.clone(), !rate, false),
        format!("{anchor}-value"),
        value_l,
        true,
        !rate,
    );
    p.item(
        opt(&format!("{anchor}-rate"), rate_label.clone(), rate, true),
        format!("{anchor}-rate"),
        rate_label,
        true,
        rate,
    );
    let list = col()
        .min_w(tk.r(10.0))
        .padding(tk.r(0.25))
        .label(t("journals:quarantine.price.unitLabel"));
    p.mount(
        tk,
        layer,
        list,
        |l| crate::shell::glass(tk, l),
        Placement::BottomEnd,
        4.0,
    );
}

/// `ListFooter`: how much shows, and one way to see more.
#[allow(clippy::too_many_arguments)]
fn list_footer(
    app: &App,
    tk: &Tk,
    gkey: &str,
    cause: &str,
    shown: usize,
    loaded: usize,
    total: usize,
    expanded: bool,
) -> Option<El> {
    let q = &app.review.rv.q;
    let hidden = loaded > shown;
    let unloaded = total > loaded;
    let can_load = unloaded && q.has_more();
    let failed = unloaded && q.more_error.is_some();
    if !hidden && !unloaded && !(expanded && shown > SHOWN_ROWS) {
        return None;
    }
    let key_id = format!("rv-cause-{}", sanitize(gkey));
    let mut r = row()
        .w_full()
        .pt(tk.r(0.25))
        .gap(tk.r(0.75))
        .center_y()
        .flex_wrap();
    let all = total.max(loaded);
    if all > shown {
        r = r.child(
            text(tr(
                "journals:quarantine.causes.shownOf",
                &[
                    ("shown", &crate::format::count(shown)),
                    ("total", &crate::format::count(all)),
                ],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    if hidden || can_load {
        let busy = !hidden && q.more_loading;
        let mut l = kit::link_named(
            tk,
            format!("{key_id}-more"),
            &t("journals:quarantine.causes.showMoreRows"),
            tr(
                "journals:quarantine.causes.showMoreRowsOf",
                &[("cause", cause)],
            ),
            !busy,
            busy,
        );
        if !busy {
            let k = gkey.to_owned();
            l = l.on_press(move |a: &mut App| {
                state::card_mut(a, &k).expanded = true;
                if !hidden {
                    state::load_more(a);
                }
            });
        }
        r = r.child(l);
    }
    if expanded && shown > SHOWN_ROWS {
        let k = gkey.to_owned();
        r = r.child(
            kit::link_named(
                tk,
                format!("{key_id}-fewer"),
                &t("journals:quarantine.causes.showFewerTransactions"),
                tr(
                    "journals:quarantine.causes.showFewerRowsOf",
                    &[("cause", cause)],
                ),
                true,
                false,
            )
            .on_press(move |a: &mut App| state::card_mut(a, &k).expanded = false),
        );
    }
    if failed {
        r = r.child(
            text(tr(
                "journals:quarantine.causes.loadMoreFailed",
                &[("message", q.more_error.as_deref().unwrap_or(""))],
            ))
            .font(tk.txs(400.0))
            .color(tk.destructive)
            .role("alert"),
        );
    }
    Some(r)
}

/// One side of a pair (`PairLegLine`).
fn leg_line(tk: &Tk, label: String, leg: &Value, hidden: bool) -> El {
    let bitcoin = ["BTC", "LBTC"].contains(&s(leg, "asset").to_uppercase().as_str());
    let amount = if bitcoin {
        model::msat(model::num(leg, "amount_msat"))
    } else {
        s(leg, "asset")
    };
    row()
        .w_full()
        .gap(tk.r(0.75))
        .items_end()
        .child(
            text(label)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .w(tk.r(5.5)),
        )
        .child(
            sensitive(
                format!(
                    "{} · {}",
                    crate::format::short_date(leg["occurred_at"].as_str()),
                    s(leg, "wallet")
                ),
                tk.tsm(400.0),
                tk.fg,
                hidden,
            )
            .wrap()
            .min_w(0.0)
            .grow(1.0),
        )
        .child(sensitive(
            amount,
            tk.tsm(400.0).features("'tnum' 1"),
            tk.fg,
            hidden,
        ))
}

/// `QuarantinePairList`: each pair in its own box, picked one by one.
fn pair_list(
    app: &App,
    tk: &Tk,
    g: &Group,
    cause: &str,
    pairs: &[&Item],
    total: usize,
    scope: Option<Value>,
) -> El {
    let hidden = app.prefs.hide_sensitive();
    let st = app.review.rv.card(&g.key);
    let expanded = st.is_some_and(|c| c.expanded);
    let picked: Vec<Item> = st.map(|c| c.picked.clone()).unwrap_or_default();
    let shown: Vec<&Item> = if expanded {
        pairs.to_vec()
    } else {
        pairs.iter().take(SHOWN_ROWS).copied().collect()
    };
    let is_picked = |it: &Item| {
        picked
            .iter()
            .find(|p| p.pair_id() == it.pair_id())
            .is_some_and(|p| model::same_pair_case(p, Some(it)))
    };
    let picked_items: Vec<Item> = picked
        .iter()
        .filter(|p| {
            pairs
                .iter()
                .any(|i| i.pair_id() == p.pair_id() && model::same_pair_case(p, Some(i)))
        })
        .cloned()
        .collect();
    let lapsed = picked.len() - picked_items.len();
    let full = picked_items.len() >= model::MAX_FIX_OPERATIONS;
    let key_id = format!("rv-cause-{}", sanitize(&g.key));
    let mut c = col()
        .key(format!("{key_id}-pairs"))
        .w_full()
        .mt(tk.r(0.75))
        .pt(tk.r(0.75))
        .gap(tk.r(0.5))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .child(
            text(tc("journals:quarantine.pair.title", pairs.len(), &[]))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        );
    let mut ul = col().w_full().gap(tk.r(0.5)).role("list");
    for it in &shown {
        let (o, i) = it.pair_legs().unwrap();
        let facts = model::cause_facts(it.evidence());
        let on = is_picked(it);
        let mut li = kit::inset(tk).role("listitem").clip_rounded(tk.r(0.5));
        let mut line = row().w_full().gap(tk.r(0.5)).items_stretch();
        if pairs.len() > 1 {
            let (gk, item) = (g.key.clone(), (*it).clone());
            let mut cb = kit::checkbox(
                tk,
                format!("{key_id}-pick-{}", it.id),
                on,
                on || !full,
                tr(
                    "journals:quarantine.pair.pick",
                    &[
                        ("outWallet", &s(o, "wallet")),
                        ("inWallet", &s(i, "wallet")),
                    ],
                ),
            );
            if on || !full {
                cb = cb.on_press(move |a: &mut App| {
                    let c = state::card_mut(a, &gk);
                    let pid = item.pair_id();
                    if let Some(pos) = c.picked.iter().position(|p| p.pair_id() == pid) {
                        c.picked.remove(pos);
                        if !on {
                            c.picked.push(item.clone());
                        }
                    } else {
                        c.picked.push(item.clone());
                    }
                });
            }
            line = line.child(row().pl(tk.r(0.75)).center_y().child(cb));
        }
        let mut legs = col()
            .min_w(0.0)
            .grow(1.0)
            .gap(tk.r(0.25))
            .child(leg_line(tk, t("journals:quarantine.pair.sent"), o, hidden))
            .child(leg_line(
                tk,
                t("journals:quarantine.pair.received"),
                i,
                hidden,
            ));
        if !facts.is_empty() {
            legs = legs.child(
                text(facts.join(" "))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        let (gg, id) = (g.clone(), it.id.clone());
        line = line.child(
            row()
                .key(format!("{key_id}-pair-{}", it.id))
                .min_w(0.0)
                .grow(1.0)
                .padding(tk.r(0.75))
                .gap(tk.r(0.75))
                .center_y()
                .hover(tk.muted.alpha(0.4))
                .role("button")
                .label(format!(
                    "{} {} → {}",
                    t("journals:quarantine.pair.sent"),
                    s(o, "wallet"),
                    s(i, "wallet")
                ))
                .focusable(true)
                .on_press(move |a: &mut App| open_root(a, &gg, &id, None))
                .child(legs)
                .child(ic(&I::ARROW_RIGHT, tk.r(0.875), tk.muted_fg)),
        );
        let (item, sc) = ((*it).clone(), scope.clone());
        line = line.child(
            row().pr(tk.r(0.75)).center_y().child(
                kit::btn(
                    tk,
                    format!("{key_id}-unpair-{}", it.id),
                    Var::Outline,
                    None,
                    &t("journals:quarantine.pair.unpair"),
                    true,
                )
                .on_press(move |a: &mut App| state::start_fix(a, vec![item.clone()], sc.clone())),
            ),
        );
        li = li.child(line);
        ul = ul.child(li);
    }
    c = c.child(ul);
    if lapsed > 0 {
        c = c.child(
            text(tc("journals:quarantine.pair.tickLapsed", lapsed, &[]))
                .font(tk.txs(400.0))
                .color(kit::amber_text(tk))
                .role("status")
                .wrap(),
        );
    }
    if !picked_items.is_empty() || lapsed > 0 {
        let mut r = row().gap(tk.r(0.5)).center_y().flex_wrap();
        if !picked_items.is_empty() {
            let (items, sc) = (picked_items.clone(), scope.clone());
            r = r.child(
                kit::btn(
                    tk,
                    format!("{key_id}-unpair-picked"),
                    Var::Default,
                    None,
                    &tc(
                        "journals:quarantine.pair.unpairPicked",
                        picked_items.len(),
                        &[],
                    ),
                    true,
                )
                .on_press(move |a: &mut App| state::start_fix(a, items.clone(), sc.clone())),
            );
        }
        let gk = g.key.clone();
        r = r.child(
            kit::btn(
                tk,
                format!("{key_id}-clear-picked"),
                Var::Ghost,
                None,
                &t("journals:quarantine.pair.clearPicked"),
                true,
            )
            .on_press(move |a: &mut App| state::card_mut(a, &gk).picked.clear()),
        );
        if full {
            r = r.child(
                text(tc(
                    "journals:quarantine.pair.pickLimit",
                    model::MAX_FIX_OPERATIONS,
                    &[],
                ))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            );
        }
        c = c.child(r);
    }
    if let Some(f) = list_footer(
        app,
        tk,
        &g.key,
        cause,
        shown.len(),
        pairs.len(),
        total,
        expanded,
    ) {
        c = c.child(f);
    }
    c
}

/// `QuarantineQueue`: what only waits on a cause, paged.
fn queue(app: &App, tk: &Tk, att: &Snap) -> El {
    let hidden = app.prefs.hide_sensitive();
    let q = &app.review.rv.q;
    let waiting = q.waiting_snap();
    let items: Vec<Item> = waiting
        .as_ref()
        .map(|w| w.items.clone())
        .unwrap_or_default();
    let total = waiting
        .as_ref()
        .map(Snap::scope_count)
        .filter(|_| {
            waiting
                .as_ref()
                .is_some_and(|w| w.summary["scope_count"].is_number())
        })
        .unwrap_or_else(|| att.waiting_count());
    let title = t("journals:quarantine.queue.title");
    let mut sec = surface(tk)
        .key("rv-queue")
        .w_full()
        .role("region")
        .label(title.clone())
        .clip_rounded(tk.r(0.75))
        .child(
            row()
                .w_full()
                .padding(tk.r(0.75))
                .px(tk.r(1.0))
                .gap(tk.r(0.5))
                .center_y()
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .child(
                    text(title.clone())
                        .font(tk.tbase(500.0))
                        .color(tk.fg)
                        .role("heading")
                        .level(2),
                )
                .child(
                    text(crate::format::count(total))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.muted_fg),
                )
                .child(el().grow(1.0))
                .child(
                    kit::btn(
                        tk,
                        "rv-queue-hide",
                        Var::Ghost,
                        None,
                        &t("journals:quarantine.queue.hide"),
                        true,
                    )
                    .h(tk.r(1.75))
                    .on_press(|a: &mut App| state::set_waiting_shown(a, false)),
                ),
        );
    if let Some(e) = &q.waiting_error {
        let mut b = col()
            .w_full()
            .px(tk.r(1.0))
            .py(tk.r(2.0))
            .gap(tk.r(0.5))
            .items_center()
            .role("alert")
            .child(
                text(t("journals:quarantine.queue.loadError"))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            );
        if !e.is_empty() {
            b = b.child(
                text(e.clone())
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            );
        }
        sec = sec.child(
            b.child(
                kit::btn(
                    tk,
                    "rv-queue-retry",
                    Var::Outline,
                    None,
                    &t("common:actions.retry"),
                    true,
                )
                .on_press(state::retry_waiting),
            ),
        );
        return sec;
    }
    if q.waiting_loading {
        sec = sec.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(2.0))
                .justify_center()
                .role("status")
                .child(kit::spinner(tk, tk.r(1.0))),
        );
    } else if items.is_empty() {
        sec = sec.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(2.0))
                .justify_center()
                .child(
                    text(t("journals:quarantine.queue.waitingEmpty"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                ),
        );
    } else {
        let mut ul = col().key("rv-queue-list").w_full().role("list");
        for (n, it) in items.iter().enumerate() {
            let tab =
                model::sheet_tab(&it.reason, &it.category_or_derived(), it.evidence()).to_owned();
            let id = it.id.clone();
            let ctx = super::links::item_context(it);
            let mut li = row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.5))
                .gap(tk.r(0.75))
                .center_y()
                .flex_wrap()
                .role("listitem")
                .child(
                    row()
                        .key(format!("rv-wait-{}", it.id))
                        .min_w(0.0)
                        .grow(1.0)
                        .gap(tk.r(0.75))
                        .center_y()
                        .justify_between()
                        .role("button")
                        .label(format!("{} {}", model::row_meta(it), model::row_amount(it)))
                        .focusable(true)
                        .hover_group()
                        .on_press(move |a: &mut App| {
                            super::links::open_waiting_row(a, &id, &tab, ctx.clone())
                        })
                        .child(
                            sensitive(model::row_meta(it), tk.txs(400.0), tk.muted_fg, hidden)
                                .truncate()
                                .min_w(0.0)
                                .grow(1.0),
                        )
                        .child(sensitive(
                            model::row_amount(it),
                            tk.tsm(400.0).features("'tnum' 1"),
                            tk.muted_fg,
                            hidden,
                        )),
                );
            if let Some(root) = it.root() {
                let rid = s(root, "transaction_id");
                li = li.child(
                    row()
                        .key(format!("rv-wait-root-{}", it.id))
                        .gap(tk.r(0.25))
                        .center_y()
                        .radius(tk.r(0.125))
                        .role("button")
                        .label(tr(
                            "journals:quarantine.queue.waitsOn",
                            &[("root", &model::root_label(root))],
                        ))
                        .focusable(true)
                        .on_press(move |a: &mut App| {
                            super::links::open_waiting_row(a, &rid, "details", None)
                        })
                        .child(sensitive(
                            tr(
                                "journals:quarantine.queue.waitsOn",
                                &[("root", &model::root_label(root))],
                            ),
                            tk.txs(400.0),
                            tk.muted_fg,
                            hidden,
                        ))
                        .child(ic(&I::ARROW_RIGHT, tk.r(0.75), tk.muted_fg)),
                );
            }
            if n > 0 {
                li = li.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
            }
            ul = ul.child(li);
        }
        sec = sec.child(ul);
    }
    if total > PAGE {
        let end = (q.offset + items.len()).min(total);
        let from = if items.is_empty() { 0 } else { q.offset + 1 };
        let off = q.offset;
        let mut prev = kit::btn(
            tk,
            "rv-queue-prev",
            Var::Outline,
            None,
            &t("journals:quarantine.paging.previous"),
            off > 0,
        );
        if off > 0 {
            prev = prev.on_press(move |a: &mut App| state::set_offset(a, off.saturating_sub(PAGE)));
        }
        let mut next = kit::btn(
            tk,
            "rv-queue-next",
            Var::Outline,
            None,
            &t("journals:quarantine.paging.next"),
            end < total,
        );
        if end < total {
            next = next.on_press(move |a: &mut App| state::set_offset(a, off + PAGE));
        }
        sec = sec.child(
            row()
                .w_full()
                .px(tk.r(1.0))
                .py(tk.r(0.5))
                .gap(tk.r(0.5))
                .center_y()
                .justify_end()
                .role("navigation")
                .label(title)
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    text(tr(
                        "journals:quarantine.paging.range",
                        &[
                            ("from", &crate::format::count(from)),
                            ("to", &crate::format::count(end)),
                            ("total", &crate::format::count(total)),
                        ],
                    ))
                    .font(tk.txs(400.0).features("'tnum' 1"))
                    .color(tk.muted_fg),
                )
                .child(prev)
                .child(next),
        );
    }
    sec
}
