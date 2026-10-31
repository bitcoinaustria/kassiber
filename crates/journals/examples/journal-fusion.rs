use kassiber_journals::{boundary, engine};
use serde_json::Value;
use std::io::{self, BufRead, Write};

fn compare(request: &str) -> String {
    let value: Value = serde_json::from_str(request).expect("fusion request");
    let actual = engine::ledger(&value);
    let text = kassiber_tax_core::ledger(request);
    let expected = boundary::ledger_response(&serde_json::from_str(&text).expect("text response"));
    match (actual, expected) {
        (Ok(actual), Ok(_)) => {
            let actual = actual.to_json().expect("typed response");
            assert_eq!(actual, text, "typed engine response bytes");
            actual
        }
        (Err(actual), Err(expected)) => {
            assert_eq!(
                serde_json::to_value(actual).unwrap(),
                serde_json::to_value(expected).unwrap(),
                "typed engine refusal attributes"
            );
            text
        }
        _ => panic!("typed engine outcome differs"),
    }
}

fn main() {
    for line in io::stdin().lock().lines() {
        let input: Value = serde_json::from_str(&line.expect("input line")).expect("probe request");
        println!(
            "{}",
            kassiber_custody::custody_with_fusion(
                input["request"].as_str().expect("snapshot text"),
                input["finalized"].as_bool().expect("finalization control"),
                compare,
            )
        );
        io::stdout().flush().expect("probe output");
        #[cfg(feature = "timing")]
        {
            kassiber_custody::timing::take();
            kassiber_journals::timing::take();
        }
    }
}
