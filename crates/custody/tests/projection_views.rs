//! The pipeline reads the finalized projection's rows through views
//! (`json_with_views`). Each view must read what the row's JSON object
//! holds, and the request builder must build the same request, or refuse
//! the same way, from views as from the JSON rows. The quantity stage's
//! observation and posting views read what their JSON objects hold, and
//! transfer rows built from them match those built from the JSON.
use kassiber_custody::{codec, ledger_request};
use serde_json::Value;

fn convert(value: &kassiber_pyjson::Value) -> Result<Value, String> {
    Ok(codec::to_json(value))
}

fn outcome(result: Result<ledger_request::Outcome, ledger_request::Refusal>) -> Value {
    match result {
        Ok(ledger_request::Outcome::Empty) => Value::from("empty"),
        Ok(ledger_request::Outcome::Request(request)) => request,
        Err(error) => error.document(),
    }
}

#[test]
fn projection_views_read_the_rows_and_build_the_same_request() {
    // Every fixture of v1 custody snapshots with a finalized projection.
    let documents = [
        include_str!("../../../tests/fixtures/custody_projection_stage.json"),
        include_str!("../../../tests/fixtures/custody_projection_adapter.json"),
        include_str!("../../../tests/fixtures/custody_pipeline_cases.json"),
        include_str!("../../../tests/fixtures/custody_component_cases.json"),
        include_str!("../../../tests/fixtures/custody_consolidation_cases.json"),
        include_str!("../../../tests/fixtures/custody_interpretation_cases.json"),
        include_str!("../../../tests/fixtures/custody_ownership_cases.json"),
        include_str!("../../../tests/fixtures/custody_pair_cases.json"),
        include_str!("../../../tests/fixtures/custody_reconciliation_cases.json"),
    ];
    let cases: Vec<Value> = documents
        .iter()
        .flat_map(|text| {
            let document: Value = serde_json::from_str(text).unwrap();
            match document {
                Value::Array(cases) => cases,
                mut document => document["cases"]
                    .take()
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
            }
        })
        .filter(|case| case["input"]["schema_version"] == 1)
        .collect();
    let (mut rows_read, mut requests, mut refusals) = (0, 0, 0);
    let (mut observations_read, mut postings_read) = (0, 0);
    for case in &cases {
        let name = &case["case"];
        // Component-level fixtures include inputs the custody stages refuse
        // or that only their own harness admits.
        let input = case["input"].to_string();
        let computed = std::panic::catch_unwind(|| {
            let snapshot = kassiber_custody::parse(&input)?;
            let (rows, stages) = kassiber_custody::run_with_rows(&snapshot, true)?;
            Ok::<_, kassiber_custody::Refusal>((snapshot, rows, stages))
        });
        let Ok(Ok((snapshot, rows, stages))) = computed else {
            continue;
        };
        let projection = stages.finalized_projection.as_ref().unwrap();
        let records = &rows.transactions;
        let json = projection.json_with(records, convert).unwrap();
        let (head, views) = projection.json_with_views(records, convert).unwrap();
        let mut expected = json.clone();
        let rows = expected.as_object_mut().unwrap().remove("rows").unwrap();
        assert_eq!(head, expected, "{name}");
        let rows = rows.as_array().unwrap();
        assert_eq!(views.len(), rows.len(), "{name}");
        for (view, row) in views.iter().zip(rows) {
            assert_eq!(&view.json(), row, "{name}");
            for (key, value) in row.as_object().unwrap() {
                assert_eq!(view.get(key), Some(value), "{name} {key}");
            }
            assert_eq!(view.get("no such field"), None, "{name}");
            rows_read += 1;
        }
        let python = |record: &kassiber_custody::snapshot::Record| {
            codec::to_json(&codec::from_json(&Value::Object(record.clone().into_map())).unwrap())
        };
        let profile = python(&snapshot.profile);
        let wallets = snapshot
            .wallets
            .iter()
            .map(|wallet| {
                let wallet = python(&wallet.record());
                (wallet["id"].clone(), wallet)
            })
            .collect::<Vec<_>>();
        let from_json = outcome(ledger_request::build_request(
            &profile, &json, &wallets, "ledger",
        ));
        let from_views = outcome(ledger_request::build_request_rows(
            &profile,
            &views,
            head["intra_pairs"].as_array().unwrap(),
            head["cross_asset_pairs"].as_array().unwrap(),
            &wallets,
            "ledger",
        ));
        assert_eq!(from_views, from_json, "{name}");
        // The quantity stage's observations and postings.
        let quantity = stages.quantity.json_with(false, convert).unwrap();
        let (quantity_head, quantity_views) = stages.quantity.json_with_views(convert).unwrap();
        let mut expected = quantity.clone();
        let parts = expected["projection"].as_object_mut().unwrap();
        let observations = parts.remove("observations").unwrap();
        let postings = parts.remove("postings").unwrap();
        assert_eq!(quantity_head, expected, "{name}");
        let observations = observations.as_array().unwrap();
        assert_eq!(quantity_views.observations.len(), observations.len());
        for (view, observation) in quantity_views.observations.iter().zip(observations) {
            for (key, value) in observation.as_object().unwrap() {
                assert_eq!(view.at(key), value, "{name} {key}");
            }
            assert!(view.at("no such field").is_null());
            observations_read += 1;
        }
        let postings = postings.as_array().unwrap();
        assert_eq!(quantity_views.postings.len(), postings.len());
        for (view, posting) in quantity_views.postings.iter().zip(postings) {
            for (key, value) in posting.as_object().unwrap() {
                // A posting read in place has no `$type` or `domain`.
                if matches!(view, kassiber_custody::quantity::PostingView::Read { .. })
                    && matches!(key.as_str(), "$type" | "domain")
                {
                    assert!(view.at(key).is_null());
                    continue;
                }
                assert_eq!(view.at(key), value, "{name} {key}");
            }
            assert!(view.at("no such field").is_null());
            postings_read += 1;
        }
        let refs = Value::Object(
            wallets
                .iter()
                .map(|(id, wallet)| (codec::pystr(id), wallet.clone()))
                .collect(),
        );
        let eligible = quantity["tax_eligibility"]["eligible_decisions"]
            .as_array()
            .unwrap();
        assert_eq!(
            kassiber_custody::quantity::transfers::transfer_rows(
                &quantity_views.observations,
                quantity_head["projection"]["decisions"].as_array().unwrap(),
                eligible,
                &refs,
            ),
            kassiber_custody::quantity::transfers::canonical_internal_transfer_rows(
                &quantity["projection"],
                eligible,
                &refs,
            ),
            "{name}"
        );
        if from_json.get("schema_version").is_some() {
            requests += 1;
        } else if from_json.get("family").is_some() {
            refusals += 1;
        }
    }
    assert!(
        rows_read > 100 && requests > 10 && refusals > 0,
        "{rows_read} rows, {requests} requests, {refusals} refusals"
    );
    assert!(
        observations_read > 100 && postings_read > 100,
        "{observations_read} observations, {postings_read} postings"
    );
}
