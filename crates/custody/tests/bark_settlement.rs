//! BTCPay Bark on-chain transfers (`send-onchain`, `offboard`, `onchain-send`):
//! an exact Bark on-chain send naming the txid of an owned wallet's observed
//! receipt, for exactly its one owned output, books a `bark_txid_exact` move
//! without review, and every less exact variant keeps its hold or stays
//! unscoped.
use kassiber_custody::{codec, evidence, journal_support};
use serde_json::{json, Value};

const TXID: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER_TXID: &str = "2222222222222222222222222222222222222222222222222222222222222222";
const FOREIGN_INPUT_TXID: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const OWNED_SCRIPT: &str = "00141111111111111111111111111111111111111111";
const SECOND_OWNED_SCRIPT: &str = "00142222222222222222222222222222222222222222";
const FOREIGN_SCRIPT: &str = "0014ffffffffffffffffffffffffffffffffffffffff";
const INPUT_SCRIPT: &str = "0014eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn bark_raw(
    kind: &str,
    part: &str,
    amount_msat: i64,
    fee_msat: i64,
    tx_hash: Option<&str>,
) -> String {
    json!({
        "amount_msat": amount_msat,
        "bark_payment_hash": null,
        "bark_row_id": "14-1",
        "fee_msat": fee_msat,
        "invoice_id": null,
        "kind": kind,
        "movement_id": 14,
        "outpoint": null,
        "part": part,
        "source": "btcpay_bark_wallet",
        "store_id": "shop",
        "transaction_hash": tx_hash,
    })
    .to_string()
}

fn receipt_raw(txid: &str, inputs: Value, outputs: Value) -> String {
    json!({
        "txid": txid,
        "status": {"confirmed": true, "block_height": 840000},
        "vin": inputs,
        "vout": outputs,
    })
    .to_string()
}

fn default_inputs(script: &str) -> Value {
    json!([{
        "txid": FOREIGN_INPUT_TXID,
        "vout": 0,
        "prevout": {"scriptpubkey": script, "value": 100000}
    }])
}

fn default_outputs(value_sats: i64, script: &str) -> Value {
    json!([
        {"scriptpubkey": FOREIGN_SCRIPT, "value": 60000},
        {"scriptpubkey": script, "value": value_sats}
    ])
}

fn stamp_authoritative(mut row: Value) -> Value {
    let graph_hash = evidence::graph_hash(&row["raw_json"]).unwrap().0;
    let quantity_hash = evidence::observed_hash(&row).unwrap().0;
    row["observation_authority_version"] = json!(1);
    row["observation_graph_hash"] = json!(graph_hash);
    row["observation_quantity_hash"] = json!(quantity_hash);
    row["observation_fee_attribution"] = json!("exact");
    row["observation_application_revision"] = json!("test-rev");
    row["observation_observed_at"] = json!("2026-11-13T12:00:00Z");
    row
}

fn bark_wallet(id: &str, kind: &str, network: &str) -> Value {
    json!({
        "id": id,
        "label": "BTCPay Bark",
        "kind": kind,
        "config_json": json!({"chain": "bitcoin", "network": network}).to_string(),
    })
}

fn multisig_wallet(id: &str, network: &str) -> Value {
    json!({
        "id": id,
        "label": "Store multisig",
        "kind": "descriptor",
        "config_json": json!({"chain": "bitcoin", "network": network}).to_string(),
    })
}

#[allow(clippy::too_many_arguments)]
fn bark_send_row(
    id: &str,
    wallet_id: &str,
    wallet_kind: &str,
    network: &str,
    bark_kind: &str,
    part: &str,
    principal_msat: i64,
    fee_msat: i64,
    amount_includes_fee: bool,
    tx_hash: Option<&str>,
) -> Value {
    let raw_signed = -(principal_msat + fee_msat);
    json!({
        "id": id,
        "profile_id": "p",
        "workspace_id": "w",
        "wallet_id": wallet_id,
        "wallet_kind": wallet_kind,
        "config_json": json!({"chain": "bitcoin", "network": network}).to_string(),
        "external_id": format!("btcpay-bark:shop:{id}"),
        "external_id_kind": null,
        "occurred_at": "2026-11-13T11:51:50Z",
        "confirmed_at": "2026-11-13T11:51:50Z",
        "direction": "outbound",
        "asset": "BTC",
        "amount": principal_msat,
        "fee": fee_msat,
        "amount_includes_fee": amount_includes_fee,
        "kind": "withdrawal",
        "fingerprint": format!("fp-{id}"),
        "raw_json": bark_raw(bark_kind, part, raw_signed, fee_msat, tx_hash),
    })
}

fn multisig_receipt_row(
    id: &str,
    wallet_id: &str,
    network: &str,
    txid: &str,
    amount_msat: i64,
    raw: String,
    authoritative: bool,
) -> Value {
    let row = json!({
        "id": id,
        "profile_id": "p",
        "workspace_id": "w",
        "wallet_id": wallet_id,
        "wallet_kind": "descriptor",
        "config_json": json!({"chain": "bitcoin", "network": network}).to_string(),
        "external_id": txid,
        "external_id_kind": "txid",
        "occurred_at": "2026-11-13T11:52:02Z",
        "confirmed_at": "2026-11-13T11:52:02Z",
        "direction": "inbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": 0,
        "amount_includes_fee": false,
        "kind": "deposit",
        "fingerprint": format!("fp-{id}"),
        "raw_json": raw,
    });
    if authoritative {
        stamp_authoritative(row)
    } else {
        row
    }
}

fn owned_index(network: &str, entries: &[(&str, &str)]) -> Value {
    let mut by_script = serde_json::Map::new();
    for (script, wallet_id) in entries {
        by_script
            .entry((*script).to_owned())
            .or_insert_with(|| json!([]))
            .as_array_mut()
            .unwrap()
            .push(json!({
                "wallet_id": wallet_id,
                "chain": "bitcoin",
                "network": network,
            }));
    }
    json!({
        "by_script": by_script,
        "by_outpoint": {},
        "by_txid": {},
    })
}

struct Case {
    wallets: Vec<Value>,
    transactions: Vec<Value>,
    owned_index: Value,
    pair_dismissals: Vec<Value>,
    components: Vec<Value>,
    bark_sync_provenance: Option<Vec<Value>>,
}

fn provenance_entry(row: &Value) -> Value {
    json!({
        "transaction_id": row["id"],
        "observed_hash": evidence::observed_hash(row).unwrap().0,
        "graph_hash": evidence::graph_hash(&row["raw_json"]).unwrap().0,
    })
}

impl Case {
    fn exact(bark_kind: &str, principal_msat: i64, fee_msat: i64) -> Self {
        Self::exact_with_network(bark_kind, principal_msat, fee_msat, "main", TXID)
    }

    fn exact_with_network(
        bark_kind: &str,
        principal_msat: i64,
        fee_msat: i64,
        network: &str,
        bark_tx_hash: &str,
    ) -> Self {
        let raw = receipt_raw(
            TXID,
            default_inputs(INPUT_SCRIPT),
            default_outputs(principal_msat / 1000, OWNED_SCRIPT),
        );
        let bark_row = bark_send_row(
            "out-bark",
            "w-bark",
            "custom",
            network,
            bark_kind,
            "principal",
            principal_msat,
            fee_msat,
            false,
            Some(bark_tx_hash),
        );
        let multi_row = multisig_receipt_row(
            "in-multi",
            "w-multi",
            network,
            TXID,
            principal_msat,
            raw,
            true,
        );
        Self {
            wallets: vec![
                bark_wallet("w-bark", "custom", network),
                multisig_wallet("w-multi", network),
            ],
            transactions: vec![bark_row, multi_row],
            owned_index: owned_index(network, &[(OWNED_SCRIPT, "w-multi")]),
            pair_dismissals: vec![],
            components: vec![],
            bark_sync_provenance: None,
        }
    }

    fn default_provenance(&self) -> Vec<Value> {
        self.transactions
            .iter()
            .filter(|tx| {
                tx["direction"] == "outbound"
                    && tx["raw_json"]
                        .as_str()
                        .and_then(|s| serde_json::from_str::<Value>(s).ok())
                        .is_some_and(|raw| {
                            raw["source"] == "btcpay_bark_wallet"
                                && raw["part"] == "principal"
                                && matches!(
                                    raw["kind"].as_str(),
                                    Some("send-onchain" | "offboard" | "onchain-send")
                                )
                        })
            })
            .map(provenance_entry)
            .collect()
    }

    fn run(&self) -> (Value, Value) {
        let bark_sync_provenance = self
            .bark_sync_provenance
            .clone()
            .unwrap_or_else(|| self.default_provenance());
        let input = json!({
            "schema_version": 1,
            "operation": "custody",
            "as_of": "2026-11-13T12:15:31Z",
            "profile": {"id": "p", "workspace_id": "w", "label": "Shop", "tax_jurisdiction": "generic"},
            "wallets": self.wallets,
            "transactions": self.transactions,
            "rate_rows": [],
            "owned_index": self.owned_index,
            "ownership_warnings": [],
            "components": self.components,
            "anchors": {},
            "anchor_wallets": [],
            "revision_headers": [],
            "receipt_sources": [],
            "channel_records": [],
            "loan_legs": [],
            "evidence_snapshots": [],
            "migration_issues": [],
            "import_duplicate_quarantines": [],
            "reset_hold_quarantines": [],
            "provider_hold_quarantines": [],
            "excluded_txids": [],
            "bark_sync_provenance": bark_sync_provenance,
            "gap_reviews": [],
            "pair_dismissals": self.pair_dismissals,
            "gap_worklist": {"large_book": false, "rows": []},
        });
        let snapshot = kassiber_custody::parse(&input.to_string()).unwrap();
        let (rows, stages) = kassiber_custody::run_with_rows(&snapshot, true).unwrap();
        let quantity = codec::to_json(&stages.quantity.py().unwrap());
        let projection = codec::to_json(
            &stages
                .finalized_projection
                .unwrap()
                .py(&rows.transactions)
                .unwrap(),
        );
        (quantity, projection)
    }
}

fn decisions(quantity: &Value) -> &[Value] {
    quantity["projection"]["decisions"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn quarantines(projection: &Value) -> &[Value] {
    projection["quarantines"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
}

fn has_bark_move(quantity: &Value, projection: &Value) -> bool {
    decisions(quantity)
        .iter()
        .any(|d| d["state"] == "internal_verified" && d["reason"] == "bark_txid_exact")
        && projection["intra_pairs"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|p| p["source"] == "bark_txid_exact")
}

fn has_authority_conflict(projection: &Value, tx_id: &str) -> bool {
    quarantines(projection).iter().any(|q| {
        q["transaction_id"] == tx_id && q["reason"] == "recorded_transfer_authority_conflict"
    })
}

fn has_quarantine(projection: &Value, tx_id: &str, reason: &str) -> bool {
    quarantines(projection)
        .iter()
        .any(|q| q["transaction_id"] == tx_id && q["reason"] == reason)
}

fn projected_row_ids(projection: &Value) -> Vec<String> {
    projection["rows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| r["id"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn bark_send_onchain_offboard_and_onchain_send_book_exact_transfers() {
    for (kind, principal, fee) in [
        ("send-onchain", 30_000_000, 2_802_000),
        ("offboard", 14_387_000, 2_768_000),
        ("onchain-send", 20_000_000, 201_000),
    ] {
        let (quantity, projection) = Case::exact(kind, principal, fee).run();
        assert!(
            has_bark_move(&quantity, &projection),
            "{kind} should book bark_txid_exact: quantity={quantity}, projection={projection}"
        );
        assert!(
            quarantines(&projection).is_empty(),
            "{kind} should leave zero quarantines: {projection}"
        );
        let decision = decisions(&quantity)
            .iter()
            .find(|d| d["reason"] == "bark_txid_exact")
            .unwrap();
        assert_eq!(decision["state"], "internal_verified", "{kind}");
        assert_eq!(
            decision["atomic_bundle_id"],
            format!("pair:bark-txid:{TXID}:1"),
            "{kind}"
        );
        let pair = projection["intra_pairs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["source"] == "bark_txid_exact")
            .unwrap();
        assert_eq!(pair["out"]["amount"], principal, "{kind}");
        assert_eq!(
            pair["out"]["fee"], fee,
            "{kind} fee must book as transfer fee"
        );
        assert_eq!(pair["in"]["amount"], principal, "{kind}");
    }
}

#[test]
fn bark_uppercase_transaction_hash_and_regtest_book_exact_transfer() {
    let upper = TXID.to_uppercase();
    let (quantity, projection) =
        Case::exact_with_network("send-onchain", 30_000_000, 2_802_000, "regtest", &upper).run();
    assert!(has_bark_move(&quantity, &projection));
    assert!(quarantines(&projection).is_empty());
}

#[test]
fn bark_onchain_scope_accepts_onchain_kinds_and_rejects_vtxo_fee_and_bad_hashes() {
    let row = |kind: &str, part: &str, hash: Option<&str>| {
        bark_send_row(
            "r", "w-bark", "custom", "main", kind, part, 30_000_000, 1_000, false, hash,
        )
    };
    for kind in [
        "send-onchain",
        "offboard",
        "onchain-send",
        "onchain-receive",
    ] {
        assert_eq!(
            journal_support::onchain_scope(&row(kind, "principal", Some(TXID))),
            Some(("bitcoin".into(), "main".into(), TXID.into(), "BTC".into())),
            "{kind} principal should carry on-chain scope"
        );
    }
    for kind in [
        "ark-receive",
        "ark-send",
        "lightning-receive",
        "lightning-send",
        "board",
        "refresh",
        "exit",
        "exit-fee",
        "exit-claim",
        "onchain-internal",
    ] {
        assert_eq!(
            journal_support::onchain_scope(&row(kind, "principal", Some(TXID))),
            None,
            "{kind} must never carry on-chain scope"
        );
    }
    assert_eq!(
        journal_support::onchain_scope(&row("send-onchain", "fee", Some(TXID))),
        None,
        "fee-part rows must never carry on-chain scope"
    );
    assert_eq!(
        journal_support::onchain_scope(&row("send-onchain", "principal", Some("not-a-txid"))),
        None,
        "malformed transaction_hash must fail closed"
    );
    assert_eq!(
        journal_support::onchain_scope(&row("send-onchain", "principal", None)),
        None,
        "missing transaction_hash must have no scope"
    );
}

#[test]
fn bark_gross_amount_and_fee_inclusive_keep_authority_conflict_hold() {
    // Gross amount (32,802 sat) != owned output (30,000 sat).
    let mut gross = Case::exact("send-onchain", 30_000_000, 2_802_000);
    gross.transactions[0]["amount"] = json!(32_802_000);
    let (quantity, projection) = gross.run();
    assert!(!has_bark_move(&quantity, &projection));
    assert!(has_authority_conflict(&projection, "out-bark"));

    // amount_includes_fee = true violates Bark's principal convention.
    let mut inclusive = Case::exact("send-onchain", 30_000_000, 2_802_000);
    inclusive.transactions[0]["amount"] = json!(32_802_000);
    inclusive.transactions[0]["amount_includes_fee"] = json!(true);
    let (quantity, projection) = inclusive.run();
    assert!(!has_bark_move(&quantity, &projection));
    assert!(has_authority_conflict(&projection, "out-bark"));
}

#[test]
fn bark_graph_and_ownership_mismatches_refuse_promotion() {
    // Not owned output.
    let mut not_owned = Case::exact("send-onchain", 30_000_000, 2_802_000);
    not_owned.owned_index = owned_index("main", &[]);
    let (q, p) = not_owned.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));

    // Owned input.
    let mut owned_input = Case::exact("send-onchain", 30_000_000, 2_802_000);
    owned_input.owned_index = owned_index(
        "main",
        &[(OWNED_SCRIPT, "w-multi"), (INPUT_SCRIPT, "w-multi")],
    );
    let (q, p) = owned_input.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));

    // Two owned outputs.
    let mut two_outputs = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        default_inputs(INPUT_SCRIPT),
        json!([
            {"scriptpubkey": OWNED_SCRIPT, "value": 15000},
            {"scriptpubkey": SECOND_OWNED_SCRIPT, "value": 15000}
        ]),
    );
    two_outputs.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    two_outputs.owned_index = owned_index(
        "main",
        &[(OWNED_SCRIPT, "w-multi"), (SECOND_OWNED_SCRIPT, "w-multi")],
    );
    let (q, p) = two_outputs.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));

    // Foreign owner on the matched output script.
    let mut foreign_owner = Case::exact("send-onchain", 30_000_000, 2_802_000);
    foreign_owner
        .wallets
        .push(multisig_wallet("w-other", "main"));
    foreign_owner.owned_index = owned_index("main", &[(OWNED_SCRIPT, "w-other")]);
    let (q, p) = foreign_owner.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));

    // Short receipt (output value 35,000 sat != receipt amount 30,000 sat).
    let mut short_receipt = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        default_inputs(INPUT_SCRIPT),
        default_outputs(35_000, OWNED_SCRIPT),
    );
    short_receipt.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = short_receipt.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));
}

#[test]
fn bark_duplicate_legs_other_kinds_and_network_or_authority_gaps_refuse_promotion() {
    // Two receiving wallets for one txid.
    let mut two_wallets = Case::exact("send-onchain", 30_000_000, 2_802_000);
    two_wallets
        .wallets
        .push(multisig_wallet("w-multi-2", "main"));
    let raw = two_wallets.transactions[1]["raw_json"]
        .as_str()
        .unwrap()
        .to_owned();
    two_wallets.transactions.push(multisig_receipt_row(
        "in-multi-2",
        "w-multi-2",
        "main",
        TXID,
        30_000_000,
        raw,
        true,
    ));
    let (q, p) = two_wallets.run();
    assert!(!has_bark_move(&q, &p));
    for tx_id in ["out-bark", "in-multi", "in-multi-2"] {
        assert!(
            has_quarantine(&p, tx_id, "owned_fanout_unresolved"),
            "two receiving wallets must hold {tx_id}: {p}"
        );
    }
    assert!(
        projected_row_ids(&p).is_empty(),
        "unresolved fan-out must book no standalone rows: {p}"
    );

    // Two Bark sends in the same wallet sharing one transaction_hash:
    // held as canonical_event_leg_contradiction across all three rows.
    let mut two_sends_same_wallet = Case::exact("send-onchain", 30_000_000, 2_802_000);
    two_sends_same_wallet.transactions.push(bark_send_row(
        "out-bark-2",
        "w-bark",
        "custom",
        "main",
        "offboard",
        "principal",
        30_000_000,
        2_802_000,
        false,
        Some(TXID),
    ));
    let (q, p) = two_sends_same_wallet.run();
    assert!(!has_bark_move(&q, &p));
    for tx_id in ["out-bark", "out-bark-2", "in-multi"] {
        assert!(
            has_quarantine(&p, tx_id, "custody_quantity_unresolved"),
            "duplicate same-wallet Bark sends must hold {tx_id}: {p}"
        );
    }
    assert!(
        projected_row_ids(&p).is_empty(),
        "duplicate same-wallet Bark sends must book no standalone receipt acquisition: {p}"
    );

    // Two Bark sends across two wallets sharing one transaction_hash:
    // held by fan-out safety as owned_fanout_unresolved across all three rows.
    let mut two_sends_two_wallets = Case::exact("send-onchain", 30_000_000, 2_802_000);
    two_sends_two_wallets
        .wallets
        .push(bark_wallet("w-bark-2", "custom", "main"));
    two_sends_two_wallets.transactions.push(bark_send_row(
        "out-bark-2",
        "w-bark-2",
        "custom",
        "main",
        "offboard",
        "principal",
        30_000_000,
        2_802_000,
        false,
        Some(TXID),
    ));
    let (q, p) = two_sends_two_wallets.run();
    assert!(!has_bark_move(&q, &p));
    for tx_id in ["out-bark", "out-bark-2", "in-multi"] {
        assert!(
            has_quarantine(&p, tx_id, "owned_fanout_unresolved"),
            "duplicate multi-wallet Bark sends must hold {tx_id}: {p}"
        );
    }
    assert!(
        projected_row_ids(&p).is_empty(),
        "duplicate multi-wallet Bark sends must book no standalone receipt acquisition: {p}"
    );

    // Non-custom wallet kind cannot claim Bark promotion.
    let mut other_wallet_kind = Case::exact("send-onchain", 30_000_000, 2_802_000);
    other_wallet_kind.wallets[0]["kind"] = json!("generic");
    other_wallet_kind.transactions[0]["wallet_kind"] = json!("generic");
    let (q, p) = other_wallet_kind.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));

    // Network mismatch (Bark on test vs multisig on main).
    let mut network_mismatch = Case::exact("send-onchain", 30_000_000, 2_802_000);
    network_mismatch.wallets[0] = bark_wallet("w-bark", "custom", "test");
    network_mismatch.transactions[0]["config_json"] =
        json!({"chain": "bitcoin", "network": "test"})
            .to_string()
            .into();
    let (q, p) = network_mismatch.run();
    assert!(!has_bark_move(&q, &p));

    // Non-authoritative target (e.g. BTCPay BTC-CHAIN wallet_source):
    // refuses automatic promotion and holds BOTH legs so neither books standalone.
    let mut non_auth = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = non_auth.transactions[1]["raw_json"]
        .as_str()
        .unwrap()
        .to_owned();
    non_auth.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, false);
    let (q, p) = non_auth.run();
    assert!(!has_bark_move(&q, &p));
    assert!(
        has_authority_conflict(&p, "out-bark"),
        "statement-only Bark match must hold out-bark: {p}"
    );
    assert!(
        has_quarantine(&p, "in-multi", "transfer_pair_dependency_blocked"),
        "statement-only Bark match must also hold in-multi: {p}"
    );
    assert!(
        projected_row_ids(&p).is_empty(),
        "statement-only Bark match must book neither leg standalone: {p}"
    );

    // Amount-balanced fan-out (10,000 + 20,000 = 30,000 sat): both authoritative
    // and statement-only multi-receipt groups must hold all legs as owned_fanout_unresolved
    // with zero standalone rows.
    for authoritative in [true, false] {
        let mut balanced = Case::exact("send-onchain", 30_000_000, 2_802_000);
        balanced.wallets.push(multisig_wallet("w-multi-2", "main"));
        let raw_a = receipt_raw(
            TXID,
            default_inputs(INPUT_SCRIPT),
            default_outputs(10_000, OWNED_SCRIPT),
        );
        let raw_b = receipt_raw(
            TXID,
            default_inputs(INPUT_SCRIPT),
            default_outputs(20_000, SECOND_OWNED_SCRIPT),
        );
        balanced.transactions[1] = multisig_receipt_row(
            "in-multi-1",
            "w-multi",
            "main",
            TXID,
            10_000_000,
            raw_a,
            authoritative,
        );
        balanced.transactions.push(multisig_receipt_row(
            "in-multi-2",
            "w-multi-2",
            "main",
            TXID,
            20_000_000,
            raw_b,
            authoritative,
        ));
        balanced.owned_index = owned_index(
            "main",
            &[
                (OWNED_SCRIPT, "w-multi"),
                (SECOND_OWNED_SCRIPT, "w-multi-2"),
            ],
        );
        let (q, p) = balanced.run();
        assert!(!has_bark_move(&q, &p));
        for tx_id in ["out-bark", "in-multi-1", "in-multi-2"] {
            assert!(
                has_quarantine(&p, tx_id, "owned_fanout_unresolved"),
                "amount-balanced Bark fan-out (auth={authoritative}) must hold {tx_id}: {p}"
            );
        }
        assert!(
            projected_row_ids(&p).is_empty(),
            "amount-balanced Bark fan-out (auth={authoritative}) must emit no standalone rows: {p}"
        );
    }

    // Bark ark-receive / lightning-send / exit / ark-send must not promote.
    for kind in ["ark-send", "lightning-send", "exit", "ark-receive"] {
        let mut case = Case::exact("send-onchain", 30_000_000, 2_802_000);
        case.transactions[0]["raw_json"] = json!(bark_raw(
            kind,
            "principal",
            -32_802_000,
            2_802_000,
            Some(TXID)
        ));
        let (q, p) = case.run();
        assert!(!has_bark_move(&q, &p), "{kind} must not promote");
        assert!(
            !has_authority_conflict(&p, "out-bark"),
            "{kind} must not get an on-chain scope or authority hold"
        );
    }

    // Missing and malformed transaction_hash must not promote.
    for bad in [None, Some("not-a-txid"), Some(OTHER_TXID)] {
        let mut case = Case::exact("send-onchain", 30_000_000, 2_802_000);
        case.transactions[0]["raw_json"] = json!(bark_raw(
            "send-onchain",
            "principal",
            -32_802_000,
            2_802_000,
            bad
        ));
        let (q, p) = case.run();
        assert!(!has_bark_move(&q, &p), "{bad:?} must not promote");
    }
}

#[test]
fn bark_onchain_receive_from_owned_wallet_keeps_authority_conflict_hold() {
    let raw = receipt_raw(
        TXID,
        default_inputs(OWNED_SCRIPT),
        default_outputs(30_000, FOREIGN_SCRIPT),
    );
    let mut owned_send =
        multisig_receipt_row("out-multi", "w-multi", "main", TXID, 30_000_000, raw, false);
    owned_send["direction"] = json!("outbound");
    owned_send["kind"] = json!("withdrawal");
    owned_send["fee"] = json!(1_000_000);
    let owned_send = stamp_authoritative(owned_send);
    let mut bark_receive = bark_send_row(
        "in-bark",
        "w-bark",
        "custom",
        "main",
        "onchain-receive",
        "principal",
        30_000_000,
        0,
        false,
        Some(TXID),
    );
    bark_receive["direction"] = json!("inbound");
    bark_receive["kind"] = json!("deposit");
    bark_receive["raw_json"] = json!(bark_raw(
        "onchain-receive",
        "principal",
        30_000_000,
        0,
        Some(TXID)
    ));
    let case = Case {
        wallets: vec![
            bark_wallet("w-bark", "custom", "main"),
            multisig_wallet("w-multi", "main"),
        ],
        transactions: vec![owned_send, bark_receive],
        owned_index: owned_index("main", &[(OWNED_SCRIPT, "w-multi")]),
        pair_dismissals: vec![],
        components: vec![],
        bark_sync_provenance: Some(vec![]),
    };
    let (q, p) = case.run();
    assert!(!has_bark_move(&q, &p));
    assert!(
        has_authority_conflict(&p, "in-bark"),
        "owned send into Bark onchain-receive should hold in-bark for review: {p}"
    );
    assert!(
        has_quarantine(&p, "out-multi", "transfer_pair_dependency_blocked"),
        "owned send into Bark must also hold out-multi so it cannot book as a disposal: {p}"
    );
    assert!(
        projected_row_ids(&p).is_empty(),
        "held owned send into Bark must not emit standalone disposal or acquisition rows: {p}"
    );
}

#[test]
fn bark_active_dismissal_reverts_to_authority_conflict_and_expired_dismissal_promotes() {
    // Active dismissal: falls back to recorded_transfer_authority_conflict
    // with the exact detail shape desktop_journals.rs expects for restore_pairing,
    // and holds the multisig receipt as transfer_pair_dependency_blocked so it
    // never books a standalone acquisition.
    let mut dismissed = Case::exact("send-onchain", 30_000_000, 2_802_000);
    dismissed.pair_dismissals.push(json!({
        "id": "d-undo",
        "out_transaction_id": "out-bark",
        "in_transaction_id": "in-multi",
        "reason": "Automatic pairing undone from custody lineage",
        "expires_at": "2036-11-10T12:15:31Z",
    }));
    let (q, p) = dismissed.run();
    assert!(!has_bark_move(&q, &p));
    let hold = quarantines(&p)
        .iter()
        .find(|item| {
            item["transaction_id"] == "out-bark"
                && item["reason"] == "recorded_transfer_authority_conflict"
        })
        .expect("dismissed Bark pair must hold out-bark");
    let detail: Value = serde_json::from_str(hold["detail_json"].as_str().unwrap()).unwrap();
    assert_eq!(detail["pairing_source"], "row_matched");
    assert_eq!(detail["authoritative_transaction_id"], "in-multi");
    assert_eq!(detail["observation_authority_reason"], "provenance_missing");
    assert_eq!(
        detail["required_for"],
        "authoritative_same_event_wallet_observation"
    );
    assert!(
        has_quarantine(&p, "in-multi", "transfer_pair_dependency_blocked"),
        "dismissed Bark sweep must also hold in-multi: {p}"
    );
    assert!(
        projected_row_ids(&p).is_empty(),
        "dismissed Bark sweep must book neither leg as standalone: {p}"
    );

    // Expired dismissal: promotion resumes automatically.
    let mut expired = Case::exact("send-onchain", 30_000_000, 2_802_000);
    expired.pair_dismissals.push(json!({
        "id": "d-old",
        "out_transaction_id": "out-bark",
        "in_transaction_id": "in-multi",
        "reason": "Automatic pairing undone from custody lineage",
        "expires_at": "2026-11-13T12:15:31Z",
    }));
    let (q, p) = expired.run();
    assert!(has_bark_move(&q, &p));
    assert!(quarantines(&p).is_empty());
}

#[test]
fn bark_requires_bound_sync_provenance_and_complete_observer_graph_scripts() {
    // 1. Missing bark_sync_provenance (as from a generic file import) must refuse promotion.
    let mut no_prov = Case::exact("send-onchain", 30_000_000, 2_802_000);
    no_prov.bark_sync_provenance = Some(vec![]);
    let (q, p) = no_prov.run();
    assert!(!has_bark_move(&q, &p));
    assert!(has_authority_conflict(&p, "out-bark"));
    assert!(has_quarantine(
        &p,
        "in-multi",
        "transfer_pair_dependency_blocked"
    ));
    assert!(projected_row_ids(&p).is_empty());

    // 2. Tampered observed_hash or graph_hash in bark_sync_provenance must refuse promotion.
    for field in ["observed_hash", "graph_hash"] {
        let mut tampered = Case::exact("send-onchain", 30_000_000, 2_802_000);
        let mut entry = provenance_entry(&tampered.transactions[0]);
        entry[field] = json!("0".repeat(64));
        tampered.bark_sync_provenance = Some(vec![entry]);
        let (q, p) = tampered.run();
        assert!(!has_bark_move(&q, &p), "tampered {field} must refuse");
        assert!(has_authority_conflict(&p, "out-bark"));
    }

    // 3. Incomplete observer graph: missing prevout (BDK partial receipt).
    let mut missing_prevout = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        json!([{"txid": FOREIGN_INPUT_TXID, "vout": 0, "prevout": null}]),
        default_outputs(30_000, OWNED_SCRIPT),
    );
    missing_prevout.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = missing_prevout.run();
    assert!(
        !has_bark_move(&q, &p),
        "missing prevout script must not promote"
    );
    assert!(has_authority_conflict(&p, "out-bark"));
    assert!(has_quarantine(
        &p,
        "in-multi",
        "transfer_pair_dependency_blocked"
    ));

    // 4. Incomplete observer graph: empty vin.
    let mut empty_vin = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(TXID, json!([]), default_outputs(30_000, OWNED_SCRIPT));
    empty_vin.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = empty_vin.run();
    assert!(!has_bark_move(&q, &p), "empty vin must not promote");
    assert!(has_authority_conflict(&p, "out-bark"));

    // 5. Incomplete observer graph: output missing scriptpubkey.
    let mut missing_out_script = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        default_inputs(INPUT_SCRIPT),
        json!([
            {"value": 40_000},
            {"scriptpubkey": OWNED_SCRIPT, "value": 30_000},
        ]),
    );
    missing_out_script.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = missing_out_script.run();
    assert!(
        !has_bark_move(&q, &p),
        "missing output scriptpubkey must not promote"
    );
    assert!(has_authority_conflict(&p, "out-bark"));

    // 6. Incomplete observer graph: prevout with script but missing value.
    let mut missing_prevout_value = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        json!([{"txid": FOREIGN_INPUT_TXID, "vout": 0, "prevout": {"scriptpubkey": INPUT_SCRIPT}}]),
        default_outputs(30_000, OWNED_SCRIPT),
    );
    missing_prevout_value.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = missing_prevout_value.run();
    assert!(
        !has_bark_move(&q, &p),
        "prevout without value must not promote"
    );
    assert!(has_authority_conflict(&p, "out-bark"));

    // 7. Malformed non-hex input or output script string must not promote.
    for (vin, vout, label) in [
        (
            json!([{"txid": FOREIGN_INPUT_TXID, "vout": 0, "prevout": {"scriptpubkey": "not-valid-hex!", "value": 80_000}}]),
            default_outputs(30_000, OWNED_SCRIPT),
            "malformed input script",
        ),
        (
            default_inputs(INPUT_SCRIPT),
            json!([
                {"scriptpubkey": "zzz", "value": 40_000},
                {"scriptpubkey": OWNED_SCRIPT, "value": 30_000},
            ]),
            "malformed output script",
        ),
    ] {
        let mut bad_script = Case::exact("send-onchain", 30_000_000, 2_802_000);
        let raw = receipt_raw(TXID, vin, vout);
        bad_script.transactions[1] =
            multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
        let (q, p) = bad_script.run();
        assert!(!has_bark_move(&q, &p), "{label} must not promote");
        assert!(has_authority_conflict(&p, "out-bark"));
    }

    // 8. Dropped non-object entry in vin or vout must not promote.
    for (vin, vout, label) in [
        (
            json!([
                {"txid": FOREIGN_INPUT_TXID, "vout": 0, "prevout": {"scriptpubkey": INPUT_SCRIPT, "value": 80_000}},
                "dropped-input",
            ]),
            default_outputs(30_000, OWNED_SCRIPT),
            "non-object vin entry",
        ),
        (
            default_inputs(INPUT_SCRIPT),
            json!([
                {"scriptpubkey": FOREIGN_SCRIPT, "value": 40_000},
                {"scriptpubkey": OWNED_SCRIPT, "value": 30_000},
                12345,
            ]),
            "non-object vout entry",
        ),
    ] {
        let mut dropped_leg = Case::exact("send-onchain", 30_000_000, 2_802_000);
        let raw = receipt_raw(TXID, vin, vout);
        dropped_leg.transactions[1] =
            multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
        let (q, p) = dropped_leg.run();
        assert!(!has_bark_move(&q, &p), "{label} must not promote");
        assert!(has_authority_conflict(&p, "out-bark"));
    }

    // 9. Fractional value_sats on output must not be truncated to match.
    let mut fractional = Case::exact("send-onchain", 30_000_000, 2_802_000);
    let raw = receipt_raw(
        TXID,
        default_inputs(INPUT_SCRIPT),
        json!([
            {"scriptpubkey": FOREIGN_SCRIPT, "value_sats": 40_000},
            {"scriptpubkey": OWNED_SCRIPT, "value_sats": 30_000.5},
        ]),
    );
    fractional.transactions[1] =
        multisig_receipt_row("in-multi", "w-multi", "main", TXID, 30_000_000, raw, true);
    let (q, p) = fractional.run();
    assert!(
        !has_bark_move(&q, &p),
        "fractional value_sats must not promote"
    );
    assert!(has_authority_conflict(&p, "out-bark"));
}

#[test]
fn bark_event_key_rejects_vtxo_and_fee_hashes_and_accepts_onchain_principal() {
    for kind in [
        "send-onchain",
        "offboard",
        "onchain-send",
        "onchain-receive",
    ] {
        let row = bark_send_row(
            "ok",
            "w-bark",
            "custom",
            "main",
            kind,
            "principal",
            30_000_000,
            2_802_000,
            false,
            Some(&TXID.to_uppercase()),
        );
        let key = evidence::event_key(&row).unwrap();
        assert_eq!(key.native_namespace, "chain", "{kind}");
        assert_eq!(key.native_event_id, TXID, "{kind}");
    }
    for (kind, part) in [
        ("ark-receive", "principal"),
        ("send-onchain", "fee"),
        ("board", "principal"),
        ("refresh", "principal"),
        ("exit", "principal"),
    ] {
        let row = bark_send_row(
            "non-chain",
            "w-bark",
            "custom",
            "main",
            kind,
            part,
            30_000_000,
            0,
            false,
            Some(TXID),
        );
        let key = evidence::event_key(&row).unwrap();
        assert_ne!(
            key.native_namespace, "chain",
            "{kind}/{part} must not get a chain event key: {key:?}"
        );
    }

    // Conflicting txid and transaction_hash in a Bark payload must fail closed
    // in both onchain_scope and evidence::event_key.
    let mut conflicting = bark_send_row(
        "conflict",
        "w-bark",
        "custom",
        "main",
        "send-onchain",
        "principal",
        30_000_000,
        2_802_000,
        false,
        Some(TXID),
    );
    let mut raw: Value = serde_json::from_str(conflicting["raw_json"].as_str().unwrap()).unwrap();
    raw["txid"] = json!(OTHER_TXID);
    conflicting["raw_json"] = json!(raw.to_string());
    assert_eq!(journal_support::onchain_scope(&conflicting), None);
    let key = evidence::event_key(&conflicting).unwrap();
    assert_ne!(
        key.native_namespace, "chain",
        "conflicting Bark txid vs transaction_hash must not get a chain event key: {key:?}"
    );
}
