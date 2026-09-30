"""The owned-script attestation names only the scripts of its own graph."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock

from kassiber.core.chain_observer.provenance import (
    persist_chain_observation_provenance,
    row_has_current_authoritative_observation,
)
from kassiber.core.imports import ImportCoordinatorHooks, insert_wallet_records
from kassiber.core.onchain import graph_scoped_scripts
from kassiber.core.sync_backends import record_from_bitcoin_esplora_tx
from kassiber.db import open_db
from kassiber.time_utils import now_iso

TXID = "ab" * 32
PARENT = "cd" * 32
OWNED_INPUT = "0014" + "11" * 20
OWNED_CHANGE = "0014" + "22" * 20
PAYEE = "0014" + "33" * 20
# Scripts the wallet owns that this transaction never touches.
ELSEWHERE = ["0014" + f"{index:02x}" * 20 for index in range(0x40, 0x50)]
TRACKED = {OWNED_INPUT, OWNED_CHANGE, *ELSEWHERE}
GRAPH = {
    "txid": TXID,
    "vin": [{"txid": PARENT, "vout": 0, "prevout": {"scriptpubkey": OWNED_INPUT, "value": 100_000}}],
    "vout": [
        {"scriptpubkey": PAYEE, "value": 60_000},
        {"scriptpubkey": OWNED_CHANGE, "value": 39_000},
    ],
    "status": {"confirmed": True, "block_height": 100, "block_time": 1_700_000_000},
    "observer": "bdk",
}


def _observer_record(graph=GRAPH):
    record = record_from_bitcoin_esplora_tx(dict(graph), TRACKED, "fulcrum")
    record["amount"] = str(record["amount"])
    record["fee"] = str(record["fee"])
    return record


def _legacy_record():
    """A record as observers wrote it before the attestation was scoped."""
    record = _observer_record()
    raw = json.loads(record["raw_json"])
    raw["observer_owned_scripts"] = sorted(TRACKED)
    record["raw_json"] = json.dumps(raw, sort_keys=True)
    return record


class GraphScopedScriptsTest(unittest.TestCase):
    def test_keeps_only_the_graphs_own_scripts(self):
        self.assertEqual(graph_scoped_scripts(GRAPH, TRACKED), sorted([OWNED_INPUT, OWNED_CHANGE]))

    def test_an_unresolved_input_keeps_every_script(self):
        graph = {**GRAPH, "vin": [*GRAPH["vin"], {"txid": "ef" * 32, "vout": 1}]}
        self.assertEqual(graph_scoped_scripts(graph, TRACKED), sorted(TRACKED))

    def test_observer_records_attest_their_own_scripts_with_unchanged_quantities(self):
        record = _observer_record()
        raw = json.loads(record["raw_json"])
        self.assertEqual(raw["observer_owned_scripts"], sorted([OWNED_INPUT, OWNED_CHANGE]))
        self.assertEqual(record["direction"], "outbound")
        self.assertEqual(record["amount"], "0.0006")
        self.assertEqual(record["fee"], "0.00001")


class LegacyAttestationNarrowingTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="kassiber-owned-scope-")
        self.addCleanup(self.temp.cleanup)
        self.conn = open_db(Path(self.temp.name) / "data")
        self.addCleanup(self.conn.close)
        timestamp = now_iso()
        self.conn.execute("INSERT INTO workspaces(id, label, created_at) VALUES('ws', 'WS', ?)", (timestamp,))
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES('profile', 'ws', 'Profile', 'EUR', 'generic', 365, 'FIFO', ?)
            """,
            (timestamp,),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, label, kind, config_json, created_at)
            VALUES('wallet', 'ws', 'profile', 'Cold', 'descriptor', '{}', ?)
            """,
            (timestamp,),
        )
        self.profile = self.conn.execute("SELECT * FROM profiles WHERE id = 'profile'").fetchone()
        self.wallet = self.conn.execute("SELECT * FROM wallets WHERE id = 'wallet'").fetchone()

    def _insert(self, record, *, authoritative=True):
        outcome = insert_wallet_records(
            self.conn,
            self.profile,
            self.wallet,
            [record],
            "backend:fulcrum",
            ImportCoordinatorHooks(ensure_tag_row=Mock(), invalidate_journals=Mock()),
            commit=False,
            report_updates=True,
            authoritative_chain_observer=authoritative,
        )
        if authoritative:
            persist_chain_observation_provenance(
                self.conn,
                self.profile,
                self.wallet,
                application_revision="apply",
                chain="bitcoin",
                network="regtest",
                entries=[{
                    "external_id": TXID,
                    "asset": "BTC",
                    "direction": "outbound",
                    "observer_ids": ["descriptor:bdk"],
                    "observer_kinds": ["bdk"],
                }],
                resolved_records=outcome.pop("_observer_resolved_records"),
            )
        return outcome

    def _row(self):
        return self.conn.execute(
            """
            SELECT tx.*,
                   proof.authority_version AS observation_authority_version,
                   proof.graph_hash AS observation_graph_hash,
                   proof.quantity_hash AS observation_quantity_hash
            FROM transactions tx
            LEFT JOIN chain_observation_provenance proof ON proof.transaction_id = tx.id
            """
        ).fetchone()

    def test_resync_narrows_a_legacy_row_and_keeps_its_provenance_current(self):
        self._insert(_legacy_record())
        before = self._row()
        self.assertTrue(row_has_current_authoritative_observation(before))

        outcome = self._insert(_observer_record())

        after = self._row()
        self.assertEqual(outcome["attestations_narrowed"], 1)
        self.assertEqual(outcome["unchanged"], 1)
        self.assertNotIn("updated", outcome)
        self.assertEqual(outcome["updated_records"], [])
        self.assertFalse(outcome["journal_invalidated"])
        stored = json.loads(after["raw_json"])
        legacy = json.loads(before["raw_json"])
        self.assertEqual(stored["observer_owned_scripts"], sorted([OWNED_INPUT, OWNED_CHANGE]))
        legacy.pop("observer_owned_scripts")
        stored.pop("observer_owned_scripts")
        self.assertEqual(stored, legacy)
        for column in ("amount", "fee", "direction", "occurred_at", "confirmed_at", "fingerprint"):
            self.assertEqual(after[column], before[column], column)
        self.assertTrue(row_has_current_authoritative_observation(after))

    def test_an_already_scoped_row_is_left_alone(self):
        self._insert(_observer_record())
        outcome = self._insert(_observer_record())
        self.assertNotIn("attestations_narrowed", outcome)

    def test_an_import_without_observer_authority_never_narrows(self):
        self._insert(_legacy_record())
        before = self._row()["raw_json"]

        outcome = self._insert(_observer_record(), authoritative=False)

        self.assertNotIn("attestations_narrowed", outcome)
        self.assertEqual(self._row()["raw_json"], before)

    def test_a_stored_graph_with_an_unresolved_input_keeps_its_full_scope(self):
        incomplete = {**GRAPH, "vin": [*GRAPH["vin"], {"txid": "ef" * 32, "vout": 1}]}
        legacy = _observer_record(incomplete)
        self.assertEqual(json.loads(legacy["raw_json"])["observer_owned_scripts"], sorted(TRACKED))
        self._insert(legacy)
        before = self._row()["raw_json"]

        # Even a narrower claim cannot drop scripts the stored graph cannot vouch for.
        narrower = dict(legacy)
        raw = json.loads(narrower["raw_json"])
        raw["observer_owned_scripts"] = sorted([OWNED_INPUT, OWNED_CHANGE])
        narrower["raw_json"] = json.dumps(raw, sort_keys=True)
        outcome = self._insert(narrower)

        self.assertNotIn("attestations_narrowed", outcome)
        self.assertEqual(self._row()["raw_json"], before)


if __name__ == "__main__":
    unittest.main()
