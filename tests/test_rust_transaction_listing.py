"""R5 observed CLI pages compare exact envelopes and cursor bytes with Python."""
import json
import os
import subprocess
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests import test_rust_transaction_services as evidence

RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustTransactionListingTest(unittest.TestCase):
    cli = evidence.RustTransactionServicesTest.cli
    ok = evidence.RustTransactionServicesTest.ok

    def setUp(self):
        evidence.RustTransactionServicesTest.setUp(self)
        conn = open_db(str(self.root))
        workspace, profile, wallet = conn.execute("SELECT workspace_id,profile_id,id FROM wallets WHERE label='Cold'").fetchone()
        for ident, label, kind, config in (
            ("ex", "Exchange", "kraken", '{}'),
            ("ln", "Lightning", "lnd", '{}'),
            ("liquid", "Liquid", "liquid", '{}'),
            ("invalid", "Invalid config", "custom", 'malformed'),
        ):
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,?,?,?,?,?,?)", (ident, workspace, profile, label, kind, config, "2025"))
        for ident, wal, amount, fee, rate, value, exact, status, external, kind in (
            ("a", wallet, 9007199254740997, 1, -1, 0, "123.000000", "completed", "A" * 64, "deposit"),
            ("b", "ln", 100001, 0, 2, 3, None, "failed", "payment-hash", "deposit"),
            ("c", "ex", 100001, 1, 0, 0, None, "completed", None, "withdrawal"),
            ("d", "liquid", 0, 1001, None, None, None, "review", "ef" * 32, "peg-in"),
            ("e", "invalid", 100001, 0, None, None, None, None, "external-e", "swap"),
        ):
            conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,asset,direction,amount,fee,occurred_at,confirmed_at,created_at,fiat_rate,fiat_value,fiat_rate_exact,review_status,external_id,kind)
                VALUES(?,?,?,?,?,'BTC',?,?,?,'2025-02-01T00:00:00Z',NULL,?,?,?,?,?,?,?)""",
                (ident, workspace, profile, wal, ident, "outbound" if ident == "c" else "inbound", amount, fee, "2025-02-02" if ident in {"a", "b"} else "2025-02-01", rate, value, exact, status, external, kind))
        conn.execute("INSERT INTO journal_quarantines(transaction_id,workspace_id,profile_id,reason,created_at) VALUES('c',?,?,'blocked','2025')", (workspace, profile))
        conn.execute("INSERT INTO tags(id,workspace_id,profile_id,code,label,created_at) VALUES('z',?,?,'z','Last','2025'),('a',?,?,'a','First','2025')", (workspace, profile, workspace, profile))
        conn.execute("INSERT INTO transaction_tags(transaction_id,tag_id) VALUES('a','z'),('a','a')")
        conn.commit()
        conn.close()

    def compare(self, *args):
        reference = self.cli(*args, reference=True)
        candidate = self.cli(*args, rust=cli_runner.candidate_prefix() is None)
        self.assertEqual((candidate.returncode, candidate.stdout, candidate.stderr),
                         (reference.returncode, reference.stdout, reference.stderr), args)
        return json.loads(candidate.stdout)

    def test_filters_and_typed_refusals_match_reference_bytes(self):
        filters = [(), ("--with-fees",), ("--wallet", " Cold "), ("--wallet", "absent"),
                   ("--asset", "btc"), ("--asset", "ß"), ("--network", "regtest"),
                   ("--txid", " A," + "a" * 64 + ",,a", "--txids", "payment-HASH, OUT"),
                   ("--start", "2025-02-01"), ("--end", "2025-02-01"),
                   ("--start", "2025-W05-6T12:00:00.123456+02:00"),
                   ("--end", "20250201T000000Z"), ("--start", "bad"),
                   ("--start", " "), ("--start", ""), ("--end", "bad"),
                   ("--limit", "0"), ("--limit", "1001"), ("--limit", str(10**100)),
                   ("--cursor", "broken"), ("--txid", "absent"),
                   ("--wallet", "ex", "--with-fees", "--flow", "outgoing")]
        for flag, values in (
            ("--direction", ("inbound", "outbound")),
            ("--status", ("completed", "pending", "failed", "review")),
            ("--flow", ("incoming", "outgoing", "transfer", "swap", "layer-transition")),
            ("--payment-method", ("On-chain", "Exchange", "Lightning", "Liquid")),
            ("--network", ("onchain", "Exchange", " Lightning ", "liquid", "kraken", "btc")),
            ("--quick", ("external_flow", "review_queue", "no_explorer_id", "missing_price", "failed_import")),
            ("--period", ("30days", "3months", "6months", "ytd", "1year", "5years", "10years", "15years", "all")),
        ):
            filters.extend((flag, value) for value in values)
        for flags in filters:
            with self.subTest(flags=flags):
                self.compare("transactions", "list", *flags)
        # Ordinary CLI renderers receive the same ordered projection as machine JSON.
        for fmt in ("json", "table", "plain", "csv"):
            argv = ["--data-root", str(self.root), "--format", fmt, "transactions", "list", "--limit", "2"]
            kwargs = dict(env=self.env, capture_output=True, timeout=120)
            reference = cli_runner.run(argv, reference=True, **kwargs)
            candidate = (cli_runner.run(argv, **kwargs) if cli_runner.candidate_prefix()
                         else subprocess.run([RUST_BIN, *argv], **kwargs))
            self.assertEqual((candidate.returncode, candidate.stdout, candidate.stderr),
                             (reference.returncode, reference.stdout, reference.stderr), fmt)

    def test_all_sorts_and_pages_keep_exact_cursors_and_filtered_counts(self):
        for sort in ("occurred-at", "amount", "fee", "fiat-value"):
            for order in ("asc", "desc"):
                args = ("transactions", "list", "--sort", sort, "--order", order, "--limit", "2")
                first = self.compare(*args)
                cursor = first["next_cursor"]
                seen = [row["id"] for row in first["data"]]
                while cursor:
                    page = self.compare(*args, "--cursor", cursor)
                    self.assertEqual(page["count"], 7)
                    seen.extend(row["id"] for row in page["data"])
                    self.assertLessEqual(len(seen), 7)
                    cursor = page["next_cursor"]
                self.assertEqual(len(set(seen)), 7)
                cursor = first["next_cursor"]
                for flags in (("--with-fees",), ("--wallet", "ex"), ("--start", "2025-01-01"),
                              ("--order", "desc" if order == "asc" else "asc"),
                              ("--sort", "fee" if sort == "amount" else "amount")):
                    self.assertEqual(self.compare(*args, "--cursor", cursor, *flags)["error"]["code"], "validation")
        # Relative bounds change with the clock, but a cursor binds the period,
        # rather than the generated 'since' timestamp.
        args = ("transactions", "list", "--period", "all", "--limit", "1")
        cursor = self.compare(*args)["next_cursor"]
        self.compare(*args, "--cursor", cursor)

    def test_custody_filters_ignore_stale_projections(self):
        conn = open_db(str(self.root))
        workspace, profile = conn.execute("SELECT workspace_id,id FROM profiles").fetchone()
        conn.execute("""INSERT INTO journal_custody_economic_relations(relation_id,workspace_id,profile_id,relation_kind,source_transaction_id,target_transaction_id,source_asset,target_asset,source_amount_msat,target_amount_msat,basis_state,created_at)
            VALUES(?,?,?,?,?,?,?,?,?,?,?,?)""", ("a" * 64, workspace, profile, "conversion", "out", "in", "BTC", "LBTC", 1, 1, "eligible", "2025"))
        conn.commit()
        for current in (False, True, False):
            conn.execute("UPDATE profiles SET last_processed_at='2025',journal_input_version=2,last_processed_input_version=?", (2 if current else 1,))
            conn.commit()
            for flow in ("incoming", "outgoing", "transfer", "swap"):
                self.compare("transactions", "list", "--flow", flow)
        conn.close()

    def test_fractional_bounds_match_counts_cursor_bytes_and_history_daemon_reads(self):
        conn = open_db(str(self.root))
        workspace, profile, wallet = conn.execute(
            "SELECT workspace_id,profile_id,wallet_id FROM transactions WHERE id='a'").fetchone()
        for ident, stamp in (("a", "2025-02-01T00:00:00Z"),
                             ("b", "2025-02-01T00:00:01Z"),
                             ("c", "2025-02-01T00:00:02Z")):
            conn.execute("UPDATE transactions SET occurred_at=? WHERE id=?", (stamp, ident))
            conn.execute("""INSERT INTO transaction_edit_events(id,workspace_id,profile_id,
                transaction_id,wallet_id,source,changed_at) VALUES(?,?,?,?,?,'cli',?)""",
                ("time-" + ident, workspace, profile, ident, wallet, stamp))
        conn.commit()
        conn.close()
        bounds = ("2025-02-01T00:00:00.5Z", "2025-02-01T00:00:00.000001Z",
                  "2025-02-01T00:00:00.999999Z", "2025-02-01T01:00:00.5+01:00")
        requests = []
        for bound in bounds:
            for command, family, key, expected in (
                (("transactions", "list"), None, "id", ["c", "b"]),
                (("metadata", "records", "list"), "records", "transaction_id", ["c", "b"]),
                (("metadata", "records", "history", "activity"), "events", "id", ["time-c", "time-b"]),
            ):
                args = (*command, "--start", bound, "--limit", "1")
                first = self.compare(*args)
                container = first if family is None else first["data"]
                rows = first["data"] if family is None else container[family]
                self.assertEqual([r[key] for r in rows], expected[:1])
                self.assertTrue(container["has_more"])
                last = self.compare(*args, "--cursor", container["next_cursor"])
                container = last if family is None else last["data"]
                rows = last["data"] if family is None else container[family]
                self.assertEqual([r[key] for r in rows], expected[1:])
                self.assertFalse(container["has_more"])
                if family is None:
                    self.assertEqual(first["count"], 2)
                    self.assertEqual(last["count"], 2)
                self.compare(*command, "--end", bound)
            requests.extend((
                {"kind": "ui.activity.history", "args": {"start": bound}},
                {"kind": "ui.activity.history", "args": {"end": bound}},
                {"kind": "ui.transactions.history", "args": {"transaction": "a", "start": bound}},
                {"kind": "ui.transactions.history", "args": {"transaction": "a", "end": bound}},
            ))
        requests.append({"kind": "daemon.shutdown"})
        wire = "".join(json.dumps({"request_id": str(i), **request}) + "\n"
                       for i, request in enumerate(requests)).encode()
        argv = ["--machine", "--data-root", str(self.root), "daemon"]
        options = dict(input=wire, capture_output=True, timeout=120)
        reference = cli_runner.run(argv, reference=True, env=self.env, **options)
        candidate = (cli_runner.run(argv, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, **options)
                     if cli_runner.candidate_prefix() else subprocess.run([RUST_BIN, *argv], env=self.env, **options))
        self.assertEqual((reference.returncode, reference.stderr), (0, b""))
        self.assertEqual((candidate.returncode, candidate.stderr), (0, b""))
        self.assertEqual(candidate.stdout.splitlines()[1:], reference.stdout.splitlines()[1:])

    def test_unported_desktop_surfaces_remain_explicit_refusals(self):
        # The graph and review context are native since the desktop projections.
        kinds = ["ui.transactions.ledger_template"]
        kinds += ["ui.btcpay.provenance.sync", "ui.btcpay.provenance.review"]
        wire = "".join(json.dumps({"request_id": str(i), "kind": kind}) + "\n" for i, kind in enumerate([*kinds, "daemon.shutdown"]))
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(self.root), "daemon"], env=self.env, input=wire, text=True, capture_output=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines()]
        for record in records[1:-1]:
            self.assertEqual(record["error"]["code"], "not_implemented")
