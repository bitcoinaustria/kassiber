import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent

EXCHANGE_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-10T09:00:00Z,withdraw-1,outbound,BTC,0.30010000,0.00010000,40000,Exchange withdrawal to self custody
"""

COLD_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-10T09:30:00Z,withdraw-1,inbound,BTC,0.30000000,0,40000,Received from exchange
2026-01-11T12:00:00Z,self-hop-1,outbound,BTC,0.20000000,0.00010000,41000,Move to privacy wallet
"""

PRIVACY_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-11T12:05:00Z,self-hop-1,inbound,BTC,0.20000000,0,41000,Privacy wallet receive
2026-01-12T13:00:00Z,coinjoin-hop-1,outbound,BTC,0.15000000,0.00005000,42000,Coinjoin privacy hop
"""

TARGET_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-12T14:00:00Z,target-deposit-1,inbound,BTC,0.20000000,0,42000,Target exchange deposit
"""

SWAP_BTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-13T09:00:00Z,swap-out-leg,outbound,BTC,0.05000000,0.00001000,43000,Peg-in out leg
"""

SWAP_LBTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-13T09:10:00Z,swap-in-leg,inbound,L-BTC,0.04900000,0,43000,Peg-in receive
"""


def run_cli(data_root: Path, *args: str):
    result = subprocess.run(
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
    try:
        payload = json.loads(result.stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(
            f"CLI did not return JSON for {args!r}\nstdout={result.stdout}\nstderr={result.stderr}"
        ) from exc
    return payload, result.returncode


class SourceFundsCliTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory(prefix="kassiber-source-funds-")
        self.root = Path(self.tmp.name)
        self.data_root = self.root / "data"
        self.evidence_file = self.root / "exchange-statement.txt"
        self.evidence_file.write_text("Exchange statement for reviewed fiat purchase\n", encoding="utf-8")
        self.csvs = {
            "exchange.csv": EXCHANGE_CSV,
            "cold.csv": COLD_CSV,
            "privacy.csv": PRIVACY_CSV,
            "target.csv": TARGET_CSV,
            "swap-btc.csv": SWAP_BTC_CSV,
            "swap-lbtc.csv": SWAP_LBTC_CSV,
        }
        for name, content in self.csvs.items():
            (self.root / name).write_text(content, encoding="utf-8")

    def tearDown(self):
        self.tmp.cleanup()

    def cli(self, *args: str):
        payload, code = run_cli(self.data_root, *args)
        if code != 0:
            self.fail(f"CLI failed for {args!r}: {json.dumps(payload)[:700]}")
        return payload

    def cli_error(self, *args: str):
        payload, code = run_cli(self.data_root, *args)
        self.assertNotEqual(code, 0, payload)
        self.assertEqual(payload["kind"], "error")
        return payload

    def _create_wallet_and_import(self, label: str, csv_name: str):
        self.cli(
            "wallets",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--label",
            label,
            "--kind",
            "custom",
        )
        self.cli(
            "wallets",
            "import-csv",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--wallet",
            label,
            "--file",
            str(self.root / csv_name),
        )

    def _tx_id(self, wallet: str, external_id: str) -> str:
        payload = self.cli(
            "transactions",
            "list",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--wallet",
            wallet,
            "--limit",
            "10",
        )
        rows = payload["data"].get("transactions") if isinstance(payload["data"], dict) else payload["data"]
        for row in rows:
            if row["external_id"] == external_id:
                return row["id"]
        self.fail(f"transaction {external_id} not found in wallet {wallet}")

    def test_source_funds_review_gates_snapshot_and_pdf(self):
        self.cli("init")
        self.cli("workspaces", "create", "Sof")
        self.cli(
            "profiles",
            "create",
            "--workspace",
            "Sof",
            "--fiat-currency",
            "EUR",
            "--tax-country",
            "generic",
            "Default",
        )
        for label, csv_name in [
            ("Exchange", "exchange.csv"),
            ("Cold", "cold.csv"),
            ("Privacy", "privacy.csv"),
            ("Target Exchange", "target.csv"),
            ("Swap BTC", "swap-btc.csv"),
            ("Liquid", "swap-lbtc.csv"),
        ]:
            self._create_wallet_and_import(label, csv_name)

        self.cli(
            "transfers",
            "pair",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--tx-out",
            "swap-out-leg",
            "--tx-in",
            "swap-in-leg",
            "--kind",
            "peg-in",
            "--policy",
            "taxable",
        )
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "target-deposit-1",
        )
        methods = {row["method"] for row in suggested["data"]["links"]}
        self.assertIn("same_external_id", methods)
        self.assertIn("transaction_pair", methods)
        self.assertTrue(any(row["link_type"] == "swap" for row in suggested["data"]["links"]))
        bulk_reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
        )["data"]
        self.assertGreaterEqual(bulk_reviewed["reviewed"], 3)
        self.assertGreaterEqual(bulk_reviewed["skipped"], 1)
        self.assertTrue(
            all(link["allocation_policy"] == "explicit" for link in bulk_reviewed["links"])
        )

        exchange_tx_id = self._tx_id("Exchange", "withdraw-1")
        attachment = self.cli(
            "attachments",
            "add",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--transaction",
            exchange_tx_id,
            "--file",
            str(self.evidence_file),
            "--label",
            "Exchange fiat purchase statement",
        )["data"]
        source = self.cli(
            "source-funds",
            "sources",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--type",
            "fiat_purchase",
            "--label",
            "Reviewed fiat purchase at exchange",
            "--asset",
            "BTC",
            "--amount",
            "0.15000000",
            "--fiat-currency",
            "EUR",
            "--fiat-value",
            "6000",
            "--attachment",
            attachment["id"],
        )["data"]
        self.assertEqual(source["attachments"][0]["id"], attachment["id"])

        cold_in_tx_id = self._tx_id("Cold", "withdraw-1")
        cold_out_tx_id = self._tx_id("Cold", "self-hop-1")
        privacy_in_tx_id = self._tx_id("Privacy", "self-hop-1")
        privacy_out_tx_id = self._tx_id("Privacy", "coinjoin-hop-1")

        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-source",
            source["id"],
            "--to-transaction",
            exchange_tx_id,
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.15000000",
            "--allocation-policy",
            "explicit",
            "--explanation",
            "Reviewed exchange statement ties the withdrawal to the purchase source.",
        )
        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            cold_in_tx_id,
            "--to-transaction",
            cold_out_tx_id,
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.15000000",
            "--from-amount",
            "0.15000000",
            "--allocation-policy",
            "explicit",
            "--explanation",
            "Reviewed spend allocation from cold receive to outbound hop.",
        )
        privacy_review_link = self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            privacy_in_tx_id,
            "--to-transaction",
            privacy_out_tx_id,
            "--type",
            "coinjoin",
            "--state",
            "suggested",
            "--allocation-amount",
            "0.15000000",
            "--from-amount",
            "0.15000000",
            "--allocation-policy",
            "heuristic",
            "--explanation",
            "Suggested privacy-hop allocation that must be reviewed.",
        )["data"]
        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "coinjoin-hop-1",
            "--to-transaction",
            "target-deposit-1",
            "--type",
            "coinjoin",
            "--allocation-amount",
            "0.15000000",
            "--from-amount",
            "0.15000000",
            "--allocation-policy",
            "explicit",
            "--explanation",
            "Reviewed privacy hop; no unrelated participant inputs are disclosed.",
        )

        blocked = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "target-deposit-1",
            "--target-amount",
            "0.20000000",
        )["data"]
        blockers = {item["code"] for item in blocked["explain_gates"]["blockers"]}
        self.assertIn("unreviewed_link", blockers)
        self.assertIn("ambiguous_allocation", blockers)
        self.assertFalse(blocked["explain_gates"]["exportable"])
        self.cli_error(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "target-deposit-1",
            "--target-amount",
            "0.20000000",
            "--file",
            str(self.root / "blocked.pdf"),
        )

        links = self.cli(
            "source-funds",
            "links",
            "list",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
        )["data"]
        for link in links:
            if link["method"] == "same_external_id":
                self.cli(
                    "source-funds",
                    "links",
                    "review",
                    "--workspace",
                    "Sof",
                    "--profile",
                    "Default",
                    "--link",
                    link["id"],
                    "--state",
                    "reviewed",
                    "--allocation-amount",
                    "0.15000000",
                    "--from-amount",
                    "0.15000000",
                    "--allocation-policy",
                    "explicit",
                )
            elif link["state"] == "suggested" and link["id"] != privacy_review_link["id"]:
                self.cli(
                    "source-funds",
                    "links",
                    "review",
                    "--workspace",
                    "Sof",
                    "--profile",
                    "Default",
                    "--link",
                    link["id"],
                    "--state",
                    "rejected",
                )
        self.cli(
            "source-funds",
            "links",
            "review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--link",
            privacy_review_link["id"],
            "--state",
            "reviewed",
            "--allocation-policy",
            "explicit",
        )

        gap_source = self.cli(
            "source-funds",
            "sources",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--type",
            "missing_history",
            "--label",
            "Reviewed pre-Kassiber history gap",
            "--asset",
            "BTC",
            "--amount",
            "0.05000000",
            "--description",
            "Older records are unavailable; user reviewed the gap.",
        )["data"]
        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-source",
            gap_source["id"],
            "--to-transaction",
            "target-deposit-1",
            "--type",
            "missing_history",
            "--allocation-amount",
            "0.05000000",
            "--allocation-policy",
            "explicit",
        )

        reviewed = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "target-deposit-1",
            "--target-amount",
            "0.20000000",
            "--purpose",
            "planned_exchange_sale",
            "--planned-destination",
            "Example Exchange",
            "--planned-note",
            "Pre-disclosure for expected bank proceeds.",
            "--reveal-mode",
            "minimal",
            "--save-case",
        )["data"]
        self.assertTrue(reviewed["explain_gates"]["exportable"], reviewed["explain_gates"]["blockers"])
        self.assertEqual(reviewed["case"]["status"], "exportable")
        self.assertEqual(reviewed["purpose"]["type"], "planned_exchange_sale")
        self.assertEqual(reviewed["purpose"]["planned_destination"], "Example Exchange")
        self.assertIn("target-deposit-1", reviewed["disclosure_preview"]["txids"])
        self.assertIn("Exchange fiat purchase statement", [item["label"] for item in reviewed["disclosure_preview"]["attachments"]])
        self.assertIn("missing_history", {item["code"] for item in reviewed["gaps"]})

        cases = self.cli("source-funds", "cases", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        self.assertEqual(cases[0]["id"], reviewed["case"]["id"])
        pdf_path = self.root / "source-funds.pdf"
        exported = self.cli(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            reviewed["case"]["id"],
            "--file",
            str(pdf_path),
        )["data"]
        self.assertEqual(exported["scope"], "source_funds")
        self.assertTrue(pdf_path.exists())
        self.assertGreater(pdf_path.stat().st_size, 1000)
        self.assertTrue(pdf_path.read_bytes().startswith(b"%PDF-1.4"))


if __name__ == "__main__":
    unittest.main()
