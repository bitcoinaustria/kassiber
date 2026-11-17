//! `SelectionInspector` (the 310 px aside of the graph frame) and `FeatureDetails`.

use rune::{El, Tx, col, row, text};
use serde_json::{Value, json};

use super::kit::{self, fact, fact_text};
use super::model::{self, Query, Sel};
use super::{panels, st};
use crate::app::App;
use crate::i18n::t;
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Variant};
use crate::ui::{ic, sensitive};

pub const WIDTH: f32 = 310.0;

pub fn view(app: &App, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    let s = &app.provenance.chain;
    let r = s.result.as_ref()?;
    let sel = s.sel.as_ref()?;
    let node = (!sel.edge).then(|| r.node(&sel.id)).flatten();
    let edge = sel.edge.then(|| r.edge(&sel.id)).flatten();
    if node.is_none() && edge.is_none() {
        return None;
    }
    let hide = app.prefs.hide_sensitive();
    let selected_id = node
        .map(|n| n.identity().to_owned())
        .or(edge.map(|e| e.id.clone()))
        .unwrap_or_default();
    let mut c = col()
        .w(WIDTH)
        .min_w(0.0)
        .h_full()
        .max_h(super::graph::HEIGHT + 40.0)
        .padding(16.0)
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .role("complementary")
        .label(t("chainAnalysis:inspect"));
    if let Some(n) =
        node.filter(|n| n.kind == "output" && (n.chain == "bitcoin" || n.chain == "liquid"))
    {
        let q = Query {
            mode: "trace".into(),
            subject: Some(n.id.clone()),
            chain: Some(n.chain.clone()),
            network: Some(model::network_input(Some(&n.network))),
            observer: r.query.observer.clone(),
            direction: "forward".into(),
            depth: 2,
            node_limit: 100,
            edge_limit: 200,
            include_relations: false,
            include_hypotheses: false,
            ..Query::default()
        };
        c = c.child(panels::watch_action(
            app,
            tk,
            layer,
            &format!("insp:{}", n.id),
            q,
            None,
            true,
        ));
    }
    let heading = node
        .map(|n| {
            if n.label.is_empty() {
                n.kind.clone()
            } else {
                n.label.clone()
            }
        })
        .or(edge.map(|e| {
            if e.label.is_empty() {
                e.kind.clone()
            } else {
                e.label.clone()
            }
        }))
        .unwrap_or_default();
    let copied = s.copied.is_some_and(|until| until > app.clock);
    let copy_label = t(if copied {
        "chainAnalysis:copied"
    } else {
        "chainAnalysis:copy"
    });
    let id = selected_id.clone();
    let copy = w::icon_button(
        tk,
        "ca-insp-copy",
        Variant::Ghost,
        tk.r(2.0),
        if copied { &I::CHECK } else { &I::COPY },
        tk.r(0.875),
        copy_label,
    )
    .on_press_tx(move |a: &mut App, tx: &mut Tx| {
        crate::settings::effects::copy(a, tx, id.clone());
        st(a).copied = Some(a.clock + 1.5);
        tx.wake_after(std::time::Duration::from_millis(1550));
    });
    let close = w::icon_button(
        tk,
        "ca-insp-close",
        Variant::Ghost,
        tk.r(2.0),
        &I::X,
        tk.r(0.875),
        t("chainAnalysis:workbench.clearSelection"),
    )
    .on_press(|a: &mut App| st(a).sel = None);
    c = c.child(
        row()
            .w_full()
            .gap(8.0)
            .items_start()
            .justify_between()
            .child(
                text(heading)
                    .font(tk.tsm(600.0))
                    .color(tk.fg)
                    .wrap()
                    .grow(1.0)
                    .min_w(0.0)
                    .role("heading")
                    .level(2),
            )
            .child(row().center_y().child(copy).child(close)),
    );
    if let Some(n) = node {
        let busy = super::running(app);
        if n.amount_msat.is_some() {
            c = c.child(
                sensitive(
                    model::format_amount(n.amount_msat.as_deref(), n.asset.as_deref()),
                    tk.tlg(400.0).mono().tracking(-0.45),
                    tk.fg,
                    hide,
                )
                .mt(4.0),
            );
        }
        let back = kit::btn(
            tk,
            "ca-insp-back",
            Variant::Outline,
            Some(&I::ARROW_DOWN_LEFT),
            &t("chainAnalysis:backward"),
            !busy,
        );
        let fwd = kit::btn(
            tk,
            "ca-insp-fwd",
            Variant::Outline,
            Some(&I::ARROW_UP_RIGHT),
            &t("chainAnalysis:forward"),
            !busy,
        );
        let subject = n.id.clone();
        let target = w::button(
            tk,
            "ca-insp-target",
            Variant::Ghost,
            w::Size::Sm,
            None,
            &t("chainAnalysis:setTarget"),
            true,
        )
        .child(ic(&I::CHEVRON_RIGHT, tk.r(0.875), tk.fg))
        .w_full()
        .on_press(move |a: &mut App| {
            let q = &mut st(a).query;
            q.mode = "path".into();
            q.target = Some(subject.clone());
            super::sync_edits(a);
        });
        c = c.child(
            col()
                .w_full()
                .my(16.0)
                .gap(8.0)
                .child(
                    row()
                        .w_full()
                        .gap(8.0)
                        .child(if busy {
                            back.grow(1.0)
                        } else {
                            back.grow(1.0)
                                .on_press(|a: &mut App| super::trace(a, "backward"))
                        })
                        .child(if busy {
                            fwd.grow(1.0)
                        } else {
                            fwd.grow(1.0)
                                .on_press(|a: &mut App| super::trace(a, "forward"))
                        }),
                )
                .child(target),
        );
        let status = n.status.as_ref().map(|s| translated_status(s));
        let facts: Vec<El> = [
            fact(tk, &t("chainAnalysis:id"), &json!(n.id)),
            fact(tk, &t("chainAnalysis:kind"), &json!(n.kind)),
            fact(tk, &t("chainAnalysis:txid"), &json!(n.txid)),
            fact(tk, &t("chainAnalysis:outpoint"), &json!(n.outpoint)),
            fact(tk, &t("chainAnalysis:address"), &json!(n.address)),
            fact(
                tk,
                &t("chainAnalysis:network"),
                &json!(format!("{} / {}", n.chain, n.network)),
            ),
            fact(tk, &t("chainAnalysis:wallets"), &json!(n.wallet_ids)),
            fact(tk, &t("chainAnalysis:status"), &json!(status)),
        ]
        .into_iter()
        .flatten()
        .collect();
        c = c.child(col().w_full().gap(12.0).children(facts));
        c = c.child(kit::evidence(
            tk,
            app,
            &format!("ev-node-{}", n.id),
            &n.evidence,
            None,
        ));
        if let Some(f) = r.features(&n.id) {
            c = c.child(features(app, tk, &format!("feat-{}", n.id), f).mt(20.0));
        }
    }
    if let Some(e) = edge {
        let amount = model::format_amount(e.amount_msat.as_deref(), e.asset.as_deref());
        let mut facts: Vec<El> = [
            fact(tk, &t("chainAnalysis:kind"), &json!(e.kind)),
            fact(
                tk,
                &t("chainAnalysis:showEvidence"),
                &json!(e.evidence_level),
            ),
            fact(
                tk,
                &t("chainAnalysis:status"),
                &json!(translated_status(e.status.as_deref().unwrap_or("unknown"))),
            ),
            Some(fact_text(
                tk,
                &t("chainAnalysis:amount"),
                if hide {
                    crate::theme::MASK.to_owned()
                } else {
                    amount
                },
            )),
        ]
        .into_iter()
        .flatten()
        .collect();
        for (k, id) in [("from", e.source.clone()), ("to", e.target.clone())] {
            let to = id.clone();
            facts.push(
                col()
                    .gap(4.0)
                    .child(kit::detail_key(tk, &t(&format!("chainAnalysis:{k}"))))
                    .child(
                        kit::underlined(&id, tk.txs(400.0).mono(), tk.fg)
                            .wrap_anywhere()
                            .key(format!("ca-insp-{k}"))
                            .role("button")
                            .label(id.clone())
                            .focusable(true)
                            .on_press(move |a: &mut App| super::select(a, Sel::node(to.clone()))),
                    ),
            );
        }
        c = c.child(col().w_full().mt(16.0).gap(12.0).children(facts));
        c = c.child(kit::evidence(
            tk,
            app,
            &format!("ev-edge-{}", e.id),
            &e.evidence,
            None,
        ));
    }
    Some(
        col()
            .h(super::graph::HEIGHT + 40.0)
            .child(c.scroll(rune::ScrollSpec {
                stick_bottom: false,
            })),
    )
}

/// `observationStatus.<status>`, or the status itself.
pub fn translated_status(s: &str) -> String {
    let k = format!("chainAnalysis:observationStatus.{s}");
    if crate::i18n::has(&k) {
        t(&k)
    } else {
        s.to_owned()
    }
}

/// `FeatureDetails`: each feature a disclosure of its value and assumptions.
pub fn features(app: &App, tk: &Tk, k: &str, snapshot: &Value) -> El {
    let s = |key: &str| {
        snapshot
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned()
    };
    let mut list = col()
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout();
    let rows = snapshot
        .get("features")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for (i, f) in rows.iter().enumerate() {
        let code = f.get("code").and_then(Value::as_str).unwrap_or("");
        let key = format!("{k}-{code}");
        let open = kit::is_open(app, &key);
        let summary = kit::summary(
            tk,
            app,
            &key,
            row()
                .gap(12.0)
                .child(
                    text(code.replace('_', " "))
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                )
                .child(
                    text(
                        f.get("availability")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned(),
                    )
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
            [12.0, 12.0],
        );
        let mut item = col().w_full().child(summary);
        if i > 0 {
            item = item.border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        }
        if open {
            let mut body = col().px(12.0).pb(12.0).gap(8.0).child(kit::structured(
                tk,
                f.get("value").unwrap_or(&Value::Null),
                0,
            ));
            if let Some(l) = kit::code_list(
                tk,
                &t("chainAnalysis:entropyPanel.assumptions"),
                f.get("assumptions"),
            ) {
                body = body.child(l);
            }
            item = item.child(body);
        }
        list = list.child(item);
    }
    let mut c = col()
        .w_full()
        .gap(12.0)
        .child(
            text(t("chainAnalysis:workbench.features"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading")
                .level(3),
        )
        .child(
            text(format!("{} · {}", s("extractor_version"), s("source")))
                .font(tk.px(10.0, 400.0))
                .color(tk.muted_fg),
        )
        .child(list);
    if let Some(l) = kit::code_list(
        tk,
        &t("chainAnalysis:entropyPanel.limitations"),
        snapshot.get("limitations"),
    ) {
        c = c.child(l);
    }
    c
}
