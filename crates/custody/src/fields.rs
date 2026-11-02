//! Field access shared by JSON rows, snapshot records and typed row views.
//! An absent field is `None`; `at` reads it as JSON null, as indexing does.
use crate::snapshot::Record;
use serde_json::{Map, Value};

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

impl<T: Fields + ?Sized> Fields for &T {
    #[inline]
    fn get(&self, key: &str) -> Option<&Value> {
        (**self).get(key)
    }

    fn loaded_raw(&self) -> Option<&Result<kassiber_pyjson::Value, ()>> {
        (**self).loaded_raw()
    }
}
