"""Desktop pages compare raw Python/Rust JSONL on the same disposable book."""
import json
import os
import subprocess
import unittest

from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from tests import cli_runner
from tests import test_rust_transaction_listing as listing

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")
KINDS = ["ui.transactions." + suffix for suffix in ("list", "search", "extremes")]


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustTransactionDesktopTest(unittest.TestCase):
    cli = listing.RustTransactionListingTest.cli
    ok = listing.RustTransactionListingTest.ok
    def setUp(self):
        listing.RustTransactionListingTest.setUp(self)
        conn = open_db(str(self.root))
        conn.execute("UPDATE transactions SET amount=9007199254740995 WHERE id='a'")
        conn.commit()
        conn.close()

    def compare(self, requests):
        requests = [*requests, {"kind": "daemon.shutdown"}]
        wire = "".join(json.dumps({"request_id": str(i), **r}) + "\n"
                       for i, r in enumerate(requests)).encode()
        argv = ["--machine", "--data-root", str(self.root), "daemon"]
        options = dict(input=wire, capture_output=True, timeout=120)
        reference = cli_runner.run(argv, reference=True, env=self.env, **options)
        candidate = (cli_runner.run(argv, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, **options)
                     if cli_runner.candidate_prefix() else subprocess.run([RUST_BIN, *argv], env=self.env, **options))
        self.assertEqual((reference.returncode, reference.stderr), (0, b""))
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""))
        actual, expected = candidate.stdout.splitlines(), reference.stdout.splitlines()
        self.assertEqual(len(actual), len(expected))
        ready = json.loads(actual[0])["data"]["supported_kinds"]
        for kind in KINDS:
            self.assertIn(kind, ready)
        for request, got, wanted in zip(requests, actual[1:], expected[1:]):
            self.assertEqual(got, wanted, request)
        if path := os.environ.get("KASSIBER_DESKTOP_COUNTS"):
            with open(path, "a", encoding="utf-8") as handle:
                for request in requests[:-1]:
                    handle.write(json.dumps({"kind": request["kind"], "served_by": "rust"}) + "\n")
        return [json.loads(line) for line in actual[1:-1]]

    def paired(self):
        conn = open_db(str(self.root))
        workspace, profile = conn.execute("SELECT workspace_id,id FROM profiles").fetchone()
        conn.execute("""INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,occurred_at,created_at)
            VALUES(?,?,?,'conversion','out','b','BTC','LBTC',100000001,100001,'eligible','2025','2025')""", ("a" * 64, workspace, profile))
        conn.execute("UPDATE transactions SET review_status='review' WHERE id='out'")
        conn.execute("UPDATE transactions SET confirmed_at=occurred_at,review_status='completed' WHERE id='b'")
        conn.execute("UPDATE profiles SET last_processed_at='2025',last_processed_tx_count=(SELECT COUNT(*) FROM transactions WHERE excluded=0),journal_input_version=2,last_processed_input_version=2")
        # Stamp after the markers; the clearing trigger fires on marker writes.
        conn.execute("UPDATE profiles SET journal_semantics_version=?", (JOURNAL_CALCULATION_SEMANTICS_VERSION,))
        conn.commit()
        conn.close()

    def test_filters_refusals_and_redaction_match_raw_jsonl(self):
        filters = [{}, {"wallet": " Cold "}, {"wallet": "absent"}, {"asset": "btc"},
                   {"txids": "A,a, in,,OUT"}, {"txids": ["A", "a", "out"]},
                   {"since": "2025-02-01T00:00:00.000001Z"},
                   {"until": "2025-02-01T00:00:00.000001Z"},
                   {"start": "2025-02-01T02:00:00.000001+02:00"},
                   {"end": "2025-02-01T02:00:00.000001+02:00"},
                   {"withFees": "yes"}, {"with_fees": 1.0},
                   {"since": "bad"}, {"until": "bad"}, {"sort": "bad"}, {"order": "bad"}, {"sort": []}, {"sort": {}}, {"order": []}, {"order": {}},
                   {"unexpected": True}, {"limit": None}, {"limit": "bad"}, {"limit": 0},
                   {"limit": -1}, {"limit": 99999999999999999999999999}, {"limit": 2.9},
                   {"cursor": "broken"}, {"cursor": []}, {"query": " "}, {"query": 1},
                   {"wallet": []}, {"txids": [None]}, {"txids": False}, {"candidate_txids": 2},
                   {"txids": [str(i) for i in range(129)]},
                   {"candidate_txids": [str(i) for i in range(513)]},
                   {"direction": "bad"}, {"asset": " "}, {"status": "bad"}, {"flow": "bad"},
                   {"period": "bad"}, {"quick": "bad"}, {"payment_method": "bad"}, {"withFees": None},
                   {"query": "First"}, {"query": "%"}, {"query": "PAYMENT-HASH"},
                   {"query": "Cold"}, {"query": "deposit"},
                   {"since": "2024", "start": []}, {"payment_method": "Liquid", "paymentMethod": []}]
        for key, values in (
            ("direction", ["inbound", "outbound"]),
            ("status", ["completed", "pending", "failed", "review", "needs-review", "error"]),
            ("flow", ["incoming", "outgoing", "transfer", "swap", "layer_transition"]),
            ("paymentMethod", ["On-chain", "Exchange", "Lightning", "Liquid"]),
            ("network", ["onchain", "Lightning", "liquid", "kraken", "btc", "regtest"]),
            ("quick", ["external_flow", "review_queue", "no_explorer_id", "missing_price", "failed_import"]),
            ("period", ["30days", "3months", "6months", "ytd", "1year", "5years", "10years", "15years", "all", "max", "30-days"]),
        ):
            filters.extend({key: value} for value in values)
        conn = open_db(str(self.root))
        conn.execute("UPDATE transactions SET note='api_key=synthetic-private-token',description='Synced from private' WHERE id='a'")
        conn.commit()
        conn.close()
        self.compare([{"kind": KINDS[0], "args": args} for args in filters])
        self.compare([{"kind": KINDS[1], "args": args} for args in
                      ({}, {"query": "First"}, {"query": "%", "sort": []}, {"query": "%", "sort": {}}, {"query": "%", "order": []}, {"query": "%", "order": {}}, {"query": "deposit", "limit": 1000}, {"query": "in", "with_fees": True}, {"query": "a", "unexpected": True})])
        self.compare([{"kind": KINDS[2], "args": args} for args in
                      ({}, {"limit": 1}, {"limit": 100}, {"direction": "outbound"}, {"asset": "BTC"}, {"wallet": "ln"}, {"since": "2025-02-01T00:00:00.000001Z"}, {"until": "2025"}, {"limit": None})])
        self.compare([{"kind": kind, "args": []} for kind in KINDS])

    def test_pages_grouped_totals_queue_scope_and_cursor_bytes(self):
        self.paired()
        for kind in KINDS[:2]:
            for sort in ("occurred-at", "amount", "fee", "fiat-value"):
                for order in ("asc", "desc"):
                    args = {"limit": 1, "sort": sort, "order": order}
                    if kind == KINDS[1]:
                        args["query"] = "%"
                    seen = []
                    cursor = None
                    for _ in range(10):
                        page = self.compare([{"kind": kind, "args": {**args, **({"cursor": cursor} if cursor else {})}}])[0]["data"]
                        self.assertEqual(page["count"], 6)
                        seen.extend(tx["id"] for tx in page["txs"])
                        cursor = page["nextCursor"]
                        if not cursor:
                            self.assertFalse(page["hasMore"])
                            break
                    else:
                        self.fail("cursor failed to terminate")
                    self.assertEqual(len(seen), len(set(seen)))
                    self.assertEqual(len(seen), 6)
        first = self.compare([{"kind": KINDS[0], "args": {"limit": 1}}])[0]["data"]
        cursor = first["nextCursor"]
        self.compare([{"kind": KINDS[0], "args": {"limit": 1, "cursor": cursor, **change}}
                      for change in ({"wallet": "ln"}, {"order": "asc"}, {"sort": "amount"}, {"with_fees": True}, {"since": "2024-01-01"})])
        self.compare([{"kind": kind, "args": args} for kind in KINDS[:2]
                      for args in ({"query": "%", "status": "review"}, {"query": "%", "status": "completed", "wallet": "ln"},
                                   {"query": "%", "since": "2025-01-02", "quick": "review_queue"},
                                   {"query": "%", "flow": "swap", "candidate_txids": ["a"]})])
        conn = open_db(str(self.root))
        conn.execute("UPDATE profiles SET last_processed_tx_count=0")
        conn.commit()
        conn.close()
        self.compare([{"kind": kind, "args": args} for kind in KINDS[:2]
                      for args in ({"query": "%"}, {"query": "%", "flow": "incoming"}, {"query": "%", "flow": "swap"})])

    def test_wallet_matches_all_ids_and_labels_and_no_context_precedes_validation(self):
        conn = open_db(str(self.root))
        workspace, profile, wallet = conn.execute("SELECT workspace_id,profile_id,id FROM wallets WHERE label='Cold'").fetchone()
        conn.execute("UPDATE wallets SET label='cold' WHERE id='ln'")
        conn.execute("UPDATE wallets SET label=? WHERE id='ex'", (wallet,))
        conn.commit()
        conn.close()
        self.compare([{"kind": kind, "args": args} for kind in KINDS[:2] for args in
                      ({"query": "%", "wallet": "Cold"}, {"query": "%", "wallet": wallet})])
        conn = open_db(str(self.root))
        conn.execute("DELETE FROM settings WHERE key IN ('context_workspace','context_profile')")
        conn.commit()
        conn.close()
        self.compare([{"kind": kind, "args": args} for kind in KINDS for args in
                      ({}, [], {"unknown": True})])
