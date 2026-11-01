use super::*;
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

#[derive(Clone)]
pub(super) struct FinalizedRow<'a> {
    source: Cow<'a, Value>,
    overrides: BTreeMap<&'static str, Value>,
}

impl<'a> FinalizedRow<'a> {
    pub(super) fn borrowed(source: &'a Value) -> Self {
        Self {
            source: Cow::Borrowed(source),
            overrides: BTreeMap::new(),
        }
    }

    pub(super) fn owned(source: Value) -> Self {
        Self {
            source: Cow::Owned(source),
            overrides: BTreeMap::new(),
        }
    }

    pub(super) fn into_json(self) -> Value {
        let mut result = self.source.into_owned();
        for (key, value) in self.overrides {
            result[key] = value;
        }
        result
    }

    pub(super) fn into_python(self) -> Result<Py, String> {
        let source = self.source.as_object();
        let mut fields = Vec::with_capacity(source.map_or(0, |s| s.len()) + self.overrides.len());
        let mut overrides = self.overrides.into_iter().peekable();
        for (key, value) in source.into_iter().flatten() {
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

impl Fields for FinalizedRow<'_> {
    fn get(&self, key: &str) -> Option<&Value> {
        self.overrides.get(key).or_else(|| self.source.get(key))
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
        let source = &self.source;
        self.overrides
            .entry(key)
            .or_insert_with(|| source[key].clone())
    }
}

impl serde::Serialize for FinalizedRow<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut fields = BTreeMap::new();
        for (key, value) in self.source.as_object().into_iter().flatten() {
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
