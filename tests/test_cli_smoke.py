"""End-to-end CLI smoke test.

This exercises the AGENTS.md "safe local workflow" against a temp data root
and asserts that every command returns the expected JSON envelope shape
(kind, schema_version, data).

It is intentionally stdlib-only (no pytest dep) and invokes the CLI as a
subprocess so it pins the external contract — not implementation details.
The suite is designed to survive a pure-refactor split of kassiber/app.py
into modules.
"""

import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parent.parent


_PHOENIX_CSV = """date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description
2024-05-01T10:15:00Z,11111111-aaaa-bbbb-cccc-000000000001,swap_in,5000000000,2000 USD,0,250,0.10 USD,0,0 USD,,abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789,bc1qexamplefakedestination0000000000000000,Onchain deposit
2024-05-02T12:00:00Z,22222222-aaaa-bbbb-cccc-000000000002,lightning_received,3000000,1.20 USD,0,0,0 USD,0,0 USD,1111111111111111111111111111111111111111111111111111111111111111,,03abcdefnodepubkeyfakefakefakefakefakefakefakefakefakefakefakefake,Tip from friend
2024-05-03T14:30:00Z,33333333-aaaa-bbbb-cccc-000000000003,lightning_sent,-5000000,-2.00 USD,0,0,0 USD,50000,0.02 USD,2222222222222222222222222222222222222222222222222222222222222222,,03deadbeefcafebabefakefakefakefakefakefakefakefakefakefakefakefake,Coffee shop
2024-05-04T09:00:00Z,44444444-aaaa-bbbb-cccc-000000000004,channel_close,-500000000,-200 USD,0,1500,0.60 USD,0,0 USD,,fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210,bc1qexamplefakechannelclose0000000000000000,Channel close to self
"""

_CACHE_PRICING_CSV = """date,txid,direction,asset,amount,fee,description
2024-05-10T09:00:00Z,cache-price-1,inbound,BTC,0.01000000,0,Cached price sample
"""

_CONFIRMED_PRICING_CSV = """date,confirmed_at,txid,direction,asset,amount,fee,description
2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-price-1,inbound,BTC,0.01000000,0,Confirmed price sample
"""

# Cross-wallet self-transfer scenario: cold wallet receives 1 BTC, then sends
# 0.5 BTC + 0.001 BTC network fee to the hot wallet. The same on-chain txid
# appears in both wallet exports, which is the trigger for IntraTransaction
# detection. With detection on: only the 0.001 BTC network fee is realized as
# a disposal; the 0.5 BTC transfer carries its cost basis to the hot wallet.
_COLD_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,cold-funding-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,onchain-self-transfer-1,outbound,BTC,0.50000000,0.001,65000,Move to hot wallet
"""

_HOT_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-02-01T12:00:00Z,onchain-self-transfer-1,inbound,BTC,0.50000000,0,65000,Receive from cold wallet
"""

_COLD_TRANSFER_VALUE_ONLY_CSV = """date,txid,direction,asset,amount,fee,fiat_value,description
2026-01-01T10:00:00Z,cold-funding-value-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,onchain-self-transfer-value-1,outbound,BTC,0.50000000,0.001,32500,Move to hot wallet
"""

_HOT_TRANSFER_VALUE_ONLY_CSV = """date,txid,direction,asset,amount,fee,fiat_value,description
2026-02-01T12:00:00Z,onchain-self-transfer-value-1,inbound,BTC,0.50000000,0,32500,Receive from cold wallet
"""

# Manual same-asset pair scenario: two BTC legs whose external_ids deliberately
# don't match, so auto-detection skips them. The user knows they're paired
# (e.g., a swap via a custom counterparty) and creates a manual pair.
_MANUAL_FROM_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-01T10:00:00Z,manual-fund-1,inbound,BTC,0.20000000,0,70000,Acquisition
2026-03-15T10:00:00Z,manual-out-leg,outbound,BTC,0.10000000,0.0005,72000,Manual swap out
"""

_MANUAL_TO_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-03-15T10:05:00Z,manual-in-leg,inbound,BTC,0.10000000,0,72000,Manual swap in
"""

# Cross-asset (BTC → LBTC) scenario for the carrying-value rejection +
# taxable acceptance tests.
_CROSS_BTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T10:00:00Z,cross-fund-1,inbound,BTC,0.10000000,0,80000,BTC acquisition
2026-04-15T10:00:00Z,cross-out-leg,outbound,BTC,0.10000000,0.0001,82000,Peg-in to Liquid
"""

_CROSS_BTC_AT_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T10:00:00Z,cross-fund-1,inbound,BTC,0.10010000,0,80000,BTC acquisition with fee buffer
2026-04-15T10:00:00Z,cross-out-leg,outbound,BTC,0.10000000,0.0001,82000,Peg-in to Liquid
"""

_CROSS_LBTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-15T10:30:00Z,cross-in-leg,inbound,LBTC,0.10000000,0,82000,Peg-in receive
"""


def _sample_descriptor_pair():
    from embit import bip32

    seed = bytes.fromhex("000102030405060708090a0b0c0d0e0f" * 4)
    root = bip32.HDKey.from_seed(seed)
    account = root.derive("m/84h/0h/0h")
    xpub = account.to_public().to_base58()
    fingerprint = root.my_fingerprint.hex()
    origin = f"[{fingerprint}/84h/0h/0h]"
    return (
        f"wpkh({origin}{xpub}/0/*)",
        f"wpkh({origin}{xpub}/1/*)",
        "m/84'/0'/0'",
        fingerprint,
    )


def _sample_multisig_branch_descriptor():
    from embit import bip32

    keys = []
    for marker in range(1, 5):
        root = bip32.HDKey.from_seed(bytes([marker]) * 64)
        account = root.derive("m/48h/0h/0h/2h")
        keys.append(
            f"[{root.my_fingerprint.hex()}/48h/0h/0h/2h]{account.to_public().to_base58()}/<0;1>/*"
        )
    return "wsh(\n  sortedmulti(\n    2,\n    " + ",\n    ".join(keys) + "\n  )\n)\n"


def _run(data_root, *args):
    """Invoke `python -m kassiber --data-root DATA --machine ARGS...`.

    Returns (payload_dict, returncode). Never raises on non-zero exit; the
    caller asserts on the returncode when an error envelope is expected.
    """
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
    stdout = result.stdout.strip()
    if not stdout:
        raise AssertionError(
            f"CLI produced no stdout.\nargs: {args}\nstderr: {result.stderr}"
        )
    try:
        payload = json.loads(stdout)
    except json.JSONDecodeError as exc:
        raise AssertionError(
            f"CLI stdout was not JSON.\nargs: {args}\nstdout: {stdout[:400]}"
        ) from exc
    return payload, result.returncode


class CliSmokeTest(unittest.TestCase):
    """Walks through init → workspace → profile → wallet → Phoenix import →
    journals → reports → rates, asserting envelope shape at each step.

    Tests run in alphabetical order (unittest default); the test_NN_ prefix
    is what sequences them.
    """

    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-smoke-")
        cls.data_root = Path(cls._tmp.name) / "data"
        cls.phoenix_csv = Path(cls._tmp.name) / "phoenix.csv"
        cls.phoenix_csv.write_text(_PHOENIX_CSV, encoding="utf-8")
        cls.cache_pricing_csv = Path(cls._tmp.name) / "cache-pricing.csv"
        cls.cache_pricing_csv.write_text(_CACHE_PRICING_CSV, encoding="utf-8")
        cls.confirmed_pricing_csv = Path(cls._tmp.name) / "confirmed-pricing.csv"
        cls.confirmed_pricing_csv.write_text(_CONFIRMED_PRICING_CSV, encoding="utf-8")
        cls.cold_transfer_csv = Path(cls._tmp.name) / "cold-transfer.csv"
        cls.cold_transfer_csv.write_text(_COLD_TRANSFER_CSV, encoding="utf-8")
        cls.hot_transfer_csv = Path(cls._tmp.name) / "hot-transfer.csv"
        cls.hot_transfer_csv.write_text(_HOT_TRANSFER_CSV, encoding="utf-8")
        cls.cold_transfer_value_only_csv = Path(cls._tmp.name) / "cold-transfer-value-only.csv"
        cls.cold_transfer_value_only_csv.write_text(_COLD_TRANSFER_VALUE_ONLY_CSV, encoding="utf-8")
        cls.hot_transfer_value_only_csv = Path(cls._tmp.name) / "hot-transfer-value-only.csv"
        cls.hot_transfer_value_only_csv.write_text(_HOT_TRANSFER_VALUE_ONLY_CSV, encoding="utf-8")
        cls.manual_from_csv = Path(cls._tmp.name) / "manual-from.csv"
        cls.manual_from_csv.write_text(_MANUAL_FROM_CSV, encoding="utf-8")
        cls.manual_to_csv = Path(cls._tmp.name) / "manual-to.csv"
        cls.manual_to_csv.write_text(_MANUAL_TO_CSV, encoding="utf-8")
        cls.cross_btc_csv = Path(cls._tmp.name) / "cross-btc.csv"
        cls.cross_btc_csv.write_text(_CROSS_BTC_CSV, encoding="utf-8")
        cls.cross_btc_at_csv = Path(cls._tmp.name) / "cross-btc-at.csv"
        cls.cross_btc_at_csv.write_text(_CROSS_BTC_AT_CSV, encoding="utf-8")
        cls.cross_lbtc_csv = Path(cls._tmp.name) / "cross-lbtc.csv"
        cls.cross_lbtc_csv.write_text(_CROSS_LBTC_CSV, encoding="utf-8")
        cls.attachment_file = Path(cls._tmp.name) / "attachment-note.txt"
        cls.attachment_file.write_text("Signed invoice copy\n", encoding="utf-8")
        (
            cls.sample_descriptor,
            cls.sample_change_descriptor,
            cls.sample_derivation_root,
            cls.sample_fingerprint,
        ) = _sample_descriptor_pair()
        cls.sample_multisig_descriptor_pretty = _sample_multisig_branch_descriptor()
        cls.multisig_descriptor_file = Path(cls._tmp.name) / "multisig-descriptor.txt"
        cls.multisig_descriptor_file.write_text(cls.sample_multisig_descriptor_pretty, encoding="utf-8")

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def _cli(self, *args):
        payload, code = _run(self.data_root, *args)
        if code != 0:
            self.fail(
                f"CLI exited {code} for {args!r}; envelope: {json.dumps(payload)[:400]}"
            )
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("data", payload)
        return payload

    def _assert_kind(self, payload, expected):
        self.assertEqual(payload.get("kind"), expected)

    # -- workflow -----------------------------------------------------

    def test_01_init_status(self):
        payload = self._cli("init")
        self._assert_kind(payload, "init")
        self.assertEqual(payload["data"]["state_root"], str(self.data_root.parent))
        self.assertEqual(payload["data"]["config_root"], str(self.data_root.parent / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(self.data_root.parent / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(self.data_root.parent / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(self.data_root.parent / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(self.data_root.parent / "config" / "backends.env"))

        payload = self._cli("status")
        self._assert_kind(payload, "status")
        auth = payload["data"].get("auth", {})
        self.assertEqual(auth.get("mode"), "local")
        self.assertTrue(auth.get("authenticated"))
        self.assertEqual(payload["data"]["state_root"], str(self.data_root.parent))
        self.assertEqual(payload["data"]["config_root"], str(self.data_root.parent / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(self.data_root.parent / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(self.data_root.parent / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(self.data_root.parent / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(self.data_root.parent / "config" / "backends.env"))

        payload = self._cli("diagnostics", "collect")
        self._assert_kind(payload, "diagnostics.collect")
        self.assertTrue(payload["data"]["public_safe"])
        self.assertEqual(payload["data"]["storage"]["diagnostics_location"], "exports/diagnostics")
        self.assertIn("counts", payload["data"]["state"])

    def test_01a_backends_batch_size_roundtrip(self):
        payload = self._cli(
            "backends", "create", "bench",
            "--kind", "electrum",
            "--url", "ssl://electrum.example:50002",
            "--batch-size", "25",
        )
        self._assert_kind(payload, "backends.create")
        self.assertEqual(payload["data"]["batch_size"], 25)

        payload = self._cli(
            "backends", "update", "bench",
            "--batch-size", "40",
        )
        self._assert_kind(payload, "backends.update")
        self.assertEqual(payload["data"]["batch_size"], 40)

        payload = self._cli("backends", "get", "bench")
        self._assert_kind(payload, "backends.get")
        self.assertEqual(payload["data"]["batch_size"], 40)

        payload = self._cli("backends", "list")
        self._assert_kind(payload, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertEqual(rows["bench"]["batch_size"], 40)
        self.assertEqual(rows["fulcrum"]["batch_size"], 100)
        self.assertEqual(rows["liquid"]["batch_size"], 100)

    def test_02_workspace_profile(self):
        payload = self._cli("workspaces", "create", "Main")
        self._assert_kind(payload, "workspaces.create")

        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Default",
        )
        self._assert_kind(payload, "profiles.create")

        payload = self._cli("profiles", "list")
        self._assert_kind(payload, "profiles.list")
        profiles = payload["data"]
        self.assertIsInstance(profiles, list)
        self.assertEqual(len(profiles), 1)
        prof = profiles[0]
        self.assertIn("tax_country", prof)
        self.assertIn("tax_long_term_days", prof)
        self.assertEqual(prof["tax_country"], "generic")
        self.assertEqual(prof["fiat_currency"], "USD")

    def test_02a_ui_snapshot(self):
        payload = self._cli("ui", "--workspace", "Main", "--profile", "Default")
        self._assert_kind(payload, "ui.snapshot")
        self.assertEqual(payload["data"]["scope"]["workspace_label"], "Main")
        self.assertEqual(payload["data"]["scope"]["profile_label"], "Default")
        self.assertEqual(payload["data"]["shell"]["project_label"], "Main / Default")
        self.assertEqual(payload["data"]["shell"]["connection_count"], 0)
        self.assertTrue(payload["data"]["shell"]["is_empty"])
        self.assertEqual(len(payload["data"]["profiles"]), 1)
        self.assertIn("placeholder", payload["data"]["shell"]["notices"][0].lower())
        self.assertIn("GENERIC POLICY", payload["data"]["reports"]["header_eyebrow"])
        self.assertNotIn("\u00a727", payload["data"]["reports"]["header_eyebrow"])

    def test_03_wallet_create(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Phoenix",
            "--kind", "phoenix",
        )
        self._assert_kind(payload, "wallets.create")
        self.assertEqual(payload["data"]["label"], "Phoenix")
        self.assertEqual(payload["data"]["kind"], "phoenix")

    def test_03a_descriptor_derive_exposes_paths(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Vault",
            "--kind", "descriptor",
            "--descriptor", self.sample_descriptor,
            "--change-descriptor", self.sample_change_descriptor,
            "--gap-limit", "5",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--count", "2",
        )
        self._assert_kind(payload, "wallets.derive")
        rows = payload["data"]
        self.assertEqual(len(rows), 4)

        receive_0 = rows[0]
        self.assertEqual(receive_0["branch_label"], "receive")
        self.assertEqual(receive_0["derivation_path"], f"{self.sample_derivation_root}/0/0")
        self.assertEqual(receive_0["derivation_paths"], [f"{self.sample_derivation_root}/0/0"])
        self.assertEqual(receive_0["key_origins"], [f"[{self.sample_fingerprint}/84'/0'/0'/0/0]"])

        change_0 = rows[2]
        self.assertEqual(change_0["branch_label"], "change")
        self.assertEqual(change_0["derivation_path"], f"{self.sample_derivation_root}/1/0")
        self.assertEqual(change_0["derivation_paths"], [f"{self.sample_derivation_root}/1/0"])
        self.assertEqual(change_0["key_origins"], [f"[{self.sample_fingerprint}/84'/0'/0'/1/0]"])

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Vault",
            "--branch", "change",
            "--start", "1",
            "--count", "1",
        )
        self._assert_kind(payload, "wallets.derive")
        change_only = payload["data"]
        self.assertEqual(len(change_only), 1)
        self.assertEqual(change_only[0]["branch_label"], "change")
        self.assertEqual(change_only[0]["derivation_path"], f"{self.sample_derivation_root}/1/1")

    def test_03b_descriptor_file_accepts_pretty_printed_multisig(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Pretty Vault",
            "--kind", "descriptor",
            "--descriptor-file", str(self.multisig_descriptor_file),
            "--gap-limit", "5",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "derive",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Pretty Vault",
            "--count", "1",
        )
        self._assert_kind(payload, "wallets.derive")
        rows = payload["data"]
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["branch_label"], "receive")
        self.assertEqual(rows[1]["branch_label"], "change")
        self.assertEqual(len(rows[0]["key_origins"]), 4)

    def test_04_phoenix_import(self):
        payload = self._cli(
            "wallets", "import-phoenix",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Phoenix",
            "--file", str(self.phoenix_csv),
        )
        self._assert_kind(payload, "wallets.import-phoenix")
        data = payload["data"]
        self.assertEqual(data["imported"], 4)
        self.assertEqual(data["skipped"], 0)
        self.assertEqual(data["phoenix_notes_set"], 4)
        self.assertEqual(data["phoenix_tags_added"], 4)
        self.assertEqual(data["phoenix_tags_created"], 4)

    def test_04a_ui_snapshot_uses_live_routed_data(self):
        payload = self._cli("ui", "--workspace", "Main", "--profile", "Default")
        self._assert_kind(payload, "ui.snapshot")
        transactions = payload["data"]["transactions"]
        self.assertEqual(transactions["total_count"], 4)
        self.assertEqual(len(transactions["items"]), 4)
        first_row = transactions["items"][0]
        self.assertEqual(first_row["account_label"], "Treasury")
        self.assertIn("type_label", first_row)
        self.assertIn("type_badge_tone", first_row)
        self.assertEqual(first_row["direction"], "outbound")
        self.assertEqual(first_row["amount_sats_signed_label"], "- 500,000")
        self.assertEqual(first_row["type_tone"], "negative")

        reports = payload["data"]["reports"]
        self.assertEqual(reports["summary_cards"][0]["value"], "4")
        self.assertEqual(reports["preview_title"], "Recent transaction inputs")
        self.assertEqual(len(reports["preview_rows"]), 4)
        self.assertEqual(reports["preview_rows"][0]["amount_label"], "-0.00500000 BTC")
        self.assertNotIn("AUSTRIA", reports["header_eyebrow"])
        self.assertNotIn("\u00a727", reports["header_eyebrow"])

    def test_05_msat_exposed_on_records(self):
        payload = self._cli(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "metadata.records.list")
        records = payload["data"]["records"]
        self.assertEqual(len(records), 4)
        for rec in records:
            # dual BTC/msat fields must be present on every record
            self.assertIn("amount", rec)
            self.assertIn("amount_msat", rec)
            self.assertIsInstance(rec["amount_msat"], int)
            self.assertIn("fee_msat", rec)
            self.assertIsInstance(rec["fee_msat"], int)
        # expected msat totals from the 4-row Phoenix sample
        inbound_msat = sum(r["amount_msat"] for r in records if r["direction"] == "inbound")
        outbound_msat = sum(r["amount_msat"] for r in records if r["direction"] == "outbound")
        self.assertEqual(inbound_msat, 5_000_000_000 + 3_000_000)
        self.assertEqual(outbound_msat, 5_000_000 + 500_000_000)

    def test_05a_attachments_lifecycle(self):
        payload = self._cli(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "metadata.records.list")
        tx_ref = payload["data"]["records"][0]["transaction_id"]

        payload = self._cli(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
            "--file", str(self.attachment_file),
            "--label", "Invoice copy",
        )
        self._assert_kind(payload, "attachments.add")
        file_attachment = payload["data"]
        self.assertEqual(file_attachment["attachment_type"], "file")
        self.assertEqual(file_attachment["label"], "Invoice copy")
        self.assertTrue(file_attachment["exists"])
        stored_path = self.data_root.parent / "attachments" / file_attachment["stored_relpath"]
        self.assertTrue(stored_path.exists())

        payload = self._cli(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
            "--url", "https://example.com/tx/notes/1",
            "--label", "Support ticket",
        )
        self._assert_kind(payload, "attachments.add")
        url_attachment = payload["data"]
        self.assertEqual(url_attachment["attachment_type"], "url")
        self.assertEqual(url_attachment["url"], "https://example.com/tx/notes/1")
        self.assertFalse(url_attachment["stored_relpath"])

        payload = self._cli(
            "attachments", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.list")
        rows = payload["data"]
        self.assertEqual(len(rows), 2)
        self.assertEqual(sorted(row["attachment_type"] for row in rows), ["file", "url"])

        payload = self._cli(
            "attachments", "verify",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.verify")
        self.assertEqual(payload["data"]["checked"], 2)
        self.assertEqual(payload["data"]["broken"], 0)
        self.assertEqual(payload["data"]["ok"], 2)
        by_type = {row["attachment_type"]: row for row in payload["data"]["results"]}
        self.assertEqual(by_type["file"]["status"], "ok")
        self.assertEqual(by_type["file"]["issues"], [])
        self.assertEqual(by_type["url"]["status"], "ok")
        self.assertEqual(by_type["url"]["issues"], [])

        payload = self._cli(
            "attachments", "remove",
            "--workspace", "Main",
            "--profile", "Default",
            file_attachment["id"],
        )
        self._assert_kind(payload, "attachments.remove")
        self.assertTrue(payload["data"]["removed"])
        self.assertTrue(payload["data"]["deleted_file"])
        self.assertFalse(stored_path.exists())

        payload = self._cli(
            "attachments", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", tx_ref,
        )
        self._assert_kind(payload, "attachments.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["id"], url_attachment["id"])

    def test_06_journals_process(self):
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        # 2 acquisitions + 2 disposals, 0 quarantined (fiat_rate derived from value/amount)
        self.assertEqual(data["entries_created"], 4)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["processed_transactions"], 4)

    def test_07_all_reports_succeed(self):
        for report, kind in [
            ("summary", "reports.summary"),
            ("tax-summary", "reports.tax-summary"),
            ("balance-sheet", "reports.balance-sheet"),
            ("portfolio-summary", "reports.portfolio-summary"),
            ("capital-gains", "reports.capital-gains"),
            ("journal-entries", "reports.journal-entries"),
        ]:
            payload = self._cli(
                "reports", report,
                "--workspace", "Main",
                "--profile", "Default",
            )
            self._assert_kind(payload, kind)
        payload = self._cli(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "Default",
            "--interval", "month",
        )
        self._assert_kind(payload, "reports.balance-history")

    def test_07b_summary_report_rollups(self):
        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "reports.summary")
        data = payload["data"]
        self.assertEqual(data["workspace"], "Main")
        self.assertEqual(data["profile"], "Default")
        self.assertIsNone(data["wallet"])
        self.assertEqual(data["metrics"]["wallets_in_scope"], 3)
        self.assertEqual(data["metrics"]["active_transactions"], 4)
        self.assertEqual(data["metrics"]["journal_entries"], 4)
        self.assertEqual(data["metrics"]["quarantines"], 0)
        self.assertEqual(len(data["asset_flow"]), 1)
        flow = data["asset_flow"][0]
        self.assertEqual(flow["asset"], "BTC")
        self.assertEqual(flow["fee_amount_msat"], 1800000)
        self.assertAlmostEqual(float(flow["fee_amount"]), 0.000018, places=8)

    def test_07a_export_pdf_report(self):
        pdf_path = Path(self._tmp.name) / "kassiber-report.pdf"
        if pdf_path.exists():
            pdf_path.unlink()
        payload = self._cli(
            "reports", "export-pdf",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(pdf_path),
        )
        self._assert_kind(payload, "reports.export-pdf")
        data = payload["data"]
        self.assertEqual(Path(data["file"]), pdf_path.resolve())
        self.assertGreaterEqual(data["pages"], 1)
        self.assertTrue(pdf_path.exists())
        self.assertGreater(pdf_path.stat().st_size, 1000)
        payload_bytes = pdf_path.read_bytes()
        header = payload_bytes[:8]
        self.assertTrue(header.startswith(b"%PDF-1.4"))
        self.assertRegex(payload_bytes, rb"/MediaBox \[0 0 842(?:\.0+)? 595(?:\.0+)?\]")

    def test_07aa_pdf_writer_reports_actual_page_count(self):
        from kassiber.pdf_report import write_text_pdf

        pdf_path = Path(self._tmp.name) / "kassiber-report-multipage.pdf"
        lines = ["Synthetic Report", "================", ""]
        for section in range(10):
            lines.extend(["", f"Section {section}", "-----------------"])
            lines.append("Date        Wallet          Dir  Asset        Amount           Fee  Description")
            lines.append(
                "----------  --------------  ---  ------  ------------  ------------  ----------------------------"
            )
            for index in range(45):
                lines.append(
                    f"2025-01-{(index % 28) + 1:02d}  Wallet-{section:02d}      out  BTC      "
                    f"{index * 0.12345678:,.8f}    0.00001000  Example row {index}"
                )

        result = write_text_pdf(str(pdf_path), "Synthetic Report", lines)
        payload_bytes = pdf_path.read_bytes()
        actual_pages = len(re.findall(rb"/Type /Page\b", payload_bytes))

        self.assertTrue(pdf_path.exists())
        self.assertGreater(result["pages"], 1)
        self.assertEqual(result["pages"], actual_pages)

    def test_08_capital_gains_msat_and_counts(self):
        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Default",
        )
        rows = payload["data"]
        self.assertIsInstance(rows, list)
        self.assertEqual(len(rows), 2)
        for row in rows:
            self.assertIn("quantity", row)
            self.assertIn("quantity_msat", row)
            self.assertIsInstance(row["quantity_msat"], int)
            self.assertEqual(row["entry_type"], "disposal")

    def test_09_balance_sheet_totals(self):
        payload = self._cli(
            "reports", "balance-sheet",
            "--workspace", "Main",
            "--profile", "Default",
        )
        rows = payload["data"]
        btc_rows = [r for r in rows if r.get("asset") == "BTC"]
        self.assertEqual(len(btc_rows), 1)
        # Sample math: +0.05 swap_in + 0.00003 ln_received
        #              -(0.00005 + 0.0000005) ln_sent
        #              -(0.005 + 0.000015) channel_close
        # = 0.0449645 BTC
        self.assertAlmostEqual(float(btc_rows[0]["quantity"]), 0.0449645, places=7)

    def test_10_rates_manual_roundtrip(self):
        payload = self._cli("rates", "pairs")
        self._assert_kind(payload, "rates.pairs")
        pairs = {p["pair"] for p in payload["data"]}
        self.assertIn("BTC-USD", pairs)
        self.assertIn("BTC-EUR", pairs)

        payload = self._cli(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000",
        )
        self._assert_kind(payload, "rates.set")

        payload = self._cli("rates", "latest", "BTC-USD")
        self._assert_kind(payload, "rates.latest")
        self.assertAlmostEqual(float(payload["data"]["rate"]), 65000.0, places=4)

        payload = self._cli(
            "rates", "range", "BTC-USD",
            "--start", "2024-04-01T00:00:00Z",
        )
        self._assert_kind(payload, "rates.range")
        samples = payload["data"]
        self.assertIsInstance(samples, list)
        self.assertEqual(len(samples), 1)
        self.assertEqual(samples[0]["source"], "manual")

    def test_11_rates_cache_autopricing(self):
        payload = self._cli(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "CachePriced",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "CachePriced",
            "--file", str(self.cache_pricing_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "61000",
        )
        self._assert_kind(payload, "rates.set")

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        self.assertEqual(data["entries_created"], 5)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["auto_priced"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "CachePriced",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertAlmostEqual(float(record["fiat_rate"]), 61000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 610.0, places=4)

    def test_11a_rates_cache_prefers_confirmed_at_when_present(self):
        workspace = "ConfirmedPricing"
        profile = "ConfirmedPricingDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedPriced",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedPriced",
            "--file", str(self.confirmed_pricing_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        self._cli("rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "60000")
        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", profile,
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        payload = self._cli(
            "transactions", "list",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedPriced",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(float(record["fiat_rate"]), 62000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 620.0, places=4)

    def test_11b_repeat_import_merges_confirmed_at_without_duplicate(self):
        workspace = "ConfirmedMergeSpace"
        profile = "ConfirmedMergeDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedMerge",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-merge-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,confirmed-merge-1,inbound,BTC,0.01000000,0,First import\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-merge-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,2024-05-10T12:00:00Z,confirmed-merge-1,inbound,BTC,0.01000000,0,Second import\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedMerge",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedMerge",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = (SELECT id FROM wallets WHERE label = 'ConfirmedMerge')"
        ).fetchone()
        record = conn.execute(
            "SELECT occurred_at, confirmed_at FROM transactions WHERE external_id = 'confirmed-merge-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(record["occurred_at"], "2024-05-10T12:00:00Z")
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")

    def test_11c_repeat_import_replaces_unknown_occurred_at_without_duplicate(self):
        workspace = "ConfirmedShiftSpace"
        profile = "ConfirmedShiftDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedShift",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-shift-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "1970-01-01T00:00:00Z,confirmed-shift-1,inbound,BTC,0.01000000,0,First sync placeholder\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-shift-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T12:00:00Z,2024-05-10T12:00:00Z,confirmed-shift-1,inbound,BTC,0.01000000,0,Confirmed sync\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedShift",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedShift",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE wallet_id = (SELECT id FROM wallets WHERE label = 'ConfirmedShift')"
        ).fetchone()
        record = conn.execute(
            "SELECT occurred_at, confirmed_at FROM transactions WHERE external_id = 'confirmed-shift-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(record["occurred_at"], "2024-05-10T12:00:00Z")
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")

    def test_11d_confirmed_at_merge_reprices_cache_derived_values(self):
        workspace = "ConfirmedRepriceSpace"
        profile = "ConfirmedRepriceDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedReprice",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-reprice-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,confirmed-reprice-1,inbound,BTC,0.01000000,0,First unconfirmed copy\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-reprice-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-reprice-1,inbound,BTC,0.01000000,0,Confirmed copy\n",
            encoding="utf-8",
        )

        self._cli("rates", "set", "BTC-USD", "2024-05-09T00:00:00Z", "60000")
        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            "SELECT fiat_rate, fiat_value, fiat_price_source FROM transactions WHERE external_id = 'confirmed-reprice-1'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(row["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(row["fiat_value"], 600.0, places=4)
        self.assertEqual(row["fiat_price_source"], "rates_cache")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)

        payload = self._cli(
            "transactions",
            "list",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedReprice",
        )
        self._assert_kind(payload, "transactions.list")
        record = payload["data"][0]
        self.assertEqual(record["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(float(record["fiat_rate"]), 62000.0, places=4)
        self.assertAlmostEqual(float(record["fiat_value"]), 620.0, places=4)

    def test_11e_repeat_import_does_not_desync_fingerprint(self):
        workspace = "FingerprintMergeSpace"
        profile = "FingerprintMergeDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "FingerprintMerge",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "fingerprint-merge-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,fingerprint-merge-1,inbound,BTC,0.01000000,0,First copy\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "fingerprint-merge-second.csv"
        second_csv.write_text(
            "date,txid,direction,asset,amount,fee,description\n"
            "2024-05-10T09:00:00Z,fingerprint-merge-1,inbound,BTC,0.01000000,0,Conflicting timestamp copy\n",
            encoding="utf-8",
        )

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "FingerprintMerge",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        before = conn.execute(
            "SELECT occurred_at, fingerprint FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        conn.close()

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "FingerprintMerge",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        count = conn.execute(
            "SELECT COUNT(*) AS n FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        after = conn.execute(
            "SELECT occurred_at, fingerprint FROM transactions WHERE external_id = 'fingerprint-merge-1'"
        ).fetchone()
        conn.close()

        self.assertEqual(count["n"], 1)
        self.assertEqual(after["occurred_at"], before["occurred_at"])
        self.assertEqual(after["fingerprint"], before["fingerprint"])

    def test_11f_confirmed_at_merge_preserves_imported_price(self):
        workspace = "ConfirmedImportedPriceSpace"
        profile = "ConfirmedImportedPriceDefault"
        self._assert_kind(self._cli("workspaces", "create", workspace), "workspaces.create")
        self._assert_kind(
            self._cli("profiles", "create", "--workspace", workspace, profile),
            "profiles.create",
        )
        payload = self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", profile,
            "--label", "ConfirmedImportedPrice",
            "--kind", "custom",
        )
        self._assert_kind(payload, "wallets.create")

        first_csv = Path(self._tmp.name) / "confirmed-imported-price-first.csv"
        first_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2024-05-09T09:00:00Z,confirmed-imported-price-1,inbound,BTC,0.01000000,0,60000,Imported price\n",
            encoding="utf-8",
        )
        second_csv = Path(self._tmp.name) / "confirmed-imported-price-second.csv"
        second_csv.write_text(
            "date,confirmed_at,txid,direction,asset,amount,fee,description\n"
            "2024-05-09T09:00:00Z,2024-05-10T12:00:00Z,confirmed-imported-price-1,inbound,BTC,0.01000000,0,Confirmed copy\n",
            encoding="utf-8",
        )

        self._cli("rates", "set", "BTC-USD", "2024-05-10T00:00:00Z", "62000")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedImportedPrice",
            "--file", str(first_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 0)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", profile,
            "--wallet", "ConfirmedImportedPrice",
            "--file", str(second_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        payload = self._cli("journals", "process", "--workspace", workspace, "--profile", profile)
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 0)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        row = conn.execute(
            """
            SELECT confirmed_at, fiat_rate, fiat_value, fiat_price_source
            FROM transactions
            WHERE external_id = 'confirmed-imported-price-1'
            """
        ).fetchone()
        conn.close()

        self.assertEqual(row["confirmed_at"], "2024-05-10T12:00:00Z")
        self.assertAlmostEqual(row["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(row["fiat_value"], 600.0, places=4)
        self.assertEqual(row["fiat_price_source"], "import")

    def test_12_error_envelope_shape(self):
        # bad pair syntax (no hyphen) → validation error envelope
        payload, code = _run(
            self.data_root,
            "rates", "set", "BTCUSD", "2024-05-01T00:00:00Z", "65000",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload.get("schema_version"), 1)
        err = payload.get("error")
        self.assertIsInstance(err, dict)
        for field in ("code", "message", "hint", "details", "retryable"):
            self.assertIn(field, err)
        self.assertEqual(err["code"], "validation")

    def test_13_cross_wallet_intra_transfer(self):
        # New profile so the assertions don't tangle with prior tests.
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Transfer",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("Cold", "Hot"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "Transfer",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Cold",
            "--file", str(self.cold_transfer_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 2)

        payload = self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Hot",
            "--file", str(self.hot_transfer_csv),
        )
        self._assert_kind(payload, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "journals.process")
        data = payload["data"]
        # 1 acquisition (cold inbound) + 1 transfer_fee + 1 transfer_out + 1 transfer_in
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["entries_created"], 4)
        self.assertEqual(data["quarantined"], 0)
        self.assertEqual(data["processed_transactions"], 3)

        payload = self._cli(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "reports.journal-entries")
        entries = payload["data"]
        types = sorted(e["entry_type"] for e in entries)
        self.assertEqual(types, ["acquisition", "transfer_fee", "transfer_in", "transfer_out"])

        # The transfer_out / transfer_in pair must zero out across wallets.
        out_entry = next(e for e in entries if e["entry_type"] == "transfer_out")
        in_entry = next(e for e in entries if e["entry_type"] == "transfer_in")
        self.assertEqual(out_entry["wallet"], "Cold")
        self.assertEqual(in_entry["wallet"], "Hot")
        self.assertAlmostEqual(float(out_entry["quantity"]), -0.501, places=8)
        self.assertAlmostEqual(float(in_entry["quantity"]), 0.5, places=8)

        payload = self._cli(
            "journals", "transfers", "list",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "journals.transfers.list")
        audit = payload["data"]
        self.assertEqual(audit["summary"]["same_asset_transfers"], 1)
        self.assertEqual(audit["summary"]["cross_asset_pairs"], 0)
        transfer_row = audit["same_asset_transfers"][0]
        self.assertEqual(transfer_row["from_wallet"], "Cold")
        self.assertEqual(transfer_row["to_wallet"], "Hot")
        self.assertEqual(transfer_row["sent_msat"], 50100000000)
        self.assertEqual(transfer_row["received_msat"], 50000000000)
        self.assertEqual(transfer_row["fee_msat"], 100000000)

        # Only the 0.001 BTC network fee is realized as a taxable disposal.
        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        rows = payload["data"]
        self.assertEqual(len(rows), 1)
        gain_row = rows[0]
        self.assertEqual(gain_row["entry_type"], "transfer_fee")
        self.assertEqual(gain_row["wallet"], "Cold")
        self.assertAlmostEqual(float(gain_row["quantity"]), 0.001, places=8)
        self.assertAlmostEqual(float(gain_row["proceeds"]), 65.0, places=4)
        self.assertAlmostEqual(float(gain_row["cost_basis"]), 60.0, places=4)
        self.assertAlmostEqual(float(gain_row["gain_loss"]), 5.0, places=4)

        # Cost basis follows the moved coins to Hot, so both wallets show non-zero
        # holdings with positive average cost.
        payload = self._cli(
            "reports", "portfolio-summary",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        rows = {r["wallet"]: r for r in payload["data"]}
        self.assertEqual(set(rows), {"Cold", "Hot"})
        self.assertAlmostEqual(float(rows["Cold"]["quantity"]), 0.499, places=8)
        self.assertAlmostEqual(float(rows["Hot"]["quantity"]), 0.5, places=8)
        # Average cost is global ($59,940 / 0.999 BTC = $60,000) since the only
        # acquisition was at $60k.
        self.assertAlmostEqual(float(rows["Cold"]["avg_cost"]), 60000.0, places=2)
        self.assertAlmostEqual(float(rows["Hot"]["avg_cost"]), 60000.0, places=2)

        # Aggregate BTC across both wallets: 0.499 + 0.5 = 0.999 BTC.
        payload = self._cli(
            "reports", "balance-sheet",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        btc_rows = [r for r in payload["data"] if r.get("asset") == "BTC"]
        total_qty = sum(float(r["quantity"]) for r in btc_rows)
        self.assertAlmostEqual(total_qty, 0.999, places=8)

        payload = self._cli(
            "reports", "summary",
            "--workspace", "Main",
            "--profile", "Transfer",
            "--wallet", "Hot",
        )
        self._assert_kind(payload, "reports.summary")
        summary = payload["data"]
        self.assertEqual(summary["wallet"], "Hot")
        self.assertEqual(summary["metrics"]["wallets_in_scope"], 1)
        self.assertEqual(summary["metrics"]["active_transactions"], 1)
        self.assertEqual(summary["asset_flow"][0]["fee_amount_msat"], 0)

        payload = self._cli(
            "reports", "tax-summary",
            "--workspace", "Main",
            "--profile", "Transfer",
        )
        self._assert_kind(payload, "reports.tax-summary")
        rows = payload["data"]
        detail_rows = [row for row in rows if row["row_type"] == "detail"]
        self.assertEqual(len(detail_rows), 1)
        self.assertEqual(detail_rows[0]["transaction_type"], "move")
        self.assertEqual(detail_rows[0]["quantity_msat"], 100000000)
        self.assertAlmostEqual(float(detail_rows[0]["gain_loss"]), 5.0, places=4)
        grand_total = next(row for row in rows if row["row_type"] == "grand_total")
        self.assertEqual(grand_total["quantity_msat"], 100000000)
        self.assertAlmostEqual(float(grand_total["gain_loss"]), 5.0, places=4)

    def test_13a_intra_transfer_fiat_value_spot_price(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "TransferValueOnly",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("ColdValue", "HotValue"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "TransferValueOnly",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
            "--wallet", "ColdValue",
            "--file", str(self.cold_transfer_value_only_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
            "--wallet", "HotValue",
            "--file", str(self.hot_transfer_value_only_csv),
        )

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
        )
        self._assert_kind(payload, "journals.process")
        self.assertEqual(payload["data"]["transfers_detected"], 1)

        payload = self._cli(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "TransferValueOnly",
        )
        self._assert_kind(payload, "reports.capital-gains")
        rows = payload["data"]
        self.assertEqual(len(rows), 1)
        gain_row = rows[0]
        self.assertEqual(gain_row["entry_type"], "transfer_fee")
        self.assertAlmostEqual(float(gain_row["proceeds"]), 65.0, places=4)
        self.assertAlmostEqual(float(gain_row["cost_basis"]), 60.0, places=4)
        self.assertAlmostEqual(float(gain_row["gain_loss"]), 5.0, places=4)

    def test_13b_pair_by_shared_external_id(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "SharedTxid",
        )
        self._assert_kind(payload, "profiles.create")

        for label in ("ColdShared", "HotShared"):
            payload = self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "SharedTxid",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_kind(payload, "wallets.create")

        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--wallet", "ColdShared",
            "--file", str(self.cold_transfer_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--wallet", "HotShared",
            "--file", str(self.hot_transfer_csv),
        )

        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "SharedTxid",
            "--tx-out", "onchain-self-transfer-1",
            "--tx-in", "onchain-self-transfer-1",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        pair_id = payload["data"]["id"]
        self.assertNotEqual(payload["data"]["out_transaction_id"], payload["data"]["in_transaction_id"])

        payload = self._cli(
            "transfers", "list",
            "--workspace", "Main",
            "--profile", "SharedTxid",
        )
        self._assert_kind(payload, "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["id"], pair_id)
        self.assertEqual(payload["data"][0]["out"]["wallet"], "ColdShared")
        self.assertEqual(payload["data"][0]["in"]["wallet"], "HotShared")

    def test_14_manual_same_asset_pairing(self):
        # Auto-detection only fires when external_ids match. The two BTC legs
        # below deliberately have different external_ids; the user pairs them
        # explicitly so the journal pipeline still treats them as an
        # IntraTransaction.
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "ManualPair",
        )
        self._assert_kind(payload, "profiles.create")
        for label in ("From", "To"):
            self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "ManualPair",
                "--label", label,
                "--kind", "custom",
            )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--wallet", "From",
            "--file", str(self.manual_from_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--wallet", "To",
            "--file", str(self.manual_to_csv),
        )

        # Without a pair, processing books the outbound as a real disposal.
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        self.assertEqual(payload["data"]["transfers_detected"], 0)
        self.assertEqual(payload["data"]["cross_asset_pairs"], 0)

        payload, code = _run(
            self.data_root,
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--tx-out", "manual-out-leg",
            "--tx-in", "manual-in-leg",
            "--kind", "manual",
            "--policy", "taxable",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("Same-asset taxable", payload["error"]["message"])

        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--tx-out", "manual-out-leg",
            "--tx-in", "manual-in-leg",
            "--kind", "manual",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        pair_id = payload["data"]["id"]

        # Listing surfaces both legs with their wallets and assets.
        payload = self._cli("transfers", "list", "--workspace", "Main", "--profile", "ManualPair")
        self._assert_kind(payload, "transfers.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["out"]["wallet"], "From")
        self.assertEqual(payload["data"][0]["in"]["wallet"], "To")

        # Reprocessing now treats the pair as an IntraTransaction: only the
        # 0.0005 BTC fee is realized; the 0.1 BTC carries basis to the To wallet.
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        data = payload["data"]
        self.assertEqual(data["transfers_detected"], 1)
        self.assertEqual(data["cross_asset_pairs"], 0)
        # 1 acquisition + transfer_fee + transfer_out + transfer_in = 4 entries.
        self.assertEqual(data["entries_created"], 4)

        # Unpairing reverts behavior to a straight disposal on next process.
        payload = self._cli(
            "transfers", "unpair",
            "--workspace", "Main",
            "--profile", "ManualPair",
            "--pair-id", pair_id,
        )
        self._assert_kind(payload, "transfers.unpair")
        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "ManualPair",
        )
        self.assertEqual(payload["data"]["transfers_detected"], 0)

    def test_15_cross_asset_pair_policies(self):
        payload = self._cli(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "CrossAsset",
        )
        self._assert_kind(payload, "profiles.create")
        for label in ("OnchainBTC", "Liquid"):
            self._cli(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "CrossAsset",
                "--label", label,
                "--kind", "custom",
            )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "OnchainBTC",
            "--file", str(self.cross_btc_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "Liquid",
            "--file", str(self.cross_lbtc_csv),
        )

        # Carrying-value across BTC ↔ LBTC is not yet supported — the CLI must
        # reject the pair creation with a clear validation error envelope.
        payload, code = _run(
            self.data_root,
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--policy", "carrying-value",
        )
        self.assertNotEqual(code, 0)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("carrying-value", payload["error"]["message"])

        # Taxable cross-asset pair is accepted and surfaces in the envelope.
        payload = self._cli(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--kind", "peg-in",
            "--policy", "taxable",
        )
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["policy"], "taxable")
        self.assertEqual(payload["data"]["kind"], "peg-in")

        payload = self._cli(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        data = payload["data"]
        # Cross-asset taxable pair: legs processed independently as SELL+BUY,
        # so transfers_detected stays 0 and cross_asset_pairs reports 1.
        self.assertEqual(data["transfers_detected"], 0)
        self.assertEqual(data["cross_asset_pairs"], 1)

        payload = self._cli(
            "journals", "transfers", "list",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        self._assert_kind(payload, "journals.transfers.list")
        audit = payload["data"]
        self.assertEqual(audit["summary"]["same_asset_transfers"], 0)
        self.assertEqual(audit["summary"]["cross_asset_pairs"], 1)
        pair = audit["cross_asset_pairs"][0]
        self.assertEqual(pair["kind"], "peg-in")
        self.assertEqual(pair["policy"], "taxable")
        self.assertEqual(pair["out_wallet"], "OnchainBTC")
        self.assertEqual(pair["in_wallet"], "Liquid")

    def test_16_austrian_cross_asset_carrying_value_accepts_same_wallet(self):
        workspace = "CrossAssetAT"
        self._cli("init")
        payload = self._cli("workspaces", "create", workspace)
        self._assert_kind(payload, "workspaces.create")
        payload = self._cli(
            "profiles", "create",
            "--workspace", workspace,
            "--fiat-currency", "EUR",
            "--tax-country", "at",
            "CrossAssetProfile",
        )
        self._assert_kind(payload, "profiles.create")
        self._cli(
            "wallets", "create",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--label", "Unified",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--wallet", "Unified",
            "--file", str(self.cross_btc_at_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--wallet", "Unified",
            "--file", str(self.cross_lbtc_csv),
        )

        payload = self._cli(
            "transfers", "pair",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--kind", "peg-in",
            "--policy", "carrying-value",
        )
        self._assert_kind(payload, "transfers.pair")
        self.assertEqual(payload["data"]["policy"], "carrying-value")

        payload = self._cli(
            "journals", "process",
            "--workspace", workspace,
            "--profile", "CrossAssetProfile",
        )
        data = payload["data"]
        self.assertEqual(data["cross_asset_pairs"], 1)
        self.assertEqual(data["quarantined"], 0)


class AccountBucketBehaviorTest(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-account-buckets-")
        self.data_root = Path(self._tmp.name) / "data"
        self._cli("init")
        self._cli("workspaces", "create", "Buckets")
        self._cli(
            "profiles", "create",
            "--workspace", "Buckets",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Default",
        )

    def tearDown(self):
        self._tmp.cleanup()

    def _cli(self, *args):
        payload, code = _run(self.data_root, *args)
        if code != 0:
            self.fail(
                f"CLI exited {code} for {args!r}; envelope: {json.dumps(payload)[:400]}"
            )
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("data", payload)
        return payload

    def _cli_error(self, *args):
        payload, code = _run(self.data_root, *args)
        self.assertNotEqual(code, 0, f"CLI unexpectedly succeeded for {args!r}")
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload.get("schema_version"), 1)
        self.assertIn("error", payload)
        return payload

    def test_new_profiles_seed_only_the_default_reporting_bucket(self):
        payload = self._cli("accounts", "list", "--workspace", "Buckets", "--profile", "Default")
        rows = payload["data"]
        self.assertEqual([row["code"] for row in rows], ["treasury"])
        self.assertEqual(rows[0]["label"], "Treasury")
        self.assertEqual(rows[0]["account_type"], "asset")
        self.assertEqual(rows[0]["asset"], "BTC")

    def test_duplicate_account_label_is_ambiguous_but_code_still_resolves(self):
        for code in ("ops-a", "ops-b"):
            self._cli(
                "accounts", "create",
                "--workspace", "Buckets",
                "--profile", "Default",
                "--code", code,
                "--label", "Operations",
                "--type", "asset",
                "--asset", "BTC",
            )

        payload = self._cli_error(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Ambiguous Wallet",
            "--kind", "custom",
            "--account", "Operations",
        )
        error = payload["error"]
        self.assertEqual(error["code"], "validation")
        self.assertIn("ambiguous", error["message"])
        self.assertEqual(
            [match["code"] for match in error["details"]["matches"]],
            ["ops-a", "ops-b"],
        )

        payload = self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Operations Wallet",
            "--kind", "custom",
            "--account", "ops-a",
        )
        self.assertEqual(payload["data"]["account_code"], "ops-a")

    def test_balance_sheet_groups_holdings_by_wallet_bucket(self):
        events_csv = Path(self._tmp.name) / "events.csv"
        treasury_csv = Path(self._tmp.name) / "treasury.csv"
        events_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-01T10:00:00Z,events-in,inbound,BTC,0.02000000,0,50000,Event income\n",
            encoding="utf-8",
        )
        treasury_csv.write_text(
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2026-01-02T10:00:00Z,treasury-in,inbound,BTC,0.10000000,0,51000,Treasury receive\n",
            encoding="utf-8",
        )

        self._cli(
            "accounts", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--code", "events",
            "--label", "Events",
            "--type", "income",
            "--asset", "LBTC",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Events Wallet",
            "--kind", "custom",
            "--account", "events",
        )
        self._cli(
            "wallets", "create",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--label", "Treasury Wallet",
            "--kind", "custom",
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Events Wallet",
            "--file", str(events_csv),
        )
        self._cli(
            "wallets", "import-csv",
            "--workspace", "Buckets",
            "--profile", "Default",
            "--wallet", "Treasury Wallet",
            "--file", str(treasury_csv),
        )
        self._cli("journals", "process", "--workspace", "Buckets", "--profile", "Default")

        payload = self._cli("reports", "balance-sheet", "--workspace", "Buckets", "--profile", "Default")
        rows = {row["account"]: row for row in payload["data"]}
        self.assertEqual(set(rows), {"events", "treasury"})
        self.assertAlmostEqual(float(rows["events"]["quantity"]), 0.02, places=8)
        self.assertAlmostEqual(float(rows["treasury"]["quantity"]), 0.1, places=8)
        self.assertEqual(rows["events"]["asset"], "BTC")


if __name__ == "__main__":
    unittest.main()
