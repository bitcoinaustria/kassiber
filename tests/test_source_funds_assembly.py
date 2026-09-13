import json
import sqlite3
import unittest

from kassiber.core.source_funds_assembly import build_owned_outpoint_index


class SourceFundsOwnedOutpointIndexTests(unittest.TestCase):
    def test_missing_legacy_network_uses_chain_default(self):
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
            INSERT INTO wallet_utxos VALUES(
                'profile', 'wallet', 'bitcoin', 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (txid,),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(list(index), [("bitcoin", "main", txid, 0)])

    def test_identical_outpoints_on_different_networks_stay_separate(self):
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
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'bitcoin', ?, 'BTC', ?, 0, 100,
                'receive', NULL
            )
            """,
            (("main-wallet", "main", txid), ("regtest-wallet", "regtest", txid)),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(
            set(index),
            {("bitcoin", "main", txid, 0), ("bitcoin", "regtest", txid, 0)},
        )

    def test_liquid_requires_consensus_asset_identity(self):
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
            INSERT INTO wallet_utxos VALUES(
                'profile', ?, 'liquid', 'liquidv1', 'LBTC', ?, ?, 100,
                'receive', NULL, ?
            )
            """,
            (
                ("known", txid, 0, json.dumps({"asset_id": asset_id})),
                ("unknown", txid, 1, "{}"),
            ),
        )
        try:
            index = build_owned_outpoint_index(conn, "profile")
        finally:
            conn.close()
        self.assertEqual(set(index), {("liquid", "liquidv1", txid, 0)})
        self.assertEqual(index[("liquid", "liquidv1", txid, 0)]["asset_identity"], asset_id)


def _script(tag: str) -> str:
    return "0014" + (tag * 40)[:40]


def _authoritative(row: dict) -> dict:
    """Attach the closed observation commitment an authoritative apply writes."""
    from kassiber.core.chain_observer.provenance import (
        AUTHORITY_VERSION,
        canonical_graph_hash,
        canonical_observed_quantity_hash,
    )

    row = dict(row)
    row["observation_authority_version"] = AUTHORITY_VERSION
    row["observation_graph_hash"] = canonical_graph_hash(row["raw_json"])
    row["observation_quantity_hash"] = canonical_observed_quantity_hash(row)
    return row


def _parent_row(txid: str, amount_msat: int, *, wallet: str = "w", authoritative: bool = True) -> dict:
    raw = {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "observer_owned_scripts": [_script("a")],
        "vin": [],
        "vout": [{"n": 0, "scriptpubkey": _script("a"), "value": amount_msat // 1000}],
    }
    row = {
        "id": f"tx-{txid[:8]}",
        "wallet_id": wallet,
        "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
        "external_id": txid,
        "external_id_kind": "txid",
        "direction": "inbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": 0,
        "amount_includes_fee": 0,
        "occurred_at": "2026-01-01T00:00:00Z",
        "raw_json": json.dumps(raw, sort_keys=True),
    }
    return _authoritative(row) if authoritative else row


def _spend_row(txid: str, parents, amount_msat: int, fee_msat: int, *, wallet: str = "w",
               authoritative: bool = True, owned_scripts=None) -> dict:
    raw = {
        "txid": txid,
        "chain": "bitcoin",
        "network": "main",
        "observer_owned_scripts": owned_scripts if owned_scripts is not None else [_script("a")],
        "vin": [
            {
                "txid": parent_txid,
                "vout": 0,
                "prevout": {"scriptpubkey": _script("a"), "value": value_msat // 1000},
            }
            for parent_txid, value_msat in parents
        ],
        "vout": [{"n": 0, "scriptpubkey": _script("f"), "value": amount_msat // 1000}],
    }
    row = {
        "id": f"tx-{txid[:8]}",
        "wallet_id": wallet,
        "wallet_config_json": json.dumps({"chain": "bitcoin", "network": "main"}),
        "external_id": txid,
        "external_id_kind": "txid",
        "direction": "outbound",
        "asset": "BTC",
        "amount": amount_msat,
        "fee": fee_msat,
        "amount_includes_fee": 0,
        "occurred_at": "2026-02-01T00:00:00Z",
        "raw_json": json.dumps(raw, sort_keys=True),
    }
    return _authoritative(row) if authoritative else row


def _index(parents, *, wallet: str = "w"):
    return {
        ("bitcoin", "main", txid, 0): {
            "wallet_id": wallet, "amount_msat": value, "branch_label": "receive",
            "spent_by": "", "asset": "BTC", "asset_identity": "BTC", "ambiguous": False,
        }
        for txid, value in parents
    }


class ParentSpendDerivationTests(unittest.TestCase):
    """The shape a real consolidation spend has: inputs sum to amount + fee."""

    def setUp(self):
        from kassiber.core.source_funds_assembly import derive_parent_spend_pairs

        self.derive = derive_parent_spend_pairs
        self.parents = [("a1" * 32, 200_000_000), ("b2" * 32, 300_000_000), ("c3" * 32, 500_000_000)]
        self.amount = 999_900_000
        self.fee = 100_000
        self.spend = _spend_row("d4" * 32, self.parents, self.amount, self.fee)
        self.rows = [_parent_row(t, v) for t, v in self.parents] + [self.spend]

    def test_owned_inputs_become_funding_edges_that_exactly_cover_the_spend(self):
        pairs = self.derive(self.rows, _index(self.parents), skip_row=lambda row: False)

        self.assertEqual(len(pairs), 3)
        # Gross upstream demand is every owned input, including the fee.
        self.assertEqual(sum(p["from_allocation_msat"] for p in pairs), 1_000_000_000)
        # Allocations cover the target exactly, so the report cannot raise
        # ambiguous_allocation for a spend whose lineage is fully observed.
        self.assertEqual(sum(p["allocation_msat"] for p in pairs), self.amount)
        self.assertTrue(all(p["to_row"] is self.spend for p in pairs))

    def test_an_unowned_input_emits_nothing_rather_than_a_partial_history(self):
        """Partial cover would raise ambiguous_allocation, which has no fix-it path."""
        partial = _index(self.parents[:2])

        self.assertEqual(self.derive(self.rows, partial, skip_row=lambda row: False), [])

    def test_a_row_without_observation_authority_founds_no_lineage(self):
        """A hand-written vin array must never author provenance."""
        rows = [_parent_row(t, v) for t, v in self.parents] + [
            _spend_row("d4" * 32, self.parents, self.amount, self.fee, authoritative=False)
        ]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])

    def test_a_parent_without_observation_authority_founds_no_lineage(self):
        rows = [_parent_row(self.parents[0][0], self.parents[0][1], authoritative=False)] + [
            _parent_row(t, v) for t, v in self.parents[1:]
        ] + [self.spend]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])

    def test_inputs_owned_by_another_wallet_stay_a_custody_question(self):
        foreign = dict(_index(self.parents))
        foreign[("bitcoin", "main", self.parents[0][0], 0)] = {
            **foreign[("bitcoin", "main", self.parents[0][0], 0)], "wallet_id": "other",
        }

        self.assertEqual(self.derive(self.rows, foreign, skip_row=lambda row: False), [])

    def test_a_privacy_boundary_row_is_never_traversed(self):
        self.assertEqual(
            self.derive(self.rows, _index(self.parents), skip_row=lambda row: True), [],
        )

    def test_an_ambiguous_owned_outpoint_blocks_the_whole_spend(self):
        ambiguous = dict(_index(self.parents))
        ambiguous[("bitcoin", "main", self.parents[1][0], 0)] = {
            **ambiguous[("bitcoin", "main", self.parents[1][0], 0)], "ambiguous": True,
        }

        self.assertEqual(self.derive(self.rows, ambiguous, skip_row=lambda row: False), [])

    def test_a_truncated_graph_cannot_prove_complete_ownership(self):
        """Fewer parsed inputs than declared means the evidence is incomplete."""
        spend = _spend_row("d4" * 32, self.parents, self.amount, self.fee)
        raw = json.loads(spend["raw_json"])
        raw["vin"].append({"txid": "ee" * 32, "vout": 0})  # declared, unparseable prevout
        spend["raw_json"] = json.dumps(raw, sort_keys=True)
        spend = _authoritative(spend)
        rows = [_parent_row(t, v) for t, v in self.parents] + [spend]

        self.assertEqual(self.derive(rows, _index(self.parents), skip_row=lambda row: False), [])
