//! The Linked tab (TransactionLinkedTab.tsx): paired movement with Unpair, loan legs with Link,
//! and the journal entries of the row.

use rune::{El, col, row, text};
use serde_json::Value;

use super::Open;
use super::draft::{self, f, opt_s, s};
use crate::app::App;
use crate::format;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

fn amount(v: f64, asset: &str) -> String {
    format!(
        "{:.8} {}",
        (v / 1e8).abs(),
        if asset.is_empty() { "BTC" } else { asset }
    )
}

fn short_loan(id: &str) -> String {
    if id.chars().count() > 12 {
        let head: String = id.chars().take(8).collect();
        let tail: String = id
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("{head}...{tail}")
    } else {
        id.to_owned()
    }
}

pub fn role_label(role: &str) -> String {
    t(match role {
        "collateral_release" => "transactions:table.row.collateral.returnedAccounting",
        "loan_principal_received" => {
            "transactions:table.row.collateral.principalReceivedAccounting"
        }
        "loan_principal_repaid" => "transactions:table.row.collateral.principalRepaidAccounting",
        _ => "transactions:table.row.collateral.collateralAccounting",
    })
}

pub fn linked(app: &App, tk: &Tk, o: &Open, r: &Value) -> El {
    use super::widgets::{
        ledger_header, ledger_row, ledger_table, ledger_value, outline_badge_2xs,
    };
    let hide = app.prefs.hide_sensitive();
    // Unpair, pairing review and loan links are wired by the Transactions page; the Review page
    // wires pairing review always and Unpair only for a pair that is one of its cases.
    let host = super::host_loans(app);
    let review = super::review_host(app).is_some();
    // `space-y-3`.
    let mut body = col().key("txd-linked").w_full().gap(tk.r(0.75));
    match r.get("pair").filter(|p| p.is_object()) {
        Some(p) => {
            let mut action = None;
            if (host || review && crate::review::sheet_can_unpair(app, r))
                && s(p, "kind") != "journal-derived"
            {
                let pid = opt_s(p, "id").unwrap_or_default();
                let busy = if review {
                    crate::review::sheet_unpairing(app)
                } else {
                    o.busy.unpair
                };
                let mut b = super::widgets::button(
                    tk,
                    "txd-unpair",
                    super::widgets::Variant::Ghost,
                    super::widgets::Size::Xs,
                    None,
                    &if busy {
                        t("transactions:linked.unpairing")
                    } else {
                        t("transactions:linked.unpair")
                    },
                    !busy,
                );
                if !busy {
                    b = b.on_press(move |a: &mut App| super::actions::unpair(a, &pid));
                }
                action = Some(b);
            }
            let fee = f(p, "feeSat")
                .filter(|v| *v != 0.0)
                .map(|v| draft::btc_amount((v / 1e8).abs()))
                .unwrap_or_else(|| "-".into());
            let kind = opt_s(p, "kind");
            let mut c = ledger_table(tk)
                .key("txd-pair")
                .child(ledger_header(
                    tk,
                    &I::REPEAT_2,
                    &t("transactions:linked.pairedMovement"),
                    opt_s(p, "policy").map(|pol| outline_badge_2xs(tk, &pol.to_uppercase())),
                    action,
                ))
                .child(ledger_row(
                    tk,
                    &t("transactions:linked.outWallet"),
                    None,
                    ledger_value(
                        tk,
                        opt_s(p, "outWallet").unwrap_or_else(|| t("transactions:linked.unknown")),
                        false,
                    ),
                    false,
                    false,
                ))
                .child(ledger_row(
                    tk,
                    &t("transactions:linked.outAmount"),
                    None,
                    ledger_value(
                        tk,
                        amount(f(p, "outAmountSat").unwrap_or(0.0), s(p, "outAsset")),
                        hide,
                    ),
                    false,
                    false,
                ))
                .child(ledger_row(
                    tk,
                    &t("transactions:linked.inWallet"),
                    None,
                    ledger_value(
                        tk,
                        opt_s(p, "inWallet").unwrap_or_else(|| t("transactions:linked.unknown")),
                        false,
                    ),
                    false,
                    false,
                ))
                .child(ledger_row(
                    tk,
                    &t("transactions:linked.inAmount"),
                    None,
                    ledger_value(
                        tk,
                        amount(f(p, "inAmountSat").unwrap_or(0.0), s(p, "inAsset")),
                        hide,
                    ),
                    false,
                    false,
                ))
                .child(ledger_row(
                    tk,
                    &t("transactions:linked.pairFee"),
                    Some(t("transactions:linked.pairFeeHint")),
                    ledger_value(tk, fee, hide),
                    true,
                    kind.is_none(),
                ));
            if let Some(k) = kind {
                c = c.child(ledger_row(
                    tk,
                    &t("transactions:linked.pairKind"),
                    None,
                    ledger_value(tk, k, false),
                    true,
                    true,
                ));
            }
            body = body.child(c);
        }
        None => {
            // `rounded-md border border-dashed bg-muted/40 p-4 text-sm`.
            let id = s(r, "id").to_owned();
            let mut c = col()
                .key("txd-no-pair")
                .w_full()
                .padding(tk.r(1.0))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_dash(4.0, 3.0)
                .fill(tk.muted.alpha(0.4))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::REPEAT_2, tk.r(1.0), tk.muted_fg))
                        .child(
                            text(t("transactions:linked.noPairedMovement"))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        ),
                )
                .child(
                    text(t("transactions:linked.noPairedMovementBody"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .wrap()
                        .mt(tk.r(0.25)),
                );
            if host || review {
                c = c.child(
                    row().mt(tk.r(0.75)).child(
                        super::widgets::button_with(
                            tk,
                            "txd-open-pairing",
                            super::widgets::Variant::Outline,
                            super::widgets::Size::Sm,
                            Some(&I::REPEAT_2),
                            Some(tk.r(0.875)),
                            &t("transactions:linked.openPairingReview"),
                            true,
                        )
                        .on_press(move |a: &mut App| {
                            if super::review_host(a).is_some() {
                                return crate::review::open_pairing_review(a);
                            }
                            super::close(a);
                            a.navigate(&format!(
                                "/transfers?link={}",
                                if id.is_empty() { "new" } else { id.as_str() }
                            ));
                        }),
                    ),
                );
            }
            body = body.child(c);
        }
    }
    // Loan legs (the Transactions page passes `loanMark`).
    if let Some(mark) = super::tabs::loan_of(app, s(r, "id")).filter(|_| host) {
        let loan_id = s(&mark, "loan_id").to_owned();
        let all = app.detail.loans.clone().unwrap_or_default();
        let linked: Vec<&Value> = all
            .iter()
            .filter(|m| {
                !loan_id.is_empty()
                    && s(m, "loan_id") == loan_id
                    && s(m, "transaction_id") != s(r, "id")
            })
            .collect();
        let mut c = ledger_table(tk)
            .key("txd-loan")
            .child(ledger_header(
                tk,
                &I::COINS,
                &t("transactions:linked.loanLegs"),
                None,
                (!loan_id.is_empty()).then(|| {
                    outline_badge_2xs(
                        tk,
                        &tr(
                            "transactions:linked.loanId",
                            &[("id", &short_loan(&loan_id))],
                        )
                        .to_uppercase(),
                    )
                }),
            ))
            .child(ledger_row(
                tk,
                &t("transactions:linked.loanThisLeg"),
                None,
                ledger_value(tk, role_label(s(&mark, "role")), false),
                false,
                false,
            ));
        let leg = |m: &Value| {
            let sub = format!(
                "{}{}",
                opt_s(m, "description").unwrap_or_else(|| short_loan(s(m, "transaction_id"))),
                opt_s(m, "occurred_at")
                    .map(|d| format!(" · {d}"))
                    .unwrap_or_default()
            );
            col()
                .min_w(0.0)
                .grow(1.0)
                .child(
                    text(role_label(s(m, "role")))
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .truncate(),
                )
                .child(
                    text(sub.clone())
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .truncate()
                        .help_when_truncated(sub),
                )
        };
        if linked.is_empty() {
            c = c.child(
                row()
                    .w_full()
                    .px(tk.r(0.75))
                    .py(tk.r(0.5))
                    .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                    .child(
                        text(t("transactions:linked.noLoanLinks"))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                    ),
            );
        }
        for m in &linked {
            c = c.child(
                row()
                    .w_full()
                    .px(tk.r(0.75))
                    .py(tk.r(0.5))
                    .gap(tk.r(0.75))
                    .center_y()
                    .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                    .child(leg(m))
                    .child(outline_badge_2xs(tk, &t("transactions:linked.loanLinked"))),
            );
        }
        let candidates: Vec<&Value> = all
            .iter()
            .filter(|m| {
                s(m, "transaction_id") != s(r, "id")
                    && (loan_id.is_empty() || s(m, "loan_id") != loan_id)
            })
            .take(6)
            .collect();
        if !candidates.is_empty() {
            // `border-t bg-muted/30 px-3 py-2`, cards `rounded-md border bg-background px-2 py-1.5`.
            let mut cand = col()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.375))
                .fill(tk.muted.alpha(0.3))
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .child(
                    text(t("transactions:linked.loanLinkCandidates").to_uppercase())
                        .font(tk.t2xs(600.0).tracking(0.25))
                        .color(tk.muted_fg)
                        .mb(tk.r(0.125)),
                );
            for m in candidates {
                let target = s(m, "transaction_id").to_owned();
                let busy = o.busy.link.as_deref() == Some(target.as_str());
                let label = if busy {
                    t("transactions:linked.loanLinking")
                } else {
                    t("transactions:linked.loanLink")
                };
                let mut b = super::widgets::button(
                    tk,
                    format!("txd-loan-link-{target}"),
                    super::widgets::Variant::Outline,
                    super::widgets::Size::Xs,
                    None,
                    &label,
                    o.busy.link.is_none(),
                );
                if o.busy.link.is_none() {
                    let tg = target.clone();
                    b = b.on_press(move |a: &mut App| super::actions::loan_link(a, &tg));
                }
                cand = cand.child(
                    row()
                        .w_full()
                        .px(tk.r(0.5))
                        .py(tk.r(0.375))
                        .gap(tk.r(0.75))
                        .center_y()
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .fill(tk.background)
                        .child(leg(m))
                        .child(b),
                );
            }
            c = c.child(cand);
        }
        body = body.child(c);
    }
    // Journal entries.
    let events: Vec<Value> = o
        .events
        .as_ref()
        .and_then(|e| e["events"].as_array().cloned())
        .unwrap_or_default();
    let mut j = ledger_table(tk).key("txd-journal").child(ledger_header(
        tk,
        &I::FILE_TEXT,
        &t("transactions:linked.journalEntries"),
        None,
        None,
    ));
    if events.is_empty() {
        j = j.child(
            row().w_full().padding(tk.r(0.75)).child(
                text(t("transactions:linked.noJournalEntries"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            ),
        );
    }
    let n = events.len();
    for (i, e) in events.iter().enumerate() {
        let label = [opt_s(e, "entryType"), opt_s(e, "atCategory")]
            .into_iter()
            .flatten()
            .map(|x| entry_label(&x))
            .collect::<Vec<_>>()
            .join(" · ");
        let v = format!(
            "{:.8} {} · {}",
            f(e, "quantity").unwrap_or(0.0),
            s(e, "asset"),
            f(e, "fiatValueEur")
                .map(|x| format::fiat(x, &super::tax::row_currency(Some(app), r)))
                .unwrap_or_else(|| "—".into())
        );
        j = j.child(ledger_row(
            tk,
            &label,
            opt_s(e, "description"),
            ledger_value(tk, v, hide),
            false,
            i + 1 == n,
        ));
    }
    body.child(j)
}

/// `ledger.entryType.*`, else "Neu Gain"-style title case.
fn entry_label(code: &str) -> String {
    let camel: String = code
        .split('_')
        .enumerate()
        .map(|(i, w)| {
            if i == 0 {
                w.to_owned()
            } else {
                w[..1].to_uppercase() + &w[1..]
            }
        })
        .collect();
    let key = format!("journals:ledger.entryType.{camel}");
    if crate::i18n::has(&key) {
        return t(&key);
    }
    code.split('_')
        .filter(|w| !w.is_empty())
        .map(|w| w[..1].to_uppercase() + &w[1..])
        .collect::<Vec<_>>()
        .join(" ")
}
