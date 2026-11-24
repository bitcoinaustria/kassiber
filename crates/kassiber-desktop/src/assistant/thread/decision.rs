//! The two review decisions as the app's own cards (`super::action`): resolving a held
//! transaction (`ui.journals.quarantine.resolve`) and applying a custody-gap review
//! (`ui.custody.review.apply`).
//!
//! A waiting card names what the decision touches, the decision itself and what it means for the
//! books, in the interface's words. The held transaction is read as `super::action` reads any
//! transaction; the gap through `ui.custody.gaps.review_context`, the custody page's own local
//! read of one gap. A custody apply runs exactly one plan (`ui.custody.review.plan`, a read-only
//! tool call earlier in the conversation): when the conversation holds that plan, for the same
//! gap, action, classification and input version, the card shows its exact amounts. Nothing here
//! authorizes anything; the daemon re-checks the plan when the approved call runs, and fails
//! closed when the book moved on.
//!
//! An applied decision adds what its result reports: whether the transaction cleared and what
//! waited on it, or the gap's new status. Resolving a held transaction is undone by
//! `ui.transactions.history.revert`, which the card's Undo sends (`super::action`); a custody
//! decision's inverse is a reopen the assistant plans first, so that card has no Undo button.

use rune::{El, col, text};
use serde_json::{Map, Value};

use super::action::{Action, fact, id_of, leg_view, opt_str, value_text};
use crate::app::App;
use crate::assistant::model::{ToolCall, ToolStatus};
use crate::custody::gaps::model::{Gap, RESIDUAL_CLASSIFICATIONS, fmt_date, fmt_msat};
use crate::i18n::{t, tc, tr};
use crate::theme::Tk;

/// The custody-gap actions `ui.custody.review.apply` takes.
const GAP_ACTIONS: [&str; 5] = ["create", "dismiss", "revise", "reopen", "classify_residual"];

/// The reviewed price a resolution sets, exactly as the call writes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Price {
    /// Per unit of the asset (`fiat_rate`).
    Rate(String),
    /// For the whole transaction (`fiat_value`).
    Total(String),
}

/// `ui.journals.quarantine.resolve`.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolve {
    /// The held transaction: the one named, or the root of the named group.
    pub transaction: String,
    /// Named through its quarantine group (`group_id`): other rows wait on it.
    pub root: bool,
    /// `None`: exclude it.
    pub price: Option<Price>,
    pub reason: String,
    pub reprocess: bool,
}

impl Resolve {
    pub fn title_key(&self) -> &'static str {
        if self.price.is_some() {
            "assistant:action.resolve.title.price"
        } else {
            "assistant:action.resolve.title.exclude"
        }
    }
}

/// `ui.custody.review.apply`.
#[derive(Clone, Debug, PartialEq)]
pub struct Custody {
    pub gap: String,
    pub action: &'static str,
    /// The residual's classification (`classify_residual` only).
    pub classification: Option<&'static str>,
    pub reason: Option<String>,
    /// The input version the plan was computed at.
    pub version: u64,
}

impl Custody {
    pub fn title_key(&self) -> &'static str {
        match self.action {
            "create" => "assistant:action.custody.title.create",
            "dismiss" => "assistant:action.custody.title.dismiss",
            "revise" => "assistant:action.custody.title.revise",
            "reopen" => "assistant:action.custody.title.reopen",
            _ => "assistant:action.custody.title.classify_residual",
        }
    }
}

fn only(o: &Map<String, Value>, keys: &[&str]) -> bool {
    o.keys().all(|k| keys.contains(&k.as_str()))
}

/// A positive plain decimal (`58210.40`), as a string or a JSON number, kept as written.
fn decimal(v: &Value) -> Option<String> {
    let s = match v {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        _ => return None,
    };
    let (whole, frac) = s.split_once('.').unwrap_or((&s, ""));
    let ok = !whole.is_empty()
        && whole.chars().all(|c| c.is_ascii_digit())
        && (!s.contains('.') || (!frac.is_empty() && frac.chars().all(|c| c.is_ascii_digit())))
        && s.chars().any(|c| matches!(c, '1'..='9'));
    ok.then_some(s)
}

/// The resolution as the core accepts it: exactly one target, a price only with
/// `price_override` (exactly one of rate and value), a reason, and `reprocess` a boolean.
pub fn parse_resolve(o: &Map<String, Value>) -> Option<Resolve> {
    if !only(
        o,
        &[
            "transaction",
            "group_id",
            "action",
            "fiat_rate",
            "fiat_value",
            "reason",
            "reprocess",
        ],
    ) {
        return None;
    }
    let present = |k: &str| o.get(k).filter(|v| !v.is_null());
    let (transaction, root) = match (present("transaction"), present("group_id")) {
        (Some(tx), None) => (id_of(tx)?, false),
        // Only a `root:` group resolves, and it acts on its root (`root_transaction_for_group`).
        (None, Some(g)) => (
            id_of(&Value::String(
                g.as_str()?.trim().strip_prefix("root:")?.to_owned(),
            ))?,
            true,
        ),
        _ => return None,
    };
    let (rate, total) = (present("fiat_rate"), present("fiat_value"));
    let price = match o.get("action")?.as_str()? {
        "exclude" if rate.is_none() && total.is_none() => None,
        "price_override" => match (rate, total) {
            (Some(r), None) => Some(Price::Rate(decimal(r)?)),
            (None, Some(v)) => Some(Price::Total(decimal(v)?)),
            _ => return None,
        },
        _ => return None,
    };
    let reprocess = match o.get("reprocess") {
        None => true,
        Some(v) => v.as_bool()?,
    };
    Some(Resolve {
        transaction,
        root,
        price,
        reason: id_of(o.get("reason")?)?,
        reprocess,
    })
}

/// The custody review as its schema documents it: a classification with `classify_residual`
/// only (and always there), and the plan's input version.
pub fn parse_custody(o: &Map<String, Value>) -> Option<Custody> {
    if !only(
        o,
        &[
            "action",
            "gap_id",
            "classification",
            "reason",
            "expected_input_version",
        ],
    ) {
        return None;
    }
    let action = o.get("action")?.as_str()?;
    let action = *GAP_ACTIONS.iter().find(|a| **a == action)?;
    let classification = match o.get("classification") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            *RESIDUAL_CLASSIFICATIONS
                .iter()
                .find(|c| Some(**c) == v.as_str())?,
        ),
    };
    if classification.is_some() != (action == "classify_residual") {
        return None;
    }
    Some(Custody {
        gap: id_of(o.get("gap_id")?)?,
        action,
        classification,
        reason: opt_str(o.get("reason").unwrap_or(&Value::Null))?.filter(|r| !r.trim().is_empty()),
        version: o.get("expected_input_version")?.as_u64()?,
    })
}

/// `custody(classification)` in `kassiber_transfers::gap_review`: where the residual ends up.
fn custody_state(classification: &str) -> &'static str {
    if classification.starts_with("external_") {
        "external_confirmed"
    } else if classification == "retained_custody" {
        "internal_reviewed"
    } else {
        "custody_suspense"
    }
}

fn int(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

/// The plan this apply runs, when the conversation holds it: the latest finished
/// `ui.custody.review.plan` for the same gap, action, classification and input version.
pub fn custody_plan<'a>(app: &'a App, d: &Custody) -> Option<&'a Value> {
    app.assistant
        .messages
        .iter()
        .rev()
        .flat_map(|m| m.tool_calls.iter().rev())
        .filter(|c| c.name == "ui.custody.review.plan" && c.status == ToolStatus::Done)
        .filter_map(|c| c.result.as_ref().map(|r| &r["data"]))
        .find(|p| {
            p["gap_id"].as_str().map(str::trim) == Some(d.gap.as_str())
                && p["action"].as_str() == Some(d.action)
                && int(&p["input_version"]) == Some(d.version)
                && (d.classification.is_none() || p["classification"].as_str() == d.classification)
        })
}

fn para(tk: &Tk, s: String) -> El {
    text(s).font(tk.txs(400.0)).color(tk.fg).wrap()
}

fn alert(tk: &Tk, s: String) -> El {
    para(tk, s).role("alert")
}

fn amount(app: &App, tk: &Tk, s: String) -> El {
    crate::ui::sensitive(
        s,
        tk.tsm(500.0).features("'tnum' 1"),
        tk.fg,
        app.prefs.hide_sensitive(),
    )
}

/// Why the transaction is held, in the review page's words, when the reason is one it knows.
fn held_because(app: &App, id: &str) -> Option<String> {
    use crate::review::model as qm;
    let tx = app.assistant.thread.actions.legs.get(id)?.tx.as_ref()?;
    let low = tx["quarantineReason"].as_str()?.to_lowercase();
    qm::cause_key(&low, &Value::Null, &Value::Null)?;
    let category = qm::category_for_reason(&low, &Value::Null, &Value::Null);
    let copy = qm::cause_copy_with_root(
        &low,
        category,
        &Value::Null,
        tx["account"].as_str(),
        tx["asset"].as_str(),
        None,
    );
    Some(copy.title).filter(|s| !s.is_empty())
}

/// The held transaction, the decision and what it means for the books.
pub fn resolve_body(app: &App, tk: &Tk, mut c: El, r: &Resolve, pending: bool) -> El {
    c = c.child(leg_view(
        app,
        tk,
        t(if r.root {
            "assistant:action.resolve.root"
        } else {
            "assistant:action.resolve.transaction"
        }),
        &r.transaction,
        None,
    ));
    if pending && let Some(why) = held_because(app, &r.transaction) {
        c = c.child(fact(
            tk,
            t("assistant:action.resolve.heldBecause"),
            value_text(tk, why),
        ));
    }
    let currency = crate::format::book_currency(app);
    let asset = app
        .assistant
        .thread
        .actions
        .legs
        .get(&r.transaction)
        .and_then(|l| l.tx.as_ref())
        .and_then(|tx| tx["asset"].as_str())
        .filter(|a| !a.is_empty())
        .unwrap_or("BTC")
        .to_owned();
    c = c.child(match &r.price {
        None => fact(
            tk,
            t("assistant:action.resolve.decision"),
            value_text(tk, t("assistant:action.resolve.exclude")),
        ),
        Some(Price::Rate(rate)) => fact(
            tk,
            t("assistant:action.resolve.rate"),
            amount(
                app,
                tk,
                tr(
                    "assistant:action.resolve.perUnit",
                    &[("rate", rate), ("currency", &currency), ("asset", &asset)],
                ),
            ),
        ),
        Some(Price::Total(v)) => fact(
            tk,
            t("assistant:action.resolve.total"),
            amount(app, tk, format!("{v} {currency}")),
        ),
    });
    c = c.child(para(
        tk,
        t(if r.price.is_some() {
            "assistant:action.resolve.effect.price"
        } else {
            "assistant:action.resolve.effect.exclude"
        }),
    ));
    if r.root {
        c = c.child(para(tk, t("assistant:action.resolve.effect.root")));
    }
    if !r.reprocess {
        c = c.child(alert(tk, t("assistant:action.resolve.effect.noReprocess")));
    }
    c
}

/// The gap the decision is about: where the bitcoin left and went, when, and how much.
fn gap_view(app: &App, tk: &Tk, id: &str) -> El {
    let label = t("assistant:action.custody.gap");
    let mut c = col()
        .w_full()
        .gap(2.0)
        .role("group")
        .label(label.clone())
        .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg));
    let read = app.assistant.thread.actions.gaps.get(id);
    match read.and_then(|l| l.tx.as_ref()) {
        Some(v) => {
            let g = Gap::parse(v);
            // An unknown destination is what makes it a gap: the route names only the source.
            let route = [
                g.source_wallet_label.clone(),
                g.destination_wallet_labels.join(", "),
            ]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" → ");
            let (start, end) = (
                fmt_date(g.started_at.as_deref()),
                fmt_date(g.ended_at.as_deref()),
            );
            let when = if start == end {
                start
            } else {
                format!("{start} – {end}")
            };
            c = c
                .child(
                    text([route, when].join(" · "))
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .wrap(),
                )
                .child(fact(
                    tk,
                    t("custodyGaps:card.principal"),
                    amount(app, tk, fmt_msat(&g.source_total_msat, &g.asset)),
                ))
                .child(fact(
                    tk,
                    t("custodyGaps:card.residual"),
                    amount(app, tk, fmt_msat(&g.residual_msat, &g.asset)),
                ));
        }
        None => match read {
            Some(l) if l.loading => {
                c = c.child(para(tk, t("assistant:action.custody.loading")).role("status"));
            }
            Some(l) if l.failed => c = c.child(alert(tk, t("assistant:action.custody.missing"))),
            _ => {}
        },
    }
    c.child(
        text(id.to_owned())
            .font(tk.t2xs(400.0).mono())
            .color(tk.muted_fg)
            .wrap_anywhere(),
    )
}

fn gap_asset(app: &App, id: &str) -> String {
    app.assistant
        .thread
        .actions
        .gaps
        .get(id)
        .and_then(|l| l.tx.as_ref())
        .and_then(|g| g["asset"].as_str())
        .filter(|a| !a.is_empty())
        .unwrap_or("BTC")
        .to_owned()
}

/// The gap, the decision and what it means for the books; waiting, the plan's exact amounts.
pub fn custody_body(app: &App, tk: &Tk, mut c: El, d: &Custody, pending: bool) -> El {
    c = c.child(gap_view(app, tk, &d.gap));
    let decision = match d.classification {
        Some(class) => t(&format!("custodyGaps:residual.options.{class}.label")),
        None => t(&format!("assistant:action.custody.decision.{}", d.action)),
    };
    c = c.child(fact(
        tk,
        t("assistant:action.resolve.decision"),
        value_text(tk, decision),
    ));
    let asset = gap_asset(app, &d.gap);
    let plan = custody_plan(app, d).filter(|_| pending);
    if let Some(p) = plan {
        let rows: &[(&str, &str)] = match d.action {
            "create" => &[
                ("retained_msat", "assistant:action.custody.retained"),
                ("residual_msat", "assistant:action.custody.suspense"),
                ("fee_msat", "custodyGaps:card.knownFee"),
            ],
            "revise" => &[
                ("retained_msat", "assistant:action.custody.retained"),
                ("residual_msat", "assistant:action.custody.suspense"),
            ],
            "classify_residual" => &[("residual_msat", "assistant:action.custody.classified")],
            _ => &[],
        };
        for (field, label) in rows {
            if !p[*field].is_null() {
                c = c.child(fact(
                    tk,
                    t(label),
                    amount(app, tk, fmt_msat(&p[*field], &asset)),
                ));
            }
        }
    }
    c = c.child(para(
        tk,
        match d.classification {
            Some(class) => t(&format!(
                "custodyGaps:residual.custodyState.{}",
                custody_state(class)
            )),
            None => t(&format!("assistant:action.custody.effect.{}", d.action)),
        },
    ));
    if d.classification.is_some() {
        c = c.child(fact(
            tk,
            t("custodyGaps:residual.taxMeaningTitle"),
            value_text(tk, t("custodyGaps:residual.taxMeaningUnassigned")),
        ));
    }
    if plan.is_some_and(|p| {
        p["filed_report_impacts"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
    }) {
        c = c.child(alert(tk, t("custodyGaps:filedImpact.title")));
    }
    c
}

/// What an applied decision's result reports, or `None` for the other cards.
pub fn outcome(app: &App, tk: &Tk, a: &Action, call: &ToolCall) -> Option<El> {
    let data = &call.result.as_ref()?["data"];
    let mut c = col().w_full().gap(tk.r(0.375));
    match a {
        Action::Resolve(r) => {
            if data["reprocessed"].as_bool() == Some(false) {
                c = c.child(para(tk, t("assistant:action.resolve.outcome.notMeasured")));
            } else {
                c = match data["cleared"].as_bool()? {
                    true => c.child(
                        para(tk, t("assistant:action.resolve.outcome.cleared")).role("status"),
                    ),
                    false => c.child(alert(tk, t("assistant:action.resolve.outcome.stillHeld"))),
                };
            }
            let count = |v: &Value| v["count"].as_u64().filter(|n| *n > 0);
            if let Some(n) = count(&data["dependents_cleared"]) {
                c = c.child(para(
                    tk,
                    tc("assistant:action.resolve.outcome.released", n as usize, &[]),
                ));
            }
            if let Some(n) = count(&data["quarantine_delta"]["newly_quarantined"]) {
                c = c.child(alert(
                    tk,
                    tc(
                        "assistant:action.resolve.outcome.newlyHeld",
                        n as usize,
                        &[],
                    ),
                ));
            }
            // The value a reviewed rate comes to (a reviewed total is shown as given).
            if let Some(v) = data["resolution"]["fiat_value_exact"]
                .as_str()
                .filter(|v| !v.is_empty())
                .filter(|_| matches!(r.price, Some(Price::Rate(_))))
            {
                c = c.child(fact(
                    tk,
                    t("assistant:action.resolve.outcome.value"),
                    amount(
                        app,
                        tk,
                        format!("{v} {}", crate::format::book_currency(app)),
                    ),
                ));
            }
        }
        Action::Custody(d) => {
            let status = data["status"].as_str()?;
            let key = format!("custodyGaps:status.{status}");
            if crate::i18n::has(&key) {
                c = c.child(fact(
                    tk,
                    t("assistant:action.custody.statusNow"),
                    value_text(tk, t(&key)),
                ));
            }
            let asset = gap_asset(app, &d.gap);
            for (field, label) in [
                ("retained_msat", "assistant:action.custody.retained"),
                (
                    "residual_msat",
                    if d.action == "classify_residual" {
                        "assistant:action.custody.classified"
                    } else {
                        "assistant:action.custody.suspense"
                    },
                ),
            ] {
                if !data[field].is_null() {
                    c = c.child(fact(
                        tk,
                        t(label),
                        amount(app, tk, fmt_msat(&data[field], &asset)),
                    ));
                }
            }
            if data["filed_report_impacts"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                c = c.child(alert(tk, t("custodyGaps:filedImpact.title")));
            }
        }
        _ => return None,
    }
    Some(c)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn resolve(args: Value) -> Option<Resolve> {
        parse_resolve(args.as_object()?)
    }

    fn custody(args: Value) -> Option<Custody> {
        parse_custody(args.as_object()?)
    }

    #[test]
    fn resolutions_parse_as_the_core_accepts_them() {
        let r =
            resolve(json!({"transaction": " tx-1 ", "action": "exclude", "reason": "Not mine"}))
                .expect("an exclusion");
        assert_eq!(
            r,
            Resolve {
                transaction: "tx-1".into(),
                root: false,
                price: None,
                reason: "Not mine".into(),
                reprocess: true,
            }
        );
        let r = resolve(json!({"group_id": "root:tx-9", "action": "price_override", "fiat_rate": "58210.40", "reason": "Kraken invoice", "reprocess": false}))
            .expect("a priced root");
        assert_eq!(r.transaction, "tx-9");
        assert!(r.root && !r.reprocess);
        assert_eq!(r.price, Some(Price::Rate("58210.40".into())));
        let r = resolve(
            json!({"transaction": "t", "action": "price_override", "fiat_value": 120, "reason": "r"}),
        )
        .expect("a total");
        assert_eq!(r.price, Some(Price::Total("120".into())));
        // What the core refuses, or the card could not state exactly: the generic row.
        for bad in [
            json!({"action": "exclude", "reason": "r"}),
            json!({"transaction": "t", "group_id": "root:t", "action": "exclude", "reason": "r"}),
            json!({"group_id": "pair:t", "action": "exclude", "reason": "r"}),
            json!({"group_id": "root: ", "action": "exclude", "reason": "r"}),
            json!({"transaction": "t", "action": "exclude"}),
            json!({"transaction": "t", "action": "exclude", "reason": " "}),
            json!({"transaction": "t", "action": "exclude", "fiat_rate": "1", "reason": "r"}),
            json!({"transaction": "t", "action": "price_override", "reason": "r"}),
            json!({"transaction": "t", "action": "price_override", "fiat_rate": "1", "fiat_value": "2", "reason": "r"}),
            json!({"transaction": "t", "action": "price_override", "fiat_rate": "-1", "reason": "r"}),
            json!({"transaction": "t", "action": "price_override", "fiat_rate": "0", "reason": "r"}),
            json!({"transaction": "t", "action": "price_override", "fiat_rate": "1e5", "reason": "r"}),
            json!({"transaction": "t", "action": "delete", "reason": "r"}),
            json!({"transaction": "t", "action": "exclude", "reason": "r", "reprocess": null}),
            json!({"transaction": "t", "action": "exclude", "reason": "r", "extra": 1}),
        ] {
            assert_eq!(resolve(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn custody_reviews_parse_as_their_schema_documents() {
        let d = custody(json!({"action": "classify_residual", "gap_id": "gap-1", "classification": "external_gift", "expected_input_version": 45, "reason": "Birthday"}))
            .expect("a classification");
        assert_eq!(d.classification, Some("external_gift"));
        assert_eq!(d.version, 45);
        assert_eq!(d.reason.as_deref(), Some("Birthday"));
        let d =
            custody(json!({"action": "create", "gap_id": "gap-1", "expected_input_version": 0}))
                .expect("a bridge");
        assert_eq!((d.action, d.classification), ("create", None));
        for bad in [
            json!({"action": "create", "gap_id": "gap-1"}),
            json!({"action": "create", "gap_id": "", "expected_input_version": 1}),
            json!({"action": "create", "gap_id": "g", "expected_input_version": -1}),
            json!({"action": "merge", "gap_id": "g", "expected_input_version": 1}),
            json!({"action": "classify_residual", "gap_id": "g", "expected_input_version": 1}),
            json!({"action": "classify_residual", "gap_id": "g", "classification": "sold", "expected_input_version": 1}),
            json!({"action": "create", "gap_id": "g", "classification": "external_gift", "expected_input_version": 1}),
            json!({"action": "create", "gap_id": "g", "expected_input_version": 1, "authored_source": "user"}),
        ] {
            assert_eq!(custody(bad.clone()), None, "{bad}");
        }
    }

    #[test]
    fn the_card_uses_only_the_plan_the_apply_runs() {
        let mut app = crate::app::tests::quiet_app();
        let plan = |version: u64, class: &str| {
            let mut c = ToolCall {
                call_id: format!("p{version}{class}"),
                name: "ui.custody.review.plan".into(),
                arguments: json!({}),
                kind_class: "read_only".into(),
                needs_consent: false,
                status: ToolStatus::Done,
                approval: None,
                result: Some(json!({"kind": "ui.custody.review.plan", "data": {
                    "gap_id": "gap-1", "action": "classify_residual", "classification": class,
                    "input_version": version, "residual_msat": 5_000_000_000_i64}})),
                reason: None,
                review_preview: None,
                segment_id: None,
            };
            c.arguments = json!({"gap_id": "gap-1"});
            c
        };
        let mut m =
            crate::assistant::model::Msg::new("a1", crate::assistant::model::Role::Assistant, "");
        m.tool_calls = vec![plan(44, "external_gift"), plan(45, "external_loss")];
        app.assistant.messages = vec![m];
        let d = custody(json!({"action": "classify_residual", "gap_id": "gap-1", "classification": "external_loss", "expected_input_version": 45}))
            .unwrap();
        assert!(custody_plan(&app, &d).is_some());
        // Another input version or another classification is not this apply's plan.
        for other in [
            json!({"action": "classify_residual", "gap_id": "gap-1", "classification": "external_loss", "expected_input_version": 44}),
            json!({"action": "classify_residual", "gap_id": "gap-1", "classification": "external_gift", "expected_input_version": 45}),
            json!({"action": "classify_residual", "gap_id": "gap-2", "classification": "external_loss", "expected_input_version": 45}),
        ] {
            let d = custody(other.clone()).unwrap();
            assert!(custody_plan(&app, &d).is_none(), "{other}");
        }
    }

    #[test]
    fn residual_classifications_say_where_the_amount_ends_up() {
        for class in RESIDUAL_CLASSIFICATIONS {
            let key = format!("custodyGaps:residual.custodyState.{}", custody_state(class));
            assert!(crate::i18n::has(&key), "{key}");
            assert!(crate::i18n::has(&format!(
                "custodyGaps:residual.options.{class}.label"
            )));
        }
        for action in GAP_ACTIONS {
            let d = Custody {
                gap: "g".into(),
                action,
                classification: None,
                reason: None,
                version: 0,
            };
            assert!(crate::i18n::has(d.title_key()), "{action}");
            if action != "classify_residual" {
                for k in ["decision", "effect"] {
                    let key = format!("assistant:action.custody.{k}.{action}");
                    assert!(crate::i18n::has(&key), "{key}");
                }
            }
        }
    }
}
