use super::*;
use crate::evidence::canonical::{Canonical, Leg, LegRef};
use crate::snapshot::Record;
use serde::ser::SerializeMap;
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::ops::{Index, IndexMut};

pub(super) trait Fields {
    fn get(&self, key: &str) -> Option<&Value>;

    fn field(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&Value::Null)
    }
}

impl Fields for Value {
    fn get(&self, key: &str) -> Option<&Value> {
        self.get(key)
    }
}

/// A generated field: a JSON value, or a part of the row's observation or
/// an integer that is written as JSON only when the row is.
#[derive(Clone, Debug)]
pub(super) enum Field<'a> {
    Value(Value),
    Static(&'static Value),
    /// A field of a typed canonical observation, by its slot.
    Leg(&'a Leg, usize),
    /// An integer, built as a JSON number on first read.
    /// The built number is boxed, so an unread integer keeps fields small.
    Int(i64, std::cell::OnceCell<Box<Value>>),
}

impl Field<'_> {
    fn get(&self) -> &Value {
        match self {
            Self::Value(value) => value,
            Self::Static(value) => value,
            Self::Leg(leg, slot) => leg.field(*slot),
            Self::Int(n, value) => value.get_or_init(|| Box::new(Value::from(*n))),
        }
    }

    fn into_value(self) -> Value {
        match self {
            Self::Value(value) => value,
            Self::Int(n, value) => value
                .into_inner()
                .map_or_else(|| Value::from(n), |value| *value),
            other => other.get().clone(),
        }
    }
}

/// The generated field names the projection writes, in key order.
#[cfg(test)]
const NAMES: [&str; 24] = [
    "_closed_confirmation_observation",
    "amount",
    "amount_includes_fee",
    "asset",
    "custody_chain_order",
    "custody_external_economic_subtype",
    "custody_finalized_tax_projection",
    "custody_quantity_hash",
    "custody_slice_end_msat",
    "custody_slice_start_msat",
    "direction",
    "external_id",
    "fee",
    "fiat_rate",
    "fiat_rate_exact",
    "fiat_value",
    "fiat_value_exact",
    "id",
    "journal_transaction_id",
    "kind",
    "occurred_at",
    "privacy_boundary",
    "raw_json",
    "wallet_id",
];

/// The position of `key` in `NAMES`, or `NONE`; ranks order as their names.
fn rank(key: &str) -> u16 {
    match key {
        "_closed_confirmation_observation" => 0,
        "amount" => 1,
        "amount_includes_fee" => 2,
        "asset" => 3,
        "custody_chain_order" => 4,
        "custody_external_economic_subtype" => 5,
        "custody_finalized_tax_projection" => 6,
        "custody_quantity_hash" => 7,
        "custody_slice_end_msat" => 8,
        "custody_slice_start_msat" => 9,
        "direction" => 10,
        "external_id" => 11,
        "fee" => 12,
        "fiat_rate" => 13,
        "fiat_rate_exact" => 14,
        "fiat_value" => 15,
        "fiat_value_exact" => 16,
        "id" => 17,
        "journal_transaction_id" => 18,
        "kind" => 19,
        "occurred_at" => 20,
        "privacy_boundary" => 21,
        "raw_json" => 22,
        "wallet_id" => 23,
        _ => NONE,
    }
}
const NONE: u16 = u16::MAX;

/// Inline ranks kept for this many generated fields; more are searched in
/// the entries.
const INLINE: usize = 24;

/// Generated fields in the order they were set. Known names are found by
/// their ranks, kept inline beside the entries, so setting a field moves
/// none and a lookup reads one cache line; key order is restored only
/// when the row is written.
#[derive(Clone, Debug)]
struct Overrides<'a> {
    entries: Vec<(&'static str, Field<'a>)>,
    ranks: [u16; INLINE],
}

impl Default for Overrides<'_> {
    fn default() -> Self {
        Self::with_capacity(0)
    }
}

impl<'a> Overrides<'a> {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            entries: Vec::with_capacity(capacity),
            ranks: [NONE; INLINE],
        }
    }

    fn position(&self, key: &str) -> Option<usize> {
        let wanted = rank(key);
        if wanted == NONE {
            return self.entries.iter().position(|(name, _)| *name == key);
        }
        let inline = self.entries.len().min(INLINE);
        self.ranks[..inline]
            .iter()
            .position(|rank| *rank == wanted)
            .or_else(|| {
                self.entries[inline..]
                    .iter()
                    .position(|(name, _)| rank(name) == wanted)
                    .map(|at| at + inline)
            })
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.position(key).map(|at| self.entries[at].1.get())
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn push(&mut self, key: &'static str, value: Field<'a>) -> usize {
        let at = self.entries.len();
        if at < INLINE {
            self.ranks[at] = rank(key);
        }
        self.entries.push((key, value));
        at
    }

    fn insert(&mut self, key: &'static str, value: Field<'a>) {
        match self.position(key) {
            Some(at) => self.entries[at].1 = value,
            None => {
                self.push(key, value);
            }
        }
    }

    fn entry(&mut self, key: &'static str, value: impl FnOnce() -> Value) -> &mut Value {
        let at = match self.position(key) {
            Some(at) => at,
            None => self.push(key, Field::Value(value())),
        };
        let field = &mut self.entries[at].1;
        if !matches!(field, Field::Value(_)) {
            *field = Field::Value(std::mem::replace(field, Field::Value(Value::Null)).into_value());
        }
        match field {
            Field::Value(value) => value,
            _ => unreachable!("made a value above"),
        }
    }

    /// The fields in no particular order.
    fn iter(&self) -> impl Iterator<Item = (&&'static str, &Value)> {
        self.entries.iter().map(|(name, value)| (name, value.get()))
    }

    /// The fields in key order: known names by rank, any other by text,
    /// which is the same order.
    fn into_sorted(self) -> impl Iterator<Item = (&'static str, Field<'a>)> {
        let rank_at = |at: usize| {
            if at < INLINE {
                self.ranks[at]
            } else {
                rank(self.entries[at].0)
            }
        };
        let mut order: Vec<(u16, usize)> = (0..self.entries.len())
            .map(|at| (rank_at(at), at))
            .collect();
        order.sort_unstable_by(|(a_rank, a), (b_rank, b)| {
            if *a_rank != NONE && *b_rank != NONE {
                a_rank.cmp(b_rank)
            } else {
                self.entries[*a].0.cmp(self.entries[*b].0)
            }
        });
        let mut slots: Vec<Option<(&'static str, Field<'a>)>> =
            self.entries.into_iter().map(Some).collect();
        order
            .into_iter()
            .map(move |(_, at)| slots[at].take().expect("each field once"))
    }
}

impl<'a> IntoIterator for Overrides<'a> {
    type Item = (&'static str, Value);
    type IntoIter = Box<dyn Iterator<Item = (&'static str, Value)> + 'a>;

    /// The fields in key order.
    fn into_iter(self) -> Self::IntoIter {
        Box::new(
            self.into_sorted()
                .map(|(name, value)| (name, value.into_value())),
        )
    }
}

/// A source row: a JSON object or a loaded snapshot record, which holds the
/// same fields in the same order.
#[derive(Clone, Copy, Debug)]
pub(super) enum RowRef<'a> {
    Value(&'a Value),
    Record(&'a Record),
}

impl<'a> RowRef<'a> {
    pub(super) fn get(self, key: &str) -> Option<&'a Value> {
        match self {
            Self::Value(row) => row.get(key),
            Self::Record(row) => row.get(key),
        }
    }

    pub(super) fn at(self, key: &str) -> &'a Value {
        self.get(key).unwrap_or(&Value::Null)
    }

    /// Python truthiness of the row: a record is a non-empty dict.
    pub(super) fn truth(self) -> bool {
        match self {
            Self::Value(row) => codec::truth(row),
            Self::Record(row) => !row.is_empty(),
        }
    }

    pub(super) fn is_null(self) -> bool {
        matches!(self, Self::Value(Value::Null))
    }

    /// The row as a JSON object, borrowed when it is one.
    pub(super) fn value(self) -> Cow<'a, Value> {
        match self {
            Self::Value(row) => Cow::Borrowed(row),
            Self::Record(row) => Cow::Owned(Value::Object(row.clone().into_map())),
        }
    }

    /// The row's fields in key order; nothing for a non-object.
    fn entries(self) -> Box<dyn Iterator<Item = (&'a String, &'a Value)> + 'a> {
        match self {
            Self::Value(row) => Box::new(row.as_object().into_iter().flatten()),
            Self::Record(row) => Box::new(row.iter()),
        }
    }

    fn len(self) -> usize {
        match self {
            Self::Value(row) => row.as_object().map_or(0, |row| row.len()),
            Self::Record(row) => row.len(),
        }
    }
}

impl Index<&str> for RowRef<'_> {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.at(key)
    }
}

#[derive(Clone)]
enum Src<'a> {
    Value(Cow<'a, Value>),
    Record(&'a Record),
}

impl Src<'_> {
    fn row(&self) -> RowRef<'_> {
        match self {
            Self::Value(row) => RowRef::Value(row),
            Self::Record(row) => RowRef::Record(row),
        }
    }
}

#[derive(Clone)]
pub(super) struct FinalizedRow<'a> {
    source: Src<'a>,
    overrides: Overrides<'a>,
}

impl<'a> FinalizedRow<'a> {
    pub(super) fn borrowed(source: &'a Value) -> Self {
        Self {
            source: Src::Value(Cow::Borrowed(source)),
            overrides: Overrides::with_capacity(24),
        }
    }

    pub(super) fn owned(source: Value) -> Self {
        Self {
            source: Src::Value(Cow::Owned(source)),
            overrides: Overrides::default(),
        }
    }

    pub(super) fn of(source: RowRef<'a>) -> Self {
        match source {
            RowRef::Value(row) => Self::borrowed(row),
            RowRef::Record(row) => Self {
                source: Src::Record(row),
                overrides: Overrides::with_capacity(24),
            },
        }
    }

    /// The source row the generated fields override.
    pub(super) fn source(&self) -> RowRef<'_> {
        self.source.row()
    }

    /// Replace a field without first copying its source value.
    pub(super) fn set(&mut self, key: &'static str, value: Value) {
        self.overrides.insert(key, Field::Value(value));
    }

    /// Set a field to a constant.
    pub(super) fn set_static(&mut self, key: &'static str, value: &'static Value) {
        self.overrides.insert(key, Field::Static(value));
    }

    /// Set a field to an integer, written as JSON only when read or written.
    pub(super) fn set_int(&mut self, key: &'static str, value: i64) {
        self.overrides
            .insert(key, Field::Int(value, std::cell::OnceCell::new()));
    }

    /// Set a field to the observation's field `name`, borrowed from a typed
    /// observation.
    pub(super) fn set_observed(&mut self, key: &'static str, observation: LegRef<'a>, name: &str) {
        let field = match (observation, crate::evidence::canonical::slot(name)) {
            (LegRef::Typed(leg), Some(slot)) => Field::Leg(leg, slot),
            _ => Field::Value(observation.at(name).clone()),
        };
        self.overrides.insert(key, field);
    }

    pub(super) fn into_json(self) -> Value {
        let mut result = match self.source {
            Src::Value(row) => row.into_owned(),
            Src::Record(row) => Value::Object(row.clone().into_map()),
        };
        for (key, value) in self.overrides {
            result[key] = value;
        }
        result
    }

    pub(super) fn into_python(self) -> Result<Py, String> {
        let source = self.source.row();
        let mut fields = Vec::with_capacity(source.len() + self.overrides.len());
        let mut overrides = self.overrides.into_iter().peekable();
        for (key, value) in source.entries() {
            while overrides
                .peek()
                .is_some_and(|(name, _)| *name < key.as_str())
            {
                let (name, value) = overrides.next().unwrap();
                fields.push((
                    kassiber_pyjson::Key::Str(name.into()),
                    super::composition::into_python(value)?,
                ));
            }
            let value = if overrides
                .peek()
                .is_some_and(|(name, _)| *name == key.as_str())
            {
                super::composition::into_python(overrides.next().unwrap().1)?
            } else {
                codec::from_json(value)?
            };
            fields.push((kassiber_pyjson::Key::Str(key.as_str().into()), value));
        }
        for (key, value) in overrides {
            fields.push((
                kassiber_pyjson::Key::Str(key.into()),
                super::composition::into_python(value)?,
            ));
        }
        Ok(Py::Dict(fields))
    }
}

/// A finalized row's source, independent of the borrow it was built from.
#[derive(Clone, Debug)]
enum Source {
    Row(usize),
    Owned(Value),
}

/// A detached generated field: a part of an observation is named by its
/// position in the canonical input the projection holds.
#[derive(Clone, Debug)]
enum Stored {
    Value(Value),
    Static(&'static Value),
    Leg((u32, u32), usize),
    Int(i64),
}

/// A field of a detached row as it is written.
enum Written<'a> {
    Value(&'a Value),
    Int(i64),
}

/// A finalized row that names its source row by index in the snapshot rows.
#[derive(Clone, Debug)]
pub struct Detached {
    source: Source,
    overrides: Vec<(&'static str, Stored)>,
}

impl FinalizedRow<'_> {
    /// Detach from `records`: a source borrowed from them becomes its index,
    /// and an observation field of `canonical` its position there.
    pub(super) fn detach(self, records: &[Record], canonical: Option<&Canonical>) -> Detached {
        let source = match self.source {
            Src::Record(source) => {
                let at = (source as *const Record as usize).wrapping_sub(records.as_ptr() as usize)
                    / std::mem::size_of::<Record>();
                if at < records.len() && std::ptr::eq(&records[at], source) {
                    Source::Row(at)
                } else {
                    Source::Owned(Value::Object(source.clone().into_map()))
                }
            }
            Src::Value(source) => Source::Owned(source.into_owned()),
        };
        let overrides = self
            .overrides
            .into_sorted()
            .map(|(name, field)| {
                let stored = match field {
                    Field::Value(value) => Stored::Value(value),
                    Field::Static(value) => Stored::Static(value),
                    Field::Int(n, _) => Stored::Int(n),
                    Field::Leg(leg, slot) => match canonical
                        .and_then(|canonical| canonical.leg_at(leg.position()))
                    {
                        Some(held) if std::ptr::eq(held, leg) => Stored::Leg(leg.position(), slot),
                        _ => Stored::Value(leg.field(slot).clone()),
                    },
                };
                (name, stored)
            })
            .collect();
        Detached { source, overrides }
    }
}

impl Detached {
    fn source<'a>(&'a self, records: &'a [Record]) -> RowRef<'a> {
        match &self.source {
            Source::Row(at) => RowRef::Record(&records[*at]),
            Source::Owned(value) => RowRef::Value(value),
        }
    }

    /// The merged fields in key order: source fields, replaced or joined by
    /// the generated ones.
    fn fields<'a>(
        &'a self,
        records: &'a [Record],
        canonical: Option<&'a Canonical>,
    ) -> Vec<(&'a str, Written<'a>)> {
        let written = |stored: &'a Stored| match stored {
            Stored::Value(value) => Written::Value(value),
            Stored::Static(value) => Written::Value(value),
            Stored::Int(n) => Written::Int(*n),
            Stored::Leg(position, slot) => Written::Value(
                canonical
                    .and_then(|canonical| canonical.leg_at(*position))
                    .expect("detached observation in the projection's input")
                    .field(*slot),
            ),
        };
        let source = self.source(records);
        let mut fields = Vec::with_capacity(source.len() + self.overrides.len());
        let mut overrides = self.overrides.iter().peekable();
        for (key, value) in source.entries() {
            while overrides
                .peek()
                .is_some_and(|(name, _)| *name < key.as_str())
            {
                let (name, value) = overrides.next().unwrap();
                fields.push((*name, written(value)));
            }
            if overrides
                .peek()
                .is_some_and(|(name, _)| *name == key.as_str())
            {
                let (name, value) = overrides.next().unwrap();
                fields.push((*name, written(value)));
            } else {
                fields.push((key.as_str(), Written::Value(value)));
            }
        }
        fields.extend(overrides.map(|(name, value)| (*name, written(value))));
        fields
    }

    /// `FinalizedRow::into_python` of the row this was detached from.
    pub fn py(&self, records: &[Record], canonical: Option<&Canonical>) -> Result<Py, String> {
        Ok(Py::Dict(
            self.fields(records, canonical)
                .into_iter()
                .map(|(key, value)| {
                    let value = match value {
                        Written::Value(value) => codec::from_json(value)?,
                        Written::Int(n) => Py::Int(BigInt::from(n)),
                    };
                    Ok((kassiber_pyjson::Key::Str(key.into()), value))
                })
                .collect::<Result<_, String>>()?,
        ))
    }

    /// The row's JSON object, each field cloned when `exact` keeps it.
    pub fn json_with<E>(
        &self,
        records: &[Record],
        canonical: Option<&Canonical>,
        exact: &impl Fn(&Value) -> Result<Value, E>,
    ) -> Result<Value, E> {
        Ok(Value::Object(
            self.fields(records, canonical)
                .into_iter()
                .map(|(key, value)| {
                    let value = match value {
                        Written::Value(value) => exact(value)?,
                        Written::Int(n) => Value::from(n),
                    };
                    Ok((key.to_owned(), value))
                })
                .collect::<Result<_, E>>()?,
        ))
    }
}

/// A detached row's fields as its JSON object (`Detached::json_with`) holds
/// them, without building the object. Fields `exact` converts and integer
/// fields are built once, when the view is; every other field is borrowed.
#[derive(Debug)]
pub struct RowView<'a> {
    source: RowRef<'a>,
    overrides: &'a [(&'static str, Stored)],
    canonical: Option<&'a Canonical>,
    /// Converted and integer fields, by name.
    built: Vec<(&'a str, Value)>,
}

impl<'a> RowView<'a> {
    /// The view of `row`. `exact` returns a field's conversion, or `None`
    /// when the field is its own JSON; as in `json_with`, the first field
    /// whose conversion fails, in key order, fails the view.
    pub(super) fn new<E>(
        row: &'a Detached,
        records: &'a [Record],
        canonical: Option<&'a Canonical>,
        exact: &impl Fn(&Value) -> Result<Option<Value>, E>,
    ) -> Result<Self, E> {
        let mut built = Vec::new();
        for (key, value) in row.fields(records, canonical) {
            match value {
                Written::Value(value) => {
                    if let Some(value) = exact(value)? {
                        built.push((key, value));
                    }
                }
                Written::Int(n) => built.push((key, Value::from(n))),
            }
        }
        Ok(Self {
            source: row.source(records),
            overrides: &row.overrides,
            canonical,
            built,
        })
    }

    /// The field `key`, as the row's JSON object holds it.
    pub fn get(&self, key: &str) -> Option<&Value> {
        if let Some((_, value)) = self.built.iter().find(|(name, _)| *name == key) {
            return Some(value);
        }
        match self
            .overrides
            .binary_search_by(|(name, _)| (*name).cmp(key))
        {
            Ok(at) => Some(match &self.overrides[at].1 {
                Stored::Value(value) => value,
                Stored::Static(value) => value,
                Stored::Leg(position, slot) => self
                    .canonical
                    .and_then(|canonical| canonical.leg_at(*position))
                    .expect("detached observation in the projection's input")
                    .field(*slot),
                Stored::Int(_) => unreachable!("integer fields are built"),
            }),
            Err(_) => self.source.get(key),
        }
    }

    /// Whether the row has a field whose name satisfies `test`.
    pub fn any_key(&self, test: impl Fn(&str) -> bool) -> bool {
        self.overrides.iter().any(|(name, _)| test(name))
            || self.source.entries().any(|(name, _)| test(name))
    }

    /// The row's JSON object.
    pub fn json(&self) -> Value {
        let mut fields = serde_json::Map::new();
        for (key, value) in self.source.entries() {
            fields.insert(key.clone(), value.clone());
        }
        for (key, _) in self.overrides {
            let value = self.get(key).expect("an override is a field");
            fields.insert((*key).to_owned(), value.clone());
        }
        for (key, value) in &self.built {
            fields.insert((*key).to_owned(), value.clone());
        }
        Value::Object(fields)
    }
}

impl Fields for FinalizedRow<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.overrides
            .get(key)
            .or_else(|| self.source.row().get(key))
    }
}

impl Index<&str> for FinalizedRow<'_> {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.field(key)
    }
}

impl IndexMut<&'static str> for FinalizedRow<'_> {
    fn index_mut(&mut self, key: &'static str) -> &mut Value {
        let source = self.source.row();
        self.overrides.entry(key, || source[key].clone())
    }
}

impl serde::Serialize for FinalizedRow<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = BTreeMap::new();
        for (key, value) in self.source.row().entries() {
            fields.insert(key.as_str(), value);
        }
        fields.extend(self.overrides.iter().map(|(key, value)| (*key, value)));
        let mut map = serializer.serialize_map(Some(fields.len()))?;
        for (key, value) in fields {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn many_overrides_keep_their_values_and_key_order() {
        // More names than the inline ranks hold, known and unknown, set out
        // of order and some twice.
        const EXTRA: [&str; 12] = [
            "zz",
            "a",
            "b_unknown",
            "m",
            "y",
            "c",
            "q",
            "x1",
            "x2",
            "x3",
            "x4",
            "x5",
        ];
        let mut row = FinalizedRow::owned(json!({"id": "source", "zz": 0, "kept": 1}));
        let names: Vec<&'static str> = EXTRA.iter().chain(NAMES.iter().rev()).copied().collect();
        for (at, name) in names.iter().enumerate() {
            row.set(name, json!(at));
        }
        row.set("id", json!("again"));
        row.set_int("amount", 7);
        for (at, name) in names.iter().enumerate() {
            let expected = match *name {
                "id" => json!("again"),
                "amount" => json!(7),
                _ => json!(at),
            };
            assert_eq!(row[name], expected, "{name}");
        }
        assert_eq!(row["kept"], json!(1));
        let written = row.clone().into_json();
        let python = row.into_python().unwrap();
        assert_eq!(crate::codec::to_json(&python), written);
        // The Python dict lists the merged fields in key order.
        let Py::Dict(items) = python else {
            panic!("a dict")
        };
        let keys: Vec<String> = items
            .iter()
            .map(|(key, _)| match key {
                kassiber_pyjson::Key::Str(key) => key.as_str().unwrap().to_owned(),
                _ => panic!("text keys"),
            })
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        assert_eq!(keys.len(), names.len() + 1);
    }

    #[test]
    fn generated_fields_stay_small() {
        assert!(std::mem::size_of::<Field<'_>>() <= 40);
    }

    #[test]
    fn override_ranks_order_as_names() {
        assert!(NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        for (at, name) in NAMES.iter().enumerate() {
            assert_eq!(rank(name), at as u16, "{name}");
        }
        for name in ["", "Id", "amount_", "zz", "custody"] {
            assert_eq!(rank(name), NONE, "{name}");
        }
    }

    use super::*;

    #[test]
    fn finalized_borrowed_rows_preserve_overrides_and_source_bytes() {
        let source: Value = serde_json::from_str(
            r#"{"a":1,"id":"raw","nested":{"v":[18446744073709551616,-0.0]},"z":"tail"}"#,
        )
        .unwrap();
        let before = source.to_string();
        let mut row = FinalizedRow::borrowed(&source);
        row["a"] = Value::Null;
        row["id"] = json!("slice");
        row["fee"] = json!(7);
        row["zz"] = json!(true);
        let mut sibling = row.clone();
        sibling["id"] = json!("sibling");
        assert_eq!(row["id"], "slice");
        let expected = r#"{"a":null,"fee":7,"id":"slice","nested":{"v":[18446744073709551616,-0.0]},"z":"tail","zz":true}"#;
        assert_eq!(row.clone().into_json().to_string(), expected);
        assert_eq!(serde_json::to_string(&row).unwrap(), expected);
        assert_eq!(
            codec::dumps(&row.into_python().unwrap(), true).unwrap(),
            expected
        );
        let Py::Dict(emitted) = sibling.into_python().unwrap() else {
            panic!("finalized row");
        };
        let names: Vec<_> = emitted.into_iter().map(|(key, _)| key).collect();
        assert_eq!(
            names,
            ["a", "fee", "id", "nested", "z", "zz"]
                .map(|key| kassiber_pyjson::Key::Str(key.into()))
        );
        assert_eq!(source.to_string(), before);
    }

    #[test]
    fn finalized_rows_without_source_keep_generated_fields() {
        let mut row = FinalizedRow::borrowed(&Value::Null);
        row["id"] = json!("generated");
        row["amount"] = json!(0);
        let emitted = row.into_python().unwrap();
        assert_eq!(
            codec::dumps(&emitted, true).unwrap(),
            r#"{"amount":0,"id":"generated"}"#
        );
    }

    #[test]
    fn views_read_the_fields_json_with_writes() {
        use crate::snapshot::Record;
        let source: serde_json::Map<String, Value> = serde_json::from_str(
            r#"{"amount": 5, "custody_x": 1, "fee": 1.5, "id": "s", "kept": "k", "zz": 2.25}"#,
        )
        .unwrap();
        let records = vec![Record::from(source)];
        let owned = json!({"a": 1, "fiat_rate": 0.5, "id": "o"});
        static FIXED: Value = Value::Null;
        let build = |of: RowRef<'_>| {
            let mut row = FinalizedRow::of(of);
            row.set("id", json!("generated"));
            row.set("fiat_value", json!(1.25));
            row.set_int("amount", 7);
            row.set_int("custody_slice_end_msat", 9);
            row.set_static("privacy_boundary", &FIXED);
            row.set("b_unknown", json!(["x", 3.5]));
            row.detach(&records, None)
        };
        // `plain` JSON is its own conversion; any other value converts to a
        // tagged copy, and 1.5 and 2.25 fail: the first in key order wins.
        let exact = |v: &Value| -> Result<Value, String> {
            if crate::quantity::runtime::plain(v) {
                Ok(v.clone())
            } else if v == &json!(1.5) || v == &json!(2.25) {
                Err(format!("cannot convert {v}"))
            } else {
                Ok(json!({"converted": v}))
            }
        };
        let built = |v: &Value| -> Result<Option<Value>, String> {
            if crate::quantity::runtime::plain(v) {
                Ok(None)
            } else {
                exact(v).map(Some)
            }
        };
        for detached in [
            build(RowRef::Record(&records[0])),
            build(RowRef::Value(&owned)),
        ] {
            let json = detached.json_with(&records, None, &exact);
            let view = RowView::new(&detached, &records, None, &built);
            let (json, view) = match (json, view) {
                (Err(a), Err(b)) => {
                    assert_eq!(a, b);
                    assert_eq!(a, "cannot convert 1.5");
                    continue;
                }
                (Ok(json), Ok(view)) => (json, view),
                (json, view) => panic!("{json:?} / {view:?}"),
            };
            assert_eq!(view.json(), json);
            let object = json.as_object().unwrap();
            for (key, value) in object {
                assert_eq!(view.get(key), Some(value), "{key}");
            }
            assert_eq!(view.get("absent"), None);
            assert_eq!(view.get("custody_slice_start_msat"), None);
            for prefix in ["custody_", "kept", "b_", "missing"] {
                assert_eq!(
                    view.any_key(|k| k.starts_with(prefix)),
                    object.keys().any(|k| k.starts_with(prefix)),
                    "{prefix}"
                );
            }
            assert_eq!(view.get("amount"), Some(&json!(7)));
            assert_eq!(view.get("fiat_value"), Some(&json!({"converted": 1.25})));
        }
        // The record source fails on its own fields `fee` and `zz`; without
        // them it gives a view.
        let mut source = records[0].clone().into_map();
        source.remove("fee");
        source.remove("zz");
        let records = vec![Record::from(source)];
        let detached = build(RowRef::Record(&records[0]));
        assert!(RowView::new(&detached, &records, None, &built).is_ok());
    }
}
