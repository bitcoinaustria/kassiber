//! Lossless v1 loader model. Untyped neighbours retain their original wire
//! fragments until their own R4 slice, and have no mutable access to those facts.
use super::*;
use serde::de::{MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserializer, Serializer};
use std::{fmt, ops::Deref};

#[derive(Clone, Debug)]
pub struct Unported<T> {
    value: T,
    wire: Box<serde_json::value::RawValue>,
}
impl<T> Unported<Option<T>> {
    fn null() -> Self {
        Self {
            value: None,
            wire: serde_json::value::RawValue::from_string("null".into()).expect("null"),
        }
    }
}
impl<T> Unported<Vec<T>> {
    fn empty() -> Self {
        Self {
            value: Vec::new(),
            wire: serde_json::value::RawValue::from_string("[]".into()).expect("list"),
        }
    }
}
impl<T> Deref for Unported<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}
impl<'de, T: serde::de::DeserializeOwned> Deserialize<'de> for Unported<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let wire = Box::<serde_json::value::RawValue>::deserialize(d)?;
        let value = serde_json::from_str(wire.get()).map_err(serde::de::Error::custom)?;
        Ok(Self { value, wire })
    }
}
impl<T> Serialize for Unported<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        self.wire.serialize(s)
    }
}

// Preserve v1 object order and wrapper admission without a Value tree of the
// typed rows. All fields remain required, as in the original v1 deserializer.
macro_rules! book {
    ($($field:ident: $ty:ty),* $(,)?) => {
        #[derive(Clone, Debug)]
        pub struct BookSnapshot {
            $(pub $field: $ty,)*
            order: Vec<Field>,
        }
        #[allow(non_camel_case_types)]
        #[derive(Clone, Debug, Deserialize)]
        #[serde(field_identifier)]
        enum Field { $($field,)* }
        impl<'de> Deserialize<'de> for BookSnapshot {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                struct Book;
                impl<'de> Visitor<'de> for Book {
                    type Value = BookSnapshot;
                    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { f.write_str("a custody snapshot") }
                    fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                        let _identifiers = super::identifiers::Scope::enter();
                        $(let mut $field = None;)*
                        let mut order = Vec::new();
                        while let Some(key) = a.next_key()? {
                            match key { $(Field::$field => {
                                if $field.is_some() { return Err(serde::de::Error::duplicate_field(stringify!($field))); }
                                let _span = crate::timing::span(concat!("parse.", stringify!($field)));
                                $field = Some(a.next_value()?);
                            },)* }
                            order.push(key);
                        }
                        Ok(BookSnapshot {
                            $($field: book!(@required $field, $field),)*
                            order,
                        })
                    }
                }
                d.deserialize_map(Book)
            }
        }
        impl Serialize for BookSnapshot {
            fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
                let mut m = s.serialize_map(Some(self.order.len()))?;
                for key in &self.order { match key { $(Field::$field => m.serialize_entry(stringify!($field), &self.$field)?,)* } }
                m.end()
            }
        }
    };
    (@required book_network, $value:ident) => { $value.unwrap_or_else(Unported::null) };
    (@required owned_index, $value:ident) => { $value.unwrap_or_else(Unported::null) };
    // Absent in snapshots older than the import duplicate hold; stays absent.
    (@required import_duplicate_quarantines, $value:ident) => { $value.unwrap_or_else(Unported::empty) };
    // Present only while the book keeps "Reset book data" records.
    (@required reset_hold_quarantines, $value:ident) => { $value.unwrap_or_else(Unported::empty) };
    (@required $field:ident, $value:ident) => { $value.ok_or_else(|| serde::de::Error::missing_field(stringify!($field)))? };
}

#[derive(Clone, Debug)]
pub struct TypedWorklist {
    pub large_book: bool,
    pub rows: Vec<WorklistRow>,
    rows_first: bool,
}
impl<'de> Deserialize<'de> for TypedWorklist {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(field_identifier, rename_all = "snake_case")]
        enum Key {
            LargeBook,
            Rows,
        }
        struct Worklist;
        impl<'de> Visitor<'de> for Worklist {
            type Value = TypedWorklist;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a gap worklist")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut a: A) -> Result<Self::Value, A::Error> {
                let (mut large_book, mut rows) = (None, None);
                let mut rows_first = false;
                while let Some(key) = a.next_key()? {
                    match key {
                        Key::LargeBook => {
                            if large_book.is_some() {
                                return Err(serde::de::Error::duplicate_field("large_book"));
                            }
                            large_book = Some(a.next_value()?);
                        }
                        Key::Rows => {
                            if rows.is_some() {
                                return Err(serde::de::Error::duplicate_field("rows"));
                            }
                            rows_first = large_book.is_none();
                            rows = Some(a.next_value()?);
                        }
                    }
                }
                Ok(TypedWorklist {
                    large_book: large_book
                        .ok_or_else(|| serde::de::Error::missing_field("large_book"))?,
                    rows: rows.ok_or_else(|| serde::de::Error::missing_field("rows"))?,
                    rows_first,
                })
            }
        }
        d.deserialize_map(Worklist)
    }
}
impl Serialize for TypedWorklist {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let mut m = s.serialize_map(Some(2))?;
        if self.rows_first {
            m.serialize_entry("rows", &self.rows)?;
        }
        m.serialize_entry("large_book", &self.large_book)?;
        if !self.rows_first {
            m.serialize_entry("rows", &self.rows)?;
        }
        m.end()
    }
}

book! {
    schema_version: u32,
    operation: String,
    as_of: String,
    profile: Unported<Record>,
    book_network: Unported<Option<NetworkObject<NetworkFacts>>>,
    wallets: Vec<WalletRef>,
    transactions: Vec<TxRow>,
    rate_rows: Vec<RateRow>,
    owned_index: Unported<Option<Value>>,
    ownership_warnings: Unported<Vec<String>>,
    components: Unported<Vec<Component>>,
    anchors: Unported<Map<String, Value>>,
    anchor_wallets: Unported<Vec<Record>>,
    revision_headers: Unported<Vec<Record>>,
    receipt_sources: Unported<Vec<Record>>,
    channel_records: Unported<Vec<Record>>,
    loan_legs: Unported<Vec<Record>>,
    evidence_snapshots: Unported<Vec<Record>>,
    migration_issues: Unported<Vec<Record>>,
    import_duplicate_quarantines: Unported<Vec<Record>>,
    reset_hold_quarantines: Unported<Vec<Record>>,
    gap_reviews: Unported<Vec<Record>>,
    pair_dismissals: Unported<Vec<Record>>,
    gap_worklist: TypedWorklist,
}

impl BookSnapshot {
    /// CPython's v1 request text. This reconstructs the typed columns, their
    /// presence and order; only not-yet-ported fields use their retained wire.
    pub fn text(&self) -> Result<String, String> {
        let wire = serde_json::to_string(self).map_err(|e| e.to_string())?;
        let value = crate::codec::loads(&wire)?;
        kassiber_pyjson::dumps(&value, &kassiber_pyjson::Options::new().ensure_ascii(true))
            .map_err(|e| e.to_string())
    }
    /// The only full-row conversion: after typed admission, at the boundary to
    /// the unported stages. There is no cached second transaction row view.
    pub fn stages(&self) -> StageSnapshot {
        let mut snapshot = StageSnapshot {
            schema_version: self.schema_version,
            operation: self.operation.clone(),
            as_of: self.as_of.clone(),
            profile: (*self.profile).clone(),
            // Network inventory has been admitted already. No downstream stage
            // reads it; the lossless facts remain on BookSnapshot for the oracle.
            book_network: None,
            wallets: self.wallets.iter().map(WalletRef::record).collect(),
            transactions: self
                .transactions
                .iter()
                .map(|row| row.json().as_object().cloned().unwrap().into())
                .collect(),
            rate_rows: self.rate_rows.iter().map(RateRow::record).collect(),
            owned_index: (*self.owned_index).clone(),
            ownership_warnings: (*self.ownership_warnings).clone(),
            components: (*self.components).clone(),
            anchors: (*self.anchors).clone(),
            anchor_wallets: (*self.anchor_wallets).clone(),
            revision_headers: (*self.revision_headers).clone(),
            receipt_sources: (*self.receipt_sources).clone(),
            channel_records: (*self.channel_records).clone(),
            loan_legs: (*self.loan_legs).clone(),
            evidence_snapshots: (*self.evidence_snapshots).clone(),
            migration_issues: (*self.migration_issues).clone(),
            import_duplicate_quarantines: self
                .order
                .iter()
                .any(|f| matches!(f, Field::import_duplicate_quarantines))
                .then(|| (*self.import_duplicate_quarantines).clone()),
            reset_hold_quarantines: self
                .order
                .iter()
                .any(|f| matches!(f, Field::reset_hold_quarantines))
                .then(|| (*self.reset_hold_quarantines).clone()),
            gap_reviews: (*self.gap_reviews).clone(),
            pair_dismissals: (*self.pair_dismissals).clone(),
            rows: Default::default(),
            gap_worklist: GapWorklist {
                large_book: self.gap_worklist.large_book,
                rows: self
                    .gap_worklist
                    .rows
                    .iter()
                    .map(WorklistRow::record)
                    .collect(),
            },
        };
        snapshot.escape_wire_tags();
        snapshot
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 || self.operation != "custody" {
            return Err("unsupported custody snapshot schema or operation".into());
        }
        if self.as_of.is_empty() {
            return Err("custody snapshot requires as_of".into());
        }
        fn config(value: &SqlValue) -> Result<(), String> {
            if value.is_null() {
                return Ok(());
            }
            let v = if let Some(s) = value.as_str() {
                serde_json::from_str(s)
                    .map_err(|_| "custody configuration must be normalized facts")?
            } else {
                value.json()
            };
            let object = v
                .as_object()
                .ok_or("custody configuration must be normalized facts")?;
            if object.keys().any(|k| {
                ![
                    "chain",
                    "network",
                    "bitcoin_network",
                    "chain_network",
                    "chain_instance_id",
                    "samourai",
                ]
                .contains(&k.as_str())
            }) {
                return Err(
                    "custody configuration contains fields outside normalized facts".into(),
                );
            }
            if v.get("samourai")
                .and_then(Value::as_object)
                .is_some_and(|b| {
                    b.keys()
                        .any(|k| !["role", "group_id", "section"].contains(&k.as_str()))
                })
            {
                return Err(
                    "custody Samourai metadata contains fields outside normalized facts".into(),
                );
            }
            Ok(())
        }
        for (a, b) in self
            .wallets
            .iter()
            .map(|r| (&r.config_json, &r.wallet_config_json))
            .chain(
                self.transactions
                    .iter()
                    .map(|r| (&r.config_json, &r.wallet_config_json)),
            )
        {
            config(a.value())?;
            config(b.value())?;
        }
        // Unported row collections retain their existing configuration gate.
        for row in self
            .anchor_wallets
            .iter()
            .chain(self.channel_records.iter())
        {
            for key in ["config_json", "wallet_config_json"] {
                if let Some(v) = row.get(key) {
                    config(
                        &serde_json::from_value::<SqlValue>(v.clone())
                            .map_err(|e| e.to_string())?,
                    )?;
                }
            }
        }
        for row in &self.gap_worklist.rows {
            config(row.config_json.value())?;
            config(row.wallet_config_json.value())?;
        }
        for row in self.anchors.values().filter_map(Value::as_object) {
            for key in ["config_json", "wallet_config_json"] {
                if let Some(v) = row.get(key) {
                    config(
                        &serde_json::from_value::<SqlValue>(v.clone())
                            .map_err(|e| e.to_string())?,
                    )?;
                }
            }
        }
        for row in &self.transactions {
            for (name, value) in [("amount", &row.amount), ("fee", &row.fee)] {
                if !matches!(value.value(), SqlValue::Null | SqlValue::Int(_)) {
                    return Err(format!("{name} must be an exact signed-64 msat integer"));
                }
            }
        }
        Ok(())
    }
}
