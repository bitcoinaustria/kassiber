//! Test-only JSONL driver for primitive parity; this is not snapshot admission.
use kassiber_custody::quantity::{
    allocation, balances, holds, native, project, reviewed, runtime, transfers,
};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
fn main() {
    let mut output = io::BufWriter::new(io::stdout().lock());
    for line in io::stdin().lock().lines() {
        let v: Value = serde_json::from_str(&line.unwrap()).unwrap();
        if v["op"] == "reviewed" {
            writeln!(
                output,
                "{}",
                reviewed::outcome(&v["component"], &v["observations"])
            )
            .unwrap();
            continue;
        }
        let result = match v["op"].as_str().unwrap_or("project") {
            "fifo" => {
                allocation::allocate_msat_fifo(&v["sources"], &v["sinks"], v.get("amount_msat"))
                    .map_err(|e| e.code.to_owned())
            }
            "balances" => balances::compare_wallet_balances(
                &v["projection"],
                v["current"].as_array().unwrap(),
                &v["known"],
            ),
            "transfers" => transfers::canonical_internal_transfer_rows(
                &v["projection"],
                v["eligible"].as_array().unwrap(),
                &v["refs"],
            ),
            "component_runtime" => runtime::component_claims(
                &v["canonical"],
                v["components"].as_array().unwrap(),
                &v["evidence"],
            ),
            "runtime" => runtime::build(&v),
            "gap_holds" => holds::compose(
                &v["canonical"],
                v["ignored"].as_array().unwrap(),
                &v["dismissed"],
                &v["search"],
            ),
            "tax_eligibility" => runtime::tax_eligibility(
                &v["canonical"],
                &v["projection"],
                v["issues"].as_array().unwrap(),
                &v["rows"],
            ),
            "native" => native::compile(
                &v["canonical"],
                v["audit"].as_array().unwrap(),
                v["excluded"].as_array().unwrap(),
                &v["reserved"],
            ),
            "project" => project(
                v["observations"].as_array().unwrap(),
                v["claims"].as_array().unwrap(),
            ),
            _ => panic!("unknown test operation"),
        };
        let result = match result {
            Ok(expected) => json!({"expected":expected}),
            Err(error) => json!({"error":error}),
        };
        writeln!(output, "{result}").unwrap();
    }
}
