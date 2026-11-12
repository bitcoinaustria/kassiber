//! Typed SQLite rows at the snapshot seam. Missing columns are distinct from NULL.
//! JSON-only historical shapes stay lossless until the owning stage admits them.
use super::{InternedId, Record};
use serde::de::{MapAccess, SeqAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::Value;
use std::{fmt, sync::OnceLock};

/// A stored SQLite value, with transport-only variants for historical JSON
/// inputs. In particular a bool, a large integer and an object must never be
/// coerced to an INTEGER, REAL or TEXT at this boundary.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum SqlValue {
    #[default]
    Null,
    Int(i64),
    Real(f64),
    Text(Box<str>),
    Identifier(InternedId),
    Blob(Box<[u8]>),
    Bool(bool),
    IntegerText(Box<str>),
    Array(Vec<SqlValue>),
    Object(Vec<(Box<str>, SqlValue)>),
}

impl SqlValue {
    pub fn json(&self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Int(v) => Value::from(*v),
            Self::Real(v) if v.is_infinite() => serde_json::from_str(if v.is_sign_negative() {
                "-1e999"
            } else {
                "1e999"
            })
            .expect("infinite SQLite scalar transport"),
            Self::Real(v) => Value::from(*v),
            Self::Text(v) => Value::String(v.to_string()),
            Self::Identifier(v) => Value::String(v.as_str().to_owned()),
            Self::Blob(v) => Value::Array(v.iter().map(|b| Value::from(*b)).collect()),
            Self::Bool(v) => Value::Bool(*v),
            Self::IntegerText(v) => serde_json::from_str(v).expect("JSON integer"),
            Self::Array(v) => Value::Array(v.iter().map(Self::json).collect()),
            Self::Object(v) => {
                Value::Object(v.iter().map(|(k, v)| (k.to_string(), v.json())).collect())
            }
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(value) => Some(value),
            Self::Identifier(value) => Some(value.as_str()),
            _ => None,
        }
    }
    fn intern(self) -> Self {
        match self {
            Self::Text(v) => Self::Identifier(InternedId::new(&v)),
            other => other,
        }
    }
    pub fn identifier(&self) -> Option<&InternedId> {
        match self {
            Self::Identifier(v) => Some(v),
            _ => None,
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn truth(&self) -> bool {
        match self {
            Self::Null => false,
            Self::Int(v) => *v != 0,
            Self::Real(v) => *v != 0.0,
            Self::Text(v) => !v.is_empty(),
            Self::Identifier(v) => !v.as_str().is_empty(),
            Self::Blob(v) => !v.is_empty(),
            Self::Bool(v) => *v,
            Self::IntegerText(v) => v.as_ref() != "0",
            Self::Array(v) => !v.is_empty(),
            Self::Object(v) => !v.is_empty(),
        }
    }
}

impl<'de> Deserialize<'de> for SqlValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Scalar;
        impl<'de> Visitor<'de> for Scalar {
            type Value = SqlValue;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a SQLite value or historical JSON value")
            }
            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(SqlValue::Null)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Self::Value, E> {
                Ok(SqlValue::Bool(v))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Self::Value, E> {
                Ok(SqlValue::Int(v))
            }
            fn visit_u64<E>(self, v: u64) -> Result<Self::Value, E> {
                Ok(i64::try_from(v)
                    .map(SqlValue::Int)
                    .unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))
            }
            fn visit_i128<E>(self, v: i128) -> Result<Self::Value, E> {
                Ok(i64::try_from(v)
                    .map(SqlValue::Int)
                    .unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))
            }
            fn visit_u128<E>(self, v: u128) -> Result<Self::Value, E> {
                Ok(i64::try_from(v)
                    .map(SqlValue::Int)
                    .unwrap_or_else(|_| SqlValue::IntegerText(v.to_string().into())))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Self::Value, E> {
                Ok(SqlValue::Real(v))
            }
            fn visit_str<E>(self, v: &str) -> Result<Self::Value, E> {
                Ok(SqlValue::Text(v.into()))
            }
            fn visit_string<E>(self, v: String) -> Result<Self::Value, E> {
                Ok(SqlValue::Text(v.into_boxed_str()))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(v) = a.next_element()? {
                    values.push(v);
                }
                Ok(SqlValue::Array(values))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let mut values = Vec::new();
                while let Some(k) = a.next_key::<Box<str>>()? {
                    // serde_json's arbitrary-precision number transport. Floats
                    // parse with Rust's correctly rounded conversion, as CPython.
                    if values.is_empty() && k.as_ref() == "$serde_json::private::Number" {
                        let n: String = a.next_value()?;
                        return if n.contains(['.', 'e', 'E']) {
                            n.parse()
                                .map(SqlValue::Real)
                                .map_err(serde::de::Error::custom)
                        } else {
                            Ok(n.parse()
                                .map(SqlValue::Int)
                                .unwrap_or_else(|_| SqlValue::IntegerText(n.into_boxed_str())))
                        };
                    }
                    values.push((k, a.next_value()?));
                }
                Ok(SqlValue::Object(values))
            }
        }
        d.deserialize_any(Scalar)
    }
}
impl Serialize for SqlValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Null => s.serialize_unit(),
            Self::Int(v) => s.serialize_i64(*v),
            Self::Real(v) => s.serialize_f64(*v),
            Self::Text(v) => s.serialize_str(v),
            Self::Identifier(v) => s.serialize_str(v.as_str()),
            Self::Blob(v) => v.serialize(s),
            Self::Bool(v) => s.serialize_bool(*v),
            Self::IntegerText(v) => serde_json::from_str::<Value>(v)
                .expect("JSON integer")
                .serialize(s),
            Self::Array(v) => v.serialize(s),
            Self::Object(v) => {
                let mut m = s.serialize_map(Some(v.len()))?;
                for (k, v) in v {
                    m.serialize_entry(k, v)?;
                }
                m.end()
            }
        }
    }
}

/// `None` means the column was absent, not SQL NULL.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Column(pub Option<SqlValue>);
impl Column {
    pub fn value(&self) -> &SqlValue {
        self.0.as_ref().unwrap_or(&SqlValue::Null)
    }
    pub fn present(&self) -> Option<&SqlValue> {
        self.0.as_ref()
    }
    pub fn json(&self) -> Value {
        self.value().json()
    }
    pub fn truth(&self) -> bool {
        self.value().truth()
    }
}

// A direct visitor avoids serde's flatten buffer and allocates no known key.
// The order and unknown columns are wire facts, not a second Value row view.
enum ColumnKey<K> {
    Known(K),
    Unknown(Box<str>),
}
macro_rules! row {
    ($name:ident, $key:ident, {$($field:ident),* $(,)?} $(, $cache:ident)?) => {
        #[derive(Clone, Debug, Default)]
        pub struct $name {
            $(pub $field: Column,)*
            order: Vec<$key>,
            extra: Vec<(Box<str>, SqlValue)>,
            $($cache: OnceLock<Record>,)?
        }
        #[allow(non_camel_case_types)]
        #[derive(Clone, Debug)]
        enum $key { $($field,)* Unknown }
        impl<'de> Deserialize<'de> for ColumnKey<$key> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Key;
                impl Visitor<'_> for Key {
                    type Value = ColumnKey<$key>;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a row column") }
                    fn visit_str<E>(self, key: &str) -> Result<Self::Value, E> {
                        Ok(match key { $(stringify!($field) => ColumnKey::Known($key::$field),)* _ => ColumnKey::Unknown(key.into()) })
                    }
                }
                d.deserialize_identifier(Key)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Row;
                impl<'de> Visitor<'de> for Row {
                    type Value = $name;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a database row") }
                    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                        let mut row = $name::default();
                        while let Some(key) = a.next_key()? {
                            match key {
                                $(ColumnKey::Known($key::$field) => {
                                    let value: SqlValue = a.next_value()?;
                                    row.$field.0 = Some(if matches!(stringify!($field), "id" | "workspace_id" | "profile_id" | "wallet_id" | "asset" | "wallet_account_id" | "journal_transaction_id") { value.intern() } else { value });
                                    row.order.push($key::$field);
                                },)*
                                ColumnKey::Unknown(k) => {
                                    row.extra.push((k, a.next_value()?));
                                    row.order.push($key::Unknown);
                                }
                                ColumnKey::Known($key::Unknown) => unreachable!("known column"),
                            }
                        }
                        Ok(row)
                    }
                }
                d.deserialize_map(Row)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut m = s.serialize_map(Some(self.order.len()))?;
                let mut extra = self.extra.iter();
                for key in &self.order {
                    match key {
                        $($key::$field => m.serialize_entry(stringify!($field), self.$field.value())?,)*
                        $key::Unknown => {
                            let (k, v) = extra.next().expect("unknown column");
                            m.serialize_entry(k, v)?;
                        }
                    }
                }
                m.end()
            }
        }
        impl $name {
            /// Explicit adapter for a neighbour whose rows are not typed yet.
            pub fn record(&self) -> Record {
                let mut row = serde_json::Map::new();
                $(if let Some(v) = &self.$field.0 { row.insert(stringify!($field).into(), v.json()); })*
                row.extend(self.extra.iter().map(|(k, v)| (k.to_string(), v.json())));
                row.into()
            }
            pub fn json(&self) -> Value { Value::Object(self.record().into_map()) }
        }
    };
}

row!(TxRow, TxKey, {
    id, workspace_id, profile_id, wallet_id, external_id, external_id_kind,
    fingerprint, occurred_at, confirmed_at, direction, asset, amount, fee,
    amount_includes_fee, fiat_currency, fiat_rate, fiat_value, fiat_price_source,
    fiat_rate_exact, fiat_value_exact, pricing_source_kind, pricing_provider,
    pricing_pair, pricing_timestamp, pricing_fetched_at, pricing_granularity,
    pricing_method, pricing_external_ref, pricing_quality, commercial_applied_link_id,
    review_status, taxability_override, at_regime_override, at_category_override,
    kind_override, privacy_boundary, kind, description, counterparty, note, excluded,
    raw_json, payment_hash, payment_hash_source, swap_refund_funding_txid,
    swap_refund_funding_vout, created_at, wallet_label, wallet_kind, wallet_account_id,
    config_json, wallet_config_json, observation_authority_version,
    observation_observer_kinds_json, observation_graph_hash, observation_quantity_hash,
    observation_fee_attribution, observation_application_revision, observation_observed_at,
    account_code, account_label, config_text_mentions_samourai, journal_transaction_id,
    native_event_id, native_namespace, source_ref, backend_name, chain, network,
    bitcoin_network, chain_network, chain_instance_id,
});
row!(WalletRef, WalletKey, {
    id, label, kind, config_json, wallet_config_json, wallet_account_id,
    account_code, account_label, config_text_mentions_samourai,
}, legacy);
// R4-8's existing fusion reader takes &Record. Only this small wallet boundary
// uses a coercion, so the independent journals render work need not be changed.
impl std::ops::Deref for WalletRef {
    type Target = Record;
    fn deref(&self) -> &Record {
        self.legacy.get_or_init(|| self.record())
    }
}
row!(RateRow, RateKey, { asset, fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount });
row!(WorklistRow, WorklistKey, {
    id, profile_id, wallet_id, wallet_label, wallet_kind, occurred_at, direction,
    asset, amount, fee, amount_includes_fee, excluded, kind, kind_override,
    privacy_boundary, external_id, raw_json, wallet_config_json, config_json,
});

row!(NetworkRow, NetworkKey, {
    id, wallet_id, asset, raw_json, wallet_kind, config_json, wallet_config_json,
    chain, network, bitcoin_network, chain_network, chain_instance_id,
});
