use super::*;
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

/// Generated fields in key order. A row has a dozen or so, so a sorted
/// vector finds them faster than a tree and allocates once.
#[derive(Clone, Debug, Default)]
struct Overrides(Vec<(&'static str, Value)>);

impl Overrides {
    fn position(&self, key: &str) -> Result<usize, usize> {
        self.0.binary_search_by(|(name, _)| (*name).cmp(key))
    }

    fn get(&self, key: &str) -> Option<&Value> {
        self.position(key).ok().map(|at| &self.0[at].1)
    }

    fn len(&self) -> usize {
        self.0.len()
    }

    fn insert(&mut self, key: &'static str, value: Value) {
        match self.position(key) {
            Ok(at) => self.0[at].1 = value,
            Err(at) => self.0.insert(at, (key, value)),
        }
    }

    fn entry(&mut self, key: &'static str, value: impl FnOnce() -> Value) -> &mut Value {
        let at = match self.position(key) {
            Ok(at) => at,
            Err(at) => {
                self.0.insert(at, (key, value()));
                at
            }
        };
        &mut self.0[at].1
    }

    fn iter(&self) -> impl Iterator<Item = (&&'static str, &Value)> {
        self.0.iter().map(|(name, value)| (name, value))
    }
}

impl IntoIterator for Overrides {
    type Item = (&'static str, Value);
    type IntoIter = std::vec::IntoIter<(&'static str, Value)>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
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
    overrides: Overrides,
}

impl<'a> FinalizedRow<'a> {
    pub(super) fn borrowed(source: &'a Value) -> Self {
        Self {
            source: Src::Value(Cow::Borrowed(source)),
            overrides: Overrides(Vec::with_capacity(16)),
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
                overrides: Overrides(Vec::with_capacity(16)),
            },
        }
    }

    /// The source row the generated fields override.
    pub(super) fn source(&self) -> RowRef<'_> {
        self.source.row()
    }

    /// Replace a field without first copying its source value.
    pub(super) fn set(&mut self, key: &'static str, value: Value) {
        self.overrides.insert(key, value);
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

/// A finalized row that names its source row by index in the snapshot rows.
#[derive(Clone, Debug)]
pub struct Detached {
    source: Source,
    overrides: Overrides,
}

impl FinalizedRow<'_> {
    /// Detach from `records`: a source borrowed from them becomes its index.
    pub(super) fn detach(self, records: &[Record]) -> Detached {
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
        Detached {
            source,
            overrides: self.overrides,
        }
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
    fn fields<'a>(&'a self, records: &'a [Record]) -> Vec<(&'a str, &'a Value)> {
        let source = self.source(records);
        let mut fields = Vec::with_capacity(source.len() + self.overrides.len());
        let mut overrides = self.overrides.iter().peekable();
        for (key, value) in source.entries() {
            while overrides
                .peek()
                .is_some_and(|(name, _)| **name < key.as_str())
            {
                let (name, value) = overrides.next().unwrap();
                fields.push((*name, value));
            }
            if overrides
                .peek()
                .is_some_and(|(name, _)| **name == key.as_str())
            {
                let (name, value) = overrides.next().unwrap();
                fields.push((*name, value));
            } else {
                fields.push((key.as_str(), value));
            }
        }
        fields.extend(overrides.map(|(name, value)| (*name, value)));
        fields
    }

    /// `FinalizedRow::into_python` of the row this was detached from.
    pub fn py(&self, records: &[Record]) -> Result<Py, String> {
        Ok(Py::Dict(
            self.fields(records)
                .into_iter()
                .map(|(key, value)| {
                    Ok((
                        kassiber_pyjson::Key::Str(key.into()),
                        codec::from_json(value)?,
                    ))
                })
                .collect::<Result<_, String>>()?,
        ))
    }

    /// The row's JSON object, each field cloned when `exact` keeps it.
    pub fn json_with<E>(
        &self,
        records: &[Record],
        exact: &impl Fn(&Value) -> Result<Value, E>,
    ) -> Result<Value, E> {
        Ok(Value::Object(
            self.fields(records)
                .into_iter()
                .map(|(key, value)| Ok((key.to_owned(), exact(value)?)))
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
