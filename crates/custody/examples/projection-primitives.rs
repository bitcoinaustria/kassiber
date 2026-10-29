//! Unpackaged comparison driver. Never accepts a custody snapshot as operands.
use std::io::{self, BufRead, Write};
fn main() {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let value = serde_json::from_str(&line.unwrap()).unwrap();
        writeln!(
            output,
            "{}",
            kassiber_custody::finalized_projection::primitive(&value)
        )
        .unwrap();
    }
}
