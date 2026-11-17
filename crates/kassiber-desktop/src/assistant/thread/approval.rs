//! Rows and cards the user acts on: the pending-approval row (PendingApprovalRow.tsx) with its
//! plan (PlanConsentBody.tsx, planConsent.ts) or review proposal (ReviewWorkflowCard.tsx,
//! reviewWorkflow.ts), the review receipt, and the evidence and connection request cards
//! (EvidenceRequestCard.tsx, WalletInputRequestCard.tsx).
//!
//! Decline and Approve answer the daemon once (`ai.tool_call.consent`); More offers "Allow for
//! this chat" where the daemon allows it and "Stop the answer". Plan-bound and review approvals
//! run exactly the plan the daemon showed, so without a current one only Decline is live.

use rune::{Color, El, col, row, text};
use serde_json::Value;

use super::{is_open, toggle};
use crate::app::App;
use crate::assistant::model::{Consent, ToolCall, ToolStatus};
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::ui::ic;

const PLAN_BOUND: [&str; 3] = [
    "ui.wallets.import_attachment.apply",
    "ui.wallets.import_document.apply",
    "ui.imports.rollback",
];

fn amber(tk: &Tk) -> Color {
    rune::color::hex(if tk.dark { "#fbbf24" } else { "#b45309" }).unwrap_or(tk.warn.text)
}

fn obj(v: &Value) -> bool {
    v.is_object()
}

fn count_ok(v: &Value) -> bool {
    v.as_u64().is_some()
}

fn effects_ok(v: &Value) -> bool {
    obj(v)
        && count_ok(&v["entries_count"])
        && count_ok(&v["quarantine_count"])
        && v["report_ready"].is_boolean()
        && v["quarantines"].is_array()
}

const OPERATION_TYPES: [&str; 10] = [
    "price_override",
    "exclude",
    "custody_component",
    "unpair",
    "pair",
    "dismiss_pair",
    "undismiss_pair",
    "custody_gap",
    "kind_override",
    "accept_coarse_prices",
];

/// `reviewArtifact`: a well-formed plan, or `None`.
pub fn review_artifact(v: &Value) -> Option<&Value> {
    let ops = v["operations"].as_array()?;
    let digest_ok = v["digest"].as_str().is_some_and(|d| {
        d.len() == 64
            && d.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    });
    (v["schema_version"].as_u64() == Some(1)
        && v["workspace_id"].is_string()
        && v["profile_id"].is_string()
        && count_ok(&v["base_input_version"])
        && digest_ok
        && !ops.is_empty()
        && ops.len() <= 50
        && ops
            .iter()
            .all(|o| o.is_object() && OPERATION_TYPES.contains(&o["type"].as_str().unwrap_or("")))
        && effects_ok(&v["before"])
        && effects_ok(&v["after"]))
    .then_some(v)
}

/// `reviewReceipt`: a verified receipt, or `None`.
pub fn review_receipt(v: &Value) -> Option<&Value> {
    (v["schema_version"].as_u64() == Some(1)
        && v["status"].as_str() == Some("verified")
        && v["id"].is_string()
        && v["artifact_digest"].is_string()
        && v["created_at"].is_string()
        && count_ok(&v["result_input_version"])
        && v["operations"].is_array()
        && effects_ok(&v["verification"])
        && (v.get("before").is_none() || effects_ok(&v["before"])))
    .then_some(v)
}

/// `reviewToolResult`: a plan or receipt a finished review call returned.
pub fn review_result(c: &ToolCall) -> Option<(bool, &Value)> {
    if c.status != ToolStatus::Done {
        return None;
    }
    let r = c.result.as_ref()?;
    match r["kind"].as_str()? {
        "ui.review.plan" => review_artifact(&r["data"]).map(|a| (true, a)),
        "ui.review.apply" | "ui.review.receipt" => review_receipt(&r["data"]).map(|a| (false, a)),
        _ => None,
    }
}

fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

/// `formatCustodyMsat`: exact msat as `1.23456789 BTC`.
pub fn custody_msat(v: &Value, asset: &str) -> String {
    let raw = text_of(v);
    let Ok(msat) = raw.parse::<i128>() else {
        return format!("— {asset}");
    };
    let sign = if msat < 0 { "−" } else { "" };
    let abs = msat.unsigned_abs();
    let whole = abs / 100_000_000_000;
    let frac = format!("{:011}", abs % 100_000_000_000);
    let frac = frac.trim_end_matches('0');
    if frac.is_empty() {
        format!("{sign}{whole} {asset}")
    } else {
        format!("{sign}{whole}.{frac} {asset}")
    }
}

/// A `<dl>` row: label left (`text-xs muted`), value right (`font-medium tabular-nums`).
fn fact(tk: &Tk, label: &str, value: El) -> El {
    row()
        .w_full()
        .flex_wrap()
        .gap(tk.r(0.75))
        .child(
            text(label.to_owned())
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .grow(1.0),
        )
        .child(value)
}

fn value_text(tk: &Tk, s: impl Into<String>) -> El {
    text(s.into())
        .font(tk.tsm(500.0).features("'tnum' 1"))
        .color(tk.fg)
}

fn h4(tk: &Tk, s: String) -> El {
    text(s).font(tk.txs(500.0)).color(tk.muted_fg)
}

/// An amber notice box (`rounded-md border border-amber-500/30 bg-amber-500/5 p-3 text-sm`).
fn amber_box(tk: &Tk) -> El {
    col()
        .w_full()
        .gap(tk.r(0.25))
        .radius(tk.r(0.375))
        .border(1.0, tk.warn.border.alpha(0.3))
        .border_layout()
        .fill(tk.warn.bg.alpha(0.05))
        .padding(tk.r(0.75))
}

fn blockers(tk: &Tk, list: &Value) -> Option<El> {
    let items = list.as_array().filter(|a| !a.is_empty())?;
    let mut b = amber_box(tk).role("alert").child(
        text(t("assistant:consent.plan.blockers"))
            .font(tk.txs(500.0))
            .color(tk.fg),
    );
    for item in items {
        let code = item["code"].as_str().unwrap_or("");
        let key = format!("assistant:consent.plan.blocker.{code}");
        let label = if crate::i18n::has(&key) {
            t(&key)
        } else {
            item["message"]
                .as_str()
                .filter(|m| !m.is_empty())
                .unwrap_or(code)
                .to_owned()
        };
        let mut li = col()
            .w_full()
            .child(text(label).font(tk.tsm(400.0)).color(tk.fg).wrap());
        if item["remediation"]["desktop_only"].as_bool() == Some(true) {
            li = li.child(
                text(t("assistant:consent.plan.desktopOnly"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        }
        b = b.child(li);
    }
    Some(b)
}

/// `PlanConsentBody`: the import or rollback plan the daemon recomputed when it asked.
pub fn plan_body(app: &App, tk: &Tk, name: &str, preview: &Value) -> El {
    let status = preview["status"].as_str().unwrap_or("unavailable");
    if !matches!(status, "ready" | "blocked") {
        let mut b = amber_box(tk).role("alert").child(
            text(t(if status == "stale" {
                "assistant:consent.plan.stale"
            } else {
                "assistant:consent.plan.unavailable"
            }))
            .font(tk.tsm(400.0))
            .color(tk.fg)
            .wrap(),
        );
        if let Some(code) = preview["code"].as_str().filter(|c| !c.is_empty()) {
            b = b.child(
                text(code.to_owned())
                    .font(tk.txs(400.0).mono())
                    .color(tk.fg)
                    .mt(tk.r(0.5)),
            );
        }
        return b;
    }
    let plan = &preview["plan"];
    let num = |v: &Value| v.as_u64().unwrap_or(0).to_string();
    let hide = app.prefs.hide_sensitive();
    let mut c = col().w_full().gap(tk.r(1.0));
    let mut dl = col().w_full().gap(tk.r(0.5));
    if name == "ui.imports.rollback" {
        dl = dl.child(fact(
            tk,
            &t("assistant:consent.plan.rowsToDelete"),
            value_text(tk, num(&plan["transactions_to_delete"])),
        ));
        if let Some(at) = plan["batch"]["imported_at"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            dl = dl.child(fact(
                tk,
                &t("assistant:consent.plan.importedAt"),
                value_text(tk, at),
            ));
        }
        if let Some(f) = plan["batch"]["source_format"]
            .as_str()
            .filter(|s| !s.is_empty())
        {
            dl = dl.child(fact(
                tk,
                &t("assistant:consent.plan.sourceFormat"),
                value_text(tk, f),
            ));
        }
        c = c.child(dl);
        let removed: Vec<(String, u64)> = plan["also_removed"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(k, v)| v.as_u64().filter(|n| *n > 0).map(|n| (k.clone(), n)))
            .collect();
        if !removed.is_empty() {
            let mut s = col()
                .w_full()
                .gap(tk.r(0.25))
                .child(h4(tk, t("assistant:consent.plan.alsoRemoved")));
            for (table, n) in removed {
                s = s.child(
                    text(format!("{table}: {n}"))
                        .font(tk.txs(400.0).mono())
                        .color(tk.fg),
                );
            }
            c = c.child(s);
        }
    } else {
        let target = &plan["target"];
        let label = target["label"].as_str().unwrap_or("");
        let into = tr(
            if target["action"].as_str() == Some("create") {
                "assistant:consent.plan.newWallet"
            } else {
                "assistant:consent.plan.existingWallet"
            },
            &[("label", label)],
        );
        dl = dl.child(fact(
            tk,
            &t("assistant:consent.plan.target"),
            value_text(tk, into),
        ));
        dl = dl.child(fact(
            tk,
            &t("assistant:consent.plan.rowsToImport"),
            value_text(tk, num(&plan["rows"]["to_import"])),
        ));
        dl = dl.child(fact(
            tk,
            &t("assistant:consent.plan.rowsPresent"),
            value_text(tk, num(&plan["rows"]["already_present"])),
        ));
        if plan["rows"]["would_update_existing"].as_u64().unwrap_or(0) > 0 {
            dl = dl.child(fact(
                tk,
                &t("assistant:consent.plan.rowsUpdated"),
                value_text(tk, num(&plan["rows"]["would_update_existing"])),
            ));
        }
        let range = &plan["date_range"];
        let dates = if range.is_object() {
            format!(
                "{} → {}",
                range["first"].as_str().unwrap_or("—"),
                range["last"].as_str().unwrap_or("—")
            )
        } else {
            t("assistant:consent.plan.withheld")
        };
        dl = dl.child(fact(
            tk,
            &t("assistant:consent.plan.dateRange"),
            value_text(tk, dates),
        ));
        for (k, label) in [
            (
                "existing_rows_in_date_range",
                "assistant:consent.plan.overlapRange",
            ),
            (
                "external_ids_in_other_wallets",
                "assistant:consent.plan.overlapOther",
            ),
        ] {
            if plan["overlap"][k].as_u64().unwrap_or(0) > 0 {
                dl = dl.child(fact(
                    tk,
                    &t(label),
                    value_text(tk, num(&plan["overlap"][k])),
                ));
            }
        }
        c = c.child(dl);
        if let Some(totals) = plan["totals"].as_array().filter(|a| !a.is_empty()) {
            let mut s = col()
                .w_full()
                .gap(tk.r(0.25))
                .child(h4(tk, t("assistant:consent.plan.totals")));
            for total in totals {
                let asset = total["asset"].as_str().unwrap_or("");
                let dir = total["direction"].as_str().unwrap_or("");
                let dkey = format!("assistant:consent.plan.direction.{dir}");
                let mut line = row().flex_wrap().child(
                    text(format!(
                        "{} · {}",
                        if crate::i18n::has(&dkey) {
                            t(&dkey)
                        } else {
                            dir.to_owned()
                        },
                        tc(
                            "assistant:consent.plan.rowCount",
                            total["rows"].as_u64().unwrap_or(0) as usize,
                            &[]
                        )
                    ))
                    .font(tk.txs(400.0).mono())
                    .color(tk.fg),
                );
                if total["amount_msat"].is_i64() || total["amount_msat"].is_u64() {
                    line = line
                        .child(text(" · ").font(tk.txs(400.0).mono()).color(tk.fg))
                        .child(crate::ui::sensitive(
                            custody_msat(&total["amount_msat"], asset),
                            tk.txs(400.0).mono(),
                            tk.fg,
                            hide,
                        ));
                }
                if total["fee_msat"].as_i64().unwrap_or(0) != 0 {
                    line = line
                        .child(
                            text(format!(" · {} ", t("assistant:consent.plan.fee")))
                                .font(tk.txs(400.0).mono())
                                .color(tk.fg),
                        )
                        .child(crate::ui::sensitive(
                            custody_msat(&total["fee_msat"], asset),
                            tk.txs(400.0).mono(),
                            tk.fg,
                            hide,
                        ));
                }
                s = s.child(line);
            }
            c = c.child(s);
        }
        if let Some(warnings) = plan["warnings"].as_array().filter(|a| !a.is_empty()) {
            let mut s = col()
                .w_full()
                .gap(tk.r(0.25))
                .child(h4(tk, t("assistant:consent.plan.warnings")));
            for w in warnings.iter().filter_map(Value::as_str) {
                let key = format!("assistant:consent.plan.warning.{w}");
                s = s.child(
                    row()
                        .w_full()
                        .gap(tk.r(0.375))
                        .pl(tk.r(0.25))
                        .child(text("•").font(tk.txs(400.0)).color(tk.fg))
                        .child(
                            text(if crate::i18n::has(&key) {
                                t(&key)
                            } else {
                                w.to_owned()
                            })
                            .font(tk.txs(400.0))
                            .color(tk.fg)
                            .wrap()
                            .grow(1.0)
                            .min_w(0.0),
                        ),
                );
            }
            c = c.child(s);
        }
    }
    if let Some(b) = blockers(tk, &plan["blockers"]) {
        c = c.child(b);
    }
    c
}

/// `ReviewEffectsView`: quarantine and readiness, before → after.
fn effects_view(tk: &Tk, before: Option<&Value>, after: &Value) -> El {
    let ready = |v: &Value| {
        t(if v["report_ready"].as_bool() == Some(true) {
            "assistant:review.effects.ready"
        } else {
            "assistant:review.effects.blocked"
        })
    };
    let rows = [
        (
            t("assistant:review.effects.quarantine"),
            before.map(|b| text_of(&b["quarantine_count"])),
            text_of(&after["quarantine_count"]),
        ),
        (
            t("assistant:review.effects.readiness"),
            before.map(ready),
            ready(after),
        ),
    ];
    let mut dl = col().w_full().gap(tk.r(0.5));
    for (label, b, a) in rows {
        let mut v = row().gap(tk.r(0.5)).center_y();
        if let Some(b) = b {
            v = v.child(value_text(tk, b)).child(
                ic(&I::ARROW_RIGHT, tk.r(0.875), tk.muted_fg)
                    .label(t("assistant:review.effects.becomes")),
            );
        }
        dl = dl.child(fact(tk, &label, v.child(value_text(tk, a))));
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .role("group")
        .label(t("assistant:review.computed"))
        .child(
            row()
                .gap(tk.r(0.375))
                .center_y()
                .child(ic(&I::CIRCLE_CHECK, tk.r(0.875), tk.muted_fg).a11y_hidden())
                .child(h4(tk, t("assistant:review.computed"))),
        )
        .child(dl)
}

fn operation_view(tk: &Tk, op: &Value, hide: bool) -> El {
    let ty = op["type"].as_str().unwrap_or("custody_component");
    let key = format!("assistant:review.op.{ty}");
    let title = if OPERATION_TYPES.contains(&ty) {
        t(&key)
    } else {
        t("assistant:review.op.custody_component")
    };
    let request = &op["request"];
    let mono = |s: String| {
        text(s)
            .font(tk.txs(400.0).mono())
            .color(tk.fg)
            .wrap_anywhere()
    };
    let mut li = col()
        .w_full()
        .gap(tk.r(0.25))
        .py(tk.r(0.5))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .child(text(title).font(tk.tsm(500.0)).color(tk.fg));
    if op["transaction_id"].is_string() {
        li = li.child(mono(text_of(&op["transaction_id"])));
    }
    if op["out_transaction_id"].is_string() {
        li = li.child(mono(format!(
            "{} → {}",
            text_of(&op["out_transaction_id"]),
            text_of(&op["in_transaction_id"])
        )));
    }
    if op["gap_id"].is_string() {
        li = li.child(mono(text_of(&op["gap_id"])));
    }
    let detail: Vec<String> = match ty {
        "custody_gap" => vec![text_of(&op["action"]), text_of(&op["classification"])],
        "kind_override" => vec![
            op["kind"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| t("assistant:review.op.kindCleared")),
        ],
        "accept_coarse_prices" => vec![match op["transaction_ids"].as_array() {
            Some(ids) => tc("assistant:review.op.transactions", ids.len(), &[]),
            None => t("assistant:review.op.allCoarse"),
        }],
        _ => vec![
            text_of(&op["pair_kind"]),
            text_of(&op["policy"]),
            op["expires_in_days"]
                .as_u64()
                .map(|n| tc("assistant:review.op.expiresDays", n as usize, &[]))
                .unwrap_or_default(),
        ],
    };
    let detail: Vec<String> = detail.into_iter().filter(|s| !s.is_empty()).collect();
    if !detail.is_empty() {
        li = li.child(
            text(detail.join(" · "))
                .font(tk.txs(400.0))
                .color(tk.fg)
                .wrap(),
        );
    }
    if op["pair_id"].is_string() {
        li = li.child(mono(text_of(&op["pair_id"])));
    }
    for (field, label) in [
        ("fiat_rate", "assistant:review.op.rate"),
        ("fiat_value", "assistant:review.op.value"),
    ] {
        if op.get(field).is_some() {
            li = li.child(
                row()
                    .flex_wrap()
                    .child(
                        text(format!("{}: ", t(label)))
                            .font(tk.txs(400.0))
                            .color(tk.fg),
                    )
                    .child(crate::ui::sensitive(
                        text_of(&op[field]),
                        tk.txs(400.0).mono(),
                        tk.fg,
                        hide,
                    )),
            );
        }
    }
    let reason = op["reason"]
        .as_str()
        .or_else(|| request["reason"].as_str())
        .unwrap_or("");
    if !reason.is_empty() {
        li = li.child(
            text(format!(
                "{}: {reason}",
                t("assistant:review.proposedInterpretation")
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        );
    }
    li
}

/// A `<details>`: a summary row that toggles its content.
fn details(app: &App, tk: &Tk, key: String, summary: String, body: impl FnOnce() -> El) -> El {
    let open = is_open(app, &key, false);
    let k = key.clone();
    let mut c = col().key(key.clone()).w_full().child(
        row()
            .key(format!("{key}-summary"))
            .gap(tk.r(0.25))
            .center_y()
            .py(tk.r(0.5))
            .role("button")
            .expanded(open)
            .label(summary.clone())
            .focusable(true)
            .on_press(move |a: &mut App| toggle(a, &k, false))
            .child(
                ic(
                    if open {
                        &I::CHEVRON_DOWN
                    } else {
                        &I::CHEVRON_RIGHT
                    },
                    tk.r(0.75),
                    tk.muted_fg,
                )
                .a11y_hidden(),
            )
            .child(text(summary).font(tk.txs(400.0)).color(tk.muted_fg)),
    );
    if open {
        c = c.child(body());
    }
    c
}

fn raw_json(app: &App, tk: &Tk, key: String, v: &Value) -> El {
    let body = serde_json::to_string_pretty(v).unwrap_or_default();
    details(app, tk, key, t("assistant:review.rawJson"), || {
        super::nested_scroll(col().w_full().max_h(tk.r(16.0)))
            .radius(tk.r(0.375))
            .fill(tk.muted)
            .padding(tk.r(0.75))
            .child(
                text(body)
                    .font(tk.txs(400.0).mono())
                    .color(tk.muted_fg)
                    .wrap_anywhere(),
            )
    })
}

fn remaining(app: &App, tk: &Tk, key: String, effects: &Value) -> Option<El> {
    let count = effects["quarantine_count"].as_u64().unwrap_or(0) as usize;
    if count == 0 {
        return None;
    }
    let listed = effects["quarantines"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let body = move || {
        let mut ul = col().w_full().gap(tk.r(0.25));
        for q in listed.iter().take(100) {
            ul = ul.child(
                text(format!(
                    "{} · {}",
                    text_of(&q["transaction_id"]),
                    text_of(&q["reason"])
                ))
                .font(tk.txs(400.0))
                .color(tk.fg)
                .wrap(),
            );
        }
        if count > listed.len() {
            ul = ul.child(
                text(tc("assistant:review.notListed", count - listed.len(), &[]))
                    .font(tk.txs(400.0))
                    .color(tk.fg)
                    .mt(tk.r(0.5)),
            );
        }
        ul
    };
    Some(
        amber_box(tk)
            .label(t("assistant:review.openQuestions"))
            .child(
                row()
                    .gap(tk.r(0.375))
                    .center_y()
                    .child(ic(&I::CIRCLE_HELP, tk.r(0.875), tk.fg).a11y_hidden())
                    .child(
                        text(tc("assistant:review.remaining", count, &[]))
                            .font(tk.txs(500.0))
                            .color(tk.fg),
                    ),
            )
            .child(details(
                app,
                tk,
                key,
                t("assistant:review.openQuestions"),
                body,
            )),
    )
}

/// `ReviewTaxDeltaView`: the per-year tax change the server computed for the plan.
fn tax_delta(app: &App, tk: &Tk, artifact: &Value) -> Option<El> {
    let tax = if artifact["after"].is_object() {
        &artifact["after"]["tax"]
    } else {
        &artifact["verification"]["tax"]
    };
    let years = artifact["delta"]["years"].as_array()?;
    let cur = tax["fiat_currency"].as_str()?;
    tax["years"].as_array()?;
    let hide = app.prefs.hide_sensitive();
    let money = |v: f64| {
        if hide {
            crate::theme::MASK.to_owned()
        } else {
            format!(
                "{}{}",
                if v > 0.0 { "+" } else { "" },
                crate::format::fiat(v, cur)
            )
        }
    };
    let mut s = col()
        .w_full()
        .gap(tk.r(0.375))
        .label(t("assistant:review.tax.title"))
        .child(h4(tk, t("assistant:review.tax.title")));
    if years.is_empty() {
        return Some(
            s.child(
                text(t("assistant:review.tax.unchanged"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    for y in years {
        let num = |v: &Value| {
            v.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| v.as_f64())
                .unwrap_or(0.0)
        };
        let mut parts = vec![
            format!(
                "{} {}",
                t("assistant:review.tax.realized"),
                money(num(&y["realized_gain_loss"]))
            ),
            format!(
                "{} {}",
                t("assistant:review.tax.income"),
                money(num(&y["income"]))
            ),
        ];
        if tax["jurisdiction"].as_str() == Some("AT") {
            parts.push(match y["estimated_tax_eur_cents"].as_f64() {
                Some(c) => format!(
                    "{} {}",
                    t("assistant:review.tax.indicative"),
                    money(c / 100.0)
                ),
                None => t("assistant:review.tax.withheld"),
            });
        }
        s = s.child(
            row()
                .w_full()
                .flex_wrap()
                .gap(tk.r(0.75))
                .child(
                    text(text_of(&y["year"]))
                        .font(tk.txs(500.0).features("'tnum' 1"))
                        .color(tk.fg),
                )
                .child(
                    text(parts.join(" · "))
                        .font(tk.txs(400.0).features("'tnum' 1"))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    Some(s)
}

/// `ReviewProposalView`: revision, effects + tax change beside the corrections, remaining issues,
/// fingerprint and the raw plan.
pub fn proposal_view(app: &App, tk: &Tk, key: &str, a: &Value, wide: bool) -> El {
    let hide = app.prefs.hide_sensitive();
    let ops = a["operations"].as_array().cloned().unwrap_or_default();
    let mut left =
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(effects_view(tk, Some(&a["before"]), &a["after"]));
    if let Some(d) = tax_delta(app, tk, a) {
        left = left.child(d);
    }
    let mut list = col()
        .w_full()
        .label(tc("assistant:review.operations", ops.len(), &[]))
        .child(h4(tk, tc("assistant:review.operations", ops.len(), &[])));
    for op in ops.iter().take(6) {
        list = list.child(operation_view(tk, op, hide));
    }
    if ops.len() > 6 {
        let rest: Vec<Value> = ops[6..].to_vec();
        list = list.child(details(
            app,
            tk,
            format!("{key}-more"),
            tc("assistant:review.more", ops.len() - 6, &[]),
            move || {
                let mut c = col().w_full();
                for op in &rest {
                    c = c.child(operation_view(tk, op, hide));
                }
                c
            },
        ));
    }
    let grid = if wide {
        row()
            .w_full()
            .gap(tk.r(1.25))
            .child(left.grow(1.0).w(0.0).min_w(0.0))
            .child(list.grow(1.0).w(0.0).min_w(0.0))
    } else {
        col().w_full().gap(tk.r(1.25)).child(left).child(list)
    };
    let mut c = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(
            text(tr(
                "assistant:review.proposal.revision",
                &[("revision", &text_of(&a["base_input_version"]))],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        )
        .child(grid);
    if let Some(r) = remaining(app, tk, format!("{key}-remaining"), &a["after"]) {
        c = c.child(r);
    }
    c.child(
        text(format!(
            "{}: {}",
            t("assistant:review.fingerprint"),
            text_of(&a["digest"])
        ))
        .font(tk.t2xs(400.0).mono())
        .color(tk.muted_fg)
        .wrap_anywhere(),
    )
    .child(raw_json(app, tk, format!("{key}-raw"), a))
}

/// `kb-surface-inset my-3 space-y-3 p-4` (a primary/40 border on a proposal).
fn inset_card(tk: &Tk, key: String, primary: bool) -> El {
    col()
        .key(key)
        .w_full()
        .min_w(0.0)
        .my(tk.r(0.75))
        .gap(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.5))
        .fill(tk.card)
        .border(
            1.0,
            if primary {
                tk.primary.alpha(0.4)
            } else {
                tk.border.alpha(tk.border.a * 0.7)
            },
        )
        .border_layout()
}

/// `ReviewProposalCard`.
pub fn proposal_card(app: &App, tk: &Tk, a: &Value, applied: bool, wide: bool) -> El {
    let key = format!(
        "asst-review-plan-{}",
        text_of(&a["digest"]).chars().take(12).collect::<String>()
    );
    inset_card(tk, key.clone(), true)
        .role("region")
        .label(t("assistant:review.proposal.title"))
        .child(
            row()
                .gap(tk.r(0.5))
                .center_y()
                .child(
                    text(t("assistant:review.proposal.title"))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .role("heading"),
                )
                .child(
                    text(t(if applied {
                        "assistant:review.status.applied"
                    } else {
                        "assistant:review.status.proposed"
                    }))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
        )
        .child(proposal_view(app, tk, &key, a, wide))
}

/// `ReviewReceiptCard`.
pub fn receipt_card(app: &App, tk: &Tk, r: &Value) -> El {
    let key = format!("asst-review-receipt-{}", text_of(&r["id"]));
    let mut c = inset_card(tk, key.clone(), false)
        .role("region")
        .label(t("assistant:review.receipt.title"))
        .child(
            text(t("assistant:review.receipt.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(
            text(tc(
                "assistant:review.receipt.verified",
                r["operations"].as_array().map_or(0, Vec::len),
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        )
        .child(
            text(format!(
                "{} · {}",
                text_of(&r["created_at"]),
                text_of(&r["id"])
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        )
        .child(effects_view(
            tk,
            r.get("before").filter(|b| b.is_object()),
            &r["verification"],
        ));
    if let Some(d) = tax_delta(app, tk, r) {
        c = c.child(d);
    }
    if let Some(rem) = remaining(app, tk, format!("{key}-remaining"), &r["verification"]) {
        c = c.child(rem);
    }
    c.child(
        crate::txdetail::widgets::button(
            tk,
            format!("{key}-open"),
            crate::txdetail::widgets::Variant::Ghost,
            crate::txdetail::widgets::Size::Sm,
            Some(&I::ARROW_UP_RIGHT),
            &t("assistant:review.receipt.openQuarantine"),
            true,
        )
        .on_press(|a: &mut App| a.navigate("/review")),
    )
    .child(raw_json(app, tk, format!("{key}-raw"), r))
}

/// An inline action of the approval row (`font-medium underline-offset-2`, underlined on hover).
fn inline_action(tk: &Tk, key: String, label: String, ink: Color, enabled: bool) -> El {
    let b = row()
        .key(key)
        .center_y()
        .gap(2.0)
        .radius(tk.r(0.125))
        .role("button")
        .label(label.clone())
        .child(text(label).font(tk.txs(500.0)).color(ink));
    if enabled {
        b.focusable(true)
    } else {
        b.disabled(true).opacity(0.5)
    }
}

/// The row's More menu (`side="top" align="start" min-w-44`).
fn more_menu(tk: &Tk, r: &Consent, can_session: bool, layer: &mut Vec<El>) {
    let key = format!("asst-approval-{}", r.call_id);
    let mut p = crate::ui::popup::Popup::menu(format!("{key}-more"), |s: &mut App| {
        s.assistant.thread.consent_menu = None
    })
    .list_key(format!("{key}-menu"));
    if can_session {
        let id = r.call_id.clone();
        let l = t("assistant:consent.allowChat");
        p.item(
            crate::shell::menu_row(tk, format!("{key}-session"), None, &l, None).on_press(
                move |s: &mut App| {
                    s.assistant.thread.consent_menu = None;
                    crate::assistant::chat::send_consent(s, &id, "allow_session");
                },
            ),
            format!("{key}-session"),
            l,
            true,
            false,
        );
    }
    let l = t("assistant:consent.stopAnswer");
    let stop = crate::assistant::page::destructive_item(tk, &format!("{key}-stop"), &l).on_press(
        |s: &mut App| {
            s.assistant.thread.consent_menu = None;
            crate::assistant::chat::abort(s);
        },
    );
    p.item(stop, format!("{key}-stop"), l, true, false);
    p.mount(
        tk,
        layer,
        col()
            .min_w(tk.r(11.0))
            .padding(tk.r(0.25))
            .label(t("assistant:consent.moreOptions")),
        |l| crate::shell::glass(tk, l),
        rune_kit::Placement::TopStart,
        4.0,
    );
}

/// `PendingApprovalRow`: what will run, the plan or arguments, the risks, then "Waiting for
/// approval · Decline · Approve · More".
pub fn pending_row(app: &App, tk: &Tk, r: &Consent, layer: &mut Vec<El>) -> El {
    let key = format!("asst-approval-{}", r.call_id);
    let kind = if r.name == "ui.review.apply" {
        "review"
    } else if PLAN_BOUND.contains(&r.name.as_str()) {
        if r.name == "ui.imports.rollback" {
            "rollback"
        } else {
            "import"
        }
    } else {
        "tool"
    };
    let glyph: &'static rune::Icon = match kind {
        "tool" if r.egresses => &I::GLOBE,
        "review" => &I::LIST_CHECKS,
        "import" => &I::FILE_UP,
        "rollback" => &I::ROTATE_CCW,
        _ => &I::SQUARE_PEN,
    };
    let applied = r
        .review_preview
        .as_ref()
        .is_some_and(|p| p["status"].as_str() == Some("applied"));
    let approve = t(match kind {
        "review" if applied => "assistant:consent.apply.returnReceipt",
        "review" => "assistant:consent.apply.confirm",
        "import" => "assistant:consent.plan.confirmImport",
        "rollback" => "assistant:consent.plan.confirmRollback",
        _ => "assistant:consent.approve",
    });
    let reason = match kind {
        "review" => Some(t("assistant:consent.apply.description")),
        "tool" => None,
        _ => Some(t("assistant:consent.plan.description")),
    };
    let mut risks = vec![];
    if r.egresses {
        risks.push(t("assistant:consent.risk.network"));
    }
    if !r.reversible {
        risks.push(t("assistant:consent.risk.permanent"));
    }
    let head = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.375))
        .child(
            row()
                .size(tk.r(1.5), tk.r(1.5))
                .center()
                .child(ic(glyph, tk.r(1.0), tk.muted_fg).a11y_hidden()),
        )
        .child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .pt(2.0)
                .child(
                    text(r.summary.clone())
                        .font(tk.tsm(500.0).lh(tk.r(0.875) * 1.625))
                        .color(tk.fg)
                        .wrap(),
                )
                .child(
                    text(r.name.clone())
                        .font(tk.t2xs(400.0).mono())
                        .color(tk.muted_fg)
                        .truncate(),
                ),
        );
    let mut body = col()
        .w_full()
        .min_w(0.0)
        .pl(tk.r(1.875))
        .pb(tk.r(0.25))
        .gap(tk.r(0.375));
    if let Some(reason) = reason {
        body = body.child(text(reason).font(tk.txs(400.0)).color(tk.muted_fg).wrap());
    }
    let has_body = kind != "tool";
    if has_body {
        let bkey = format!("{key}-body");
        let open = is_open(app, &bkey, true);
        let label = t(if kind == "review" {
            "assistant:consent.proposalDetails"
        } else {
            "assistant:consent.planDetails"
        });
        let bk = bkey.clone();
        let mut coll = col().w_full().min_w(0.0).child(
            row()
                .key(format!("{bkey}-trigger"))
                .gap(tk.r(0.25))
                .center_y()
                .radius(tk.r(0.25))
                .role("button")
                .expanded(open)
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| toggle(a, &bk, true))
                .child(
                    ic(
                        if open {
                            &I::CHEVRON_DOWN
                        } else {
                            &I::CHEVRON_RIGHT
                        },
                        tk.r(0.75),
                        tk.muted_fg,
                    )
                    .a11y_hidden(),
                )
                .child(text(label.clone()).font(tk.txs(400.0)).color(tk.muted_fg)),
        );
        if open {
            let inner = if kind == "review" {
                match r.review_preview.as_ref() {
                    Some(p)
                        if p["status"].as_str() == Some("ready")
                            && review_artifact(&p["artifact"]).is_some() =>
                    {
                        proposal_view(app, tk, &bkey, &p["artifact"], false)
                    }
                    Some(p)
                        if p["status"].as_str() == Some("applied")
                            && review_receipt(&p["receipt"]).is_some() =>
                    {
                        col()
                            .w_full()
                            .gap(tk.r(0.5))
                            .child(
                                text(t("assistant:consent.apply.alreadyApplied"))
                                    .font(tk.tsm(400.0))
                                    .color(tk.fg)
                                    .wrap(),
                            )
                            .child(receipt_card(app, tk, &p["receipt"]))
                    }
                    p => {
                        let mut b = amber_box(tk).role("alert").child(
                            text(t("assistant:consent.apply.unavailable"))
                                .font(tk.tsm(400.0))
                                .color(tk.fg)
                                .wrap(),
                        );
                        if let Some(code) = p.and_then(|p| p["code"].as_str()) {
                            b = b.child(
                                text(code.to_owned())
                                    .font(tk.txs(400.0).mono())
                                    .color(tk.fg)
                                    .mt(tk.r(0.5)),
                            );
                        }
                        b
                    }
                }
            } else {
                plan_body(app, tk, &r.name, &r.arguments_preview)
            };
            coll = coll.child(
                super::nested_scroll(
                    col()
                        .w_full()
                        .mt(tk.r(0.375))
                        .max_h(tk.r(26.0).min(app.shell.window_h * 0.55).max(tk.r(10.0))),
                )
                .radius(tk.r(0.375))
                .border(1.0, tk.border.alpha(tk.border.a * 0.6))
                .border_layout()
                .fill(tk.background.alpha(0.6))
                .padding(tk.r(0.75))
                .label(label)
                .focusable(true)
                .child(inner),
            );
        }
        body = body.child(coll);
    } else if r
        .arguments_preview
        .as_object()
        .is_some_and(|o| !o.is_empty())
    {
        body = body.child(
            super::nested_scroll(col().w_full().max_h(tk.r(10.0)))
                .radius(tk.r(0.375))
                .fill(tk.muted.alpha(0.55))
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .label(t("assistant:consent.arguments"))
                .focusable(true)
                .child(
                    text(serde_json::to_string_pretty(&r.arguments_preview).unwrap_or_default())
                        .font(tk.t2xs(400.0).mono().lh(tk.r(1.25)))
                        .color(tk.fg)
                        .wrap_anywhere(),
                ),
        );
    }
    if !risks.is_empty() {
        let mut line = row()
            .flex_wrap()
            .gap(tk.r(0.375))
            .center_y()
            .child(ic(&I::TRIANGLE_ALERT, tk.r(0.75), amber(tk)).a11y_hidden());
        for (i, risk) in risks.iter().enumerate() {
            if i > 0 {
                line = line.child(
                    text("·")
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .a11y_hidden(),
                );
            }
            line = line.child(text(risk.clone()).font(tk.txs(400.0)).color(amber(tk)));
        }
        body = body.child(line);
    }
    // Plan-bound and review approvals run exactly the plan the daemon showed.
    let can_approve = match kind {
        "review" => r
            .review_preview
            .as_ref()
            .is_some_and(|p| match p["status"].as_str() {
                Some("ready") => review_artifact(&p["artifact"]).is_some(),
                Some("applied") => review_receipt(&p["receipt"]).is_some(),
                _ => false,
            }),
        "tool" => true,
        _ => r.arguments_preview["status"].as_str() == Some("ready"),
    };
    let can_session = can_approve && r.can_allow_session;
    let live = app.prefs.ai_features && app.assistant.streaming;
    let dot = || {
        text("·")
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .a11y_hidden()
    };
    let (id_d, id_a) = (r.call_id.clone(), r.call_id.clone());
    let mut decline = inline_action(
        tk,
        format!("{key}-decline"),
        t("assistant:consent.decline"),
        tk.muted_fg,
        live,
    );
    if live {
        decline = decline
            .on_press(move |s: &mut App| crate::assistant::chat::send_consent(s, &id_d, "deny"));
    }
    let mut approve_btn = inline_action(
        tk,
        format!("{key}-approve"),
        approve,
        if kind == "rollback" {
            tk.destructive.alpha(0.85)
        } else {
            tk.fg
        },
        live && can_approve,
    );
    if live && can_approve {
        approve_btn = approve_btn.on_press(move |s: &mut App| {
            crate::assistant::chat::send_consent(s, &id_a, "allow_once")
        });
    }
    let menu_open = app.assistant.thread.consent_menu.as_deref() == Some(r.call_id.as_str());
    let id_m = r.call_id.clone();
    let mut more = inline_action(
        tk,
        format!("{key}-more"),
        t("assistant:consent.more"),
        tk.muted_fg,
        live,
    )
    .label(t("assistant:consent.moreOptions"))
    .expanded(menu_open)
    .child(ic(&I::CHEVRON_DOWN, tk.r(0.75), tk.muted_fg).a11y_hidden());
    if live {
        more = crate::ui::popup::trigger(
            more,
            crate::ui::popup::Kind::Menu,
            menu_open,
            move |s: &mut App| {
                s.assistant.thread.consent_menu =
                    if s.assistant.thread.consent_menu.as_deref() == Some(id_m.as_str()) {
                        None
                    } else {
                        Some(id_m.clone())
                    };
            },
        );
    }
    if menu_open && live {
        more_menu(tk, r, can_session, layer);
    }
    body = body.child(
        row()
            .flex_wrap()
            .gap(tk.r(0.5))
            .center_y()
            .child(
                row()
                    .gap(tk.r(0.25))
                    .center_y()
                    .child(ic(&I::SHIELD, tk.r(0.75), amber(tk)).a11y_hidden())
                    .child(
                        text(t("assistant:consent.waiting"))
                            .font(tk.txs(400.0))
                            .color(amber(tk)),
                    ),
            )
            .child(dot())
            .child(decline)
            .child(approve_btn)
            .child(more),
    );
    col()
        .key(key)
        .w_full()
        .min_w(0.0)
        .mb(tk.r(0.5))
        .radius(tk.r(0.375))
        .px(2.0)
        .py(2.0)
        .role("group")
        .label(format!(
            "{}: {}",
            t("assistant:consent.rowLabel"),
            r.summary
        ))
        // `tabIndex={-1}`: focus moves here when the row appears (`closed::poll`), Tab skips it.
        .focusable(true)
        .tab_index(-1)
        .focus_ring(2.0, 0.0, tk.ring.alpha(tk.ring.a * 0.7))
        .child(head)
        .child(body)
}

/// `EvidenceRequestCard`: missing evidence as a local hand-off (never an approval).
pub fn evidence_card(app: &App, tk: &Tk, c: &ToolCall) -> Option<El> {
    crate::assistant::evidence::card(app, tk, c)
}

const WALLET_SOURCES: [&str; 11] = [
    "descriptor",
    "address-list",
    "silent-payment",
    "liquid-descriptor",
    "samourai",
    "core-ln",
    "lnd",
    "btcpay",
    "kraken",
    "coinbase",
    "binance",
];

/// `WalletInputRequestCard`: a connection the assistant may not set up itself; the button opens
/// the add-connection dialog on that source.
pub fn wallet_request_card(app: &App, tk: &Tk, c: &ToolCall) -> Option<El> {
    if c.name != "ui.wallets.request_input" || c.status != ToolStatus::Done {
        return None;
    }
    let env = c.result.as_ref()?;
    let d = &env["data"];
    let source = d["source"].as_str()?;
    let ok = env["kind"].as_str() == Some("ui.wallets.request_input")
        && d["schema_version"].as_u64() == Some(1)
        && d["action"].as_str() == Some("connect_wallet")
        && d["desktop_only"].as_bool() == Some(true)
        && d["request_id"].as_str().is_some_and(|s| s.len() == 64)
        && d["workspace_id"].as_str().is_some_and(|s| !s.is_empty())
        && d["profile_id"].as_str().is_some_and(|s| !s.is_empty())
        && WALLET_SOURCES.contains(&source)
        && (d["explanation"].is_null()
            || d["explanation"]
                .as_str()
                .is_some_and(|s| s.chars().count() <= 500));
    if !ok {
        return None;
    }
    let scope = app.expected_scope();
    let same_book = scope["workspace_id"].as_str() == d["workspace_id"].as_str()
        && scope["profile_id"].as_str() == d["profile_id"].as_str();
    let key = format!("asst-walletreq-{}", c.call_id);
    let explanation = d["explanation"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| t("assistant:walletRequest.why"));
    let state = crate::assistant::evidence::wallet_card_state(app, &c.call_id);
    let enabled = same_book && state == "idle";
    let mut act = crate::txdetail::widgets::button(
        tk,
        format!("{key}-act"),
        crate::txdetail::widgets::Variant::Outline,
        crate::txdetail::widgets::Size::Sm,
        None,
        &t("assistant:walletRequest.action"),
        enabled,
    );
    if enabled {
        let (call, src) = (c.call_id.clone(), source.to_owned());
        act = act.on_press(move |a: &mut App| {
            crate::assistant::evidence::open_wallet_request(a, &call, &src)
        });
    }
    Some(
        col()
            .key(key.clone())
            .w_full()
            .my(tk.r(0.75))
            .pl(tk.r(1.0))
            .border_sides([0.0, 0.0, 0.0, 2.0], tk.border)
            .role("region")
            .label(t("assistant:walletRequest.label"))
            .child(
                text(t(&format!("assistant:walletRequest.source.{source}")))
                    .font(tk.tsm(500.0))
                    .color(tk.fg),
            )
            .child(
                text(explanation)
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .mt(tk.r(0.25)),
            )
            .child(row().mt(tk.r(0.75)).child(act))
            .child(
                text(if same_book {
                    t(&format!("assistant:walletRequest.status.{state}"))
                } else {
                    t("assistant:walletRequest.otherBook")
                })
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .mt(tk.r(0.5))
                .role("status"),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn custody_amounts_are_exact() {
        assert_eq!(
            custody_msat(&json!(123_456_789_000_i64), "BTC"),
            "1.23456789 BTC"
        );
        assert_eq!(custody_msat(&json!("-1"), "BTC"), "−0.00000000001 BTC");
        assert_eq!(custody_msat(&json!(200_000_000_000_i64), "BTC"), "2 BTC");
        assert_eq!(custody_msat(&json!("x"), "BTC"), "— BTC");
    }

    #[test]
    fn review_packets_validate() {
        let fx = |n: u64| json!({"entries_count": 10, "quarantine_count": n, "report_ready": n == 0, "quarantines": []});
        let a = json!({"schema_version": 1, "workspace_id": "w", "profile_id": "p", "base_input_version": 4,
            "digest": "a".repeat(64), "operations": [{"type": "exclude"}], "before": fx(2), "after": fx(0)});
        assert!(review_artifact(&a).is_some());
        let mut bad = a.clone();
        bad["operations"] = json!([{"type": "delete_everything"}]);
        assert!(review_artifact(&bad).is_none());
        let r = json!({"schema_version": 1, "status": "verified", "id": "r1", "artifact_digest": "a", "created_at": "2026-01-01",
            "result_input_version": 5, "operations": [], "verification": fx(0)});
        assert!(review_receipt(&r).is_some());
    }
}
