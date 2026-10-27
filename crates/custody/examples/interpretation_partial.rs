//! Test-only raw-fact comparison endpoint; never packaged or exposed by PyO3.
use std::io::{self, BufRead};

fn main() {
    for line in io::stdin().lock().lines() {
        let result = line.map_err(|e| e.to_string()).and_then(|line| {
            let snapshot: kassiber_custody::snapshot::Snapshot =
                serde_json::from_str(&line).map_err(|e| e.to_string())?;
            snapshot.validate()?;
            kassiber_custody::interpretation::partial(&snapshot)
        });
        println!(
            "{}",
            match result {
                Ok(value) => value,
                Err(message) => serde_json::json!({"partial_error":message}),
            }
        );
    }
}
