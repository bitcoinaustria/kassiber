//! `NodeSnapshot` (`core/lightning/types.py`) as the adapters build it, and its
//! two wire projections. The types only hold fields the reference keeps after
//! its discard policy; Tier-1 material never reaches them.
use super::*;

/// `NodeChannelState`; the adapters map node states onto these.
pub(super) const CLOSED_STATES: [&str; 2] = ["closed", "force_closed"];

#[derive(Clone, Debug, PartialEq)]
pub struct NodeChannel {
    pub id: PyStr,
    pub peer_alias: PyStr,
    /// The raw node value for public channels; `None` for private ones.
    pub peer_pubkey: Value,
    pub capacity_sat: BigInt,
    pub local_balance_sat: BigInt,
    pub remote_balance_sat: BigInt,
    pub state: &'static str,
    pub is_private: bool,
    pub is_initiator: bool,
    pub short_channel_id: Option<PyStr>,
    pub funding_outpoint: Option<PyStr>,
    pub base_fee_msat: Option<BigInt>,
    pub fee_rate_ppm: Option<BigInt>,
    pub opened_at: Option<String>,
    pub closed_at: Option<String>,
    pub close_kind: Option<&'static str>,
    pub forward_count: Option<BigInt>,
    pub earned_routing_sat: Option<BigInt>,
    pub htlc_count: Option<BigInt>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeForward {
    pub id: PyStr,
    pub occurred_at: String,
    pub in_peer_alias: PyStr,
    pub out_peer_alias: PyStr,
    pub amount_in_msat: BigInt,
    pub amount_out_msat: BigInt,
    pub fee_msat: BigInt,
    pub status: &'static str,
    pub in_short_channel_id: Option<PyStr>,
    pub out_short_channel_id: Option<PyStr>,
}

#[derive(Debug, PartialEq)]
pub struct NodeSnapshot {
    pub alias: PyStr,
    pub pubkey: PyStr,
    pub network: PyStr,
    pub implementation_version: Option<PyStr>,
    pub peer_count: BigInt,
    pub block_height: Option<BigInt>,
    pub invoice_count: usize,
    pub paid_invoice_count: usize,
    pub expired_invoice_count: usize,
    pub payment_count: usize,
    pub completed_payment_count: usize,
    pub failed_payment_count: usize,
    pub onchain_balance_sat: BigInt,
    pub total_local_balance_sat: BigInt,
    pub total_remote_balance_sat: BigInt,
    pub total_capacity_sat: BigInt,
    pub channels: Vec<NodeChannel>,
    pub closed_channels: Vec<NodeChannel>,
    pub routing: Option<Routing>,
    pub forwards: Vec<NodeForward>,
}

fn optional<T>(value: Option<T>, wrap: impl FnOnce(T) -> Value) -> Value {
    value.map(wrap).unwrap_or(Value::Null)
}
fn int(value: &BigInt) -> Value {
    Value::Int(value.clone())
}
fn count(value: usize) -> Value {
    Value::Int(value.into())
}
fn text(value: &PyStr) -> Value {
    Value::Str(value.clone())
}
fn iso(value: &Option<String>) -> Value {
    optional(value.as_deref(), Value::from)
}

impl NodeChannel {
    fn operational(&self) -> Vec<(&'static str, Value)> {
        vec![
            ("capacitySat", int(&self.capacity_sat)),
            ("localBalanceSat", int(&self.local_balance_sat)),
            ("remoteBalanceSat", int(&self.remote_balance_sat)),
            ("state", Value::from(self.state)),
            ("isPrivate", Value::Bool(self.is_private)),
            ("isInitiator", Value::Bool(self.is_initiator)),
            ("baseFeeMsat", optional(self.base_fee_msat.as_ref(), int)),
            ("feeRatePpm", optional(self.fee_rate_ppm.as_ref(), int)),
            ("openedAt", iso(&self.opened_at)),
            ("closedAt", iso(&self.closed_at)),
            ("closeKind", optional(self.close_kind, Value::from)),
            ("forwardCount", optional(self.forward_count.as_ref(), int)),
            (
                "earnedRoutingSat",
                optional(self.earned_routing_sat.as_ref(), int),
            ),
            ("htlcCount", optional(self.htlc_count.as_ref(), int)),
            ("lastActivityAt", Value::Null),
        ]
    }

    /// `channel_to_dict`.
    fn to_dict(&self) -> Value {
        let mut fields = vec![
            ("id", text(&self.id)),
            (
                "shortChannelId",
                optional(self.short_channel_id.as_ref(), text),
            ),
            (
                "fundingOutpoint",
                optional(self.funding_outpoint.as_ref(), text),
            ),
            ("peerAlias", text(&self.peer_alias)),
            ("peerPubkey", self.peer_pubkey.clone()),
        ];
        fields.extend(self.operational());
        dict(fields)
    }

    /// `_channel_to_dict_for_ai`: every Tier-3 identifier is omitted.
    fn to_ai_dict(&self) -> Value {
        dict(self.operational())
    }
}

impl NodeForward {
    /// `forward_to_dict`.
    fn to_dict(&self) -> Value {
        dict([
            ("id", text(&self.id)),
            ("occurredAt", Value::from(self.occurred_at.as_str())),
            ("inPeerAlias", text(&self.in_peer_alias)),
            (
                "inShortChannelId",
                optional(self.in_short_channel_id.as_ref(), text),
            ),
            ("outPeerAlias", text(&self.out_peer_alias)),
            (
                "outShortChannelId",
                optional(self.out_short_channel_id.as_ref(), text),
            ),
            ("amountInMsat", int(&self.amount_in_msat)),
            ("amountOutMsat", int(&self.amount_out_msat)),
            ("feeMsat", int(&self.fee_msat)),
            ("status", Value::from(self.status)),
            ("failureReason", Value::Null),
        ])
    }

    /// `_forward_to_dict_for_ai`: no peer or channel identifiers.
    fn to_ai_dict(&self) -> Value {
        dict([
            ("occurredAt", Value::from(self.occurred_at.as_str())),
            ("amountInMsat", int(&self.amount_in_msat)),
            ("amountOutMsat", int(&self.amount_out_msat)),
            ("feeMsat", int(&self.fee_msat)),
            ("status", Value::from(self.status)),
            ("failureReason", Value::Null),
        ])
    }
}

/// `routing_to_dict`.
fn routing_to_dict(routing: &Routing) -> Value {
    dict([
        ("windowLabel", Value::from(routing.window_label.as_str())),
        ("routingRevenueSat", int(&routing.routing_revenue_sat)),
        ("paymentCostSat", int(&routing.payment_cost_sat)),
        ("rebalanceCostSat", int(&routing.rebalance_cost_sat)),
        ("onchainCostSat", int(&routing.onchain_cost_sat)),
        ("netProfitSat", int(&routing.net_profit_sat)),
        ("forwardCount", count(routing.forward_count)),
        ("paymentCount", count(routing.payment_count)),
        ("rebalanceCount", count(routing.rebalance_count)),
    ])
}

impl NodeSnapshot {
    fn wire(
        &self,
        pubkey: Value,
        channel: fn(&NodeChannel) -> Value,
        forward: fn(&NodeForward) -> Value,
    ) -> Value {
        dict([
            ("alias", text(&self.alias)),
            ("pubkey", pubkey),
            ("network", text(&self.network)),
            (
                "implementationVersion",
                optional(self.implementation_version.as_ref(), text),
            ),
            ("peerCount", int(&self.peer_count)),
            ("blockHeight", optional(self.block_height.as_ref(), int)),
            ("invoiceCount", count(self.invoice_count)),
            ("paidInvoiceCount", count(self.paid_invoice_count)),
            ("expiredInvoiceCount", count(self.expired_invoice_count)),
            ("paymentCount", count(self.payment_count)),
            ("completedPaymentCount", count(self.completed_payment_count)),
            ("failedPaymentCount", count(self.failed_payment_count)),
            ("onchainBalanceSat", int(&self.onchain_balance_sat)),
            ("totalLocalBalanceSat", int(&self.total_local_balance_sat)),
            ("totalRemoteBalanceSat", int(&self.total_remote_balance_sat)),
            ("totalCapacitySat", int(&self.total_capacity_sat)),
            (
                "channels",
                Value::List(self.channels.iter().map(channel).collect()),
            ),
            (
                "closedChannels",
                Value::List(self.closed_channels.iter().map(channel).collect()),
            ),
            ("routing", optional(self.routing.as_ref(), routing_to_dict)),
            (
                "forwards",
                Value::List(self.forwards.iter().map(forward).collect()),
            ),
        ])
    }

    /// `snapshot_to_dict`: the operator's own desktop projection.
    pub fn to_dict(&self) -> Value {
        self.wire(
            text(&self.pubkey),
            NodeChannel::to_dict,
            NodeForward::to_dict,
        )
    }

    /// `snapshot_to_dict_for_ai`: the operator pubkey and every per-channel
    /// and per-forward Tier-3 identifier are removed; the shape stays stable.
    pub fn to_ai_dict(&self) -> Value {
        self.wire(
            Value::Null,
            NodeChannel::to_ai_dict,
            NodeForward::to_ai_dict,
        )
    }

    /// The fields the profitability report reads: open channels in adapter
    /// order, the routing summary and the node's public counters.
    pub fn profitability(self) -> ProfitabilitySnapshot {
        ProfitabilitySnapshot {
            channels: self
                .channels
                .into_iter()
                .map(|c| Channel {
                    id: c.id,
                    peer_alias: c.peer_alias,
                    capacity_sat: c.capacity_sat,
                    earned_routing_sat: c.earned_routing_sat,
                })
                .collect(),
            routing: self.routing,
            node: NodeInfo {
                alias: self.alias,
                network: self.network,
                block_height: self.block_height,
                peer_count: self.peer_count,
            },
        }
    }
}

pub(super) const UNKNOWN_OCCURRED_AT: &str = "1970-01-01T00:00:00Z";

fn year_out_of_range() -> AppError {
    AppError::new("internal_error", "Lightning timestamp is out of range")
}

/// `datetime.fromtimestamp(seconds, tz=utc)` rendered as `timestamp_to_iso`
/// does. `Ok(None)` is CPython's `ValueError` for a year outside 1..=9999;
/// `Err` is its `OSError` when the year does not fit the C `tm_year`.
pub(crate) fn utc_iso(seconds: i64) -> Result<Option<String>> {
    let days = seconds.div_euclid(86_400);
    let clock = seconds.rem_euclid(86_400);
    let (year, month, day) = kassiber_pytext::zone::civil_from_days(days);
    if i32::try_from(year - 1900).is_err() {
        return Err(year_out_of_range());
    }
    if !(1..=9999).contains(&year) {
        return Ok(None);
    }
    Ok(Some(format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        clock / 3600,
        clock % 3600 / 60,
        clock % 60
    )))
}

/// LND `_timestamp`: non-positive values are absent; any year past 9999
/// escapes the reference as an error.
pub(super) fn lnd_iso(value: &Value) -> Result<Option<String>> {
    let seconds = integer(value)?;
    if seconds <= BigInt::from(0) {
        return Ok(None);
    }
    let seconds = i64::try_from(seconds).map_err(|_| year_out_of_range())?;
    utc_iso(seconds)?.map(Some).ok_or_else(year_out_of_range)
}
