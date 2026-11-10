//! The 2,000 generated component snapshots of `test_custody_component_port`,
//! frozen with the Python stage output where Rust agreed (plan 22): every
//! snapshot, and its permutation of the unordered reads, must yield exactly
//! the frozen components and blockers, and the generator's branches stay
//! covered.
use std::collections::BTreeMap;
use std::io::Read;

use kassiber_custody::custody_with_finalized;
use serde_json::Value;

fn vectors() -> Vec<Value> {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/fixtures/custody_component_generated.json.gz"
    );
    let file = std::fs::File::open(path)
        .unwrap_or_else(|e| panic!("{path}: {e} (run scripts/fetch-test-data.sh)"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(file)
        .read_to_string(&mut text)
        .unwrap();
    serde_json::from_str(&text).unwrap()
}

fn reverse(value: &mut Value) {
    if let Some(items) = value.as_array_mut() {
        items.reverse();
    }
}

/// `permuted_snapshot`: reverse the unordered SQL reads while keeping
/// ordered rows and rate ties, then reverse the transactions, which the
/// component stage sorts itself.
fn permuted(snapshot: &Value) -> Value {
    let mut result = snapshot.clone();
    for field in [
        "wallets",
        "loan_legs",
        "channel_records",
        "pair_dismissals",
        "receipt_sources",
        "components",
        "anchor_wallets",
        "revision_headers",
    ] {
        reverse(&mut result[field]);
    }
    if let Some(anchors) = result["anchors"].as_object() {
        let reversed: serde_json::Map<String, Value> = anchors
            .iter()
            .rev()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        result["anchors"] = Value::Object(reversed);
    }
    if let Some(index) = result.get_mut("owned_index").filter(|i| !i.is_null()) {
        for field in ["by_script", "by_outpoint", "txid_wallets"] {
            if let Some(map) = index[field].as_object_mut() {
                map.values_mut().for_each(reverse);
            }
        }
        for field in ["by_scoped_outpoint", "txid_wallets_by_scope"] {
            if let Some(rows) = index[field].as_array_mut() {
                rows.iter_mut().for_each(|row| reverse(&mut row["matches"]));
            }
        }
    }
    reverse(&mut result["transactions"]);
    result
}

fn components(input: &Value) -> Value {
    let response: Value =
        serde_json::from_str(&custody_with_finalized(&input.to_string(), false)).unwrap();
    response["stages"]["components"].clone()
}

#[test]
fn generated_component_snapshots_and_permutations_match_the_frozen_stage() {
    let vectors = vectors();
    assert_eq!(vectors.len(), 2000);
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut count = |key: String| *counts.entry(key).or_default() += 1;
    for vector in &vectors {
        let (seed, expected) = (&vector["seed"], &vector["components"]);
        assert_eq!(&components(&vector["input"]), expected, "seed {seed}");
        assert_eq!(
            &components(&permuted(&vector["input"])),
            expected,
            "seed {seed}, permuted"
        );
        count(format!("component:{}", vector["case"].as_str().unwrap()));
        for component in expected["components"].as_array().unwrap() {
            count(format!("effective:{}", component["effective_state"]));
            for issue in component["validation"]["issues"].as_array().unwrap() {
                count(format!("issue:{}", issue["code"].as_str().unwrap()));
            }
        }
    }
    assert!(counts.contains_key("effective:\"active\""), "{counts:?}");
    let cases: Vec<&String> = counts.keys().filter(|k| k.starts_with("component:")).collect();
    let per_case = 2000 / cases.len();
    assert!(cases.iter().all(|c| counts[*c] >= per_case), "{counts:?}");
    for code in [
        "active_lineage_conflict",
        "active_transaction_membership_conflict",
        "component_leg_count_mismatch",
        "anchor_transaction_missing",
        "anchor_transaction_excluded",
        "unbalanced_quantity",
        "allocation_source_coverage_mismatch",
        "allocation_sink_coverage_mismatch",
        "custody_network_scope_laundering",
        "custody_location_continuity_mismatch",
        "revision_link_missing",
        "revision_link_invalid",
        "component_native_support_contradicted",
        "allocation_chronology_mismatch",
    ] {
        assert!(counts.contains_key(&format!("issue:{code}")), "{code}: {counts:?}");
    }
}
