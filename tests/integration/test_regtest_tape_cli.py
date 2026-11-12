"""Core RPC tape replay through the Rust binary's CLI.

`RegtestTapeCliTest` builds the replay book with the binary
`tests/integration/cli.py` starts and pins the replay's numbers.

`RegtestTapeGoldenTest` once compared the Python and the Rust runtime on one
book: setup, sync, `rates set`, `journals process`, every report read and the
CSV/XLSX/PDF exports. The Python half is frozen (docs/plan/22-python-removal.md,
D5): `goldens/tape_<name>.json` holds Python's masked envelopes and books, and
`goldens/tape_<name>_synced.sqlite3.gz` the book Python's sync left, from
which the post-sync steps start, so they compare strictly as before. Export
artifacts follow the shared report design (docs/reference/report-design.md),
so their bytes, layout keys and digests are masked and only their validity
is checked. `KASSIBER_RECORD_GOLDENS=1` rewrites the goldens from the binary
under test after a reviewed change.
"""
from __future__ import annotations

from datetime import date, datetime, timezone
import gzip
import hashlib
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import unittest
import zipfile

from tests.integration import cli
from tests.integration import regtest_tape_cli as tape_cli
from tests.integration.runtime_parity import (
    GOLDENS, Masker, book_identities, canonical, child_env, envelope, jsonable, load_golden,
    mask_design, masked_book, scrub_paths, write_golden,
)

RECORD = os.environ.get("KASSIBER_RECORD_GOLDENS") == "1"


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class RegtestTapeCliTest(unittest.TestCase):
    """The replay on the binary `tests/integration/cli.py` selects."""

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-tape-cli-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.root = self.base / "data"
        self.env = child_env(self.base / "home", rust_bin=cli.binary())

    def cli(self, *args, ok=True):
        result = cli.run(["--machine", "--data-root", str(self.root), *args],
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
        # `elapsed_ms` covers the fetch and its publication; Rust once
        # reported a constant 0.
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


def _mask_keys(value, keys: set[str]):
    if isinstance(value, dict):
        return {key: f"<{key}>" if key in keys else _mask_keys(item, keys) for key, item in value.items()}
    if isinstance(value, list):
        return [_mask_keys(item, keys) for item in value]
    if isinstance(value, tuple):
        return tuple(_mask_keys(item, keys) for item in value)
    return value


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class RegtestTapeGoldenTest(unittest.TestCase):
    maxDiff = None

    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix="kassiber-tape-golden-")
        self.addCleanup(directory.cleanup)
        self.base = Path(directory.name).resolve()
        self.work = self.base / "work"
        self.root = self.work / "data"
        self.today = datetime.now(timezone.utc)
        self.env = child_env(self.base / "home", rust_bin=cli.binary())

    def run_cli(self, *args):
        argv = ["--machine", "--data-root", str(self.root), *args]
        return cli.run(argv, env=self.env, capture_output=True, timeout=300)

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

    def scrub(self, value, server):
        """Paths under the scratch root and the tape server's port vary by run,
        and so does the endpoint fingerprint a sync reports over that URL."""
        value = scrub_paths(value, self.base)
        if server is None:
            return value
        value = scrub_paths(value, server.url, "<tape-url>")
        return _mask_keys(value, {"backend_endpoint_fingerprint"})

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

    def run_steps(self, start: Path | None, steps, masker: Masker, server=None):
        self.restore(start)
        outputs = []
        for step in steps:
            result = self.run_cli(*step)
            payload = envelope(result)
            self.export_artifacts(payload, masker)
            outputs.append((result.returncode, masker.value(self.scrub(payload, server))))
        masker.freeze()
        for index, (code, payload) in enumerate(outputs):
            self.assertEqual(code, 0, f"step {index} {steps[index][:2]} failed: {payload}")
        book = masked_book(self.root, masker, scrub=lambda rows: self.scrub(rows, server))
        return jsonable({"steps": outputs, "book": book})

    def assert_same(self, label, expected, actual):
        for index, (want, got) in enumerate(zip(expected["steps"], actual["steps"])):
            self.assertEqual(got, want, f"{label}: step {index} envelope differs")
        self.assertEqual(len(actual["steps"]), len(expected["steps"]))
        self.assertEqual(sorted(actual["book"]), sorted(expected["book"]))
        for table in expected["book"]:
            self.assertEqual(actual["book"][table], expected["book"][table], f"{label}: table {table} differs")

    def check_tape(self, tape_name):
        addresses, rates = tape_cli.TAPE_CASES[tape_name]
        synced_book = GOLDENS / f"tape_{tape_name}_synced.sqlite3.gz"
        golden_name = f"tape_{tape_name}.json.gz"
        with tape_cli.serve(tape_name) as server:
            steps = tape_cli.setup_steps(server, addresses)
            setup = self.run_steps(None, steps, Masker(set(), self.today), server)
            setup_book = self.save("setup")

            self.restore(setup_book)
            conn = sqlite3.connect(self.root / "kassiber.sqlite3")
            try:
                wallet_id = conn.execute("SELECT id FROM wallets WHERE label='Core replay'").fetchone()[0]
            finally:
                conn.close()
            server.client_wallet = tape_cli.core_wallet_name(wallet_id)
            server.misses.clear()
            results = [self.run_cli("wallets", "sync", "--wallet", "Core replay") for _ in range(2)]
            self.assertEqual(server.misses, [], "Core RPC sync asked for requests the tape lacks")
            for result in results:
                self.assertEqual(result.returncode, 0, result.stdout)
            # Each sync numbers its own fresh identities; the synced book is
            # compared identity- and order-blind, as the two runtimes'
            # independent setups always were.
            sync = jsonable({
                "steps": [(r.returncode, Masker(set(), self.today).value(self.scrub(envelope(r), server)))
                          for r in results],
                "book": canonical(masked_book(self.root, Masker(set(), self.today, anonymous=True),
                                              scrub=lambda rows: self.scrub(rows, server))),
            })
            synced = self.save("synced")

        if RECORD:
            conn = sqlite3.connect(synced / "kassiber.sqlite3")
            conn.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            conn.close()
            GOLDENS.mkdir(parents=True, exist_ok=True)
            synced_book.write_bytes(gzip.compress((synced / "kassiber.sqlite3").read_bytes(), mtime=0))
            recorded_on = self.today.date().isoformat()
        else:
            golden = load_golden(golden_name)
            recorded_on = golden["recorded_on"]
        # The post-sync steps start from the frozen synced book.
        start = self.base / "frozen" / "data"
        start.mkdir(parents=True)
        (start / "kassiber.sqlite3").write_bytes(gzip.decompress(synced_book.read_bytes()))

        out = self.work
        after = [("rates", "set", "BTC-EUR", at, rate) for at, rate in rates]
        after += [("journals", "process"), ("transactions", "list", "--limit", "100")]
        # Balance history runs up to now unless told otherwise; a frozen
        # record needs a fixed end after the tape's last block.
        end = ("--end", "2023-11-16T00:00:00Z")
        after += [("reports", name) for name in tape_cli.REPORTS if name != "balance-history"]
        after += [
            ("reports", "balance-history", *end),
            ("reports", "balance-history", "--interval", "hour", *end),
            ("reports", "export-csv", "--file", str(out / "report.csv")),
            ("reports", "export-xlsx", "--file", str(out / "report.xlsx")),
            ("reports", "export-pdf", "--file", str(out / "report.pdf")),
        ]
        known = book_identities(start)
        masker = Masker(known, self.today, also=(date.fromisoformat(recorded_on),))
        post = self.run_steps(start, after, masker)
        actual = {"recorded_on": recorded_on, "setup": setup, "sync": sync, "after": post}
        if RECORD:
            write_golden(golden_name, actual)
            self.skipTest(f"recorded {golden_name} from {cli.binary()}")
        self.assert_same(f"{tape_name} setup", golden["setup"], setup)
        self.assertEqual(sync["steps"], golden["sync"]["steps"], f"{tape_name} sync envelopes differ")
        for table in golden["sync"]["book"]:
            self.assertEqual(sync["book"][table], golden["sync"]["book"][table], f"{tape_name} sync: table {table} differs")
        self.assert_same(f"{tape_name} after sync", golden["after"], post)

    def test_baseline_tape_matches_frozen_python_run(self):
        self.check_tape("bitcoin_core_address_baseline")

    def test_edge_tape_matches_frozen_python_run(self):
        self.check_tape("bitcoin_core_edge_cases")


if __name__ == "__main__":
    unittest.main()
