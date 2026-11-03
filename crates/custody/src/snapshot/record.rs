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

/// A remembered column position: the columns' and key's addresses.
#[derive(Clone, Copy, Default)]
struct Slot {
    columns: usize,
    key: usize,
    len: usize,
    at: usize,
}

impl Slot {
    fn line(&self) -> usize {
        let mixed = (self.key ^ self.columns.rotate_left(17) ^ self.len)
            .wrapping_mul(0x9e37_79b9_7f4a_7c15);
        mixed >> (usize::BITS - 8)
    }
}

thread_local! {
    static SLOTS: std::cell::RefCell<[Slot; 256]> =
        const { std::cell::RefCell::new([Slot { columns: 0, key: 0, len: 0, at: 0 }; 256]) };
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
        let names = &self.columns.0;
        // Readers name fields with literals, and records of a load share
        // their columns, so a column found once is remembered by address.
        // A remembered position counts only when its name is `key`.
        let slot = Slot {
            columns: names.as_ptr() as usize,
            key: key.as_ptr() as usize,
            len: key.len(),
            at: 0,
        };
        let line = slot.line();
        let cached = SLOTS.with(|slots| slots.borrow()[line]);
        if cached.columns == slot.columns && cached.key == slot.key && cached.len == slot.len {
            if let Some(name) = names.get(cached.at) {
                if name == key {
                    return Some(&self.values[cached.at]);
                }
            }
        }
        let at = names.binary_search_by(|name| name.as_str().cmp(key)).ok()?;
        SLOTS.with(|slots| slots.borrow_mut()[line] = Slot { at, ..slot });
        Some(&self.values[at])
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

    #[test]
    fn remembered_columns_answer_only_their_own_name() {
        let record: Record = serde_json::from_value(json!({"a": 1, "b": 2, "c": 3})).unwrap();
        let other: Record = serde_json::from_value(json!({"b": 4})).unwrap();
        // One buffer, so each key has the same address and length.
        let mut key = String::from("a");
        for (name, expected) in [("a", Some(1)), ("b", Some(2)), ("c", Some(3)), ("d", None)] {
            key.replace_range(.., name);
            for _ in 0..2 {
                assert_eq!(
                    record.get(&key),
                    expected.map(|n| json!(n)).as_ref(),
                    "{name}"
                );
            }
            assert_eq!(
                other.get(&key),
                (name == "b").then(|| json!(4)).as_ref(),
                "{name}"
            );
        }
    }
}
