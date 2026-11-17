//! An exchange trade without a chain transaction (TransactionRecordFlow.tsx,
//! `graphlessTradeKind` in TransactionGraphModel.ts): where the graph would be, a Sale or
//! Purchase receipt from the source record, Sold → Reported trade value or the reverse.
//!
//! The fiat side is shown only when the source itself priced the trade (`exchange_execution`,
//! exact, a three-letter currency); a market valuation or manual price is not evidence of the
//! counter-leg, so it reads "Not in the source record".

use rune::{El, col, row, text};
use serde_json::Value;

use super::Ctx;
use super::data::Payload;
use crate::format;
use crate::i18n::t;
use crate::icons as I;
use crate::ui::ic;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trade {
    Buy,
    Sell,
}

fn is_txid(v: Option<&str>) -> bool {
    v.is_some_and(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

fn str_of<'a>(r: &'a Value, k: &str) -> Option<&'a str> {
    r.get(k).and_then(Value::as_str).filter(|s| !s.is_empty())
}

/// `graphlessTradeKind`: a buy or sell row whose graph read is graphless, with no swap route
/// and no chain txid anywhere (graph, explorer id, the row's own id).
pub fn kind(record: &Value, p: &Payload) -> Option<Trade> {
    if p.support_level != "graphless" || p.swap_route.is_some() {
        return None;
    }
    let txn_id = str_of(record, "externalId").or_else(|| str_of(record, "id"));
    if [
        p.transaction.as_ref().and_then(|x| x.txid.as_deref()),
        str_of(record, "explorerId"),
        txn_id,
    ]
    .into_iter()
    .any(is_txid)
    {
        return None;
    }
    // `kindOverride ?? kind ?? sourceType` (the row's `type`).
    let k = record
        .get("kindOverride")
        .and_then(Value::as_str)
        .or_else(|| record.get("kind").and_then(Value::as_str))
        .or_else(|| record.get("type").and_then(Value::as_str))
        .unwrap_or("")
        .trim()
        .to_lowercase();
    match k.as_str() {
        "buy" => Some(Trade::Buy),
        "sell" | "sale" => Some(Trade::Sell),
        _ => None,
    }
}

fn num(r: &Value, k: &str) -> Option<f64> {
    r.get(k).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// `formatAssetAmount`: eight decimals and the asset code.
fn asset_amount(v: f64, asset: &str) -> String {
    format!(
        "{:.8} {}",
        v.abs(),
        if asset.is_empty() { "BTC" } else { asset }
    )
}

/// The receipt for `record` (the row as the list/resolve kinds return it).
pub fn flow(c: &Ctx, record: &Value, kind: Trade) -> El {
    let tk = c.tk;
    let hidden = t("transactions:graph.hidden");
    let asset = str_of(record, "asset").unwrap_or("BTC");
    // The table's `amountBtc` (a consolidation shows its fee) and `amount` (EUR value, else
    // amount × rate), both unsigned.
    let sat = num(record, "amountSat");
    let fee_sat = num(record, "feeSat").unwrap_or(0.0);
    let display_sat = sat.map(|s| {
        if str_of(record, "type") == Some("Consolidation") && s == 0.0 && fee_sat != 0.0 {
            fee_sat
        } else {
            s
        }
    });
    let btc = display_sat
        .map(|s| asset_amount(s / 1e8, asset))
        .unwrap_or_else(|| t("transactions:details.unknown"));
    let fiat_amount =
        num(record, "eur")
            .map(f64::abs)
            .or_else(|| match (display_sat, num(record, "rate")) {
                (Some(s), Some(rate)) => Some((s / 1e8 * rate).abs()),
                _ => None,
            });
    let currency = str_of(record, "fiatCurrency").unwrap_or("");
    let exact = str_of(record, "pricingSourceKind") == Some("exchange_execution")
        && str_of(record, "pricingQuality") == Some("exact")
        && currency.len() == 3
        && currency.chars().all(|ch| ch.is_ascii_uppercase());
    let fiat = match fiat_amount.filter(|_| exact) {
        Some(v) => format::fiat(v, currency),
        None => t("transactions:recordFlow.valueMissing"),
    };
    let crypto = (
        t(if kind == Trade::Sell {
            "transactions:recordFlow.sold"
        } else {
            "transactions:recordFlow.bought"
        }),
        btc,
    );
    let cash = (t("transactions:recordFlow.reportedValue"), fiat);
    let sides = if kind == Trade::Sell {
        [crypto, cash]
    } else {
        [cash, crypto]
    };
    let venue = str_of(record, "counter")
        .or_else(|| str_of(record, "account"))
        .map(str::to_owned)
        .unwrap_or_else(|| t("transactions:fallback.unassignedWallet"));
    let title = t(if kind == Trade::Sell {
        "transactions:recordFlow.sale"
    } else {
        "transactions:recordFlow.purchase"
    });
    // `flex items-center justify-between gap-3 text-sm`: the kind, the venue cut on the right.
    let venue_text = if c.hide { hidden.clone() } else { venue };
    let head = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .justify_between()
        .child(text(title).font(tk.tsm(500.0)).color(tk.fg).single_line())
        .child(
            text(venue_text.clone())
                .font(tk.tsm(400.0))
                .color(tk.muted_fg)
                .single_line()
                .truncate()
                .min_w(0.0)
                .shrink()
                .help_when_truncated(venue_text),
        );
    // `grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] gap-3`: two equal boxes, the arrow between.
    let side = |(label, value): (String, String)| {
        col()
            .grow(1.0)
            .w(0.0)
            .min_w(0.0)
            .padding(tk.r(0.75))
            .radius(tk.r(0.375))
            .border(1.0, tk.border)
            .fill(tk.background)
            .child(
                text(label)
                    .font(tk.txs(400.0))
                    .color(tk.muted_fg)
                    .wrap()
                    .w_full(),
            )
            .child(
                text(if c.hide { hidden.clone() } else { value })
                    .font(tk.tsm(400.0).mono().features("'tnum' 1"))
                    .color(tk.fg)
                    .wrap_anywhere()
                    .w_full()
                    .mt(tk.r(0.25)),
            )
    };
    let [a, b] = sides;
    let boxes = row()
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .center_y()
        .child(side(a))
        .child(ic(&I::ARROW_RIGHT, tk.r(1.0), tk.muted_fg).a11y_hidden())
        .child(side(b));
    // `flex flex-wrap justify-between gap-2 text-xs`: no chain transaction, the reported fee.
    let mut foot = row()
        .w_full()
        .flex_wrap()
        .gaps(tk.r(0.5), tk.r(0.5))
        .justify_between()
        .child(
            text(t("transactions:recordFlow.noChainTransaction"))
                .font(tk.txs(400.0))
                .color(tk.muted_fg),
        );
    if fee_sat != 0.0 {
        let fee = if c.hide {
            hidden.clone()
        } else {
            asset_amount(fee_sat / 1e8, asset)
        };
        foot = foot.child(
            text(format!(
                "{}: {fee}",
                t("transactions:recordFlow.reportedFee")
            ))
            .font(tk.txs(400.0))
            .color(tk.muted_fg),
        );
    }
    col()
        .key(format!("graph-record-flow{}", c.sfx))
        .w_full()
        .min_w(0.0)
        .gap(tk.r(0.75))
        .child(head)
        .child(boxes)
        .child(foot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn graphless() -> Payload {
        Payload::parse(&json!({"transaction": {"id": "sale"}, "supportLevel": "graphless", "inputs": [], "outputs": []})).unwrap()
    }

    #[test]
    fn trades_follow_the_web_rule() {
        let sale = json!({"id": "sale", "externalId": "exchange:sale", "kind": "sell", "type": "Withdrawal"});
        assert_eq!(kind(&sale, &graphless()), Some(Trade::Sell));
        assert_eq!(
            kind(&json!({"id": "b", "kind": "buy"}), &graphless()),
            Some(Trade::Buy)
        );
        // A chain reference, a drawable graph or another kind keeps the graph path.
        assert_eq!(
            kind(
                &json!({"id": "s", "kind": "sell", "explorerId": "a".repeat(64)}),
                &graphless()
            ),
            None
        );
        let mut partial = graphless();
        partial.support_level = "partial".into();
        assert_eq!(kind(&sale, &partial), None);
        assert_eq!(
            kind(
                &json!({"id": "s", "kind": "sell", "kindOverride": "withdrawal"}),
                &graphless()
            ),
            None
        );
    }
}
