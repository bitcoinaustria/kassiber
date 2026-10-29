//! Wallet quantity differences are diagnostics, never engine authority.
use super::{array, exact, number};
use crate::{codec, graph::stored};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

/// Current balances use an ordered array of [wallet id, asset, amount] rows at
/// this internal comparison seam; snapshot v1 remains unchanged and raw-only.
pub fn compare_wallet_balances(
    projection: &Value,
    current: &[Value],
    known_reasons: &Value,
) -> Result<Value, String> {
    let observations: BTreeMap<_, _> = array(&projection["observations"])
        .iter()
        .map(|o| (codec::text(&o["quantity_hash"]), o))
        .collect();
    let mut canonical: BTreeMap<(String, String), BigInt> = BTreeMap::new();
    let mut reasons: BTreeMap<(String, String), BTreeSet<String>> = BTreeMap::new();
    for posting in array(&projection["postings"]) {
        if posting["location_kind"] != "wallet" {
            continue;
        }
        let key = (
            codec::text(&posting["location_id"]),
            codec::text(&posting["asset"]),
        );
        *canonical.entry(key.clone()).or_default() += exact(&posting["amount_msat"])?;
        if let Some(o) = observations.get(&codec::text(&posting["observation_hash"])) {
            let reason = &known_reasons[codec::text(&o["transaction_id"])];
            if codec::truth(reason) {
                reasons.entry(key).or_default().insert(codec::pystr(reason));
            }
        }
    }
    let mut balances = BTreeMap::new();
    for row in current {
        let key = (codec::text(&row[0]), codec::text(&row[1]));
        // Python int() for this diagnostic seam; never for raw msat facts.
        let amount = stored::integer(&codec::from_json(&row[2])?).map_err(str::to_owned)?;
        balances.insert(key, amount);
    }
    let keys: BTreeSet<_> = canonical.keys().chain(balances.keys()).cloned().collect();
    let mut result = Vec::new();
    for (wallet, asset) in keys {
        let key = (wallet.clone(), asset.clone());
        let a = canonical.get(&key).cloned().unwrap_or_default();
        let b = balances.get(&key).cloned().unwrap_or_default();
        if a == b {
            continue;
        }
        let named = reasons.get(&key);
        let reason = named
            .filter(|r| r.len() == 1)
            .and_then(|r| r.first())
            .cloned();
        result.push(json!({"$type":"WalletBalanceDifference","wallet_id":wallet,"asset":asset,"canonical_msat":number(&a),"current_msat":number(&b),"reason":reason}));
    }
    Ok(json!(result))
}
