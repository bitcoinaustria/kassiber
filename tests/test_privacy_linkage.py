from __future__ import annotations

import base64
import json
import socket
import sqlite3
import unittest
from unittest.mock import patch

from kassiber.core import privacy_linkage
from kassiber.core.privacy_linkage import (
    ADVERSARY_KNOWN_COUNTERPARTY,
    ADVERSARY_KYC_SOURCE_FUNDS,
    ADVERSARY_PASSIVE_CHAIN,
    analyze_psbt_privacy,
    build_privacy_linkage_graph,
)


PROFILE_ID = "profile-privacy"


def _conn() -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE wallet_utxos (
            profile_id TEXT NOT NULL,
            wallet_id TEXT NOT NULL,
            txid TEXT NOT NULL,
            vout INTEGER NOT NULL,
            amount INTEGER NOT NULL,
            branch_label TEXT,
            branch_index INTEGER,
            spent_by TEXT,
            asset TEXT NOT NULL,
            chain TEXT NOT NULL,
            address TEXT,
            script_pubkey TEXT
        );
        CREATE TABLE transactions (
            id TEXT,
            profile_id TEXT NOT NULL,
            external_id TEXT,
            direction TEXT,
            fee INTEGER,
            amount INTEGER,
            asset TEXT,
            counterparty TEXT,
            raw_json TEXT
        );
        CREATE TABLE source_funds_sources (
            id TEXT,
            profile_id TEXT NOT NULL,
            source_type TEXT,
            asset TEXT,
            amount INTEGER,
            review_state TEXT
        );
        CREATE TABLE source_funds_links (
            id TEXT,
            profile_id TEXT NOT NULL,
            from_source_id TEXT,
            from_transaction_id TEXT,
            to_transaction_id TEXT,
            state TEXT,
            confidence TEXT,
            allocation_amount INTEGER,
            asset TEXT,
            from_asset TEXT
        );
        """
    )
    return conn


def _txid(prefix: str) -> str:
    return prefix * 32


def _insert_utxo(
    conn: sqlite3.Connection,
    *,
    wallet_id: str,
    txid: str,
    vout: int = 0,
    amount: int = 100_000_000,
    address: str | None = None,
    script_pubkey: str | None = None,
    branch_label: str | None = "receive",
    branch_index: int | None = 0,
    spent_by: str | None = None,
) -> None:
    conn.execute(
        """
        INSERT INTO wallet_utxos(
            profile_id, wallet_id, txid, vout, amount, branch_label, branch_index,
            spent_by, asset, chain, address, script_pubkey
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            PROFILE_ID,
            wallet_id,
            txid,
            vout,
            amount,
            branch_label,
            branch_index,
            spent_by,
            "BTC",
            "bitcoin",
            address,
            script_pubkey,
        ),
    )


def _insert_tx(
    conn: sqlite3.Connection,
    txid: str,
    vin: list[tuple[str, int]],
    *,
    direction: str | None = None,
    fee: int = 0,
    amount: int = 100_000_000,
    asset: str = "BTC",
    counterparty: str | None = None,
    raw_json: dict[str, object] | None = None,
) -> None:
    payload = raw_json or {
        "vin": [{"txid": prev_txid, "vout": vout} for prev_txid, vout in vin]
    }
    conn.execute(
        """
        INSERT INTO transactions(
            id, profile_id, external_id, direction, fee, amount, asset, counterparty, raw_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            txid,
            PROFILE_ID,
            txid,
            direction,
            fee,
            amount,
            asset,
            counterparty,
            json.dumps(payload),
        ),
    )


def _insert_source(
    conn: sqlite3.Connection,
    *,
    source_id: str = "source-anchor",
    source_type: str = "fiat_purchase",
    amount: int = 100_000_000,
    asset: str = "BTC",
    review_state: str = "reviewed",
) -> None:
    conn.execute(
        """
        INSERT INTO source_funds_sources(
            id, profile_id, source_type, asset, amount, review_state
        )
        VALUES (?, ?, ?, ?, ?, ?)
        """,
        (source_id, PROFILE_ID, source_type, asset, amount, review_state),
    )


def _insert_source_anchor(
    conn: sqlite3.Connection,
    *,
    to_transaction_id: str,
    source_id: str = "source-anchor",
    source_type: str = "fiat_purchase",
    source_amount: int = 100_000_000,
    allocation_amount: int | None = None,
    confidence: str = "exact",
    state: str = "reviewed",
) -> None:
    _insert_source(
        conn,
        source_id=source_id,
        source_type=source_type,
        amount=source_amount,
    )
    conn.execute(
        """
        INSERT INTO source_funds_links(
            id, profile_id, from_source_id, from_transaction_id,
            to_transaction_id, state, confidence, allocation_amount, asset, from_asset
        )
        VALUES (?, ?, ?, NULL, ?, ?, ?, ?, ?, ?)
        """,
        (
            f"link-{source_id}-{to_transaction_id}",
            PROFILE_ID,
            source_id,
            to_transaction_id,
            state,
            confidence,
            allocation_amount,
            "BTC",
            "BTC",
        ),
    )


def _insert_source_tx_link(
    conn: sqlite3.Connection,
    *,
    from_transaction_id: str,
    to_transaction_id: str,
    allocation_amount: int,
    confidence: str = "exact",
    state: str = "reviewed",
) -> None:
    conn.execute(
        """
        INSERT INTO source_funds_links(
            id, profile_id, from_source_id, from_transaction_id,
            to_transaction_id, state, confidence, allocation_amount, asset, from_asset
        )
        VALUES (?, ?, NULL, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            f"link-{from_transaction_id}-{to_transaction_id}",
            PROFILE_ID,
            from_transaction_id,
            to_transaction_id,
            state,
            confidence,
            allocation_amount,
            "BTC",
            "BTC",
        ),
    )


def _adversary_view(graph, tier: str):
    return next(view for view in graph.adversary_views if view.tier == tier)


def _source_proximity_fact(graph, coin_id: str):
    return next(fact for fact in graph.source_proximity if fact.coin_id == coin_id)


def _compact_size(value: int) -> bytes:
    if value < 0xFD:
        return bytes([value])
    if value <= 0xFFFF:
        return b"\xfd" + value.to_bytes(2, "little")
    if value <= 0xFFFFFFFF:
        return b"\xfe" + value.to_bytes(4, "little")
    return b"\xff" + value.to_bytes(8, "little")


def _unsigned_tx(
    inputs: list[tuple[str, int]],
    outputs: list[tuple[int, str]],
    *,
    sequences: list[int] | None = None,
) -> bytes:
    sequences = sequences or [0xFFFFFFFF for _ in inputs]
    payload = (2).to_bytes(4, "little")
    payload += _compact_size(len(inputs))
    for index, (txid, vout) in enumerate(inputs):
        payload += bytes.fromhex(txid)[::-1]
        payload += vout.to_bytes(4, "little")
        payload += b"\x00"
        payload += sequences[index].to_bytes(4, "little")
    payload += _compact_size(len(outputs))
    for value_sats, script_hex in outputs:
        script = bytes.fromhex(script_hex)
        payload += value_sats.to_bytes(8, "little")
        payload += _compact_size(len(script))
        payload += script
    payload += (0).to_bytes(4, "little")
    return payload


def _psbt(
    inputs: list[tuple[str, int]],
    outputs: list[tuple[int, str]],
    *,
    sequences: list[int] | None = None,
) -> str:
    tx = _unsigned_tx(inputs, outputs, sequences=sequences)
    payload = b"psbt\xff"
    payload += _compact_size(1) + b"\x00" + _compact_size(len(tx)) + tx
    payload += b"\x00"
    payload += b"\x00" * len(inputs)
    payload += b"\x00" * len(outputs)
    return base64.b64encode(payload).decode("ascii")


class PrivacyLinkageTests(unittest.TestCase):
    def test_mixed_network_outpoint_collision_fails_closed(self):
        conn = _conn()
        try:
            conn.execute("ALTER TABLE wallet_utxos ADD COLUMN network TEXT")
            shared_txid = _txid("10")
            _insert_utxo(
                conn,
                wallet_id="main-wallet",
                txid=shared_txid,
                address="bc1qmaincollision",
            )
            conn.execute(
                "UPDATE wallet_utxos SET network = 'main' "
                "WHERE wallet_id = 'main-wallet'"
            )
            _insert_utxo(
                conn,
                wallet_id="regtest-wallet",
                txid=shared_txid,
                address="bcrt1qregtestcollision",
            )
            conn.execute(
                "UPDATE wallet_utxos SET network = 'regtest' "
                "WHERE wallet_id = 'regtest-wallet'"
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        self.assertEqual(graph.nodes, {})
        self.assertEqual(graph.edges, ())
        limitation_codes = {
            limitation["code"] for limitation in graph.limitations
        }
        self.assertIn("mixed_bitcoin_networks_require_selection", limitation_codes)
        self.assertNotIn("no_owned_bitcoin_outputs", limitation_codes)

    def test_already_linked_consolidation_does_not_score_cioh_again(self):
        conn = _conn()
        try:
            first = _txid("11")
            second = _txid("22")
            spend = _txid("33")
            reused_address = "bc1qreusedlocaladdress"
            _insert_utxo(conn, wallet_id="w-a", txid=first, address=reused_address)
            _insert_utxo(conn, wallet_id="w-a", txid=second, address=reused_address)
            _insert_tx(conn, spend, [(first, 0), (second, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        common_edges = [edge for edge in graph.edges if edge.kind == "common_input"]
        reuse_edges = [edge for edge in graph.edges if edge.kind == "address_reuse"]
        self.assertEqual(len(reuse_edges), 1)
        self.assertTrue(reuse_edges[0].new_linkage)
        self.assertEqual(len(common_edges), 1)
        self.assertFalse(common_edges[0].new_linkage)
        self.assertEqual(common_edges[0].merged_cluster_count, 0)
        self.assertEqual(graph.linkage_score, 1)
        self.assertFalse(any(finding.kind == "common_input" for finding in graph.findings))

    def test_distinct_clusters_common_input_scores_new_cluster_merge(self):
        conn = _conn()
        try:
            first = _txid("44")
            second = _txid("55")
            spend = _txid("66")
            _insert_utxo(conn, wallet_id="w-a", txid=first, address="bc1qfirst")
            _insert_utxo(conn, wallet_id="w-b", txid=second, address="bc1qsecond")
            _insert_tx(conn, spend, [(first, 0), (second, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        common_edges = [edge for edge in graph.edges if edge.kind == "common_input"]
        self.assertEqual(len(common_edges), 1)
        self.assertTrue(common_edges[0].new_linkage)
        self.assertEqual(common_edges[0].merged_cluster_count, 1)
        self.assertEqual(graph.linkage_score, 1)
        finding = next(finding for finding in graph.findings if finding.kind == "common_input")
        self.assertEqual(finding.linkage_score, 1)
        self.assertEqual(finding.evidence["new_cluster_merges"], 1)
        self.assertEqual(finding.evidence_level, "exact")

    def test_branch_label_change_uses_ground_truth_instead_of_guessing(self):
        conn = _conn()
        try:
            parent = _txid("77")
            spend = _txid("88")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparent")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qchange",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        change_edges = [edge for edge in graph.edges if edge.kind == "change_output"]
        self.assertEqual(len(change_edges), 1)
        self.assertTrue(change_edges[0].new_linkage)
        self.assertEqual(change_edges[0].source, "stored_vin")
        self.assertEqual(change_edges[0].evidence_level, "exact")
        self.assertEqual(change_edges[0].evidence["change_evidence"], "ground_truth")
        self.assertEqual(change_edges[0].evidence["change_source"], "wallet_branch_role")
        self.assertEqual(graph.linkage_score, 1)
        self.assertEqual(len(graph.observer_entities), 1)
        self.assertEqual(graph.observer_entities[0].heuristics, ("change",))

    def test_spent_by_path_links_owned_input_without_raw_json(self):
        conn = _conn()
        try:
            parent = _txid("ab")
            spend = _txid("bc")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=parent,
                address="bc1qspentbyparent",
                spent_by=spend,
            )
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qspentbychange",
                branch_label="change",
                branch_index=1,
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        change_edges = [edge for edge in graph.edges if edge.kind == "change_output"]
        self.assertEqual(len(change_edges), 1)
        self.assertTrue(change_edges[0].new_linkage)
        self.assertEqual(change_edges[0].source, "spent_by")
        self.assertEqual(graph.linkage_score, 1)

    def test_multi_script_change_label_beats_numeric_convention(self):
        conn = _conn()
        try:
            parent = _txid("cd")
            spend = _txid("de")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparentmulti")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qmultiscriptchange",
                branch_label="p2tr change",
                branch_index=6,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        change_edges = [edge for edge in graph.edges if edge.kind == "change_output"]
        self.assertEqual(len(change_edges), 1)
        self.assertEqual(change_edges[0].evidence_level, "exact")
        self.assertEqual(change_edges[0].evidence["change_evidence"], "imported")
        self.assertEqual(
            change_edges[0].evidence["change_source"], "imported_branch_role"
        )
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn("p2tr change", serialized)
        self.assertNotIn("descriptor", serialized)
        self.assertNotIn("branch_index", serialized)

    def test_numeric_branch_convention_is_only_a_fallback_heuristic(self):
        conn = _conn()
        try:
            parent = _txid("ef")
            spend = _txid("f0")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparentfallback")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qfallbackchange",
                branch_label=None,
                branch_index=1,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        change_edges = [edge for edge in graph.edges if edge.kind == "change_output"]
        self.assertEqual(len(change_edges), 1)
        self.assertEqual(change_edges[0].evidence_level, "derived")
        self.assertEqual(change_edges[0].evidence["change_evidence"], "heuristic")
        self.assertEqual(
            change_edges[0].evidence["change_source"], "numeric_branch_convention"
        )

    def test_receive_branch_label_prevents_numeric_change_guess(self):
        conn = _conn()
        try:
            parent = _txid("0f")
            spend = _txid("10")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparentreceive")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qreceivedestination",
                branch_label="receive",
                branch_index=1,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        self.assertFalse([edge for edge in graph.edges if edge.kind == "change_output"])
        self.assertEqual(graph.linkage_score, 0)

    def test_missing_branch_metadata_degrades_to_unavailable_without_guessing(self):
        conn = _conn()
        try:
            parent = _txid("12")
            spend = _txid("13")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparentunknown")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qunknownrole",
                branch_label=None,
                branch_index=99,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        self.assertFalse([edge for edge in graph.edges if edge.kind == "change_output"])
        limitation = next(
            item
            for item in graph.limitations
            if item["code"] == "change_role_unavailable"
        )
        self.assertEqual(limitation["evidence_level"], "unknown")
        self.assertEqual(limitation["evidence"]["owned_output_count"], 1)

    def test_outbound_sender_tells_are_emitted_by_you(self):
        conn = _conn()
        try:
            parent = _txid("14")
            spend = _txid("15")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qparentout")
            _insert_tx(
                conn,
                spend,
                [(parent, 0)],
                direction="outbound",
                fee=1_000,
                raw_json={
                    "vin": [{"txid": parent, "vout": 0}],
                    "rbf": True,
                },
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(
            {tell.kind for tell in graph.transaction_tells},
            {"sender_rbf", "fee_fingerprint"},
        )
        self.assertTrue(
            all(tell.attribution == "emitted_by_you" for tell in graph.transaction_tells)
        )
        self.assertTrue(all(tell.penalizes_wallet for tell in graph.transaction_tells))
        self.assertEqual(
            payload["summary"]["wallet_grade_penalty_count"],
            len(graph.transaction_tells),
        )

    def test_inbound_sender_tells_are_counterparty_observations_without_wallet_penalty(self):
        conn = _conn()
        try:
            inbound = _txid("21")
            external_a = _txid("22")
            external_b = _txid("23")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=inbound,
                address="bc1qinboundreceipt",
                branch_label="receive",
                branch_index=0,
            )
            _insert_tx(
                conn,
                inbound,
                [(external_a, 0), (external_b, 1)],
                direction="inbound",
                fee=5_000,
                raw_json={
                    "vin": [
                        {"txid": external_a, "vout": 0},
                        {"txid": external_b, "vout": 1},
                    ],
                    "rbf": True,
                    "vout": [
                        {"n": 0, "scriptpubkey_type": "v0_p2wpkh", "value": 1000},
                        {
                            "n": 1,
                            "scriptpubkey_type": "op_return",
                            "scriptpubkey": "6a026b62736563726574",
                            "value": 0,
                        },
                    ],
                },
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(
            {tell.kind for tell in graph.transaction_tells},
            {
                "sender_common_input",
                "sender_rbf",
                "op_return_output",
                "fee_fingerprint",
            },
        )
        self.assertTrue(
            all(
                tell.attribution == "observed_from_counterparty"
                for tell in graph.transaction_tells
            )
        )
        self.assertFalse(any(tell.penalizes_wallet for tell in graph.transaction_tells))
        self.assertEqual(payload["summary"]["wallet_grade_penalty_count"], 0)
        self.assertEqual(
            payload["summary"]["counterparty_observation_count"],
            len(graph.transaction_tells),
        )
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn("raw_json", serialized)
        self.assertNotIn("6a026b62736563726574", serialized)
        self.assertEqual(graph.linkage_score, 0)

    def test_passive_and_reviewed_source_anchor_tiers_differ(self):
        conn = _conn()
        try:
            parent_a = _txid("24")
            spend_a = _txid("25")
            parent_b = _txid("26")
            spend_b = _txid("27")
            _insert_utxo(conn, wallet_id="w-a", txid=parent_a, address="bc1qparenta")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend_a,
                address="bc1qchangea",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(conn, spend_a, [(parent_a, 0)])
            _insert_source_anchor(conn, to_transaction_id=spend_a)
            _insert_utxo(conn, wallet_id="w-b", txid=parent_b, address="bc1qparentb")
            _insert_utxo(
                conn,
                wallet_id="w-b",
                txid=spend_b,
                address="bc1qchangeb",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(conn, spend_b, [(parent_b, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        passive = _adversary_view(graph, ADVERSARY_PASSIVE_CHAIN)
        kyc = _adversary_view(graph, ADVERSARY_KYC_SOURCE_FUNDS)
        self.assertEqual(passive.summary["observer_entity_count"], 2)
        self.assertEqual(passive.summary["exposed_cluster_count"], 2)
        self.assertEqual(passive.summary["wallet_count"], 2)
        self.assertEqual(kyc.summary["observer_entity_count"], 1)
        self.assertEqual(kyc.summary["exposed_cluster_count"], 1)
        self.assertEqual(kyc.summary["wallet_count"], 1)
        self.assertEqual(
            kyc.summary["unknown_coverage"]["node_count"],
            2,
        )
        self.assertEqual(kyc.clusters[0].support_status, "supported_by_local_ground_truth")
        self.assertEqual(kyc.clusters[0].anchor_kinds, ("reviewed_source_anchor",))

    def test_adversary_views_emit_required_metrics_and_evidence_levels(self):
        conn = _conn()
        try:
            parent = _txid("2b")
            spend = _txid("2c")
            _insert_utxo(conn, wallet_id="w-a", txid=parent, address="bc1qmetricparent")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=spend,
                address="bc1qmetricchange",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(conn, spend, [(parent, 0)])

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        views = {view["tier"]: view for view in payload["adversary_views"]}
        self.assertEqual(
            set(views),
            {
                ADVERSARY_PASSIVE_CHAIN,
                ADVERSARY_KYC_SOURCE_FUNDS,
                ADVERSARY_KNOWN_COUNTERPARTY,
            },
        )
        for view in views.values():
            self.assertIn(view["evidence_level"], {"exact", "derived", "unknown"})
            self.assertTrue(view["model_assumptions"])
            for assumption in view["model_assumptions"]:
                self.assertIn(assumption["evidence_level"], {"exact", "derived", "unknown"})
            for key in (
                "observer_entity_count",
                "wallet_count",
                "exposed_cluster_count",
                "unknown_coverage",
            ):
                self.assertIn(key, view["summary"])
            self.assertIn(
                view["summary"]["unknown_coverage"]["evidence_level"],
                {"exact", "derived", "unknown"},
            )
            for cluster in view["clusters"]:
                self.assertIn(cluster["evidence_level"], {"exact", "derived", "unknown"})
                self.assertIn(
                    cluster["support_status"],
                    {
                        "supported_by_local_ground_truth",
                        "not_supported_by_local_ground_truth_under_this_model",
                    },
                )
            for anchor in view["unsupported_anchors"]:
                self.assertIn(anchor["evidence_level"], {"exact", "derived", "unknown"})
                self.assertEqual(
                    anchor["support_status"],
                    "not_supported_by_local_ground_truth_under_this_model",
                )

    def test_known_counterparty_anchor_alters_exposed_inference_without_label_leak(self):
        conn = _conn()
        try:
            parent = _txid("28")
            spend = _txid("29")
            secret_counterparty = "Acme OTC Desk"
            _insert_utxo(conn, wallet_id="w-known", txid=parent, address="bc1qknownparent")
            _insert_utxo(
                conn,
                wallet_id="w-known",
                txid=spend,
                address="bc1qknownchange",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(
                conn,
                spend,
                [(parent, 0)],
                direction="outbound",
                counterparty=secret_counterparty,
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        known = _adversary_view(graph, ADVERSARY_KNOWN_COUNTERPARTY)
        self.assertEqual(known.summary["exposed_cluster_count"], 1)
        self.assertEqual(known.summary["wallet_count"], 1)
        self.assertEqual(known.clusters[0].anchor_kinds, ("known_counterparty_transaction",))
        self.assertEqual(known.clusters[0].support_status, "supported_by_local_ground_truth")
        self.assertEqual(known.unsupported_anchors, ())
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn(secret_counterparty, serialized)
        self.assertIn("known_counterparty_hypothetical", serialized)

    def test_missing_reviewed_anchor_degrades_without_supported_cluster(self):
        conn = _conn()
        try:
            unrelated = _txid("2a")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=unrelated,
                address="bc1qunrelatedanchor",
            )
            _insert_source_anchor(conn, to_transaction_id="missing-local-transaction")

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        kyc = _adversary_view(graph, ADVERSARY_KYC_SOURCE_FUNDS)
        self.assertEqual(kyc.summary["exposed_cluster_count"], 0)
        self.assertEqual(kyc.summary["unsupported_anchor_count"], 1)
        self.assertEqual(
            kyc.summary["unknown_coverage"]["anchor_count_without_local_graph"],
            1,
        )
        self.assertEqual(
            kyc.unsupported_anchors[0].support_status,
            "not_supported_by_local_ground_truth_under_this_model",
        )
        self.assertEqual(kyc.unsupported_anchors[0].matched_node_ids, ())

    def test_reviewed_source_anchor_proximity_marks_owned_coin(self):
        conn = _conn()
        try:
            txid = _txid("30")
            amount = 80_000_000
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=txid,
                amount=amount,
                address="bc1qsourceanchored",
            )
            _insert_tx(conn, txid, [], amount=amount)
            _insert_source_anchor(
                conn,
                to_transaction_id=txid,
                source_id="exchange-source",
                source_type="exchange_withdrawal",
                source_amount=amount,
                allocation_amount=amount,
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        fact = _source_proximity_fact(graph, f"{txid}:0")
        self.assertEqual(fact.provenance_status, "known_source_proximity")
        self.assertEqual(fact.evidence_level, "exact")
        self.assertEqual(fact.source_types, ("exchange_withdrawal",))
        self.assertEqual(fact.nearest_hop_count, 0)
        self.assertEqual(fact.supported_value_msat, amount)
        self.assertEqual(fact.unknown_value_msat, 0)
        self.assertEqual(fact.coverage_ratio_ppm, 1_000_000)
        self.assertEqual(payload["summary"]["source_proximity_known_coin_count"], 1)
        self.assertEqual(payload["summary"]["source_proximity_unknown_coin_count"], 0)
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn("exchange-source", serialized)

    def test_unknown_source_provenance_is_emitted_as_coverage_gap(self):
        conn = _conn()
        try:
            txid = _txid("31")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=txid,
                amount=50_000_000,
                address="bc1qunknownsource",
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        fact = _source_proximity_fact(graph, f"{txid}:0")
        self.assertEqual(fact.provenance_status, "unknown_provenance")
        self.assertEqual(fact.evidence_level, "unknown")
        self.assertEqual(fact.supported_value_msat, 0)
        self.assertEqual(fact.unknown_value_msat, 50_000_000)
        self.assertEqual(fact.evidence["reason"], "no_reviewed_source_path")
        self.assertTrue(
            any(
                limitation["code"] == "source_proximity_coverage_gaps"
                for limitation in graph.limitations
            )
        )

    def test_mixed_value_reviewed_path_reports_partial_source_proximity(self):
        conn = _conn()
        try:
            parent = _txid("32")
            child = _txid("33")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=parent,
                amount=300_000_000,
                address="bc1qparentmixed",
                spent_by=child,
            )
            _insert_tx(conn, parent, [], amount=300_000_000)
            _insert_source_anchor(
                conn,
                to_transaction_id=parent,
                source_id="fiat-source",
                source_type="fiat_purchase",
                source_amount=300_000_000,
                allocation_amount=300_000_000,
            )
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=child,
                amount=200_000_000,
                address="bc1qchildmixed",
                branch_label="change",
                branch_index=1,
            )
            _insert_tx(conn, child, [(parent, 0)], amount=200_000_000)
            _insert_source_tx_link(
                conn,
                from_transaction_id=parent,
                to_transaction_id=child,
                allocation_amount=120_000_000,
            )

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        child_fact = _source_proximity_fact(graph, f"{child}:0")
        self.assertEqual(child_fact.provenance_status, "partial_source_proximity")
        self.assertEqual(child_fact.nearest_hop_count, 1)
        self.assertEqual(child_fact.source_types, ("fiat_purchase",))
        self.assertEqual(child_fact.supported_value_msat, 120_000_000)
        self.assertEqual(child_fact.unknown_value_msat, 80_000_000)
        self.assertEqual(child_fact.coverage_ratio_ppm, 600_000)
        self.assertEqual(child_fact.evidence_level, "unknown")

    def test_source_proximity_does_not_open_network_connections(self):
        conn = _conn()
        try:
            txid = _txid("34")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=txid,
                address="bc1qnoegress",
            )
            _insert_tx(conn, txid, [])
            _insert_source_anchor(conn, to_transaction_id=txid)

            with patch.object(socket, "socket", side_effect=AssertionError("network egress")):
                graph = build_privacy_linkage_graph(conn, PROFILE_ID)
        finally:
            conn.close()

        fact = _source_proximity_fact(graph, f"{txid}:0")
        self.assertEqual(fact.provenance_status, "known_source_proximity")

    def test_address_reuse_links_outputs_without_leaking_address_payload(self):
        conn = _conn()
        try:
            first = _txid("99")
            second = _txid("aa")
            reused_address = "bc1qaddressreusedacrossoutputs"
            _insert_utxo(conn, wallet_id="w-a", txid=first, address=reused_address)
            _insert_utxo(conn, wallet_id="w-b", txid=second, address=reused_address)

            graph = build_privacy_linkage_graph(conn, PROFILE_ID)
            payload = graph.to_redacted_payload()
        finally:
            conn.close()

        reuse_edges = [edge for edge in graph.edges if edge.kind == "address_reuse"]
        self.assertEqual(len(reuse_edges), 1)
        self.assertTrue(reuse_edges[0].new_linkage)
        self.assertEqual(reuse_edges[0].evidence_level, "exact")
        self.assertEqual(payload["summary"]["linkage_score"], 1)
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn(reused_address, serialized)
        self.assertNotIn("script_pubkey", serialized)

    def test_psbt_cluster_merging_scores_prebroadcast_delta(self):
        conn = _conn()
        try:
            first = _txid("35")
            second = _txid("36")
            first_script = "0014" + "11" * 20
            second_script = "0014" + "22" * 20
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=first,
                address="bc1qpsbtfirst",
                script_pubkey=first_script,
            )
            _insert_utxo(
                conn,
                wallet_id="w-b",
                txid=second,
                address="bc1qpsbtsecond",
                script_pubkey=second_script,
            )
            psbt = _psbt(
                [(first, 0), (second, 0)],
                [(150_000, "0014" + "33" * 20)],
            )

            analysis = analyze_psbt_privacy(conn, PROFILE_ID, psbt)
            payload = analysis.to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(payload["summary"]["decode_status"], "decoded")
        self.assertEqual(payload["summary"]["cluster_merge_delta"], 1)
        self.assertEqual(payload["summary"]["blast_radius_score"], 1)
        self.assertEqual(payload["unknowns"]["input_count"], 0)
        self.assertEqual(payload["cluster_merge"]["evidence_level"], "exact")
        self.assertEqual(payload["cluster_merge"]["owned_input_cluster_count"], 2)
        passive = next(
            item
            for item in payload["adversary_deltas"]
            if item["tier"] == ADVERSARY_PASSIVE_CHAIN
        )
        self.assertEqual(passive["cluster_merge_delta"], 1)
        self.assertTrue(
            any(finding["kind"] == "cluster_merge" for finding in payload["findings"])
        )
        self.assertEqual(
            {scenario["scenario"] for scenario in payload["what_if"]},
            {
                "fresh_receive_output",
                "existing_receive_reuse",
                "hypothetical_input_consolidation",
            },
        )
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn(psbt, serialized)
        self.assertNotIn(first_script, serialized)
        self.assertNotIn(second_script, serialized)
        self.assertNotIn("bc1qpsbtfirst", serialized)
        self.assertNotIn("branch_index", serialized)

    def test_psbt_same_cluster_spend_does_not_score_again(self):
        conn = _conn()
        try:
            first = _txid("37")
            second = _txid("38")
            reused_address = "bc1qpsbtreused"
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=first,
                address=reused_address,
                script_pubkey="0014" + "44" * 20,
            )
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=second,
                address=reused_address,
                script_pubkey="0014" + "55" * 20,
            )
            psbt = _psbt(
                [(first, 0), (second, 0)],
                [(150_000, "0014" + "66" * 20)],
            )

            payload = analyze_psbt_privacy(
                conn, PROFILE_ID, psbt
            ).to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(payload["summary"]["cluster_merge_delta"], 0)
        self.assertEqual(payload["cluster_merge"]["owned_input_cluster_count"], 1)
        self.assertFalse(
            any(finding["kind"] == "cluster_merge" for finding in payload["findings"])
        )
        common_input = next(
            tell for tell in payload["transaction_tells"] if tell["kind"] == "sender_common_input"
        )
        self.assertEqual(common_input["evidence_level"], "exact")

    def test_psbt_unknown_inputs_degrade_without_network_egress(self):
        conn = _conn()
        try:
            known = _txid("39")
            unknown = _txid("3a")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=known,
                address="bc1qpsbtknown",
                script_pubkey="0014" + "77" * 20,
            )
            psbt = _psbt(
                [(known, 0), (unknown, 1)],
                [(150_000, "0014" + "88" * 20)],
            )

            with patch.object(socket, "socket", side_effect=AssertionError("network egress")):
                payload = analyze_psbt_privacy(
                    conn, PROFILE_ID, psbt
                ).to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(payload["summary"]["unknown_input_count"], 1)
        self.assertEqual(payload["summary"]["evidence_level"], "unknown")
        self.assertEqual(payload["unknowns"]["evidence_level"], "unknown")
        self.assertTrue(
            any(finding["kind"] == "unknown_inputs" for finding in payload["findings"])
        )
        fee_tell = next(
            tell for tell in payload["transaction_tells"] if tell["kind"] == "fee_fingerprint"
        )
        self.assertEqual(fee_tell["evidence_level"], "unknown")
        self.assertFalse(fee_tell["evidence"]["fee_known"])

    def test_psbt_receive_reuse_what_if_reports_change_evidence(self):
        conn = _conn()
        try:
            parent = _txid("3b")
            existing_receive = _txid("3c")
            reused_script = "0014" + "99" * 20
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=parent,
                address="bc1qpsbtparent",
                script_pubkey="0014" + "aa" * 20,
            )
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=existing_receive,
                address="bc1qpsbtreceive",
                script_pubkey=reused_script,
                branch_label="receive",
                branch_index=0,
            )
            psbt = _psbt([(parent, 0)], [(50_000, reused_script)])

            payload = analyze_psbt_privacy(
                conn, PROFILE_ID, psbt
            ).to_redacted_payload()
        finally:
            conn.close()

        self.assertEqual(payload["change_evidence"]["receive_reuse_output_count"], 1)
        self.assertEqual(payload["change_evidence"]["receive_reuse_cluster_delta"], 1)
        reuse = next(
            item for item in payload["what_if"] if item["scenario"] == "existing_receive_reuse"
        )
        fresh = next(
            item for item in payload["what_if"] if item["scenario"] == "fresh_receive_output"
        )
        self.assertEqual(reuse["support_status"], "supported_by_local_ground_truth")
        self.assertEqual(reuse["cluster_merge_delta"], 1)
        self.assertEqual(fresh["cluster_merge_delta"], 0)
        serialized = json.dumps(payload, sort_keys=True)
        self.assertNotIn(reused_script, serialized)
        self.assertNotIn("bc1qpsbtreceive", serialized)

    def test_psbt_payload_has_no_signing_or_broadcast_path(self):
        conn = _conn()
        try:
            parent = _txid("3d")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=parent,
                address="bc1qpsbtnopath",
                script_pubkey="0014" + "bb" * 20,
            )
            psbt = _psbt([(parent, 0)], [(50_000, "0014" + "cc" * 20)])

            payload = analyze_psbt_privacy(
                conn, PROFILE_ID, psbt
            ).to_redacted_payload()
        finally:
            conn.close()

        self.assertFalse(payload["signing_supported"])
        self.assertFalse(payload["broadcast_supported"])
        self.assertFalse(hasattr(privacy_linkage, "sign_psbt"))
        self.assertFalse(hasattr(privacy_linkage, "broadcast_psbt"))

    def test_psbt_payload_has_no_coin_selection_language(self):
        conn = _conn()
        try:
            first = _txid("3e")
            second = _txid("3f")
            _insert_utxo(
                conn,
                wallet_id="w-a",
                txid=first,
                address="bc1qnolanguagea",
                script_pubkey="0014" + "dd" * 20,
            )
            _insert_utxo(
                conn,
                wallet_id="w-b",
                txid=second,
                address="bc1qnolanguageb",
                script_pubkey="0014" + "ee" * 20,
            )
            psbt = _psbt(
                [(first, 0), (second, 0)],
                [(150_000, "0014" + "ff" * 20)],
            )

            payload = analyze_psbt_privacy(
                conn, PROFILE_ID, psbt
            ).to_redacted_payload()
        finally:
            conn.close()

        serialized = json.dumps(payload, sort_keys=True).lower()
        self.assertNotIn("coin-selection", serialized)
        self.assertNotIn("coin selection", serialized)
        self.assertNotIn("select", serialized)
        self.assertNotIn("recommend", serialized)
        self.assertNotIn("choose", serialized)


if __name__ == "__main__":
    unittest.main()
