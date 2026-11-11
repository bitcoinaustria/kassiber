"""Fast production-import replay for the Docker exchange lifecycle oracle.

`RegtestExchangeGoldenTest` once ran the lifecycle's CLI steps on the Python
and the Rust runtime and compared them. The Python half is frozen
(docs/plan/22-python-removal.md, D5): `goldens/exchange_lifecycle.json` holds
the masked envelopes and book Python produced, and the Rust binary must match
them. `KASSIBER_RECORD_GOLDENS=1` rewrites the golden from the binary under
test after a reviewed change.
"""
from datetime import datetime, timezone
import json
import os
from pathlib import Path
import shutil
import sqlite3
import tempfile
import unittest
from unittest.mock import patch

from tests.integration import cli
from tests.integration import regtest_demo
from tests.integration.regtest_demo import run_cli
from tests.integration.regtest_exchange_cases import EXPECTED, FAST_TXIDS, native_replay_lifecycle, write_exports
from tests.integration.runtime_parity import (
    Masker, canonical, child_env, jsonable, load_golden, masked_book, scrub_paths, write_golden,
)

RECORD = os.environ.get("KASSIBER_RECORD_GOLDENS") == "1"
GOLDEN = "exchange_lifecycle.json.gz"


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class RegtestExchangeCasesTest(unittest.TestCase):
    def test_export_pages_preserve_execution_cost_and_distinct_account_fee(self):
        """The Strike pages the fixture writes import as the oracle expects."""
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            files = write_exports(base / "exports", txids={"withdrawal": "a" * 64, "deposit": "b" * 64}, times={
                "buy_time": "2020-01-01T00:00:00Z", "withdrawal_time": "2020-01-02T00:00:00Z",
                "deposit_time": "2020-01-03T00:00:00Z", "sell_time": "2020-01-04T00:00:00Z",
            })
            env = child_env(base / "home", rust_bin=cli.binary())
            rows = {}
            for name, path in files.items():
                root = base / name
                with patch.dict(os.environ, env, clear=True):
                    run_cli(root, "init")
                    run_cli(root, "workspaces", "create", "Strike")
                    run_cli(root, "profiles", "create", "Main", "--workspace", "Strike",
                            "--fiat-currency", "EUR", "--tax-country", "generic", "--gains-algorithm", "FIFO")
                    run_cli(root, "wallets", "import-strike", "--workspace", "Strike", "--profile", "Main",
                            "--file", str(path))
                conn = sqlite3.connect(root / "kassiber.sqlite3")
                try:
                    rows[name] = {
                        external_id: (fiat, fee)
                        for external_id, fiat, fee in conn.execute(
                            "SELECT external_id, fiat_value_exact, fee FROM transactions ORDER BY occurred_at")
                    }
                finally:
                    conn.close()
            self.assertEqual(len(rows["incomplete"]), 3)
            complete = rows["complete"]
            self.assertEqual(len(complete), 4)
            # The buy's execution cost includes its EUR fee; the sale's
            # proceeds are net of it; the withdrawal's BTC fee stays its own.
            self.assertEqual(complete["strike:demo-buy"][0], "1001")
            self.assertEqual(complete["strike:demo-sell"][0], "149")
            self.assertEqual(complete["a" * 64][1], 10_000_000)

    def test_real_cli_partial_full_and_repeated_export_matches_independent_oracle(self):
        with tempfile.TemporaryDirectory() as tmp:
            base = Path(tmp).resolve()
            with patch.dict(os.environ, child_env(base / "home", rust_bin=cli.binary()), clear=True):
                result = native_replay_lifecycle(base / "work", run_cli)
            self.assertTrue(any(row["reason"] == "insufficient_lots" for row in result["incomplete_export_cases"]), result["incomplete_export_cases"])
            self.assertEqual(result["actual"]["combined_quantity_msat"], EXPECTED["combined_quantity_msat"])
            self.assertEqual(result["actual"]["excluded_transactions"], 0)
            self.assertEqual(result["imports"]["repeat"]["imported"], 0)
            self.assertTrue(all(row["imported"] == 0 for row in result["imports"]["file_sync"]))


@unittest.skipUnless(cli.configured(), cli.NO_BINARY)
class RegtestExchangeGoldenTest(unittest.TestCase):
    """The lifecycle's CLI steps on the Rust binary, compared with Python's frozen run.

    The two in-process steps (row pricing, state reads) are stdlib harness
    code. Each run creates its own random row identities, so lists ordered by
    them and digests over them cannot line up: a step whose numbered
    comparison differs must still match identity-blind and order-blind
    (`canonical`), and the book is compared that way.
    """

    maxDiff = None

    def run_lifecycle(self, base: Path):
        root = base / "work"
        if root.exists():
            shutil.rmtree(root)
        root.mkdir()
        env = child_env(base / "home", rust_bin=cli.binary())
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
        steps = []
        for args, payload in envelopes:
            args, payload = scrub_paths(list(args), base), scrub_paths(payload, base)
            steps.append((blind.value(args), numbered.value(payload), canonical(blind.value(payload))))
        return jsonable({"steps": steps, "book": canonical(masked_book(root, blind, scrub=base))})

    def test_lifecycle_matches_frozen_python_run(self):
        with tempfile.TemporaryDirectory() as tmp:
            actual = self.run_lifecycle(Path(tmp).resolve())
        if RECORD:
            write_golden(GOLDEN, actual)
            self.skipTest(f"recorded {GOLDEN} from {cli.binary()}")
        expected = load_golden(GOLDEN)
        self.assertEqual(len(actual["steps"]), len(expected["steps"]))
        order_only = []
        for (args, want, want_blind), (got_args, got, got_blind) in zip(expected["steps"], actual["steps"]):
            self.assertEqual(got_args, args)
            if got != want:
                self.assertEqual(got_blind, want_blind, f"kassiber {' '.join(args)} differs")
                order_only.append(" ".join(args[:2]))
        self.assertEqual(sorted(actual["book"]), sorted(expected["book"]))
        for table in expected["book"]:
            self.assertEqual(actual["book"][table], expected["book"][table], f"table {table} differs")
        if order_only:
            print(f"identity-ordered steps compared blind: {order_only}")


if __name__ == "__main__":
    unittest.main()
