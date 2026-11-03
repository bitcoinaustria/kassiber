//! Typed canonical quantity input. Observations keep their JSON field values
//! under fixed names, so readers index them like the JSON objects they
//! replace; the objects themselves are built only when a caller needs them.
use super::EventKey;
use serde_json::{Map, Value};
use std::borrow::Cow;
use std::ops::Index;
use std::sync::{Arc, LazyLock, OnceLock};

static NULL: Value = Value::Null;
static OBSERVATION: LazyLock<Value> = LazyLock::new(|| Value::String("QuantityObservation".into()));
static EVENT: LazyLock<Value> = LazyLock::new(|| Value::String("CanonicalQuantityEvent".into()));
static SNAPSHOT: LazyLock<Value> = LazyLock::new(|| Value::String("EvidenceSnapshot".into()));

/// An observation's fields other than `$type` and `event_key`, in key order.
const FIELDS: [&str; 16] = [
    "amount_includes_fee",
    "amount_msat",
    "anchor_transaction_id",
    "asset",
    "authoritative_chain_observation",
    "direction",
    "evidence_detail_hash",
    "evidence_payload_json",
    "fee_attribution",
    "fee_msat",
    "observation_authority_reason",
    "occurred_at",
    "profile_id",
    "quantity_hash",
    "transaction_id",
    "wallet_id",
];

/// The position of `key` in `FIELDS`. Field names differ in length or first
/// letters, so a match finds them without searching.
pub(crate) fn slot(key: &str) -> Option<usize> {
    Some(match key {
        "amount_includes_fee" => 0,
        "amount_msat" => 1,
        "anchor_transaction_id" => 2,
        "asset" => 3,
        "authoritative_chain_observation" => 4,
        "direction" => 5,
        "evidence_detail_hash" => 6,
        "evidence_payload_json" => 7,
        "fee_attribution" => 8,
        "fee_msat" => 9,
        "observation_authority_reason" => 10,
        "occurred_at" => 11,
        "profile_id" => 12,
        "quantity_hash" => 13,
        "transaction_id" => 14,
        "wallet_id" => 15,
        _ => return None,
    })
}

/// One `QuantityObservation`.
#[derive(Clone, Debug)]
pub struct Leg {
    fields: [Value; 16],
    event_key: Arc<Value>,
    /// The (event, leg) position in the canonical input that holds it.
    at: (u32, u32),
}

impl Leg {
    pub(crate) fn new(fields: [Value; 16], event_key: Arc<Value>) -> Self {
        Self {
            fields,
            event_key,
            at: (u32::MAX, u32::MAX),
        }
    }

    /// The field at `slot` of `FIELDS`.
    pub(crate) fn field(&self, slot: usize) -> &Value {
        &self.fields[slot]
    }

    /// The (event, leg) position in its canonical input.
    pub(crate) fn position(&self) -> (u32, u32) {
        self.at
    }

    /// The observation's JSON object.
    pub fn value(&self) -> Value {
        let mut object = Map::new();
        object.insert("$type".into(), OBSERVATION.clone());
        object.insert("event_key".into(), (*self.event_key).clone());
        for (key, value) in FIELDS.iter().zip(&self.fields) {
            object.insert((*key).into(), value.clone());
        }
        Value::Object(object)
    }
}

impl Index<&str> for Leg {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        match key {
            "$type" => &OBSERVATION,
            "event_key" => &self.event_key,
            _ => match slot(key) {
                Some(at) => &self.fields[at],
                None => &NULL,
            },
        }
    }
}

/// One `EvidenceSnapshot`: a selected leg's commitment, which the leg's
/// evidence fields hold, or another observation's.
#[derive(Clone, Debug)]
pub(crate) enum Snapshot {
    Leg(usize),
    Owned {
        quantity_hash: String,
        detail_hash: String,
        payload_json: String,
    },
}

/// One `CanonicalQuantityEvent`.
#[derive(Clone, Debug)]
pub struct Event {
    key: EventKey,
    key_value: Arc<Value>,
    legs: Vec<Leg>,
    snapshots: Vec<Snapshot>,
    /// The snapshots' JSON objects, built on first request.
    snapshot_values: OnceLock<Vec<Value>>,
    sources: Vec<Value>,
    aliases: Vec<Value>,
}

impl Event {
    pub(crate) fn new(
        key: EventKey,
        key_value: Arc<Value>,
        legs: Vec<Leg>,
        snapshots: Vec<Snapshot>,
        sources: Vec<Value>,
        aliases: Vec<Value>,
    ) -> Self {
        Self {
            key,
            key_value,
            legs,
            snapshots,
            snapshot_values: OnceLock::new(),
            sources,
            aliases,
        }
    }

    pub fn key(&self) -> &EventKey {
        &self.key
    }

    pub fn legs(&self) -> &[Leg] {
        &self.legs
    }

    /// A snapshot's quantity hash, detail hash and payload text.
    fn snapshot_fields<'a>(&'a self, snapshot: &'a Snapshot) -> (&'a str, &'a str, &'a str) {
        match snapshot {
            Snapshot::Leg(at) => {
                let leg = &self.legs[*at];
                let text = |slot: usize| leg.fields[slot].as_str().expect("leg commitment text");
                (text(13), text(6), text(7))
            }
            Snapshot::Owned {
                quantity_hash,
                detail_hash,
                payload_json,
            } => (quantity_hash, detail_hash, payload_json),
        }
    }

    /// The `evidence_snapshots` objects.
    fn snapshot_values(&self) -> &[Value] {
        self.snapshot_values.get_or_init(|| {
            self.snapshots
                .iter()
                .map(|snapshot| {
                    let (quantity_hash, detail_hash, payload_json) = self.snapshot_fields(snapshot);
                    let mut object = Map::new();
                    object.insert("$type".into(), SNAPSHOT.clone());
                    object.insert("detail_hash".into(), detail_hash.into());
                    object.insert("payload_json".into(), payload_json.into());
                    object.insert("quantity_hash".into(), quantity_hash.into());
                    Value::Object(object)
                })
                .collect()
        })
    }

    /// The event's JSON object.
    pub fn value(&self) -> Value {
        let mut object = Map::new();
        object.insert("$type".into(), EVENT.clone());
        object.insert("event_key".into(), (*self.key_value).clone());
        object.insert(
            "evidence_snapshots".into(),
            Value::Array(self.snapshot_values().to_vec()),
        );
        object.insert(
            "legs".into(),
            Value::Array(self.legs.iter().map(Leg::value).collect()),
        );
        object.insert(
            "observation_aliases".into(),
            Value::Array(self.aliases.clone()),
        );
        object.insert(
            "source_transaction_ids".into(),
            Value::Array(self.sources.clone()),
        );
        Value::Object(object)
    }
}

/// The `CanonicalQuantityInput`.
#[derive(Debug, Default)]
pub struct Canonical {
    events: Vec<Event>,
    rejected: Vec<Value>,
    value: OnceLock<Value>,
}

impl Canonical {
    pub(crate) fn new(mut events: Vec<Event>, rejected: Vec<Value>) -> Self {
        for (event_at, event) in events.iter_mut().enumerate() {
            for (leg_at, leg) in event.legs.iter_mut().enumerate() {
                leg.at = (event_at as u32, leg_at as u32);
            }
        }
        Self {
            events,
            rejected,
            value: OnceLock::new(),
        }
    }

    /// The leg at `position`, when it is this input's leg.
    pub(crate) fn leg_at(&self, position: (u32, u32)) -> Option<&Leg> {
        self.events
            .get(position.0 as usize)?
            .legs
            .get(position.1 as usize)
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn rejected(&self) -> &[Value] {
        &self.rejected
    }

    /// The input's JSON object if a reader has built it.
    pub fn built_value(&self) -> Option<&Value> {
        self.value.get()
    }

    /// The input's JSON object, built once on first request.
    pub fn value(&self) -> &Value {
        self.value.get_or_init(|| {
            let mut object = Map::new();
            object.insert(
                "$type".into(),
                Value::String("CanonicalQuantityInput".into()),
            );
            object.insert(
                "events".into(),
                Value::Array(self.events.iter().map(Event::value).collect()),
            );
            object.insert(
                "rejected_events".into(),
                Value::Array(self.rejected.clone()),
            );
            Value::Object(object)
        })
    }
}

impl Canonical {
    /// The JSON object, moving its parts.
    pub fn into_value(self) -> Value {
        match self.value.into_inner() {
            Some(value) => value,
            None => Self::new(self.events, self.rejected).value().clone(),
        }
    }
}

impl Clone for Canonical {
    fn clone(&self) -> Self {
        Self::new(self.events.clone(), self.rejected.clone())
    }
}

/// A canonical observation, typed or as a JSON object.
#[derive(Clone, Copy, Debug)]
pub enum LegRef<'a> {
    Typed(&'a Leg),
    Json(&'a Value),
}

impl<'a> LegRef<'a> {
    /// The field `key`, borrowed for the observation's lifetime.
    pub fn at(self, key: &str) -> &'a Value {
        match self {
            Self::Typed(leg) => &leg[key],
            Self::Json(value) => &value[key],
        }
    }

    /// The observation as a JSON object, borrowed when it is one.
    pub fn value(self) -> Cow<'a, Value> {
        match self {
            Self::Typed(leg) => Cow::Owned(leg.value()),
            Self::Json(value) => Cow::Borrowed(value),
        }
    }

    /// The observation's address, which identifies it while it is borrowed.
    pub fn address(self) -> usize {
        match self {
            Self::Typed(leg) => leg as *const Leg as usize,
            Self::Json(value) => value as *const Value as usize,
        }
    }

    /// Whether both name the same observation.
    pub fn same(self, other: Self) -> bool {
        match (self, other) {
            (Self::Typed(a), Self::Typed(b)) => std::ptr::eq(a, b),
            (Self::Json(a), Self::Json(b)) => std::ptr::eq(a, b),
            _ => false,
        }
    }
}

impl Index<&str> for LegRef<'_> {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        match self {
            Self::Typed(leg) => &leg[key],
            Self::Json(value) => &value[key],
        }
    }
}

fn array(value: &Value) -> &[Value] {
    value.as_array().map(Vec::as_slice).unwrap_or(&[])
}

/// A canonical event, typed or as a JSON object.
#[derive(Clone, Copy, Debug)]
pub enum EventRef<'a> {
    Typed(&'a Event),
    Json(&'a Value),
}

impl<'a> EventRef<'a> {
    /// The `event_key` object.
    pub fn key(self) -> &'a Value {
        match self {
            Self::Typed(event) => &event.key_value,
            Self::Json(value) => &value["event_key"],
        }
    }

    pub fn leg_count(self) -> usize {
        match self {
            Self::Typed(event) => event.legs.len(),
            Self::Json(value) => array(&value["legs"]).len(),
        }
    }

    pub fn leg(self, at: usize) -> LegRef<'a> {
        match self {
            Self::Typed(event) => LegRef::Typed(&event.legs[at]),
            Self::Json(value) => LegRef::Json(&array(&value["legs"])[at]),
        }
    }

    pub fn legs(self) -> impl Iterator<Item = LegRef<'a>> {
        (0..self.leg_count()).map(move |at| self.leg(at))
    }

    /// `observation_aliases`: `[transaction_id, quantity_hash]` pairs.
    pub fn aliases(self) -> &'a [Value] {
        match self {
            Self::Typed(event) => &event.aliases,
            Self::Json(value) => array(&value["observation_aliases"]),
        }
    }

    pub fn snapshots(self) -> &'a [Value] {
        match self {
            Self::Typed(event) => event.snapshot_values(),
            Self::Json(value) => array(&value["evidence_snapshots"]),
        }
    }

    /// Each snapshot's `quantity_hash` text, without building typed
    /// snapshots' objects.
    pub fn snapshot_quantity_hashes(self) -> impl Iterator<Item = Cow<'a, str>> {
        let (typed, json) = match self {
            Self::Typed(event) => (Some(event), &[][..]),
            Self::Json(value) => (None, array(&value["evidence_snapshots"])),
        };
        typed
            .into_iter()
            .flat_map(|event| {
                event
                    .snapshots
                    .iter()
                    .map(move |snapshot| Cow::Borrowed(event.snapshot_fields(snapshot).0))
            })
            .chain(
                json.iter()
                    .map(|snapshot| Cow::Owned(crate::codec::text(&snapshot["quantity_hash"]))),
            )
    }

    /// The event as a JSON object, borrowed when it is one.
    pub fn value(self) -> Cow<'a, Value> {
        match self {
            Self::Typed(event) => Cow::Owned(event.value()),
            Self::Json(value) => Cow::Borrowed(value),
        }
    }
}

/// The events of a JSON canonical input.
pub fn json_events(canonical: &Value) -> Vec<EventRef<'_>> {
    array(&canonical["events"])
        .iter()
        .map(EventRef::Json)
        .collect()
}

/// An observation read by field name: a JSON object, a typed leg or a
/// reference to either.
pub trait Obs: for<'k> Index<&'k str, Output = Value> {}
impl<T: for<'k> Index<&'k str, Output = Value> + ?Sized> Obs for T {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leg_slots_name_their_fields() {
        for (at, name) in FIELDS.iter().enumerate() {
            assert_eq!(slot(name), Some(at), "{name}");
        }
        for name in ["", "$type", "event_key", "amount", "wallet_idx", "Asset"] {
            assert_eq!(slot(name), None, "{name}");
        }
    }
}
