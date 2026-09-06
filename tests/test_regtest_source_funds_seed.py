from __future__ import annotations

import csv
import tempfile
import subprocess
import unittest
from pathlib import Path

from dev.regtest.source_funds_seed import purchase_fixture


class RegtestSourceFundsFixtureTest(unittest.TestCase):
    def fixture(self, root: Path, *, amount: str = "0.01", complete: bool = True) -> Path:
        path = root / "strike-complete.csv"
        fields = ["Reference", "Amount BTC", "Amount EUR", "Fee EUR", "Fee BTC", "Date & Time (UTC)", "Transaction Hash"]
        with path.open("w", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=fields)
            writer.writeheader()
            if complete:
                writer.writerow({"Reference": "demo-buy", "Amount BTC": amount, "Amount EUR": "-1000", "Fee EUR": "1", "Date & Time (UTC)": "2025-11-09T17:38:17Z"})
            writer.writerow({"Reference": "demo-withdraw", "Amount BTC": "-0.006", "Fee BTC": "0.0001", "Transaction Hash": "a" * 64})
        return path

    def test_reads_exact_purchase_and_withdrawal_reference(self):
        with tempfile.TemporaryDirectory() as directory:
            fixture = purchase_fixture(self.fixture(Path(directory)))
        self.assertEqual(fixture, {"acquired_at": "2025-11-09T17:38:17Z", "txid": "a" * 64})

    def test_missing_purchase_does_not_become_reviewed_origin(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "complete generated Strike"):
                purchase_fixture(self.fixture(Path(directory), complete=False))

    def test_changed_source_amount_requires_fixture_review(self):
        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(ValueError, "Unexpected synthetic purchase"):
                purchase_fixture(self.fixture(Path(directory), amount="0.001"))

    def test_rebuild_removes_receipt_before_seeding_new_transaction_ids(self):
        root = Path(__file__).resolve().parents[1]
        harness = (root / "scripts/integration-harness.sh").read_text()
        build = "demo_build_book() {" + harness.split("demo_build_book() {", 1)[1].split("\ndemo_print_instructions()", 1)[0]
        with tempfile.TemporaryDirectory() as directory:
            home = Path(directory)
            receipt = home / "source-funds-seed.json"
            receipt.write_text('{"source_id":"old-book-source"}')
            (home / "manual-note.txt").write_text("keep")
            body = build + '''
DEMO_HOME="$1"
DEMO_MANIFEST="$DEMO_HOME/demo-manifest.json"
KASSIBER_REGTEST_REUSE_CORE=1
demo_scenario_checksum() { echo new; }
demo_manifest_get() { echo old; }
demo_assert_safe_home() { :; }
py() { :; }
demo_refresh_live_rate() { :; }
demo_seed_btcpay() { :; }
demo_seed_lightning() { :; }
demo_seed_source_funds() { test ! -e "$DEMO_HOME/source-funds-seed.json"; }
demo_write_manifest() { :; }
set -e
demo_build_book
'''
            result = subprocess.run(["bash", "-c", body, "fixture", str(home)], capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertFalse(receipt.exists())
            self.assertEqual((home / "manual-note.txt").read_text(), "keep")
