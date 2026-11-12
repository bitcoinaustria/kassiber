"""Fast production-import replay for the Docker exchange lifecycle oracle."""
from datetime import datetime, timezone
import os
from pathlib import Path
import shutil
import tempfile
import unittest
from unittest.mock import patch

from tests.integration.regtest_demo import run_cli
from tests.integration.regtest_exchange_cases import native_replay_lifecycle, write_exports, EXPECTED
from tests.integration import regtest_demo
from tests.integration.regtest_exchange_cases import FAST_TXIDS
from tests.integration.runtime_parity import RUST_BIN, Masker, canonical, child_env, masked_book
from kassiber.importers import load_strike_csv_records


class RegtestExchangeCasesTest(unittest.TestCase):
    def test_export_pages_preserve_execution_cost_and_distinct_account_fee(self):
        with tempfile.TemporaryDirectory() as tmp:
            files = write_exports(Path(tmp), txids={"withdrawal":"a"*64,"deposit":"b"*64}, times={
                "buy_time":"2020-01-01T00:00:00Z", "withdrawal_time":"2020-01-02T00:00:00Z",
                "deposit_time":"2020-01-03T00:00:00Z", "sell_time":"2020-01-04T00:00:00Z",
            })
            self.assertEqual(len(load_strike_csv_records(files["incomplete"])), 3)
            records = load_strike_csv_records(files["complete"])
            self.assertEqual(str(records[0]["fiat_value"]), "1001")
            self.assertEqual(str(records[1]["fee"]), "0.00010000")
            self.assertEqual(str(records[3]["fiat_value"]), "149")

    def test_real_cli_partial_full_and_repeated_export_matches_independent_oracle(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = native_replay_lifecycle(Path(tmp), run_cli)
            self.assertTrue(any(row["reason"] == "insufficient_lots" for row in result["incomplete_export_cases"]), result["incomplete_export_cases"])
            self.assertEqual(result["actual"]["combined_quantity_msat"], EXPECTED["combined_quantity_msat"])
            self.assertEqual(result["actual"]["excluded_transactions"], 0)
            self.assertEqual(result["imports"]["repeat"]["imported"], 0)
            self.assertTrue(all(row["imported"] == 0 for row in result["imports"]["file_sync"]))


@unittest.skipUnless(RUST_BIN, "KASSIBER_RUST_BIN names the Rust binary to compare")
class RegtestExchangeRuntimeParityTest(unittest.TestCase):
    """The lifecycle's CLI steps on each runtime, from one path, compared.

    The two in-process steps (row pricing, state reads) stay Python on both
    runs; every `kassiber` child is Python in one run and Rust in the other.
    Each run creates its own random row identities, so lists ordered by them
    and digests over them cannot line up: a step whose numbered comparison
    differs must still match identity-blind and order-blind (`canonical`),
    and the book is compared that way. The tape replay's post-sync phase
    compares strictly from one shared book.
    """

    maxDiff = None

    def run_lifecycle(self, base: Path, rust: bool):
        root = base / "work"
        if root.exists():
            shutil.rmtree(root)
        root.mkdir()
        env = child_env(base / ("home-rust" if rust else "home-python"), python_child=not rust,
                        rust_bin=RUST_BIN if rust else None)
        if rust:
            env["KASSIBER_TEST_CLI"] = RUST_BIN
        envelopes = []
        original = regtest_demo.run_cli

        def recording(data_root, *args, **kwargs):
            payload = original(data_root, *args, **kwargs)
            envelopes.append((args, payload))
            return payload

        with patch.dict(os.environ, env, clear=True):
            native_replay_lifecycle(root, recording)
        now = datetime.now(timezone.utc)
        numbered = Masker(set(), now)
        blind = Masker(set(FAST_TXIDS.values()), now, anonymous=True)
        return ([(args, numbered.value(payload), canonical(blind.value(payload))) for args, payload in envelopes],
                canonical(masked_book(root, blind)))

    def test_lifecycle_matches_python(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            python = self.run_lifecycle(base, rust=False)
            rust = self.run_lifecycle(base, rust=True)
        self.assertEqual(len(rust[0]), len(python[0]))
        order_only = []
        for (args, expected, expected_blind), (_, actual, actual_blind) in zip(python[0], rust[0]):
            if actual != expected:
                self.assertEqual(actual_blind, expected_blind, f"kassiber {' '.join(args)} differs")
                order_only.append(" ".join(args[:2]))
        for table in python[1]:
            self.assertEqual(rust[1][table], python[1][table], f"table {table} differs")
        if order_only:
            print(f"identity-ordered steps compared blind: {order_only}")
