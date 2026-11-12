import json
import os
from pathlib import Path
import sqlite3
import tempfile
import unittest

from kassiber.db import open_db
from kassiber.secrets.sqlcipher import open_encrypted
from tests import cli_runner
from tests.rust_desktop_parity import run_candidate
from tests.test_rust_shell_parity import _environment
from tests.rust_port.book_schema import TABLES

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KIND = "ui.transactions.resolve"


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the harness build")
class RustTransactionResolveTest(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-r5-resolve-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name)
        self.root = self.base / "data"
        self.passphrase = None
        self.env = _environment(self.base / "preferences")
        self.env["HOME"] = os.environ["HOME"]
        self.env["KASSIBER_STATE_ROOT"] = str(self.base / "state")
        self.env["KASSIBER_TEST_NO_EGRESS"] = "1"
        self.env["KASSIBER_NO_EGRESS"] = "1"
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript((ROOT / "crates/transactions/tests/data/desktop.sql").read_text())
        self.change("")

    def change(self, sql):
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript(sql)

    def snapshot(self):
        path = self.root / "kassiber.sqlite3"
        conn = open_encrypted(path, self.passphrase) if self.passphrase else sqlite3.connect(path)
        try:
            tables = sorted(row[0] for row in conn.execute(
                "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'"))
            self.assertEqual(tuple(tables), TABLES)
            return {table: sorted(conn.execute('SELECT * FROM "' + table + '"').fetchall(), key=repr)
                    for table in tables}
        finally:
            conn.close()

    def compare(self, args):
        requests = [dict(request_id=str(index), kind=KIND, args=arg)
                    for index, arg in enumerate(args)]
        requests.append(dict(request_id="stop", kind="daemon.shutdown"))
        wire = "".join(json.dumps(request) + "\n" for request in requests).encode()
        argv = ["--machine", "--data-root", str(self.root)]
        if self.passphrase:
            argv.extend(["--db-passphrase-fd", cli_runner.InputFd(self.passphrase.encode())])
        argv.append("daemon")
        baseline = self.snapshot()
        reference = cli_runner.run(argv, reference=True, env=self.env, input=wire,
                                   capture_output=True, timeout=120)
        self.assertEqual((reference.returncode, reference.stderr), (0, b""), reference.stdout)
        self.assertEqual(self.snapshot(), baseline)
        candidate = run_candidate(argv, env=self.env, input=wire)
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""), candidate.stdout)
        actual, expected = candidate.stdout.splitlines(), reference.stdout.splitlines()
        ready = json.loads(actual[0])["data"]["supported_kinds"]
        # The native daemon advertises a strict subset of Python's kinds.
        self.assertNotEqual(ready, json.loads(expected[0])["data"]["supported_kinds"],
                            "candidate must be the Rust daemon")
        self.assertIn(KIND, ready)
        self.assertEqual(len(actual), len(expected))
        for request, got, wanted in zip(requests, actual[1:], expected[1:]):
            self.assertEqual(got, wanted, request)
        self.assertEqual(self.snapshot(), baseline)
        responses = [json.loads(line) for line in actual[1:-1]]
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for response in responses:
                    handle.write(json.dumps(dict(kind=KIND, served_by="rust",
                                                 request_id=response["request_id"], ok=response["kind"] != "error",
                                                 error_code=response.get("error", {}).get("code"))) + "\n")
        return responses

    def relation(self):
        self.change("""
            INSERT INTO journal_custody_economic_relations(
                relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,
                target_transaction_id,source_asset,target_asset,source_amount_msat,
                target_amount_msat,basis_state,occurred_at,created_at)
            VALUES('aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'w','p','conversion','g','b','BTC','LBTC',100001,100001,'eligible','2025','2025');
            UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,
                journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=10 WHERE id='p';
        """)

    def test_aliases_order_refusals_redaction_and_all_tables_match_wire_bytes(self):
        self.compare([None, [], 1, False, "query", {}, {"query": " "}, {"query": 1},
                      {"query": "a", "z": 1, "extra": 2},
                      *({"query": query} for query in (
                          "a", "A", " a ", "d", "external-E", "PAYMENT-HASH", "absent", "foreign"))])
        self.change("UPDATE transactions SET external_id='a' WHERE id='e';")
        result = self.compare([{"query": "a"}])[0]
        self.assertEqual(result["data"]["transaction"]["id"], "a")
        self.change("UPDATE transactions SET external_id='duplicate' WHERE id IN ('b','e','g');")
        result = self.compare([{"query": "duplicate"}])[0]
        self.assertEqual(result["data"]["transaction"]["id"], "b")
        self.change("INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,created_at) VALUES('A','w','p','cold','case-id','BTC','inbound',2001,0,'2020','2020');")
        result = self.compare([{"query": "A"}])[0]
        self.assertEqual(result["data"]["transaction"]["id"], "A")
        self.change("DELETE FROM settings WHERE key IN ('context_workspace','context_profile');")
        self.compare([None, [], {"query": "a"}, {"z": 1}])

    def test_focused_leg_preserves_principal_pair_fee_and_whole_book_queue(self):
        self.relation()
        result = self.compare([{"query": query} for query in
                               ("b", "B", "g", "payment-hash", "external-g")])
        focused = result[0]["data"]["transaction"]
        aggregate = result[3]["data"]["transaction"]
        self.assertEqual(focused["amountSat"], 100.001)
        self.assertEqual(focused["queueStatus"], "review")
        self.assertEqual(focused["pair"]["feeSat"], focused["feeSat"])
        self.assertEqual(aggregate["amountSat"], -100.001)
        self.assertIsNone(focused["pair"]["reviewPairId"])
        for marker in ("last_processed_tx_count=0", "last_processed_input_version=1", "last_processed_at=NULL",
                       "journal_semantics_version=1"):
            self.change("UPDATE profiles SET " + marker + " WHERE id='p';")
            result = self.compare([{"query": "b"}, {"query": "payment-hash"}])
            self.assertNotIn("pair", result[0]["data"]["transaction"])
            self.change("UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=10 WHERE id='p';")

    def test_review_reading_fingerprint_binds_observations_and_unique_review(self):
        self.change("UPDATE wallets SET config_json='{\"network\":\"regtest\"}' WHERE id='ln';")
        paired = cli_runner.run(
            ["--machine", "--data-root", str(self.root), "transfers", "pair", "--tx-out", "g", "--tx-in", "b"],
            reference=True, env=self.env, capture_output=True, timeout=120)
        self.assertEqual(paired.returncode, 0, paired.stdout)
        pair_id = json.loads(paired.stdout)["data"]["id"]
        self.relation()
        first = self.compare([{"query": "b"}, {"query": "g"}])[0]["data"]["transaction"]["pair"]
        self.assertEqual(first["reviewPairId"], pair_id)
        self.assertRegex(first["pairFingerprint"], "^[0-9a-f]{64}$")
        self.change("UPDATE transactions SET external_id=NULL WHERE id='b';")
        changed = self.compare([{"query": "b"}])[0]["data"]["transaction"]["pair"]
        self.assertNotEqual(first["pairFingerprint"], changed["pairFingerprint"])
        updated = cli_runner.run(
            ["--machine", "--data-root", str(self.root), "transfers", "update", "--pair-id", pair_id, "--note", "note only"],
            reference=True, env=self.env, capture_output=True, timeout=120)
        self.assertEqual(updated.returncode, 0, updated.stdout)
        self.change("UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=7,journal_input_version=2,last_processed_input_version=2 WHERE id='p'; UPDATE profiles SET journal_semantics_version=10 WHERE id='p';")
        note = self.compare([{"query": "b"}])[0]["data"]["transaction"]["pair"]
        self.assertEqual(changed["pairFingerprint"], note["pairFingerprint"])
        self.change("DELETE FROM custody_component_transaction_memberships;")
        removed = self.compare([{"query": "b"}])[0]["data"]["transaction"]["pair"]
        self.assertIsNone(removed["reviewPairId"])

    def test_sqlcipher_resolution_matches_wire_bytes_and_all_tables(self):
        self.root = self.base / "encrypted" / "data"
        self.passphrase = "synthetic resolver key"
        with open_db(self.root, passphrase=self.passphrase) as conn:
            conn.executescript((ROOT / "crates/transactions/tests/data/desktop.sql").read_text())
        self.change("")
        self.relation()
        self.compare([{"query": query} for query in ("b", "g", "payment-hash", "a", "foreign")])
