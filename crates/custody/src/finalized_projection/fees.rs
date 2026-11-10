//! Fee state only. Callers retain responsibility for selection and barriers.
use super::*;
use crate::evidence::canonical::LegRef;
use crate::quantity::DecisionRef;
use std::collections::BTreeMap;

fn bundle(decision: DecisionRef<'_>) -> Option<String> {
    if codec::truth(decision.at("atomic_bundle_id")) {
        return Some(string(decision.at("atomic_bundle_id")));
    }
    let claim = text(decision.at("selected_claim_id"));
    claim
        .strip_prefix("pair:")
        .map(|tail| format!("pair:{}", tail.split(':').next().unwrap()))
}
#[derive(Default)]
pub struct FeeAllocation<'a> {
    allocated_sources: crate::fxhash::HashSet<std::borrow::Cow<'a, str>>,
    by_bundle: BTreeMap<String, BigInt>,
    consumed_bundles: BTreeSet<String>,
}
impl<'a> FeeAllocation<'a> {
    pub fn new(decisions: &[Value]) -> Result<Self, String> {
        Self::from_decisions(decisions.iter().map(DecisionRef::Json))
    }
    pub(super) fn from_decisions<'d>(
        decisions: impl IntoIterator<Item = DecisionRef<'d>>,
    ) -> Result<Self, String> {
        let mut result = Self::default();
        for decision in decisions {
            if decision.at("destination_kind") != "fee" {
                continue;
            }
            if let Some(bundle) = bundle(decision) {
                let source = decision.at("source");
                let amount =
                    integer_big(&source["end_msat"])? - integer_big(&source["start_msat"])?;
                *result.by_bundle.entry(bundle).or_default() += amount;
            }
        }
        Ok(result)
    }
    pub fn raw(&mut self, source: LegRef<'a>) -> Result<BigInt, String> {
        if !self
            .allocated_sources
            .insert(string_ref(source.at("quantity_hash")))
        {
            return Ok(BigInt::from(0));
        }
        integer_big(&source["fee_msat"])
    }
    pub(crate) fn move_fee(
        &mut self,
        decision: DecisionRef<'_>,
        source: LegRef<'a>,
    ) -> Result<BigInt, String> {
        let bundle = bundle(decision);
        let reviewed = bundle
            .as_ref()
            .filter(|b| !self.consumed_bundles.contains(*b))
            .and_then(|b| self.by_bundle.get(b))
            .cloned()
            .unwrap_or_default();
        if reviewed != BigInt::from(0) {
            if let Some(bundle) = bundle {
                self.consumed_bundles.insert(bundle);
            }
        }
        let raw = self.raw(source)?;
        Ok(
            if reviewed != BigInt::from(0)
                && !decision.at("component_id").is_null()
                && decision.at("reason") == "reviewed_custody_component"
                && decision.at("transfer_kind") != "swap-refund"
            {
                reviewed
            } else {
                raw + reviewed
            },
        )
    }
    pub(crate) fn reviewed_fee(&self, decision: DecisionRef<'_>) -> Result<Option<BigInt>, String> {
        if bundle(decision).is_some_and(|b| self.consumed_bundles.contains(&b)) {
            return Ok(None);
        }
        Ok(Some(
            integer_big(&decision.at("source")["end_msat"])?
                - integer_big(&decision.at("source")["start_msat"])?,
        ))
    }
}
/// Private explicit call sequence; does not select rows or change stage inventory.
pub(super) fn evaluate(v: &Value) -> Result<Value, String> {
    let decisions = v["decisions"].as_array().unwrap();
    let mut state = FeeAllocation::new(decisions)?;
    let mut result = Vec::new();
    for action in v["actions"].as_array().unwrap() {
        let decision = DecisionRef::Json(&decisions[action["decision"].as_u64().unwrap() as usize]);
        let source =
            LegRef::Json(&v["observations"][string(&decision.at("source")["observation_hash"])]);
        result.push(match action["kind"].as_str().unwrap() {
            "raw" => number(&state.raw(source)?),
            "move" => number(&state.move_fee(decision, source)?),
            "reviewed" => state
                .reviewed_fee(decision)?
                .as_ref()
                .map(number)
                .unwrap_or(Value::Null),
            _ => panic!("unknown fee fixture action"),
        });
    }
    Ok(json!(result))
}
