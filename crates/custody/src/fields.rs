//! Field access shared by JSON rows, snapshot records and typed row views.
//! An absent field is `None`; `at` reads it as JSON null, as indexing does.
use crate::snapshot::Record;
use serde_json::{Map, Value};
use std::borrow::Cow;

pub(crate) static NULL: Value = Value::Null;

pub(crate) trait Fields {
    fn get(&self, key: &str) -> Option<&Value>;

    #[inline]
    fn at(&self, key: &str) -> &Value {
        self.get(key).unwrap_or(&NULL)
    }

    /// `codec::loads` of this row's string `raw_json`, when the row caches it.
    fn loaded_raw(&self) -> Option<&Result<kassiber_pyjson::Value, ()>> {
        None
    }
}

impl Fields for Value {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        Value::get(self, key)
    }
}

impl Fields for Map<String, Value> {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        Map::get(self, key)
    }
}

impl Fields for Record {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        Record::get(self, key)
    }
}

/// A transaction row: a JSON object, or a snapshot record holding the same
/// fields in the same order.
pub(crate) trait Row: Fields + Sync {
    /// `graph::onchain_scope` of the row.
    fn onchain_scope(&self) -> crate::graph::Scope;

    /// The row as a JSON object.
    fn json(&self) -> Cow<'_, Value>;
}

impl Row for Value {
    fn onchain_scope(&self) -> crate::graph::Scope {
        crate::graph::onchain_scope(self)
    }

    fn json(&self) -> Cow<'_, Value> {
        Cow::Borrowed(self)
    }
}

impl Row for Record {
    fn onchain_scope(&self) -> crate::graph::Scope {
        crate::graph::record_scope(self)
    }

    fn json(&self) -> Cow<'_, Value> {
        Cow::Owned(Value::Object(self.clone().into_map()))
    }
}

/// A row a pass kept from its input, or a JSON object that replaces it.
#[derive(Debug)]
pub(crate) enum Layered<'a, R> {
    Row(&'a R),
    Value(&'a Value),
}

impl<R> Clone for Layered<'_, R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R> Copy for Layered<'_, R> {}

impl<R: Fields> Fields for Layered<'_, R> {
    fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Self::Row(row) => row.get(key),
            Self::Value(row) => Fields::get(*row, key),
        }
    }

    fn loaded_raw(&self) -> Option<&Result<kassiber_pyjson::Value, ()>> {
        match self {
            Self::Row(row) => row.loaded_raw(),
            Self::Value(_) => None,
        }
    }
}

impl<R: Row> Row for Layered<'_, R> {
    fn onchain_scope(&self) -> crate::graph::Scope {
        match self {
            Self::Row(row) => row.onchain_scope(),
            Self::Value(row) => crate::graph::onchain_scope(row),
        }
    }

    fn json(&self) -> Cow<'_, Value> {
        match self {
            Self::Row(row) => row.json(),
            Self::Value(row) => Cow::Borrowed(row),
        }
    }
}

impl<T: Row + ?Sized> Row for &T {
    fn onchain_scope(&self) -> crate::graph::Scope {
        (**self).onchain_scope()
    }

    fn json(&self) -> Cow<'_, Value> {
        (**self).json()
    }
}

impl<T: Fields + ?Sized> Fields for &T {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        (**self).get(key)
    }

    fn loaded_raw(&self) -> Option<&Result<kassiber_pyjson::Value, ()>> {
        (**self).loaded_raw()
    }
}
