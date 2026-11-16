//! `WatchAction` / `WatchInbox` (WatchControls.tsx), `SavedInvestigations`, `CaseComparison`,
//! `AcquisitionPanel` (InvestigationPanels.tsx), `SourceAcquisitionPanel` and
//! `ReportImpactDetails`.
//!
//! Previews (`*.plan`, `watches.preview`) are local. `acquire.apply`, `sources.authorize` and
//! `sources.run` reach the chosen backend: each runs only from its own button, after a preview
//! of exactly the inputs on screen.

use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use super::kit::{self, field, input, opt, select, val};
use super::model::{self, Query, network_input, short_id};
use super::{Bind, report, st};
use crate::app::{App, Reply};
use crate::i18n::{t, tr};
use crate::theme::Tk;
use crate::txdetail::widgets::Variant;

// ---- Watches ----------------------------------------------------------------------------

#[derive(Default)]
pub struct Inbox {
    pub watches: Option<Value>,
    pub events: Option<Value>,
    pub error: Option<String>,
    pub before: Option<i64>,
    pub removing: Option<String>,
    pub busy: bool,
}

#[derive(Default, Clone)]
pub struct WatchUi {
    pub open: bool,
    pub rule: Option<String>,
    pub plan: Option<Value>,
    pub saved: bool,
    pub needs_encryption: bool,
}

fn refresh_inbox(app: &mut App) {
    let before = app.provenance.chain.inbox.before;
    app.send(
        "ui.chain_analysis.watches.list",
        Value::Null,
        "provenance.chain.watches.list",
    );
    let mut args = json!({"limit": 20});
    if let Some(b) = before {
        args["before"] = json!(b);
    }
    app.send(
        "ui.chain_analysis.watches.inbox",
        args,
        "provenance.chain.watches.inbox",
    );
}

fn rules(output: bool, query: &Query) -> Vec<&'static str> {
    if output {
        vec![
            "output_spent",
            "confirmations",
            "attribution_changed",
            "findings_changed",
        ]
    } else if query.mode == "path" {
        vec!["connection_supported", "findings_changed"]
    } else {
        vec!["findings_changed"]
    }
}

/// `WatchAction`: "Watch" opens a local preview of a rule; Start watching saves exactly it.
pub fn watch_action(
    app: &App,
    tk: &Tk,
    layer: &mut Vec<El>,
    key: &str,
    query: Query,
    case_id: Option<String>,
    output: bool,
) -> El {
    let ui = app
        .provenance
        .chain
        .watch
        .get(key)
        .cloned()
        .unwrap_or_default();
    let options = rules(output, &query);
    let rule = ui.rule.clone().unwrap_or_else(|| options[0].to_owned());
    let k = key.to_owned();
    let mut c = col().min_w(0.0).items_start().child(
        kit::btn(
            tk,
            format!("ca-watch-{key}"),
            Variant::Ghost,
            None,
            &t("chainAnalysis:watch.action"),
            true,
        )
        .on_press(move |a: &mut App| {
            let w = st(a).watch.entry(k.clone()).or_default();
            w.open = !w.open;
            w.plan = None;
            let tkey = format!("w.{k}.threshold");
            if !a.provenance.chain.edits.contains_key(&tkey) {
                kit::set_val(a, &tkey, "1");
            }
            super::load_binding(a);
        }),
    );
    if !ui.open {
        return c;
    }
    let binding = app.provenance.chain.binding.clone();
    let bound = binding
        .ready()
        .is_some_and(|b| b.get("state").and_then(Value::as_str) == Some("bound"));
    let scoped = model::watch_query_scope(&query, binding.ready());
    let busy = app.provenance.chain.watch_busy.is_some();
    let small = tk.txs(400.0);
    let mut b = row()
        .w_full()
        .my(8.0)
        .padding(12.0)
        .gap(8.0)
        .flex_wrap()
        .center_y()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout();
    b = b.child(
        text(t("chainAnalysis:watch.localOnly"))
            .font(small)
            .color(tk.muted_fg)
            .wrap()
            .w_full(),
    );
    if !bound {
        b = b.child(match &binding {
            Bind::Loading | Bind::Idle => text(t("chainAnalysis:scopeLoading"))
                .font(small)
                .color(tk.fg)
                .w_full(),
            Bind::Failed(e) => text(e.clone()).font(small).color(tk.fg).w_full(),
            Bind::Ready(_) => row().w_full().child(kit::link(
                tk,
                format!("ca-watch-bind-{key}"),
                &t("chainAnalysis:watch.bindBook"),
                small,
                "/settings/bitcoin",
            )),
        });
    } else {
        let scope = match (&scoped.chain, &scoped.network) {
            (Some(c), n) => format!("{c} / {}", n.clone().unwrap_or_default()),
            _ => tr(
                "chainAnalysis:watch.bookScope",
                &[(
                    "environment",
                    binding
                        .ready()
                        .and_then(|b| b.get("environment"))
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                )],
            ),
        };
        b = b.child(text(scope).font(small).color(tk.muted_fg).w_full());
    }
    let k = key.to_owned();
    b = b.child(select(
        app,
        tk,
        layer,
        &format!("ca-watch-rule-{key}"),
        &t("chainAnalysis:watch.rule"),
        &rule,
        options
            .iter()
            .map(|r| opt(*r, t(&format!("chainAnalysis:watch.rules.{r}"))))
            .collect(),
        !busy,
        32.0,
        move |a: &mut App, v: &str| {
            let w = st(a).watch.entry(k.clone()).or_default();
            w.rule = Some(v.to_owned());
            w.plan = None;
            w.saved = false;
        },
    ));
    let tkey = format!("w.{key}.threshold");
    if rule == "confirmations" {
        if !app.provenance.chain.edits.contains_key(&tkey) {
            // The field starts at 1 (`useState(1)`); seeded on first draw by the edit callback.
        }
        b = b.child(
            input(
                tk,
                app,
                &tkey,
                &t("chainAnalysis:watch.threshold"),
                "1",
                false,
                32.0,
                true,
                None,
            )
            .w(96.0),
        );
    }
    match &ui.plan {
        None => {
            let enabled = !busy && !ui.saved && bound;
            let (k, case, q, r) = (
                key.to_owned(),
                case_id.clone(),
                scoped.clone(),
                rule.clone(),
            );
            let btn = kit::btn(
                tk,
                format!("ca-watch-preview-{key}"),
                Variant::Outline,
                None,
                &t("chainAnalysis:watch.preview"),
                enabled,
            );
            b = b.child(if enabled {
                btn.on_press(move |a: &mut App| {
                    let mut args = json!({"rule": r});
                    match &case {
                        Some(id) => args["case_id"] = json!(id),
                        None => args["query"] = q.to_json(),
                    }
                    if r == "confirmations" {
                        let v = val(a, &format!("w.{k}.threshold"));
                        args["threshold"] = json!(if v.is_empty() {
                            1
                        } else {
                            v.trim().parse::<i64>().unwrap_or(0)
                        });
                    }
                    if a.send(
                        "ui.chain_analysis.watches.preview",
                        args,
                        "provenance.chain.watch.preview",
                    )
                    .is_some()
                    {
                        st(a).watch_busy = Some(k.clone());
                    }
                })
            } else {
                btn
            });
        }
        Some(plan) => {
            let status = plan
                .pointer("/baseline/status")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            b = b.child(
                text(tr(
                    "chainAnalysis:watch.baseline",
                    &[(
                        "status",
                        &t(&format!("chainAnalysis:watch.status.{status}")),
                    )],
                ))
                .font(small)
                .color(tk.fg),
            );
            let (k, p) = (key.to_owned(), plan.clone());
            let btn = kit::btn(
                tk,
                format!("ca-watch-enable-{key}"),
                Variant::Default,
                None,
                &t("chainAnalysis:watch.enable"),
                !busy,
            );
            b = b.child(if busy {
                btn
            } else {
                btn.on_press(move |a: &mut App| {
                    if a.send(
                        "ui.chain_analysis.watches.create",
                        json!({"plan": p}),
                        "provenance.chain.watch.create",
                    )
                    .is_some()
                    {
                        st(a).watch_busy = Some(k.clone());
                    }
                })
            });
        }
    }
    if ui.needs_encryption {
        b = b.child(kit::link(
            tk,
            format!("ca-watch-enc-{key}"),
            &t("chainAnalysis:watch.encryption"),
            small,
            "/settings/security",
        ));
    }
    if ui.saved {
        b = b.child(
            text(t("chainAnalysis:watch.saved"))
                .font(small)
                .color(tk.fg)
                .role("status"),
        );
    }
    c = c.child(b);
    c
}

/// `WatchInbox`: the watches and their local events (the page frames it as a section).
pub fn inbox(app: &App, tk: &Tk) -> El {
    let ib = &app.provenance.chain.inbox;
    let mut c = col().w_full();
    let small = tk.txs(400.0);
    let busy = ib.busy;
    let watches: Vec<Value> = ib
        .watches
        .as_ref()
        .and_then(|w| w.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut body = col().w_full().px(16.0).pb(16.0).gap(12.0).child(
        text(t("chainAnalysis:watch.localOnly"))
            .font(small)
            .color(tk.muted_fg)
            .wrap(),
    );
    if let Some(e) = &ib.error {
        body = body.child(text(e.clone()).font(small).color(tk.fg).role("alert"));
    }
    if watches.is_empty() {
        body = body.child(
            text(t("chainAnalysis:watch.empty"))
                .font(small)
                .color(tk.fg),
        );
    }
    for wv in &watches {
        let id = wv
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let rev = wv.get("revision").cloned().unwrap_or(json!(0));
        let enabled = wv.get("enabled").and_then(Value::as_bool).unwrap_or(false);
        let rule = wv
            .pointer("/definition/rule")
            .and_then(Value::as_str)
            .unwrap_or("");
        let status = wv
            .pointer("/baseline/status")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let q = wv.pointer("/definition/query").cloned();
        let qs = |k: &str| {
            q.as_ref()
                .and_then(|q| q.get(k))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned()
        };
        let observer = qs("observer");
        let subj = qs("subject");
        let meta = format!(
            "{} / {} · {} · {}",
            qs("chain"),
            qs("network"),
            if observer.is_empty() {
                String::new()
            } else {
                t(&format!("chainAnalysis:{observer}"))
            },
            if subj.is_empty() {
                t("chainAnalysis:overview")
            } else {
                short_id(&subj, 24)
            }
        );
        let removing = ib.removing.as_deref() == Some(id.as_str());
        let mut r = row()
            .w_full()
            .py(8.0)
            .gap(8.0)
            .flex_wrap()
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .child(
                        text(format!(
                            "{} · {}",
                            t(&format!("chainAnalysis:watch.rules.{rule}")),
                            t(&format!("chainAnalysis:watch.status.{status}"))
                        ))
                        .font(small)
                        .color(tk.fg),
                    )
                    .child(text(meta).font(small.mono()).color(tk.muted_fg).wrap()),
            );
        let (did, drev) = (id.clone(), rev.clone());
        let del = kit::btn(
            tk,
            format!("ca-w-del-{id}"),
            Variant::Ghost,
            None,
            &t(if removing {
                "chainAnalysis:watch.confirmDelete"
            } else {
                "chainAnalysis:watch.delete"
            }),
            !busy,
        );
        r = r.child(if busy {
            del
        } else {
            del.on_press(move |a: &mut App| {
                if a.provenance.chain.inbox.removing.as_deref() != Some(did.as_str()) {
                    st(a).inbox.removing = Some(did.clone());
                    return;
                }
                if a.send(
                    "ui.chain_analysis.watches.delete",
                    json!({"id": did, "expected_revision": drev}),
                    "provenance.chain.watch.delete",
                )
                .is_some()
                {
                    st(a).inbox.busy = true;
                }
            })
        });
        if let Some(q) = q.clone() {
            r = r.child(
                kit::btn(
                    tk,
                    format!("ca-w-open-{id}"),
                    Variant::Ghost,
                    None,
                    &t("chainAnalysis:watch.open"),
                    true,
                )
                .on_press(move |a: &mut App| super::execute(a, Query::from_json(&q))),
            );
        }
        let (cid, crev) = (id.clone(), rev);
        let toggle = kit::btn(
            tk,
            format!("ca-w-toggle-{id}"),
            Variant::Ghost,
            None,
            &t(if enabled {
                "chainAnalysis:watch.pause"
            } else {
                "chainAnalysis:watch.resume"
            }),
            !busy,
        );
        r = r.child(if busy {
            toggle
        } else {
            toggle.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.watches.configure",
                    json!({"id": cid, "expected_revision": crev, "enabled": !enabled}),
                    "provenance.chain.watch.configure",
                )
                .is_some()
                {
                    st(a).inbox.busy = true;
                }
            })
        });
        body = body.child(r);
    }
    if !watches.is_empty() {
        let b = kit::btn(
            tk,
            "ca-w-check",
            Variant::Outline,
            None,
            &t("chainAnalysis:watch.check"),
            !busy,
        );
        body = body.child(row().child(if busy {
            b
        } else {
            b.on_press(|a: &mut App| {
                if a.send(
                    "ui.chain_analysis.watches.evaluate",
                    json!({}),
                    "provenance.chain.watch.evaluate",
                )
                .is_some()
                {
                    st(a).inbox.busy = true;
                }
            })
        }));
    }
    let events: Vec<Value> = ib
        .events
        .as_ref()
        .and_then(|e| e.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for ev in &events {
        let id = ev
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let code = ev.get("code").and_then(Value::as_str).unwrap_or("");
        let mut r = row()
            .w_full()
            .py(8.0)
            .gap(8.0)
            .flex_wrap()
            .center_y()
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
        if let Some(impact) = ev.get("report_impact").filter(|v| v.is_object()) {
            r = r.child(report_impact(
                app,
                tk,
                &format!("impact-{id}"),
                impact,
                ev.get("observation").unwrap_or(&Value::Null),
            ));
        }
        r = r.child(
            row()
                .grow(1.0)
                .min_w(0.0)
                .gap(4.0)
                .flex_wrap()
                .child(
                    text(t(&format!("chainAnalysis:watch.events.{code}")))
                        .font(small)
                        .color(tk.fg),
                )
                .child(
                    text(
                        ev.get("created_at")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    )
                    .font(small)
                    .color(tk.muted_fg),
                ),
        );
        let wq = ev
            .get("watch_id")
            .and_then(Value::as_str)
            .and_then(|wid| {
                watches
                    .iter()
                    .find(|w| w.get("id").and_then(Value::as_str) == Some(wid))
            })
            .and_then(|w| w.pointer("/definition/query"))
            .cloned();
        if let Some(q) = wq {
            r = r.child(
                kit::btn(
                    tk,
                    format!("ca-e-open-{id}"),
                    Variant::Ghost,
                    None,
                    &t("chainAnalysis:watch.open"),
                    true,
                )
                .on_press(move |a: &mut App| super::execute(a, Query::from_json(&q))),
            );
        }
        if ev.get("acknowledged_at").is_none_or(Value::is_null) {
            let aid = id.clone();
            let b = kit::btn(
                tk,
                format!("ca-e-ack-{id}"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:watch.acknowledge"),
                !busy,
            );
            r = r.child(if busy {
                b
            } else {
                b.on_press(move |a: &mut App| {
                    if a.send(
                        "ui.chain_analysis.watches.acknowledge",
                        json!({"id": aid}),
                        "provenance.chain.watch.ack",
                    )
                    .is_some()
                    {
                        st(a).inbox.busy = true;
                    }
                })
            });
        }
        body = body.child(r);
    }
    if ib.before.is_some() {
        body = body.child(
            kit::btn(
                tk,
                "ca-w-latest",
                Variant::Ghost,
                None,
                &t("chainAnalysis:watch.latest"),
                true,
            )
            .on_press(|a: &mut App| {
                st(a).inbox.before = None;
                refresh_inbox(a);
            }),
        );
    }
    if let Some(next) = ib
        .events
        .as_ref()
        .and_then(|e| e.get("next_cursor"))
        .and_then(Value::as_i64)
    {
        body = body.child(
            kit::btn(
                tk,
                "ca-w-older",
                Variant::Ghost,
                None,
                &t("chainAnalysis:watch.older"),
                true,
            )
            .on_press(move |a: &mut App| {
                st(a).inbox.before = Some(next);
                refresh_inbox(a);
            }),
        );
    }
    c = c.child(body);
    c
}

/// `ReportImpactDetails`: a saved or filed report's identity and exact before/after gains.
fn report_impact(app: &App, tk: &Tk, k: &str, impact: &Value, observation: &Value) -> El {
    let s = |v: &Value, p: &str| {
        v.pointer(p)
            .map(|x| match x {
                Value::String(s) => s.clone(),
                Value::Null => "?".into(),
                other => other.to_string(),
            })
            .unwrap_or_else(|| "?".into())
    };
    let evidence = |v: &Value| {
        let status = v
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("unavailable");
        if status == "observed" {
            let confirmed = v.get("confirmed").and_then(Value::as_bool).unwrap_or(false);
            tr(
                if confirmed {
                    "chainAnalysis:watch.report.confirmed"
                } else {
                    "chainAnalysis:watch.report.unconfirmed"
                },
                &[("height", &s(v, "/block_height"))],
            )
        } else {
            t(&format!("chainAnalysis:watch.report.{status}"))
        }
    };
    let small = tk.txs(400.0);
    let before = observation.get("before").cloned().unwrap_or(Value::Null);
    let after = observation.get("after").cloned().unwrap_or(Value::Null);
    let title = format!(
        "{} · {}",
        tr(
            "chainAnalysis:watch.report.title",
            &[
                ("start", &s(impact, "/period_start_year")),
                ("end", &s(impact, "/period_end_year"))
            ]
        ),
        t(&format!(
            "chainAnalysis:watch.report.{}",
            impact
                .get("report_state")
                .and_then(Value::as_str)
                .unwrap_or("saved")
        ))
    );
    let mut c = col()
        .w_full()
        .padding(12.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(kit::summary_plain(
            tk,
            app,
            k,
            text(title).font(tk.txs(500.0)).color(tk.fg),
        ));
    if !kit::is_open(app, k) {
        return c;
    }
    let p = |s: String| text(s).font(small).color(tk.fg).wrap();
    let mut body = col().mt(8.0).gap(8.0).child(p(tr(
        "chainAnalysis:watch.report.evidence",
        &[("before", &evidence(&before)), ("after", &evidence(&after))],
    )));
    if !before.get("block_hash").is_none_or(Value::is_null)
        || !after.get("block_hash").is_none_or(Value::is_null)
    {
        body = body.child(p(format!(
            "{} {} → {}",
            t("chainAnalysis:watch.report.blocks"),
            s(&before, "/block_hash"),
            s(&after, "/block_hash")
        )));
    }
    body = body
        .child(p(format!(
            "{} {}",
            t("chainAnalysis:watch.report.identity"),
            s(impact, "/snapshot_id")
        )))
        .child(
            text(format!("SHA-256: {}", s(impact, "/content_sha256")))
                .font(small.mono())
                .color(tk.fg)
                .wrap(),
        )
        .child(p(tr(
            "chainAnalysis:watch.report.gainBefore",
            &[
                ("value", &s(impact, "/before_gain_summary/gain_loss_exact")),
                (
                    "currency",
                    impact
                        .pointer("/before_gain_summary/fiat_currency")
                        .and_then(Value::as_str)
                        .unwrap_or(""),
                ),
            ],
        )));
    match impact.get("resolution").filter(|r| r.is_object()) {
        Some(res) => {
            body = body
                .child(p(tr(
                    "chainAnalysis:watch.report.gainAfter",
                    &[
                        ("value", &s(res, "/after_gain_summary/gain_loss_exact")),
                        (
                            "currency",
                            res.pointer("/after_gain_summary/fiat_currency")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                        ),
                    ],
                )))
                .child(p(t(&format!(
                    "chainAnalysis:watch.report.{}",
                    res.get("amendment_status")
                        .and_then(Value::as_str)
                        .unwrap_or("review_required")
                ))));
        }
        None => body = body.child(p(t("chainAnalysis:watch.report.pending"))),
    }
    body = body.child(p(t("chainAnalysis:watch.report.filingAuthority")));
    let mut links = row().gap(12.0).flex_wrap();
    if impact
        .get("transaction_available")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        let tx_id = impact
            .get("transaction_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let label = t("chainAnalysis:watch.report.transaction");
        links = links.child(
            kit::underlined(&label, small, tk.fg)
                .key(format!("{k}-tx"))
                .role("link")
                .label(label.clone())
                .focusable(true)
                .on_press(move |a: &mut App| {
                    crate::shell::go(a, &format!("/transactions?tx={tx_id}"))
                }),
        );
    }
    links = links
        .child(kit::link(
            tk,
            format!("{k}-journals"),
            &t("chainAnalysis:watch.report.journals"),
            small,
            "/journals",
        ))
        .child(kit::link(
            tk,
            format!("{k}-reports"),
            &t("chainAnalysis:watch.report.reports"),
            small,
            "/reports",
        ));
    c = c.child(body.child(links));
    c
}

// ---- Saved investigations ---------------------------------------------------------------

#[derive(Default)]
pub struct Saved {
    pub page: Option<Value>,
    pub loading: bool,
    pub error: Option<String>,
    pub previous: Vec<Value>,
    pub cursor: Option<String>,
    pub comparison: Option<Value>,
    pub confirm_delete: Option<String>,
    pub busy: bool,
}

/// `ui.chain_analysis.cases.list` for the current cursor.
pub fn load_cases(app: &mut App) {
    let mut args = json!({"limit": 20});
    if let Some(c) = &app.provenance.chain.saved.cursor {
        args["cursor"] = json!(c);
    }
    if app
        .send(
            "ui.chain_analysis.cases.list",
            args,
            "provenance.chain.cases.list",
        )
        .is_some()
    {
        st(app).saved.loading = true;
    }
}

fn refresh_cases(app: &mut App) {
    let s = &mut st(app).saved;
    s.previous.clear();
    s.cursor = None;
    load_cases(app);
}

/// `SavedInvestigations`.
pub fn saved(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let s = &app.provenance.chain.saved;
    let result = app.provenance.chain.result.clone();
    let title = val(app, "case.title");
    let busy = s.busy;
    let can_save = result.is_some() && !title.trim().is_empty() && !busy;
    let save = kit::btn(
        tk,
        "ca-case-save",
        Variant::Outline,
        None,
        &t("chainAnalysis:case.save"),
        can_save,
    );
    let mut c = col().w_full().px(16.0).pb(16.0).child(
        row()
            .w_full()
            .gap(8.0)
            .flex_wrap()
            .child(
                input(
                    tk,
                    app,
                    "case.title",
                    &t("chainAnalysis:case.name"),
                    &t("chainAnalysis:case.placeholder"),
                    false,
                    32.0,
                    true,
                    None,
                )
                .min_w(192.0)
                .grow(1.0),
            )
            .child(if can_save {
                save.on_press(save_case)
            } else {
                save
            }),
    );
    if let Some(e) = &s.error {
        c = c.child(kit::alert_text(tk, e.clone(), tk.tsm(400.0)).mt(12.0));
    }
    let mut items = s.previous.clone();
    items.extend(
        s.page
            .as_ref()
            .and_then(|p| p.get("items"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
    );
    if items.is_empty() && !s.loading {
        c = c.child(
            text(t("chainAnalysis:case.empty"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .mt(16.0),
        );
    }
    let mut list = col().w_full().mt(12.0).gap(4.0);
    let n = items.len();
    for (i, item) in items.iter().enumerate() {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let meta = format!(
            "{} · {}",
            item.get("created_at").and_then(Value::as_str).unwrap_or(""),
            short_id(
                item.get("snapshot_id")
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                24
            )
        );
        let mut r = row()
            .w_full()
            .py(8.0)
            .gap(8.0)
            .flex_wrap()
            .center_y()
            .child(
                col()
                    .min_w(160.0)
                    .grow(1.0)
                    .child(
                        text(
                            item.get("title")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .to_owned(),
                        )
                        .font(tk.tsm(500.0))
                        .color(tk.fg)
                        .wrap(),
                    )
                    .child(
                        text(meta)
                            .font(tk.px(10.0, 400.0).mono())
                            .color(tk.muted_fg),
                    ),
            );
        if i + 1 < n {
            r = r.border_sides([0.0, 0.0, 1.0, 0.0], tk.border);
        }
        let (gid, cid) = (id.clone(), id.clone());
        let load = kit::btn(
            tk,
            format!("ca-case-load-{id}"),
            Variant::Ghost,
            None,
            &t("chainAnalysis:case.load"),
            !busy,
        );
        let cmp = kit::btn(
            tk,
            format!("ca-case-cmp-{id}"),
            Variant::Ghost,
            None,
            &t("chainAnalysis:case.compare"),
            !busy,
        );
        r = r.child(if busy {
            load
        } else {
            load.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.cases.get",
                    json!({"id": gid}),
                    "provenance.chain.cases.get",
                )
                .is_some()
                {
                    st(a).saved.busy = true;
                }
            })
        });
        r = r.child(if busy {
            cmp
        } else {
            cmp.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.cases.compare",
                    json!({"id": cid}),
                    "provenance.chain.cases.compare",
                )
                .is_some()
                {
                    st(a).saved.busy = true;
                }
            })
        });
        r = r.child(watch_action(
            app,
            tk,
            layer,
            &format!("case:{id}"),
            Query::from_json(item.get("query").unwrap_or(&Value::Null)),
            Some(id.clone()),
            false,
        ));
        if s.confirm_delete.as_deref() == Some(id.as_str()) {
            let did = id.clone();
            let del = kit::btn_destructive(
                tk,
                format!("ca-case-del-{id}"),
                &t("chainAnalysis:case.delete"),
                !busy,
            );
            r = r.child(
                row()
                    .gap(8.0)
                    .center_y()
                    .child(
                        text(t("chainAnalysis:case.deleteConfirm"))
                            .font(tk.txs(400.0))
                            .color(tk.fg),
                    )
                    .child(if busy {
                        del
                    } else {
                        del.on_press(move |a: &mut App| {
                            if a.send(
                                "ui.chain_analysis.cases.delete",
                                json!({"id": did}),
                                "provenance.chain.cases.delete",
                            )
                            .is_some()
                            {
                                st(a).saved.busy = true;
                            }
                        })
                    })
                    .child(
                        kit::btn(
                            tk,
                            format!("ca-case-keep-{id}"),
                            Variant::Ghost,
                            None,
                            &t("chainAnalysis:dismiss"),
                            true,
                        )
                        .on_press(|a: &mut App| st(a).saved.confirm_delete = None),
                    ),
            );
        } else {
            let did = id.clone();
            let del = kit::btn(
                tk,
                format!("ca-case-askdel-{id}"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:case.delete"),
                !busy,
            );
            r = r.child(if busy {
                del
            } else {
                del.on_press(move |a: &mut App| st(a).saved.confirm_delete = Some(did.clone()))
            });
        }
        list = list.child(r);
    }
    c = c.child(
        col()
            .w_full()
            .max_h(288.0)
            .child(list.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })),
    );
    if let Some(next) = s
        .page
        .as_ref()
        .and_then(|p| p.get("next_cursor"))
        .and_then(Value::as_str)
        .map(str::to_owned)
    {
        let more = kit::btn(
            tk,
            "ca-case-more",
            Variant::Ghost,
            None,
            &t("chainAnalysis:case.more"),
            !s.loading,
        );
        c = c.child(row().mt(8.0).child(if s.loading {
            more
        } else {
            more.on_press(move |a: &mut App| {
                let s = &mut st(a).saved;
                let page: Vec<Value> = s
                    .page
                    .as_ref()
                    .and_then(|p| p.get("items"))
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default();
                s.previous.extend(page);
                s.cursor = Some(next.clone());
                load_cases(a);
            })
        }));
    }
    if let Some(cmp) = &s.comparison {
        c = c.child(comparison(app, tk, cmp));
    }
    c
}

fn save_case(a: &mut App) {
    let title = val(a, "case.title").trim().to_owned();
    let Some(r) = a.provenance.chain.result.clone() else {
        return;
    };
    if title.is_empty() || a.provenance.chain.saved.busy {
        return;
    }
    let args = json!({"title": title.chars().take(200).collect::<String>(), "query": r.raw.get("query").cloned().unwrap_or(Value::Null), "expected_snapshot_id": r.snapshot_id});
    if a.send(
        "ui.chain_analysis.cases.save",
        args,
        "provenance.chain.cases.save",
    )
    .is_some()
    {
        st(a).saved.busy = true;
    }
}

/// `CaseComparison`.
fn comparison(app: &App, tk: &Tk, cmp: &Value) -> El {
    let changed = cmp.get("changed").and_then(Value::as_bool).unwrap_or(false);
    let mut c = col()
        .w_full()
        .mt(12.0)
        .padding(12.0)
        .gap(12.0)
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.15))
        .child(
            text(t(if changed {
                "chainAnalysis:case.changed"
            } else {
                "chainAnalysis:case.unchanged"
            }))
            .font(tk.tsm(400.0))
            .color(tk.fg),
        );
    let mut groups_el = col().w_full().gap(8.0);
    for kind in [
        "nodes",
        "edges",
        "findings",
        "clusters",
        "patterns",
        "exposure",
        "transaction_features",
    ] {
        let groups: Vec<(&str, Vec<Value>)> = ["added", "removed", "changed"]
            .iter()
            .map(|ch| {
                (
                    *ch,
                    cmp.get(format!("{ch}_{kind}"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                )
            })
            .collect();
        if groups.iter().all(|(_, r)| r.is_empty()) {
            continue;
        }
        let name = if kind == "transaction_features" {
            t("chainAnalysis:workbench.features")
        } else {
            t(&format!("chainAnalysis:{kind}"))
        };
        let counts: Vec<String> = groups
            .iter()
            .map(|(ch, r)| crate::i18n::tc(&format!("chainAnalysis:workbench.{ch}"), r.len(), &[]))
            .collect();
        let k = format!("cmp-{kind}");
        let mut d = col()
            .w_full()
            .padding(12.0)
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .border_layout()
            .child(kit::summary_plain(
                tk,
                app,
                &k,
                text(format!("{name} · {}", counts.join(" · ")))
                    .font(tk.txs(500.0))
                    .color(tk.fg),
            ));
        if kit::is_open(app, &k) {
            let mut body = col().mt(12.0).gap(12.0);
            for (ch, rows) in groups.iter().filter(|(_, r)| !r.is_empty()) {
                let mut g = col().gap(8.0).child(
                    text(crate::i18n::tc(
                        &format!("chainAnalysis:workbench.{ch}"),
                        rows.len(),
                        &[],
                    ))
                    .font(tk.txs(500.0))
                    .color(tk.fg),
                );
                if kind == "transaction_features" {
                    for (i, row_v) in rows.iter().take(32).enumerate() {
                        let id = row_v
                            .get("id")
                            .or(row_v.get("subject"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        let mut fc = col().gap(12.0);
                        if let Some(f) = kit::fact(tk, &t("chainAnalysis:id"), &id) {
                            fc = fc.child(f);
                        }
                        let sides: Vec<Value> =
                            if row_v.get("before").is_some() || row_v.get("after").is_some() {
                                vec![
                                    row_v.get("before").cloned().unwrap_or(Value::Null),
                                    row_v.get("after").cloned().unwrap_or(Value::Null),
                                ]
                            } else {
                                vec![row_v.clone()]
                            };
                        let mut grid = row().w_full().gap(12.0);
                        for (si, side) in sides.iter().enumerate() {
                            let mut sc = col().grow(1.0).min_w(0.0);
                            if sides.len() > 1 {
                                sc = sc.child(
                                    text(t(if si == 0 {
                                        "chainAnalysis:workbench.before"
                                    } else {
                                        "chainAnalysis:workbench.after"
                                    }))
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg)
                                    .mb(8.0),
                                );
                            }
                            sc = match side
                                .get("features")
                                .filter(|f| f.get("features").is_some_and(Value::is_array))
                            {
                                Some(f) => sc.child(super::inspector::features(
                                    app,
                                    tk,
                                    &format!("cmpf-{kind}-{ch}-{i}-{si}"),
                                    f,
                                )),
                                None => sc.child(kit::structured(tk, side, 0)),
                            };
                            grid = grid.child(sc);
                        }
                        g = g.child(fc.child(grid));
                    }
                    if rows.len() > 32 {
                        g = g.child(
                            text(tr(
                                "chainAnalysis:workbench.firstRows",
                                &[("count", "32"), ("total", &rows.len().to_string())],
                            ))
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                        );
                    }
                } else {
                    g = g.child(kit::structured(tk, &Value::Array(rows.clone()), 0));
                }
                body = body.child(g);
            }
            d = d.child(body);
        }
        groups_el = groups_el.child(d);
    }
    c = c.child(groups_el);
    if let Some(cov) = cmp
        .get("coverage_changed")
        .filter(|v| !v.is_null() && v != &&json!(false))
    {
        let mut d = col()
            .w_full()
            .padding(12.0)
            .radius(tk.r(0.5))
            .border(1.0, tk.border)
            .border_layout()
            .child(kit::summary_plain(
                tk,
                app,
                "cmp-coverage",
                text(t("chainAnalysis:coverage"))
                    .font(tk.txs(500.0))
                    .color(tk.fg),
            ));
        if kit::is_open(app, "cmp-coverage") {
            d = d.child(col().mt(12.0).child(kit::structured(tk, cov, 0)));
        }
        c = c.child(d);
    }
    c
}

// ---- Acquisition ------------------------------------------------------------------------

#[derive(Default)]
pub struct Acquire {
    /// The binding the form was set up for (`key={profile:environment:revision}`).
    pub for_binding: Option<String>,
    pub backend: String,
    pub chain: String,
    pub plan: Option<Value>,
    pub planned_input: String,
    pub outcome: Option<Value>,
    pub busy: bool,
}

fn supported_domains(b: &Value) -> Vec<Value> {
    b.get("domains")
        .and_then(Value::as_array)
        .map(|d| {
            d.iter()
                .filter(|x| {
                    matches!(
                        x.get("chain").and_then(Value::as_str),
                        Some("bitcoin" | "liquid")
                    )
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn binding_key(b: &Value) -> String {
    format!(
        "{}:{}:{}",
        b.get("profile_id")
            .map(Value::to_string)
            .unwrap_or_default(),
        b.get("environment_id")
            .map(Value::to_string)
            .unwrap_or_default(),
        b.get("revision").map(Value::to_string).unwrap_or_default()
    )
}

/// Shows the acquisition section: reads the binding and backends, sets the form up.
pub fn show_acquire(app: &mut App) {
    let s = st(app);
    s.show_acquire = true;
    s.show_saved = false;
    s.workspace = model::Workspace::Graph;
    super::load_binding(app);
    super::load_backends(app);
}

/// The acquisition args from the form, and the input the plan is bound to.
fn acquire_args(app: &App, b: &Value) -> (Value, String, bool) {
    let a = &app.provenance.chain.acquire;
    let q = &app.provenance.chain.query;
    let domains = supported_domains(b);
    let chain = if a.chain.is_empty() {
        domains
            .iter()
            .find(|d| d.get("chain").and_then(Value::as_str) == q.chain.as_deref())
            .or(domains.first())
            .and_then(|d| d.get("chain"))
            .and_then(Value::as_str)
            .unwrap_or("bitcoin")
            .to_owned()
    } else {
        a.chain.clone()
    };
    let domain = domains
        .iter()
        .find(|d| d.get("chain").and_then(Value::as_str) == Some(chain.as_str()));
    let network = network_input(
        domain
            .and_then(|d| d.get("network"))
            .and_then(Value::as_str),
    );
    let needs_genesis = chain == "liquid" && network != "main";
    let num = |k: &str| {
        val(app, k)
            .trim()
            .parse::<f64>()
            .map(|f| f as i64)
            .unwrap_or(0)
    };
    let subject = val(app, "acq.subject").trim().to_owned();
    let genesis = val(app, "acq.genesis").trim().to_owned();
    let mut args = json!({"backend": a.backend, "subject": subject, "chain": chain, "network": network, "direction": q.direction, "depth": num("acq.depth"), "max_transactions": num("acq.limit")});
    if needs_genesis && !genesis.is_empty() {
        args["genesis_hash"] = json!(genesis);
    }
    let input = json!({"args": args, "environment_id": b.get("environment_id"), "revision": b.get("revision"), "chain_instance_id": b.get("chain_instance_id")}).to_string();
    let backends = compatible_backends(app, &chain, &network, b);
    let ready = domain.is_some()
        && backends
            .iter()
            .any(|x| x.get("name").and_then(Value::as_str) == Some(a.backend.as_str()))
        && matches!(app.provenance.chain.backends, Bind::Ready(_))
        && !subject.is_empty()
        && (!needs_genesis
            || (genesis.len() == 64
                && genesis
                    .bytes()
                    .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))));
    (args, input, ready)
}

fn compatible_backends(app: &App, chain: &str, network: &str, b: &Value) -> Vec<Value> {
    let instance = b.get("chain_instance_id").and_then(Value::as_str);
    app.provenance
        .chain
        .backends
        .ready()
        .and_then(|o| o.get("backends"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter(|x| {
                    let kind = x.get("kind").and_then(Value::as_str).unwrap_or("");
                    let backend_network = x.get("network").and_then(Value::as_str);
                    ["bitcoinrpc", "esplora", "liquid-esplora", "electrum"].contains(&kind)
                        && x.get("chain").and_then(Value::as_str) == Some(chain)
                        && backend_network
                            .is_some_and(|n| !n.is_empty() && network_input(Some(n)) == network)
                        && x.get("chain_instance_id")
                            .and_then(Value::as_str)
                            .is_none_or(|i| Some(i) == instance)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

/// `AcquisitionPanel`.
pub fn acquisition(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let small = tk.tsm(400.0);
    let binding = &app.provenance.chain.binding;
    let b = match binding {
        Bind::Idle | Bind::Loading => {
            return col()
                .px(16.0)
                .pb(16.0)
                .child(text(t("common:state.loading")).font(small).color(tk.fg));
        }
        Bind::Failed(e) => {
            return col()
                .px(16.0)
                .pb(16.0)
                .gap(8.0)
                .child(text(e.clone()).font(small).color(tk.fg).role("alert"))
                .child(kit::link(
                    tk,
                    "ca-acq-bind",
                    &t("settings:bookNetwork.title"),
                    small,
                    "/settings/bitcoin",
                ))
                .child(
                    row().child(
                        crate::txdetail::widgets::button(
                            tk,
                            "ca-acq-retry",
                            Variant::Outline,
                            crate::txdetail::widgets::Size::Default,
                            None,
                            &t("common:actions.retry"),
                            true,
                        )
                        .on_press(|a: &mut App| {
                            st(a).binding = Bind::Idle;
                            super::load_binding(a);
                        }),
                    ),
                );
        }
        Bind::Ready(b) => b.clone(),
    };
    let domains = supported_domains(&b);
    if b.get("state").and_then(Value::as_str) != Some("bound") || domains.is_empty() {
        return col()
            .px(16.0)
            .pb(16.0)
            .gap(8.0)
            .child(
                text(t("settings:bookNetwork.help"))
                    .font(small)
                    .color(tk.fg)
                    .wrap(),
            )
            .child(kit::link(
                tk,
                "ca-acq-bind",
                &t("settings:bookNetwork.title"),
                small,
                "/settings/bitcoin",
            ));
    }
    let a = &app.provenance.chain.acquire;
    let (args, input_key, ready) = acquire_args(app, &b);
    let chain = args["chain"].as_str().unwrap_or("bitcoin").to_owned();
    let network = args["network"].as_str().unwrap_or("main").to_owned();
    let needs_genesis = chain == "liquid" && network != "main";
    let busy = a.busy;
    let backends = compatible_backends(app, &chain, &network, &b);
    let domain = domains
        .iter()
        .find(|d| d.get("chain").and_then(Value::as_str) == Some(chain.as_str()));
    let mut opts = vec![opt("", t("chainAnalysis:acquire.select"))];
    for x in &backends {
        let name = x.get("name").and_then(Value::as_str).unwrap_or("");
        let network_suffix = x
            .get("network")
            .and_then(Value::as_str)
            .map(|n| format!(" · {n}"))
            .unwrap_or_default();
        opts.push(opt(
            name,
            format!(
                "{name} · {}{network_suffix}",
                x.get("kind").and_then(Value::as_str).unwrap_or("")
            ),
        ));
    }
    let backend_sel = select(
        app,
        tk,
        layer,
        "ca-acq-backend",
        &t("chainAnalysis:acquire.backend"),
        &a.backend,
        opts,
        !busy,
        32.0,
        |a: &mut App, v: &str| st(a).acquire.backend = v.to_owned(),
    );
    let chain_sel = select(
        app,
        tk,
        layer,
        "ca-acq-chain",
        &t("chainAnalysis:chain"),
        &chain,
        domains
            .iter()
            .map(|d| {
                let c = d.get("chain").and_then(Value::as_str).unwrap_or("");
                opt(c, if c == "bitcoin" { "Bitcoin" } else { "Liquid" })
            })
            .collect(),
        !busy,
        32.0,
        |a: &mut App, v: &str| st(a).acquire.chain = v.to_owned(),
    );
    let mut net_col = col().gap(5.0);
    if let Some(f) = kit::fact(
        tk,
        &t("chainAnalysis:network"),
        domain
            .and_then(|d| d.get("network"))
            .unwrap_or(&Value::Null),
    ) {
        net_col = net_col.child(f);
    }
    net_col = net_col.child(kit::link(
        tk,
        "ca-acq-netlink",
        &t("settings:bookNetwork.title"),
        tk.txs(400.0),
        "/settings/bitcoin",
    ));
    if let Some(f) = kit::fact(
        tk,
        &t("settings:bookNetwork.instance"),
        b.get("chain_instance_id").unwrap_or(&Value::Null),
    ) {
        net_col = net_col.child(f);
    }
    let mut grid = col().w_full().gap(12.0).child(
        row()
            .w_full()
            .gap(12.0)
            .child(
                field(tk, &t("chainAnalysis:acquire.backend"), backend_sel)
                    .w(0.0)
                    .grow(1.0),
            )
            .child(
                field(
                    tk,
                    &t("chainAnalysis:acquire.subject"),
                    input(
                        tk,
                        app,
                        "acq.subject",
                        &t("chainAnalysis:acquire.subject"),
                        "",
                        true,
                        32.0,
                        !busy,
                        None,
                    ),
                )
                .w(0.0)
                .grow(2.0),
            ),
    );
    grid = grid.child(
        row()
            .w_full()
            .gap(12.0)
            .child(
                field(tk, &t("chainAnalysis:chain"), chain_sel)
                    .w(0.0)
                    .grow(1.0),
            )
            .child(net_col.w(0.0).grow(1.0))
            .child(
                row()
                    .w(0.0)
                    .grow(1.0)
                    .gap(8.0)
                    .child(
                        field(
                            tk,
                            &t("chainAnalysis:acquire.depth"),
                            input(
                                tk,
                                app,
                                "acq.depth",
                                &t("chainAnalysis:acquire.depth"),
                                "",
                                false,
                                32.0,
                                !busy,
                                None,
                            ),
                        )
                        .w(0.0)
                        .grow(1.0),
                    )
                    .child(
                        field(
                            tk,
                            &t("chainAnalysis:acquire.limit"),
                            input(
                                tk,
                                app,
                                "acq.limit",
                                &t("chainAnalysis:acquire.limit"),
                                "",
                                false,
                                32.0,
                                !busy,
                                None,
                            ),
                        )
                        .w(0.0)
                        .grow(1.0),
                    ),
            ),
    );
    if needs_genesis {
        grid = grid.child(field(
            tk,
            &t("chainAnalysis:acquire.genesis"),
            input(
                tk,
                app,
                "acq.genesis",
                &t("chainAnalysis:acquire.genesis"),
                "",
                true,
                32.0,
                !busy,
                None,
            ),
        ));
    }
    let mut c = col()
        .w_full()
        .px(16.0)
        .pb(16.0)
        .child(
            text(t("chainAnalysis:acquire.description"))
                .font(tk.txs(400.0).lh(tk.r(1.21875)))
                .color(tk.muted_fg)
                .wrap()
                .max_w(768.0),
        )
        .child(grid.mt(12.0));
    if let Bind::Failed(e) = &app.provenance.chain.backends {
        c = c.child(kit::alert_text(tk, e.clone(), tk.txs(400.0)).mt(12.0));
    }
    if backends.is_empty() && matches!(app.provenance.chain.backends, Bind::Ready(_)) {
        let to: &'static str = if chain == "liquid" {
            "/settings/liquid"
        } else {
            "/settings/bitcoin"
        };
        c = c.child(
            row()
                .mt(12.0)
                .gap(4.0)
                .flex_wrap()
                .child(
                    text(t("chainAnalysis:acquire.none"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                )
                .child(kit::link(
                    tk,
                    "ca-acq-none",
                    &t("chainAnalysis:acquire.backend"),
                    tk.txs(400.0),
                    to,
                )),
        );
    }
    let preview = kit::btn(
        tk,
        "ca-acq-preview",
        Variant::Outline,
        None,
        &t("chainAnalysis:acquire.preview"),
        !busy && ready,
    );
    let (pargs, pinput) = (args.clone(), input_key.clone());
    c = c.child(row().mt(12.0).child(if !busy && ready {
        preview.on_press(move |a: &mut App| {
            let s = &mut st(a).acquire;
            s.plan = None;
            s.outcome = None;
            s.planned_input = pinput.clone();
            if a.send(
                "ui.chain_analysis.acquire.plan",
                pargs.clone(),
                "provenance.chain.acquire.plan",
            )
            .is_some()
            {
                st(a).acquire.busy = true;
            }
        })
    } else {
        preview
    }));
    if let Some(plan) = &a.plan {
        let f = |k: &str, p: &str| {
            kit::fact(
                tk,
                &t(&format!("chainAnalysis:acquire.{k}")),
                plan.pointer(p).unwrap_or(&Value::Null),
            )
            .map(|e| e.w(0.0).grow(1.0))
            .unwrap_or_else(|| col().w(0.0).grow(1.0))
        };
        let changed = input_key != a.planned_input;
        let mut pb = col()
            .w_full()
            .mt(16.0)
            .padding(12.0)
            .gap(12.0)
            .radius(tk.r(0.375))
            .border(1.0, rune::color::hex("#f59e0b").unwrap().alpha(0.4))
            .border_layout()
            .fill(rune::color::hex("#f59e0b").unwrap().alpha(0.05))
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .child(f("backend", "/backend/name"))
                    .child(f("transactions", "/effects/max_transactions"))
                    .child(f("requests", "/effects/max_requests")),
            )
            .child(
                row()
                    .w_full()
                    .gap(12.0)
                    .child(f("deadline", "/effects/scheduling_deadline_seconds"))
                    .child(f("timeout", "/effects/request_timeout_seconds"))
                    .child(f("responseCap", "/effects/max_response_bytes")),
            )
            .child(kit::evidence(
                tk,
                app,
                "acq-limits",
                plan.get("limitations").unwrap_or(&Value::Null),
                Some(t("chainAnalysis:acquire.limitations")),
            ));
        if changed {
            pb = pb.child(
                text(t("chainAnalysis:acquire.planChanged"))
                    .font(tk.txs(400.0))
                    .color(tk.fg),
            );
        }
        let ok = !busy && ready && !changed;
        let apply = kit::btn(
            tk,
            "ca-acq-apply",
            Variant::Default,
            None,
            &t("chainAnalysis:acquire.apply"),
            ok,
        );
        let p = plan.clone();
        pb = pb.child(row().child(if ok {
            // Egress: the one press that fetches from the backend the plan names.
            apply.on_press(move |a: &mut App| {
                if a.send(
                    "ui.chain_analysis.acquire.apply",
                    json!({"plan": p}),
                    "provenance.chain.acquire.apply",
                )
                .is_some()
                {
                    st(a).acquire.busy = true;
                }
            })
        } else {
            apply
        }));
        c = c.child(pb);
    }
    if let Some(o) = &a.outcome {
        let n = |k: &str| o.get(k).map(Value::to_string).unwrap_or_default();
        let mut oc = col().w_full().mt(16.0).gap(4.0).role("status").child(
            text(tr(
                "chainAnalysis:acquire.finished",
                &[
                    ("count", &n("acquired_count")),
                    ("requests", &n("request_count")),
                ],
            ))
            .font(tk.txs(400.0))
            .color(tk.fg)
            .wrap(),
        );
        if !o.get("complete").and_then(Value::as_bool).unwrap_or(false) {
            oc = oc.child(
                text(t("chainAnalysis:acquire.incomplete"))
                    .font(tk.txs(400.0))
                    .color(tk.fg),
            );
        }
        c = c.child(oc.child(kit::evidence(
            tk,
            app,
            "acq-frontier",
            o.get("frontier").unwrap_or(&Value::Null),
            Some(t("chainAnalysis:frontier")),
        )));
    }
    c
}

/// Seeds the acquisition form from the query when the binding (re)arrives.
fn seed_acquire(app: &mut App) {
    let Some(b) = app.provenance.chain.binding.ready().cloned() else {
        return;
    };
    let k = binding_key(&b);
    if app.provenance.chain.acquire.for_binding.as_deref() == Some(k.as_str()) {
        return;
    }
    let q = app.provenance.chain.query.clone();
    app.provenance.chain.acquire = Acquire {
        for_binding: Some(k),
        ..Acquire::default()
    };
    kit::set_val(app, "acq.subject", q.subject.as_deref().unwrap_or(""));
    kit::set_val(app, "acq.depth", &q.depth.min(10).to_string());
    kit::set_val(app, "acq.limit", "50");
    kit::set_val(app, "acq.genesis", "");
}

// ---- Recurring sources ------------------------------------------------------------------

#[derive(Default)]
pub struct Sources {
    pub list: Option<Value>,
    pub list_error: Option<String>,
    pub last_read: f64,
    pub reading: bool,
    pub backend: String,
    pub mode: String,
    pub preview: Option<(String, Value)>,
    pub busy: bool,
    pub seeded: bool,
}

fn sources_spec(app: &App) -> (Value, bool) {
    let s = &app.provenance.chain.sources;
    let b = app.provenance.chain.binding.ready();
    let bitcoin = b
        .and_then(|b| b.get("domains"))
        .and_then(Value::as_array)
        .and_then(|d| {
            d.iter()
                .find(|x| x.get("chain").and_then(Value::as_str) == Some("bitcoin"))
        })
        .cloned();
    let selected = core_backends(app)
        .into_iter()
        .find(|x| x.get("name").and_then(Value::as_str) == Some(s.backend.as_str()));
    let num = |k: &str| val(app, k).trim().parse::<f64>().unwrap_or(f64::NAN);
    let mode = if s.mode.is_empty() {
        "subject"
    } else {
        s.mode.as_str()
    };
    let mut spec = json!({"backend": s.backend, "network": selected.as_ref().and_then(|x| x.get("network")).cloned(), "mode": mode});
    if let Some(i) = bitcoin
        .as_ref()
        .and_then(|d| d.get("chain_instance_id"))
        .filter(|v| !v.is_null())
    {
        spec["chain_instance_id"] = i.clone();
    }
    if mode == "subject" {
        spec["subject"] = json!(val(app, "src.txid").trim());
    } else {
        spec["start_height"] = json!(num("src.start"));
        if !val(app, "src.end").trim().is_empty() {
            spec["end_height"] = json!(num("src.end"));
        }
    }
    spec["interval_seconds"] = json!(num("src.interval"));
    spec["duration_days"] = json!(num("src.days"));
    spec["max_requests"] = json!(num("src.requests"));
    spec["max_bytes"] = json!(num("src.mb") * 1024.0 * 1024.0);
    (spec, selected.is_some())
}

fn core_backends(app: &App) -> Vec<Value> {
    let bound_network = app
        .provenance
        .chain
        .binding
        .ready()
        .and_then(|b| b.get("domains"))
        .and_then(Value::as_array)
        .and_then(|d| {
            d.iter()
                .find(|x| x.get("chain").and_then(Value::as_str) == Some("bitcoin"))
        })
        .and_then(|d| d.get("network"))
        .cloned();
    app.provenance
        .chain
        .backends
        .ready()
        .and_then(|o| o.get("backends"))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter(|x| {
                    x.get("kind").and_then(Value::as_str) == Some("bitcoinrpc")
                        && bound_network.is_some()
                        && x.get("network") == bound_network.as_ref()
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default()
}

fn load_sources(app: &mut App) {
    if app.provenance.chain.sources.reading {
        return;
    }
    if app
        .send(
            "ui.chain_analysis.sources.list",
            json!({"limit": 100}),
            "provenance.chain.sources.list",
        )
        .is_some()
    {
        let clock = app.clock;
        let s = &mut st(app).sources;
        s.reading = true;
        s.last_read = clock;
    }
}

fn reason_key(code: &str) -> &'static str {
    match code {
        "backfilling" => "chainAnalysis:sources.backfilling",
        "caught_up" | "range_complete" | "observed" => "chainAnalysis:sources.current",
        "reorg_reconciling" => "chainAnalysis:sources.reorg",
        "acquisition_quota" => "chainAnalysis:sources.limitReached",
        "acquisition_expired" => "chainAnalysis:sources.expired",
        "acquisition_source_changed" | "scope_changed" | "backend_network_mismatch" => {
            "chainAnalysis:sources.changed"
        }
        _ => "chainAnalysis:sources.unavailable",
    }
}

/// `SourceAcquisitionPanel`: the "Keep local evidence up to date" disclosure.
pub fn sources(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let open = kit::is_open(app, "sources");
    let mut c = col()
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .child(kit::summary(
            tk,
            app,
            "sources",
            text(t("chainAnalysis:sources.title"))
                .font(tk.tsm(500.0))
                .color(tk.fg),
            [16.0, 12.0],
        ));
    if !open {
        return c;
    }
    let s = &app.provenance.chain.sources;
    let busy = s.busy;
    let small = tk.tsm(400.0);
    let mut body = col().w_full().px(16.0).pb(12.0).pt(4.0).gap(16.0).child(
        text(t("chainAnalysis:sources.lifecycle"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
    );
    let failed = matches!(app.provenance.chain.backends, Bind::Failed(_))
        || matches!(app.provenance.chain.binding, Bind::Failed(_))
        || s.list_error.is_some();
    if failed {
        body = body.child(
            col()
                .gap(8.0)
                .role("alert")
                .child(
                    text(t("chainAnalysis:sources.loadError"))
                        .font(small)
                        .color(tk.fg),
                )
                .child(
                    row().child(
                        kit::btn(
                            tk,
                            "ca-src-retry",
                            Variant::Outline,
                            None,
                            &t("chainAnalysis:sources.retry"),
                            true,
                        )
                        .on_press(|a: &mut App| {
                            let s = st(a);
                            s.binding = Bind::Idle;
                            s.backends = Bind::Idle;
                            s.sources.list_error = None;
                            super::load_binding(a);
                            super::load_backends(a);
                            load_sources(a);
                        }),
                    ),
                ),
        );
    }
    let binding = app.provenance.chain.binding.ready();
    let bitcoin = binding
        .and_then(|b| b.get("domains"))
        .and_then(Value::as_array)
        .and_then(|d| {
            d.iter()
                .find(|x| x.get("chain").and_then(Value::as_str) == Some("bitcoin"))
        })
        .is_some();
    let backends = core_backends(app);
    match &app.provenance.chain.binding {
        Bind::Idle | Bind::Loading => {
            body = body.child(
                text(t("chainAnalysis:sources.loading"))
                    .font(small)
                    .color(tk.muted_fg)
                    .role("status"),
            )
        }
        Bind::Ready(b) if b.get("state").and_then(Value::as_str) == Some("unbound") => {
            body = body.child(kit::link(
                tk,
                "ca-src-bind",
                &t("chainAnalysis:sources.bindBook"),
                small,
                "/settings/bitcoin",
            ))
        }
        _ if bitcoin
            && matches!(app.provenance.chain.backends, Bind::Ready(_))
            && backends.is_empty() =>
        {
            body = body.child(kit::link(
                tk,
                "ca-src-core",
                &t("chainAnalysis:sources.connectCore"),
                small,
                "/connections",
            ))
        }
        _ => {}
    }
    let (spec, has_backend) = sources_spec(app);
    let input_key = spec.to_string();
    if bitcoin && !backends.is_empty() {
        let mode = if s.mode.is_empty() {
            "subject".to_owned()
        } else {
            s.mode.clone()
        };
        let mut opts = vec![opt("", t("chainAnalysis:sources.chooseCore"))];
        opts.extend(backends.iter().map(|x| {
            let n = x.get("name").and_then(Value::as_str).unwrap_or("");
            opt(n, n)
        }));
        let mut form = col().w_full().gap(12.0).child(
            row()
                .w_full()
                .gap(12.0)
                .child(
                    field(
                        tk,
                        &t("chainAnalysis:acquire.backend"),
                        select(
                            app,
                            tk,
                            layer,
                            "ca-src-backend",
                            &t("chainAnalysis:acquire.backend"),
                            &s.backend,
                            opts,
                            !busy,
                            32.0,
                            |a: &mut App, v: &str| st(a).sources.backend = v.to_owned(),
                        ),
                    )
                    .w(0.0)
                    .grow(1.0),
                )
                .child(
                    field(
                        tk,
                        &t("chainAnalysis:sources.mode"),
                        select(
                            app,
                            tk,
                            layer,
                            "ca-src-mode",
                            &t("chainAnalysis:sources.mode"),
                            &mode,
                            vec![
                                opt("subject", t("chainAnalysis:sources.subject")),
                                opt("blocks", t("chainAnalysis:sources.blocks")),
                            ],
                            !busy,
                            32.0,
                            |a: &mut App, v: &str| st(a).sources.mode = v.to_owned(),
                        ),
                    )
                    .w(0.0)
                    .grow(1.0),
                ),
        );
        form = if mode == "subject" {
            form.child(field(
                tk,
                &t("chainAnalysis:sources.txid"),
                input(
                    tk,
                    app,
                    "src.txid",
                    &t("chainAnalysis:sources.txid"),
                    "",
                    true,
                    32.0,
                    !busy,
                    None,
                ),
            ))
        } else {
            form.child(
                row()
                    .w_full()
                    .gap(12.0)
                    .child(
                        field(
                            tk,
                            &t("chainAnalysis:sources.start"),
                            input(
                                tk,
                                app,
                                "src.start",
                                &t("chainAnalysis:sources.start"),
                                "",
                                false,
                                32.0,
                                !busy,
                                None,
                            ),
                        )
                        .w(0.0)
                        .grow(1.0),
                    )
                    .child(
                        field(
                            tk,
                            &t("chainAnalysis:sources.end"),
                            input(
                                tk,
                                app,
                                "src.end",
                                &t("chainAnalysis:sources.end"),
                                &t("chainAnalysis:sources.followTip"),
                                false,
                                32.0,
                                !busy,
                                None,
                            ),
                        )
                        .w(0.0)
                        .grow(1.0),
                    ),
            )
        };
        let mut budgets = col().w_full().child(kit::summary_plain(
            tk,
            app,
            "src-budgets",
            text(t("chainAnalysis:sources.budgets"))
                .font(tk.txs(400.0))
                .color(tk.fg),
        ));
        if kit::is_open(app, "src-budgets") {
            let f = |k: &str, key: &str| {
                field(
                    tk,
                    &t(&format!("chainAnalysis:sources.{k}")),
                    input(
                        tk,
                        app,
                        key,
                        &t(&format!("chainAnalysis:sources.{k}")),
                        "",
                        false,
                        32.0,
                        !busy,
                        None,
                    ),
                )
                .w(0.0)
                .grow(1.0)
            };
            budgets = budgets.child(
                col()
                    .mt(12.0)
                    .gap(12.0)
                    .child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .child(f("interval", "src.interval"))
                            .child(f("days", "src.days")),
                    )
                    .child(
                        row()
                            .w_full()
                            .gap(12.0)
                            .child(f("requests", "src.requests"))
                            .child(f("megabytes", "src.mb")),
                    ),
            );
        }
        let txid_ok = mode != "subject" || {
            let v = val(app, "src.txid");
            v.trim().len() == 64
                && v.trim()
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        };
        let enabled = !busy && has_backend && txid_ok;
        let (sp, ik) = (spec.clone(), input_key.clone());
        let btn = kit::btn(
            tk,
            "ca-src-preview",
            Variant::Outline,
            None,
            &t("chainAnalysis:sources.preview"),
            !busy && has_backend,
        );
        form = form.child(budgets).child(row().child(if enabled {
            btn.on_press(move |a: &mut App| {
                st(a).sources.preview = None;
                st(a).sources.preview = Some((ik.clone(), Value::Null));
                if a.send(
                    "ui.chain_analysis.sources.plan",
                    sp.clone(),
                    "provenance.chain.sources.plan",
                )
                .is_some()
                {
                    st(a).sources.busy = true;
                }
            })
        } else {
            btn
        }));
        body = body.child(form);
    }
    if let Some((k, plan)) = &s.preview
        && *k == input_key
        && plan.is_object()
    {
        let spec = plan.get("spec").cloned().unwrap_or(Value::Null);
        let g = |p: &str| {
            spec.pointer(p)
                .map(|v| match v {
                    Value::String(s) => s.clone(),
                    Value::Null => t("chainAnalysis:sources.followTip"),
                    o => o.to_string(),
                })
                .unwrap_or_default()
        };
        let what = if g("/mode") == "subject" {
            g("/subject")
        } else {
            format!(
                "{} → {}",
                g("/start_height"),
                if spec.get("end_height").is_none_or(Value::is_null) {
                    t("chainAnalysis:sources.followTip")
                } else {
                    g("/end_height")
                }
            )
        };
        let mb = spec.get("max_bytes").and_then(Value::as_f64).unwrap_or(0.0) / 1024.0 / 1024.0;
        let p = plan.clone();
        let auth = kit::btn(
            tk,
            "ca-src-authorize",
            Variant::Default,
            None,
            &t("chainAnalysis:sources.authorize"),
            !busy,
        );
        body = body.child(
            col()
                .w_full()
                .padding(12.0)
                .gap(12.0)
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .role("status")
                .child(
                    text(format!("{} · {what}", g("/network")))
                        .font(tk.txs(400.0).mono())
                        .color(tk.fg)
                        .wrap(),
                )
                .child(
                    text(tr(
                        "chainAnalysis:sources.permission",
                        &[
                            ("backend", &g("/backend")),
                            ("seconds", &g("/interval_seconds")),
                            ("days", &g("/duration_days")),
                        ],
                    ))
                    .font(small)
                    .color(tk.fg)
                    .wrap(),
                )
                .child(
                    text(tr(
                        "chainAnalysis:sources.quota",
                        &[
                            ("requests", &g("/max_requests")),
                            ("megabytes", &format!("{}", mb)),
                        ],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
                )
                .child(row().child(if busy {
                    auth
                } else {
                    // Grants recurring egress to the named backend; only from this press.
                    auth.on_press(move |a: &mut App| {
                        if a.send(
                            "ui.chain_analysis.sources.authorize",
                            json!({"plan": p}),
                            "provenance.chain.sources.authorize",
                        )
                        .is_some()
                        {
                            st(a).sources.busy = true;
                        }
                    })
                })),
        );
    }
    for item in s
        .list
        .as_ref()
        .and_then(|l| l.get("items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
    {
        let id = item
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let status = item.get("status").and_then(Value::as_str).unwrap_or("");
        let mode = item
            .pointer("/spec/mode")
            .and_then(Value::as_str)
            .unwrap_or("");
        let height = item
            .get("cursor_height")
            .filter(|v| !v.is_null())
            .map(Value::to_string)
            .unwrap_or_else(|| "—".into());
        let mut line = row().gap(8.0).child(
            text(t(match status {
                "active" => "chainAnalysis:sources.active",
                "revoked" => "chainAnalysis:sources.revoked",
                _ => "chainAnalysis:sources.paused",
            }))
            .font(tk.txs(400.0))
            .color(tk.fg),
        );
        if let Some(code) = item.get("last_code").and_then(Value::as_str) {
            line = line.child(
                text(t(reason_key(code)))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        }
        let mut r = row()
            .w_full()
            .pt(12.0)
            .gap(12.0)
            .flex_wrap()
            .center_y()
            .justify_between()
            .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
            .child(
                col()
                    .min_w(0.0)
                    .gap(4.0)
                    .child(
                        text(format!(
                            "{} · {}",
                            item.pointer("/spec/backend")
                                .and_then(Value::as_str)
                                .unwrap_or(""),
                            t(if mode == "blocks" {
                                "chainAnalysis:sources.blocks"
                            } else {
                                "chainAnalysis:sources.subject"
                            })
                        ))
                        .font(tk.tsm(500.0))
                        .color(tk.fg),
                    )
                    .child(
                        text(tr(
                            "chainAnalysis:sources.progress",
                            &[
                                ("height", &height),
                                (
                                    "used",
                                    &item
                                        .get("requests_used")
                                        .map(Value::to_string)
                                        .unwrap_or_default(),
                                ),
                                (
                                    "total",
                                    &item
                                        .pointer("/spec/max_requests")
                                        .map(Value::to_string)
                                        .unwrap_or_default(),
                                ),
                            ],
                        ))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    )
                    .child(line),
            );
        if status == "active" {
            let (rid, vid, rev) = (
                id.clone(),
                id.clone(),
                item.get("revision").cloned().unwrap_or(json!(0)),
            );
            let run = kit::btn(
                tk,
                format!("ca-src-run-{id}"),
                Variant::Ghost,
                None,
                &t("chainAnalysis:sources.run"),
                !busy,
            );
            let revoke = kit::btn(
                tk,
                format!("ca-src-revoke-{id}"),
                Variant::Outline,
                None,
                &t("chainAnalysis:sources.revoke"),
                !busy,
            );
            r = r.child(
                row()
                    .gap(8.0)
                    .child(if busy {
                        run
                    } else {
                        // "Check now" asks the authorized backend: egress, from this press only.
                        run.on_press(move |a: &mut App| {
                            if a.send(
                                "ui.chain_analysis.sources.run",
                                json!({"id": rid}),
                                "provenance.chain.sources.run",
                            )
                            .is_some()
                            {
                                st(a).sources.busy = true;
                            }
                        })
                    })
                    .child(if busy {
                        revoke
                    } else {
                        revoke.on_press(move |a: &mut App| {
                            if a.send(
                                "ui.chain_analysis.sources.revoke",
                                json!({"id": vid, "expected_revision": rev}),
                                "provenance.chain.sources.revoke",
                            )
                            .is_some()
                            {
                                st(a).sources.busy = true;
                            }
                        })
                    }),
            );
        }
        body = body.child(r);
    }
    c = c.child(body);
    c
}

fn seed_sources(app: &mut App) {
    if app.provenance.chain.sources.seeded {
        return;
    }
    let subject = app
        .provenance
        .chain
        .query
        .subject
        .clone()
        .unwrap_or_default();
    let txid = subject
        .as_bytes()
        .windows(64)
        .find(|w| {
            w.iter()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
        })
        .map(|w| String::from_utf8_lossy(w).into_owned())
        .unwrap_or_default();
    for (k, v) in [
        ("src.txid", txid.as_str()),
        ("src.start", "0"),
        ("src.end", ""),
        ("src.interval", "300"),
        ("src.days", "30"),
        ("src.requests", "10000"),
        ("src.mb", "1024"),
    ] {
        kit::set_val(app, k, v);
    }
    st(app).sources.seeded = true;
}

/// Seeds forms, and refreshes the sources list every 5 s while it is open (a local read).
pub fn poll(app: &mut App, tx: &mut Tx) -> bool {
    let mut changed = false;
    if app.provenance.chain.show_acquire {
        let before = app.provenance.chain.acquire.for_binding.clone();
        seed_acquire(app);
        changed |= before != app.provenance.chain.acquire.for_binding;
        if !app.provenance.chain.sources.seeded {
            seed_sources(app);
            changed = true;
        }
        if kit::is_open(app, "sources") {
            let s = &app.provenance.chain.sources;
            if !s.reading
                && (s.list.is_none() && s.list_error.is_none() || app.clock - s.last_read >= 5.0)
            {
                load_sources(app);
                changed = true;
            }
            tx.wake_after(std::time::Duration::from_secs(5));
        }
    }
    if app.provenance.chain.show_saved
        && app.provenance.chain.saved.page.is_none()
        && !app.provenance.chain.saved.loading
        && app.provenance.chain.saved.error.is_none()
    {
        load_cases(app);
        changed = true;
    }
    changed
}

pub fn on_reply(app: &mut App, sub: &str, r: &Reply) {
    let err = r.error.clone();
    match sub {
        "watches.list" => {
            let ib = &mut st(app).inbox;
            match err {
                Some(e) => ib.error = Some(e),
                None => ib.watches = Some(r.data.clone()),
            }
        }
        "watches.inbox" => {
            let ib = &mut st(app).inbox;
            match err {
                Some(e) => ib.error = Some(e),
                None => {
                    ib.events = Some(r.data.clone());
                    ib.error = None;
                }
            }
        }
        "watch.preview" | "watch.create" => {
            let Some(k) = st(app).watch_busy.take() else {
                return;
            };
            match err {
                Some(e) => {
                    let code = r.error_detail.get("code").and_then(Value::as_str)
                        == Some("watch_requires_encrypted_database");
                    let w = st(app).watch.entry(k).or_default();
                    w.plan = None;
                    if sub == "watch.preview" {
                        w.needs_encryption = code;
                    }
                    report(app, e);
                }
                None if sub == "watch.preview" => {
                    st(app).watch.entry(k).or_default().plan =
                        Some(r.data.clone()).filter(|v| v.is_object())
                }
                None => {
                    let w = st(app).watch.entry(k).or_default();
                    w.plan = None;
                    w.saved = true;
                    refresh_inbox(app);
                }
            }
        }
        "watch.delete" | "watch.configure" | "watch.ack" | "watch.evaluate" => {
            st(app).inbox.busy = false;
            match err {
                Some(e) => report(app, e),
                None => {
                    if sub == "watch.delete" {
                        st(app).inbox.removing = None;
                    }
                    refresh_inbox(app);
                }
            }
        }
        "cases.list" => {
            let s = &mut st(app).saved;
            s.loading = false;
            match err {
                Some(e) => s.error = Some(e),
                None => {
                    s.page = Some(r.data.clone());
                    s.error = None;
                }
            }
        }
        "cases.save" => {
            st(app).saved.busy = false;
            match err {
                Some(e) => report(app, e),
                None => {
                    kit::set_val(app, "case.title", "");
                    refresh_cases(app);
                    st(app).notice = Some(t("chainAnalysis:case.saved"));
                }
            }
        }
        "cases.get" => {
            st(app).saved.busy = false;
            match err {
                Some(e) => report(app, e),
                None => super::load_case(app, &r.data),
            }
        }
        "cases.delete" => {
            st(app).saved.busy = false;
            match err {
                Some(e) => report(app, e),
                None => {
                    st(app).saved.confirm_delete = None;
                    refresh_cases(app);
                }
            }
        }
        "cases.compare" => {
            st(app).saved.busy = false;
            match err {
                Some(e) => report(app, e),
                None => st(app).saved.comparison = Some(r.data.clone()).filter(|v| v.is_object()),
            }
        }
        "acquire.plan" => {
            st(app).acquire.busy = false;
            match err {
                Some(e) => report(app, e),
                None => st(app).acquire.plan = Some(r.data.clone()).filter(|v| v.is_object()),
            }
        }
        "acquire.apply" => {
            let a = &mut st(app).acquire;
            a.busy = false;
            a.plan = None;
            match err {
                Some(e) => report(app, e),
                None => {
                    st(app).acquire.outcome = Some(r.data.clone());
                    st(app).historical = true;
                }
            }
        }
        "sources.list" => {
            let s = &mut st(app).sources;
            s.reading = false;
            match err {
                Some(e) => s.list_error = Some(e),
                None => {
                    s.list = Some(r.data.clone());
                    s.list_error = None;
                }
            }
        }
        "sources.plan" => {
            st(app).sources.busy = false;
            match err {
                Some(e) => {
                    st(app).sources.preview = None;
                    report(app, e);
                }
                None => {
                    if let Some((k, _)) = st(app).sources.preview.take() {
                        st(app).sources.preview = Some((k, r.data.clone()));
                    }
                }
            }
            load_sources(app);
        }
        "sources.authorize" | "sources.revoke" | "sources.run" => {
            st(app).sources.busy = false;
            if sub == "sources.authorize" && err.is_none() {
                st(app).sources.preview = None;
            }
            if let Some(e) = err {
                report(app, e);
            }
            load_sources(app);
        }
        _ => {}
    }
}
