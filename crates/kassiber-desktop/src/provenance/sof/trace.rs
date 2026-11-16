//! Stage 2, `TraceStage`: the recorded route (the flow graph), the assembly summary, the open
//! questions (finding cards with their one-click fixes) and the advanced review editor (review
//! queue, link review, source or gap, manual link) with the case brief.

use rune::{Color, Cx, El, col, el, row, text};
use serde_json::Value;

use super::kit::{self, amber, emerald};
use super::model::{self, CONFIDENCE_LEVELS, LINK_TYPES, NO_ATTACHMENT, SOURCE_TYPES};
use super::target::{optional_section, stage_header, toggle_open};
use crate::app::App;
use crate::i18n::{t, tc, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};

fn case(a: &mut App) -> &mut super::Case {
    a.provenance.sof.case.as_mut().expect("case")
}

fn method_label(m: &str) -> String {
    let k = format!("sourceFunds:method.{m}");
    if crate::i18n::has(&k) {
        t(&k)
    } else {
        model::pretty(m)
    }
}

fn label_or_pretty(prefix: &str, v: &str) -> String {
    let k = format!("sourceFunds:{prefix}.{v}");
    if crate::i18n::has(&k) {
        t(&k)
    } else {
        model::pretty(v)
    }
}

/// `FlowPathPreview`: the section title, the privacy-hop pill, the route's limits and the graph.
pub fn flow_preview(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    key: &'static str,
    flow: &Value,
    open_tx: bool,
) -> Option<El> {
    if model::arr(flow, "levels").is_empty() {
        return None;
    }
    let mut head = row()
        .w_full()
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(
            text(t("sourceFunds:flowPath.title"))
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(3),
        );
    if !model::arr(flow, "deferred_privacy_hops").is_empty() {
        let a = amber();
        let (fill, border, ink) = kit::notice(tk, &a, a.c900, a.c200);
        head = head.child(
            row()
                .px(tk.r(0.5))
                .py(2.0)
                .radius(999.0)
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .child(
                    text(t("sourceFunds:flowPath.privacyHopDeferred"))
                        .font(tk.txs(400.0))
                        .color(ink),
                ),
        );
    }
    let mut out = col().w_full().min_w(0.0).gap(tk.r(0.5)).child(head);
    let note = model::s(flow, "note");
    if !note.is_empty() {
        let k = format!("flow-limits-{key}");
        let open = app
            .provenance
            .sof
            .case
            .as_ref()
            .is_some_and(|c| c.is_open(&k));
        let k2 = k.clone();
        let mut d = col().w_full().child(kit::summary(
            tk,
            &format!("sof-{k}"),
            open,
            &t("sourceFunds:journey.flowLimits"),
            tk.txs(400.0),
            tk.muted_fg,
            move |a| toggle_open(a, &k2),
        ));
        if open {
            d = d.child(
                text(note.to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .mt(tk.r(0.5)),
            );
        }
        out = out.child(d);
    }
    Some(out.child(super::graph::view(app, cx, tk, key, flow, open_tx)))
}

/// `GateRow`: one finding, its localized copy and its direct action.
pub fn gate_row(app: &App, tk: &Tk, i: usize, finding: &Value, actions: bool) -> El {
    let (title_k, msg_k, next_k) = model::finding_keys(finding);
    let code = model::s(finding, "code");
    let blocker = model::s(finding, "severity") == "blocker";
    let raw_head = finding["next_step"]["headline"]
        .as_str()
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let headline = match &next_k {
        Some(k) => t(k),
        None => raw_head.clone(),
    };
    let action = finding["next_step"]["action"]
        .as_str()
        .map(str::trim)
        .unwrap_or("")
        .to_owned();
    let action_label = (!action.is_empty() && model::GAP_ACTIONS.contains(&action.as_str()))
        .then(|| label_or_pretty("gapAction", &action));
    let a = amber();
    let (fill, border, ink) = if blocker {
        kit::notice(tk, &a, a.c950, a.c100)
    } else {
        (Color::TRANSPARENT, tk.border, tk.fg)
    };
    let title = title_k
        .map(|k| t(&k))
        .unwrap_or_else(|| model::pretty(code));
    let message = msg_k
        .map(|k| t(&k))
        .unwrap_or_else(|| model::s(finding, "message").to_owned());
    let mut b = col()
        .key(format!("sof-gate-{i}"))
        .w_full()
        .min_w(0.0)
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .child(text(title).font(tk.tsm(500.0)).color(ink).wrap())
        .child(
            text(message)
                .font(tk.txs(400.0))
                .color(ink.alpha(0.8))
                .wrap()
                .mt(tk.r(0.25)),
        );
    if !headline.is_empty() && (!actions || action_label.is_none() || next_k.is_none()) {
        b = b.child(
            text(tr("sourceFunds:gates.nextStep", &[("headline", &headline)]))
                .font(tk.txs(500.0))
                .color(ink.alpha(0.9))
                .wrap()
                .mt(tk.r(0.5)),
        );
    }
    let mut row_ = row().gap(tk.r(0.75)).flex_wrap();
    if actions {
        if let Some(l) = action_label {
            let f = finding.clone();
            let act = action.clone();
            row_ = row_.child(kit::accent_link(
                tk,
                format!("sof-gate-{i}-act"),
                &I::ARROW_RIGHT,
                &l,
                move |a: &mut App| super::gap_action(a, &act, &f),
            ));
        }
        let reff = model::s(finding, "ref").to_owned();
        if !reff.is_empty() && super::tx_by_id(app).contains_key(&reff) {
            row_ = row_.child(kit::accent_link(
                tk,
                format!("sof-gate-{i}-tx"),
                &I::EYE,
                &t("sourceFunds:workstation.openTransactionToFix"),
                move |a: &mut App| super::open_tx(a, &reff),
            ));
        }
    }
    b.child(row_)
}

pub fn view(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let report = super::report(app).cloned();
    let blockers = report
        .as_ref()
        .map(|r| model::arr(&r["explain_gates"], "blockers").to_vec())
        .unwrap_or_default();
    let warnings = report
        .as_ref()
        .map(|r| model::arr(&r["explain_gates"], "warnings").to_vec())
        .unwrap_or_default();
    let gaps: Vec<Value> = blockers.iter().chain(warnings.iter()).cloned().collect();
    let action = app.prefs.ai_features.then(|| {
        let label = t(if gaps.is_empty() {
            "sourceFunds:journey.reviewWithAssistant"
        } else {
            "sourceFunds:journey.resolveWithAssistant"
        });
        let on = super::assistant_available(app);
        let b = kit::button(
            tk,
            "sof-investigate",
            Variant::Default,
            Size::Default,
            Some(&I::SPARKLES),
            &label,
            on,
        );
        if on {
            b.on_press(super::investigate)
        } else {
            b
        }
    });
    let mut out = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.25))
        .child(stage_header(
            tk,
            &t("sourceFunds:journey.traceTitle"),
            &t("sourceFunds:journey.traceHint"),
            action,
        ));
    if let Some(r) = &report
        && let Some(f) = flow_preview(app, cx, tk, "trace", &r["simplified_flow"], true)
    {
        out = out.child(f);
    }
    if let Some(asm) = &c.assembled {
        let methods: Vec<String> = asm["methods"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| format!("{} ×{}", method_label(k), v))
                    .collect()
            })
            .unwrap_or_default();
        let mut b = row()
            .w_full()
            .gap(tk.r(0.75))
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .items_end()
            .flex_wrap()
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .fill(tk.muted.alpha(0.3))
            .child(
                text(tc(
                    "sourceFunds:actionsBar.assembleResult",
                    asm["auto_reviewed"].as_u64().unwrap_or(0) as usize,
                    &[],
                ))
                .font(tk.tsm(600.0))
                .color(tk.fg),
            )
            .child(
                text(if methods.is_empty() {
                    t("sourceFunds:actionsBar.assembleResultEmpty")
                } else {
                    methods.join(" · ")
                })
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
            );
        let manual = asm["awaiting_manual_review"].as_u64().unwrap_or(0) as usize;
        if manual > 0 {
            b = b.child(
                text(tc("sourceFunds:actionsBar.manualReviewHint", manual, &[]))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
            );
        }
        out = out.child(b);
    }
    // Open questions.
    let mut q = col()
        .w_full()
        .gap(tk.r(0.5))
        .role("region")
        .label(t("sourceFunds:case.findingsTitle"))
        .child(
            row()
                .w_full()
                .items_end()
                .justify_between()
                .child(
                    text(t("sourceFunds:journey.openQuestions").to_uppercase())
                        .font(tk.tsm(600.0).tracking(1.4))
                        .color(tk.muted_fg)
                        .role("heading")
                        .level(2),
                )
                .child(
                    text(tr(
                        "sourceFunds:case.findings",
                        &[
                            ("blockers", &blockers.len().to_string()),
                            ("warnings", &warnings.len().to_string()),
                        ],
                    ))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg),
                ),
        );
    if sof.preview.loading() {
        q = q.child(kit::empty_state(
            tk,
            t("sourceFunds:workstation.buildingReviewedFlow"),
        ));
    }
    if sof.preview.error() {
        q = q.child(gate_row(app, tk, 9999, &serde_json::json!({"code": "preview_unavailable", "message": t("sourceFunds:gates.previewUnavailable")}), false));
    }
    if !sof.preview.loading() && !sof.preview.error() && gaps.is_empty() && report.is_some() {
        let e = emerald();
        let (fill, border, ink) = kit::notice(tk, &e, e.c900, e.c200);
        q = q.child(
            kit::notice_box(tk, (fill, border, ink)).child(
                text(t("sourceFunds:case.noBlockers"))
                    .font(tk.tsm(400.0))
                    .color(ink)
                    .wrap(),
            ),
        );
    }
    let origins: Vec<&Value> = gaps
        .iter()
        .filter(|g| model::s(g, "code") == "missing_history")
        .collect();
    if origins.len() >= 2 {
        let total: f64 = origins
            .iter()
            .map(|g| model::f(g, "amount").unwrap_or(0.0))
            .sum();
        let asset = report
            .as_ref()
            .map(|r| model::s(&r["overview"], "target_asset").to_owned())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "BTC".into());
        q = q.child(
            text(tc(
                "sourceFunds:case.originsNeeded",
                origins.len(),
                &[("amount", &model::format_btc(Some(total), &asset))],
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap(),
        );
    }
    let cards: Vec<El> = gaps
        .iter()
        .enumerate()
        .map(|(i, g)| gate_row(app, tk, i, g, true))
        .collect();
    let ww = crate::reports::ledger::common::win_w(app, cx);
    q = q.child(kit::grid(
        tk,
        if ww >= 1280.0 { 2 } else { 1 },
        tk.r(0.5),
        cards,
    ));
    out = out.child(q);
    // The advanced review editor.
    let summary = tr(
        "sourceFunds:advancedReview.summary",
        &[
            ("links", &super::links(app).len().to_string()),
            ("sources", &super::sources(app).len().to_string()),
            ("evidence", &super::evidence(app).len().to_string()),
        ],
    );
    let body = if c.show_advanced {
        advanced(app, cx, tk, layer, ww)
    } else {
        el()
    };
    out.child(optional_section(
        tk,
        "sof-advanced",
        c.show_advanced,
        &I::SLIDERS_HORIZONTAL,
        &t("sourceFunds:advancedReview.title"),
        Some(summary),
        |a| case(a).show_advanced ^= true,
        body,
    ))
}

fn advanced(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>, ww: f32) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let assembling = sof.busy.contains("assemble");
    let reasm = kit::button(
        tk,
        "sof-reassemble",
        Variant::Outline,
        Size::Default,
        Some(&I::GIT_BRANCH),
        &t(if assembling {
            "sourceFunds:actionsBar.assembling"
        } else {
            "sourceFunds:actionsBar.reassemble"
        }),
        !c.selected_target().is_empty() && !assembling,
    );
    let reasm = if !assembling {
        reasm.on_press(|a: &mut App| super::run_assembly(a, true))
    } else {
        reasm
    };
    let gap = kit::button(
        tk,
        "sof-mark-gap",
        Variant::Outline,
        Size::Default,
        Some(&I::TRIANGLE_ALERT),
        &t("sourceFunds:actionsBar.markGap"),
        true,
    )
    .on_press(|a: &mut App| {
        super::prefill_gap(a, None);
        case(a).show_advanced = true;
    });
    let mut out = col()
        .w_full()
        .gap(tk.r(1.0))
        .child(row().gap(tk.r(0.5)).flex_wrap().child(reasm).child(gap));
    // `<details>` evidence and history graph (the case brief).
    let open = c.is_open("evidence");
    let mut d = col()
        .w_full()
        .padding(tk.r(0.75))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .child(kit::summary(
            tk,
            "sof-evidence-section",
            open,
            &t("sourceFunds:case.evidenceSection"),
            tk.tsm(500.0),
            tk.fg,
            |a| toggle_open(a, "evidence"),
        ));
    if open {
        d = d.child(col().w_full().pt(tk.r(0.75)).child(case_brief(app, cx, tk)));
    }
    out = out.child(d);
    let wide = ww >= 1536.0;
    let queue = review_queue(app, tk);
    let review = link_review(app, tk, layer);
    out = out.child(if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(col().grow(1.0).w(0.0).min_w(0.0).child(queue))
            .child(col().w(420.0).child(review))
    } else {
        col().w_full().gap(tk.r(1.0)).child(queue).child(review)
    });
    let source = source_card(app, tk, layer);
    let manual = manual_card(app, tk, layer);
    out.child(if wide {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(col().grow(1.0).w(0.0).min_w(0.0).child(source))
            .child(col().grow(1.0).w(0.0).min_w(0.0).child(manual))
    } else {
        col().w_full().gap(tk.r(1.0)).child(source).child(manual)
    })
}

/// `StatusPill`.
fn status_pill(tk: &Tk, state: &str) -> El {
    let (fill, border, ink) = match state {
        "reviewed" => {
            let e = emerald();
            kit::notice(tk, &e, e.c800, e.c200)
        }
        "rejected" => (tk.muted, tk.muted, tk.muted_fg),
        _ => {
            let a = amber();
            kit::notice(tk, &a, a.c900, a.c200)
        }
    };
    row()
        .px(tk.r(0.5))
        .py(2.0)
        .radius(999.0)
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .child(
            text(label_or_pretty("linkState", state))
                .font(tk.txs(400.0))
                .color(ink),
        )
}

fn review_queue(app: &App, tk: &Tk) -> El {
    let links = super::links(app);
    let selected = super::selected_link(app)
        .map(|l| model::s(&l, "id").to_owned())
        .unwrap_or_default();
    let target = super::selected_tx_id(app);
    let hidden = app.prefs.hide_sensitive();
    let mut body = col().w_full().gap(tk.r(0.75));
    if links.is_empty() {
        body = body.child(kit::empty_state(
            tk,
            t("sourceFunds:workstation.noMatchedLinksYetAssemble"),
        ));
    }
    for l in &links {
        let id = model::s(l, "id").to_owned();
        let active = id == selected;
        let from = if !model::s(l, "from_source_id").is_empty() {
            super::source_name(app, model::s(l, "from_source_id"))
        } else {
            super::tx_name(app, model::s(l, "from_transaction_id"))
        };
        let to = super::tx_name(app, model::s(l, "to_transaction_id"));
        let badge = t(if model::s(l, "to_transaction_id") == target {
            "sourceFunds:reviewQueue.badge.target"
        } else {
            "sourceFunds:reviewQueue.badge.path"
        });
        let pick = id.clone();
        body = body.child(
            col()
                .key(format!("sof-link-{id}"))
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, if active { tk.primary } else { tk.border })
                .border_layout()
                .fill(if active {
                    tk.primary.alpha(0.05)
                } else {
                    Color::TRANSPARENT
                })
                .hover(if active {
                    tk.primary.alpha(0.05)
                } else {
                    tk.muted.alpha(0.6)
                })
                .role("button")
                .label(format!(
                    "{} {from} -> {to}",
                    super::link_type_label(model::s(l, "link_type"))
                ))
                .checked(active)
                .focusable(true)
                .on_press(move |a: &mut App| case(a).selected_link = pick.clone())
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .flex_wrap()
                        .child(status_pill(tk, model::s(l, "state")))
                        .child(
                            row()
                                .px(tk.r(0.5))
                                .py(2.0)
                                .radius(tk.r(0.375))
                                .fill(tk.muted)
                                .child(text(badge).font(tk.txs(400.0)).color(tk.muted_fg)),
                        )
                        .child(
                            text(super::link_type_label(model::s(l, "link_type")))
                                .font(tk.tsm(500.0))
                                .color(tk.fg),
                        )
                        .child(
                            text(method_label(model::s(l, "method")))
                                .font(tk.tsm(400.0))
                                .color(tk.muted_fg),
                        ),
                )
                .child(
                    col()
                        .mt(tk.r(0.5))
                        .gap(tk.r(0.25))
                        .child(
                            text(format!("{from} -> {to}"))
                                .font(tk.txs(400.0))
                                .color(tk.muted_fg)
                                .wrap_anywhere(),
                        )
                        .child(
                            row()
                                .child(crate::ui::sensitive(
                                    model::format_btc(
                                        model::f(l, "allocation_amount"),
                                        model::s(l, "asset"),
                                    ),
                                    tk.txs(400.0),
                                    tk.muted_fg,
                                    hidden,
                                ))
                                .child(
                                    text(format!(
                                        " · {}",
                                        label_or_pretty("confidence", model::s(l, "confidence"))
                                    ))
                                    .font(tk.txs(400.0))
                                    .color(tk.muted_fg),
                                ),
                        ),
                ),
        );
    }
    kit::card(
        tk,
        "sof-queue",
        Some(&I::LINK_2),
        &t("sourceFunds:reviewQueue.title"),
        Some(t("sourceFunds:reviewQueue.description")),
        body,
    )
}

fn opts(values: &[&str]) -> Vec<(String, String, bool)> {
    values
        .iter()
        .map(|v| ((*v).to_owned(), model::pretty(v), false))
        .collect()
}

fn evidence_options(app: &App) -> Vec<(String, String, bool)> {
    let mut o = vec![(
        NO_ATTACHMENT.to_owned(),
        t("sourceFunds:select.noAttachment"),
        false,
    )];
    for e in super::evidence(app) {
        let label = [
            model::s(&e, "label"),
            model::s(&e, "wallet"),
            model::s(&e, "external_id"),
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
        o.push((model::s(&e, "id").to_owned(), label, false));
    }
    o
}

/// `TransactionSelect`: the loaded rows by label, optionally "Use selected target" first.
fn tx_options(app: &App, default_label: Option<String>) -> Vec<(String, String, bool)> {
    let mut o = vec![];
    if let Some(d) = default_label {
        o.push((String::new(), d, false));
    }
    if let Some(c) = &app.provenance.sof.case {
        for r in &c.pages {
            o.push((model::tx_ref(r), model::tx_label(r), false));
        }
    }
    o
}

fn link_review(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let body = match super::selected_link(app) {
        None => kit::empty_state(tk, t("sourceFunds:linkReview.empty")),
        Some(l) => {
            let from = if !model::s(&l, "from_source_id").is_empty() {
                super::sources(app)
                    .iter()
                    .find(|s| model::s(s, "id") == model::s(&l, "from_source_id"))
                    .map(|s| model::s(s, "label").to_owned())
                    .unwrap_or_else(|| super::tx_name(app, model::s(&l, "from_transaction_id")))
            } else {
                super::tx_name(app, model::s(&l, "from_transaction_id"))
            };
            let busy = sof.busy.contains("review") || sof.busy.contains("attach");
            let h = tk.r(2.5);
            let fields = vec![
                kit::field(
                    tk,
                    &t("sourceFunds:manualLink.type"),
                    kit::select(
                        app,
                        tk,
                        layer,
                        "rv-type",
                        &t("sourceFunds:manualLink.type"),
                        opts(LINK_TYPES),
                        &c.link.link_type,
                        "",
                        h,
                        |a, v| case(a).link.link_type = v.to_owned(),
                    ),
                ),
                kit::field(
                    tk,
                    &t("sourceFunds:manualLink.confidence"),
                    kit::select(
                        app,
                        tk,
                        layer,
                        "rv-conf",
                        &t("sourceFunds:manualLink.confidence"),
                        opts(CONFIDENCE_LEVELS),
                        &c.link.confidence,
                        "",
                        h,
                        |a, v| case(a).link.confidence = v.to_owned(),
                    ),
                ),
                kit::field(
                    tk,
                    &t("sourceFunds:manualLink.allocation"),
                    kit::input(
                        tk,
                        "sof-rv-alloc",
                        &c.link.allocation,
                        &t("sourceFunds:manualLink.allocation"),
                        "",
                        |a| &mut case(a).link.allocation,
                        tk.r(2.25),
                    ),
                ),
                kit::field(
                    tk,
                    &t("sourceFunds:manualLink.fromAmount"),
                    kit::input(
                        tk,
                        "sof-rv-from",
                        &c.link.from_allocation,
                        &t("sourceFunds:manualLink.fromAmount"),
                        "",
                        |a| &mut case(a).link.from_allocation,
                        tk.r(2.25),
                    ),
                ),
            ];
            let accept = kit::button(
                tk,
                "sof-rv-accept",
                Variant::Default,
                Size::Default,
                Some(&I::CHECK),
                &t("sourceFunds:linkReview.accept"),
                !busy,
            );
            let reject = kit::button(
                tk,
                "sof-rv-reject",
                Variant::Outline,
                Size::Default,
                Some(&I::X),
                &t("sourceFunds:linkReview.reject"),
                !sof.busy.contains("review"),
            );
            let accept = if !busy {
                accept.on_press(|a: &mut App| super::review_link(a, true))
            } else {
                accept
            };
            let reject = if !sof.busy.contains("review") {
                reject.on_press(|a: &mut App| super::review_link(a, false))
            } else {
                reject
            };
            col()
                .w_full()
                .gap(tk.r(0.75))
                .child(
                    kit::bordered(tk)
                        .padding(tk.r(0.75))
                        .child(text(from).font(tk.tsm(500.0)).color(tk.fg).wrap_anywhere())
                        .child(
                            text(format!(
                                "to {}",
                                super::tx_name(app, model::s(&l, "to_transaction_id"))
                            ))
                            .font(tk.tsm(400.0))
                            .color(tk.muted_fg)
                            .wrap_anywhere(),
                        ),
                )
                .child(kit::grid(tk, 2, tk.r(0.75), fields))
                .child(kit::field(
                    tk,
                    &t("sourceFunds:evidence.label"),
                    kit::select(
                        app,
                        tk,
                        layer,
                        "rv-evidence",
                        &t("sourceFunds:evidence.label"),
                        evidence_options(app),
                        &c.link.attachment_id,
                        "",
                        h,
                        |a, v| case(a).link.attachment_id = v.to_owned(),
                    ),
                ))
                .child(kit::field(
                    tk,
                    &t("sourceFunds:manualLink.reviewNote"),
                    kit::textarea(
                        tk,
                        "sof-rv-note",
                        &c.link.explanation,
                        &t("sourceFunds:manualLink.reviewNote"),
                        |a| &mut case(a).link.explanation,
                    ),
                ))
                .child(kit::grid(tk, 2, tk.r(0.5), vec![accept, reject]))
        }
    };
    kit::card(
        tk,
        "sof-link-review",
        Some(&I::FILE_CHECK),
        &t("sourceFunds:linkReview.title"),
        Some(t("sourceFunds:linkReview.description")),
        body,
    )
}

fn source_card(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let s = &c.source;
    let h = tk.r(2.5);
    let fields = vec![
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.sourceType"),
            kit::select(
                app,
                tk,
                layer,
                "src-type",
                &t("sourceFunds:sourceOrGap.sourceType"),
                opts(SOURCE_TYPES),
                &s.source_type,
                "",
                h,
                |a, v| {
                    let s = &mut case(a).source;
                    s.source_type = v.to_owned();
                    s.link_type = if v == "missing_history" {
                        "missing_history".into()
                    } else if s.link_type == "missing_history" {
                        "manual_source".into()
                    } else {
                        s.link_type.clone()
                    };
                },
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.linkType"),
            kit::select(
                app,
                tk,
                layer,
                "src-link",
                &t("sourceFunds:sourceOrGap.linkType"),
                opts(LINK_TYPES),
                &s.link_type,
                "",
                h,
                |a, v| case(a).source.link_type = v.to_owned(),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.label"),
            kit::input(
                tk,
                "sof-src-label",
                &s.label,
                &t("sourceFunds:sourceOrGap.label"),
                "",
                |a| &mut case(a).source.label,
                tk.r(2.25),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.amount"),
            kit::input(
                tk,
                "sof-src-amount",
                &s.amount,
                &t("sourceFunds:sourceOrGap.amount"),
                "",
                |a| &mut case(a).source.amount,
                tk.r(2.25),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.asset"),
            kit::input(
                tk,
                "sof-src-asset",
                &s.asset,
                &t("sourceFunds:sourceOrGap.asset"),
                "",
                |a| &mut case(a).source.asset,
                tk.r(2.25),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.appliesTo"),
            kit::select(
                app,
                tk,
                layer,
                "src-to",
                &t("sourceFunds:sourceOrGap.appliesTo"),
                tx_options(app, Some(t("sourceFunds:case.useTarget"))),
                &s.to_transaction,
                &t("sourceFunds:select.transaction"),
                h,
                |a, v| case(a).source.to_transaction = v.to_owned(),
            ),
        ),
    ];
    let busy = sof.busy.contains("create_source") || sof.busy.contains("create_link");
    let enabled = !busy && !s.label.text.trim().is_empty() && !s.amount.text.trim().is_empty();
    let create = kit::button(
        tk,
        "sof-src-create",
        Variant::Default,
        Size::Default,
        Some(&I::PLUS),
        &t("sourceFunds:sourceOrGap.create"),
        enabled,
    )
    .w_full();
    let create = if enabled {
        create.on_press(super::create_source_link)
    } else {
        create
    };
    let body = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(kit::grid(tk, 2, tk.r(0.75), fields))
        .child(kit::field(
            tk,
            &t("sourceFunds:evidence.label"),
            kit::select(
                app,
                tk,
                layer,
                "src-evidence",
                &t("sourceFunds:evidence.label"),
                evidence_options(app),
                &s.attachment_id,
                "",
                h,
                |a, v| case(a).source.attachment_id = v.to_owned(),
            ),
        ))
        .child(kit::field(
            tk,
            &t("sourceFunds:sourceOrGap.evidenceNote"),
            kit::textarea(
                tk,
                "sof-src-note",
                &s.description,
                &t("sourceFunds:sourceOrGap.evidenceNote"),
                |a| &mut case(a).source.description,
            ),
        ))
        .child(create);
    kit::card(
        tk,
        "sof-source",
        Some(&I::PLUS),
        &t("sourceFunds:sourceOrGap.title"),
        Some(t("sourceFunds:sourceOrGap.description")),
        body,
    )
}

fn manual_card(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let m = &c.manual;
    let h = tk.r(2.5);
    let to_value = m.to_transaction.clone();
    let fields = vec![
        kit::field(
            tk,
            &t("sourceFunds:manualLink.from"),
            kit::select(
                app,
                tk,
                layer,
                "ml-from",
                &t("sourceFunds:manualLink.from"),
                tx_options(app, None),
                &m.from_transaction,
                &t("sourceFunds:select.transaction"),
                h,
                |a, v| case(a).manual.from_transaction = v.to_owned(),
            ),
        ),
        kit::field(
            tk,
            "To",
            kit::select(
                app,
                tk,
                layer,
                "ml-to",
                "To",
                tx_options(app, Some(t("sourceFunds:case.useTarget"))),
                &to_value,
                &t("sourceFunds:select.transaction"),
                h,
                |a, v| case(a).manual.to_transaction = v.to_owned(),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:manualLink.type"),
            kit::select(
                app,
                tk,
                layer,
                "ml-type",
                &t("sourceFunds:manualLink.type"),
                opts(LINK_TYPES),
                &m.link_type,
                "",
                h,
                |a, v| case(a).manual.link_type = v.to_owned(),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:manualLink.confidence"),
            kit::select(
                app,
                tk,
                layer,
                "ml-conf",
                &t("sourceFunds:manualLink.confidence"),
                opts(CONFIDENCE_LEVELS),
                &m.confidence,
                "",
                h,
                |a, v| case(a).manual.confidence = v.to_owned(),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:manualLink.allocation"),
            kit::input(
                tk,
                "sof-ml-alloc",
                &m.allocation,
                &t("sourceFunds:manualLink.allocation"),
                "",
                |a| &mut case(a).manual.allocation,
                tk.r(2.25),
            ),
        ),
        kit::field(
            tk,
            &t("sourceFunds:manualLink.fromAmount"),
            kit::input(
                tk,
                "sof-ml-from-amount",
                &m.from_allocation,
                &t("sourceFunds:manualLink.fromAmount"),
                "",
                |a| &mut case(a).manual.from_allocation,
                tk.r(2.25),
            ),
        ),
    ];
    let enabled = !sof.busy.contains("create_link")
        && !m.from_transaction.is_empty()
        && !m.allocation.text.trim().is_empty();
    let add = kit::button(
        tk,
        "sof-ml-add",
        Variant::Default,
        Size::Default,
        Some(&I::PLUS),
        &t("sourceFunds:manualLink.add"),
        enabled,
    )
    .w_full();
    let add = if enabled {
        add.on_press(super::create_manual_link)
    } else {
        add
    };
    let body = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(kit::grid(tk, 2, tk.r(0.75), fields))
        .child(kit::field(
            tk,
            &t("sourceFunds:evidence.label"),
            kit::select(
                app,
                tk,
                layer,
                "ml-evidence",
                &t("sourceFunds:evidence.label"),
                evidence_options(app),
                &m.attachment_id,
                "",
                h,
                |a, v| case(a).manual.attachment_id = v.to_owned(),
            ),
        ))
        .child(kit::field(
            tk,
            &t("sourceFunds:manualLink.reviewNote"),
            kit::textarea(
                tk,
                "sof-ml-note",
                &m.explanation,
                &t("sourceFunds:manualLink.reviewNote"),
                |a| &mut case(a).manual.explanation,
            ),
        ))
        .child(add);
    kit::card(
        tk,
        "sof-manual",
        Some(&I::LINK_2),
        &t("sourceFunds:manualLink.title"),
        Some(t("sourceFunds:manualLink.description")),
        body,
    )
}

/// `CaseBrief`: target, metrics, narrative, source mix, data sources and the route.
fn case_brief(app: &App, cx: &mut Cx<App>, tk: &Tk) -> El {
    let report = super::report(app).cloned().unwrap_or(Value::Null);
    let ov = &report["overview"];
    let asset = [
        model::s(ov, "target_asset"),
        model::s(&report["target"], "asset"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .unwrap_or("BTC")
    .to_owned();
    let hidden = app.prefs.hide_sensitive();
    let title = [
        model::s(ov, "target_label"),
        model::s(&report["target"], "label"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .map(str::to_owned)
    .unwrap_or_else(|| t("sourceFunds:caseBrief.fallbackTarget"));
    let wallet = [
        model::s(ov, "target_wallet"),
        model::s(&report["target"], "wallet"),
    ]
    .into_iter()
    .find(|s| !s.is_empty())
    .map(str::to_owned)
    .unwrap_or_else(|| t("sourceFunds:caseBrief.fallbackWallet"));
    let ctx = &report["report_context"];
    let juris: Vec<&str> = [
        model::s(ctx, "jurisdiction_label"),
        model::s(ctx, "fiat_currency"),
    ]
    .into_iter()
    .filter(|s| !s.is_empty())
    .collect();
    let amount =
        model::f(ov, "target_amount").or_else(|| model::f(&report["target"], "required_amount"));
    let mut sub = row()
        .flex_wrap()
        .child(
            text(format!(
                "{} · {} · ",
                model::format_datetime(model::s(ov, "target_date")),
                wallet
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg),
        )
        .child(crate::ui::sensitive(
            model::format_btc(amount, &asset),
            tk.tsm(400.0),
            tk.muted_fg,
            hidden,
        ));
    if !juris.is_empty() {
        sub = sub.child(
            text(format!(" · {}", juris.join(" / ")))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg),
        );
    }
    let head = row()
        .w_full()
        .gap(tk.r(0.5))
        .items_start()
        .justify_between()
        .child(
            col()
                .min_w(0.0)
                .shrink()
                .child(
                    text(title)
                        .font(tk.tbase(600.0))
                        .color(tk.fg)
                        .wrap_anywhere()
                        .role("heading")
                        .level(2),
                )
                .child(sub),
        )
        .child(
            text(t(if super::exportable(app) {
                "sourceFunds:case.exportable"
            } else {
                "sourceFunds:case.needsEvidence"
            }))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    let metric = |label: String, v: u64| {
        col()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .child(text(label).font(tk.txs(400.0)).color(tk.muted_fg))
            .child(
                text(crate::format::count(v as usize))
                    .font(kit::tnum(tk.tlg(600.0).mono()))
                    .color(tk.fg)
                    .mt(tk.r(0.25)),
            )
    };
    let n = |k: &str| ov[k].as_u64().unwrap_or(0);
    let manual = super::links(app)
        .iter()
        .filter(|l| {
            model::s(l, "state") == "suggested"
                && !super::bulk_eligible(app).contains(&model::s(l, "id").to_owned())
        })
        .count();
    let metrics = vec![
        metric(
            t("sourceFunds:caseBrief.metric.transactions"),
            n("transaction_count"),
        ),
        metric(
            t("sourceFunds:caseBrief.metric.reviewedLinks"),
            n("link_count"),
        ),
        metric(
            t("sourceFunds:caseBrief.metric.sources"),
            n("source_category_count"),
        ),
        metric(
            t("sourceFunds:caseBrief.metric.blockers"),
            n("blocker_count"),
        ),
        metric(
            t("sourceFunds:caseBrief.metric.batchable"),
            super::bulk_eligible(app).len() as u64,
        ),
    ];
    let mut narrative = col().w_full().gap(tk.r(0.5));
    let paragraphs = model::arr(&report["narrative"], "paragraphs");
    if paragraphs.is_empty() {
        narrative = narrative.child(
            text(t("sourceFunds:workstation.localPreviewDataIsNot"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    for p in paragraphs.iter().take(3) {
        narrative = narrative.child(
            text(p.as_str().unwrap_or("").to_owned())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    if manual > 0 {
        narrative = narrative.child(
            text(tc("sourceFunds:caseBrief.manualReview", manual, &[]))
                .font(tk.txs(400.0))
                .color(kit::ink(tk, &amber()))
                .wrap(),
        );
    }
    let beyond = super::context(app)
        .and_then(|c| c["bulk_review"]["eligible_beyond_inspection"].as_u64())
        .unwrap_or(0) as usize;
    if beyond > 0 {
        narrative = narrative.child(
            text(tc(
                "sourceFunds:caseBrief.batchableBeyondInspection",
                beyond,
                &[],
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg)
            .wrap(),
        );
    }
    let mut mix = kit::bordered(tk).fill(tk.background);
    let rows = model::arr(&report, "source_mix");
    let unresolved = [serde_json::json!({"source_type": "unresolved", "amount": 0.0, "count": 0})];
    let src = if rows.is_empty() {
        &unresolved[..]
    } else {
        rows
    };
    for (i, s) in src.iter().enumerate() {
        let a = {
            let x = model::s(s, "asset");
            if x.is_empty() {
                asset.clone()
            } else {
                x.to_owned()
            }
        };
        let mut r = row()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.5))
            .gap(tk.r(0.75))
            .center_y()
            .justify_between()
            .child(
                text(model::pretty(model::s(s, "source_type")))
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .truncate()
                    .shrink(),
            )
            .child(crate::ui::sensitive(
                model::format_btc(model::f(s, "amount"), &a),
                kit::tnum(tk.txs(400.0).mono()),
                tk.fg,
                hidden,
            ));
        if i + 1 < src.len() {
            r = r
                .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                .border_layout();
        }
        mix = mix.child(r);
    }
    let ds = model::arr(&report, "data_sources");
    let mut chips = row().gap(tk.r(0.5)).flex_wrap();
    for d in ds.iter().take(4) {
        let n =
            d["transaction_count"].as_u64().unwrap_or(0) + d["source_count"].as_u64().unwrap_or(0);
        chips = chips.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .radius(999.0)
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .child(
                    text(format!("{} · {n}", model::s(d, "label")))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    if ds.len() > 4 {
        chips = chips.child(
            row()
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .radius(999.0)
                .border(1.0, tk.border)
                .border_layout()
                .fill(tk.background)
                .child(
                    text(format!("+{}", ds.len() - 4))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let ww = crate::reports::ledger::common::win_w(app, cx);
    let side = col().gap(tk.r(0.5)).child(mix).child(chips);
    let lower = if ww >= 1280.0 {
        row()
            .w_full()
            .gap(tk.r(1.0))
            .items_start()
            .child(narrative.grow(1.0).w(0.0))
            .child(side.w(340.0))
    } else {
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(narrative)
            .child(side.w_full())
    };
    let mut out = col()
        .w_full()
        .gap(tk.r(1.0))
        .padding(tk.r(1.0))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .fill(tk.muted.alpha(0.2))
        .child(head)
        .child(kit::grid(
            tk,
            if ww >= 768.0 { 5 } else { 1 },
            tk.r(0.75),
            metrics,
        ))
        .child(lower);
    if let Some(f) = flow_preview(app, cx, tk, "brief", &report["simplified_flow"], true) {
        out = out.child(f);
    }
    out
}
