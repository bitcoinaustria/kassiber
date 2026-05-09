import json
import sqlite3
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

    def _init_default_workspace(self):
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

    def _write_csv(self, name: str, content: str):
        (self.root / name).write_text(content, encoding="utf-8")

    def _seed_cycle_wallets(self):
        self._init_default_workspace()
        for label, name, txid in [
            ("Target", "target-a.csv", "target-a"),
            ("Parent B", "parent-b.csv", "parent-b"),
            ("Parent C", "parent-c.csv", "parent-c"),
            ("Parent D", "parent-d.csv", "parent-d"),
        ]:
            self._write_csv(
                name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-02-01T09:00:00Z,{txid},inbound,BTC,0.20000000,0,50000,{txid}\n",
            )
            self._create_wallet_and_import(label, name)

    def _seed_single_target(self, amount: str = "0.20000000"):
        self._init_default_workspace()
        self._write_csv(
            "target-basic.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-02-01T09:00:00Z,target-basic,inbound,BTC,{amount},0,50000,Target deposit\n",
        )
        self._create_wallet_and_import("Target", "target-basic.csv")

    def _db(self):
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        return conn

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
        raise AssertionError(f"transaction {external_id} not found in wallet {wallet}")

    def _report_blockers(self, target: str = "target-basic", amount: str = "0.20000000", *, max_depth: str | None = None):
        args = [
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target,
            "--target-amount",
            amount,
        ]
        if max_depth:
            args.extend(["--max-depth", max_depth])
        report = self.cli(*args)["data"]
        return {item["code"] for item in report["explain_gates"]["blockers"]}, report

    def _seed_exportable_disclosure_path(self):
        self._init_default_workspace()
        for wallet, csv_name, txid in [
            ("Grandparent", "disclosure-grand.csv", "disclosure-grand"),
            ("Parent", "disclosure-parent.csv", "disclosure-parent"),
            ("Target", "disclosure-target.csv", "disclosure-target"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-04-01T09:00:00Z,{txid},inbound,BTC,0.10000000,0,50000,Reviewed path row\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        file_attachment = self.cli(
            "attachments",
            "add",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--transaction",
            self._tx_id("Grandparent", "disclosure-grand"),
            "--file",
            str(self.evidence_file),
            "--label",
            "Disclosure file evidence",
        )["data"]
        url_attachment = self.cli(
            "attachments",
            "add",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--transaction",
            self._tx_id("Grandparent", "disclosure-grand"),
            "--url",
            "https://exchange.example/source-statement",
            "--label",
            "Disclosure URL evidence",
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
            "Reviewed disclosure source",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
            "--attachment",
            file_attachment["id"],
            "--attachment",
            url_attachment["id"],
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
            source["id"],
            "--to-transaction",
            "disclosure-grand",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        for from_tx, to_tx in [
            ("disclosure-grand", "disclosure-parent"),
            ("disclosure-parent", "disclosure-target"),
        ]:
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                from_tx,
                "--to-transaction",
                to_tx,
                "--type",
                "self_transfer",
                "--allocation-amount",
                "0.10000000",
                "--from-amount",
                "0.10000000",
                "--allocation-policy",
                "explicit",
            )
        return {
            "target": "disclosure-target",
            "file_attachment": file_attachment["id"],
            "url_attachment": url_attachment["id"],
        }

    def _source_funds_report(self, *, reveal_mode: str = "standard", save_case: bool = False):
        return self._source_funds_report_for_target(
            target="disclosure-target",
            amount="0.10000000",
            reveal_mode=reveal_mode,
            save_case=save_case,
        )

    def _source_funds_report_for_target(
        self,
        *,
        target: str,
        amount: str,
        reveal_mode: str = "standard",
        save_case: bool = False,
    ):
        args = [
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target,
            "--target-amount",
            amount,
            "--reveal-mode",
            reveal_mode,
        ]
        if save_case:
            args.append("--save-case")
        return self.cli(*args)["data"]

    def test_source_funds_review_gates_snapshot_and_pdf(self):
        self._init_default_workspace()
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
        )
        methods = {row["method"] for row in suggested["data"]["links"]}
        self.assertIn("same_external_id", methods)
        self.assertIn("transaction_pair", methods)
        self.assertTrue(any(row["link_type"] == "swap" for row in suggested["data"]["links"]))

        exchange_tx_id = self._tx_id("Exchange", "withdraw-1")
        cold_in_tx_id = self._tx_id("Cold", "withdraw-1")
        cold_out_tx_id = self._tx_id("Cold", "self-hop-1")
        privacy_in_tx_id = self._tx_id("Privacy", "self-hop-1")
        privacy_out_tx_id = self._tx_id("Privacy", "coinjoin-hop-1")
        swap_in_tx_id = self._tx_id("Liquid", "swap-in-leg")

        bulk_reviewed_links = []
        for target_id in (cold_in_tx_id, privacy_in_tx_id, swap_in_tx_id):
            bulk_reviewed = self.cli(
                "source-funds",
                "links",
                "bulk-review",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--target-transaction",
                target_id,
            )["data"]
            bulk_reviewed_links.extend(bulk_reviewed["links"])
        self.assertGreaterEqual(len(bulk_reviewed_links), 3)
        self.assertTrue(
            all(link["allocation_policy"] == "explicit" for link in bulk_reviewed_links)
        )

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

    def test_reveal_modes_redact_txids_and_attachment_paths(self):
        self._seed_exportable_disclosure_path()
        expected_txids = {
            "labels_only": [],
            "minimal": ["disclosure-target"],
            "standard": ["disclosure-grand", "disclosure-parent", "disclosure-target"],
            "full": ["disclosure-grand", "disclosure-parent", "disclosure-target"],
        }
        for mode, txids in expected_txids.items():
            with self.subTest(mode=mode):
                report = self._source_funds_report(reveal_mode=mode)
                self.assertEqual(report["disclosure_preview"]["txids"], txids)
                serialized = json.dumps(report)
                if mode == "labels_only":
                    self.assertNotIn("disclosure-target", serialized)
                if mode in {"labels_only", "minimal"}:
                    self.assertNotIn("disclosure-parent", serialized)
                    self.assertNotIn("disclosure-grand", serialized)
                attachments = {
                    item["label"]: item
                    for item in report["disclosure_preview"]["attachments"]
                }
                self.assertIn("Disclosure file evidence", attachments)
                self.assertIn("Disclosure URL evidence", attachments)
                for attachment in attachments.values():
                    if mode != "full":
                        self.assertNotIn("source_url", attachment)
                        self.assertNotIn("stored_relpath", attachment)
                    if mode in {"labels_only", "minimal"}:
                        self.assertNotIn("sha256", attachment)
                        self.assertNotIn("media_type", attachment)
                    else:
                        self.assertIn("sha256", attachment)
                        self.assertIn("media_type", attachment)
                if mode == "full":
                    file_attachment = next(
                        item
                        for item in report["disclosure_preview"]["attachments"]
                        if item["label"] == "Disclosure file evidence"
                    )
                    url_attachment = next(
                        item
                        for item in report["disclosure_preview"]["attachments"]
                        if item["label"] == "Disclosure URL evidence"
                    )
                    self.assertIn("stored_relpath", file_attachment)
                    self.assertTrue(file_attachment["stored_relpath"])
                    self.assertIn("source_url", url_attachment)
                    self.assertEqual(url_attachment["source_url"], "https://exchange.example/source-statement")

    def test_reveal_modes_redact_free_text_description(self):
        """Free-text fields (description, counterparty) leak personal
        memos. labels_only and minimal modes must drop them; standard
        and full modes keep them."""
        self._seed_exportable_disclosure_path()
        for mode in ("labels_only", "minimal"):
            with self.subTest(mode=mode):
                report = self._source_funds_report(reveal_mode=mode)
                serialized = json.dumps(report)
                self.assertNotIn("Reviewed path row", serialized)
                for node in report["graph"]["nodes"]:
                    if node.get("node_type") == "transaction":
                        self.assertEqual(node.get("description"), "")
                        self.assertEqual(node.get("counterparty"), "")
        for mode in ("standard", "full"):
            with self.subTest(mode=mode):
                report = self._source_funds_report(reveal_mode=mode)
                serialized = json.dumps(report)
                self.assertIn("Reviewed path row", serialized)

    def test_reveal_modes_redact_provider_ids_in_link_explanations(self):
        """Suggestion-builder explanations carry provider key/value
        pairs (trade ID, order ID, ...). Those values would leak through
        the edge.explanation field on the report envelope at every
        reveal mode if the publisher never redacted free text. Pin the
        gate so labels_only and minimal mode never serialize the
        provider ID, while standard and full keep it visible."""
        self._init_default_workspace()
        # Two same-asset trades that share a provider trade_id pair.
        for wallet, csv_name, txid, direction, raw in [
            (
                "Exchange Out",
                "provider-out.csv",
                "exchange-trade-1",
                "outbound",
                '{"trade_id":"PROVIDER-TRADE-LEAK"}',
            ),
            (
                "Exchange In",
                "provider-in.csv",
                "exchange-trade-2",
                "inbound",
                '{"trade_id":"PROVIDER-TRADE-LEAK"}',
            ),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description,raw_json\n"
                f"2026-04-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,row,{raw}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        # Run suggest, then promote the suggested link to reviewed and
        # set explicit allocation. (Suggested state would block export
        # via unreviewed_link.)
        self.cli(
            "source-funds", "suggest", "--workspace", "Sof", "--profile", "Default",
        )
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default",
        )["data"]
        leaked = next(
            (link for link in links if "PROVIDER-TRADE-LEAK" in (link.get("explanation") or "")),
            None,
        )
        if leaked is None:
            self.skipTest("Suggestion seed did not produce a provider-id link")
        self.cli(
            "source-funds",
            "links",
            "review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--link",
            leaked["id"],
            "--state",
            "reviewed",
            "--allocation-policy",
            "explicit",
            "--allocation-amount",
            "0.10000000",
        )
        # Need a reviewed source to root the reviewed path.
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
            "Provider source",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
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
            source["id"],
            "--to-transaction",
            "exchange-trade-1",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        for mode in ("labels_only", "minimal"):
            with self.subTest(mode=mode):
                report = self._source_funds_report_for_target(
                    target="exchange-trade-2",
                    amount="0.10000000",
                    reveal_mode=mode,
                )
                serialized = json.dumps(report)
                self.assertNotIn("PROVIDER-TRADE-LEAK", serialized)
                for edge in report["graph"]["edges"]:
                    self.assertEqual(edge.get("explanation", ""), "")
        for mode in ("standard", "full"):
            with self.subTest(mode=mode):
                report = self._source_funds_report_for_target(
                    target="exchange-trade-2",
                    amount="0.10000000",
                    reveal_mode=mode,
                )
                serialized = json.dumps(report)
                self.assertIn("PROVIDER-TRADE-LEAK", serialized)

    def test_export_requires_saved_case_snapshot(self):
        self._seed_exportable_disclosure_path()
        error = self.cli_error(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "disclosure-target",
            "--target-amount",
            "0.10000000",
            "--file",
            str(self.root / "live-export.pdf"),
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_export_via_case_matches_preview_snapshot_hash(self):
        self._seed_exportable_disclosure_path()
        preview = self._source_funds_report(save_case=True)
        pdf_path = self.root / "case-export.pdf"
        exported = self.cli(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            preview["case"]["id"],
            "--file",
            str(pdf_path),
        )["data"]
        self.assertEqual(exported["snapshot_hash"], preview["case"]["snapshot_hash"])
        self.assertTrue(pdf_path.exists())

    def test_export_case_uses_frozen_snapshot_after_live_mutation(self):
        self._seed_exportable_disclosure_path()
        preview = self._source_funds_report(save_case=True)
        with self._db() as conn:
            conn.execute(
                """
                UPDATE source_funds_links
                SET state = 'rejected'
                WHERE to_transaction_id = (
                    SELECT id FROM transactions WHERE external_id = 'disclosure-target'
                )
                """
            )
        live = self._source_funds_report()
        self.assertFalse(live["explain_gates"]["exportable"])
        exported = self.cli(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            preview["case"]["id"],
            "--file",
            str(self.root / "frozen-export.pdf"),
        )["data"]
        self.assertEqual(exported["snapshot_hash"], preview["case"]["snapshot_hash"])

    def test_cases_list_snapshots_target_external_id(self):
        """A later rename of the target transaction's external_id must
        not rewrite history in cases list. Snapshot the value once at
        save time."""
        self._seed_exportable_disclosure_path()
        preview = self._source_funds_report(save_case=True)
        case_id = preview["case"]["id"]
        with self._db() as conn:
            conn.execute(
                "UPDATE transactions SET external_id = ? WHERE external_id = ?",
                ("renamed-after-save", "disclosure-target"),
            )
        listing = self.cli(
            "source-funds",
            "cases",
            "list",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
        )["data"]
        case = next(item for item in listing if item["id"] == case_id)
        self.assertEqual(case["target_external_id"], "disclosure-target")

    def test_self_link_rejected_at_create_time(self):
        self._seed_cycle_wallets()
        error = self.cli_error(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "target-a",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_two_node_cycle_emits_path_cycle_blocker(self):
        self._seed_cycle_wallets()
        for from_tx, to_tx in [("parent-b", "target-a"), ("target-a", "parent-b")]:
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                from_tx,
                "--to-transaction",
                to_tx,
                "--type",
                "self_transfer",
                "--allocation-amount",
                "0.20000000",
                "--from-amount",
                "0.20000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, report = self._report_blockers("target-a", "0.20000000")
        self.assertIn("path_cycle", blockers)
        self.assertFalse(report["explain_gates"]["exportable"])

    def test_long_cycle_caught_before_path_truncated(self):
        self._seed_cycle_wallets()
        for from_tx, to_tx in [
            ("parent-b", "target-a"),
            ("parent-c", "parent-b"),
            ("parent-d", "parent-c"),
            ("target-a", "parent-d"),
        ]:
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                from_tx,
                "--to-transaction",
                to_tx,
                "--type",
                "self_transfer",
                "--allocation-amount",
                "0.20000000",
                "--from-amount",
                "0.20000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, _ = self._report_blockers("target-a", "0.20000000", max_depth="10")
        self.assertIn("path_cycle", blockers)
        self.assertNotIn("path_truncated", blockers)

    def test_export_blocks_when_source_asset_differs_from_link_asset(self):
        self._seed_single_target("0.10000000")
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
            "Liquid purchase",
            "--asset",
            "L-BTC",
            "--amount",
            "0.10000000",
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
            source["id"],
            "--to-transaction",
            "target-basic",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--from-asset",
            "BTC",
            "--allocation-policy",
            "explicit",
        )
        blockers, _ = self._report_blockers("target-basic", "0.10000000")
        self.assertIn("source_asset_mismatch", blockers)

    def test_export_blocks_when_two_links_overallocate_one_source(self):
        self._seed_single_target("0.20000000")
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
            "Small purchase",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
        )["data"]
        for method in ("manual-a", "manual-b"):
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
                "target-basic",
                "--type",
                "manual_source",
                "--method",
                method,
                "--allocation-amount",
                "0.10000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, report = self._report_blockers("target-basic", "0.20000000")
        self.assertIn("source_overallocation", blockers)
        self.assertFalse(report["explain_gates"]["exportable"])

    def test_links_create_chain_observation_defaults_to_unconfirmed(self):
        """A manually-created chain_observation link must not satisfy
        the export gate by default. The user has to explicitly mark
        the observation as confirmed via --chain-data-confirmed."""
        self._init_default_workspace()
        for wallet, csv_name, txid in [
            ("Origin", "origin-tx.csv", "origin-tx"),
            ("Target", "target-tx.csv", "target-tx"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-04-01T09:00:00Z,{txid},inbound,BTC,0.10000000,0,50000,row\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        link = self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "origin-tx",
            "--to-transaction",
            "target-tx",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
            "--uses-chain-observation",
        )["data"]
        self.assertTrue(link["uses_chain_observation"])
        self.assertFalse(link["chain_data_confirmed"])

    def test_export_blocks_when_attestation_source_with_amount_overallocates(self):
        self._seed_single_target("0.20000000")
        source = self.cli(
            "source-funds",
            "sources",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--type",
            "opening_balance_attestation",
            "--label",
            "Opening balance attestation",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
        )["data"]
        for method in ("attest-a", "attest-b"):
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
                "target-basic",
                "--type",
                "manual_source",
                "--method",
                method,
                "--allocation-amount",
                "0.10000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, report = self._report_blockers("target-basic", "0.20000000")
        self.assertIn("source_overallocation", blockers)
        self.assertFalse(report["explain_gates"]["exportable"])

    def test_export_blocks_when_concrete_source_has_null_amount(self):
        self._seed_single_target("0.10000000")
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
            "Unquantified purchase",
            "--asset",
            "BTC",
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
            source["id"],
            "--to-transaction",
            "target-basic",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        blockers, _ = self._report_blockers("target-basic", "0.10000000")
        self.assertIn("source_amount_missing", blockers)

    def test_export_allows_under_allocation_against_source_with_amount(self):
        self._seed_single_target("0.10000000")
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
            "Larger purchase",
            "--asset",
            "BTC",
            "--amount",
            "0.20000000",
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
            source["id"],
            "--to-transaction",
            "target-basic",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        blockers, report = self._report_blockers("target-basic", "0.10000000")
        self.assertNotIn("source_overallocation", blockers)
        self.assertNotIn("source_amount_missing", blockers)
        self.assertTrue(report["explain_gates"]["exportable"], blockers)

    def test_repeated_parent_allocations_sum_before_upstream_gate(self):
        self._init_default_workspace()
        self._write_csv(
            "target-repeat.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-04-01T09:00:00Z,target-repeat,inbound,BTC,1.00000000,0,50000,Target deposit\n",
        )
        self._write_csv(
            "parent-repeat.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-repeat,inbound,BTC,1.00000000,0,40000,Parent funds\n",
        )
        self._create_wallet_and_import("Target", "target-repeat.csv")
        self._create_wallet_and_import("Parent", "parent-repeat.csv")
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
            "Partial parent source",
            "--asset",
            "BTC",
            "--amount",
            "0.50000000",
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
            source["id"],
            "--to-transaction",
            "parent-repeat",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.50000000",
            "--allocation-policy",
            "explicit",
        )
        for method in ("split-a", "split-b"):
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                "parent-repeat",
                "--to-transaction",
                "target-repeat",
                "--type",
                "self_transfer",
                "--method",
                method,
                "--allocation-amount",
                "0.50000000",
                "--from-amount",
                "0.50000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, report = self._report_blockers("target-repeat", "1.00000000")
        self.assertIn("ambiguous_allocation", blockers)
        parent_node = next(
            node for node in report["graph"]["nodes"] if node.get("transaction_id") and node["label"] == "parent-repeat"
        )
        self.assertEqual(parent_node["required_amount_msat"], 100_000_000_000)
        self.assertFalse(report["explain_gates"]["exportable"])

    def test_repeated_parent_allocations_pass_with_summed_evidence(self):
        self._init_default_workspace()
        self._write_csv(
            "target-repeat.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-04-01T09:00:00Z,target-repeat,inbound,BTC,1.00000000,0,50000,Target deposit\n",
        )
        self._write_csv(
            "parent-repeat.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-repeat,inbound,BTC,1.00000000,0,40000,Parent funds\n",
        )
        self._create_wallet_and_import("Target", "target-repeat.csv")
        self._create_wallet_and_import("Parent", "parent-repeat.csv")
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
            "Complete parent source",
            "--asset",
            "BTC",
            "--amount",
            "1.00000000",
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
            source["id"],
            "--to-transaction",
            "parent-repeat",
            "--type",
            "manual_source",
            "--allocation-amount",
            "1.00000000",
            "--allocation-policy",
            "explicit",
        )
        for method in ("split-a", "split-b"):
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                "parent-repeat",
                "--to-transaction",
                "target-repeat",
                "--type",
                "self_transfer",
                "--method",
                method,
                "--allocation-amount",
                "0.50000000",
                "--from-amount",
                "0.50000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, report = self._report_blockers("target-repeat", "1.00000000")
        self.assertNotIn("ambiguous_allocation", blockers)
        self.assertTrue(report["explain_gates"]["exportable"], blockers)

    def test_create_rejects_allocation_above_transaction_amount(self):
        self._seed_cycle_wallets()
        error = self.cli_error(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.30000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")
        error = self.cli_error(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.30000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_export_blocks_when_link_allocation_exceeds_parent_tx_amount(self):
        self._seed_cycle_wallets()
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
            "Parent source",
            "--asset",
            "BTC",
            "--amount",
            "0.20000000",
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
            source["id"],
            "--to-transaction",
            "parent-b",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )
        link = self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )["data"]
        with self._db() as conn:
            conn.execute(
                "UPDATE source_funds_links SET from_allocation_amount = ? WHERE id = ?",
                (50_000_000_000, link["id"]),
            )
        blockers, _ = self._report_blockers("target-a", "0.20000000")
        self.assertIn("transaction_overallocation", blockers)

    def test_export_blocks_when_two_downstream_links_overconsume_one_parent(self):
        self._init_default_workspace()
        self._write_csv(
            "target-overconsume.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-04-01T09:00:00Z,target-overconsume,inbound,BTC,1.00000000,0,50000,Target deposit\n",
        )
        self._write_csv(
            "parent-overconsume.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-overconsume,inbound,BTC,0.50000000,0,40000,Parent funds\n",
        )
        self._create_wallet_and_import("Target", "target-overconsume.csv")
        self._create_wallet_and_import("Parent", "parent-overconsume.csv")
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
            "Half bitcoin source",
            "--asset",
            "BTC",
            "--amount",
            "0.50000000",
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
            source["id"],
            "--to-transaction",
            "parent-overconsume",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.50000000",
            "--allocation-policy",
            "explicit",
        )
        for method in ("branch-a", "branch-b"):
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                "parent-overconsume",
                "--to-transaction",
                "target-overconsume",
                "--type",
                "self_transfer",
                "--method",
                method,
                "--allocation-amount",
                "0.50000000",
                "--from-amount",
                "0.50000000",
                "--allocation-policy",
                "explicit",
            )
        blockers, _ = self._report_blockers("target-overconsume", "1.00000000")
        self.assertIn("transaction_overallocation", blockers)

    def test_self_transfer_link_with_fee_tolerance_passes(self):
        self._init_default_workspace()
        self._write_csv(
            "target-fee.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-04-01T09:00:00Z,target-fee,inbound,BTC,0.10000000,0,50000,Target deposit\n",
        )
        self._write_csv(
            "parent-fee.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-fee,inbound,BTC,0.10010000,0,40000,Parent funds\n",
        )
        self._create_wallet_and_import("Target", "target-fee.csv")
        self._create_wallet_and_import("Parent", "parent-fee.csv")
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
            "Fee-inclusive source",
            "--asset",
            "BTC",
            "--amount",
            "0.10010000",
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
            source["id"],
            "--to-transaction",
            "parent-fee",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10010000",
            "--allocation-policy",
            "explicit",
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
            "parent-fee",
            "--to-transaction",
            "target-fee",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10010000",
            "--allocation-policy",
            "explicit",
        )
        blockers, report = self._report_blockers("target-fee", "0.10000000")
        self.assertNotIn("transaction_overallocation", blockers)
        self.assertTrue(report["explain_gates"]["exportable"], blockers)

    def test_self_transfer_link_rejects_asset_mismatch_at_create(self):
        self._seed_cycle_wallets()
        error = self.cli_error(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--from-asset",
            "EUR",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_self_transfer_link_blocks_export_on_asset_mismatch(self):
        self._seed_cycle_wallets()
        link = self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )["data"]
        with self._db() as conn:
            conn.execute("UPDATE source_funds_links SET from_asset = 'EUR' WHERE id = ?", (link["id"],))
        blockers, _ = self._report_blockers("target-a", "0.20000000")
        self.assertIn("asset_mismatch", blockers)

    def test_bulk_review_is_target_scoped(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction in [
            ("First Out", "first-out.csv", "pair-one", "outbound"),
            ("First In", "first-in.csv", "pair-one", "inbound"),
            ("Second Out", "second-out.csv", "pair-two", "outbound"),
            ("Second In", "second-in.csv", "pair-two", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")
        first_target = self._tx_id("First In", "pair-one")
        second_target = self._tx_id("Second In", "pair-two")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            first_target,
        )["data"]
        self.assertEqual(reviewed["reviewed"], 1)
        links = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        first_link = next(link for link in links if link["to_transaction_id"] == first_target)
        second_link = next(link for link in links if link["to_transaction_id"] == second_target)
        self.assertEqual(first_link["state"], "reviewed")
        self.assertEqual(second_link["state"], "suggested")

    def test_bulk_review_skips_same_external_id_when_third_row_appears(self):
        self._init_default_workspace()
        for wallet, csv_name, direction in [
            ("Pair Out", "stale-pair-out.csv", "outbound"),
            ("Pair In", "stale-pair-in.csv", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,stale-pair,{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        target_id = self._tx_id("Pair In", "stale-pair")
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]["links"]
        self.assertEqual(len([link for link in suggested if link["method"] == "same_external_id"]), 1)
        self._write_csv(
            "stale-third-in.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:05:00Z,stale-pair,inbound,BTC,0.10000000,0,50000,Third matching row\n",
        )
        self._create_wallet_and_import("Pair Third", "stale-third-in.csv")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)
        link = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"][0]
        self.assertEqual(link["state"], "suggested")

    def test_bulk_review_skips_transaction_pair_when_pair_row_deleted(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction in [
            ("Pair Out", "deleted-pair-out.csv", "deleted-pair-out", "outbound"),
            ("Pair In", "deleted-pair-in.csv", "deleted-pair-in", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        out_id = self._tx_id("Pair Out", "deleted-pair-out")
        in_id = self._tx_id("Pair In", "deleted-pair-in")
        self.cli(
            "transfers",
            "pair",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--tx-out",
            out_id,
            "--tx-in",
            in_id,
            "--kind",
            "manual",
            "--policy",
            "carrying-value",
        )
        self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            in_id,
        )
        with self._db() as conn:
            conn.execute("DELETE FROM transaction_pairs")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            in_id,
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)

    def test_suggest_links_with_target_does_not_write_unrelated_suggestions(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction in [
            ("First Out", "first-out.csv", "pair-one", "outbound"),
            ("First In", "first-in.csv", "pair-one", "inbound"),
            ("Second Out", "second-out.csv", "pair-two", "outbound"),
            ("Second In", "second-in.csv", "pair-two", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        first_target = self._tx_id("First In", "pair-one")
        second_target = self._tx_id("Second In", "pair-two")
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            first_target,
        )["data"]["links"]
        self.assertEqual(len(suggested), 1)
        self.assertEqual(suggested[0]["to_transaction_id"], first_target)
        links = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        self.assertFalse(any(link["to_transaction_id"] == second_target for link in links))

    def test_suggest_links_caps_writes_per_call(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction in [
            ("First Out", "cap-first-out.csv", "cap-one", "outbound"),
            ("First In", "cap-first-in.csv", "cap-one", "inbound"),
            ("Second Out", "cap-second-out.csv", "cap-two", "outbound"),
            ("Second In", "cap-second-in.csv", "cap-two", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        error = self.cli_error(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--max-suggestions",
            "1",
        )
        self.assertEqual(error["error"]["code"], "validation")
        links = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        self.assertEqual(links, [])

    def test_create_link_rejects_parent_after_child(self):
        self._init_default_workspace()
        self._write_csv(
            "child-early.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-02-01T09:00:00Z,child-early,inbound,BTC,0.10000000,0,50000,Child transaction\n",
        )
        self._write_csv(
            "parent-late.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-late,inbound,BTC,0.10000000,0,50000,Future parent\n",
        )
        self._create_wallet_and_import("Child", "child-early.csv")
        self._create_wallet_and_import("Parent", "parent-late.csv")
        error = self.cli_error(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "parent-late",
            "--to-transaction",
            "child-early",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_create_link_rejects_source_acquired_after_child(self):
        self._seed_single_target("0.10000000")
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
            "Future purchase",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
            "--acquired-at",
            "2026-03-01T00:00:00Z",
        )["data"]
        error = self.cli_error(
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
            "target-basic",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_export_blocks_chronology_violation_on_existing_reviewed_link(self):
        self._init_default_workspace()
        self._write_csv(
            "target-chronology.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-04-01T09:00:00Z,target-chronology,inbound,BTC,0.10000000,0,50000,Target\n",
        )
        self._write_csv(
            "parent-chronology.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,parent-chronology,inbound,BTC,0.10000000,0,50000,Parent\n",
        )
        self._create_wallet_and_import("Target", "target-chronology.csv")
        self._create_wallet_and_import("Parent", "parent-chronology.csv")
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
            "Chronology source",
            "--asset",
            "BTC",
            "--amount",
            "0.10000000",
            "--acquired-at",
            "2026-02-01T00:00:00Z",
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
            source["id"],
            "--to-transaction",
            "parent-chronology",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
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
            "parent-chronology",
            "--to-transaction",
            "target-chronology",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        with self._db() as conn:
            conn.execute(
                "UPDATE transactions SET occurred_at = ? WHERE external_id = ?",
                ("2026-05-01T09:00:00Z", "parent-chronology"),
            )
        blockers, _ = self._report_blockers("target-chronology", "0.10000000")
        self.assertIn("chronology_violation", blockers)

    def test_same_timestamp_link_is_allowed(self):
        self._seed_cycle_wallets()
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
            "Same timestamp source",
            "--asset",
            "BTC",
            "--amount",
            "0.20000000",
            "--acquired-at",
            "2026-02-01T09:00:00Z",
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
            source["id"],
            "--to-transaction",
            "parent-b",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
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
            "parent-b",
            "--to-transaction",
            "target-a",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )
        blockers, report = self._report_blockers("target-a", "0.20000000")
        self.assertNotIn("chronology_violation", blockers)
        self.assertTrue(report["explain_gates"]["exportable"], blockers)

    def test_undated_attestation_source_emits_warning_not_blocker(self):
        self._seed_single_target("0.10000000")
        source = self.cli(
            "source-funds",
            "sources",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--type",
            "opening_balance_attestation",
            "--label",
            "Reviewed prior history",
            "--asset",
            "BTC",
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
            source["id"],
            "--to-transaction",
            "target-basic",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        blockers, report = self._report_blockers("target-basic", "0.10000000")
        warning_codes = {item["code"] for item in report["explain_gates"]["warnings"]}
        self.assertNotIn("chronology_violation", blockers)
        self.assertIn("opening_balance_attestation", warning_codes)
        self.assertTrue(report["explain_gates"]["exportable"], blockers)

    def _seed_provider_rows(self, *, out_rows: list[tuple[str, str, str]], in_rows: list[tuple[str, str, str]], headers: str):
        self._init_default_workspace()
        out_lines = ["date,txid,direction,asset,amount,fee,fiat_rate,description," + headers]
        for txid, amount, extra in out_rows:
            out_lines.append(f"2026-03-01T09:00:00Z,{txid},outbound,BTC,{amount},0,50000,{txid},{extra}")
        in_lines = ["date,txid,direction,asset,amount,fee,fiat_rate,description," + headers]
        for txid, amount, extra in in_rows:
            in_lines.append(f"2026-03-01T09:05:00Z,{txid},inbound,BTC,{amount},0,50000,{txid},{extra}")
        self._write_csv("provider-out.csv", "\n".join(out_lines) + "\n")
        self._write_csv("provider-in.csv", "\n".join(in_lines) + "\n")
        self._create_wallet_and_import("Provider Out", "provider-out.csv")
        self._create_wallet_and_import("Provider In", "provider-in.csv")

    def test_provider_id_with_three_outs_three_ins_does_not_bulk_review(self):
        self._seed_provider_rows(
            out_rows=[("out-1", "0.10000000", "acct-1"), ("out-2", "0.20000000", "acct-1"), ("out-3", "0.30000000", "acct-1")],
            in_rows=[("in-1", "0.10000000", "acct-1"), ("in-2", "0.20000000", "acct-1"), ("in-3", "0.30000000", "acct-1")],
            headers="provider_id",
        )
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--include-broad-hints",
        )["data"]["links"]
        provider_links = [link for link in suggested if link["method"] == "provider_id"]
        self.assertEqual(len(provider_links), 9)
        self.assertTrue(all(link["confidence"] == "weak" for link in provider_links))
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Provider In", "in-1"),
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)

    def test_provider_trade_id_one_to_one_still_bulk_reviews(self):
        self._seed_provider_rows(
            out_rows=[("trade-out", "0.10000000", "trade-1")],
            in_rows=[("trade-in", "0.10000000", "trade-1")],
            headers="trade_id",
        )
        suggested = self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")["data"]["links"]
        trade_links = [link for link in suggested if link["method"] == "provider_trade_id"]
        self.assertEqual(len(trade_links), 1)
        self.assertEqual(trade_links[0]["confidence"], "strong")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Provider In", "trade-in"),
        )["data"]
        self.assertEqual(reviewed["reviewed"], 1)
        self.assertEqual(reviewed["links"][0]["method"], "provider_trade_id")

    def test_bulk_review_skips_provider_trade_id_when_imports_made_it_n_to_m(self):
        self._seed_provider_rows(
            out_rows=[("trade-out", "0.10000000", "trade-1")],
            in_rows=[("trade-in", "0.10000000", "trade-1")],
            headers="trade_id",
        )
        target_id = self._tx_id("Provider In", "trade-in")
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]["links"]
        self.assertEqual(len([link for link in suggested if link["method"] == "provider_trade_id"]), 1)
        self._write_csv(
            "provider-extra-out.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description,trade_id\n"
            "2026-03-01T09:02:00Z,trade-extra-out,outbound,BTC,0.10000000,0,50000,extra,trade-1\n",
        )
        self._create_wallet_and_import("Provider Extra", "provider-extra-out.csv")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)

    def test_broad_provider_id_requires_explicit_opt_in(self):
        self._seed_provider_rows(
            out_rows=[("provider-out", "0.10000000", "acct-1")],
            in_rows=[("provider-in", "0.10000000", "acct-1")],
            headers="provider_id",
        )
        suggested = self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")["data"]["links"]
        provider_links = [link for link in suggested if link["method"] == "provider_id"]
        self.assertEqual(provider_links, [])
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--include-broad-hints",
        )["data"]["links"]
        provider_links = [link for link in suggested if link["method"] == "provider_id"]
        self.assertEqual(len(provider_links), 1)
        self.assertEqual(provider_links[0]["confidence"], "weak")

    def test_bulk_review_skips_amount_mismatched_suggestions(self):
        self._seed_provider_rows(
            out_rows=[("mismatch-out", "0.10000000", "trade-1")],
            in_rows=[("mismatch-in", "0.50000000", "trade-1")],
            headers="trade_id",
        )
        suggested = self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")["data"]["links"]
        trade_links = [link for link in suggested if link["method"] == "provider_trade_id"]
        self.assertEqual(len(trade_links), 1)
        self.assertEqual(trade_links[0]["confidence"], "weak")
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Provider In", "mismatch-in"),
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)


if __name__ == "__main__":
    unittest.main()
