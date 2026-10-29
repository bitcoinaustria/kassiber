//! Python str()/repr() for ordered raw configuration values and dataclass keys.
use crate::codec;
use kassiber_pyjson::{Key, Value as Py};
use std::fmt::Write;

include!("printable.rs");

fn printable(point: u32) -> bool {
    let at = NONPRINTABLE.partition_point(|&(_, end)| end < point);
    !NONPRINTABLE
        .get(at)
        .is_some_and(|&(start, _)| start <= point)
}

pub(super) fn quoted(points: impl Iterator<Item = u32>) -> String {
    let points: Vec<_> = points.collect();
    let quote = if points.contains(&39) && !points.contains(&34) {
        '"'
    } else {
        '\''
    };
    let mut result = String::from(quote);
    for point in points {
        match point {
            9 => result.push_str("\\t"),
            10 => result.push_str("\\n"),
            13 => result.push_str("\\r"),
            92 => result.push_str("\\\\"),
            p if p == u32::from(quote) => {
                result.push('\\');
                result.push(quote);
            }
            p if printable(p) => result.push(char::from_u32(p).expect("printable scalar")),
            p if p <= 0xff => write!(result, "\\x{p:02x}").unwrap(),
            p if p <= 0xffff => write!(result, "\\u{p:04x}").unwrap(),
            p => write!(result, "\\U{p:08x}").unwrap(),
        }
    }
    result.push(quote);
    result
}

pub(crate) fn repr(value: &Py) -> String {
    match value {
        Py::Str(value) => quoted(value.code_points()),
        Py::List(values) => format!(
            "[{}]",
            values.iter().map(repr).collect::<Vec<_>>().join(", ")
        ),
        Py::Dict(values) => format!(
            "{{{}}}",
            values
                .iter()
                .map(|(key, value)| {
                    let Key::Str(key) = key else {
                        unreachable!("JSON configuration key")
                    };
                    format!("{}: {}", quoted(key.code_points()), repr(value))
                })
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Py::Float(value) => kassiber_pyjson::float_repr(*value),
        _ => codec::pystr(&codec::to_json(value)),
    }
}

pub(super) fn string(value: &Py) -> String {
    if let Py::Str(value) = value {
        value.as_str().expect("JSON scalar").into()
    } else {
        repr(value)
    }
}

pub(super) fn text_or_empty(value: &Py) -> String {
    // A valid JSON exponent can overflow to Python's truthy inf. Converting
    // through serde first would replace that float with null and lose it.
    let truth = match value {
        Py::Float(value) => *value != 0.0,
        _ => codec::truth(&codec::to_json(value)),
    };
    if !truth {
        return String::new();
    }
    if let Py::Str(value) = value {
        return value.as_str().expect("validated config scalar").into();
    }
    repr(value)
}

pub(super) fn field<'a>(value: &'a Py, name: &str) -> &'a Py {
    if let Py::Dict(values) = value {
        for (key, value) in values {
            if matches!(key, Key::Str(key) if key.is(name)) {
                return value;
            }
        }
    }
    &Py::Null
}

pub(super) fn event_repr(event: &serde_json::Value) -> String {
    let fields = ["chain", "network", "native_namespace", "native_event_id"];
    format!(
        "CanonicalEventKey({})",
        fields
            .iter()
            .map(|name| {
                format!(
                    "{name}={}",
                    quoted(codec::text(&event[name]).chars().map(u32::from))
                )
            })
            .collect::<Vec<_>>()
            .join(", ")
    )
}
