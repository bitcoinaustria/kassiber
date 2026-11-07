//! One test per scanner rule family, against Python's output for small
//! inputs (`tests/data/scanner_families.json`, written by
//! `scripts/gen_scanner_vectors.py fixtures`). The mutation entries in
//! `scripts/mutations.json` name these tests.

use kassiber_envelope::redaction::{is_sensitive_key, redact_secret_text};
use serde_json::Value;

const FAMILIES: &str = include_str!("data/scanner_families.json");

fn family(name: &str) {
    let families: Value = serde_json::from_str(FAMILIES).unwrap();
    let cases = families[name]
        .as_array()
        .unwrap_or_else(|| panic!("no family {name}"));
    assert!(!cases.is_empty());
    let mismatches: Vec<String> = cases
        .iter()
        .filter_map(|case| {
            let (input, expected) = (case[0].as_str().unwrap(), case[1].as_str().unwrap());
            let output = redact_secret_text(input);
            (output != expected)
                .then(|| format!("{input:?}\n  python {expected:?}\n  rust   {output:?}"))
        })
        .collect();
    assert!(mismatches.is_empty(), "{name}:\n{}", mismatches.join("\n"));
}

#[test]
fn names_segment_into_words() {
    family("segment");
}

#[test]
fn count_shaped_names_keep_metadata() {
    family("count");
}

#[test]
fn literals_end_at_value_boundaries() {
    family("literal");
}

#[test]
fn flag_values_end_at_known_flags() {
    family("flag");
}

#[test]
fn continuations_own_deeper_lines() {
    family("continuation");
}

#[test]
fn url_userinfo_is_masked() {
    family("url");
}

#[test]
fn path_tokens_run_to_whitespace() {
    family("path_token");
}

#[test]
fn keyed_values_are_masked() {
    family("keyed");
}

#[test]
fn xml_values_are_masked() {
    family("xml");
}

#[test]
fn named_pairs_are_masked() {
    family("pair");
}

#[test]
fn cli_credentials_are_masked() {
    family("cli");
}

#[test]
fn bare_userinfo_is_masked() {
    family("bare");
}

#[test]
fn decoded_views_are_scanned() {
    family("view");
}

#[test]
fn overlapping_spans_merge() {
    family("merge");
}

#[test]
fn floor_patterns_follow_the_scanner() {
    family("floor");
}

#[test]
fn sensitive_keys_match_python() {
    // tests/test_redaction_shared_corpus.py::test_rpc_and_lightning_secret_fields
    for key in [
        "rpcauth",
        "rpc_auth",
        "rpc-auth",
        "macaroon",
        "rpcpassword",
        "cookie",
        "scan_key",
        "BITCOIN_RPC_PASSWORD",
        "LND_MACAROON",
        "macaroon_hex",
        "db_passphrase",
        "Authorization",
        "rune",
        "bitcoin.rpcpassword",
        "api-key",
        "silentPayment",
        "spspend",
    ] {
        assert!(is_sensitive_key(key), "{key}");
    }
    for key in ["message", "url", "amount_sat", "auth", "user", "pin"] {
        assert!(!is_sensitive_key(key), "{key}");
    }
}
