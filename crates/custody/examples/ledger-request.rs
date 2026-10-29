//! Unpackaged builder/fusion probe. Raw mode computes its own finalized projection.
use kassiber_custody::{
    codec,
    ledger_request::{build_request, request_json, Outcome},
};
use std::io::{self, BufRead};
fn main() {
    for line in io::stdin().lock().lines() {
        let line = line.expect("input line");
        let input: serde_json::Value = serde_json::from_str(&line).expect("builder operands");
        let wallets = input["wallets"]
            .as_array()
            .expect("ordered wallets")
            .iter()
            .map(|pair| (pair[0].clone(), pair[1].clone()))
            .collect::<Vec<_>>();
        let computed;
        let (profile, projection) = if input.get("snapshot").is_some() {
            let response = kassiber_custody::custody(&input["snapshot"].to_string());
            computed = codec::to_json(&codec::loads(&response).expect("custody output"));
            if !computed["error"].is_null() {
                println!(
                    "{}",
                    serde_json::json!({"outcome":"upstream_refusal", "error":computed["error"]})
                );
                continue;
            }
            (
                &input["snapshot"]["profile"],
                &computed["stages"]["finalized_projection"],
            )
        } else {
            (&input["profile"], &input["projection"])
        };
        let result = match build_request(
            profile,
            projection,
            &wallets,
            input["operation"].as_str().unwrap_or("ledger"),
        ) {
            Ok(Outcome::Empty) => serde_json::json!({"outcome":"empty"}),
            Ok(Outcome::Request(request)) => {
                if input["fusion"] == true {
                    let request_text = request_json(&request);
                    let response: serde_json::Value =
                        serde_json::from_str(&kassiber_tax_core::ledger(&request_text))
                            .expect("ledger response");
                    serde_json::json!({"outcome":"request","request":request,"response":response,"request_text":request_text})
                } else {
                    serde_json::json!({"outcome":"request","request":request})
                }
            }
            Err(error) => serde_json::json!({"outcome":"refusal","error":error.document()}),
        };
        println!(
            "{}",
            codec::dumps(&codec::from_json(&result).expect("output"), true).expect("output")
        );
    }
}
