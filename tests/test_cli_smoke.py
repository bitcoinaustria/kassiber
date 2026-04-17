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
        (
            cls.sample_descriptor,
            cls.sample_change_descriptor,
            cls.sample_derivation_root,
            cls.sample_fingerprint,
        ) = _sample_descriptor_pair()

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

        payload = self._cli("status")
        self._assert_kind(payload, "status")
        auth = payload["data"].get("auth", {})
        self.assertEqual(auth.get("mode"), "local")
        self.assertTrue(auth.get("authenticated"))

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
        self.assertTrue(pdf_path.exists())
        self.assertGreater(pdf_path.stat().st_size, 1000)
        payload_bytes = pdf_path.read_bytes()
        header = payload_bytes[:8]
        self.assertTrue(header.startswith(b"%PDF-1.4"))
        self.assertIn(b"/MediaBox [0 0 842 595]", payload_bytes)

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


if __name__ == "__main__":
    unittest.main()
