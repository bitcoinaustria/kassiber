//! Rows, labels and display text for graph legs (TransactionGraphModel.ts, the label helpers in
//! TransactionGraphTab.tsx, ExchangeTransferModel.ts, lib/explorer.ts).

use super::data::{LegKey, Node, Payload, SwapRoute};
use crate::format;
use crate::i18n::{t, tc, tr};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Input,
    Output,
    Fee,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub node: Node,
    pub side: Side,
}

impl Row {
    /// `graphPart`: one name per leg across drawing and list; the fee counts as an output.
    pub fn part(&self) -> String {
        part(self.side, &self.node.id)
    }
}

pub fn part(side: Side, id: &str) -> String {
    format!(
        "{}:{id}",
        if side == Side::Input {
            "input"
        } else {
            "output"
        }
    )
}

pub const MAX_COMPACT_ROWS: usize = 24;

/// `compactGraphRows`: past `max` rows, keep `max - 1` and fold the rest into "+N more".
pub fn compact(nodes: &[Node], side: Side, max: usize) -> Vec<Row> {
    let rows: Vec<Row> = nodes
        .iter()
        .map(|n| Row {
            node: n.clone(),
            side,
        })
        .collect();
    if rows.len() <= max {
        return rows;
    }
    let keep = max.saturating_sub(1).max(1);
    let hidden = &rows[keep..];
    let count: f64 = hidden
        .iter()
        .map(|r| r.node.overflow_count.unwrap_or(1.0))
        .sum();
    let all_known = hidden.iter().all(|r| r.node.value_sats.is_some());
    let total = all_known.then(|| {
        hidden
            .iter()
            .map(|r| r.node.value_sats.unwrap_or(0.0))
            .sum::<f64>()
    });
    let name = if side == Side::Input {
        "input"
    } else {
        "output"
    };
    let mut out = rows[..keep].to_vec();
    out.push(Row {
        side,
        node: Node {
            id: format!("{name}-overflow"),
            label: Some(format!("+{count} more")),
            role: Some("overflow".into()),
            ownership: Some("overflow".into()),
            overflow: true,
            overflow_count: Some(count),
            value_sats: total,
            value_btc: total.map(|s| s / 1e8),
            ..Default::default()
        },
    });
    out
}

fn redact(rows: &[Row]) -> Vec<Row> {
    rows.iter()
        .map(|r| {
            let mut n = r.node.clone();
            n.value_sats = None;
            n.value_btc = None;
            if !matches!(n.state(), "confidential" | "other_asset") {
                n.value_state = Some("missing".into());
            }
            Row {
                node: n,
                side: r.side,
            }
        })
        .collect()
}

/// `graphLayoutRows`: the legs a drawing shows (fee first on Bitcoin, last on Liquid).
pub struct LayoutRows {
    pub inputs: Vec<Row>,
    pub destinations: Vec<Row>,
    pub layout_inputs: Vec<Row>,
    pub layout_destinations: Vec<Row>,
    pub total_inputs: Vec<Row>,
    pub total_destinations: Vec<Row>,
}

pub fn layout_rows(g: &Payload, hide: bool, max: usize) -> LayoutRows {
    let fee = g.fee.as_ref().map(|f| Row {
        node: f.clone(),
        side: Side::Fee,
    });
    let liquid = g.is_liquid();
    let with_fee = |mut rows: Vec<Row>| {
        if let Some(f) = &fee {
            if liquid {
                rows.push(f.clone());
            } else {
                rows.insert(0, f.clone());
            }
        }
        rows
    };
    let inputs = compact(&g.inputs, Side::Input, max);
    let destinations = with_fee(compact(
        &g.outputs,
        Side::Output,
        if fee.is_some() {
            max.saturating_sub(1).max(1)
        } else {
            max
        },
    ));
    let maybe = |rows: &[Row]| if hide { redact(rows) } else { rows.to_vec() };
    let all_in: Vec<Row> = g
        .inputs
        .iter()
        .map(|n| Row {
            node: n.clone(),
            side: Side::Input,
        })
        .collect();
    let all_out = with_fee(
        g.outputs
            .iter()
            .map(|n| Row {
                node: n.clone(),
                side: Side::Output,
            })
            .collect(),
    );
    LayoutRows {
        layout_inputs: maybe(&inputs),
        layout_destinations: maybe(&destinations),
        total_inputs: maybe(&all_in),
        total_destinations: maybe(&all_out),
        inputs,
        destinations,
    }
}

/// `formatShortTxid`.
pub fn short(s: &str) -> String {
    if s.chars().count() <= 18 {
        return s.to_owned();
    }
    let head: String = s.chars().take(10).collect();
    let tail: String = s
        .chars()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}...{tail}")
}

/// `nodeTooltipTitle` (with the overflow label of `nodeDisplayTitle`).
pub fn title(n: &Node) -> String {
    if n.overflow {
        return tc(
            "transactions:graph.overflowMore",
            n.overflow_count.unwrap_or(0.0) as usize,
            &[],
        );
    }
    let raw = [&n.wallet, &n.address, &n.outpoint, &n.label]
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty())
        .cloned()
        .unwrap_or_else(|| "Transaction leg".into());
    if n.outpoint.as_deref() == Some(raw.as_str()) || raw.chars().count() > 48 {
        return short(&raw);
    }
    raw
}

pub fn role_label(role: Option<&str>) -> String {
    let key = match role.unwrap_or("") {
        "input" => "input",
        "output" => "output",
        "change" => "change",
        "external_recipient" => "externalRecipient",
        "incoming_payment" => "incomingPayment",
        "incoming_payment_candidate" => "incomingPaymentCandidate",
        "owned_return" => "ownedReturn",
        "owned_destination" => "ownedDestination",
        "op_return" => "opReturn",
        "coinbase" => "coinbase",
        "peg_in" => "pegIn",
        "peg_out" => "pegOut",
        "fee" => "fee",
        "overflow" => "overflow",
        "ambiguous_owned_output" => "ambiguousOwnedOutput",
        "" => return t("transactions:graph.roles.leg"),
        other => return other.replace('_', " "),
    };
    t(&format!("transactions:graph.roles.{key}"))
}

/// `ownershipBoundaryLabel`.
pub fn ownership_label(n: &Node) -> String {
    let key = match (n.role.as_deref(), n.ownership.as_deref()) {
        (Some("fee"), _) | (_, Some("network_fee")) => "networkFee",
        (Some("change"), Some("owned")) => "internalChange",
        (_, Some("owned")) => "internalWallet",
        (_, Some("external")) => "externalWallet",
        (_, Some("ambiguous")) => "ambiguousWallet",
        (_, Some("unspendable")) => "unspendable",
        (_, Some("peg_out")) => "pegOut",
        (_, Some("coinbase")) => "coinbase",
        (_, Some("overflow")) => "aggregated",
        _ => "unknown",
    };
    t(&format!("transactions:graph.ownership.{key}"))
}

/// The amount a leg shows (`formatNodeAmount`); `fiat` = the transaction's own EUR price when
/// the app shows fiat.
pub fn amount(n: &Node, hide: bool, fiat: Option<f64>) -> String {
    if hide {
        return t("transactions:graph.hidden");
    }
    match n.state() {
        "confidential" => return t("transactions:graph.confidentialAmount"),
        "other_asset" => {
            return match &n.asset {
                Some(a) => tr("transactions:graph.otherAssetNamed", &[("asset", a)]),
                None => t("transactions:graph.otherAsset"),
            };
        }
        _ => {}
    }
    match n.value_btc {
        Some(btc) => money(btc, fiat),
        None => String::new(),
    }
}

pub fn money(btc: f64, fiat: Option<f64>) -> String {
    match fiat {
        Some(p) => format::fiat(btc * p, "EUR"),
        None => format::btc(btc, 8, false),
    }
}

pub fn has_known_btc(n: &Node) -> bool {
    !matches!(n.state(), "confidential" | "other_asset") && n.value_btc.is_some()
}

/// `displayReference`: a row identifies a leg by address first.
pub fn display_reference(n: &Node) -> String {
    [&n.address, &n.outpoint, &n.txid, &n.label]
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty())
        .cloned()
        .unwrap_or_default()
}

/// `copyReference`: the strand's copy target, never a synthetic label.
pub fn copy_reference(n: &Node) -> Option<String> {
    [&n.outpoint, &n.address, &n.txid]
        .into_iter()
        .flatten()
        .find(|v| !v.is_empty())
        .cloned()
}

pub fn detail_reference(n: &Node, hide: bool) -> String {
    let r = display_reference(n);
    if r.is_empty() {
        return t("transactions:graph.inputsOutputs.unknownReference");
    }
    if hide {
        t("transactions:graph.hidden")
    } else {
        short(&r)
    }
}

fn concise_script(s: Option<&str>) -> String {
    let Some(s) = s else { return String::new() };
    let norm = s
        .replace(['_', '-'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let lower = norm.to_lowercase();
    if lower == "unknown" {
        String::new()
    } else if lower.contains("taproot") {
        "taproot".into()
    } else if lower.contains("witness v0") && lower.contains("keyhash") {
        "segwit v0".into()
    } else if lower.contains("witness v0") && lower.contains("scripthash") {
        "segwit script".into()
    } else {
        norm
    }
}

fn block_distance(n: &Node, side: Side, height: Option<f64>) -> Option<String> {
    let h = height?;
    let other = if side == Side::Input {
        n.prevout_block_height
    } else {
        n.spent_by_block_height
    }?;
    let delta = if side == Side::Input {
        h - other
    } else {
        other - h
    };
    if delta < 0.0 {
        return None;
    }
    if delta == 0.0 {
        return Some(t("transactions:graph.inputsOutputs.sameBlock"));
    }
    let key = if side == Side::Input {
        "transactions:graph.inputsOutputs.blocksEarlier"
    } else {
        "transactions:graph.inputsOutputs.blocksLater"
    };
    Some(tc(key, delta as usize, &[]))
}

/// `nodeDetailMeta`.
pub fn detail_meta(n: &Node, side: Side, hide: bool, height: Option<f64>) -> Vec<String> {
    let mut parts = Vec::new();
    let side_name = if side == Side::Input {
        "input"
    } else {
        "output"
    };
    if let Some(r) = n.role.as_deref().filter(|r| *r != side_name) {
        parts.push(role_label(Some(r)));
    }
    parts.push(ownership_label(n));
    let script = concise_script(n.script_type.as_deref());
    if !script.is_empty() {
        parts.push(script);
    }
    if let Some(i) = n.index {
        parts.push(tr(
            "transactions:graph.inputsOutputs.index",
            &[("index", &format!("{}", i as i64))],
        ));
    }
    if !hide && let (Some(_), Some(op)) = (&n.address, &n.outpoint) {
        parts.push(short(op));
    }
    if let Some(sp) = &n.spent_by_txid {
        parts.push(if hide {
            t("transactions:graph.inputsOutputs.spent")
        } else {
            tr(
                "transactions:graph.inputsOutputs.spentBy",
                &[("reference", &short(sp))],
            )
        });
    }
    if let Some(d) = block_distance(n, side, height) {
        parts.push(d);
    }
    parts
}

/// `amountSummary`.
pub struct Summary {
    pub known_sats: f64,
    pub known: usize,
    pub confidential: usize,
    pub other_asset: usize,
}

pub fn summary(nodes: &[Node]) -> Summary {
    let mut s = Summary {
        known_sats: 0.0,
        known: 0,
        confidential: 0,
        other_asset: 0,
    };
    for n in nodes {
        match n.state() {
            "confidential" => s.confidential += 1,
            "other_asset" => s.other_asset += 1,
            _ => {
                if let Some(v) = n.value_sats {
                    s.known += 1;
                    s.known_sats += v;
                }
            }
        }
    }
    s
}

pub fn complete_total(nodes: &[Node]) -> bool {
    nodes
        .iter()
        .all(|n| !matches!(n.state(), "confidential" | "other_asset") && n.value_sats.is_some())
}

pub fn format_total(nodes: &[Node], fiat: Option<f64>) -> String {
    let s = summary(nodes);
    if s.known > 0 {
        money(s.known_sats / 1e8, fiat)
    } else if s.confidential > 0 {
        t("transactions:graph.confidentialAmount")
    } else if s.other_asset > 0 {
        t("transactions:graph.otherAsset")
    } else {
        t("transactions:graph.inputsOutputs.unknownAmount")
    }
}

// ---- Routes ---------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RouteKind {
    Swap,
    Coinjoin,
    Transfer,
    Pair,
}

/// `pairedRouteKind` / `classifyRouteKind`.
pub fn route_kind(r: &SwapRoute) -> RouteKind {
    match r
        .route_kind
        .as_deref()
        .unwrap_or("")
        .to_lowercase()
        .as_str()
    {
        "swap" => return RouteKind::Swap,
        "coinjoin" => return RouteKind::Coinjoin,
        "transfer" => return RouteKind::Transfer,
        "pair" => return RouteKind::Pair,
        _ => {}
    }
    let text = r.kind.as_deref().unwrap_or("").to_lowercase();
    if text.contains("coinjoin") || text.contains("whirlpool") {
        return RouteKind::Coinjoin;
    }
    let out = r
        .out_leg
        .asset
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_uppercase();
    let into = r
        .in_leg
        .asset
        .as_deref()
        .unwrap_or("")
        .trim()
        .to_uppercase();
    if text.contains("swap")
        || text.starts_with("peg-")
        || (!out.is_empty() && !into.is_empty() && out != into)
    {
        return RouteKind::Swap;
    }
    if r.policy.as_deref() == Some("carrying-value") {
        RouteKind::Transfer
    } else {
        RouteKind::Pair
    }
}

/// Custody of a wallet kind (ExchangeTransferModel.ts over the connection catalog).
fn custody(kind: Option<&str>) -> Option<&'static str> {
    let k = kind?.trim().to_lowercase();
    const EXCHANGES: &[&str] = &[
        "kraken",
        "kraken-csv",
        "bitstamp",
        "bitpanda",
        "coinbase",
        "binance",
        "strike",
        "river",
        "bisq",
        "bullbitcoin",
        "relai",
        "pocket",
        "21bitcoin",
        "coinfinity",
        "swan",
        "exchange",
    ];
    const WALLETS: &[&str] = &[
        "descriptor",
        "xpub",
        "address",
        "core-ln",
        "coreln",
        "lnd",
        "nwc",
        "phoenix",
        "cashu",
        "silent-payment",
        "liquid",
        "btcpay",
    ];
    if EXCHANGES.contains(&k.as_str()) {
        Some("exchange")
    } else if WALLETS.contains(&k.as_str()) {
        Some("wallet")
    } else {
        None
    }
}

/// `exchangeTransfer`: (direction, exchange leg, chain leg).
pub fn exchange_transfer(r: &SwapRoute) -> Option<(&'static str, LegKey, LegKey)> {
    if route_kind(r) != RouteKind::Transfer {
        return None;
    }
    let (oa, ia) = (r.out_leg.asset.as_deref()?, r.in_leg.asset.as_deref()?);
    if oa != ia {
        return None;
    }
    match (
        custody(r.out_leg.wallet.kind.as_deref()),
        custody(r.in_leg.wallet.kind.as_deref()),
    ) {
        (Some("exchange"), Some("wallet")) => Some(("withdrawal", LegKey::Out, LegKey::In)),
        (Some("wallet"), Some("exchange")) => Some(("deposit", LegKey::In, LegKey::Out)),
        _ => None,
    }
}

/// `preloadableSwapLegGraphReference`: a leg's reference unless it names the current row.
pub fn preload_reference(r: &SwapRoute, leg: LegKey, current: &[String]) -> Option<String> {
    if let Some((_, _, chain)) = exchange_transfer(r)
        && chain != leg
    {
        return None;
    }
    let reference = r.leg(leg).reference().trim().to_owned();
    if reference.is_empty() || current.iter().any(|c| c.eq_ignore_ascii_case(&reference)) {
        return None;
    }
    Some(reference)
}

/// `publicLookupCanAddToGraph`.
pub fn lookup_can_add(g: &Payload) -> bool {
    if g.support_level == "graphless" {
        return true;
    }
    if g.is_liquid() {
        return false;
    }
    g.inputs.iter().any(|n| n.state() == "missing")
}

/// `hasPublicGraphLookupReference` on a transaction row.
pub fn has_lookup_reference(record: &serde_json::Value) -> bool {
    let explorer = record["explorerId"].as_str().unwrap_or("");
    if explorer.len() != 64 || !explorer.chars().all(|c| c.is_ascii_hexdigit()) {
        return false;
    }
    let chain = record["chain"].as_str().unwrap_or("").trim().to_lowercase();
    chain == "bitcoin"
        || chain == "liquid"
        || matches!(
            record["paymentMethod"].as_str(),
            Some("On-chain") | Some("Liquid")
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(id: &str, sats: Option<f64>) -> Node {
        Node {
            id: id.into(),
            value_sats: sats,
            value_state: Some(if sats.is_some() { "known" } else { "missing" }.into()),
            ..Default::default()
        }
    }

    #[test]
    fn compaction_folds_and_sums_only_when_known() {
        let nodes: Vec<Node> = (0..30)
            .map(|i| node(&format!("in-{i}"), Some(1.0)))
            .collect();
        let rows = compact(&nodes, Side::Input, 24);
        assert_eq!(rows.len(), 24);
        assert!(rows[23].node.overflow);
        assert_eq!(rows[23].node.overflow_count, Some(7.0));
        assert_eq!(rows[23].node.value_sats, Some(7.0));
        let mut nodes = nodes;
        nodes[29].value_sats = None;
        assert_eq!(compact(&nodes, Side::Input, 24)[23].node.value_sats, None);
    }

    #[test]
    fn fee_leads_on_bitcoin_and_trails_on_liquid() {
        let mut g = Payload {
            outputs: vec![node("out-0", Some(5.0))],
            fee: Some(node("fee", Some(1.0))),
            ..Default::default()
        };
        assert_eq!(layout_rows(&g, false, 24).destinations[0].side, Side::Fee);
        g.transaction = Some(super::super::data::TxInfo {
            chain: Some("liquid".into()),
            ..Default::default()
        });
        assert_eq!(layout_rows(&g, false, 24).destinations[1].side, Side::Fee);
        assert_eq!(
            layout_rows(&g, true, 24).layout_destinations[1]
                .node
                .value_sats,
            None
        );
    }

    #[test]
    fn short_and_titles() {
        assert_eq!(
            short("373b15e0a627488e80c3ad301d0fe61efdcfd3a2d8cf387b5577bb6faf3ea6cb"),
            "373b15e0a6...3ea6cb"
        );
        let n = Node {
            outpoint: Some("a".repeat(64) + ":1"),
            ..Default::default()
        };
        assert_eq!(title(&n), short(&("a".repeat(64) + ":1")));
    }
}
