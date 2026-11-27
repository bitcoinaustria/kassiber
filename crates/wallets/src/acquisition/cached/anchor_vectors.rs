//! Python's compatibility history anchors (rounds 5 and 5b), replayed.
//! `generate_anchor_vectors.py` records every request the unchanged Python
//! adapter made against a deterministic chain and the reply it received. The
//! Rust acquisition receives the same replies through loopback fakes and must
//! derive the same freshness facts. Esplora hashes in the captures are translated
//! from legacy Electrum order before serving them to the current client.
use super::*;
use crate::acquisition::tests::{run, scope};
use kassiber_net::test_support::{FakeElectrum, FakeProvider};
use sha2::{Digest, Sha256};

fn cases(protocol: &str) -> Vec<Value> {
    let all: Value = serde_json::from_str(include_str!(
        "../../../tests/vectors/python_history_anchors.json"
    ))
    .unwrap();
    all.as_array()
        .unwrap()
        .iter()
        .filter(|c| c["protocol"] == protocol)
        .cloned()
        .collect()
}

/// The generator's three targets: two used scripts and one unused.
fn targets() -> Vec<Value> {
    let mut result = vec![];
    for i in 0..3 {
        let digest = Sha256::digest(format!("anchor-{i}").as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        result.push(json!({"script_pubkey":format!("0014{}", &hex[..40]),
            "branch_index":0,"address_index":i,"chain":"bitcoin","network":"main"}));
    }
    result
}

const PYTHON_URL: &str = "https://example.invalid/";

/// Esplora absence memberships bind the whole transport, including the URL.
/// The generator's URL and the loopback URL differ only there, so map one
/// backend key to the other, after checking Python's key is Rust's for it.
fn membership_key(url: &str) -> String {
    let backend = json!({"kind":"esplora","name":"fixture","url":url,"batch_size":1});
    let identity = cache::backend_identity(&backend, "bitcoin", "main", 1);
    cache::esplora_key(&backend, &identity)
}
fn rekey(checkpoint: &Value, from: &str, to: &str) -> Value {
    let mut checkpoint = checkpoint.clone();
    let key = &mut checkpoint["esplora_history_memberships"]["backend_key"];
    if key.is_string() {
        assert_eq!(key.as_str(), Some(from), "Python membership key");
        *key = json!(to);
    }
    checkpoint
}

fn forward_hash(hash: &str) -> String {
    let mut bytes = Vec::<u8>::from_hex(hash).unwrap();
    bytes.reverse();
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn forward_keys(entries: &mut Value) {
    if let Some(entries) = entries.as_object_mut() {
        *entries = std::mem::take(entries)
            .into_iter()
            .map(|(hash, value)| (forward_hash(&hash), value))
            .collect();
    }
}

fn forward_checkpoint(mut checkpoint: Value) -> Value {
    if !checkpoint.is_object() {
        return checkpoint;
    }
    if let Some(stats) = checkpoint.get_mut("esplora_scripthashes") {
        forward_keys(stats);
    }
    if let Some(scripts) = checkpoint
        .get_mut("esplora_history_memberships")
        .and_then(|v| v.get_mut("scripts"))
    {
        forward_keys(scripts);
    }
    checkpoint["esplora_scripthash_order"] = json!(cache::ESPLORA_HASH_ORDER);
    checkpoint
}

fn acquisition(case: &Value, kind: &str, url: &str) -> Acquisition {
    let checkpoint = if kind == "esplora" {
        forward_checkpoint(rekey(
            &case["checkpoint"],
            &membership_key(PYTHON_URL),
            &membership_key(url),
        ))
    } else {
        case["checkpoint"].clone()
    };
    Acquisition::new_cached(
        scope(),
        &json!({"kind":kind,"name":"fixture","url":url,"batch_size":1}),
        &targets(),
        "fixture",
        &checkpoint,
        &json!({}),
        "main",
    )
    .unwrap()
}

fn same<T: PartialEq + std::fmt::Debug>(actual: &[T], expected: &[T], name: &str) {
    if let Some(i) =
        (0..actual.len().max(expected.len())).find(|i| actual.get(*i) != expected.get(*i))
    {
        panic!(
            "{name}: request {i} differs: {:?} != Python {:?}",
            actual.get(i),
            expected.get(i)
        );
    }
}

/// The confirmation facts Python writes for each listed Electrum UTXO: depth
/// from the second connection's tip and block time from the UTXO's header.
fn utxo_rows(actual: &Value) -> Value {
    let tip = actual["tip_height"].as_i64();
    let mut rows = vec![];
    for script in actual["scripts"].as_array().unwrap() {
        for utxo in script["utxos"].as_array().into_iter().flatten() {
            let height = crate::scan::integer(&utxo["height"])
                .and_then(|h| i64::try_from(h).ok())
                .filter(|h| *h > 0);
            let time = height
                .and_then(|h| actual["utxo_headers"][h.to_string()].as_i64())
                .and_then(kassiber_chain::format_unix_time);
            let confirmations = height
                .zip(tip)
                .filter(|(h, t)| t >= h)
                .map(|(h, t)| t - h + 1);
            rows.push(json!({"txid":utxo["tx_hash"],"vout":utxo["tx_pos"],
                "confirmation_status":if height.is_some() {"confirmed"} else {"mempool"},
                "confirmations":confirmations,"block_height":height,"block_time":time}));
        }
    }
    Value::Array(rows)
}

fn compare(case: &Value, actual: &Value, url: &str) {
    let name = case["name"].as_str().unwrap();
    let checkpoint = if case["protocol"] == "esplora" {
        rekey(
            &actual["freshness_checkpoint"],
            &membership_key(url),
            &membership_key(PYTHON_URL),
        )
    } else {
        actual["freshness_checkpoint"].clone()
    };
    let expected = case["freshness_checkpoint"].clone();
    let expected = if case["protocol"] == "esplora" {
        forward_checkpoint(expected)
    } else {
        expected
    };
    assert_eq!(checkpoint, expected, "{name}: checkpoint");
    assert_eq!(
        actual["scripts_changed"], case["scripts_changed"],
        "{name}: changed"
    );
    assert_eq!(
        actual["utxos_skipped_unchanged"], case["utxos_skipped_unchanged"],
        "{name}: UTXO reuse"
    );
}

#[test]
fn esplora_history_anchors_match_python_requests_and_checkpoints() {
    let cases = cases("esplora");
    assert!(cases.len() >= 20);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let requests = case["requests"].as_array().unwrap();
        let server = FakeProvider::replies(
            requests
                .iter()
                .map(|r| {
                    (
                        u16::try_from(r["status"].as_u64().unwrap()).unwrap(),
                        r["body"].as_str().unwrap().to_owned(),
                    )
                })
                .collect(),
        )
        .unwrap();
        let actual = run(&acquisition(case, "esplora", server.base())).unwrap();
        let paths: Vec<String> = server
            .requests()
            .iter()
            .map(|line| {
                let target = line.split(' ').nth(1).unwrap();
                target.trim_start_matches('/').to_owned()
            })
            .collect();
        let expected: Vec<String> = requests
            .iter()
            .map(|r| {
                let path = r["path"].as_str().unwrap();
                path.strip_prefix("scripthash/")
                    .and_then(|suffix| suffix.split_at_checked(64))
                    .map_or_else(
                        || path.to_owned(),
                        |(hash, suffix)| format!("scripthash/{}{suffix}", forward_hash(hash)),
                    )
            })
            .collect();
        same(&paths, &expected, name);
        compare(case, &actual, server.base());
    }
}

#[test]
fn electrum_history_anchors_match_python_requests_and_checkpoints() {
    let cases = cases("electrum");
    assert!(cases.len() >= 20);
    for case in &cases {
        let name = case["name"].as_str().unwrap();
        let calls = case["requests"].as_array().unwrap();
        let reply = |id: usize, call: &Value| {
            let body = match call.get("error") {
                Some(message) => json!({"id":id,"error":{"code":1,"message":message}}),
                None => json!({"id":id,"result":call["result"]}),
            };
            format!("{body}\n")
        };
        // Each connection opens with the protocol handshake (request id 1);
        // Python reads the UTXOs over a second connection.
        let mut sessions: Vec<Vec<(usize, String)>> = vec![];
        let mut next = 0;
        for call in calls {
            if call["connect"] == true {
                sessions.push(vec![(
                    1,
                    format!("{}\n", json!({"id":1,"result":["fixture","1.6"]})),
                )]);
                next = 2;
                continue;
            }
            sessions.last_mut().unwrap().push((1, reply(next, call)));
            next += 1;
        }
        let server = FakeElectrum::sessions(sessions).unwrap();
        let actual = run(&acquisition(case, "electrum", server.url())).unwrap();
        let sent: Vec<Value> = server
            .requests()
            .iter()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter(|request| request["method"] != "server.version")
            .map(|request| json!({"method":request["method"],"params":request["params"]}))
            .collect();
        let expected: Vec<Value> = calls
            .iter()
            .filter(|c| c["connect"] != true)
            .map(|c| json!({"method":c["method"],"params":c["params"]}))
            .collect();
        same(&sent, &expected, name);
        let handshakes = server
            .requests()
            .iter()
            .filter(|line| line.contains("server.version"))
            .count();
        let connections = calls.iter().filter(|c| c["connect"] == true).count();
        assert_eq!(handshakes, connections, "{name}: connections");
        assert_eq!(utxo_rows(&actual), case["utxos"], "{name}: UTXO rows");
        compare(case, &actual, server.url());
    }
}
