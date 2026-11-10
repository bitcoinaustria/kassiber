//! Private selected-row and complete diagnostic composition.
//! Snapshot stage admission remains a separate parity gate.
use super::*;
use crate::evidence::canonical::{EventRef, LegRef};
use crate::python_text::PythonText;
use crate::quantity::DecisionRef;
use std::borrow::Cow;
use std::collections::BTreeMap;
/// Observations by quantity hash.
pub(super) type Observations<'a> = crate::fxhash::HashMap<Cow<'a, str>, LegRef<'a>>;
fn array(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// The compiler's operands. Observations, decisions and events are borrowed
/// from their stage; JSON fixtures read them from one operand object.
pub(super) struct Input<'a> {
    pub op: &'a str,
    pub profile: Cow<'a, Value>,
    pub observations: Vec<LegRef<'a>>,
    pub decisions: Vec<DecisionRef<'a>>,
    pub eligible: Vec<Eligible<'a>>,
    pub events: Vec<EventRef<'a>>,
    pub pool_barriers: Cow<'a, Value>,
    pub pool_barrier_roots: Cow<'a, Value>,
    pub issues: Cow<'a, [Value]>,
    pub blocked_anchor_ids: Cow<'a, [Value]>,
    pub blocked_transaction_ids: Cow<'a, [Value]>,
    pub non_event_transaction_ids: Cow<'a, [Value]>,
    pub payouts: Cow<'a, Value>,
    pub reviewed: Cow<'a, [Value]>,
    /// Each transaction id's place in the source order, by first occurrence
    /// (Python's `source_rank` over `rows_by_id`).
    pub source_rank: crate::fxhash::HashMap<String, usize>,
}

/// First-occurrence places of `ids`, as Python numbers `rows_by_id`'s keys.
pub(super) fn source_rank(
    ids: impl IntoIterator<Item = String>,
) -> crate::fxhash::HashMap<String, usize> {
    let mut rank = crate::fxhash::HashMap::default();
    for id in ids {
        let next = rank.len();
        rank.entry(id).or_insert(next);
    }
    rank
}

impl<'a> Input<'a> {
    pub fn from_json(v: &'a Value) -> Self {
        Self {
            op: v["op"].as_str().unwrap_or(""),
            profile: Cow::Borrowed(&v["profile"]),
            observations: array(&v["observations"]).iter().map(LegRef::Json).collect(),
            decisions: array(&v["decisions"])
                .iter()
                .map(DecisionRef::Json)
                .collect(),
            eligible: array(&v["eligible"])
                .iter()
                .map(|e| Eligible::Slice(Cow::Borrowed(e)))
                .collect(),
            events: array(&v["events"]).iter().map(EventRef::Json).collect(),
            pool_barriers: Cow::Borrowed(&v["pool_barriers"]),
            pool_barrier_roots: Cow::Borrowed(&v["pool_barrier_roots"]),
            issues: Cow::Borrowed(array(&v["issues"])),
            blocked_anchor_ids: Cow::Borrowed(array(&v["blocked_anchor_ids"])),
            blocked_transaction_ids: Cow::Borrowed(array(&v["blocked_transaction_ids"])),
            non_event_transaction_ids: Cow::Borrowed(array(&v["non_event_transaction_ids"])),
            payouts: Cow::Borrowed(&v["payouts"]),
            reviewed: Cow::Borrowed(array(&v["reviewed"])),
            // A JSON object loses Python's key order: operands name it in
            // `row_order`, else the ids sort.
            source_rank: match v["row_order"].as_array() {
                Some(order) => source_rank(order.iter().map(string)),
                None => source_rank(
                    v["rows"]
                        .as_object()
                        .into_iter()
                        .flat_map(|rows| rows.keys().cloned()),
                ),
            },
        }
    }
}
fn internal(d: DecisionRef<'_>) -> bool {
    !d.at("target").is_null()
        && matches!(
            d.at("state").as_str(),
            Some("internal_verified" | "internal_reviewed")
        )
}
fn strings(v: &[Value]) -> BTreeSet<String> {
    v.iter().filter(|v| codec::truth(v)).map(string).collect()
}
/// An eligible slice: a slice object, or a decision's source slice.
pub(super) enum Eligible<'a> {
    Slice(Cow<'a, Value>),
    Source(DecisionRef<'a>),
}
impl Eligible<'_> {
    fn key(&self) -> Result<(Cow<'_, str>, Msat, Msat), String> {
        match self {
            Self::Slice(slice) => slice_key(slice),
            Self::Source(decision) => source_key(*decision),
        }
    }
}
/// `slice_key` of a decision's source slice, typed when the decision is.
fn source_key(d: DecisionRef<'_>) -> Result<(Cow<'_, str>, Msat, Msat), String> {
    match d.typed_source() {
        Some((hash, start, end)) => {
            Ok((Cow::Borrowed(hash), Msat::of_big(start), Msat::of_big(end)))
        }
        None => slice_key(d.at("source")),
    }
}
/// `observation` of a decision's source slice.
fn source_observation<'a>(
    observations: &Observations<'a>,
    d: DecisionRef<'_>,
) -> Result<LegRef<'a>, String> {
    match d.typed_source() {
        Some((hash, ..)) => observations
            .get(hash)
            .copied()
            .ok_or_else(|| "KeyError".into()),
        None => observation(observations, d.at("source")),
    }
}
fn slice_key(v: &Value) -> Result<(Cow<'_, str>, Msat, Msat), String> {
    let start = Msat::of(&v["start_msat"])?;
    let end = Msat::of(&v["end_msat"])?;
    Ok((string_ref(&v["observation_hash"]), start, end))
}
fn observation<'a>(observations: &Observations<'a>, slice: &Value) -> Result<LegRef<'a>, String> {
    observations
        .get(&*string_ref(&slice["observation_hash"]))
        .copied()
        .ok_or_else(|| "KeyError".into())
}
/// Close selected internal paths and propagate their complete root origins.
pub(super) fn connected(
    decisions: &[DecisionRef<'_>],
    observations: &Observations<'_>,
    explicit: &BTreeSet<String>,
) -> Result<Value, String> {
    let mut neighbors: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for d in decisions.iter().filter(|d| internal(**d)) {
        let a = string(&source_observation(observations, *d)?["anchor_transaction_id"]);
        let b = string(&observation(observations, d.at("target"))?["anchor_transaction_id"]);
        neighbors.entry(a.clone()).or_default().insert(b.clone());
        neighbors.entry(b).or_default().insert(a);
    }
    let mut blocked = explicit.clone();
    let mut frontier: Vec<String> = explicit.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        for neighbor in neighbors.get(&id).into_iter().flatten() {
            if blocked.insert(neighbor.clone()) {
                frontier.push(neighbor.clone());
            }
        }
    }
    let mut origins: BTreeMap<String, BTreeSet<String>> = explicit
        .iter()
        .map(|id| (id.clone(), BTreeSet::from([id.clone()])))
        .collect();
    let mut frontier: Vec<_> = explicit.iter().cloned().collect();
    while let Some(id) = frontier.pop() {
        let inherited = origins[&id].clone();
        for neighbor in neighbors.get(&id).into_iter().flatten() {
            let known = origins.entry(neighbor.clone()).or_default();
            if !inherited.is_subset(known) {
                known.extend(inherited.clone());
                // Roots can bridge other roots; propagate every growing set.
                frontier.push(neighbor.clone());
            }
        }
    }
    Ok(json!({"blocked":blocked,"origins":origins}))
}
pub(super) fn pool(o: LegRef<'_>) -> Value {
    let network = match o["event_key"]["network"].as_str().unwrap_or("") {
        "liquidv1" => "main",
        "liquidtestnet" => "test",
        "elementsregtest" => "regtest",
        n => n,
    };
    let asset = string(&o["asset"]);
    let exposure = if matches!(asset.as_str(), "BTC" | "LBTC") {
        "bitcoin".into()
    } else {
        format!("asset:{asset}")
    };
    json!({"profile_id":o["profile_id"],"network":network,"exposure":exposure,"unit":"msat"})
}
pub(super) fn barrier(o: LegRef<'_>, v: &Input) -> Option<Value> {
    let own = pool(o);
    array(&v.pool_barriers)
        .iter()
        .find(|p| {
            ["profile_id", "network", "exposure", "unit"]
                .iter()
                .all(|k| p[0][*k] == own[*k])
        })
        .map(|p| p[1].clone())
}
fn orders(events: &[EventRef]) -> BTreeMap<String, Vec<String>> {
    let mut result = BTreeMap::new();
    for event in events {
        let mut order = vec![event
            .legs()
            .map(|o| string(&o["occurred_at"]))
            .min()
            .unwrap_or_default()];
        for k in ["chain", "network", "native_namespace", "native_event_id"] {
            order.push(string(&event.key()[k]));
        }
        for leg in event.legs() {
            result.insert(string(&leg["quantity_hash"]), order.clone());
        }
    }
    result
}
fn held(o: LegRef<'_>, v: &Input, orders: &BTreeMap<String, Vec<String>>) -> bool {
    if array(&v.pool_barriers).is_empty() {
        return false;
    }
    let Some(b) = barrier(o, v) else {
        return false;
    };
    let Some(order) = orders.get(&*string_ref(&o["quantity_hash"])) else {
        return true;
    };
    let b: Vec<String> = array(&b).iter().map(string).collect();
    order[0] > b[0] || (order[0] == b[0] && *order != b)
}
/// Whether the book's country lets only a pair's own policy decide a
/// BTC/LBTC rail change (`rail_decision_required`, Germany).
fn rail_decision_required(v: &Input) -> bool {
    kassiber_tax_countries::lookup(&text(&v.profile["tax_country"]).py_strip().py_lower())
        .is_some_and(|entry| entry.rail_decision_required)
}
/// Whether two assets are the Bitcoin rail pair (BTC and LBTC).
fn bitcoin_rail(out: &str, incoming: &str) -> bool {
    matches!(
        (
            out.py_strip().py_upper().as_str(),
            incoming.py_strip().py_upper().as_str()
        ),
        ("BTC", "LBTC") | ("LBTC", "BTC")
    )
}
fn privacy(row: &row::FinalizedRow<'_>) -> bool {
    matches!(
        text(&row["privacy_boundary"])
            .py_strip()
            .py_lower()
            .replace('-', "_")
            .as_str(),
        "coinjoin" | "collaborative" | "payjoin" | "payment_in_coinjoin" | "sweep"
    )
}
pub(super) fn evaluate(v: &Value) -> Result<Value, String> {
    let diagnostics = codec::from_json(&v["interpreter_quarantines"])?;
    evaluate_lossless(v, crate::graph::stored::list(&diagnostics))
        .map(|v| codec::to_json(&v))
        .map_err(Error::legacy_code)
}

/// Diagnostic records remain CPython values for their entire lifetime.
/// Private operands carry their rows and a fixed tax eligibility: valuation
/// holds are found here and only block their transactions (the stage
/// recomputes eligibility with them; see [`super::runtime`]).
pub(super) fn evaluate_lossless(v: &Value, diagnostics: &[Py]) -> Result<Py, Error> {
    let input = Input::from_json(v);
    if !matches!(v["op"].as_str(), Some("compose" | "finalize")) {
        return evaluate_with_rows(&input, diagnostics, &v["rows"]);
    }
    let rows = v["rows"]
        .as_object()
        .into_iter()
        .flat_map(|rows| rows.values())
        .map(row::RowRef::Value);
    let blockers = holds(&input, rows, &v["rows"])?;
    if blockers.is_empty() {
        return evaluate_with_rows(&input, diagnostics, &v["rows"]);
    }
    let mut blocked = input.blocked_transaction_ids.to_vec();
    blocked.extend(blocked_ids(&blockers));
    let input = Input {
        blocked_transaction_ids: Cow::Owned(blocked),
        ..input
    };
    evaluate_with_rows(&input, &[diagnostics, &blockers].concat(), &v["rows"])
}

/// The valuation holds of `rows` (`compile_finalized_tax_projection`'s
/// invalid prices and split payouts), before any decision is composed.
pub(super) fn holds<'r>(
    v: &Input,
    rows: impl IntoIterator<Item = row::RowRef<'r>>,
    source: &dyn Rows,
) -> Result<Vec<Py>, Error> {
    let payouts = payouts::valid(&v.payouts, source)?;
    let observations: Observations = v
        .observations
        .iter()
        .map(|o| (string_ref(o.at("quantity_hash")), *o))
        .collect();
    super::valuation::blockers(&v.profile, rows, &observations, &v.decisions, &payouts)
}

/// The transaction ids valuation holds explicitly block.
pub(super) fn blocked_ids(blockers: &[Py]) -> impl Iterator<Item = Value> + '_ {
    blockers
        .iter()
        .map(|item| codec::to_json(crate::graph::stored::get(item, "transaction_id")))
}

/// A finalized composition's parts, before they become a Python value.
pub(super) struct Finalized<'r> {
    pub rows: Vec<row::FinalizedRow<'r>>,
    pub cross: Vec<Value>,
    pub intra: Vec<Value>,
    pub moves: Vec<Value>,
    pub quarantines: Vec<Py>,
}
impl Finalized<'_> {
    pub fn py(self) -> Result<Py, String> {
        let _span = crate::timing::span("finalized.emit");
        let rows = self
            .rows
            .into_iter()
            .map(row::FinalizedRow::into_python)
            .collect::<Result<Vec<_>, _>>()?;
        let mut result = Py::Dict(vec![
            (
                kassiber_pyjson::Key::Str("$type".into()),
                Py::Str("FinalizedTaxProjection".into()),
            ),
            (
                kassiber_pyjson::Key::Str("cross_asset_pairs".into()),
                into_python(Value::Array(self.cross))?,
            ),
            (
                kassiber_pyjson::Key::Str("intra_pairs".into()),
                into_python(Value::Array(self.intra))?,
            ),
            (kassiber_pyjson::Key::Str("rows".into()), Py::List(rows)),
            (
                kassiber_pyjson::Key::Str("selected_move_ids".into()),
                into_python(Value::Array(self.moves))?,
            ),
        ]);
        crate::graph::stored::put(&mut result, "quarantines", Py::List(self.quarantines));
        Ok(result)
    }
}

pub(super) enum Composition<'r> {
    Finalized(Finalized<'r>),
    Other(Py),
}

pub(super) fn evaluate_with_rows(
    v: &Input,
    diagnostics: &[Py],
    rows: &dyn Rows,
) -> Result<Py, Error> {
    match evaluate_parts(v, diagnostics, rows)? {
        Composition::Finalized(finalized) => Ok(finalized.py()?),
        Composition::Other(value) => Ok(value),
    }
}

pub(super) fn evaluate_parts<'r>(
    v: &Input<'r>,
    diagnostics: &[Py],
    rows: &'r dyn Rows,
) -> Result<Composition<'r>, Error> {
    use super::fee_row_with_rows as fee_row;
    use super::principal::slice_row_with_rows as slice_row;
    let selection_span = crate::timing::span("finalized.select");
    let finalize = v.op == "finalize";
    let payouts = if v.op == "compose" || finalize {
        payouts::valid(&v.payouts, rows)?
    } else {
        Value::Null
    };
    let observations: Observations = v
        .observations
        .iter()
        .map(|o| (string_ref(o.at("quantity_hash")), *o))
        .collect();
    let mut decisions: Vec<DecisionRef> = v.decisions.clone();
    // Resolve every source before sorting, matching Python's key admission.
    let mut keyed = Vec::new();
    for d in decisions.drain(..) {
        let source = source_observation(&observations, d)?;
        keyed.push(((string_ref(source.at("occurred_at")), source_key(d)?), d));
    }
    keyed.sort_by(|a, b| a.0.cmp(&b.0));
    decisions = keyed.into_iter().map(|(_, d)| d).collect();
    let explicit = strings(&v.blocked_transaction_ids);
    let closure = connected(&decisions, &observations, &explicit)?;
    if v.op == "connected" {
        return Ok(Composition::Other(codec::from_json(
            &json!({"blocked":closure["blocked"]}),
        )?));
    }
    let blocked = strings(array(&closure["blocked"]));
    let non_events = strings(&v.non_event_transaction_ids);
    let mut blocked_anchors = strings(&v.blocked_anchor_ids);
    if finalize {
        for issue in v.issues.iter() {
            if text(&issue["reason"]).starts_with("custody_component_")
                || matches!(
                    issue["issue_type"].as_str(),
                    Some("component_claim_compile_failed" | "quantity_claim_bundle_invalid")
                )
            {
                blocked_anchors.extend(array(&issue["transaction_ids"]).iter().map(string));
            }
        }
    }
    let order = if array(&v.pool_barriers).is_empty() {
        BTreeMap::new()
    } else {
        orders(&v.events)
    };
    if v.op == "basis" {
        return Ok(Composition::Other(codec::from_json(&json!(v
            .observations
            .iter()
            .map(|o| held(*o, v, &order))
            .collect::<Vec<_>>()))?));
    }
    let eligible: crate::fxhash::HashSet<_> = v
        .eligible
        .iter()
        .map(Eligible::key)
        .collect::<Result<_, _>>()?;
    let mut projected = Vec::new();
    let mut intra = Vec::new();
    let mut cross = Vec::new();
    let mut moves = Vec::new();
    let mut barrier_receipts = Vec::new();
    let mut quarantines =
        super::quarantines::Quarantines::with_interpreter(v, &closure, diagnostics, rows)?;
    let mut fees = FeeAllocation::from_decisions(decisions.iter().copied())?;
    for d in &decisions {
        let source = source_observation(&observations, *d)?;
        let target = if d.at("target").is_null() {
            None
        } else {
            Some(observation(&observations, d.at("target"))?)
        };
        if blocked.contains(&*string_ref(&source["anchor_transaction_id"]))
            || target.is_some_and(|t| blocked.contains(&*string_ref(&t["anchor_transaction_id"])))
        {
            continue;
        }
        if !eligible.contains(&source_key(*d)?) {
            if finalize {
                // Standalone events keep their own valuation cause. A reviewed
                // payout supplies its price; internal moves need none.
                let standalone = matches!(
                    d.at("state").as_str(),
                    Some("external_confirmed" | "external_presumed")
                ) && payouts
                    .get(&*string_ref(&source["anchor_transaction_id"]))
                    .is_none();
                quarantines.basis(source, standalone)?;
            }
            continue;
        }
        if internal(*d) {
            let target = target.expect("internal has target");
            let out = slice_row(
                source,
                d.at("source"),
                rows,
                "move-out",
                &fees.move_fee(*d, source)?,
            )?;
            let input = slice_row(target, d.at("target"), rows, "move-in", &BigInt::from(0))?;
            let id = if codec::truth(d.at("selected_claim_id")) {
                d.at("selected_claim_id").clone()
            } else {
                out["id"].clone()
            };
            moves.push(id.clone());
            if source["asset"] == target["asset"] {
                let mut pair = json!({"out":out,"in":input,"pair_id":id,"source":d.at("reason"),"policy":"carrying-value","group_id":d.at("atomic_bundle_id")});
                if privacy(&out) || privacy(&input) {
                    pair["kind"] = json!("custody-reviewed-coinjoin");
                }
                intra.push(pair);
            } else if !codec::truth(d.at("transfer_policy"))
                && rail_decision_required(v)
                && bitcoin_rail(&string(&source["asset"]), &string(&target["asset"]))
            {
                // The country does not let the book's default decide a rail
                // change (DE-RAIL): without the pair's own policy it is held.
                if finalize {
                    quarantines.rail_policy(source, target)?;
                }
                continue;
            } else {
                let id_text = string(&id);
                let component = id_text
                    .strip_prefix("component:")
                    .map(|s| s.split(':').next().unwrap());
                let durable = id_text
                    .strip_prefix("pair:")
                    .map(|s| s.split(':').next().unwrap())
                    .unwrap_or(&id_text);
                cross.push(json!({"pair_id":durable,"component_id":if codec::truth(d.at("component_id")) {d.at("component_id").clone()} else {json!(component)},"out_id":out["id"],"in_id":input["id"],"out_transaction_id":source["anchor_transaction_id"],"in_transaction_id":target["anchor_transaction_id"],"out_asset":source["asset"],"in_asset":target["asset"],"policy":if codec::truth(d.at("transfer_policy")) {d.at("transfer_policy").clone()} else {json!("carrying-value")},"kind":if codec::truth(d.at("transfer_kind")) {d.at("transfer_kind").clone()} else {json!("custody_cross_rail")}}));
            }
            projected.extend([out, input]);
            continue;
        }
        if non_events.contains(&*string_ref(&source["anchor_transaction_id"])) {
            let fee = fees.raw(source)?;
            if fee != BigInt::from(0) {
                projected.push(fee_row(source, &fee, rows, "non-event-fee")?);
            }
            continue;
        }
        if !matches!(
            d.at("state").as_str(),
            Some("external_confirmed" | "external_presumed")
        ) {
            continue;
        }
        if d.at("destination_kind") == "fee" {
            if let Some(amount) = fees.reviewed_fee(*d)? {
                projected.push(fee_row(source, &amount, rows, "reviewed-fee")?);
            }
            continue;
        }
        let candidate = &payouts[string(&source["anchor_transaction_id"])];
        let payout = if d.at("reason") == "reviewed_direct_payout_source"
            && *d.at("component_id") == candidate["component_id"]
        {
            candidate
        } else {
            &Value::Null
        };
        let asset = text(&payout["payout_asset"]).py_upper();
        let source_asset = string(&source["asset"]);
        let carry = codec::truth(payout)
            && payout["policy"] == "carrying-value"
            && source_asset != asset
            && (kassiber_tax_countries::lookup(&text(&v.profile["tax_country"]).py_lower())
                .is_some_and(|entry| entry.cross_asset_carry)
                || (BTreeSet::from([
                    source_asset.py_strip().py_upper(),
                    asset.py_strip().py_upper(),
                ]) == BTreeSet::from(["BTC".to_owned(), "LBTC".to_owned()])));
        if carry {
            let settlement = payouts::settlement(source, payout, rows)?;
            if settlement.is_null() {
                if finalize {
                    quarantines.payout(source, payout)?;
                }
                continue;
            }
            let out = slice_row(
                source,
                d.at("source"),
                rows,
                "direct-payout-source",
                &fees.raw(source)?,
            )?;
            let input = settlement[0].clone();
            cross.push(json!({"pair_id":format!("direct-payout:{}",string(&payout["id"])),"component_id":null,"kind":if codec::truth(&payout["kind"]) {payout["kind"].clone()} else {json!("direct-swap-payout")},"policy":"carrying-value","out_id":out["id"],"in_id":input["id"],"out_transaction_id":source["anchor_transaction_id"],"in_transaction_id":null,"out_asset":source["asset"],"in_asset":asset}));
            projected.extend([
                out,
                row::FinalizedRow::owned(input),
                row::FinalizedRow::owned(settlement[1].clone()),
            ]);
            continue;
        }
        let fee = fees.raw(source)?;
        let mut row = match d.typed_source() {
            Some((hash, start, end)) => match (i64::try_from(start), i64::try_from(end)) {
                (Ok(start), Ok(end)) => {
                    let slice = super::principal::Slice::Bounds { hash, start, end };
                    slice_row(source, slice, rows, "external", &fee)?
                }
                _ => slice_row(source, d.at("source"), rows, "external", &fee)?,
            },
            None => slice_row(source, d.at("source"), rows, "external", &fee)?,
        };
        if !d.at("external_economic_subtype").is_null() {
            row["kind"] = d.at("external_economic_subtype").clone();
            row["custody_external_economic_subtype"] = d.at("external_economic_subtype").clone();
            row["privacy_boundary"] = Value::Null;
        }
        let amount = match d.typed_source() {
            Some((_, start, end)) => end - start,
            None => {
                integer_big(&d.at("source")["end_msat"])?
                    - integer_big(&d.at("source")["start_msat"])?
            }
        };
        projected.push(payouts::price_row(row, payout, &amount)?);
    }
    // Index the complete decision population once, including unselected targets.
    // Re-scanning it for every receipt makes finalization quadratic.
    let mut targets_by_observation: BTreeMap<Cow<str>, Vec<(Msat, Msat)>> = BTreeMap::new();
    for decision in &decisions {
        if !decision.at("target").is_null() {
            let (hash, start, end) = slice_key(decision.at("target"))?;
            targets_by_observation
                .entry(hash)
                .or_default()
                .push((start, end));
        }
    }
    let mut ordered: Vec<LegRef> = v.observations.clone();
    ordered.sort_by_cached_key(|o| {
        (
            string_ref(o.at("occurred_at")),
            string_ref(o.at("quantity_hash")),
        )
    });
    for o in ordered {
        if o["direction"] != "inbound" || rows.get(&string_ref(&o["transaction_id"])).is_none() {
            continue;
        }
        let anchor = string(&o["anchor_transaction_id"]);
        if non_events.contains(&anchor) {
            continue;
        }
        if held(o, v, &order) {
            barrier_receipts.push(o["quantity_hash"].clone());
            continue;
        }
        if blocked.contains(&anchor) || blocked_anchors.contains(&anchor) {
            continue;
        }
        let mut targets = targets_by_observation
            .get(&*string_ref(&o["quantity_hash"]))
            .cloned()
            .unwrap_or_default();
        targets.sort();
        // An uncovered inbound slice; machine-integer bounds need no object.
        let uncovered = |start: &Msat, end: &Msat| match (start, end) {
            (Msat::Small(start), Msat::Small(end)) if o.at("quantity_hash").is_string() => {
                let slice = super::principal::Slice::Bounds {
                    hash: o.at("quantity_hash").as_str().expect("a text hash"),
                    start: *start,
                    end: *end,
                };
                slice_row(o, slice, rows, "inbound", &BigInt::from(0))
            }
            _ => {
                let q = json!({"observation_hash":o["quantity_hash"],"start_msat":start.value(),"end_msat":end.value()});
                slice_row(o, &q, rows, "inbound", &BigInt::from(0))
            }
        };
        let mut cursor = Msat::Small(0);
        for (start, end) in targets {
            if start > cursor {
                projected.push(uncovered(&cursor, &start)?);
            }
            cursor = cursor.max(end);
        }
        let principal = Msat::of(&o["amount_msat"])?;
        if cursor < principal {
            projected.push(uncovered(&cursor, &principal)?);
        }
    }
    let hashes: crate::fxhash::HashSet<_> = projected
        .iter()
        .map(|r| text_ref(&r["custody_quantity_hash"]))
        .collect();
    let mut fee_only = Vec::new();
    for o in v.observations.iter().copied() {
        if o["direction"] == "outbound"
            && Msat::of(&o["fee_msat"])? > Msat::Small(0)
            && !hashes.contains(&*string_ref(&o["quantity_hash"]))
            && !blocked.contains(&*string_ref(&o["anchor_transaction_id"]))
            && !non_events.contains(&*string_ref(&o["anchor_transaction_id"]))
            && !held(o, v, &order)
        {
            fee_only.push(fee_row(o, &integer_big(&o["fee_msat"])?, rows, "fee-only")?);
        }
    }
    drop(hashes);
    projected.extend(fee_only);
    // Stable, as `sort_by_cached_key` is; the keys borrow the rows.
    // The engine keeps this order for same-time lots: the anchors' source
    // order, then the row id within one anchor.
    let unranked = v.source_rank.len();
    let keys: Vec<_> = projected
        .iter()
        .map(|r| {
            let rank = v
                .source_rank
                .get(text_ref(&r["journal_transaction_id"]).as_ref())
                .copied()
                .unwrap_or(unranked);
            (string_ref(&r["occurred_at"]), rank, string_ref(&r["id"]))
        })
        .collect();
    let mut order: Vec<usize> = (0..projected.len()).collect();
    order.sort_by(|a, b| keys[*a].cmp(&keys[*b]));
    drop(keys);
    let mut slots: Vec<_> = projected.into_iter().map(Some).collect();
    let projected: Vec<_> = order
        .into_iter()
        .map(|at| slots[at].take().expect("each row once"))
        .collect();
    moves.sort_by_key(string);
    drop(selection_span);
    if finalize {
        quarantines.finish(&projected, &barrier_receipts, &observations)?;
        cross = super::conversions::evaluate_rows(&projected, cross, &v.reviewed)?;
        let quarantines = quarantines.sorted()?;
        validate_rows(&projected, &intra).map_err(|(family, _)| family.to_owned())?;
        return Ok(Composition::Finalized(Finalized {
            rows: projected,
            cross,
            intra,
            moves,
            quarantines,
        }));
    }
    validate_rows(&projected, &intra).map_err(|(family, _)| family.to_owned())?;
    Ok(Composition::Other(codec::from_json(
        &json!({"rows":projected,"intra_pairs":intra,"cross_asset_pairs":cross,"selected_move_ids":moves,"barrier_held_receipts":barrier_receipts,"blocked_transaction_ids":closure["blocked"]}),
    )?))
}

// The compiler owns its completed JSON tree. Move its strings and container
// storage into the existing Python-value output instead of copying that tree
// and then immediately destroying the original. Scalar admission stays shared.
pub(super) fn into_python(value: Value) -> Result<Py, String> {
    if codec::raw_bytes(&value).is_some() || codec::escaped_dict(&value).is_some() {
        return codec::from_json(&value);
    }
    match value {
        Value::String(value) => Ok(Py::Str(value.into())),
        Value::Number(value) => match value.as_i64() {
            Some(value) => Ok(Py::Int(BigInt::from(value))),
            None => codec::from_json(&Value::Number(value)),
        },
        Value::Array(values) => values
            .into_iter()
            .map(into_python)
            .collect::<Result<Vec<_>, _>>()
            .map(Py::List),
        Value::Object(values)
            if values.len() == 1
                && (values.contains_key("$kassiber_bytes")
                    || values.contains_key("$kassiber_dict")) =>
        {
            codec::from_json(&Value::Object(values))
        }
        Value::Object(values) => values
            .into_iter()
            .map(|(key, value)| Ok((kassiber_pyjson::Key::Str(key.into()), into_python(value)?)))
            .collect::<Result<Vec<_>, _>>()
            .map(Py::Dict),
        value => codec::from_json(&value),
    }
}

#[cfg(test)]
mod owned_value_tests {
    use super::*;
    use kassiber_pyjson::{Key, Options};

    #[test]
    fn owned_finalization_preserves_bytes_and_literal_mapping_identity() {
        for note in [
            Py::Bytes(vec![]),
            Py::Bytes(vec![255, 0]),
            Py::Bytes(b"77".to_vec()),
            Py::Dict(vec![(Key::from("$kassiber_bytes"), Py::from("ff00"))]),
            Py::Dict(vec![(
                Key::from("$kassiber_dict"),
                Py::Dict(vec![(Key::from("$kassiber_bytes"), Py::from("ff00"))]),
            )]),
        ] {
            let expected = Py::Dict(vec![(
                Key::from("rows"),
                Py::List(vec![Py::Dict(vec![(Key::from("note"), note.clone())])]),
            )]);
            let actual = into_python(codec::to_json(&expected)).unwrap();
            assert_eq!(actual, expected);
            let encoded = kassiber_pyjson::dumps(&actual, &Options::new());
            if matches!(note, Py::Bytes(_)) {
                assert_eq!(
                    encoded.unwrap_err(),
                    kassiber_pyjson::Error::BytesNotSerializable
                );
            } else {
                assert_eq!(
                    encoded.unwrap(),
                    kassiber_pyjson::dumps(&expected, &Options::new()).unwrap()
                );
            }
        }
    }
}
