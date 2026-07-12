import json
import shutil
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

    def _create_wallet_and_import(
        self,
        label: str,
        csv_name: str,
        *,
        chain: str | None = None,
        network: str | None = None,
    ):
        create_args = [
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
        ]
        if chain:
            create_args.extend(["--chain", chain])
        if network:
            create_args.extend(["--network", network])
        self.cli(*create_args)
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

    def test_source_funds_report_warns_on_privacy_boundary(self):
        self._seed_single_target()
        target_id = self._tx_id("Target", "target-basic")
        with self._db() as conn:
            conn.execute(
                "UPDATE transactions SET privacy_boundary = ?, raw_json = ? WHERE id = ?",
                (
                    "payjoin",
                    json.dumps({"privacy_hop": "payjoin", "source": "privacy_import"}),
                    target_id,
                ),
            )
            conn.commit()

        report = self._source_funds_report_for_target(
            target="target-basic",
            amount="0.20000000",
        )
        codes = {finding["code"] for finding in report["findings"]}
        self.assertIn("privacy_hop_unresolved", codes)

    def test_privacy_boundary_import_skips_same_onchain_scope_suggestion(self):
        self._init_default_workspace()
        self._write_csv(
            "privacy-out.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description,privacyHop\n"
            "2026-03-01T09:00:00Z,privacy-pair,outbound,BTC,0.10000000,0,50000,Privacy out,coinjoin\n",
        )
        self._write_csv(
            "privacy-in.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:05:00Z,privacy-pair,inbound,BTC,0.10000000,0,50000,Privacy in\n",
        )
        self._create_wallet_and_import("Privacy Out", "privacy-out.csv")
        self._create_wallet_and_import("Privacy In", "privacy-in.csv")

        with self._db() as conn:
            stored = conn.execute(
                "SELECT privacy_boundary FROM transactions WHERE external_id = ? AND direction = ?",
                ("privacy-pair", "outbound"),
            ).fetchone()
        self.assertEqual(stored["privacy_boundary"], "coinjoin")

        target_id = self._tx_id("Privacy In", "privacy-pair")
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
        self.assertFalse(
            [
                link
                for link in suggested
                if link["method"] == "same_onchain_scope"
                and link["to_transaction_id"] == target_id
            ]
        )

    def test_invalid_privacy_boundary_import_is_validation_error(self):
        self._init_default_workspace()
        self._write_csv(
            "bad-privacy.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description,privacy_boundary\n"
            "2026-03-01T09:00:00Z,bad-privacy,inbound,BTC,0.10000000,0,50000,Bad privacy,mixish\n",
        )
        self.cli(
            "wallets",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--label",
            "Bad Privacy",
            "--kind",
            "custom",
        )

        error = self.cli_error(
            "wallets",
            "import-csv",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--wallet",
            "Bad Privacy",
            "--file",
            str(self.root / "bad-privacy.csv"),
        )
        self.assertEqual(error["error"]["code"], "validation")

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
        self.assertNotIn("same_onchain_scope", methods)
        self.assertIn("transaction_pair", methods)
        self.assertTrue(any(row["link_type"] == "swap" for row in suggested["data"]["links"]))

        exchange_tx_id = self._tx_id("Exchange", "withdraw-1")
        cold_in_tx_id = self._tx_id("Cold", "withdraw-1")
        cold_out_tx_id = self._tx_id("Cold", "self-hop-1")
        privacy_in_tx_id = self._tx_id("Privacy", "self-hop-1")
        privacy_out_tx_id = self._tx_id("Privacy", "coinjoin-hop-1")
        swap_in_tx_id = self._tx_id("Liquid", "swap-in-leg")

        preview = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            cold_in_tx_id,
            "--dry-run",
        )["data"]
        self.assertTrue(preview["dry_run"])
        self.assertEqual(preview["reviewed"], 0)

        bulk_reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            swap_in_tx_id,
        )["data"]
        bulk_reviewed_links = bulk_reviewed["links"]
        self.assertEqual(len(bulk_reviewed_links), 1)
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
            "BTC ↔ EUR exchange statement",
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
        for from_id, to_id, explanation in (
            (
                exchange_tx_id,
                cold_in_tx_id,
                "Reviewed exchange withdrawal into the owned cold wallet.",
            ),
            (
                cold_out_tx_id,
                privacy_in_tx_id,
                "Reviewed cold-wallet transfer into the owned privacy wallet.",
            ),
        ):
            self.cli(
                "source-funds",
                "links",
                "create",
                "--workspace",
                "Sof",
                "--profile",
                "Default",
                "--from-transaction",
                from_id,
                "--to-transaction",
                to_id,
                "--type",
                "self_transfer",
                "--allocation-amount",
                "0.15000000",
                "--from-amount",
                "0.15000000",
                "--allocation-policy",
                "explicit",
                "--explanation",
                explanation,
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
            if link["state"] == "suggested" and link["id"] != privacy_review_link["id"]:
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
            "--mask-recipient",
            "--save-case",
        )["data"]
        self.assertTrue(reviewed["explain_gates"]["exportable"], reviewed["explain_gates"]["blockers"])
        self.assertEqual(reviewed["case"]["status"], "exportable")
        self.assertEqual(reviewed["purpose"]["type"], "planned_exchange_sale")
        self.assertEqual(reviewed["purpose"]["planned_destination"], "Example Exchange")
        self.assertIn("target-deposit-1", reviewed["disclosure_preview"]["txids"])
        self.assertIn("BTC ↔ EUR exchange statement", [item["label"] for item in reviewed["disclosure_preview"]["attachments"]])
        self.assertIn("missing_history", {item["code"] for item in reviewed["gaps"]})
        self.assertGreaterEqual(reviewed["overview"]["transaction_count"], 5)
        self.assertGreaterEqual(reviewed["overview"]["data_source_count"], 3)
        self.assertTrue(reviewed["narrative"]["paragraphs"])
        self.assertEqual(reviewed["narrative"]["generated_by"], "local_rule_summary")
        self.assertTrue(any(row["label"] == "Cold" for row in reviewed["data_sources"]))
        self.assertEqual(reviewed["flow_levels"][0]["role"], "target")
        self.assertGreaterEqual(len(reviewed["flow_levels"]), 3)
        self.assertTrue(reviewed["simplified_flow"]["deferred_privacy_hops"])
        self.assertTrue(
            any(
                node["deferred_privacy_hop"]
                for level in reviewed["simplified_flow"]["levels"]
                for node in level["nodes"]
            )
        )

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
        self.assertEqual(exported["renderer"], "reportlab")
        self.assertGreater(exported["pages"], 0)
        self.assertTrue(pdf_path.exists())
        self.assertGreater(pdf_path.stat().st_size, 1000)
        self.assertTrue(pdf_path.read_bytes().startswith(b"%PDF"))
        if shutil.which("pdftotext"):
            extracted = subprocess.run(
                ["pdftotext", "-layout", str(pdf_path), "-"],
                check=True,
                text=True,
                capture_output=True,
            ).stdout
            self.assertIn("Kassiber Source of Funds Report", extracted)
            self.assertIn("Reviewed local evidence", extracted)
            self.assertIn("Source of Funds Overview", extracted)
            self.assertIn("Origin and Transaction Flow", extracted)
            self.assertIn("Simplified Flow Path", extracted)
            self.assertIn("(recipient masked)", extracted)
            self.assertNotIn("Example Exchange", extracted)
            self.assertNotIn("Pre-disclosure for expected bank proceeds.", extracted)
            self.assertIn("CoinJoin/PayJoin traversal deferred", extracted)
            self.assertIn("Data Sources", extracted)
            self.assertIn("Transaction Details", extracted)
            self.assertRegex(extracted, r"BTC\s+↔\s+EUR")

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
            "--file",
            str(self.root / "live-export.pdf"),
        )
        self.assertEqual(error["error"]["code"], "validation")

    def test_report_granularity_fields_fee_provenance_levels(self):
        """Every transaction node carries fee + import provenance, levels
        carry per-level fiat subtotals, and the disclosure preview names the
        wallets whose common ownership the report demonstrates."""
        self._init_default_workspace()
        self._write_csv(
            "gran-parent.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,gran-parent,outbound,BTC,0.10005000,0.00005000,50000,Parent spend\n",
        )
        self._write_csv(
            "gran-target.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:30:00Z,gran-target,inbound,BTC,0.10000000,0,50000,Target deposit\n",
        )
        self._create_wallet_and_import("Gran Parent", "gran-parent.csv")
        self._create_wallet_and_import("Gran Target", "gran-target.csv")
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
            "Granularity source",
            "--asset",
            "BTC",
            "--amount",
            "0.10005000",
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
            "gran-parent",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10005000",
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
            "gran-parent",
            "--to-transaction",
            "gran-target",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10005000",
            "--allocation-policy",
            "explicit",
        )
        report = self._source_funds_report_for_target(
            target="gran-target",
            amount="0.10000000",
        )
        tx_nodes = {
            node["external_id"]: node
            for node in report["graph"]["nodes"]
            if node["node_type"] == "transaction"
        }
        parent = tx_nodes["gran-parent"]
        self.assertEqual(parent["fee_msat"], 5_000_000)
        self.assertEqual(parent["fee"], 0.00005)
        self.assertEqual(parent["data_provenance"], "manual_import")
        self.assertEqual(tx_nodes["gran-target"]["fee_msat"], 0)
        self.assertEqual(report["target"]["data_provenance"], "manual_import")

        self.assertEqual(
            report["data_provenance_summary"],
            [
                {
                    "provenance": "manual_import",
                    "label": "Manual / custom import",
                    "count": 2,
                    "percent": 100.0,
                }
            ],
        )

        levels = report["flow_levels"]
        self.assertEqual([level["level"] for level in levels], [1, 2, 3])
        target_level = levels[0]
        self.assertEqual(target_level["role"], "target")
        self.assertEqual(target_level["assets"], ["BTC"])
        self.assertEqual(target_level["fiat_currency"], "EUR")
        self.assertEqual(target_level["fiat_value_total"], 5000.0)
        target_node = target_level["nodes"][0]
        self.assertEqual(target_node["direction"], "inbound")
        self.assertEqual(target_node["data_provenance"], "manual_import")
        parent_node = levels[1]["nodes"][0]
        self.assertEqual(parent_node["direction"], "outbound")
        self.assertEqual(parent_node["fee_msat"], 5_000_000)
        source_node = levels[2]["nodes"][0]
        self.assertEqual(source_node["node_type"], "source")
        self.assertEqual(source_node["direction"], "")

        wallet_rows = {
            row["label"]: row
            for row in report["data_sources"]
            if row["kind"] == "wallet"
        }
        self.assertEqual(wallet_rows["Gran Parent"]["provenance"], "manual_import")
        source_rows = [row for row in report["data_sources"] if row["kind"] == "fiat_purchase"]
        self.assertEqual(source_rows[0]["provenance"], "attested_source")

        preview = report["disclosure_preview"]
        self.assertEqual(preview["wallets_named"], ["Gran Parent", "Gran Target"])
        self.assertIn("common ownership", preview["ownership_note"])

    P_TXID = "aa" * 32
    T_TXID = "bb" * 32

    def _seed_utxo_chain(self):
        """On-chain shaped fixture: P funds T inside wallet Chain A, and T
        pays wallet Chain B. raw_json carries T's vin outpoints (as esplora/
        electrum sync stores them) and wallet_utxos carries the owned
        outputs, so assembly can prove both hops without any heuristics."""
        self._init_default_workspace()
        self._write_csv(
            "chain-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{self.P_TXID},inbound,BTC,0.30000000,0,50000,Funding deposit\n"
            f"2026-05-02T09:00:00Z,{self.T_TXID},outbound,BTC,0.20000000,0.00001000,50000,Spend to Chain B\n",
        )
        self._write_csv(
            "chain-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{self.T_TXID},inbound,BTC,0.20000000,0,50000,Received from Chain A\n",
        )
        self._create_wallet_and_import("Chain A", "chain-a.csv")
        self._create_wallet_and_import("Chain B", "chain-b.csv")
        conn = self._db()
        try:
            ids = {
                row["label"]: (row["id"], row["workspace_id"], row["profile_id"])
                for row in conn.execute(
                    "SELECT w.id, w.label, w.workspace_id, w.profile_id FROM wallets w"
                ).fetchall()
            }
            wallet_a, workspace_id, profile_id = ids["Chain A"]
            wallet_b = ids["Chain B"][0]
            # T's inputs, as chain sync stores them on every leg's raw_json.
            vin_json = json.dumps(
                {
                    "txid": self.T_TXID,
                    "vin": [{"txid": self.P_TXID, "vout": 0}],
                }
            )
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (vin_json, self.T_TXID),
            )
            for utxo_id, wallet_id, txid, vout, amount_msat in (
                ("utxo-p0", wallet_a, self.P_TXID, 0, 30_000_000_000),
                ("utxo-t0", wallet_b, self.T_TXID, 0, 20_000_000_000),
            ):
                conn.execute(
                    """
                    INSERT INTO wallet_utxos(
                        id, workspace_id, profile_id, wallet_id, chain, network,
                        asset, amount, txid, vout, outpoint, confirmation_status,
                        first_seen_at, last_seen_at
                    ) VALUES(?, ?, ?, ?, 'bitcoin', 'main', 'BTC', ?, ?, ?, ?, 'confirmed',
                             '2026-05-02T10:00:00Z', '2026-05-02T10:00:00Z')
                    """,
                    (
                        utxo_id,
                        workspace_id,
                        profile_id,
                        wallet_id,
                        amount_msat,
                        txid,
                        vout,
                        f"{txid}:{vout}",
                    ),
                )
            conn.commit()
        finally:
            conn.close()

    def test_assemble_builds_utxo_proven_chain_transitively(self):
        self._seed_utxo_chain()
        target_id = self._tx_id("Chain B", self.T_TXID)
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertEqual(result["auto_reviewed"], 2)
        # The same-txid leg hop may be claimed by same_onchain_scope (equally
        # exact, runs first); the parent hop is only provable from the UTXO
        # structure.
        self.assertEqual(sum(result["methods"].values()), 2)
        self.assertGreaterEqual(result["methods"].get("utxo_spend", 0), 1)
        self.assertGreaterEqual(result["passes"], 2)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertEqual({link["state"] for link in links}, {"reviewed"})
        self.assertEqual({link["confidence"] for link in links}, {"exact"})
        # Documenting the root source makes the whole chain exportable: the
        # parent hop demands the gross 0.3 BTC input that fed the spend.
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
            "Chain root purchase",
            "--asset",
            "BTC",
            "--amount",
            "0.30000000",
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
            self.P_TXID,
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.30000000",
            "--allocation-policy",
            "explicit",
        )
        report = self._source_funds_report_for_target(target=target_id, amount="0.20000000")
        self.assertTrue(report["explain_gates"]["exportable"], report["explain_gates"]["blockers"])
        self.assertEqual(len(report["flow_levels"]), 4)

    def test_assemble_skips_ambiguous_owned_outpoints(self):
        self._seed_utxo_chain()
        with self._db() as conn:
            self._insert_utxos(
                conn,
                [("Chain B", self.P_TXID, 0, 30_000_000_000)],
            )
            conn.commit()
        target_id = self._tx_id("Chain B", self.T_TXID)
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertEqual(result["methods"].get("utxo_spend", 0), 0)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertFalse(any(link["method"] == "utxo_spend" for link in links))

    def _insert_utxos(self, conn, rows):
        ids = {
            row["label"]: (row["id"], row["workspace_id"], row["profile_id"])
            for row in conn.execute(
                "SELECT w.id, w.label, w.workspace_id, w.profile_id FROM wallets w"
            ).fetchall()
        }
        for index, (wallet_label, txid, vout, amount_msat) in enumerate(rows):
            wallet_id, workspace_id, profile_id = ids[wallet_label]
            conn.execute(
                """
                INSERT INTO wallet_utxos(
                    id, workspace_id, profile_id, wallet_id, chain, network,
                    asset, amount, txid, vout, outpoint, confirmation_status,
                    first_seen_at, last_seen_at
                ) VALUES(?, ?, ?, ?, 'bitcoin', 'main', 'BTC', ?, ?, ?, ?, 'confirmed',
                         '2026-05-02T10:00:00Z', '2026-05-02T10:00:00Z')
                """,
                (
                    f"utxo-x{index}",
                    workspace_id,
                    profile_id,
                    wallet_id,
                    amount_msat,
                    txid,
                    vout,
                    f"{txid}:{vout}",
                ),
            )

    def test_assemble_multi_parent_consolidation_covers_exactly(self):
        """Two parents feeding one spend must be sized as a group: per-edge
        capping would over-cover the spend leg by the fee. The exact-sum
        pro-rata proposal still needs review because Bitcoin does not identify
        which parent paid the fee."""
        p1, p2, tt = "11" * 32, "22" * 32, "33" * 32
        self._init_default_workspace()
        self._write_csv(
            "multi-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{p1},inbound,BTC,0.30000000,0,50000,Parent one\n"
            f"2026-05-01T10:00:00Z,{p2},inbound,BTC,0.50000000,0,50000,Parent two\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,0.79900000,0.00100000,50000,Consolidated spend\n",
        )
        self._write_csv(
            "multi-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.79900000,0,50000,Received\n",
        )
        self._create_wallet_and_import("Multi A", "multi-a.csv")
        self._create_wallet_and_import("Multi B", "multi-b.csv")
        conn = self._db()
        try:
            vin_json = json.dumps(
                {
                    "txid": tt,
                    "vin": [{"txid": p1, "vout": 0}, {"txid": p2, "vout": 0}],
                }
            )
            conn.execute("UPDATE transactions SET raw_json = ? WHERE external_id = ?", (vin_json, tt))
            self._insert_utxos(
                conn,
                [
                    ("Multi A", p1, 0, 30_000_000_000),
                    ("Multi A", p2, 0, 50_000_000_000),
                    ("Multi B", tt, 0, 79_900_000_000),
                ],
            )
            conn.commit()
        finally:
            conn.close()
        target_id = self._tx_id("Multi B", tt)
        result = self.cli(
            "source-funds", "assemble", "--workspace", "Sof", "--profile", "Default",
            "--target-transaction", target_id,
        )["data"]
        self.assertEqual(result["auto_reviewed"], 1)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        parent_allocs = sorted(
            link["allocation_amount"]
            for link in links
            if link["method"] == "utxo_spend" and link["to_transaction_id"] == self._tx_id("Multi A", tt)
        )
        # 0.799 split pro-rata over 0.3 + 0.5 contributed: exact-sum group sizing.
        self.assertEqual(parent_allocs, [0.2996250, 0.4993750])
        self.assertAlmostEqual(sum(parent_allocs), 0.799, places=8)
        parent_links = [
            link
            for link in links
            if link["method"] == "utxo_spend"
            and link["to_transaction_id"] == self._tx_id("Multi A", tt)
        ]
        self.assertTrue(all(link["confidence"] == "strong" for link in parent_links))
        self.assertTrue(all(link["requires_review"] for link in parent_links))
        for link in parent_links:
            self.cli(
                "source-funds", "links", "review",
                "--workspace", "Sof", "--profile", "Default",
                "--link", link["id"], "--state", "reviewed",
                "--allocation-policy", "explicit",
            )
        source = self.cli(
            "source-funds", "sources", "create", "--workspace", "Sof", "--profile", "Default",
            "--type", "fiat_purchase", "--label", "Multi root", "--asset", "BTC",
            "--amount", "0.80000000",
        )["data"]
        for parent_txid, amount in ((p1, "0.30000000"), (p2, "0.50000000")):
            self.cli(
                "source-funds", "links", "create", "--workspace", "Sof", "--profile", "Default",
                "--from-source", source["id"], "--to-transaction", parent_txid,
                "--type", "manual_source", "--allocation-amount", amount,
                "--allocation-policy", "explicit",
            )
        report = self._source_funds_report_for_target(target=target_id, amount="0.79900000")
        self.assertTrue(report["explain_gates"]["exportable"], report["explain_gates"]["blockers"])

    def test_assemble_apportions_shared_receive_across_spenders(self):
        """A transaction funded by multiple owned wallets must not assign the
        full received output to every spender wallet."""
        p1, p2, tt = "71" * 32, "72" * 32, "73" * 32
        self._init_default_workspace()
        self._write_csv(
            "shared-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{p1},inbound,BTC,0.30000000,0,50000,A parent\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,0.30000000,0,50000,A contributes\n",
        )
        self._write_csv(
            "shared-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T10:00:00Z,{p2},inbound,BTC,0.50000000,0,50000,B parent\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,0.50000000,0,50000,B contributes\n",
        )
        self._write_csv(
            "shared-c.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.79900000,0,50000,C receives\n",
        )
        self._create_wallet_and_import("Shared A", "shared-a.csv")
        self._create_wallet_and_import("Shared B", "shared-b.csv")
        self._create_wallet_and_import("Shared C", "shared-c.csv")
        conn = self._db()
        try:
            vin_json = json.dumps(
                {
                    "txid": tt,
                    "vin": [{"txid": p1, "vout": 0}, {"txid": p2, "vout": 0}],
                }
            )
            conn.execute("UPDATE transactions SET raw_json = ? WHERE external_id = ?", (vin_json, tt))
            self._insert_utxos(
                conn,
                [
                    ("Shared A", p1, 0, 30_000_000_000),
                    ("Shared B", p2, 0, 50_000_000_000),
                    ("Shared C", tt, 0, 79_900_000_000),
                ],
            )
            conn.commit()
        finally:
            conn.close()

        target_id = self._tx_id("Shared C", tt)
        result = self.cli(
            "source-funds", "assemble", "--workspace", "Sof", "--profile", "Default",
            "--target-transaction", target_id,
        )["data"]
        self.assertEqual(result["methods"].get("utxo_spend", 0), 2)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        spend_rows = {self._tx_id("Shared A", tt), self._tx_id("Shared B", tt)}
        target_allocs = sorted(
            link["allocation_amount"]
            for link in links
            if link["method"] == "utxo_spend"
            and link["from_transaction_id"] in spend_rows
            and link["to_transaction_id"] == target_id
        )
        self.assertEqual(target_allocs, [0.2996250, 0.4993750])
        self.assertAlmostEqual(sum(target_allocs), 0.799, places=8)
        target_links = [
            link
            for link in links
            if link["method"] == "utxo_spend"
            and link["from_transaction_id"] in spend_rows
            and link["to_transaction_id"] == target_id
        ]
        self.assertTrue(all(link["confidence"] == "strong" for link in target_links))
        self.assertTrue(all(link["requires_review"] for link in target_links))
        for link in target_links:
            self.cli(
                "source-funds", "links", "review",
                "--workspace", "Sof", "--profile", "Default",
                "--link", link["id"], "--state", "reviewed",
                "--allocation-policy", "explicit",
            )

        source = self.cli(
            "source-funds", "sources", "create", "--workspace", "Sof", "--profile", "Default",
            "--type", "fiat_purchase", "--label", "Shared roots", "--asset", "BTC",
            "--amount", "0.80000000",
        )["data"]
        for parent_txid, amount in ((p1, "0.30000000"), (p2, "0.50000000")):
            self.cli(
                "source-funds", "links", "create", "--workspace", "Sof", "--profile", "Default",
                "--from-source", source["id"], "--to-transaction", parent_txid,
                "--type", "manual_source", "--allocation-amount", amount,
                "--allocation-policy", "explicit",
            )
        report = self._source_funds_report_for_target(target=target_id, amount="0.79900000")
        blockers = {item["code"] for item in report["explain_gates"]["blockers"]}
        self.assertNotIn("ambiguous_allocation", blockers)
        self.assertTrue(report["explain_gates"]["exportable"], report["explain_gates"]["blockers"])

    def test_multi_source_multi_destination_utxo_allocations_require_review(self):
        """Bitcoin inputs do not identify a source wallet for each output.

        A deterministic 2x2 pro-rata allocation is useful as a proposal, but
        it must remain suggested until a person confirms it.
        """
        p1, p2, tt = "81" * 32, "82" * 32, "83" * 32
        self._init_default_workspace()
        self._write_csv(
            "matrix-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{p1},inbound,BTC,0.60000000,0,50000,A parent\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,0.60000000,0,50000,A contributes\n",
        )
        self._write_csv(
            "matrix-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T10:00:00Z,{p2},inbound,BTC,0.40000000,0,50000,B parent\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,0.40000000,0,50000,B contributes\n",
        )
        self._write_csv(
            "matrix-c.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.70000000,0,50000,C receives\n",
        )
        self._write_csv(
            "matrix-d.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.30000000,0,50000,D receives\n",
        )
        for label, filename in (
            ("Matrix A", "matrix-a.csv"),
            ("Matrix B", "matrix-b.csv"),
            ("Matrix C", "matrix-c.csv"),
            ("Matrix D", "matrix-d.csv"),
        ):
            self._create_wallet_and_import(label, filename)
        with self._db() as conn:
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {
                            "txid": tt,
                            "vin": [
                                {"txid": p1, "vout": 0},
                                {"txid": p2, "vout": 0},
                            ],
                        }
                    ),
                    tt,
                ),
            )
            self._insert_utxos(
                conn,
                [
                    ("Matrix A", p1, 0, 60_000_000_000),
                    ("Matrix B", p2, 0, 40_000_000_000),
                    ("Matrix C", tt, 0, 70_000_000_000),
                ],
            )
            conn.commit()

        # With only C's output known, the missing 0.3 BTC is a fee/external/
        # unknown-output residual. Bitcoin cannot identify which source funded
        # that residual, so even this apparent N:1 topology needs review.
        suggested = self.cli(
            "source-funds", "suggest", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertGreaterEqual(suggested["inserted"], 4)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        spend_ids = {self._tx_id("Matrix A", tt), self._tx_id("Matrix B", tt)}
        destination_ids = {self._tx_id("Matrix C", tt), self._tx_id("Matrix D", tt)}
        initial_c_links = [
            link
            for link in links
            if link["method"] == "utxo_spend"
            and link["from_transaction_id"] in spend_ids
            and link["to_transaction_id"] == self._tx_id("Matrix C", tt)
        ]
        self.assertEqual(len(initial_c_links), 2)
        self.assertTrue(all(link["confidence"] == "strong" for link in initial_c_links))
        self.assertTrue(all(link["requires_review"] for link in initial_c_links))

        # Discovering D later changes the live evidence into a genuine 2x2
        # matrix. Bulk review must re-derive topology and refuse the now-stale
        # exact C links even before suggestions are refreshed.
        with self._db() as conn:
            wallet_d = conn.execute(
                "SELECT id, workspace_id, profile_id FROM wallets WHERE label = 'Matrix D'"
            ).fetchone()
            conn.execute(
                """
                INSERT INTO wallet_utxos(
                    id, workspace_id, profile_id, wallet_id, chain, network,
                    asset, amount, txid, vout, outpoint, confirmation_status,
                    first_seen_at, last_seen_at
                ) VALUES('utxo-matrix-d', ?, ?, ?, 'bitcoin', 'main', 'BTC', ?, ?, 1, ?,
                         'confirmed', '2026-05-02T10:00:00Z', '2026-05-02T10:00:00Z')
                """,
                (
                    wallet_d["workspace_id"],
                    wallet_d["profile_id"],
                    wallet_d["id"],
                    30_000_000_000,
                    tt,
                    f"{tt}:1",
                ),
            )
            conn.commit()

        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Matrix C", tt),
        )["data"]
        self.assertEqual(reviewed["reviewed"], 2)
        self.assertEqual(reviewed["skipped"], 2)

        refreshed = self.cli(
            "source-funds", "suggest", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertGreaterEqual(refreshed["inserted"], 4)
        links_after_refresh = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        matrix_links = [
            link
            for link in links_after_refresh
            if link["method"] == "utxo_spend"
            and link["from_transaction_id"] in spend_ids
            and link["to_transaction_id"] in destination_ids
        ]
        self.assertEqual(len(matrix_links), 4)
        self.assertEqual(
            sorted(link["allocation_amount_msat"] for link in matrix_links),
            [12_000_000_000, 18_000_000_000, 28_000_000_000, 42_000_000_000],
        )
        self.assertTrue(all(link["confidence"] == "strong" for link in matrix_links))
        self.assertTrue(all(link["requires_review"] for link in matrix_links))

        strong_review = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Matrix D", tt),
        )["data"]
        self.assertEqual(strong_review["reviewed"], 0)
        self.assertEqual(strong_review["skipped"], 2)

    def test_assemble_apportions_split_receive_parent_requirements(self):
        """A single spender funding multiple owned receive legs must split
        its proof requirement across those children."""
        p1, tt = "74" * 32, "75" * 32
        self._init_default_workspace()
        self._write_csv(
            "split-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{p1},inbound,BTC,1.00000000,0,50000,A parent\n"
            f"2026-05-02T09:00:00Z,{tt},outbound,BTC,1.00000000,0,50000,A splits\n",
        )
        self._write_csv(
            "split-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.60000000,0,50000,B receives\n",
        )
        self._write_csv(
            "split-c.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{tt},inbound,BTC,0.40000000,0,50000,C receives\n",
        )
        self._create_wallet_and_import("Split A", "split-a.csv")
        self._create_wallet_and_import("Split B", "split-b.csv")
        self._create_wallet_and_import("Split C", "split-c.csv")
        conn = self._db()
        try:
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {"txid": tt, "vin": [{"txid": p1, "vout": 0}]}
                    ),
                    tt,
                ),
            )
            self._insert_utxos(
                conn,
                [
                    ("Split A", p1, 0, 100_000_000_000),
                    ("Split B", tt, 0, 60_000_000_000),
                    ("Split C", tt, 1, 40_000_000_000),
                ],
            )
            conn.commit()
        finally:
            conn.close()

        suggested = self.cli(
            "source-funds", "suggest", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertGreaterEqual(suggested["inserted"], 3)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        spend_id = self._tx_id("Split A", tt)
        by_target = {
            link["to_transaction_id"]: link
            for link in links
            if link["method"] == "utxo_spend"
            and link["from_transaction_id"] == spend_id
        }
        split_b = by_target[self._tx_id("Split B", tt)]
        split_c = by_target[self._tx_id("Split C", tt)]
        self.assertEqual(split_b["allocation_amount"], 0.6)
        self.assertEqual(split_b["from_allocation_amount"], 0.6)
        self.assertEqual(split_c["allocation_amount"], 0.4)
        self.assertEqual(split_c["from_allocation_amount"], 0.4)

    def test_assemble_resolves_through_net_zero_consolidation(self):
        """An in-wallet consolidation nets to a 0-amount row that cannot carry
        allocation demand; assembly must link its parents directly to the
        following spend instead of producing unfixable 0-amount edges."""
        pp, cc, ss = "44" * 32, "55" * 32, "66" * 32
        self._init_default_workspace()
        self._write_csv(
            "cons-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{pp},inbound,BTC,0.30000000,0,50000,Funding deposit\n"
            f"2026-05-01T12:00:00Z,{cc},outbound,BTC,0.00000001,0.00010000,50000,In-wallet consolidation\n"
            f"2026-05-02T09:00:00Z,{ss},outbound,BTC,0.25000000,0.00001000,50000,Spend to B\n",
        )
        self._write_csv(
            "cons-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{ss},inbound,BTC,0.25000000,0,50000,Received\n",
        )
        self._create_wallet_and_import("Cons A", "cons-a.csv")
        self._create_wallet_and_import("Cons B", "cons-b.csv")
        conn = self._db()
        try:
            # The consolidation nets to zero (all outputs back to the wallet).
            conn.execute("UPDATE transactions SET amount = 0 WHERE external_id = ?", (cc,))
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {"txid": cc, "vin": [{"txid": pp, "vout": 0}]}
                    ),
                    cc,
                ),
            )
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {"txid": ss, "vin": [{"txid": cc, "vout": 0}]}
                    ),
                    ss,
                ),
            )
            self._insert_utxos(
                conn,
                [
                    ("Cons A", pp, 0, 30_000_000_000),
                    ("Cons A", cc, 0, 29_990_000_000),
                    ("Cons B", ss, 0, 25_000_000_000),
                ],
            )
            conn.commit()
        finally:
            conn.close()
        target_id = self._tx_id("Cons B", ss)
        result = self.cli(
            "source-funds", "assemble", "--workspace", "Sof", "--profile", "Default",
            "--target-transaction", target_id,
        )["data"]
        self.assertGreaterEqual(result["auto_reviewed"], 2)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        cons_leg = self._tx_id("Cons A", cc)
        self.assertFalse(
            [link for link in links if cons_leg in (link["from_transaction_id"], link["to_transaction_id"])],
            "net-zero consolidation leg must be bypassed, not linked",
        )
        parent_links = [
            link
            for link in links
            if link["from_transaction_id"] == self._tx_id("Cons A", pp)
            and link["to_transaction_id"] == self._tx_id("Cons A", ss)
        ]
        self.assertEqual(len(parent_links), 1)
        self.assertEqual(parent_links[0]["state"], "reviewed")
        self.assertIn("via in-wallet consolidation", parent_links[0]["explanation"])
        source = self.cli(
            "source-funds", "sources", "create", "--workspace", "Sof", "--profile", "Default",
            "--type", "fiat_purchase", "--label", "Cons root", "--asset", "BTC",
            "--amount", "0.30000000",
        )["data"]
        self.cli(
            "source-funds", "links", "create", "--workspace", "Sof", "--profile", "Default",
            "--from-source", source["id"], "--to-transaction", pp,
            "--type", "manual_source", "--allocation-amount", "0.30000000",
            "--allocation-policy", "explicit",
        )
        report = self._source_funds_report_for_target(target=target_id, amount="0.25000000")
        self.assertTrue(report["explain_gates"]["exportable"], report["explain_gates"]["blockers"])

    def test_assemble_resolves_through_owned_change_output(self):
        """A spend from owned change must trace through the change tx inputs,
        not cap source proof at the small external payment row."""
        pp, cc, ss = "76" * 32, "77" * 32, "78" * 32
        self._init_default_workspace()
        self._write_csv(
            "change-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-01T09:00:00Z,{pp},inbound,BTC,1.00000000,0,50000,Funding deposit\n"
            f"2026-05-01T12:00:00Z,{cc},outbound,BTC,0.10000000,0.00010000,50000,Spend with change\n"
            f"2026-05-02T09:00:00Z,{ss},outbound,BTC,0.80000000,0.00001000,50000,Spend from change\n",
        )
        self._write_csv(
            "change-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-05-02T09:00:00Z,{ss},inbound,BTC,0.80000000,0,50000,Received\n",
        )
        self._create_wallet_and_import("Change A", "change-a.csv")
        self._create_wallet_and_import("Change B", "change-b.csv")
        conn = self._db()
        try:
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {"txid": cc, "vin": [{"txid": pp, "vout": 0}]}
                    ),
                    cc,
                ),
            )
            conn.execute(
                "UPDATE transactions SET raw_json = ? WHERE external_id = ?",
                (
                    json.dumps(
                        {"txid": ss, "vin": [{"txid": cc, "vout": 1}]}
                    ),
                    ss,
                ),
            )
            self._insert_utxos(
                conn,
                [
                    ("Change A", pp, 0, 100_000_000_000),
                    ("Change A", cc, 1, 89_990_000_000),
                    ("Change B", ss, 0, 80_000_000_000),
                ],
            )
            conn.commit()
        finally:
            conn.close()

        target_id = self._tx_id("Change B", ss)
        result = self.cli(
            "source-funds", "assemble", "--workspace", "Sof", "--profile", "Default",
            "--target-transaction", target_id,
        )["data"]
        self.assertGreaterEqual(result["auto_reviewed"], 2)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        change_leg = self._tx_id("Change A", cc)
        spend_leg = self._tx_id("Change A", ss)
        self.assertFalse(
            [link for link in links if link["from_transaction_id"] == change_leg and link["to_transaction_id"] == spend_leg],
            "owned change must be a passthrough hop, not a small parent proof",
        )
        root_links = [
            link
            for link in links
            if link["from_transaction_id"] == self._tx_id("Change A", pp)
            and link["to_transaction_id"] == spend_leg
        ]
        self.assertEqual(len(root_links), 1)
        self.assertEqual(root_links[0]["state"], "reviewed")
        self.assertGreater(root_links[0]["from_allocation_amount"], 0.1)

    def test_assemble_links_lightning_legs_by_payment_hash(self):
        self._init_default_workspace()
        payment_hash = "cd" * 32
        self._write_csv(
            "ln-a.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-05-03T09:00:00Z,ln-send-1,outbound,BTC,0.01000000,0.00000100,50000,LN payment out\n",
        )
        self._write_csv(
            "ln-b.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-05-03T09:00:05Z,ln-recv-1,inbound,BTC,0.01000000,0,50000,LN invoice settled\n",
        )
        self._create_wallet_and_import("LN A", "ln-a.csv")
        self._create_wallet_and_import("LN B", "ln-b.csv")
        conn = self._db()
        try:
            conn.execute(
                "UPDATE transactions SET payment_hash = ?, payment_hash_source = 'core_lightning', "
                "kind = CASE direction WHEN 'outbound' THEN 'cln_pay' ELSE 'cln_invoice' END, "
                "raw_json = ? "
                "WHERE external_id IN ('ln-send-1', 'ln-recv-1')",
                (
                    payment_hash,
                    json.dumps(
                        {
                            "_kassiber_provenance": {"import_source": "core-lightning"},
                            "chain": "lightning",
                            "network": "main",
                        }
                    ),
                ),
            )
            conn.commit()
        finally:
            conn.close()
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "ln-recv-1",
        )["data"]
        self.assertEqual(result["auto_reviewed"], 1)
        self.assertEqual(result["methods"], {"payment_hash": 1})
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertEqual(len(links), 1)
        self.assertEqual(links[0]["method"], "payment_hash")
        self.assertEqual(links[0]["state"], "reviewed")
        self.assertEqual(links[0]["confidence"], "exact")

    def test_assemble_rejects_payment_hash_when_receive_exceeds_send(self):
        self._init_default_workspace()
        payment_hash = "ef" * 32
        self._write_csv(
            "ln-small-send.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-05-03T09:00:00Z,ln-send-small,outbound,BTC,0.01000000,0.00000100,50000,LN payment out\n",
        )
        self._write_csv(
            "ln-large-recv.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-05-03T09:00:05Z,ln-recv-large,inbound,BTC,0.01100000,0,50000,LN invoice settled\n",
        )
        self._create_wallet_and_import("LN Small Send", "ln-small-send.csv")
        self._create_wallet_and_import("LN Large Receive", "ln-large-recv.csv")
        conn = self._db()
        try:
            conn.execute(
                "UPDATE transactions SET payment_hash = ?, payment_hash_source = 'core_lightning', "
                "kind = CASE direction WHEN 'outbound' THEN 'cln_pay' ELSE 'cln_invoice' END, "
                "raw_json = ? "
                "WHERE external_id IN ('ln-send-small', 'ln-recv-large')",
                (
                    payment_hash,
                    json.dumps(
                        {
                            "_kassiber_provenance": {"import_source": "core-lightning"},
                            "chain": "lightning",
                            "network": "main",
                        }
                    ),
                ),
            )
            conn.commit()
        finally:
            conn.close()
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "ln-recv-large",
        )["data"]
        self.assertEqual(result["auto_reviewed"], 0)
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        self.assertFalse([link for link in links if link["method"] == "payment_hash"])

    def test_assemble_does_not_cross_privacy_boundaries(self):
        self._seed_utxo_chain()
        conn = self._db()
        try:
            conn.execute(
                "UPDATE transactions SET privacy_boundary = 'coinjoin' WHERE external_id = ?",
                (self.T_TXID,),
            )
            conn.commit()
        finally:
            conn.close()
        target_id = self._tx_id("Chain B", self.T_TXID)
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertEqual(result["inserted"], 0)
        self.assertEqual(result["auto_reviewed"], 0)

    def test_assemble_does_not_auto_review_privacy_boundary_transaction_pair(self):
        self._seed_utxo_chain()
        out_leg = self._tx_id("Chain A", self.T_TXID)
        in_leg = self._tx_id("Chain B", self.T_TXID)
        with self._db() as conn:
            conn.execute(
                "UPDATE transactions SET privacy_boundary = 'coinjoin' WHERE external_id = ?",
                (self.T_TXID,),
            )
            conn.commit()
        self.cli(
            "transfers",
            "pair",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--tx-out",
            out_leg,
            "--tx-in",
            in_leg,
            "--kind",
            "manual",
            "--policy",
            "carrying-value",
        )
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            in_leg,
        )["data"]
        self.assertEqual(result["inserted"], 0)
        self.assertEqual(result["auto_reviewed"], 0)

    def test_assemble_skips_pairs_already_linked_by_any_method(self):
        self._seed_utxo_chain()
        out_leg = self._tx_id("Chain A", self.T_TXID)
        in_leg = self._tx_id("Chain B", self.T_TXID)
        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            out_leg,
            "--to-transaction",
            in_leg,
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.20000000",
            "--from-amount",
            "0.20000000",
            "--allocation-policy",
            "explicit",
        )
        result = self.cli(
            "source-funds",
            "assemble",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            in_leg,
        )["data"]
        # Only the parent hop is added; the manually linked leg pair is not
        # duplicated into a double allocation.
        self.assertEqual(result["methods"], {"utxo_spend": 1})
        links = self.cli(
            "source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default"
        )["data"]
        pair_links = [
            link
            for link in links
            if link["from_transaction_id"] == out_leg and link["to_transaction_id"] == in_leg
        ]
        self.assertEqual(len(pair_links), 1)
        self.assertEqual(pair_links[0]["method"], "manual")

    def test_bulk_review_skips_utxo_suggestion_when_inventory_changes(self):
        self._seed_utxo_chain()
        target_id = self._tx_id("Chain B", self.T_TXID)
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertGreaterEqual(suggested["inserted"], 1)
        conn = self._db()
        try:
            # Isolate the utxo_spend suggestions, then invalidate the
            # evidence they were derived from.
            conn.execute("DELETE FROM source_funds_links WHERE method != 'utxo_spend'")
            conn.execute("DELETE FROM wallet_utxos")
            conn.execute("UPDATE transactions SET raw_json = '{}'")
            conn.commit()
        finally:
            conn.close()
        result = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            # Scope to the spend leg: the surviving utxo_spend suggestion is
            # the parent hop feeding it.
            self._tx_id("Chain A", self.T_TXID),
        )["data"]
        self.assertEqual(result["reviewed"], 0)
        self.assertGreaterEqual(result["skipped"], 1)

    def test_bulk_review_skips_utxo_suggestion_when_allocation_changes(self):
        self._seed_utxo_chain()
        target_id = self._tx_id("Chain A", self.T_TXID)
        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            target_id,
        )["data"]
        self.assertGreaterEqual(suggested["inserted"], 1)
        conn = self._db()
        try:
            conn.execute("DELETE FROM source_funds_links WHERE method != 'utxo_spend'")
            conn.execute(
                "UPDATE wallet_utxos SET amount = ? WHERE txid = ? AND vout = 0",
                (10_000_000_000, self.P_TXID),
            )
            conn.commit()
        finally:
            conn.close()
        result = self.cli(
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
        self.assertEqual(result["reviewed"], 0)
        self.assertGreaterEqual(result["skipped"], 1)

    def test_wallet_data_provenance_mapping(self):
        from kassiber.core.source_funds import _wallet_data_provenance

        self.assertEqual(_wallet_data_provenance("descriptor", None), "chain_sync")
        self.assertEqual(_wallet_data_provenance("xpub", "{}"), "chain_sync")
        # File-sourced descriptor/address wallets are platform exports, not
        # chain-verified rows.
        self.assertEqual(
            _wallet_data_provenance(
                "descriptor", '{"source_file": "wallet.csv", "source_format": "sparrow_csv"}'
            ),
            "platform_export",
        )
        self.assertEqual(
            _wallet_data_provenance("address", '{"source_file": "rows.csv"}'),
            "platform_export",
        )
        self.assertEqual(_wallet_data_provenance("address", '{"addresses": ["bc1..."]}'), "chain_sync")
        self.assertEqual(_wallet_data_provenance("river", None), "platform_export")
        self.assertEqual(_wallet_data_provenance("strike", None), "platform_export")
        self.assertEqual(_wallet_data_provenance("custom", None), "manual_import")
        self.assertEqual(_wallet_data_provenance(None, None), "manual_import")

    def test_level_fiat_subtotals_scale_to_the_allocated_slice(self):
        """A 1.0 BTC parent of which only 0.1 BTC feeds the target must
        contribute its pro-rata fiat slice to the level subtotal, not the
        full transaction value."""
        self._init_default_workspace()
        self._write_csv(
            "alloc-parent.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:00:00Z,alloc-parent,outbound,BTC,1.00000000,0,50000,Big parent spend\n",
        )
        self._write_csv(
            "alloc-target.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-03-01T09:30:00Z,alloc-target,inbound,BTC,0.10000000,0,50000,Target deposit\n",
        )
        self._create_wallet_and_import("Alloc Parent", "alloc-parent.csv")
        self._create_wallet_and_import("Alloc Target", "alloc-target.csv")
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
            "Allocation source",
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
            "alloc-parent",
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
            "alloc-parent",
            "--to-transaction",
            "alloc-target",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        report = self._source_funds_report_for_target(
            target="alloc-target",
            amount="0.10000000",
        )
        levels = {level["level"]: level for level in report["flow_levels"]}
        # Target: 0.1 BTC fully traced at 50,000 EUR/BTC.
        self.assertEqual(levels[1]["fiat_value_total"], 5000.0)
        # Parent: full tx is 1.0 BTC (50,000 EUR) but only 0.1 BTC feeds the
        # target, so the subtotal is the 5,000 EUR slice.
        self.assertEqual(levels[2]["fiat_value_total"], 5000.0)
        parent_node = levels[2]["nodes"][0]
        self.assertEqual(parent_node["fiat_value"], 50000.0)
        self.assertEqual(parent_node["fiat_value_allocated"], 5000.0)

    def test_missing_history_gap_carries_unexplained_amount(self):
        self._seed_single_target(amount="0.20000000")
        blockers, report = self._report_blockers()
        self.assertIn("missing_history", blockers)
        gap = next(item for item in report["gaps"] if item["code"] == "missing_history")
        self.assertEqual(gap["amount_msat"], 20_000_000_000)
        self.assertEqual(gap["amount"], 0.2)
        self.assertEqual(gap["asset"], "BTC")

    def test_export_via_case_matches_preview_snapshot_hash(self):
        self._seed_exportable_disclosure_path()
        preview = self._source_funds_report(save_case=True)
        flow = preview["simplified_flow"]
        self.assertFalse(flow["deferred_privacy_hops"])
        self.assertGreaterEqual(len(flow["levels"]), 4)
        self.assertEqual(flow["levels"][-1]["role"], "target")
        flow_labels = {
            node["label"]
            for level in flow["levels"]
            for node in level["nodes"]
        }
        flow_transaction_ids = {
            node["transaction_id"]
            for level in flow["levels"]
            for node in level["nodes"]
            if node["node_type"] == "transaction"
        }
        self.assertIn("Reviewed disclosure source", flow_labels)
        self.assertIn(preview["target"]["transaction_id"], flow_transaction_ids)
        self.assertGreaterEqual(len(flow["edges"]), 3)
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

    def test_export_bundle_ships_pdf_evidence_and_manifest(self):
        """The bundle export zips the report PDF plus the original evidence
        files attached to disclosed sources, with a SHA-256 manifest. In
        ``standard`` reveal mode files are included; in ``labels_only`` the
        files are withheld and only recorded as withheld in the manifest."""
        import hashlib
        import zipfile

        seed = self._seed_exportable_disclosure_path()

        # standard mode: original evidence files are bundled.
        preview = self._source_funds_report(reveal_mode="standard", save_case=True)
        self.assertTrue(
            preview["explain_gates"]["exportable"],
            preview["explain_gates"]["blockers"],
        )
        conn = self._db()
        try:
            row = conn.execute(
                "SELECT stored_relpath, sha256 FROM attachments WHERE id = ?",
                (seed["file_attachment"],),
            ).fetchone()
            stale_sha = row["sha256"]
            managed_path = self.root / "attachments" / row["stored_relpath"]
            managed_path.write_text("Tampered evidence bytes after attach\n", encoding="utf-8")
        finally:
            conn.close()
        bundle_path = self.root / "bundle.zip"
        exported = self.cli(
            "reports",
            "export-source-funds-bundle",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            preview["case"]["id"],
            "--file",
            str(bundle_path),
        )["data"]
        self.assertEqual(exported["format"], "zip")
        self.assertEqual(exported["scope"], "source_funds")
        self.assertEqual(exported["snapshot_hash"], preview["case"]["snapshot_hash"])
        self.assertGreaterEqual(exported["evidence_files"], 1)
        self.assertTrue(bundle_path.exists())

        with zipfile.ZipFile(bundle_path) as archive:
            names = set(archive.namelist())
            self.assertIn("source-of-funds-report.pdf", names)
            self.assertIn("manifest.json", names)
            manifest = json.loads(archive.read("manifest.json"))
            self.assertEqual(manifest["kind"], "kassiber.source_funds.bundle.manifest")
            self.assertEqual(manifest["reveal_mode"], "standard")
            self.assertEqual(
                manifest["snapshot_hash"], preview["case"]["snapshot_hash"]
            )
            pdf_bytes = archive.read("source-of-funds-report.pdf")
            self.assertTrue(pdf_bytes.startswith(b"%PDF"))
            self.assertEqual(
                hashlib.sha256(pdf_bytes).hexdigest(),
                manifest["report_pdf"]["sha256"],
            )
            file_items = [e for e in manifest["evidence"] if e.get("source") == "file"]
            self.assertGreaterEqual(len(file_items), 1)
            for item in file_items:
                self.assertIn(item["filename"], names)
                self.assertEqual(
                    hashlib.sha256(archive.read(item["filename"])).hexdigest(),
                    item["sha256"],
                )
                self.assertNotEqual(item["sha256"], stale_sha)
            url_items = [e for e in manifest["evidence"] if e.get("source") == "url"]
            self.assertGreaterEqual(len(url_items), 1)
            self.assertTrue(all("source_url" not in item for item in url_items))

        # labels_only mode: evidence files are withheld by reveal mode.
        preview_lo = self._source_funds_report(reveal_mode="labels_only", save_case=True)
        bundle_lo = self.root / "bundle-labels-only.zip"
        exported_lo = self.cli(
            "reports",
            "export-source-funds-bundle",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            preview_lo["case"]["id"],
            "--file",
            str(bundle_lo),
        )["data"]
        self.assertEqual(exported_lo["evidence_files"], 0)
        with zipfile.ZipFile(bundle_lo) as archive:
            names = set(archive.namelist())
            self.assertFalse(any(name.startswith("evidence/") for name in names))
            manifest = json.loads(archive.read("manifest.json"))
            self.assertTrue(
                all(
                    item.get("source") == "withheld_by_reveal_mode"
                    for item in manifest["evidence"]
                )
            )

    def test_report_options_precision_masking_and_section_omission(self):
        """Advanced report options (amount precision, recipient masking, and
        section omission) normalize into the snapshot and shape the PDF."""
        import shutil
        import subprocess

        self._seed_exportable_disclosure_path()
        report = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "disclosure-target",
            "--target-amount",
            "0.10000000",
            "--reveal-mode",
            "standard",
            "--amount-precision",
            "sats",
            "--mask-recipient",
            "--omit-section",
            "graph_nodes",
            "--omit-section",
            "flow_links",
            "--save-case",
        )["data"]
        options = report["report_options"]
        self.assertEqual(options["amount_precision"], "sats")
        self.assertTrue(options["mask_recipient"])
        self.assertEqual(set(options["omit_sections"]), {"graph_nodes", "flow_links"})

        pdf_path = self.root / "options.pdf"
        self.cli(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            report["case"]["id"],
            "--file",
            str(pdf_path),
        )
        if shutil.which("pdftotext"):
            extracted = subprocess.run(
                ["pdftotext", "-layout", str(pdf_path), "-"],
                check=True,
                text=True,
                capture_output=True,
            ).stdout
            self.assertIn("sats", extracted)
            self.assertIn("(recipient masked)", extracted)
            self.assertNotIn("Disclosure Graph Nodes", extracted)
            self.assertNotIn("Reviewed Flow Links", extracted)
            self.assertIn("Source of Funds Overview", extracted)

    def test_reveal_overrides_hide_and_show_specific_transactions(self):
        """Per-node reveal overrides win over the global reveal mode:
        'hide' redacts a txid the mode would show; 'show' reveals one the
        mode would drop. Overrides freeze into report_options."""
        self._seed_exportable_disclosure_path()
        target_id = self._tx_id("Target", "disclosure-target")
        parent_id = self._tx_id("Parent", "disclosure-parent")

        # Baseline: standard mode reveals the target txid.
        base = self._source_funds_report(reveal_mode="standard")
        self.assertIn("disclosure-target", base["disclosure_preview"]["txids"])

        # Hide the target txid even though standard mode would show it.
        hidden = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "disclosure-target",
            "--target-amount",
            "0.10000000",
            "--reveal-mode",
            "standard",
            "--reveal-override",
            f"{target_id}=hide",
        )["data"]
        self.assertEqual(
            hidden["report_options"]["reveal_overrides"], {target_id: "hide"}
        )
        self.assertNotIn("disclosure-target", hidden["disclosure_preview"]["txids"])
        self.assertEqual(hidden["target"]["external_id"], "")

        # Reveal a parent txid that minimal mode would otherwise drop.
        shown = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            "disclosure-target",
            "--target-amount",
            "0.10000000",
            "--reveal-mode",
            "minimal",
            "--reveal-override",
            f"{parent_id}=show",
        )["data"]
        self.assertIn("disclosure-parent", shown["disclosure_preview"]["txids"])

    def test_austrian_eur_basic_source_funds_pdf_context(self):
        exchange_withdraw_txid = "4e9f0b7d8c6a5b4c3d2e1f0099887766554433221100ffeeddccbbaa99887766"
        cold_consolidation_txid = "6f1e2d3c4b5a69788776655443322110ffeeddccbbaa00998877665544332211"
        self.cli("init")
        self.cli("workspaces", "create", "AtSof")
        self.cli(
            "profiles",
            "create",
            "--workspace",
            "AtSof",
            "--fiat-currency",
            "EUR",
            "--tax-country",
            "at",
            "Austria",
        )
        self._write_csv(
            "at-exchange.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-03-01T09:00:00Z,{exchange_withdraw_txid},outbound,BTC,0.30010000,0.00010000,55000,Fictitious exchange withdrawal\n",
        )
        self._write_csv(
            "at-cold.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-03-01T09:30:00Z,{exchange_withdraw_txid},inbound,BTC,0.30000000,0,55000,Cold storage receive\n"
            f"2025-11-06T08:45:00Z,{cold_consolidation_txid},outbound,BTC,0.15005000,0.00005000,70000,Reviewed consolidation spend\n",
        )
        self._write_csv(
            "at-target.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2025-11-06T09:10:00Z,{cold_consolidation_txid},inbound,BTC,0.15000000,0,70000,Fictitious target broker deposit\n",
        )
        for label, csv_name in [
            ("Example Exchange AT", "at-exchange.csv"),
            ("Cold Storage AT", "at-cold.csv"),
            ("Target Broker AT", "at-target.csv"),
        ]:
            self.cli(
                "wallets",
                "create",
                "--workspace",
                "AtSof",
                "--profile",
                "Austria",
                "--label",
                label,
                "--kind",
                "custom",
            )
            self.cli(
                "wallets",
                "import-csv",
                "--workspace",
                "AtSof",
                "--profile",
                "Austria",
                "--wallet",
                label,
                "--file",
                str(self.root / csv_name),
            )

        def tx_id(wallet: str, external_id: str) -> str:
            payload = self.cli(
                "transactions",
                "list",
                "--workspace",
                "AtSof",
                "--profile",
                "Austria",
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

        exchange_out = tx_id("Example Exchange AT", exchange_withdraw_txid)
        cold_in = tx_id("Cold Storage AT", exchange_withdraw_txid)
        cold_out = tx_id("Cold Storage AT", cold_consolidation_txid)
        target_in = tx_id("Target Broker AT", cold_consolidation_txid)
        attachment = self.cli(
            "attachments",
            "add",
            "--workspace",
            "AtSof",
            "--profile",
            "Austria",
            "--transaction",
            exchange_out,
            "--file",
            str(self.evidence_file),
            "--label",
            "Fictitious Austrian EUR purchase statement",
        )["data"]
        source = self.cli(
            "source-funds",
            "sources",
            "create",
            "--workspace",
            "AtSof",
            "--profile",
            "Austria",
            "--type",
            "fiat_purchase",
            "--label",
            "Fictitious Austrian EUR purchase",
            "--asset",
            "BTC",
            "--amount",
            "0.30000000",
            "--fiat-currency",
            "EUR",
            "--fiat-value",
            "16500",
            "--acquired-at",
            "2025-02-20T10:00:00Z",
            "--attachment",
            attachment["id"],
        )["data"]
        for from_arg, from_ref, to_ref, allocation, from_amount in [
            ("--from-source", source["id"], exchange_out, "0.15005000", None),
            ("--from-transaction", exchange_out, cold_in, "0.15005000", "0.15005000"),
            ("--from-transaction", cold_in, cold_out, "0.15005000", "0.15005000"),
            ("--from-transaction", cold_out, target_in, "0.15000000", "0.15005000"),
        ]:
            args = [
                "source-funds",
                "links",
                "create",
                "--workspace",
                "AtSof",
                "--profile",
                "Austria",
                from_arg,
                from_ref,
                "--to-transaction",
                to_ref,
                "--type",
                "manual_source" if from_arg == "--from-source" else "self_transfer",
                "--allocation-amount",
                allocation,
                "--allocation-policy",
                "explicit",
            ]
            if from_amount:
                args.extend(["--from-amount", from_amount])
            self.cli(*args)
        report = self.cli(
            "reports",
            "source-funds",
            "--workspace",
            "AtSof",
            "--profile",
            "Austria",
            "--target-transaction",
            target_in,
            "--target-amount",
            "0.15000000",
            "--purpose",
            "planned_exchange_sale",
            "--planned-destination",
            "Example Broker Austria",
            "--reveal-mode",
            "standard",
            "--save-case",
        )["data"]
        context = report["report_context"]
        self.assertEqual(context["template_key"], "at_eur_basic")
        self.assertEqual(context["jurisdiction_label"], "Austria")
        self.assertEqual(context["fiat_currency"], "EUR")
        self.assertIn("Mittelherkunftsnachweis", context["report_title"])
        self.assertEqual(len(report["simplified_flow"]["edges"]), 4)
        self.assertEqual(
            {link["txid"] for link in report["disclosure_preview"]["explorer_links"]},
            {exchange_withdraw_txid, cold_consolidation_txid},
        )
        self.assertTrue(
            all(link["url"].startswith("https://mempool.space/tx/") for link in report["disclosure_preview"]["explorer_links"])
        )
        self.assertTrue(report["explain_gates"]["exportable"], report["explain_gates"]["blockers"])
        pdf_path = self.root / "at-source-funds.pdf"
        self.cli(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "AtSof",
            "--profile",
            "Austria",
            "--case",
            report["case"]["id"],
            "--file",
            str(pdf_path),
        )
        if shutil.which("pdftotext"):
            extracted = subprocess.run(
                ["pdftotext", "-layout", str(pdf_path), "-"],
                check=True,
                text=True,
                capture_output=True,
            ).stdout
            self.assertIn("Mittelherkunftsnachweis", extracted)
            self.assertIn("Evidence Checklist", extracted)
            self.assertIn("Austrian/EUR", extracted)
            self.assertIn("mempool.space", extracted)
        pdf_bytes = pdf_path.read_bytes()
        self.assertIn(f"https://mempool.space/tx/{exchange_withdraw_txid}".encode(), pdf_bytes)
        self.assertIn(f"https://mempool.space/tx/{cold_consolidation_txid}".encode(), pdf_bytes)

    def test_explorer_links_follow_wallet_network_config(self):
        self._init_default_workspace()

        def report_for_wallet(
            *,
            wallet: str,
            txid: str,
            asset: str,
            chain: str,
            network: str,
        ) -> dict:
            csv_name = f"{wallet.lower().replace(' ', '-')}.csv"
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-05-01T09:00:00Z,{txid},inbound,{asset},0.10000000,0,50000,Network target\n",
            )
            if chain == "liquid":
                self._create_wallet_and_import(wallet, csv_name)
                with self._db() as conn:
                    conn.execute(
                        """
                        UPDATE wallets
                        SET config_json = ?
                        WHERE label = ?
                        """,
                        (json.dumps({"chain": chain, "network": network}), wallet),
                    )
            else:
                self._create_wallet_and_import(wallet, csv_name, chain=chain, network=network)
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
                f"{wallet} source",
                "--asset",
                asset,
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
                txid,
                "--type",
                "manual_source",
                "--allocation-amount",
                "0.10000000",
                "--allocation-policy",
                "explicit",
            )
            return self._source_funds_report_for_target(
                target=txid,
                amount="0.10000000",
                reveal_mode="standard",
            )

        bitcoin_testnet_txid = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        bitcoin_signet_txid = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        bitcoin_regtest_txid = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc"
        liquid_txid = "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"

        testnet = report_for_wallet(
            wallet="Bitcoin Testnet",
            txid=bitcoin_testnet_txid,
            asset="BTC",
            chain="bitcoin",
            network="testnet",
        )
        signet = report_for_wallet(
            wallet="Bitcoin Signet",
            txid=bitcoin_signet_txid,
            asset="BTC",
            chain="bitcoin",
            network="signet",
        )
        regtest = report_for_wallet(
            wallet="Bitcoin Regtest",
            txid=bitcoin_regtest_txid,
            asset="BTC",
            chain="bitcoin",
            network="regtest",
        )
        liquid = report_for_wallet(
            wallet="Liquid Main",
            txid=liquid_txid,
            asset="L-BTC",
            chain="liquid",
            network="main",
        )

        self.assertEqual(
            testnet["disclosure_preview"]["explorer_links"][0]["url"],
            f"https://mempool.space/testnet/tx/{bitcoin_testnet_txid}",
        )
        self.assertEqual(
            signet["disclosure_preview"]["explorer_links"][0]["url"],
            f"https://mempool.space/signet/tx/{bitcoin_signet_txid}",
        )
        self.assertEqual(regtest["disclosure_preview"]["explorer_links"], [])
        self.assertEqual(
            liquid["disclosure_preview"]["explorer_links"][0]["url"],
            f"https://liquid.network/tx/{liquid_txid}",
        )

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

    def test_export_blocks_when_chain_observation_link_is_unconfirmed(self):
        """End-to-end gate: a reviewed self-transfer link with
        uses_chain_observation=1 and chain_data_confirmed=0 must keep
        the case blocked, surface unconfirmed_chain_data in blockers,
        and refuse export-source-funds-pdf even after a save-case."""
        self._init_default_workspace()
        for wallet, csv_name, txid, occurred_at in [
            ("ChainParent", "chain-parent.csv", "chain-parent", "2026-04-01T09:00:00Z"),
            ("ChainTarget", "chain-target.csv", "chain-target", "2026-04-02T09:00:00Z"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"{occurred_at},{txid},inbound,BTC,0.10000000,0,50000,row\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        # Root the parent in a reviewed source so the only outstanding
        # gate is the unconfirmed chain observation on the parent->target
        # link.
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
            "Reviewed root",
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
            "chain-parent",
            "--type",
            "manual_source",
            "--allocation-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
        )
        # Reviewed self-transfer link with chain observation but no
        # confirmation. --chain-data-confirmed is omitted on purpose
        # so the link records uses_chain_observation=1,
        # chain_data_confirmed=0 (the failure mode the gate guards).
        self.cli(
            "source-funds",
            "links",
            "create",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--from-transaction",
            "chain-parent",
            "--to-transaction",
            "chain-target",
            "--type",
            "self_transfer",
            "--allocation-amount",
            "0.10000000",
            "--from-amount",
            "0.10000000",
            "--allocation-policy",
            "explicit",
            "--uses-chain-observation",
        )
        blockers, report = self._report_blockers("chain-target", "0.10000000")
        self.assertIn("unconfirmed_chain_data", blockers)
        self.assertFalse(report["explain_gates"]["exportable"])
        # Save-case path also stamps blocked, and the export gate
        # refuses the saved snapshot.
        preview = self._source_funds_report_for_target(
            target="chain-target",
            amount="0.10000000",
            save_case=True,
        )
        self.assertEqual(preview["case"]["status"], "blocked")
        error = self.cli_error(
            "reports",
            "export-source-funds-pdf",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--case",
            preview["case"]["id"],
            "--file",
            str(self.root / "unconfirmed-chain.pdf"),
        )
        self.assertEqual(error["error"]["code"], "export_blocked")
        details_blockers = {
            item["code"] for item in error["error"]["details"]["blockers"]
        }
        self.assertIn("unconfirmed_chain_data", details_blockers)

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
        first_txid = "11" * 32
        second_txid = "22" * 32
        for wallet, csv_name, txid, direction in [
            ("First Out", "first-out.csv", first_txid, "outbound"),
            ("First In", "first-in.csv", first_txid, "inbound"),
            ("Second Out", "second-out.csv", second_txid, "outbound"),
            ("Second In", "second-in.csv", second_txid, "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")
        first_target = self._tx_id("First In", first_txid)
        second_target = self._tx_id("Second In", second_txid)
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

    def test_bulk_review_skips_same_onchain_scope_when_third_row_appears(self):
        self._init_default_workspace()
        txid = "33" * 32
        for wallet, csv_name, direction in [
            ("Pair Out", "stale-pair-out.csv", "outbound"),
            ("Pair In", "stale-pair-in.csv", "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        target_id = self._tx_id("Pair In", txid)
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
        self.assertEqual(
            len(
                [
                    link
                    for link in suggested
                    if link["method"] == "same_onchain_scope"
                ]
            ),
            1,
        )
        self._write_csv(
            "stale-third-in.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            f"2026-03-01T09:05:00Z,{txid},inbound,BTC,0.10000000,0,50000,Third matching row\n",
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

    def test_arbitrary_shared_import_id_is_not_a_physical_link(self):
        self._init_default_workspace()
        for wallet, csv_name, direction in (
            ("Import Out", "import-id-out.csv", "outbound"),
            ("Import In", "import-id-in.csv", "inbound"),
        ):
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,provider-batch,{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)

        suggested = self.cli(
            "source-funds",
            "suggest",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            self._tx_id("Import In", "provider-batch"),
        )["data"]["links"]

        self.assertFalse(
            [link for link in suggested if link["method"] == "same_onchain_scope"]
        )

    def test_same_onchain_scope_partial_rows_stay_manual(self):
        self._init_default_workspace()
        txid = "aa" * 32
        for wallet, csv_name, direction, amount in (
            ("Partial Out", "partial-scope-out.csv", "outbound", "0.10010000"),
            ("Partial In", "partial-scope-in.csv", "inbound", "0.10000000"),
        ):
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,{amount},0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        target_id = self._tx_id("Partial In", txid)

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
        link = next(
            link for link in suggested if link["method"] == "same_onchain_scope"
        )
        self.assertEqual(link["confidence"], "strong")

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
        self.assertEqual(reviewed["skipped"], 1)

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

    def test_transaction_pair_suggestions_allocate_reused_outbound_leg(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction, amount in [
            ("CJ Out", "cj-out.csv", "cj-out", "outbound", "1.00000000"),
            ("Postmix B", "postmix-b.csv", "postmix-b", "inbound", "0.40000000"),
            ("Postmix C", "postmix-c.csv", "postmix-c", "inbound", "0.60000000"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,{amount},0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        out_id = self._tx_id("CJ Out", "cj-out")
        in_b_id = self._tx_id("Postmix B", "postmix-b")
        in_c_id = self._tx_id("Postmix C", "postmix-c")
        for in_id in (in_b_id, in_c_id):
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
                "whirlpool",
                "--policy",
                "carrying-value",
            )

        self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")
        links = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        pair_links = {
            link["to_transaction_id"]: link
            for link in links
            if link["method"] == "transaction_pair"
        }
        self.assertEqual(pair_links[in_b_id]["allocation_amount"], 0.4)
        self.assertEqual(pair_links[in_b_id]["from_allocation_amount"], 0.4)
        self.assertEqual(pair_links[in_c_id]["allocation_amount"], 0.6)
        self.assertEqual(pair_links[in_c_id]["from_allocation_amount"], 0.6)

    def test_transaction_pair_suggestions_allocate_reused_inbound_leg(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction, amount in [
            ("Premix A", "premix-a.csv", "premix-a", "outbound", "0.40000000"),
            ("Premix B", "premix-b.csv", "premix-b", "outbound", "0.60000000"),
            ("Postmix C", "postmix-c.csv", "postmix-c", "inbound", "1.00000000"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,{amount},0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        out_a_id = self._tx_id("Premix A", "premix-a")
        out_b_id = self._tx_id("Premix B", "premix-b")
        in_id = self._tx_id("Postmix C", "postmix-c")
        for out_id in (out_a_id, out_b_id):
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
                "whirlpool",
                "--policy",
                "carrying-value",
            )

        self.cli("source-funds", "suggest", "--workspace", "Sof", "--profile", "Default")
        links = self.cli("source-funds", "links", "list", "--workspace", "Sof", "--profile", "Default")["data"]
        pair_links = {
            link["from_transaction_id"]: link
            for link in links
            if link["method"] == "transaction_pair"
        }
        self.assertEqual(pair_links[out_a_id]["allocation_amount"], 0.4)
        self.assertEqual(pair_links[out_a_id]["from_allocation_amount"], 0.4)
        self.assertEqual(pair_links[out_b_id]["allocation_amount"], 0.6)
        self.assertEqual(pair_links[out_b_id]["from_allocation_amount"], 0.6)

    def test_bulk_review_skips_transaction_pair_with_stale_allocation(self):
        self._init_default_workspace()
        for wallet, csv_name, txid, direction, amount in [
            ("CJ Out", "stale-cj-out.csv", "stale-cj-out", "outbound", "1.00000000"),
            ("Postmix B", "stale-postmix-b.csv", "stale-postmix-b", "inbound", "0.40000000"),
            ("Postmix C", "stale-postmix-c.csv", "stale-postmix-c", "inbound", "0.60000000"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,{amount},0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        out_id = self._tx_id("CJ Out", "stale-cj-out")
        in_b_id = self._tx_id("Postmix B", "stale-postmix-b")
        in_c_id = self._tx_id("Postmix C", "stale-postmix-c")
        for in_id in (in_b_id, in_c_id):
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
                "whirlpool",
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
            in_b_id,
        )
        with self._db() as conn:
            conn.execute(
                """
                UPDATE source_funds_links
                SET from_allocation_amount = ?
                WHERE method = 'transaction_pair' AND to_transaction_id = ?
                """,
                (100_000_000_000, in_b_id),
            )
        reviewed = self.cli(
            "source-funds",
            "links",
            "bulk-review",
            "--workspace",
            "Sof",
            "--profile",
            "Default",
            "--target-transaction",
            in_b_id,
        )["data"]
        self.assertEqual(reviewed["reviewed"], 0)

    def test_suggest_links_with_target_does_not_write_unrelated_suggestions(self):
        self._init_default_workspace()
        first_txid = "44" * 32
        second_txid = "55" * 32
        for wallet, csv_name, txid, direction in [
            ("First Out", "first-out.csv", first_txid, "outbound"),
            ("First In", "first-in.csv", first_txid, "inbound"),
            ("Second Out", "second-out.csv", second_txid, "outbound"),
            ("Second In", "second-in.csv", second_txid, "inbound"),
        ]:
            self._write_csv(
                csv_name,
                "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
                f"2026-03-01T09:00:00Z,{txid},{direction},BTC,0.10000000,0,50000,{wallet}\n",
            )
            self._create_wallet_and_import(wallet, csv_name)
        first_target = self._tx_id("First In", first_txid)
        second_target = self._tx_id("Second In", second_txid)
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
        first_txid = "66" * 32
        second_txid = "77" * 32
        for wallet, csv_name, txid, direction in [
            ("First Out", "cap-first-out.csv", first_txid, "outbound"),
            ("First In", "cap-first-in.csv", first_txid, "inbound"),
            ("Second Out", "cap-second-out.csv", second_txid, "outbound"),
            ("Second In", "cap-second-in.csv", second_txid, "inbound"),
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

    def test_provider_trade_id_one_to_one_stays_manual(self):
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
        self.assertEqual(reviewed["reviewed"], 0)
        self.assertEqual(reviewed["skipped"], 1)

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
