import sqlite3
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch
from urllib import request as urlrequest

from kassiber.egress_ledger import (
    EgressAllowlistEntry,
    EgressLedger,
    db_header_proof,
    get_egress_ledger,
)
from kassiber.proxy import urlopen_with_proxy


class EgressLedgerTest(unittest.TestCase):
    def test_snapshot_annotates_expected_built_in_and_unexpected(self):
        ledger = EgressLedger()
        ledger.record(
            subsystem="sync",
            host="NODE.EXAMPLE",
            port=50002,
            scheme="electrum",
            operation="socket.connect",
        )
        ledger.record(
            subsystem="pricing",
            host="api.coingecko.com",
            port=443,
            scheme="https",
            operation="http.request",
            bytes_out=42,
        )
        ledger.record(
            subsystem="ai",
            host="surprise.example",
            port=443,
            scheme="https",
            operation="http.request",
        )

        snapshot = ledger.snapshot(
            allowlist=[
                EgressAllowlistEntry(
                    host="node.example",
                    port=50002,
                    subsystem="sync",
                    label="backend:node",
                    source="database",
                    user_allowlisted=True,
                ),
                EgressAllowlistEntry(
                    host="api.coingecko.com",
                    port=443,
                    subsystem="pricing",
                    label="CoinGecko",
                    source="built-in",
                    user_allowlisted=False,
                ),
            ],
            allowlist_complete=True,
        )

        records = snapshot["records"]
        self.assertEqual(records[0]["allowlist_status"], "expected")
        self.assertTrue(records[0]["user_allowlisted"])
        self.assertEqual(records[1]["allowlist_status"], "expected")
        self.assertFalse(records[1]["user_allowlisted"])
        self.assertEqual(records[2]["allowlist_status"], "unexpected")
        self.assertEqual(snapshot["summary"]["unexpected"], 1)
        self.assertEqual(snapshot["summary"]["by_subsystem"]["pricing"]["bytes_out"], 42)

    def test_port_specific_allowlist_does_not_match_unknown_or_different_ports(self):
        entry = EgressAllowlistEntry(
            host="node.example",
            port=50002,
            subsystem="sync",
            label="backend:node",
        )

        self.assertTrue(entry.matches("node.example", 50002, "sync"))
        self.assertFalse(entry.matches("node.example", None, "sync"))
        self.assertFalse(entry.matches("node.example", 50001, "sync"))
        wildcard = EgressAllowlistEntry(
            host="node.example",
            port=None,
            subsystem="sync",
            label="backend:any-port",
        )
        self.assertTrue(wildcard.matches("node.example", None, "sync"))
        self.assertTrue(wildcard.matches("node.example", 50002, "sync"))

    def test_db_header_proof_distinguishes_plaintext_sqlite_header(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-egress-") as tmp:
            db_path = Path(tmp) / "plain.sqlite3"
            conn = sqlite3.connect(db_path)
            conn.execute("CREATE TABLE t(x INTEGER)")
            conn.close()

            proof = db_header_proof(db_path)
            self.assertTrue(proof["exists"])
            self.assertTrue(proof["sqlite_plaintext_header"])
            self.assertFalse(proof["encrypted_like"])
            self.assertEqual(proof["classification"], "plaintext-sqlite")

            encrypted_like = Path(tmp) / "cipher.sqlite3"
            encrypted_like.write_bytes(b"\x01\x02\x03\x04" * 8)
            proof = db_header_proof(encrypted_like)
            self.assertFalse(proof["sqlite_plaintext_header"])
            self.assertTrue(proof["encrypted_like"])
            self.assertEqual(proof["classification"], "ciphertext-like")

    def test_urlopen_with_proxy_records_metadata_without_url_or_headers(self):
        ledger = get_egress_ledger()
        cursor = ledger.snapshot()["last_id"]
        request = urlrequest.Request(
            "https://node.example/api/address/bc1qsecret?token=abc",
            data=b'{"txid":"secret"}',
            method="POST",
            headers={"Authorization": "Bearer secret-token"},
        )

        class Response:
            status = 200
            reason = "OK"
            headers = {}

            def read(self, *args):
                return b"{}"

            def __enter__(self):
                return self

            def __exit__(self, exc_type, exc, tb):
                return False

        with patch("kassiber.proxy.urlrequest.urlopen", return_value=Response()):
            with urlopen_with_proxy(request, request.full_url, timeout=1) as response:
                self.assertEqual(response.read(), b"{}")

        records = ledger.snapshot(after_id=cursor, limit=10)["records"]
        self.assertEqual(len(records), 1)
        record = records[0]
        self.assertEqual(record["host"], "node.example")
        self.assertEqual(record["subsystem"], "sync")
        self.assertEqual(record["method"], "POST")
        self.assertGreater(record["bytes_out"], 0)
        self.assertNotIn("bc1qsecret", repr(record))
        self.assertNotIn("secret-token", repr(record))


if __name__ == "__main__":
    unittest.main()
