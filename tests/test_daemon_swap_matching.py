"""Daemon dispatch pins for the swap-matching kinds.

Exercises ``ui.transfers.*`` and ``ui.saved_views.*`` end-to-end through
the real subprocess daemon — the same dispatch path the Tauri shell and
the AI tool catalog drive off. The CLI handlers underneath are pinned
by ``tests/test_swap_matching_cli.py``; these tests cover the daemon
envelope routing.
"""

import json
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


_PHOENIX_CSV = """date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description
2026-03-14T17:30:00Z,11111111-aaaa-bbbb-cccc-000000000001,lightning_sent,-100000000,-40.00 USD,0,0,0 USD,0,0 USD,abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789,,03somenode,LN to swap
"""

_LIQUID_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-14T17:32:00Z,liquid-claim-1,inbound,LBTC,0.000995,0,40000,Boltz claim
"""


def _run_cli(data_root, *args):
    return subprocess.run(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "--machine",
            *args,
        ],
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )


def _start_daemon(data_root):
    return subprocess.Popen(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "daemon",
        ],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def _request_response(proc, payload, timeout=10.0):
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        ready, _, _ = select.select([proc.stdout.fileno()], [], [], deadline - time.monotonic())
        if not ready:
            break
        line = proc.stdout.readline()
        if not line:
            break
        envelope = json.loads(line)
        if envelope.get("request_id") == payload.get("request_id"):
            return envelope
    raise AssertionError(
        f"daemon never responded to request_id={payload.get('request_id')!r}"
    )


class DaemonSwapMatchingTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-daemon-swap-")
        cls.data_root = Path(cls._tmp.name) / "data"
        cls.phoenix_csv = Path(cls._tmp.name) / "phoenix.csv"
        cls.phoenix_csv.write_text(_PHOENIX_CSV, encoding="utf-8")
        cls.liquid_csv = Path(cls._tmp.name) / "liquid.csv"
        cls.liquid_csv.write_text(_LIQUID_CSV, encoding="utf-8")

        for args in (
            ("init",),
            ("workspaces", "create", "Main"),
            ("profiles", "create", "--workspace", "Main", "--tax-country", "at", "Swap"),
            ("wallets", "create", "--workspace", "Main", "--profile", "Swap",
             "--label", "phoenix-ln", "--kind", "phoenix"),
            ("wallets", "create", "--workspace", "Main", "--profile", "Swap",
             "--label", "liquid-onchain", "--kind", "custom"),
            ("wallets", "import-phoenix", "--workspace", "Main", "--profile", "Swap",
             "--wallet", "phoenix-ln", "--file", str(cls.phoenix_csv)),
            ("wallets", "import-csv", "--workspace", "Main", "--profile", "Swap",
             "--wallet", "liquid-onchain", "--file", str(cls.liquid_csv)),
        ):
            result = _run_cli(cls.data_root, *args)
            if result.returncode != 0:
                raise AssertionError(
                    f"setup CLI {args!r} failed: {result.stderr}"
                )

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _with_daemon(self, callback):
        proc = _start_daemon(self.data_root)
        try:
            callback(proc)
        finally:
            proc.stdin.close()
            try:
                proc.wait(timeout=5.0)
            except subprocess.TimeoutExpired:
                proc.kill()

    def test_01_ui_transfers_suggest_returns_candidate(self):
        def call(proc):
            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.suggest",
                    "request_id": "req-suggest",
                    "args": {"workspace": "Main", "profile": "Swap"},
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.suggest")
            self.assertIn("candidates", envelope["data"])
            self.assertGreaterEqual(envelope["data"]["counts"]["total"], 1)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.suggest",
                    "request_id": "req-suggest-swap-type",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "candidate_type": "swap",
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.suggest")
            self.assertEqual(envelope["data"]["counts"]["total"], 0)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.suggest",
                    "request_id": "req-suggest-transfer-type",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "candidate_type": "transfer",
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.suggest")
            self.assertGreaterEqual(envelope["data"]["counts"]["total"], 1)
            self.assertTrue(
                all(
                    candidate["candidate_type"] == "transfer"
                    for candidate in envelope["data"]["candidates"]
                )
            )

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.suggest",
                    "request_id": "req-suggest-route",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "route_pair": "LNBTC-LBTC",
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.suggest")
            self.assertGreaterEqual(envelope["data"]["counts"]["total"], 1)

        self._with_daemon(call)

    def test_015_ui_transfers_review_context_returns_review_packet(self):
        def call(proc):
            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.review_context",
                    "request_id": "req-review",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "limit": 5,
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.review_context")
            data = envelope["data"]
            self.assertGreaterEqual(data["summary"]["candidate_count"], 1)
            self.assertGreaterEqual(data["summary"]["review_items"], 1)
            item = data["review_items"][0]
            self.assertEqual(item["candidate"]["default_policy"], "carrying-value")
            self.assertEqual(item["fee"]["swap_fee_msat"], 500000)
            self.assertEqual(item["fee"]["assessment"], "normal")
            self.assertIn("confidence", item)
            self.assertIn("report_impact_if_left_unpaired", item)
            self.assertIn("suggested_action", item)
            mention_keywords = {
                mention["keyword"]
                for mention in item["metadata_mentions"]
            }
            self.assertTrue({"swap", "boltz"} & mention_keywords)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.review_context",
                    "request_id": "req-review-transfer-type",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "limit": 5,
                        "candidate_type": "transfer",
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.review_context")
            self.assertGreaterEqual(envelope["data"]["summary"]["candidate_count"], 1)

        self._with_daemon(call)

    def test_02_ui_transfers_bulk_pair_and_list(self):
        def call(proc):
            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.bulk_pair",
                    "request_id": "req-bulk",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "confidence": "strong",
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.bulk_pair")
            self.assertGreaterEqual(envelope["data"]["summary"]["count"], 1)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.list",
                    "request_id": "req-list",
                    "args": {"workspace": "Main", "profile": "Swap"},
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.list")
            self.assertGreaterEqual(len(envelope["data"]["pairs"]), 1)
            pair = envelope["data"]["pairs"][0]
            self.assertIn("swap_fee_msat", pair)
            self.assertEqual(pair["pair_source"], "bulk_selected")
            # Enriched legs power the paired view's rail badges + timestamps.
            self.assertIn("wallet_kind", pair["out"])
            self.assertIn("wallet_kind", pair["in"])
            self.assertTrue(pair["out"]["occurred_at"])
            self.assertTrue(pair["in"]["occurred_at"])

            # ui.transfers.update edits an existing pair's kind in place.
            target_kind = "peg-out" if pair["kind"] != "peg-out" else "swap-refund"
            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.update",
                    "request_id": "req-update",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "pair_id": pair["id"],
                        "kind": target_kind,
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.transfers.update")
            self.assertEqual(envelope["data"]["kind"], target_kind)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.transfers.list",
                    "request_id": "req-list-2",
                    "args": {"workspace": "Main", "profile": "Swap"},
                },
            )
            updated = next(
                p for p in envelope["data"]["pairs"] if p["id"] == pair["id"]
            )
            self.assertEqual(updated["kind"], target_kind)

        self._with_daemon(call)

    def test_03_ui_saved_views_round_trip(self):
        def call(proc):
            envelope = _request_response(
                proc,
                {
                    "kind": "ui.saved_views.create",
                    "request_id": "req-vc",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "surface": "swap_candidates",
                        "name": "Pegouts to review",
                        "filter": {"asset_pair": "LBTC-BTC"},
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.saved_views.create")
            view_id = envelope["data"]["id"]

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.saved_views.list",
                    "request_id": "req-vl",
                    "args": {"workspace": "Main", "profile": "Swap"},
                },
            )
            self.assertEqual(envelope["kind"], "ui.saved_views.list")
            self.assertGreaterEqual(len(envelope["data"]["views"]), 1)

            envelope = _request_response(
                proc,
                {
                    "kind": "ui.saved_views.delete",
                    "request_id": "req-vd",
                    "args": {
                        "workspace": "Main",
                        "profile": "Swap",
                        "view_id": view_id,
                    },
                },
            )
            self.assertEqual(envelope["kind"], "ui.saved_views.delete")
            self.assertEqual(envelope["data"]["deleted"], view_id)

        self._with_daemon(call)


class SwapReviewSuggestedActionTest(unittest.TestCase):
    def test_exact_candidate_action_pairs_only_the_reviewed_legs(self):
        try:
            from kassiber.daemon_swap_review import _swap_review_suggested_action
        except ModuleNotFoundError as exc:
            self.skipTest(f"project dependency unavailable: {exc}")

        action = _swap_review_suggested_action(
            {
                "out_id": "tx-out-1",
                "in_id": "tx-in-1",
                "default_kind": "swap",
                "default_policy": "carrying-value",
                "confidence": "exact",
                "method": "payment_hash",
            },
            conflict_size=1,
            fee_assessment="normal",
        )

        self.assertEqual(action["daemon_kind"], "ui.transfers.pair")
        self.assertEqual(
            action["arguments"],
            {
                "tx_out": "tx-out-1",
                "tx_in": "tx-in-1",
                "kind": "swap",
                "policy": "carrying-value",
                "confidence_at_pair": "exact",
            },
        )


if __name__ == "__main__":
    unittest.main()
