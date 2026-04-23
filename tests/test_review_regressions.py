import json
import os
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
from argparse import Namespace
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlparse

from kassiber.cli.main import command_needs_db
from kassiber.cli.handlers import _audit_transaction_refs
from kassiber.core.engines import TaxEngineLedgerInputs, build_tax_engine
from kassiber.core.runtime import bootstrap_runtime, close_runtime
from kassiber.errors import AppError


ROOT = Path(__file__).resolve().parent.parent
FIXTURES = Path(__file__).resolve().parent / "fixtures"


_OLD_SCHEMA_SQL = """
PRAGMA foreign_keys = ON;

CREATE TABLE settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE workspaces (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE profiles (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    label TEXT NOT NULL,
    fiat_currency TEXT NOT NULL DEFAULT 'USD',
    tax_country TEXT NOT NULL DEFAULT 'generic',
    tax_long_term_days INTEGER NOT NULL DEFAULT 365,
    gains_algorithm TEXT NOT NULL DEFAULT 'FIFO',
    last_processed_at TEXT,
    last_processed_tx_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (workspace_id, label)
);

CREATE TABLE accounts (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    account_type TEXT NOT NULL,
    asset TEXT,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE wallets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    label TEXT NOT NULL,
    kind TEXT NOT NULL,
    config_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, label)
);

CREATE TABLE transactions (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    external_id TEXT,
    fingerprint TEXT NOT NULL UNIQUE,
    occurred_at TEXT NOT NULL,
    direction TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount REAL NOT NULL,
    fee REAL NOT NULL DEFAULT 0,
    fiat_currency TEXT,
    fiat_rate REAL,
    fiat_value REAL,
    kind TEXT,
    description TEXT,
    counterparty TEXT,
    note TEXT,
    excluded INTEGER NOT NULL DEFAULT 0,
    raw_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE tags (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    code TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    UNIQUE (profile_id, code)
);

CREATE TABLE transaction_tags (
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    tag_id TEXT NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (transaction_id, tag_id)
);

CREATE TABLE journal_entries (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    transaction_id TEXT NOT NULL REFERENCES transactions(id) ON DELETE CASCADE,
    wallet_id TEXT NOT NULL REFERENCES wallets(id) ON DELETE CASCADE,
    account_id TEXT REFERENCES accounts(id) ON DELETE SET NULL,
    occurred_at TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    quantity REAL NOT NULL,
    fiat_value REAL NOT NULL DEFAULT 0,
    unit_cost REAL NOT NULL DEFAULT 0,
    cost_basis REAL,
    proceeds REAL,
    gain_loss REAL,
    description TEXT,
    at_category TEXT,
    at_kennzahl INTEGER,
    created_at TEXT NOT NULL
);

CREATE TABLE journal_quarantines (
    transaction_id TEXT PRIMARY KEY REFERENCES transactions(id) ON DELETE CASCADE,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    detail_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);

CREATE TABLE bip329_labels (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    profile_id TEXT NOT NULL REFERENCES profiles(id) ON DELETE CASCADE,
    wallet_id TEXT REFERENCES wallets(id) ON DELETE SET NULL,
    record_type TEXT NOT NULL,
    ref TEXT NOT NULL,
    label TEXT,
    origin TEXT,
    spendable INTEGER,
    data_json TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL
);
"""

_FIXTURE_COLD_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,cold-funding-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,onchain-self-transfer-1,outbound,BTC,0.50000000,0.001,65000,Move to hot wallet
"""

_FIXTURE_HOT_TRANSFER_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-02-01T12:00:00Z,onchain-self-transfer-1,inbound,BTC,0.50000000,0,65000,Receive from cold wallet
"""

_CROSS_BTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-04-01T10:00:00Z,cross-fund-1,inbound,BTC,0.10010000,0,80000,BTC acquisition
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
    )

_MIXED_DISPOSALS_BTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,btc-buy,inbound,BTC,0.10000000,0,50000,BTC buy
2026-02-01T10:00:00Z,btc-sell,outbound,BTC,0.05000000,0,60000,BTC sell
"""

_MIXED_DISPOSALS_LBTC_CSV = """date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-05T10:00:00Z,lbtc-buy,inbound,LBTC,0.20000000,0,40000,LBTC buy
2026-02-05T10:00:00Z,lbtc-sell,outbound,LBTC,0.10000000,0,45000,LBTC sell
"""


def _json_decimal(value):
    if isinstance(value, Decimal):
        return float(value)
    return value


def _normalize_engine_entries(entries):
    return sorted(
        [
            {key: _json_decimal(value) for key, value in entry.items() if key != "id"}
            for entry in entries
        ],
        key=lambda row: (
            row["occurred_at"],
            row["entry_type"],
            row["wallet_id"],
            row.get("at_category", ""),
            row["description"],
        ),
    )


def _normalize_holdings(holdings, keys):
    rows = []
    for key, totals in holdings.items():
        row = {label: value for label, value in zip(keys, key)}
        row["quantity"] = _json_decimal(totals["quantity"])
        row["cost_basis"] = _json_decimal(totals["cost_basis"])
        rows.append(row)
    return sorted(rows, key=lambda row: tuple(row[label] for label in keys))


def _normalize_quarantines(quarantines):
    return sorted(
        [
            {
                "transaction_id": quarantine["transaction_id"],
                "workspace_id": quarantine["workspace_id"],
                "profile_id": quarantine["profile_id"],
                "reason": quarantine["reason"],
                "detail": json.loads(quarantine["detail_json"]),
            }
            for quarantine in quarantines
        ],
        key=lambda row: (row["transaction_id"], row["reason"]),
    )


def _normalize_intra_audit(rows):
    return sorted(
        [
            {
                key: _json_decimal(value)
                for key, value in row.items()
            }
            for row in rows
        ],
        key=lambda row: (row["occurred_at"], row["out_id"], row["in_id"]),
    )


class ReviewRegressionTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls._tmp = tempfile.TemporaryDirectory(prefix="kassiber-review-regressions-")
        cls.tmp_path = Path(cls._tmp.name)

    @classmethod
    def tearDownClass(cls):
        cls._tmp.cleanup()

    def setUp(self):
        self.case_dir = self.tmp_path / self.id().split(".")[-1]
        self.case_dir.mkdir(parents=True, exist_ok=True)
        self.data_root = self.case_dir / "data"

    def _run_cli(self, *args, machine=False, output=None, explicit_data_root=True, env=None, cwd=None):
        cmd = [sys.executable, "-m", "kassiber"]
        if explicit_data_root:
            cmd.extend(["--data-root", str(self.data_root)])
        if machine:
            cmd.append("--machine")
        if output is not None:
            cmd.extend(["--output", str(output)])
        cmd.extend(args)
        return subprocess.run(
            cmd,
            cwd=cwd or ROOT,
            capture_output=True,
            env=env,
            text=True,
            check=False,
        )

    def _run_json(self, *args, **run_kwargs):
        result = self._run_cli(*args, machine=True, **run_kwargs)
        stdout = result.stdout.strip()
        self.assertTrue(stdout, msg=f"No stdout for {args!r}; stderr={result.stderr!r}")
        try:
            payload = json.loads(stdout)
        except json.JSONDecodeError as exc:
            self.fail(f"stdout was not JSON for {args!r}: {stdout[:400]}")
        self.assertEqual(payload.get("schema_version"), 1)
        return payload, result

    def _assert_ok(self, payload, result, kind):
        self.assertEqual(result.returncode, 0, msg=f"{payload!r}")
        self.assertEqual(payload.get("kind"), kind)

    def _bootstrap_runtime_state(self, *, env_file=None, persist_bootstrap=False):
        args = Namespace(
            data_root=str(self.data_root),
            env_file=str(env_file) if env_file is not None else None,
            machine=True,
            format="json",
            debug=False,
        )
        runtime = bootstrap_runtime(args, needs_db=True, persist_bootstrap=persist_bootstrap)
        self.addCleanup(close_runtime, runtime)
        return runtime

    def _load_fixture(self, name):
        return json.loads((FIXTURES / name).read_text(encoding="utf-8"))

    def _bootstrap_wallet(self, label="Wallet", kind="phoenix"):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json("profiles", "create", "--workspace", "Main", "Default")
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", label,
            "--kind", kind,
        )
        self._assert_ok(payload, result, "wallets.create")

    def _bootstrap_profile(self, profile_label="Default"):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json("profiles", "create", "--workspace", "Main", profile_label)
        self._assert_ok(payload, result, "profiles.create")

    def _set_profile_tax_country(self, profile_label, tax_country):
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.execute(
            "UPDATE profiles SET tax_country = ? WHERE label = ?",
            (tax_country, profile_label),
        )
        conn.commit()
        conn.close()

    def _assert_austrian_policy(self, payload):
        """AT profiles inherit defaults from rp2's `AT` country plugin."""
        self.assertEqual(payload["data"]["tax_country"], "at")
        self.assertEqual(payload["data"]["fiat_currency"], "EUR")
        self.assertEqual(payload["data"]["gains_algorithm"], "MOVING_AVERAGE_AT")

    def _write_case_file(self, name, contents):
        path = self.case_dir / name
        path.write_text(contents, encoding="utf-8")
        return path

    def test_command_needs_db_skips_static_command_surfaces(self):
        self.assertFalse(command_needs_db(Namespace(command="backends", backends_command="kinds")))
        self.assertFalse(command_needs_db(Namespace(command="wallets", wallets_command="kinds")))
        self.assertTrue(command_needs_db(Namespace(command="status")))
        self.assertTrue(command_needs_db(Namespace(command="backends", backends_command="list")))
        self.assertTrue(command_needs_db(Namespace(command="backends", backends_command="get")))
        self.assertTrue(command_needs_db(Namespace(command="rates", rates_command="pairs")))

    def test_metadata_limit_errors_keep_cursor_hint(self):
        self._bootstrap_wallet()

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "1001",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("cursor-based pagination", payload["error"]["hint"])

        payload, result = self._run_json(
            "metadata", "bip329", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "1001",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertEqual(payload["error"]["hint"], "Use a smaller --limit; max page size is 1000.")

    def test_audit_transaction_refs_chunks_large_input(self):
        class _Cursor:
            def __init__(self, rows):
                self._rows = rows

            def fetchall(self):
                return self._rows

        class _Conn:
            def __init__(self):
                self.calls = []

            def execute(self, query, params):
                self.calls.append(len(params))
                rows = [
                    {
                        "id": tx_id,
                        "external_id": f"ext-{tx_id}",
                        "occurred_at": "2026-01-01T00:00:00Z",
                        "asset": "BTC",
                        "wallet": "Wallet",
                    }
                    for tx_id in params[1:]
                ]
                return _Cursor(rows)

        conn = _Conn()
        refs = _audit_transaction_refs(conn, "profile-1", [f"tx-{index}" for index in range(1200)])
        self.assertEqual(len(refs), 1200)
        self.assertEqual(conn.calls, [401, 401, 401])

    def test_reports_summary_plain_output_is_human_readable(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "BTC",
            "--kind", "custom",
        )
        self._assert_ok(payload, result, "wallets.create")

        btc_csv = self._write_case_file("summary-plain-btc.csv", _MIXED_DISPOSALS_BTC_CSV)
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTC",
            "--file", str(btc_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json("journals", "process", "--workspace", "Main", "--profile", "Default")
        self._assert_ok(payload, result, "journals.process")

        result = self._run_cli("reports", "summary", "--workspace", "Main", "--profile", "Default")
        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertIn("Kassiber Summary Report - Default", result.stdout)
        self.assertIn("Financial Summary", result.stdout)
        self.assertIn("Asset Flow", result.stdout)
        self.assertNotIn("{'wallets_in_scope':", result.stdout)
        self.assertNotIn("{'cost_basis':", result.stdout)

    def test_pdf_export_bundled_fonts_are_packaged_and_loadable(self):
        from kassiber import pdf_report

        for key in pdf_report.PDF_FONT_FILES:
            self.assertTrue(pdf_report._bundled_pdf_font_path(key).exists())

        os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")
        try:
            from PySide6.QtGui import QFontDatabase, QGuiApplication
        except ImportError as exc:
            self.skipTest(f"PySide6 QtGui unavailable in this environment: {exc}")

        _app = QGuiApplication.instance() or QGuiApplication(["kassiber-font-test"])
        families = pdf_report._load_bundled_pdf_font_families(QFontDatabase)

        self.assertIn("body", families)
        self.assertIn("mono", families)
        self.assertTrue(families["body"])
        self.assertTrue(families["mono"])

    def test_tax_summary_total_rows_leave_quantity_blank_for_mixed_assets(self):
        self._bootstrap_profile()
        for label in ("BTC", "LBTC"):
            payload, result = self._run_json(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "Default",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_ok(payload, result, "wallets.create")

        btc_csv = self._write_case_file("mixed-btc.csv", _MIXED_DISPOSALS_BTC_CSV)
        lbtc_csv = self._write_case_file("mixed-lbtc.csv", _MIXED_DISPOSALS_LBTC_CSV)
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTC",
            "--file", str(btc_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "LBTC",
            "--file", str(lbtc_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json("journals", "process", "--workspace", "Main", "--profile", "Default")
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json("reports", "tax-summary", "--workspace", "Main", "--profile", "Default")
        self._assert_ok(payload, result, "reports.tax-summary")
        rows = payload["data"]
        detail_rows = [row for row in rows if row["row_type"] == "detail"]
        self.assertEqual({row["asset"] for row in detail_rows}, {"BTC", "LBTC"})
        year_total = next(row for row in rows if row["row_type"] == "year_total")
        grand_total = next(row for row in rows if row["row_type"] == "grand_total")
        self.assertIsNone(year_total["quantity"])
        self.assertIsNone(year_total["quantity_msat"])
        self.assertIsNone(grand_total["quantity"])
        self.assertIsNone(grand_total["quantity_msat"])

    def test_accounts_create_and_wallet_binding(self):
        self._bootstrap_profile()

        payload, result = self._run_json(
            "accounts", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--code", "Cash Ops",
            "--label", "Cash Operations",
            "--type", "asset",
            "--asset", "btc",
        )
        self._assert_ok(payload, result, "accounts.create")
        self.assertEqual(payload["data"]["code"], "cash-ops")
        self.assertEqual(payload["data"]["label"], "Cash Operations")
        self.assertEqual(payload["data"]["account_type"], "asset")
        self.assertEqual(payload["data"]["asset"], "BTC")

        payload, result = self._run_json(
            "accounts", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "accounts.list")
        self.assertEqual(
            [row["code"] for row in payload["data"]],
            ["cash-ops", "external", "fees", "treasury"],
        )

        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "Ops",
            "--kind", "custom",
            "--account", "cash-ops",
        )
        self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "wallets.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["label"], "Ops")
        self.assertEqual(payload["data"][0]["account"], "cash-ops")

    def test_wallets_update_preserves_legacy_altbestand_config(self):
        self._bootstrap_wallet(label="LegacyWallet", kind="custom")

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        wallet = conn.execute(
            "SELECT id, config_json FROM wallets WHERE label = 'LegacyWallet'"
        ).fetchone()
        config = json.loads(wallet[1])
        config["altbestand"] = True
        conn.execute(
            "UPDATE wallets SET config_json = ? WHERE id = ?",
            (json.dumps(config, sort_keys=True), wallet[0]),
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "wallets", "get",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "LegacyWallet",
        )
        self._assert_ok(payload, result, "wallets.get")
        self.assertTrue(payload["data"]["config"]["altbestand"])

        payload, result = self._run_json(
            "wallets", "update",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "LegacyWallet",
            "--label", "LegacyWalletRenamed",
        )
        self._assert_ok(payload, result, "wallets.update")
        self.assertTrue(payload["data"]["config"]["altbestand"])

        conn = sqlite3.connect(db_path)
        stored = conn.execute(
            "SELECT config_json FROM wallets WHERE label = 'LegacyWalletRenamed'"
        ).fetchone()[0]
        conn.close()
        self.assertTrue(json.loads(stored)["altbestand"])

    def test_wallet_outputs_redact_descriptor_material_but_keep_state_flags(self):
        self._bootstrap_profile()
        descriptor, change_descriptor = _sample_descriptor_pair()

        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "Vault",
            "--kind",
            "descriptor",
            "--config",
            json.dumps({"mnemonic": "seed words", "api_key": "leak-me"}),
            "--descriptor",
            descriptor,
            "--change-descriptor",
            change_descriptor,
            "--gap-limit",
            "5",
        )
        self._assert_ok(payload, result, "wallets.create")
        self.assertTrue(payload["data"]["descriptor"])
        self.assertTrue(payload["data"]["change_descriptor"])
        self.assertEqual(payload["data"]["config"]["descriptor"], "[redacted]")
        self.assertEqual(payload["data"]["config"]["change_descriptor"], "[redacted]")
        self.assertNotIn("mnemonic", payload["data"]["config"])
        self.assertNotIn("api_key", payload["data"]["config"])
        self.assertNotIn("config_json", payload["data"])

        payload, result = self._run_json(
            "wallets",
            "get",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "Vault",
        )
        self._assert_ok(payload, result, "wallets.get")
        self.assertTrue(payload["data"]["descriptor"])
        self.assertTrue(payload["data"]["change_descriptor"])
        self.assertEqual(payload["data"]["descriptor_state"], "bitcoin:main")
        self.assertEqual(payload["data"]["config"]["descriptor"], "[redacted]")
        self.assertEqual(payload["data"]["config"]["change_descriptor"], "[redacted]")
        self.assertNotIn("mnemonic", payload["data"]["config"])
        self.assertNotIn("api_key", payload["data"]["config"])

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        stored = conn.execute(
            "SELECT config_json FROM wallets WHERE label = 'Vault'"
        ).fetchone()[0]
        conn.close()
        stored_config = json.loads(stored)
        self.assertEqual(stored_config["descriptor"], descriptor)
        self.assertEqual(stored_config["change_descriptor"], change_descriptor)

    def test_custom_env_file_backend_bootstrap_persists_into_db(self):
        env_file = self.case_dir / "custom-backends.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_ALPHA_KIND=esplora",
                    "KASSIBER_BACKEND_ALPHA_URL=https://alpha.example/api",
                    "KASSIBER_DEFAULT_BACKEND=alpha",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json(
            "--env-file", str(env_file), "init",
        )
        self._assert_ok(payload, result, "init")
        self.assertEqual(payload["data"]["env_file"], str(env_file))

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "get", "alpha",
        )
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["source"], "database")
        self.assertTrue(payload["data"]["is_default"])
        self.assertEqual(payload["data"]["url"], "https://alpha.example/api")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "create", "alpha",
            "--kind", "electrum",
            "--url", "ssl://alpha-override.example:50002",
            "--batch-size", "25",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "update", "alpha",
            "--kind", "electrum",
            "--url", "ssl://alpha-override.example:50002",
            "--batch-size", "25",
        )
        self._assert_ok(payload, result, "backends.update")
        self.assertEqual(payload["data"]["source"], "database")
        self.assertEqual(payload["data"]["url"], "ssl://alpha-override.example:50002")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "create", "benchdb",
            "--kind", "electrum",
            "--url", "ssl://bench.example:50002",
        )
        self._assert_ok(payload, result, "backends.create")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "list",
        )
        self._assert_ok(payload, result, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertEqual(rows["alpha"]["source"], "database")
        self.assertEqual(rows["alpha"]["url"], "ssl://alpha-override.example:50002")
        self.assertEqual(rows["alpha"]["default"], "yes")
        self.assertEqual(rows["benchdb"]["default"], "")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "set-default", "benchdb",
        )
        self._assert_ok(payload, result, "backends.set-default")
        self.assertEqual(payload["data"]["default_backend"], "benchdb")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "delete", "benchdb",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "clear-default",
        )
        self._assert_ok(payload, result, "backends.clear-default")
        self.assertEqual(payload["data"]["default_backend"], "alpha")
        self.assertTrue(payload["data"]["cleared"])

        env_file.unlink()

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "backends", "get", "alpha",
        )
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["source"], "database")
        self.assertEqual(payload["data"]["url"], "ssl://alpha-override.example:50002")

        payload, result = self._run_json(
            "--env-file", str(env_file),
            "status",
        )
        self._assert_ok(payload, result, "status")
        self.assertEqual(payload["data"]["default_backend"], "alpha")
        self.assertEqual(payload["data"]["env_file"], str(env_file))

    def test_bitcoinrpc_backend_bootstrap_persists_cookiefile_and_wallet_prefix(self):
        env_file = self.case_dir / "bitcoinrpc.env"
        cookie_file = self.case_dir / ".cookie"
        cookie_file.write_text("rpcuser:rpcpass\n", encoding="utf-8")
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc",
                    "KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332",
                    f"KASSIBER_BACKEND_CORE_COOKIEFILE={cookie_file}",
                    "KASSIBER_BACKEND_CORE_WALLETPREFIX=review-core",
                    "KASSIBER_DEFAULT_BACKEND=core",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "core")
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["kind"], "bitcoinrpc")
        self.assertEqual(payload["data"]["walletprefix"], "review-core")
        self.assertTrue(payload["data"]["has_cookiefile"])
        self.assertNotIn("cookiefile", payload["data"])
        self.assertTrue(payload["data"]["is_default"])

        env_file.unlink()

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "core")
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["kind"], "bitcoinrpc")
        self.assertEqual(payload["data"]["walletprefix"], "review-core")
        self.assertTrue(payload["data"]["has_cookiefile"])
        self.assertNotIn("cookiefile", payload["data"])
        self.assertTrue(payload["data"]["is_default"])

        runtime = self._bootstrap_runtime_state(env_file=env_file)
        backend = runtime.runtime_config["backends"]["core"]
        self.assertEqual(runtime.runtime_config["default_backend"], "core")
        self.assertEqual(backend["kind"], "bitcoinrpc")
        self.assertEqual(backend["cookiefile"], str(cookie_file))
        self.assertEqual(backend["walletprefix"], "review-core")

    def test_deleted_bootstrap_backend_stays_deleted_across_restarts(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json("backends", "delete", "fulcrum")
        self._assert_ok(payload, result, "backends.delete")
        self.assertTrue(payload["data"]["deleted"])

        payload, result = self._run_json("backends", "list")
        self._assert_ok(payload, result, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertNotIn("fulcrum", rows)

        runtime = self._bootstrap_runtime_state()
        self.assertNotIn("fulcrum", runtime.runtime_config["backends"])

        payload, result = self._run_json("backends", "get", "fulcrum")
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "not_found")

    def test_current_dotenv_backend_restores_deleted_name(self):
        env_file = self.case_dir / "restore-alpha.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_ALPHA_KIND=electrum",
                    "KASSIBER_BACKEND_ALPHA_URL=ssl://alpha.example:50002",
                    "KASSIBER_DEFAULT_BACKEND=alpha",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json("--env-file", str(env_file), "backends", "set-default", "mempool")
        self._assert_ok(payload, result, "backends.set-default")

        payload, result = self._run_json("--env-file", str(env_file), "backends", "delete", "alpha")
        self._assert_ok(payload, result, "backends.delete")
        self.assertTrue(payload["data"]["deleted"])

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        row = conn.execute("SELECT name FROM backends WHERE name = 'alpha'").fetchone()
        conn.close()
        self.assertIsNone(row)

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "alpha")
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["source"], str(env_file))

        conn = sqlite3.connect(db_path)
        row = conn.execute("SELECT name FROM backends WHERE name = 'alpha'").fetchone()
        conn.close()
        self.assertIsNone(row)

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")

        conn = sqlite3.connect(db_path)
        row = conn.execute("SELECT name FROM backends WHERE name = 'alpha'").fetchone()
        conn.close()
        self.assertIsNotNone(row)

    def test_process_environment_backend_override_wins_over_seeded_db_value(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json(
            "backends",
            "update",
            "mempool",
            "--url",
            "https://db.example/api",
            "--batch-size",
            "25",
        )
        self._assert_ok(payload, result, "backends.update")

        env = {
            **os.environ,
            "KASSIBER_BACKEND_MEMPOOL_URL": "https://env.example/api",
            "KASSIBER_DEFAULT_BACKEND": "fulcrum",
        }
        payload, result = self._run_json("backends", "get", "mempool", env=env)
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["url"], "https://env.example/api")
        self.assertEqual(payload["data"]["source"], "environment")

        with patch.dict(
            os.environ,
            {
                "KASSIBER_BACKEND_MEMPOOL_URL": "https://env.example/api",
                "KASSIBER_DEFAULT_BACKEND": "fulcrum",
            },
            clear=False,
        ):
            runtime = self._bootstrap_runtime_state()
            backend = runtime.runtime_config["backends"]["mempool"]
            self.assertEqual(runtime.runtime_config["default_backend"], "fulcrum")
            self.assertEqual(backend["url"], "https://env.example/api")
            self.assertEqual(backend["batch_size"], 25)

    def test_process_environment_default_can_target_seeded_sqlite_backend(self):
        env_file = self.case_dir / "bitcoincore.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_BITCOINCORE_KIND=bitcoinrpc",
                    "KASSIBER_BACKEND_BITCOINCORE_URL=http://127.0.0.1:8332",
                    "KASSIBER_DEFAULT_BACKEND=bitcoincore",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")

        env_file.unlink()

        with patch.dict(
            os.environ,
            {"KASSIBER_DEFAULT_BACKEND": "bitcoincore"},
            clear=False,
        ):
            runtime = self._bootstrap_runtime_state(env_file=env_file)
            self.assertEqual(runtime.runtime_config["default_backend"], "bitcoincore")
            self.assertIn("bitcoincore", runtime.runtime_config["backends"])
            self.assertEqual(
                runtime.runtime_config["backends"]["bitcoincore"]["source"],
                "database",
            )

    def test_process_only_backend_fields_do_not_block_bootstrap_seed(self):
        env_file = self.case_dir / "bitcoincore.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_BITCOINCORE_KIND=bitcoinrpc",
                    "KASSIBER_BACKEND_BITCOINCORE_URL=http://127.0.0.1:8332",
                    "KASSIBER_DEFAULT_BACKEND=bitcoincore",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        with patch.dict(
            os.environ,
            {"KASSIBER_BACKEND_BITCOINCORE_TIMEOUT": "45"},
            clear=False,
        ):
            runtime = self._bootstrap_runtime_state(env_file=env_file, persist_bootstrap=True)
            self.assertEqual(runtime.runtime_config["default_backend"], "bitcoincore")
            self.assertEqual(runtime.runtime_config["backends"]["bitcoincore"]["timeout"], "45")

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        backend_row = conn.execute(
            "SELECT name, timeout FROM backends WHERE name = 'bitcoincore'"
        ).fetchone()
        bootstrap_default = conn.execute(
            "SELECT value FROM settings WHERE key = 'bootstrap_default_backend'"
        ).fetchone()
        stored_default = conn.execute(
            "SELECT value FROM settings WHERE key = 'default_backend'"
        ).fetchone()
        conn.close()

        self.assertIsNotNone(backend_row)
        self.assertEqual(backend_row[0], "bitcoincore")
        self.assertIsNone(backend_row[1])
        self.assertEqual(bootstrap_default[0], "bitcoincore")
        self.assertEqual(stored_default[0], "bitcoincore")

    def test_read_only_backend_get_does_not_import_bootstrap_config_into_sqlite(self):
        env_file = self.case_dir / "readonly-alpha.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_ALPHA_KIND=electrum",
                    "KASSIBER_BACKEND_ALPHA_URL=ssl://alpha.example:50002",
                    "KASSIBER_DEFAULT_BACKEND=alpha",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "alpha")
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["source"], str(env_file))
        self.assertTrue(payload["data"]["is_default"])

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        backend_count = conn.execute("SELECT COUNT(*) FROM backends").fetchone()[0]
        default_backend = conn.execute(
            "SELECT value FROM settings WHERE key = 'default_backend'"
        ).fetchone()
        bootstrap_default = conn.execute(
            "SELECT value FROM settings WHERE key = 'bootstrap_default_backend'"
        ).fetchone()
        conn.close()
        self.assertEqual(backend_count, 0)
        self.assertIsNone(default_backend)
        self.assertIsNone(bootstrap_default)

    def test_electrum_insecure_backend_bootstrap_persists_into_runtime_config(self):
        env_file = self.case_dir / "electrum-insecure.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_ALPHA_KIND=electrum",
                    "KASSIBER_BACKEND_ALPHA_URL=ssl://alpha.example:50002",
                    "KASSIBER_BACKEND_ALPHA_INSECURE=1",
                    "KASSIBER_DEFAULT_BACKEND=alpha",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "alpha")
        self._assert_ok(payload, result, "backends.get")
        self.assertIs(payload["data"]["insecure"], True)
        self.assertTrue(payload["data"]["is_default"])

        env_file.unlink()

        runtime = self._bootstrap_runtime_state(env_file=env_file)
        backend = runtime.runtime_config["backends"]["alpha"]
        self.assertEqual(runtime.runtime_config["default_backend"], "alpha")
        self.assertIs(backend["insecure"], True)

    def test_backends_create_bitcoinrpc_supports_cookiefile_and_wallet_prefix(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        cookie_file = self.case_dir / ".cli-cookie"
        cookie_file.write_text("rpcuser:rpcpass\n", encoding="utf-8")

        payload, result = self._run_json(
            "backends",
            "create",
            "core",
            "--kind",
            "bitcoinrpc",
            "--url",
            "http://127.0.0.1:8332",
            "--cookiefile",
            str(cookie_file),
            "--wallet-prefix",
            "cli-core",
        )
        self._assert_ok(payload, result, "backends.create")
        self.assertEqual(payload["data"]["kind"], "bitcoinrpc")
        self.assertEqual(payload["data"]["walletprefix"], "cli-core")
        self.assertTrue(payload["data"]["has_cookiefile"])
        self.assertNotIn("cookiefile", payload["data"])

    def test_backends_update_clear_removes_stored_credentials(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        cookie_file = self.case_dir / ".clear-cookie"
        cookie_file.write_text("rpcuser:rpcpass\n", encoding="utf-8")

        payload, result = self._run_json(
            "backends",
            "create",
            "clear-core",
            "--kind",
            "bitcoinrpc",
            "--url",
            "http://127.0.0.1:8332",
            "--auth-header",
            "Bearer keep-me",
            "--token",
            "secret-token",
            "--cookiefile",
            str(cookie_file),
            "--username",
            "rpcuser",
            "--password",
            "rpcpass",
            "--wallet-prefix",
            "clear-core",
        )
        self._assert_ok(payload, result, "backends.create")

        payload, result = self._run_json(
            "backends",
            "update",
            "clear-core",
            "--clear",
            "auth-header",
            "--clear",
            "token",
            "--clear",
            "cookiefile",
            "--clear",
            "username",
            "--clear",
            "password",
            "--clear",
            "wallet-prefix",
        )
        self._assert_ok(payload, result, "backends.update")
        self.assertFalse(payload["data"]["has_auth_header"])
        self.assertFalse(payload["data"]["has_token"])
        self.assertFalse(payload["data"]["has_cookiefile"])
        self.assertFalse(payload["data"]["has_username"])
        self.assertFalse(payload["data"]["has_password"])
        self.assertNotIn("cookiefile", payload["data"])
        self.assertNotIn("username", payload["data"])
        self.assertNotIn("password", payload["data"])
        self.assertNotIn("walletprefix", payload["data"])

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        row = conn.execute(
            "SELECT auth_header, token, config_json FROM backends WHERE name = 'clear-core'"
        ).fetchone()
        conn.close()
        self.assertIsNone(row[0])
        self.assertIsNone(row[1])
        stored_config = json.loads(row[2])
        self.assertNotIn("cookiefile", stored_config)
        self.assertNotIn("username", stored_config)
        self.assertNotIn("password", stored_config)
        self.assertNotIn("walletprefix", stored_config)

    def test_backend_outputs_redact_secret_values_but_keep_presence_flags(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json(
            "backends",
            "create",
            "secure-core",
            "--kind",
            "bitcoinrpc",
            "--url",
            "http://rpcuser:rpcpass@127.0.0.1:8332/wallet/review?session=topsecret",
            "--username",
            "rpcuser",
            "--password",
            "rpcpass",
            "--auth-header",
            "Bearer secret-header",
            "--token",
            "secret-token",
        )
        self._assert_ok(payload, result, "backends.create")
        self.assertEqual(payload["data"]["url"], "http://<redacted>@127.0.0.1:8332/wallet/review")
        self.assertTrue(payload["data"]["has_auth_header"])
        self.assertTrue(payload["data"]["has_token"])
        self.assertTrue(payload["data"]["has_username"])
        self.assertTrue(payload["data"]["has_password"])
        self.assertNotIn("auth_header", payload["data"])
        self.assertNotIn("token", payload["data"])
        self.assertNotIn("username", payload["data"])
        self.assertNotIn("password", payload["data"])

        payload, result = self._run_json("backends", "get", "secure-core")
        self._assert_ok(payload, result, "backends.get")
        self.assertEqual(payload["data"]["url"], "http://<redacted>@127.0.0.1:8332/wallet/review")
        self.assertTrue(payload["data"]["has_auth_header"])
        self.assertTrue(payload["data"]["has_token"])
        self.assertTrue(payload["data"]["has_username"])
        self.assertTrue(payload["data"]["has_password"])
        self.assertNotIn("auth_header", payload["data"])
        self.assertNotIn("token", payload["data"])
        self.assertNotIn("username", payload["data"])
        self.assertNotIn("password", payload["data"])

        payload, result = self._run_json("backends", "list")
        self._assert_ok(payload, result, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertEqual(rows["secure-core"]["url"], "http://<redacted>@127.0.0.1:8332/wallet/review")
        self.assertTrue(rows["secure-core"]["has_auth_header"])
        self.assertTrue(rows["secure-core"]["has_token"])
        self.assertTrue(rows["secure-core"]["has_username"])
        self.assertTrue(rows["secure-core"]["has_password"])
        self.assertNotIn("auth_header", rows["secure-core"])
        self.assertNotIn("token", rows["secure-core"])

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        row = conn.execute(
            "SELECT url, auth_header, token, config_json FROM backends WHERE name = 'secure-core'"
        ).fetchone()
        conn.close()
        self.assertEqual(
            row[0],
            "http://rpcuser:rpcpass@127.0.0.1:8332/wallet/review?session=topsecret",
        )
        self.assertEqual(row[1], "Bearer secret-header")
        self.assertEqual(row[2], "secret-token")
        stored_config = json.loads(row[3])
        self.assertEqual(stored_config["username"], "rpcuser")
        self.assertEqual(stored_config["password"], "rpcpass")

    def test_backend_outputs_hide_alias_credentials_and_unknown_config(self):
        env_file = self.case_dir / "backend-aliases.env"
        env_file.write_text(
            "\n".join(
                [
                    "KASSIBER_BACKEND_CORE_KIND=bitcoinrpc",
                    "KASSIBER_BACKEND_CORE_URL=http://127.0.0.1:8332",
                    "KASSIBER_BACKEND_CORE_RPCUSER=rpcuser",
                    "KASSIBER_BACKEND_CORE_RPCPASSWORD=rpcpass",
                    "KASSIBER_BACKEND_CORE_API_KEY=leak-me",
                    "KASSIBER_DEFAULT_BACKEND=core",
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json("--env-file", str(env_file), "init")
        self._assert_ok(payload, result, "init")
        env_file.unlink()

        payload, result = self._run_json("--env-file", str(env_file), "backends", "get", "core")
        self._assert_ok(payload, result, "backends.get")
        self.assertTrue(payload["data"]["has_username"])
        self.assertTrue(payload["data"]["has_password"])
        self.assertNotIn("username", payload["data"])
        self.assertNotIn("password", payload["data"])
        self.assertNotIn("rpcuser", payload["data"])
        self.assertNotIn("rpcpassword", payload["data"])
        self.assertNotIn("api_key", payload["data"])

    def test_backends_delete_refuses_when_wallets_reference_backend(self):
        self._bootstrap_profile()

        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "Tracked",
            "--kind",
            "address",
            "--backend",
            "mempool",
            "--address",
            "bc1qar0srrr7xfkvy5l643lydnw9re59gtzzwf5mdq",
        )
        self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json("backends", "set-default", "fulcrum")
        self._assert_ok(payload, result, "backends.set-default")

        payload, result = self._run_json("backends", "delete", "mempool")
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")
        self.assertIn("Main/Default/Tracked", payload["error"]["hint"])

    def test_metadata_record_mutations_roundtrip_and_invalidate_journals(self):
        self._bootstrap_wallet(label="Meta")
        json_file = self.case_dir / "meta-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "fiat_rate": 40000,
                        "txid": "meta-tx-1",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Meta",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")

        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)

        payload, result = self._run_json(
            "profiles", "get",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertIsNotNone(payload["data"]["last_processed_at"])

        payload, result = self._run_json(
            "metadata", "records", "note", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "meta-tx-1",
            "--note", "Needs review",
        )
        self._assert_ok(payload, result, "metadata.records.note.set")
        self.assertEqual(payload["data"]["note"], "Needs review")

        payload, result = self._run_json(
            "profiles", "get",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertIsNone(payload["data"]["last_processed_at"])
        self.assertEqual(payload["data"]["last_processed_tx_count"], 0)

        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "metadata", "tags", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--code", "review",
            "--label", "Review",
        )
        self._assert_ok(payload, result, "metadata.tags.create")

        payload, result = self._run_json(
            "metadata", "records", "tag", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "meta-tx-1",
            "--tag", "review",
        )
        self._assert_ok(payload, result, "metadata.records.tag")
        self.assertEqual(payload["data"]["status"], "added")

        payload, result = self._run_json(
            "profiles", "get",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertIsNone(payload["data"]["last_processed_at"])

        payload, result = self._run_json(
            "metadata", "records", "get",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "meta-tx-1",
        )
        self._assert_ok(payload, result, "metadata.records.get")
        self.assertEqual(payload["data"]["note"], "Needs review")
        self.assertEqual(payload["data"]["tags"], [{"code": "review", "label": "Review"}])
        self.assertFalse(payload["data"]["excluded"])

        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "metadata", "records", "excluded", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "meta-tx-1",
        )
        self._assert_ok(payload, result, "metadata.records.excluded")
        self.assertTrue(payload["data"]["excluded"])

        payload, result = self._run_json(
            "profiles", "get",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertIsNone(payload["data"]["last_processed_at"])

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--excluded",
        )
        self._assert_ok(payload, result, "metadata.records.list")
        self.assertEqual([row["external_id"] for row in payload["data"]["records"]], ["meta-tx-1"])

    def test_wallets_sync_all_file_source_is_idempotent_and_reports_skips(self):
        self._bootstrap_profile()
        phoenix_csv = self.case_dir / "sync-phoenix.csv"
        phoenix_csv.write_text(
            "date,id,type,amount_msat,amount_fiat,fee_credit_msat,mining_fee_sat,mining_fee_fiat,service_fee_msat,service_fee_fiat,payment_hash,tx_id,destination,description\n"
            "2024-05-01T10:15:00Z,11111111-aaaa-bbbb-cccc-000000000001,swap_in,5000000000,2000 USD,0,250,0.10 USD,0,0 USD,,abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789,bc1qexamplefakedestination0000000000000000,Onchain deposit\n",
            encoding="utf-8",
        )

        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "SyncMe",
            "--kind", "phoenix",
            "--source-file", str(phoenix_csv),
            "--source-format", "phoenix_csv",
        )
        self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "SkipMe",
            "--kind", "custom",
        )
        self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets", "sync",
            "--workspace", "Main",
            "--profile", "Default",
            "--all",
        )
        self._assert_ok(payload, result, "wallets.sync")
        self.assertEqual(
            payload["data"],
            [
                {
                    "wallet": "SkipMe",
                    "status": "skipped",
                    "reason": "no file source, descriptor, or backend addresses configured",
                },
                {
                    "wallet": "SyncMe",
                    "status": "synced",
                    "source": "file:phoenix_csv",
                    "imported": 1,
                    "skipped": 0,
                    "phoenix_notes_set": 1,
                    "phoenix_tags_added": 1,
                    "phoenix_tags_created": 1,
                    "input_format": "phoenix_csv",
                    "file": str(phoenix_csv),
                },
            ],
        )

        payload, result = self._run_json(
            "wallets", "sync",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "SyncMe",
        )
        self._assert_ok(payload, result, "wallets.sync")
        self.assertEqual(
            payload["data"],
            [
                {
                    "wallet": "SyncMe",
                    "status": "synced",
                    "source": "file:phoenix_csv",
                    "imported": 0,
                    "skipped": 1,
                    "phoenix_notes_set": 0,
                    "phoenix_tags_added": 0,
                    "phoenix_tags_created": 0,
                    "input_format": "phoenix_csv",
                    "file": str(phoenix_csv),
                }
            ],
        )

        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["wallet"], "SyncMe")
        self.assertEqual(payload["data"][0]["external_id"], "11111111-aaaa-bbbb-cccc-000000000001")

    def test_context_switch_clears_stale_profile_and_supports_implicit_scope(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json("profiles", "create", "--workspace", "Main", "Default")
        self._assert_ok(payload, result, "profiles.create")

        payload, result = self._run_json("context", "show")
        self._assert_ok(payload, result, "context.show")
        self.assertEqual(payload["data"]["workspace_label"], "Main")
        self.assertEqual(payload["data"]["profile_label"], "Default")

        payload, result = self._run_json("workspaces", "create", "Alt")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json("context", "current")
        self._assert_ok(payload, result, "context.current")
        self.assertEqual(payload["data"]["workspace_label"], "Alt")
        self.assertEqual(payload["data"]["profile_label"], "")

        payload, result = self._run_json("profiles", "create", "--workspace", "Alt", "Treasury")
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json("context", "show")
        self._assert_ok(payload, result, "context.show")
        self.assertEqual(payload["data"]["workspace_label"], "Alt")
        self.assertEqual(payload["data"]["profile_label"], "Treasury")

        payload, result = self._run_json("context", "set", "--workspace", "Main")
        self._assert_ok(payload, result, "context.set")
        self.assertEqual(payload["data"]["workspace_label"], "Main")
        self.assertEqual(payload["data"]["profile_label"], "")

        payload, result = self._run_json("context", "set", "--workspace", "Main", "--profile", "Default")
        self._assert_ok(payload, result, "context.set")
        self.assertEqual(payload["data"]["workspace_label"], "Main")
        self.assertEqual(payload["data"]["profile_label"], "Default")

        payload, result = self._run_json("wallets", "create", "--label", "Scoped", "--kind", "custom")
        self._assert_ok(payload, result, "wallets.create")
        self.assertEqual(payload["data"]["label"], "Scoped")

    def test_metadata_records_cursor_roundtrip_and_invalid_cursor_error(self):
        self._bootstrap_wallet(label="CursorWallet")
        self._insert_transaction(
            wallet_label="CursorWallet",
            tx_id="tx-1",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="CursorWallet",
            tx_id="tx-2",
            occurred_at="2024-05-02T12:00:00Z",
            amount_msat=200_000_000,
        )
        self._insert_transaction(
            wallet_label="CursorWallet",
            tx_id="tx-3",
            occurred_at="2024-05-03T12:00:00Z",
            amount_msat=300_000_000,
        )

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "2",
        )
        self._assert_ok(payload, result, "metadata.records.list")
        first_page = payload["data"]
        self.assertEqual([row["transaction_id"] for row in first_page["records"]], ["tx-3", "tx-2"])
        self.assertTrue(first_page["has_more"])
        self.assertEqual(first_page["limit"], 2)
        self.assertTrue(first_page["next_cursor"])

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "2",
            "--cursor", first_page["next_cursor"],
        )
        self._assert_ok(payload, result, "metadata.records.list")
        second_page = payload["data"]
        self.assertEqual([row["transaction_id"] for row in second_page["records"]], ["tx-1"])
        self.assertFalse(second_page["has_more"])
        self.assertIsNone(second_page["next_cursor"])

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--cursor", "not-a-real-cursor",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("Pass the exact next_cursor value", payload["error"]["hint"])

    def test_journal_events_cursor_roundtrip(self):
        self._bootstrap_wallet(label="Events")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="Events",
            tx_id="event-1",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="Events",
            tx_id="event-2",
            occurred_at="2024-05-02T12:00:00Z",
            amount_msat=200_000_000,
        )
        self._insert_transaction(
            wallet_label="Events",
            tx_id="event-3",
            occurred_at="2024-05-03T12:00:00Z",
            amount_msat=300_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 3)
        self.assertEqual(payload["data"]["auto_priced"], 3)

        payload, result = self._run_json(
            "journals", "events", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "2",
        )
        self._assert_ok(payload, result, "journals.events.list")
        first_page = payload["data"]
        self.assertEqual([row["transaction_id"] for row in first_page["events"]], ["event-3", "event-2"])
        self.assertTrue(first_page["has_more"])
        self.assertEqual(first_page["limit"], 2)
        self.assertTrue(first_page["next_cursor"])

        payload, result = self._run_json(
            "journals", "events", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "2",
            "--cursor", first_page["next_cursor"],
        )
        self._assert_ok(payload, result, "journals.events.list")
        second_page = payload["data"]
        self.assertEqual([row["transaction_id"] for row in second_page["events"]], ["event-1"])
        self.assertFalse(second_page["has_more"])
        self.assertIsNone(second_page["next_cursor"])

    def test_bip329_import_list_export_bridges_transaction_tags(self):
        self._bootstrap_wallet(label="Labels")
        json_file = self.case_dir / "labels-wallet.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "demo-bip329-tx",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")

        bip329_file = self.case_dir / "labels.jsonl"
        bip329_file.write_text(
            "\n".join(
                [
                    json.dumps({"type": "tx", "ref": "demo-bip329-tx", "label": "merchant"}),
                    json.dumps(
                        {
                            "type": "output",
                            "ref": "demo-bip329-tx:0",
                            "label": "change",
                            "origin": "wallet",
                            "spendable": False,
                        }
                    ),
                ]
            )
            + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
            "--file", str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")
        self.assertEqual(payload["data"]["records"], 2)
        self.assertEqual(payload["data"]["imported"], 2)
        self.assertEqual(payload["data"]["updated"], 0)
        self.assertEqual(payload["data"]["transaction_tags_created"], 1)
        self.assertEqual(payload["data"]["transaction_tags_added"], 1)

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
        )
        self._assert_ok(payload, result, "metadata.records.list")
        self.assertEqual(len(payload["data"]["records"]), 1)
        self.assertEqual(payload["data"]["records"][0]["tags"], [{"code": "merchant", "label": "merchant"}])

        payload, result = self._run_json(
            "metadata", "bip329", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        rows = sorted(payload["data"], key=lambda row: (row["type"], row["ref"]))
        self.assertEqual(
            rows,
            [
                {
                    "type": "output",
                    "ref": "demo-bip329-tx:0",
                    "label": "change",
                    "origin": "wallet",
                    "spendable": "false",
                    "created_at": rows[0]["created_at"],
                },
                {
                    "type": "tx",
                    "ref": "demo-bip329-tx",
                    "label": "merchant",
                    "origin": "",
                    "spendable": "",
                    "created_at": rows[1]["created_at"],
                },
            ],
        )

        export_file = self.case_dir / "labels-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
            "--file", str(export_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        self.assertEqual(payload["data"]["exported"], 2)
        exported = [json.loads(line) for line in export_file.read_text(encoding="utf-8").splitlines() if line.strip()]
        self.assertEqual(
            exported,
            [
                {"type": "tx", "ref": "demo-bip329-tx", "label": "merchant"},
                {
                    "type": "output",
                    "ref": "demo-bip329-tx:0",
                    "label": "change",
                    "origin": "wallet",
                    "spendable": False,
                },
            ],
        )

    def _insert_transaction(self, *, wallet_label, tx_id, occurred_at, amount_msat, direction="inbound", asset="BTC"):
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        profile = conn.execute(
            "SELECT id, workspace_id, fiat_currency FROM profiles WHERE label = 'Default'"
        ).fetchone()
        wallet = conn.execute(
            "SELECT id FROM wallets WHERE label = ?",
            (wallet_label,),
        ).fetchone()
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, counterparty, note,
                excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                tx_id,
                f"fp-{tx_id}",
                occurred_at,
                direction,
                asset,
                amount_msat,
                0,
                profile["fiat_currency"],
                None,
                None,
                "deposit",
                tx_id,
                None,
                None,
                0,
                "{}",
                occurred_at,
            ),
        )
        conn.commit()
        conn.close()

    def _direct_transfer_engine_inputs(self):
        profile = {
            "id": "profile-transfer",
            "workspace_id": "workspace-main",
            "label": "FixtureTransfer",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs_by_id = {
            "wallet-cold": {
                "id": "wallet-cold",
                "label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-hot": {
                "id": "wallet-hot",
                "label": "Hot",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "cold-funding-1",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-01-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 60000,
                "fiat_value": 60000,
                "kind": "deposit",
                "description": "Cold acquisition",
                "note": None,
                "external_id": "cold-funding-1",
                "created_at": "2026-01-01T10:00:00Z",
            },
            {
                "id": "onchain-self-transfer-1-out",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-02-01T12:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 100_000_000,
                "fiat_rate": 65000,
                "fiat_value": 32500,
                "kind": "withdrawal",
                "description": "Move to hot wallet",
                "note": None,
                "external_id": "onchain-self-transfer-1",
                "created_at": "2026-02-01T12:00:00Z",
            },
            {
                "id": "onchain-self-transfer-1-in",
                "wallet_id": "wallet-hot",
                "wallet_label": "Hot",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-02-01T12:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 65000,
                "fiat_value": 32500,
                "kind": "deposit",
                "description": "Receive from cold wallet",
                "note": None,
                "external_id": "onchain-self-transfer-1",
                "created_at": "2026-02-01T12:00:00Z",
            },
        ]
        return profile, TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[],
        )

    def _direct_austrian_engine_inputs(self):
        profile = {
            "id": "profile-at",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrian",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 9223372036854775807,
            "gains_algorithm": "MOVING_AVERAGE_AT",
        }
        wallet_refs_by_id = {
            "wallet-a": {
                "id": "wallet-a",
                "label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-b": {
                "id": "wallet-b",
                "label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "alt-buy-1",
                "wallet_id": "wallet-a",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2020-05-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 9000,
                "fiat_value": 4500,
                "kind": "deposit",
                "description": "Vienna Alt buy",
                "note": None,
                "external_id": "alt-buy-1",
                "created_at": "2020-05-01T10:00:00Z",
            },
            {
                "id": "neu-buy-1",
                "wallet_id": "wallet-a",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2024-06-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 30000,
                "fiat_value": 30000,
                "kind": "deposit",
                "description": "Vienna Neu buy",
                "note": None,
                "external_id": "neu-buy-1",
                "created_at": "2024-06-01T10:00:00Z",
            },
            {
                "id": "neu-sell-1",
                "wallet_id": "wallet-a",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-01T09:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 30_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 15000,
                "kind": "withdrawal",
                "description": "Vienna Neu sell",
                "note": None,
                "external_id": "neu-sell-1",
                "created_at": "2025-03-01T09:00:00Z",
            },
        ]
        return profile, TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[],
        )

    def _direct_austrian_cross_asset_swap_inputs(self):
        profile = {
            "id": "profile-at",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianSwap",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 9223372036854775807,
            "gains_algorithm": "MOVING_AVERAGE_AT",
        }
        wallet_refs_by_id = {
            "wallet-a": {
                "id": "wallet-a",
                "label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-b": {
                "id": "wallet-b",
                "label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "neu-buy-1",
                "wallet_id": "wallet-a",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2024-06-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 30000,
                "fiat_value": 30000,
                "kind": "deposit",
                "description": "Vienna Neu buy",
                "note": None,
                "external_id": "neu-buy-1",
                "created_at": "2024-06-01T10:00:00Z",
            },
            {
                "id": "swap-out-1",
                "wallet_id": "wallet-a",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-01T09:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 25000,
                "kind": "withdrawal",
                "description": "Peg-out BTC->LBTC",
                "note": None,
                "external_id": "swap-1-out",
                "created_at": "2025-03-01T09:00:00Z",
            },
            {
                "id": "swap-in-1",
                "wallet_id": "wallet-b",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-01T09:00:00Z",
                "direction": "inbound",
                "asset": "LBTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 25000,
                "kind": "deposit",
                "description": "Peg-in BTC->LBTC",
                "note": None,
                "external_id": "swap-1-in",
                "created_at": "2025-03-01T09:00:00Z",
            },
        ]
        manual_pairs = [
            {
                "id": "mp-at-swap-1",
                "out_transaction_id": "swap-out-1",
                "in_transaction_id": "swap-in-1",
                "policy": "carrying-value",
                "kind": "swap",
            },
        ]
        return profile, TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pairs,
        )

    def _direct_missing_quarantine_engine_inputs(self):
        profile = {
            "id": "profile-missing-price",
            "workspace_id": "workspace-main",
            "label": "FixtureMissingPrice",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs_by_id = {
            "wallet-cold": {
                "id": "wallet-cold",
                "label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "priced-buy-1",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-03-01T09:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 60000,
                "fiat_value": 60000,
                "kind": "deposit",
                "description": "Seed BTC",
                "note": None,
                "external_id": "priced-buy-1",
                "created_at": "2026-03-01T09:00:00Z",
            },
            {
                "id": "unpriced-spend-1",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-03-02T09:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 10_000_000_000,
                "fee": 0,
                "fiat_rate": None,
                "fiat_value": None,
                "kind": "withdrawal",
                "description": "Unpriced spend",
                "note": None,
                "external_id": "unpriced-spend-1",
                "created_at": "2026-03-02T09:00:00Z",
            },
        ]
        return profile, TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[],
        )

    def _direct_cross_asset_pair_engine_inputs(self):
        profile = {
            "id": "profile-cross-asset",
            "workspace_id": "workspace-main",
            "label": "FixtureCrossAsset",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs_by_id = {
            "wallet-cold": {
                "id": "wallet-cold",
                "label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-liquid": {
                "id": "wallet-liquid",
                "label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "cross-fund-1",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-04-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 10_010_000_000,
                "fee": 0,
                "fiat_rate": 80000,
                "fiat_value": 8008,
                "kind": "buy",
                "description": "BTC acquisition",
                "note": None,
                "external_id": "cross-fund-1",
                "created_at": "2026-04-01T10:00:00Z",
            },
            {
                "id": "cross-out-leg",
                "wallet_id": "wallet-cold",
                "wallet_label": "Cold",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-04-15T10:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 10_000_000_000,
                "fee": 10_000_000,
                "fiat_rate": 82000,
                "fiat_value": 8200,
                "kind": "withdrawal",
                "description": "Peg-in to Liquid",
                "note": None,
                "external_id": "cross-out-leg",
                "created_at": "2026-04-15T10:00:00Z",
            },
            {
                "id": "cross-in-leg",
                "wallet_id": "wallet-liquid",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2026-04-15T10:30:00Z",
                "direction": "inbound",
                "asset": "LBTC",
                "amount": 10_000_000_000,
                "fee": 0,
                "fiat_rate": 82000,
                "fiat_value": 8200,
                "kind": "deposit",
                "description": "Peg-in receive",
                "note": None,
                "external_id": "cross-in-leg",
                "created_at": "2026-04-15T10:30:00Z",
            },
        ]
        return profile, TaxEngineLedgerInputs(
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[
                {
                    "id": "pair-cross-1",
                    "out_transaction_id": "cross-out-leg",
                    "in_transaction_id": "cross-in-leg",
                    "kind": "peg-in",
                    "policy": "taxable",
                },
            ],
        )

    def _direct_engine_snapshot(self, profile, inputs):
        state = build_tax_engine(profile).build_ledger_state(inputs)
        return {
            "entries": _normalize_engine_entries(state.entries),
            "quarantines": _normalize_quarantines(state.quarantines),
            "intra_audit": _normalize_intra_audit(state.intra_audit),
            "cross_asset_pairs": sorted(
                state.cross_asset_pairs,
                key=lambda row: (row["pair_id"], row["out_id"], row["in_id"]),
            ),
            "account_holdings": _normalize_holdings(
                state.account_holdings,
                ("account_id", "account_code", "account_label", "asset"),
            ),
            "wallet_holdings": _normalize_holdings(
                state.wallet_holdings,
                ("wallet_id", "wallet_label", "account_code", "asset"),
            ),
        }

    def test_btcpay_import_machine_mode_keeps_json_envelope(self):
        self._bootstrap_wallet(label="BTCPay")
        btcpay_csv = self.case_dir / "btcpay.csv"
        btcpay_csv.write_text(
            "TransactionId,Timestamp,Currency,Amount,Comment,Labels\n"
            "tx-1,2024-01-01T00:00:00Z,BTC,0.001 BTC,seeded,merchant\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPay",
            "--file", str(btcpay_csv),
            "--format", "csv",
        )
        self._assert_ok(payload, result, "wallets.import-btcpay")
        self.assertEqual(payload["data"]["input_format"], "btcpay_csv")
        self.assertEqual(payload["data"]["imported"], 1)

    def test_btcpay_import_json_accepts_object_label_shape(self):
        self._bootstrap_wallet(label="BTCPayJSON")
        btcpay_json = self.case_dir / "btcpay.json"
        btcpay_json.write_text(
            json.dumps(
                [
                    {
                        "TransactionId": "tx-json-1",
                        "Timestamp": "2024-01-01T00:00:00Z",
                        "Currency": "BTC",
                        "Amount": "0.001 BTC",
                        "Comment": "seeded",
                        "Labels": {
                            "merchant": {"type": "invoice", "text": "merchant"},
                            "fallback-label": {},
                        },
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets",
            "import-btcpay",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "BTCPayJSON",
            "--file",
            str(btcpay_json),
            "--format",
            "json",
        )
        self._assert_ok(payload, result, "wallets.import-btcpay")
        self.assertEqual(payload["data"]["input_format"], "btcpay_json")
        self.assertEqual(payload["data"]["imported"], 1)
        self.assertEqual(payload["data"]["btcpay_notes_set"], 1)
        self.assertEqual(payload["data"]["btcpay_tags_added"], 2)
        self.assertEqual(payload["data"]["btcpay_tags_created"], 2)

    def test_btcpay_sync_greenfield_api_pages_and_sets_metadata(self):
        page_one = [
            {
                "transactionHash": "tx-remote-1",
                "comment": "remote deposit",
                "amount": "0.001",
                "timestamp": 1704067200,
                "status": "Confirmed",
                "confirmations": 6,
                "labels": {
                    "label-1": {"type": "invoice", "text": "merchant"},
                },
            },
            {
                "transactionHash": "tx-mempool-ignored",
                "comment": "should stay out",
                "amount": "-0.0002",
                "timestamp": 1704153600,
                "status": "Unconfirmed",
                "confirmations": 0,
                "labels": [],
            },
        ]
        page_two = [
            {
                "transactionHash": "tx-remote-2",
                "comment": "",
                "amount": "-0.0005",
                "timestamp": 1704240000,
                "status": "Confirmed",
                "confirmations": 2,
                "labels": [],
            },
        ]
        received = {"paths": [], "auth": []}

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                received["paths"].append(self.path)
                received["auth"].append(self.headers.get("Authorization"))
                parsed = urlparse(self.path)
                if self.headers.get("Authorization") != "token testkey":
                    self.send_error(401, "unauthorized")
                    return
                expected = "/api/v1/stores/STORE1/payment-methods/BTC-CHAIN/wallet/transactions"
                if parsed.path != expected:
                    self.send_error(404, "not found")
                    return
                skip = int(parse_qs(parsed.query).get("skip", ["0"])[0])
                body = json.dumps(page_one if skip == 0 else page_two if skip == 2 else []).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        port = server.server_address[1]
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        try:
            self._bootstrap_wallet(label="BTCPayRemote", kind="custom")
            payload, result = self._run_json(
                "backends", "create",
                "btcpay1",
                "--kind", "btcpay",
                "--url", f"http://127.0.0.1:{port}",
                "--token", "testkey",
            )
            self._assert_ok(payload, result, "backends.create")
            payload, result = self._run_json(
                "wallets", "sync-btcpay",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayRemote",
                "--backend", "btcpay1",
                "--store-id", "STORE1",
                "--page-size", "2",
            )
            self._assert_ok(payload, result, "wallets.sync-btcpay")
            data = payload["data"]
            self.assertEqual(data["imported"], 2)
            self.assertEqual(data["fetched"], 2)
            self.assertEqual(data["store_id"], "STORE1")
            self.assertEqual(data["payment_method_id"], "BTC-CHAIN")
            self.assertEqual(data["backend"], "btcpay1")
            self.assertEqual(data["backend_kind"], "btcpay")
            self.assertEqual(data["btcpay_notes_set"], 1)
            self.assertEqual(data["btcpay_tags_added"], 1)
            self.assertEqual(data["btcpay_tags_created"], 1)
            self.assertTrue(any("skip=0" in path and "limit=2" in path for path in received["paths"]))
            self.assertTrue(any("skip=2" in path for path in received["paths"]))
            self.assertTrue(all("statusFilter=Confirmed" in path for path in received["paths"]))
            for auth in received["auth"]:
                self.assertEqual(auth, "token testkey")
        finally:
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=5)

    def test_btcpay_sync_persists_wallet_config_and_resyncs_via_wallets_sync(self):
        page_one = [
            {
                "transactionHash": "tx-remote-1",
                "comment": "paid order 100",
                "amount": "0.0012",
                "timestamp": 1704067200,
                "status": "Confirmed",
                "confirmations": 6,
                "labels": ["merchant"],
            },
            {
                "transactionHash": "tx-mempool-ignored",
                "comment": "should stay out",
                "amount": "-0.0002",
                "timestamp": 1704153600,
                "status": "Unconfirmed",
                "confirmations": 0,
                "labels": [],
            },
        ]
        page_two = [
            {
                "transactionHash": "tx-remote-2",
                "comment": "",
                "amount": "-0.0005",
                "timestamp": 1704240000,
                "status": "Confirmed",
                "confirmations": 2,
                "labels": [],
            },
        ]
        received = {"paths": [], "auth": []}

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                received["paths"].append(self.path)
                received["auth"].append(self.headers.get("Authorization"))
                parsed = urlparse(self.path)
                if self.headers.get("Authorization") != "token testkey":
                    self.send_error(401, "unauthorized")
                    return
                expected = "/api/v1/stores/STORE1/payment-methods/BTC-CHAIN/wallet/transactions"
                if parsed.path != expected:
                    self.send_error(404, "not found")
                    return
                query = parse_qs(parsed.query)
                skip = int(query.get("skip", ["0"])[0])
                limit = int(query.get("limit", ["100"])[0])
                if skip == 0 and limit >= 3:
                    page = page_one + page_two
                elif skip == 0:
                    page = page_one
                elif skip == 2:
                    page = page_two
                else:
                    page = []
                body = json.dumps(page).encode("utf-8")
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *_args):
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        port = server.server_address[1]
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        try:
            self._bootstrap_wallet(label="BTCPayRemote", kind="custom")
            payload, result = self._run_json(
                "backends", "create",
                "btcpay1",
                "--kind", "btcpay",
                "--url", f"http://127.0.0.1:{port}",
                "--token", "testkey",
            )
            self._assert_ok(payload, result, "backends.create")

            payload, result = self._run_json(
                "wallets", "sync-btcpay",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayRemote",
                "--backend", "btcpay1",
                "--store-id", "STORE1",
                "--page-size", "2",
            )
            self._assert_ok(payload, result, "wallets.sync-btcpay")
            data = payload["data"]
            self.assertEqual(data["imported"], 2)
            self.assertEqual(data["fetched"], 2)
            self.assertEqual(data["store_id"], "STORE1")
            self.assertEqual(data["payment_method_id"], "BTC-CHAIN")
            self.assertEqual(data["backend"], "btcpay1")
            self.assertEqual(data["backend_kind"], "btcpay")
            self.assertEqual(data["btcpay_notes_set"], 1)
            self.assertEqual(data["btcpay_tags_added"], 1)
            self.assertEqual(data["btcpay_tags_created"], 1)

            payload, result = self._run_json(
                "wallets", "get",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayRemote",
            )
            self._assert_ok(payload, result, "wallets.get")
            self.assertEqual(
                payload["data"]["config"],
                {
                    "backend": "btcpay1",
                    "sync_source": "btcpay",
                    "store_id": "STORE1",
                    "payment_method_id": "BTC-CHAIN",
                },
            )

            conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
            stored_config = json.loads(
                conn.execute(
                    "SELECT config_json FROM wallets WHERE label = 'BTCPayRemote'"
                ).fetchone()[0]
            )
            conn.close()
            self.assertEqual(stored_config["backend"], "btcpay1")
            self.assertEqual(stored_config["sync_source"], "btcpay")
            self.assertEqual(stored_config["store_id"], "STORE1")
            self.assertEqual(stored_config["payment_method_id"], "BTC-CHAIN")

            payload, result = self._run_json(
                "wallets", "sync",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayRemote",
            )
            self._assert_ok(payload, result, "wallets.sync")
            self.assertEqual(len(payload["data"]), 1)
            single_sync = payload["data"][0]
            self.assertEqual(single_sync["wallet"], "BTCPayRemote")
            self.assertEqual(single_sync["status"], "synced")
            self.assertEqual(single_sync["imported"], 0)
            self.assertEqual(single_sync["skipped"], 2)
            self.assertEqual(single_sync["fetched"], 2)
            self.assertEqual(single_sync["store_id"], "STORE1")
            self.assertEqual(single_sync["payment_method_id"], "BTC-CHAIN")
            self.assertEqual(single_sync["backend"], "btcpay1")
            self.assertEqual(single_sync["backend_kind"], "btcpay")

            payload, result = self._run_json(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "Default",
                "--label", "SkipMe",
                "--kind", "custom",
            )
            self._assert_ok(payload, result, "wallets.create")

            payload, result = self._run_json(
                "wallets", "sync",
                "--workspace", "Main",
                "--profile", "Default",
                "--all",
            )
            self._assert_ok(payload, result, "wallets.sync")
            rows = {row["wallet"]: row for row in payload["data"]}
            self.assertEqual(rows["BTCPayRemote"]["status"], "synced")
            self.assertEqual(rows["BTCPayRemote"]["imported"], 0)
            self.assertEqual(rows["BTCPayRemote"]["skipped"], 2)
            self.assertEqual(rows["BTCPayRemote"]["fetched"], 2)
            self.assertEqual(rows["BTCPayRemote"]["backend_kind"], "btcpay")
            self.assertEqual(
                rows["SkipMe"],
                {
                    "wallet": "SkipMe",
                    "status": "skipped",
                    "reason": "no file source, descriptor, or backend addresses configured",
                },
            )

            self.assertTrue(any("skip=0" in path and "limit=2" in path for path in received["paths"]))
            self.assertTrue(any("skip=2" in path for path in received["paths"]))
            self.assertTrue(any("limit=100" in path for path in received["paths"]))
            self.assertTrue(all("statusFilter=Confirmed" in path for path in received["paths"]))
            for auth in received["auth"]:
                self.assertEqual(auth, "token testkey")
        finally:
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=5)

    def test_btcpay_sync_requires_explicit_marker_not_generic_config_keys(self):
        self._bootstrap_profile()
        generic_config = json.dumps(
            {
                "backend": "btcpay1",
                "store_id": "STORE1",
                "payment_method_id": "BTC-CHAIN",
            }
        )
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "GenericKeys",
            "--kind", "custom",
            "--config", generic_config,
        )
        self._assert_ok(payload, result, "wallets.create")
        self.assertNotIn("sync_source", payload["data"]["config"])

        payload, result = self._run_json(
            "wallets", "sync",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "GenericKeys",
        )
        self._assert_ok(payload, result, "wallets.sync")
        self.assertEqual(
            payload["data"],
            [
                {
                    "wallet": "GenericKeys",
                    "status": "skipped",
                    "reason": "no file source, descriptor, or backend addresses configured",
                }
            ],
        )

        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "BTCPayFlags",
            "--kind", "custom",
            "--backend", "btcpay1",
            "--store-id", "STORE2",
        )
        self._assert_ok(payload, result, "wallets.create")
        self.assertEqual(
            payload["data"]["config"],
            {
                "backend": "btcpay1",
                "sync_source": "btcpay",
                "store_id": "STORE2",
                "payment_method_id": "BTC-CHAIN",
            },
        )

        payload, result = self._run_json(
            "wallets", "update",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "GenericKeys",
            "--store-id", "STORE3",
        )
        self._assert_ok(payload, result, "wallets.update")
        self.assertEqual(
            payload["data"]["config"],
            {
                "backend": "btcpay1",
                "sync_source": "btcpay",
                "store_id": "STORE3",
                "payment_method_id": "BTC-CHAIN",
            },
        )

    def test_btcpay_sync_rejects_explicit_blank_payment_method_id(self):
        self._bootstrap_wallet(label="BTCPayBlank", kind="custom")
        payload, result = self._run_json(
            "backends", "create",
            "btcpay1",
            "--kind", "btcpay",
            "--url", "http://127.0.0.1:9",
            "--token", "testkey",
        )
        self._assert_ok(payload, result, "backends.create")

        payload, result = self._run_json(
            "wallets", "sync-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPayBlank",
            "--backend", "btcpay1",
            "--store-id", "STORE1",
            "--payment-method-id", "",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("payment method id cannot be empty", payload["error"]["message"])

        payload, result = self._run_json(
            "wallets", "get",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPayBlank",
        )
        self._assert_ok(payload, result, "wallets.get")
        self.assertEqual(payload["data"]["config"], {})

    def test_btcpay_sync_surfaces_auth_failure_envelope(self):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_error(403, "forbidden")

            def log_message(self, *_args):
                return

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        port = server.server_address[1]
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        try:
            self._bootstrap_wallet(label="BTCPayDenied", kind="custom")
            payload, result = self._run_json(
                "backends", "create",
                "btcpay2",
                "--kind", "btcpay",
                "--url", f"http://127.0.0.1:{port}",
                "--token", "stale",
            )
            self._assert_ok(payload, result, "backends.create")
            payload, result = self._run_json(
                "wallets", "sync-btcpay",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayDenied",
                "--backend", "btcpay2",
                "--store-id", "STORE1",
            )
            self.assertNotEqual(result.returncode, 0)
            self.assertEqual(payload["kind"], "error")
            self.assertEqual(payload["error"]["code"], "auth_error")

            payload, result = self._run_json(
                "wallets", "get",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "BTCPayDenied",
            )
            self._assert_ok(payload, result, "wallets.get")
            self.assertEqual(
                payload["data"]["config"],
                {
                    "backend": "btcpay2",
                    "sync_source": "btcpay",
                    "store_id": "STORE1",
                    "payment_method_id": "BTC-CHAIN",
                },
            )
        finally:
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=5)

    def test_transactions_list_returns_btc_and_msat_fields(self):
        self._bootstrap_wallet(label="One")
        json_file = self.case_dir / "import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "demo",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "One",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(len(payload["data"]), 1)
        record = payload["data"][0]
        self.assertAlmostEqual(record["amount"], 0.001, places=12)
        self.assertEqual(record["amount_msat"], 100_000_000)
        self.assertEqual(record["fee_msat"], 0)

    def test_rates_latest_prefers_manual_override_at_same_timestamp(self):
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000", "--source", "coingecko"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000", "--source", "manual"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json("rates", "latest", "BTC-USD")
        self._assert_ok(payload, result, "rates.latest")
        self.assertEqual(payload["data"]["source"], "manual")
        self.assertAlmostEqual(payload["data"]["rate"], 65000.0, places=4)

    def test_journals_process_autopriced_from_cached_rate(self):
        self._bootstrap_wallet(label="CacheA")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheA",
            tx_id="cache-a",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-a'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 600.0, places=4)

    def test_journals_process_misses_future_only_rate(self):
        self._bootstrap_wallet(label="CacheFuture")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-02T00:00:00Z", "70000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheFuture",
            tx_id="cache-future",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 0)
        self.assertEqual(payload["data"]["quarantined"], 1)
        self.assertEqual(payload["data"]["auto_priced"], 0)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-future'"
        ).fetchone()
        quarantine = conn.execute(
            "SELECT reason FROM journal_quarantines WHERE transaction_id = (SELECT id FROM transactions WHERE external_id = 'cache-future')"
        ).fetchone()
        conn.close()
        self.assertIsNone(tx["fiat_rate"])
        self.assertIsNone(tx["fiat_value"])
        self.assertEqual(quarantine["reason"], "missing_spot_price")

    def test_journals_process_prefers_manual_rate_same_timestamp(self):
        self._bootstrap_wallet(label="CacheManual")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000", "--source", "coingecko"
        )
        self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "65000", "--source", "manual"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CacheManual",
            tx_id="cache-manual",
            occurred_at="2024-05-01T00:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-manual'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 65000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 650.0, places=4)

    def test_journals_process_autoprices_lbtc_from_btc_rate(self):
        self._bootstrap_wallet(label="LiquidLike")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="LiquidLike",
            tx_id="cache-lbtc",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
            asset="LBTC",
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)
        self.assertEqual(payload["data"]["auto_priced"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'cache-lbtc'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 60000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 600.0, places=4)

    def test_quarantine_price_override_sets_missing_price_fields(self):
        self._bootstrap_wallet(label="OverrideMe")
        self._insert_transaction(
            wallet_label="OverrideMe",
            tx_id="override-demo",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["quarantined"], 1)

        payload, result = self._run_json(
            "journals", "quarantine", "resolve", "price-override",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "override-demo",
            "--fiat-rate", "61000",
        )
        self._assert_ok(payload, result, "journals.quarantine.resolve.price-override")
        self.assertAlmostEqual(payload["data"]["fiat_rate"], 61000.0, places=4)
        self.assertAlmostEqual(payload["data"]["fiat_value"], 610.0, places=4)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT fiat_rate, fiat_value FROM transactions WHERE external_id = 'override-demo'"
        ).fetchone()
        quarantine = conn.execute(
            "SELECT COUNT(*) AS n FROM journal_quarantines WHERE transaction_id = (SELECT id FROM transactions WHERE external_id = 'override-demo')"
        ).fetchone()
        profile = conn.execute(
            "SELECT last_processed_at, last_processed_tx_count FROM profiles WHERE label = 'Default'"
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 61000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 610.0, places=4)
        self.assertEqual(quarantine["n"], 0)
        self.assertIsNone(profile["last_processed_at"])
        self.assertEqual(profile["last_processed_tx_count"], 0)

        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["quarantined"], 0)

    def test_direct_generic_rp2_missing_price_quarantine_snapshot_matches_fixture(self):
        profile, inputs = self._direct_missing_quarantine_engine_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("generic_rp2_missing_price_quarantine_snapshot.json")
        self.assertEqual(actual, expected)

    def test_direct_generic_rp2_cross_asset_pair_snapshot_matches_fixture(self):
        profile, inputs = self._direct_cross_asset_pair_engine_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("generic_rp2_cross_asset_pair_snapshot.json")
        self.assertEqual(actual, expected)

    def test_generic_rp2_transfer_snapshot_matches_fixture(self):
        payload, result = self._run_json(
            "init",
        )
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "FixtureTransfer",
        )
        self._assert_ok(payload, result, "profiles.create")

        cold_csv = self.case_dir / "fixture-cold.csv"
        hot_csv = self.case_dir / "fixture-hot.csv"
        cold_csv.write_text(_FIXTURE_COLD_TRANSFER_CSV, encoding="utf-8")
        hot_csv.write_text(_FIXTURE_HOT_TRANSFER_CSV, encoding="utf-8")

        for label in ("Cold", "Hot"):
            payload, result = self._run_json(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "FixtureTransfer",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--wallet", "Cold",
            "--file", str(cold_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--wallet", "Hot",
            "--file", str(hot_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")

        summary, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
        )
        self._assert_ok(summary, result, "journals.process")

        journal_entries, result = self._run_json(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
        )
        self._assert_ok(journal_entries, result, "reports.journal-entries")

        capital_gains, result = self._run_json(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
        )
        self._assert_ok(capital_gains, result, "reports.capital-gains")

        portfolio_summary, result = self._run_json(
            "reports", "portfolio-summary",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
        )
        self._assert_ok(portfolio_summary, result, "reports.portfolio-summary")

        actual = {
            "summary": {
                key: value
                for key, value in summary["data"].items()
                if key not in {"processed_at", "profile"}
            },
            "journal_entries": sorted(
                [
                    {key: value for key, value in row.items() if key != "id"}
                    for row in journal_entries["data"]
                ],
                key=lambda row: (row["occurred_at"], row["entry_type"], row["wallet"], row["description"]),
            ),
            "capital_gains": sorted(
                [
                    {key: value for key, value in row.items() if key != "transaction_id"}
                    for row in capital_gains["data"]
                ],
                key=lambda row: (row["occurred_at"], row["entry_type"], row["wallet"]),
            ),
            "portfolio_summary": sorted(
                portfolio_summary["data"],
                key=lambda row: row["wallet"],
            ),
        }
        expected = self._load_fixture("generic_rp2_transfer_snapshot.json")
        self.assertEqual(actual, expected)

    def test_generic_rp2_engine_snapshot_matches_fixture(self):
        profile, inputs = self._direct_transfer_engine_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("generic_rp2_engine_snapshot.json")
        self.assertEqual(actual, expected)

    def test_austrian_rp2_engine_snapshot_matches_fixture(self):
        """End-to-end: AT profile produces rp2-AT-marked notes, moving-average disposal math, and Alt/Neu classification."""
        profile, inputs = self._direct_austrian_engine_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("austrian_rp2_engine_snapshot.json")
        self.assertEqual(actual, expected)

    def test_austrian_rp2_engine_emits_income_entry_for_staking_receipt(self):
        profile = {
            "id": "profile-at-income",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianIncome",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 365,
            "gains_algorithm": "moving_average_at",
        }
        inputs = TaxEngineLedgerInputs(
            rows=[
                {
                    "id": "staking-receipt-1",
                    "wallet_id": "wallet-austrian",
                    "wallet_label": "AustrianIncome",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "direction": "inbound",
                    "asset": "BTC",
                    "amount": 100_000_000,
                    "fee": 0,
                    "fiat_rate": 40_000,
                    "fiat_value": 40,
                    "kind": "staking",
                    "description": "Staking reward",
                    "note": None,
                    "external_id": "staking-receipt-1",
                    "created_at": "2024-01-01T00:00:00Z",
                }
            ],
            wallet_refs_by_id={
                "wallet-austrian": {
                    "id": "wallet-austrian",
                    "label": "AustrianIncome",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                }
            },
            manual_pair_records=[],
        )
        actual = self._direct_engine_snapshot(profile, inputs)
        self.assertEqual(actual["quarantines"], [])
        self.assertEqual(
            [entry["entry_type"] for entry in actual["entries"]],
            ["acquisition", "income"],
        )
        income_entry = next(entry for entry in actual["entries"] if entry["entry_type"] == "income")
        self.assertEqual(income_entry["quantity"], 0.001)
        self.assertEqual(income_entry["fiat_value"], 40.0)
        self.assertEqual(income_entry["gain_loss"], 40.0)
        self.assertEqual(income_entry["at_category"], "income_capital_yield")
        self.assertEqual(income_entry["at_kennzahl"], 175)
        self.assertEqual(
            actual["wallet_holdings"],
            [
                {
                    "wallet_id": "wallet-austrian",
                    "wallet_label": "AustrianIncome",
                    "account_code": "treasury",
                    "asset": "BTC",
                    "quantity": 0.001,
                    "cost_basis": 40.0,
                }
            ],
        )

    def test_austrian_rp2_cross_asset_swap_carries_basis(self):
        """End-to-end: AT profile carries basis across matched Neu cross-asset swaps."""
        profile, inputs = self._direct_austrian_cross_asset_swap_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("austrian_rp2_cross_asset_swap_snapshot.json")
        self.assertEqual(actual, expected)

    def test_build_tax_engine_accepts_austrian_profile_and_routes_to_rp2_at(self):
        profile = {
            "id": "profile-at",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrian",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 365,
            "gains_algorithm": "moving_average_at",
        }
        engine = build_tax_engine(profile)
        self.assertIsNotNone(engine)
        # Policy should reflect rp2's AT plugin — moving_average_at default,
        # English fallback generators, and Austrian accounting methods.
        from kassiber.tax_policy import build_tax_policy

        policy = build_tax_policy(profile)
        self.assertEqual(policy.tax_country, "at")
        self.assertEqual(policy.fiat_currency, "EUR")
        self.assertEqual(policy.default_accounting_method, "moving_average_at")
        self.assertIn("moving_average_at", policy.accounting_methods)
        self.assertEqual(policy.generation_language, "en")
        self.assertEqual(policy.report_generators, ("open_positions",))

    def test_generic_rp2_engine_quarantines_unfunded_transfer(self):
        profile, _ = self._direct_transfer_engine_inputs()
        inputs = TaxEngineLedgerInputs(
            rows=[
                {
                    "id": "transfer-out",
                    "wallet_id": "wallet-cold",
                    "wallet_label": "Cold",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                    "occurred_at": "2026-02-01T12:00:00Z",
                    "direction": "outbound",
                    "asset": "BTC",
                    "amount": 50_000_000_000,
                    "fee": 100_000_000,
                    "fiat_rate": 65000,
                    "fiat_value": 32500,
                    "kind": "withdrawal",
                    "description": "Move to hot wallet",
                    "note": None,
                    "external_id": "onchain-self-transfer-1",
                    "created_at": "2026-02-01T12:00:00Z",
                },
                {
                    "id": "transfer-in",
                    "wallet_id": "wallet-hot",
                    "wallet_label": "Hot",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                    "occurred_at": "2026-02-01T12:00:00Z",
                    "direction": "inbound",
                    "asset": "BTC",
                    "amount": 50_000_000_000,
                    "fee": 0,
                    "fiat_rate": 65000,
                    "fiat_value": 32500,
                    "kind": "deposit",
                    "description": "Receive from cold wallet",
                    "note": None,
                    "external_id": "onchain-self-transfer-1",
                    "created_at": "2026-02-01T12:00:00Z",
                },
            ],
            wallet_refs_by_id={
                "wallet-cold": {
                    "id": "wallet-cold",
                    "label": "Cold",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                },
                "wallet-hot": {
                    "id": "wallet-hot",
                    "label": "Hot",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                },
            },
            manual_pair_records=[],
        )
        state = build_tax_engine(profile).build_ledger_state(inputs)
        self.assertEqual(state.entries, [])
        self.assertEqual(len(state.quarantines), 1)
        self.assertEqual(state.quarantines[0]["reason"], "insufficient_lots")

    def test_missing_spot_price_snapshot_matches_fixture(self):
        self._bootstrap_wallet(label="MissingPrice")
        self._insert_transaction(
            wallet_label="MissingPrice",
            tx_id="missing-price",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        summary, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(summary, result, "journals.process")
        quarantines, result = self._run_json(
            "journals", "quarantined",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(quarantines, result, "journals.quarantined")
        journal_entries, result = self._run_json(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(journal_entries, result, "reports.journal-entries")
        actual = {
            "summary": {
                key: value
                for key, value in summary["data"].items()
                if key not in {"processed_at", "profile"}
            },
            "quarantines": [
                {key: value for key, value in row.items() if key != "transaction_id"}
                for row in quarantines["data"]
            ],
            "journal_entries": journal_entries["data"],
        }
        expected = self._load_fixture("generic_rp2_missing_spot_price_snapshot.json")
        self.assertEqual(actual, expected)

    def test_cross_asset_pair_snapshot_matches_fixture(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "CrossAsset",
        )
        self._assert_ok(payload, result, "profiles.create")

        cross_btc_csv = self.case_dir / "cross-btc.csv"
        cross_lbtc_csv = self.case_dir / "cross-lbtc.csv"
        cross_btc_csv.write_text(_CROSS_BTC_CSV, encoding="utf-8")
        cross_lbtc_csv.write_text(_CROSS_LBTC_CSV, encoding="utf-8")

        for label in ("Bitcoin", "Liquid"):
            payload, result = self._run_json(
                "wallets", "create",
                "--workspace", "Main",
                "--profile", "CrossAsset",
                "--label", label,
                "--kind", "custom",
            )
            self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "Bitcoin",
            "--file", str(cross_btc_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--wallet", "Liquid",
            "--file", str(cross_lbtc_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "CrossAsset",
            "--tx-out", "cross-out-leg",
            "--tx-in", "cross-in-leg",
            "--kind", "peg-in",
            "--policy", "taxable",
        )
        self._assert_ok(payload, result, "transfers.pair")

        summary, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        self._assert_ok(summary, result, "journals.process")
        quarantines, result = self._run_json(
            "journals", "quarantined",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        self._assert_ok(quarantines, result, "journals.quarantined")
        journal_entries, result = self._run_json(
            "reports", "journal-entries",
            "--workspace", "Main",
            "--profile", "CrossAsset",
        )
        self._assert_ok(journal_entries, result, "reports.journal-entries")
        actual = {
            "summary": {
                key: value
                for key, value in summary["data"].items()
                if key not in {"processed_at", "profile"}
            },
            "quarantines": [
                {key: value for key, value in row.items() if key != "transaction_id"}
                for row in quarantines["data"]
            ],
            "journal_entries": sorted(
                [{key: value for key, value in row.items() if key != "id"} for row in journal_entries["data"]],
                key=lambda row: (row["occurred_at"], row["entry_type"], row["wallet"], row["description"]),
            ),
        }
        expected = self._load_fixture("generic_rp2_cross_asset_pairs_snapshot.json")
        self.assertEqual(actual, expected)

    def test_balance_history_uses_historical_rate_and_remaining_basis(self):
        self._bootstrap_wallet(label="W")
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        profile = conn.execute("SELECT id, workspace_id FROM profiles WHERE label = 'Default'").fetchone()
        wallet = conn.execute("SELECT id, account_id FROM wallets WHERE label = 'W'").fetchone()
        conn.executescript(
            f"""
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES(
                'tx1', '{profile["workspace_id"]}', '{profile["id"]}', '{wallet["id"]}', 'buy', 'fp1',
                '2024-01-10T00:00:00Z', 'inbound', 'BTC', 100000000000, 0, 'USD',
                10000, 10000, 'deposit', 'buy', '{{}}', '2024-01-10T00:00:00Z'
            );
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, direction, asset, amount, fee, fiat_currency,
                fiat_rate, fiat_value, kind, description, raw_json, created_at
            ) VALUES(
                'tx2', '{profile["workspace_id"]}', '{profile["id"]}', '{wallet["id"]}', 'sell', 'fp2',
                '2024-02-10T00:00:00Z', 'outbound', 'BTC', 50000000000, 0, 'USD',
                20000, 10000, 'withdrawal', 'sell', '{{}}', '2024-02-10T00:00:00Z'
            );
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(
                'je1', '{profile["workspace_id"]}', '{profile["id"]}', 'tx1', '{wallet["id"]}', '{wallet["account_id"]}',
                '2024-01-10T00:00:00Z', 'acquisition', 'BTC', 100000000000, 10000, 10000,
                NULL, NULL, NULL, 'buy', '2024-01-10T00:00:00Z'
            );
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(
                'je2', '{profile["workspace_id"]}', '{profile["id"]}', 'tx2', '{wallet["id"]}', '{wallet["account_id"]}',
                '2024-02-10T00:00:00Z', 'disposal', 'BTC', -50000000000, 10000, 0,
                5000, 10000, 5000, 'sell', '2024-02-10T00:00:00Z'
            );
            UPDATE profiles
            SET last_processed_at = '2024-02-10T00:00:00Z', last_processed_tx_count = 2
            WHERE id = '{profile["id"]}';
            """
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "Default",
            "--interval", "month",
        )
        self._assert_ok(payload, result, "reports.balance-history")
        january = next(row for row in payload["data"] if row["period_start"] == "2024-01-01T00:00:00Z")
        february = next(row for row in payload["data"] if row["period_start"] == "2024-02-01T00:00:00Z")
        self.assertAlmostEqual(january["market_value"], 10000.0, places=4)
        self.assertAlmostEqual(february["cumulative_cost_basis"], 5000.0, places=4)
        self.assertAlmostEqual(february["market_value"], 10000.0, places=4)

    def test_table_output_honors_output_path(self):
        output_path = self.case_dir / "init.txt"
        result = self._run_cli("init", output=output_path)
        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertEqual(result.stdout.strip(), "")
        self.assertTrue(output_path.exists())
        text = output_path.read_text(encoding="utf-8")
        self.assertIn("version", text)
        self.assertIn("data_root", text)

    def test_default_home_state_ignores_repo_local_env(self):
        repo_dir = self.case_dir / "repo"
        repo_dir.mkdir(parents=True, exist_ok=True)
        (repo_dir / ".env").write_text("KASSIBER_DEFAULT_BACKEND=broken\n", encoding="utf-8")
        home_dir = self.case_dir / "home"
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("KASSIBER_", "SATBOOKS_"))
        }
        env["HOME"] = str(home_dir)
        env["PYTHONPATH"] = str(ROOT) if not env.get("PYTHONPATH") else f"{ROOT}{os.pathsep}{env['PYTHONPATH']}"

        payload, result = self._run_json(
            "init",
            explicit_data_root=False,
            env=env,
            cwd=repo_dir,
        )
        self._assert_ok(payload, result, "init")
        expected_root = home_dir / ".kassiber"
        self.assertEqual(payload["data"]["state_root"], str(expected_root))
        self.assertEqual(payload["data"]["data_root"], str(expected_root / "data"))
        self.assertEqual(payload["data"]["database"], str(expected_root / "data" / "kassiber.sqlite3"))
        self.assertEqual(payload["data"]["config_root"], str(expected_root / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(expected_root / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(expected_root / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(expected_root / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(expected_root / "config" / "backends.env"))
        self.assertTrue((expected_root / "data" / "kassiber.sqlite3").exists())
        self.assertTrue((expected_root / "config" / "settings.json").exists())
        settings_payload = json.loads((expected_root / "config" / "settings.json").read_text(encoding="utf-8"))
        self.assertEqual(settings_payload["paths"]["state_root"], str(expected_root))
        self.assertEqual(settings_payload["paths"]["data_root"], str(expected_root / "data"))
        self.assertEqual(settings_payload["paths"]["env_file"], str(expected_root / "config" / "backends.env"))
        self.assertEqual(settings_payload["paths"]["attachments_root"], str(expected_root / "attachments"))
        self.assertFalse((repo_dir / "kassiber.sqlite3").exists())

        payload, result = self._run_json(
            "status",
            explicit_data_root=False,
            env=env,
            cwd=repo_dir,
        )
        self._assert_ok(payload, result, "status")
        self.assertEqual(payload["data"]["state_root"], str(expected_root))
        self.assertEqual(payload["data"]["data_root"], str(expected_root / "data"))
        self.assertEqual(payload["data"]["config_root"], str(expected_root / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(expected_root / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(expected_root / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(expected_root / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(expected_root / "config" / "backends.env"))
        self.assertEqual(payload["data"]["default_backend"], "mempool")

    def test_profiles_create_accepts_austrian_tax_country(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--tax-country", "at",
            "--gains-algorithm", "MOVING_AVERAGE_AT",
            "Austrian",
        )
        self._assert_ok(payload, result, "profiles.create")
        self._assert_austrian_policy(payload)

    def test_profiles_set_accepts_switching_to_austrian_tax_country(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--tax-country", "at",
            "--gains-algorithm", "MOVING_AVERAGE_AT",
        )
        self._assert_ok(payload, result, "profiles.set")
        self._assert_austrian_policy(payload)

    def test_austrian_profile_journals_process_succeeds(self):
        self._bootstrap_wallet(label="AustrianJournal")
        json_file = self.case_dir / "austrian-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "kind": "buy",
                        "txid": "at-demo",
                        "fiat_value": "40",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianJournal",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        self._set_profile_tax_country("Default", "at")
        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--gains-algorithm", "MOVING_AVERAGE_AT",
        )
        self._assert_ok(payload, result, "profiles.set")

        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

    def test_austrian_profile_reports_capital_gains_succeeds(self):
        self._bootstrap_wallet(label="AustrianReport")
        json_file = self.case_dir / "austrian-report-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "kind": "buy",
                        "txid": "at-report-demo",
                        "fiat_value": "40",
                    },
                    {
                        "date": "2024-06-01",
                        "direction": "outbound",
                        "asset": "BTC",
                        "amount": "0.0005",
                        "fee": "0",
                        "kind": "sell",
                        "txid": "at-report-demo-sell",
                        "fiat_value": "30",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianReport",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        self._set_profile_tax_country("Default", "at")
        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--gains-algorithm", "MOVING_AVERAGE_AT",
        )
        self._assert_ok(payload, result, "profiles.set")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "reports.capital-gains")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["at_category"], "neu_gain")
        self.assertEqual(payload["data"][0]["at_kennzahl"], 174)

    def test_austrian_profile_reports_staking_receipt_as_income(self):
        self._bootstrap_wallet(label="AustrianIncome")
        json_file = self.case_dir / "austrian-income-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "kind": "staking",
                        "txid": "at-staking-demo",
                        "fiat_value": "40",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianIncome",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        self._set_profile_tax_country("Default", "at")
        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--gains-algorithm", "MOVING_AVERAGE_AT",
        )
        self._assert_ok(payload, result, "profiles.set")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "reports", "capital-gains",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "reports.capital-gains")
        self.assertEqual(len(payload["data"]), 1)
        self.assertEqual(payload["data"][0]["entry_type"], "income")
        self.assertEqual(payload["data"][0]["at_category"], "income_capital_yield")
        self.assertEqual(payload["data"][0]["at_kennzahl"], 175)
        self.assertEqual(payload["data"][0]["gain_loss"], 40.0)

    def test_attachments_verify_reports_missing_file(self):
        self._bootstrap_wallet(label="Attachable")
        json_file = self.case_dir / "attachment-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "attach-demo",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Attachable",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")

        attachment_file = self.case_dir / "receipt.txt"
        attachment_file.write_text("receipt\n", encoding="utf-8")
        payload, result = self._run_json(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "attach-demo",
            "--file", str(attachment_file),
        )
        self._assert_ok(payload, result, "attachments.add")
        stored_path = self.case_dir / "attachments" / payload["data"]["stored_relpath"]
        stored_path.unlink()

        payload, result = self._run_json(
            "attachments", "verify",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "attachments.verify")
        self.assertEqual(payload["data"]["checked"], 1)
        self.assertEqual(payload["data"]["broken"], 1)
        self.assertEqual(payload["data"]["ok"], 0)
        broken = payload["data"]["results"][0]
        self.assertEqual(broken["status"], "broken")
        self.assertEqual(broken["issues"], ["missing_file"])

    def test_attachments_verify_and_remove_ignore_escaped_storage_path(self):
        self._bootstrap_wallet(label="AttachEscape")
        json_file = self.case_dir / "attachment-escape-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "attach-escape",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AttachEscape",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")

        attachment_file = self.case_dir / "escape-receipt.txt"
        attachment_file.write_text("escape\n", encoding="utf-8")
        payload, result = self._run_json(
            "attachments", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "attach-escape",
            "--file", str(attachment_file),
        )
        self._assert_ok(payload, result, "attachments.add")
        attachment_id = payload["data"]["id"]
        external_path = self.case_dir / "escape-target.txt"
        external_path.write_text("do not delete\n", encoding="utf-8")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.execute(
            "UPDATE attachments SET stored_relpath = ? WHERE id = ?",
            ("../escape-target.txt", attachment_id),
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "attachments", "verify",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "attachments.verify")
        self.assertEqual(payload["data"]["checked"], 1)
        self.assertEqual(payload["data"]["broken"], 1)
        broken = payload["data"]["results"][0]
        self.assertEqual(broken["issues"], ["invalid_storage_path"])

        payload, result = self._run_json(
            "attachments", "remove",
            "--workspace", "Main",
            "--profile", "Default",
            attachment_id,
        )
        self._assert_ok(payload, result, "attachments.remove")
        self.assertTrue(payload["data"]["removed"])
        self.assertFalse(payload["data"]["deleted_file"])
        self.assertTrue(external_path.exists())

    def test_attachments_gc_removes_orphan_files(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        attachments_root = Path(payload["data"]["attachments_root"])
        orphan_path = attachments_root / "orphan" / "lost.bin"
        orphan_path.parent.mkdir(parents=True, exist_ok=True)
        orphan_path.write_bytes(b"orphan")

        payload, result = self._run_json("attachments", "gc", "--dry-run")
        self._assert_ok(payload, result, "attachments.gc")
        self.assertEqual(payload["data"]["orphaned_files"], 1)
        self.assertEqual(payload["data"]["removed_files"], 0)
        self.assertEqual(payload["data"]["files"][0]["stored_relpath"], "orphan/lost.bin")
        self.assertTrue(orphan_path.exists())

        payload, result = self._run_json("attachments", "gc")
        self._assert_ok(payload, result, "attachments.gc")
        self.assertEqual(payload["data"]["orphaned_files"], 1)
        self.assertEqual(payload["data"]["removed_files"], 1)
        self.assertFalse(orphan_path.exists())

    def test_migration_preserves_child_rows(self):
        self.data_root.mkdir(parents=True, exist_ok=True)
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.executescript(_OLD_SCHEMA_SQL)
        conn.executescript(
            """
            INSERT INTO workspaces VALUES('ws', 'Main', '2024-01-01T00:00:00Z');
            INSERT INTO profiles VALUES('pf', 'ws', 'Default', 'USD', 'generic', 365, 'FIFO', NULL, 0, '2024-01-01T00:00:00Z');
            INSERT INTO accounts VALUES('acct', 'ws', 'pf', 'cash', 'Cash', 'asset', 'BTC', '2024-01-01T00:00:00Z');
            INSERT INTO wallets VALUES('wal', 'ws', 'pf', 'acct', 'Wallet', 'address', '{}', '2024-01-01T00:00:00Z');
            INSERT INTO tags VALUES('tag', 'ws', 'pf', 'important', 'Important', '2024-01-01T00:00:00Z');
            INSERT INTO transactions VALUES('tx', 'ws', 'pf', 'wal', 'ext', 'fp', '2024-01-01T00:00:00Z', 'inbound', 'BTC', 1.0, 0.0, 'USD', 10000, 10000, 'deposit', 'desc', NULL, NULL, 0, '{}', '2024-01-01T00:00:00Z');
            INSERT INTO transaction_tags VALUES('tx', 'tag');
            INSERT INTO journal_entries VALUES('je', 'ws', 'pf', 'tx', 'wal', 'acct', '2024-01-01T00:00:00Z', 'acquisition', 'BTC', 1.0, 10000, 10000, NULL, NULL, NULL, 'desc', NULL, NULL, '2024-01-01T00:00:00Z');
            INSERT INTO journal_quarantines VALUES('tx', 'ws', 'pf', 'reason', '{}', '2024-01-01T00:00:00Z');
            """
        )
        conn.commit()
        conn.close()

        result = self._run_cli("status")
        self.assertEqual(result.returncode, 0, msg=result.stderr)

        conn = sqlite3.connect(db_path)
        counts = conn.execute(
            """
            SELECT
                (SELECT COUNT(*) FROM transactions),
                (SELECT COUNT(*) FROM transaction_tags),
                (SELECT COUNT(*) FROM journal_entries),
                (SELECT COUNT(*) FROM journal_quarantines)
            """
        ).fetchone()
        conn.close()
        self.assertEqual(counts, (1, 1, 1, 1))

    def test_invalid_import_timestamp_returns_validation_error(self):
        self._bootstrap_wallet(label="BadTS")
        json_file = self.case_dir / "bad-ts.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "not-a-timestamp",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                    }
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BadTS",
            "--file", str(json_file),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")


if __name__ == "__main__":
    unittest.main()
