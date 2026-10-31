use std::io::BufRead;

fn sha(text: &str) -> String {
    kassiber_custody::codec::sha256(text)
}
fn quantity_text(stages: &kassiber_custody::Stages) -> String {
    kassiber_pyjson::dumps(&stages.quantity, &kassiber_pyjson::Options::new()).unwrap()
}
fn main() {
    for line in std::io::stdin().lock().lines() {
        let line = line.unwrap();
        let mut result = serde_json::json!({"input":sha(&line)});
        match kassiber_custody::parse(&line)
            .and_then(|snapshot| kassiber_custody::run(&snapshot, false))
        {
            Err(error) => result["decision_refusal"] = serde_json::json!(error.document()),
            Ok(stages) => {
                result["quantity"] = serde_json::json!(sha(&quantity_text(&stages)));
                let value = kassiber_custody::codec::to_json(&stages.quantity);
                let decisions = value["projection"]["decisions"]
                    .as_array()
                    .map_or(0, Vec::len);
                let issues = value["issues"].as_array().map_or(0, Vec::len);
                result["activity"] = serde_json::json!(decisions + issues);
                result["quantity_decisions"] = serde_json::json!(decisions);
                result["quantity_issues"] = serde_json::json!(issues);
                result["decisions"] =
                    serde_json::json!(sha(&kassiber_custody::response(&stages, false)));
            }
        }
        match kassiber_journals::compute(&line, true) {
            Err(error) => result["refusal"] = serde_json::json!(error.document()),
            Ok(computed) => {
                let quantity = sha(&quantity_text(&computed.stages));
                assert_eq!(result["quantity"], quantity);
                result["full_quantity"] = serde_json::json!(quantity);
                result["whole"] =
                    serde_json::json!(sha(&kassiber_custody::response(&computed.stages, false)));
                result["fusion"] = match computed.fusion {
                    kassiber_journals::Fusion::Empty => serde_json::json!(["empty"]),
                    kassiber_journals::Fusion::Skipped => serde_json::json!(["skipped"]),
                    kassiber_journals::Fusion::Refused(error) => {
                        serde_json::json!(["refused", error])
                    }
                    kassiber_journals::Fusion::Ledger { request, response } => {
                        serde_json::json!(["ledger", sha(&request), sha(&response)])
                    }
                };
            }
        }
        println!("{result}");
    }
}
