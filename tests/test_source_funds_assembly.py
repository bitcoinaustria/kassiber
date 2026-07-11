import json
import sqlite3
import unittest

from kassiber.core.source_funds_assembly import (
    build_owned_outpoint_index,
    derive_payment_hash_pairs,
    derive_utxo_spend_pairs,
)


def _row(row_id, direction, amount, *, source="core_lightning", wallet="w1"):
    return {
        "id": row_id,
        "wallet_id": wallet,
        "wallet_kind": "core-ln",
        "direction": direction,
        "asset": "BTC",
        "amount": amount,
        "occurred_at": "2026-01-01T00:00:00Z",
        "payment_hash": "AB" * 32,
        "payment_hash_source": source,
        "kind": "cln_pay" if direction == "outbound" else "cln_invoice",
        "raw_json": json.dumps(
            {
                "_kassiber_provenance": {"import_source": "core-lightning"},
                "chain": "lightning",
                "network": "main",
            }
        ),
    }


class SourceFundsPaymentHashTests(unittest.TestCase):
    def test_uses_journal_lightning_hash_gate_and_allows_same_wallet(self):
        outbound = _row("out", "outbound", 1_000_000)
        inbound = _row("in", "inbound", 1_000_000)

        pairs = derive_payment_hash_pairs(
            [outbound, inbound], skip_row=lambda _row: False
        )

        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["allocation_msat"], 1_000_000)

    def test_node_hash_does_not_hide_a_principal_mismatch(self):
        outbound = _row("out", "outbound", 1_001_000)
        inbound = _row("in", "inbound", 1_000_000)
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )

    def test_malformed_hash_is_not_lineage_evidence(self):
        outbound = _row("out", "outbound", 1_000_000)
        inbound = _row("in", "inbound", 1_000_000)
        outbound["payment_hash"] = inbound["payment_hash"] = "invoice-123"
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )

    def test_chain_script_hash_does_not_assert_lightning_lineage(self):
        outbound = _row("out", "outbound", 1_001_000, source="chain_script")
        inbound = _row("in", "inbound", 1_000_000, source="chain_script")
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )

    def test_untyped_import_hash_does_not_auto_assert_lineage(self):
        outbound = _row("out", "outbound", 1_001_000, source="import")
        inbound = _row("in", "inbound", 1_000_000, source="import")
        outbound["kind"] = "withdrawal"
        inbound["kind"] = "deposit"
        outbound["wallet_kind"] = "custom"
        inbound["wallet_kind"] = "custom"
        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )

    def test_payment_hash_lineage_never_crosses_bitcoin_networks(self):
        outbound = _row("out", "outbound", 1_000_000)
        inbound = _row("in", "inbound", 1_000_000)
        inbound_payload = json.loads(inbound["raw_json"])
        inbound_payload["network"] = "regtest"
        inbound["raw_json"] = json.dumps(inbound_payload)

        self.assertEqual(
            derive_payment_hash_pairs(
                [outbound, inbound], skip_row=lambda _row: False
            ),
            [],
        )


class SourceFundsUtxoScopeTests(unittest.TestCase):
    def test_owned_index_defaults_only_a_missing_legacy_network_column(self):
        txid = "10" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, asset TEXT,
                txid TEXT, vout INTEGER, amount INTEGER, branch_label TEXT,
                spent_by TEXT
            )
            """
        )
        conn.execute(
            """
            INSERT INTO wallet_utxos(
                profile_id, wallet_id, chain, asset, txid, vout, amount,
                branch_label, spent_by
            ) VALUES ('profile', 'wallet', 'bitcoin', 'BTC', ?, 0, 100,
                      'receive', NULL)
            """,
            (txid,),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()

        self.assertEqual(list(index), [("bitcoin", "main", txid, 0)])

    def test_owned_index_keeps_identical_outpoints_on_networks_separate(self):
        txid = "20" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos(
                profile_id, wallet_id, chain, network, asset, txid, vout,
                amount, branch_label, spent_by
            ) VALUES ('profile', ?, 'bitcoin', ?, 'BTC', ?, 0, 100,
                      'receive', NULL)
            """,
            (("main-wallet", "main", txid), ("regtest-wallet", "regtest", txid)),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()

        self.assertEqual(
            set(index),
            {
                ("bitcoin", "main", txid, 0),
                ("bitcoin", "regtest", txid, 0),
            },
        )

    def test_owned_index_retains_liquid_consensus_asset_identity(self):
        txid = "21" * 32
        asset_id = "ab" * 32
        conn = sqlite3.connect(":memory:")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE wallet_utxos (
                profile_id TEXT, wallet_id TEXT, chain TEXT, network TEXT,
                asset TEXT, txid TEXT, vout INTEGER, amount INTEGER,
                branch_label TEXT, spent_by TEXT, raw_json TEXT
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO wallet_utxos(
                profile_id, wallet_id, chain, network, asset, txid, vout,
                amount, branch_label, spent_by, raw_json
            ) VALUES ('profile', ?, 'liquid', 'liquidv1', 'LBTC', ?, ?, 100,
                      'receive', NULL, ?)
            """,
            [
                ("known", txid, 0, json.dumps({"asset_id": asset_id})),
                ("unknown", txid, 1, "{}"),
            ],
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()

        self.assertEqual(set(index), {("liquid", "liquidv1", txid, 0)})
        self.assertEqual(
            index[("liquid", "liquidv1", txid, 0)]["asset_identity"],
            asset_id,
        )

    def _chain_row(self, row_id, wallet, direction, amount, txid, raw_json):
        return {
            "id": row_id,
            "wallet_id": wallet,
            "wallet_kind": "descriptor",
            "direction": direction,
            "asset": "BTC",
            "amount": amount,
            "occurred_at": "2026-01-01T00:00:00Z",
            "external_id": txid,
            "raw_json": json.dumps(raw_json),
            "wallet_config_json": json.dumps(
                {"chain": "bitcoin", "network": "main"}
            ),
        }

    def _liquid_row(
        self,
        row_id,
        wallet,
        direction,
        amount,
        txid,
        asset_id,
        *,
        vin=(),
    ):
        return {
            "id": row_id,
            "wallet_id": wallet,
            "wallet_kind": "descriptor",
            "direction": direction,
            "asset": "LBTC",
            "amount": amount,
            "occurred_at": "2026-01-02T00:00:00Z",
            "external_id": txid,
            "raw_json": json.dumps(
                {
                    "txid": txid,
                    "chain": "liquid",
                    "network": "liquidv1",
                    "component": {"asset_id": asset_id, "asset": "LBTC"},
                    "vin": list(vin),
                    "vout": [],
                }
            ),
            "wallet_config_json": json.dumps(
                {"chain": "liquid", "network": "liquidv1"}
            ),
        }

    def test_liquid_components_with_same_display_label_never_cross_link(self):
        parent_txid = "23" * 32
        spend_txid = "24" * 32
        asset_a = "aa" * 32
        asset_b = "bb" * 32
        vin = [{"txid": parent_txid, "vout": 0}]
        rows = [
            self._liquid_row(
                "asset-a-out",
                "wallet-a",
                "outbound",
                100,
                spend_txid,
                asset_a,
                vin=vin,
            ),
            self._liquid_row(
                "asset-b-in",
                "wallet-b",
                "inbound",
                100,
                spend_txid,
                asset_b,
                vin=vin,
            ),
        ]
        owned_index = {
            ("liquid", "liquidv1", parent_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "LBTC",
                "asset_identity": asset_a,
                "ambiguous": False,
            },
            ("liquid", "liquidv1", spend_txid, 0): {
                "wallet_id": "wallet-b",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": "",
                "asset": "LBTC",
                "asset_identity": asset_b,
                "ambiguous": False,
            },
        }

        self.assertEqual(
            derive_utxo_spend_pairs(
                rows, owned_index, skip_row=lambda _row: False
            ),
            [],
        )

    def test_liquid_same_component_still_derives_exact_funding_edge(self):
        parent_txid = "25" * 32
        spend_txid = "26" * 32
        asset_id = "cc" * 32
        vin = [{"txid": parent_txid, "vout": 0}]
        rows = [
            self._liquid_row(
                "asset-out",
                "wallet-a",
                "outbound",
                100,
                spend_txid,
                asset_id,
                vin=vin,
            ),
            self._liquid_row(
                "asset-in",
                "wallet-b",
                "inbound",
                100,
                spend_txid,
                asset_id,
                vin=vin,
            ),
        ]
        owned_index = {
            ("liquid", "liquidv1", parent_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "LBTC",
                "asset_identity": asset_id,
                "ambiguous": False,
            },
            ("liquid", "liquidv1", spend_txid, 0): {
                "wallet_id": "wallet-b",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": "",
                "asset": "LBTC",
                "asset_identity": asset_id,
                "ambiguous": False,
            },
        }

        pairs = derive_utxo_spend_pairs(
            rows, owned_index, skip_row=lambda _row: False
        )

        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["kind"], "leg_funding")
        self.assertEqual(
            (pairs[0]["from_row"]["id"], pairs[0]["to_row"]["id"]),
            ("asset-out", "asset-in"),
        )
        self.assertEqual(pairs[0]["confidence"], "exact")
        self.assertFalse(pairs[0]["requires_review"])

    def test_utxo_lineage_is_scoped_to_chain_and_network(self):
        parent_txid = "11" * 32
        spend_txid = "22" * 32
        parent = self._chain_row(
            "parent",
            "wallet-a",
            "inbound",
            100_000_000,
            parent_txid,
            {"txid": parent_txid, "vin": [], "vout": []},
        )
        spend = self._chain_row(
            "spend",
            "wallet-a",
            "outbound",
            99_000_000,
            spend_txid,
            {
                "txid": spend_txid,
                "vin": [{"txid": parent_txid, "vout": 0}],
                "vout": [],
            },
        )
        info = {
            "wallet_id": "wallet-a",
            "amount_msat": 100_000_000,
            "branch_label": "receive",
            "spent_by": spend_txid,
            "asset": "BTC",
            "ambiguous": False,
        }

        wrong_network = {
            ("bitcoin", "regtest", parent_txid, 0): info,
        }
        self.assertEqual(
            derive_utxo_spend_pairs(
                [parent, spend], wrong_network, skip_row=lambda _row: False
            ),
            [],
        )

        correct_network = {
            ("bitcoin", "main", parent_txid, 0): info,
        }
        pairs = derive_utxo_spend_pairs(
            [parent, spend], correct_network, skip_row=lambda _row: False
        )
        self.assertEqual(len(pairs), 1)
        self.assertEqual(pairs[0]["from_row"]["id"], "parent")
        self.assertEqual(pairs[0]["to_row"]["id"], "spend")
        self.assertEqual(pairs[0]["confidence"], "exact")
        self.assertFalse(pairs[0]["requires_review"])

    def test_duplicate_wallet_direction_component_fails_closed(self):
        parent_txid = "27" * 32
        spend_txid = "28" * 32
        parent = self._chain_row(
            "parent",
            "wallet-a",
            "inbound",
            100,
            parent_txid,
            {"txid": parent_txid, "vin": [], "vout": []},
        )
        spend_graph = {
            "txid": spend_txid,
            "vin": [{"txid": parent_txid, "vout": 0}],
            "vout": [],
        }
        duplicate_out_1 = self._chain_row(
            "duplicate-out-1",
            "wallet-a",
            "outbound",
            100,
            spend_txid,
            spend_graph,
        )
        duplicate_out_2 = self._chain_row(
            "duplicate-out-2",
            "wallet-a",
            "outbound",
            100,
            spend_txid,
            spend_graph,
        )
        inbound = self._chain_row(
            "inbound",
            "wallet-b",
            "inbound",
            100,
            spend_txid,
            spend_graph,
        )
        owned_index = {
            ("bitcoin", "main", parent_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", spend_txid, 0): {
                "wallet_id": "wallet-b",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": "",
                "asset": "BTC",
                "ambiguous": False,
            },
        }

        for duplicate_order in (
            [duplicate_out_1, duplicate_out_2],
            [duplicate_out_2, duplicate_out_1],
        ):
            self.assertEqual(
                derive_utxo_spend_pairs(
                    [parent, *duplicate_order, inbound],
                    owned_index,
                    skip_row=lambda _row: False,
                ),
                [],
            )

    def test_duplicate_parent_leg_cannot_be_first_match_lineage(self):
        parent_txid = "29" * 32
        spend_txid = "2a" * 32
        parent_graph = {"txid": parent_txid, "vin": [], "vout": []}
        parents = [
            self._chain_row(
                row_id,
                "wallet-a",
                "inbound",
                100,
                parent_txid,
                parent_graph,
            )
            for row_id in ("duplicate-parent-1", "duplicate-parent-2")
        ]
        spend = self._chain_row(
            "spend",
            "wallet-a",
            "outbound",
            100,
            spend_txid,
            {
                "txid": spend_txid,
                "vin": [{"txid": parent_txid, "vout": 0}],
                "vout": [],
            },
        )
        owned_index = {
            ("bitcoin", "main", parent_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            }
        }

        self.assertEqual(
            derive_utxo_spend_pairs(
                [*parents, spend],
                owned_index,
                skip_row=lambda _row: False,
            ),
            [],
        )

    def test_multi_source_multi_destination_is_pro_rata_review_evidence(self):
        parent_a_txid = "31" * 32
        parent_b_txid = "32" * 32
        spend_txid = "33" * 32
        parent_a = self._chain_row(
            "parent-a",
            "wallet-a",
            "inbound",
            60,
            parent_a_txid,
            {"txid": parent_a_txid, "vin": [], "vout": []},
        )
        parent_b = self._chain_row(
            "parent-b",
            "wallet-b",
            "inbound",
            40,
            parent_b_txid,
            {"txid": parent_b_txid, "vin": [], "vout": []},
        )
        spend_graph = {
            "txid": spend_txid,
            "vin": [
                {"txid": parent_a_txid, "vout": 0},
                {"txid": parent_b_txid, "vout": 0},
            ],
            "vout": [],
        }
        rows = [
            parent_a,
            parent_b,
            self._chain_row("out-a", "wallet-a", "outbound", 60, spend_txid, spend_graph),
            self._chain_row("out-b", "wallet-b", "outbound", 40, spend_txid, spend_graph),
            self._chain_row("in-c", "wallet-c", "inbound", 70, spend_txid, spend_graph),
            self._chain_row("in-d", "wallet-d", "inbound", 30, spend_txid, spend_graph),
        ]
        owned_index = {
            ("bitcoin", "main", parent_a_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 60,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", parent_b_txid, 0): {
                "wallet_id": "wallet-b",
                "amount_msat": 40,
                "branch_label": "receive",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", spend_txid, 0): {
                "wallet_id": "wallet-c",
                "amount_msat": 70,
                "branch_label": "receive",
                "spent_by": "",
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", spend_txid, 1): {
                "wallet_id": "wallet-d",
                "amount_msat": 30,
                "branch_label": "receive",
                "spent_by": "",
                "asset": "BTC",
                "ambiguous": False,
            },
        }

        pairs = derive_utxo_spend_pairs(
            rows, owned_index, skip_row=lambda _row: False
        )

        parent_edges = [pair for pair in pairs if pair["kind"] == "parent_spend"]
        funding_edges = [pair for pair in pairs if pair["kind"] == "leg_funding"]
        self.assertEqual(len(parent_edges), 2)
        self.assertTrue(all(pair["confidence"] == "exact" for pair in parent_edges))
        self.assertTrue(all(not pair["requires_review"] for pair in parent_edges))
        self.assertEqual(len(funding_edges), 4)
        self.assertEqual(
            sorted(pair["allocation_msat"] for pair in funding_edges),
            [12, 18, 28, 42],
        )
        self.assertTrue(all(pair["confidence"] == "strong" for pair in funding_edges))
        self.assertTrue(all(pair["requires_review"] for pair in funding_edges))
        self.assertTrue(
            all("pro-rata accounting allocation" in pair["explanation"] for pair in funding_edges)
        )

    def test_multi_source_single_destination_with_residual_requires_review(self):
        parent_a_txid = "41" * 32
        parent_b_txid = "42" * 32
        spend_txid = "43" * 32
        parent_a = self._chain_row(
            "parent-a", "wallet-a", "inbound", 60, parent_a_txid,
            {"txid": parent_a_txid, "vin": [], "vout": []},
        )
        parent_b = self._chain_row(
            "parent-b", "wallet-b", "inbound", 40, parent_b_txid,
            {"txid": parent_b_txid, "vin": [], "vout": []},
        )
        spend_graph = {
            "txid": spend_txid,
            "vin": [
                {"txid": parent_a_txid, "vout": 0},
                {"txid": parent_b_txid, "vout": 0},
            ],
            "vout": [],
        }
        rows = [
            parent_a,
            parent_b,
            self._chain_row("out-a", "wallet-a", "outbound", 60, spend_txid, spend_graph),
            self._chain_row("out-b", "wallet-b", "outbound", 40, spend_txid, spend_graph),
            self._chain_row("in-c", "wallet-c", "inbound", 90, spend_txid, spend_graph),
        ]
        owned_index = {
            ("bitcoin", "main", parent_a_txid, 0): {
                "wallet_id": "wallet-a", "amount_msat": 60,
                "branch_label": "receive", "spent_by": spend_txid,
                "asset": "BTC", "ambiguous": False,
            },
            ("bitcoin", "main", parent_b_txid, 0): {
                "wallet_id": "wallet-b", "amount_msat": 40,
                "branch_label": "receive", "spent_by": spend_txid,
                "asset": "BTC", "ambiguous": False,
            },
            ("bitcoin", "main", spend_txid, 0): {
                "wallet_id": "wallet-c", "amount_msat": 90,
                "branch_label": "receive", "spent_by": "",
                "asset": "BTC", "ambiguous": False,
            },
        }

        pairs = derive_utxo_spend_pairs(
            rows, owned_index, skip_row=lambda _row: False
        )
        funding_edges = [pair for pair in pairs if pair["kind"] == "leg_funding"]

        self.assertEqual(
            sorted(pair["allocation_msat"] for pair in funding_edges), [36, 54]
        )
        self.assertTrue(all(pair["confidence"] == "strong" for pair in funding_edges))
        self.assertTrue(all(pair["requires_review"] for pair in funding_edges))

    def test_convergent_passthrough_keeps_every_branch_in_the_lineage(self):
        """A sibling branch must not inherit another branch's visited set."""
        ancestor_txid = "50" * 32
        root_txid = "51" * 32
        common_txid = "52" * 32
        passthrough_txids = ["53" * 32, "54" * 32]
        spend_txid = "55" * 32

        rows = [
            self._chain_row(
                "root",
                "wallet-a",
                "inbound",
                100,
                root_txid,
                {"txid": root_txid, "vin": [], "vout": []},
            ),
            self._chain_row(
                "ancestor-passthrough",
                "wallet-a",
                "outbound",
                0,
                ancestor_txid,
                {
                    "txid": ancestor_txid,
                    "vin": [{"txid": root_txid, "vout": 0}],
                    "vout": [],
                },
            ),
            self._chain_row(
                "common",
                "wallet-a",
                "outbound",
                0,
                common_txid,
                {
                    "txid": common_txid,
                    "vin": [{"txid": ancestor_txid, "vout": 0}],
                    "vout": [],
                },
            ),
        ]
        previous_txid = common_txid
        for index, txid in enumerate(passthrough_txids):
            rows.append(
                self._chain_row(
                    f"passthrough-{index}",
                    "wallet-a",
                    "outbound",
                    0,
                    txid,
                    {
                        "txid": txid,
                        "vin": [{"txid": previous_txid, "vout": 0}],
                        "vout": [],
                    },
                )
            )
            previous_txid = txid
        rows.append(
            self._chain_row(
                "spend",
                "wallet-a",
                "outbound",
                100,
                spend_txid,
                {
                    "txid": spend_txid,
                    # Resolve the direct branch first.  A globally shared
                    # visited set then drops the longer sibling when it
                    # reconverges on ``common``.
                    "vin": [
                        {"txid": common_txid, "vout": 1},
                        {"txid": previous_txid, "vout": 0},
                    ],
                    "vout": [],
                },
            )
        )

        owned_index = {
            ("bitcoin", "main", root_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": ancestor_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", ancestor_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "change",
                "spent_by": common_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", common_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 50,
                "branch_label": "change",
                "spent_by": passthrough_txids[0],
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", common_txid, 1): {
                "wallet_id": "wallet-a",
                "amount_msat": 50,
                "branch_label": "change",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
        }
        for index, txid in enumerate(passthrough_txids):
            next_txid = (
                passthrough_txids[index + 1]
                if index + 1 < len(passthrough_txids)
                else spend_txid
            )
            owned_index[("bitcoin", "main", txid, 0)] = {
                "wallet_id": "wallet-a",
                "amount_msat": 50,
                "branch_label": "change",
                "spent_by": next_txid,
                "asset": "BTC",
                "ambiguous": False,
            }

        pairs = derive_utxo_spend_pairs(
            rows, owned_index, skip_row=lambda _row: False
        )
        parent_edges = [pair for pair in pairs if pair["kind"] == "parent_spend"]

        self.assertEqual(len(parent_edges), 1)
        self.assertEqual(parent_edges[0]["from_row"]["id"], "root")
        self.assertEqual(parent_edges[0]["to_row"]["id"], "spend")
        self.assertEqual(parent_edges[0]["allocation_msat"], 100)
        self.assertIn("consumes 1 owned output", parent_edges[0]["explanation"])
        self.assertIn("535353535353", parent_edges[0]["explanation"])

    def test_passthrough_lineage_is_not_truncated_after_eight_hops(self):
        root_txid = "61" * 32
        passthrough_txids = [
            f"{value:02x}" * 32 for value in range(0x62, 0x6C)
        ]
        spend_txid = "6c" * 32
        rows = [
            self._chain_row(
                "long-root",
                "wallet-a",
                "inbound",
                100,
                root_txid,
                {"txid": root_txid, "vin": [], "vout": []},
            )
        ]
        owned_index = {
            ("bitcoin", "main", root_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "receive",
                "spent_by": passthrough_txids[0],
                "asset": "BTC",
                "ambiguous": False,
            }
        }
        previous_txid = root_txid
        for index, txid in enumerate(passthrough_txids):
            rows.append(
                self._chain_row(
                    f"long-passthrough-{index}",
                    "wallet-a",
                    "outbound",
                    0,
                    txid,
                    {
                        "txid": txid,
                        "vin": [{"txid": previous_txid, "vout": 0}],
                        "vout": [],
                    },
                )
            )
            next_txid = (
                passthrough_txids[index + 1]
                if index + 1 < len(passthrough_txids)
                else spend_txid
            )
            owned_index[("bitcoin", "main", txid, 0)] = {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "change",
                "spent_by": next_txid,
                "asset": "BTC",
                "ambiguous": False,
            }
            previous_txid = txid
        rows.append(
            self._chain_row(
                "long-spend",
                "wallet-a",
                "outbound",
                100,
                spend_txid,
                {
                    "txid": spend_txid,
                    "vin": [{"txid": previous_txid, "vout": 0}],
                    "vout": [],
                },
            )
        )

        pairs = derive_utxo_spend_pairs(
            rows, owned_index, skip_row=lambda _row: False
        )
        parent_edges = [pair for pair in pairs if pair["kind"] == "parent_spend"]

        self.assertEqual(len(parent_edges), 1)
        self.assertEqual(parent_edges[0]["from_row"]["id"], "long-root")
        self.assertEqual(parent_edges[0]["to_row"]["id"], "long-spend")
        self.assertEqual(parent_edges[0]["allocation_msat"], 100)

    def test_malformed_passthrough_cycle_fails_closed(self):
        first_txid = "71" * 32
        second_txid = "72" * 32
        spend_txid = "73" * 32
        rows = [
            self._chain_row(
                "cycle-first",
                "wallet-a",
                "outbound",
                0,
                first_txid,
                {
                    "txid": first_txid,
                    "vin": [{"txid": second_txid, "vout": 0}],
                    "vout": [],
                },
            ),
            self._chain_row(
                "cycle-second",
                "wallet-a",
                "outbound",
                0,
                second_txid,
                {
                    "txid": second_txid,
                    "vin": [{"txid": first_txid, "vout": 0}],
                    "vout": [],
                },
            ),
            self._chain_row(
                "cycle-spend",
                "wallet-a",
                "outbound",
                100,
                spend_txid,
                {
                    "txid": spend_txid,
                    "vin": [{"txid": first_txid, "vout": 1}],
                    "vout": [],
                },
            ),
        ]
        owned_index = {
            ("bitcoin", "main", first_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "change",
                "spent_by": second_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", first_txid, 1): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "change",
                "spent_by": spend_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
            ("bitcoin", "main", second_txid, 0): {
                "wallet_id": "wallet-a",
                "amount_msat": 100,
                "branch_label": "change",
                "spent_by": first_txid,
                "asset": "BTC",
                "ambiguous": False,
            },
        }

        self.assertEqual(
            derive_utxo_spend_pairs(
                rows, owned_index, skip_row=lambda _row: False
            ),
            [],
        )


if __name__ == "__main__":
    unittest.main()
