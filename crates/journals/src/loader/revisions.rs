//! Preserve the SQLite binding type and Python's numeric set/order semantics.
use super::*;
use rusqlite::types::{ToSql, ToSqlOutput, Value as SqlValue};
use std::cmp::Ordering;

#[derive(Debug)]
pub(super) struct Reference(SqlValue);

impl Reference {
    pub(super) fn from_json(value: &serde_json::Value) -> Result<Self> {
        if let Some(bytes) = kassiber_custody::codec::raw_bytes(value) {
            return Ok(Self(SqlValue::Blob(bytes)));
        }
        Ok(Self(match value {
            serde_json::Value::String(value) => SqlValue::Text(value.clone()),
            serde_json::Value::Number(value) => {
                if let Some(value) = value.as_i64() {
                    SqlValue::Integer(value)
                } else {
                    SqlValue::Real(value.to_string().parse().map_err(|_| {
                        Error::Unsupported("custody revision reference is not a SQLite scalar")
                    })?)
                }
            }
            _ => {
                return Err(Error::Unsupported(
                    "custody revision reference is not a SQLite scalar",
                ))
            }
        }))
    }

    pub(super) fn is_text(&self) -> bool {
        matches!(self.0, SqlValue::Text(_))
    }
    pub(super) fn is_bytes(&self) -> bool {
        matches!(self.0, SqlValue::Blob(_))
    }

    #[cfg(test)]
    pub(super) fn python_type(&self) -> &'static str {
        match self.0 {
            SqlValue::Text(_) => "str",
            SqlValue::Integer(_) => "int",
            SqlValue::Real(_) => "float",
            SqlValue::Blob(_) => "bytes",
            _ => unreachable!("revision reference scalar"),
        }
    }
}

// A float cast loses integer identity near 2^53. Compare against the float's
// integer part instead, after handling the endpoints where a cast saturates.
fn int_float(integer: i64, real: f64) -> Ordering {
    if real >= -(i64::MIN as f64) {
        return Ordering::Less;
    }
    if real < i64::MIN as f64 {
        return Ordering::Greater;
    }
    match integer.cmp(&(real as i64)) {
        Ordering::Equal => 0.0_f64.partial_cmp(&real.fract()).expect("SQLite REAL"),
        order => order,
    }
}

impl Ord for Reference {
    fn cmp(&self, other: &Self) -> Ordering {
        match (&self.0, &other.0) {
            (SqlValue::Text(a), SqlValue::Text(b)) => a.cmp(b),
            (SqlValue::Blob(a), SqlValue::Blob(b)) => a.cmp(b),
            (SqlValue::Integer(a), SqlValue::Integer(b)) => a.cmp(b),
            (SqlValue::Real(a), SqlValue::Real(b)) => a.partial_cmp(b).expect("SQLite REAL"),
            (SqlValue::Integer(a), SqlValue::Real(b)) => int_float(*a, *b),
            (SqlValue::Real(a), SqlValue::Integer(b)) => int_float(*b, *a).reverse(),
            // Mixed text/numeric populations remain guarded before any query.
            (SqlValue::Text(_), _) => Ordering::Greater,
            (_, SqlValue::Text(_)) => Ordering::Less,
            (SqlValue::Blob(_), _) => Ordering::Greater,
            (_, SqlValue::Blob(_)) => Ordering::Less,
            _ => unreachable!("revision reference scalar"),
        }
    }
}
impl PartialOrd for Reference {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Reference {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Reference {}
impl ToSql for Reference {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        self.0.to_sql()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn native_loader_revision_order_preserves_python_numeric_identity() {
        let refs = [
            json!(9007199254740993_i64),
            json!(9007199254740992.0),
            json!(-1.5),
            json!(-1),
            json!(0),
            json!(-0.0),
            json!(1.0),
            json!(1),
            json!(i64::MIN),
            json!(i64::MAX),
            scalar_json(f64::INFINITY),
            scalar_json(f64::NEG_INFINITY),
            json!(-(i64::MIN as f64)),
            json!((i64::MIN as f64) - 2048.0),
            json!(-0.5),
            json!(0.5),
        ];
        let mut sorted = BTreeSet::new();
        for value in &refs {
            sorted.insert(Reference::from_json(value).unwrap());
        }
        assert_eq!(sorted.len(), refs.len() - 2);
        let values: Vec<_> = sorted.into_iter().map(|id| id.0).collect();
        assert_eq!(
            values,
            vec![
                SqlValue::Real(f64::NEG_INFINITY),
                SqlValue::Real((i64::MIN as f64) - 2048.0),
                SqlValue::Integer(i64::MIN),
                SqlValue::Real(-1.5),
                SqlValue::Integer(-1),
                SqlValue::Real(-0.5),
                SqlValue::Integer(0),
                SqlValue::Real(0.5),
                SqlValue::Real(1.0),
                SqlValue::Real(9007199254740992.0),
                SqlValue::Integer(9007199254740993),
                SqlValue::Integer(i64::MAX),
                SqlValue::Real(-(i64::MIN as f64)),
                SqlValue::Real(f64::INFINITY),
            ]
        );
        let text = Reference::from_json(&json!("1")).unwrap();
        let numeric = Reference::from_json(&json!(1)).unwrap();
        assert!(text.is_text());
        assert!(!numeric.is_text());
        assert_eq!(text.python_type(), "str");
        assert_eq!(numeric.python_type(), "int");
        assert_eq!(
            Reference::from_json(&json!(1.5)).unwrap().python_type(),
            "float"
        );
        assert_ne!(text, numeric);
        assert!(numeric < text);
        assert!(text > numeric);
        assert!(
            Reference::from_json(&json!("a")).unwrap() < Reference::from_json(&json!("b")).unwrap()
        );
        assert!(Reference::from_json(&json!(null)).is_err());
    }

    fn scalar_json(value: f64) -> serde_json::Value {
        super::super::direct::scalar(ValueRef::Real(value)).unwrap()
    }
}

#[cfg(test)]
mod byte_tests {
    use super::*;
    #[test]
    fn native_loader_blob_revision_identity_is_distinct_and_lexicographic() {
        let bytes = |b: &[u8]| {
            Reference::from_json(&kassiber_custody::codec::to_json(&Value::Bytes(b.to_vec())))
                .unwrap()
        };
        let a = bytes(b"77");
        assert_eq!(a.python_type(), "bytes");
        assert!(a.is_bytes());
        assert!(!a.is_text());
        let b = bytes(b"78");
        assert!(a < b);
        let text = Reference::from_json(&serde_json::json!("77")).unwrap();
        let number = Reference::from_json(&serde_json::json!(77)).unwrap();
        assert_ne!(a, text);
        assert_ne!(a, number);
        assert!(a < text);
        assert!(text > a);
        assert!(a > number);
        assert!(number < a);
        assert_eq!(
            a.to_sql().unwrap(),
            ToSqlOutput::Borrowed(ValueRef::Blob(b"77"))
        );
    }
}
