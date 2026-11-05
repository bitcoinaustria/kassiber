use crate::Refusal;
use kassiber_pyjson::Options;
pub use kassiber_pyjson::{BigInt, Key, PyStr, Value};

pub fn decode(text: &str) -> Result<Value, Refusal> {
    kassiber_pytext::loads(text).map_err(|error| {
        let family = match error.kind {
            kassiber_pytext::JsonErrorKind::Decode { .. } => "JSONDecodeError",
            kassiber_pytext::JsonErrorKind::IntTooLarge { .. }
            | kassiber_pytext::JsonErrorKind::Constant { .. } => "ValueError",
            kassiber_pytext::JsonErrorKind::TooDeep { .. } => "RecursionError",
        };
        Refusal::new(family, error.to_string())
    })
}

pub fn encode(value: &Value) -> Result<String, kassiber_pyjson::Error> {
    kassiber_pyjson::dumps(value, &Options::new().sort_keys(true).compact())
}

pub fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Int(_) => "int",
        Value::Float(_) => "float",
        Value::Str(_) => "str",
        Value::Bytes(_) => "bytes",
        Value::List(_) => "list",
        Value::Dict(_) => "dict",
        Value::Decimal(_) => "Decimal",
    }
}

pub fn truth(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(v) => *v,
        Value::Int(v) => *v != BigInt::from(0),
        Value::Float(v) => *v != 0.0,
        Value::Str(v) => v.code_points().next().is_some(),
        Value::Bytes(v) => !v.is_empty(),
        Value::List(v) => !v.is_empty(),
        Value::Dict(v) => !v.is_empty(),
        Value::Decimal(v) => !v.is_zero(),
    }
}

pub fn repr(value: &Value) -> String {
    match value {
        Value::Str(v) => kassiber_pytext::repr_py(v),
        Value::Bytes(v) => kassiber_pyjson::bytes_repr(v),
        Value::List(v) => format!("[{}]", v.iter().map(repr).collect::<Vec<_>>().join(", ")),
        Value::Dict(v) => format!(
            "{{{}}}",
            v.iter()
                .map(|(key, value)| format!("{}: {}", repr(&key_value(key)), repr(value)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Null => "None".into(),
        Value::Bool(true) => "True".into(),
        Value::Bool(false) => "False".into(),
        Value::Int(v) => v.to_string(),
        Value::Float(v) => kassiber_pyjson::float_repr(*v),
        Value::Decimal(v) => format!("Decimal('{}')", v),
    }
}

pub fn text(value: &Value) -> PyStr {
    match value {
        Value::Str(v) => v.clone(),
        Value::Decimal(v) => v.to_string().into(),
        _ => repr(value).into(),
    }
}

pub fn key_value(key: &Key) -> Value {
    match key {
        Key::Null => Value::Null,
        Key::Bool(v) => Value::Bool(*v),
        Key::Int(v) => Value::Int(v.clone()),
        Key::Float(v) => Value::Float(*v),
        Key::Str(v) => Value::Str(v.clone()),
    }
}

pub fn to_json(value: &Value) -> Option<serde_json::Value> {
    use serde_json::Value as Json;
    Some(match value {
        Value::Null => Json::Null,
        Value::Bool(v) => Json::Bool(*v),
        Value::Int(v) => serde_json::from_str(&v.to_string()).ok()?,
        Value::Float(v) => Json::Number(serde_json::Number::from_f64(*v)?),
        Value::Str(v) => Json::String(v.as_str()?.into()),
        Value::List(v) => Json::Array(v.iter().map(to_json).collect::<Option<_>>()?),
        Value::Dict(v) => {
            let mut map = serde_json::Map::new();
            let mut previous: Option<&str> = None;
            for (key, value) in v {
                let Key::Str(key) = key else { return None };
                let key = key.as_str()?;
                if previous.is_some_and(|previous| previous >= key) {
                    return None;
                }
                previous = Some(key);
                if map.insert(key.into(), to_json(value)?).is_some() {
                    return None;
                }
            }
            Json::Object(map)
        }
        Value::Decimal(_) | Value::Bytes(_) => return None,
    })
}

pub fn snapshot(value: &Value) -> Value {
    match value {
        Value::Float(v) if !v.is_finite() => {
            dict([("$float", Value::from(kassiber_pyjson::float_repr(*v)))])
        }
        Value::Decimal(v) => dict([("$decimal", Value::from(v.to_string()))]),
        Value::Bytes(v) => dict([(
            "$bytes",
            Value::from(
                v.iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>(),
            ),
        )]),
        Value::List(v) => Value::List(v.iter().map(snapshot).collect()),
        Value::Dict(v) => Value::Dict(v.iter().map(|(k, v)| (k.clone(), snapshot(v))).collect()),
        _ => value.clone(),
    }
}

pub fn dict<const N: usize>(fields: [(&str, Value); N]) -> Value {
    Value::Dict(fields.into_iter().map(|(k, v)| (Key::from(k), v)).collect())
}
