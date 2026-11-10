"""Qualified transaction references through the Rust CLI and daemon (S2).

Every leaf and kind that takes a transaction reference resolves it through
one lookup (`core/transaction_lookup.py`): an id, a unique external id, or a
qualified `<txid>:<wallet>:<in|out>`; a shared txid fails closed with every
candidate. Each case runs the Python reference and the Rust binary on copies
of one book and compares the bytes and all 113 tables, plaintext and
SQLCipher.
"""
import json
from pathlib import Path
import unittest

from tests.rust_desktop_parity import RUST_BIN
from tests.rust_port.sync_parity import SyncParity

ROOT = Path(__file__).resolve().parent.parent
VECTORS = json.loads((ROOT / "crates/transactions/tests/data/lookup-vectors.json").read_text())
SHARED = "a" * 64
MANY = "b" * 64
REFERENCES = [
    SHARED, f"{SHARED}:out", f"{SHARED}:cold b", f"{SHARED}:Cold C:inbound",
    f"{SHARED}:in", f"{SHARED}:Cold B:out", " tx-b-in ", "kraken:L123:in",
    "dup:1:out", MANY, "missing", "",
]


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary")
class TransactionReferenceParity(SyncParity, unittest.TestCase):
    SQL = VECTORS["sql"]

    def test_reads_resolve_qualified_references_like_python(self):
        commands = []
        for reference in REFERENCES:
            commands += [
                ["metadata", "records", "get", "--transaction", reference],
                ["attachments", "list", "--transaction", reference],
                ["transfers", "components", "list", "--transaction", reference],
                ["source-funds", "review-context", "--target-transaction", reference],
            ]
        envelopes = self.cli(commands, mutates=False)
        by_reference = dict(zip(REFERENCES, envelopes[::4]))
        self.assertEqual(by_reference[f"{SHARED}:cold b"]["data"]["transaction_id"], "tx-b-in")
        self.assertEqual(by_reference[SHARED]["error"]["code"], "ambiguous_reference")
        self.assertEqual(by_reference[MANY]["error"]["details"]["candidates_truncated"], True)
        self.assertEqual(
            by_reference[f"{SHARED}:Cold B:out"]["error"]["code"], "reference_qualifier_unmatched"
        )

    def test_writes_resolve_qualified_references_like_python(self):
        envelopes = self.cli([
            ["metadata", "notes", "set", "--transaction", f"{SHARED}:in", "--note", "refused"],
            ["metadata", "notes", "set", "--transaction", f"{SHARED}:cold b", "--note", "cold"],
            ["metadata", "tags", "create", "--code", "mix", "--label", "Mix"],
            ["metadata", "tags", "add", "--transaction", f"{SHARED}:out", "--tag", "mix"],
            ["attachments", "add", "--transaction", f"{SHARED}:wallet-c:in", "--url",
             "https://example.invalid/receipt"],
            ["transfers", "pair", "--tx-out", f"{SHARED}:Mixer Out", "--tx-in", f"{SHARED}:cold b"],
            ["transfers", "pair", "--tx-out", "dup:1:out", "--tx-in", "dup:wallet-b"],
            ["source-funds", "links", "create", "--to-transaction", f"{SHARED}:cold b",
             "--from-transaction", f"{SHARED}:out", "--type", "self_transfer"],
        ], hashes=True)
        self.assertEqual(envelopes[0]["error"]["code"], "ambiguous_reference")
        self.assertEqual(envelopes[1]["data"]["transaction_id"], "tx-b-in")
        self.assertEqual(envelopes[5]["data"]["out_transaction_id"], "tx-a-out")

    def test_daemon_kinds_resolve_qualified_references_like_python(self):
        resolve = self.compare(
            [{"query": reference} for reference in [
                SHARED, f"{SHARED}:Cold B", f"{SHARED}:in", f"{SHARED}:nope",
                "kraken:L123", "tx-colon", MANY, "dup:1:in",
            ]],
            kind="ui.transactions.resolve",
        )
        self.assertEqual(resolve[0]["data"]["sharedExternalIdCount"], 3)
        self.assertEqual(
            {item["transaction_id"] for item in resolve[0]["data"]["sharedExternalIdMatches"]},
            {"tx-a-out", "tx-b-in"},
        )
        self.assertEqual(resolve[1]["data"]["transaction"]["id"], "tx-b-in")
        self.assertEqual(resolve[2]["error"]["code"], "ambiguous_reference")
        self.assertEqual(resolve[6]["data"]["sharedExternalIdCount"], 21)
        self.compare(
            [{"transaction": reference} for reference in [f"{SHARED}:cold b", SHARED, "missing"]],
            kind="ui.transactions.review_context",
        )
        self.compare(
            [{"transaction": reference} for reference in [f"{SHARED}:out", SHARED, 5, [], True]],
            kind="ui.transfers.components.list",
        )
        evidence = self.masked(
            [{"transaction": f"{SHARED}:cold b"}, {"transactions": [SHARED]},
             {"transactions": [f"{SHARED}:out", "tx-colon"]}, {"transaction": "missing"}],
            kind="ui.audit.evidence.summary",
        )
        self.assertEqual(evidence[1]["error"]["code"], "ambiguous_reference")
        quarantine = self.masked(
            [{"transaction": reference, "action": "exclude", "reason": "reviewed"}
             for reference in [f"{SHARED}:cold b", SHARED, f"{SHARED}:nope", "missing"]],
            kind="ui.journals.quarantine.resolve",
        )
        self.assertEqual(quarantine[0]["error"]["message"], "Transaction is not quarantined")
        self.assertEqual(quarantine[1]["error"]["code"], "ambiguous_reference")


class EncryptedTransactionReferenceParity(TransactionReferenceParity):
    PASSPHRASE = "sync-s2-passphrase"


if __name__ == "__main__":
    unittest.main()
