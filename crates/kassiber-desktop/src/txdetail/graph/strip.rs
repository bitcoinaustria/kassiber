//! Above the drawing: the route card (route.rs) or the exchange transfer summary
//! (ExchangeTransferSummary.tsx), and the annotation badges (AnnotationStrip).

use rune::{Color, El, col, row, text};

use super::Ctx;
use super::data::{LegKey, Payload, SwapRoute};
use super::model;
use crate::app::App;
use crate::format;
use crate::i18n::{has, t};
use crate::icons as I;
use crate::ui::ic;

/// The exchange summary for an exchange deposit or withdrawal, else the route card
/// (route.rs): one transaction, a swap, or a link across two transactions.
pub fn route(
    c: &Ctx,
    record: &serde_json::Value,
    r: &SwapRoute,
    leg: Option<LegKey>,
    on_select: impl Fn(&mut App, LegKey) + Clone + 'static,
    layer: &mut Vec<El>,
    width: f32,
) -> El {
    if let Some((direction, exchange_leg, chain_leg)) = model::exchange_transfer(r) {
        return exchange_summary(c, r, direction, exchange_leg, chain_leg);
    }
    super::route::card(c, record, r, leg, on_select, layer, width)
}

fn exchange_summary(
    c: &Ctx,
    r: &SwapRoute,
    direction: &str,
    exchange_leg: LegKey,
    chain_leg: LegKey,
) -> El {
    let tk = c.tk;
    let hidden = t("transactions:graph.hidden");
    let chain = r.leg(chain_leg);
    let exchange = r.leg(exchange_leg);
    let txid = chain
        .txid
        .clone()
        .filter(|x| x.len() == 64 && x.chars().all(|c| c.is_ascii_hexdigit()));
    let side = |key: LegKey| {
        let leg = r.leg(key);
        let is_exchange = key == exchange_leg;
        let label = leg
            .wallet
            .label
            .clone()
            .unwrap_or_else(|| t("transactions:graph.swapRouteUnknownLeg"));
        let amount = match leg.amount_btc {
            Some(a) => format!(
                "{} {}",
                format::btc(a, 8, false).trim_start_matches("₿ "),
                leg.asset.clone().unwrap_or_else(|| "BTC".into())
            ),
            None => t("transactions:details.unknown"),
        };
        col()
            .grow(1.0)
            .min_w(0.0)
            .padding(tk.r(0.75))
            .gap(tk.r(0.25))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.background)
            .child(
                row()
                    .gap(tk.r(0.5))
                    .center_y()
                    .child(ic(
                        if is_exchange {
                            &I::BUILDING_2
                        } else {
                            &I::WALLET
                        },
                        tk.r(1.0),
                        tk.muted_fg,
                    ))
                    .child(
                        text(t(if is_exchange {
                            "transactions:exchangeTransfer.account"
                        } else {
                            "transactions:exchangeTransfer.wallet"
                        }))
                        .font(tk.txs(400.0))
                        .color(tk.muted_fg),
                    ),
            )
            .child(
                text(if c.hide { hidden.clone() } else { label })
                    .font(tk.tsm(500.0))
                    .color(tk.fg)
                    .wrap(),
            )
            .child(
                text(if c.hide { hidden.clone() } else { amount })
                    .font(tk.tsm(400.0).mono())
                    .color(tk.fg),
            )
    };
    let mut record = row().gap(tk.r(0.25)).child(
        text(t("transactions:exchangeTransfer.chainRecord"))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
    );
    if let Some(x) = &txid {
        record = record.child(
            text(format!(
                "· {}",
                if c.hide {
                    hidden.clone()
                } else {
                    model::short(x)
                }
            ))
            .font(tk.txs(400.0).mono())
            .color(tk.muted_fg),
        );
    }
    let mut out = col()
        .key("graph-exchange")
        .w_full()
        .gap(tk.r(0.75))
        .child(
            text(t(&format!("transactions:exchangeTransfer.{direction}")))
                .font(tk.tsm(500.0))
                .color(tk.fg),
        )
        .child(
            row()
                .w_full()
                .gap(tk.r(0.75))
                .center_y()
                .child(side(LegKey::Out))
                .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg))
                .child(side(LegKey::In)),
        )
        .child(record);
    if let Some(fee) = exchange.fee_btc.filter(|f| f.is_finite() && *f > 0.0) {
        let v = if c.hide {
            hidden.clone()
        } else {
            format!(
                "{} {}",
                format::btc(fee, 8, false).trim_start_matches("₿ "),
                exchange.asset.clone().unwrap_or_else(|| "BTC".into())
            )
        };
        out = out.child(
            text(format!("{}: {v}", t("transactions:exchangeTransfer.fee")))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    }
    let reference = exchange
        .external_id
        .clone()
        .map(|s| s.trim().to_owned())
        .filter(|s| {
            !s.is_empty() && Some(s.to_lowercase()) != txid.as_ref().map(|x| x.to_lowercase())
        });
    if let Some(reference) = reference {
        out = out.child(
            text(format!(
                "{}: {}",
                t("transactions:exchangeTransfer.reference"),
                if c.hide { hidden } else { reference }
            ))
            .font(tk.txs(400.0).mono())
            .color(tk.muted_fg)
            .wrap_anywhere(),
        );
    }
    out
}

/// At most eight badges, repeats collapsed into " · N".
pub fn annotations(c: &Ctx, p: &Payload) -> Option<El> {
    let tk = c.tk;
    let mut badges: Vec<(String, bool, usize)> = Vec::new();
    for a in &p.annotations {
        let key = format!("transactions:graph.annotations.{}", a.code.trim());
        let label = if has(&key) {
            t(&key)
        } else {
            a.label.clone().unwrap_or_else(|| a.code.clone())
        };
        if label.is_empty() {
            continue;
        }
        if let Some(b) = badges.iter_mut().find(|b| b.0 == label) {
            b.2 += 1;
        } else if badges.len() < 8 {
            badges.push((label, a.severity.as_deref() == Some("warning"), 1));
        }
    }
    if badges.is_empty() {
        return None;
    }
    let mut r = row().key("graph-annotations").flex_wrap().gap(tk.r(0.375));
    for (label, warning, count) in badges {
        let text_value = if count > 1 {
            format!("{label} · {}", format::count(count))
        } else {
            label
        };
        let (fill, fg) = if warning {
            (tk.destructive, Color::WHITE)
        } else {
            (tk.muted, tk.fg)
        };
        r = r.child(
            row()
                .px(tk.r(0.5))
                .py(2.0)
                .radius(tk.r(0.375))
                .fill(fill)
                .child(text(text_value).font(tk.txs(500.0)).color(fg)),
        );
    }
    Some(r)
}
