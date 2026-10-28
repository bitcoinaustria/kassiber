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

    def test_exports_and_unported_desktop_surfaces_remain_explicit_refusals(self):
        absent = self.base / "uncreated"
        for fmt in ("csv", "xlsx", "cointracking-csv", "blockpit-csv"):
            output = self.base / ("unwritten." + fmt)
            result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(absent), "transactions", "export", "--export-format", fmt, "--file", str(output)], env=self.env, capture_output=True, timeout=120)
            self.assertEqual(json.loads(result.stdout)["error"]["code"], "not_implemented")
            self.assertFalse(absent.exists())
            self.assertFalse(output.exists())
        kinds = ["ui.transactions." + suffix for suffix in (
            "list", "dashboard", "extremes", "search", "resolve", "graph", "review_context", "ledger_template",
            "export_csv", "export_xlsx", "export_platform_csv", "commercial_context")]
        wire = "".join(json.dumps({"request_id": str(i), "kind": kind}) + "\n" for i, kind in enumerate([*kinds, "daemon.shutdown"]))
        result = subprocess.run([RUST_BIN, "--machine", "--data-root", str(self.root), "daemon"], env=self.env, input=wire, text=True, capture_output=True, timeout=120)
        self.assertEqual(result.returncode, 0, result.stderr)
        records = [json.loads(line) for line in result.stdout.splitlines()]
        for record in records[1:-1]:
            self.assertEqual(record["error"]["code"], "not_implemented")
