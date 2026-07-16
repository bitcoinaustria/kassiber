"""Self-transfer export labels must not cross-contaminate unrelated pairs."""

from __future__ import annotations

import json
import sqlite3
import unittest

from kassiber.core.reports import _self_transfer_legs_by_transaction


def _conn_with_two_blank_description_transfers():
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE wallets (
          id TEXT PRIMARY KEY, label TEXT, profile_id TEXT,
          kind TEXT, config_json TEXT
        );
        CREATE TABLE transactions (
          id TEXT PRIMARY KEY, profile_id TEXT, wallet_id TEXT,
          external_id TEXT, kind TEXT, asset TEXT, direction TEXT, amount INTEGER,
          payment_hash TEXT, payment_hash_source TEXT, raw_json TEXT,
          excluded INTEGER DEFAULT 0
        );
        CREATE TABLE transaction_pairs (
          id TEXT PRIMARY KEY, profile_id TEXT,
          out_transaction_id TEXT, in_transaction_id TEXT,
          policy TEXT, deleted_at TEXT
        );
        CREATE TABLE journal_entries (
          id TEXT PRIMARY KEY, profile_id TEXT, transaction_id TEXT,
          wallet_id TEXT, entry_type TEXT, occurred_at TEXT,
          description TEXT, asset TEXT, quantity INTEGER
        );
        CREATE TABLE journal_quarantines (
          transaction_id TEXT, profile_id TEXT
        );
        CREATE TABLE direct_swap_payouts (
          profile_id TEXT, out_transaction_id TEXT, out_amount INTEGER,
          deleted_at TEXT
        );
        CREATE TABLE journal_custody_economic_relations (
          profile_id TEXT, relation_kind TEXT, source_transaction_id TEXT,
          target_transaction_id TEXT, policy TEXT, basis_state TEXT
        );
        """
    )
    for wallet_id, label in (
        ("wa", "A"),
        ("wb", "B"),
        ("wc", "C"),
        ("wd", "D"),
    ):
        conn.execute(
            "INSERT INTO wallets VALUES (?, ?, ?, 'descriptor', ?)",
            (wallet_id, label, "p1", '{"chain":"bitcoin","network":"main"}'),
        )
    # Two same-timestamp transfers with blank descriptions: A->B and C->D.
    ts = "2026-01-01T12:00:00Z"
    legs = (
        ("ta", "wa", "outbound", 100_000_000_000, "aa" * 32),
        ("tb", "wb", "inbound", 100_000_000_000, "aa" * 32),
        ("tc", "wc", "outbound", 200_000_000_000, "bb" * 32),
        ("td", "wd", "inbound", 200_000_000_000, "bb" * 32),
    )
    for tx_id, wallet_id, direction, amount, physical_txid in legs:
        conn.execute(
            "INSERT INTO transactions("
            "id, profile_id, wallet_id, external_id, kind, asset, direction, amount, "
            "payment_hash, payment_hash_source, raw_json, excluded"
            ") VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 0)",
            (
                tx_id,
                "p1",
                wallet_id,
                physical_txid,
                "onchain",
                "BTC",
                direction,
                amount,
                None,
                None,
                json.dumps(
                    {
                        "txid": physical_txid,
                        "chain": "bitcoin",
                        "network": "main",
                    }
                ),
            ),
        )
    # Outgoing transfer quantities include their network fees; incoming
    # quantities contain only what arrived. The two blank-description pairs
    # must still be matched independently.
    journal = (
        ("ja", "ta", "wa", "transfer_out", -100_100_000_000),
        ("jb", "tb", "wb", "transfer_in", 100_000_000_000),
        ("jc", "tc", "wc", "transfer_out", -200_200_000_000),
        ("jd", "td", "wd", "transfer_in", 200_000_000_000),
    )
    for entry_id, tx_id, wallet_id, entry_type, qty in journal:
        conn.execute(
            "INSERT INTO journal_entries VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            (entry_id, "p1", tx_id, wallet_id, entry_type, ts, "", "BTC", qty),
        )
    return conn


class SelfTransferLabelTests(unittest.TestCase):
    def test_blank_description_same_timestamp_does_not_cross_contaminate(self):
        conn = _conn_with_two_blank_description_transfers()
        labels = _self_transfer_legs_by_transaction(
            conn, {"id": "p1"}, journals_current=True
        )
        self.assertEqual(labels["ta"], "B")
        self.assertEqual(labels["tb"], "A")
        self.assertEqual(labels["tc"], "D")
        self.assertEqual(labels["td"], "C")

    def test_stale_journal_does_not_redetect_physical_transfer_metadata(self):
        conn = _conn_with_two_blank_description_transfers()

        labels = _self_transfer_legs_by_transaction(
            conn, {"id": "p1"}, journals_current=False
        )

        self.assertEqual(labels, {})

    def test_stale_journal_does_not_redetect_native_lightning_payment_hash(self):
        conn = _conn_with_two_blank_description_transfers()
        payment_hash = "11" * 32
        provenance = json.dumps(
            {"_kassiber_provenance": {"import_source": "lnd"}}
        )
        conn.execute(
            "UPDATE transactions SET external_id = 'ln-out', kind = 'lnd_pay', "
            "payment_hash = ?, payment_hash_source = 'lnd', raw_json = ? WHERE id = 'ta'",
            (payment_hash, provenance),
        )
        conn.execute(
            "UPDATE transactions SET external_id = 'ln-in', kind = 'lnd_invoice', "
            "payment_hash = ?, payment_hash_source = 'lnd', raw_json = ? WHERE id = 'tb'",
            (payment_hash, provenance),
        )

        labels = _self_transfer_legs_by_transaction(
            conn, {"id": "p1"}, journals_current=False
        )

        self.assertEqual(labels, {})


if __name__ == "__main__":
    unittest.main()
