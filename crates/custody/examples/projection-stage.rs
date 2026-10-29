//! Unpackaged whole-stage parity driver; raw v1 requests only.
use kassiber_custody::{codec, finalized_projection::runtime, snapshot::Snapshot};
use std::io::{self, BufRead};

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("input line");
        let result = (|| -> Result<kassiber_pyjson::Value, String> {
            let response = codec::loads(&kassiber_custody::custody(&line))?;
            let encoded = codec::to_json(&response);
            if let Some(code) = encoded["error"]["code"].as_str() {
                return Err(code.into());
            }
            let snapshot: Snapshot = serde_json::from_str(&line).map_err(|_| "ValueError")?;
            let stages = match &response {
                kassiber_pyjson::Value::Dict(entries) => entries
                    .iter()
                    .find(|(k, _)| *k == kassiber_pyjson::Key::Str("stages".into()))
                    .map(|(_, v)| v)
                    .ok_or("KeyError")?,
                _ => return Err("TypeError".into()),
            };
            let get = |name: &str| match stages {
                kassiber_pyjson::Value::Dict(entries) => entries
                    .iter()
                    .find(|(k, _)| *k == kassiber_pyjson::Key::Str(name.into()))
                    .map(|(_, v)| v)
                    .ok_or("KeyError".to_owned()),
                _ => Err("TypeError".into()),
            };
            runtime::stage(&snapshot, get("quantity")?, get("interpretation")?)
        })();
        let output = match result {
            Ok(value) => {
                kassiber_pyjson::dumps(&value, &kassiber_pyjson::Options::new().ensure_ascii(true))
                    .expect("output")
            }
            Err(code) => serde_json::json!({"error":code}).to_string(),
        };
        println!("{output}");
    }
}
