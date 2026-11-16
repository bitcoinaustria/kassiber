//! Captions, warnings (with the add/review backend action), empty states and the folded
//! technical details (GraphWarnings, GraphEmptyState, graphSupportText,
//! TransactionGraphTechnicalDetails.tsx).

use rune::{El, col, row, text};

use super::Ctx;
use super::data::Payload;
use crate::app::App;
use crate::i18n::{has, t};
use crate::icons as I;
use crate::ui::{ic, outline_button};

/// The caption under the title, for partial graphs only.
pub fn support_caption(p: &Payload) -> Option<String> {
    if p.support_level != "partial" {
        return None;
    }
    Some(match p.unsupported_reason.as_deref() {
        Some("confidential_values_hidden") => t("transactions:graph.confidentialSupport"),
        Some("input_prevout_values_missing") => t("transactions:graph.inputPrevoutSupport"),
        _ => t("transactions:graph.partialSupport"),
    })
}

/// `graphDiagnosticAction`: (target, label key).
fn diagnostic(p: Option<&Payload>) -> Option<(&'static str, &'static str)> {
    let p = p?;
    let codes: Vec<&str> = p.warnings.iter().map(|w| w.code.as_str()).collect();
    let liquid = p.unsupported_reason.as_deref() == Some("liquid_reference_graph_not_local")
        || codes
            .iter()
            .any(|c| c.starts_with("liquid_reference_lookup_"));
    if liquid {
        let key = if codes.contains(&"liquid_reference_lookup_unavailable") {
            "transactions:graph.addLiquidBackend"
        } else {
            "transactions:graph.reviewLiquidBackend"
        };
        return Some(("liquid", key));
    }
    if codes
        .iter()
        .any(|c| c.starts_with("bitcoin_reference_lookup_"))
    {
        let key = if codes.contains(&"bitcoin_reference_lookup_unavailable") {
            "transactions:graph.addBitcoinBackend"
        } else {
            "transactions:graph.reviewBitcoinBackend"
        };
        return Some(("bitcoin", key));
    }
    None
}

fn action_button(c: &Ctx, target: &'static str, key: &'static str) -> El {
    outline_button(
        c.tk,
        &format!("graph-backend-{target}"),
        Some(&crate::icons::ARROW_RIGHT),
        &t(key),
    )
    .on_press(move |a: &mut App| {
        // The web also hands Settings a deferred setup reason ("Transaction graph needs a …
        // backend"); this app's Settings takes no setup reason, so only the navigation remains.
        a.navigate(&format!("/settings#{target}"));
    })
}

fn code_text(code: &str, fallback: &str) -> String {
    let key = format!("transactions:graph.warnings.{}", code.trim());
    if has(&key) {
        t(&key)
    } else if fallback.is_empty() {
        code.to_owned()
    } else {
        fallback.to_owned()
    }
}

/// Warning and error rows (info notes stay hidden), then the backend action.
pub fn warnings(c: &Ctx, p: Option<&Payload>) -> Option<El> {
    let tk = c.tk;
    let p_ref = p?;
    let alerts: Vec<_> = p_ref
        .warnings
        .iter()
        .filter(|w| matches!(w.level.as_deref(), Some("warning") | Some("error")))
        .collect();
    if alerts.is_empty() {
        return None;
    }
    let mut out = col().key("graph-warnings").w_full().gap(tk.r(0.5));
    for w in alerts {
        out = out.child(
            row()
                .w_full()
                .gap(tk.r(0.5))
                .px(tk.r(0.75))
                .py(tk.r(0.5))
                .radius(tk.r(0.375))
                .border(1.0, tk.border)
                .fill(tk.muted.alpha(0.35))
                .role("status")
                .child(ic(&I::TRIANGLE_ALERT, tk.r(1.0), tk.warn.solid))
                .child(
                    text(code_text(&w.code, &w.message))
                        .font(tk.tsm(400.0))
                        .color(tk.fg)
                        .wrap()
                        .grow(1.0)
                        .min_w(0.0),
                ),
        );
    }
    if let Some((target, key)) = diagnostic(p) {
        out = out.child(row().child(action_button(c, target, key)));
    }
    Some(out)
}

/// The box shown instead of a drawing: loading, error, unsupported, graphless.
pub fn empty_state(c: &Ctx, p: Option<&Payload>, loading: bool, error: Option<&str>) -> El {
    let tk = c.tk;
    let reason = p.and_then(|p| p.unsupported_reason.as_deref());
    let title = if loading {
        t("transactions:graph.loading")
    } else if error.is_some() {
        t("transactions:graph.error")
    } else if p.is_some_and(|p| p.support_level == "unsupported") {
        t("transactions:graph.unsupported")
    } else if reason == Some("liquid_reference_graph_not_local") {
        t("transactions:graph.liquidGraphless")
    } else {
        t("transactions:graph.graphless")
    };
    let body = if loading {
        t("transactions:graph.loadingBody")
    } else if let Some(e) = error {
        e.to_owned()
    } else if reason == Some("liquid_reference_graph_not_local") {
        t("transactions:graph.liquidBody")
    } else {
        t("transactions:graph.graphlessBody")
    };
    let mut inner = col()
        .grow(1.0)
        .min_w(0.0)
        .gap(tk.r(0.25))
        .child(text(title).font(tk.tsm(500.0)).color(tk.fg))
        .child(text(body).font(tk.tsm(400.0)).color(tk.muted_fg).wrap());
    if let Some((target, key)) = diagnostic(p) {
        inner = inner.child(row().mt(tk.r(0.5)).child(action_button(c, target, key)));
    }
    let boxed = row()
        .key("graph-empty")
        .w_full()
        .gap(tk.r(0.75))
        .padding(tk.r(1.0))
        .radius(tk.r(0.375))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.35))
        .role("status")
        .busy(loading)
        .child(
            row()
                .size(32.0, 32.0)
                .center()
                .radius(tk.r(0.375))
                .fill(tk.background)
                .child(ic(
                    if error.is_some() {
                        &I::TRIANGLE_ALERT
                    } else {
                        &I::INFO
                    },
                    tk.r(1.0),
                    tk.muted_fg,
                )),
        )
        .child(inner);
    col()
        .w_full()
        .gap(tk.r(0.5))
        .child(boxed)
        .children(warnings(c, p))
}

/// "Technical graph": observed facts only, folded to one summary line.
pub fn technical(c: &Ctx, p: &Payload) -> Option<El> {
    let tk = c.tk;
    let tx = p.transaction.as_ref()?;
    let has_graph = matches!(p.support_level.as_str(), "full" | "partial");
    let fee = p
        .fee
        .as_ref()
        .and_then(|f| f.value_btc.or(f.value_sats.map(|s| s / 1e8)));
    let int = |v: f64| format!("{}", v as i64);
    let fields: Vec<(String, String, bool)> = [
        (
            "details.inputCount",
            if has_graph {
                tx.input_count.map(int)
            } else {
                None
            },
            false,
        ),
        (
            "details.outputCount",
            if has_graph {
                tx.output_count.map(int)
            } else {
                None
            },
            false,
        ),
        // `formatBtcAmount`: "0.00001120 BTC".
        (
            "details.networkFee",
            fee.map(crate::txdetail::draft::btc_amount),
            true,
        ),
        (
            "details.feeRate",
            tx.fee_rate_sat_vb.map(|r| format!("{r} sat/vB")),
            true,
        ),
        ("details.version", tx.version.map(int), false),
        ("details.locktime", tx.locktime.map(int), false),
        (
            "details.size",
            tx.size.map(|v| format!("{} B", v as i64)),
            false,
        ),
        (
            "details.vsize",
            tx.vsize.map(|v| format!("{} vB", v as i64)),
            false,
        ),
        (
            "details.weight",
            tx.weight.map(|v| format!("{} WU", v as i64)),
            false,
        ),
    ]
    .into_iter()
    .filter_map(|(k, v, s)| v.map(|v| (t(&format!("transactions:{k}")), v, s)))
    .collect();
    // "Investigate public exposure": a local navigation to Chain Analysis for this txid.
    let txid = [tx.txid.clone(), tx.external_id.clone()]
        .into_iter()
        .flatten()
        .find(|v| v.len() == 64 && v.chars().all(|c| c.is_ascii_hexdigit()));
    let investigate = match (&txid, tx.chain.as_deref(), tx.network.as_deref()) {
        (Some(id), Some(chain), Some(network)) => {
            let href = format!(
                "/chain-analysis?chain={chain}&network={network}&subject={}&mode=trace&observer=public",
                id.to_lowercase()
            );
            Some(
                row()
                    .key("graph-investigate")
                    .h(tk.r(1.75))
                    .px(tk.r(0.5))
                    .gap(tk.r(0.375))
                    .center_y()
                    .radius(tk.r(0.375))
                    .hover(tk.accent)
                    .role("button")
                    .label(t("privacyMirror:investigateTransaction"))
                    .focusable(true)
                    .on_press(move |a: &mut App| a.navigate(&href))
                    .child(ic(&I::EYE, tk.r(0.875), tk.fg))
                    .child(
                        text(t("privacyMirror:investigateTransaction"))
                            .font(tk.txs(400.0))
                            .color(tk.fg),
                    ),
            )
        }
        _ => None,
    };
    if fields.is_empty() {
        return investigate.map(|b| row().w_full().justify_end().child(b));
    }
    let shown = |(_, v, s): &(String, String, bool)| {
        if *s && c.hide {
            t("transactions:graph.hidden")
        } else {
            v.clone()
        }
    };
    let summary = fields
        .iter()
        .filter(|f| !f.2 || !c.hide)
        .take(5)
        .map(|f| format!("{} {}", f.0, shown(f)))
        .collect::<Vec<_>>()
        .join(" · ");
    let open = c.app.graph.tech_open;
    let head = row()
        .key("graph-tech-toggle")
        .w_full()
        .gap(tk.r(0.5))
        .px(tk.r(0.75))
        .py(tk.r(0.5))
        .center_y()
        .role("button")
        .expanded(open)
        .label(t("transactions:details.technical"))
        .focusable(true)
        .on_press(|a: &mut App| a.graph.tech_open = !a.graph.tech_open)
        .child(ic(
            if open {
                &I::CHEVRON_DOWN
            } else {
                &I::CHEVRON_RIGHT
            },
            tk.r(0.875),
            tk.muted_fg,
        ))
        .child(
            text(t("transactions:details.technical"))
                .font(tk.txs(500.0))
                .color(tk.fg),
        )
        .child(
            text(summary)
                .font(tk.txs(400.0))
                .color(tk.muted_fg)
                .truncate()
                .grow(1.0)
                .min_w(0.0),
        )
        .children(investigate);
    let mut out = col()
        .key("graph-tech")
        .w_full()
        .radius(tk.r(0.5))
        .border(1.0, tk.border)
        .fill(tk.muted.alpha(0.2))
        .clip_rounded(tk.r(0.5))
        .child(head);
    if open {
        let mut grid = col().w_full().border_sides([1.0, 0.0, 0.0, 0.0], tk.border);
        for chunk in fields.chunks(2) {
            let mut r = row().w_full();
            for f in chunk {
                r = r.child(
                    row()
                        .grow(1.0)
                        .px(tk.r(0.75))
                        .py(tk.r(0.375))
                        .justify_between()
                        .child(text(f.0.clone()).font(tk.txs(400.0)).color(tk.muted_fg))
                        .child(
                            text(shown(f))
                                .font(tk.txs(500.0).features("'tnum' 1"))
                                .color(tk.fg),
                        ),
                );
            }
            grid = grid.child(r);
        }
        out = out.child(grid);
    }
    Some(out)
}
