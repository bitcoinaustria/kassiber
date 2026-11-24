//! The projection schema stays complete: every column the loader hands the
//! journal pipeline is classified (dropped columns are named too), and every
//! residual marker it keeps is still interpreted where it says. These fail,
//! they never pass silently, when the loader or the engine changes.
#![recursion_limit = "256"]

use kassiber_replay::projection::{
    class_of, kept, section, unclassified, ENGINE_CONSTANTS, F, MARKERS,
};
use kassiber_store::{Book, OpenOptions};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Columns the loader reads that the projection deliberately drops: named
/// here so a new column cannot slip by unclassified.
const DROPPED: &[(&str, &str)] = &[
    ("profile", "ownership_review_counts_json"),
    ("migration_issues", "details_json"),
];

/// Database tables whose every column reaches a snapshot section through
/// `SELECT *` (or an explicit column list that may grow).
const TABLES: &[(&str, &str)] = &[
    ("profile", "profiles"),
    ("transactions", "transactions"),
    ("anchors", "transactions"),
    ("components.header", "custody_components"),
    ("revision_headers", "custody_components"),
    ("components.legs", "custody_component_legs"),
    ("components.allocations", "custody_component_allocations"),
    (
        "components.economic_terms",
        "custody_component_economic_terms",
    ),
    (
        "components.commitments",
        "custody_component_evidence_commitments",
    ),
    ("gap_reviews", "custody_gap_reviews"),
    ("pair_dismissals", "transaction_pair_dismissals"),
    ("migration_issues", "custody_authored_migration_issues"),
    ("book_network.binding", "book_network_bindings"),
];

fn classified(section_name: &str, column: &str) -> bool {
    section(section_name).is_some_and(|schema| schema.iter().any(|(name, _)| *name == column))
        || DROPPED.contains(&(section_name, column))
}

#[test]
fn every_schema_column_of_the_loaded_tables_is_classified() {
    let schema: Value =
        serde_json::from_str(include_str!("../../store/fixtures/current-schema.json")).unwrap();
    let mut columns: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for object in schema["objects"].as_array().unwrap() {
        if object[0] != "table" {
            continue;
        }
        let sql = object[3].as_str().unwrap();
        let body = &sql[sql.find('(').unwrap() + 1..];
        let names = body
            .lines()
            .map(str::trim)
            .filter_map(|line| line.split_whitespace().next())
            .filter(|word| {
                word.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
                    && !["unique", "primary", "check", "foreign", "constraint"].contains(word)
            })
            .map(str::to_owned)
            .collect();
        columns.insert(object[1].as_str().unwrap().to_owned(), names);
    }
    let mut missing = Vec::new();
    for (section_name, table) in TABLES {
        for column in &columns[*table] {
            if !classified(section_name, column) {
                missing.push(format!("{section_name}.{column}"));
            }
        }
    }
    assert!(
        missing.is_empty(),
        "unclassified loaded columns: {missing:?}"
    );
}

/// Load a frozen book's snapshot as the pipeline gets it.
fn snapshot(name: &str) -> Value {
    let root =
        std::env::temp_dir().join(format!("kassiber-coverage-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    drop(Book::create(&root, None).unwrap());
    let rows = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../kassiber/tests/data/books")
            .join(format!("{name}.sql")),
    )
    .unwrap();
    {
        let book = Book::open_migrating(&root, OpenOptions::default(), true).unwrap();
        let conn = book.connection();
        conn.set_db_config(
            kassiber_store::rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
            false,
        )
        .unwrap();
        conn.execute_batch("PRAGMA foreign_keys=OFF; BEGIN")
            .unwrap();
        let names: Vec<String> = conn
            .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_stat%'")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        for name in names {
            conn.execute_batch(&format!("DELETE FROM \"{name}\""))
                .unwrap();
        }
        conn.execute_batch(&rows).unwrap();
        conn.execute_batch("COMMIT").unwrap();
    }
    let book = Book::open_migrating(&root, OpenOptions::default(), true).unwrap();
    let source =
        kassiber_journals::reproduce::bundle_source(book.connection(), None, None).unwrap();
    let value = serde_json::to_value(&source.snapshot).unwrap();
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
    value
}

#[test]
fn every_field_the_loader_emits_is_classified() {
    let mut missing = BTreeSet::new();
    let mut check = |name: &str, rows: &Value| {
        let rows: Vec<&Value> = match rows {
            Value::Array(rows) => rows.iter().collect(),
            Value::Object(_) => vec![rows],
            _ => vec![],
        };
        let schema = section(name).unwrap_or_else(|| panic!("unclassified section {name}"));
        for row in rows {
            // Recursively, into raw_json, evidence payloads and configuration.
            for path in unclassified(schema, row) {
                let top = path.trim_start_matches('.');
                if !DROPPED.contains(&(name, top)) {
                    missing.insert(format!("{name}{path}"));
                }
            }
        }
    };
    for book in [
        "paired-fee-processed",
        "paired-short-processed",
        "plain-blocked-processed",
        "gaps-10-processed",
        "plain-unchanged",
        "identify-onboarding",
        "legacy-pair-fee-c0a015de",
    ] {
        let snapshot = snapshot(book);
        for (name, value) in snapshot.as_object().unwrap() {
            match name.as_str() {
                "schema_version" | "operation" | "as_of" | "owned_index" | "ownership_warnings" => {
                }
                "anchors" => {
                    for row in value.as_object().unwrap().values() {
                        check("anchors", row);
                    }
                }
                "components" => {
                    for component in value.as_array().unwrap() {
                        for (part, rows) in component.as_object().unwrap() {
                            check(&format!("components.{part}"), rows);
                        }
                    }
                }
                "gap_worklist" => check("gap_worklist.rows", &value["rows"]),
                "book_network" => {
                    for (part, rows) in value.as_object().into_iter().flatten() {
                        check(&format!("book_network.{part}"), rows);
                    }
                }
                other => {
                    assert!(section(other).is_some(), "unclassified section {other}");
                    check(other, value);
                }
            }
        }
    }
    assert!(
        missing.is_empty(),
        "unclassified loaded fields: {missing:?}"
    );
}

/// Every value the engine compares a field against survives the schema as
/// written, and the source file named for it still names it: a constant the
/// projection replaced would make the projected book compute differently.
#[test]
fn every_engine_constant_survives_the_schema() {
    let mut problems = Vec::new();
    for (section_name, path, values, site) in ENGINE_CONSTANTS {
        let source =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(site))
                .unwrap_or_else(|e| panic!("{site}: {e}"));
        for value in *values {
            if !source.contains(&format!("\"{value}\"")) {
                problems.push(format!("{site} no longer names {value:?}"));
            }
            if !kept(section_name, path, value) {
                problems.push(format!("{section_name}.{} drops {value:?}", path.join(".")));
            }
        }
    }
    assert!(problems.is_empty(), "{problems:#?}");
    // The observer's chain instance in raw_json is kept as an identity, so
    // the network gate still sees it agree or disagree with the wallet's.
    for section_name in ["transactions", "book_network.observations"] {
        assert!(matches!(
            class_of(section_name, &["raw_json", "chain_instance_id"]),
            Some(F::External)
        ));
    }
}

#[test]
fn every_kept_marker_is_interpreted_where_it_says() {
    for (marker, site) in MARKERS {
        let source =
            std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join(site))
                .unwrap_or_else(|_| panic!("{site}"));
        let suffix = marker.strip_prefix("reviewed_residual:").unwrap();
        // Named whole where it is compared, or read through the prefix the
        // engine strips, with a classification the gap review writes.
        let classifications = include_str!("../../transfers/src/gap_review.rs");
        assert!(
            source.contains(&format!("\"{marker}\""))
                || (source.contains("strip_prefix(\"reviewed_residual:\")")
                    && classifications.contains(&format!("\"{suffix}\""))),
            "{marker} is no longer interpreted in {site}"
        );
    }
}

#[test]
fn every_kept_local_reference_prefix_is_written_by_the_review() {
    let writers = [
        include_str!("../../transfers/src/pair_spec.rs"),
        include_str!("../../transfers/src/gap_review.rs"),
    ];
    for prefix in kassiber_replay::projection::LOCAL_REFS {
        assert!(
            writers.iter().any(|source| source.contains(prefix)),
            "{prefix} is no longer written"
        );
    }
}

/// The documents the custody interpreters read, with every field they
/// interpret, optional branches included: the graph and confirmation
/// parsers (`custody/src/graph.rs`, `custody/src/confirmation.rs`), the
/// network gate (`custody/src/gates/network.rs`), the Lightning channel
/// interpreter (`custody/src/interpretation/channels.rs`), the reviewed
/// residual reader (`custody/src/quantity/reviewed.rs`) and the loader's
/// configuration facts (`custody/src/snapshot.rs` `validate_loaded`). A
/// field missing from the projection fails here even when no frozen book
/// carries it.
#[test]
fn every_field_the_interpreters_read_in_documents_is_classified() {
    let txid = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
    let raw = serde_json::json!({
        "txid": txid, "hash": txid, "tx_hash": txid, "transaction_hash": txid,
        "chain": "bitcoin", "network": "main", "asset": "BTC", "asset_id": txid,
        "fee": 1, "hex": "00", "raw_hex": "00", "direction": "inbound",
        "_kassiber_verified_chain_observation": true, "custody_native_fee_normalized": true,
        "observer_owned_scripts": ["0014"],
        "status": {"confirmed": true, "block_height": 1, "block_time": 1},
        "history": {"height": 1},
        "_kassiber_electrum_graph": {"kind": "bitcoin_electrum", "version": 2},
        "vin": [{"txid": txid, "vout": 0, "prevout": {"scriptpubkey": "0014", "scriptpubkey_address": "bc1q", "value": 1}}],
        "vout": [{"n": 0, "index": 0, "scriptpubkey": "0014", "script_pubkey": "0014", "address": "bc1q", "value": 1, "value_sats": 1}],
        "inputs": [{"outpoint": format!("{txid}:0"), "script": "0014", "amount": 1}],
        "outputs": [{"script": "0014", "amount_msat": 1}],
        "tx": {"txid": txid},
        "channel_id": "c", "funding_txid": txid, "closing_txid": txid, "sweep_txid": txid,
        "funding_outpoint": format!("{txid}:0"), "outpoint": format!("{txid}:0"),
        "amount_msat": 1, "fee_msat": 1, "funding_amount_msat": 1, "close_balance_msat": 1,
        "payment_hash": txid, "tag": "open",
        "ownership_graph": {"txid": txid, "vin": [{"txid": txid, "vout": 0}], "vout": [{"n": 0, "scriptpubkey": "0014", "value": 1}]},
        "component": {"asset_id": txid, "asset": "LBTC"},
        "_kassiber_provenance": {"import_source": "lnd_adapter"},
        "channel_close_local_sweeps": [{"sweep_txid": txid, "outpoint": format!("{txid}:0")}],
    })
    .to_string();
    let config = serde_json::json!({
        "chain": "bitcoin", "network": "main", "bitcoin_network": "main", "chain_network": "main",
        "chain_instance_id": txid,
        "samourai": {"role": "postmix", "group_id": "g", "section": "postmix"},
    })
    .to_string();
    let evidence = serde_json::json!({
        "review_id": "r", "term_kind": "transaction_pair",
        "residual_classification": {"classification": "retained_custody"},
    })
    .to_string();
    for (name, row) in [
        (
            "transactions",
            serde_json::json!({"raw_json": raw, "config_json": config, "wallet_config_json": config}),
        ),
        (
            "channel_records",
            serde_json::json!({"raw_json": raw, "config_json": config}),
        ),
        ("wallets", serde_json::json!({"config_json": config})),
        (
            "components.header",
            serde_json::json!({"evidence_json": evidence}),
        ),
    ] {
        let missing = unclassified(section(name).unwrap(), &row);
        assert!(missing.is_empty(), "{name}: unclassified {missing:?}");
    }
}

/// The custody interpreters that read book documents (`raw_json`, evidence,
/// configuration, observations) by literal key.
const INTERPRETERS: &[&str] = &[
    "custody/src/graph.rs",
    "custody/src/graph",
    "custody/src/confirmation.rs",
    "custody/src/interpretation",
    "custody/src/evidence.rs",
    "custody/src/liquid.rs",
    "custody/src/source_funds.rs",
    "custody/src/finalized_projection.rs",
    "custody/src/quantity/reviewed.rs",
    "custody/src/gates/network.rs",
];

/// Keys the interpreters read that no book document carries: structures the
/// engine builds itself, and row fields no loaded table has. Adding a key
/// here is a review decision: check it is never read from `raw_json`,
/// evidence, configuration or a loaded column first.
const COMPUTED: &[&str] = &[
    // Pair specs and movements the matcher and reviewed-pair compiler build
    // (interpretation/pairs.rs, matcher.rs, fanout.rs, exchange.rs).
    "allow_cross_rail",
    "allow_unclaimed_residual",
    "atomic_bundle_id",
    "authoritative_chain_observation",
    "confidence",
    "conflict_set_id",
    "conflict_size",
    "default_kind",
    "derived_pairs",
    "destination_kind",
    "event_key",
    "fee_attribution",
    "in",
    "in_id",
    "method",
    "observation_authority_reason",
    "out",
    "out_amount",
    "out_amount_msat",
    "out_id",
    "out_occurred_at",
    "pair_id",
    "pair_source",
    "paired_leg",
    "pairs",
    "policy",
    "provider_or_import",
    "roles",
    "target",
    // Claims, holds, quarantines and reconciliations the interpretation
    // pipeline builds (consolidation.rs, safety.rs, reconciliation.rs,
    // pipeline.rs, native.rs, holds.rs).
    "_closed_confirmation_observation",
    "blocked_transaction_ids",
    "candidate_ids",
    "claim_id",
    "claims",
    "detail",
    "effective_state",
    "end_msat",
    "fee_claims",
    "journal_transaction_id",
    "native_component_reconciliations",
    "observation_hash",
    "observed_at",
    "physical_scope",
    "priority",
    "quarantines",
    "row",
    "start_msat",
    "touched_ids",
    "unallocated_fee_msat",
    // Observation authority and canonical event issues evidence.rs caches.
    "authoritative",
    "code",
    "transaction_ids",
    // The finalized projection and reviewed quantity stages
    // (finalized_projection.rs, quantity/reviewed.rs).
    "custody_finalized_tax_projection",
    "evidence",
    "observation",
    "op",
    "payout",
    "payout_id",
    "projection_code",
    "records",
    "side",
    // The merged graph graph/stored.rs builds.
    "evidence_conflicts",
    // Row fields no loaded table has and no loader sets: always absent from
    // a snapshot.
    "backend_name",
    "source_ref",
    "swap_refund_funding_outpoint",
];

/// Every literal key an interpreter reads (`x["key"]`, `get(x, "key")`,
/// `.field("key")`, `.at("key")`), test modules excluded.
fn interpreter_keys() -> BTreeMap<String, String> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut files = Vec::new();
    for entry in INTERPRETERS {
        let path = base.join(entry);
        if path.is_dir() {
            let mut stack = vec![path];
            while let Some(dir) = stack.pop() {
                for item in std::fs::read_dir(&dir).unwrap().flatten() {
                    let p = item.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|e| e == "rs") {
                        files.push(p);
                    }
                }
            }
        } else {
            files.push(path);
        }
    }
    let mut keys = BTreeMap::new();
    for file in files {
        let source = std::fs::read_to_string(&file).unwrap_or_else(|_| panic!("{file:?}"));
        let source = source.split("#[cfg(test)]").next().unwrap_or_default();
        let bytes = source.as_bytes();
        let mut at = 0;
        while let Some(offset) = source[at..].find('"') {
            let start = at + offset + 1;
            let Some(len) = source[start..].find('"') else {
                break;
            };
            let key = &source[start..start + len];
            at = start + len + 1;
            if key.is_empty()
                || !key
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
                || !key.as_bytes()[0].is_ascii_lowercase() && key.as_bytes()[0] != b'_'
            {
                continue;
            }
            let before = source[..start - 1].trim_end_matches(' ');
            let in_call = |name: &str| {
                before.rfind(name).is_some_and(|open| {
                    let args = &source[open + name.len()..start - 1];
                    !args.contains(')') && !args.contains(';') && args.trim_end().ends_with(',')
                })
            };
            let closes = source[at..].starts_with(']');
            let read = (before.ends_with('[') && closes)
                || before.ends_with("field(")
                || before.ends_with(".at(")
                || before.ends_with(".get(")
                || in_call("get(");
            if read && bytes[start - 2] != b'\\' {
                keys.entry(key.to_owned()).or_insert_with(|| {
                    file.strip_prefix(&base)
                        .unwrap_or(&file)
                        .display()
                        .to_string()
                });
            }
        }
    }
    keys
}

/// Fourth review, 5: the fixture above lists the document fields by hand;
/// this keeps it from drifting. A key an interpreter starts reading fails
/// here until the projection classifies it, or until it is named as a key
/// of a computed structure.
#[test]
fn every_key_the_interpreters_read_is_classified_or_computed() {
    let classified = kassiber_replay::projection::field_names();
    let missing: Vec<String> = interpreter_keys()
        .into_iter()
        .filter(|(key, _)| !classified.contains(key.as_str()) && !COMPUTED.contains(&key.as_str()))
        .map(|(key, file)| format!("{key} ({file})"))
        .collect();
    assert!(
        missing.is_empty(),
        "unclassified interpreter keys: {missing:#?}"
    );
    for key in COMPUTED {
        assert!(
            !classified.contains(key),
            "{key} is classified; drop it from COMPUTED"
        );
    }
}
