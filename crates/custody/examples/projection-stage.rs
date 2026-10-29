//! Unpackaged whole-stage parity driver; raw v1 requests only.
use kassiber_custody::codec;
use std::io::{self, BufRead};

fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("input line");
        let result = (|| -> Result<kassiber_pyjson::Value, serde_json::Value> {
            let response = codec::loads(&kassiber_custody::custody(&line))
                .map_err(|e| serde_json::json!({"error":e}))?;
            let encoded = codec::to_json(&response);
            if let Some(code) = encoded["error"]["code"].as_str() {
                return Err(serde_json::json!({"error":code,"stage":encoded["error"]["stage"]}));
            }
            let stages = match &response {
                kassiber_pyjson::Value::Dict(entries) => entries
                    .iter()
                    .find(|(k, _)| *k == kassiber_pyjson::Key::Str("stages".into()))
                    .map(|(_, v)| v)
                    .ok_or(serde_json::json!({"error":"KeyError"}))?,
                _ => return Err(serde_json::json!({"error":"TypeError"})),
            };
            let get = |name: &str| match stages {
                kassiber_pyjson::Value::Dict(entries) => entries
                    .iter()
                    .find(|(k, _)| *k == kassiber_pyjson::Key::Str(name.into()))
                    .map(|(_, v)| v)
                    .ok_or(serde_json::json!({"error":"KeyError"})),
                _ => Err(serde_json::json!({"error":"TypeError"})),
            };
            Ok(get("finalized_projection")?.clone())
        })();
        let output = match result {
            Ok(value) => {
                kassiber_pyjson::dumps(&value, &kassiber_pyjson::Options::new().ensure_ascii(true))
                    .expect("output")
            }
            Err(error) => error.to_string(),
        };
        println!("{output}");
    }
}
