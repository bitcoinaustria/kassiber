use kassiber_pyjson::{BigInt, Key, PyStr, Value as Py};
use std::collections::{BTreeMap, BTreeSet};
use std::ops::{Index, IndexMut};

#[derive(Clone, Debug, PartialEq)]
pub(super) enum Value {
    Null,
    Bool(bool),
    Int(BigInt),
    Float(f64),
    String(PyStr),
    Array(Vec<Value>),
    Object(Object),
}

#[derive(Clone, Debug, Default)]
pub(super) struct Object(Vec<(PyStr, Value)>);

impl PartialEq for Object {
    fn eq(&self, other: &Self) -> bool {
        self.0.len() == other.0.len()
            && self.0.iter().all(|(key, value)| {
                other
                    .0
                    .iter()
                    .find(|(candidate, _)| candidate == key)
                    .is_some_and(|(_, candidate)| candidate == value)
            })
    }
}

impl Object {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.0.iter().find(|(k, _)| k.is(key)).map(|(_, v)| v)
    }
    pub fn insert(&mut self, key: PyStr, value: Value) {
        if let Some((_, old)) = self.0.iter_mut().find(|(k, _)| k == &key) {
            *old = value;
        } else {
            self.0.push((key, value));
        }
    }
    pub fn remove(&mut self, key: &str) {
        self.0.retain(|(k, _)| !k.is(key));
    }
    pub fn extend(&mut self, other: Self) {
        for (key, value) in other.0 {
            self.insert(key, value);
        }
    }
}

impl Value {
    pub fn from_py(value: &Py) -> Self {
        match value {
            Py::Null => Self::Null,
            Py::Bool(v) => Self::Bool(*v),
            Py::Int(v) => Self::Int(v.clone()),
            Py::Float(v) => Self::Float(*v),
            Py::Str(v) => Self::String(v.clone()),
            Py::List(v) => Self::Array(v.iter().map(Self::from_py).collect()),
            Py::Dict(v) => Self::Object(Object(
                v.iter()
                    .map(|(key, value)| {
                        let Key::Str(key) = key else {
                            unreachable!("JSON mapping key")
                        };
                        (key.clone(), Self::from_py(value))
                    })
                    .collect(),
            )),
            Py::Decimal(_) | Py::Bytes(_) => unreachable!("JSON or SQLite value"),
        }
    }
    pub fn py(&self) -> Py {
        match self {
            Self::Null => Py::Null,
            Self::Bool(v) => Py::Bool(*v),
            Self::Int(v) => Py::Int(v.clone()),
            Self::Float(v) => Py::Float(*v),
            Self::String(v) => Py::Str(v.clone()),
            Self::Array(v) => Py::List(v.iter().map(Self::py).collect()),
            Self::Object(v) => Py::Dict(
                v.0.iter()
                    .map(|(key, value)| (Key::Str(key.clone()), value.py()))
                    .collect(),
            ),
        }
    }
    pub fn serde(&self) -> serde_json::Value {
        match self {
            Self::Null => serde_json::Value::Null,
            Self::Bool(v) => (*v).into(),
            Self::Int(v) => serde_json::Value::Number(v.to_string().parse().unwrap()),
            Self::Float(v) if v.is_finite() => {
                serde_json::Value::Number(kassiber_pyjson::float_repr(*v).parse().unwrap())
            }
            Self::Float(_) => kassiber_envelope::py_str(&self.py()).into(),
            Self::String(v) => v
                .code_points()
                .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
                .collect::<String>()
                .into(),
            Self::Array(v) => serde_json::Value::Array(v.iter().map(Self::serde).collect()),
            Self::Object(v) => serde_json::Value::Object(
                v.0.iter()
                    .map(|(key, value)| {
                        let key = key
                            .code_points()
                            .map(|c| char::from_u32(c).unwrap_or('\u{fffd}'))
                            .collect();
                        (key, value.serde())
                    })
                    .collect(),
            ),
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Self::String(v) = self {
            v.as_str()
        } else {
            None
        }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_array_mut(&mut self) -> Option<&mut Vec<Value>> {
        if let Self::Array(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_object(&self) -> Option<&Object> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_object_mut(&mut self) -> Option<&mut Object> {
        if let Self::Object(v) = self {
            Some(v)
        } else {
            None
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        if let Self::Int(v) = self {
            u64::try_from(v).ok()
        } else {
            None
        }
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_object()?.get(key)
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn is_boolean(&self) -> bool {
        matches!(self, Self::Bool(_))
    }
    pub fn is_string(&self) -> bool {
        matches!(self, Self::String(_))
    }
    pub fn is_array(&self) -> bool {
        matches!(self, Self::Array(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Self::Object(_))
    }
}

impl Index<&str> for Value {
    type Output = Self;
    fn index(&self, key: &str) -> &Self {
        self.get(key).unwrap_or(&Value::Null)
    }
}
impl IndexMut<&str> for Value {
    fn index_mut(&mut self, key: &str) -> &mut Self {
        if self.is_null() {
            *self = Value::Object(Object::default());
        }
        let Self::Object(values) = self else {
            panic!("mapping required")
        };
        let position = values
            .0
            .iter()
            .position(|(k, _)| k.is(key))
            .unwrap_or_else(|| {
                values.0.push((key.into(), Value::Null));
                values.0.len() - 1
            });
        &mut values.0[position].1
    }
}
impl PartialEq<str> for Value {
    fn eq(&self, other: &str) -> bool {
        matches!(self, Self::String(v) if v.is(other))
    }
}
impl PartialEq<&str> for Value {
    fn eq(&self, other: &&str) -> bool {
        self == *other
    }
}
impl PartialEq<String> for Value {
    fn eq(&self, other: &String) -> bool {
        self == other.as_str()
    }
}
impl PartialEq<bool> for Value {
    fn eq(&self, other: &bool) -> bool {
        matches!(self, Self::Bool(v) if v == other)
    }
}
impl PartialEq<i32> for Value {
    fn eq(&self, other: &i32) -> bool {
        match self {
            Self::Int(v) => v == &BigInt::from(*other),
            Self::Float(v) => *v == f64::from(*other),
            _ => false,
        }
    }
}

pub(super) trait Capture {
    fn capture(&self) -> Value;
}
impl Capture for Value {
    fn capture(&self) -> Value {
        self.clone()
    }
}
impl Capture for str {
    fn capture(&self) -> Value {
        Value::String(self.into())
    }
}
impl Capture for String {
    fn capture(&self) -> Value {
        self.as_str().capture()
    }
}
impl Capture for PyStr {
    fn capture(&self) -> Value {
        Value::String(self.clone())
    }
}
impl<T: Capture + ?Sized> Capture for &T {
    fn capture(&self) -> Value {
        (**self).capture()
    }
}
impl Capture for bool {
    fn capture(&self) -> Value {
        Value::Bool(*self)
    }
}
macro_rules! numbers {
    ($($t:ty),*) => { $(impl Capture for $t {
        fn capture(&self) -> Value { Value::Int(BigInt::from(*self)) }
    })* };
}
numbers!(i32, i64, u32, u64, usize);
impl<T: Capture> Capture for Option<T> {
    fn capture(&self) -> Value {
        self.as_ref().map_or(Value::Null, Capture::capture)
    }
}
impl<T: Capture> Capture for Vec<T> {
    fn capture(&self) -> Value {
        Value::Array(self.iter().map(Capture::capture).collect())
    }
}
impl<T: Capture> Capture for BTreeSet<T> {
    fn capture(&self) -> Value {
        Value::Array(self.iter().map(Capture::capture).collect())
    }
}
impl<K: Clone + Into<PyStr>, V: Capture> Capture for BTreeMap<K, V> {
    fn capture(&self) -> Value {
        Value::Object(Object(
            self.iter()
                .map(|(k, v)| (k.clone().into(), v.capture()))
                .collect(),
        ))
    }
}
impl<A: Capture, B: Capture> Capture for (A, B) {
    fn capture(&self) -> Value {
        Value::Array(vec![self.0.capture(), self.1.capture()])
    }
}
impl Capture for serde_json::Value {
    fn capture(&self) -> Value {
        Value::from_py(&kassiber_pytext::loads(&self.to_string()).unwrap())
    }
}

macro_rules! json {
    (null) => { $crate::analysis_index::value::Value::Null };
    ([]) => { $crate::analysis_index::value::Value::Array(vec![]) };
    ([$($item:expr),* $(,)?]) => { $crate::analysis_index::value::Value::Array(vec![$($crate::analysis_index::value::Capture::capture(&$item)),*]) };
    ({$($rest:tt)*}) => {{
        #[allow(unused_mut)]
        let mut object = $crate::analysis_index::value::Object::default();
        $crate::analysis_index::value::json!(@object object; $($rest)*);
        $crate::analysis_index::value::Value::Object(object)
    }};
    (@object $object:ident;) => {};
    (@object $object:ident; $key:literal : null $(, $($rest:tt)*)?) => {
        $object.insert($key.into(), $crate::analysis_index::value::Value::Null);
        $crate::analysis_index::value::json!(@object $object; $($($rest)*)?);
    };
    (@object $object:ident; $key:literal : [] $(, $($rest:tt)*)?) => {
        $object.insert($key.into(), $crate::analysis_index::value::Value::Array(vec![]));
        $crate::analysis_index::value::json!(@object $object; $($($rest)*)?);
    };
    (@object $object:ident; $key:literal : {} $(, $($rest:tt)*)?) => {
        $object.insert($key.into(), $crate::analysis_index::value::json!({}));
        $crate::analysis_index::value::json!(@object $object; $($($rest)*)?);
    };
    (@object $object:ident; $key:literal : $item:expr $(, $($rest:tt)*)?) => {
        $object.insert($key.into(), $crate::analysis_index::value::Capture::capture(&$item));
        $crate::analysis_index::value::json!(@object $object; $($($rest)*)?);
    };
    ($item:expr) => { $crate::analysis_index::value::Capture::capture(&$item) };
}
pub(super) use json;

/// `evaluate_projection_freshness`'s calculation-semantics term: a profile
/// row without both markers is never current.
pub(super) fn journal_semantics_current(profile: &Value) -> bool {
    let marker = |key| match profile.get(key)? {
        Value::Null => Some(BigInt::from(0)),
        Value::Bool(value) => Some(BigInt::from(u8::from(*value))),
        Value::Int(value) => Some(value.clone()),
        _ => None,
    };
    let (Some(required), Some(built)) = (
        marker("required_journal_semantics_version"),
        marker("journal_semantics_version"),
    ) else {
        return false;
    };
    let version = BigInt::from(kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION);
    required == BigInt::from(0) || (built == version && built >= required)
}
