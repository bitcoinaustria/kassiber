//! Network admission facts. Malformed containers remain values until admission,
//! so protocol parsing cannot move a network refusal ahead of snapshot checks.
use super::rows::NetworkRow;
use super::SqlValue;
use serde::de::{
    value::MapAccessDeserializer, value::SeqAccessDeserializer, MapAccess, SeqAccess, Visitor,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum NetworkObject<T> {
    Object(T),
    Other(SqlValue),
}
#[derive(Clone, Debug, Serialize)]
#[serde(untagged)]
pub enum NetworkArray<T> {
    Array(Vec<T>),
    Other(SqlValue),
}

macro_rules! loose {
    ($name:ident, $visit:ident, $access:ident, $adapter:ident, $variant:ident, $ty:ty) => {
        impl<'de, T: Deserialize<'de>> Deserialize<'de> for $name<T> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Loose<T>(std::marker::PhantomData<T>);
                impl<'de, T: Deserialize<'de>> Visitor<'de> for Loose<T> {
                    type Value = $name<T>;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("network facts") }
                    fn visit_unit<E>(self) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Null)) }
                    fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Bool(v))) }
                    fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Int(v))) }
                    fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> { Ok($name::Other(i64::try_from(v).map(SqlValue::Int).unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))) }
                    fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E> { Ok($name::Other(i64::try_from(v).map(SqlValue::Int).unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))) }
                    fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E> { Ok($name::Other(i64::try_from(v).map(SqlValue::Int).unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))) }
                    fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Real(v))) }
                    fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Text(v.into()))) }
                    fn visit_string<E>(self, v: String) -> Result<Self::Value, E> { Ok($name::Other(SqlValue::Text(v.into_boxed_str()))) }
                    fn $visit<A: $access<'de>>(self, a: A) -> Result<Self::Value, A::Error> {
                        <$ty>::deserialize($adapter::new(a)).map($name::$variant)
                    }
                    loose!(@other $visit, $name);
                }
                d.deserialize_any(Loose(std::marker::PhantomData))
            }
        }
    };
    (@other visit_map, $name:ident) => {
        fn visit_seq<A: SeqAccess<'de>>(self, a: A) -> Result<Self::Value, A::Error> {
            SqlValue::deserialize(SeqAccessDeserializer::new(a)).map($name::Other)
        }
    };
    (@other visit_seq, $name:ident) => {
        fn visit_map<A: MapAccess<'de>>(self, a: A) -> Result<Self::Value, A::Error> {
            SqlValue::deserialize(MapAccessDeserializer::new(a)).map($name::Other)
        }
    };
}
loose!(
    NetworkObject,
    visit_map,
    MapAccess,
    MapAccessDeserializer,
    Object,
    T
);
loose!(
    NetworkArray,
    visit_seq,
    SeqAccess,
    SeqAccessDeserializer,
    Array,
    Vec<T>
);

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkFacts {
    #[serde(default)]
    pub observations: Option<NetworkArray<NetworkObject<NetworkRow>>>,
    #[serde(default)]
    pub references: Value,
    #[serde(default)]
    pub binding: Value,
}
