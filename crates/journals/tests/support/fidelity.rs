use kassiber_journals::{native::field, scalar};
use scalar::{Key, Value};

pub fn list(value: &Value) -> &[Value] {
    let Value::List(items) = value else {
        panic!("expected list")
    };
    items
}

fn replace(value: &mut Value, path: &[Value], replacement: &Value) {
    if path.is_empty() {
        *value = replacement.clone();
        return;
    }
    let child = match value {
        Value::Dict(items) => {
            let Value::Str(key) = &path[0] else {
                panic!("expected string key")
            };
            &mut items
                .iter_mut()
                .find(|(k, _)| matches!(k, Key::Str(k) if k == key))
                .expect("existing patch key")
                .1
        }
        Value::List(items) => {
            let index = scalar::text(&path[0]);
            &mut items[index.as_str().unwrap().parse::<usize>().unwrap()]
        }
        _ => panic!("expected patch container"),
    };
    replace(child, &path[1..], replacement);
}

pub fn cases(source: &str) -> Vec<Value> {
    let document = scalar::decode(source).unwrap();
    let bases = list(field(&document, "bases"));
    list(field(&document, "cases"))
        .iter()
        .map(|case| {
            let base = scalar::text(field(case, "base"));
            let mut inputs = bases[base.as_str().unwrap().parse::<usize>().unwrap()].clone();
            for patch in list(field(case, "patches")) {
                let patch = list(patch);
                replace(&mut inputs, list(&patch[0]), &patch[1]);
            }
            let Value::Dict(items) = &mut inputs else {
                panic!("expected inputs mapping")
            };
            for key in ["name", "entry_ids", "expected", "ledger"] {
                items.push((Key::from(key), field(case, key).clone()));
            }
            inputs
        })
        .collect()
}
