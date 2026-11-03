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
    Int(i64, std::cell::OnceCell<Value>),
}

impl Field<'_> {
    fn get(&self) -> &Value {
        match self {
            Self::Value(value) => value,
            Self::Static(value) => value,
            Self::Leg(leg, slot) => leg.field(*slot),
            Self::Int(n, value) => value.get_or_init(|| Value::from(*n)),
        }
    }

    fn into_value(self) -> Value {
        match self {
            Self::Value(value) => value,
            Self::Int(n, value) => value.into_inner().unwrap_or_else(|| Value::from(n)),
            other => other.get().clone(),
        }
    }
}

/// Generated fields in key order. A row has a dozen or so, so a sorted
/// vector finds them faster than a tree and allocates once.
#[derive(Clone, Debug, Default)]
struct Overrides<'a>(Vec<(&'static str, Field<'a>)>);

impl<'a> Overrides<'a> {
    fn position(&self, key: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|(name, _)| (*name).cmp(key))
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.position(key).ok().map(|at| self.0[at].1.get())
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn insert(&mut self, key: &'static str, value: Field<'a>) {
        match self.position(key) {
            Ok(at) => self.0[at].1 = value,
            Err(at) => self.0.insert(at, (key, value)),
        }
    }

    fn entry(&mut self, key: &'static str, value: impl FnOnce() -> Value) -> &mut Value {
        let at = match self.position(key) {
            Ok(at) => at,
            Err(at) => {
                self.0.insert(at, (key, Field::Value(value())));
                at
            }
        };
        let field = &mut self.0[at].1;
        if !matches!(field, Field::Value(_)) {
            *field = Field::Value(std::mem::replace(field, Field::Value(Value::Null)).into_value());
        }
        match field {
            Field::Value(value) => value,
            _ => unreachable!("made a value above"),
        }
    }

    fn iter(&self) -> impl Iterator<Item = (&&'static str, &Value)> {
        self.0.iter().map(|(name, value)| (name, value.get()))
    }
}

impl<'a> IntoIterator for Overrides<'a> {
    type Item = (&'static str, Value);
    type IntoIter = std::iter::Map<
        std::vec::IntoIter<(&'static str, Field<'a>)>,
        fn((&'static str, Field<'a>)) -> (&'static str, Value),
    >;

    fn into_iter(self) -> Self::IntoIter {
        self.0
            .into_iter()
            .map(|(name, value)| (name, value.into_value()))
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
            overrides: Overrides(Vec::with_capacity(24)),
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
                overrides: Overrides(Vec::with_capacity(24)),
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
            .0
            .into_iter()
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
}
