//! Stage 3, `DiscloseStage` (what you share: reveal mode, summary, the route, references,
//! per-transaction overrides, evidence, exclusions, level details, report options, recipient
//! and the original PDF graphics) and stage 4, `ExportStage` (save the frozen case, export the
//! PDF or the evidence bundle).

use rune::{Color, Cx, El, col, el, row, text};
use serde_json::Value;

use super::kit::{self, amber, emerald, rose};
use super::model::{self, REVEAL_MODES};
use super::target::{stage_header, toggle_open};
use crate::app::App;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{Size, Variant};
use crate::ui::ic;

fn case(a: &mut App) -> &mut super::Case {
    a.provenance.sof.case.as_mut().expect("case")
}

fn reveal_label(m: &str) -> String {
    let k = format!("sourceFunds:reveal.{m}");
    if crate::i18n::has(&k) {
        t(&k)
    } else {
        m.to_owned()
    }
}

fn details(
    app: &App,
    tk: &Tk,
    key: &str,
    label: &str,
    body: impl FnOnce() -> El,
    last: bool,
) -> El {
    let open = app
        .provenance
        .sof
        .case
        .as_ref()
        .is_some_and(|c| c.is_open(key));
    let k = key.to_owned();
    let mut d = col().w_full().py(tk.r(1.0)).child(kit::summary(
        tk,
        &format!("sof-{key}"),
        open,
        label,
        tk.tsm(500.0),
        tk.fg,
        move |a| toggle_open(a, &k),
    ));
    if open {
        d = d.child(body().pt(tk.r(1.0)));
    }
    if !last {
        d = d
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout();
    }
    d
}

pub fn disclose(app: &App, cx: &mut Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> El {
    let c = app.provenance.sof.case.as_ref().expect("case");
    let report = super::report(app).cloned();
    let r = report.clone().unwrap_or(Value::Null);
    let reveal_opts: Vec<(String, String, bool)> = REVEAL_MODES
        .iter()
        .map(|m| ((*m).to_owned(), reveal_label(m), false))
        .collect();
    let rl = t("sourceFunds:disclosure.revealMode");
    let head = row()
        .w_full()
        .gap(tk.r(1.0))
        .items_end()
        .justify_between()
        .flex_wrap()
        .child(
            text(t("sourceFunds:review.title"))
                .font(tk.tlg(600.0))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(col().w(tk.r(11.0)).child(kit::field(
            tk,
            &rl,
            kit::select(
                app,
                tk,
                layer,
                "reveal",
                &rl,
                reveal_opts,
                &c.reveal_mode,
                "",
                tk.r(2.25),
                |a, v| case(a).reveal_mode = v.to_owned(),
            ),
        )));
    let mut out = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.25))
        .child(head)
        .child(summary(app, tk, &r));
    if let Some(f) =
        super::trace::flow_preview(app, cx, tk, "disclose", &r["simplified_flow"], false)
    {
        out = out.child(f);
    }
    let preview = &r["disclosure_preview"];
    // Keyed: the flow preview above comes and goes while an option reloads the report, and an
    // unkeyed column would remount here, taking the focused select with it.
    let mut sections = col()
        .key("sof-disclose-sections")
        .w_full()
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout();
    let has_svg = !model::s(super::diagrams(app).unwrap_or(&Value::Null), "flow_svg").is_empty();
    sections = sections.child(details(
        app,
        tk,
        "preview-details",
        &t("sourceFunds:journey.previewDetails"),
        || {
            let mut b = col()
                .w_full()
                .gap(tk.r(1.0))
                .child(txid_list(tk, preview))
                .child(node_overrides(app, tk, &r));
            let att: Vec<String> = model::arr(preview, "attachments")
                .iter()
                .map(|a| model::s(a, "label").to_owned())
                .collect();
            b = b.child(disclosure_list(tk, &t("sourceFunds:evidence.label"), &att));
            let excl: Vec<String> = model::arr(preview, "excluded")
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            b = b.child(disclosure_list(
                tk,
                &t("sourceFunds:disclosure.excluded"),
                &excl,
            ));
            let pn = model::s(preview, "privacy_note");
            if !pn.is_empty() {
                b = b.child(
                    kit::bordered(tk).px(tk.r(0.75)).py(tk.r(0.5)).child(
                        text(pn.to_owned())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg)
                            .wrap(),
                    ),
                );
            }
            let on = model::s(preview, "ownership_note");
            if !on.is_empty() {
                b = b.child(
                    text(on.to_owned())
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                );
            }
            if let Some(l) = level_details(
                app,
                tk,
                &r,
                c.omit_sections.iter().any(|s| s == "transaction_details"),
            ) {
                b = b.child(l);
            }
            b
        },
        false,
    ));
    sections = sections.child(details(
        app,
        tk,
        "report-options",
        &t("sourceFunds:journey.reportOptions"),
        || report_options(app, tk, layer),
        !has_svg,
    ));
    if has_svg {
        let d = super::diagrams(app).cloned().unwrap_or(Value::Null);
        let ww = cx
            .layout_size("sof-stage-disclose")
            .map(|[w, _]| w)
            .unwrap_or(1000.0)
            - 2.0 * tk.card_pad();
        sections = sections.child(details(
            app,
            tk,
            "print-graphics",
            &t("sourceFunds:review.printGraphics"),
            || {
                let half = (ww - tk.r(1.0)) / 2.0;
                col()
                    .w_full()
                    .gap(tk.r(1.0))
                    .child(diagram(
                        app,
                        cx,
                        tk,
                        model::s(&d, "flow_svg"),
                        &t("sourceFunds:flowPath.title"),
                        ww,
                        true,
                    ))
                    .child(
                        row()
                            .w_full()
                            .gap(tk.r(1.0))
                            .items_start()
                            .child(col().grow(1.0).w(0.0).min_w(0.0).child(diagram(
                                app,
                                cx,
                                tk,
                                model::s(&d, "source_mix_ring_svg"),
                                &t("sourceFunds:workstation.sourceMix"),
                                half,
                                false,
                            )))
                            .child(col().grow(1.0).w(0.0).min_w(0.0).child(diagram(
                                app,
                                cx,
                                tk,
                                model::s(&d, "data_source_ring_svg"),
                                &t("sourceFunds:workstation.dataSources"),
                                half,
                                false,
                            ))),
                    )
            },
            true,
        ));
    }
    out.child(sections)
}

/// `DisclosureSummary`.
fn summary(app: &App, tk: &Tk, r: &Value) -> El {
    let p = &r["disclosure_preview"];
    let wallets: Vec<String> = match p.get("wallets_named").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        None => model::unique_sorted(
            model::arr(&r["graph"], "nodes")
                .iter()
                .map(|n| model::s(n, "wallet").to_owned()),
        ),
    };
    let _ = app;
    let mut out = col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(tr(
                "sourceFunds:review.contents",
                &[
                    ("references", &model::arr(p, "txids").len().to_string()),
                    ("evidence", &model::arr(p, "attachments").len().to_string()),
                ],
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap(),
        )
        .child(
            row()
                .flex_wrap()
                .child(
                    text(format!("{}: ", t("sourceFunds:disclosure.walletLabels")))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg),
                )
                .child(
                    text(if wallets.is_empty() {
                        t("sourceFunds:disclosure.none")
                    } else {
                        wallets.join(", ")
                    })
                    .font(tk.tsm(400.0))
                    .color(tk.fg)
                    .wrap()
                    .shrink(),
                ),
        );
    if !model::s(p, "ownership_note").is_empty() {
        out = out.child(
            text(t("sourceFunds:review.ownership"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        );
    }
    out
}

/// `DisclosureTxidList`: every disclosed reference; an explorer link opens only on a press.
fn txid_list(tk: &Tk, p: &Value) -> El {
    let txids: Vec<String> = model::arr(p, "txids")
        .iter()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    let links = model::arr(p, "explorer_links");
    let mut list = col().w_full().gap(tk.r(0.25));
    if txids.is_empty() {
        list = list.child(
            kit::bordered(tk).px(tk.r(0.75)).py(tk.r(0.5)).child(
                text(t("sourceFunds:disclosure.none"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    for (i, txid) in txids.iter().enumerate() {
        let link = links.iter().find(|l| model::s(l, "txid") == txid);
        let right = match link {
            Some(l) => {
                let url = model::s(l, "url").to_owned();
                let label = model::s(l, "label").to_owned();
                kit::button(
                    tk,
                    format!("sof-explorer-{i}"),
                    Variant::Outline,
                    Size::Sm,
                    Some(&I::EXTERNAL_LINK),
                    &label,
                    true,
                )
                .help(tr(
                    "sourceFunds:disclosure.openTitle",
                    &[("txid", txid), ("label", &label)],
                ))
                .on_press(move |a: &mut App| {
                    // Opening the user's explorer is egress the user asked for with this press.
                    // Only an http(s) link opens (`openExternalUrl`'s check).
                    let ok = crate::txdetail::actions::external_url(&url)
                        .is_ok_and(|u| crate::txdetail::open_external(&u));
                    if !ok {
                        a.toast(
                            crate::app::ToastTone::Error,
                            &t("sourceFunds:disclosure.openError"),
                            None,
                            Some("sof"),
                        );
                    }
                })
            }
            None => text(t("sourceFunds:disclosure.noExplorerLink"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        };
        list = list.child(
            kit::bordered(tk).px(tk.r(0.75)).py(tk.r(0.5)).child(
                row()
                    .w_full()
                    .gap(tk.r(0.5))
                    .center_y()
                    .justify_between()
                    .child(
                        text(txid.clone())
                            .font(tk.txs(400.0).mono())
                            .color(tk.fg)
                            .wrap_anywhere()
                            .shrink(),
                    )
                    .child(right),
            ),
        );
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(t("sourceFunds:disclosure.txids"))
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(list)
}

/// `DisclosureNodeOverrides`: Show / Hide per disclosed transaction.
fn node_overrides(app: &App, tk: &Tk, r: &Value) -> El {
    let nodes: Vec<&Value> = model::arr(&r["graph"], "nodes")
        .iter()
        .filter(|n| model::s(n, "node_type") == "transaction")
        .collect();
    if nodes.is_empty() {
        return el();
    }
    let c = app.provenance.sof.case.as_ref().expect("case");
    let mut list = col().w_full().gap(tk.r(0.25));
    for n in nodes {
        let id = model::s(n, "transaction_id").to_owned();
        if id.is_empty() {
            continue;
        }
        let ext = model::s(n, "external_id");
        let decision = c.reveal_overrides.get(&id).cloned();
        let btn = |tone: &str, label: String| {
            let active = decision.as_deref() == Some(tone);
            let (fill, border, ink) = match (active, tone) {
                (true, "show") => (
                    if tk.dark {
                        emerald().c950.alpha(0.4)
                    } else {
                        emerald().c50
                    },
                    emerald().c500,
                    kit::ink(tk, &emerald()),
                ),
                (true, _) => (
                    if tk.dark {
                        rose().c950.alpha(0.4)
                    } else {
                        rose().c50
                    },
                    rose().c500,
                    kit::ink(tk, &rose()),
                ),
                _ => (Color::TRANSPARENT, tk.border, tk.muted_fg),
            };
            let id2 = id.clone();
            let tone2 = tone.to_owned();
            row()
                .key(format!("sof-ovr-{tone}-{id}"))
                .px(tk.r(0.5))
                .py(2.0)
                .radius(tk.r(0.375))
                .fill(fill)
                .border(1.0, border)
                .border_layout()
                .hover(if active { fill } else { tk.muted.alpha(0.5) })
                .role("button")
                .label(format!("{label} {}", model::s(n, "label")))
                .checked(active)
                .focusable(true)
                .on_press(move |a: &mut App| {
                    let o = &mut case(a).reveal_overrides;
                    if o.get(&id2).map(String::as_str) == Some(tone2.as_str()) {
                        o.remove(&id2);
                    } else {
                        o.insert(id2.clone(), tone2.clone());
                    }
                })
                .child(text(label).font(tk.txs(400.0)).color(ink))
        };
        let label = {
            let l = model::s(n, "label");
            if l.is_empty() {
                id.clone()
            } else {
                l.to_owned()
            }
        };
        list = list.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .px(tk.r(0.5))
                .py(tk.r(0.25))
                .center_y()
                .justify_between()
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .border_layout()
                .child(
                    col()
                        .min_w(0.0)
                        .shrink()
                        .child(text(label).font(tk.txs(400.0)).color(tk.fg).truncate())
                        .child(
                            text(if ext.is_empty() {
                                "(redacted)".to_owned()
                            } else {
                                model::short_id(ext)
                            })
                            .font(tk.t2xs(400.0).mono())
                            .color(tk.muted_fg)
                            .truncate(),
                        ),
                )
                .child(
                    row()
                        .gap(tk.r(0.25))
                        .child(btn("show", t("sourceFunds:workstation.show")))
                        .child(btn("hide", t("sourceFunds:workstation.hide"))),
                ),
        );
    }
    col()
        .w_full()
        .gap(tk.r(0.25))
        .child(
            text(t("sourceFunds:workstation.pertransactionDisclosure").to_uppercase())
                .font(tk.txs(500.0).tracking(0.3))
                .color(tk.muted_fg),
        )
        .child(
            text(t("sourceFunds:workstation.overrideTheRevealModeFor"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(list)
}

/// `DisclosureList`.
fn disclosure_list(tk: &Tk, label: &str, values: &[String]) -> El {
    let mut list = col().w_full().gap(tk.r(0.25));
    if values.is_empty() {
        list = list.child(
            kit::bordered(tk).px(tk.r(0.75)).py(tk.r(0.5)).child(
                text(t("sourceFunds:disclosure.none"))
                    .font(tk.tsm(400.0))
                    .color(tk.muted_fg),
            ),
        );
    }
    for v in values {
        list = list.child(
            kit::bordered(tk).px(tk.r(0.75)).py(tk.r(0.5)).child(
                text(v.clone())
                    .font(tk.txs(400.0).mono())
                    .color(tk.fg)
                    .wrap_anywhere(),
            ),
        );
    }
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(
            text(label.to_owned())
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(2),
        )
        .child(list)
}

/// `FlowLevelDetailPreview`.
fn level_details(app: &App, tk: &Tk, r: &Value, omitted: bool) -> Option<El> {
    let levels = model::arr(r, "flow_levels");
    if levels.is_empty() {
        return None;
    }
    let hidden = app.prefs.hide_sensitive();
    let mut head = row()
        .w_full()
        .gap(tk.r(0.5))
        .center_y()
        .justify_between()
        .child(
            text(t("sourceFunds:workstation.transactionDetailsByLevel"))
                .font(tk.tsm(600.0))
                .color(tk.fg)
                .role("heading")
                .level(3),
        );
    if omitted {
        head = head.child(
            row()
                .px(tk.r(0.5))
                .py(2.0)
                .radius(999.0)
                .border(1.0, tk.border)
                .border_layout()
                .child(
                    text(t("sourceFunds:case.omitted"))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                ),
        );
    }
    let mut body = col()
        .w_full()
        .gap(tk.r(0.75))
        .opacity(if omitted { 0.5 } else { 1.0 });
    for lv in levels {
        let mut lh = row()
            .w_full()
            .px(tk.r(0.75))
            .py(tk.r(0.375))
            .center_y()
            .justify_between()
            .fill(tk.muted.alpha(0.4))
            .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
            .border_layout()
            .child(
                text(tr(
                    "sourceFunds:case.levelSummary",
                    &[
                        ("level", &lv["level"].to_string()),
                        ("transactions", &lv["transaction_count"].to_string()),
                        ("sources", &lv["source_count"].to_string()),
                    ],
                ))
                .font(tk.txs(500.0))
                .color(tk.fg),
            );
        if let Some(v) = model::f(lv, "fiat_value_total") {
            let cur = {
                let c = model::s(lv, "fiat_currency");
                if c.is_empty() { "EUR" } else { c }
            };
            lh = lh.child(crate::ui::sensitive(
                crate::format::fiat(v, cur),
                kit::tnum(tk.txs(500.0).mono()),
                tk.fg,
                hidden,
            ));
        }
        let mut b = kit::bordered(tk).clip_rounded(tk.r(0.375)).child(lh);
        let nodes = model::arr(lv, "nodes");
        for (i, n) in nodes.iter().enumerate() {
            let src = model::s(n, "node_type") == "source";
            let amount = model::f(n, "required_amount").or_else(|| model::f(n, "amount"));
            let inbound = src || model::s(n, "direction") == "inbound";
            let prov = if src {
                "attested".to_owned()
            } else {
                match model::s(n, "data_provenance") {
                    "chain_sync" => "chain".into(),
                    "platform_export" => "platform".into(),
                    "manual_import" => "manual".into(),
                    _ => String::new(),
                }
            };
            let date = {
                let d = model::s(n, "occurred_at");
                model::format_datetime(if d.is_empty() {
                    model::s(n, "acquired_at")
                } else {
                    d
                })
            };
            let name = if src {
                model::s(n, "label").to_owned()
            } else {
                model::first(n, &["wallet", "label"]).to_owned()
            };
            let kind = if src {
                model::pretty({
                    let s = model::s(n, "source_type");
                    if s.is_empty() { "source" } else { s }
                })
            } else {
                model::pretty(model::s(n, "direction"))
            };
            let amt = match amount {
                None => text("—").font(tk.txs(400.0)).color(tk.fg),
                Some(v) => crate::ui::sensitive(
                    format!(
                        "{}{}",
                        if inbound { "+" } else { "−" },
                        model::format_btc(Some(v), {
                            let a = model::s(n, "asset");
                            if a.is_empty() { "BTC" } else { a }
                        })
                    ),
                    kit::tnum(tk.txs(400.0).mono()),
                    tk.fg,
                    hidden,
                ),
            };
            let mut r = row()
                .w_full()
                .px(tk.r(0.75))
                .py(tk.r(0.375))
                .gap(tk.r(0.75))
                .center_y()
                .child(text(date).font(tk.txs(400.0)).color(tk.muted_fg).w(110.0))
                .child(
                    text(name)
                        .font(tk.txs(500.0))
                        .color(tk.fg)
                        .truncate()
                        .grow(1.0)
                        .w(0.0),
                )
                .child(text(kind).font(tk.txs(400.0)).color(tk.muted_fg).w(90.0))
                .child(amt.w(120.0).align(rune::Align::End))
                .child(
                    text(prov)
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg)
                        .w(70.0)
                        .align(rune::Align::End),
                );
            if i + 1 < nodes.len() {
                r = r
                    .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
                    .border_layout();
            }
            b = b.child(r);
        }
        body = body.child(b);
    }
    Some(
        col()
            .w_full()
            .gap(tk.r(0.5))
            .child(head)
            .child(
                text(t("sourceFunds:workstation.level1IsTheReport"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap(),
            )
            .child(body),
    )
}

/// "Adjust report": diagram detail, amount precision, masking, omitted sections, recipient.
fn report_options(app: &App, tk: &Tk, layer: &mut Vec<El>) -> El {
    let c = app.provenance.sof.case.as_ref().expect("case");
    let dl = t("sourceFunds:workstation.diagramDetail");
    let al = t("sourceFunds:workstation.amounts");
    let mut left = col()
        .grow(1.0)
        .w(0.0)
        .min_w(0.0)
        .gap(tk.r(1.0))
        .child(kit::field(
            tk,
            &dl,
            kit::select(
                app,
                tk,
                layer,
                "diagram",
                &dl,
                vec![
                    (
                        "summary".into(),
                        t("sourceFunds:workstation.summary"),
                        false,
                    ),
                    (
                        "detailed".into(),
                        t("sourceFunds:workstation.detailed"),
                        false,
                    ),
                ],
                &c.diagram_detail,
                "",
                tk.r(2.25),
                |a, v| case(a).diagram_detail = v.to_owned(),
            ),
        ))
        .child(kit::field(
            tk,
            &al,
            kit::select(
                app,
                tk,
                layer,
                "precision",
                &al,
                vec![
                    ("btc".into(), "BTC".into(), false),
                    ("sats".into(), "sats".into(), false),
                ],
                &c.amount_precision,
                "",
                tk.r(2.25),
                |a, v| case(a).amount_precision = v.to_owned(),
            ),
        ))
        .child(kit::checkbox(
            tk,
            "sof-mask",
            c.mask_recipient,
            &t("sourceFunds:workstation.maskRecipientNameOnThe"),
            |a| case(a).mask_recipient ^= true,
        ));
    let mut omit = col()
        .w_full()
        .gap(tk.r(0.375))
        .pt(tk.r(0.75))
        .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
        .border_layout()
        .child(
            text(t("sourceFunds:workstation.omitVerboseSections"))
                .font(tk.txs(500.0))
                .color(tk.muted_fg),
        );
    for (k, l) in [
        ("flow_levels", "case.omitFlow"),
        ("transaction_details", "case.omitTransactions"),
        ("flow_links", "case.omitLinks"),
        ("graph_nodes", "case.omitNodes"),
    ] {
        let on = c.omit_sections.iter().any(|s| s == k);
        omit = omit.child(kit::checkbox(
            tk,
            &format!("sof-omit-{k}"),
            on,
            &t(&format!("sourceFunds:{l}")),
            move |a| {
                let o = &mut case(a).omit_sections;
                if let Some(i) = o.iter().position(|s| s == k) {
                    o.remove(i);
                } else {
                    o.push(k.to_owned());
                }
            },
        ));
    }
    left = left.child(omit);
    let recipients: Vec<Value> = app
        .provenance
        .sof
        .recipients
        .data()
        .map(|d| model::arr(d, "recipients").to_vec())
        .unwrap_or_default();
    let mut right = col().grow(1.0).w(0.0).min_w(0.0).gap(tk.r(1.0));
    if recipients.is_empty() {
        right = right.child(
            kit::bordered(tk)
                .px(tk.r(0.75))
                .py(tk.r(0.75))
                .fill(tk.muted.alpha(0.3))
                .child(
                    text(t("sourceFunds:case.optionalRecipient"))
                        .font(tk.tsm(400.0))
                        .color(tk.muted_fg)
                        .wrap(),
                ),
        );
    } else {
        let mut opts = vec![(String::new(), t("sourceFunds:recipient.none"), false)];
        for r in &recipients {
            let inactive = r["active"].as_bool() == Some(false);
            let label = format!(
                "{} · {}{}",
                model::s(r, "label"),
                reveal_label(model::s(r, "default_reveal_mode")),
                if inactive {
                    t("sourceFunds:recipient.inactiveSuffix")
                } else {
                    String::new()
                }
            );
            opts.push((model::s(r, "id").to_owned(), label, inactive));
        }
        let rl = t("sourceFunds:recipient.ariaLabel");
        let mut picker = kit::bordered(tk)
            .px(tk.r(0.75))
            .py(tk.r(0.75))
            .child(
                text(rl.clone())
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .mb(tk.r(0.25)),
            )
            .child(
                text(t("sourceFunds:recipient.hint"))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .mb(tk.r(0.5)),
            )
            .child(kit::select(
                app,
                tk,
                layer,
                "recipient",
                &rl,
                opts,
                &c.recipient,
                "",
                tk.r(2.25),
                |a, v| case(a).recipient = v.to_owned(),
            ));
        let sel = recipients.iter().find(|r| model::s(r, "id") == c.recipient);
        if let Some(s) = sel
            && !model::s(s, "notes").is_empty()
        {
            picker = picker.child(
                text(model::s(s, "notes").to_owned())
                    .font(tk.txs(400.0))
                    .color(tk.fg.alpha(0.8))
                    .wrap()
                    .mt(tk.r(0.5)),
            );
        }
        right = right.child(picker);
        // `RecipientPreferenceAdvisory`: the recipient's default is advice; the user's choice wins.
        if let Some(s) = sel {
            let pref = model::s(s, "default_reveal_mode").to_owned();
            if !pref.is_empty() && pref != c.reveal_mode {
                let raw = tr(
                    "sourceFunds:recipient.advisory",
                    &[
                        ("label", model::s(s, "label")),
                        ("preferred", &reveal_label(&pref)),
                        ("current", &reveal_label(&c.reveal_mode)),
                    ],
                );
                right = right.child(
                    row()
                        .w_full()
                        .gap(tk.r(0.75))
                        .px(tk.r(0.75))
                        .py(tk.r(0.5))
                        .center_y()
                        .justify_between()
                        .radius(tk.r(0.375))
                        .border(1.0, tk.border)
                        .border_layout()
                        .fill(tk.muted.alpha(0.3))
                        .child(rich(tk, &raw))
                        .child(
                            kit::button(
                                tk,
                                "sof-apply-pref",
                                Variant::Outline,
                                Size::Sm,
                                None,
                                &t("sourceFunds:recipient.applyPreference"),
                                true,
                            )
                            .on_press(move |a: &mut App| case(a).reveal_mode = pref.clone()),
                        ),
                );
            }
        }
    }
    row()
        .w_full()
        .gap(tk.r(1.5))
        .items_start()
        .child(left)
        .child(right)
}

/// `<Trans>` with `<0>`/`<1>` emphasis: muted text with the tagged parts in the foreground.
fn rich(tk: &Tk, raw: &str) -> El {
    let mut r = row().flex_wrap().shrink().min_w(0.0);
    let mut rest = raw;
    while !rest.is_empty() {
        let next = ["<0>", "<1>"]
            .iter()
            .filter_map(|tag| rest.find(tag).map(|i| (i, *tag)))
            .min();
        match next {
            Some((i, tag)) => {
                if i > 0 {
                    r = r.child(
                        text(rest[..i].to_owned())
                            .font(tk.txs(400.0))
                            .color(tk.muted_fg),
                    );
                }
                let close = tag.replace('<', "</");
                let after = &rest[i + tag.len()..];
                let end = after.find(&close).unwrap_or(after.len());
                r = r.child(
                    text(after[..end].to_owned())
                        .font(tk.txs(500.0))
                        .color(tk.fg),
                );
                rest = after.get(end + close.len()..).unwrap_or("");
            }
            None => {
                r = r.child(text(rest.to_owned()).font(tk.txs(400.0)).color(tk.muted_fg));
                rest = "";
            }
        }
    }
    r
}

/// `ReportDiagram`: the server-rendered SVG, inert, on white in its original print palette.
fn diagram(
    app: &App,
    cx: &mut Cx<App>,
    tk: &Tk,
    svg: &str,
    label: &str,
    avail: f32,
    wide: bool,
) -> El {
    if svg.is_empty() {
        return el();
    }
    let (vw, vh) = view_box(svg).unwrap_or((900.0, 400.0));
    let w = if wide { avail.max(900.0) } else { avail };
    let h = w * vh / vw.max(1.0);
    let img = app
        .ov
        .images()
        .svg(svg.as_bytes().to_vec(), w.max(h), cx.env().scale.max(1.0));
    let pic = el()
        .size(w, h)
        .pixels(img.pixels())
        .role("img")
        .label(label.to_owned());
    let frame = kit::bordered(tk)
        .fill(Color::WHITE)
        .clip_rounded(tk.r(0.375));
    let frame = if wide && w > avail {
        frame.child(row().w_full().scroll_x().child(pic))
    } else {
        frame.child(pic)
    };
    col().w_full().gap(tk.r(0.5)).child(frame).child(
        text(label.to_owned())
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
    )
}

fn view_box(svg: &str) -> Option<(f32, f32)> {
    let i = svg.find("viewBox=\"")? + 9;
    let rest = &svg[i..];
    let v: Vec<f32> = rest[..rest.find('"')?]
        .split([' ', ','])
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    (v.len() == 4).then(|| (v[2], v[3]))
}

/// Stage 4: save and hand over.
pub fn export(app: &App, tk: &Tk) -> El {
    let sof = &app.provenance.sof;
    let c = sof.case.as_ref().expect("case");
    let exportable = super::exportable(app)
        && !sof.preview.fetching()
        && !sof.preview.error()
        && !sof.resolved.error();
    let mut out = col()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(1.25))
        .child(stage_header(
            tk,
            &t("sourceFunds:journey.exportTitle"),
            &t("sourceFunds:journey.exportHint"),
            None,
        ));
    if !exportable && !sof.preview.fetching() {
        let a = amber();
        let tone = kit::notice(tk, &a, a.c950, a.c100);
        out = out.child(
            kit::notice_box(tk, tone)
                .py(tk.r(0.625))
                .child(
                    row()
                        .gap(tk.r(0.5))
                        .center_y()
                        .child(ic(&I::SHIELD_ALERT, tk.r(1.0), tone.2).a11y_hidden())
                        .child(
                            text(t("sourceFunds:case.exportBlocked"))
                                .font(tk.tsm(500.0))
                                .color(tone.2),
                        ),
                )
                .child(
                    text(t("sourceFunds:workstation.goToTheTraceWork"))
                        .font(tk.txs(500.0))
                        .color(tone.2)
                        .mt(tk.r(0.25))
                        .role("button")
                        .label(t("sourceFunds:workstation.goToTheTraceWork"))
                        .focusable(true)
                        .cursor(rune::Cursor::Pointer)
                        .on_press(|a: &mut App| super::go_stage(a, super::Stage::Trace)),
                ),
        );
    }
    let running = c.exporting.as_ref();
    let saving = running.is_some_and(|(_, _, saved)| saved.is_none());
    let rendering_pdf = running.is_some_and(|(b, _, saved)| !*b && saved.is_some());
    let rendering_zip = running.is_some_and(|(b, _, saved)| *b && saved.is_some());
    let done = super::completed(app);
    let pdf_label = t(if saving {
        "sourceFunds:export.savingCase"
    } else if rendering_pdf {
        "sourceFunds:case.rendering"
    } else {
        "sourceFunds:export.saveAndExport"
    });
    let can = exportable && running.is_none();
    let pdf_btn = kit::button(
        tk,
        "sof-export-pdf",
        Variant::Default,
        Size::Default,
        Some(&I::FILE_DOWN),
        &pdf_label,
        can,
    )
    .w_full();
    let pdf_btn = if can {
        pdf_btn.on_press(|a: &mut App| super::export(a, false))
    } else {
        pdf_btn
    };
    let mut pdf = col().w_full().gap(tk.r(0.75)).child(pdf_btn);
    if let Some(d) = done {
        let row_ = |l: String, v: String, mono: bool| {
            row()
                .w_full()
                .gap(tk.r(0.75))
                .justify_between()
                .child(text(l).font(tk.txs(400.0)).color(tk.muted_fg))
                .child(
                    text(v)
                        .font(if mono {
                            tk.txs(400.0).mono()
                        } else {
                            tk.txs(400.0)
                        })
                        .color(tk.fg)
                        .truncate()
                        .shrink(),
                )
        };
        pdf = pdf.child(
            kit::bordered(tk)
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .gap(tk.r(0.25))
                .child(row_(
                    t("sourceFunds:workstation.case"),
                    model::s(&d.case, "id").to_owned(),
                    true,
                ))
                .child(row_(
                    t("sourceFunds:workstation.status"),
                    model::s(&d.case, "status").to_owned(),
                    false,
                ))
                .child(row_(
                    t("sourceFunds:workstation.snapshot"),
                    model::s(&d.case, "snapshot_hash").to_owned(),
                    true,
                )),
        );
        if let Some(f) = &d.pdf {
            pdf = pdf.child(
                text(tr("sourceFunds:case.exported", &[("filename", f)]))
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap_anywhere(),
            );
        }
    }
    let zip_label = t(if saving {
        "sourceFunds:export.savingCase"
    } else if rendering_zip {
        "sourceFunds:case.buildingBundle"
    } else {
        "sourceFunds:case.exportBundle"
    });
    let zip_btn = kit::button(
        tk,
        "sof-export-bundle",
        Variant::Outline,
        Size::Default,
        Some(&I::FILE_DOWN),
        &zip_label,
        can,
    );
    let zip_btn = if can {
        zip_btn.on_press(|a: &mut App| super::export(a, true))
    } else {
        zip_btn
    };
    let mut zip = col()
        .w_full()
        .gap(tk.r(0.75))
        .child(
            text(t("sourceFunds:workstation.saveAFrozenCaseAnd"))
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .wrap(),
        )
        .child(row().child(zip_btn));
    if let Some(f) = done.and_then(|d| d.bundle.as_ref()) {
        zip = zip.child(
            text(tr("sourceFunds:case.exported", &[("filename", f)]))
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .wrap_anywhere(),
        );
    }
    let cards = vec![
        kit::card(
            tk,
            "sof-export-pdf-card",
            None,
            &t("sourceFunds:workstation.sourceoffundsPdf"),
            Some(t("sourceFunds:workstation.savesTheCaseSnapshotFirst")),
            pdf,
        ),
        kit::card(
            tk,
            "sof-export-zip-card",
            None,
            &t("sourceFunds:workstation.evidenceBundle"),
            Some(t("sourceFunds:workstation.reportPdfPlusTheOriginal")),
            zip,
        ),
    ];
    // `grid gap-4 lg:grid-cols-2`: the two cards share a row height.
    if app.shell.window_w >= 1024.0 || app.shell.window_w == 0.0 {
        let mut r = row().w_full().gap(tk.r(1.0)).items_stretch();
        for c in cards {
            r = r.child(col().grow(1.0).w(0.0).min_w(0.0).child(c.h_full()));
        }
        out.child(r)
    } else {
        out.child(col().w_full().gap(tk.r(1.0)).children(cards))
    }
}
