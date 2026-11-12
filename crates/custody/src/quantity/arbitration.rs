//! Port of custody_quantity.py: validate bundles, split sources, reject target
//! overlap, enforce atomic completeness, and conserve exact exposure postings.
use super::{array, delta, domain, exact, number, principal, validate_observation};
use crate::{codec, evidence::canonical::LegRef, python_text::PythonText};
use kassiber_pyjson::BigInt;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Slice {
    hash: String,
    start: BigInt,
    end: BigInt,
}
impl Slice {
    fn parse(v: &Value) -> Result<Self, String> {
        let hash = codec::text(&v["observation_hash"]);
        let start = exact(&v["start_msat"])?;
        let end = exact(&v["end_msat"])?;
        if hash.is_empty() || start < BigInt::from(0) || end <= start {
            return Err("ValueError".into());
        }
        Ok(Self { hash, start, end })
    }
    fn amount(&self) -> BigInt {
        &self.end - &self.start
    }
    fn value(&self) -> Value {
        let mut slice = serde_json::Map::new();
        slice.insert("$type".into(), Value::String("QuantitySlice".into()));
        slice.insert("end_msat".into(), number(&self.end));
        slice.insert("observation_hash".into(), Value::String(self.hash.clone()));
        slice.insert("start_msat".into(), number(&self.start));
        Value::Object(slice)
    }
    fn py(self) -> kassiber_pyjson::Value {
        use kassiber_pyjson::Value as Py;
        super::py_dict([
            ("$type", Py::Str("QuantitySlice".into())),
            ("end_msat", Py::Int(self.end)),
            ("observation_hash", Py::Str(self.hash.into())),
            ("start_msat", Py::Int(self.start)),
        ])
    }
}
pub(super) fn validate_claim(value: &Value) -> Result<(), String> {
    Claim::parse(value).map(|_| ())
}

#[derive(Clone)]
struct Claim {
    value: Value,
    id: String,
    source: Slice,
    target: Option<Slice>,
    priority: u8,
    bundle: Option<String>,
}
impl Claim {
    fn parse(v: &Value) -> Result<Self, String> {
        let invalid = || "ValueError".to_owned();
        let id = codec::text(&v["claim_id"]);
        let source = Slice::parse(&v["source"])?;
        let target = if v["target"].is_null() {
            None
        } else {
            Some(Slice::parse(&v["target"])?)
        };
        let priority = match v["priority"]["value"].as_u64() {
            Some(10) => 10,
            Some(20) => 20,
            Some(40) => 40,
            _ => return Err(invalid()),
        };
        let state = v["state"].as_str().unwrap_or("");
        if id.is_empty() || !codec::truth(&v["reason"]) {
            return Err(invalid());
        }
        let supported = match state {
            "internal_verified" => priority == 20,
            "internal_reviewed" => priority == 10,
            "external_confirmed" => matches!(priority, 10 | 20),
            "custody_suspense" => true,
            _ => false,
        };
        if !supported {
            return Err(invalid());
        }
        let destination = v["destination_kind"].as_str();
        let target_state = matches!(state, "internal_verified" | "internal_reviewed");
        let retained = state == "internal_reviewed"
            && priority == 10
            && target.is_none()
            && destination == Some("retained_custody");
        if target_state && target.is_none() && !retained
            || !target_state && target.is_some()
            || target
                .as_ref()
                .is_some_and(|t| t.amount() != source.amount())
        {
            return Err(invalid());
        }
        if !v["atomic_bundle_id"].is_null()
            && codec::pystr(&v["atomic_bundle_id"]).py_strip().is_empty()
        {
            return Err(invalid());
        }
        if !v["destination_kind"].is_null()
            && !matches!(destination, Some("external" | "fee" | "retained_custody"))
            || matches!(destination, Some("external" | "fee"))
                && !(state == "external_confirmed" && target.is_none())
            || destination == Some("retained_custody") && !retained
        {
            return Err(invalid());
        }
        if !v["external_economic_subtype"].is_null()
            && (!matches!(
                v["external_economic_subtype"].as_str(),
                Some("payment" | "disposal" | "gift" | "lost")
            ) || !(state == "external_confirmed"
                && priority == 10
                && target.is_none()
                && destination == Some("external")))
            || codec::truth(&v["allow_cross_rail"]) && target.is_none()
        {
            return Err(invalid());
        }
        let bundle = if !v["atomic_bundle_id"].is_null() {
            Some(codec::pystr(&v["atomic_bundle_id"]))
        } else if priority == 10 {
            Some(format!("single:{id}"))
        } else {
            None
        };
        Ok(Self {
            value: v.clone(),
            id,
            source,
            target,
            priority,
            bundle,
        })
    }
    fn target_at(&self, source: &Slice) -> Option<Slice> {
        self.target.as_ref().map(|t| {
            let start = &t.start + &source.start - &self.source.start;
            Slice {
                hash: t.hash.clone(),
                end: &start + source.amount(),
                start,
            }
        })
    }
    fn same_semantics(&self, other: &Self, source: &Slice) -> bool {
        [
            "state",
            "destination_kind",
            "external_economic_subtype",
            "transfer_kind",
            "transfer_policy",
            "component_id",
        ]
        .iter()
        .all(|key| self.value[key] == other.value[key])
            && self.target_at(source) == other.target_at(source)
    }
}
#[derive(Clone)]
pub(crate) struct Decision {
    value: std::cell::OnceCell<Value>,
    /// The default decision's source slice object, built on first read.
    source_value: std::cell::OnceCell<Value>,
    source: Slice,
    target: Option<Slice>,
}

static NULL: Value = Value::Null;
static DEFAULT_TYPE: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| Value::String("ArbitratedSlice".into()));
static DEFAULT_STATE: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| Value::String("external_presumed".into()));
static DEFAULT_REASON: std::sync::LazyLock<Value> =
    std::sync::LazyLock::new(|| Value::String("unmatched_outbound_default".into()));
static EMPTY: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Array(Vec::new()));

/// An arbitrated decision read by field: the default decision, whose fields
/// are fixed but its source, or a decision's JSON object.
#[derive(Clone, Copy)]
pub(crate) enum DecisionRef<'a> {
    Default(&'a Decision),
    Json(&'a Value),
}

impl<'a> DecisionRef<'a> {
    /// A default decision's source slice hash and bounds, read without
    /// building its slice object.
    pub(crate) fn typed_source(self) -> Option<(&'a str, &'a BigInt, &'a BigInt)> {
        match self {
            Self::Default(decision) => Some((
                decision.source.hash.as_str(),
                &decision.source.start,
                &decision.source.end,
            )),
            Self::Json(_) => None,
        }
    }

    /// The field `key`, borrowed for the decision's lifetime.
    pub(crate) fn at(self, key: &str) -> &'a Value {
        match self {
            Self::Json(value) => &value[key],
            Self::Default(decision) => match key {
                "$type" => &DEFAULT_TYPE,
                "state" => &DEFAULT_STATE,
                "reason" => &DEFAULT_REASON,
                "contender_claim_ids" => &EMPTY,
                "source" => decision
                    .source_value
                    .get_or_init(|| decision.source.value()),
                _ => &NULL,
            },
        }
    }
}

impl std::ops::Index<&str> for DecisionRef<'_> {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.at(key)
    }
}
impl Decision {
    fn new(source: Slice, state: &str, reason: &str) -> Self {
        let value = std::cell::OnceCell::new();
        if state != "external_presumed" || reason != "unmatched_outbound_default" {
            value.set(json!({"$type":"ArbitratedSlice","source":source.value(),"state":state,"reason":reason,"selected_claim_id":null,"atomic_bundle_id":null,"transfer_kind":null,"transfer_policy":null,"component_id":null,"target":null,"destination_kind":null,"external_economic_subtype":null,"contender_claim_ids":[]})).unwrap();
        }
        Self {
            value,
            source_value: std::cell::OnceCell::new(),
            source,
            target: None,
        }
    }
    /// The decision read by field, without building a default decision.
    pub(crate) fn as_ref(&self) -> DecisionRef<'_> {
        match self.value.get() {
            Some(value) => DecisionRef::Json(value),
            None => DecisionRef::Default(self),
        }
    }
    pub(super) fn value(&self) -> &Value {
        self.value.get_or_init(|| {
            // The default decision `new` leaves unbuilt, without `json!`
            // serializing each part again.
            let mut decision = serde_json::Map::new();
            for (key, value) in [
                ("$type", Value::String("ArbitratedSlice".into())),
                ("atomic_bundle_id", Value::Null),
                ("component_id", Value::Null),
                ("contender_claim_ids", Value::Array(Vec::new())),
                ("destination_kind", Value::Null),
                ("external_economic_subtype", Value::Null),
                ("reason", Value::String("unmatched_outbound_default".into())),
                ("selected_claim_id", Value::Null),
                ("source", self.source.value()),
                ("state", Value::String("external_presumed".into())),
                ("target", Value::Null),
                ("transfer_kind", Value::Null),
                ("transfer_policy", Value::Null),
            ] {
                decision.insert(key.into(), value);
            }
            Value::Object(decision)
        })
    }
    fn value_mut(&mut self) -> &mut Value {
        self.value();
        self.value.get_mut().unwrap()
    }
    fn selected(&self) -> Option<&str> {
        self.value
            .get()
            .and_then(|v| v["selected_claim_id"].as_str())
    }
    fn py(&self) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::Value as Py;
        if let Some(value) = self.value.get() {
            return super::borrowed_py(value);
        }
        Ok(super::py_dict([
            ("$type", Py::Str("ArbitratedSlice".into())),
            ("atomic_bundle_id", Py::Null),
            ("component_id", Py::Null),
            ("contender_claim_ids", Py::List(vec![])),
            ("destination_kind", Py::Null),
            ("external_economic_subtype", Py::Null),
            ("reason", Py::Str("unmatched_outbound_default".into())),
            ("selected_claim_id", Py::Null),
            ("source", self.source.clone().py()),
            ("state", Py::Str("external_presumed".into())),
            ("target", Py::Null),
            ("transfer_kind", Py::Null),
            ("transfer_policy", Py::Null),
        ]))
    }
    fn conflict(&mut self, reason: &str, ids: BTreeSet<String>) {
        self.value_mut()["state"] = json!("conflicting");
        self.value_mut()["reason"] = json!(reason);
        for key in [
            "selected_claim_id",
            "target",
            "destination_kind",
            "external_economic_subtype",
        ] {
            self.value_mut()[key] = Value::Null;
        }
        self.value_mut()["contender_claim_ids"] = json!(ids);
        self.target = None;
    }
}

/// Observations by quantity hash, borrowing typed observations' hashes.
type ByHash<'a> = BTreeMap<std::borrow::Cow<'a, str>, LegRef<'a>>;
type Validated<'a> = (ByHash<'a>, Vec<Claim>, Vec<Value>);
fn validate<'a>(observations: &[LegRef<'a>], claims: &[Value]) -> Result<Validated<'a>, String> {
    let mut by_hash = BTreeMap::new();
    for &observation in observations {
        match observation {
            // Evidence builds typed observations under the same rules.
            LegRef::Typed(_) => debug_assert!(validate_observation(&observation).is_ok()),
            LegRef::Json(_) => validate_observation(&observation)?,
        }
        if by_hash
            .insert(
                codec::text_ref(observation.at("quantity_hash")),
                observation,
            )
            .is_some()
        {
            return Err("ValueError".into());
        }
    }
    let mut claims = claims
        .iter()
        .map(Claim::parse)
        .collect::<Result<Vec<_>, _>>()?;
    claims.sort_by_cached_key(|c| (c.priority, c.id.clone()));
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut bundles: BTreeMap<String, Vec<Claim>> = BTreeMap::new();
    for claim in claims {
        *counts.entry(claim.id.clone()).or_default() += 1;
        bundles
            .entry(
                claim
                    .bundle
                    .clone()
                    .unwrap_or_else(|| format!("claim:{}", claim.id)),
            )
            .or_default()
            .push(claim);
    }
    let mut valid = Vec::new();
    let mut errors = Vec::new();
    let mut invalid: BTreeMap<String, u8> = BTreeMap::new();
    for (bundle, members) in bundles {
        let mut reasons = BTreeSet::new();
        let mut sources = BTreeSet::new();
        if members
            .iter()
            .map(|c| c.priority)
            .collect::<BTreeSet<_>>()
            .len()
            != 1
        {
            reasons.insert("atomic_bundle_priority_mismatch");
        }
        if members.iter().any(|c| counts[&c.id] > 1) {
            reasons.insert("duplicate_claim_id");
        }
        for claim in &members {
            let source = by_hash
                .get(claim.source.hash.as_str())
                .filter(|s| s["direction"] == "outbound");
            if let Some(source) = source {
                sources.insert(claim.source.hash.clone());
                if claim.source.end > principal(source)? {
                    reasons.insert("claim_source_exceeds_principal");
                }
            } else {
                reasons.insert("claim_source_invalid");
            }
            let Some(target_slice) = &claim.target else {
                continue;
            };
            let Some(target) = by_hash
                .get(target_slice.hash.as_str())
                .filter(|t| t["direction"] == "inbound")
            else {
                reasons.insert("claim_target_invalid");
                continue;
            };
            if target_slice.end > principal(target)? {
                reasons.insert("claim_target_exceeds_principal");
            }
            if let Some(source) = source {
                let a = domain(source);
                let b = domain(target);
                if ["network", "exposure", "unit"].iter().any(|k| a[k] != b[k])
                    || a["rail"] != b["rail"] && !codec::truth(&claim.value["allow_cross_rail"])
                {
                    reasons.insert("claim_domain_incompatible");
                }
            }
        }
        if reasons.is_empty() {
            valid.extend(members);
            continue;
        }
        let involved: BTreeSet<_> = members
            .iter()
            .flat_map(|c| {
                std::iter::once(c.source.hash.clone())
                    .chain(c.target.iter().map(|t| t.hash.clone()))
            })
            .collect();
        let ids: BTreeSet<_> = members.iter().map(|c| c.id.clone()).collect();
        errors.push(json!({"$type":"QuantityClaimError","bundle_id":bundle,"reasons":reasons,"claim_ids":ids,"source_observation_hashes":sources,"involved_observation_hashes":involved}));
        let priority = members.iter().map(|c| c.priority).min().unwrap_or(40);
        for hash in sources {
            invalid
                .entry(hash)
                .and_modify(|p| *p = (*p).min(priority))
                .or_insert(priority);
        }
    }
    for (hash, priority) in invalid {
        let source = Slice {
            hash: hash.clone(),
            start: BigInt::from(0),
            end: principal(&by_hash[hash.as_str()])?,
        };
        let value = json!({"claim_id":format!("malformed-claim-bundle:{hash}"),"source":source.value(),"state":"custody_suspense","priority":{"value":priority},"reason":"malformed_claim_bundle"});
        valid.push(Claim::parse(&value)?);
    }
    valid.sort_by_cached_key(|c| (c.priority, c.id.clone()));
    Ok((by_hash, valid, errors))
}

fn source_decisions(by_hash: &ByHash<'_>, claims: &[Claim]) -> Result<Vec<Decision>, String> {
    let mut by_source: BTreeMap<&str, Vec<&Claim>> = BTreeMap::new();
    for claim in claims {
        by_source.entry(&claim.source.hash).or_default().push(claim);
    }
    let mut sources: Vec<_> = by_hash
        .iter()
        .filter(|(_, o)| o["direction"] == "outbound")
        .collect();
    sources.sort_by_cached_key(|(h, o)| (codec::text(&o["occurred_at"]), *h));
    let mut result = Vec::new();
    for (hash, observation) in sources {
        let claims = by_source.get(&**hash).cloned().unwrap_or_default();
        let suspense = claims
            .iter()
            .any(|c| c.value["state"] == "custody_suspense");
        let mut boundaries = BTreeSet::from([BigInt::from(0), principal(observation)?]);
        for claim in &claims {
            boundaries.extend([claim.source.start.clone(), claim.source.end.clone()]);
        }
        let boundaries: Vec<_> = boundaries.into_iter().collect();
        for ends in boundaries.windows(2) {
            let source = Slice {
                hash: hash.to_string(),
                start: ends[0].clone(),
                end: ends[1].clone(),
            };
            let contenders: Vec<_> = claims
                .iter()
                .copied()
                .filter(|c| c.source.start <= source.start && c.source.end >= source.end)
                .collect();
            if contenders.is_empty() {
                result.push(Decision::new(
                    source,
                    if suspense {
                        "custody_suspense"
                    } else {
                        "external_presumed"
                    },
                    if suspense {
                        "unclaimed_source_residual"
                    } else {
                        "unmatched_outbound_default"
                    },
                ));
                continue;
            }
            let authoritative: Vec<_> = contenders
                .iter()
                .copied()
                .filter(|c| c.priority <= 20)
                .collect();
            if let Some(first) = authoritative.first() {
                if authoritative
                    .iter()
                    .skip(1)
                    .any(|c| !c.same_semantics(first, &source))
                {
                    let mut decision =
                        Decision::new(source, "conflicting", "incompatible_reviewed_claims");
                    decision.value_mut()["contender_claim_ids"] = json!(authoritative
                        .iter()
                        .map(|c| c.id.clone())
                        .collect::<BTreeSet<_>>());
                    result.push(decision);
                    continue;
                }
            }
            let priority = contenders.iter().map(|c| c.priority).min().unwrap();
            let winners: Vec<_> = contenders
                .into_iter()
                .filter(|c| c.priority == priority)
                .collect();
            if winners.len() != 1 {
                let mut decision =
                    Decision::new(source, "conflicting", "equal_priority_source_overlap");
                decision.value_mut()["contender_claim_ids"] =
                    json!(winners.iter().map(|c| c.id.clone()).collect::<Vec<_>>());
                result.push(decision);
                continue;
            }
            let winner = winners[0];
            let mut decision = Decision::new(
                source,
                codec::text(&winner.value["state"]).as_str(),
                codec::text(&winner.value["reason"]).as_str(),
            );
            decision.target = winner.target_at(&decision.source);
            decision.value_mut()["target"] = decision
                .target
                .as_ref()
                .map(|t| t.value())
                .unwrap_or(Value::Null);
            decision.value_mut()["selected_claim_id"] = json!(winner.id);
            decision.value_mut()["atomic_bundle_id"] = json!(winner.bundle);
            for key in [
                "transfer_kind",
                "transfer_policy",
                "component_id",
                "destination_kind",
                "external_economic_subtype",
            ] {
                decision.value_mut()[key] = winner.value[key].clone();
            }
            result.push(decision);
        }
    }
    Ok(result)
}

fn destination_overlaps(decisions: &mut [Decision]) {
    let mut by_target: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, d) in decisions.iter().enumerate() {
        if let Some(t) = &d.target {
            by_target.entry(t.hash.clone()).or_default().push(i);
        }
    }
    let mut conflicts: BTreeMap<usize, BTreeSet<String>> = BTreeMap::new();
    for indices in by_target.values_mut() {
        indices.sort_by_cached_key(|i| {
            let t = decisions[*i].target.as_ref().unwrap();
            (t.start.clone(), t.end.clone())
        });
        let mut cluster = Vec::new();
        let mut end = BigInt::from(-1);
        let mut mark = |cluster: &[usize]| {
            if cluster.len() > 1 {
                let ids: BTreeSet<_> = cluster
                    .iter()
                    .filter_map(|i| {
                        decisions[*i]
                            .selected()
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                    })
                    .collect();
                for i in cluster {
                    conflicts.entry(*i).or_default().extend(ids.clone());
                }
            }
        };
        for &i in indices.iter() {
            let t = decisions[i].target.as_ref().unwrap();
            if !cluster.is_empty() && t.start >= end {
                mark(&cluster);
                cluster.clear();
            }
            if cluster.is_empty() {
                end = t.end.clone();
            } else {
                end = end.max(t.end.clone());
            }
            cluster.push(i);
        }
        mark(&cluster);
    }
    for (i, ids) in conflicts {
        if !ids.is_empty() {
            decisions[i].conflict("destination_slice_claimed_twice", ids);
        }
    }
}

fn atomic_bundles(decisions: &mut [Decision], claims: &[Claim]) {
    let mut totals: BTreeMap<String, (BigInt, BigInt, bool)> = BTreeMap::new();
    for decision in decisions.iter() {
        let Some(id) = decision.selected() else {
            continue;
        };
        let t = totals.entry(id.into()).or_default();
        t.0 += decision.source.amount();
        if let Some(target) = &decision.target {
            t.1 += target.amount();
            t.2 = true;
        }
    }
    let mut bundles: BTreeMap<String, Vec<&Claim>> = BTreeMap::new();
    for claim in claims {
        if let Some(bundle) = &claim.bundle {
            bundles.entry(bundle.clone()).or_default().push(claim);
        }
    }
    let mut invalid = BTreeMap::new();
    for (bundle, members) in &bundles {
        let fully_selected = members.iter().all(|c| {
            totals
                .get(&c.id)
                .is_some_and(|(source, target, has_target)| {
                    *source == c.source.amount()
                        && c.target
                            .as_ref()
                            .map(|t| *target == t.amount())
                            .unwrap_or(!has_target)
                })
        });
        if !fully_selected {
            for claim in members {
                invalid.insert(claim.id.clone(), bundle);
            }
        }
    }
    for decision in decisions {
        let Some(id) = decision.selected() else {
            continue;
        };
        if let Some(bundle) = invalid.get(id) {
            let mut ids: BTreeSet<_> = array(&decision.value()["contender_claim_ids"])
                .iter()
                .map(codec::pystr)
                .collect();
            ids.extend(bundles[*bundle].iter().map(|c| c.id.clone()));
            decision.conflict("atomic_bundle_incomplete", ids);
        }
    }
}

struct Posting<'a> {
    id: String,
    observation: LegRef<'a>,
    kind: String,
    location: Value,
    amount: BigInt,
    state: Value,
}
impl Posting<'_> {
    fn py(self) -> Result<kassiber_pyjson::Value, String> {
        posting_py(
            self.id,
            self.observation,
            self.kind,
            &self.location,
            self.amount,
            &self.state,
        )
    }
}
fn posting_py(
    id: String,
    observation: LegRef<'_>,
    kind: String,
    location: &Value,
    amount: BigInt,
    state: &Value,
) -> Result<kassiber_pyjson::Value, String> {
    use kassiber_pyjson::Value as Py;
    // A posting's domain is its observation's.
    let domain = &super::Domain::of_leg(observation);
    Ok(super::py_dict([
        ("$type", Py::Str("QuantityPosting".into())),
        ("amount_msat", Py::Int(amount)),
        ("asset", super::borrowed_py(&observation["asset"])?),
        ("domain", domain.py()?),
        ("location_id", super::borrowed_py(location)?),
        ("location_kind", Py::Str(kind.into())),
        (
            "observation_hash",
            super::borrowed_py(&observation["quantity_hash"])?,
        ),
        ("posting_id", Py::Str(id.into())),
        ("state", super::borrowed_py(state)?),
    ]))
}

/// The parts joined, without format machinery.
fn concat(parts: &[&str]) -> String {
    let mut text = String::with_capacity(parts.iter().map(|part| part.len()).sum());
    for part in parts {
        text.push_str(part);
    }
    text
}

fn postings<'a>(
    by_hash: &ByHash<'a>,
    input: &[LegRef<'a>],
    decisions: &[Decision],
) -> Result<Vec<Posting<'a>>, String> {
    let mut result = Vec::with_capacity(2 * by_hash.len() + decisions.len());
    let posting = |id: String,
                   observation: LegRef<'a>,
                   kind: &str,
                   location: Value,
                   n: BigInt,
                   state: Value| Posting {
        id,
        observation,
        kind: kind.into(),
        location,
        amount: n,
        state,
    };
    for (hash, &o) in by_hash {
        result.push(posting(
            concat(&["observed:", hash]),
            o,
            "wallet",
            o["wallet_id"].clone(),
            delta(&o)?,
            json!("observed"),
        ));
        let fee = exact(&o["fee_msat"])?;
        if o["direction"] == "outbound" && fee != BigInt::from(0) {
            result.push(posting(
                concat(&["fee:", hash]),
                o,
                "fee",
                json!("network_fee"),
                fee,
                json!("external_confirmed"),
            ));
        }
    }
    let mut targets: BTreeMap<String, Vec<&Slice>> = BTreeMap::new();
    for decision in decisions {
        if let Some(target) = &decision.target {
            targets.entry(target.hash.clone()).or_default().push(target);
            continue;
        }
        if decision.value.get().is_none() {
            let source = &decision.source;
            result.push(posting(
                format!("decision:{}:{}:{}", source.hash, source.start, source.end),
                by_hash[source.hash.as_str()],
                "external",
                json!("unmatched_outbound_default"),
                source.amount(),
                json!("external_presumed"),
            ));
            continue;
        }
        let d = decision.value();
        let kind = if codec::truth(&d["destination_kind"]) {
            codec::text(&d["destination_kind"])
        } else {
            match d["state"].as_str() {
                Some("external_confirmed" | "external_presumed") => "external",
                Some("custody_suspense") => "custody_suspense",
                Some("conflicting") => "conflicting",
                _ => return Err("ValueError".into()),
            }
            .into()
        };
        let source = &decision.source;
        let location = if codec::truth(&d["selected_claim_id"]) {
            d["selected_claim_id"].clone()
        } else {
            d["reason"].clone()
        };
        result.push(posting(
            format!("decision:{}:{}:{}", source.hash, source.start, source.end),
            by_hash[source.hash.as_str()],
            &kind,
            location,
            source.amount(),
            d["state"].clone(),
        ));
    }
    for targets in targets.values_mut() {
        targets.sort_by_cached_key(|s| (&s.start, &s.end));
    }
    for &o in input.iter().filter(|o| o["direction"] == "inbound") {
        let hash = codec::text(&o["quantity_hash"]);
        let mut cursor = BigInt::from(0);
        let mut uncovered = Vec::new();
        for t in targets.get(&hash).into_iter().flatten() {
            if t.start > cursor {
                uncovered.push((cursor, t.start.clone()));
            }
            cursor = t.end.clone();
        }
        let end = principal(&o)?;
        if cursor < end {
            uncovered.push((cursor, end));
        }
        for (start, end) in uncovered {
            result.push(posting(
                format!("origin:{hash}:{start}:{end}"),
                o,
                "external_origin",
                json!("unclassified_origin"),
                start - end,
                json!("unclassified_origin"),
            ));
        }
    }
    result.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(result)
}

/// Arbitrate Python canonical observations and compiled claims without bounded
/// integer conversions or any side effects. Malformed bundles become suspense.
pub fn project(observations: &[Value], claims: &[Value]) -> Result<Value, String> {
    project_refs(&observations.iter().collect::<Vec<_>>(), claims)
}

/// `project` over borrowed observations; only the projection's own list
/// copies them.
pub(crate) fn project_refs(observations: &[&Value], claims: &[Value]) -> Result<Value, String> {
    let observations: Vec<_> = observations.iter().map(|o| LegRef::Json(o)).collect();
    arbitrate(&observations, claims)?
        .py()
        .map(|v| codec::to_json(&v))
}

pub(super) struct Projection<'a> {
    by_hash: ByHash<'a>,
    decisions: Vec<Decision>,
    postings: Vec<Posting<'a>>,
    errors: Vec<Value>,
}

pub(super) struct View<'a> {
    pub observations: Vec<LegRef<'a>>,
    pub decisions: Vec<&'a Value>,
    pub errors: &'a [Value],
}
impl<'a> View<'a> {
    pub fn of(value: &'a Value) -> Self {
        Self {
            observations: array(&value["observations"])
                .iter()
                .map(LegRef::Json)
                .collect(),
            decisions: array(&value["decisions"]).iter().collect(),
            errors: array(&value["claim_errors"]),
        }
    }
}
impl Projection<'_> {
    pub fn view(&self) -> View<'_> {
        View {
            observations: self.by_hash.values().copied().collect(),
            decisions: self.decisions.iter().map(Decision::value).collect(),
            errors: &self.errors,
        }
    }
    pub fn issues_view(&self) -> View<'_> {
        View {
            observations: self.by_hash.values().copied().collect(),
            decisions: self
                .decisions
                .iter()
                .filter_map(|d| d.value.get())
                .collect(),
            errors: &self.errors,
        }
    }
    pub fn py(self) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::Value as Py;
        Ok(super::py_dict([
            ("$type", Py::Str("QuantityProjection".into())),
            ("claim_errors", super::into_py(Value::Array(self.errors))?),
            (
                "decisions",
                Py::List(
                    self.decisions
                        .iter()
                        .map(Decision::py)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                "observations",
                Py::List(
                    self.by_hash
                        .into_values()
                        .map(|o| super::borrowed_py(&o.value()))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                "postings",
                Py::List(
                    self.postings
                        .into_iter()
                        .map(Posting::py)
                        .collect::<Result<_, _>>()?,
                ),
            ),
        ]))
    }
}
/// A located observation: its event slot and leg index.
pub(super) type Located = (u32, u32);

/// A posting whose observation is located rather than borrowed.
pub struct PostingState {
    id: String,
    observation: Located,
    kind: String,
    location: Value,
    amount: BigInt,
    state: Value,
}

impl PostingState {
    /// The posting's JSON object (`postings_json`).
    fn json<E: From<String>>(
        &self,
        o: LegRef<'_>,
        exact: &impl Fn(&Value) -> Result<Value, E>,
    ) -> Result<Value, E> {
        let domain = &super::Domain::of_leg(o);
        let mut domain_value = serde_json::Map::new();
        domain_value.insert("$type".into(), json!("QuantityDomain"));
        domain_value.insert("exposure".into(), Value::String(domain.exposure.clone()));
        domain_value.insert("network".into(), Value::String(domain.network.clone()));
        domain_value.insert("rail".into(), exact(&domain.rail)?);
        domain_value.insert("unit".into(), json!("msat"));
        let mut posting = serde_json::Map::new();
        posting.insert("$type".into(), json!("QuantityPosting"));
        posting.insert("amount_msat".into(), super::number(&self.amount));
        posting.insert("asset".into(), exact(&o["asset"])?);
        posting.insert("domain".into(), Value::Object(domain_value));
        posting.insert("location_id".into(), exact(&self.location)?);
        posting.insert("location_kind".into(), Value::String(self.kind.clone()));
        posting.insert("observation_hash".into(), exact(&o["quantity_hash"])?);
        posting.insert("posting_id".into(), Value::String(self.id.clone()));
        posting.insert("state".into(), exact(&self.state)?);
        Ok(Value::Object(posting))
    }
}

/// A projection posting as `postings_json` writes it: read in place when
/// every converted part is its own JSON, else its built object. A read
/// posting holds the three fields it does not borrow; `$type` and `domain`,
/// which no journal reads, are only in a built one.
pub enum PostingView<'a> {
    Read {
        posting: &'a PostingState,
        observation: LegRef<'a>,
        amount: Value,
        kind: Value,
        id: Value,
    },
    Built(Value),
}
impl PostingView<'_> {
    /// The field `key` of the posting's JSON object, or null; `$type` and
    /// `domain` only when the posting was built.
    pub fn at(&self, key: &str) -> &Value {
        static NULL: Value = Value::Null;
        let Self::Read {
            posting: p,
            observation: o,
            amount,
            kind,
            id,
        } = self
        else {
            let Self::Built(posting) = self else {
                unreachable!("two variants")
            };
            return &posting[key];
        };
        match key {
            "amount_msat" => amount,
            "asset" => &o["asset"],
            "location_id" => &p.location,
            "location_kind" => kind,
            "observation_hash" => &o["quantity_hash"],
            "posting_id" => id,
            "state" => &p.state,
            _ => &NULL,
        }
    }
}

/// The arbitration result, owning everything but its observations.
pub(super) struct ProjectionState {
    pub observations: Vec<Located>,
    pub decisions: Vec<Decision>,
    postings: Vec<PostingState>,
    pub errors: Vec<Value>,
}
impl<'a> Projection<'a> {
    pub fn located(self, locate: impl Fn(LegRef<'a>) -> Located) -> ProjectionState {
        ProjectionState {
            observations: self.by_hash.into_values().map(&locate).collect(),
            decisions: self.decisions,
            postings: self
                .postings
                .into_iter()
                .map(|p| PostingState {
                    id: p.id,
                    observation: locate(p.observation),
                    kind: p.kind,
                    location: p.location,
                    amount: p.amount,
                    state: p.state,
                })
                .collect(),
            errors: self.errors,
        }
    }
}
impl ProjectionState {
    /// The postings' JSON objects, as `py` would convert back to JSON.
    pub fn postings_json<'v, E: From<String>>(
        &'v self,
        observation: impl Fn(Located) -> LegRef<'v>,
        exact: &impl Fn(&Value) -> Result<Value, E>,
    ) -> Result<Vec<Value>, E> {
        self.postings
            .iter()
            .map(|p| p.json(observation(p.observation), exact))
            .collect()
    }
    /// Views of the postings `postings_json` writes, converting in its
    /// order: a posting whose parts are all their own JSON is read in
    /// place, any other is built as `postings_json` builds it.
    pub fn posting_views<'v, E: From<String>>(
        &'v self,
        observation: impl Fn(Located) -> LegRef<'v>,
        exact: &impl Fn(&Value) -> Result<Value, E>,
    ) -> Result<Vec<PostingView<'v>>, E> {
        use super::runtime::plain;
        self.postings
            .iter()
            .map(|p| {
                let o = observation(p.observation);
                let rail_plain = match o {
                    LegRef::Typed(_) => true,
                    LegRef::Json(_) => plain(&o["event_key"]["chain"]),
                };
                Ok(
                    if rail_plain
                        && plain(&o["asset"])
                        && plain(&p.location)
                        && plain(&o["quantity_hash"])
                        && plain(&p.state)
                    {
                        PostingView::Read {
                            posting: p,
                            observation: o,
                            amount: super::number(&p.amount),
                            kind: Value::String(p.kind.clone()),
                            id: Value::String(p.id.clone()),
                        }
                    } else {
                        PostingView::Built(p.json(o, exact)?)
                    },
                )
            })
            .collect()
    }
    /// `Projection::py` of the arbitration this state came from.
    pub fn py<'v>(
        &'v self,
        observation: impl Fn(Located) -> LegRef<'v>,
    ) -> Result<kassiber_pyjson::Value, String> {
        use kassiber_pyjson::Value as Py;
        Ok(super::py_dict([
            ("$type", Py::Str("QuantityProjection".into())),
            (
                "claim_errors",
                super::borrowed_py(&Value::Array(self.errors.clone()))?,
            ),
            (
                "decisions",
                Py::List(
                    self.decisions
                        .iter()
                        .map(Decision::py)
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                "observations",
                Py::List(
                    self.observations
                        .iter()
                        .map(|at| super::borrowed_py(&observation(*at).value()))
                        .collect::<Result<_, _>>()?,
                ),
            ),
            (
                "postings",
                Py::List(
                    self.postings
                        .iter()
                        .map(|p| {
                            posting_py(
                                p.id.clone(),
                                observation(p.observation),
                                p.kind.clone(),
                                &p.location,
                                p.amount.clone(),
                                &p.state,
                            )
                        })
                        .collect::<Result<_, _>>()?,
                ),
            ),
        ]))
    }
}

/// Whether every domain's postings sum to zero. Each observation's postings
/// are summed, then each domain's: a posting's domain is its observation's,
/// and integer sums regroup exactly. Only balance is read, so no order is
/// kept.
fn balanced(postings: &[Posting<'_>]) -> bool {
    let mut by_observation: crate::fxhash::HashMap<usize, (LegRef<'_>, BigInt)> =
        Default::default();
    for p in postings {
        by_observation
            .entry(p.observation.address())
            .or_insert_with(|| (p.observation, BigInt::from(0)))
            .1 += &p.amount;
    }
    let mut totals: crate::fxhash::HashMap<_, BigInt> = Default::default();
    for (observation, sum) in by_observation.values() {
        *totals
            .entry(super::Domain::leg_key(*observation))
            .or_default() += sum;
    }
    totals.values().all(|n| *n == BigInt::from(0))
}

/// `balanced` in machine integers, or `None` when an amount or a sum does
/// not fit one.
fn balanced_small(postings: &[Posting<'_>]) -> Option<bool> {
    let mut by_observation: crate::fxhash::HashMap<usize, (LegRef<'_>, i128)> =
        crate::fxhash::HashMap::with_capacity_and_hasher(postings.len(), Default::default());
    for p in postings {
        let amount = i128::try_from(&p.amount).ok()?;
        let sum = &mut by_observation
            .entry(p.observation.address())
            .or_insert((p.observation, 0))
            .1;
        *sum = sum.checked_add(amount)?;
    }
    let mut totals: crate::fxhash::HashMap<_, i128> = Default::default();
    for (observation, sum) in by_observation.values() {
        let total = totals
            .entry(super::Domain::leg_key(*observation))
            .or_default();
        *total = total.checked_add(*sum)?;
    }
    Some(totals.values().all(|n| *n == 0))
}

pub(super) fn arbitrate<'a>(
    observations: &[LegRef<'a>],
    claims: &[Value],
) -> Result<Projection<'a>, String> {
    let (by_hash, claims, errors) = validate(observations, claims)?;
    let mut decisions = source_decisions(&by_hash, &claims)?;
    destination_overlaps(&mut decisions);
    atomic_bundles(&mut decisions, &claims);
    let postings = postings(&by_hash, observations, &decisions)?;
    if !balanced_small(&postings).unwrap_or_else(|| balanced(&postings)) {
        return Err("AssertionError".into());
    }
    decisions.sort_by(|a, b| a.source.cmp(&b.source));
    Ok(Projection {
        by_hash,
        decisions,
        postings,
        errors,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn posting_views_read_what_postings_json_writes() {
        let event_key = json!({"chain": "bitcoin", "network": "main"});
        let legs = [
            json!({"asset": "BTC", "quantity_hash": "h0", "event_key": event_key}),
            json!({"asset": 1.5, "quantity_hash": "h1", "event_key": event_key}),
            json!({"asset": "BTC", "quantity_hash": "h2",
                   "event_key": {"chain": 2.5, "network": "main"}}),
            json!({"asset": "BTC", "quantity_hash": 3.5, "event_key": event_key}),
        ];
        let posting = |at: u32, location: Value, state: Value| PostingState {
            id: format!("p{at}"),
            observation: (at, 0),
            kind: "wallet".into(),
            location,
            amount: BigInt::from(-7),
            state,
        };
        let projection = || ProjectionState {
            observations: vec![],
            decisions: vec![],
            postings: vec![
                posting(0, json!("w"), json!("active")),
                posting(1, json!("w"), json!("active")),
                posting(2, json!("w"), json!("active")),
                posting(3, json!("w"), json!("active")),
                posting(0, json!(4.5), json!("active")),
                posting(0, json!("w"), json!(5.5)),
            ],
            errors: vec![],
        };
        let state = projection();
        let exact = |v: &Value| -> Result<Value, String> {
            if crate::quantity::runtime::plain(v) {
                Ok(v.clone())
            } else if v == &json!(9.5) || v == &json!(9.25) {
                Err(format!("conversion of {v}"))
            } else {
                Ok(json!({"converted": v}))
            }
        };
        let locate = |(at, _): Located| LegRef::Json(&legs[at as usize]);
        let json = state.postings_json(locate, &exact).unwrap();
        let views = state.posting_views(locate, &exact).unwrap();
        assert_eq!(views.len(), json.len());
        for (index, (view, posting)) in views.iter().zip(&json).enumerate() {
            let read = matches!(view, PostingView::Read { .. });
            assert_eq!(read, index == 0, "{index}");
            for (key, value) in posting.as_object().unwrap() {
                if read && matches!(key.as_str(), "$type" | "domain") {
                    assert!(view.at(key).is_null(), "{key}");
                    continue;
                }
                assert_eq!(view.at(key), value, "{index} {key}");
            }
            assert!(view.at("absent").is_null());
        }
        // The first failing conversion fails both, alike.
        let mut failing = projection();
        failing.postings[5].state = json!(9.25);
        failing.postings[4].location = json!(9.5);
        let views = failing.posting_views(locate, &exact).err();
        assert_eq!(views, failing.postings_json(locate, &exact).err());
        assert_eq!(views, Some("conversion of 9.5".to_owned()));
    }

    #[test]
    fn domain_balance_matches_for_machine_and_big_integers() {
        let btc = json!({"event_key": {"network": "regtest"}, "asset": "BTC"});
        let usd = json!({"event_key": {"network": "regtest"}, "asset": "USD"});
        let posting = |observation: &'static Value, amount: BigInt| Posting {
            id: String::new(),
            observation: LegRef::Json(observation),
            kind: String::new(),
            location: Value::Null,
            amount,
            state: Value::Null,
        };
        let (btc, usd): (&'static Value, &'static Value) =
            (Box::leak(Box::new(btc)), Box::leak(Box::new(usd)));
        let big: BigInt = BigInt::from(1) << 130usize;
        let cases = [
            (
                vec![(btc, big.clone()), (btc, -big.clone()), (usd, big.clone())],
                false,
            ),
            (
                vec![
                    (btc, BigInt::from(5)),
                    (btc, BigInt::from(-5)),
                    (usd, BigInt::from(1)),
                ],
                false,
            ),
            (vec![(btc, BigInt::from(5)), (btc, BigInt::from(-5))], true),
            (vec![(btc, BigInt::from(5)), (usd, BigInt::from(-5))], false),
            (vec![(btc, big.clone()), (btc, -big.clone())], true),
            (vec![(btc, big.clone()), (usd, -big.clone())], false),
            (
                vec![
                    (btc, BigInt::from(i128::MAX)),
                    (btc, BigInt::from(1)),
                    (btc, -(BigInt::from(i128::MAX) + BigInt::from(1))),
                ],
                true,
            ),
        ];
        for (amounts, expected) in cases {
            let postings: Vec<_> = amounts.into_iter().map(|(o, n)| posting(o, n)).collect();
            assert_eq!(balanced(&postings), expected);
            if let Some(small) = balanced_small(&postings) {
                assert_eq!(small, expected);
            }
        }
    }
    #[test]
    fn quantity_claim_constructor_vectors() {
        let vectors: Value = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_quantity_primitives.json"
        ))
        .unwrap();
        for case in vectors["claim_admission"].as_array().unwrap() {
            let actual = Claim::parse(&case["claim"]);
            if let Some(error) = case["error"].as_str() {
                assert_eq!(actual.err().as_deref(), Some(error), "{}", case["claim"]);
            } else {
                assert!(actual.is_ok(), "{}", case["claim"]);
            }
        }
    }
}
