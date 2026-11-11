//! `ui.transactions.graph` on the local graph book. Replaces the
//! kill-holding test of `tests/test_rust_transaction_graph.py`.

use serde_json::json;

use crate::desktop::{self, compare, graph, some};
use crate::support::Transcript;

const TX1: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
const TX2: &str = "b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2b2";

#[test]
fn test_local_graphs_match_wire_bytes() {
    let sandbox = desktop::book();
    sandbox.sql(&graph());
    let mut t = Transcript::new("transaction_graph/local_graphs");
    let mut args: Vec<_> = [
        "g1", TX1, TX2, "g3", "g4", "g5", "g6", "h", "k", "a", " g2 ",
    ]
    .iter()
    .map(|reference| json!({"transaction": reference}))
    .collect();
    args.extend([
        json!({"transaction": "g1", "allowPublicLookup": false}),
        json!({"transaction": "g4", "allow_public_lookup": "TRUE"}),
        json!({"transaction": "g3", "allowPublicLookup": true}),
    ]);
    let responses = compare(&mut t, &sandbox, "ui.transactions.graph", &some(args));
    let graph = &responses[0]["data"];
    assert_eq!(graph["supportLevel"], "partial");
    let owned = graph["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .chain(graph["outputs"].as_array().unwrap())
        .any(|node| node["ownership"] == "owned");
    assert!(owned);
    assert_eq!(graph["transaction"]["size"], 60);
    assert_eq!(responses[6]["data"]["outputs"][0]["role"], "change");
    t.finish();
}
