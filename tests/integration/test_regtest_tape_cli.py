"""Core RPC tape replay through the CLI, for either runtime and across both.

`RegtestTapeCliTest` builds the replay book with whichever runtime
`tests/cli_runner.py` starts, so `integration-harness.sh fast` with
`KASSIBER_RUNTIME=rust` runs it against the Rust binary. It pins the same
numbers as the in-process replay in `tests/test_regtest_harness.py`.

`RegtestTapeRuntimeParityTest` needs `KASSIBER_RUST_BIN` and compares the two
runtimes on one book: setup, sync, `rates set`, `journals process`, every
report read and the CSV/XLSX/PDF exports. Envelopes and every table of the
store's schema manifest must match after masking fresh identities and clocks.
Export artifacts follow the shared report design (docs/reference/report-design.md),
so their bytes, layout keys and digests are masked and only their validity is
checked. Where Rust refuses Core RPC wallet sync, the test requires a clean
refusal that leaves the book untouched; where it syncs, both syncs and synced
books must match. Everything after sync runs on copies of the Python-synced
book.
"""
from __future__ import annotations

from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import shutil
import sqlite3
import tempfile
import unittest
import zipfile

from tests import cli_runner
from tests.integration import regtest_tape_cli as tape_cli
from tests.integration.runtime_parity import (
    RUST_BIN, Masker, book_identities, canonical, child_env, envelope, mask_design, masked_book,
)

# Rust refuses Core RPC sync until the route is ported; any other outcome
# means the gap closed and the synced books must then be compared directly.
NATIVE_SYNC_GAP_CODES = {"native_fallback_required", "not_implemented", "unsupported"}


class RegtestTapeCliTest(unittest.TestCase):
    """The replay on the runtime `cli_runner` selects."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-tape-cli-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.root = self.base / "data"
        self.env = child_env(self.base / "home", python_child=cli_runner.candidate_prefix() is None,
                             rust_bin=(cli_runner.candidate_prefix() or [None])[0])

    def cli(self, *args, ok=True):
        result = cli_runner.run(["--machine", "--data-root", str(self.root), *args],
                                env=self.env, capture_output=True, text=True, timeout=300)
        payload = envelope(result)
        if ok:
            self.assertEqual(result.returncode, 0, f"kassiber {' '.join(map(str, args))}: {result.stdout}{result.stderr}")
        return payload

    def build(self, server, addresses):
        wallet = None
        for step in tape_cli.setup_steps(server, addresses):
            wallet = self.cli(*step)["data"]
        server.client_wallet = tape_cli.core_wallet_name(wallet["id"])
        return wallet

    def test_baseline_tape_syncs_journals_reports_and_exports(self):
        with tape_cli.serve("bitcoin_core_address_baseline") as server:
            self.build(server, tape_cli.BASELINE_ADDRESSES)
            first = self.cli("wallets", "sync", "--wallet", "Core replay")["data"]
            first = first[0] if isinstance(first, list) else first
            self.assertEqual(server.misses, [])
            self.assertEqual(first["backend_kind"], "bitcoinrpc")
            self.assertEqual(first["records_fetched"], 2)
            self.assertEqual(first["imported"], 2)
            repeat = self.cli("wallets", "sync", "--wallet", "Core replay")["data"]
            repeat = repeat[0] if isinstance(repeat, list) else repeat
            self.assertEqual(server.misses, [])
            self.assertEqual(repeat["imported"], 0)
            self.assertEqual(server.unauthorized, 0)
            self.assertEqual(server.unused_interactions(), [])

        rows = sorted(self.cli("transactions", "list", "--limit", "100")["data"], key=lambda r: r["occurred_at"])
        self.assertEqual([r["direction"] for r in rows], ["inbound", "outbound"])
        for at, rate in tape_cli.BASELINE_RATES:
            self.cli("rates", "set", "BTC-EUR", at, rate)
        journal = self.cli("journals", "process")["data"]
        self.assertEqual(journal["entries_created"], 2)
        self.assertEqual(journal["quarantined"], 0)
        summary = self.cli("reports", "summary")["data"]
        self.assertEqual(summary["metrics"]["journal_entries"], 2)
        self.assertEqual(summary["metrics"]["quarantines"], 0)
        self.assertAlmostEqual(summary["holdings"]["cost_basis"], 6999.65, places=2)
        self.assertAlmostEqual(summary["realized"]["gain_loss"], 49.65, places=2)
        target = self.base / "report.xlsx"
        export = self.cli("reports", "export-xlsx", "--file", str(target))["data"]
        self.assertEqual(export["file"], str(target))
        self.assertTrue(zipfile.is_zipfile(target))

        conn = sqlite3.connect(self.root / "kassiber.sqlite3")
        try:
            journal_rows = conn.execute(
                "SELECT entry_type, quantity, fiat_value, proceeds FROM journal_entries ORDER BY occurred_at"
            ).fetchall()
            utxo = conn.execute("SELECT txid, vout, amount, block_height FROM wallet_utxos").fetchall()
        finally:
            conn.close()
        self.assertEqual(journal_rows, [("acquisition", 25_000_000_000, 8750.0, None),
                                        ("disposal", -5_001_000_000, 1800.0, 1800.0)])
        self.assertEqual(utxo, [("b" * 64, 0, 19_999_000_000, 102)])

    def test_sync_reports_its_wall_time(self):
        # `elapsed_ms` covers the fetch and its publication (Python's
        # `sync_wallet_from_backend`); Rust once reported a constant 0.
        with tape_cli.serve("bitcoin_core_address_baseline") as server:
            self.build(server, tape_cli.BASELINE_ADDRESSES)
            server.delay = 0.05
            outcome = self.cli("wallets", "sync", "--wallet", "Core replay")["data"]
            outcome = outcome[0] if isinstance(outcome, list) else outcome
            calls = len(server.calls)
        self.assertGreaterEqual(calls, 5)
        self.assertGreaterEqual(outcome["elapsed_ms"], 50 * calls)

    def test_edge_tape_skips_immature_coinbase_and_replaced_spend(self):
        with tape_cli.serve("bitcoin_core_edge_cases") as server:
            self.build(server, tape_cli.EDGE_ADDRESSES)
            outcome = self.cli("wallets", "sync", "--wallet", "Core replay")["data"]
            outcome = outcome[0] if isinstance(outcome, list) else outcome
            self.assertEqual(server.misses, [])
            self.assertEqual(outcome["records_fetched"], 5)
            self.assertEqual(outcome["imported"], 5)
            self.assertEqual(server.unused_interactions(), [])
        ids = {row["external_id"] for row in self.cli("transactions", "list", "--limit", "100")["data"]}
        self.assertNotIn("c1" * 32, ids)
        self.assertNotIn("e1" * 32, ids)
        self.assertEqual(len(ids), 5)
        for at, rate in tape_cli.EDGE_RATES:
            self.cli("rates", "set", "BTC-EUR", at, rate)
        journal = self.cli("journals", "process")["data"]
        self.assertEqual(journal["quarantined"], 0)
        self.assertEqual(journal["processed_transactions"], 5)
        summary = self.cli("reports", "summary")["data"]
        self.assertEqual(summary["metrics"]["active_transactions"], 5)


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary to compare")
class RegtestTapeRuntimeParityTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-tape-parity-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.work = self.base / "work"
        self.root = self.work / "data"
        self.today = datetime.now(timezone.utc)
        self.py_env = child_env(self.base / "home-python", python_child=True)
        self.rs_env = child_env(self.base / "home-rust", python_child=False, rust_bin=RUST_BIN)

    def run_cli(self, rust: bool, *args):
        argv = ["--machine", "--data-root", str(self.root), *args]
        if rust:
            from tests.rust_desktop_parity import run_candidate
            return run_candidate(argv, env=self.rs_env, input=None)
        return cli_runner.run(argv, reference=True, env=self.py_env, capture_output=True, timeout=300)

    def restore(self, saved: Path | None):
        if self.work.exists():
            shutil.rmtree(self.work)
        self.work.mkdir(parents=True)
        if saved is not None:
            shutil.copytree(saved, self.root)

    def save(self, name: str) -> Path:
        saved = self.base / "saved" / name
        shutil.copytree(self.root, saved)
        return saved

    def export_artifacts(self, payload: dict, masker: Masker) -> None:
        """Check the artifact an export wrote and mask what the design changes."""
        data = payload.get("data")
        if payload.get("kind") == "error" or not isinstance(data, dict) or "file" not in data:
            return
        artifact = Path(data["file"])
        candidates = [artifact, artifact.with_suffix(".zip")]
        written = next((p for p in candidates if p.exists()), None)
        self.assertIsNotNone(written, f"export wrote no artifact: {data['file']}")
        raw = written.read_bytes()
        if written.suffix == ".pdf":
            self.assertTrue(raw.startswith(b"%PDF-"))
        elif written.suffix in (".xlsx", ".zip"):
            with zipfile.ZipFile(written) as archive:
                self.assertIsNone(archive.testzip())
        else:
            raw.decode("utf-8")
        snapshot = data.get("report_snapshot")
        if isinstance(snapshot, dict) and snapshot.get("content_sha256"):
            self.assertEqual(snapshot["content_sha256"], hashlib.sha256(raw).hexdigest())
        masker.digests |= mask_design(payload)

    def run_steps(self, rust: bool, start: Path | None, steps, known: set[str]):
        self.restore(start)
        masker = Masker(known, self.today)
        outputs = []
        for step in steps:
            result = self.run_cli(rust, *step)
            payload = envelope(result)
            self.export_artifacts(payload, masker)
            outputs.append((result.returncode, masker.value(payload)))
        masker.freeze()
        return outputs, masked_book(self.root, masker), masker

    def assert_same(self, label, python, rust):
        # Every step must succeed, or two identical refusals would pass.
        for index, (code, payload) in enumerate(python[0]):
            self.assertEqual(code, 0, f"{label}: Python step {index} failed: {payload}")
        for index, (expected, actual) in enumerate(zip(python[0], rust[0])):
            self.assertEqual(actual, expected, f"{label}: step {index} envelope differs")
        self.assertEqual(len(rust[0]), len(python[0]))
        for table in python[1]:
            self.assertEqual(rust[1][table], python[1][table], f"{label}: table {table} differs")

    def check_tape(self, tape_name):
        addresses, rates = tape_cli.TAPE_CASES[tape_name]
        with tape_cli.serve(tape_name) as server:
            steps = tape_cli.setup_steps(server, addresses)
            python = self.run_steps(False, None, steps, set())
            python_setup = self.save("python-setup")
            rust = self.run_steps(True, None, steps, set())
            rust_setup = self.save("rust-setup")
            self.assert_same(f"{tape_name} setup", python, rust)

            def sync(rust_runtime, saved):
                self.restore(saved)
                conn = sqlite3.connect(self.root / "kassiber.sqlite3")
                try:
                    wallet_id = conn.execute("SELECT id FROM wallets WHERE label='Core replay'").fetchone()[0]
                finally:
                    conn.close()
                server.client_wallet = tape_cli.core_wallet_name(wallet_id)
                server.misses.clear()
                results = [self.run_cli(rust_runtime, "wallets", "sync", "--wallet", "Core replay") for _ in range(2)]
                return results, list(server.misses)

            python_sync, python_misses = sync(False, python_setup)
            self.assertEqual(python_misses, [])
            for result in python_sync:
                self.assertEqual(result.returncode, 0, result.stdout)
            python_synced = self.save("python-synced")

            before = masked_book(rust_setup, Masker(set(), self.today))
            rust_sync, rust_misses = sync(True, rust_setup)
            first = envelope(rust_sync[0])
            code = (first.get("error") or {}).get("code") if first.get("kind") == "error" else None
            if code in NATIVE_SYNC_GAP_CODES:
                # Known gap: the refusal must leave the book exactly as it was.
                self.assertNotEqual(rust_sync[0].returncode, 0)
                self.assertEqual(masked_book(self.root, Masker(set(), self.today)), before)
                self.assertEqual(rust_misses, [])
            else:
                self.assertEqual(rust_misses, [], "Rust Core RPC sync asked for requests the tape lacks")
                # Each side synced its own setup book, so every identity is fresh.
                python_view = ([(r.returncode, Masker(set(), self.today).value(envelope(r))) for r in python_sync],)
                rust_view = ([(r.returncode, Masker(set(), self.today).value(envelope(r))) for r in rust_sync],)
                self.assertEqual(rust_view, python_view)
                # Different setup books mean different row identities: compare
                # the synced books identity- and order-blind.
                blind = lambda root: canonical(masked_book(root, Masker(set(), self.today, anonymous=True)))
                rust_synced = blind(self.root)
                python_synced_view = blind(python_synced)
                for table in python_synced_view:
                    self.assertEqual(rust_synced[table], python_synced_view[table], f"{tape_name} sync: table {table} differs")

        out = self.work
        after = [("rates", "set", "BTC-EUR", at, rate) for at, rate in rates]
        after += [("journals", "process"), ("transactions", "list", "--limit", "100")]
        after += [("reports", name) for name in tape_cli.REPORTS]
        after += [
            ("reports", "balance-history", "--interval", "hour"),
            ("reports", "export-csv", "--file", str(out / "report.csv")),
            ("reports", "export-xlsx", "--file", str(out / "report.xlsx")),
            ("reports", "export-pdf", "--file", str(out / "report.pdf")),
        ]
        known = book_identities(python_synced)
        python = self.run_steps(False, python_synced, after, known)
        rust = self.run_steps(True, python_synced, after, known)
        self.assert_same(f"{tape_name} after sync", python, rust)

    def test_baseline_tape_matches_python(self):
        self.check_tape("bitcoin_core_address_baseline")

    def test_edge_tape_matches_python(self):
        self.check_tape("bitcoin_core_edge_cases")


if __name__ == "__main__":
    unittest.main()
