//! End-to-end BTCPay Bark -> on-chain multisig transfer flow through SQLite,
//! journal processing, custody lineage, transfers list, dashboard totals,
//! quarantine restore_pairing, and non-authoritative transfer suggestions.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(tag: &str) -> Self {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "kassiber-bark-settlement-{tag}-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

use kassiber_custody::evidence;
use kassiber_envelope::{dict, get, py_str, s};
use kassiber_journals::{
    process::{self, Prebuild, Repairs},
    views, Refusal,
};
use kassiber_pyjson::Value as Py;
use kassiber_pytext::zone::Zone;
use kassiber_store::rusqlite::{params, Connection};
use kassiber_transactions::{desktop, Scope};
use kassiber_transfers::pairing::{suggest, MatchOptions, Population};
use serde_json::{json, Value};

const MULTISIG_ADDRESS: &str = "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3";
const FOREIGN_SCRIPT: &str = "0014ffffffffffffffffffffffffffffffffffffffff";
const INPUT_SCRIPT: &str = "0014eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const FOREIGN_INPUT_TXID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TXID_SEND: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const TXID_OFFBOARD: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const TXID_ONCHAIN: &str = "3333333333333333333333333333333333333333333333333333333333333333";
const TXID_UNRELATED: &str = "4444444444444444444444444444444444444444444444444444444444444444";

struct TestPrebuild {
    profile: Value,
}

impl Prebuild for TestPrebuild {
    fn resolve(&mut self, _: &Connection) -> Result<Value, Refusal> {
        Ok(self.profile.clone())
    }
    fn repair(&mut self, _: &Connection, _: &mut Value) -> Result<Repairs, Refusal> {
        Ok(Repairs::default())
    }
    fn price(&mut self, _: &Connection, _: &Value) -> Result<usize, Refusal> {
        Ok(0)
    }
}

fn open_book() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    let manifest: Value =
        serde_json::from_str(include_str!("../../store/fixtures/current-schema.json")).unwrap();
    for kind in ["table", "index", "view", "trigger"] {
        for object in manifest["objects"].as_array().unwrap() {
            if object[0].as_str() == Some(kind) {
                conn.execute_batch(object[3].as_str().unwrap()).unwrap();
            }
        }
    }
    conn.execute_batch(
        "INSERT OR IGNORE INTO chain_index_clock(id, revision) VALUES(1, 0);
         INSERT INTO workspaces(id,label,created_at) VALUES('w','Shop','2026-01-01T00:00:00Z');
         INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Main','EUR','generic','2026-01-01T00:00:00Z');",
    )
    .unwrap();
    kassiber_setup::context::execute(
        &conn,
        "context.set",
        &dict([("workspace", s("w")), ("profile", s("p"))]),
    )
    .unwrap();
    conn
}

fn insert_wallet(conn: &Connection, id: &str, label: &str, kind: &str, config: Value) {
    conn.execute(
        "INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,?,?,'2026-01-01T00:00:00Z')",
        params![id, label, kind, config.to_string()],
    )
    .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn insert_tx(
    conn: &Connection,
    id: &str,
    wallet_id: &str,
    external_id: &str,
    external_id_kind: Option<&str>,
    occurred_at: &str,
    direction: &str,
    amount_msat: i64,
    fee_msat: i64,
    kind: &str,
    raw_json: &str,
    fiat_rate: f64,
) {
    let fiat_value = (amount_msat as f64 / 100_000_000_000.0) * fiat_rate;
    conn.execute(
        "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,external_id_kind,fingerprint,occurred_at,confirmed_at,direction,asset,amount,fee,amount_includes_fee,fiat_currency,fiat_rate,fiat_value,fiat_price_source,fiat_rate_exact,fiat_value_exact,kind,raw_json,created_at) VALUES(?,'w','p',?,?,?,?,?,?,?,'BTC',?,?,0,'EUR',?,?,'manual',?,?,?,?,'2026-01-01T00:00:00Z')",
        params![
            id,
            wallet_id,
            external_id,
            external_id_kind,
            format!("fp-{id}"),
            occurred_at,
            occurred_at,
            direction,
            amount_msat,
            fee_msat,
            fiat_rate,
            fiat_value,
            format!("{fiat_rate}"),
            format!("{fiat_value:.8}"),
            kind,
            raw_json,
        ],
    )
    .unwrap();
}

fn stamp_authoritative_receipt(
    conn: &Connection,
    id: &str,
    wallet_id: &str,
    txid: &str,
    amount_msat: i64,
    raw_json: &str,
) {
    let row = json!({
        "wallet_id": wallet_id,
        "external_id": txid,
        "direction": "inbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": 0,
        "amount_includes_fee": false,
        "raw_json": raw_json,
    });
    let graph_hash = evidence::graph_hash(&row["raw_json"]).unwrap().0;
    let quantity_hash = evidence::observed_hash(&row).unwrap().0;
    conn.execute(
        "INSERT INTO chain_observation_provenance(transaction_id,workspace_id,profile_id,wallet_id,authority_version,observer_ids_json,observer_kinds_json,chain,network,application_revision,graph_hash,quantity_hash,fee_attribution,observed_at,updated_at) VALUES(?,'w','p',?,1,'[\"obs-1\"]','[\"esplora\"]','bitcoin','main','test-rev',?,?,'exact','2026-11-13T12:00:00Z','2026-11-13T12:00:00Z')",
        params![id, wallet_id, graph_hash, quantity_hash],
    )
    .unwrap();
}

fn bark_raw_with_provenance(
    kind: &str,
    row_id: &str,
    raw_amount_msat: i64,
    fee_msat: i64,
    tx_hash: Option<&str>,
    with_sync_provenance: bool,
) -> String {
    let mut obj = json!({
        "amount_msat": raw_amount_msat,
        "bark_payment_hash": null,
        "bark_row_id": row_id,
        "fee_msat": fee_msat,
        "invoice_id": null,
        "kind": kind,
        "movement_id": 1,
        "outpoint": null,
        "part": "principal",
        "source": "btcpay_bark_wallet",
        "store_id": "shop",
        "transaction_hash": tx_hash,
    });
    if with_sync_provenance && matches!(kind, "send-onchain" | "offboard" | "onchain-send") {
        obj["_kassiber_provenance"] = json!({"import_source": "btcpay-bark"});
    }
    obj.to_string()
}

fn bark_raw(
    kind: &str,
    row_id: &str,
    raw_amount_msat: i64,
    fee_msat: i64,
    tx_hash: Option<&str>,
) -> String {
    bark_raw_with_provenance(kind, row_id, raw_amount_msat, fee_msat, tx_hash, true)
}

fn receipt_raw(txid: &str, input_vout: i64, value_sats: i64, owned_script: &str) -> String {
    json!({
        "txid": txid,
        "status": {"confirmed": true, "block_height": 840000},
        "vin": [{
            "txid": FOREIGN_INPUT_TXID,
            "vout": input_vout,
            "prevout": {"scriptpubkey": INPUT_SCRIPT, "value": value_sats + 50_000}
        }],
        "vout": [
            {"scriptpubkey": FOREIGN_SCRIPT, "value": 40_000},
            {"scriptpubkey": owned_script, "value": value_sats}
        ]
    })
    .to_string()
}

fn run_journals(conn: &Connection) {
    let mut seq = 0u64;
    let mut prebuild = TestPrebuild {
        profile: json!({
            "id": "p",
            "workspace_id": "w",
            "label": "Main",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
            "require_coarse_review": 0,
            "bitcoin_rail_carrying_value": 1,
            "cost_basis_pool_scope": "global",
            "journal_input_version": 1,
        }),
    };
    let mut next_id = || {
        seq += 1;
        Ok(format!("id-{seq:06}"))
    };
    process::process(
        conn,
        &mut prebuild,
        "2026-11-13T12:15:31Z",
        &mut next_id,
        &mut |_| {},
        &mut (),
    )
    .unwrap();
}

fn py_list<'a>(value: &'a Py, key: &str) -> &'a [Py] {
    match get(value, key) {
        Some(Py::List(items)) => items,
        other => panic!("expected list for {key}, got {other:?}"),
    }
}

#[test]
fn bark_lightning_sweeps_to_multisig_book_automatically_without_double_counting() {
    let conn = open_book();
    let owned_script = kassiber_wallets::config::bitcoin_address_script(MULTISIG_ADDRESS)
        .expect("P2WSH multisig script");
    insert_wallet(
        &conn,
        "w-bark",
        "BTCPay Bark",
        "custom",
        json!({
            "sync_source": "btcpay",
            "backend": "merchant",
            "store_id": "shop",
            "payment_method_id": "BTC-ARK",
            "chain": "bitcoin",
            "network": "main",
        }),
    );
    insert_wallet(
        &conn,
        "w-multi",
        "Store multisig",
        "address",
        json!({
            "chain": "bitcoin",
            "network": "main",
            "addresses": [MULTISIG_ADDRESS],
        }),
    );

    // 1. Lightning receive into Bark: 100,000 sat (100,000,000 msat).
    insert_tx(
        &conn,
        "ln-in",
        "w-bark",
        "btcpay-bark:shop:ln-1",
        None,
        "2026-11-10T10:00:00Z",
        "inbound",
        100_000_000,
        0,
        "income",
        &bark_raw("lightning-receive", "ln-1", 100_000_000, 0, None),
        80_000.0,
    );

    // 2. Three on-chain sweeps from Bark to the store's separate multisig wallet:
    //    send-onchain (30,000 sat + 2,802 fee), offboard (14,387 sat + 2,768 fee),
    //    onchain-send (20,000 sat + 201 fee).
    let sweeps = [
        (
            "send",
            "send-onchain",
            TXID_SEND,
            30_000_000,
            2_802_000,
            "2026-11-11T10:00:00Z",
            "2026-11-11T10:01:00Z",
        ),
        (
            "offb",
            "offboard",
            TXID_OFFBOARD,
            14_387_000,
            2_768_000,
            "2026-11-12T10:00:00Z",
            "2026-11-12T10:01:00Z",
        ),
        (
            "onch",
            "onchain-send",
            TXID_ONCHAIN,
            20_000_000,
            201_000,
            "2026-11-13T10:00:00Z",
            "2026-11-13T10:01:00Z",
        ),
    ];
    for (index, (tag, bark_kind, txid, principal, fee, out_at, in_at)) in
        sweeps.into_iter().enumerate()
    {
        let out_id = format!("out-{tag}");
        let in_id = format!("in-{tag}");
        insert_tx(
            &conn,
            &out_id,
            "w-bark",
            &format!("btcpay-bark:shop:{tag}"),
            None,
            out_at,
            "outbound",
            principal,
            fee,
            "withdrawal",
            &bark_raw(bark_kind, tag, -(principal + fee), fee, Some(txid)),
            80_000.0,
        );
        let raw = receipt_raw(txid, index as i64, principal / 1000, &owned_script);
        insert_tx(
            &conn,
            &in_id,
            "w-multi",
            txid,
            Some("txid"),
            in_at,
            "inbound",
            principal,
            0,
            "deposit",
            &raw,
            80_000.0,
        );
        stamp_authoritative_receipt(&conn, &in_id, "w-multi", txid, principal, &raw);
    }

    run_journals(&conn);

    // Zero quarantines.
    let mut stmt = conn
        .prepare("SELECT transaction_id, reason, detail_json FROM journal_quarantines ORDER BY transaction_id, reason")
        .unwrap();
    let held_rows: Vec<(String, String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(
        held_rows.is_empty(),
        "exact Bark sweeps must leave zero quarantines, got: {held_rows:#?}"
    );

    // All three sweeps appear in ui.custody.lineage.snapshot (Transfers -> Linked).
    let scope = Scope {
        workspace_id: "w",
        profile_id: "p",
    };
    let lineage = desktop::execute(
        &conn,
        &scope,
        desktop::Projection::CustodyLineage,
        Some(&dict([("priced", Py::Bool(true))])),
        &|_, _| Ok(Py::Null),
    )
    .unwrap();
    let items = py_list(&lineage, "items");
    assert_eq!(
        items.len(),
        3,
        "all three Bark sweeps must appear in Transfers -> Linked: {lineage:?}"
    );
    for item in items {
        assert_eq!(get(item, "custody_state"), Some(&s("internal_verified")));
        assert_eq!(get(item, "evidence_reason"), Some(&s("bark_txid_exact")));
        assert_eq!(get(item, "from_wallet_label"), Some(&s("BTCPay Bark")));
        assert_eq!(get(item, "to_wallet_label"), Some(&s("Store multisig")));
    }

    // All three sweeps appear in ui.journals.transfers.list.
    let transfers = views::execute(&conn, "ui.journals.transfers.list", None).unwrap();
    assert_eq!(py_list(&transfers, "pairs").len(), 3);

    // Transactions dashboard counts incoming ONCE (0.001 BTC = 100,000 sat),
    // never double-counting the 64,387 sats swept to multisig, and lists all 3 as transfers.
    let utc = |_: &str| Ok(Zone::utc("UTC"));
    let book_utc = |_: Option<&str>| Ok(Zone::utc("UTC"));
    let zones = desktop::Zones {
        display: &utc,
        book: &book_utc,
    };
    let dashboard = desktop::dashboard(
        &conn,
        &scope,
        &dict([("period", s("all")), ("timezone", s("UTC"))]),
        &[],
        &zones,
    )
    .unwrap();
    let summary = get(&dashboard, "summary").unwrap();
    let incoming = get(summary, "incoming").unwrap();
    let transfers_seg = get(summary, "transfers").unwrap();
    let outgoing = get(summary, "outgoing").unwrap();
    assert_eq!(
        get(incoming, "count"),
        Some(&Py::Int(1.into())),
        "{dashboard:?}"
    );
    let incoming_btc = match get(incoming, "btc") {
        Some(Py::Float(v)) => *v,
        other => panic!("incoming btc: {other:?}"),
    };
    assert!(
        (incoming_btc - 0.001).abs() < 1e-12,
        "incoming must count only the 100,000 sat Lightning receive, not the multisig sweeps: {incoming_btc}"
    );
    assert_eq!(get(transfers_seg, "count"), Some(&Py::Int(3.into())));
    assert_eq!(get(outgoing, "count"), Some(&Py::Int(0.into())));

    // Overview rows fold the 7 raw rows (1 LN receive + 3 Bark sends + 3 multisig receipts)
    // into 4 activity rows (1 income + 3 Transfer rows).
    let overview = desktop::overview_rows(&conn, &scope, desktop::OverviewRows::Recent).unwrap();
    assert_eq!(
        overview.len(),
        4,
        "overview must fold each Bark transfer pair into one row"
    );
    assert_eq!(
        overview
            .iter()
            .filter(|row| get(row, "type") == Some(&s("Transfer")))
            .count(),
        3
    );

    // Bark transfer fees booked as transfer_fee journal entries.
    let fee_sum: i64 = conn
        .query_row(
            "SELECT COALESCE(SUM(ABS(quantity)), 0) FROM journal_entries WHERE entry_type = 'transfer_fee'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(fee_sum, 2_802_000 + 2_768_000 + 201_000);

    // Undoing (dismissing) one Bark automatic pairing reverts it to
    // recorded_transfer_authority_conflict with restore_pairing first.
    conn.execute(
        "INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,reason,created_at,expires_at) VALUES('d-undo','w','p','out-send','in-send','Automatic pairing undone from custody lineage','2026-11-13T12:15:31Z','2036-11-10T12:15:31Z')",
        [],
    )
    .unwrap();
    run_journals(&conn);

    let quarantine = views::execute(&conn, "ui.journals.quarantine", None).unwrap();
    let q_summary = get(&quarantine, "summary").unwrap();
    assert_eq!(get(q_summary, "attention_count"), Some(&Py::Int(1.into())));
    assert_eq!(get(q_summary, "waiting_count"), Some(&Py::Int(5.into())));
    let held = py_list(&quarantine, "items");
    let root = held
        .iter()
        .find(|item| get(item, "transaction_id") == Some(&s("out-send")))
        .expect("dismissed Bark sweep should hold out-send");
    assert_eq!(
        get(root, "reason"),
        Some(&s("recorded_transfer_authority_conflict"))
    );
    let first_action = &py_list(root, "actions")[0];
    assert_eq!(get(first_action, "kind"), Some(&s("restore_pairing")));
    assert_eq!(get(first_action, "dismissal_id"), Some(&s("d-undo")));
    let receipt_hold = held
        .iter()
        .find(|item| get(item, "transaction_id") == Some(&s("in-send")))
        .expect("dismissed Bark sweep must also hold in-send");
    assert_eq!(
        get(receipt_hold, "reason"),
        Some(&s("transfer_pair_dependency_blocked"))
    );
    // Neither leg of the dismissed Bark sweep may book a standalone journal entry.
    let dismissed_entries: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_entries WHERE transaction_id IN ('out-send', 'in-send')",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        dismissed_entries, 0,
        "dismissed Bark sweep must not book a standalone acquisition or disposal"
    );

    // Restoring (deleting dismissal) re-books all 3 bark_txid_exact moves.
    conn.execute(
        "DELETE FROM transaction_pair_dismissals WHERE id = 'd-undo'",
        [],
    )
    .unwrap();
    run_journals(&conn);
    let lineage_after = desktop::execute(
        &conn,
        &scope,
        desktop::Projection::CustodyLineage,
        None,
        &|_, _| Ok(Py::Null),
    )
    .unwrap();
    assert_eq!(py_list(&lineage_after, "items").len(), 3);
}

#[test]
fn bark_non_authoritative_sweep_populates_scope_in_matcher_and_rejects_mismatched_txid() {
    let rows = kassiber_pytext::loads(&json!([
        {
            "id": "out-bark",
            "wallet_id": "w-bark",
            "wallet_label": "BTCPay Bark",
            "wallet_kind": "custom",
            "config_json": json!({"sync_source": "btcpay", "payment_method_id": "BTC-ARK", "chain": "bitcoin", "network": "main"}).to_string(),
            "external_id": "btcpay-bark:shop:14-1",
            "direction": "outbound",
            "asset": "BTC",
            "amount": 30_000_000,
            "fee": 2_802_000,
            "amount_includes_fee": false,
            "occurred_at": "2026-11-11T10:00:00Z",
            "raw_json": bark_raw("send-onchain", "14-1", -32_802_000, 2_802_000, Some(TXID_SEND)),
        },
        {
            "id": "in-btcpay-match",
            "wallet_id": "w-chain",
            "wallet_label": "BTCPay BTC-CHAIN",
            "wallet_kind": "custom",
            "config_json": json!({"sync_source": "btcpay", "payment_method_id": "BTC-CHAIN", "chain": "bitcoin", "network": "main"}).to_string(),
            "external_id": TXID_SEND,
            "external_id_kind": "txid",
            "direction": "inbound",
            "asset": "BTC",
            "amount": 30_000_000,
            "fee": 0,
            "amount_includes_fee": false,
            "occurred_at": "2026-11-11T10:05:00Z",
            "raw_json": json!({"TransactionId": TXID_SEND, "Amount": "0.0003"}).to_string(),
        },
        {
            "id": "in-btcpay-other-tx",
            "wallet_id": "w-chain",
            "wallet_label": "BTCPay BTC-CHAIN",
            "wallet_kind": "custom",
            "config_json": json!({"sync_source": "btcpay", "payment_method_id": "BTC-CHAIN", "chain": "bitcoin", "network": "main"}).to_string(),
            "external_id": TXID_UNRELATED,
            "external_id_kind": "txid",
            "direction": "inbound",
            "asset": "BTC",
            "amount": 30_000_000,
            "fee": 0,
            "amount_includes_fee": false,
            "occurred_at": "2026-11-11T10:02:00Z",
            "raw_json": json!({"TransactionId": TXID_UNRELATED, "Amount": "0.0003"}).to_string(),
        },
    ]).to_string()).unwrap();
    let Py::List(rows) = rows else { panic!("rows") };
    let pop = Population::new(&rows).unwrap();
    let options = MatchOptions {
        now: "2026-11-11T12:00:00Z".into(),
        ..MatchOptions::default()
    };
    let candidates = suggest(&pop, &[], &[], &HashSet::new(), &options).unwrap();
    assert_eq!(
        candidates.len(),
        1,
        "Bark send must match only the same-txid receipt and reject the unrelated txid: {:?}",
        candidates.iter().map(|c| c.key()).collect::<Vec<_>>()
    );
    assert_eq!(
        py_str(get(&candidates[0].data, "out_id").unwrap()),
        "out-bark"
    );
    assert_eq!(
        py_str(get(&candidates[0].data, "in_id").unwrap()),
        "in-btcpay-match"
    );
}

#[test]
fn generic_file_import_cannot_claim_bark_authority_even_into_bark_wallet() {
    let owned_script = kassiber_wallets::config::bitcoin_address_script(MULTISIG_ADDRESS)
        .expect("P2WSH multisig script");
    for (label, wallet_config) in [
        (
            "Plain custom",
            json!({"chain": "bitcoin", "network": "main"}),
        ),
        (
            "BTCPay Bark",
            json!({
                "sync_source": "btcpay",
                "backend": "merchant",
                "store_id": "shop",
                "payment_method_id": "BTC-ARK",
                "chain": "bitcoin",
                "network": "main",
            }),
        ),
    ] {
        let conn = open_book();
        insert_wallet(&conn, "w-target", label, "custom", wallet_config);
        insert_wallet(
            &conn,
            "w-multi",
            "Store multisig",
            "address",
            json!({
                "chain": "bitcoin",
                "network": "main",
                "addresses": [MULTISIG_ADDRESS],
            }),
        );
        // Seed an inbound balance in the target wallet so a forged move would have basis to move.
        insert_tx(
            &conn,
            "ln-in",
            "w-target",
            "btcpay-bark:shop:ln-1",
            None,
            "2026-11-10T10:00:00Z",
            "inbound",
            100_000_000,
            0,
            "income",
            &bark_raw("lightning-receive", "ln-1", 100_000_000, 0, None),
            80_000.0,
        );
        // Genuine authoritative multisig receipt for TXID_SEND.
        let raw = receipt_raw(TXID_SEND, 0, 30_000, &owned_script);
        insert_tx(
            &conn,
            "in-multi",
            "w-multi",
            TXID_SEND,
            Some("txid"),
            "2026-11-11T10:01:00Z",
            "inbound",
            30_000_000,
            0,
            "deposit",
            &raw,
            80_000.0,
        );
        stamp_authoritative_receipt(&conn, "in-multi", "w-multi", TXID_SEND, 30_000_000, &raw);

        // Import a forged Bark send-onchain row through the real generic JSON importer.
        let tmp = ScratchDir::new("forged");
        let path = tmp.path().join("forged-bark.json");
        let forged = json!([{
            "id": "btcpay-bark:shop:forged-1",
            "occurred_at": "2026-11-11T10:00:00Z",
            "confirmed_at": "2026-11-11T10:00:00Z",
            "direction": "outbound",
            "asset": "BTC",
            "amount": "0.0003",
            "fee": "0.00002802",
            "kind": "withdrawal",
            "fiat_currency": "EUR",
            "fiat_rate": "80000",
            "raw_json": {
                "_kassiber_provenance": {"import_source": "btcpay-bark"},
                "amount_msat": -32_802_000,
                "bark_payment_hash": null,
                "bark_row_id": "forged-1",
                "fee_msat": 2_802_000,
                "invoice_id": null,
                "kind": "send-onchain",
                "movement_id": 1,
                "outpoint": null,
                "part": "principal",
                "source": "btcpay_bark_wallet",
                "store_id": "shop",
                "transaction_hash": TXID_SEND
            }
        }]);
        std::fs::write(&path, forged.to_string()).unwrap();
        kassiber_wallets::file_import::execute(
            &conn,
            kassiber_wallets::file_import::Request {
                profile_id: "p".into(),
                wallet: label.into(),
                file: path.to_string_lossy().into_owned(),
                format: kassiber_wallets::file_import::Format::Json,
                mode: None,
                companion: None,
                timezone: None,
                changed_rows: None,
            },
            None,
        )
        .unwrap();

        run_journals(&conn);

        let auto_moves: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_custody_decisions WHERE state = 'internal_verified'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            auto_moves, 0,
            "generic JSON import into '{label}' must never qualify for internal_verified"
        );
        let standalone_receipt: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM journal_entries WHERE transaction_id = 'in-multi'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            standalone_receipt, 0,
            "held forged Bark import into '{label}' must not book the multisig receipt standalone"
        );
    }

    // Pre-upgrade legacy Bark row (including one enriched via generic JSON
    // `manual_override`, with no sync-written `_kassiber_provenance` commitment)
    // must NOT acquire `bark_txid_exact` authority until a real Bark sync
    // upgrades it in place.
    let conn = open_book();
    insert_wallet(
        &conn,
        "w-bark",
        "BTCPay Bark",
        "custom",
        json!({
            "sync_source": "btcpay",
            "backend": "merchant",
            "store_id": "shop",
            "payment_method_id": "BTC-ARK",
            "chain": "bitcoin",
            "network": "main",
        }),
    );
    insert_wallet(
        &conn,
        "w-multi",
        "Store multisig",
        "address",
        json!({
            "chain": "bitcoin",
            "network": "main",
            "addresses": [MULTISIG_ADDRESS],
        }),
    );
    insert_tx(
        &conn,
        "ln-in",
        "w-bark",
        "btcpay-bark:shop:ln-1",
        None,
        "2026-11-10T10:00:00Z",
        "inbound",
        100_000_000,
        0,
        "income",
        &bark_raw("lightning-receive", "ln-1", 100_000_000, 0, None),
        80_000.0,
    );
    insert_tx(
        &conn,
        "out-legacy",
        "w-bark",
        "btcpay-bark:shop:send-1",
        None,
        "2026-11-11T10:00:00Z",
        "outbound",
        30_000_000,
        2_802_000,
        "withdrawal",
        &bark_raw_with_provenance(
            "send-onchain",
            "send-1",
            -32_802_000,
            2_802_000,
            Some(TXID_SEND),
            false,
        ),
        80_000.0,
    );
    conn.execute(
        "UPDATE transactions SET pricing_source_kind = 'manual_override', fiat_price_source = 'manual' WHERE id = 'out-legacy'",
        [],
    )
    .unwrap();
    let raw = receipt_raw(TXID_SEND, 0, 30_000, &owned_script);
    insert_tx(
        &conn,
        "in-multi",
        "w-multi",
        TXID_SEND,
        Some("txid"),
        "2026-11-11T10:01:00Z",
        "inbound",
        30_000_000,
        0,
        "deposit",
        &raw,
        80_000.0,
    );
    stamp_authoritative_receipt(&conn, "in-multi", "w-multi", TXID_SEND, 30_000_000, &raw);
    run_journals(&conn);
    let legacy_auto: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_custody_decisions WHERE state = 'internal_verified'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        legacy_auto, 0,
        "pre-upgrade legacy Bark row without sync-written _kassiber_provenance must not promote"
    );
    let legacy_reason: String = conn
        .query_row(
            "SELECT reason FROM journal_quarantines WHERE transaction_id = 'out-legacy'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(legacy_reason, "recorded_transfer_authority_conflict");

    // Once Bark sync writes the `_kassiber_provenance` commitment onto the
    // existing row, rebuilding promotes it to `bark_txid_exact`.
    conn.execute(
        "UPDATE transactions SET raw_json = ?1 WHERE id = 'out-legacy'",
        [bark_raw(
            "send-onchain",
            "send-1",
            -32_802_000,
            2_802_000,
            Some(TXID_SEND),
        )],
    )
    .unwrap();
    run_journals(&conn);
    let resynced_auto: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM journal_custody_decisions WHERE state = 'internal_verified' AND reason = 'bark_txid_exact'",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(
        resynced_auto, 1,
        "a Bark row carrying sync-written _kassiber_provenance must promote to bark_txid_exact"
    );
}

#[test]
fn upgrade_invalidates_existing_processed_bark_book_and_restores_report_readiness_on_rebuild() {
    let root = ScratchDir::new("upgrade");
    let owned_script = kassiber_wallets::config::bitcoin_address_script(MULTISIG_ADDRESS)
        .expect("P2WSH multisig script");
    {
        let book = kassiber_store::Book::create(root.path(), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Shop','2026-01-01T00:00:00Z');
             INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Main','EUR','generic','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        kassiber_setup::context::execute(
            conn,
            "context.set",
            &dict([("workspace", s("w")), ("profile", s("p"))]),
        )
        .unwrap();
        insert_wallet(
            conn,
            "w-bark",
            "BTCPay Bark",
            "custom",
            json!({
                "sync_source": "btcpay",
                "backend": "merchant",
                "store_id": "shop",
                "payment_method_id": "BTC-ARK",
                "chain": "bitcoin",
                "network": "main",
            }),
        );
        insert_wallet(
            conn,
            "w-multi",
            "Store multisig",
            "address",
            json!({
                "chain": "bitcoin",
                "network": "main",
                "addresses": [MULTISIG_ADDRESS],
            }),
        );
        insert_tx(
            conn,
            "ln-in",
            "w-bark",
            "btcpay-bark:shop:ln-1",
            None,
            "2026-11-10T10:00:00Z",
            "inbound",
            100_000_000,
            0,
            "income",
            &bark_raw("lightning-receive", "ln-1", 100_000_000, 0, None),
            80_000.0,
        );
        insert_tx(
            conn,
            "out-send",
            "w-bark",
            "btcpay-bark:shop:send",
            None,
            "2026-11-11T10:00:00Z",
            "outbound",
            30_000_000,
            2_802_000,
            "withdrawal",
            &bark_raw(
                "send-onchain",
                "send",
                -32_802_000,
                2_802_000,
                Some(TXID_SEND),
            ),
            80_000.0,
        );
        let raw = receipt_raw(TXID_SEND, 0, 30_000, &owned_script);
        insert_tx(
            conn,
            "in-send",
            "w-multi",
            TXID_SEND,
            Some("txid"),
            "2026-11-11T10:01:00Z",
            "inbound",
            30_000_000,
            0,
            "deposit",
            &raw,
            80_000.0,
        );
        stamp_authoritative_receipt(conn, "in-send", "w-multi", TXID_SEND, 30_000_000, &raw);

        // Simulate a book already processed on an older build at the current
        // semantics version, with double-counted acquisitions and no holds.
        let sem = kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION;
        conn.execute(
            "UPDATE profiles SET last_processed_at = '2026-11-12T00:00:00Z', \
             last_processed_tx_count = 3, journal_input_version = 1, \
             last_processed_input_version = 1, journal_semantics_version = ?1, \
             required_journal_semantics_version = ?1 WHERE id = 'p'",
            [sem],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO journal_entries(id,workspace_id,profile_id,wallet_id,transaction_id,entry_type,occurred_at,asset,quantity,created_at) \
             VALUES('old-dup','w','p','w-multi','in-send','acquisition','2026-11-11T10:01:00Z','BTC',30000000,'2026-11-12T00:00:00Z')",
            [],
        )
        .unwrap();
    }

    let blockers_snapshot = |conn: &Connection| {
        let (context, active) = kassiber_workspace::readiness::scope(conn, None).unwrap();
        let health =
            kassiber_workspace::readiness::health(conn, &context, active.as_ref()).unwrap();
        let coverage = kassiber_journals::report_blockers::rates_coverage(
            conn,
            &dict([("limit", Py::Int(10.into()))]),
        )
        .unwrap();
        (
            health.clone(),
            kassiber_journals::report_blockers::snapshot(conn, health, coverage).unwrap(),
        )
    };

    // Production open (`Book::open_migrating`) runs compat_ordered -> bark_onchain_settlement
    // and invalidates the affected processed book once.
    {
        let book = kassiber_store::Book::open_migrating(
            root.path(),
            kassiber_store::OpenOptions::default(),
            true,
        )
        .unwrap();
        let (health_before, before) = blockers_snapshot(book.connection());
        assert_eq!(
            get(&before, "ready"),
            Some(&Py::Bool(false)),
            "upgraded book with stale Bark journals must block reports before rebuild: {before:?}"
        );
        let journal_before = get(&health_before, "journals").unwrap();
        assert_eq!(get(journal_before, "status"), Some(&s("not_processed")));

        // Rebuild with current Kassiber: report readiness is restored and double-counting is gone.
        run_journals(book.connection());
        let (health_after, after) = blockers_snapshot(book.connection());
        assert_eq!(
            get(&after, "ready"),
            Some(&Py::Bool(true)),
            "rebuilt Bark book must be report ready: {after:?}"
        );
        let journal_after = get(&health_after, "journals").unwrap();
        assert_eq!(get(journal_after, "status"), Some(&s("current")));
        let acquisitions: i64 = book
            .connection()
            .query_row(
                "SELECT COALESCE(SUM(quantity), 0) FROM journal_entries WHERE entry_type = 'acquisition'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(acquisitions, 100_000_000);
    }

    // Reopening via production `Book::open_migrating` is a no-op and keeps the book current.
    {
        let book = kassiber_store::Book::open_migrating(
            root.path(),
            kassiber_store::OpenOptions::default(),
            true,
        )
        .unwrap();
        let (health_reopen, reopen) = blockers_snapshot(book.connection());
        assert_eq!(get(&reopen, "ready"), Some(&Py::Bool(true)));
        assert_eq!(
            get(get(&health_reopen, "journals").unwrap(), "status"),
            Some(&s("current"))
        );
    }

    // A brand-new book created and processed on this build must stay current on reopen.
    let fresh_root = ScratchDir::new("fresh-reopen");
    {
        let book = kassiber_store::Book::create(fresh_root.path(), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Shop','2026-01-01T00:00:00Z');
             INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Main','EUR','generic','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        kassiber_setup::context::execute(
            conn,
            "context.set",
            &dict([("workspace", s("w")), ("profile", s("p"))]),
        )
        .unwrap();
        insert_wallet(
            conn,
            "w-bark",
            "BTCPay Bark",
            "custom",
            json!({
                "sync_source": "btcpay",
                "backend": "merchant",
                "store_id": "shop",
                "payment_method_id": "BTC-ARK",
                "chain": "bitcoin",
                "network": "main",
            }),
        );
        insert_wallet(
            conn,
            "w-multi",
            "Store multisig",
            "address",
            json!({
                "chain": "bitcoin",
                "network": "main",
                "addresses": [MULTISIG_ADDRESS],
            }),
        );
        insert_tx(
            conn,
            "ln-in",
            "w-bark",
            "btcpay-bark:shop:ln-1",
            None,
            "2026-11-10T10:00:00Z",
            "inbound",
            100_000_000,
            0,
            "income",
            &bark_raw("lightning-receive", "ln-1", 100_000_000, 0, None),
            80_000.0,
        );
        insert_tx(
            conn,
            "out-send",
            "w-bark",
            "btcpay-bark:shop:send",
            None,
            "2026-11-11T10:00:00Z",
            "outbound",
            30_000_000,
            2_802_000,
            "withdrawal",
            &bark_raw(
                "send-onchain",
                "send",
                -32_802_000,
                2_802_000,
                Some(TXID_SEND),
            ),
            80_000.0,
        );
        let raw = receipt_raw(TXID_SEND, 0, 30_000, &owned_script);
        insert_tx(
            conn,
            "in-send",
            "w-multi",
            TXID_SEND,
            Some("txid"),
            "2026-11-11T10:01:00Z",
            "inbound",
            30_000_000,
            0,
            "deposit",
            &raw,
            80_000.0,
        );
        stamp_authoritative_receipt(conn, "in-send", "w-multi", TXID_SEND, 30_000_000, &raw);
        run_journals(conn);
    }
    let reopened = kassiber_store::Book::open_migrating(
        fresh_root.path(),
        kassiber_store::OpenOptions::default(),
        true,
    )
    .unwrap();
    let (health_fresh, fresh_blockers) = blockers_snapshot(reopened.connection());
    assert_eq!(
        get(&fresh_blockers, "ready"),
        Some(&Py::Bool(true)),
        "brand-new Bark book must stay report ready after reopen: {fresh_blockers:?}"
    );
    assert_eq!(
        get(get(&health_fresh, "journals").unwrap(), "status"),
        Some(&s("current"))
    );
}

#[test]
fn bark_adapter_rows_agree_between_evidence_event_key_and_transaction_graph_txid() {
    let conn = open_book();
    insert_wallet(
        &conn,
        "w-bark",
        "BTCPay Bark",
        "custom",
        json!({
            "sync_source": "btcpay",
            "backend": "merchant",
            "store_id": "shop",
            "payment_method_id": "BTC-ARK",
            "chain": "bitcoin",
            "network": "main",
        }),
    );

    // Feed real Bark 0.13.0 JSON shapes through the Bark adapter.
    let send_api = kassiber_pytext::loads(
        &json!({
            "id": "14-1",
            "kind": "send-onchain",
            "amount": "-0.00032802",
            "fee": "0.00002802",
            "timestamp": 1762948310,
            "status": "Confirmed",
            "transactionHash": TXID_SEND,
            "outpoint": null,
            "movementId": 14,
            "paymentHash": null,
            "labels": {}
        })
        .to_string(),
    )
    .unwrap();
    let ark_api = kassiber_pytext::loads(
        &json!({
            "id": "5-1",
            "kind": "ark-receive",
            "amount": "0.0002",
            "fee": null,
            "timestamp": 1762948000,
            "status": "Confirmed",
            "transactionHash": TXID_UNRELATED,
            "outpoint": format!("{TXID_UNRELATED}:0"),
            "movementId": 5,
            "paymentHash": null,
            "labels": {}
        })
        .to_string(),
    )
    .unwrap();

    let send_rec = &kassiber_wallets::btcpay::bark::records(&send_api, "shop").unwrap()[0].record;
    let ark_rec = &kassiber_wallets::btcpay::bark::records(&ark_api, "shop").unwrap()[0].record;
    let send_raw = py_str(get(send_rec, "raw_json").unwrap());
    let ark_raw = py_str(get(ark_rec, "raw_json").unwrap());

    // Canonical evidence event_key agrees: send-onchain is chain/TXID_SEND; ark-receive is NOT chain.
    let send_row_json = json!({
        "id": "tx-send",
        "wallet_id": "w-bark",
        "wallet_kind": "custom",
        "config_json": json!({"chain": "bitcoin", "network": "main"}).to_string(),
        "external_id": "btcpay-bark:shop:14-1",
        "external_id_kind": null,
        "direction": "outbound",
        "asset": "BTC",
        "amount": 30_000_000,
        "fee": 2_802_000,
        "amount_includes_fee": false,
        "raw_json": send_raw,
    });
    let ark_row_json = json!({
        "id": "tx-ark",
        "wallet_id": "w-bark",
        "wallet_kind": "custom",
        "config_json": json!({"chain": "bitcoin", "network": "main"}).to_string(),
        "external_id": "btcpay-bark:shop:5-1",
        "external_id_kind": null,
        "direction": "inbound",
        "asset": "BTC",
        "amount": 20_000_000,
        "fee": 0,
        "amount_includes_fee": false,
        "raw_json": ark_raw,
    });
    let send_key = evidence::event_key(&send_row_json).unwrap();
    assert_eq!(send_key.native_namespace, "chain");
    assert_eq!(send_key.native_event_id, TXID_SEND);
    let ark_key = evidence::event_key(&ark_row_json).unwrap();
    assert_ne!(ark_key.native_namespace, "chain");
    assert_ne!(ark_key.native_event_id, TXID_UNRELATED);

    // Transaction detail/graph snapshot agrees: send-onchain surfaces TXID_SEND;
    // ark-receive keeps its Bark external_id rather than exposing the VTXO hash as txid.
    insert_tx(
        &conn,
        "tx-send",
        "w-bark",
        "btcpay-bark:shop:14-1",
        None,
        "2026-11-11T10:00:00Z",
        "outbound",
        30_000_000,
        2_802_000,
        "withdrawal",
        &send_raw,
        80_000.0,
    );
    insert_tx(
        &conn,
        "tx-ark",
        "w-bark",
        "btcpay-bark:shop:5-1",
        None,
        "2026-11-10T10:00:00Z",
        "inbound",
        20_000_000,
        0,
        "deposit",
        &ark_raw,
        80_000.0,
    );
    let send_graph = kassiber_journals::transaction_graph::snapshot(
        &conn,
        Some(&dict([("transaction", s("tx-send"))])),
    )
    .unwrap();
    assert_eq!(
        get(get(&send_graph, "transaction").unwrap(), "txid"),
        Some(&s(TXID_SEND))
    );
    let ark_graph = kassiber_journals::transaction_graph::snapshot(
        &conn,
        Some(&dict([("transaction", s("tx-ark"))])),
    )
    .unwrap();
    assert_eq!(
        get(get(&ark_graph, "transaction").unwrap(), "txid"),
        Some(&s("btcpay-bark:shop:5-1"))
    );

    // Conflicting txid vs transaction_hash in a stored Bark payload falls back
    // to external_id rather than trusting the conflicting txid.
    let mut conflicting_raw: Value = serde_json::from_str(&send_raw).unwrap();
    conflicting_raw["txid"] = json!(TXID_UNRELATED);
    insert_tx(
        &conn,
        "tx-conflict",
        "w-bark",
        "btcpay-bark:shop:14-conflict",
        None,
        "2026-11-11T10:02:00Z",
        "outbound",
        30_000_000,
        2_802_000,
        "withdrawal",
        &conflicting_raw.to_string(),
        80_000.0,
    );
    let conflict_graph = kassiber_journals::transaction_graph::snapshot(
        &conn,
        Some(&dict([("transaction", s("tx-conflict"))])),
    )
    .unwrap();
    assert_eq!(
        get(get(&conflict_graph, "transaction").unwrap(), "txid"),
        Some(&s("btcpay-bark:shop:14-conflict"))
    );

    // transaction_graph::verify finds the clean Bark on-chain sibling on the
    // receiving multisig tx as an external_record, and excludes the conflicting
    // Bark payload whose raw_json.txid disagrees with transaction_hash.
    let owned_script = kassiber_wallets::config::bitcoin_address_script(MULTISIG_ADDRESS)
        .expect("P2WSH multisig script");
    insert_wallet(
        &conn,
        "w-multi",
        "Store multisig",
        "address",
        json!({
            "chain": "bitcoin",
            "network": "main",
            "addresses": [MULTISIG_ADDRESS],
        }),
    );
    let mut multi_raw_obj: Value =
        serde_json::from_str(&receipt_raw(TXID_SEND, 0, 30_000, &owned_script)).unwrap();
    multi_raw_obj["fee"] = json!(10_000);
    let multi_raw = multi_raw_obj.to_string();
    insert_tx(
        &conn,
        "in-send",
        "w-multi",
        TXID_SEND,
        Some("txid"),
        "2026-11-11T10:01:00Z",
        "inbound",
        30_000_000,
        0,
        "deposit",
        &multi_raw,
        80_000.0,
    );
    let verified = kassiber_journals::transaction_graph::verify(&conn, "p", "in-send").unwrap();
    assert_eq!(get(&verified, "status"), Some(&s("ok")));
    let unexplained = py_list(&verified, "unexplained_rows");
    assert_eq!(
        unexplained.len(),
        1,
        "expected only clean Bark sibling row in verify unexplained_rows, got {verified:?}"
    );
    assert_eq!(get(&unexplained[0], "transaction_id"), Some(&s("tx-send")));
    assert_eq!(get(&unexplained[0], "status"), Some(&s("external_record")));
}

#[test]
fn upgrade_invalidates_retained_exchange_txid_exact_with_missing_prevouts() {
    let root = ScratchDir::new("exchange-upgrade");
    let owned_script = kassiber_wallets::config::bitcoin_address_script(MULTISIG_ADDRESS)
        .expect("P2WSH multisig script");
    {
        let book = kassiber_store::Book::create(root.path(), None).unwrap();
        let conn = book.connection();
        conn.execute_batch(
            "INSERT INTO workspaces(id,label,created_at) VALUES('w','Shop','2026-01-01T00:00:00Z');
             INSERT INTO profiles(id,workspace_id,label,fiat_currency,tax_country,created_at) VALUES('p','w','Main','EUR','generic','2026-01-01T00:00:00Z');",
        )
        .unwrap();
        kassiber_setup::context::execute(
            conn,
            "context.set",
            &dict([("workspace", s("w")), ("profile", s("p"))]),
        )
        .unwrap();
        insert_wallet(
            conn,
            "w-river",
            "River",
            "custom",
            json!({
                "chain": "bitcoin",
                "network": "main",
            }),
        );
        insert_wallet(
            conn,
            "w-multi",
            "Store multisig",
            "address",
            json!({
                "chain": "bitcoin",
                "network": "main",
                "addresses": [MULTISIG_ADDRESS],
            }),
        );
        insert_tx(
            conn,
            "buy-river",
            "w-river",
            "river-buy-1",
            None,
            "2026-11-10T10:00:00Z",
            "inbound",
            100_000_000,
            0,
            "buy",
            &json!({"source": "river_csv"}).to_string(),
            80_000.0,
        );
        insert_tx(
            conn,
            "out-river",
            "w-river",
            TXID_SEND,
            Some("txid"),
            "2026-11-11T10:00:00Z",
            "outbound",
            30_000_000,
            0,
            "withdrawal",
            &json!({"source": "river_csv", "txid": TXID_SEND}).to_string(),
            80_000.0,
        );
        // Authoritative BDK-style receipt missing input prevout data.
        let incomplete_raw = json!({
            "txid": TXID_SEND,
            "status": {"confirmed": true, "block_height": 840000},
            "vin": [{"txid": FOREIGN_INPUT_TXID, "vout": 0}],
            "vout": [
                {"scriptpubkey": FOREIGN_SCRIPT, "value": 40_000},
                {"scriptpubkey": owned_script, "value": 30_000}
            ]
        })
        .to_string();
        insert_tx(
            conn,
            "in-multi",
            "w-multi",
            TXID_SEND,
            Some("txid"),
            "2026-11-11T10:01:00Z",
            "inbound",
            30_000_000,
            0,
            "deposit",
            &incomplete_raw,
            80_000.0,
        );
        stamp_authoritative_receipt(
            conn,
            "in-multi",
            "w-multi",
            TXID_SEND,
            30_000_000,
            &incomplete_raw,
        );
        let sem = kassiber_store::semantics::JOURNAL_CALCULATION_SEMANTICS_VERSION;
        conn.execute(
            "UPDATE profiles SET last_processed_at = '2026-11-12T00:00:00Z', \
             last_processed_tx_count = 3, journal_input_version = 1, \
             last_processed_input_version = 1, journal_semantics_version = ?1, \
             required_journal_semantics_version = ?1 WHERE id = 'p'",
            [sem],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,state,basis_state,reason,created_at) \
             VALUES(?1,'w','p','out-river','in-multi',?1,0,30000000,?1,0,30000000,'main','main','onchain','onchain','BTC','BTC','internal_verified','eligible','exchange_txid_exact','2026-11-12T00:00:00Z')",
            [TXID_SEND],
        )
        .unwrap();
    }

    // Reopening via Book::open_migrating must invalidate the retained exchange_txid_exact book.
    {
        let book = kassiber_store::Book::open_migrating(
            root.path(),
            kassiber_store::OpenOptions::default(),
            true,
        )
        .unwrap();
        let (context, active) =
            kassiber_workspace::readiness::scope(book.connection(), None).unwrap();
        let health =
            kassiber_workspace::readiness::health(book.connection(), &context, active.as_ref())
                .unwrap();
        let coverage = kassiber_journals::report_blockers::rates_coverage(
            book.connection(),
            &dict([("limit", Py::Int(10.into()))]),
        )
        .unwrap();
        let blockers = kassiber_journals::report_blockers::snapshot(
            book.connection(),
            health.clone(),
            coverage,
        )
        .unwrap();
        let journal_before = get(&health, "journals").unwrap();
        assert_eq!(get(journal_before, "status"), Some(&s("not_processed")));
        assert_eq!(get(&blockers, "ready"), Some(&Py::Bool(false)));

        // Rebuilding rejects exchange_txid_exact on the missing-prevout receipt.
        run_journals(book.connection());
        let auto_moves: i64 = book
            .connection()
            .query_row(
                "SELECT COUNT(*) FROM journal_custody_decisions WHERE state = 'internal_verified'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(auto_moves, 0);
        let held: String = book
            .connection()
            .query_row(
                "SELECT reason FROM journal_quarantines WHERE transaction_id = 'out-river'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(held, "recorded_transfer_authority_conflict");
    }
}
