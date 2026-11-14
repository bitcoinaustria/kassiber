//! JSON the Codex adapter writes, in the key order `codex.ts` builds it.
//!
//! `serde_json::Value` sorts object keys, and the TypeScript broker's
//! objects (and the `-c key=value` TOML it derives from them) keep insertion
//! order, including JavaScript's rule that spreading an existing key
//! replaces its value in place. [`Json`] keeps that order so the app-server
//! sees the same bytes.

use serde::ser::{SerializeMap, SerializeSeq};
use serde::{Serialize, Serializer};
use serde_json::value::RawValue;
use serde_json::Value;

/// An ordered JSON value.
#[derive(Clone, Debug)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
    /// A value passed through as received (an option, a tool's output).
    Value(Value),
    /// JSON text passed through in its own key order (a tool schema).
    Raw(Box<RawValue>),
}

impl Json {
    /// `{}`.
    pub fn obj() -> Self {
        Json::Obj(Vec::new())
    }

    /// `object[key] = value`: an existing key keeps its position.
    pub fn set(&mut self, key: &str, value: impl Into<Json>) {
        let Json::Obj(entries) = self else {
            return;
        };
        let value = value.into();
        match entries.iter_mut().find(|(name, _)| name == key) {
            Some(entry) => entry.1 = value,
            None => entries.push((key.to_owned(), value)),
        }
    }

    /// [`Json::set`], chained.
    pub fn with(mut self, key: &str, value: impl Into<Json>) -> Self {
        self.set(key, value);
        self
    }

    /// `{...self, ...other}`.
    pub fn spread(mut self, other: Json) -> Self {
        if let Json::Obj(entries) = other {
            for (key, value) in entries {
                self.set(&key, value);
            }
        }
        self
    }

    /// `JSON.stringify(self)`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "null".to_owned())
    }

    /// `tomlValue(self)` in `codex.ts`: arrays and objects as TOML inline
    /// tables with JSON-quoted keys, leaves as JSON.
    pub fn to_toml(&self) -> String {
        match self {
            Json::Arr(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(Json::to_toml)
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            Json::Obj(entries) => format!(
                "{{{}}}",
                entries
                    .iter()
                    .map(|(key, value)| format!(
                        "{}={}",
                        serde_json::to_string(key).unwrap_or_default(),
                        value.to_toml()
                    ))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            leaf => leaf.to_json(),
        }
    }
}

impl From<&str> for Json {
    fn from(text: &str) -> Self {
        Json::Str(text.to_owned())
    }
}

impl From<String> for Json {
    fn from(text: String) -> Self {
        Json::Str(text)
    }
}

impl From<bool> for Json {
    fn from(flag: bool) -> Self {
        Json::Bool(flag)
    }
}

impl From<i64> for Json {
    fn from(number: i64) -> Self {
        Json::Int(number)
    }
}

impl From<Vec<Json>> for Json {
    fn from(items: Vec<Json>) -> Self {
        Json::Arr(items)
    }
}

impl Serialize for Json {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Json::Null => serializer.serialize_unit(),
            Json::Bool(flag) => serializer.serialize_bool(*flag),
            Json::Int(number) => serializer.serialize_i64(*number),
            Json::Str(text) => serializer.serialize_str(text),
            Json::Arr(items) => {
                let mut seq = serializer.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Json::Obj(entries) => {
                let mut map = serializer.serialize_map(Some(entries.len()))?;
                for (key, value) in entries {
                    map.serialize_entry(key, value)?;
                }
                map.end()
            }
            Json::Value(value) => value.serialize(serializer),
            Json::Raw(raw) => raw.serialize(serializer),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_insertion_order_and_replaces_spread_keys_in_place() {
        let base = Json::obj().with("b", 1).with("a", "x").with("c", true);
        let spread = base.spread(Json::obj().with("a", "y").with("d", Json::Null));
        assert_eq!(spread.to_json(), r#"{"b":1,"a":"y","c":true,"d":null}"#);
    }

    #[test]
    fn writes_toml_inline_tables_with_quoted_keys() {
        let value = Json::obj()
            .with(":root", "deny")
            .with("list", vec![Json::from(1), Json::from("two")])
            .with("nested", Json::obj().with("enabled", false));
        assert_eq!(
            value.to_toml(),
            r#"{":root"="deny","list"=[1,"two"],"nested"={"enabled"=false}}"#
        );
        assert_eq!(Json::from("a\"b").to_toml(), r#""a\"b""#);
    }
}
