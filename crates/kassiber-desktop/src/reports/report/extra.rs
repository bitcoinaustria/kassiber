//! The Reports page's two satellites: the Lightning routing-profitability panel
//! (LightningProfitabilityPanel.tsx), shown only for books with a Lightning connection, and the
//! capital-gain explanation page `/reports?result=<reference>` (CapitalGainExplanation.tsx's
//! `ExplanationBody`, the link target of an explained result).

use rune::{Color, Cx, El, TextStyle, col, row, text};
use serde_json::Value;

use super::view::{connections, hexc};
use super::widgets::{select_menu, select_trigger, tnum, toggle};
use super::{Popup, model};
use crate::app::{App, Load};
use crate::format;
use crate::i18n::{t, tr};
use crate::icons as I;
use crate::theme::Tk;
use crate::txdetail::widgets::{self as w, Size, Variant};
use crate::ui::{ic, sensitive};

// ---- Lightning ------------------------------------------------------------------------------

const LN_KINDS: [&str; 4] = ["core-ln", "coreln", "lnd", "nwc"];

/// `connectionSupportsLightningCapability(connection, "routingProfitability")`.
fn routing_capable(c: &Value) -> bool {
    match c.get("lightningCapabilities").filter(|v| v.is_object()) {
        Some(caps) => caps["routingProfitability"].as_bool() == Some(true),
        None => matches!(c["kind"].as_str(), Some("core-ln" | "coreln" | "lnd")),
    }
}

fn sat(v: f64) -> String {
    format!("{} sat", format::count(v.abs().round() as usize))
}

fn sat_signed(v: f64) -> String {
    format!("{}{}", if v >= 0.0 { "+ " } else { "- " }, sat(v))
}

fn amber_box(tk: &Tk, msg: String) -> El {
    let (fill, border, ink) = if tk.dark {
        (
            hexc("#451a03").alpha(0.4),
            hexc("#78350f").alpha(0.6),
            hexc("#fcd34d"),
        )
    } else {
        (hexc("#fffbeb"), hexc("#fde68a"), hexc("#b45309"))
    };
    col()
        .w_full()
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .radius(tk.r(0.375))
        .fill(fill)
        .border(1.0, border)
        .border_layout()
        .child(text(msg).font(tk.tsm(400.0)).color(ink).wrap().w_full())
}

pub fn lightning_panel(app: &App, cx: &Cx<App>, tk: &Tk, layer: &mut Vec<El>) -> Option<El> {
    let all: Vec<Value> = connections(app)
        .into_iter()
        .filter(|c| c["kind"].as_str().is_some_and(|k| LN_KINDS.contains(&k)))
        .collect();
    if all.is_empty() {
        return None;
    }
    let capable: Vec<&Value> = all.iter().filter(|c| routing_capable(c)).collect();
    let ui = &app.reports.report;
    let active = capable
        .iter()
        .find(|c| c["id"].as_str() == ui.ln_selected.as_deref())
        .or_else(|| capable.first())
        .copied();
    let title = row()
        .gap(tk.r(0.5))
        .center_y()
        .child(ic(&I::TRENDING_UP, tk.r(1.0), tk.fg))
        .child(
            text(t("connections:node.profitabilityReport.title"))
                .font(if crate::reports::ledger::common::win_w(app, cx) >= 640.0 {
                    tk.tbase(600.0)
                } else {
                    tk.tsm(600.0)
                })
                .color(tk.fg),
        );
    let header = col()
        .w_full()
        .px(tk.r(1.0))
        .pb(tk.r(0.75))
        .gap(tk.r(0.375))
        .border_sides([0.0, 0.0, 1.0, 0.0], tk.border)
        .border_layout()
        .child(title)
        .child(
            text(tr(
                "connections:node.profitabilityReport.description",
                &[("cost", &format::count(2_500))],
            ))
            .font(tk.tsm(400.0))
            .color(tk.muted_fg)
            .wrap()
            .w_full(),
        );
    let mut content = col().w_full().px(tk.r(1.0)).pt(tk.r(1.0)).gap(tk.r(1.0));
    match active {
        None => {
            content = content.child(amber_box(
                tk,
                t("connections:node.profitabilityReport.unsupported"),
            ))
        }
        Some(conn) => {
            let id = conn["id"].as_str().unwrap_or("").to_owned();
            let label = conn["label"].as_str().unwrap_or("").to_owned();
            let open = ui.popup == Some(Popup::LnConnection);
            let trigger = select_trigger(
                tk,
                "rep-ln-connection",
                Some(&t("connections:node.profitabilityReport.connection")),
                &label,
                false,
                false,
                true,
                open,
                false,
            );
            let trigger = crate::ui::popup::trigger(
                trigger,
                crate::ui::popup::Kind::Listbox,
                open,
                |a: &mut App| toggle(a, Popup::LnConnection),
            );
            if open {
                let w = cx
                    .layout_size("rep-ln-connection")
                    .map(|[w, _]| w)
                    .unwrap_or(tk.r(10.0));
                let options = capable
                    .iter()
                    .map(|c| {
                        (
                            c["id"].as_str().unwrap_or("").to_owned(),
                            c["label"].as_str().unwrap_or("").to_owned(),
                            false,
                        )
                    })
                    .collect();
                select_menu(
                    tk,
                    layer,
                    "rep-ln-connection",
                    w,
                    options,
                    &id,
                    false,
                    |a: &mut App, v: &str| {
                        a.reports.report.ln_selected = Some(v.to_owned());
                    },
                );
            }
            content = content.child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(
                        text(t("connections:node.profitabilityReport.connection"))
                            .font(tk.txs(500.0))
                            .color(tk.muted_fg),
                    )
                    .child(trigger),
            );
            content = content.child(ln_body(app, tk, &id));
        }
    }
    Some(
        col()
            .key("rep-ln")
            .w_full()
            .py(tk.r(1.5))
            .gap(tk.r(1.5))
            .radius(tk.r(0.75))
            .fill(tk.card)
            .border(1.0, tk.border)
            .border_layout()
            .role("region")
            .label(t("connections:node.profitabilityReport.title"))
            .child(header)
            .child(content),
    )
}

fn ln_body(app: &App, tk: &Tk, id: &str) -> El {
    let load = app
        .reports
        .report
        .ln
        .as_ref()
        .filter(|(c, _)| c == id)
        .map(|(_, l)| l);
    let fetching = matches!(load, Some(Load::Loading));
    let label = if fetching {
        t("connections:node.profitabilityReport.loading")
    } else {
        t("connections:node.profitabilityReport.run")
    };
    let conn = id.to_owned();
    let mut run = w::button_surface(
        tk,
        row()
            .key("rep-ln-run")
            .h(tk.r(2.0))
            .px(tk.r(0.625))
            .gap(tk.r(0.375))
            .center(),
        Variant::Outline,
    )
    .role("button")
    .label(label.clone())
    .focusable(!fetching)
    .disabled(fetching);
    let mut glyph = ic(&I::REFRESH_CW, tk.r(0.875), tk.fg);
    if fetching {
        glyph = glyph.looping(rune::LoopSpec::rotation(1.0));
        run = run.opacity(0.5);
    } else {
        // Contacts the node: only from this press.
        run = run.on_press(move |a: &mut App| super::run_ln(a, &conn));
    }
    run = run
        .child(glyph)
        .child(text(label).font(tk.tsm(500.0)).color(tk.fg));
    let wrap = |inner: El| {
        col()
            .w_full()
            .gap(tk.r(0.75))
            .child(row().gap(tk.r(0.5)).child(run))
            .child(inner)
    };
    let muted = |s: String| text(s).font(tk.tsm(400.0)).color(tk.muted_fg);
    let report = match load {
        None => return wrap(muted(t("connections:node.profitabilityReport.idle"))),
        Some(Load::Loading) | Some(Load::Idle) => {
            return wrap(muted(t("connections:node.profitabilityReport.loading")));
        }
        Some(Load::Failed(e)) => {
            return wrap(amber_box(
                tk,
                if e.is_empty() {
                    t("connections:node.profitabilityReport.errorFallback")
                } else {
                    e.clone()
                },
            ));
        }
        Some(Load::Ready(d)) if d.is_null() => {
            return wrap(muted(t("connections:node.profitabilityReport.empty")));
        }
        Some(Load::Ready(d)) => d,
    };
    let hidden = app.prefs.hide_sensitive();
    let s = &report["summary"];
    let n = |k: &str| s[k].as_f64().unwrap_or(0.0);
    let green = hexc(if tk.dark { "#6ee7b7" } else { "#047857" });
    let red = hexc(if tk.dark { "#fca5a5" } else { "#b91c1c" });
    let profit = n("netProfitSat");
    let profit_ink = if profit > 0.0 {
        green
    } else if profit < 0.0 {
        red
    } else {
        tk.muted_fg
    };
    let mono = |w: f32| TextStyle::new(tk.r(0.875), w).mono().features("'tnum' 1");
    let stat = |label: String, value: String, ink: Color| {
        col()
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .px(tk.r(0.75))
            .py(tk.r(0.625))
            .gap(tk.r(0.25))
            .radius(tk.r(0.375))
            .fill(tk.background)
            .border(1.0, tk.border)
            .border_layout()
            .child(
                text(label.to_uppercase())
                    .font(tk.t2xs(500.0).tracking(tk.r(0.694) * 0.025))
                    .color(tk.muted_fg),
            )
            .child(sensitive(value, mono(600.0), ink, hidden))
    };
    let summary = tr(
        "connections:node.profitabilityReport.windowSummary",
        &[
            ("window", report["windowLabel"].as_str().unwrap_or("")),
            ("forwards", &format::count(n("forwardCount") as usize)),
            ("payments", &format::count(n("paymentCount") as usize)),
            ("rebalances", &format::count(n("rebalanceCount") as usize)),
        ],
    );
    let stats = row()
        .w_full()
        .gap(tk.r(0.75))
        .child(stat(
            t("connections:node.profitabilityReport.revenue"),
            sat_signed(n("routingRevenueSat")),
            green,
        ))
        .child(stat(
            t("connections:node.profitabilityReport.paymentFees"),
            sat_signed(-n("paymentCostSat")),
            red,
        ))
        .child(stat(
            t("connections:node.profitabilityReport.rebalanceFees"),
            sat_signed(-n("rebalanceCostSat")),
            red,
        ))
        .child(stat(
            t("connections:node.profitabilityReport.onChainCosts"),
            sat_signed(-n("onchainCostSat")),
            red,
        ))
        .child(stat(
            t("connections:node.profitabilityReport.netProfit"),
            sat_signed(profit),
            profit_ink,
        ));
    let heads = [
        "peer",
        "capacity",
        "earnedRouting",
        "openCost",
        "coversOpenCost",
    ];
    let cell = |i: usize, e: El| {
        let c = row()
            .grow(if i == 0 { 2.0 } else { 1.0 })
            .w(0.0)
            .min_w(0.0)
            .px(tk.r(0.5))
            .py(tk.r(0.5))
            .center_y();
        (if i == 0 { c } else { c.justify_end() }).child(e)
    };
    let mut table = col().w_full().child(
        crate::ui::table::head(tk)
            .h(tk.r(crate::ui::table::HEAD_H))
            .children(heads.iter().enumerate().map(|(i, h)| {
                cell(
                    i,
                    crate::ui::table::th_text(
                        tk,
                        t(&format!("connections:node.profitabilityReport.{h}")),
                    ),
                )
            })),
    );
    let channels = report["channels"].as_array().cloned().unwrap_or_default();
    if channels.is_empty() {
        table = table.child(
            row()
                .w_full()
                .padding(tk.r(0.5))
                .justify_center()
                .child(muted(t("connections:node.profitabilityReport.noChannels"))),
        );
    }
    for ch in &channels {
        let covers = ch["coversOpenCost"].as_bool().unwrap_or(false);
        let (_, ink, _) = match (covers, tk.dark) {
            (true, false) => (hexc("#ecfdf5"), hexc("#047857"), hexc("#059669").alpha(0.2)),
            (true, true) => (
                hexc("#064e3b").alpha(0.3),
                hexc("#6ee7b7"),
                hexc("#34d399").alpha(0.2),
            ),
            (false, _) => (tk.muted, tk.muted_fg, tk.border),
        };
        table = table.child(
            row()
                .w_full()
                .border_sides([1.0, 0.0, 0.0, 0.0], tk.border)
                .hover(tk.muted.alpha(0.35))
                .child(cell(
                    0,
                    row()
                        .gap(tk.r(0.375))
                        .center_y()
                        .child(ic(&I::ZAP, tk.r(0.75), hexc("#f59e0b")))
                        .child(
                            text(ch["peerAlias"].as_str().unwrap_or("").to_owned())
                                .font(tk.tsm(500.0))
                                .color(tk.fg)
                                .truncate(),
                        ),
                ))
                .child(cell(
                    1,
                    sensitive(
                        sat(ch["capacitySat"].as_f64().unwrap_or(0.0)),
                        mono(400.0),
                        tk.fg,
                        hidden,
                    ),
                ))
                .child(cell(
                    2,
                    sensitive(
                        sat(ch["earnedRoutingSat"].as_f64().unwrap_or(0.0)),
                        mono(400.0),
                        tk.fg,
                        hidden,
                    ),
                ))
                .child(cell(
                    3,
                    sensitive(
                        sat(ch["openCostSat"].as_f64().unwrap_or(0.0)),
                        mono(400.0),
                        tk.fg,
                        hidden,
                    ),
                ))
                .child(cell(
                    4,
                    row().child(
                        text(t(if covers {
                            "connections:node.profitabilityReport.yes"
                        } else {
                            "connections:node.profitabilityReport.notYet"
                        }))
                        .font(tk.t2xs(500.0))
                        .color(ink),
                    ),
                )),
        );
    }
    // `overflow-hidden rounded-md border` around shadcn's `overflow-x-auto` container: five
    // columns don't fit the side column, so the table keeps their least widths and scrolls.
    let least = tk.r(30.0);
    let w = crate::ui::table::width("rep-ln-table", least, least);
    let table = crate::ui::table::hscroll(
        "rep-ln-table",
        t("connections:node.profitabilityReport.title"),
        least,
        table.w(w),
    );
    let table = col()
        .w_full()
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .border_layout()
        .clip_rounded(tk.r(0.375))
        .child(table);
    wrap(
        col()
            .w_full()
            .gap(tk.r(1.0))
            .child(text(summary).font(tk.txs(400.0)).color(tk.muted_fg))
            .child(stats)
            .child(table),
    )
}

// ---- Explanation ----------------------------------------------------------------------------

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn mono_text(tk: &Tk, v: String, small: bool) -> El {
    let size = if small { tk.r(0.75) } else { tk.r(0.875) };
    text(v)
        .font(
            TextStyle::new(size, 400.0)
                .mono()
                .lh(if small { tk.r(1.0) } else { tk.r(1.25) }),
        )
        .color(tk.fg)
        .wrap_anywhere()
        .w_full()
}

fn para(tk: &Tk, v: String, muted: bool) -> El {
    text(v)
        .font(tk.tsm(400.0))
        .color(if muted { tk.muted_fg } else { tk.fg })
        .wrap()
        .w_full()
}

/// An underlined link (`underline`) that navigates to `href`.
fn link(tk: &Tk, key: String, label: String, href: String, small: bool) -> El {
    let style = if small { tk.txs(400.0) } else { tk.tsm(400.0) };
    let n = label.len();
    row()
        .key(key)
        .role("link")
        .label(label.clone())
        .focusable(true)
        .on_press(move |a: &mut App| a.navigate(&href))
        .child(
            rune::rich(label, vec![rune::Span::new(0, n).underline(tk.fg)])
                .font(style)
                .color(tk.fg),
        )
}

fn result_link(reference: &Value) -> String {
    format!("/reports?result={}", model::encode(&reference.to_string()))
}

/// `explanationPage`: `screenPanelClassName` around `ExplanationBody`.
pub fn explanation_page(app: &App, tk: &Tk) -> El {
    let ui = &app.reports.report;
    let page = col()
        .key("rep-page")
        .w_full()
        .padding(tk.page_pad())
        .fill(tk.background);
    let Some((raw, load)) = &ui.explain else {
        return page;
    };
    let reference: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let body = match load {
        Load::Idle | Load::Loading => para(tk, t("journals:explanation.loading"), false),
        Load::Failed(e) => para(
            tk,
            if e.is_empty() {
                t("journals:explanation.unavailable")
            } else {
                e.clone()
            },
            false,
        )
        .role("alert"),
        Load::Ready(d) => explanation_body(app, tk, d, &reference),
    };
    page.child(body)
}

pub fn explanation_body(app: &App, tk: &Tk, d: &Value, reference: &Value) -> El {
    let calc = &d["calculation"];
    let status = d["status"].as_str().unwrap_or("");
    if status != "available" || !calc.is_object() {
        return para(
            tk,
            t(if status == "engine_detail_mismatch" {
                "journals:explanation.mismatch"
            } else {
                "journals:explanation.unavailable"
            }),
            false,
        )
        .role("alert");
    }
    let cur = s(&d["currency"]);
    let version = s(&reference["input_version"]);
    let totals = &d["totals"];
    let mut out = col().w_full().gap(tk.r(1.0));
    out = out
        .child(para(
            tk,
            format!(
                "{} · {} · {cur} · {}",
                s(&d["book_label"]),
                s(&calc["method"]),
                tr("journals:explanation.version", &[("version", &version)])
            ),
            true,
        ))
        .child(link(
            tk,
            "rep-explain-link".into(),
            t("journals:explanation.link"),
            result_link(reference),
            false,
        ))
        .child(mono_text(
            tk,
            format!(
                "{} − {} = {} {cur}",
                s(&totals["proceeds_exact"]),
                s(&totals["cost_basis_exact"]),
                s(&totals["gain_loss_exact"])
            ),
            false,
        ));
    let q = d["quarantines"].as_u64().unwrap_or(0);
    if q > 0 {
        out = out.child(
            para(
                tk,
                tr(
                    "journals:explanation.quarantines",
                    &[("count", &q.to_string())],
                ),
                false,
            )
            .role("status"),
        );
    }
    out = out.child(para(tk, t("journals:explanation.poolNote"), true));
    out = calculation(app, tk, out, calc, &cur, reference, "");
    let mut custody = col()
        .w_full()
        .gap(tk.r(0.25))
        .child(
            text(t("journals:explanation.custody"))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading"),
        )
        .child(para(tk, t("journals:explanation.custodyContext"), true));
    let decisions = d["custody_decisions"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if decisions.is_empty() {
        custody = custody.child(para(tk, t("journals:explanation.noCustody"), false));
    }
    for c in decisions {
        custody = custody.child(mono_text(
            tk,
            format!(
                "{} → {} · {} · {} · {}",
                s(&c["source_transaction_id"]),
                s(&c["target_transaction_id"]),
                s(&c["custody_state"]),
                s(&c["basis_state"]),
                s(&c["reason"])
            ),
            true,
        ));
    }
    if d["custody_truncated"].as_bool() == Some(true) {
        custody = custody.child(para(tk, t("journals:explanation.custodyTruncated"), false));
    }
    let hidden = app.prefs.hide_sensitive();
    let out = out.child(custody);
    if hidden { out.blur(3.5) } else { out }
}

#[allow(clippy::too_many_arguments)]
fn calculation(
    app: &App,
    tk: &Tk,
    mut out: El,
    calc: &Value,
    cur: &str,
    reference: &Value,
    prefix: &str,
) -> El {
    for (i, f) in calc["fragments"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .enumerate()
    {
        let event = &f["event"];
        let mut sec = col()
            .w_full()
            .padding(tk.r(0.75))
            .gap(tk.r(0.5))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .border_layout()
            .child(
                text(format!(
                    "{} · {} msat",
                    tr(
                        "journals:explanation.fragment",
                        &[("index", &(i + 1).to_string())]
                    ),
                    s(&f["quantity_msat_exact"])
                ))
                .font(tk.tsm(500.0))
                .color(tk.fg)
                .role("heading"),
            )
            .child(mono_text(
                tk,
                format!(
                    "{} − {} = {} {cur}",
                    s(&f["proceeds_exact"]),
                    s(&f["cost_basis_exact"]),
                    s(&f["gain_loss_exact"])
                ),
                false,
            ));
        if !f["unit_basis_override_exact"].is_null() {
            sec = sec.child(para(
                tk,
                format!(
                    "{}: {} {cur}/{}",
                    t("journals:explanation.unitBasis"),
                    s(&f["unit_basis_override_exact"]),
                    s(&event["asset"])
                ),
                false,
            ));
        }
        sec = sec.child(source(
            app,
            tk,
            event,
            &t("journals:explanation.disposal"),
            reference,
            &format!("{prefix}event-{i}"),
        ));
        let lot = &f["lot"];
        if lot.is_object() {
            sec = sec
                .child(para(
                    tk,
                    format!(
                        "{}: {} {cur} / {} msat",
                        t("journals:explanation.acquisitionBasis"),
                        s(&f["acquisition_basis_exact"]),
                        s(&f["acquisition_quantity_msat_exact"])
                    ),
                    false,
                ))
                .child(source(
                    app,
                    tk,
                    lot,
                    &t("journals:explanation.acquisition"),
                    reference,
                    &format!("{prefix}lot-{i}"),
                ));
            let inh = &lot["inherited_basis"];
            if inh.is_object() {
                let mut b = col()
                    .w_full()
                    .pl(tk.r(0.75))
                    .gap(tk.r(0.5))
                    .border_sides([0.0, 0.0, 0.0, 2.0], tk.border)
                    .border_layout()
                    .child(
                        text(t("journals:explanation.inheritedBasis"))
                            .font(tk.tsm(500.0))
                            .color(tk.fg),
                    )
                    .child(para(tk, t("journals:explanation.inheritedContext"), true));
                for r in inh["relations"].as_array().cloned().unwrap_or_default() {
                    b = b.child(mono_text(
                        tk,
                        format!(
                            "{} → {} · {} · {} · {}\n{} {} msat → {} {} msat · {}: {} msat",
                            s(&r["source_transaction_id"]),
                            s(&r["target_transaction_id"]),
                            s(&r["reason"]),
                            s(&r["policy"]),
                            s(&r["basis_state"]),
                            s(&r["source_quantity_msat_exact"]),
                            s(&r["source_asset"]),
                            s(&r["target_quantity_msat_exact"]),
                            s(&r["target_asset"]),
                            t("journals:explanation.fee"),
                            if r["swap_fee_msat_exact"].is_null() {
                                "—".into()
                            } else {
                                s(&r["swap_fee_msat_exact"])
                            }
                        ),
                        true,
                    ));
                }
                let st = inh["status"].as_str().unwrap_or("");
                if st != "available" {
                    let key = match st {
                        "inherited_detail_truncated" => "journals:explanation.inheritedTruncated",
                        "engine_detail_mismatch" => "journals:explanation.mismatch",
                        _ => "journals:explanation.inheritedUnavailable",
                    };
                    b = b.child(para(tk, t(key), false).role("status"));
                }
                for (si, src) in inh["source_calculations"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .iter()
                    .enumerate()
                {
                    let mut sc = col().w_full().gap(tk.r(0.5)).child(
                        text(format!(
                            "{} · {} · {}",
                            s(&src["transaction_id"]),
                            s(&src["asset"]),
                            s(&src["calculation"]["method"])
                        ))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap_anywhere()
                        .w_full(),
                    );
                    let tt = &src["totals"];
                    if tt.is_object() {
                        sc = sc.child(mono_text(
                            tk,
                            format!(
                                "{} − {} = {} {cur} · {} msat",
                                s(&tt["proceeds_exact"]),
                                s(&tt["cost_basis_exact"]),
                                s(&tt["gain_loss_exact"]),
                                s(&tt["quantity_msat_exact"])
                            ),
                            false,
                        ));
                    }
                    if src["calculation"].is_object() {
                        sc = calculation(
                            app,
                            tk,
                            sc,
                            &src["calculation"],
                            cur,
                            reference,
                            &format!("{prefix}carry-{i}-{si}-"),
                        );
                    }
                    b = b.child(sc);
                }
                sec = sec.child(b);
            }
        }
        out = out.child(sec);
    }
    out
}

/// `SourceEvidence`: a `<details>` with the source's timestamp, spot price, fee and pricing.
fn source(app: &App, tk: &Tk, src: &Value, label: &str, reference: &Value, node: &str) -> El {
    let open = app.reports.report.explain_open.iter().any(|n| n == node);
    let summary_text = format!(
        "{label}: {}",
        if src["transaction_id"].is_null() {
            t("journals:explanation.missingSource")
        } else {
            s(&src["transaction_id"])
        }
    );
    let n = node.to_owned();
    let summary = row()
        .key(format!("rep-explain-{node}"))
        .w_full()
        .gap(tk.r(0.375))
        .items_start()
        .role("button")
        .label(summary_text.clone())
        .expanded(open)
        .focusable(true)
        .on_press(move |a: &mut App| {
            let v = &mut a.reports.report.explain_open;
            if let Some(i) = v.iter().position(|x| *x == n) {
                v.remove(i);
            } else {
                v.push(n.clone());
            }
        })
        .child(
            text(if open { "▾" } else { "▸" })
                .font(tk.tsm(400.0))
                .color(tk.fg),
        )
        .child(
            text(summary_text)
                .font(tk.tsm(400.0))
                .color(tk.fg)
                .wrap_anywhere()
                .grow(1.0)
                .w(0.0),
        );
    let mut d = col()
        .w_full()
        .padding(tk.r(0.5))
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .border_layout()
        .child(summary);
    if open {
        let labels = |k: &str| -> String {
            let key = match k {
                "fiat_currency" => "priceCurrency",
                "fiat_rate_exact" => "sourceRate",
                "fiat_value_exact" => "sourceValue",
                "pricing_source_kind" => "priceKind",
                "pricing_quality" => "priceQuality",
                "fiat_price_source" => "priceSource",
                "pricing_timestamp" => "priceTimestamp",
                "pricing_provider" => "priceProvider",
                "pricing_pair" => "pricePair",
                "pricing_fetched_at" => "priceFetched",
                "pricing_granularity" => "priceGranularity",
                "pricing_method" => "priceMethod",
                _ => return String::new(),
            };
            t(&format!("journals:explanation.{key}"))
        };
        let mut facts: Vec<(String, String)> = vec![
            (t("journals:explanation.timestamp"), s(&src["occurred_at"])),
            (
                t("journals:explanation.spotPrice"),
                s(&src["spot_price_exact"]),
            ),
            (
                t("journals:explanation.fee"),
                format!(
                    "{} · {} msat",
                    if src["fiat_fee_exact"].is_null() {
                        "—".into()
                    } else {
                        s(&src["fiat_fee_exact"])
                    },
                    s(&src["crypto_fee_msat_exact"])
                ),
            ),
        ];
        if let Some(p) = src["pricing"].as_object() {
            for (k, v) in p {
                if !v.is_null() {
                    facts.push((labels(k), s(v)));
                }
            }
        }
        let label_w = facts
            .iter()
            .map(|(l, _)| crate::reports::ledger::common::tw(l, tk.txs(400.0)))
            .fold(0.0, f32::max);
        let mut dl = col().w_full().mt(tk.r(0.5)).gap(tk.r(0.25));
        for (l, v) in facts {
            dl = dl.child(
                row()
                    .w_full()
                    .gap(tk.r(0.75))
                    .items_start()
                    .child(
                        text(l)
                            .font(tk.txs(400.0))
                            .color(tk.fg)
                            .w(label_w)
                            .min_w(label_w),
                    )
                    .child(
                        text(v)
                            .font(tk.txs(400.0))
                            .color(tk.fg)
                            .wrap_anywhere()
                            .grow(1.0)
                            .w(0.0),
                    ),
            );
        }
        d = d
            .child(link(
                tk,
                format!("rep-explain-{node}-link"),
                t("journals:explanation.sourceLink"),
                format!("{}#{node}", result_link(reference)),
                true,
            ))
            .child(dl);
    }
    let _ = (Size::Sm, tnum(tk.txs(400.0)));
    d
}
