//! Test-only JSONL adapter. Production callers use render(), never a subprocess.
use std::io::{self, BufRead, Write};

// Workspace builds enable serde_json's arbitrary_precision feature. Serde's
// tagged-enum buffer otherwise treats noncanonical Python float notation as a
// private number map. The renderer schema uses f64, so canonicalize those JSON
// numbers before deserializing; production constructs Document directly.
fn normalize_numbers(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Number(number)
            if number.as_u64().is_none() && number.as_i64().is_none() =>
        {
            if let Some(number_f64) = number.as_f64().and_then(serde_json::Number::from_f64) {
                *number = number_f64;
            }
        }
        serde_json::Value::Array(values) => values.iter_mut().for_each(normalize_numbers),
        serde_json::Value::Object(fields) => fields.values_mut().for_each(normalize_numbers),
        _ => (),
    }
}

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let mut request: serde_json::Value = serde_json::from_str(&line?)?;
            normalize_numbers(&mut request);
            let document = serde_json::from_value(request["document"].clone())?;
            let bytes = kassiber_xlsx::render(&document)?;
            let path = request["path"].as_str().ok_or("missing output path")?;
            std::fs::write(path, bytes)?;
            Ok(())
        })();
        let response = match result {
            Ok(()) => serde_json::json!({"ok": true}),
            Err(error) => serde_json::json!({"error": error.to_string()}),
        };
        writeln!(stdout, "{response}").unwrap();
        stdout.flush().unwrap();
    }
}

#[test]
fn python_float_notation_survives_workspace_feature_unification() {
    let mut raw = serde_json::from_str(
        r#"{"op":"cell","row":0,"col":0,"style":0,"value":{"kind":"number","value":1.001e-05}}"#,
    )
    .unwrap();
    normalize_numbers(&mut raw);
    let operation: kassiber_xlsx::Operation = serde_json::from_value(raw).unwrap();
    assert!(matches!(
        operation,
        kassiber_xlsx::Operation::Cell {
            value: kassiber_xlsx::Cell::Number(value), ..
        } if value == 1.001e-5
    ));
}
