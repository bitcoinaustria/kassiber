//! Replay bundles of frozen processed books: round trip, tampering, the
//! secret floor, and that export writes nothing but the bundle.
use kassiber_journals::service::ReplayGuard;
use kassiber_store::{rusqlite::Connection, Book, OpenOptions};
use serde_json::Value;
use std::path::{Path, PathBuf};

const PASSPHRASE: &str = "replay-test-passphrase";
const TXID: &str = "4a5e1e4baab89f3a32518a88c31bc87f618f76673e2cc77ab2127b7afdeda33b";
const ADDRESS: &str = "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq";
const XPUB: &str = "xpub6CUGRUonZSQ4TWtTMmzXdrXDtypWKiKrhko4egpiMZbpiaQL2jkwSB1icqYh2cfDfVxdx4df189oLKnC5fSwqPfgyP3hooxujYzAu3fDVmz";
const BLINDING: &str = "1111111111111111111111111111111111111111111111111111111111111abc";
const SCAN: &str = "2222222222222222222222222222222222222222222222222222222222222def";

fn root(name: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "kassiber-replay-{name}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// A fresh book holding a frozen book's rows, with `extra` applied while
/// triggers are off, then processed so its journals are current.
fn book(name: &str, extra: &str) -> (PathBuf, Book) {
    let root = root(name);
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
        conn.execute_batch(extra).unwrap();
        conn.execute_batch("COMMIT").unwrap();
    }
    let book = Book::open_migrating(&root, OpenOptions::default(), true).unwrap();
    kassiber_journals::service::process(book.connection(), None, None, &mut |_| {})
        .unwrap()
        .after_reply()
        .unwrap();
    (root, book)
}

/// The fixture's private material: wallet configuration secrets, a backend
/// with credentials, chain identifiers, labels and notes.
fn secrets() -> String {
    format!(
        "UPDATE wallets SET label='Alice Cold Storage', config_json='{{\"descriptor\":\"wpkh([d34db33f/84h/0h/0h]{XPUB}/0/*)\",\"blinding_key\":\"slip77({BLINDING})\",\"silent_payments_scan_key\":\"{SCAN}\",\"backend_url\":\"https://electrum.example.org:50002\"}}' WHERE id='wallet';
         UPDATE transactions SET external_id='{TXID}:1', note='Rent for flat 4B', description='Payment to Bob Builder', counterparty='Carol Example GmbH', raw_json='{{\"txid\": \"{TXID}\", \"address\": \"{ADDRESS}\", \"label\": \"Alice savings\"}}' WHERE id='a';
         INSERT INTO backends(name,kind,url,auth_header,token,notes,created_at,updated_at) VALUES('private-node','electrum','ssl://node.example.org:50002','Basic c2VjcmV0','backend-token-secret','home node','2024','2024');"
    )
}

fn export(conn: &Connection) -> kassiber_replay::Export {
    kassiber_replay::export(
        conn,
        None,
        None,
        PASSPHRASE,
        "2026-11-12T00:00:00+00:00",
        "0.0.0-test",
        &kassiber_transfers::SystemSources,
    )
    .unwrap()
}

fn open(sealed: &[u8]) -> Value {
    serde_json::from_slice(&kassiber_backup::open_with_passphrase(sealed, PASSPHRASE).unwrap())
        .unwrap()
}

/// Reseal `document` with a manifest matching its (edited) parts.
fn reseal(mut document: Value) -> Vec<u8> {
    for part in ["inputs", "outputs"] {
        let text = document["parts"][part].as_str().unwrap().to_owned();
        document["manifest"]["parts"][part] = serde_json::json!({
            "sha256": kassiber_replay::pseudonym::sha256_hex(text.as_bytes()),
            "bytes": text.len(),
        });
    }
    kassiber_backup::seal_with_passphrase(&serde_json::to_vec(&document).unwrap(), PASSPHRASE)
        .unwrap()
}

#[test]
fn an_exported_bundle_verifies_identical_and_holds_no_private_material() {
    let (root, book) = book("paired-fee-processed", &secrets());
    let conn = book.connection();
    let guard = ReplayGuard::capture(conn);
    let files_before = files(&root);
    let exported = export(conn);
    assert!(guard.unchanged(conn), "export wrote to the book");
    assert_eq!(files(&root), files_before, "export wrote a file");
    let manifest = &exported.manifest;
    assert_eq!(manifest["format"], "kassiber.replay-bundle");
    assert_eq!(
        manifest["local_check"]["status"], "identical",
        "{manifest:#}"
    );
    assert_eq!(
        manifest["pseudonymization"]["faithful"], true,
        "{manifest:#}"
    );
    assert!(manifest["counts"]["journal_entries"].as_u64().unwrap() > 0);

    let plaintext = String::from_utf8(
        kassiber_backup::open_with_passphrase(&exported.sealed, PASSPHRASE).unwrap(),
    )
    .unwrap();
    for secret in [
        XPUB,
        &XPUB[4..20],
        "d34db33f",
        "wpkh(",
        "slip77",
        BLINDING,
        SCAN,
        "electrum.example.org",
        "node.example.org",
        "backend-token-secret",
        "c2VjcmV0",
        "private-node",
        "home node",
        TXID,
        ADDRESS,
        "Alice",
        "Bob Builder",
        "Carol",
        "Rent for flat",
        "Treasury",
    ] {
        assert!(!plaintext.contains(secret), "the bundle holds {secret:?}");
    }
    // The ciphertext holds none of it either, nor any plaintext structure.
    assert!(!String::from_utf8_lossy(&exported.sealed).contains("kassiber.replay-bundle"));

    let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 20).unwrap();
    assert_eq!(report["status"], "identical", "{report:#}");
    assert_eq!(report["deterministic"], true);
    assert_eq!(report["differing"], 0);
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

fn files(root: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if !path.to_string_lossy().ends_with("-shm") {
                out.push((path.clone(), entry.metadata().unwrap().len()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn tampered_outputs_and_mutated_inputs_give_precise_diffs_and_corruption_is_refused() {
    let (root, book) = book("paired-fee-processed", "");
    let exported = export(book.connection());
    let document = open(&exported.sealed);

    // A recorded output changed after export.
    let mut tampered = document.clone();
    let mut outputs: Value =
        serde_json::from_str(tampered["parts"]["outputs"].as_str().unwrap()).unwrap();
    let entries = outputs["tables"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|t| t["table"] == "journal_entries")
        .unwrap();
    let row = &mut entries["rows"][0];
    let key = row["key"].as_str().unwrap().to_owned();
    row["columns"]["fiat_value"] = Value::from("999999.99");
    tampered["parts"]["outputs"] = Value::from(serde_json::to_string(&outputs).unwrap());
    let report = kassiber_replay::verify(&reseal(tampered), PASSPHRASE, 5).unwrap();
    assert_eq!(report["status"], "different", "{report:#}");
    assert_eq!(report["differing"], 1);
    let difference = &report["differences"][0];
    assert_eq!(difference["table"], "journal_entries");
    assert_eq!(difference["key"], key.as_str());
    assert_eq!(difference["columns"][0]["column"], "fiat_value");
    assert_eq!(difference["columns"][0]["stored"], "999999.99");

    // An input changed after export: a transaction's amount.
    let mut mutated = document.clone();
    let mut inputs: Value =
        serde_json::from_str(mutated["parts"]["inputs"].as_str().unwrap()).unwrap();
    let transaction = &mut inputs["transactions"][0];
    let amount = transaction["amount"].as_i64().unwrap();
    transaction["amount"] = Value::from(amount + 1_000_000);
    mutated["parts"]["inputs"] = Value::from(serde_json::to_string(&inputs).unwrap());
    let report = kassiber_replay::verify(&reseal(mutated), PASSPHRASE, 5).unwrap();
    assert_eq!(report["status"], "different", "{report:#}");
    assert!(report["differing"].as_u64().unwrap() > 0);

    // A part edited without its manifest digest is corruption, not a diff.
    let mut corrupted = document.clone();
    corrupted["parts"]["outputs"] = Value::from("{\"tables\": []}");
    let sealed =
        kassiber_backup::seal_with_passphrase(&serde_json::to_vec(&corrupted).unwrap(), PASSPHRASE)
            .unwrap();
    let error = kassiber_replay::verify(&sealed, PASSPHRASE, 5).unwrap_err();
    assert_eq!(error.code, "replay_bundle_corrupt");

    // Inputs missing a field the pipeline indexes are corruption, not a
    // crash (checksums updated, so only the content is wrong).
    for field in ["id", "fiat_currency"] {
        let mut malformed = document.clone();
        let mut inputs: Value =
            serde_json::from_str(malformed["parts"]["inputs"].as_str().unwrap()).unwrap();
        inputs["profile"].as_object_mut().unwrap().remove(field);
        malformed["parts"]["inputs"] = Value::from(serde_json::to_string(&inputs).unwrap());
        match kassiber_replay::verify(&reseal(malformed), PASSPHRASE, 5) {
            Err(error) => assert_eq!(error.code, "replay_bundle_corrupt", "{field}"),
            Ok(report) => assert_eq!(report["status"], "refused", "{field}: {report:#}"),
        }
    }

    let error = kassiber_replay::verify(&exported.sealed, "wrong-passphrase", 5).unwrap_err();
    assert_eq!(error.code, "age_decrypt_failed");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn export_refuses_stale_journals_and_short_passphrases() {
    let (root, book) = book("plain-unchanged", "");
    let conn = book.connection();
    let short = kassiber_replay::export(
        conn,
        None,
        None,
        "short",
        "now",
        "0",
        &kassiber_transfers::SystemSources,
    )
    .err()
    .unwrap();
    assert_eq!(short.code, "invalid_passphrase");
    conn.execute_batch("UPDATE profiles SET journal_input_version=journal_input_version+1")
        .unwrap();
    let stale = kassiber_replay::export(
        conn,
        None,
        None,
        PASSPHRASE,
        "now",
        "0",
        &kassiber_transfers::SystemSources,
    )
    .err()
    .unwrap();
    assert_eq!(stale.code, "journals_stale");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn write_new_never_overwrites_and_is_owner_only() {
    let dir = root("write");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("bundle.kassiber-replay");
    kassiber_replay::write_new(&path, b"sealed").unwrap();
    assert_eq!(
        kassiber_replay::write_new(&path, b"other")
            .unwrap_err()
            .code,
        "file_exists"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"sealed");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Pseudonymization keeps the books' numbers: reviewed custody components
/// stay bound and every compared output agrees with the book's own run.
#[test]
fn pseudonymized_runs_reproduce_the_books_own_outputs() {
    for name in [
        "paired-short-processed",
        "plain-blocked-processed",
        "gaps-10-processed",
    ] {
        let (root, book) = book(name, "");
        let exported = export(book.connection());
        let manifest = &exported.manifest;
        assert_eq!(
            manifest["pseudonymization"]["faithful"], true,
            "{name}: {manifest:#}"
        );
        assert_eq!(manifest["local_check"]["status"], "identical", "{name}");
        assert_eq!(
            manifest["stored_check"]["status"], "identical",
            "{name}: {manifest:#}"
        );
        let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 3).unwrap();
        assert_eq!(report["status"], "identical", "{name}: {report:#}");
        drop(book);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// A distinct, letter-bearing 64-hex value per alias.
fn hex(i: u8) -> String {
    format!("c0ffee{i:02x}").repeat(8)
}

/// Review blockers 1–3 and 5: every txid/address/outpoint alias, credentials
/// in embedded documents, authored prose and labels never reach the bundle,
/// while the engine's exact residual markers keep their meaning.
#[test]
fn aliases_credentials_and_authored_text_never_reach_the_bundle() {
    let aliases = [
        "txid",
        "transaction_hash",
        "tx_hash",
        "hash",
        "funding_txid",
        "closing_txid",
        "sweep_txid",
        "payment_hash",
        "preimage",
        "script_pubkey",
    ];
    let mut raw = serde_json::Map::new();
    for (i, alias) in aliases.iter().enumerate() {
        raw.insert((*alias).into(), Value::from(hex(i as u8)));
    }
    raw.insert("outpoint".into(), Value::from(format!("{}:1", hex(20))));
    raw.insert("address".into(), Value::from(ADDRESS));
    raw.insert("password".into(), Value::from("correct-horse"));
    raw.insert("api_key".into(), Value::from("secret_123"));
    raw.insert("memo".into(), Value::from("alice_birthday_gift"));
    let raw = Value::Object(raw).to_string().replace('\'', "''");
    let seed = format!(
        "UPDATE wallets SET label='alice_cold_storage' WHERE id='wallet';
         UPDATE transactions SET raw_json='{raw}', external_id='{}:4', payment_hash='{}', swap_refund_funding_txid='{}', note='rent_for_alice', description='Payment to Bob Builder', counterparty='Carol Example GmbH' WHERE id='a';
         UPDATE custody_components SET change_reason='Alice changed her mind', notes='Bob paid in cash';",
        hex(30),
        hex(31),
        hex(32)
    );
    let (root, book) = book("paired-fee-processed", &seed);
    let exported = export(book.connection());
    let plaintext = String::from_utf8(
        kassiber_backup::open_with_passphrase(&exported.sealed, PASSPHRASE).unwrap(),
    )
    .unwrap();
    let mut secrets: Vec<String> = (0..33u8).map(hex).collect();
    secrets.extend(
        [
            ADDRESS,
            "correct-horse",
            "secret_123",
            "alice_birthday_gift",
            "alice_cold_storage",
            "rent_for_alice",
            "Bob Builder",
            "Carol",
            "Alice changed",
            "Bob paid",
        ]
        .map(str::to_owned),
    );
    for secret in &secrets {
        assert!(
            !plaintext.contains(secret.as_str()),
            "the bundle holds {secret:?}"
        );
    }
    assert_eq!(exported.manifest["pseudonymization"]["faithful"], true);
    let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap();
    assert_eq!(report["status"], "identical", "{report:#}");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// Review finding 4: a wrong stored figure, the thing the user reports, must
/// travel in the bundle; verify shows it next to the reproduction.
#[test]
fn the_stored_discrepancy_travels_in_the_bundle() {
    let (root, book) = book("paired-fee-processed", "");
    let conn = book.connection();
    let (transaction, wallet): (String, String) = conn
        .query_row(
            "SELECT transaction_id,wallet_id FROM journal_entries WHERE cost_basis_exact IS NOT NULL ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    conn.execute(
        "UPDATE journal_entries SET cost_basis_exact='99999.99' WHERE transaction_id=? AND wallet_id=? AND cost_basis_exact IS NOT NULL",
        [&transaction, &wallet],
    )
    .unwrap();
    let exported = export(conn);
    assert_eq!(exported.manifest["local_check"]["status"], "different");
    assert_eq!(exported.manifest["stored_check"]["status"], "different");
    let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap();
    // The bundle reproduces its own computation…
    assert_eq!(report["status"], "identical", "{report:#}");
    // …and shows the stored figure the user saw differing from it.
    let stored = &report["stored"];
    assert_eq!(stored["status"], "different", "{report:#}");
    let difference = &stored["differences"][0];
    assert_eq!(difference["table"], "journal_entries");
    // Identifiers travel as keyed pseudonyms, never as the book's ids.
    assert!(
        difference["transaction_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("id-") && id != transaction),
        "{report:#}"
    );
    assert_eq!(difference["columns"][0]["column"], "cost_basis_exact");
    assert_eq!(difference["columns"][0]["stored"], "99999.99");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// Second review: authored and outside values under fields the pipeline
/// names, unknown codec tags, UUID-shaped and mixed external ids, authored
/// account codes and the word `private` in notes never reach the bundle.
#[test]
fn reviewer_sentinels_never_reach_the_bundle() {
    let txid = hex(40);
    let raw = serde_json::json!({
        "credentials": "SENTINEL_CRED",
        "username": "SENTINEL_USER",
        "in_counterparty": "SENTINEL_IN_CP",
        "out_description": "SENTINEL_OUT_DESC",
        "value": "AT611904300234573201",
        "source_url": "node.private.example:50002",
        "$anything": "SENTINEL_TAG",
        "txid": format!("SENTINEL-ALICE-{txid}"),
    })
    .to_string()
    .replace('\'', "''");
    let seed = format!(
        "INSERT INTO accounts(id,workspace_id,profile_id,code,label,account_type,created_at) VALUES('acct-x','w','p','SENTINEL_ALICE_ACCOUNT','SENTINEL_ACCOUNT_LABEL','asset','2020');
         UPDATE wallets SET account_id='acct-x' WHERE id='wallet';
         UPDATE transactions SET raw_json='{raw}', external_id='5e471e1a-0000-4000-8000-a11ce0000001' WHERE id='a';
         UPDATE transactions SET external_id='SENTINEL-BOB-{txid}' WHERE id='b';
         UPDATE custody_component_legs SET notes='private' WHERE notes IS NULL;
         UPDATE custody_components SET notes='private';"
    );
    let (root, book) = book("paired-fee-processed", &seed);
    let exported = export(book.connection());
    let plaintext = String::from_utf8(
        kassiber_backup::open_with_passphrase(&exported.sealed, PASSPHRASE).unwrap(),
    )
    .unwrap();
    for secret in [
        "SENTINEL",
        "AT611904300234573201",
        "node.private",
        "credentials\\\"",
        "username\\\"",
        "in_counterparty",
        "out_description",
        "source_url",
        "$anything",
        "5e471e1a",
        txid.as_str(),
        "\"private\"",
        "\\\"private\\\"",
    ] {
        assert!(!plaintext.contains(secret), "the bundle holds {secret:?}");
    }
    let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap();
    assert_eq!(report["status"], "identical", "{report:#}");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// Second review, D: reports read the REAL monetary columns too. A stored
/// REAL cost basis that disagrees with its exact text travels as a
/// difference.
#[test]
fn a_tampered_real_cost_basis_travels_as_a_stored_difference() {
    let (root, book) = book("paired-fee-processed", "");
    let conn = book.connection();
    let (id, transaction): (String, String) = conn
        .query_row(
            "SELECT id,transaction_id FROM journal_entries WHERE cost_basis IS NOT NULL ORDER BY id LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    conn.execute(
        "UPDATE journal_entries SET cost_basis=cost_basis+1000 WHERE id=?",
        [&id],
    )
    .unwrap();
    let exported = export(conn);
    assert_eq!(exported.manifest["local_check"]["status"], "different");
    assert_eq!(
        exported.manifest["stored_check"]["status"], "different",
        "{:#}",
        exported.manifest
    );
    let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap();
    let difference = &report["stored"]["differences"][0];
    assert!(
        difference["transaction_id"]
            .as_str()
            .is_some_and(|id| id.starts_with("id-") && id != transaction),
        "{report:#}"
    );
    assert_eq!(difference["columns"][0]["column"], "cost_basis");
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// Third review: values that only looked safe (an id-shaped host, an `ln`
/// prefix, uniform hex, an IPv6 address in a time field) never reach the
/// bundle; every kept value is generated from a parsed type.
#[test]
fn generated_values_only_never_copied_book_bytes() {
    let seed = "UPDATE wallets SET config_json='{\"chain\":\"bitcoin\",\"samourai\":{\"role\":\"postmix\",\"group_id\":\"alice.private.example\",\"section\":\"postmix\"}}' WHERE id='wallet';
         UPDATE transactions SET external_id='lnalicesentinelqqqqqqqqqqqqqqqqqqqqqq' WHERE id='a';
         UPDATE transactions SET external_id='aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa' WHERE id='b';
         UPDATE transactions SET confirmed_at='2001:db8::1' WHERE id='a';
         UPDATE profiles SET require_coarse_review='password=SENTINEL_CREDENTIAL private-sentinel.example:50002 AliceSentinel', bitcoin_rail_carrying_value='SENTINEL_RAIL';";
    let (root, book) = book("paired-fee-processed", seed);
    // Processing stamps its own time; a damaged stamp must not travel either.
    // Written past the profile triggers, which would mark the journals stale.
    let conn = book.connection();
    conn.set_db_config(
        kassiber_store::rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        false,
    )
    .unwrap();
    conn.execute_batch("UPDATE profiles SET last_processed_at='2026-11-12T00:00:00Z SENTINEL_AT'")
        .unwrap();
    conn.set_db_config(
        kassiber_store::rusqlite::config::DbConfig::SQLITE_DBCONFIG_ENABLE_TRIGGER,
        true,
    )
    .unwrap();
    let exported = export(book.connection());
    // The sealed document (manifest included) and the manifest the caller
    // prints.
    let plaintext = String::from_utf8(
        kassiber_backup::open_with_passphrase(&exported.sealed, PASSPHRASE).unwrap(),
    )
    .unwrap()
        + &exported.manifest.to_string();
    for secret in [
        "SENTINEL",
        "private-sentinel",
        "AliceSentinel",
        "password=",
        "alice.private.example",
        "lnalice",
        "alicesentinel",
        &"a".repeat(64),
        "2001:db8::1",
        "db8::",
    ] {
        assert!(!plaintext.contains(secret), "the bundle holds {secret:?}");
    }
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}

/// Third review, 1: stored output cells go through a closed schema too. A
/// stored calculation with an unknown field or a REAL column holding text
/// refuses the export, naming no value.
#[test]
fn unsafe_stored_output_cells_refuse_the_export_without_values() {
    for (column, value) in [
        (
            "calculation_json",
            "{\"schema_version\": 1, \"engine\": \"kassiber_tax\", \"method\": \"fifo\", \"fragments\": [], \"password\": \"SENTINEL_PW\", \"backend\": \"node.private.example\"}",
        ),
        ("cost_basis", "SENTINEL_REAL"),
    ] {
        let (root, book) = book("paired-fee-processed", "");
        let conn = book.connection();
        let id: String = conn
            .query_row("SELECT id FROM journal_entries WHERE cost_basis IS NOT NULL ORDER BY id LIMIT 1", [], |r| r.get(0))
            .unwrap();
        conn.execute(
            &format!("UPDATE journal_entries SET {column}=?, calculation_compact=NULL WHERE id=?"),
            [value, &id],
        )
        .unwrap();
        let error = kassiber_replay::export(
            conn,
            None,
            None,
            PASSPHRASE,
            "2026-11-14T00:00:00+00:00",
            "0.0.0-test",
            &kassiber_transfers::SystemSources,
        )
        .err()
        .unwrap_or_else(|| panic!("{column}: the export accepted an unsafe stored cell"));
        assert_eq!(error.code, "replay_bundle_unsafe_stored", "{column}: {error:?}");
        let text = format!("{error:?}");
        for secret in ["SENTINEL", "node.private"] {
            assert!(!text.contains(secret), "{column}: {text}");
        }
        drop(book);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Fourth review, 3 and 4: a processed book of every registered country,
/// with the registry's default method, pool scope and currency, exports
/// a bundle that reproduces the book (the export's recomputation is the
/// one `journals process` runs) and verifies identical.
#[test]
fn every_registered_country_default_exports_and_reproduces() {
    for country in kassiber_tax_countries::COUNTRIES {
        let fiat = match country.fiat {
            kassiber_tax_countries::Fiat::Fixed(code) => code,
            kassiber_tax_countries::Fiat::Profile => "EUR",
        };
        let seed = format!(
            "UPDATE profiles SET tax_country='{}', gains_algorithm='{}', cost_basis_pool_scope='{}', fiat_currency='{fiat}';",
            country.code, country.default_method, country.default_pool_scope
        );
        let (root, book) = book("paired-fee-processed", &seed);
        let exported = kassiber_replay::export(
            book.connection(),
            None,
            None,
            PASSPHRASE,
            "2026-11-14T00:00:00+00:00",
            "0.0.0-test",
            &kassiber_transfers::SystemSources,
        )
        .unwrap_or_else(|error| panic!("{}: {error:?}", country.code));
        let manifest = &exported.manifest;
        assert_eq!(manifest["tax"]["country"], country.code, "{manifest}");
        assert_eq!(
            manifest["tax"]["gains_algorithm"], country.default_method,
            "{manifest}"
        );
        assert_eq!(
            manifest["stored_check"]["status"], "identical",
            "{}: the bundle's recomputation is not the book's: {manifest}",
            country.code
        );
        let report = kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap();
        assert_eq!(report["status"], "identical", "{}: {report}", country.code);
        drop(book);
        std::fs::remove_dir_all(root).unwrap();
    }
}

/// Fifth review: a `status` held in `raw_json` text keeps both shapes. An
/// explicitly unconfirmed transaction (Esplora's object) still exports
/// faithfully, and a recognized provider status stays itself.
#[test]
fn raw_json_status_survives_as_object_and_as_text() {
    let seed = format!("UPDATE transactions SET external_id='{TXID}', raw_json=json_object('txid', '{TXID}', 'status', json('{{\"confirmed\": false}}')) WHERE id='a';
         UPDATE transactions SET raw_json='{{\"status\": \"transaction.refunded\"}}' WHERE id='b';");
    let (root, book) = book("paired-fee-processed", &seed);
    let exported = kassiber_replay::export(
        book.connection(),
        None,
        None,
        PASSPHRASE,
        "2026-11-14T00:00:00+00:00",
        "0.0.0-test",
        &kassiber_transfers::SystemSources,
    )
    .unwrap_or_else(|error| panic!("an unconfirmed transaction refused the export: {error:?}"));
    assert_eq!(exported.manifest["stored_check"]["status"], "identical");
    let document = open(&exported.sealed);
    let inputs: Value =
        serde_json::from_str(document["parts"]["inputs"].as_str().unwrap()).unwrap();
    let raws: Vec<Value> = inputs["transactions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|row| row["raw_json"].as_str())
        .filter_map(|text| serde_json::from_str(text).ok())
        .collect();
    assert!(
        raws.iter()
            .any(|raw| raw["status"] == serde_json::json!({"confirmed": false})),
        "{raws:?}"
    );
    assert!(
        raws.iter()
            .any(|raw| raw["status"] == "transaction.refunded"),
        "{raws:?}"
    );
    assert_eq!(
        kassiber_replay::verify(&exported.sealed, PASSPHRASE, 5).unwrap()["status"],
        "identical"
    );
    drop(book);
    std::fs::remove_dir_all(root).unwrap();
}
