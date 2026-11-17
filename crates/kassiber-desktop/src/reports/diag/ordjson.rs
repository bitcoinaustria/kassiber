//! JSON with the key order the web writes (`JSON.stringify` keeps insertion order; this app's
//! `serde_json` sorts object keys), for log exports, the support bundle and expanded rows.

use serde_json::Value;

pub enum J {
    Obj(Vec<(String, J)>),
    Arr(Vec<J>),
    V(Value),
}

impl From<Value> for J {
    fn from(v: Value) -> J {
        J::V(v)
    }
}

/// An ordered object from `(key, value)` pairs.
pub fn obj(pairs: Vec<(&str, J)>) -> J {
    J::Obj(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

pub fn v(x: impl Into<Value>) -> J {
    J::V(x.into())
}

fn esc(s: &str) -> String {
    Value::String(s.to_owned()).to_string()
}

/// `JSON.stringify(x)`.
pub fn compact(j: &J) -> String {
    match j {
        J::V(x) => x.to_string(),
        J::Arr(a) => format!("[{}]", a.iter().map(compact).collect::<Vec<_>>().join(",")),
        J::Obj(o) => format!(
            "{{{}}}",
            o.iter()
                .map(|(k, x)| format!("{}:{}", esc(k), compact(x)))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// `JSON.stringify(x, null, 2)`.
pub fn pretty(j: &J) -> String {
    fn go(j: &J, ind: usize, out: &mut String) {
        let pad = |n: usize| " ".repeat(n);
        match j {
            J::V(Value::Object(m)) if !m.is_empty() => go(
                &J::Obj(
                    m.iter()
                        .map(|(k, x)| (k.clone(), J::V(x.clone())))
                        .collect(),
                ),
                ind,
                out,
            ),
            J::V(Value::Array(a)) if !a.is_empty() => {
                go(&J::Arr(a.iter().cloned().map(J::V).collect()), ind, out)
            }
            J::V(x) => out.push_str(&x.to_string()),
            J::Arr(a) if a.is_empty() => out.push_str("[]"),
            J::Obj(o) if o.is_empty() => out.push_str("{}"),
            J::Arr(a) => {
                out.push_str("[\n");
                for (i, x) in a.iter().enumerate() {
                    out.push_str(&pad(ind + 2));
                    go(x, ind + 2, out);
                    out.push_str(if i + 1 < a.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(ind));
                out.push(']');
            }
            J::Obj(o) => {
                out.push_str("{\n");
                for (i, (k, x)) in o.iter().enumerate() {
                    out.push_str(&pad(ind + 2));
                    out.push_str(&esc(k));
                    out.push_str(": ");
                    go(x, ind + 2, out);
                    out.push_str(if i + 1 < o.len() { ",\n" } else { "\n" });
                }
                out.push_str(&pad(ind));
                out.push('}');
            }
        }
    }
    let mut s = String::new();
    go(j, 0, &mut s);
    s
}
