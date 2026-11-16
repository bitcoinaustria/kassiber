//! `ui.transactions.graph` payload types (crates/journals/src/transaction_graph.rs, mirrored by
//! TransactionGraphModel.ts). Lenient like the web: missing or null fields take defaults.

use serde::{Deserialize, Deserializer};

fn nullable<'de, D, T>(d: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(d)?.unwrap_or_default())
}

fn opt_num<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = serde_json::Value::deserialize(d)?;
    Ok(v.as_f64().filter(|x| x.is_finite()))
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Annotation {
    #[serde(deserialize_with = "nullable")]
    pub code: String,
    pub label: Option<String>,
    pub severity: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Node {
    #[serde(deserialize_with = "nullable")]
    pub id: String,
    #[serde(deserialize_with = "opt_num")]
    pub index: Option<f64>,
    pub outpoint: Option<String>,
    pub txid: Option<String>,
    pub address: Option<String>,
    pub script_type: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub value_sats: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub value_btc: Option<f64>,
    pub value_state: Option<String>,
    pub asset: Option<String>,
    pub label: Option<String>,
    pub wallet: Option<String>,
    pub ownership: Option<String>,
    pub role: Option<String>,
    pub overflow: bool,
    #[serde(deserialize_with = "opt_num")]
    pub overflow_count: Option<f64>,
    pub funded_by_transaction_id: Option<String>,
    pub spent_by_txid: Option<String>,
    pub spent_by_transaction_id: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub prevout_block_height: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub spent_by_block_height: Option<f64>,
    #[serde(deserialize_with = "nullable")]
    pub annotations: Vec<Annotation>,
}

impl Node {
    pub fn state(&self) -> &str {
        self.value_state.as_deref().unwrap_or("")
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Wallet {
    pub id: Option<String>,
    pub label: Option<String>,
    pub kind: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Leg {
    pub id: Option<String>,
    pub external_id: Option<String>,
    pub txid: Option<String>,
    pub role: Option<String>,
    pub asset: Option<String>,
    pub network: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub amount_btc: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub fee_btc: Option<f64>,
    pub kind: Option<String>,
    pub counterparty: Option<String>,
    pub description: Option<String>,
    /// When the leg's transaction happened (ISO).
    pub occurred_at: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub wallet: Wallet,
}

impl Leg {
    /// `legGraphReference`.
    pub fn reference(&self) -> String {
        [&self.id, &self.txid, &self.external_id]
            .into_iter()
            .flatten()
            .find(|v| !v.is_empty())
            .cloned()
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct SwapRoute {
    pub id: Option<String>,
    pub kind: Option<String>,
    pub route_kind: Option<String>,
    pub policy: Option<String>,
    /// Who made the link: `manual`, `bulk_exact`, `bulk_selected`, `rule_auto` (the owner), or
    /// `journal_builder` (journal processing, from wallet or chain evidence).
    pub pair_source: Option<String>,
    /// When the link was recorded (ISO).
    pub created_at: Option<String>,
    pub current_leg: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub swap_fee_btc: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub out_amount_btc: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub out_full_amount_btc: Option<f64>,
    #[serde(rename = "out", deserialize_with = "nullable")]
    pub out_leg: Leg,
    #[serde(rename = "in", deserialize_with = "nullable")]
    pub in_leg: Leg,
}

impl SwapRoute {
    pub fn leg(&self, key: LegKey) -> &Leg {
        match key {
            LegKey::Out => &self.out_leg,
            LegKey::In => &self.in_leg,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LegKey {
    Out,
    In,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TxInfo {
    pub id: Option<String>,
    pub txid: Option<String>,
    pub external_id: Option<String>,
    pub chain: Option<String>,
    pub network: Option<String>,
    pub asset: Option<String>,
    #[serde(deserialize_with = "opt_num")]
    pub block_height: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub input_count: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub output_count: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub version: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub locktime: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub size: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub vsize: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub weight: Option<f64>,
    #[serde(deserialize_with = "opt_num")]
    pub fee_rate_sat_vb: Option<f64>,
    pub confirmed_at: Option<String>,
    /// The wallet whose row this is (its backend is the first a lookup asks).
    #[serde(deserialize_with = "nullable")]
    pub wallet: Wallet,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Warning {
    #[serde(deserialize_with = "nullable")]
    pub code: String,
    pub level: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub message: String,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Payload {
    pub transaction: Option<TxInfo>,
    #[serde(deserialize_with = "nullable")]
    pub support_level: String,
    pub unsupported_reason: Option<String>,
    #[serde(deserialize_with = "nullable")]
    pub warnings: Vec<Warning>,
    #[serde(deserialize_with = "nullable")]
    pub inputs: Vec<Node>,
    #[serde(deserialize_with = "nullable")]
    pub outputs: Vec<Node>,
    pub fee: Option<Node>,
    #[serde(deserialize_with = "nullable")]
    pub annotations: Vec<Annotation>,
    pub swap_route: Option<SwapRoute>,
    #[serde(deserialize_with = "nullable")]
    pub context: Context,
}

/// The book the graph was read from (`context`).
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Context {
    pub workspace: Option<String>,
    pub profile: Option<String>,
}

impl Payload {
    pub fn parse(v: &serde_json::Value) -> Result<Payload, String> {
        Payload::deserialize(v).map_err(|e| e.to_string())
    }
    pub fn chain(&self) -> &str {
        self.transaction
            .as_ref()
            .and_then(|t| t.chain.as_deref())
            .unwrap_or("")
    }
    pub fn is_liquid(&self) -> bool {
        self.chain() == "liquid"
    }
    /// The daemon's note of where the references came from, or why a lookup stopped
    /// (`_graphLookupWarning`, projected into `warnings`).
    pub fn has_warning(&self, code: &str) -> bool {
        self.warnings.iter().any(|w| w.code == code)
    }
    /// Spent inputs whose amount is not known (`valueState: missing`).
    pub fn missing_inputs(&self) -> usize {
        self.inputs
            .iter()
            .filter(|n| n.state() == "missing")
            .count()
    }
    /// `showDiagram`.
    pub fn drawable(&self) -> bool {
        matches!(self.support_level.as_str(), "full" | "partial")
            && (!self.inputs.is_empty() || !self.outputs.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_daemon_payload() {
        let v = serde_json::json!({
            "transaction": {"id": "t", "chain": "bitcoin", "vsize": 130, "feeRateSatVb": 27.18},
            "supportLevel": "full", "unsupportedReason": null, "warnings": [],
            "inputs": [{"id": "in-0", "valueSats": 10, "valueState": "known", "ownership": "owned"}],
            "outputs": [{"id": "out-0", "valueState": "missing"}],
            "fee": null, "annotations": [], "swapRoute": {"routeKind": "swap", "out": {"id": "a"}, "in": {"txid": "b"}}
        });
        let p = Payload::parse(&v).unwrap();
        assert!(p.drawable());
        assert_eq!(p.inputs[0].value_sats, Some(10.0));
        assert_eq!(p.outputs[0].state(), "missing");
        assert_eq!(p.swap_route.as_ref().unwrap().in_leg.reference(), "b");
    }
}
