//! Complete interpreter compilation from raw snapshots and Rust stages.
//! Python remains authoritative; these results are comparison-only.
pub(crate) mod channels;
mod chronology;
mod claims;
mod consolidation;
mod fanout;
mod fifo;
mod holds;
mod htlc;
mod matcher;
mod migration;
mod native;
mod ownership;
mod pairs;
mod pipeline;
mod privacy;
mod reconciliation;
mod recorded;
mod reviewed;
mod safety;
mod samourai;
pub(crate) mod text;
use crate::evidence::canonical::{EventRef, LegRef};
use crate::{
    codec::{text, truth},
    components, evidence,
    snapshot::Snapshot,
};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

pub(crate) use holds::{capture_confirmation, ledger_conflicts, ledger_pending};
pub use migration::quarantines as migration_quarantines;
pub(crate) use text::repr as value_repr;

pub(crate) fn canonical_event_repr(event: &Value) -> String {
    text::event_repr(event)
}

/// Python loads channel context before materializing components.
pub fn admit_channels(snapshot: &Snapshot) -> Result<(), String> {
    channels::compute(snapshot).map(|_| ())
}

pub fn journal_context(
    snapshot: &Snapshot,
    stages: &crate::Stages,
) -> Result<(Value, Vec<String>, Vec<String>), String> {
    let roles = channels::compute(snapshot)?["roles"].clone();
    let (ignored, accounting) = gap_ignored_ids(
        snapshot,
        &stages.components,
        &stages.evidence,
        &stages.interpretation,
    )?;
    Ok((
        roles,
        ignored.into_iter().collect(),
        accounting.into_iter().collect(),
    ))
}

/// Channel context remains an unported neighbour; empty channel populations
/// require no row materialization. Nonempty contexts retain their exact reader.
pub fn admit_typed_channels(snapshot: &crate::snapshot::BookSnapshot) -> Result<(), String> {
    channels::admit_typed(snapshot)
}

/// Channel lifecycle exclusions used by finalized projection.
pub(crate) fn channel_non_event_ids(snapshot: &Snapshot) -> Result<Vec<String>, String> {
    let channels = channels::compute(snapshot)?;
    Ok(channels["roles"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, role)| matches!(role.as_str(), Some("channel_open" | "channel_close")))
        .map(|(id, _)| id.clone())
        .collect())
}

/// Gap discovery ignores authoritative target-bearing edges, component anchors,
/// loans and channel lifecycle rows. Suspense claims remain advisory sources;
/// blocked rows are excluded only from the separate accounting population.
pub(crate) fn gap_ignored_ids(
    snapshot: &Snapshot,
    components: &Value,
    evidence: &evidence::Evidence,
    interpretation: &kassiber_pyjson::Value,
) -> Result<(BTreeSet<String>, BTreeSet<String>), String> {
    use crate::graph::stored;
    use kassiber_pyjson::Value as Py;
    let mut ignored = BTreeSet::new();
    for leg in &snapshot.loan_legs {
        let value = leg.get("transaction_id").unwrap_or(&Value::Null);
        if !value.is_null() && value != "" {
            ignored.insert(crate::gaps::raw_string(value));
        }
    }
    let channels = channels::compute(snapshot)?;
    if let Some(roles) = channels["roles"].as_object() {
        ignored.extend(roles.keys().cloned());
    }
    for pair in array(&channels["pairs"]) {
        for side in ["out", "in"] {
            let row = &pair[side];
            let id = row
                .get("journal_transaction_id")
                .filter(|id| truth(id))
                .unwrap_or(&row["id"]);
            if !id.is_null() && id != "" {
                ignored.insert(crate::gaps::raw_string(id));
            }
        }
    }
    ignored.extend(excluded(components));
    let needed: BTreeSet<_> = stored::list(stored::get(interpretation, "claims"))
        .iter()
        .filter(|claim| !matches!(stored::get(claim, "target"), Py::Null))
        .flat_map(|claim| {
            ["source", "target"].map(|side| {
                python_value_string(stored::get(stored::get(claim, side), "observation_hash"))
            })
        })
        .collect();
    let observations: BTreeMap<_, _> = if needed.is_empty() {
        BTreeMap::new()
    } else {
        evidence
            .canonical_input()
            .events()
            .iter()
            .flat_map(|event| event.legs())
            .filter(|leg| needed.contains(&text(&leg["quantity_hash"])))
            .map(|leg| (text(&leg["quantity_hash"]), text(&leg["transaction_id"])))
            .collect()
    };
    for claim in stored::list(stored::get(interpretation, "claims")) {
        if matches!(stored::get(claim, "target"), Py::Null) {
            continue;
        }
        for side in ["source", "target"] {
            let hash =
                python_value_string(stored::get(stored::get(claim, side), "observation_hash"));
            if let Some(id) = observations.get(&hash) {
                ignored.insert(id.clone());
            }
        }
    }
    let mut accounting = ignored.clone();
    accounting.extend(
        stored::list(stored::get(interpretation, "blocked_transaction_ids"))
            .iter()
            // A migration can name an unpaired surrogate. Keep it in the
            // interpreter output; no UTF-8 raw transaction id can match it.
            .filter_map(|value| match value {
                Py::Str(id) => id.as_str().map(str::to_owned),
                _ => unreachable!("compiled blocked identity"),
            }),
    );
    Ok((ignored, accounting))
}

pub(crate) fn python_value_string(value: &kassiber_pyjson::Value) -> String {
    text::string(value)
}

/// Python's int() family, before any bounded arithmetic. Raw row and compiler
/// facts may contain strings or containers even though canonical quantities do not.
pub(super) fn integer(value: &Value) -> Result<Value, String> {
    crate::codec::integer(value).map_err(|_| {
        if value.is_null() || value.is_array() || value.is_object() {
            "TypeError"
        } else {
            "ValueError"
        }
        .to_owned()
    })
}

pub(super) fn integer_or_zero(value: &Value) -> Result<Value, String> {
    if truth(value) {
        integer(value)
    } else {
        Ok(json!(0))
    }
}

pub(super) fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

pub(super) fn anchor<R: crate::fields::Fields + ?Sized>(row: &R) -> String {
    if truth(row.at("journal_transaction_id")) {
        text(row.at("journal_transaction_id"))
    } else {
        text(row.at("id"))
    }
}

/// One context observation: a canonical leg read by field, or an owned
/// JSON object. A typed leg builds its JSON object only when a reader needs it.
#[derive(Clone, Debug)]
pub(super) enum Observed<'a> {
    Leg(LegRef<'a>, std::sync::OnceLock<Value>),
    Owned(Value),
}
impl Observed<'_> {
    fn leg(leg: LegRef<'_>) -> Observed<'_> {
        Observed::Leg(leg, std::sync::OnceLock::new())
    }
    /// The observation's JSON object, owned from here on.
    #[cfg(test)]
    pub fn to_mut(&mut self) -> &mut Value {
        if let Observed::Leg(..) = self {
            *self = Observed::Owned((**self).clone());
        }
        match self {
            Observed::Owned(value) => value,
            Observed::Leg(..) => unreachable!("owned observation"),
        }
    }
    fn into_owned(self) -> Observed<'static> {
        match self {
            Observed::Owned(value) => Observed::Owned(value),
            leg => Observed::Owned((*leg).clone()),
        }
    }
}
impl std::ops::Deref for Observed<'_> {
    type Target = Value;
    fn deref(&self) -> &Value {
        match self {
            Observed::Leg(LegRef::Json(value), _) => value,
            Observed::Leg(LegRef::Typed(leg), value) => value.get_or_init(|| leg.value()),
            Observed::Owned(value) => value,
        }
    }
}
impl std::ops::Index<&str> for Observed<'_> {
    type Output = Value;
    fn index(&self, key: &str) -> &Value {
        match self {
            Observed::Leg(leg, _) => leg.at(key),
            Observed::Owned(value) => &value[key],
        }
    }
}

/// Keep the canonical event alias iteration: an alias may resolve to a keeper
/// whose transaction id differs from the original row's id.
fn observations<'a>(events: &[EventRef<'a>]) -> BTreeMap<String, Observed<'a>> {
    fn identity(value: &Value) -> Cow<'_, str> {
        match value.as_str() {
            Some(value) => Cow::Borrowed(value),
            None => Cow::Owned(text(value)),
        }
    }
    // A typed alias names its leg.
    if events.iter().all(|event| event.typed_aliases().is_some()) {
        return events
            .iter()
            .flat_map(|event| {
                let aliases = event.typed_aliases().expect("typed aliases");
                aliases.iter().map(move |alias| {
                    (
                        alias.transaction_id.clone(),
                        Observed::leg(event.leg(alias.leg)),
                    )
                })
            })
            .collect();
    }
    let by_hash: crate::fxhash::HashMap<_, _> = events
        .iter()
        .flat_map(|event| event.legs())
        .map(|item| (identity(item.at("quantity_hash")), item))
        .collect();
    events
        .iter()
        .flat_map(|event| event.aliases())
        .map(|alias| {
            (
                text(&alias[0]),
                Observed::leg(by_hash[&identity(&alias[1])]),
            )
        })
        .collect()
}

/// Owned context observations from a JSON object of fixture values.
#[cfg(test)]
pub(super) fn owned_observations(value: &Value) -> BTreeMap<String, Observed<'static>> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .map(|(k, v)| (k.clone(), Observed::Owned(v.clone())))
        .collect()
}

/// The context's canonical events: borrowed from evidence, or owned JSON.
#[derive(Clone, Debug)]
pub(super) enum Events<'a> {
    Borrowed(Vec<EventRef<'a>>),
    Owned(Vec<Value>),
}
impl Events<'_> {
    /// The events of a JSON canonical input fixture.
    #[cfg(test)]
    pub fn json(canonical: &Value) -> Events<'static> {
        Events::Owned(array(&canonical["events"]).to_vec())
    }
    pub fn iter(&self) -> Vec<EventRef<'_>> {
        match self {
            Events::Borrowed(events) => events.clone(),
            Events::Owned(events) => events.iter().map(EventRef::Json).collect(),
        }
    }
}

/// Canonical evidence and its observations are borrowed from the evidence
/// stage; interpreters only replace whole observations in their own copies.
#[derive(Clone)]
pub(super) struct Context<'a> {
    pub observations: BTreeMap<String, Observed<'a>>,
    pub canonical: Events<'a>,
    pub excluded: BTreeSet<String>,
}
impl Context<'_> {
    fn load(snapshot: &Snapshot) -> Result<Context<'static>, String> {
        let components = components::compute(snapshot).map_err(|e| e.code.to_owned())?;
        let evidence = evidence::stage(snapshot)?;
        Ok(Context::from_stages(&components, &evidence).into_owned())
    }
    fn into_owned(self) -> Context<'static> {
        Context {
            observations: self
                .observations
                .into_iter()
                .map(|(k, v)| (k, v.into_owned()))
                .collect(),
            canonical: Events::Owned(
                self.canonical
                    .iter()
                    .into_iter()
                    .map(|event| event.value().into_owned())
                    .collect(),
            ),
            excluded: self.excluded,
        }
    }
}
impl<'a> Context<'a> {
    fn from_stages(components: &Value, evidence: &'a evidence::Evidence) -> Self {
        let excluded = excluded(components);
        let events: Vec<EventRef<'a>> = evidence
            .canonical_input()
            .events()
            .iter()
            .map(EventRef::Typed)
            .collect();
        Self {
            observations: observations(&events),
            canonical: Events::Borrowed(events),
            excluded,
        }
    }
}

fn excluded(components: &Value) -> BTreeSet<String> {
    array(&components["components"])
        .iter()
        .flat_map(|item| array(&item["legs"]))
        .filter_map(|leg| {
            let value = if truth(&leg["anchor_transaction_id"]) {
                &leg["anchor_transaction_id"]
            } else {
                &leg["transaction_id"]
            };
            truth(value).then(|| text(value))
        })
        .collect()
}

/// Complete interpreter compilation over the already computed raw-fact stages.
pub fn compute(
    snapshot: &Snapshot,
    components: &Value,
    evidence: &evidence::Evidence,
) -> Result<Value, String> {
    let context = Context::from_stages(components, evidence);
    let mut composition = safety::compose(snapshot, &context, array(&components["components"]))?;
    let mut non_events: BTreeSet<_> = array(&composition["non_event_transaction_ids"])
        .iter()
        .map(crate::codec::text)
        .collect();
    non_events.extend(evidence.non_event_ids().iter().cloned());
    Ok(
        json!({"$type":"CustodyInterpreterCompilation","claims":composition["claims"].take(),"native_audits":composition["native_audits"].take(),"non_event_transaction_ids":non_events,"blocked_transaction_ids":composition["blocked_transaction_ids"].take(),"quarantines":composition["quarantines"].take(),"native_component_reconciliations":composition.get("native_component_reconciliations").cloned().unwrap_or(json!([])),"transient_blocked_transaction_ids":composition["transient_blocked_transaction_ids"].take()}),
    )
}

/// Add the legacy migration wrapper without narrowing decoded CPython strings
/// or nonfinite values through serde. Python remains the accounting reference.
pub fn stage(
    snapshot: &Snapshot,
    components: &Value,
    evidence: &evidence::Evidence,
    migration: Vec<kassiber_pyjson::Value>,
) -> Result<kassiber_pyjson::Value, String> {
    use crate::graph::stored;
    use kassiber_pyjson::Value as Py;
    let mut result = crate::codec::from_json(&compute(snapshot, components, evidence)?)?;
    let mut blocked: BTreeSet<_> = stored::list(stored::get(&result, "blocked_transaction_ids"))
        .iter()
        .map(|v| {
            let Py::Str(s) = v else {
                unreachable!("compiled identifier")
            };
            s.clone()
        })
        .collect();
    let mut quarantines = stored::list(stored::get(&result, "quarantines")).to_vec();
    for quarantine in migration {
        let Py::Str(id) = stored::get(&quarantine, "transaction_id") else {
            unreachable!("migration identifier")
        };
        blocked.insert(id.clone());
        quarantines.push(quarantine);
    }
    stored::put(
        &mut result,
        "blocked_transaction_ids",
        Py::List(blocked.into_iter().map(Py::Str).collect()),
    );
    stored::put(&mut result, "quarantines", Py::List(quarantines));
    Ok(result)
}

/// Frozen isolated-vector API; independent of product stage composition.
pub fn partial(snapshot: &Snapshot) -> Result<Value, String> {
    let context = Context::load(snapshot)?;
    Ok(
        json!({"native": native::compile(snapshot, &context), "channels": channels::compile(snapshot), "graph": crate::graph::prerequisites(snapshot).unwrap_or_else(|error|json!({"error":error})), "safety": safety::compile(snapshot, &context)?, "pipeline": pipeline::compile(snapshot, &context)?, "consolidation": consolidation::compile(snapshot, &context)?, "ownership": ownership::compile(snapshot, &context)?, "fanout": fanout::compile(snapshot, &context)?, "recorded": recorded::compile(snapshot, &context)?, "samourai": samourai::compile(snapshot, &context)?, "shared": {
            "observations_by_transaction": context.observations.iter().map(|(k, v)| (k.clone(), (**v).clone())).collect::<BTreeMap<_, _>>(),
            "excluded_transaction_ids": context.excluded,
            "row_anchor_ids": snapshot.transactions.iter().map(|row| anchor(&json!(row))).collect::<Vec<_>>()
        }}),
    )
}

#[cfg(test)]
mod stage_tests {
    #[test]
    fn json_aliases_resolve_as_typed_aliases() {
        let row = |id: &str, event: &str, wallet: &str, at: &str| json!({"id":id,"profile_id":"p","wallet_id":wallet,"wallet_kind":"descriptor","direction":"inbound","asset":"BTC","amount":1000,"fee":0,"occurred_at":at,"source_ref":"src","external_id":event,"raw_json":"{}","network":"regtest"});
        // Two observations share a slot, so one alias names the other's leg;
        // a transaction also appears in two events.
        let rows = [
            row("t1", "e1", "w", "2024-01-02T00:00:00+00:00"),
            row("t2", "e1", "w", "2024-01-01T00:00:00+00:00"),
            row("t3", "e2", "v", "2024-01-01T00:00:00+00:00"),
            row("t1", "e3", "w", "2024-01-01T00:00:00+00:00"),
            row("t4", "e1", "w", "2024-01-03T00:00:00+00:00"),
        ];
        let typed = crate::evidence::canonical_of_rows(&rows).unwrap();
        let json = typed.value().clone();
        let typed_events: Vec<_> = typed.events().iter().map(EventRef::Typed).collect();
        let json_events = crate::evidence::canonical::json_events(&json);
        let flatten = |events: &[EventRef<'_>]| -> Vec<(String, Value)> {
            observations(events)
                .into_iter()
                .map(|(id, o)| (id, (*o).clone()))
                .collect()
        };
        let expected = flatten(&typed_events);
        assert_eq!(expected.len(), 4);
        assert_eq!(flatten(&json_events), expected);
        // A later alias of t1 replaces the earlier one; t4 shares t2's slot
        // and names t2's earlier observation.
        let by_id: BTreeMap<_, _> = expected.into_iter().collect();
        assert_eq!(by_id["t1"]["event_key"]["native_event_id"], "e3");
        assert_eq!(by_id["t4"]["transaction_id"], "t2");
    }

    use super::*;
    use crate::{codec, graph::stored};
    use kassiber_pyjson::Value as Py;
    fn run(input: &Value) -> Result<Py, String> {
        let snapshot: Snapshot = serde_json::from_value(input.clone()).unwrap();
        admit_channels(&snapshot)?;
        let components = components::compute(&snapshot).map_err(|e| e.code.to_owned())?;
        let migration = migration_quarantines(&snapshot)?;
        let evidence = evidence::stage(&snapshot)?;
        stage(&snapshot, &components, &evidence, migration)
    }
    #[test]
    fn interpretation_full_vectors() {
        let inputs: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_interpretation_cases.json"
        ))
        .unwrap();
        let expected: Vec<Value> = serde_json::from_str(include_str!(
            "../../../../tests/fixtures/custody_interpretation_stages.json"
        ))
        .unwrap();
        assert_eq!(inputs.len(), expected.len());
        for (input, expected) in inputs.iter().zip(expected) {
            assert_eq!(input["case"], expected["case"]);
            let actual = run(&input["input"]);
            if let Some(error) = expected.get("error") {
                assert_eq!(
                    actual.err(),
                    error.as_str().map(str::to_owned),
                    "{}",
                    input["case"]
                );
            } else {
                assert_eq!(
                    actual.unwrap(),
                    codec::from_json(&expected["expected"]).unwrap(),
                    "{}",
                    input["case"]
                );
            }
        }
    }
    #[test]
    fn interpretation_wrapper_vectors() {
        let vectors = codec::loads(include_str!(
            "../../../../tests/fixtures/custody_interpretation_wrappers.json"
        ))
        .unwrap();
        for v in stored::list(&vectors) {
            let actual = run(&codec::to_json(stored::get(v, "input")));
            let error = stored::get(v, "error");
            if !matches!(error, Py::Null) {
                assert_eq!(
                    actual.err(),
                    Some(text::string(error)),
                    "{}",
                    text::string(stored::get(v, "case"))
                );
            } else {
                assert_eq!(
                    codec::dumps(&actual.unwrap(), true).unwrap(),
                    codec::dumps(stored::get(v, "expected"), true).unwrap(),
                    "{}",
                    text::string(stored::get(v, "case"))
                );
            }
        }
    }

    #[test]
    fn interpretation_operation_vectors() {
        let vectors = codec::loads(include_str!(
            "../../../../tests/fixtures/custody_interpretation_wrappers.json"
        ))
        .unwrap();
        for v in stored::list(&vectors) {
            let request = codec::dumps(stored::get(v, "input"), false).unwrap();
            let response = codec::loads(&crate::custody_with_finalized(&request, false)).unwrap();
            let error = stored::get(v, "error");
            if !matches!(error, Py::Null) {
                assert_eq!(
                    stored::get(stored::get(&response, "error"), "code"),
                    error,
                    "{}",
                    text::string(stored::get(v, "case"))
                );
                assert!(matches!(stored::get(&response, "stages"), Py::Null));
            } else {
                assert_eq!(
                    codec::dumps(
                        stored::get(stored::get(&response, "stages"), "interpretation"),
                        true
                    )
                    .unwrap(),
                    codec::dumps(stored::get(v, "expected"), true).unwrap(),
                    "{}",
                    text::string(stored::get(v, "case"))
                );
                assert_eq!(
                    stored::get(&response, "skipped_stages"),
                    &codec::from_json(&json!(["finalized_projection", "fusion"])).unwrap()
                );
            }
        }
    }
}
