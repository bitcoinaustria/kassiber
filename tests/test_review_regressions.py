import json
import os
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import threading
import unittest
import zipfile
from argparse import Namespace
from copy import deepcopy
from decimal import Decimal
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch
from urllib.parse import parse_qs, urlparse

from kassiber.backends import (
    redact_backend_for_output,
    redact_backend_text,
    redact_backend_value,
)
from kassiber.cli.main import command_needs_db
from kassiber.cli.handlers import (
    _attachment_hooks,
    _audit_transaction_refs,
    _report_hooks,
    cache_swap_candidate_count,
    create_direct_swap_payout,
    create_transaction_pair,
    list_transaction_pairs,
    process_journals,
    suggest_transfer_candidates,
    update_transaction_pair,
)
from kassiber.core import attachments as core_attachments
from kassiber.core import custody_journal as core_custody_journal
from kassiber.core import pricing
from kassiber.core import rates as core_rates
from kassiber.core.engines import rp2 as rp2_engine
from kassiber.core.engines import build_tax_engine
from kassiber.core.reports import (
    ReportHooks,
    _generic_report_transfer_pair_rows,
    _holdings_basis_delta,
    _holdings_quantity_delta,
    latest_transaction_rates_for_profile,
    report_austrian_e1kv,
    report_balance_history,
    report_balance_sheet,
    report_portfolio_summary,
    report_tax_summary,
)
from kassiber.core.report_context import ReportContext
from kassiber.core.runtime import bootstrap_runtime, close_runtime
from kassiber.core.tax_events import normalize_tax_asset_inputs
from kassiber.core.ui_snapshot import (
    _tax_free_wallet_summaries,
    _transaction_pair_display_meta,
    build_capital_gains_snapshot,
    build_journal_events_list_snapshot,
    build_journals_snapshot,
    build_overview_snapshot,
    build_rates_coverage_snapshot,
    build_next_actions_snapshot,
    build_report_blockers_snapshot,
    build_review_badges_snapshot,
    build_transactions_search_snapshot,
    build_transactions_resolve_snapshot,
    build_transactions_snapshot,
)
from kassiber.db import open_db, set_setting
from kassiber.errors import AppError
from kassiber.importers import normalize_river_record
from kassiber.msat import btc_to_msat
from tests.custody_tax_helpers import finalized_tax_inputs


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

_FIXTURE_SELF_TRANSFER_TXID = "4" * 64
_AT_TRANSFER_TXID = "5" * 64
_AT_ALT_TRANSFER_TXID = "6" * 64
_AT_MIXED_TRANSFER_TXID = "7" * 64


def _typed_onchain_raw(external_id):
    text = str(external_id or "")
    if len(text) == 64 and all(char in "0123456789abcdefABCDEF" for char in text):
        return json.dumps({"txid": text})
    return "{}"

_FIXTURE_COLD_TRANSFER_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description
2026-01-01T10:00:00Z,cold-funding-1,inbound,BTC,1.00000000,0,60000,Cold acquisition
2026-02-01T12:00:00Z,{_FIXTURE_SELF_TRANSFER_TXID},outbound,BTC,0.50000000,0.001,65000,Move to hot wallet
"""

_FIXTURE_HOT_TRANSFER_CSV = f"""date,txid,direction,asset,amount,fee,fiat_rate,description
2026-02-01T12:00:00Z,{_FIXTURE_SELF_TRANSFER_TXID},inbound,BTC,0.50000000,0,65000,Receive from cold wallet
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
            {
                key: _json_decimal(value)
                for key, value in entry.items()
                if key not in {"id", "capital_gains_type"}
            }
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


def _legacy_snapshot_identity(snapshot):
    """Compare economic output while ignoring finalized-slice identifiers.

    The historical JSON fixtures predate the custody projection and therefore
    name imported rows directly. Finalized slice IDs and their provenance are
    covered by custody-specific tests; these fixtures still guard the RP2
    entries, holdings, quarantine, and transfer economics.
    """

    result = deepcopy(snapshot)
    for transfer in result["intra_audit"]:
        transfer["out_id"] = transfer.get(
            "out_anchor_transaction_id", transfer["out_id"]
        )
        transfer["in_id"] = transfer.get(
            "in_anchor_transaction_id", transfer["in_id"]
        )
        transfer["pairing_source"] = None
        transfer.pop("transfer_group_id", None)
    for pair in result["cross_asset_pairs"]:
        pair["out_id"] = pair.pop("out_transaction_id", pair["out_id"])
        pair["in_id"] = pair.pop("in_transaction_id", pair["in_id"])
        pair.pop("component_id", None)
    return result


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

    def test_latest_transaction_rates_are_shared_between_reports_and_ledger_rebuild(self):
        conn = sqlite3.connect(":memory:")
        self.addCleanup(conn.close)
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            CREATE TABLE transactions(
                profile_id TEXT NOT NULL,
                excluded INTEGER NOT NULL DEFAULT 0,
                occurred_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                asset TEXT NOT NULL,
                fiat_rate REAL,
                fiat_value REAL,
                fiat_rate_exact TEXT,
                fiat_value_exact TEXT,
                amount INTEGER NOT NULL
            )
            """
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                profile_id, excluded, occurred_at, created_at, asset,
                fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "pf-rate",
                    0,
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                    "BTC",
                    50000.0,
                    None,
                    None,
                    None,
                    btc_to_msat("0.1"),
                ),
                (
                    "pf-rate",
                    0,
                    "2026-02-01T00:00:00Z",
                    "2026-02-01T00:00:00Z",
                    "BTC",
                    1.0,
                    None,
                    "62000.123456789",
                    None,
                    btc_to_msat("0.1"),
                ),
                (
                    "pf-rate",
                    0,
                    "2026-02-02T00:00:00Z",
                    "2026-02-02T00:00:00Z",
                    "LBTC",
                    None,
                    None,
                    None,
                    "310.00",
                    btc_to_msat("0.005"),
                ),
                (
                    "pf-rate",
                    1,
                    "2026-03-01T00:00:00Z",
                    "2026-03-01T00:00:00Z",
                    "BTC",
                    99999.0,
                    None,
                    None,
                    None,
                    btc_to_msat("0.1"),
                ),
                (
                    "pf-other",
                    0,
                    "2026-04-01T00:00:00Z",
                    "2026-04-01T00:00:00Z",
                    "BTC",
                    12345.0,
                    None,
                    None,
                    None,
                    btc_to_msat("0.1"),
                ),
            ],
        )

        report_rates = latest_transaction_rates_for_profile(conn, "pf-rate")
        ledger_rates = core_custody_journal.latest_transaction_rates_for_profile(
            conn, "pf-rate"
        )

        self.assertEqual(report_rates, ledger_rates)
        self.assertEqual(report_rates["BTC"], Decimal("62000.123456789"))
        self.assertEqual(report_rates["LBTC"], Decimal("62000"))

    def test_current_portfolio_reports_use_latest_cached_market_rate(self):
        conn = sqlite3.connect(":memory:")
        self.addCleanup(conn.close)
        conn.row_factory = sqlite3.Row
        conn.executescript(
            """
            CREATE TABLE transactions(
                profile_id TEXT NOT NULL,
                excluded INTEGER NOT NULL DEFAULT 0,
                occurred_at TEXT NOT NULL,
                created_at TEXT NOT NULL,
                asset TEXT NOT NULL,
                fiat_rate REAL,
                fiat_value REAL,
                fiat_rate_exact TEXT,
                fiat_value_exact TEXT,
                amount INTEGER NOT NULL
            );
            CREATE TABLE journal_account_holdings(
                id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                account_code TEXT,
                account_label TEXT,
                asset TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                cost_basis REAL NOT NULL
            );
            CREATE TABLE journal_wallet_holdings(
                id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                wallet_id TEXT NOT NULL,
                wallet_label TEXT NOT NULL,
                account_code TEXT,
                asset TEXT NOT NULL,
                quantity INTEGER NOT NULL,
                cost_basis REAL NOT NULL
            );
            CREATE TABLE rates_cache(
                pair TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                rate REAL NOT NULL,
                rate_exact TEXT,
                source TEXT NOT NULL,
                fetched_at TEXT NOT NULL,
                granularity TEXT,
                method TEXT
            );
            """
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                profile_id, excluded, occurred_at, created_at, asset,
                fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact, amount
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "pf-current",
                    0,
                    "2026-05-15T17:42:32Z",
                    "2026-05-15T17:42:32Z",
                    "BTC",
                    68_277.1,
                    None,
                    None,
                    None,
                    btc_to_msat("0.5"),
                ),
                (
                    "pf-current",
                    0,
                    "2026-05-26T16:00:10Z",
                    "2026-05-26T16:00:10Z",
                    "LBTC",
                    65_777.1,
                    None,
                    None,
                    None,
                    btc_to_msat("0.25"),
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, rate_exact, source, fetched_at, granularity, method)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "BTC-EUR",
                "2026-06-13T13:09:00Z",
                55_455.25,
                "55455.25",
                "coinbase-exchange",
                "2026-06-13T13:09:04Z",
                "minute",
                "product_candles",
            ),
        )
        conn.executemany(
            """
            INSERT INTO journal_account_holdings(
                id, profile_id, account_code, account_label, asset, quantity, cost_basis
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "acct-holding-btc",
                    "pf-current",
                    "treasury",
                    "Treasury",
                    "BTC",
                    btc_to_msat("0.5"),
                    20_000,
                ),
                (
                    "acct-holding-lbtc",
                    "pf-current",
                    "treasury",
                    "Treasury",
                    "LBTC",
                    btc_to_msat("0.25"),
                    10_000,
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_wallet_holdings(
                id, profile_id, wallet_id, wallet_label, account_code, asset, quantity, cost_basis
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "wallet-holding-btc",
                    "pf-current",
                    "wallet-btc",
                    "Onchain",
                    "treasury",
                    "BTC",
                    btc_to_msat("0.5"),
                    20_000,
                ),
                (
                    "wallet-holding-lbtc",
                    "pf-current",
                    "wallet-lbtc",
                    "Liquid",
                    "treasury",
                    "LBTC",
                    btc_to_msat("0.25"),
                    10_000,
                ),
            ],
        )
        profile = {
            "id": "pf-current",
            "label": "Current",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }

        def unused(*_args, **_kwargs):
            raise AssertionError("unexpected hook call")

        hooks = ReportHooks(
            resolve_scope=lambda _conn, _workspace_ref, _profile_ref: (
                {"id": "ws-current"},
                profile,
            ),
            resolve_account=unused,
            resolve_wallet=unused,
            list_journal_entries=unused,
            list_wallets=unused,
            parse_iso_datetime=unused,
            iso_z=unused,
            now_iso=unused,
            format_table=unused,
            write_text_pdf=unused,
        )
        report_context = ReportContext(
            workspace={"id": "ws-current"},
            profile=profile,
            active_transaction_count=2,
            journal_input_version=0,
            last_processed_input_version=0,
            last_processed_at="2026-06-13T13:09:04Z",
        )

        balance_rows = {
            row["asset"]: row
            for row in report_balance_sheet(
                conn, None, None, hooks, report_context=report_context
            )
        }
        portfolio_rows = {
            row["asset"]: row
            for row in report_portfolio_summary(
                conn, None, None, hooks, report_context=report_context
            )
        }

        self.assertAlmostEqual(balance_rows["BTC"]["market_value"], 27_727.625)
        self.assertAlmostEqual(balance_rows["LBTC"]["market_value"], 13_863.8125)
        self.assertAlmostEqual(portfolio_rows["BTC"]["market_value"], 27_727.625)
        self.assertAlmostEqual(portfolio_rows["LBTC"]["market_value"], 13_863.8125)

    def test_ui_snapshots_use_populated_profile_rows(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-ui", "UI Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-ui",
                "ws-ui",
                "UI Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-02-02T00:00:00Z",
                2,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-ui", "ws-ui", "pf-ui", "Cold Wallet", "address", "{}", now),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-ui-in",
                    "ws-ui",
                    "pf-ui",
                    "wal-ui",
                    "a" * 64,
                    "fp-ui-in",
                    "2026-01-10T10:00:00Z",
                    "2026-01-10T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("1.0"),
                    0,
                    "EUR",
                    50_000,
                    50_000,
                    "import",
                    "transfer",
                    "Initial funding",
                    "Exchange",
                    None,
                    0,
                    "{}",
                    "2026-01-10T10:00:00Z",
                ),
                (
                    "tx-ui-spend",
                    "ws-ui",
                    "pf-ui",
                    "wal-ui",
                    "external-spend",
                    "fp-ui-spend",
                    "2026-02-01T12:00:00Z",
                    "2026-02-01T12:10:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("0.1"),
                    btc_to_msat("0.001"),
                    "EUR",
                    60_000,
                    6_000,
                    "import",
                    "payment",
                    "Merchant spend",
                    "Merchant",
                    None,
                    0,
                    "{}",
                    "2026-02-01T12:00:00Z",
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-ui-in",
                    "ws-ui",
                    "pf-ui",
                    "tx-ui-in",
                    "wal-ui",
                    "2026-01-10T10:00:00Z",
                    "acquisition",
                    "BTC",
                    btc_to_msat("1.0"),
                    50_000,
                    50_000,
                    None,
                    None,
                    None,
                    "Initial funding",
                    "2026-01-10T10:00:00Z",
                ),
                (
                    "je-ui-spend",
                    "ws-ui",
                    "pf-ui",
                    "tx-ui-spend",
                    "wal-ui",
                    "2026-02-01T12:00:00Z",
                    "disposal",
                    "BTC",
                    -btc_to_msat("0.1"),
                    -6_000,
                    50_000,
                    5_000,
                    6_000,
                    1_000,
                    "Merchant spend",
                    "2026-02-01T12:00:00Z",
                ),
                (
                    "je-ui-fee",
                    "ws-ui",
                    "pf-ui",
                    "tx-ui-spend",
                    "wal-ui",
                    "2026-02-01T12:00:00Z",
                    "fee",
                    "BTC",
                    -btc_to_msat("0.001"),
                    -60,
                    60_000,
                    50,
                    60,
                    10,
                    "Network fee",
                    "2026-02-01T12:00:00Z",
                ),
                (
                    "je-ui-income",
                    "ws-ui",
                    "pf-ui",
                    "tx-ui-in",
                    "wal-ui",
                    "2026-01-15T12:00:00Z",
                    "income",
                    "BTC",
                    btc_to_msat("0.02"),
                    1_000,
                    50_000,
                    None,
                    None,
                    None,
                    "Income recognition; basis rides on the paired acquisition",
                    "2026-01-15T12:00:00Z",
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            ("tx-ui-spend", "ws-ui", "pf-ui", "missing_fee_price", "{}", now),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            ("tx-ui-in", "ws-ui", "pf-ui", "missing_spot_price", "{}", now),
        )
        conn.executemany(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
            VALUES(?, ?, ?, ?, ?)
            """,
            [
                ("BTC-EUR", "2026-01-10T00:00:00Z", 50_000, "manual", now),
                ("BTC-EUR", "2026-02-01T00:00:00Z", 65_000, "manual", now),
            ],
        )
        set_setting(conn, "context_workspace", "ws-ui")
        set_setting(conn, "context_profile", "pf-ui")
        conn.commit()

        overview = build_overview_snapshot(conn)
        self.assertEqual(overview["status"]["workspace"], "UI Workspace")
        self.assertEqual(overview["status"]["profile"], "UI Profile")
        self.assertEqual(overview["status"]["transactionCount"], 2)
        self.assertFalse(overview["status"]["needsJournals"])
        self.assertEqual(overview["status"]["quarantines"], 2)
        self.assertEqual(overview["priceEur"], 65_000)
        self.assertEqual(overview["marketRate"]["fiatCurrency"], "EUR")
        self.assertEqual(overview["marketRate"]["pair"], "BTC-EUR")
        self.assertEqual(overview["marketRate"]["rate"], 65_000)
        self.assertEqual(overview["marketRate"]["timestamp"], "2026-02-01T00:00:00Z")
        self.assertEqual(overview["marketRate"]["source"], "manual")
        self.assertEqual(overview["marketRate"]["fetchedAt"], now)
        self.assertEqual(len(overview["connections"]), 1)
        self.assertEqual(overview["connections"][0]["label"], "Cold Wallet")
        self.assertAlmostEqual(overview["connections"][0]["balance"], 0.899)
        self.assertAlmostEqual(overview["balanceSeries"][-1], 0.899)
        self.assertEqual(overview["fiat"]["fiatCurrency"], "EUR")
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 58_435)
        self.assertAlmostEqual(overview["fiat"]["eurCostBasis"], 44_950)
        self.assertAlmostEqual(overview["fiat"]["eurUnrealized"], 13_485)
        self.assertGreaterEqual(len(overview["portfolioSeries"]), 2)
        self.assertEqual(
            [point["date"] for point in overview["portfolioSeries"]],
            ["2026-01-10", "2026-02-01"],
        )
        self.assertAlmostEqual(overview["portfolioSeries"][0]["priceEur"], 50_000)
        self.assertAlmostEqual(overview["portfolioSeries"][0]["valueEur"], 50_000)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["balanceBtc"], 0.899)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["valueEur"], 58_435)
        self.assertEqual(overview["txs"][0]["id"], "tx-ui-spend")
        self.assertEqual(overview["txs"][0]["externalId"], "external-spend")
        self.assertIsNone(overview["txs"][0]["explorerId"])
        self.assertEqual(overview["txs"][0]["type"], "Fee")
        self.assertEqual(overview["txs"][0]["amountSat"], -10_000_000)
        self.assertEqual(overview["txs"][0]["tag"], "Review")
        self.assertEqual(overview["txs"][1]["explorerId"], "a" * 64)
        self.assertEqual(
            [row["id"] for row in overview["activityTxs"]],
            ["tx-ui-in", "tx-ui-spend"],
        )
        self.assertEqual(
            overview["activityTxs"][0]["occurredAt"],
            "2026-01-10T10:00:00Z",
        )
        self.assertAlmostEqual(overview["activityTxs"][0]["balanceBtc"], 1.0)
        self.assertAlmostEqual(overview["activityTxs"][0]["costBasisEur"], 50_000)
        self.assertEqual(overview["activityTxs"][1]["feeSat"], 100_000)
        self.assertAlmostEqual(overview["activityTxs"][1]["balanceBtc"], 0.899)
        self.assertAlmostEqual(overview["activityTxs"][1]["costBasisEur"], 44_950)

        transactions = build_transactions_snapshot(conn, {"limit": 10})
        self.assertEqual(transactions["year"], 2026)
        self.assertEqual([row["id"] for row in transactions["txs"]], ["tx-ui-spend", "tx-ui-in"])
        self.assertEqual(transactions["txs"][1]["type"], "Income")
        self.assertEqual(transactions["txs"][1]["tag"], "Review")
        self.assertEqual(transactions["txs"][1]["externalId"], "a" * 64)
        self.assertEqual(transactions["txs"][1]["explorerId"], "a" * 64)
        self.assertEqual(transactions["txs"][1]["quarantineReason"], "missing_spot_price")
        self.assertEqual(transactions["txs"][1]["amountSat"], 100_000_000)
        self.assertEqual(transactions["txs"][1]["eur"], 50_000)

        resolved_by_txid = build_transactions_resolve_snapshot(conn, {"query": "A" * 64})
        self.assertEqual(resolved_by_txid["transaction"]["id"], "tx-ui-in")
        self.assertEqual(resolved_by_txid["transaction"]["explorerId"], "a" * 64)

        conn.execute(
            "UPDATE transactions SET external_id = ? WHERE id = ?",
            ("A" * 64, "tx-ui-in"),
        )
        conn.commit()
        resolved_by_lower_txid = build_transactions_resolve_snapshot(
            conn,
            {"query": "a" * 64},
        )
        self.assertEqual(resolved_by_lower_txid["transaction"]["id"], "tx-ui-in")
        self.assertEqual(resolved_by_lower_txid["transaction"]["explorerId"], "A" * 64)

        resolved_by_id = build_transactions_resolve_snapshot(conn, {"query": "tx-ui-spend"})
        self.assertEqual(resolved_by_id["transaction"]["id"], "tx-ui-spend")

        missing = build_transactions_resolve_snapshot(conn, {"query": "b" * 64})
        self.assertIsNone(missing["transaction"])

        with self.assertRaises(AppError) as empty_query:
            build_transactions_resolve_snapshot(conn, {"query": "  "})
        self.assertEqual(empty_query.exception.code, "validation")

        with self.assertRaises(AppError) as unknown_filter:
            build_transactions_resolve_snapshot(conn, {"query": "tx-ui-spend", "limit": 1})
        self.assertEqual(unknown_filter.exception.code, "validation")
        self.assertEqual(unknown_filter.exception.details, {"unknown": ["limit"]})

    def test_overview_balance_defaults_to_chain_inventory_when_available(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-chain-balance", "Chain Balance Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-chain-balance",
                "ws-chain-balance",
                "Chain Balance Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-02T00:00:00Z",
                0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO backends(
                name, kind, chain, network, url, config_json, created_at, updated_at
            ) VALUES(?, ?, ?, ?, ?, '{}', ?, ?)
            """,
            (
                "private",
                "esplora",
                "bitcoin",
                "mainnet",
                "https://private-node.example/api",
                now,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-chain-balance",
                "ws-chain-balance",
                "pf-chain-balance",
                "Cold Wallet",
                "address",
                json.dumps(
                    {
                        "addresses": ["bc1qchainbalance"],
                        "backend": "private",
                        "chain": "bitcoin",
                        "network": "mainnet",
                    }
                ),
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_wallet_holdings(
                id, workspace_id, profile_id, wallet_id, wallet_label,
                account_code, asset, quantity, cost_basis, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "holding-book-btc",
                "ws-chain-balance",
                "pf-chain-balance",
                "wal-chain-balance",
                "Cold Wallet",
                "",
                "BTC",
                btc_to_msat("1.0"),
                20_000,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, backend_name,
                backend_kind, chain, network, asset, amount, txid, vout,
                outpoint, confirmation_status, confirmations, block_height,
                block_time, address, first_seen_at, last_seen_at, raw_json
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, '{}')
            """,
            (
                "utxo-chain-balance",
                "ws-chain-balance",
                "pf-chain-balance",
                "wal-chain-balance",
                "private",
                "esplora",
                "bitcoin",
                "mainnet",
                "BTC",
                btc_to_msat("0.25"),
                "aa" * 32,
                0,
                f"{'aa' * 32}:0",
                "confirmed",
                6,
                800_000,
                "2026-01-02T00:00:00Z",
                "bc1qchainbalance",
                now,
                "2026-01-02T00:00:00Z",
            ),
        )
        conn.execute(
            """
            INSERT INTO wallet_utxo_refreshes(
                wallet_id, workspace_id, profile_id, backend_name, backend_kind,
                chain, network, observed_count, active_count, last_seen_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-chain-balance",
                "ws-chain-balance",
                "pf-chain-balance",
                "private",
                "esplora",
                "bitcoin",
                "mainnet",
                1,
                1,
                "2026-01-02T00:00:00Z",
            ),
        )
        conn.execute(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
            VALUES(?, ?, ?, ?, ?)
            """,
            ("BTC-EUR", "2026-01-02T00:00:00Z", 50_000, "manual", now),
        )
        set_setting(conn, "context_workspace", "ws-chain-balance")
        set_setting(conn, "context_profile", "pf-chain-balance")
        conn.commit()

        overview = build_overview_snapshot(conn)
        connection = overview["connections"][0]

        self.assertAlmostEqual(connection["bookBalance"], 1.0)
        self.assertAlmostEqual(connection["chainBalance"], 0.25)
        self.assertEqual(connection["balanceSource"], "chain")
        self.assertAlmostEqual(connection["balance"], 0.25)
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 12_500)

    def test_overview_snapshot_exposes_austrian_tax_free_balance(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-tax-free", "AT Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-tax-free",
                "ws-tax-free",
                "AT Profile",
                "EUR",
                "at",
                9223372036854775807,
                "MOVING_AVERAGE_AT",
                "2026-02-02T00:00:00Z",
                3,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-tax-free", "ws-tax-free", "pf-tax-free", "Cold", "address", "{}", now),
        )
        tx_rows = [
            ("tx-alt", "2020-06-01T00:00:00Z", "inbound", btc_to_msat("1.0"), 0, 8000),
            ("tx-neu", "2022-01-01T00:00:00Z", "inbound", btc_to_msat("0.5"), 0, 20000),
            ("tx-neu-sale", "2023-03-01T00:00:00Z", "outbound", btc_to_msat("0.1"), 0, 5000),
        ]
        for tx_id, occurred_at, direction, amount, fee, fiat_value in tx_rows:
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, confirmed_at, direction, asset, amount, fee,
                    fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                    description, counterparty, note, excluded, raw_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    tx_id,
                    "ws-tax-free",
                    "pf-tax-free",
                    "wal-tax-free",
                    tx_id,
                    f"fp-{tx_id}",
                    occurred_at,
                    occurred_at,
                    direction,
                    "BTC",
                    amount,
                    fee,
                    "EUR",
                    60_000,
                    fiat_value,
                    "manual",
                    "trade",
                    tx_id,
                    "",
                    None,
                    0,
                    "{}",
                    occurred_at,
                ),
            )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, at_category,
                at_kennzahl, capital_gains_type, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-alt",
                    "ws-tax-free",
                    "pf-tax-free",
                    "tx-alt",
                    "wal-tax-free",
                    None,
                    "2020-06-01T00:00:00Z",
                    "acquisition",
                    "BTC",
                    btc_to_msat("1.0"),
                    8000,
                    8000,
                    None,
                    None,
                    None,
                    "Alt buy",
                    None,
                    None,
                    None,
                    now,
                ),
                (
                    "je-neu",
                    "ws-tax-free",
                    "pf-tax-free",
                    "tx-neu",
                    "wal-tax-free",
                    None,
                    "2022-01-01T00:00:00Z",
                    "acquisition",
                    "BTC",
                    btc_to_msat("0.5"),
                    20000,
                    40000,
                    None,
                    None,
                    None,
                    "Neu buy",
                    None,
                    None,
                    None,
                    now,
                ),
                (
                    "je-neu-sale",
                    "ws-tax-free",
                    "pf-tax-free",
                    "tx-neu-sale",
                    "wal-tax-free",
                    None,
                    "2023-03-01T00:00:00Z",
                    "disposal",
                    "BTC",
                    btc_to_msat("-0.1"),
                    -5000,
                    50000,
                    4000,
                    5000,
                    1000,
                    "Neu sale",
                    "neu_gain",
                    174,
                    "short",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_wallet_holdings(
                id, workspace_id, profile_id, wallet_id, wallet_label,
                account_code, asset, quantity, cost_basis, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "holding-tax-free",
                "ws-tax-free",
                "pf-tax-free",
                "wal-tax-free",
                "Cold",
                "treasury",
                "BTC",
                btc_to_msat("1.4"),
                24000,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
            VALUES(?, ?, ?, ?, ?)
            """,
            ("BTC-EUR", "2026-06-15T12:00:00Z", 60_000, "manual", now),
        )
        set_setting(conn, "context_workspace", "ws-tax-free")
        set_setting(conn, "context_profile", "pf-tax-free")
        conn.commit()

        overview = build_overview_snapshot(conn)
        tax_free = overview["taxFreeBalance"]

        self.assertEqual(tax_free["rule"], "austrian_altbestand")
        self.assertEqual(tax_free["jurisdictionCode"], "AT")
        self.assertEqual(tax_free["taxFreeQuantitySats"], 100_000_000)
        self.assertEqual(tax_free["taxableQuantitySats"], 40_000_000)
        self.assertEqual(tax_free["totalQuantitySats"], 140_000_000)
        self.assertEqual(
            tax_free["wallets"],
            [{"walletId": "wal-tax-free", "hasTaxFreeBalance": True}],
        )
        self.assertEqual(
            [bucket["id"] for bucket in tax_free["buckets"]],
            ["altbestand", "neubestand"],
        )
        self.assertEqual(tax_free["status"], "current")
        self.assertFalse(tax_free["needsJournals"])
        self.assertEqual(tax_free["quarantines"], 0)

        conn.execute(
            "UPDATE profiles SET last_processed_tx_count = ? WHERE id = ?",
            (2, "pf-tax-free"),
        )
        conn.commit()

        stale_overview = build_overview_snapshot(conn)
        stale_tax_free = stale_overview["taxFreeBalance"]

        self.assertEqual(stale_tax_free["status"], "needs_journals")
        self.assertTrue(stale_tax_free["needsJournals"])
        self.assertEqual(stale_tax_free["taxFreeQuantitySats"], 100_000_000)

        conn.execute(
            "UPDATE profiles SET last_processed_tx_count = ? WHERE id = ?",
            (3, "pf-tax-free"),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason,
                detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-neu-sale",
                "ws-tax-free",
                "pf-tax-free",
                "missing_price",
                "{}",
                now,
            ),
        )
        conn.commit()

        quarantine_overview = build_overview_snapshot(conn)
        quarantine_tax_free = quarantine_overview["taxFreeBalance"]

        self.assertEqual(quarantine_tax_free["status"], "quarantines")
        self.assertFalse(quarantine_tax_free["needsJournals"])
        self.assertEqual(quarantine_tax_free["quarantines"], 1)

    def test_overview_snapshot_exposes_zero_austrian_tax_free_balance(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-tax-free-zero", "AT Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-tax-free-zero",
                "ws-tax-free-zero",
                "AT Profile",
                "EUR",
                "at",
                9223372036854775807,
                "MOVING_AVERAGE_AT",
                None,
                0,
                now,
            ),
        )
        set_setting(conn, "context_workspace", "ws-tax-free-zero")
        set_setting(conn, "context_profile", "pf-tax-free-zero")
        conn.commit()

        overview = build_overview_snapshot(conn)
        tax_free = overview["taxFreeBalance"]

        self.assertEqual(tax_free["rule"], "austrian_altbestand")
        self.assertEqual(tax_free["status"], "current")
        self.assertEqual(tax_free["taxFreeQuantitySats"], 0)
        self.assertEqual(tax_free["taxableQuantitySats"], 0)
        self.assertEqual(tax_free["totalQuantitySats"], 0)
        self.assertEqual(tax_free["wallets"], [])
        self.assertEqual(
            [bucket["quantitySats"] for bucket in tax_free["buckets"]],
            [0, 0],
        )

    def test_overview_wallet_balance_prefers_processed_book_quantity(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-book-balance", "Book Balance Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-book-balance",
                "ws-book-balance",
                "Book Balance Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-02T00:00:00Z",
                2,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-book",
                "ws-book-balance",
                "pf-book-balance",
                "Cold Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-book-in",
                    "ws-book-balance",
                    "pf-book-balance",
                    "wal-book",
                    "book-in",
                    "fp-book-in",
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("1.0"),
                    0,
                    "EUR",
                    50_000,
                    50_000,
                    "import",
                    "transfer",
                    "Initial funding",
                    "Exchange",
                    None,
                    0,
                    "{}",
                    "2026-01-01T10:00:00Z",
                ),
                (
                    "tx-book-review",
                    "ws-book-balance",
                    "pf-book-balance",
                    "wal-book",
                    "review-out",
                    "fp-book-review",
                    "2026-01-02T12:00:00Z",
                    "2026-01-02T12:10:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("0.4"),
                    btc_to_msat("0.001"),
                    "EUR",
                    50_000,
                    20_000,
                    "import",
                    "payment",
                    "Needs review",
                    "Counterparty",
                    None,
                    0,
                    "{}",
                    "2026-01-02T12:00:00Z",
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-book-in",
                "ws-book-balance",
                "pf-book-balance",
                "tx-book-in",
                "wal-book",
                "2026-01-01T10:00:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("1.0"),
                50_000,
                50_000,
                None,
                None,
                None,
                "Initial funding",
                "2026-01-01T10:00:00Z",
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-book-review",
                "ws-book-balance",
                "pf-book-balance",
                "needs_review",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
            VALUES(?, ?, ?, ?, ?)
            """,
            ("BTC-EUR", "2026-01-02T00:00:00Z", 50_000, "manual", now),
        )
        set_setting(conn, "context_workspace", "ws-book-balance")
        set_setting(conn, "context_profile", "pf-book-balance")
        conn.commit()

        overview = build_overview_snapshot(conn)

        # Raw imported activity is 0.599 BTC, but the processed book still holds
        # 1.0 BTC because the outflow is quarantined. The wallet tile and fiat
        # overview must follow the book/report quantity, not the raw import sum.
        self.assertAlmostEqual(overview["connections"][0]["balance"], 1.0)
        self.assertEqual(overview["balanceSummary"]["status"], "quarantines")
        self.assertEqual(overview["balanceSummary"]["source"], "books")
        self.assertEqual(overview["balanceSummary"]["quarantines"], 1)
        self.assertAlmostEqual(overview["balanceSummary"]["totalBtc"], 1.0)
        self.assertAlmostEqual(overview["balanceSeries"][-1], 1.0)
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 50_000)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["balanceBtc"], 1.0)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["valueEur"], 50_000)
        activity_by_id = {row["id"]: row for row in overview["activityTxs"]}
        self.assertAlmostEqual(activity_by_id["tx-book-review"]["balanceBtc"], 1.0)

    def test_overview_wallet_balance_uses_zero_book_when_all_rows_quarantined(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-all-quarantine", "All Quarantine Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-all-quarantine",
                "ws-all-quarantine",
                "All Quarantine Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-02T00:00:00Z",
                1,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-all-quarantine",
                "ws-all-quarantine",
                "pf-all-quarantine",
                "Review Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-all-quarantine",
                "ws-all-quarantine",
                "pf-all-quarantine",
                "wal-all-quarantine",
                "all-quarantine-in",
                "fp-all-quarantine",
                "2026-01-01T10:00:00Z",
                "2026-01-01T10:10:00Z",
                "inbound",
                "BTC",
                btc_to_msat("0.25"),
                0,
                "EUR",
                None,
                None,
                None,
                "deposit",
                "Needs review",
                None,
                None,
                0,
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-all-quarantine",
                "ws-all-quarantine",
                "pf-all-quarantine",
                "missing_spot_price",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at)
            VALUES(?, ?, ?, ?, ?)
            """,
            ("BTC-EUR", "2026-01-02T00:00:00Z", 50_000, "manual", now),
        )
        set_setting(conn, "context_workspace", "ws-all-quarantine")
        set_setting(conn, "context_profile", "pf-all-quarantine")
        conn.commit()

        overview = build_overview_snapshot(conn)

        self.assertFalse(overview["status"]["needsJournals"])
        self.assertEqual(overview["status"]["quarantines"], 1)
        self.assertEqual(overview["balanceSummary"]["status"], "quarantines")
        self.assertAlmostEqual(overview["balanceSummary"]["totalBtc"], 0.0)
        self.assertAlmostEqual(overview["connections"][0]["balance"], 0.0)
        self.assertAlmostEqual(overview["balanceSeries"][-1], 0.0)
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 0.0)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["balanceBtc"], 0.0)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["valueEur"], 0.0)
        self.assertAlmostEqual(overview["activityTxs"][0]["balanceBtc"], 0.0)

    def test_overview_wallet_balance_uses_raw_transactions_when_journals_are_stale(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-stale-book", "Stale Book Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-stale-book",
                "ws-stale-book",
                "Stale Book Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-02T00:00:00Z",
                1,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-stale", "ws-stale-book", "pf-stale-book", "Stale Wallet", "address", "{}", now),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-stale-old",
                    "ws-stale-book",
                    "pf-stale-book",
                    "wal-stale",
                    "stale-old",
                    "fp-stale-old",
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("1.0"),
                    0,
                    "EUR",
                    50_000,
                    50_000,
                    "import",
                    "deposit",
                    "Processed funding",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-stale-new",
                    "ws-stale-book",
                    "pf-stale-book",
                    "wal-stale",
                    "stale-new",
                    "fp-stale-new",
                    "2026-01-02T10:00:00Z",
                    "2026-01-02T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.25"),
                    0,
                    "EUR",
                    50_000,
                    12_500,
                    "import",
                    "deposit",
                    "Unprocessed funding",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-stale-excluded",
                    "ws-stale-book",
                    "pf-stale-book",
                    "wal-stale",
                    "stale-excluded",
                    "fp-stale-excluded",
                    "2026-01-02T11:00:00Z",
                    "2026-01-02T11:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.5"),
                    0,
                    "EUR",
                    50_000,
                    25_000,
                    "import",
                    "deposit",
                    "Excluded duplicate",
                    None,
                    None,
                    1,
                    "{}",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-stale-old",
                "ws-stale-book",
                "pf-stale-book",
                "tx-stale-old",
                "wal-stale",
                "2026-01-01T10:00:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("1.0"),
                50_000,
                50_000,
                None,
                None,
                None,
                "Processed funding",
                now,
            ),
        )
        conn.execute(
            "INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at) VALUES(?, ?, ?, ?, ?)",
            ("BTC-EUR", "2026-01-02T00:00:00Z", 50_000, "manual", now),
        )
        set_setting(conn, "context_workspace", "ws-stale-book")
        set_setting(conn, "context_profile", "pf-stale-book")
        conn.commit()

        overview = build_overview_snapshot(conn)

        self.assertTrue(overview["status"]["needsJournals"])
        self.assertEqual(overview["balanceSummary"]["status"], "needs_journals")
        self.assertEqual(overview["balanceSummary"]["source"], "transactions")
        self.assertAlmostEqual(overview["balanceSummary"]["totalBtc"], 1.25)
        self.assertAlmostEqual(overview["connections"][0]["balance"], 1.25)
        self.assertAlmostEqual(overview["balanceSeries"][-1], 1.25)
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 62_500)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["balanceBtc"], 1.25)
        self.assertAlmostEqual(overview["portfolioSeries"][-1]["valueEur"], 62_500)
        self.assertAlmostEqual(overview["activityTxs"][-1]["balanceBtc"], 1.25)
        self.assertNotIn(
            "tx-stale-excluded",
            {row["id"] for row in overview["activityTxs"]},
        )

    def test_overview_market_rate_uses_active_book_fiat_currency(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-03-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-usd", "USD Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-usd",
                "ws-usd",
                "USD Book",
                "USD",
                "generic",
                365,
                "FIFO",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at, granularity, method)
            VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "BTC-EUR",
                    "2026-03-01T00:00:00Z",
                    65_000,
                    "manual",
                    "2026-03-01T00:01:00Z",
                    None,
                    None,
                ),
                (
                    "BTC-USD",
                    "2026-03-01T00:00:00Z",
                    70_000,
                    "coingecko",
                    "2026-03-01T00:02:00Z",
                    "daily",
                    "close",
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-usd", "ws-usd", "pf-usd", "USD Wallet", "address", "{}", now),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-usd-in",
                "ws-usd",
                "pf-usd",
                "wal-usd",
                "usd-in",
                "fp-usd-in",
                "2026-03-01T00:00:00Z",
                "2026-03-01T00:00:00Z",
                "inbound",
                "BTC",
                btc_to_msat("2.0"),
                0,
                "USD",
                68_000,
                136_000,
                "import",
                "transfer",
                "USD funding",
                "Exchange",
                None,
                0,
                "{}",
                now,
            ),
        )
        set_setting(conn, "context_workspace", "ws-usd")
        set_setting(conn, "context_profile", "pf-usd")
        conn.commit()

        overview = build_overview_snapshot(conn)

        self.assertEqual(overview["priceEur"], 65_000)
        self.assertEqual(overview["priceUsd"], 70_000)
        self.assertEqual(overview["marketRate"]["fiatCurrency"], "USD")
        self.assertEqual(overview["marketRate"]["pair"], "BTC-USD")
        self.assertEqual(overview["marketRate"]["rate"], 70_000)
        self.assertEqual(overview["marketRate"]["source"], "coingecko")
        self.assertEqual(overview["marketRate"]["fetchedAt"], "2026-03-01T00:02:00Z")
        self.assertEqual(overview["marketRate"]["granularity"], "daily")
        self.assertEqual(overview["marketRate"]["method"], "close")
        self.assertEqual(overview["fiat"]["fiatCurrency"], "USD")
        self.assertEqual(overview["fiat"]["eurBalance"], 140_000)
        self.assertEqual(overview["portfolioSeries"][-1]["valueEur"], 140_000)

    def test_overview_portfolio_series_tracks_cached_daily_market_rates(self):
        self._bootstrap_wallet(label="Daily Rates")
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        profile = conn.execute(
            "SELECT id, workspace_id FROM profiles WHERE label = 'Default'"
        ).fetchone()
        wallet = conn.execute(
            "SELECT id, account_id FROM wallets WHERE label = 'Daily Rates'"
        ).fetchone()
        conn.execute(
            "UPDATE profiles SET fiat_currency = 'EUR' WHERE id = ?",
            (profile["id"],),
        )
        conn.executemany(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at, granularity, method)
            VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                ("BTC-EUR", "2026-01-01T00:00:00Z", 50_000, "manual", now, "daily", "close"),
                ("BTC-EUR", "2026-01-02T00:00:00Z", 60_000, "manual", now, "daily", "close"),
                (
                    "BTC-EUR",
                    "2026-01-02T12:00:00Z",
                    1,
                    "coinbase-exchange",
                    now,
                    "minute",
                    "close",
                ),
                ("BTC-EUR", "2026-01-03T00:00:00Z", 70_000, "manual", now, "daily", "close"),
            ],
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-daily-in",
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                "daily-in",
                "fp-daily-in",
                "2026-01-01T10:00:00Z",
                "2026-01-01T10:10:00Z",
                "inbound",
                "BTC",
                btc_to_msat("1.0"),
                0,
                "EUR",
                50_000,
                50_000,
                "import",
                "transfer",
                "Initial funding",
                "Exchange",
                None,
                0,
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-daily-in",
                profile["workspace_id"],
                profile["id"],
                "tx-daily-in",
                wallet["id"],
                wallet["account_id"],
                "2026-01-01T10:00:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("1.0"),
                50_000,
                50_000,
                None,
                None,
                None,
                "Initial funding",
                now,
            ),
        )
        set_setting(conn, "context_workspace", profile["workspace_id"])
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()

        overview = build_overview_snapshot(conn)
        self.assertEqual(
            [point["date"] for point in overview["portfolioSeries"]],
            ["2026-01-01", "2026-01-02", "2026-01-03"],
        )
        self.assertEqual(
            [point["priceEur"] for point in overview["portfolioSeries"]],
            [50_000, 60_000, 70_000],
        )
        self.assertEqual(
            [point["valueEur"] for point in overview["portfolioSeries"]],
            [50_000, 60_000, 70_000],
        )
        self.assertEqual(
            [point["balanceBtc"] for point in overview["portfolioSeries"]],
            [1.0, 1.0, 1.0],
        )

    def test_overview_portfolio_series_labels_kraken_daily_rates_by_effective_day(self):
        self._bootstrap_wallet(label="Kraken Daily")
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        profile = conn.execute(
            "SELECT id, workspace_id FROM profiles WHERE label = 'Default'"
        ).fetchone()
        wallet = conn.execute(
            "SELECT id, account_id FROM wallets WHERE label = 'Kraken Daily'"
        ).fetchone()
        conn.execute(
            "UPDATE profiles SET fiat_currency = 'EUR' WHERE id = ?",
            (profile["id"],),
        )
        conn.executemany(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at, granularity, method)
            VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                ("BTC-EUR", "2026-01-02T00:00:00Z", 50_000, "kraken-csv", now, "daily", "ohlcvt_csv"),
                ("BTC-EUR", "2026-01-03T00:00:00Z", 60_000, "kraken-csv", now, "daily", "ohlcvt_csv"),
            ],
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-kraken-daily-in",
                profile["workspace_id"],
                profile["id"],
                wallet["id"],
                "kraken-daily-in",
                "fp-kraken-daily-in",
                "2026-01-01T10:00:00Z",
                "2026-01-01T10:10:00Z",
                "inbound",
                "BTC",
                btc_to_msat("1.0"),
                0,
                "EUR",
                50_000,
                50_000,
                "rates_cache",
                "transfer",
                "Initial funding",
                "Exchange",
                None,
                0,
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-kraken-daily-in",
                profile["workspace_id"],
                profile["id"],
                "tx-kraken-daily-in",
                wallet["id"],
                wallet["account_id"],
                "2026-01-01T10:00:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("1.0"),
                50_000,
                50_000,
                None,
                None,
                None,
                "Initial funding",
                now,
            ),
        )
        set_setting(conn, "context_workspace", profile["workspace_id"])
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()

        overview = build_overview_snapshot(conn)

        self.assertEqual(
            [point["date"] for point in overview["portfolioSeries"]],
            ["2026-01-01", "2026-01-02"],
        )
        self.assertEqual(
            [point["priceEur"] for point in overview["portfolioSeries"]],
            [50_000, 60_000],
        )
        self.assertEqual(
            [point["priceTimestamp"] for point in overview["portfolioSeries"]],
            ["2026-01-02T00:00:00Z", "2026-01-03T00:00:00Z"],
        )

    def test_overview_portfolio_series_treats_btc_lbtc_swaps_as_bitcoin_balance(self):
        self._bootstrap_wallet(label="BTC Wallet")
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        profile = conn.execute(
            "SELECT id, workspace_id FROM profiles WHERE label = 'Default'"
        ).fetchone()
        btc_wallet = conn.execute(
            "SELECT id FROM wallets WHERE label = 'BTC Wallet'"
        ).fetchone()
        conn.execute(
            "UPDATE profiles SET fiat_currency = 'EUR' WHERE id = ?",
            (profile["id"],),
        )
        conn.execute(
            """
            INSERT INTO accounts(id, workspace_id, profile_id, code, label, account_type, asset, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "acct-liquid-overview",
                profile["workspace_id"],
                profile["id"],
                "asset:lbtc-overview",
                "Liquid Overview",
                "asset",
                "LBTC",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(id, workspace_id, profile_id, account_id, label, kind, config_json, created_at)
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wallet-liquid-overview",
                profile["workspace_id"],
                profile["id"],
                "acct-liquid-overview",
                "Liquid Wallet",
                "descriptor",
                "{}",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO rates_cache(pair, timestamp, rate, source, fetched_at, granularity, method)
            VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                ("BTC-EUR", "2026-01-01T00:00:00Z", 62_000, "manual", now, "daily", "close"),
                ("BTC-EUR", "2026-01-02T00:00:00Z", 62_000, "manual", now, "daily", "close"),
                ("BTC-EUR", "2026-01-03T00:00:00Z", 62_000, "manual", now, "daily", "close"),
            ],
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "lbtc-before-peg-out",
                    profile["workspace_id"],
                    profile["id"],
                    "wallet-liquid-overview",
                    "lbtc-before-peg-out",
                    "fp-lbtc-before-peg-out",
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T10:10:00Z",
                    "inbound",
                    "LBTC",
                    btc_to_msat("0.12426275"),
                    0,
                    "EUR",
                    62_000,
                    7704.2905,
                    "import",
                    "deposit",
                    "Initial Liquid balance",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "peg-out-lbtc",
                    profile["workspace_id"],
                    profile["id"],
                    "wallet-liquid-overview",
                    "peg-out-lbtc",
                    "fp-peg-out-lbtc",
                    "2026-01-02T10:00:00Z",
                    "2026-01-02T10:10:00Z",
                    "outbound",
                    "LBTC",
                    btc_to_msat("0.12426275"),
                    0,
                    "EUR",
                    62_000,
                    -7704.2905,
                    "import",
                    "swap",
                    "Peg-out send leg",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "peg-out-btc",
                    profile["workspace_id"],
                    profile["id"],
                    btc_wallet["id"],
                    "peg-out-btc",
                    "fp-peg-out-btc",
                    "2026-01-02T10:05:00Z",
                    "2026-01-02T10:15:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.12413298"),
                    0,
                    "EUR",
                    62_000,
                    7696.24476,
                    "import",
                    "swap",
                    "Peg-out receive leg",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "swap-payment-out",
                    profile["workspace_id"],
                    profile["id"],
                    btc_wallet["id"],
                    "swap-payment-out",
                    "fp-swap-payment-out",
                    "2026-01-03T10:00:00Z",
                    "2026-01-03T10:10:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("0.01000000"),
                    0,
                    "EUR",
                    62_000,
                    -620,
                    "import",
                    "swap",
                    "Swap payment out",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id, in_transaction_id,
                kind, policy, swap_fee_msat, swap_fee_kind, pair_source, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pair-overview-peg-out",
                profile["workspace_id"],
                profile["id"],
                "peg-out-lbtc",
                "peg-out-btc",
                "peg-out",
                "carrying-value",
                btc_to_msat("0.00012977"),
                "combined",
                "manual",
                now,
            ),
        )
        set_setting(conn, "context_workspace", profile["workspace_id"])
        set_setting(conn, "context_profile", profile["id"])
        conn.commit()

        overview = build_overview_snapshot(conn)

        self.assertEqual(
            [point["date"] for point in overview["portfolioSeries"]],
            ["2026-01-01", "2026-01-02", "2026-01-03"],
        )
        self.assertAlmostEqual(
            overview["portfolioSeries"][0]["balanceBtc"],
            0.12426275,
            places=8,
        )
        self.assertAlmostEqual(
            overview["portfolioSeries"][1]["balanceBtc"],
            0.12413298,
            places=8,
        )
        self.assertAlmostEqual(
            overview["portfolioSeries"][2]["balanceBtc"],
            0.11413298,
            places=8,
        )
        self.assertLess(
            overview["portfolioSeries"][1]["balanceBtc"],
            overview["portfolioSeries"][0]["balanceBtc"],
        )
        self.assertLess(
            overview["portfolioSeries"][2]["balanceBtc"],
            overview["portfolioSeries"][1]["balanceBtc"],
        )
        payment_activity = [
            row for row in overview["activityTxs"] if row["id"] == "swap-payment-out"
        ]
        self.assertEqual(len(payment_activity), 1)
        self.assertEqual(payment_activity[0]["type"], "Swap")
        self.assertNotIn("pair", payment_activity[0])
        self.assertAlmostEqual(payment_activity[0]["balanceBtc"], 0.11413298)
        self.assertAlmostEqual(overview["fiat"]["eurBalance"], 7076.24476, places=4)

    def test_overview_connection_balances_use_book_quantity_when_journals_are_partial(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-ui-raw", "UI Raw Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-ui-raw",
                "ws-ui-raw",
                "UI Raw Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-02T00:00:00Z",
                2,
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                ("wal-onchain", "ws-ui-raw", "pf-ui-raw", "Onchain", "descriptor", "{}", now),
                ("wal-other", "ws-ui-raw", "pf-ui-raw", "Other", "address", "{}", now),
            ],
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-onchain",
                    "ws-ui-raw",
                    "pf-ui-raw",
                    "wal-onchain",
                    "onchain-quarantined",
                    "fp-onchain",
                    "2026-01-01T10:00:00Z",
                    "2026-01-01T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.25"),
                    0,
                    "EUR",
                    None,
                    None,
                    None,
                    "deposit",
                    "Synced onchain funds",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-other",
                    "ws-ui-raw",
                    "pf-ui-raw",
                    "wal-other",
                    "other-journaled",
                    "fp-other",
                    "2026-01-01T11:00:00Z",
                    "2026-01-01T11:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.10"),
                    0,
                    "EUR",
                    50_000,
                    5_000,
                    "import",
                    "deposit",
                    "Other funds",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-other",
                "ws-ui-raw",
                "pf-ui-raw",
                "tx-other",
                "wal-other",
                "2026-01-01T11:00:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("0.10"),
                5_000,
                50_000,
                5_000,
                None,
                None,
                "Other funds",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            ("tx-onchain", "ws-ui-raw", "pf-ui-raw", "missing_spot_price", "{}", now),
        )
        set_setting(conn, "context_workspace", "ws-ui-raw")
        set_setting(conn, "context_profile", "pf-ui-raw")
        conn.commit()

        overview = build_overview_snapshot(conn)
        balances = {
            connection["label"]: connection["balance"]
            for connection in overview["connections"]
        }
        self.assertFalse(overview["status"]["needsJournals"])
        self.assertEqual(overview["status"]["quarantines"], 1)
        self.assertAlmostEqual(balances["Onchain"], 0.0)
        self.assertAlmostEqual(balances["Other"], 0.10)

    def test_rates_coverage_ignores_zero_amount_rate_only_rows(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-zero-rate", "Zero Rate Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, journal_input_version,
                last_processed_input_version, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-zero-rate",
                "ws-zero-rate",
                "Zero Rate Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                now,
                2,
                0,
                0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-zero-rate",
                "ws-zero-rate",
                "pf-zero-rate",
                "Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-zero-rate",
                "ws-zero-rate",
                "pf-zero-rate",
                "wal-zero-rate",
                "zero-rate-ext",
                "zero-rate-fp",
                "2026-01-02T00:00:00Z",
                "2026-01-02T00:05:00Z",
                "outbound",
                "BTC",
                0,
                0,
                "EUR",
                65_000,
                None,
                "rates-cache",
                "payment",
                "Zero amount marker",
                0.0,
                0.0,
                0,
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source,
                fiat_rate_exact, fiat_value_exact, kind, description, counterparty,
                note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-exact-rate",
                "ws-zero-rate",
                "pf-zero-rate",
                "wal-zero-rate",
                "exact-rate-ext",
                "exact-rate-fp",
                "2026-01-03T00:00:00Z",
                "2026-01-03T00:05:00Z",
                "inbound",
                "BTC",
                btc_to_msat("0.25"),
                0,
                "EUR",
                None,
                None,
                "import",
                "65000",
                None,
                "deposit",
                "Exact-rate priced row",
                None,
                None,
                0,
                "{}",
                now,
            ),
        )
        set_setting(conn, "context_workspace", "ws-zero-rate")
        set_setting(conn, "context_profile", "pf-zero-rate")
        conn.commit()

        coverage = build_rates_coverage_snapshot(conn, {"limit": 5})
        self.assertEqual(coverage["summary"]["active_transactions"], 2)
        self.assertEqual(coverage["summary"]["priced_transactions"], 2)
        self.assertEqual(coverage["summary"]["missing_price_transactions"], 0)
        self.assertEqual(coverage["summary"]["cache_coverable_missing"], 0)
        self.assertEqual(coverage["items"], [])

        blockers = build_report_blockers_snapshot(conn)
        self.assertTrue(blockers["ready"])
        self.assertEqual(blockers["blockers"], [])

    def test_missing_price_repair_points_to_rate_rebuild(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-missing-price-action", "Missing Price Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, journal_input_version,
                last_processed_input_version, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-missing-price-action",
                "ws-missing-price-action",
                "Missing Price Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                now,
                1,
                0,
                0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-missing-price-action",
                "ws-missing-price-action",
                "pf-missing-price-action",
                "Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-missing-price-action",
                "ws-missing-price-action",
                "pf-missing-price-action",
                "wal-missing-price-action",
                "missing-price-action-ext",
                "missing-price-action-fp",
                "2026-01-02T00:00:00Z",
                "2026-01-02T00:05:00Z",
                "inbound",
                "BTC",
                btc_to_msat("0.25"),
                0,
                "EUR",
                None,
                None,
                None,
                "deposit",
                "Incoming payment",
                None,
                None,
                0,
                "{}",
                now,
            ),
        )
        set_setting(conn, "context_workspace", "ws-missing-price-action")
        set_setting(conn, "context_profile", "pf-missing-price-action")
        conn.commit()

        blockers = build_report_blockers_snapshot(conn)
        self.assertFalse(blockers["ready"])
        self.assertEqual(blockers["blockers"][0]["id"], "missing_prices")
        self.assertEqual(blockers["blockers"][0]["daemon_kind"], "ui.rates.rebuild")

        next_actions = build_next_actions_snapshot(conn)
        self.assertEqual(next_actions["suggestions"][0]["id"], "fetch_missing_prices")
        self.assertEqual(
            next_actions["suggestions"][0]["daemon_kind"],
            "ui.rates.rebuild",
        )

        transactions = build_transactions_snapshot(conn, {"limit": 1})
        self.assertIsNone(transactions["txs"][0]["eur"])
        self.assertIsNone(transactions["txs"][0]["rate"])

        core_rates.upsert_rate(
            conn,
            "BTC-EUR",
            "2026-01-02T00:05:00Z",
            "65000.00",
            core_rates.RATE_SOURCE_COINBASE_EXCHANGE,
            fetched_at=now,
            granularity="minute",
            method="product_candles",
        )
        processed = process_journals(conn, None, None)
        self.assertEqual(processed["auto_priced"], 1)
        self.assertEqual(processed["quarantined"], 0)
        priced = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider
            FROM transactions
            WHERE id = 'tx-missing-price-action'
            """
        ).fetchone()
        self.assertEqual(priced["fiat_rate_exact"], "65000.00")
        self.assertEqual(priced["fiat_value_exact"], "16250.0000")
        self.assertEqual(priced["pricing_source_kind"], pricing.SOURCE_FMV_PROVIDER)
        self.assertEqual(priced["pricing_provider"], core_rates.RATE_SOURCE_COINBASE_EXCHANGE)

    def test_multi_pair_leg_does_not_multiply_journal_events(self):
        # A whirlpool-style out leg carries N active pairs since the multi-pair
        # feature dropped the per-leg UNIQUE indexes. The journal event list
        # joins pair metadata per leg — it must pick ONE representative pair,
        # not multiply every journal entry of the transaction N times.
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-multi-pair", "Multi Pair Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, journal_input_version,
                last_processed_input_version, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-multi-pair",
                "ws-multi-pair",
                "Multi Pair Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                now,
                3,
                0,
                0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-multi-pair",
                "ws-multi-pair",
                "pf-multi-pair",
                "Multi Pair Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-mp-out",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "wal-multi-pair",
                    "mp-out",
                    "fp-mp-out",
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("1.0"),
                    0,
                    "EUR",
                    50_000,
                    50_000,
                    None,
                    "payment",
                    None,
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-mp-in-1",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "wal-multi-pair",
                    "mp-in-1",
                    "fp-mp-in-1",
                    "2026-01-01T00:01:00Z",
                    "2026-01-01T00:01:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.5"),
                    0,
                    "EUR",
                    50_000,
                    25_000,
                    None,
                    "receive",
                    None,
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-mp-in-2",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "wal-multi-pair",
                    "mp-in-2",
                    "fp-mp-in-2",
                    "2026-01-01T00:01:00Z",
                    "2026-01-01T00:01:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.4999"),
                    0,
                    "EUR",
                    50_000,
                    24_995,
                    None,
                    "receive",
                    None,
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO transaction_pairs(
                id, workspace_id, profile_id, out_transaction_id,
                in_transaction_id, kind, policy, swap_fee_msat, pair_source,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "pair-mp-1",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "tx-mp-out",
                    "tx-mp-in-1",
                    "whirlpool",
                    "carrying-value",
                    None,
                    "manual",
                    now,
                ),
                (
                    "pair-mp-2",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "tx-mp-out",
                    "tx-mp-in-2",
                    "whirlpool",
                    "carrying-value",
                    None,
                    "manual",
                    "2026-01-01T00:00:01Z",
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-mp-out",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "tx-mp-out",
                    "wal-multi-pair",
                    "2026-01-01T00:00:00Z",
                    "transfer_out",
                    "BTC",
                    -btc_to_msat("1.0"),
                    -50_000,
                    now,
                ),
                (
                    "je-mp-fee",
                    "ws-multi-pair",
                    "pf-multi-pair",
                    "tx-mp-out",
                    "wal-multi-pair",
                    "2026-01-01T00:00:00Z",
                    "transfer_fee",
                    "BTC",
                    -btc_to_msat("0.0001"),
                    -5,
                    now,
                ),
            ],
        )
        set_setting(conn, "context_workspace", "ws-multi-pair")
        set_setting(conn, "context_profile", "pf-multi-pair")
        conn.commit()

        events = build_journal_events_list_snapshot(conn, {"limit": 10})
        # Two journal entries on the out tx, NOT multiplied by the two pairs.
        self.assertEqual(events["summary"]["count"], 2)
        self.assertEqual(len(events["events"]), 2)
        # Legacy compatibility rows are not display authority. They neither
        # multiply journal rows nor appear as booked pair metadata.
        self.assertEqual(
            {event["pair"] for event in events["events"]},
            {None},
        )

        journals = build_journals_snapshot(conn)
        self.assertEqual(len(journals["recent"]), 2)

        # Transaction-list display metadata no longer reads legacy pair rows;
        # only the current stored projection may group these rows.
        window_rows = conn.execute(
            "SELECT id FROM transactions WHERE id = 'tx-mp-in-1'"
        ).fetchall()
        meta = _transaction_pair_display_meta(conn, window_rows)
        self.assertEqual(meta, {})

    def test_journal_pair_payload_picks_one_pair_for_chain_edge_case(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-pair-chain", "Pair Chain Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, journal_input_version,
                last_processed_input_version, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-pair-chain",
                "ws-pair-chain",
                "Pair Chain Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                now,
                3,
                0,
                0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-pair-chain",
                "ws-pair-chain",
                "pf-pair-chain",
                "Chain Wallet",
                "address",
                "{}",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-chain-a",
                    "ws-pair-chain",
                    "pf-pair-chain",
                    "wal-pair-chain",
                    "chain-a",
                    "fp-chain-a",
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:01:00Z",
                    "outbound",
                    "LBTC",
                    btc_to_msat("0.5"),
                    0,
                    "EUR",
                    50_000,
                    25_000,
                    "import",
                    "withdrawal",
                    "First leg",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-chain-middle",
                    "ws-pair-chain",
                    "pf-pair-chain",
                    "wal-pair-chain",
                    "chain-middle",
                    "fp-chain-middle",
                    "2026-01-01T00:02:00Z",
                    "2026-01-01T00:03:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.49"),
                    0,
                    "EUR",
                    50_000,
                    24_500,
                    "import",
                    "deposit",
                    "Middle leg",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-chain-c",
                    "ws-pair-chain",
                    "pf-pair-chain",
                    "wal-pair-chain",
                    "chain-c",
                    "fp-chain-c",
                    "2026-01-01T00:04:00Z",
                    "2026-01-01T00:05:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.48"),
                    0,
                    "EUR",
                    50_000,
                    24_000,
                    "import",
                    "deposit",
                    "Second receive",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_custody_economic_relations(
                relation_id, workspace_id, profile_id, relation_kind,
                source_transaction_id, target_transaction_id,
                source_asset, target_asset, source_amount_msat,
                target_amount_msat, review_kind, policy, swap_fee_msat,
                basis_state, occurred_at, target_occurred_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "a" * 64,
                "ws-pair-chain",
                "pf-pair-chain",
                "conversion",
                "tx-chain-a",
                "tx-chain-middle",
                "LBTC",
                "BTC",
                btc_to_msat("0.5"),
                btc_to_msat("0.49"),
                "peg-out",
                "carrying-value",
                btc_to_msat("0.01"),
                "eligible",
                "2026-01-01T00:00:00Z",
                "2026-01-01T00:02:00Z",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_custody_decisions(
                decision_id, workspace_id, profile_id,
                source_transaction_id, target_transaction_id,
                source_observation_hash, source_start_msat, source_end_msat,
                target_observation_hash, target_start_msat, target_end_msat,
                source_wallet_id, target_wallet_id,
                source_network, target_network, source_rail, target_rail,
                source_asset, target_asset, state, basis_state, reason,
                occurred_at, target_occurred_at, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "b" * 64,
                "ws-pair-chain",
                "pf-pair-chain",
                "tx-chain-middle",
                "tx-chain-c",
                "1" * 64,
                0,
                btc_to_msat("0.48"),
                "2" * 64,
                0,
                btc_to_msat("0.48"),
                "wal-pair-chain",
                "wal-pair-chain",
                "main",
                "main",
                "bitcoin",
                "bitcoin",
                "BTC",
                "BTC",
                "internal_reviewed",
                "eligible",
                "reviewed_custody_component",
                "2026-01-01T00:02:00Z",
                "2026-01-01T00:04:00Z",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-chain-middle",
                "ws-pair-chain",
                "pf-pair-chain",
                "tx-chain-middle",
                "wal-pair-chain",
                "2026-01-01T00:02:00Z",
                "acquisition",
                "BTC",
                btc_to_msat("0.49"),
                24_500,
                now,
            ),
        )
        set_setting(conn, "context_workspace", "ws-pair-chain")
        set_setting(conn, "context_profile", "pf-pair-chain")
        conn.commit()

        events = build_journal_events_list_snapshot(conn, {"limit": 10})
        self.assertEqual(events["summary"]["count"], 1)
        self.assertEqual(len(events["events"]), 1)
        self.assertEqual(events["events"][0]["pair"]["pairId"], "b" * 64)
        self.assertEqual(
            events["events"][0]["pair"]["out"]["amountMsat"],
            btc_to_msat("0.48"),
        )

        journals = build_journals_snapshot(conn)
        self.assertEqual(len(journals["recent"]), 1)
        self.assertEqual(journals["recent"][0]["pair"]["pairId"], "b" * 64)
        self.assertEqual(
            journals["recent"][0]["pair"]["out"]["amountMsat"],
            btc_to_msat("0.48"),
        )

    def test_ui_snapshots_show_reviewed_swap_movement_with_fee(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-swap-ui", "Swap UI Workspace", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-swap-ui",
                "ws-swap-ui",
                "Swap UI Profile",
                "EUR",
                "generic",
                365,
                "FIFO",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "wal-swap-btc",
                    "ws-swap-ui",
                    "pf-swap-ui",
                    "BTC Wallet",
                    "address",
                    "{}",
                    now,
                ),
                (
                    "wal-swap-lbtc",
                    "ws-swap-ui",
                    "pf-swap-ui",
                    "Liquid Wallet",
                    "address",
                    "{}",
                    now,
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "swap-out-leg",
                    "ws-swap-ui",
                    "pf-swap-ui",
                    "wal-swap-btc",
                    "swap-out-ext",
                    "swap-out-fp",
                    "2026-03-01T10:00:00Z",
                    "2026-03-01T10:10:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("0.10000000"),
                    0,
                    "EUR",
                    65_000,
                    6_500,
                    "import",
                    "payment",
                    "Swap send leg",
                    None,
                    None,
                    0,
                    "{}",
                    "2026-03-01T10:00:00Z",
                ),
                (
                    "swap-in-leg",
                    "ws-swap-ui",
                    "pf-swap-ui",
                    "wal-swap-lbtc",
                    "swap-in-ext",
                    "swap-in-fp",
                    "2026-03-01T10:05:00Z",
                    "2026-03-01T10:15:00Z",
                    "inbound",
                    "LBTC",
                    btc_to_msat("0.09990000"),
                    0,
                    "EUR",
                    0,
                    0,
                    "import",
                    "deposit",
                    "Swap receive leg",
                    None,
                    None,
                    0,
                    "{}",
                    "2026-03-01T10:05:00Z",
                ),
                (
                    "older-income",
                    "ws-swap-ui",
                    "pf-swap-ui",
                    "wal-swap-btc",
                    "older-income-ext",
                    "older-income-fp",
                    "2026-02-01T10:00:00Z",
                    "2026-02-01T10:10:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.01000000"),
                    0,
                    "EUR",
                    60_000,
                    600,
                    "import",
                    "deposit",
                    "Older income",
                    None,
                    None,
                    0,
                    "{}",
                    "2026-02-01T10:00:00Z",
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_custody_economic_relations(
                relation_id, workspace_id, profile_id, relation_kind,
                source_transaction_id, target_transaction_id,
                source_asset, target_asset, source_amount_msat,
                target_amount_msat, review_kind, policy, swap_fee_msat,
                swap_fee_kind, basis_state, occurred_at, target_occurred_at,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "c" * 64,
                "ws-swap-ui",
                "pf-swap-ui",
                "conversion",
                "swap-out-leg",
                "swap-in-leg",
                "BTC",
                "LBTC",
                btc_to_msat("0.10000000"),
                btc_to_msat("0.09990000"),
                "manual",
                "carrying-value",
                btc_to_msat("0.00010000"),
                "deducted",
                "eligible",
                "2026-03-01T10:00:00Z",
                "2026-03-01T10:05:00Z",
                now,
            ),
        )
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?, last_processed_tx_count = 3,
                last_processed_input_version = journal_input_version
            WHERE id = 'pf-swap-ui'
            """,
            (now,),
        )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, created_at
            ) VALUES(?, 'ws-swap-ui', 'pf-swap-ui', ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-older-income",
                    "older-income",
                    "wal-swap-btc",
                    "2026-02-01T10:00:00Z",
                    "acquisition",
                    "BTC",
                    btc_to_msat("0.01000000"),
                    600,
                    now,
                ),
                (
                    "je-swap-out",
                    "swap-out-leg",
                    "wal-swap-btc",
                    "2026-03-01T10:00:00Z",
                    "disposal",
                    "BTC",
                    -btc_to_msat("0.10000000"),
                    -6_500,
                    now,
                ),
                (
                    "je-swap-in",
                    "swap-in-leg",
                    "wal-swap-lbtc",
                    "2026-03-01T10:05:00Z",
                    "acquisition",
                    "LBTC",
                    btc_to_msat("0.09990000"),
                    0,
                    now,
                ),
            ],
        )
        set_setting(conn, "context_workspace", "ws-swap-ui")
        set_setting(conn, "context_profile", "pf-swap-ui")
        conn.commit()

        overview = build_overview_snapshot(conn)
        overview_swap_rows = [
            row
            for row in overview["txs"]
            if row["id"] in {"swap-in-leg", "swap-out-leg"}
        ]
        self.assertEqual(len(overview_swap_rows), 1)
        overview_swap = overview_swap_rows[0]
        self.assertEqual(overview_swap["id"], "swap-in-leg")
        self.assertEqual(overview_swap["type"], "Swap")
        self.assertEqual(overview_swap["tag"], "Swap")
        self.assertNotIn("tags", overview_swap)
        self.assertEqual(overview_swap["account"], "BTC Wallet -> Liquid Wallet")
        self.assertEqual(overview_swap["counter"], "Swap BTC -> LBTC")
        self.assertEqual(overview_swap["amountSat"], -10_000_000)
        self.assertEqual(overview_swap["feeSat"], 10_000)
        self.assertAlmostEqual(overview_swap["eur"], -6500.0)
        self.assertEqual(overview_swap["pair"]["outAmountSat"], 10_000_000)
        self.assertEqual(overview_swap["pair"]["inAmountSat"], 9_990_000)
        self.assertEqual(overview_swap["pair"]["feeSat"], 10_000)
        overview_activity_swap_rows = [
            row
            for row in overview["activityTxs"]
            if row["id"] in {"swap-in-leg", "swap-out-leg"}
        ]
        self.assertEqual(len(overview_activity_swap_rows), 1)
        self.assertEqual(overview_activity_swap_rows[0]["id"], "swap-in-leg")
        self.assertAlmostEqual(overview_activity_swap_rows[0]["balanceBtc"], 0.0099)
        connection_by_label = {
            connection["label"]: connection
            for connection in overview["connections"]
        }
        self.assertEqual(connection_by_label["Liquid Wallet"]["transactionCount"], 1)
        self.assertAlmostEqual(connection_by_label["Liquid Wallet"]["balance"], 0.0999)

        transactions = build_transactions_snapshot(conn, {"limit": 10})
        swap_rows = [
            row
            for row in transactions["txs"]
            if row["id"] in {"swap-in-leg", "swap-out-leg"}
        ]
        self.assertEqual(len(swap_rows), 1)
        self.assertEqual(swap_rows[0]["type"], "Swap")
        self.assertNotEqual(swap_rows[0]["type"], "Income")
        self.assertEqual(swap_rows[0]["amountSat"], -10_000_000)
        self.assertEqual(swap_rows[0]["feeSat"], 10_000)

        ascending = build_transactions_snapshot(
            conn,
            {"limit": 10, "sort": "occurred-at", "order": "asc"},
        )
        ascending_swap = [
            row
            for row in ascending["txs"]
            if row["id"] in {"swap-in-leg", "swap-out-leg"}
        ]
        self.assertEqual(len(ascending_swap), 1)
        self.assertEqual(ascending_swap[0]["id"], "swap-out-leg")
        self.assertAlmostEqual(ascending_swap[0]["eur"], -6500.0)

        limited = build_transactions_snapshot(conn, {"limit": 2})
        self.assertEqual(
            [row["id"] for row in limited["txs"]],
            ["swap-in-leg", "older-income"],
        )
        first_page = build_transactions_snapshot(conn, {"limit": 1})
        self.assertEqual([row["id"] for row in first_page["txs"]], ["swap-in-leg"])
        self.assertTrue(first_page["hasMore"])
        self.assertTrue(first_page["nextCursor"])
        second_page = build_transactions_snapshot(
            conn,
            {"limit": 1, "cursor": first_page["nextCursor"]},
        )
        self.assertEqual([row["id"] for row in second_page["txs"]], ["older-income"])
        self.assertFalse(second_page["hasMore"])
        self.assertIsNone(second_page["nextCursor"])
        with self.assertRaises(AppError) as changed_cursor_filter:
            build_transactions_snapshot(
                conn,
                {
                    "limit": 1,
                    "cursor": first_page["nextCursor"],
                    "asset": "BTC",
                },
            )
        self.assertEqual(changed_cursor_filter.exception.code, "validation")

        outbound = build_transactions_snapshot(
            conn,
            {"limit": 10, "direction": "outbound"},
        )
        self.assertEqual(len(outbound["txs"]), 1)
        self.assertEqual(outbound["txs"][0]["id"], "swap-out-leg")
        self.assertEqual(outbound["txs"][0]["type"], "Swap")
        self.assertEqual(outbound["txs"][0]["amountSat"], -10_000_000)
        self.assertAlmostEqual(outbound["txs"][0]["eur"], -6500.0)

        outbound_search = build_transactions_search_snapshot(
            conn,
            {"query": "swap-out-ext", "limit": 10},
        )
        self.assertEqual(len(outbound_search["txs"]), 1)
        self.assertEqual(outbound_search["txs"][0]["id"], "swap-out-leg")
        self.assertEqual(outbound_search["txs"][0]["type"], "Swap")
        list_search = build_transactions_snapshot(
            conn,
            {"query": "older income", "limit": 10},
        )
        self.assertEqual([row["id"] for row in list_search["txs"]], ["older-income"])
        self.assertEqual(list_search["filters"]["query"], "older income")

    def test_ui_transactions_snapshot_cursor_roundtrips_sort_ties(self):
        self._bootstrap_wallet(label="Cursor Sort")
        for index, tx_id in enumerate(
            [
                "cursor-sort-a",
                "cursor-sort-b",
                "cursor-sort-c",
                "cursor-sort-d",
            ],
        ):
            self._insert_transaction(
                wallet_label="Cursor Sort",
                tx_id=tx_id,
                occurred_at=f"2024-01-01T00:00:0{index}Z",
                amount_msat=100_000_000,
            )

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            UPDATE transactions
            SET fee = ?, fiat_value = ?
            WHERE id LIKE 'cursor-sort-%'
            """,
            (1_000, 42.0),
        )
        conn.commit()
        self.addCleanup(conn.close)

        for sort in ("occurred-at", "amount", "fee", "fiat-value"):
            for order in ("asc", "desc"):
                with self.subTest(sort=sort, order=order):
                    first_page = build_transactions_snapshot(
                        conn,
                        {
                            "query": "cursor-sort",
                            "limit": 2,
                            "sort": sort,
                            "order": order,
                        },
                    )
                    self.assertTrue(first_page["hasMore"])
                    self.assertTrue(first_page["nextCursor"])
                    second_page = build_transactions_snapshot(
                        conn,
                        {
                            "query": "cursor-sort",
                            "limit": 2,
                            "sort": sort,
                            "order": order,
                            "cursor": first_page["nextCursor"],
                        },
                    )
                    ids = [
                        row["id"]
                        for row in [*first_page["txs"], *second_page["txs"]]
                    ]
                    self.assertEqual(len(ids), 4)
                    self.assertEqual(len(set(ids)), 4)
                    self.assertEqual(set(ids), {
                        "cursor-sort-a",
                        "cursor-sort-b",
                        "cursor-sort-c",
                        "cursor-sort-d",
                    })
                    self.assertFalse(second_page["hasMore"])
                    self.assertIsNone(second_page["nextCursor"])

    def test_report_transfer_rows_treat_null_swap_fee_as_zero(self):
        context = {
            "query_rows": {
                "transfer_pairs": [
                    {
                        "id": "pair-old-null-fee",
                        "kind": "submarine-swap",
                        "policy": "carrying-value",
                        "swap_fee_msat": None,
                        "swap_fee_kind": "combined",
                        "out_occurred_at": "2026-03-01T10:00:00Z",
                        "out_wallet": "BTC Wallet",
                        "out_transaction_id": "swap-out-leg",
                        "out_asset": "BTC",
                        "out_amount": btc_to_msat("0.10000000"),
                        "out_fee": 0,
                        "in_occurred_at": "2026-03-01T10:05:00Z",
                        "in_wallet": "Liquid Wallet",
                        "in_transaction_id": "swap-in-leg",
                        "in_asset": "LBTC",
                        "in_amount": btc_to_msat("0.09990000"),
                        "in_fee": 0,
                        "notes": "",
                        "created_at": "2026-03-01T10:06:00Z",
                    }
                ]
            }
        }

        rows = _generic_report_transfer_pair_rows(context)

        self.assertEqual(rows[0]["swap_fee_msat"], 0)
        self.assertEqual(rows[0]["swap_fee"], 0.0)

    def test_river_import_rejects_price_currency_mismatch(self):
        self._bootstrap_austrian_e1kv_wallet(label="RiverEUR")
        river_csv = self.case_dir / "river-usd.csv"
        river_csv.write_text(
            "\n".join(
                [
                    "Date,Reference Code,Transaction Type,Sent Amount,Sent Currency,Received Amount,Received Currency,Fee Amount,Fee Currency,Total Amount,Total Currency,Method,Source,Destination,Cost Basis Amount,Cost Basis Currency,Bitcoin Price Amount,Bitcoin Price Currency,Transaction ID,Recurring,Tag",
                    "2026-01-02T12:00:00Z,RIV-USD-1,Buy,1000.00,USD,0.01000000,BTC,5.00,USD,-1005.00,USD,ACH,Linked bank,Bitcoin balance,,,100000.00,USD,,False,Buy",
                    "",
                ]
            ),
            encoding="utf-8",
        )

        payload, result = self._run_json(
            "wallets",
            "import-river",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "RiverEUR",
            "--file",
            str(river_csv),
        )

        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("USD", payload["error"]["message"])
        self.assertIn("EUR", payload["error"]["message"])

    def test_river_parser_accepts_xbt_amount_suffix(self):
        record = normalize_river_record(
            {
                "Date": "2026-01-02T12:00:00Z",
                "Reference Code": "RIV-XBT-1",
                "Transaction Type": "Interest",
                "Sent Amount": "",
                "Sent Currency": "",
                "Received Amount": "0.01000000 XBT",
                "Received Currency": "XBT",
                "Fee Amount": "0 XBT",
                "Fee Currency": "XBT",
                "Bitcoin Price Amount": "100000.00 USD",
                "Bitcoin Price Currency": "USD",
                "Tag": "Interest",
            }
        )

        self.assertIsNotNone(record)
        self.assertEqual(record["amount"], Decimal("0.01000000"))
        self.assertEqual(record["fee"], Decimal("0"))
        self.assertEqual(record["pricing_pair"], "BTC-USD")

    def test_capital_gains_snapshot_uses_latest_reportable_year_and_forms(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-at-years", "Main", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-at-years",
                "ws-at-years",
                "Default",
                "EUR",
                "at",
                365,
                "moving_average_at",
                "2026-01-01T01:00:00Z",
                3,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-at-years", "ws-at-years", "pf-at-years", "Cold", "address", "{}", now),
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-alt-sale",
                    "ws-at-years",
                    "pf-at-years",
                    "wal-at-years",
                    "alt-sale",
                    "fp-alt-sale",
                    "2024-06-01T12:00:00Z",
                    "2024-06-01T12:05:00Z",
                    "outbound",
                    "BTC",
                    -btc_to_msat("0.1"),
                    0,
                    "EUR",
                    1_400,
                    140,
                    "import",
                    "sell",
                    "Legacy sale",
                    None,
                    None,
                    0,
                    "{}",
                    "2024-06-01T12:00:00Z",
                ),
                (
                    "tx-transfer-only",
                    "ws-at-years",
                    "pf-at-years",
                    "wal-at-years",
                    "transfer-only",
                    "fp-transfer-only",
                    "2025-02-01T12:00:00Z",
                    "2025-02-01T12:05:00Z",
                    "outbound",
                    "BTC",
                    -btc_to_msat("0.01"),
                    0,
                    "EUR",
                    1_500,
                    15,
                    "import",
                    "transfer",
                    "Transfer only",
                    None,
                    None,
                    0,
                    "{}",
                    "2025-02-01T12:00:00Z",
                ),
                (
                    "tx-transaction-only-year",
                    "ws-at-years",
                    "pf-at-years",
                    "wal-at-years",
                    "transaction-only-year",
                    "fp-transaction-only-year",
                    "2023-02-01T12:00:00Z",
                    "2023-02-01T12:05:00Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.01"),
                    0,
                    "EUR",
                    1_300,
                    13,
                    "import",
                    "deposit",
                    "Transaction-only year",
                    None,
                    None,
                    0,
                    "{}",
                    "2023-02-01T12:00:00Z",
                ),
            ],
        )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, at_category,
                at_kennzahl, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-alt-sale",
                    "ws-at-years",
                    "pf-at-years",
                    "tx-alt-sale",
                    "wal-at-years",
                    "2024-06-01T12:00:00Z",
                    "disposal",
                    "BTC",
                    -btc_to_msat("0.1"),
                    -140,
                    1_000,
                    100,
                    140,
                    40,
                    "Legacy sale",
                    "alt_spekulation",
                    801,
                    "2024-06-01T12:00:00Z",
                ),
                (
                    "je-transfer-only",
                    "ws-at-years",
                    "pf-at-years",
                    "tx-transfer-only",
                    "wal-at-years",
                    "2025-02-01T12:00:00Z",
                    "transfer_fee",
                    "BTC",
                    -btc_to_msat("0.01"),
                    -15,
                    1_500,
                    10,
                    15,
                    5,
                    "Transfer only",
                    None,
                    None,
                    "2025-02-01T12:00:00Z",
                ),
            ],
        )
        set_setting(conn, "context_workspace", "ws-at-years")
        set_setting(conn, "context_profile", "pf-at-years")
        conn.commit()

        snapshot = build_capital_gains_snapshot(conn)
        self.assertEqual(snapshot["year"], 2024)
        self.assertEqual(snapshot["availableYears"], [2025, 2024, 2023])
        self.assertEqual(len(snapshot["lots"]), 1)
        selected_year_snapshot = build_capital_gains_snapshot(conn, tax_year=2025)
        self.assertEqual(selected_year_snapshot["year"], 2025)
        self.assertEqual(selected_year_snapshot["availableYears"], [2025, 2024, 2023])
        self.assertEqual(selected_year_snapshot["lots"], [])
        with self.assertRaises(AppError) as invalid_year:
            build_capital_gains_snapshot(conn, tax_year=0)
        self.assertEqual(invalid_year.exception.code, "validation")
        self.assertIn("plausible", str(invalid_year.exception))
        snapshot_rows = {row["code"]: row for row in snapshot["kennzahlRows"]}
        self.assertEqual(snapshot_rows["801"]["amountEurCents"], 4_000)
        self.assertEqual(snapshot_rows["801"]["form"], "E 1")
        self.assertEqual(snapshot_rows["801"]["formSection"], "E 1 Spekulationsgeschaefte")

        payload, result = self._run_json(
            "reports",
            "austrian-e1kv",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--year",
            "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-e1kv")
        report = payload["data"]
        summary_by_kennzahl = {row["kennzahl"]: row for row in report["summary_rows"]}
        self.assertEqual(summary_by_kennzahl[801]["amount_eur_cents"], 4_000)
        self.assertEqual(summary_by_kennzahl[801]["form"], "E 1")
        self.assertEqual(summary_by_kennzahl[801]["form_section"], "E 1 Spekulationsgeschaefte")
        self.assertEqual(report["kennzahl_totals"]["801"]["form"], "E 1")
        self.assertEqual(report["rows"][0]["form"], "E 1")

        plain_result = self._run_cli(
            "--format",
            "plain",
            "reports",
            "austrian-e1kv",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--year",
            "2024",
        )
        self.assertEqual(plain_result.returncode, 0, msg=plain_result.stderr)
        self.assertIn("E 1kv Kennzahlen", plain_result.stdout)
        self.assertIn("Other Austrian Kennzahlen", plain_result.stdout)
        self.assertIn("| E 1 | 801 |", plain_result.stdout)

    def test_capital_gains_snapshot_includes_income_only_report_year(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-income-year", "Main", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-income-year",
                "ws-income-year",
                "Default",
                "EUR",
                "generic",
                365,
                "FIFO",
                "2026-01-01T01:00:00Z",
                1,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal-income-year",
                "ws-income-year",
                "pf-income-year",
                "Node",
                "manual",
                "{}",
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-income-only",
                "ws-income-year",
                "pf-income-year",
                "wal-income-year",
                "income-only",
                "fp-income-only",
                "2024-03-01T12:00:00Z",
                "2024-03-01T12:05:00Z",
                "inbound",
                "BTC",
                btc_to_msat("0.01"),
                0,
                "EUR",
                4_000,
                40,
                "import",
                "income",
                "Income only",
                None,
                None,
                0,
                "{}",
                "2024-03-01T12:00:00Z",
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "je-income-only",
                "ws-income-year",
                "pf-income-year",
                "tx-income-only",
                "wal-income-year",
                "2024-03-01T12:00:00Z",
                "income",
                "BTC",
                btc_to_msat("0.01"),
                40,
                4_000,
                None,
                None,
                40,
                "Income only",
                "2024-03-01T12:00:00Z",
            ),
        )
        set_setting(conn, "context_workspace", "ws-income-year")
        set_setting(conn, "context_profile", "pf-income-year")
        conn.commit()

        snapshot = build_capital_gains_snapshot(conn)

        self.assertEqual(snapshot["year"], 2024)
        self.assertEqual(snapshot["availableYears"], [2024])
        self.assertEqual(len(snapshot["lots"]), 1)
        self.assertEqual(snapshot["lots"][0]["disposed"], "2024-03-01")
        self.assertEqual(snapshot["lots"][0]["proceedsEur"], 40.0)

    def test_capital_gains_snapshot_splits_neutral_austrian_swap_rows(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-neutral-swap", "Main", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-neutral-swap",
                "ws-neutral-swap",
                "Default",
                "EUR",
                "at",
                365,
                "moving_average_at",
                now,
                3,
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            [
                ("wal-lbtc-neutral", "ws-neutral-swap", "pf-neutral-swap", "Liquid", "descriptor", "{}", now),
                ("wal-btc-neutral", "ws-neutral-swap", "pf-neutral-swap", "Onchain", "descriptor", "{}", now),
            ],
        )
        conn.executemany(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "tx-neutral-out",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    "wal-lbtc-neutral",
                    "neutral-out",
                    "fp-neutral-out",
                    "2026-03-14T17:30:10Z",
                    "2026-03-14T17:30:20Z",
                    "outbound",
                    "LBTC",
                    btc_to_msat("0.12426275"),
                    btc_to_msat("0.00000509"),
                    "EUR",
                    61_878.46,
                    7_689.18,
                    "import",
                    "withdrawal",
                    "Liquid peg-out",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-neutral-in",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    "wal-btc-neutral",
                    "neutral-in",
                    "fp-neutral-in",
                    "2026-03-14T17:32:32Z",
                    "2026-03-14T17:32:42Z",
                    "inbound",
                    "BTC",
                    btc_to_msat("0.12413298"),
                    0,
                    "EUR",
                    61_878.46,
                    7_681.16,
                    "import",
                    "deposit",
                    "On-chain peg-out receive",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
                (
                    "tx-taxable-out",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    "wal-btc-neutral",
                    "taxable-out",
                    "fp-taxable-out",
                    "2026-04-15T09:38:43Z",
                    "2026-04-15T09:39:00Z",
                    "outbound",
                    "BTC",
                    btc_to_msat("0.06811291"),
                    btc_to_msat("0.00001413"),
                    "EUR",
                    62_896.96,
                    4_284.09,
                    "import",
                    "withdrawal",
                    "Taxable spend",
                    None,
                    None,
                    0,
                    "{}",
                    now,
                ),
            ],
        )
        conn.execute(
            """
            INSERT INTO journal_custody_economic_relations(
                relation_id, workspace_id, profile_id, relation_kind,
                source_transaction_id, target_transaction_id,
                source_asset, target_asset, source_amount_msat,
                target_amount_msat, review_kind, policy, swap_fee_msat,
                swap_fee_kind, basis_state, occurred_at, target_occurred_at,
                created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "d" * 64,
                "ws-neutral-swap",
                "pf-neutral-swap",
                "conversion",
                "tx-neutral-out",
                "tx-neutral-in",
                "LBTC",
                "BTC",
                btc_to_msat("0.12426275"),
                btc_to_msat("0.12413298"),
                "peg-out",
                "carrying-value",
                btc_to_msat("0.00012977"),
                "combined",
                "eligible",
                "2026-03-14T17:30:10Z",
                "2026-03-14T17:32:32Z",
                now,
            ),
        )
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, at_category,
                at_kennzahl, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "je-neutral-swap",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    "tx-neutral-out",
                    "wal-lbtc-neutral",
                    "2026-03-14T17:30:10Z",
                    "disposal",
                    "LBTC",
                    -btc_to_msat("0.12426784"),
                    7_689.18,
                    0,
                    7_568.02,
                    7_689.18,
                    121.16,
                    "at_regime=neu at_swap_link=pair-neutral-swap",
                    "neu_swap",
                    None,
                    now,
                ),
                (
                    "je-taxable-out",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    "tx-taxable-out",
                    "wal-btc-neutral",
                    "2026-04-15T09:38:43Z",
                    "disposal",
                    "BTC",
                    -btc_to_msat("0.06812704"),
                    4_284.09,
                    0,
                    4_965.50,
                    4_284.09,
                    -681.41,
                    "at_regime=neu",
                    "neu_loss",
                    176,
                    now,
                ),
            ],
        )
        set_setting(conn, "context_workspace", "ws-neutral-swap")
        set_setting(conn, "context_profile", "pf-neutral-swap")
        conn.commit()

        snapshot = build_capital_gains_snapshot(conn)

        self.assertEqual(snapshot["year"], 2026)
        self.assertEqual(snapshot["method"], "moving_average_at")
        self.assertEqual(len(snapshot["lots"]), 1)
        self.assertEqual(snapshot["lots"][0]["disposed"], "2026-04-15")
        self.assertEqual(len(snapshot["neutralSwapLots"]), 1)
        neutral = snapshot["neutralSwapLots"][0]
        self.assertEqual(neutral["date"], "2026-03-14")
        self.assertEqual(neutral["outAsset"], "LBTC")
        self.assertEqual(neutral["inAsset"], "BTC")
        self.assertEqual(neutral["outSats"], 12_426_275)
        self.assertEqual(neutral["inSats"], 12_413_298)
        self.assertEqual(neutral["feeSats"], 12_977)
        self.assertEqual(neutral["gainEur"], 0.0)
        self.assertEqual(neutral["proceedsEur"], neutral["costEur"])
        self.assertAlmostEqual(neutral["marketDeltaEur"], 121.16, places=2)

        conn.executescript(
            """
            CREATE TABLE IF NOT EXISTS journal_tax_summary (
                id TEXT PRIMARY KEY,
                workspace_id TEXT NOT NULL,
                profile_id TEXT NOT NULL,
                year INTEGER NOT NULL,
                asset TEXT NOT NULL,
                transaction_type TEXT NOT NULL,
                capital_gains_type TEXT,
                quantity INTEGER NOT NULL,
                proceeds REAL NOT NULL DEFAULT 0,
                cost_basis REAL NOT NULL DEFAULT 0,
                gain_loss REAL NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
            """
        )
        conn.executemany(
            """
            INSERT INTO journal_tax_summary(
                id, workspace_id, profile_id, year, asset, transaction_type,
                capital_gains_type, quantity, proceeds, cost_basis, gain_loss, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "jts-taxable-btc",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    2026,
                    "BTC",
                    "sell",
                    "short",
                    btc_to_msat("0.06812704"),
                    4_284.09,
                    4_965.50,
                    -681.41,
                    now,
                ),
                (
                    "jts-neutral-lbtc",
                    "ws-neutral-swap",
                    "pf-neutral-swap",
                    2026,
                    "LBTC",
                    "sell",
                    "short",
                    btc_to_msat("0.12426784"),
                    7_689.18,
                    7_568.02,
                    121.16,
                    now,
                ),
            ],
        )

        hooks = ReportHooks(
            resolve_scope=lambda _conn, _workspace, _profile: (
                conn.execute(
                    "SELECT * FROM workspaces WHERE id = ?",
                    ("ws-neutral-swap",),
                ).fetchone(),
                conn.execute(
                    "SELECT * FROM profiles WHERE id = ?",
                    ("pf-neutral-swap",),
                ).fetchone(),
            ),
            resolve_account=lambda *_args, **_kwargs: None,
            resolve_wallet=lambda *_args, **_kwargs: None,
            list_journal_entries=lambda *_args, **_kwargs: [],
            list_wallets=lambda *_args, **_kwargs: [],
            parse_iso_datetime=lambda *_args, **_kwargs: None,
            iso_z=lambda value: value,
            now_iso=lambda: now,
            format_table=lambda *_args, **_kwargs: [],
            write_text_pdf=lambda *_args, **_kwargs: {},
        )
        tax_rows = report_tax_summary(
            conn,
            "ws-neutral-swap",
            "pf-neutral-swap",
            hooks,
        )
        detail_rows = [row for row in tax_rows if row["row_type"] == "detail"]
        self.assertEqual([row["asset"] for row in detail_rows], ["BTC"])
        year_total = next(row for row in tax_rows if row["row_type"] == "year_total")
        self.assertAlmostEqual(year_total["gain_loss"], -681.41, places=2)
        self.assertAlmostEqual(year_total["proceeds"], 4_284.09, places=2)
        swap_fee_row = next(row for row in tax_rows if row["row_type"] == "swap_fees_year")
        self.assertEqual(swap_fee_row["total_swap_fee_msat"], btc_to_msat("0.00012977"))

        events = build_journal_events_list_snapshot(conn, {"limit": 10})
        self.assertEqual(events["summary"]["reportableCount"], 1)
        type_counts = {
            row["type"]: (row["count"], row["gainLossEur"])
            for row in events["summary"]["entryTypes"]
        }
        self.assertEqual(type_counts["neutral_swap"], (1, 0.0))
        self.assertEqual(type_counts["disposal"], (1, -681.41))
        neutral_event = next(
            row for row in events["events"] if row["atCategory"] == "neu_swap"
        )
        self.assertEqual(neutral_event["entryType"], "neutral_swap")
        self.assertEqual(neutral_event["gainLossEur"], 0.0)
        self.assertEqual(
            neutral_event["proceedsEur"],
            neutral_event["costBasisEur"],
        )
        self.assertAlmostEqual(neutral_event["marketDeltaEur"], 121.16, places=2)
        self.assertEqual(neutral_event["pair"]["kind"], "peg-out")
        self.assertEqual(neutral_event["pair"]["policy"], "carrying-value")
        self.assertEqual(neutral_event["pair"]["out"]["asset"], "LBTC")
        self.assertEqual(neutral_event["pair"]["in"]["asset"], "BTC")
        self.assertEqual(
            neutral_event["pair"]["swapFeeMsat"],
            btc_to_msat("0.00012977"),
        )

        journals = build_journals_snapshot(conn)
        state_type_counts = {
            row["type"]: (row["count"], row["gainLossEur"])
            for row in journals["entryTypes"]
        }
        self.assertEqual(state_type_counts["neutral_swap"], (1, 0.0))
        self.assertEqual(state_type_counts["disposal"], (1, -681.41))
        self.assertEqual(
            journals["recentByType"]["neutral_swap"][0]["type"],
            "neutral_swap",
        )
        self.assertEqual(
            journals["recentByType"]["neutral_swap"][0]["gainLossEur"],
            0.0,
        )
        neutral_recent = journals["recentByType"]["neutral_swap"][0]
        self.assertEqual(neutral_recent["pair"]["kind"], "peg-out")
        self.assertEqual(neutral_recent["pair"]["policy"], "carrying-value")
        self.assertEqual(neutral_recent["pair"]["out"]["asset"], "LBTC")
        self.assertEqual(neutral_recent["pair"]["in"]["asset"], "BTC")

        e1kv = report_austrian_e1kv(
            conn,
            "ws-neutral-swap",
            "pf-neutral-swap",
            hooks,
            tax_year=2026,
        )
        self.assertEqual([row["asset"] for row in e1kv["rows"]], ["BTC"])
        self.assertEqual(
            [row["at_category"] for row in e1kv["sections"]["1.1"]["detail_rows"]],
            ["neu_loss"],
        )
        self.assertEqual(e1kv["sections"]["1.1"]["totals"]["row_count"], 1)
        self.assertEqual(
            e1kv["sections"]["1.1"]["totals"]["gain_loss_eur_cents"],
            -68_141,
        )

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

    def _bootstrap_austrian_e1kv_wallet(self, label="AustrianE1kv"):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "EUR",
            "--tax-country", "at",
            "Default",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", label,
            "--kind", "custom",
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
        self.assertTrue(command_needs_db(Namespace(command="diagnostics", diagnostics_command="collect")))
        self.assertTrue(command_needs_db(Namespace(command="status")))
        self.assertTrue(command_needs_db(Namespace(command="backends", backends_command="list")))
        self.assertTrue(command_needs_db(Namespace(command="backends", backends_command="get")))
        self.assertTrue(command_needs_db(Namespace(command="rates", rates_command="pairs")))

    def test_public_diagnostics_collect_omits_sensitive_state(self):
        sensitive_txid = "a" * 64
        sensitive_address = "bc1qprivatebugreportaddress000000000000000000000000"
        sensitive_description = "Sensitive coffee at Alice"
        sensitive_note = "Internal treasury note"
        sensitive_csv = self._write_case_file(
            "sensitive.csv",
            "\n".join(
                [
                    "date,txid,direction,asset,amount,fee,fiat_rate,description",
                    f"2026-04-24T09:00:00Z,{sensitive_txid},inbound,BTC,0.12345678,0.00001234,65432.10,{sensitive_description}",
                ]
            )
            + "\n",
        )

        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "SecretCo Books")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create", "--workspace", "SecretCo Books", "PrivateTax"
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "backends", "create", "secret-backend",
            "--kind", "electrum",
            "--url", "ssl://user:pass@node.private.example:50002/wallet?token=supersecret",
            "--token", "supersecret-token",
            "--notes", "private infrastructure",
        )
        self._assert_ok(payload, result, "backends.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "SecretCo Books",
            "--profile", "PrivateTax",
            "--label", "Cold Wallet Private",
            "--kind", "address",
            "--backend", "secret-backend",
            "--address", sensitive_address,
        )
        self._assert_ok(payload, result, "wallets.create")
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "SecretCo Books",
            "--profile", "PrivateTax",
            "--wallet", "Cold Wallet Private",
            "--file", str(sensitive_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        payload, result = self._run_json(
            "metadata", "records", "note", "set",
            "--workspace", "SecretCo Books",
            "--profile", "PrivateTax",
            "--transaction", sensitive_txid,
            "--note", sensitive_note,
        )
        self._assert_ok(payload, result, "metadata.records.note.set")

        payload, result = self._run_json("diagnostics", "collect", "--save")
        self._assert_ok(payload, result, "diagnostics.collect")
        data = payload["data"]
        report = data["report"]
        self.assertTrue(report["public_safe"])
        self.assertEqual(data["saved"]["relative_path"].split("/")[0:2], ["exports", "diagnostics"])
        report_path = self.data_root.parent / data["saved"]["relative_path"]
        self.assertTrue(report_path.exists())

        combined = json.dumps(payload, sort_keys=True) + report_path.read_text(encoding="utf-8")
        leaked_values = [
            str(self.data_root),
            "SecretCo",
            "PrivateTax",
            "Cold Wallet",
            "secret-backend",
            "node.private.example",
            "supersecret",
            sensitive_txid,
            sensitive_address,
            "0.12345678",
            "0.00001234",
            "65432.10",
            sensitive_description,
            sensitive_note,
        ]
        for leaked in leaked_values:
            self.assertNotIn(leaked, combined)
        self.assertEqual(report["state"]["transactions"]["total"], 1)
        self.assertEqual(report["state"]["wallets"]["total"], 1)
        self.assertEqual(report["state"]["backends"]["credential_presence"]["token"], 1)

    def test_diagnostics_out_auto_writes_public_error_report(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        result = self._run_cli(
            "--diagnostics-out", "auto",
            "rates", "range", "BTC-USD",
            "--start", "not-a-date",
            machine=True,
        )
        self.assertNotEqual(result.returncode, 0)
        error_payload = json.loads(result.stdout)
        self.assertEqual(error_payload["kind"], "error")
        diagnostics_dir = self.data_root.parent / "exports" / "diagnostics"
        reports = sorted(diagnostics_dir.glob("kassiber-diagnostics-*.json"))
        self.assertEqual(len(reports), 1)
        report = json.loads(reports[0].read_text(encoding="utf-8"))
        self.assertTrue(report["public_safe"])
        self.assertEqual(report["invocation"]["command_path"], ["rates", "range"])
        self.assertEqual(report["error"]["code"], "validation")
        self.assertIn("stack", report)
        self.assertNotIn(str(self.data_root), json.dumps(report, sort_keys=True))

        custom_path = self.case_dir / "custom-diagnostics.json"
        result = self._run_cli(
            "--diagnostics-out", str(custom_path),
            "rates", "range", "BTC-USD",
            "--start", "still-not-a-date",
            machine=True,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertTrue(custom_path.exists())
        self.assertIn(str(custom_path), result.stderr)

    def test_public_diagnostics_sanitizes_xpubs_integer_amounts_and_argument_text(self):
        from argparse import Namespace

        from kassiber.diagnostics import collect_public_diagnostics, sanitize_text

        sample_upub = "Upub" + ("A" * 80)
        sample_xprv = "xprv" + ("B" * 80)
        sanitized = sanitize_text(
            f"descriptor={sample_upub} backup={sample_xprv} api_key=sk-diagnostics-secret "
            "mnemonic=abandon abandon abandon seed_phrase=legal winner thank "
            "auth_header=Bearer diagnostics-token amount=12345 sat fee=2500msat "
            "timestamp=2026-04-24T09:00:00Z"
        )
        self.assertNotIn(sample_upub, sanitized)
        self.assertNotIn(sample_xprv, sanitized)
        self.assertNotIn("abandon", sanitized)
        self.assertNotIn("legal", sanitized)
        self.assertNotIn("winner", sanitized)
        self.assertNotIn("sk-diagnostics-secret", sanitized)
        self.assertNotIn("diagnostics-token", sanitized)
        self.assertNotIn("12345", sanitized)
        self.assertNotIn("2500", sanitized)
        self.assertNotIn("2026", sanitized)
        self.assertNotIn("09:00", sanitized)

        args = Namespace(
            command="metadata",
            metadata_command="records",
            records_command="list",
            format="json",
            machine=True,
            debug=False,
            save=False,
            tag="private-tax-review",
            backend="secret-backend",
            account="private-account",
            api_key="sk-argv-diagnostics",
            mnemonic="abandon abandon abandon",
            passphrase="very-private-passphrase",
            type="private-type",
            asset="PRIVATEASSET",
            provider="public-provider",
            trend="weekly",
        )
        report = collect_public_diagnostics(
            None,
            args,
            error=AppError(
                "Provider failed with api_key=sk-error-diagnostics",
                code="secret_error",
                details={
                    "api_key": "sk-detail-diagnostics",
                    "mnemonic": "abandon abandon abandon",
                    "nested": {"token": "nested-token"},
                    "message": "Bearer diagnostics-detail-token recovery_phrase=legal winner thank",
                },
            ),
        )
        encoded_report = json.dumps(report, sort_keys=True)
        for leaked in (
            "sk-argv-diagnostics",
            "very-private-passphrase",
            "sk-error-diagnostics",
            "sk-detail-diagnostics",
            "abandon",
            "legal",
            "winner",
            "nested-token",
            "diagnostics-detail-token",
        ):
            self.assertNotIn(leaked, encoded_report)
        values = {
            item["name"]: item
            for item in report["invocation"]["provided_arguments"]
        }
        self.assertEqual(values["tag"]["value_class"], "redacted")
        self.assertEqual(values["backend"]["value_class"], "redacted")
        self.assertEqual(values["account"]["value_class"], "redacted")
        self.assertEqual(values["api_key"]["value_class"], "redacted")
        self.assertEqual(values["mnemonic"]["value_class"], "redacted")
        self.assertEqual(values["passphrase"]["value_class"], "redacted")
        self.assertEqual(values["type"]["value_class"], "redacted")
        self.assertEqual(values["asset"]["value_class"], "redacted")
        self.assertEqual(values["provider"]["value"], "public-provider")
        self.assertEqual(values["trend"]["value"], "weekly")

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

    def test_cli_numeric_limits_reject_non_positive_values(self):
        self._bootstrap_wallet()

        checks = [
            (
                "transactions",
                "list",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--limit",
                "0",
            ),
            (
                "transactions",
                "list",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--limit",
                "1001",
            ),
            (
                "journals",
                "list",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--limit",
                "0",
            ),
            ("rates", "range", "BTC-USD", "--limit", "0"),
            ("rates", "sync", "--days", "0"),
            (
                "reports",
                "export-pdf",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--file",
                str(self.case_dir / "bad-history-limit.pdf"),
                "--history-limit",
                "-1",
            ),
        ]
        for args in checks:
            with self.subTest(args=args):
                payload, result = self._run_json(*args)
                self.assertEqual(result.returncode, 1, msg=payload)
                self.assertEqual(payload.get("kind"), "error")
                self.assertEqual(payload["error"]["code"], "validation")

    def test_wallets_sync_rejects_wallet_and_all_together(self):
        self._bootstrap_wallet(label="SyncFlags")
        payload, result = self._run_json(
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "SyncFlags",
            "--all",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

    def test_missing_import_file_returns_not_found(self):
        self._bootstrap_wallet(label="MissingImport")
        payload, result = self._run_json(
            "wallets",
            "import-json",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "MissingImport",
            "--file",
            str(self.case_dir / "missing.json"),
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "not_found")

    def test_wallets_sync_all_reports_per_wallet_file_errors(self):
        self._bootstrap_profile()
        good_file = self.case_dir / "good-wallet.json"
        good_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "good-sync",
                    }
                ]
            ),
            encoding="utf-8",
        )
        for label, source_file in (
            ("GoodSync", good_file),
            ("BadSync", self.case_dir / "missing-wallet.json"),
        ):
            payload, result = self._run_json(
                "wallets",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--label",
                label,
                "--kind",
                "custom",
                "--source-file",
                str(source_file),
                "--source-format",
                "json",
            )
            self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--all",
        )
        self._assert_ok(payload, result, "wallets.sync")
        by_wallet = {row["wallet"]: row for row in payload["data"]}
        self.assertEqual(by_wallet["GoodSync"]["status"], "synced")
        self.assertEqual(by_wallet["GoodSync"]["imported"], 1)
        self.assertEqual(by_wallet["BadSync"]["status"], "error")
        self.assertEqual(by_wallet["BadSync"]["code"], "not_found")

    def test_wallets_sync_all_rolls_back_failed_wallet_imports(self):
        self._bootstrap_profile()
        bad_file = self.case_dir / "bad-partial-wallet.json"
        bad_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "txid": "bad-first-row",
                    },
                    {
                        "date": "2024-01-02",
                        "direction": "sideways",
                        "asset": "BTC",
                        "amount": "0.002",
                        "fee": "0",
                        "txid": "bad-second-row",
                    },
                ]
            ),
            encoding="utf-8",
        )
        good_file = self.case_dir / "good-after-bad-wallet.json"
        good_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-01-03",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.003",
                        "fee": "0",
                        "txid": "good-after-bad",
                    }
                ]
            ),
            encoding="utf-8",
        )
        for label, source_file in (("A-BadPartial", bad_file), ("B-GoodAfterBad", good_file)):
            payload, result = self._run_json(
                "wallets",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--label",
                label,
                "--kind",
                "custom",
                "--source-file",
                str(source_file),
                "--source-format",
                "json",
            )
            self._assert_ok(payload, result, "wallets.create")

        payload, result = self._run_json(
            "wallets",
            "sync",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--all",
        )
        self._assert_ok(payload, result, "wallets.sync")
        by_wallet = {row["wallet"]: row for row in payload["data"]}
        self.assertEqual(by_wallet["A-BadPartial"]["status"], "error")
        self.assertEqual(by_wallet["B-GoodAfterBad"]["status"], "synced")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        rows = conn.execute(
            """
            SELECT w.label, COUNT(t.id)
            FROM wallets w
            LEFT JOIN transactions t ON t.wallet_id = w.id
            WHERE w.label IN ('A-BadPartial', 'B-GoodAfterBad')
            GROUP BY w.label
            ORDER BY w.label
            """
        ).fetchall()
        conn.close()
        counts = {label: count for label, count in rows}
        self.assertEqual(counts["A-BadPartial"], 0)
        self.assertEqual(counts["B-GoodAfterBad"], 1)

    def test_duplicate_creates_return_conflict_envelopes(self):
        self._bootstrap_wallet(label="UniqueWallet")
        payload, result = self._run_json("workspaces", "create", "Main")
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

        payload, result = self._run_json(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Default",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "UniqueWallet",
            "--kind",
            "phoenix",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

        payload, result = self._run_json(
            "metadata",
            "tags",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--code",
            "review",
            "--label",
            "Review",
        )
        self._assert_ok(payload, result, "metadata.tags.create")
        payload, result = self._run_json(
            "metadata",
            "tags",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--code",
            "review",
            "--label",
            "Review again",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "conflict")

    def test_wallet_update_rejects_clearing_required_or_unknown_config(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "AddressWallet",
            "--kind",
            "address",
            "--address",
            "bc1qexampleaddress",
        )
        self._assert_ok(payload, result, "wallets.create")

        for field in ("addresses", "not-a-field"):
            with self.subTest(field=field):
                payload, result = self._run_json(
                    "wallets",
                    "update",
                    "--workspace",
                    "Main",
                    "--profile",
                    "Default",
                    "--wallet",
                    "AddressWallet",
                    "--clear",
                    field,
                )
                self.assertEqual(result.returncode, 1, msg=payload)
                self.assertEqual(payload.get("kind"), "error")
                self.assertEqual(payload["error"]["code"], "validation")

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
            ["cash-ops", "treasury"],
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

    def test_wallets_update_invalidates_cached_output_inventory_on_config_change(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "AddressWallet",
            "--kind",
            "address",
            "--address",
            "bc1qoldwatchtarget0000000000000000000000000",
            "--backend",
            "mempool",
        )
        self._assert_ok(payload, result, "wallets.create")

        conn = open_db(self.data_root)
        try:
            wallet = conn.execute(
                """
                SELECT w.id, w.workspace_id, w.profile_id
                FROM wallets w
                WHERE w.label = ?
                """,
                ("AddressWallet",),
            ).fetchone()
            self.assertIsNotNone(wallet)
            conn.execute(
                """
                INSERT INTO wallet_utxos(
                    id, workspace_id, profile_id, wallet_id, backend_name,
                    backend_kind, chain, network, asset, amount, txid, vout,
                    outpoint, confirmation_status, confirmations, block_height,
                    block_time, address, address_label, branch_label,
                    branch_index, address_index, first_seen_at, last_seen_at,
                    spent_at, raw_json
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    "utxo-old",
                    wallet["workspace_id"],
                    wallet["profile_id"],
                    wallet["id"],
                    "mempool",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    "BTC",
                    1_000_000,
                    "aa" * 32,
                    0,
                    f"{'aa' * 32}:0",
                    "confirmed",
                    1,
                    800_000,
                    "2026-01-01T00:00:00Z",
                    "bc1qoldwatchtarget0000000000000000000000000",
                    "address #0",
                    "address",
                    None,
                    0,
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                    None,
                    "{}",
                ),
            )
            conn.execute(
                """
                INSERT INTO wallet_utxo_refreshes(
                    wallet_id, workspace_id, profile_id, backend_name,
                    backend_kind, chain, network, observed_count,
                    active_count, last_seen_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    wallet["id"],
                    wallet["workspace_id"],
                    wallet["profile_id"],
                    "mempool",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    1,
                    1,
                    "2026-01-01T00:00:00Z",
                ),
            )
            conn.commit()
        finally:
            conn.close()

        payload, result = self._run_json(
            "wallets",
            "update",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "AddressWallet",
            "--config",
            json.dumps({"addresses": ["bc1qnewwatchtarget0000000000000000000000000"]}),
        )
        self._assert_ok(payload, result, "wallets.update")

        conn = open_db(self.data_root)
        try:
            wallet_id = payload["data"]["id"]
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM wallet_utxos WHERE wallet_id = ?",
                    (wallet_id,),
                ).fetchone()[0],
                0,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM wallet_utxo_refreshes WHERE wallet_id = ?",
                    (wallet_id,),
                ).fetchone()[0],
                0,
            )
        finally:
            conn.close()

    def test_wallet_outputs_redact_descriptor_material_but_keep_state_flags(self):
        self._bootstrap_profile()
        descriptor, change_descriptor = _sample_descriptor_pair()
        stored_descriptor = f"{descriptor}#receivechk"
        stored_change_descriptor = f"{change_descriptor}#changechk"

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
            stored_descriptor,
            "--change-descriptor",
            stored_change_descriptor,
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
        self.assertEqual(stored_config["descriptor"], stored_descriptor)
        self.assertEqual(stored_config["change_descriptor"], stored_change_descriptor)

        payload, result = self._run_json(
            "wallets",
            "reveal-descriptor",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "Vault",
        )
        self._assert_ok(payload, result, "wallets.reveal-descriptor")
        expected_material = f"{stored_descriptor}\n{stored_change_descriptor}"
        self.assertEqual(payload["data"]["wallet_material"], expected_material)
        self.assertEqual(payload["data"]["descriptor"], stored_descriptor)
        self.assertEqual(
            payload["data"]["change_descriptor"], stored_change_descriptor
        )

        result = self._run_cli(
            "wallets",
            "reveal-descriptor",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "Vault",
            "--material-only",
        )
        self.assertEqual(result.returncode, 0, msg=result.stderr)
        self.assertEqual(result.stdout, expected_material + "\n")

    def test_wallet_descriptor_state_degrades_when_embit_missing(self):
        from kassiber.core import wallets as core_wallets

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
            "--descriptor",
            descriptor,
            "--change-descriptor",
            change_descriptor,
        )
        self._assert_ok(payload, result, "wallets.create")
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)

        missing_embit = AppError(
            "Descriptor-backed refresh requires the 'embit' package",
            code="dependency_missing",
            details={"missing_package": "embit"},
        )
        with patch(
            "kassiber.core.wallets.load_descriptor_plan",
            side_effect=missing_embit,
        ):
            rows = core_wallets.list_wallets(conn, "Main", "Default")
            details = core_wallets.get_wallet_details(conn, "Main", "Default", "Vault")

        self.assertEqual(rows[0]["descriptor"], "invalid")
        self.assertEqual(details["descriptor_state"], "invalid")
        self.assertTrue(details["descriptor"])

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

    def test_clear_default_prefers_user_backend_and_surfaces_fallback_notice(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json(
            "backends",
            "create",
            "own-mempool",
            "--kind",
            "mempool",
            "--url",
            "http://127.0.0.1:3006/api",
            "--chain",
            "bitcoin",
            "--network",
            "mainnet",
        )
        self._assert_ok(payload, result, "backends.create")

        conn = open_db(str(self.data_root))
        self.addCleanup(conn.close)
        set_setting(conn, "bootstrap_default_backend", "missing-bootstrap")
        set_setting(conn, "default_backend", "fulcrum")
        conn.commit()

        payload, result = self._run_json("backends", "clear-default")
        self._assert_ok(payload, result, "backends.clear-default")

        self.assertEqual(payload["data"]["default_backend"], "own-mempool")
        self.assertTrue(payload["data"]["cleared"])
        self.assertEqual(payload["data"]["notice"]["code"], "default_backend_fallback")
        self.assertEqual(payload["data"]["notice"]["missing_default"], "missing-bootstrap")
        self.assertEqual(payload["data"]["notice"]["previous_default"], "fulcrum")

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

        payload, result = self._run_json("backends", "delete", "mempool")
        self._assert_ok(payload, result, "backends.delete")
        self.assertTrue(payload["data"]["deleted"])

        payload, result = self._run_json("backends", "list")
        self._assert_ok(payload, result, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertNotIn("mempool", rows)

        runtime = self._bootstrap_runtime_state()
        self.assertNotIn("mempool", runtime.runtime_config["backends"])

        payload, result = self._run_json("backends", "get", "mempool")
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

    def test_backends_update_clears_cached_output_inventory_for_backend(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "backends",
            "create",
            "coin-source",
            "--kind",
            "esplora",
            "--url",
            "https://coins-one.example",
            "--chain",
            "bitcoin",
            "--network",
            "mainnet",
        )
        self._assert_ok(payload, result, "backends.create")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "CoinWallet",
            "--kind",
            "address",
            "--address",
            "bc1qcoinwatchtarget0000000000000000000000000",
            "--backend",
            "coin-source",
        )
        self._assert_ok(payload, result, "wallets.create")
        wallet_id = payload["data"]["id"]

        conn = open_db(self.data_root)
        try:
            wallet = conn.execute(
                "SELECT id, workspace_id, profile_id FROM wallets WHERE id = ?",
                (wallet_id,),
            ).fetchone()
            self.assertIsNotNone(wallet)
            conn.execute(
                """
                INSERT INTO wallet_utxos(
                    id, workspace_id, profile_id, wallet_id, backend_name,
                    backend_kind, chain, network, asset, amount, txid, vout,
                    outpoint, confirmation_status, confirmations, block_height,
                    block_time, address, address_label, branch_label,
                    branch_index, address_index, first_seen_at, last_seen_at,
                    spent_at, raw_json
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    "utxo-backend-update",
                    wallet["workspace_id"],
                    wallet["profile_id"],
                    wallet["id"],
                    "coin-source",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    "BTC",
                    1_000_000,
                    "cc" * 32,
                    0,
                    f"{'cc' * 32}:0",
                    "confirmed",
                    1,
                    800_000,
                    "2026-01-01T00:00:00Z",
                    "bc1qcoinwatchtarget0000000000000000000000000",
                    "address #0",
                    "address",
                    None,
                    0,
                    "2026-01-01T00:00:00Z",
                    "2026-01-01T00:00:00Z",
                    None,
                    "{}",
                ),
            )
            conn.execute(
                """
                INSERT INTO wallet_utxo_refreshes(
                    wallet_id, workspace_id, profile_id, backend_name,
                    backend_kind, chain, network, observed_count,
                    active_count, last_seen_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    wallet["id"],
                    wallet["workspace_id"],
                    wallet["profile_id"],
                    "coin-source",
                    "esplora",
                    "bitcoin",
                    "mainnet",
                    1,
                    1,
                    "2026-01-01T00:00:00Z",
                ),
            )
            conn.commit()
        finally:
            conn.close()

        payload, result = self._run_json(
            "backends",
            "update",
            "coin-source",
            "--url",
            "https://coins-two.example",
        )
        self._assert_ok(payload, result, "backends.update")

        conn = open_db(self.data_root)
        try:
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM wallet_utxos WHERE backend_name = ?",
                    ("coin-source",),
                ).fetchone()[0],
                0,
            )
            self.assertEqual(
                conn.execute(
                    "SELECT COUNT(*) FROM wallet_utxo_refreshes WHERE backend_name = ?",
                    ("coin-source",),
                ).fetchone()[0],
                0,
            )
        finally:
            conn.close()

    def test_backend_text_redaction_scrubs_urls_in_sync_error_payloads(self):
        message = (
            "HTTP 403 from backend for "
            "http://rpcuser:rpcpass@127.0.0.1:8332/wallet/review?session=topsecret: Forbidden"
        )
        redacted = redact_backend_text(message)
        self.assertIn("http://<redacted>@127.0.0.1:8332/wallet/review", redacted)
        self.assertNotIn("rpcuser", redacted)
        self.assertNotIn("rpcpass", redacted)
        self.assertNotIn("session=topsecret", redacted)
        self.assertEqual(
            redact_backend_value({"nested": [message]})["nested"][0],
            redacted,
        )

    def test_backend_output_redacts_proxy_credentials(self):
        payload = redact_backend_for_output(
            {
                "name": "mempool",
                "kind": "esplora",
                "url": "https://mempool.example/api",
                "tor_proxy": "socks5h://alice:p%40ss@127.0.0.1:9050",
            }
        )
        self.assertEqual(payload["tor_proxy"], "socks5h://redacted@127.0.0.1:9050")
        self.assertNotIn("alice", json.dumps(payload))
        self.assertNotIn("p%40ss", json.dumps(payload))

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

    def test_backend_display_name_is_safe_editable_metadata(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")

        payload, result = self._run_json(
            "backends",
            "create",
            "liquid-home",
            "--kind",
            "liquid-esplora",
            "--chain",
            "liquid",
            "--network",
            "liquidv1",
            "--url",
            "https://liquid.network/api",
            "--display-name",
            "Liquid desk node",
        )
        self._assert_ok(payload, result, "backends.create")
        self.assertEqual(payload["data"]["name"], "liquid-home")
        self.assertEqual(payload["data"]["display_name"], "Liquid desk node")

        payload, result = self._run_json(
            "backends",
            "update",
            "liquid-home",
            "--display-name",
            "Liquid office node",
        )
        self._assert_ok(payload, result, "backends.update")
        self.assertEqual(payload["data"]["name"], "liquid-home")
        self.assertEqual(payload["data"]["display_name"], "Liquid office node")

        payload, result = self._run_json("backends", "list")
        self._assert_ok(payload, result, "backends.list")
        rows = {row["name"]: row for row in payload["data"]}
        self.assertEqual(rows["liquid-home"]["display_name"], "Liquid office node")

        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        row = conn.execute(
            "SELECT name, config_json FROM backends WHERE name = 'liquid-home'"
        ).fetchone()
        conn.close()
        self.assertEqual(row[0], "liquid-home")
        stored_config = json.loads(row[1])
        self.assertEqual(stored_config["display_name"], "Liquid office node")

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

    def test_backends_delete_detaches_wallet_backend_references(self):
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
        self._assert_ok(payload, result, "backends.delete")
        self.assertTrue(payload["data"]["deleted"])
        self.assertEqual(
            payload["data"]["detached_wallet_refs"],
            ["Main/Default/Tracked"],
        )

        payload, result = self._run_json(
            "wallets",
            "get",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--wallet",
            "Tracked",
        )
        self._assert_ok(payload, result, "wallets.get")
        self.assertNotIn("backend", payload["data"]["config"])

    def test_backends_delete_removes_btcpay_account_routes(self):
        self._bootstrap_profile()

        payload, result = self._run_json(
            "backends",
            "create",
            "btcpay1",
            "--kind",
            "btcpay",
            "--url",
            "http://127.0.0.1:9",
            "--token",
            "testkey",
        )
        self._assert_ok(payload, result, "backends.create")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        workspace = conn.execute("SELECT * FROM workspaces WHERE label = 'Main'").fetchone()
        profile = conn.execute("SELECT * FROM profiles WHERE label = 'Default'").fetchone()
        conn.execute(
            """
            INSERT INTO btcpay_account_routes(
                id, workspace_id, profile_id, backend_name, store_id,
                payment_method_id, action, label, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "route-delete-regression",
                workspace["id"],
                profile["id"],
                "btcpay1",
                "STORE1",
                "BTC-LN",
                "provenance_only",
                "Store route",
                "2026-01-01T00:00:00+00:00",
                "2026-01-01T00:00:00+00:00",
            ),
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json("backends", "delete", "btcpay1")
        self._assert_ok(payload, result, "backends.delete")
        self.assertTrue(payload["data"]["deleted"])
        self.assertEqual(payload["data"]["detached_btcpay_account_routes"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        route_count = conn.execute("SELECT COUNT(*) FROM btcpay_account_routes").fetchone()[0]
        conn.close()
        self.assertEqual(route_count, 0)

        payload, result = self._run_json(
            "backends",
            "create",
            "btcpay1",
            "--kind",
            "btcpay",
            "--url",
            "http://127.0.0.1:9",
            "--token",
            "replacement",
        )
        self._assert_ok(payload, result, "backends.create")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        route_count = conn.execute("SELECT COUNT(*) FROM btcpay_account_routes").fetchone()[0]
        conn.close()
        self.assertEqual(route_count, 0)

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
        self.assertEqual(payload["data"][0], {
            "wallet": "SkipMe",
            "status": "skipped",
            "reason": "no file source, descriptor, or backend addresses configured",
        })
        sync_result = payload["data"][1]
        self.assertEqual(sync_result["wallet"], "SyncMe")
        self.assertEqual(sync_result["status"], "synced")
        self.assertEqual(sync_result["source"], "file:phoenix_csv")
        self.assertEqual(sync_result["imported"], 1)
        self.assertEqual(sync_result["skipped"], 0)
        self.assertEqual(sync_result["unchanged"], 0)
        self.assertEqual(sync_result["phoenix_notes_set"], 1)
        self.assertEqual(sync_result["phoenix_tags_added"], 1)
        self.assertEqual(sync_result["phoenix_tags_created"], 1)
        self.assertEqual(sync_result["input_format"], "phoenix_csv")
        self.assertEqual(sync_result["file"], str(phoenix_csv))
        self.assertEqual(len(sync_result["inserted_records"]), 1)
        self.assertEqual(sync_result["inserted_records"][0]["wallet"], "SyncMe")
        self.assertEqual(
            sync_result["inserted_records"][0]["external_id"],
            "11111111-aaaa-bbbb-cccc-000000000001",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        config = json.loads(
            conn.execute(
                "SELECT config_json FROM wallets WHERE label = ?",
                ("SyncMe",),
            ).fetchone()["config_json"]
        )
        last_synced_at = config.get("last_synced_at")
        self.assertIsInstance(last_synced_at, str)
        self.assertRegex(last_synced_at, r"^\d{4}-\d{2}-\d{2}T")
        overview = build_overview_snapshot(conn)
        overview_connections = {
            connection["label"]: connection for connection in overview["connections"]
        }
        self.assertEqual(
            overview_connections["SyncMe"]["lastSyncAt"],
            last_synced_at,
        )
        self.assertEqual(
            overview_connections["SyncMe"]["lastTransactionAt"],
            "2024-05-01T10:15:00Z",
        )

        payload, result = self._run_json(
            "wallets", "sync",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "SyncMe",
        )
        self._assert_ok(payload, result, "wallets.sync")
        sync_result = payload["data"][0]
        self.assertEqual(sync_result["wallet"], "SyncMe")
        self.assertEqual(sync_result["status"], "synced")
        self.assertEqual(sync_result["source"], "file:phoenix_csv")
        self.assertEqual(sync_result["imported"], 0)
        self.assertEqual(sync_result["skipped"], 1)
        self.assertIn("unchanged", sync_result)
        self.assertEqual(sync_result["inserted_records"], [])
        self.assertIn("updated_records", sync_result)
        self.assertEqual(sync_result["phoenix_notes_set"], 0)
        self.assertEqual(sync_result["phoenix_tags_added"], 0)
        self.assertEqual(sync_result["phoenix_tags_created"], 0)
        self.assertEqual(sync_result["input_format"], "phoenix_csv")
        self.assertEqual(sync_result["file"], str(phoenix_csv))

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

    def test_bip329_list_exposes_cursor_pagination(self):
        self._bootstrap_wallet(label="Bip329Page")
        bip329_file = self.case_dir / "many-labels.jsonl"
        bip329_file.write_text(
            "\n".join(
                json.dumps({"type": "tx", "ref": f"label-{idx:03d}", "label": f"Label {idx:03d}"})
                for idx in range(101)
            )
            + "\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "metadata",
            "bip329",
            "import",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--file",
            str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")

        payload, result = self._run_json(
            "metadata",
            "bip329",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "100",
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        self.assertEqual(len(payload["data"]), 100)
        self.assertTrue(payload["has_more"])
        self.assertTrue(payload["next_cursor"])
        first_cursor = payload["next_cursor"]

        payload, result = self._run_json(
            "metadata",
            "bip329",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "100",
            "--cursor",
            first_cursor,
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertFalse(payload["has_more"])
        self.assertIsNone(payload["next_cursor"])

        payload, result = self._run_json(
            "metadata",
            "bip329",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "100",
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        self.assertTrue(payload["next_cursor"])
        profile_cursor = payload["next_cursor"]
        payload, result = self._run_json(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Other",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "metadata",
            "bip329",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Other",
            "--limit",
            "100",
            "--cursor",
            profile_cursor,
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

        payload, result = self._run_json(
            "metadata",
            "bip329",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "100",
            "--cursor",
            first_cursor,
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertFalse(payload["has_more"])
        self.assertIsNone(payload["next_cursor"])

    def test_external_id_transaction_resolution_rejects_ambiguous_rows(self):
        self._bootstrap_wallet(label="WalletA")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "WalletB",
            "--kind",
            "phoenix",
        )
        self._assert_ok(payload, result, "wallets.create")
        self._insert_transaction(
            wallet_label="WalletA",
            tx_id="shared-out",
            external_id="shared-chain-tx",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
            direction="outbound",
        )
        self._insert_transaction(
            wallet_label="WalletB",
            tx_id="shared-in",
            external_id="shared-chain-tx",
            occurred_at="2024-05-01T12:02:00Z",
            amount_msat=99_000_000,
            direction="inbound",
        )

        payload, result = self._run_json(
            "metadata",
            "records",
            "get",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--transaction",
            "shared-chain-tx",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "ambiguous_reference")

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

        payload, result = self._run_json(
            "journals", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--limit", "2",
        )
        self._assert_ok(payload, result, "journals.list")
        self.assertTrue(payload["next_cursor"])
        journal_cursor = payload["next_cursor"]
        payload, result = self._run_json(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Other",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "journals", "list",
            "--workspace", "Main",
            "--profile", "Other",
            "--limit", "2",
            "--cursor", journal_cursor,
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

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
            "metadata", "records", "history", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "demo-bip329-tx",
        )
        self._assert_ok(payload, result, "metadata.records.history.list")
        self.assertTrue(
            any(
                event["reason"] == "Imported BIP329 label for demo-bip329-tx"
                and any(field["field"] == "tags" for field in event["fields"])
                for event in payload["data"]["events"]
            )
        )

        payload, result = self._run_json(
            "metadata", "bip329", "list",
            "--workspace", "Main",
            "--profile", "Default",
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
            "--file", str(export_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        self.assertEqual(payload["data"]["exported"], 2)
        exported = [json.loads(line) for line in export_file.read_text(encoding="utf-8").splitlines() if line.strip()]
        self.assertEqual(exported[0]["type"], "output")
        self.assertEqual(exported[0]["ref"], "demo-bip329-tx:0")
        self.assertEqual(exported[0]["label"], "change")
        self.assertEqual(exported[0]["origin"], "wallet")
        self.assertFalse(exported[0]["spendable"])
        self.assertEqual(exported[0]["kassiber"]["wallet_match"]["status"], "unmatched")
        self.assertEqual(exported[1]["type"], "tx")
        self.assertEqual(exported[1]["ref"], "demo-bip329-tx")
        self.assertEqual(exported[1]["label"], "merchant")
        self.assertEqual(exported[1]["kassiber"]["wallet_match"]["status"], "exact")
        self.assertEqual(exported[1]["kassiber"]["wallet_match"]["wallets"], ["Labels"])

    def test_bip329_import_preserves_ambiguous_transaction_labels_until_opt_in(self):
        self._bootstrap_wallet(label="LabelsA")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "LabelsB",
            "--kind",
            "phoenix",
        )
        self._assert_ok(payload, result, "wallets.create")
        self._insert_transaction(
            wallet_label="LabelsA",
            tx_id="bip329-a",
            external_id="shared-bip329-tx",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="LabelsB",
            tx_id="bip329-b",
            external_id="shared-bip329-tx",
            occurred_at="2024-05-01T12:01:00Z",
            amount_msat=200_000_000,
        )
        bip329_file = self.case_dir / "profile-labels.jsonl"
        bip329_file.write_text(
            json.dumps({"type": "tx", "ref": "shared-bip329-tx", "label": "merchant"}) + "\n",
            encoding="utf-8",
        )

        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")
        self.assertEqual(payload["data"]["records"], 1)
        self.assertEqual(payload["data"]["imported"], 1)
        self.assertEqual(payload["data"]["updated"], 0)
        self.assertEqual(payload["data"]["transaction_tags_created"], 0)
        self.assertEqual(payload["data"]["transaction_tags_added"], 0)
        self.assertEqual(payload["data"]["preview"]["counts"]["ambiguous"], 1)
        self.assertEqual(payload["data"]["preview"]["counts"]["tag_skipped_ambiguous"], 2)

        for wallet_label in ("LabelsA", "LabelsB"):
            payload, result = self._run_json(
                "metadata", "records", "list",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", wallet_label,
            )
            self._assert_ok(payload, result, "metadata.records.list")
            self.assertEqual(len(payload["data"]["records"]), 1)
            self.assertEqual(payload["data"]["records"][0]["tags"], [])

        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(bip329_file),
            "--apply-ambiguous",
        )
        self._assert_ok(payload, result, "metadata.bip329.import")
        self.assertEqual(payload["data"]["records"], 1)
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["updated"], 1)
        self.assertEqual(payload["data"]["transaction_tags_created"], 1)
        self.assertEqual(payload["data"]["transaction_tags_added"], 2)

        for wallet_label in ("LabelsA", "LabelsB"):
            payload, result = self._run_json(
                "metadata", "records", "list",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", wallet_label,
            )
            self._assert_ok(payload, result, "metadata.records.list")
            self.assertEqual(len(payload["data"]["records"]), 1)
            self.assertEqual(
                payload["data"]["records"][0]["tags"],
                [{"code": "merchant", "label": "merchant"}],
            )

    def test_bip329_import_deduplicates_and_merges_overwrites(self):
        self._bootstrap_wallet(label="Labels")
        self._insert_transaction(
            wallet_label="Labels",
            tx_id="dedupe-tx",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        bip329_file = self.case_dir / "duplicate-labels.jsonl"
        bip329_file.write_text(
            "\n".join(
                [
                    json.dumps({"type": "tx", "ref": "dedupe-tx", "label": "first-tx-label"}),
                    json.dumps({"type": "tx", "ref": "dedupe-tx", "label": "final-tx-label"}),
                    json.dumps(
                        {
                            "type": "output",
                            "ref": "dedupe-tx:0",
                            "label": "change",
                            "origin": "wallet",
                            "spendable": False,
                            "color": "blue",
                        }
                    ),
                    json.dumps(
                        {
                            "type": "output",
                            "ref": "dedupe-tx:0",
                            "label": "reserve",
                            "note": "second import wins",
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
            "--file", str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")
        self.assertEqual(payload["data"]["records"], 4)
        self.assertEqual(payload["data"]["imported"], 2)
        self.assertEqual(payload["data"]["updated"], 2)
        self.assertEqual(payload["data"]["transaction_tags_created"], 1)
        self.assertEqual(payload["data"]["transaction_tags_added"], 1)

        payload, result = self._run_json(
            "metadata", "bip329", "list",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "metadata.bip329.list")
        rows = sorted(payload["data"], key=lambda row: (row["type"], row["ref"]))
        self.assertEqual(len(rows), 2)
        self.assertEqual(rows[0]["type"], "output")
        self.assertEqual(rows[0]["ref"], "dedupe-tx:0")
        self.assertEqual(rows[0]["label"], "reserve")
        self.assertEqual(rows[0]["origin"], "wallet")
        self.assertEqual(rows[0]["spendable"], "false")
        self.assertEqual(rows[1]["type"], "tx")
        self.assertEqual(rows[1]["ref"], "dedupe-tx")
        self.assertEqual(rows[1]["label"], "final-tx-label")

        payload, result = self._run_json(
            "metadata", "records", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Labels",
        )
        self._assert_ok(payload, result, "metadata.records.list")
        self.assertEqual(len(payload["data"]["records"]), 1)
        self.assertEqual(
            payload["data"]["records"][0]["tags"],
            [{"code": "final-tx-label", "label": "final-tx-label"}],
        )

        export_file = self.case_dir / "dedupe-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(export_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        exported = [json.loads(line) for line in export_file.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(exported[0]["type"], "output")
        self.assertEqual(exported[0]["ref"], "dedupe-tx:0")
        self.assertEqual(exported[0]["label"], "reserve")
        self.assertEqual(exported[0]["origin"], "wallet")
        self.assertFalse(exported[0]["spendable"])
        self.assertEqual(exported[0]["color"], "blue")
        self.assertEqual(exported[0]["note"], "second import wins")
        self.assertEqual(exported[0]["kassiber"]["wallet_match"]["status"], "unmatched")
        self.assertEqual(exported[1]["type"], "tx")
        self.assertEqual(exported[1]["ref"], "dedupe-tx")
        self.assertEqual(exported[1]["label"], "final-tx-label")
        self.assertEqual(exported[1]["kassiber"]["wallet_match"]["status"], "exact")

    def test_bip329_preview_reports_match_buckets_duplicates_and_conflicts(self):
        self._bootstrap_wallet(label="LabelsA")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "LabelsB",
            "--kind", "phoenix",
        )
        self._assert_ok(payload, result, "wallets.create")
        self._insert_transaction(
            wallet_label="LabelsA",
            tx_id="exact-preview",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="LabelsA",
            tx_id="shared-preview-a",
            external_id="shared-preview",
            occurred_at="2024-05-01T12:01:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="LabelsB",
            tx_id="shared-preview-b",
            external_id="shared-preview",
            occurred_at="2024-05-01T12:02:00Z",
            amount_msat=100_000_000,
        )
        existing_file = self.case_dir / "preview-existing.jsonl"
        existing_file.write_text(
            json.dumps({"type": "tx", "ref": "exact-preview", "label": "old-label"}) + "\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(existing_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")

        preview_file = self.case_dir / "preview-labels.jsonl"
        preview_file.write_text(
            "\n".join(
                [
                    json.dumps({"type": "tx", "ref": "exact-preview", "label": "old-label"}),
                    json.dumps({"type": "tx", "ref": "shared-preview", "label": "shared-label"}),
                    json.dumps({"type": "tx", "ref": "missing-preview", "label": "missing-label"}),
                    json.dumps({"type": "tx", "ref": "exact-preview", "label": "new-label"}),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "metadata", "bip329", "preview",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(preview_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.preview")
        counts = payload["data"]["counts"]
        self.assertEqual(counts["exact"], 2)
        self.assertEqual(counts["ambiguous"], 1)
        self.assertEqual(counts["unmatched"], 1)
        self.assertEqual(counts["duplicate_refs"], 1)
        self.assertEqual(counts["duplicate_records"], 2)
        self.assertEqual(counts["conflicts"], 1)
        self.assertEqual(counts["tag_additions"], 1)
        self.assertEqual(counts["tag_skipped_ambiguous"], 2)
        conflict_rows = [row for row in payload["data"]["rows"] if row["conflicts"]]
        self.assertEqual(conflict_rows[0]["ref"], "exact-preview")
        self.assertEqual(conflict_rows[0]["conflicts"], ["label"])

    def test_bip329_import_accepts_spscan_and_preserves_unknown_fields(self):
        self._bootstrap_wallet(label="Labels")
        spscan = "spscan1q" + ("p" * 40)
        bip329_file = self.case_dir / "spscan-labels.jsonl"
        bip329_file.write_text(
            json.dumps(
                {
                    "type": "spscan",
                    "ref": spscan,
                    "label": "silent-payment scanner",
                    "note": "round-trip me",
                }
            )
            + "\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")
        self.assertEqual(payload["data"]["records"], 1)
        self.assertEqual(payload["data"]["preview"]["counts"]["preserved"], 1)

        export_file = self.case_dir / "spscan-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(export_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        exported = [json.loads(line) for line in export_file.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(exported[0]["type"], "spscan")
        self.assertEqual(exported[0]["ref"], spscan)
        self.assertEqual(exported[0]["label"], "silent-payment scanner")
        self.assertEqual(exported[0]["note"], "round-trip me")
        self.assertEqual(exported[0]["kassiber"]["wallet_match"]["status"], "preserved")
        third_party_view = {key: value for key, value in exported[0].items() if key != "kassiber"}
        self.assertEqual(third_party_view["type"], "spscan")
        self.assertEqual(third_party_view["ref"], spscan)

    def test_bip329_wallet_scoped_and_synthesized_exports(self):
        self._bootstrap_wallet(label="LabelsA")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "LabelsB",
            "--kind", "phoenix",
        )
        self._assert_ok(payload, result, "wallets.create")
        self._insert_transaction(
            wallet_label="LabelsA",
            tx_id="exact-export",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="LabelsA",
            tx_id="shared-export-a",
            external_id="shared-export",
            occurred_at="2024-05-01T12:01:00Z",
            amount_msat=100_000_000,
        )
        self._insert_transaction(
            wallet_label="LabelsB",
            tx_id="shared-export-b",
            external_id="shared-export",
            occurred_at="2024-05-01T12:02:00Z",
            amount_msat=100_000_000,
        )
        bip329_file = self.case_dir / "wallet-scope-labels.jsonl"
        bip329_file.write_text(
            "\n".join(
                [
                    json.dumps({"type": "tx", "ref": "exact-export", "label": "exact label"}),
                    json.dumps({"type": "tx", "ref": "shared-export", "label": "shared label"}),
                    json.dumps({"type": "tx", "ref": "missing-export", "label": "missing label"}),
                ]
            )
            + "\n",
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "metadata", "bip329", "import",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(bip329_file),
        )
        self._assert_ok(payload, result, "metadata.bip329.import")

        profile_export = self.case_dir / "profile-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--file", str(profile_export),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        self.assertEqual(payload["data"]["exported"], 3)

        wallet_export = self.case_dir / "wallet-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "LabelsA",
            "--file", str(wallet_export),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        exported = [json.loads(line) for line in wallet_export.read_text(encoding="utf-8").splitlines()]
        self.assertEqual([row["ref"] for row in exported], ["exact-export"])

        payload, result = self._run_json(
            "metadata", "tags", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--code", "reviewed-source",
            "--label", "reviewed-source",
        )
        self._assert_ok(payload, result, "metadata.tags.create")
        payload, result = self._run_json(
            "metadata", "tags", "add",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "exact-export",
            "--tag", "reviewed-source",
        )
        self._assert_ok(payload, result, "metadata.tags.add")
        payload, result = self._run_json(
            "metadata", "notes", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "exact-export",
            "--note", "Known merchant payout",
        )
        self._assert_ok(payload, result, "metadata.notes.set")
        synth_export = self.case_dir / "synth-export.jsonl"
        payload, result = self._run_json(
            "metadata", "bip329", "export",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "LabelsA",
            "--mode", "synthesized",
            "--file", str(synth_export),
        )
        self._assert_ok(payload, result, "metadata.bip329.export")
        self.assertEqual(payload["data"]["exported_synthesized"], 1)
        synthesized = [json.loads(line) for line in synth_export.read_text(encoding="utf-8").splitlines()]
        self.assertEqual(synthesized[0]["type"], "tx")
        self.assertEqual(synthesized[0]["ref"], "exact-export")
        self.assertIn("reviewed-source", synthesized[0]["label"])
        self.assertEqual(synthesized[0]["value"], 100000)
        self.assertEqual(synthesized[0]["kassiber"]["source"], "synthesized")
        third_party_view = {key: value for key, value in synthesized[0].items() if key != "kassiber"}
        self.assertEqual(third_party_view["type"], "tx")
        self.assertIn("label", third_party_view)

    def _insert_transaction(
        self,
        *,
        wallet_label,
        tx_id,
        occurred_at,
        amount_msat,
        direction="inbound",
        asset="BTC",
        external_id=None,
    ):
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
                external_id or tx_id,
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
                "external_id": _FIXTURE_SELF_TRANSFER_TXID,
                "raw_json": _typed_onchain_raw(_FIXTURE_SELF_TRANSFER_TXID),
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
                "external_id": _FIXTURE_SELF_TRANSFER_TXID,
                "raw_json": _typed_onchain_raw(_FIXTURE_SELF_TRANSFER_TXID),
                "created_at": "2026-02-01T12:00:00Z",
            },
        ]
        return profile, finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[
                {
                    "id": "reviewed-self-transfer",
                    "out_transaction_id": "onchain-self-transfer-1-out",
                    "in_transaction_id": "onchain-self-transfer-1-in",
                    "kind": "manual",
                    "policy": "carrying-value",
                }
            ],
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
        return profile, finalized_tax_inputs(
            profile,
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
        return profile, finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=manual_pairs,
        )

    def _direct_austrian_transfer_then_sell_inputs(self):
        # bitcoinaustria/kassiber#213: Neu BTC acquired in wallet-a, moved to wallet-b (same-asset
        # transfer), then sold from wallet-b. With per-wallet pools the sale was tagged at_pool=wallet-b
        # while the lot stayed at_pool=wallet-a, so rp2's moving_average_at found no lots in the
        # disposal's pool and aborted ("Total in-transaction crypto value < total taxable"). With the
        # single global per-asset pool both share at_pool=default and the sale resolves cleanly.
        profile = {
            "id": "profile-at",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianTransferSell",
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
                "label": "Cold",
                "wallet_account_id": "account-cold",
                "account_code": "cold",
                "account_label": "Cold",
            },
        }

        def _row(rid, wid, direction, amount, occurred_at, *, fiat_rate, fiat_value, kind, description, external_id):
            ref = wallet_refs_by_id[wid]
            return {
                "id": rid,
                "wallet_id": wid,
                "wallet_label": ref["label"],
                "wallet_account_id": ref["wallet_account_id"],
                "account_code": ref["account_code"],
                "account_label": ref["account_label"],
                "occurred_at": occurred_at,
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": 0,
                "fiat_rate": fiat_rate,
                "fiat_value": fiat_value,
                "kind": kind,
                "note": None,
                "description": description,
                "external_id": external_id,
                "raw_json": _typed_onchain_raw(external_id),
                "created_at": occurred_at,
            }

        rows = [
            _row("neu-buy", "wallet-a", "inbound", 100_000_000_000, "2024-06-01T10:00:00Z", fiat_rate=30000, fiat_value=30000, kind="deposit", description="Vienna Neu buy", external_id="neu-buy"),
            # Same-asset A -> B transfer, explicitly reviewed because these are
            # synthetic/import-shaped rows rather than observer authority.
            _row("xfer-out", "wallet-a", "outbound", 100_000_000_000, "2024-07-01T10:00:00Z", fiat_rate=30000, fiat_value=30000, kind="withdrawal", description="Move A->B", external_id=_AT_TRANSFER_TXID),
            _row("xfer-in", "wallet-b", "inbound", 100_000_000_000, "2024-07-01T10:00:00Z", fiat_rate=30000, fiat_value=30000, kind="deposit", description="Move A->B", external_id=_AT_TRANSFER_TXID),
            _row("neu-sell", "wallet-b", "outbound", 30_000_000_000, "2025-03-01T09:00:00Z", fiat_rate=50000, fiat_value=15000, kind="withdrawal", description="Sell from B", external_id="neu-sell"),
        ]
        return profile, finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[
                {
                    "id": "reviewed-at-transfer",
                    "out_transaction_id": "xfer-out",
                    "in_transaction_id": "xfer-in",
                    "kind": "manual",
                    "policy": "carrying-value",
                }
            ],
        )

    def _direct_austrian_swap_payout_inputs(self):
        profile = {
            "id": "profile-at-payout",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianSwapPayout",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 9223372036854775807,
            "gains_algorithm": "MOVING_AVERAGE_AT",
        }
        wallet_refs_by_id = {
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
                "id": "lbtc-buy-1",
                "wallet_id": "wallet-liquid",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2024-06-01T10:00:00Z",
                "direction": "inbound",
                "asset": "LBTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 30000,
                "fiat_value": 30000,
                "kind": "deposit",
                "description": "Liquid Neu buy",
                "note": None,
                "external_id": "lbtc-buy-1",
                "created_at": "2024-06-01T10:00:00Z",
            },
            {
                "id": "swap-payout-source",
                "wallet_id": "wallet-liquid",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-01T09:00:00Z",
                "direction": "outbound",
                "asset": "LBTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 25000,
                "kind": "withdrawal",
                "description": "Liquid direct swap payout",
                "note": None,
                "external_id": "swap-payout-source",
                "created_at": "2025-03-01T09:00:00Z",
            },
        ]
        direct_payouts = [
            {
                "id": "direct-payout-1",
                "out_transaction_id": "swap-payout-source",
                "kind": "direct-swap-payout",
                "policy": "carrying-value",
                "payout_asset": "BTC",
                "payout_amount": 49_990_000_000,
                "payout_occurred_at": "2025-03-01T09:00:30Z",
                "payout_fiat_value": 24995,
                "payout_external_id": "recipient-txid",
                "counterparty": "external-recipient",
                "notes": "Privacy swap payout",
                "swap_fee_msat": 10_000_000,
                "swap_fee_kind": "combined",
                "created_at": "2025-03-01T09:01:00Z",
            },
        ]
        return profile, finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[],
            direct_payout_records=direct_payouts,
        )

    def _direct_generic_swap_payout_inputs(self):
        profile = {
            "id": "profile-generic-payout",
            "workspace_id": "workspace-main",
            "label": "FixtureGenericSwapPayout",
            "fiat_currency": "EUR",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs_by_id = {
            "wallet-btc": {
                "id": "wallet-btc",
                "label": "Bitcoin",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        rows = [
            {
                "id": "btc-buy-1",
                "wallet_id": "wallet-btc",
                "wallet_label": "Bitcoin",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2024-06-01T10:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": 10000,
                "fiat_value": 10000,
                "kind": "buy",
                "description": "Initial BTC lot",
                "note": None,
                "external_id": "btc-buy-1",
                "created_at": "2024-06-01T10:00:00Z",
            },
            {
                "id": "btc-direct-payout-source",
                "wallet_id": "wallet-btc",
                "wallet_label": "Bitcoin",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-01T09:00:00Z",
                "direction": "outbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 40000,
                "fiat_value": 20000,
                "kind": "withdrawal",
                "description": "Direct provider payout",
                "note": None,
                "external_id": "btc-direct-payout-source",
                "created_at": "2025-03-01T09:00:00Z",
            },
        ]
        direct_payouts = [
            {
                "id": "generic-direct-payout-1",
                "out_transaction_id": "btc-direct-payout-source",
                "kind": "direct-swap-payout",
                "policy": "taxable",
                "payout_asset": "BTC",
                "payout_amount": 49_990_000_000,
                "payout_occurred_at": "2025-03-01T09:00:30Z",
                "payout_fiat_value": 25000,
                "payout_external_id": "exchange-deposit-txid",
                "counterparty": "external-exchange",
                "notes": "Reviewed exchange sale proceeds",
                "swap_fee_msat": 10_000_000,
                "swap_fee_kind": "combined",
                "created_at": "2025-03-01T09:01:00Z",
            },
        ]
        return profile, finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[],
            direct_payout_records=direct_payouts,
        )

    def _direct_austrian_same_timestamp_swap_chain_inputs(self):
        profile = {
            "id": "profile-at-chain",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianSwapChain",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 9223372036854775807,
            "gains_algorithm": "MOVING_AVERAGE_AT",
        }
        wallet_refs_by_id = {
            "wallet-btc": {
                "id": "wallet-btc",
                "label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-lbtc": {
                "id": "wallet-lbtc",
                "label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
            "wallet-xyz": {
                "id": "wallet-xyz",
                "label": "Sidechain",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
            },
        }
        swap_time = "2025-03-01T09:00:00Z"
        rows = [
            {
                "id": "btc-neu-buy",
                "wallet_id": "wallet-btc",
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
                "external_id": "btc-neu-buy",
                "created_at": "2024-06-01T10:00:00Z",
            },
            {
                "id": "btc-to-lbtc-out",
                "wallet_id": "wallet-btc",
                "wallet_label": "Vienna",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": swap_time,
                "direction": "outbound",
                "asset": "BTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 25000,
                "kind": "withdrawal",
                "description": "Peg-out BTC->LBTC",
                "note": None,
                "external_id": "btc-to-lbtc-out",
                "created_at": swap_time,
            },
            {
                "id": "a-lbtc-to-xyz-out",
                "wallet_id": "wallet-lbtc",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": swap_time,
                "direction": "outbound",
                "asset": "LBTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 52000,
                "fiat_value": 26000,
                "kind": "withdrawal",
                "description": "Peg-out LBTC->XYZ",
                "note": None,
                "external_id": "lbtc-to-xyz-out",
                "created_at": swap_time,
            },
            {
                "id": "z-lbtc-from-btc-in",
                "wallet_id": "wallet-lbtc",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": swap_time,
                "direction": "inbound",
                "asset": "LBTC",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 50000,
                "fiat_value": 25000,
                "kind": "deposit",
                "description": "Peg-in BTC->LBTC",
                "note": None,
                "external_id": "lbtc-from-btc-in",
                "created_at": swap_time,
            },
            {
                "id": "xyz-from-lbtc-in",
                "wallet_id": "wallet-xyz",
                "wallet_label": "Sidechain",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": swap_time,
                "direction": "inbound",
                "asset": "XYZ",
                "amount": 50_000_000_000,
                "fee": 0,
                "fiat_rate": 52000,
                "fiat_value": 26000,
                "kind": "deposit",
                "description": "Peg-in LBTC->XYZ",
                "note": None,
                "external_id": "xyz-from-lbtc-in",
                "created_at": swap_time,
            },
        ]
        manual_pairs = [
            {
                "id": "pair-btc-lbtc",
                "out_transaction_id": "btc-to-lbtc-out",
                "in_transaction_id": "z-lbtc-from-btc-in",
                "policy": "carrying-value",
                "kind": "swap",
            },
            {
                "id": "pair-lbtc-xyz",
                "out_transaction_id": "a-lbtc-to-xyz-out",
                "in_transaction_id": "xyz-from-lbtc-in",
                "policy": "carrying-value",
                "kind": "swap",
            },
        ]
        return profile, finalized_tax_inputs(
            profile,
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
        return profile, finalized_tax_inputs(
            profile,
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
        return profile, finalized_tax_inputs(
            profile,
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

    def _direct_austrian_single_wallet_inputs(self, rows):
        profile = {
            "id": "profile-at-direct",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianDirect",
            "fiat_currency": "EUR",
            "tax_country": "at",
            "tax_long_term_days": 365,
            "gains_algorithm": "moving_average_at",
        }
        wallet_ref = {
            "id": "wallet-austrian",
            "label": "AustrianDirect",
            "wallet_account_id": "account-treasury",
            "account_code": "treasury",
            "account_label": "Treasury",
        }
        normalized_rows = []
        for row in rows:
            occurred_at = row["occurred_at"]
            normalized_rows.append(
                {
                    "id": row["id"],
                    "wallet_id": wallet_ref["id"],
                    "wallet_label": wallet_ref["label"],
                    "wallet_account_id": wallet_ref["wallet_account_id"],
                    "account_code": wallet_ref["account_code"],
                    "account_label": wallet_ref["account_label"],
                    "occurred_at": occurred_at,
                    "direction": row["direction"],
                    "asset": row.get("asset", "BTC"),
                    "amount": row["amount"],
                    "fee": row.get("fee", 0),
                    "fiat_rate": row.get("fiat_rate"),
                    "fiat_value": row.get("fiat_value"),
                    "kind": row["kind"],
                    "description": row.get("description", row["id"]),
                    "note": None,
                    "external_id": row["id"],
                    "created_at": occurred_at,
                }
            )
        return profile, finalized_tax_inputs(
            profile,
            rows=normalized_rows,
            wallet_refs_by_id={wallet_ref["id"]: wallet_ref},
            manual_pair_records=[],
        )

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

    def test_btcpay_import_marks_outbound_amount_includes_fee(self):
        # BTCPay's net-wallet-delta amounts fold the miner fee into an outbound's
        # `amount`. The importer must flag outbound rows `amount_includes_fee=1`
        # (and inbound rows 0) and persist it through normalize_import_record and
        # the INSERT, so the transfer-fee guard treats the out/in gap as the fee.
        self._bootstrap_wallet(label="BTCPay")
        btcpay_csv = self.case_dir / "btcpay.csv"
        btcpay_csv.write_text(
            "TransactionId,Timestamp,Currency,Amount,Comment,Labels\n"
            "send-1,2024-02-01T00:00:00Z,BTC,-0.00103 BTC,send,\n"
            "recv-1,2024-01-01T00:00:00Z,BTC,0.5 BTC,receive,\n",
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
        self.assertEqual(payload["data"]["imported"], 2)
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        self.addCleanup(conn.close)
        flags = {
            row["external_id"]: row["amount_includes_fee"]
            for row in conn.execute(
                "SELECT external_id, amount_includes_fee FROM transactions"
            ).fetchall()
        }
        self.assertEqual(flags["send-1"], 1)
        self.assertEqual(flags["recv-1"], 0)

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

    def test_wallets_attach_btcpay_stores_provenance_route_and_dedupes(self):
        # `wallets attach-btcpay` is the CLI mirror of the desktop existing-
        # wallets mapping flow. It must validate the backend, allowlist the
        # payment method, and dedupe routes so repeated invocations are
        # idempotent without accidentally clearing earlier routes.
        self._bootstrap_wallet(label="Merchant", kind="custom")
        payload, result = self._run_json(
            "backends", "create",
            "shop-btcpay",
            "--kind", "btcpay",
            "--url", "http://127.0.0.1:9",
            "--token", "shopkey",
        )
        self._assert_ok(payload, result, "backends.create")

        # Wrong backend kind -> validation error, no config change.
        payload, result = self._run_json(
            "backends", "create",
            "esplora1",
            "--kind", "esplora",
            "--url", "https://example.invalid",
        )
        self._assert_ok(payload, result, "backends.create")
        payload, result = self._run_json(
            "wallets", "attach-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Merchant",
            "--backend", "esplora1",
            "--store-id", "STORE1",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")

        # Happy path: route is stored with the canonical payment method id.
        payload, result = self._run_json(
            "wallets", "attach-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Merchant",
            "--backend", "shop-btcpay",
            "--store-id", "STORE1",
            "--payment-method-id", "btc-chain",
        )
        self._assert_ok(payload, result, "wallets.attach-btcpay")
        self.assertEqual(
            payload["data"]["config"]["btcpay_provenance"],
            [
                {
                    "backend": "shop-btcpay",
                    "store_id": "STORE1",
                    "payment_method_id": "BTC-CHAIN",
                }
            ],
        )

        # Same backend + store + method -> dedupes, route count stays at 1.
        payload, result = self._run_json(
            "wallets", "attach-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Merchant",
            "--backend", "shop-btcpay",
            "--store-id", "STORE1",
        )
        self._assert_ok(payload, result, "wallets.attach-btcpay")
        self.assertEqual(
            len(payload["data"]["config"]["btcpay_provenance"]),
            1,
        )

        # Different store on the same instance -> appended as a new route.
        payload, result = self._run_json(
            "wallets", "attach-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Merchant",
            "--backend", "shop-btcpay",
            "--store-id", "STORE2",
        )
        self._assert_ok(payload, result, "wallets.attach-btcpay")
        self.assertEqual(
            payload["data"]["config"]["btcpay_provenance"],
            [
                {
                    "backend": "shop-btcpay",
                    "store_id": "STORE1",
                    "payment_method_id": "BTC-CHAIN",
                },
                {
                    "backend": "shop-btcpay",
                    "store_id": "STORE2",
                    "payment_method_id": "BTC-CHAIN",
                },
            ],
        )

        # BTC-LN remains blocked at the attach gate, just like sync-btcpay.
        payload, result = self._run_json(
            "wallets", "attach-btcpay",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "Merchant",
            "--backend", "shop-btcpay",
            "--store-id", "STORE1",
            "--payment-method-id", "BTC-LN",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("wallet-history sync", payload["error"]["message"])

    def test_btcpay_provenance_enriches_existing_wallet_during_wallets_sync(self):
        # The existing-wallets BTCPay mapping mode persists btcpay_provenance
        # routes on a settlement wallet, and `wallets sync` is supposed to
        # walk those routes and apply notes/tags to matching transactions.
        # Without this end-to-end test the routes are inert storage — the
        # rest of the feature could regress silently.
        received_paths: list[str] = []

        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                received_paths.append(self.path)
                if self.headers.get("Authorization") != "token testkey":
                    self.send_error(401, "unauthorized")
                    return
                parsed = urlparse(self.path)
                if parsed.path != (
                    "/api/v1/stores/STORE1/payment-methods/BTC-CHAIN/wallet/transactions"
                ):
                    self.send_error(404, "not found")
                    return
                body = json.dumps(
                    [
                        {
                            "transactionHash": "tx-settlement-1",
                            "comment": "paid order 42",
                            "amount": "0.001",
                            "timestamp": 1704067200,
                            "status": "Confirmed",
                            "confirmations": 6,
                            "labels": ["merchant"],
                        }
                    ]
                ).encode("utf-8")
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
            # The settlement wallet's CSV source seeds the row with an empty
            # Comment so the BTCPay enrichment has space to land a note.
            self._bootstrap_wallet(label="Settlement", kind="custom")
            settlement_csv = self.case_dir / "settlement.csv"
            settlement_csv.write_text(
                "TransactionId,Timestamp,Currency,Amount,Comment,Labels\n"
                "tx-settlement-1,2024-01-01T00:00:00Z,BTC,0.001 BTC,,\n",
                encoding="utf-8",
            )
            payload, result = self._run_json(
                "wallets", "update",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "Settlement",
                "--config", json.dumps(
                    {
                        "source_file": str(settlement_csv),
                        "source_format": "btcpay_csv",
                    }
                ),
            )
            self._assert_ok(payload, result, "wallets.update")

            payload, result = self._run_json(
                "backends", "create",
                "btcpay1",
                "--kind", "btcpay",
                "--url", f"http://127.0.0.1:{port}",
                "--token", "testkey",
            )
            self._assert_ok(payload, result, "backends.create")

            payload, result = self._run_json(
                "wallets", "update",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "Settlement",
                "--config", json.dumps(
                    {
                        "btcpay_provenance": [
                            {
                                "backend": "btcpay1",
                                "store_id": "STORE1",
                                "payment_method_id": "BTC-CHAIN",
                            }
                        ]
                    }
                ),
            )
            self._assert_ok(payload, result, "wallets.update")

            payload, result = self._run_json(
                "wallets", "sync",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "Settlement",
            )
            self._assert_ok(payload, result, "wallets.sync")
            self.assertEqual(len(payload["data"]), 1)
            synced = payload["data"][0]
            self.assertEqual(synced["wallet"], "Settlement")
            self.assertEqual(synced["status"], "synced")
            provenance = synced["btcpay_provenance"]
            self.assertEqual(provenance["routes"], 1)
            self.assertEqual(provenance["fetched"], 1)
            self.assertEqual(provenance["btcpay_notes_set"], 1)
            self.assertEqual(provenance["btcpay_tags_added"], 1)
            self.assertEqual(provenance["btcpay_tags_created"], 1)
            self.assertEqual(
                provenance["route_results"][0]["payment_method_id"],
                "BTC-CHAIN",
            )

            self.assertTrue(
                any(
                    "/api/v1/stores/STORE1/payment-methods/BTC-CHAIN" in path
                    for path in received_paths
                )
            )

            payload, result = self._run_json(
                "transactions", "list",
                "--workspace", "Main",
                "--profile", "Default",
                "--wallet", "Settlement",
            )
            self._assert_ok(payload, result, "transactions.list")
            self.assertEqual(len(payload["data"]), 1)
            tx = payload["data"][0]
            self.assertEqual(tx["note"], "paid order 42")
            tag_codes = {tag["code"] for tag in tx.get("tags", [])}
            self.assertIn("merchant", tag_codes)
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

    def test_btcpay_sync_rejects_lightning_payment_method_id_before_persisting(self):
        self._bootstrap_wallet(label="BTCPayLightning", kind="custom")
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
            "--wallet", "BTCPayLightning",
            "--backend", "btcpay1",
            "--store-id", "STORE1",
            "--payment-method-id", "BTC-LN",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("wallet-history sync", payload["error"]["message"])

        payload, result = self._run_json(
            "wallets", "get",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPayLightning",
        )
        self._assert_ok(payload, result, "wallets.get")
        self.assertEqual(payload["data"]["config"], {})

    def test_normalize_btcpay_payment_method_id_canonicalises_case(self):
        # BTCPay's canonical form is upper case ("BTC-CHAIN", "BTC-LN"). All
        # wallet config writes and URL constructions go through this helper,
        # so canonicalising here keeps storage consistent regardless of how
        # the value reached us (CLI arg, JSON config, daemon discovery).
        from kassiber.core import wallets as core_wallets

        self.assertEqual(
            core_wallets.normalize_btcpay_payment_method_id("btc-chain"),
            "BTC-CHAIN",
        )
        self.assertEqual(
            core_wallets.normalize_btcpay_payment_method_id("  Btc-Chain  "),
            "BTC-CHAIN",
        )
        # Already-canonical values are returned unchanged.
        self.assertEqual(
            core_wallets.normalize_btcpay_payment_method_id("LBTC-CHAIN"),
            "LBTC-CHAIN",
        )

    def test_btcpay_sync_rejects_altcoin_chain_payment_method_id(self):
        # The wallet-history allowlist is intentionally narrow (BTC-CHAIN and
        # LBTC-CHAIN). Any other -CHAIN suffix must be rejected at the same
        # gate as BTC-LN so we never persist a sync config we cannot honour.
        self._bootstrap_wallet(label="BTCPayAltcoin", kind="custom")
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
            "--wallet", "BTCPayAltcoin",
            "--backend", "btcpay1",
            "--store-id", "STORE1",
            "--payment-method-id", "DOGE-CHAIN",
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(payload["kind"], "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("wallet-history sync", payload["error"]["message"])

        payload, result = self._run_json(
            "wallets", "get",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "BTCPayAltcoin",
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

    def test_transactions_list_machine_output_uses_typed_metadata(self):
        self._bootstrap_wallet(label="One")
        self._insert_transaction(
            wallet_label="One",
            tx_id="typed-metadata",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=100_000_000,
        )
        for code in ("zeta", "alpha"):
            payload, result = self._run_json(
                "metadata",
                "tags",
                "create",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--code",
                code,
                "--label",
                code.title(),
            )
            self._assert_ok(payload, result, "metadata.tags.create")
            payload, result = self._run_json(
                "metadata",
                "tags",
                "add",
                "--workspace",
                "Main",
                "--profile",
                "Default",
                "--transaction",
                "typed-metadata",
                "--tag",
                code,
            )
            self._assert_ok(payload, result, "metadata.tags.add")
        payload, result = self._run_json(
            "metadata",
            "exclude",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--transaction",
            "typed-metadata",
        )
        self._assert_ok(payload, result, "metadata.exclude")

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
        )
        self._assert_ok(payload, result, "transactions.list")
        record = payload["data"][0]
        self.assertTrue(record["excluded"])
        self.assertEqual(
            record["tags"],
            [
                {"code": "alpha", "label": "Alpha"},
                {"code": "zeta", "label": "Zeta"},
            ],
        )

    def test_transactions_list_sorts_before_applying_limit(self):
        self._bootstrap_wallet(label="Ranked")
        for idx in range(101):
            self._insert_transaction(
                wallet_label="Ranked",
                tx_id=f"recent-in-{idx:03d}",
                occurred_at="2024-06-01T00:00:00Z",
                amount_msat=100_000_000 + idx,
                direction="inbound",
            )
            self._insert_transaction(
                wallet_label="Ranked",
                tx_id=f"recent-out-{idx:03d}",
                occurred_at="2024-06-01T00:00:00Z",
                amount_msat=100_000_000 + idx,
                direction="outbound",
            )
        self._insert_transaction(
            wallet_label="Ranked",
            tx_id="old-largest-inbound",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=9_000_000_000,
            direction="inbound",
        )
        self._insert_transaction(
            wallet_label="Ranked",
            tx_id="old-largest-outbound",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=8_000_000_000,
            direction="outbound",
        )
        self._insert_transaction(
            wallet_label="Ranked",
            tx_id="old-smallest-outbound",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=1,
            direction="outbound",
        )

        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--direction", "inbound",
            "--sort", "amount",
            "--order", "desc",
            "--limit", "1",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["data"][0]["id"], "old-largest-inbound")
        self.assertEqual(payload["data"][0]["amount_msat"], 9_000_000_000)
        self.assertTrue(payload["has_more"])
        self.assertTrue(payload["next_cursor"])

        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--direction", "inbound",
            "--sort", "amount",
            "--order", "desc",
            "--limit", "1",
            "--cursor", payload["next_cursor"],
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["data"][0]["id"], "recent-in-100")

        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--direction", "outbound",
            "--sort", "amount",
            "--order", "desc",
            "--limit", "1",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["data"][0]["id"], "old-largest-outbound")
        self.assertEqual(payload["data"][0]["amount_msat"], 8_000_000_000)

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--cursor",
            "not-a-real-cursor",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

        payload, result = self._run_json(
            "transactions", "list",
            "--workspace", "Main",
            "--profile", "Default",
            "--direction", "outbound",
            "--sort", "amount",
            "--order", "asc",
            "--limit", "1",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["data"][0]["id"], "old-smallest-outbound")
        self.assertEqual(payload["data"][0]["amount_msat"], 1)

    def test_transactions_list_filters_and_rejects_cursor_filter_changes(self):
        self._bootstrap_wallet(label="Filtered")
        self._insert_transaction(
            wallet_label="Filtered",
            tx_id="btc-early",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=100_000_000,
            asset="BTC",
        )
        self._insert_transaction(
            wallet_label="Filtered",
            tx_id="lbtc-middle",
            occurred_at="2024-02-01T00:00:00Z",
            amount_msat=200_000_000,
            asset="LBTC",
        )
        self._insert_transaction(
            wallet_label="Filtered",
            tx_id="btc-late",
            occurred_at="2024-03-01T00:00:00Z",
            amount_msat=300_000_000,
            asset="BTC",
        )

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--asset",
            "BTC",
            "--start",
            "2024-02-15T00:00:00Z",
            "--end",
            "2024-03-31T23:59:59Z",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual([row["id"] for row in payload["data"]], ["btc-late"])

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--asset",
            "BTC",
            "--limit",
            "1",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertTrue(payload["next_cursor"])
        first_cursor = payload["next_cursor"]
        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--asset",
            "LBTC",
            "--limit",
            "1",
            "--cursor",
            first_cursor,
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

        payload, result = self._run_json(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "Other",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Other",
            "--asset",
            "BTC",
            "--limit",
            "1",
            "--cursor",
            first_cursor,
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

    def test_transactions_list_supports_extended_cli_filters(self):
        self._bootstrap_wallet(label="Cold", kind="custom")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "Liquid Pocket",
            "--kind",
            "custom",
        )
        self._assert_ok(payload, result, "wallets.create")

        public_txid = "ab" * 32
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="internal-receipt",
            external_id="invoice-1",
            occurred_at="2026-01-10T00:00:00Z",
            amount_msat=100_000_000,
            direction="inbound",
        )
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="public-spend",
            external_id=public_txid,
            occurred_at="2026-02-10T00:00:00Z",
            amount_msat=200_000_000,
            direction="outbound",
        )
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="failed-import",
            external_id="failed-import-source",
            occurred_at="2026-03-10T00:00:00Z",
            amount_msat=300_000_000,
            direction="inbound",
        )
        self._insert_transaction(
            wallet_label="Liquid Pocket",
            tx_id="liquid-row",
            external_id="liquid-row-source",
            occurred_at="2026-04-10T00:00:00Z",
            amount_msat=400_000_000,
            direction="inbound",
            asset="LBTC",
        )

        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        conn.execute(
            """
            UPDATE wallets
            SET config_json = ?
            WHERE label = 'Liquid Pocket'
            """,
            (json.dumps({"chain": "liquid", "network": "regtest"}),),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 70000, fiat_value = 140, fee = 1000
            WHERE id = 'public-spend'
            """,
        )
        conn.execute(
            """
            UPDATE transactions
            SET review_status = 'failed'
            WHERE id = 'failed-import'
            """,
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 71000, fiat_value = 284
            WHERE id = 'liquid-row'
            """,
        )
        conn.commit()

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--txid",
            "internal-receipt",
            "--txid",
            public_txid,
            "--period",
            "15years",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["count"], 2)
        self.assertEqual(payload["total"], 2)
        self.assertEqual({row["id"] for row in payload["data"]}, {"internal-receipt", "public-spend"})

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--period",
            "6months",
            "--limit",
            "1",
        )
        self._assert_ok(payload, result, "transactions.list")

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--quick",
            "missing_price",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual({row["id"] for row in payload["data"]}, {"internal-receipt", "failed-import"})

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--quick",
            "no_explorer_id",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(
            {row["id"] for row in payload["data"]},
            {"internal-receipt", "failed-import", "liquid-row"},
        )

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--status",
            "failed",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual([row["id"] for row in payload["data"]], ["failed-import"])

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--payment-method",
            "Liquid",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual([row["id"] for row in payload["data"]], ["liquid-row"])

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--network",
            "regtest",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual([row["id"] for row in payload["data"]], ["liquid-row"])

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--with-fees",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual([row["id"] for row in payload["data"]], ["public-spend"])

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--flow",
            "incoming",
            "--limit",
            "2",
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertEqual(payload["count"], 3)
        self.assertTrue(payload["has_more"])
        first_cursor = payload["next_cursor"]

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--flow",
            "incoming",
            "--limit",
            "2",
            "--cursor",
            first_cursor,
        )
        self._assert_ok(payload, result, "transactions.list")
        self.assertFalse(payload["has_more"])
        self.assertEqual(len(payload["data"]), 1)

        payload, result = self._run_json(
            "transactions",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--flow",
            "outgoing",
            "--limit",
            "2",
            "--cursor",
            first_cursor,
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

    def test_ui_transactions_snapshot_filters_count_and_paginate_server_side(self):
        self._bootstrap_wallet(label="Cold", kind="custom")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "Liquid Pocket",
            "--kind",
            "custom",
        )
        self._assert_ok(payload, result, "wallets.create")

        public_txid = "ab" * 32
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="internal-receipt",
            external_id="invoice-1",
            occurred_at="2024-01-10T00:00:00Z",
            amount_msat=100_000_000,
            direction="inbound",
        )
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="public-spend",
            external_id=public_txid,
            occurred_at="2024-02-10T00:00:00Z",
            amount_msat=200_000_000,
            direction="outbound",
        )
        self._insert_transaction(
            wallet_label="Cold",
            tx_id="failed-import",
            external_id="failed-import-source",
            occurred_at="2024-03-10T00:00:00Z",
            amount_msat=300_000_000,
            direction="inbound",
        )
        self._insert_transaction(
            wallet_label="Liquid Pocket",
            tx_id="liquid-row",
            external_id="liquid-row-source",
            occurred_at="2024-04-10T00:00:00Z",
            amount_msat=400_000_000,
            direction="inbound",
            asset="LBTC",
        )

        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        conn.execute(
            """
            UPDATE wallets
            SET config_json = ?
            WHERE label = 'Liquid Pocket'
            """,
            (json.dumps({"chain": "liquid", "network": "regtest"}),),
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 70000, fiat_value = 140, fee = 1000
            WHERE id = 'public-spend'
            """,
        )
        conn.execute(
            """
            UPDATE transactions
            SET review_status = 'failed'
            WHERE id = 'failed-import'
            """,
        )
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 71000, fiat_value = 284
            WHERE id = 'liquid-row'
            """,
        )
        conn.commit()

        by_txids = build_transactions_snapshot(
            conn,
            {"txids": ["internal-receipt", public_txid], "limit": 10},
        )
        self.assertEqual(by_txids["count"], 2)
        self.assertEqual(
            {row["id"] for row in by_txids["txs"]},
            {"internal-receipt", "public-spend"},
        )

        missing_price = build_transactions_snapshot(
            conn,
            {"quick": "missing_price", "limit": 10},
        )
        self.assertEqual(missing_price["count"], 2)
        self.assertEqual(
            {row["id"] for row in missing_price["txs"]},
            {"internal-receipt", "failed-import"},
        )

        no_explorer = build_transactions_snapshot(
            conn,
            {"quick": "no_explorer_id", "limit": 10},
        )
        self.assertEqual(no_explorer["count"], 3)
        self.assertNotIn("public-spend", {row["id"] for row in no_explorer["txs"]})

        failed = build_transactions_snapshot(conn, {"status": "failed", "limit": 10})
        self.assertEqual(failed["count"], 1)
        self.assertEqual(failed["txs"][0]["id"], "failed-import")

        liquid = build_transactions_snapshot(
            conn,
            {"payment_method": "Liquid", "network": "regtest", "limit": 10},
        )
        self.assertEqual(liquid["count"], 1)
        self.assertEqual(liquid["txs"][0]["id"], "liquid-row")

        with_fees = build_transactions_snapshot(
            conn,
            {"withFees": True, "limit": 10},
        )
        self.assertEqual(with_fees["count"], 1)
        self.assertEqual(with_fees["txs"][0]["id"], "public-spend")

        incoming_page = build_transactions_snapshot(
            conn,
            {
                "flow": "incoming",
                "since": "2024-01-01T00:00:00Z",
                "until": "2024-12-31T23:59:59Z",
                "limit": 1,
                "sort": "occurred-at",
                "order": "asc",
            },
        )
        self.assertEqual(incoming_page["count"], 3)
        self.assertEqual(incoming_page["txs"][0]["id"], "internal-receipt")
        self.assertTrue(incoming_page["hasMore"])
        next_page = build_transactions_snapshot(
            conn,
            {
                "flow": "incoming",
                "since": "2024-01-01T00:00:00Z",
                "until": "2024-12-31T23:59:59Z",
                "limit": 1,
                "sort": "occurred-at",
                "order": "asc",
                "cursor": incoming_page["nextCursor"],
            },
        )
        self.assertEqual(next_page["count"], 3)
        self.assertEqual(next_page["txs"][0]["id"], "failed-import")

        with self.assertRaises(AppError):
            build_transactions_snapshot(
                conn,
                {
                    "flow": "outgoing",
                    "since": "2024-01-01T00:00:00Z",
                    "until": "2024-12-31T23:59:59Z",
                    "limit": 1,
                    "sort": "occurred-at",
                    "order": "asc",
                    "cursor": incoming_page["nextCursor"],
                },
            )

    def test_report_journal_entries_is_not_truncated_to_internal_page_size(self):
        self._bootstrap_wallet(label="Ledger")
        self._insert_transaction(
            wallet_label="Ledger",
            tx_id="ledger-source",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=100_000_000,
        )
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.row_factory = sqlite3.Row
        profile = conn.execute(
            "SELECT id, workspace_id FROM profiles WHERE label = 'Default'"
        ).fetchone()
        wallet = conn.execute("SELECT id FROM wallets WHERE label = 'Ledger'").fetchone()
        rows = [
            (
                f"journal-{idx:04d}",
                profile["workspace_id"],
                profile["id"],
                "ledger-source",
                wallet["id"],
                None,
                "2024-01-01T00:00:00Z",
                "deposit",
                "BTC",
                100_000,
                1.0,
                1.0,
                1.0,
                0.0,
                0.0,
                f"journal row {idx:04d}",
                None,
                None,
                f"2024-01-01T00:00:{idx % 60:02d}Z",
            )
            for idx in range(1001)
        ]
        conn.executemany(
            """
            INSERT INTO journal_entries(
                id, workspace_id, profile_id, transaction_id, wallet_id, account_id,
                occurred_at, entry_type, asset, quantity, fiat_value, unit_cost,
                cost_basis, proceeds, gain_loss, description, at_category,
                at_kennzahl, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            rows,
        )
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = ?, last_processed_tx_count = 1
            WHERE id = ?
            """,
            ("2024-01-01T00:00:00Z", profile["id"]),
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "journals",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "1000",
        )
        self._assert_ok(payload, result, "journals.list")
        self.assertEqual(len(payload["data"]), 1000)
        self.assertTrue(payload["has_more"])
        self.assertTrue(payload["next_cursor"])

        payload, result = self._run_json(
            "journals",
            "list",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--limit",
            "1000",
            "--cursor",
            payload["next_cursor"],
        )
        self._assert_ok(payload, result, "journals.list")
        self.assertEqual(len(payload["data"]), 1)
        self.assertFalse(payload["has_more"])

        payload, result = self._run_json(
            "reports",
            "journal-entries",
            "--workspace",
            "Main",
            "--profile",
            "Default",
        )
        self._assert_ok(payload, result, "reports.journal-entries")
        self.assertEqual(len(payload["data"]), 1001)

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

    def test_rates_range_uses_deterministic_order(self):
        for source, rate in (("coingecko", "60000"), ("manual", "65000")):
            payload, result = self._run_json(
                "rates",
                "set",
                "BTC-USD",
                "2024-05-02T00:00:00Z",
                rate,
                "--source",
                source,
            )
            self._assert_ok(payload, result, "rates.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "55000"
        )
        self._assert_ok(payload, result, "rates.set")

        payload, result = self._run_json(
            "rates",
            "range",
            "BTC-USD",
            "--order",
            "desc",
            "--limit",
            "2",
        )
        self._assert_ok(payload, result, "rates.range")
        self.assertEqual(
            [(row["timestamp"], row["source"], row["rate"]) for row in payload["data"]],
            [
                ("2024-05-02T00:00:00Z", "manual", 65000.0),
                ("2024-05-02T00:00:00Z", "coingecko", 60000.0),
            ],
        )

    def test_rates_set_invalidates_processed_journals(self):
        self._bootstrap_wallet(label="RateInvalidation")
        self._insert_transaction(
            wallet_label="RateInvalidation",
            tx_id="needs-rate",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=100_000_000,
        )
        payload, result = self._run_json(
            "journals",
            "process",
            "--workspace",
            "Main",
            "--profile",
            "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "profiles",
            "get",
            "--workspace",
            "Main",
            "--profile",
            "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertTrue(payload["data"]["last_processed_at"])

        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000"
        )
        self._assert_ok(payload, result, "rates.set")

        payload, result = self._run_json(
            "profiles",
            "get",
            "--workspace",
            "Main",
            "--profile",
            "Default",
        )
        self._assert_ok(payload, result, "profiles.get")
        self.assertIsNone(payload["data"]["last_processed_at"])
        self.assertEqual(payload["data"]["last_processed_tx_count"], 0)

    def test_rates_sync_invalidates_matching_fiat_profiles(self):
        self._bootstrap_profile()
        payload, result = self._run_json(
            "profiles",
            "create",
            "--workspace",
            "Main",
            "--fiat-currency",
            "EUR",
            "Euro",
        )
        self._assert_ok(payload, result, "profiles.create")

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = '2024-01-01T00:00:00Z',
                last_processed_tx_count = 7
            """
        )
        conn.commit()
        with patch.object(
            core_rates,
            "fetch_rates_coinbase_exchange",
            return_value=[
                {
                    "timestamp": "2024-01-01T00:00:00Z",
                    "open": "60000.0",
                    "high": "60000.0",
                    "low": "60000.0",
                    "close": "60000.0",
                    "volume": "1.0",
                    "trades": None,
                }
            ],
        ):
            summary = core_rates.sync_rates(conn, pair="BTC-EUR", days=1)
        self.assertEqual(summary[0]["pair"], "BTC-EUR")
        rows = {
            row["label"]: row
            for row in conn.execute(
                "SELECT label, last_processed_at, last_processed_tx_count FROM profiles"
            ).fetchall()
        }
        self.assertEqual(rows["Default"]["last_processed_at"], "2024-01-01T00:00:00Z")
        self.assertEqual(rows["Default"]["last_processed_tx_count"], 7)
        self.assertIsNone(rows["Euro"]["last_processed_at"])
        self.assertEqual(rows["Euro"]["last_processed_tx_count"], 0)

        conn.execute(
            """
            UPDATE profiles
            SET last_processed_at = '2024-01-02T00:00:00Z',
                last_processed_tx_count = 9
            """
        )
        conn.commit()
        row = core_rates.set_manual_rate(conn, "BTC-JPY", "2024-01-02T00:00:00Z", 9_000_000)
        self.assertEqual(row["pair"], "BTC-JPY")
        rows = {
            row["label"]: row
            for row in conn.execute(
                "SELECT label, last_processed_at, last_processed_tx_count FROM profiles"
            ).fetchall()
        }
        conn.close()
        self.assertEqual(rows["Default"]["last_processed_at"], "2024-01-02T00:00:00Z")
        self.assertEqual(rows["Default"]["last_processed_tx_count"], 9)
        self.assertEqual(rows["Euro"]["last_processed_at"], "2024-01-02T00:00:00Z")
        self.assertEqual(rows["Euro"]["last_processed_tx_count"], 9)

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

    def test_import_pricing_provenance_rank_replaces_weaker_import(self):
        self._bootstrap_wallet(label="PriceSource", kind="custom")
        first_csv = self._write_case_file(
            "generic-price.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,description\n"
            "2024-05-01T12:00:00Z,ranked-price-1,inbound,BTC,0.01000000,0,60000,Generic quote\n",
        )
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "PriceSource",
            "--file", str(first_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 1)
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        first_tx = conn.execute(
            """
            SELECT pricing_timestamp
            FROM transactions
            WHERE external_id = 'ranked-price-1'
            """
        ).fetchone()
        conn.close()
        self.assertEqual(first_tx["pricing_timestamp"], "2024-05-01T12:00:00Z")

        exchange_csv = self._write_case_file(
            "exchange-price.csv",
            "date,txid,direction,asset,amount,fee,fiat_rate,pricing_source_kind,pricing_provider,pricing_pair,pricing_timestamp,pricing_method,description\n"
            "2024-05-01T12:00:00Z,ranked-price-1,inbound,BTC,0.01000000,0,61000,exchange_execution,Kraken,BTC-EUR,2024-05-01T12:00:00Z,trade_execution,Exchange fill\n",
        )
        payload, result = self._run_json(
            "wallets", "import-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "PriceSource",
            "--file", str(exchange_csv),
        )
        self._assert_ok(payload, result, "wallets.import-csv")
        self.assertEqual(payload["data"]["imported"], 0)
        self.assertEqual(payload["data"]["skipped"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            """
            SELECT fiat_rate, fiat_value, fiat_rate_exact, fiat_value_exact,
                   fiat_price_source, pricing_source_kind, pricing_provider,
                   pricing_pair, pricing_method, pricing_quality
            FROM transactions
            WHERE external_id = 'ranked-price-1'
            """
        ).fetchone()
        conn.close()
        self.assertAlmostEqual(tx["fiat_rate"], 61000.0, places=4)
        self.assertAlmostEqual(tx["fiat_value"], 610.0, places=4)
        self.assertEqual(tx["fiat_rate_exact"], "61000")
        self.assertEqual(tx["fiat_value_exact"], "610.00000000")
        self.assertEqual(tx["fiat_price_source"], "import")
        self.assertEqual(tx["pricing_source_kind"], "exchange_execution")
        self.assertEqual(tx["pricing_provider"], "Kraken")
        self.assertEqual(tx["pricing_pair"], "BTC-EUR")
        self.assertEqual(tx["pricing_method"], "trade_execution")
        self.assertEqual(tx["pricing_quality"], "exact")

    def test_daily_provider_sample_is_review_quarantined(self):
        self._bootstrap_wallet(label="CoarseCache")
        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--require-coarse-review",
        )
        self._assert_ok(payload, result, "profiles.set")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000",
            "--source", "coingecko",
            "--granularity", "daily",
            "--method", "market_chart",
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CoarseCache",
            tx_id="coarse-price-1",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)
        self.assertEqual(payload["data"]["entries_created"], 0)
        self.assertEqual(payload["data"]["quarantined"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider, pricing_timestamp, pricing_granularity,
                   pricing_method, pricing_quality
            FROM transactions
            WHERE external_id = 'coarse-price-1'
            """
        ).fetchone()
        quarantine = conn.execute(
            """
            SELECT reason, detail_json
            FROM journal_quarantines
            WHERE transaction_id = (SELECT id FROM transactions WHERE external_id = 'coarse-price-1')
            """
        ).fetchone()
        conn.close()
        self.assertEqual(tx["fiat_rate_exact"], "60000")
        self.assertEqual(tx["fiat_value_exact"], "600.00")
        self.assertEqual(tx["pricing_source_kind"], "fmv_provider")
        self.assertEqual(tx["pricing_provider"], "coingecko")
        self.assertEqual(tx["pricing_timestamp"], "2024-05-01T00:00:00Z")
        self.assertEqual(tx["pricing_granularity"], "daily")
        self.assertEqual(tx["pricing_method"], "market_chart")
        self.assertEqual(tx["pricing_quality"], "coarse_fallback")
        self.assertEqual(quarantine["reason"], "pricing_review_required")
        detail = json.loads(quarantine["detail_json"])
        self.assertEqual(detail["pricing_quality"], "coarse_fallback")
        self.assertEqual(detail["pricing_granularity"], "daily")

    def test_daily_provider_sample_accepted_without_coarse_review(self):
        # Default policy: coarse (daily) pricing is accepted and booked at the
        # coarse spot price rather than quarantined for manual review.
        self._bootstrap_wallet(label="CoarseAccept")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000",
            "--source", "coingecko",
            "--granularity", "daily",
            "--method", "market_chart",
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="CoarseAccept",
            tx_id="coarse-accept-1",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 1)
        self.assertEqual(payload["data"]["entries_created"], 1)
        self.assertEqual(payload["data"]["quarantined"], 0)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            "SELECT pricing_quality, fiat_value_exact FROM transactions WHERE external_id = 'coarse-accept-1'"
        ).fetchone()
        quarantine_count = conn.execute(
            """
            SELECT COUNT(*) AS n FROM journal_quarantines
            WHERE transaction_id = (SELECT id FROM transactions WHERE external_id = 'coarse-accept-1')
            """
        ).fetchone()["n"]
        from kassiber.core import rates as _rates

        profile_id = conn.execute("SELECT id FROM profiles LIMIT 1").fetchone()["id"]
        coarse_priced_count = _rates.count_coarse_priced_transactions(conn, profile_id)
        conn.close()
        # The coarse price is still recorded (flagged non-blockingly in the UI),
        # the event is booked, and nothing is quarantined.
        self.assertEqual(tx["pricing_quality"], "coarse_fallback")
        self.assertEqual(tx["fiat_value_exact"], "600.00")
        self.assertEqual(quarantine_count, 0)
        # The non-blocking "priced from daily rates" notice counts this row.
        self.assertEqual(coarse_priced_count, 1)

    def test_legacy_cache_price_gets_provenance_backfill_before_review(self):
        self._bootstrap_wallet(label="LegacyCache")
        payload, result = self._run_json(
            "rates", "set", "BTC-USD", "2024-05-01T00:00:00Z", "60000",
            "--source", "coingecko",
            "--granularity", "daily",
            "--method", "market_chart",
        )
        self._assert_ok(payload, result, "rates.set")
        self._insert_transaction(
            wallet_label="LegacyCache",
            tx_id="legacy-cache-price-1",
            occurred_at="2024-05-01T12:00:00Z",
            amount_msat=1_000_000_000,
        )
        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.execute(
            """
            UPDATE transactions
            SET fiat_rate = 60000.0,
                fiat_value = 600.0,
                fiat_price_source = 'rates_cache'
            WHERE external_id = 'legacy-cache-price-1'
            """
        )
        conn.commit()
        conn.close()

        payload, result = self._run_json(
            "profiles", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--require-coarse-review",
        )
        self._assert_ok(payload, result, "profiles.set")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["auto_priced"], 0)
        self.assertEqual(payload["data"]["entries_created"], 0)
        self.assertEqual(payload["data"]["quarantined"], 1)

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        tx = conn.execute(
            """
            SELECT fiat_rate_exact, fiat_value_exact, pricing_source_kind,
                   pricing_provider, pricing_granularity, pricing_method,
                   pricing_quality
            FROM transactions
            WHERE external_id = 'legacy-cache-price-1'
            """
        ).fetchone()
        quarantine = conn.execute(
            """
            SELECT reason
            FROM journal_quarantines
            WHERE transaction_id = (
                SELECT id FROM transactions WHERE external_id = 'legacy-cache-price-1'
            )
            """
        ).fetchone()
        conn.close()
        self.assertEqual(tx["fiat_rate_exact"], "60000")
        self.assertEqual(tx["fiat_value_exact"], "600.00")
        self.assertEqual(tx["pricing_source_kind"], "fmv_provider")
        self.assertEqual(tx["pricing_provider"], "coingecko")
        self.assertEqual(tx["pricing_granularity"], "daily")
        self.assertEqual(tx["pricing_method"], "market_chart")
        self.assertEqual(tx["pricing_quality"], "coarse_fallback")
        self.assertEqual(quarantine["reason"], "pricing_review_required")

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
        self.assertEqual(_legacy_snapshot_identity(actual), expected)

    def test_direct_generic_bitcoin_rail_carrying_value_pair_carries_basis(self):
        profile, inputs = self._direct_cross_asset_pair_engine_inputs()
        carry_inputs = finalized_tax_inputs(
            profile,
            rows=[
                *inputs.source_rows,
                {
                    "id": "lbtc-sale-after-carry",
                    "wallet_id": "wallet-liquid",
                    "wallet_label": "Liquid",
                    "wallet_account_id": "account-treasury",
                    "account_code": "treasury",
                    "account_label": "Treasury",
                    "occurred_at": "2026-04-20T10:00:00Z",
                    "direction": "outbound",
                    "asset": "LBTC",
                    "amount": 10_000_000_000,
                    "fee": 0,
                    "fiat_rate": 83000,
                    "fiat_value": 8300,
                    "kind": "sell",
                    "description": "Sell carried LBTC",
                    "note": None,
                    "external_id": "lbtc-sale-after-carry",
                    "created_at": "2026-04-20T10:00:00Z",
                },
            ],
            wallet_refs_by_id=inputs.wallet_refs_by_id,
            manual_pair_records=[
                {
                    **dict(inputs.source_manual_pair_records[0]),
                    "policy": "carrying-value",
                },
            ],
        )
        actual = self._direct_engine_snapshot(profile, carry_inputs)
        self.assertEqual(actual["quarantines"], [])
        self.assertEqual(actual["account_holdings"], [])
        disposals = {
            entry["transaction_id"]: entry
            for entry in actual["entries"]
            if entry["entry_type"] == "disposal"
        }
        self.assertEqual(disposals["cross-out-leg"]["gain_loss"], 0.0)
        self.assertEqual(disposals["cross-out-leg"]["cost_basis"], 8008.0)
        self.assertEqual(disposals["cross-out-leg"]["proceeds"], 8008.0)
        self.assertEqual(disposals["lbtc-sale-after-carry"]["cost_basis"], 8008.0)
        self.assertEqual(disposals["lbtc-sale-after-carry"]["proceeds"], 8300.0)
        self.assertEqual(disposals["lbtc-sale-after-carry"]["gain_loss"], 292.0)

    def test_direct_generic_bitcoin_rail_chained_carry_stays_stable(self):
        profile = {
            "id": "profile-generic-chain",
            "workspace_id": "workspace-main",
            "label": "FixtureGenericRailChain",
            "fiat_currency": "USD",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        wallet_refs_by_id = {
            "wallet-btc": {
                "id": "wallet-btc",
                "label": "Bitcoin",
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

        def row(rid, wallet_id, direction, asset, occurred_at, fiat_rate, description):
            wallet = wallet_refs_by_id[wallet_id]
            return {
                "id": rid,
                "wallet_id": wallet_id,
                "wallet_label": wallet["label"],
                "wallet_account_id": wallet["wallet_account_id"],
                "account_code": wallet["account_code"],
                "account_label": wallet["account_label"],
                "occurred_at": occurred_at,
                "direction": direction,
                "asset": asset,
                "amount": 100_000_000_000,
                "fee": 0,
                "fiat_rate": fiat_rate,
                "fiat_value": fiat_rate,
                "kind": "buy" if direction == "inbound" else "sell",
                "description": description,
                "note": None,
                "external_id": rid,
                "created_at": occurred_at,
            }

        inputs = finalized_tax_inputs(
            profile,
            rows=[
                row("btc-buy-10k", "wallet-btc", "inbound", "BTC", "2026-01-01T00:00:00Z", 10000, "BTC buy"),
                row("btc-to-lbtc-out", "wallet-btc", "outbound", "BTC", "2026-02-01T00:00:00Z", 20000, "BTC rail out"),
                row("btc-to-lbtc-in", "wallet-liquid", "inbound", "LBTC", "2026-02-01T00:01:00Z", 20000, "LBTC rail in"),
                row("lbtc-to-btc-out", "wallet-liquid", "outbound", "LBTC", "2026-03-01T00:00:00Z", 30000, "LBTC rail out"),
                row("lbtc-to-btc-in", "wallet-btc", "inbound", "BTC", "2026-03-01T00:01:00Z", 30000, "BTC rail in"),
                row("btc-sale-40k", "wallet-btc", "outbound", "BTC", "2026-04-01T00:00:00Z", 40000, "BTC sale"),
            ],
            wallet_refs_by_id=wallet_refs_by_id,
            manual_pair_records=[
                {
                    "id": "pair-btc-lbtc",
                    "out_transaction_id": "btc-to-lbtc-out",
                    "in_transaction_id": "btc-to-lbtc-in",
                    "kind": "peg-in",
                    "policy": "carrying-value",
                },
                {
                    "id": "pair-lbtc-btc",
                    "out_transaction_id": "lbtc-to-btc-out",
                    "in_transaction_id": "lbtc-to-btc-in",
                    "kind": "peg-out",
                    "policy": "carrying-value",
                },
            ],
        )
        actual = self._direct_engine_snapshot(profile, inputs)
        self.assertEqual(actual["quarantines"], [])
        self.assertEqual(actual["account_holdings"], [])
        disposals = {
            entry["transaction_id"]: entry
            for entry in actual["entries"]
            if entry["entry_type"] == "disposal"
        }
        self.assertEqual(disposals["btc-to-lbtc-out"]["cost_basis"], 10000.0)
        self.assertEqual(disposals["btc-to-lbtc-out"]["proceeds"], 10000.0)
        self.assertEqual(disposals["btc-to-lbtc-out"]["gain_loss"], 0.0)
        self.assertEqual(disposals["lbtc-to-btc-out"]["cost_basis"], 10000.0)
        self.assertEqual(disposals["lbtc-to-btc-out"]["proceeds"], 10000.0)
        self.assertEqual(disposals["lbtc-to-btc-out"]["gain_loss"], 0.0)
        self.assertEqual(disposals["btc-sale-40k"]["cost_basis"], 10000.0)
        self.assertEqual(disposals["btc-sale-40k"]["proceeds"], 40000.0)
        self.assertEqual(disposals["btc-sale-40k"]["gain_loss"], 30000.0)

    def test_direct_generic_bitcoin_rail_carry_quarantines_when_source_basis_missing(self):
        profile, inputs = self._direct_cross_asset_pair_engine_inputs()
        carry_inputs = finalized_tax_inputs(
            profile,
            rows=[
                row for row in inputs.source_rows if row["id"] != "cross-fund-1"
            ],
            wallet_refs_by_id=inputs.wallet_refs_by_id,
            manual_pair_records=[
                {
                    **dict(inputs.source_manual_pair_records[0]),
                    "policy": "carrying-value",
                },
            ],
        )
        actual = self._direct_engine_snapshot(profile, carry_inputs)
        self.assertEqual(actual["entries"], [])
        self.assertEqual(actual["account_holdings"], [])
        self.assertEqual(actual["wallet_holdings"], [])
        quarantines = {row["transaction_id"]: row for row in actual["quarantines"]}
        self.assertEqual(set(quarantines), {"cross-out-leg", "cross-in-leg"})
        for quarantine in quarantines.values():
            self.assertEqual(quarantine["reason"], "bitcoin_rail_carry_basis_unresolved")
            self.assertEqual(
                quarantine["detail"]["reason_code"], "source_basis_unavailable"
            )
            self.assertEqual(quarantine["detail"]["rail_pair"], "pair-cross-1")

    def test_direct_generic_bitcoin_rail_carry_clears_coarse_pricing_review(self):
        profile, inputs = self._direct_cross_asset_pair_engine_inputs()
        profile = {**dict(profile), "require_coarse_review": 1}
        rows = []
        for row in inputs.source_rows:
            if row["id"] == "cross-in-leg":
                rows.append(
                    {
                        **dict(row),
                        "pricing_source_kind": pricing.SOURCE_FMV_PROVIDER,
                        "pricing_quality": pricing.QUALITY_COARSE_FALLBACK,
                        "pricing_granularity": "daily",
                    }
                )
            else:
                rows.append(row)
        carry_inputs = finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=inputs.wallet_refs_by_id,
            manual_pair_records=[
                {
                    **dict(inputs.source_manual_pair_records[0]),
                    "policy": "carrying-value",
                },
            ],
        )
        actual = self._direct_engine_snapshot(profile, carry_inputs)
        self.assertEqual(actual["quarantines"], [])
        acquisition = next(
            entry
            for entry in actual["entries"]
            if entry["entry_type"] == "acquisition"
            and entry["transaction_id"] == "cross-in-leg"
        )
        self.assertEqual(acquisition["fiat_value"], 8008.0)

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
        payload, result = self._run_json(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--tx-out", _FIXTURE_SELF_TRANSFER_TXID,
            "--tx-in", _FIXTURE_SELF_TRANSFER_TXID,
            "--kind", "manual",
            "--policy", "carrying-value",
        )
        self._assert_ok(payload, result, "transfers.pair")

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
        self.assertEqual(_legacy_snapshot_identity(actual), expected)
        self.assertEqual(
            actual["intra_audit"][0]["pairing_source"],
            "reviewed_custody_component",
        )
        self.assertTrue(
            actual["intra_audit"][0]["transfer_group_id"].startswith(
                "component:"
            )
        )

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
        inputs = finalized_tax_inputs(
            profile,
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
        self.assertEqual(income_entry["at_kennzahl"], 172)
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

    def test_austrian_rp2_engine_emits_income_entry_for_mining_receipt_and_later_disposal(self):
        profile, inputs = self._direct_austrian_single_wallet_inputs(
            [
                {
                    "id": "mining-receipt-1",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "direction": "inbound",
                    "amount": 100_000_000,
                    "fiat_value": 40,
                    "kind": "mining",
                    "description": "Mining reward",
                },
                {
                    "id": "mining-disposal-1",
                    "occurred_at": "2024-06-01T00:00:00Z",
                    "direction": "outbound",
                    "amount": 50_000_000,
                    "fiat_value": 25,
                    "kind": "sell",
                    "description": "Sell mined sats",
                },
            ]
        )
        actual = self._direct_engine_snapshot(profile, inputs)
        self.assertEqual(actual["quarantines"], [])
        income_entry = next(entry for entry in actual["entries"] if entry["entry_type"] == "income")
        self.assertEqual(income_entry["at_category"], "income_general")
        self.assertEqual(income_entry["at_kennzahl"], 172)
        disposal_entry = next(entry for entry in actual["entries"] if entry["entry_type"] == "disposal")
        self.assertEqual(disposal_entry["at_category"], "neu_gain")
        self.assertEqual(disposal_entry["at_kennzahl"], 174)

    def test_austrian_rp2_engine_emits_income_entry_for_routing_income(self):
        profile, inputs = self._direct_austrian_single_wallet_inputs(
            [
                {
                    "id": "routing-income-1",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "direction": "inbound",
                    "amount": 10_000,
                    "fiat_value": 0.004,
                    "kind": "routing_income",
                    "description": "Lightning routing fee",
                },
            ]
        )
        actual = self._direct_engine_snapshot(profile, inputs)
        self.assertEqual(actual["quarantines"], [])
        income_entry = next(entry for entry in actual["entries"] if entry["entry_type"] == "income")
        self.assertEqual(income_entry["at_category"], "income_general")
        self.assertEqual(income_entry["at_kennzahl"], 172)

    def test_austrian_rp2_engine_preserves_neu_gain_and_loss_rows_for_same_year_offset(self):
        profile, inputs = self._direct_austrian_single_wallet_inputs(
            [
                {
                    "id": "buy-low-1",
                    "occurred_at": "2024-01-01T00:00:00Z",
                    "direction": "inbound",
                    "amount": 100_000_000,
                    "fiat_value": 20,
                    "kind": "buy",
                },
                {
                    "id": "sell-gain-1",
                    "occurred_at": "2024-02-01T00:00:00Z",
                    "direction": "outbound",
                    "amount": 50_000_000,
                    "fiat_value": 30,
                    "kind": "sell",
                },
                {
                    "id": "buy-high-1",
                    "occurred_at": "2024-03-01T00:00:00Z",
                    "direction": "inbound",
                    "amount": 100_000_000,
                    "fiat_value": 100,
                    "kind": "buy",
                },
                {
                    "id": "sell-loss-1",
                    "occurred_at": "2024-04-01T00:00:00Z",
                    "direction": "outbound",
                    "amount": 50_000_000,
                    "fiat_value": 20,
                    "kind": "sell",
                },
            ]
        )
        actual = self._direct_engine_snapshot(profile, inputs)
        self.assertEqual(actual["quarantines"], [])
        disposal_entries = [entry for entry in actual["entries"] if entry["entry_type"] == "disposal"]
        self.assertEqual([entry["at_category"] for entry in disposal_entries], ["neu_gain", "neu_loss"])
        self.assertEqual([entry["at_kennzahl"] for entry in disposal_entries], [174, 176])
        self.assertGreater(sum(entry["gain_loss"] for entry in disposal_entries), 0)

    def test_austrian_rp2_cross_asset_swap_carries_basis(self):
        """End-to-end: AT profile carries basis across matched Neu cross-asset swaps."""
        profile, inputs = self._direct_austrian_cross_asset_swap_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)
        expected = self._load_fixture("austrian_rp2_cross_asset_swap_snapshot.json")
        self.assertEqual(_legacy_snapshot_identity(actual), expected)

    def test_austrian_rp2_sale_from_transfer_funded_wallet_uses_global_pool(self):
        """End-to-end (bitcoinaustria/kassiber#213): a Neu sale from a wallet funded by an internal
        transfer resolves against the single global per-asset pool instead of aborting the report.

        With the old per-wallet pools the disposal was tagged at_pool=<disposing wallet> while the
        lot kept at_pool=<acquiring wallet>, so rp2's moving_average_at found no lots in the
        disposal's pool and raised "Total in-transaction crypto value < total taxable crypto value".
        """
        profile, inputs = self._direct_austrian_transfer_then_sell_inputs()
        state = build_tax_engine(profile).build_ledger_state(inputs)
        entries = _normalize_engine_entries(state.entries)

        # No quarantine / no crash: the sale from wallet-b computes against the global Neu pool.
        self.assertEqual(state.quarantines, [])
        disposals = [e for e in entries if e["entry_type"] == "disposal"]
        self.assertEqual(len(disposals), 1)
        sale = disposals[0]
        self.assertEqual(sale["transaction_id"], "neu-sell")
        self.assertEqual(sale["wallet_id"], "wallet-b")
        self.assertEqual(sale["at_category"], "neu_gain")
        self.assertIn("at_pool=default", sale["description"])
        # Moving-average basis from the global pool: 0.3 BTC * 30000 avg = 9000; proceeds 15000.
        self.assertEqual(sale["cost_basis"], 9000.0)
        self.assertEqual(sale["gain_loss"], 6000.0)
        # The A->B move is modeled as an intra transfer, not a phantom disposal.
        self.assertEqual(len(state.intra_audit), 1)
        self.assertEqual(state.intra_audit[0]["from_wallet_id"], "wallet-a")
        self.assertEqual(state.intra_audit[0]["to_wallet_id"], "wallet-b")

    def test_austrian_alt_transfer_journal_rows_preserve_wallet_tax_free_regime(self):
        profile = {
            "id": "profile-at-transfer-alt",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianAltTransfer",
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
                "label": "Cold",
                "wallet_account_id": "account-cold",
                "account_code": "cold",
                "account_label": "Cold",
            },
        }

        def _row(rid, wid, direction, amount, occurred_at, *, description, external_id):
            ref = wallet_refs_by_id[wid]
            return {
                "id": rid,
                "wallet_id": wid,
                "wallet_label": ref["label"],
                "wallet_account_id": ref["wallet_account_id"],
                "account_code": ref["account_code"],
                "account_label": ref["account_label"],
                "occurred_at": occurred_at,
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": 0,
                "fiat_rate": 60000,
                "fiat_value": 60000,
                "kind": "transfer",
                "note": None,
                "description": description,
                "external_id": external_id,
                "raw_json": _typed_onchain_raw(external_id),
                "created_at": occurred_at,
            }

        state = build_tax_engine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=[
                    _row(
                        "alt-buy",
                        "wallet-a",
                        "inbound",
                        100_000_000_000,
                        "2020-06-01T10:00:00Z",
                        description="Alt buy",
                        external_id="alt-buy",
                    ),
                    _row(
                        "xfer-out",
                        "wallet-a",
                        "outbound",
                        100_000_000_000,
                        "2024-07-01T10:00:00Z",
                        description="Move A->B",
                        external_id=_AT_ALT_TRANSFER_TXID,
                    ),
                    _row(
                        "xfer-in",
                        "wallet-b",
                        "inbound",
                        100_000_000_000,
                        "2024-07-01T10:00:00Z",
                        description="Move A->B",
                        external_id=_AT_ALT_TRANSFER_TXID,
                    ),
                ],
                wallet_refs_by_id=wallet_refs_by_id,
                manual_pair_records=[
                    {
                        "id": "reviewed-alt-transfer",
                        "out_transaction_id": "xfer-out",
                        "in_transaction_id": "xfer-in",
                        "kind": "manual",
                        "policy": "carrying-value",
                    }
                ],
            )
        )

        self.assertEqual(state.quarantines, [])
        transfer_entries = [
            entry
            for entry in _normalize_engine_entries(state.entries)
            if entry["entry_type"] in {"transfer_in", "transfer_out"}
        ]
        self.assertEqual(len(transfer_entries), 2)
        self.assertTrue(
            all("at_regime=alt" in entry["description"] for entry in transfer_entries),
        )
        self.assertEqual(
            _tax_free_wallet_summaries(state.entries),
            [
                {"walletId": "wallet-a", "hasTaxFreeBalance": False},
                {"walletId": "wallet-b", "hasTaxFreeBalance": True},
            ],
        )

    def test_mixed_regime_transfer_carries_tax_free_share_to_destination(self):
        # A mixed Alt+Neu wallet moving MORE than its Neu stack carries Alt
        # coins to the destination even though the MOVE's fee-slice marker says
        # at_regime=neu. The tax-free hint must classify the moved QUANTITIES
        # (at_alt_out/at_alt_in flows), not the whole MOVE by the fee regime.
        profile = {
            "id": "profile-at-mixed-move",
            "workspace_id": "workspace-main",
            "label": "FixtureAustrianMixedMove",
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
                "label": "Cold",
                "wallet_account_id": "account-cold",
                "account_code": "cold",
                "account_label": "Cold",
            },
        }

        def _row(rid, wid, direction, amount, occurred_at, *, description, external_id):
            ref = wallet_refs_by_id[wid]
            return {
                "id": rid,
                "wallet_id": wid,
                "wallet_label": ref["label"],
                "wallet_account_id": ref["wallet_account_id"],
                "account_code": ref["account_code"],
                "account_label": ref["account_label"],
                "occurred_at": occurred_at,
                "direction": direction,
                "asset": "BTC",
                "amount": amount,
                "fee": 0,
                "fiat_rate": 60000,
                "fiat_value": 60000,
                "kind": "transfer",
                "note": None,
                "description": description,
                "external_id": external_id,
                "raw_json": _typed_onchain_raw(external_id),
                "created_at": occurred_at,
            }

        state = build_tax_engine(profile).build_ledger_state(
            finalized_tax_inputs(
                profile,
                rows=[
                    _row(
                        "alt-buy",
                        "wallet-a",
                        "inbound",
                        30_000_000_000,
                        "2020-06-01T10:00:00Z",
                        description="Alt buy",
                        external_id="alt-buy",
                    ),
                    _row(
                        "neu-buy",
                        "wallet-a",
                        "inbound",
                        40_000_000_000,
                        "2024-01-01T10:00:00Z",
                        description="Neu buy",
                        external_id="neu-buy",
                    ),
                    _row(
                        "xfer-out",
                        "wallet-a",
                        "outbound",
                        50_000_000_000,
                        "2024-07-01T10:00:00Z",
                        description="Move A->B",
                        external_id=_AT_MIXED_TRANSFER_TXID,
                    ),
                    _row(
                        "xfer-in",
                        "wallet-b",
                        "inbound",
                        50_000_000_000,
                        "2024-07-01T10:00:00Z",
                        description="Move A->B",
                        external_id=_AT_MIXED_TRANSFER_TXID,
                    ),
                ],
                wallet_refs_by_id=wallet_refs_by_id,
                manual_pair_records=[
                    {
                        "id": "reviewed-mixed-transfer",
                        "out_transaction_id": "xfer-out",
                        "in_transaction_id": "xfer-in",
                        "kind": "manual",
                        "policy": "carrying-value",
                    }
                ],
            )
        )

        self.assertEqual(state.quarantines, [])
        # 0.5 moved: 0.4 Neu (preferred) + 0.1 Alt carried to the destination.
        self.assertEqual(
            _tax_free_wallet_summaries(state.entries),
            [
                {"walletId": "wallet-a", "hasTaxFreeBalance": True},
                {"walletId": "wallet-b", "hasTaxFreeBalance": True},
            ],
        )

    def test_austrian_direct_swap_payout_carries_then_disposes(self):
        profile, inputs = self._direct_austrian_swap_payout_inputs()
        state = build_tax_engine(profile).build_ledger_state(inputs)
        entries = _normalize_engine_entries(state.entries)

        self.assertEqual(state.quarantines, [])
        self.assertEqual(len(state.cross_asset_pairs), 1)
        payout_pair = state.cross_asset_pairs[0]
        self.assertEqual(
            {
                "pair_id": payout_pair["pair_id"],
                "kind": payout_pair["kind"],
                "policy": payout_pair["policy"],
                "out_transaction_id": payout_pair["out_transaction_id"],
                "out_asset": payout_pair["out_asset"],
                "in_asset": payout_pair["in_asset"],
            },
            {
                "pair_id": "direct-payout:direct-payout-1",
                "kind": "direct-swap-payout",
                "policy": "carrying-value",
                "out_transaction_id": "swap-payout-source",
                "out_asset": "LBTC",
                "in_asset": "BTC",
            },
        )
        self.assertEqual(
            state.direct_swap_payouts,
            [
                {
                    "payout_id": "direct-payout-1",
                    "kind": "direct-swap-payout",
                    "policy": "carrying-value",
                    "out_id": "swap-payout-source",
                    "out_asset": "LBTC",
                    "out_amount_msat": 50_000_000_000,
                    "payout_asset": "BTC",
                    "payout_amount_msat": 49_990_000_000,
                    "payout_occurred_at": "2025-03-01T09:00:30Z",
                    "payout_external_id": "recipient-txid",
                    "counterparty": "external-recipient",
                    "swap_fee_msat": 10_000_000,
                    "swap_fee_kind": "combined",
                }
            ],
        )
        disposals = [
            entry
            for entry in entries
            if entry["transaction_id"] == "swap-payout-source"
            and entry["entry_type"] == "disposal"
        ]
        self.assertEqual(len(disposals), 2)
        neutral_source = next(entry for entry in disposals if entry["asset"] == "LBTC")
        taxable_payout = next(entry for entry in disposals if entry["asset"] == "BTC")

        self.assertEqual(neutral_source["at_category"], "neu_swap")
        self.assertEqual(neutral_source["gain_loss"], 0.0)
        self.assertEqual(taxable_payout["at_category"], "neu_gain")
        self.assertAlmostEqual(taxable_payout["cost_basis"], 15000.0)
        self.assertAlmostEqual(taxable_payout["proceeds"], 24995.0)
        self.assertAlmostEqual(taxable_payout["gain_loss"], 9995.0)
        self.assertEqual(_normalize_holdings(state.account_holdings, ("account_id", "account_code", "account_label", "asset")), [
            {
                "account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "asset": "LBTC",
                "quantity": 0.5,
                "cost_basis": 15000.0,
            }
        ])

    def test_generic_bitcoin_rail_direct_swap_payout_carries_then_disposes(self):
        profile, inputs = self._direct_austrian_swap_payout_inputs()
        profile = {
            **profile,
            "id": "profile-generic-rail-payout",
            "label": "FixtureGenericRailPayout",
            "tax_country": "generic",
            "tax_long_term_days": 365,
            "gains_algorithm": "FIFO",
        }
        state = build_tax_engine(profile).build_ledger_state(inputs)
        entries = _normalize_engine_entries(state.entries)

        self.assertEqual(state.quarantines, [])
        self.assertEqual(state.direct_swap_payouts[0]["policy"], "carrying-value")
        disposals = [
            entry
            for entry in entries
            if entry["transaction_id"] == "swap-payout-source"
            and entry["entry_type"] == "disposal"
        ]
        self.assertEqual(len(disposals), 2)
        neutral_source = next(entry for entry in disposals if entry["asset"] == "LBTC")
        taxable_payout = next(entry for entry in disposals if entry["asset"] == "BTC")
        self.assertAlmostEqual(neutral_source["cost_basis"], 15000.0)
        self.assertAlmostEqual(neutral_source["proceeds"], 15000.0)
        self.assertAlmostEqual(neutral_source["gain_loss"], 0.0)
        self.assertAlmostEqual(taxable_payout["cost_basis"], 15000.0)
        self.assertAlmostEqual(taxable_payout["proceeds"], 24995.0)
        self.assertAlmostEqual(taxable_payout["gain_loss"], 9995.0)

    def test_austrian_direct_swap_payout_sorts_synthetic_rows(self):
        profile, inputs = self._direct_austrian_swap_payout_inputs()
        rows = [
            *inputs.source_rows,
            {
                "id": "btc-buy-after-payout",
                "wallet_id": "wallet-liquid",
                "wallet_label": "Liquid",
                "wallet_account_id": "account-treasury",
                "account_code": "treasury",
                "account_label": "Treasury",
                "occurred_at": "2025-03-02T09:00:00Z",
                "direction": "inbound",
                "asset": "BTC",
                "amount": 10_000_000_000,
                "fee": 0,
                "fiat_rate": 51000,
                "fiat_value": 5100,
                "kind": "buy",
                "description": "BTC buy after direct payout",
                "note": None,
                "external_id": "btc-buy-after-payout",
                "created_at": "2025-03-02T09:00:00Z",
            },
        ]
        sorted_inputs = finalized_tax_inputs(
            profile,
            rows=rows,
            wallet_refs_by_id=inputs.wallet_refs_by_id,
            manual_pair_records=inputs.source_manual_pair_records,
            direct_payout_records=inputs.direct_payout_records,
        )
        original_normalize = rp2_engine.normalize_tax_asset_inputs
        btc_orders = []

        def spy_normalize(profile_arg, asset, asset_rows, wallet_refs_by_id, pairs, **kwargs):
            if asset == "BTC":
                btc_orders.append(
                    [
                        (
                            str(row.get("journal_transaction_id") or row["id"])
                            if str(row["id"]).startswith("custody-tax:")
                            else str(row["id"])
                        )
                        for row in asset_rows
                    ]
                )
            return original_normalize(
                profile_arg,
                asset,
                asset_rows,
                wallet_refs_by_id,
                pairs,
                **kwargs,
            )

        with patch.object(rp2_engine, "normalize_tax_asset_inputs", side_effect=spy_normalize):
            build_tax_engine(profile).build_ledger_state(sorted_inputs)

        self.assertTrue(btc_orders)
        positions = {
            row_id: (batch_index, row_index)
            for batch_index, batch in enumerate(btc_orders)
            for row_index, row_id in enumerate(batch)
        }
        self.assertLess(
            positions["direct-payout:direct-payout-1:in"],
            positions["btc-buy-after-payout"],
        )
        self.assertLess(
            positions["direct-payout:direct-payout-1:out"],
            positions["btc-buy-after-payout"],
        )

    def test_generic_direct_swap_payout_uses_reviewed_sale_proceeds(self):
        profile, inputs = self._direct_generic_swap_payout_inputs()
        state = build_tax_engine(profile).build_ledger_state(inputs)
        entries = _normalize_engine_entries(state.entries)

        self.assertEqual(state.quarantines, [])
        self.assertEqual(state.cross_asset_pairs, [])
        self.assertEqual(state.direct_swap_payouts[0]["payout_id"], "generic-direct-payout-1")
        disposals = [
            entry
            for entry in entries
            if entry["transaction_id"] == "btc-direct-payout-source"
            and entry["entry_type"] == "disposal"
        ]
        self.assertEqual(len(disposals), 1)
        disposal = disposals[0]
        self.assertEqual(disposal["asset"], "BTC")
        self.assertAlmostEqual(disposal["cost_basis"], 5000.0)
        self.assertAlmostEqual(disposal["proceeds"], 25000.0)
        self.assertAlmostEqual(disposal["gain_loss"], 20000.0)

    def test_manual_pair_rejects_transaction_with_active_direct_payout(self):
        self._bootstrap_wallet(label="PayoutSource")
        payload, result = self._run_json(
            "wallets",
            "create",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--label",
            "PayoutTarget",
            "--kind",
            "custom",
        )
        self._assert_ok(payload, result, "wallets.create")
        self._insert_transaction(
            wallet_label="PayoutSource",
            tx_id="payout-source-out",
            occurred_at="2025-03-01T09:00:00Z",
            amount_msat=50_000_000_000,
            direction="outbound",
        )
        self._insert_transaction(
            wallet_label="PayoutTarget",
            tx_id="pair-target-in",
            occurred_at="2025-03-01T09:01:00Z",
            amount_msat=49_990_000_000,
            direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        create_direct_swap_payout(
            conn,
            "Main",
            "Default",
            "payout-source-out",
            payout_asset="BTC",
            payout_amount="0.4999",
            payout_fiat_value="25000",
            payout_external_id="exchange-deposit-txid",
            counterparty="external-exchange",
            policy="taxable",
        )

        with self.assertRaises(AppError) as ctx:
            create_transaction_pair(
                conn,
                "Main",
                "Default",
                "payout-source-out",
                "pair-target-in",
            )
        self.assertEqual(ctx.exception.code, "conflict")
        self.assertIn("direct swap payout", str(ctx.exception))

    def _create_second_wallet(self, label, kind="custom"):
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", label,
            "--kind", kind,
        )
        self._assert_ok(payload, result, "wallets.create")

    def test_paired_list_exposes_wallet_kind_and_occurred_at(self):
        self._bootstrap_wallet(label="HotLN", kind="phoenix")
        self._create_second_wallet("ColdBTC", kind="custom")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="ln-out",
            occurred_at="2025-04-01T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="ColdBTC", tx_id="btc-in",
            occurred_at="2025-04-01T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        create_transaction_pair(
            conn, "Main", "Default", "ln-out", "btc-in",
            kind="coinjoin", policy="carrying-value",
        )
        pairs = list_transaction_pairs(conn, "Main", "Default")
        self.assertEqual(len(pairs), 1)
        entry = pairs[0]
        self.assertEqual(entry["kind"], "coinjoin")
        # The paired view renders rail badges off wallet_kind and shows the
        # leg occurred-at timestamps — both must survive the list query.
        self.assertEqual(entry["out"]["wallet_kind"], "phoenix")
        self.assertEqual(entry["in"]["wallet_kind"], "custom")
        self.assertEqual(entry["out"]["occurred_at"], "2025-04-01T10:00:00Z")
        self.assertEqual(entry["in"]["occurred_at"], "2025-04-01T10:05:00Z")

    def test_update_transaction_pair_changes_kind_and_policy(self):
        self._bootstrap_wallet(label="HotLN", kind="phoenix")
        self._create_second_wallet("LiquidVault", kind="custom")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="ln-out", asset="BTC",
            occurred_at="2025-04-02T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-in", asset="LBTC",
            occurred_at="2025-04-02T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        # Cross-asset taxable is valid on a generic profile.
        pair = create_transaction_pair(
            conn, "Main", "Default", "ln-out", "lbtc-in",
            kind="manual", policy="taxable",
        )
        updated = update_transaction_pair(
            conn, "Main", "Default", pair["id"], kind="submarine-swap",
        )
        self.assertEqual(updated["kind"], "submarine-swap")
        # Policy was not passed, so it stays untouched.
        self.assertEqual(updated["policy"], "taxable")
        pairs = list_transaction_pairs(conn, "Main", "Default")
        self.assertEqual(pairs[0]["kind"], "submarine-swap")

    def test_update_transaction_pair_validates_inputs(self):
        self._bootstrap_wallet(label="HotBTC", kind="phoenix")
        self._create_second_wallet("ColdBTC", kind="custom")
        self._insert_transaction(
            wallet_label="HotBTC", tx_id="btc-out",
            occurred_at="2025-04-03T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="ColdBTC", tx_id="btc-in",
            occurred_at="2025-04-03T10:05:00Z",
            amount_msat=99_900_000, direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        pair = create_transaction_pair(
            conn, "Main", "Default", "btc-out", "btc-in",
            kind="manual", policy="carrying-value",
        )
        with self.assertRaises(AppError) as bad_kind:
            update_transaction_pair(conn, "Main", "Default", pair["id"], kind="bogus")
        self.assertEqual(bad_kind.exception.code, "validation")
        # Same-asset taxable is rejected just like at creation time.
        with self.assertRaises(AppError) as bad_policy:
            update_transaction_pair(
                conn, "Main", "Default", pair["id"], policy="taxable",
            )
        self.assertEqual(bad_policy.exception.code, "validation")
        with self.assertRaises(AppError) as missing:
            update_transaction_pair(
                conn, "Main", "Default", "no-such-pair", kind="manual",
            )
        self.assertEqual(missing.exception.code, "not_found")
        # The rejected edits left the stored values untouched.
        pairs = list_transaction_pairs(conn, "Main", "Default")
        self.assertEqual(pairs[0]["kind"], "manual")
        self.assertEqual(pairs[0]["policy"], "carrying-value")

    def test_update_cross_asset_carrying_value_allows_generic_bitcoin_rails_only(self):
        self._bootstrap_wallet(label="HotLN", kind="phoenix")
        self._create_second_wallet("LiquidVault", kind="custom")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="ln-out", asset="BTC",
            occurred_at="2025-04-04T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-in", asset="LBTC",
            occurred_at="2025-04-04T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        pair = create_transaction_pair(
            conn, "Main", "Default", "ln-out", "lbtc-in",
            kind="manual", policy="taxable",
        )
        updated = update_transaction_pair(
            conn, "Main", "Default", pair["id"], policy="carrying-value",
        )
        self.assertEqual(updated["policy"], "carrying-value")
        self._create_second_wallet("AltRail", kind="custom")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="btc-out",
            asset="BTC", occurred_at="2025-04-05T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="AltRail", tx_id="xyz-in",
            asset="XYZ", occurred_at="2025-04-05T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        generic_pair = create_transaction_pair(
            conn, "Main", "Default", "btc-out", "xyz-in",
            kind="manual", policy="taxable",
        )
        with self.assertRaises(AppError) as ctx:
            update_transaction_pair(
                conn, "Main", "Default", generic_pair["id"], policy="carrying-value",
            )
        self.assertEqual(ctx.exception.code, "validation")

    def test_bitcoin_rail_carrying_value_setting_controls_generic_defaults(self):
        self._bootstrap_wallet(label="HotLN", kind="phoenix")
        self._create_second_wallet("LiquidVault", kind="custom")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="ln-out-default-on", asset="BTC",
            occurred_at="2025-04-06T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-in-default-on", asset="LBTC",
            occurred_at="2025-04-06T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        default_pair = create_transaction_pair(
            conn, "Main", "Default", "ln-out-default-on", "lbtc-in-default-on",
        )
        self.assertEqual(default_pair["policy"], "carrying-value")
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-payout-default-on", asset="LBTC",
            occurred_at="2025-04-06T11:00:00Z",
            amount_msat=50_000_000, direction="outbound",
        )
        default_payout = create_direct_swap_payout(
            conn,
            "Main",
            "Default",
            "lbtc-payout-default-on",
            payout_asset="BTC",
            payout_amount="0.00049",
            payout_fiat_value="25",
            payout_external_id="btc-recipient-default-on",
            counterparty="external-recipient",
        )
        self.assertEqual(default_payout["policy"], "carrying-value")

        payload, result = self._run_json(
            "profiles", "set",
            "--profile", "Default",
            "--no-bitcoin-rail-carrying-value",
        )
        self._assert_ok(payload, result, "profiles.set")
        self.assertFalse(payload["data"]["bitcoin_rail_carrying_value"])
        self._insert_transaction(
            wallet_label="HotLN", tx_id="ln-out-default-off", asset="BTC",
            occurred_at="2025-04-07T10:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-in-default-off", asset="LBTC",
            occurred_at="2025-04-07T10:05:00Z",
            amount_msat=99_500_000, direction="inbound",
        )
        candidates = suggest_transfer_candidates(conn, "Main", "Default")
        candidate = next(
            c for c in candidates["candidates"]
            if c["out_id"] == "ln-out-default-off"
        )
        self.assertEqual(candidate["default_policy"], "taxable")
        taxable_default_pair = create_transaction_pair(
            conn, "Main", "Default", "ln-out-default-off", "lbtc-in-default-off",
        )
        self.assertEqual(taxable_default_pair["policy"], "taxable")
        self._insert_transaction(
            wallet_label="LiquidVault", tx_id="lbtc-payout-default-off", asset="LBTC",
            occurred_at="2025-04-07T11:00:00Z",
            amount_msat=50_000_000, direction="outbound",
        )
        taxable_default_payout = create_direct_swap_payout(
            conn,
            "Main",
            "Default",
            "lbtc-payout-default-off",
            payout_asset="BTC",
            payout_amount="0.00049",
            payout_fiat_value="25",
            payout_external_id="btc-recipient-default-off",
            counterparty="external-recipient",
        )
        self.assertEqual(taxable_default_payout["policy"], "taxable")
        self._insert_transaction(
            wallet_label="HotLN", tx_id="btc-payout-explicit-carry", asset="BTC",
            occurred_at="2025-04-07T12:00:00Z",
            amount_msat=100_000_000, direction="outbound",
        )
        explicit_carry_payout = create_direct_swap_payout(
            conn,
            "Main",
            "Default",
            "btc-payout-explicit-carry",
            payout_asset="LBTC",
            payout_amount="0.00099",
            payout_fiat_value="50",
            payout_external_id="lbtc-recipient-explicit",
            counterparty="external-recipient",
            policy="carrying-value",
        )
        self.assertEqual(explicit_carry_payout["policy"], "carrying-value")

    def test_austrian_same_timestamp_swap_chain_reaches_rp2(self):
        profile, inputs = self._direct_austrian_same_timestamp_swap_chain_inputs()
        actual = self._direct_engine_snapshot(profile, inputs)

        self.assertEqual(actual["quarantines"], [])
        self.assertEqual(
            sorted(pair["pair_id"] for pair in actual["cross_asset_pairs"]),
            ["pair-btc-lbtc", "pair-lbtc-xyz"],
        )
        acquisitions_by_id = {
            entry["transaction_id"]: entry
            for entry in actual["entries"]
            if entry["entry_type"] == "acquisition"
        }
        self.assertEqual(acquisitions_by_id["z-lbtc-from-btc-in"]["fiat_value"], 15000.0)
        self.assertEqual(acquisitions_by_id["xyz-from-lbtc-in"]["fiat_value"], 15000.0)
        disposal_categories = {
            entry["transaction_id"]: entry["at_category"]
            for entry in actual["entries"]
            if entry["entry_type"] == "disposal"
        }
        self.assertEqual(disposal_categories["btc-to-lbtc-out"], "neu_swap")
        self.assertEqual(disposal_categories["a-lbtc-to-xyz-out"], "neu_swap")

    def test_austrian_cross_asset_swap_uses_rp2_multi_asset_hook(self):
        profile, inputs = self._direct_austrian_cross_asset_swap_inputs()
        from rp2.plugin.country.at import AT

        calls: list[set[str]] = []
        original = AT.compute_tax_for_assets

        def spy(self, configuration, accounting_engine, asset_to_input_data):
            calls.append(set(asset_to_input_data))
            return original(self, configuration, accounting_engine, asset_to_input_data)

        AT.compute_tax_for_assets = spy  # type: ignore[assignment]
        try:
            self._direct_engine_snapshot(profile, inputs)
        finally:
            AT.compute_tax_for_assets = original  # type: ignore[assignment]

        self.assertEqual(calls, [{"BTC", "LBTC"}])

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
        inputs = finalized_tax_inputs(
            profile,
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
                    "external_id": _FIXTURE_SELF_TRANSFER_TXID,
                    "raw_json": _typed_onchain_raw(_FIXTURE_SELF_TRANSFER_TXID),
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
                    "external_id": _FIXTURE_SELF_TRANSFER_TXID,
                    "raw_json": _typed_onchain_raw(_FIXTURE_SELF_TRANSFER_TXID),
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
            manual_pair_records=[
                {
                    "id": "reviewed-unfunded-transfer",
                    "out_transaction_id": "transfer-out",
                    "in_transaction_id": "transfer-in",
                    "kind": "manual",
                    "policy": "carrying-value",
                }
            ],
        )
        state = build_tax_engine(profile).build_ledger_state(inputs)
        self.assertEqual(state.entries, [])
        self.assertEqual(len(state.quarantines), 2)
        reasons_by_id = {q["transaction_id"]: q["reason"] for q in state.quarantines}
        self.assertEqual(
            reasons_by_id,
            {
                "transfer-out": "insufficient_lots",
                "transfer-in": "derived_transfer_group_blocked",
            },
        )
        inbound_detail = json.loads(
            next(
                q["detail_json"]
                for q in state.quarantines
                if q["transaction_id"] == "transfer-in"
            )
        )
        self.assertIs(inbound_detail["paired_leg"], True)
        self.assertEqual(inbound_detail["blocked_by_reason"], "insufficient_lots")

    def test_transfer_pricing_review_targets_used_price_leg(self):
        profile, inputs = self._direct_transfer_engine_inputs()
        # Coarse pricing only quarantines when the profile opts into review.
        profile = {**dict(profile), "require_coarse_review": 1}
        out_row = {
            **inputs.source_rows[1],
            "pricing_source_kind": pricing.SOURCE_MANUAL_RATE_CACHE,
            "pricing_quality": pricing.QUALITY_EXACT,
        }
        unused_coarse_in_row = {
            **inputs.source_rows[2],
            "pricing_source_kind": pricing.SOURCE_FMV_PROVIDER,
            "pricing_quality": pricing.QUALITY_COARSE_FALLBACK,
        }
        normalized = normalize_tax_asset_inputs(
            profile,
            "BTC",
            [out_row, unused_coarse_in_row],
            inputs.wallet_refs_by_id,
            [{"out": out_row, "in": unused_coarse_in_row}],
        )
        self.assertEqual(normalized.quarantines, [])
        self.assertEqual(len(normalized.transfers), 1)

        unpriced_out_row = {
            **out_row,
            "id": "coarse-source-transfer-out",
            "fiat_rate": None,
            "fiat_value": None,
            "pricing_source_kind": None,
            "pricing_quality": None,
        }
        used_coarse_in_row = {
            **unused_coarse_in_row,
            "id": "coarse-source-transfer-in",
        }
        normalized = normalize_tax_asset_inputs(
            profile,
            "BTC",
            [unpriced_out_row, used_coarse_in_row],
            inputs.wallet_refs_by_id,
            [{"out": unpriced_out_row, "in": used_coarse_in_row}],
        )
        self.assertEqual(len(normalized.quarantines), 2)
        quarantine = next(
            q
            for q in normalized.quarantines
            if q["transaction_id"] == "coarse-source-transfer-in"
        )
        self.assertEqual(quarantine["transaction_id"], "coarse-source-transfer-in")
        self.assertEqual(quarantine["reason"], "pricing_review_required")
        detail = json.loads(quarantine["detail_json"])
        self.assertEqual(detail["wallet"], "Hot")
        self.assertEqual(detail["pricing_quality"], pricing.QUALITY_COARSE_FALLBACK)
        partner = next(
            q
            for q in normalized.quarantines
            if q["transaction_id"] == "coarse-source-transfer-out"
        )
        self.assertEqual(partner["reason"], "pricing_review_required")
        self.assertTrue(json.loads(partner["detail_json"])["paired_leg"])

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

    def test_holdings_delta_helpers_skip_double_counted_entry_types(self):
        # The engine books a self-transfer's network fee twice (transfer_out's
        # quantity already includes it AND a separate transfer_fee disposes the
        # same fee) and earned coins twice (an acquisition lot plus an income
        # recognition line). The holdings helpers must skip transfer_fee on the
        # quantity axis and income on both axes so a raw Σ(quantity) does not
        # double-count, matching report_verify._holdings_quantity_formula.
        self.assertEqual(_holdings_quantity_delta("acquisition", Decimal("1.0")), Decimal("1.0"))
        self.assertEqual(_holdings_quantity_delta("transfer_in", Decimal("0.5")), Decimal("0.5"))
        self.assertEqual(_holdings_quantity_delta("transfer_out", Decimal("-0.501")), Decimal("-0.501"))
        self.assertEqual(_holdings_quantity_delta("disposal", Decimal("-0.2")), Decimal("-0.2"))
        self.assertEqual(_holdings_quantity_delta("fee", Decimal("-0.001")), Decimal("-0.001"))
        self.assertEqual(_holdings_quantity_delta("transfer_fee", Decimal("-0.001")), Decimal("0"))
        self.assertEqual(_holdings_quantity_delta("income", Decimal("0.1")), Decimal("0"))
        # Basis axis: add acquisition/transfer_in fiat_value, subtract every
        # sub-side cost_basis (including transfer_fee), skip income entirely.
        self.assertEqual(
            _holdings_basis_delta("acquisition", Decimal("1.0"), Decimal("60000"), Decimal("0")),
            Decimal("60000"),
        )
        self.assertEqual(
            _holdings_basis_delta("transfer_in", Decimal("0.5"), Decimal("0"), Decimal("0")),
            Decimal("0"),
        )
        self.assertEqual(
            _holdings_basis_delta("transfer_out", Decimal("-0.501"), Decimal("0"), Decimal("0")),
            Decimal("0"),
        )
        self.assertEqual(
            _holdings_basis_delta("transfer_fee", Decimal("-0.001"), Decimal("65"), Decimal("60")),
            Decimal("-60"),
        )
        self.assertEqual(
            _holdings_basis_delta("disposal", Decimal("-0.2"), Decimal("0"), Decimal("12000")),
            Decimal("-12000"),
        )
        self.assertEqual(
            _holdings_basis_delta("income", Decimal("0.1"), Decimal("6000"), Decimal("0")),
            Decimal("0"),
        )

    def test_balance_history_and_as_of_portfolio_do_not_double_count_transfer_fee(self):
        # Drive the real engine over the cold->hot self-transfer fixture (1.0 BTC
        # acquired, 0.5 BTC moved with a 0.001 BTC miner fee). The engine emits
        # transfer_out (-0.501, fee included), transfer_in (+0.5) and a separate
        # transfer_fee (-0.001). Both journal-quantity-summing report paths must
        # net the fee ONCE -> profile holdings 0.999 BTC (matching the
        # BalanceSet-derived live portfolio-summary), not 0.998.
        payload, result = self._run_json("init")
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
        payload, result = self._run_json(
            "transfers", "pair",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--tx-out", _FIXTURE_SELF_TRANSFER_TXID,
            "--tx-in", _FIXTURE_SELF_TRANSFER_TXID,
            "--kind", "manual",
            "--policy", "carrying-value",
        )
        self._assert_ok(payload, result, "transfers.pair")
        summary, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
        )
        self._assert_ok(summary, result, "journals.process")

        # balance-history (powers the GUI portfolio chart and the summary PDF):
        # profile-wide BTC holdings net the miner fee once -> 0.999, never 0.998.
        history, result = self._run_json(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--interval", "month",
        )
        self._assert_ok(history, result, "reports.balance-history")
        feb = next(
            row for row in history["data"]
            if row["asset"] == "BTC" and row["period_start"] == "2026-02-01T00:00:00Z"
        )
        self.assertAlmostEqual(feb["quantity"], 0.999, places=8)

        # source-wallet-scoped: Cold loses sent (0.501) once, not sent + fee.
        cold_history, result = self._run_json(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--wallet", "Cold",
            "--interval", "month",
        )
        self._assert_ok(cold_history, result, "reports.balance-history")
        cold_feb = next(
            row for row in cold_history["data"]
            if row["asset"] == "BTC" and row["period_start"] == "2026-02-01T00:00:00Z"
        )
        self.assertAlmostEqual(cold_feb["quantity"], 0.499, places=8)

        # destination-wallet-scoped basis is allocated from the same profile pool
        # as the live/as-of portfolio path; the transfer_in row itself carries no
        # fiat_value, so the old raw wallet sum left Hot at zero basis.
        hot_history, result = self._run_json(
            "reports", "balance-history",
            "--workspace", "Main",
            "--profile", "FixtureTransfer",
            "--wallet", "Hot",
            "--interval", "month",
        )
        self._assert_ok(hot_history, result, "reports.balance-history")
        hot_feb = next(
            row for row in hot_history["data"]
            if row["asset"] == "BTC" and row["period_start"] == "2026-02-01T00:00:00Z"
        )
        self.assertAlmostEqual(hot_feb["quantity"], 0.5, places=8)
        self.assertAlmostEqual(hot_feb["cumulative_cost_basis"], 30000.0, places=0)

        # as-of portfolio-summary (the summary-PDF holdings path; not exposed on
        # the CLI) must agree with the live BalanceSet path: Cold 0.499, Hot 0.5.
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        core_rates.upsert_rate(
            conn,
            "BTC-USD",
            "2026-12-31T00:00:00Z",
            "70000",
            "manual",
            fetched_at="2026-12-31T00:00:01Z",
            granularity="manual",
            method="regression",
        )
        core_rates.upsert_rate(
            conn,
            "BTC-USD",
            "2026-12-31T00:00:00Z",
            "65000",
            "coinbase-exchange",
            fetched_at="2026-12-31T00:00:02Z",
            granularity="manual",
            method="regression-provider",
        )
        conn.commit()
        as_of_rows = {
            row["wallet"]: row
            for row in report_portfolio_summary(
                conn,
                "Main",
                "FixtureTransfer",
                _report_hooks(),
                as_of="2099-01-01T00:00:00Z",
                include_wallet_id=True,
            )
            if row["asset"] == "BTC"
        }
        self.assertAlmostEqual(as_of_rows["Cold"]["quantity"], 0.499, places=8)
        self.assertAlmostEqual(as_of_rows["Hot"]["quantity"], 0.5, places=8)
        # Historical/as-of market value is the summary-PDF holdings path. It
        # must use the cached market rate at the as-of timestamp, not the last
        # transaction import price.
        self.assertAlmostEqual(as_of_rows["Cold"]["market_value"], 34930.0, places=4)
        self.assertAlmostEqual(as_of_rows["Hot"]["market_value"], 35000.0, places=4)
        december_history = [
            row
            for row in report_balance_history(
                conn,
                "Main",
                "FixtureTransfer",
                _report_hooks(),
                interval="month",
                start="2026-12-01T00:00:00Z",
                end="2026-12-31T23:59:59Z",
            )
            if row["asset"] == "BTC"
        ]
        self.assertEqual(len(december_history), 1)
        self.assertAlmostEqual(december_history[0]["quantity"], 0.999, places=8)
        self.assertAlmostEqual(december_history[0]["market_value"], 69930.0, places=4)

        # Per-wallet basis is allocated from the asset's pooled average (matching
        # the live report), so the moved basis follows the coins to Hot instead
        # of stranding in Cold. Both wallets share the ~60000 avg cost, and the
        # destination is no longer zero-basis (the pre-fix raw per-wallet sum gave
        # Cold avg ~120000 and Hot avg 0).
        self.assertAlmostEqual(
            as_of_rows["Cold"]["avg_cost"], as_of_rows["Hot"]["avg_cost"], places=4
        )
        self.assertAlmostEqual(as_of_rows["Cold"]["avg_cost"], 60000.0, places=0)
        self.assertGreater(as_of_rows["Hot"]["cost_basis"], 1000.0)

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
        app_root = home_dir / ".kassiber"
        expected_root = app_root / "projects" / "default"
        self.assertEqual(payload["data"]["project_id"], "default")
        self.assertEqual(payload["data"]["project_root"], str(expected_root))
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
        self.assertTrue((app_root / "config" / "projects.json").exists())
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
        self.assertEqual(payload["data"]["project_id"], "default")
        self.assertEqual(payload["data"]["state_root"], str(expected_root))
        self.assertEqual(payload["data"]["data_root"], str(expected_root / "data"))
        self.assertEqual(payload["data"]["config_root"], str(expected_root / "config"))
        self.assertEqual(payload["data"]["settings_file"], str(expected_root / "config" / "settings.json"))
        self.assertEqual(payload["data"]["exports_root"], str(expected_root / "exports"))
        self.assertEqual(payload["data"]["attachments_root"], str(expected_root / "attachments"))
        self.assertEqual(payload["data"]["env_file"], str(expected_root / "config" / "backends.env"))
        self.assertEqual(payload["data"]["default_backend"], "fulcrum")

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
        self.assertEqual(payload["data"][0]["at_kennzahl"], 172)
        self.assertEqual(payload["data"][0]["gain_loss"], 40.0)

    def test_austrian_e1kv_report_exports_summary_csv_pdf_and_xlsx(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "EUR",
            "--tax-country", "at",
            "Default",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "AustrianE1kv",
            "--kind", "custom",
        )
        self._assert_ok(payload, result, "wallets.create")
        json_file = self.case_dir / "austrian-e1kv-import.json"
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
                        "txid": "at-e1kv-buy",
                        "fiat_value": "40",
                    },
                    {
                        "date": "2024-06-01",
                        "direction": "outbound",
                        "asset": "BTC",
                        "amount": "0.0005",
                        "fee": "0",
                        "kind": "sell",
                        "txid": "at-e1kv-sell",
                        "fiat_value": "30",
                    },
                    {
                        "date": "2024-07-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.0002",
                        "fee": "0",
                        "kind": "staking",
                        "txid": "at-e1kv-staking",
                        "fiat_value": "8",
                    },
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianE1kv",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        payload, result = self._run_json(
            "metadata", "records", "note", "set",
            "--workspace", "Main",
            "--profile", "Default",
            "--transaction", "at-e1kv-staking",
            "--note", "Unicode renderer check: BTC ↔ EUR",
        )
        self._assert_ok(payload, result, "metadata.records.note.set")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        db = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        try:
            cursor = db.execute(
                """
                UPDATE journal_entries
                SET at_kennzahl = 175
                WHERE at_category = 'income_capital_yield'
                  AND transaction_id = (
                    SELECT id FROM transactions WHERE external_id = ?
                  )
                """,
                ("at-e1kv-staking",),
            )
            self.assertEqual(cursor.rowcount, 1)
            db.commit()
        finally:
            db.close()

        payload, result = self._run_json(
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-e1kv")
        report = payload["data"]
        self.assertEqual(report["form"], "E 1kv")
        self.assertEqual(report["tax_year"], 2024)
        self.assertIn(
            "AT-E1KV-FOREIGN-SELF-CUSTODY",
            {assumption["code"] for assumption in report["assumptions"]},
        )
        summary_by_kennzahl = {row["kennzahl"]: row for row in report["summary_rows"]}
        self.assertEqual(summary_by_kennzahl[172]["amount_eur_cents"], 800)
        self.assertEqual(summary_by_kennzahl[174]["amount_eur_cents"], 1000)
        self.assertEqual(summary_by_kennzahl[176]["amount_eur_cents"], 0)
        self.assertEqual(summary_by_kennzahl[801]["amount_eur_cents"], 0)
        self.assertEqual(summary_by_kennzahl[172]["form"], "E 1kv")
        self.assertEqual(summary_by_kennzahl[801]["form"], "E 1")
        self.assertEqual(report["kennzahl_totals"]["172"]["amount_eur_cents"], 800)
        self.assertEqual(report["kennzahl_totals"]["801"]["form"], "E 1")
        self.assertIn("2.1", report["sections"])
        self.assertEqual(report["sections"]["2.1"]["totals"]["amount_eur_cents"], 800)
        self.assertEqual(report["sections"]["3.3"]["status"], "not_modelled")
        self.assertEqual(report["sections"]["4.4"]["status"], "not_modelled")
        rows_by_tx = {row["tx_id"]: row for row in report["rows"]}
        self.assertEqual(rows_by_tx["at-e1kv-staking"]["kennzahl"], 172)
        self.assertEqual(rows_by_tx["at-e1kv-staking"]["stored_kennzahl"], 175)
        self.assertEqual(rows_by_tx["at-e1kv-staking"]["income_eur_cents"], 800)
        self.assertEqual(rows_by_tx["at-e1kv-sell"]["kennzahl"], 174)
        self.assertEqual(rows_by_tx["at-e1kv-sell"]["gain_loss_eur_cents"], 1000)
        self.assertEqual(
            report["data_quality"]["kennzahl_mismatches"],
            [
                {
                    "tx_id": "at-e1kv-staking",
                    "at_category": "income_capital_yield",
                    "stored_kennzahl": 175,
                    "export_kennzahl": 172,
                }
            ],
        )

        db = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        db.row_factory = sqlite3.Row
        try:
            snapshot = build_capital_gains_snapshot(db)
        finally:
            db.close()
        snapshot_rows = {row["code"]: row for row in snapshot["kennzahlRows"]}
        self.assertEqual(snapshot["year"], 2024)
        self.assertEqual(snapshot_rows["172"]["amount"], 8.0)
        self.assertEqual(snapshot_rows["174"]["amount"], 10.0)
        self.assertEqual(snapshot_rows["176"]["amount"], 0.0)
        self.assertEqual(snapshot_rows["801"]["amount"], 0.0)
        self.assertEqual(snapshot_rows["172"]["source"], "daemon")
        self.assertEqual(snapshot_rows["172"]["form"], "E 1kv")
        self.assertEqual(snapshot_rows["801"]["form"], "E 1")

        payload, result = self._run_json(
            "reports", "austrian-tax-summary",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-tax-summary")
        self.assertEqual(payload["data"]["form"], "E 1kv")
        self.assertEqual(payload["data"]["tax_year"], 2024)
        self.assertEqual(payload["data"]["sections"]["2.1"]["totals"]["amount_eur_cents"], 800)

        csv_file = self.case_dir / "austrian-e1kv.csv"
        result = self._run_cli(
            "--format", "csv",
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            output=csv_file,
        )
        self.assertEqual(result.returncode, 0, msg=result.stderr)
        csv_text = csv_file.read_text(encoding="utf-8")
        self.assertIn("form_amount_eur_cents", csv_text.splitlines()[0])
        self.assertIn("at-e1kv-staking", csv_text)
        self.assertIn(",172,", csv_text)

        pdf_file = self.case_dir / "austrian-e1kv.pdf"
        payload, result = self._run_json(
            "reports", "export-austrian-e1kv-pdf",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            "--file", str(pdf_file),
        )
        self._assert_ok(payload, result, "reports.export-austrian-e1kv-pdf")
        self.assertEqual(payload["data"]["form"], "E 1kv")
        self.assertEqual(payload["data"]["renderer"], "reportlab")
        self.assertEqual(payload["data"]["transactions"], 3)
        self.assertIn("besonderheiten", payload["data"]["sections"])
        self.assertIn("steuerformulare", payload["data"]["sections"])
        self.assertIn("faq", payload["data"]["sections"])
        self.assertGreater(payload["data"]["pages"], 0)
        pdf_bytes = pdf_file.read_bytes()
        self.assertTrue(pdf_bytes.startswith(b"%PDF"))
        self.assertGreater(len(pdf_bytes), 0)
        self.assertIn("Kassiber Steuerbericht 2024", payload["data"]["title"])
        if shutil.which("pdftotext"):
            extracted = subprocess.run(
                ["pdftotext", "-layout", str(pdf_file), "-"],
                check=True,
                text=True,
                capture_output=True,
            ).stdout
            self.assertIn("Kassiber Steuerbericht", extracted)
            self.assertIn("Transaktionsübersicht", extracted)
            self.assertIn("Steuerformulare", extracted)
            self.assertIn("FinanzOnline", extracted)
            self.assertIn("Besonderheiten", extracted)
            self.assertIn("FAQ", extracted)
            self.assertIn("€ (EUR)", extracted)
            self.assertIn("BTC ↔ EUR", extracted)
            self.assertNotIn("NFT", extracted)

        plain_result = self._run_cli(
            "--format", "plain",
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self.assertEqual(plain_result.returncode, 0, msg=plain_result.stderr)
        self.assertIn("II. Detail Sections", plain_result.stdout)
        self.assertIn("3.3. Nicht steuerbare Steuergebühren und Rückerstattungen", plain_result.stdout)
        self.assertIn("Summe laufende Einkünfte", plain_result.stdout)
        self.assertIn("Some rows had stale stored Kennzahlen", plain_result.stdout)
        self.assertIn("| at-e1kv-staking |", plain_result.stdout)
        self.assertIn("| 175 | 172 |", plain_result.stdout)

        alias_pdf_file = self.case_dir / "austrian-alias.pdf"
        payload, result = self._run_json(
            "reports", "export-austrian",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            "--file", str(alias_pdf_file),
        )
        self._assert_ok(payload, result, "reports.export-austrian")
        self.assertEqual(payload["data"]["form"], "E 1kv")
        self.assertEqual(payload["data"]["renderer"], "reportlab")
        self.assertGreater(alias_pdf_file.stat().st_size, 0)

        xlsx_file = self.case_dir / "austrian-e1kv.xlsx"
        payload, result = self._run_json(
            "reports", "export-austrian-e1kv-xlsx",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            "--file", str(xlsx_file),
        )
        self._assert_ok(payload, result, "reports.export-austrian-e1kv-xlsx")
        self.assertEqual(payload["data"]["form"], "E 1kv")
        self.assertEqual(
            payload["data"]["sheets"],
            [
                "Übersicht",
                "1.1.",
                "1.2.",
                "1.3.",
                "2.1.",
                "2.2.",
                "3.1.",
                "3.2.",
                "3.3.",
                "4.1.",
                "4.2.",
                "4.3.",
                "4.4.",
                "4.5.",
                "Erläuterungen zum Steuerreport",
            ],
        )
        self.assertIn("2.1.", payload["data"]["sheets"])
        self.assertGreater(xlsx_file.stat().st_size, 0)
        with zipfile.ZipFile(xlsx_file) as workbook:
            names = set(workbook.namelist())
            self.assertIn("xl/workbook.xml", names)
            workbook_xml = workbook.read("xl/workbook.xml").decode("utf-8")
            shared_strings = workbook.read("xl/sharedStrings.xml").decode("utf-8")
        self.assertIn('name="Übersicht"', workbook_xml)
        self.assertIn('name="1.1."', workbook_xml)
        self.assertIn('name="1.3."', workbook_xml)
        self.assertIn('name="3.3."', workbook_xml)
        self.assertIn('name="Erläuterungen zum Steuerreport"', workbook_xml)
        self.assertIn("at-e1kv-staking", shared_strings)
        self.assertIn("Summe laufende Einkünfte", shared_strings)
        self.assertIn("Kennzahl-Abweichungen", shared_strings)
        self.assertIn("Transaktion | Kategorie | gespeichert | Export", shared_strings)
        self.assertNotIn("NFT", shared_strings)
        self.assertIn(
            "AT-E1KV-KENNZAHL-REPROCESS",
            {assumption["code"] for assumption in report["assumptions"]},
        )

        csv_bundle_dir = self.case_dir / "austrian-e1kv-csv"
        payload, result = self._run_json(
            "reports", "export-austrian-e1kv-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            "--dir", str(csv_bundle_dir),
        )
        self._assert_ok(payload, result, "reports.export-austrian-e1kv-csv")
        self.assertEqual(payload["data"]["form"], "E 1kv")
        self.assertIn("Übersicht", payload["data"]["sheets"])
        self.assertIn("3.3.", payload["data"]["sheets"])
        self.assertEqual(len(payload["data"]["files"]), 15)
        expected_bundle_filenames = [
            "00_uebersicht.csv",
            "01_1.1.csv",
            "02_1.2.csv",
            "03_1.3.csv",
            "04_2.1.csv",
            "05_2.2.csv",
            "06_3.1.csv",
            "07_3.2.csv",
            "08_3.3.csv",
            "09_4.1.csv",
            "10_4.2.csv",
            "11_4.3.csv",
            "12_4.4.csv",
            "13_4.5.csv",
            "99_erlaeuterungen_zum_steuerreport.csv",
        ]
        self.assertEqual(
            [Path(file["file"]).name for file in payload["data"]["files"]],
            expected_bundle_filenames,
        )
        overview_csv = csv_bundle_dir / "00_uebersicht.csv"
        section_21_csv = csv_bundle_dir / "04_2.1.csv"
        section_33_csv = csv_bundle_dir / "08_3.3.csv"
        notes_csv = csv_bundle_dir / "99_erlaeuterungen_zum_steuerreport.csv"
        self.assertTrue(overview_csv.exists())
        self.assertTrue(section_21_csv.exists())
        self.assertTrue(section_33_csv.exists())
        self.assertTrue(notes_csv.exists())
        self.assertIn("2.1. Einkünfte aus der Überlassung", overview_csv.read_text(encoding="utf-8"))
        section_21_text = section_21_csv.read_text(encoding="utf-8")
        self.assertIn("at-e1kv-staking", section_21_text)
        self.assertIn("Summe laufende Einkünfte", section_21_text)
        self.assertIn("Summe entrichtete Steuergebühren", section_33_csv.read_text(encoding="utf-8"))
        notes_text = notes_csv.read_text(encoding="utf-8")
        self.assertIn("AT-E1KV-KENNZAHL-REPROCESS", notes_text)
        self.assertIn("Kennzahl-Abweichungen", notes_text)
        self.assertIn("at-e1kv-staking", notes_text)
        self.assertNotIn("NFT", notes_text)

    def test_pdf_report_transliterates_non_latin1_glyphs(self):
        # The generic text PDF transliterates common non-Latin-1 glyphs to
        # legible ASCII instead of silently dropping them to "?" (the exit-tax
        # lines emit em dashes and a warning sign). Austrian E 1kv and
        # source-funds PDFs use ReportLab renderers instead.
        from kassiber.pdf_report import _ascii_text

        self.assertEqual(_ascii_text("€"), "EUR")
        self.assertEqual(_ascii_text("₿"), "BTC")
        self.assertEqual(_ascii_text("↔"), "<->")
        self.assertEqual(_ascii_text("estimate — value"), "estimate - value")
        self.assertEqual(_ascii_text("⚠ 3 quarantined"), "(!) 3 quarantined")
        # Anything still outside Latin-1 degrades to "?" rather than crashing.
        self.assertEqual(_ascii_text("中"), "?")
        # Latin-1 covers German umlauts and ß, so they survive today.
        self.assertEqual(_ascii_text("Übersicht"), "Übersicht")
        self.assertEqual(_ascii_text("Größe ä ö ü ß"), "Größe ä ö ü ß")

    def test_austrian_e1kv_empty_year_keeps_unsupported_placeholders(self):
        self._bootstrap_austrian_e1kv_wallet(label="AustrianEmpty")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-e1kv")
        report = payload["data"]
        self.assertEqual(report["rows"], [])
        self.assertEqual(report["data_quality"]["quarantines"], [])
        self.assertEqual(report["sections"]["1.2"]["status"], "not_modelled")
        self.assertEqual(report["sections"]["3.3"]["status"], "not_modelled")
        self.assertEqual(report["sections"]["4.5"]["status"], "not_modelled")
        self.assertEqual(report["sections"]["1.2"]["totals"]["amount_eur_cents"], 0)
        self.assertEqual(report["sections"]["3.3"]["detail_rows"], [])
        self.assertEqual(report["sections"]["4.5"]["detail_rows"], [])

        plain_result = self._run_cli(
            "--format", "plain",
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self.assertEqual(plain_result.returncode, 0, msg=plain_result.stderr)
        self.assertIn("1.2. Steuerpflichtige Einkünfte aus Margin", plain_result.stdout)
        self.assertIn("3.3. Nicht steuerbare Steuergebühren und Rückerstattungen", plain_result.stdout)
        self.assertIn("4.5. Minting", plain_result.stdout)
        self.assertIn("No rows in scope.", plain_result.stdout)

        csv_bundle_dir = self.case_dir / "austrian-empty-e1kv-csv"
        payload, result = self._run_json(
            "reports", "export-austrian-e1kv-csv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
            "--dir", str(csv_bundle_dir),
        )
        self._assert_ok(payload, result, "reports.export-austrian-e1kv-csv")
        self.assertEqual(len(payload["data"]["files"]), 15)
        self.assertIn(
            "No rows in scope.",
            (csv_bundle_dir / "02_1.2.csv").read_text(encoding="utf-8"),
        )
        self.assertIn(
            "Summe Spekulationseinkünfte",
            (csv_bundle_dir / "03_1.3.csv").read_text(encoding="utf-8"),
        )
        self.assertIn(
            "Summe der Rückerstattungen",
            (csv_bundle_dir / "08_3.3.csv").read_text(encoding="utf-8"),
        )
        self.assertIn(
            "Summe Minting",
            (csv_bundle_dir / "13_4.5.csv").read_text(encoding="utf-8"),
        )

    def test_austrian_e1kv_quarantined_rows_stay_out_but_counts_visible(self):
        self._bootstrap_austrian_e1kv_wallet(label="AustrianQuarantine")
        json_file = self.case_dir / "austrian-e1kv-quarantine-import.json"
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
                        "txid": "at-e1kv-quarantine-buy",
                        "fiat_value": "40",
                    },
                    {
                        "date": "2024-06-01",
                        "direction": "outbound",
                        "asset": "BTC",
                        "amount": "0.0005",
                        "fee": "0",
                        "kind": "sell",
                        "txid": "at-e1kv-quarantine-sell",
                    },
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianQuarantine",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")
        self.assertEqual(payload["data"]["quarantined"], 1)

        payload, result = self._run_json(
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-e1kv")
        report = payload["data"]
        self.assertNotIn(
            "at-e1kv-quarantine-sell",
            {row["tx_id"] for row in report["rows"]},
        )
        self.assertEqual(
            report["data_quality"]["quarantines"],
            [{"reason": "missing_spot_price", "count": 1}],
        )

        plain_result = self._run_cli(
            "--format", "plain",
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self.assertEqual(plain_result.returncode, 0, msg=plain_result.stderr)
        self.assertIn("Quarantined transactions remain outside this export", plain_result.stdout)
        self.assertIn("missing_spot_price", plain_result.stdout)
        self.assertNotIn("at-e1kv-quarantine-sell", plain_result.stdout)

    def test_austrian_e1kv_reports_loss_as_positive_kz176(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "EUR",
            "--tax-country", "at",
            "Default",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "wallets", "create",
            "--workspace", "Main",
            "--profile", "Default",
            "--label", "AustrianLoss",
            "--kind", "custom",
        )
        self._assert_ok(payload, result, "wallets.create")
        json_file = self.case_dir / "austrian-loss-import.json"
        json_file.write_text(
            json.dumps(
                [
                    {
                        "date": "2024-02-01",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount": "0.001",
                        "fee": "0",
                        "kind": "buy",
                        "txid": "at-loss-buy",
                        "fiat_value": "80",
                    },
                    {
                        "date": "2024-08-01",
                        "direction": "outbound",
                        "asset": "BTC",
                        "amount": "0.0005",
                        "fee": "0",
                        "kind": "sell",
                        "txid": "at-loss-sell",
                        "fiat_value": "30",
                    },
                ]
            ),
            encoding="utf-8",
        )
        payload, result = self._run_json(
            "wallets", "import-json",
            "--workspace", "Main",
            "--profile", "Default",
            "--wallet", "AustrianLoss",
            "--file", str(json_file),
        )
        self._assert_ok(payload, result, "wallets.import-json")
        payload, result = self._run_json(
            "journals", "process",
            "--workspace", "Main",
            "--profile", "Default",
        )
        self._assert_ok(payload, result, "journals.process")

        payload, result = self._run_json(
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Default",
            "--year", "2024",
        )
        self._assert_ok(payload, result, "reports.austrian-e1kv")
        report = payload["data"]
        summary_by_kennzahl = {row["kennzahl"]: row for row in report["summary_rows"]}
        # Basis 40 EUR, proceeds 30 EUR → loss 10 EUR. KZ 176 reports abs value.
        self.assertEqual(summary_by_kennzahl[176]["amount_eur_cents"], 1000)
        self.assertEqual(summary_by_kennzahl[174]["amount_eur_cents"], 0)
        rows_by_tx = {row["tx_id"]: row for row in report["rows"]}
        loss_row = rows_by_tx["at-loss-sell"]
        self.assertEqual(loss_row["kennzahl"], 176)
        self.assertEqual(loss_row["gain_loss_eur_cents"], -1000)
        self.assertEqual(loss_row["form_amount_eur_cents"], 1000)
        self.assertEqual(report["sections"]["1.1"]["totals"]["gain_loss_eur_cents"], -1000)

    def test_austrian_e1kv_rejects_non_austrian_profile(self):
        payload, result = self._run_json("init")
        self._assert_ok(payload, result, "init")
        payload, result = self._run_json("workspaces", "create", "Main")
        self._assert_ok(payload, result, "workspaces.create")
        payload, result = self._run_json(
            "profiles", "create",
            "--workspace", "Main",
            "--fiat-currency", "USD",
            "--tax-country", "generic",
            "Generic",
        )
        self._assert_ok(payload, result, "profiles.create")
        payload, result = self._run_json(
            "reports", "austrian-e1kv",
            "--workspace", "Main",
            "--profile", "Generic",
            "--year", "2024",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")
        self.assertIn("Austrian", payload["error"]["message"])

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

    def test_attachments_add_invalid_file_label_does_not_copy_managed_file(self):
        self._bootstrap_wallet(label="AttachInvalidLabel")
        self._insert_transaction(
            wallet_label="AttachInvalidLabel",
            tx_id="attach-invalid-label",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=100_000_000,
        )
        attachment_file = self.case_dir / "invalid-label-receipt.txt"
        attachment_file.write_text("receipt\n", encoding="utf-8")

        payload, result = self._run_json(
            "attachments",
            "add",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--transaction",
            "attach-invalid-label",
            "--file",
            str(attachment_file),
            "--label",
            "   ",
        )
        self.assertEqual(result.returncode, 1, msg=payload)
        self.assertEqual(payload.get("kind"), "error")
        self.assertEqual(payload["error"]["code"], "validation")

        attachments_root = self.case_dir / "attachments"
        managed_files = (
            [path for path in attachments_root.rglob("*") if path.is_file()]
            if attachments_root.exists()
            else []
        )
        self.assertEqual(managed_files, [])

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        count = conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0]
        conn.close()
        self.assertEqual(count, 0)

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

    def test_attachments_remove_keeps_row_when_file_delete_fails(self):
        self._bootstrap_wallet(label="AttachRollback")
        self._insert_transaction(
            wallet_label="AttachRollback",
            tx_id="attach-rollback",
            occurred_at="2024-01-01T00:00:00Z",
            amount_msat=100_000_000,
        )
        attachment_file = self.case_dir / "rollback-receipt.txt"
        attachment_file.write_text("rollback\n", encoding="utf-8")
        payload, result = self._run_json(
            "attachments",
            "add",
            "--workspace",
            "Main",
            "--profile",
            "Default",
            "--transaction",
            "attach-rollback",
            "--file",
            str(attachment_file),
        )
        self._assert_ok(payload, result, "attachments.add")
        attachment_id = payload["data"]["id"]
        stored_path = self.case_dir / "attachments" / payload["data"]["stored_relpath"]
        self.assertTrue(stored_path.exists())

        conn = sqlite3.connect(self.data_root / "kassiber.sqlite3")
        conn.row_factory = sqlite3.Row
        with patch("pathlib.Path.unlink", side_effect=PermissionError("blocked")):
            with self.assertRaises(AppError) as ctx:
                core_attachments.remove_attachment(
                    conn,
                    str(self.data_root),
                    "Main",
                    "Default",
                    attachment_id,
                    _attachment_hooks(),
                )
        self.assertEqual(ctx.exception.code, "filesystem_error")
        row = conn.execute("SELECT id FROM attachments WHERE id = ?", (attachment_id,)).fetchone()
        conn.close()
        self.assertIsNotNone(row)
        self.assertTrue(stored_path.exists())

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

    def test_migration_collapses_wallet_scoped_bip329_duplicates(self):
        self.data_root.mkdir(parents=True, exist_ok=True)
        db_path = self.data_root / "kassiber.sqlite3"
        conn = sqlite3.connect(db_path)
        conn.executescript(_OLD_SCHEMA_SQL)
        conn.executescript(
            """
            INSERT INTO workspaces VALUES('ws', 'Main', '2024-01-01T00:00:00Z');
            INSERT INTO profiles VALUES('pf', 'ws', 'Default', 'USD', 'generic', 365, 'FIFO', NULL, 0, '2024-01-01T00:00:00Z');
            INSERT INTO accounts VALUES('acct', 'ws', 'pf', 'cash', 'Cash', 'asset', 'BTC', '2024-01-01T00:00:00Z');
            INSERT INTO wallets VALUES('wal-a', 'ws', 'pf', 'acct', 'Wallet A', 'address', '{}', '2024-01-01T00:00:00Z');
            INSERT INTO wallets VALUES('wal-b', 'ws', 'pf', 'acct', 'Wallet B', 'address', '{}', '2024-01-01T00:00:00Z');
            """
        )
        conn.executemany(
            """
            INSERT INTO bip329_labels(
                id, workspace_id, profile_id, wallet_id, record_type, ref,
                label, origin, spendable, data_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            [
                (
                    "label-old",
                    "ws",
                    "pf",
                    "wal-a",
                    "output",
                    "migrated-tx:0",
                    "old",
                    "wallet",
                    0,
                    '{"first": 1}',
                    "2024-01-01T00:00:00Z",
                ),
                (
                    "label-new",
                    "ws",
                    "pf",
                    "wal-b",
                    "output",
                    "migrated-tx:0",
                    "new",
                    None,
                    None,
                    '{"second": 2}',
                    "2024-01-02T00:00:00Z",
                ),
            ],
        )
        conn.commit()
        conn.close()

        conn = open_db(self.data_root)
        rows = conn.execute(
            """
            SELECT wallet_id, record_type, ref, label, origin, spendable, data_json
            FROM bip329_labels
            """
        ).fetchall()
        unique_index = conn.execute(
            """
            SELECT name
            FROM sqlite_master
            WHERE type = 'index' AND name = 'idx_bip329_labels_profile_object'
            """
        ).fetchone()
        conn.close()

        self.assertIsNotNone(unique_index)
        self.assertEqual(len(rows), 1)
        self.assertIsNone(rows[0]["wallet_id"])
        self.assertEqual(rows[0]["record_type"], "output")
        self.assertEqual(rows[0]["ref"], "migrated-tx:0")
        self.assertEqual(rows[0]["label"], "new")
        self.assertEqual(rows[0]["origin"], "wallet")
        self.assertEqual(rows[0]["spendable"], 0)
        self.assertEqual(json.loads(rows[0]["data_json"]), {"first": 1, "second": 2})

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


class ReviewBadgesSnapshotTest(unittest.TestCase):
    """ui.review.badges feeds the side-nav unresolved-item hints."""

    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-review-badges-")
        self.addCleanup(self._tmp.cleanup)
        self.data_root = Path(self._tmp.name) / "data"

    def _seed_book(self, conn, *, with_transactions, processed):
        now = "2026-01-01T00:00:00Z"
        conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws-b", "Badges WS", now),
        )
        conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, last_processed_at,
                last_processed_tx_count, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "pf-b", "ws-b", "Badges PF", "EUR", "generic", 365, "FIFO",
                "2026-02-02T00:00:00Z" if processed else None,
                1 if processed else 0,
                now,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, label, kind, config_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?)
            """,
            ("wal-b", "ws-b", "pf-b", "Cold", "address", "{}", now),
        )
        if with_transactions:
            conn.execute(
                """
                INSERT INTO transactions(
                    id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                    occurred_at, confirmed_at, direction, asset, amount, fee,
                    fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                    description, counterparty, note, excluded, raw_json, created_at
                ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                """,
                (
                    "tx-b", "ws-b", "pf-b", "wal-b", "x" * 64, "fp-b",
                    "2026-01-10T10:00:00Z", "2026-01-10T10:10:00Z", "inbound",
                    "BTC", btc_to_msat("1.0"), 0, "EUR", 50_000, 50_000, "import",
                    "transfer", "Funding", "Exchange", None, 0, "{}",
                    "2026-01-10T10:00:00Z",
                ),
            )
        set_setting(conn, "context_workspace", "ws-b")
        set_setting(conn, "context_profile", "pf-b")
        conn.commit()

    def test_no_active_profile_shows_no_hints(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        snapshot = build_review_badges_snapshot(conn)
        self.assertEqual(snapshot["quarantine"], 0)
        self.assertFalse(snapshot["journals_needs_processing"])
        self.assertIsNone(snapshot["swaps"])

    def test_quarantine_count_and_needs_processing(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        self._seed_book(conn, with_transactions=True, processed=False)
        now = "2026-01-01T00:00:00Z"
        # A second transaction so the count exercises >1 (journal_quarantines is
        # UNIQUE per transaction_id).
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id, fingerprint,
                occurred_at, confirmed_at, direction, asset, amount, fee,
                fiat_currency, fiat_rate, fiat_value, fiat_price_source, kind,
                description, counterparty, note, excluded, raw_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-b2", "ws-b", "pf-b", "wal-b", "y" * 64, "fp-b2",
                "2026-01-11T10:00:00Z", "2026-01-11T10:10:00Z", "inbound",
                "BTC", btc_to_msat("0.5"), 0, "EUR", 50_000, 25_000, "import",
                "transfer", "Funding 2", "Exchange", None, 0, "{}",
                "2026-01-11T10:00:00Z",
            ),
        )
        conn.executemany(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            ) VALUES(?, ?, ?, ?, ?, ?)
            """,
            [
                ("tx-b", "ws-b", "pf-b", "missing_spot_price", "{}", now),
                ("tx-b2", "ws-b", "pf-b", "missing_fee_price", "{}", now),
            ],
        )
        conn.commit()
        snapshot = build_review_badges_snapshot(conn)
        self.assertEqual(snapshot["quarantine"], 2)
        # Transactions exist but were never processed -> the Ledger hint fires.
        self.assertTrue(snapshot["journals_needs_processing"])
        # Matcher has not run yet -> no swaps badge (None, not a misleading 0).
        self.assertIsNone(snapshot["swaps"])

    def test_empty_book_is_quiet(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        self._seed_book(conn, with_transactions=False, processed=False)
        snapshot = build_review_badges_snapshot(conn)
        self.assertEqual(snapshot["quarantine"], 0)
        # No active transactions -> nothing to process, no nag.
        self.assertFalse(snapshot["journals_needs_processing"])
        self.assertIsNone(snapshot["swaps"])

    def test_cached_swap_count_round_trips(self):
        conn = open_db(self.data_root)
        self.addCleanup(conn.close)
        self._seed_book(conn, with_transactions=True, processed=True)
        cache_swap_candidate_count(conn, "ws-b", "pf-b", 4)
        conn.commit()
        self.assertEqual(build_review_badges_snapshot(conn)["swaps"], 4)
        # A matched count of 0 reports 0 (UI hides it), distinct from the
        # never-computed None on a fresh book.
        cache_swap_candidate_count(conn, "ws-b", "pf-b", 0)
        conn.commit()
        self.assertEqual(build_review_badges_snapshot(conn)["swaps"], 0)


if __name__ == "__main__":
    unittest.main()
