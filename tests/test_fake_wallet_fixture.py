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
        raise AssertionError(f"CLI failed for {args!r}: stdout={result.stdout!r} stderr={result.stderr!r}")
    return json.loads(result.stdout)


class FakeWalletFixtureTest(unittest.TestCase):
    def test_seed_fake_wallets_creates_demo_profile_with_swaps(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-fake-wallet-fixture-") as tmp:
            data_root = Path(tmp) / "data"
            result = subprocess.run(
                [
                    str(ROOT / "scripts" / "seed-fake-wallets.sh"),
                    "--data-root",
                    str(data_root),
                ],
                cwd=ROOT,
                capture_output=True,
                text=True,
                check=False,
            )
            self.assertEqual(result.returncode, 0, msg=f"stdout={result.stdout!r}\nstderr={result.stderr!r}")
            payload = json.loads(result.stdout)
            self.assertEqual(payload["kind"], "reports.summary")
            self.assertEqual(payload["schema_version"], 1)
            data = payload["data"]
            self.assertEqual(data["workspace"], "Fake Wallet Demo")
            self.assertEqual(data["profile"], "Generic Swaps")
            self.assertEqual(data["processed_tx_count"], 9)
            self.assertEqual(data["metrics"]["wallets_in_scope"], 3)
            self.assertEqual(data["metrics"]["assets_in_scope"], 2)
            self.assertEqual(data["metrics"]["active_transactions"], 9)
            self.assertEqual(data["metrics"]["quarantines"], 0)
            self.assertEqual(data["metrics"]["transactions_with_notes"], 7)
            self.assertEqual(data["metrics"]["transactions_with_tags"], 7)

            flow_by_asset = {row["asset"]: row for row in data["asset_flow"]}
            self.assertEqual(set(flow_by_asset), {"BTC", "LBTC"})
            self.assertEqual(flow_by_asset["BTC"]["tx_count"], 6)
            self.assertEqual(flow_by_asset["BTC"]["fee_amount_msat"], 35_000_000)
            self.assertEqual(flow_by_asset["LBTC"]["tx_count"], 3)
            self.assertEqual(flow_by_asset["LBTC"]["fee_amount_msat"], 3_000_000)

            transfers = _run_json(
                data_root,
                "journals",
                "transfers",
                "list",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
            )
            self.assertEqual(transfers["kind"], "journals.transfers.list")
            summary = transfers["data"]["summary"]
            self.assertEqual(summary["same_asset_transfers"], 1)
            self.assertEqual(summary["cross_asset_pairs"], 2)
            self.assertEqual(summary["quarantines"], 0)
            same_asset = transfers["data"]["same_asset_transfers"][0]
            self.assertEqual(same_asset["external_id"], "demo-self-transfer-1")
            self.assertEqual(same_asset["from_wallet"], "Demo Cold BTC")
            self.assertEqual(same_asset["to_wallet"], "Demo Hot BTC")

            cross_pairs = {
                row["kind"]: row
                for row in transfers["data"]["cross_asset_pairs"]
            }
            self.assertEqual(set(cross_pairs), {"peg-in", "peg-out"})
            self.assertEqual(cross_pairs["peg-in"]["policy"], "taxable")
            self.assertEqual(cross_pairs["peg-in"]["out_wallet"], "Demo Cold BTC")
            self.assertEqual(cross_pairs["peg-in"]["in_wallet"], "Demo Liquid")
            self.assertEqual(cross_pairs["peg-out"]["policy"], "taxable")
            self.assertEqual(cross_pairs["peg-out"]["out_wallet"], "Demo Liquid")
            self.assertEqual(cross_pairs["peg-out"]["in_wallet"], "Demo Hot BTC")

            tags = _run_json(
                data_root,
                "metadata",
                "tags",
                "list",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
            )
            self.assertEqual(tags["kind"], "metadata.tags.list")
            self.assertEqual(
                [row["code"] for row in tags["data"]],
                ["peg-in", "peg-out", "self-transfer", "spend", "swap"],
            )

            swap_records = _run_json(
                data_root,
                "metadata",
                "records",
                "list",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
                "--tag",
                "swap",
            )
            self.assertEqual(swap_records["kind"], "metadata.records.list")
            self.assertEqual(len(swap_records["data"]["records"]), 4)
            self.assertEqual(
                {
                    row["external_id"]
                    for row in swap_records["data"]["records"]
                },
                {
                    "demo-peg-in-out-1",
                    "demo-peg-in-in-1",
                    "demo-peg-out-out-1",
                    "demo-peg-out-in-1",
                },
            )

            spend_records = _run_json(
                data_root,
                "metadata",
                "records",
                "list",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
                "--tag",
                "spend",
            )
            self.assertEqual(len(spend_records["data"]["records"]), 2)

            peg_in_record = _run_json(
                data_root,
                "metadata",
                "records",
                "get",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
                "--transaction",
                "demo-peg-in-out-1",
            )
            self.assertEqual(peg_in_record["kind"], "metadata.records.get")
            self.assertEqual(peg_in_record["data"]["note"], "BTC leg of the fake peg-in")
            self.assertEqual(
                [row["code"] for row in peg_in_record["data"]["tags"]],
                ["peg-in", "swap"],
            )

            noted_records = _run_json(
                data_root,
                "metadata",
                "records",
                "list",
                "--workspace",
                "Fake Wallet Demo",
                "--profile",
                "Generic Swaps",
                "--has-note",
            )
            self.assertEqual(len(noted_records["data"]["records"]), 7)


if __name__ == "__main__":
    unittest.main()
