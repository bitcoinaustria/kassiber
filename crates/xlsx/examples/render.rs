//! Test-only JSONL adapter. Production callers use render(), never a subprocess.
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let result = (|| -> Result<(), Box<dyn std::error::Error>> {
            let request: serde_json::Value = serde_json::from_str(&line?)?;
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
