//! The `NodeSnapshot` shape (mocks/seed.ts) as the daemon sends it, and the formatting helpers
//! of routes/NodeConnectionDetail.tsx (`fmtSat`, `fmtEur`, `fmtPubkey`, `relativeFrom`, tones).

use rune::Color;
use serde_json::Value;

use crate::i18n::t;
use crate::theme::Tk;

fn s(v: &Value, k: &str) -> Option<String> {
    v.get(k).and_then(Value::as_str).map(str::to_owned)
}
fn f(v: &Value, k: &str) -> Option<f64> {
    v.get(k).and_then(Value::as_f64)
}
fn b(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}

#[derive(Clone, Debug)]
pub struct Channel {
    pub id: String,
    pub short_channel_id: Option<String>,
    pub funding_outpoint: Option<String>,
    pub peer_alias: String,
    pub peer_pubkey: Option<String>,
    pub capacity_sat: f64,
    pub local_balance_sat: f64,
    pub remote_balance_sat: f64,
    /// active | inactive | pending_open | pending_close | closed | force_closed
    pub state: String,
    pub is_private: bool,
    pub is_initiator: bool,
    pub base_fee_msat: Option<f64>,
    pub fee_rate_ppm: Option<f64>,
    pub opened_at: Option<String>,
    pub closed_at: Option<String>,
    pub close_kind: Option<String>,
    pub forward_count: Option<f64>,
    pub earned_routing_sat: Option<f64>,
    pub htlc_count: Option<f64>,
    pub last_activity_at: Option<String>,
}

impl Channel {
    fn parse(v: &Value) -> Channel {
        Channel {
            id: s(v, "id").unwrap_or_default(),
            short_channel_id: s(v, "shortChannelId"),
            funding_outpoint: s(v, "fundingOutpoint"),
            peer_alias: s(v, "peerAlias").unwrap_or_default(),
            peer_pubkey: s(v, "peerPubkey"),
            capacity_sat: f(v, "capacitySat").unwrap_or(0.0),
            local_balance_sat: f(v, "localBalanceSat").unwrap_or(0.0),
            remote_balance_sat: f(v, "remoteBalanceSat").unwrap_or(0.0),
            state: s(v, "state").unwrap_or_else(|| "inactive".into()),
            is_private: b(v, "isPrivate"),
            is_initiator: b(v, "isInitiator"),
            base_fee_msat: f(v, "baseFeeMsat"),
            fee_rate_ppm: f(v, "feeRatePpm"),
            opened_at: s(v, "openedAt"),
            closed_at: s(v, "closedAt"),
            close_kind: s(v, "closeKind"),
            forward_count: f(v, "forwardCount"),
            earned_routing_sat: f(v, "earnedRoutingSat"),
            htlc_count: f(v, "htlcCount"),
            last_activity_at: s(v, "lastActivityAt"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Forward {
    pub occurred_at: String,
    pub in_peer_alias: String,
    pub in_short_channel_id: Option<String>,
    pub out_peer_alias: String,
    pub out_short_channel_id: Option<String>,
    pub amount_in_msat: f64,
    pub fee_msat: f64,
    /// settled | failed | offered
    pub status: String,
    pub failure_reason: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Routing {
    pub window_label: String,
    pub routing_revenue_sat: f64,
    pub payment_cost_sat: f64,
    pub rebalance_cost_sat: f64,
    pub onchain_cost_sat: f64,
    pub net_profit_sat: f64,
    pub forward_count: f64,
    pub payment_count: f64,
    pub rebalance_count: f64,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub alias: String,
    pub pubkey: String,
    pub network: String,
    pub implementation_version: Option<String>,
    pub peer_count: f64,
    pub block_height: Option<f64>,
    pub invoice_count: Option<f64>,
    pub paid_invoice_count: Option<f64>,
    pub expired_invoice_count: Option<f64>,
    pub payment_count: Option<f64>,
    pub completed_payment_count: Option<f64>,
    pub failed_payment_count: Option<f64>,
    pub onchain_balance_sat: f64,
    pub total_local_balance_sat: f64,
    pub total_remote_balance_sat: f64,
    pub total_capacity_sat: f64,
    pub channels: Vec<Channel>,
    pub closed_channels: Vec<Channel>,
    pub routing: Option<Routing>,
    pub forwards: Option<Vec<Forward>>,
}

impl Node {
    pub fn parse(v: &Value) -> Node {
        let chans = |k: &str| {
            v.get(k)
                .and_then(Value::as_array)
                .map(|a| a.iter().map(Channel::parse).collect())
                .unwrap_or_default()
        };
        Node {
            alias: s(v, "alias").unwrap_or_default(),
            pubkey: s(v, "pubkey").unwrap_or_default(),
            network: s(v, "network").unwrap_or_default(),
            implementation_version: s(v, "implementationVersion").filter(|x| !x.is_empty()),
            peer_count: f(v, "peerCount").unwrap_or(0.0),
            block_height: f(v, "blockHeight"),
            invoice_count: f(v, "invoiceCount"),
            paid_invoice_count: f(v, "paidInvoiceCount"),
            expired_invoice_count: f(v, "expiredInvoiceCount"),
            payment_count: f(v, "paymentCount"),
            completed_payment_count: f(v, "completedPaymentCount"),
            failed_payment_count: f(v, "failedPaymentCount"),
            onchain_balance_sat: f(v, "onchainBalanceSat").unwrap_or(0.0),
            total_local_balance_sat: f(v, "totalLocalBalanceSat").unwrap_or(0.0),
            total_remote_balance_sat: f(v, "totalRemoteBalanceSat").unwrap_or(0.0),
            total_capacity_sat: f(v, "totalCapacitySat").unwrap_or(0.0),
            channels: chans("channels"),
            closed_channels: chans("closedChannels"),
            routing: v.get("routing").filter(|r| r.is_object()).map(|r| Routing {
                window_label: s(r, "windowLabel").unwrap_or_default(),
                routing_revenue_sat: f(r, "routingRevenueSat").unwrap_or(0.0),
                payment_cost_sat: f(r, "paymentCostSat").unwrap_or(0.0),
                rebalance_cost_sat: f(r, "rebalanceCostSat").unwrap_or(0.0),
                onchain_cost_sat: f(r, "onchainCostSat").unwrap_or(0.0),
                net_profit_sat: f(r, "netProfitSat").unwrap_or(0.0),
                forward_count: f(r, "forwardCount").unwrap_or(0.0),
                payment_count: f(r, "paymentCount").unwrap_or(0.0),
                rebalance_count: f(r, "rebalanceCount").unwrap_or(0.0),
            }),
            forwards: v.get("forwards").and_then(Value::as_array).map(|a| {
                a.iter()
                    .map(|x| Forward {
                        occurred_at: s(x, "occurredAt").unwrap_or_default(),
                        in_peer_alias: s(x, "inPeerAlias").unwrap_or_default(),
                        in_short_channel_id: s(x, "inShortChannelId"),
                        out_peer_alias: s(x, "outPeerAlias").unwrap_or_default(),
                        out_short_channel_id: s(x, "outShortChannelId"),
                        amount_in_msat: f(x, "amountInMsat").unwrap_or(0.0),
                        fee_msat: f(x, "feeMsat").unwrap_or(0.0),
                        status: s(x, "status").unwrap_or_else(|| "offered".into()),
                        failure_reason: s(x, "failureReason"),
                    })
                    .collect()
            }),
        }
    }

    /// `node.routing?.forwardCount ?? node.forwards?.length ?? 0`.
    pub fn forward_total(&self) -> f64 {
        self.routing
            .as_ref()
            .map(|r| r.forward_count)
            .or(self.forwards.as_ref().map(|f| f.len() as f64))
            .unwrap_or(0.0)
    }

    pub fn active_count(&self) -> usize {
        self.channels.iter().filter(|c| c.state == "active").count()
    }
}

/// `DEFAULT_OPEN_COST_SAT` (lib/lightning.ts).
pub const DEFAULT_OPEN_COST_SAT: f64 = 2_500.0;

/// `formatCount`: rounded, grouped in the UI locale, signed when negative.
pub fn count(v: f64) -> String {
    let n = v.round();
    let body = crate::format::count(n.abs() as usize);
    if n < 0.0 { format!("-{body}") } else { body }
}

/// `fmtSat`: `formatSats(value, { unit: "sat" })`.
pub fn sat(v: f64) -> String {
    format!("{} sat", count(v))
}

/// `fmtSatSigned`: `+ 1,234 sat` / `- 1,234 sat`.
pub fn sat_signed(v: f64) -> String {
    format!("{}{}", if v >= 0.0 { "+ " } else { "- " }, sat(v.abs()))
}

/// `fmtBtc`: `₿ 0.00000000`.
pub fn btc(v: f64) -> String {
    format!("₿ {v:.8}")
}

/// `fmtEur`: always `de-AT` with two decimals (`€ 1.234,56`), `Unpriced` when unknown.
pub fn eur(v: Option<f64>) -> String {
    let Some(v) = v.filter(|v| v.is_finite()) else {
        return "Unpriced".into();
    };
    let cents = (v.abs() * 100.0).round() as u128;
    let whole = (cents / 100).to_string();
    let mut grouped = String::new();
    for (i, ch) in whole.chars().enumerate() {
        if i > 0 && (whole.len() - i).is_multiple_of(3) {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    format!(
        "€ {}{grouped},{:02}",
        if v < 0.0 && cents > 0 { "-" } else { "" },
        cents % 100
    )
}

/// `fmtPubkey`: first 8 … last 6 when longer than 18.
pub fn pubkey(v: &str) -> String {
    crate::format::compact_id(v)
}

/// `relativeFrom`: `12s ago` / `5m ago` / `3h ago` / `2d ago` (rounded, English as the web).
pub fn relative(v: Option<&str>) -> Option<String> {
    let ts = crate::format::parse_ts(v?)?;
    let d = (crate::format::now_secs() - ts).max(0.0);
    Some(if d < 60.0 {
        format!("{}s ago", d.round())
    } else if d < 3600.0 {
        format!("{}m ago", (d / 60.0).round())
    } else if d < 86_400.0 {
        format!("{}h ago", (d / 3600.0).round())
    } else {
        format!("{}d ago", (d / 86_400.0).round())
    })
}

pub fn short_date(v: Option<&str>) -> String {
    crate::format::short_date(v)
}

pub fn state_label(state: &str) -> String {
    t(match state {
        "active" => "connections:node.channelState.active",
        "pending_open" => "connections:node.channelState.pendingOpen",
        "pending_close" => "connections:node.channelState.pendingClose",
        "closed" => "connections:node.channelState.closed",
        "force_closed" => "connections:node.channelState.forceClosed",
        _ => "connections:node.channelState.inactive",
    })
}

pub fn close_kind_label(kind: Option<&str>) -> String {
    t(match kind {
        Some("cooperative") => "connections:node.closeKind.cooperative",
        Some("force") => "connections:node.closeKind.force",
        Some("breach") => "connections:node.closeKind.breach",
        _ => "connections:node.channels.closedFallback",
    })
}

pub fn forward_status_label(status: &str) -> String {
    t(match status {
        "settled" => "connections:node.forwardStatus.settled",
        "failed" => "connections:node.forwardStatus.failed",
        _ => "connections:node.forwardStatus.offered",
    })
}

/// `https://mempool.space/tx/<txid>` for a funding outpoint whose txid looks real.
pub fn explorer_href(outpoint: Option<&str>) -> Option<String> {
    let txid = outpoint?.split(':').next()?;
    (txid.len() >= 32).then(|| format!("https://mempool.space/tx/{txid}"))
}

fn h(s: &str) -> Color {
    rune::color::hex(s).expect("hex")
}

/// A ring-inset pill's (fill, text, ring).
#[derive(Clone, Copy)]
pub struct Pill {
    pub fill: Color,
    pub ink: Color,
    pub ring: Color,
}

pub fn emerald(tk: &Tk) -> Pill {
    if tk.dark {
        Pill {
            fill: h("#064e3b").alpha(0.3),
            ink: h("#6ee7b7"),
            ring: h("#34d399").alpha(0.2),
        }
    } else {
        Pill {
            fill: h("#ecfdf5"),
            ink: h("#047857"),
            ring: h("#059669").alpha(0.2),
        }
    }
}
pub fn amber(tk: &Tk) -> Pill {
    if tk.dark {
        Pill {
            fill: h("#78350f").alpha(0.3),
            ink: h("#fcd34d"),
            ring: h("#fbbf24").alpha(0.2),
        }
    } else {
        Pill {
            fill: h("#fffbeb"),
            ink: h("#b45309"),
            ring: h("#d97706").alpha(0.2),
        }
    }
}
pub fn red(tk: &Tk) -> Pill {
    if tk.dark {
        Pill {
            fill: h("#7f1d1d").alpha(0.3),
            ink: h("#fca5a5"),
            ring: h("#f87171").alpha(0.2),
        }
    } else {
        Pill {
            fill: h("#fef2f2"),
            ink: h("#b91c1c"),
            ring: h("#dc2626").alpha(0.2),
        }
    }
}
pub fn neutral(tk: &Tk) -> Pill {
    Pill {
        fill: tk.muted,
        ink: tk.muted_fg,
        ring: tk.border,
    }
}
/// The count chip (`bg-gray-50 text-gray-600 ring-gray-500/10`, dark `gray-800/50 gray-400 gray-400/20`).
pub fn gray(tk: &Tk) -> Pill {
    if tk.dark {
        Pill {
            fill: h("#1f2937").alpha(0.5),
            ink: h("#9ca3af"),
            ring: h("#9ca3af").alpha(0.2),
        }
    } else {
        Pill {
            fill: h("#f9fafb"),
            ink: h("#4b5563"),
            ring: h("#6b7280").alpha(0.1),
        }
    }
}

/// `channelStateTone`.
pub fn state_tone(tk: &Tk, state: &str) -> Pill {
    match state {
        "active" => emerald(tk),
        "pending_open" | "pending_close" => amber(tk),
        "force_closed" => red(tk),
        _ => neutral(tk),
    }
}

/// `forwardStatusTone`.
pub fn forward_tone(tk: &Tk, status: &str) -> Pill {
    match status {
        "settled" => emerald(tk),
        "failed" => red(tk),
        _ => amber(tk),
    }
}

/// `text-emerald-700 dark:text-emerald-300` and friends.
pub fn ink_emerald(tk: &Tk) -> Color {
    if tk.dark { h("#6ee7b7") } else { h("#047857") }
}
pub fn ink_sky(tk: &Tk) -> Color {
    if tk.dark { h("#7dd3fc") } else { h("#0369a1") }
}
pub fn ink_red(tk: &Tk) -> Color {
    if tk.dark { h("#fca5a5") } else { h("#b91c1c") }
}
/// Liquidity bar fills (`bg-emerald-500 dark:bg-emerald-400/80`, `bg-sky-500 dark:bg-sky-400/80`).
pub fn bar_local(tk: &Tk) -> Color {
    if tk.dark {
        h("#34d399").alpha(0.8)
    } else {
        h("#10b981")
    }
}
pub fn bar_remote(tk: &Tk) -> Color {
    if tk.dark {
        h("#38bdf8").alpha(0.8)
    } else {
        h("#0ea5e9")
    }
}
/// Forward route arrows (`text-emerald-600 dark:text-emerald-400`, `text-sky-600 dark:text-sky-400`).
pub fn arrow_in(tk: &Tk) -> Color {
    if tk.dark { h("#34d399") } else { h("#059669") }
}
pub fn arrow_out(tk: &Tk) -> Color {
    if tk.dark { h("#38bdf8") } else { h("#0284c7") }
}

/// A SYNTHETIC node snapshot for headless scenes only: the demo book's Lightning wallet has
/// never synced (its snapshot is fetched from the node, which is egress), so the populated
/// states are shown from this fake regtest data. Every key/alias is invented.
pub fn synthetic() -> Value {
    let now = crate::format::now_secs();
    let iso = |ago: f64| {
        let t = (now - ago) as i64;
        let day = crate::format::Day::from_epoch_days(t.div_euclid(86_400));
        let sod = t.rem_euclid(86_400);
        format!(
            "{}T{:02}:{:02}:{:02}Z",
            day.iso(),
            sod / 3600,
            sod % 3600 / 60,
            sod % 60
        )
    };
    serde_json::json!({
        "alias": "regtest-lnd-demo",
        "pubkey": "02aa11bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66",
        "network": "regtest",
        "implementationVersion": "0.18.4-beta",
        "peerCount": 4,
        "blockHeight": 2112,
        "invoiceCount": 42,
        "paidInvoiceCount": 37,
        "expiredInvoiceCount": 3,
        "paymentCount": 19,
        "completedPaymentCount": 17,
        "failedPaymentCount": 2,
        "onchainBalanceSat": 1250000,
        "totalLocalBalanceSat": 6400000,
        "totalRemoteBalanceSat": 3600000,
        "totalCapacitySat": 10000000,
        "channels": [
            {"id": "ch-alpha", "shortChannelId": "101x1x0", "fundingOutpoint": "aa".repeat(32) + ":0", "peerAlias": "regtest-peer-alpha",
             "peerPubkey": "03bb22cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66ee77", "capacitySat": 5000000, "localBalanceSat": 3100000,
             "remoteBalanceSat": 1900000, "state": "active", "isPrivate": false, "isInitiator": true, "baseFeeMsat": 1000, "feeRatePpm": 250,
             "openedAt": iso(86_400.0 * 120.0), "forwardCount": 58, "earnedRoutingSat": 4120, "htlcCount": 1, "lastActivityAt": iso(3_600.0 * 5.0)},
            {"id": "ch-beta", "shortChannelId": "140x2x1", "fundingOutpoint": "bb".repeat(32) + ":1", "peerAlias": "regtest-peer-beta",
             "peerPubkey": "02cc33dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66ee77ff88", "capacitySat": 3000000, "localBalanceSat": 2500000,
             "remoteBalanceSat": 500000, "state": "active", "isPrivate": false, "isInitiator": false, "baseFeeMsat": 0, "feeRatePpm": 100,
             "openedAt": iso(86_400.0 * 60.0), "forwardCount": 21, "earnedRoutingSat": 980, "htlcCount": 0, "lastActivityAt": iso(86_400.0 * 2.0)},
            {"id": "ch-gamma", "shortChannelId": "188x1x0", "peerAlias": "regtest-peer-gamma", "peerPubkey": null, "capacitySat": 2000000,
             "localBalanceSat": 800000, "remoteBalanceSat": 1200000, "state": "inactive", "isPrivate": true, "isInitiator": true,
             "baseFeeMsat": 500, "feeRatePpm": 50, "openedAt": iso(86_400.0 * 20.0), "forwardCount": 0, "earnedRoutingSat": 0},
            {"id": "ch-delta", "shortChannelId": null, "peerAlias": "regtest-peer-delta", "peerPubkey": "03dd44ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66ee77ff8899",
             "capacitySat": 1000000, "localBalanceSat": 1000000, "remoteBalanceSat": 0, "state": "pending_open", "isPrivate": false, "isInitiator": true}
        ],
        "closedChannels": [
            {"id": "ch-old", "shortChannelId": "55x3x0", "fundingOutpoint": "cc".repeat(32) + ":0", "peerAlias": "regtest-peer-retired",
             "peerPubkey": "02ee55ff66aa77bb88cc99dd00ee11ff22aa33bb44cc55dd66ee77ff8899aa00", "capacitySat": 1500000, "localBalanceSat": 0,
             "remoteBalanceSat": 0, "state": "closed", "isPrivate": false, "isInitiator": true, "openedAt": iso(86_400.0 * 300.0),
             "closedAt": iso(86_400.0 * 150.0), "closeKind": "cooperative", "forwardCount": 12, "earnedRoutingSat": 3100}
        ],
        "routing": {"windowLabel": "Last 30 days", "routingRevenueSat": 5100, "paymentCostSat": 640, "rebalanceCostSat": 300,
                    "onchainCostSat": 1800, "netProfitSat": 2360, "forwardCount": 79, "paymentCount": 17, "rebalanceCount": 2},
        "forwards": [
            {"id": "fw-1", "occurredAt": iso(3_600.0 * 5.0), "inPeerAlias": "regtest-peer-beta", "inShortChannelId": "140x2x1",
             "outPeerAlias": "regtest-peer-alpha", "outShortChannelId": "101x1x0", "amountInMsat": 250_250_000, "amountOutMsat": 250_000_000,
             "feeMsat": 250_000, "status": "settled"},
            {"id": "fw-2", "occurredAt": iso(3_600.0 * 30.0), "inPeerAlias": "regtest-peer-alpha", "inShortChannelId": "101x1x0",
             "outPeerAlias": "regtest-peer-beta", "outShortChannelId": "140x2x1", "amountInMsat": 1_000_400, "amountOutMsat": 1_000_000,
             "feeMsat": 400, "status": "settled"},
            {"id": "fw-3", "occurredAt": iso(86_400.0 * 3.0), "inPeerAlias": "regtest-peer-alpha", "inShortChannelId": "101x1x0",
             "outPeerAlias": "regtest-peer-gamma", "outShortChannelId": "188x1x0", "amountInMsat": 50_000_000, "amountOutMsat": 50_000_000,
             "feeMsat": 0, "status": "failed", "failureReason": "temporary_channel_failure"},
            {"id": "fw-4", "occurredAt": iso(60.0 * 4.0), "inPeerAlias": "regtest-peer-beta", "inShortChannelId": "140x2x1",
             "outPeerAlias": "regtest-peer-alpha", "outShortChannelId": "101x1x0", "amountInMsat": 12_000_000, "amountOutMsat": 11_998_000,
             "feeMsat": 2_000, "status": "offered"}
        ]
    })
}
