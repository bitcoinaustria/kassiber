use super::*;

fn book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    let creation: Json =
        serde_json::from_str(include_str!("../../../store/fixtures/creation.json")).unwrap();
    for sql in creation["statements"].as_array().unwrap() {
        conn.execute_batch(sql.as_str().unwrap()).unwrap();
    }
    conn.execute_batch(
        "INSERT INTO workspaces(id,label,created_at) VALUES('ws','Workspace','2025-01-01');
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('p','ws','Book','2025-01-01');
         INSERT INTO profiles(id,workspace_id,label,created_at) VALUES('other','ws','Other','2025-01-01');",
    )
    .unwrap();
    conn
}

fn wallet(conn: &Connection, profile: &str, id: &str, label: &str, kind: &str, config: &str) {
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'ws',?,?,?,?,'2025-01-01')",
        params![id, profile, label, kind, config],
    )
    .unwrap();
}

fn records_only(conn: &Connection, store: &str, method: &str, label: Option<&str>) {
    conn.execute(
        "INSERT INTO btcpay_account_routes(id,workspace_id,profile_id,backend_name,store_id,payment_method_id,action,label,created_at,updated_at) \
         VALUES(?,'ws','p','shop-btcpay',?,?,'provenance_only',?,'2025-01-01','2025-01-01')",
        params![format!("{store}-{method}"), store, method, label],
    )
    .unwrap();
}

/// The owner's shape: one store pays on-chain into the user's multisig, Ark
/// into an imported Bark wallet, and Lightning into a payment ledger; a second
/// store shares the multisig and keeps LNURL as invoice records.
fn shop(conn: &Connection) {
    wallet(
        conn,
        "p",
        "ms",
        "Shop multisig",
        "descriptor",
        r#"{"backend":"fulcrum","btcpay_provenance":[
            {"backend":"Shop-BTCPay","store_id":"S1","payment_method_id":"BTC-CHAIN"},
            {"backend":"shop-btcpay","store_id":"S2","payment_method_id":"BTC-CHAIN"}]}"#,
    );
    wallet(
        conn,
        "p",
        "ark",
        "Shop - Bark store - BTC-ARK",
        "custom",
        r#"{"backend":"shop-btcpay","store_id":"S1","payment_method_id":"BTC-ARK","sync_source":"btcpay"}"#,
    );
    wallet(
        conn,
        "p",
        "ln",
        "Shop - Bark store - Lightning",
        "custom",
        r#"{"backend":"shop-btcpay","store_id":"S1","payment_method_id":"BTC-LN","payment_method_ids":["BTC-LN","BTC-LNURL"],"source_mode":"payments","sync_source":"btcpay"}"#,
    );
    records_only(conn, "S2", "BTC-LNURL", Some("Cafe"));
    // Not a store wallet, and a store wallet of another book.
    wallet(
        conn,
        "p",
        "cold",
        "Cold storage",
        "descriptor",
        r#"{"backend":"fulcrum"}"#,
    );
    wallet(
        conn,
        "other",
        "x",
        "Other - Elsewhere - BTC-CHAIN",
        "custom",
        r#"{"backend":"shop-btcpay","store_id":"S9","payment_method_id":"BTC-CHAIN","sync_source":"btcpay"}"#,
    );
}

#[test]
fn a_store_groups_the_wallets_it_pays_into_by_payment_method() {
    let conn = book();
    shop(&conn);
    let groups = groups(&conn, "p").unwrap();
    assert_eq!(groups.len(), 2);
    let bark = &groups[0];
    assert_eq!(bark.label.as_deref(), Some("Bark store"));
    assert_eq!(bark.label_source, Some("wallet_label"));
    assert_eq!(bark.key(), "btcpay:shop-btcpay:S1");
    let members: Vec<(&str, Role, Vec<&str>, usize)> = bark
        .wallets
        .iter()
        .map(|m| {
            (
                m.wallet_id.as_str(),
                m.role,
                m.payment_methods.iter().map(String::as_str).collect(),
                m.store_count,
            )
        })
        .collect();
    assert_eq!(
        members,
        vec![
            ("ms", Role::Mapped, vec!["BTC-CHAIN"], 2),
            ("ln", Role::PaymentLedger, vec!["BTC-LN", "BTC-LNURL"], 1),
            ("ark", Role::WalletSource, vec!["BTC-ARK"], 1),
        ]
    );
    assert!(bark.records_only.is_empty());
    let cafe = &groups[1];
    assert_eq!(cafe.label.as_deref(), Some("Cafe"));
    assert_eq!(cafe.label_source, Some("route"));
    assert_eq!(cafe.wallets.len(), 1);
    assert_eq!(cafe.wallets[0].wallet_id, "ms");
    assert_eq!(cafe.records_only, vec!["BTC-LNURL".to_owned()]);
}

#[test]
fn a_method_a_wallet_books_is_not_also_records_only() {
    let conn = book();
    shop(&conn);
    records_only(&conn, "S1", "BTC-LN", None);
    let groups = groups(&conn, "p").unwrap();
    assert!(groups[0].records_only.is_empty());
}

#[test]
fn a_renamed_store_wallet_leaves_the_store_unnamed_and_last() {
    let conn = book();
    wallet(
        &conn,
        "p",
        "a",
        "My Ark",
        "custom",
        r#"{"backend":"b","store_id":"Z","payment_method_id":"BTC-ARK","sync_source":"btcpay"}"#,
    );
    shop(&conn);
    let groups = groups(&conn, "p").unwrap();
    let last = groups.last().unwrap();
    assert_eq!((last.store_id.as_str(), last.label.as_deref()), ("Z", None));
    assert_eq!(last.label_source, None);
}

#[test]
fn store_names_come_only_from_names_setup_generated() {
    let ids = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        store_name_from_label("Shop - Main store - BTC-CHAIN", &ids(&["BTC-CHAIN"])).as_deref(),
        Some("Main store")
    );
    assert_eq!(
        store_name_from_label("Shop - Main - Lightning", &ids(&["BTC-LN"])).as_deref(),
        Some("Main")
    );
    assert_eq!(
        store_name_from_label("Shop - Main - Liquid Bitcoin", &ids(&["LBTC-CHAIN"])).as_deref(),
        Some("Main")
    );
    // Renamed, or a suffix of another method.
    assert_eq!(
        store_name_from_label("Main store", &ids(&["BTC-CHAIN"])),
        None
    );
    assert_eq!(
        store_name_from_label("Shop - Main - BTC-ARK", &ids(&["BTC-CHAIN"])),
        None
    );
}

#[test]
fn the_report_lists_methods_with_their_rails() {
    let conn = book();
    shop(&conn);
    let report = report(&conn, "p").unwrap();
    let json: Json = serde_json::from_str(
        &kassiber_pyjson::dumps(&report, &kassiber_pyjson::Options::new()).unwrap(),
    )
    .unwrap();
    assert_eq!(json["summary"]["groups"], 2);
    assert_eq!(json["summary"]["backends"], 1);
    assert_eq!(json["summary"]["wallets"], 3);
    let ledger = &json["groups"][0]["wallets"][1];
    assert_eq!(ledger["role"], "payment_ledger");
    assert_eq!(ledger["payment_methods"][0]["rail"], "lightning");
    assert_eq!(ledger["payment_methods"][1]["settlement"], "lnurl");
    assert_eq!(
        json["groups"][0]["wallets"][2]["payment_methods"][0]["settlement"],
        "bark"
    );
    assert_eq!(json["groups"][1]["records_only"][0]["id"], "BTC-LNURL");
}
