use serde::{ser::SerializeMap, Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::sync::Arc;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Columns(Arc<[String]>);

impl Columns {
    pub fn new(names: Vec<String>) -> Result<Self, String> {
        if names.windows(2).any(|pair| pair[0] >= pair[1]) {
            return Err("record columns must be sorted and unique".into());
        }
        Ok(Self(names.into()))
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    columns: Columns,
    values: Vec<Value>,
}

impl Record {
    pub fn new(columns: Columns, values: Vec<Value>) -> Self {
        assert_eq!(columns.0.len(), values.len(), "record column count");
        Self { columns, values }
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.columns
            .0
            .binary_search_by(|name| name.as_str().cmp(key))
            .ok()
            .map(|index| &self.values[index])
    }

    pub(super) fn escape_wire_tags(&mut self) {
        self.values
            .iter_mut()
            .for_each(crate::codec::escape_wire_tags);
    }

    pub fn len(&self) -> usize {
        self.values.len()
    }

    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    pub fn contains_key(&self, key: &str) -> bool {
        self.get(key).is_some()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &Value)> {
        self.columns.0.iter().zip(&self.values)
    }

    /// This record read as a typed row, as its JSON text would deserialize.
    pub fn typed<'a, T: Deserialize<'a>>(&'a self) -> Result<T, String> {
        let fields = self.iter().map(|(key, value)| (key.as_str(), value));
        T::deserialize(serde::de::value::MapDeserializer::<_, serde_json::Error>::new(fields))
            .map_err(|e| e.to_string())
    }

    pub fn into_map(self) -> Map<String, Value> {
        self.columns.0.iter().cloned().zip(self.values).collect()
    }
}

impl std::ops::Index<&str> for Record {
    type Output = Value;

    fn index(&self, key: &str) -> &Value {
        self.get(key).expect("no entry found for key")
    }
}

impl From<Map<String, Value>> for Record {
    fn from(map: Map<String, Value>) -> Self {
        let (names, values): (Vec<String>, Vec<Value>) = map.into_iter().unzip();
        Self::new(Columns(Arc::from(names)), values)
    }
}

impl FromIterator<(String, Value)> for Record {
    fn from_iter<T: IntoIterator<Item = (String, Value)>>(items: T) -> Self {
        Map::from_iter(items).into()
    }
}

impl<'a> IntoIterator for &'a Record {
    type Item = (&'a String, &'a Value);
    type IntoIter = std::iter::Zip<std::slice::Iter<'a, String>, std::slice::Iter<'a, Value>>;

    fn into_iter(self) -> Self::IntoIter {
        self.columns.0.iter().zip(&self.values)
    }
}

impl Serialize for Record {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(self.values.len()))?;
        for (key, value) in self.iter() {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Record {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Map::deserialize(deserializer).map(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn shared_record_preserves_map_semantics() {
        let columns = Columns::new(vec!["a".into(), "z".into()]).unwrap();
        let first = Record::new(
            columns.clone(),
            vec![json!(1), json!({"nested": [null, "钱"]})],
        );
        let second = Record::new(columns, vec![json!(2), json!(1.0)]);
        assert!(Arc::ptr_eq(&first.columns.0, &second.columns.0));
        assert_eq!(first["a"], json!(1));
        assert_eq!(second["z"], json!(1.0));
        assert_eq!(first.get("missing"), None);
        assert!(!first.contains_key("missing"));
        let encoded = serde_json::to_string(&first).unwrap();
        assert_eq!(
            encoded,
            serde_json::to_string(&first.clone().into_map()).unwrap()
        );
        let decoded: Record = serde_json::from_str(&encoded).unwrap();
        assert_eq!(decoded, first);
        assert_eq!(Record::default().into_map(), Map::new());
        assert!(Columns::new(vec!["z".into(), "a".into()]).is_err());
        assert!(Columns::new(vec!["a".into(), "a".into()]).is_err());
        let duplicate: Record = serde_json::from_str("{\"a\":1,\"a\":2}").unwrap();
        assert_eq!(duplicate["a"], json!(2));
    }
}
