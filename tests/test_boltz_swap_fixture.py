import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


def _run_json(data_root, *args):
    cmd = [
        sys.executable,
        "-m",
        "kassiber",
        "--data-root",
        str(data_root),
        "--machine",
        *args,
    ]
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"CLI failed for {args!r}: stdout={result.stdout!r} stderr={result.stderr!r}"
        )
    return json.loads(result.stdout)


class BoltzSwapFixtureTest(unittest.TestCase):
    """End-to-end fixture covering a Boltz BTC <-> LBTC chain-swap round trip.

    Proves that the full CLI pipeline — CSV import, metadata tagging, manual
    cross-asset pairing with ``--kind chain-swap --policy taxable``, journal
    processing, and ``reports summary`` — produces the expected shape for
    both a forward (BTC -> LBTC) and a reverse (LBTC -> BTC) Boltz swap with
    the service-fee spread baked into the fixture amounts.
    """

    def test_seed_boltz_swaps_produces_paired_chain_swap_round_trip(self) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-boltz-swap-fixture-") as tmp:
            data_root = Path(tmp) / "data"
            result = subprocess.run(
                [
                    str(ROOT / "scripts" / "seed-boltz-swaps.sh"),
                    "--data-root",
                    str(data_root),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(
                result.returncode,
                0,
                msg=f"stdout={result.stdout!r}\nstderr={result.stderr!r}",
            )
            payload = json.loads(result.stdout)
            self.assertEqual(payload["kind"], "reports.summary")
            self.assertEqual(payload["schema_version"], 1)
            data = payload["data"]
            self.assertEqual(data["workspace"], "Boltz Chain-Swap Demo")
            self.assertEqual(data["profile"], "Generic Chain Swaps")
            self.assertEqual(data["processed_tx_count"], 5)
            self.assertEqual(data["metrics"]["wallets_in_scope"], 2)
            self.assertEqual(data["metrics"]["assets_in_scope"], 2)
            self.assertEqual(data["metrics"]["active_transactions"], 5)
            self.assertEqual(data["metrics"]["quarantines"], 0)
            self.assertEqual(data["metrics"]["transactions_with_notes"], 4)
            self.assertEqual(data["metrics"]["transactions_with_tags"], 4)

            flow_by_asset = {row["asset"]: row for row in data["asset_flow"]}
            self.assertEqual(set(flow_by_asset), {"BTC", "LBTC"})
            self.assertEqual(flow_by_asset["BTC"]["tx_count"], 3)
            self.assertEqual(flow_by_asset["LBTC"]["tx_count"], 2)

            transfers = _run_json(
                data_root,
                "journals",
                "transfers",
                "list",
                "--workspace",
                "Boltz Chain-Swap Demo",
                "--profile",
                "Generic Chain Swaps",
            )
            self.assertEqual(transfers["kind"], "journals.transfers.list")
            summary = transfers["data"]["summary"]
            self.assertEqual(summary["same_asset_transfers"], 0)
            self.assertEqual(summary["cross_asset_pairs"], 2)
            self.assertEqual(summary["quarantines"], 0)

            pairs_by_out = {
                row["out_external_id"]: row
                for row in transfers["data"]["cross_asset_pairs"]
                if row.get("out_external_id")
            }
            self.assertIn("boltz-fwd-btc-out-1", pairs_by_out)
            self.assertIn("boltz-rev-lbtc-out-1", pairs_by_out)
            forward = pairs_by_out["boltz-fwd-btc-out-1"]
            reverse = pairs_by_out["boltz-rev-lbtc-out-1"]
            self.assertEqual(forward["kind"], "chain-swap")
            self.assertEqual(forward["policy"], "taxable")
            self.assertEqual(forward["in_external_id"], "boltz-fwd-lbtc-in-1")
            self.assertEqual(forward["out_wallet"], "Boltz Demo Hot BTC")
            self.assertEqual(forward["in_wallet"], "Boltz Demo Liquid")
            self.assertEqual(reverse["kind"], "chain-swap")
            self.assertEqual(reverse["policy"], "taxable")
            self.assertEqual(reverse["in_external_id"], "boltz-rev-btc-in-1")
            self.assertEqual(reverse["out_wallet"], "Boltz Demo Liquid")
            self.assertEqual(reverse["in_wallet"], "Boltz Demo Hot BTC")

            boltz_tagged = _run_json(
                data_root,
                "metadata",
                "records",
                "list",
                "--workspace",
                "Boltz Chain-Swap Demo",
                "--profile",
                "Generic Chain Swaps",
                "--tag",
                "boltz",
            )
            self.assertEqual(boltz_tagged["kind"], "metadata.records.list")
            self.assertEqual(
                {row["external_id"] for row in boltz_tagged["data"]["records"]},
                {
                    "boltz-fwd-btc-out-1",
                    "boltz-fwd-lbtc-in-1",
                    "boltz-rev-lbtc-out-1",
                    "boltz-rev-btc-in-1",
                },
            )

            service_fee_tagged = _run_json(
                data_root,
                "metadata",
                "records",
                "list",
                "--workspace",
                "Boltz Chain-Swap Demo",
                "--profile",
                "Generic Chain Swaps",
                "--tag",
                "service-fee",
            )
            self.assertEqual(
                {row["external_id"] for row in service_fee_tagged["data"]["records"]},
                {"boltz-fwd-lbtc-in-1", "boltz-rev-btc-in-1"},
            )


if __name__ == "__main__":
    unittest.main()
