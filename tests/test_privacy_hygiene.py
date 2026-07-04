from __future__ import annotations

import json
import importlib.util
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

from kassiber.ai.tools import get_tool, openai_tool_definitions
from kassiber.core import reports as core_reports


ROOT = Path(__file__).resolve().parent.parent
PRIVACY_KIND = "ui.reports.privacy_hygiene"
MIRROR_KIND = "ui.reports.privacy_mirror"
PSBT_KIND = "ui.reports.psbt_privacy"


def _run_cli(data_root: Path, *args: str, machine: bool = True) -> dict | str:
    cmd = [sys.executable, "-m", "kassiber", "--data-root", str(data_root)]
    if machine:
        cmd.append("--machine")
    cmd.extend(args)
    env = os.environ.copy()
    env["PYTHONPATH"] = str(ROOT)
    result = subprocess.run(
        cmd,
        cwd=ROOT,
        env=env,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise AssertionError(
            f"CLI failed: {' '.join(cmd)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    if machine:
        return json.loads(result.stdout)
    return result.stdout


def _sqlite_conn(data_root: Path) -> sqlite3.Connection:
    conn = sqlite3.connect(data_root / "kassiber.sqlite3")
    conn.row_factory = sqlite3.Row
    return conn


def _json_keys(value) -> set[str]:
    if isinstance(value, dict):
        keys = {str(key) for key in value}
        for item in value.values():
            keys.update(_json_keys(item))
        return keys
    if isinstance(value, list):
        keys: set[str] = set()
        for item in value:
            keys.update(_json_keys(item))
        return keys
    return set()


def _bootstrap_book(data_root: Path) -> tuple[str, str]:
    _run_cli(data_root, "init")
    _run_cli(data_root, "workspaces", "create", "Demo")
    _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
    _run_cli(data_root, "context", "set", "--workspace", "Demo", "--profile", "Main")
    with _sqlite_conn(data_root) as conn:
        workspace = conn.execute(
            "SELECT id FROM workspaces WHERE label = 'Demo'"
        ).fetchone()
        profile = conn.execute(
            "SELECT id FROM profiles WHERE label = 'Main'"
        ).fetchone()
        return workspace["id"], profile["id"]


def _seed_sensitive_privacy_material(
    data_root: Path,
    workspace_id: str,
    profile_id: str,
) -> None:
    ts = "2026-01-01T00:00:00Z"
    sensitive_url = "https://user:pass@api.example.com/v1?token=tok_secret"
    sensitive_descriptor = "wpkh([abcd1234/84h/0h/0h]xpub661MySecret/0/*)"
    sensitive_address = "bc1qsecretaddress000000000000000000000000"
    sensitive_script = "0014deadbeefcafebabesecretscript"
    sensitive_txid = "a" * 64
    wallet_id = "wallet-sensitive"
    tx_id = "tx-sensitive"
    with _sqlite_conn(data_root) as conn:
        conn.execute(
            """
            INSERT INTO backends(
                name, kind, chain, network, url, auth_header, token, batch_size,
                timeout, tor_proxy, config_json, notes, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "remote-sensitive",
                "esplora",
                "bitcoin",
                "main",
                sensitive_url,
                "Bearer super-secret-header",
                "tok_secret",
                None,
                None,
                None,
                json.dumps({"username": "wallet-secret-user", "password": "wallet-secret-pass"}),
                "sensitive backend notes",
                ts,
                ts,
            ),
        )
        conn.execute(
            """
            INSERT INTO ai_providers(
                name, base_url, api_key, default_model, kind, notes,
                acknowledged_at, created_at, updated_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "cloud-secret",
                "https://api.openai.example/v1",
                "sk-super-secret",
                "gpt-test",
                "remote",
                "remote model",
                ts,
                ts,
                ts,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                wallet_id,
                workspace_id,
                profile_id,
                None,
                "Sensitive wallet",
                "descriptor",
                json.dumps(
                    {
                        "backend": "remote-sensitive",
                        "descriptor": sensitive_descriptor,
                        "xpub": "xpub661MySecret",
                        "addresses": [sensitive_address],
                        "branch_index": 7,
                        "derivation_path": "m/84h/0h/0h",
                    }
                ),
                ts,
            ),
        )
        conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, confirmed_at, direction, asset,
                amount, fee, privacy_boundary, raw_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                workspace_id,
                profile_id,
                wallet_id,
                sensitive_txid,
                "privacy-fingerprint-1",
                ts,
                ts,
                "inbound",
                "BTC",
                100_000_000,
                0,
                "coinjoin",
                json.dumps(
                    {
                        "address": sensitive_address,
                        "script_pubkey": sensitive_script,
                        "descriptor": sensitive_descriptor,
                    }
                ),
                ts,
            ),
        )
        conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, backend_name,
                backend_kind, chain, network, asset, amount, txid, vout,
                outpoint, confirmation_status, confirmations, block_height,
                block_time, address, script_pubkey, branch_label, branch_index,
                address_index, first_seen_at, last_seen_at, raw_json
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "utxo-sensitive",
                workspace_id,
                profile_id,
                wallet_id,
                "remote-sensitive",
                "esplora",
                "bitcoin",
                "main",
                "BTC",
                100_000_000,
                sensitive_txid,
                0,
                f"{sensitive_txid}:0",
                "confirmed",
                6,
                840_000,
                ts,
                sensitive_address,
                sensitive_script,
                "receive",
                0,
                7,
                ts,
                ts,
                json.dumps({"address": sensitive_address, "script_pubkey": sensitive_script}),
            ),
        )
        conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                tx_id,
                workspace_id,
                profile_id,
                "privacy_hop_unresolved",
                json.dumps({"address": sensitive_address}),
                ts,
            ),
        )
        conn.commit()


def _unused_report_hook(*_args, **_kwargs):
    raise AssertionError("privacy-hygiene should not call tax/report hooks")


def _privacy_report_hooks(
    workspace_id: str,
    profile_id: str,
) -> core_reports.ReportHooks:
    def _resolve_scope(_conn, _workspace_ref, _profile_ref):
        return (
            {"id": workspace_id, "label": "Demo"},
            {
                "id": profile_id,
                "label": "Main",
                "last_processed_at": None,
                "last_processed_tx_count": 0,
                "journal_input_version": 0,
                "last_processed_input_version": 0,
            },
        )

    return core_reports.ReportHooks(
        resolve_scope=_resolve_scope,
        resolve_account=_unused_report_hook,
        resolve_wallet=_unused_report_hook,
        require_processed_journals=_unused_report_hook,
        build_ledger_state=_unused_report_hook,
        list_journal_entries=_unused_report_hook,
        list_wallets=_unused_report_hook,
        parse_iso_datetime=_unused_report_hook,
        iso_z=_unused_report_hook,
        now_iso=_unused_report_hook,
        format_table=_unused_report_hook,
        write_text_pdf=_unused_report_hook,
    )


def _minimal_privacy_conn(profile_id: str) -> sqlite3.Connection:
    conn = sqlite3.connect(":memory:")
    conn.row_factory = sqlite3.Row
    conn.executescript(
        """
        CREATE TABLE backends (
            name TEXT PRIMARY KEY,
            kind TEXT,
            chain TEXT,
            network TEXT,
            url TEXT,
            auth_header TEXT,
            token TEXT,
            tor_proxy TEXT,
            config_json TEXT
        );
        CREATE TABLE ai_providers (
            name TEXT PRIMARY KEY,
            base_url TEXT,
            api_key TEXT,
            kind TEXT,
            acknowledged_at TEXT
        );
        CREATE TABLE wallets (
            profile_id TEXT,
            kind TEXT,
            config_json TEXT
        );
        CREATE TABLE transactions (
            id TEXT,
            profile_id TEXT,
            external_id TEXT,
            direction TEXT,
            amount INTEGER,
            asset TEXT,
            fee INTEGER,
            excluded INTEGER,
            raw_json TEXT,
            privacy_boundary TEXT
        );
        CREATE TABLE wallet_utxos (
            profile_id TEXT,
            wallet_id TEXT,
            txid TEXT,
            vout INTEGER,
            amount INTEGER,
            spent_at TEXT,
            address TEXT,
            script_pubkey TEXT,
            branch_label TEXT,
            branch_index INTEGER,
            address_index INTEGER,
            spent_by TEXT,
            asset TEXT,
            chain TEXT
        );
        CREATE TABLE journal_entries (
            profile_id TEXT
        );
        CREATE TABLE journal_quarantines (
            profile_id TEXT,
            reason TEXT
        );
        """
    )
    sensitive_url = "https://user:pass@api.example.com/v1?token=tok_secret"
    sensitive_descriptor = "wpkh([abcd1234/84h/0h/0h]xpub661MySecret/0/*)"
    sensitive_address = "bc1qsecretaddress000000000000000000000000"
    sensitive_script = "0014deadbeefcafebabesecretscript"
    conn.execute(
        """
        INSERT INTO backends(
            name, kind, chain, network, url, auth_header, token, tor_proxy, config_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "remote-sensitive",
            "esplora",
            "bitcoin",
            "main",
            sensitive_url,
            "Bearer super-secret-header",
            "tok_secret",
            "",
            json.dumps({"username": "wallet-secret-user", "password": "wallet-secret-pass"}),
        ),
    )
    conn.executemany(
        """
        INSERT INTO ai_providers(name, base_url, api_key, kind, acknowledged_at)
        VALUES (?, ?, ?, ?, ?)
        """,
        [
            ("cloud-secret", "https://api.openai.example/v1", "sk-super-secret", "remote", "now"),
            ("cli-default", "claude-cli://default", None, "remote", "now"),
        ],
    )
    conn.execute(
        "INSERT INTO wallets(profile_id, kind, config_json) VALUES (?, ?, ?)",
        (
            profile_id,
            "descriptor",
            json.dumps(
                {
                    "backend": "remote-sensitive",
                    "descriptor": sensitive_descriptor,
                    "xpub": "xpub661MySecret",
                    "addresses": [sensitive_address],
                    "branch_index": 7,
                    "derivation_path": "m/84h/0h/0h",
                }
            ),
        ),
    )
    conn.execute(
        """
        INSERT INTO transactions(
            id, profile_id, external_id, direction, amount, asset, fee,
            excluded, raw_json, privacy_boundary
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            "tx-sensitive",
            profile_id,
            "a" * 64,
            "outbound",
            100_000_000,
            "BTC",
            1_000,
            0,
            json.dumps(
                {
                    "address": sensitive_address,
                    "script_pubkey": sensitive_script,
                    "descriptor": sensitive_descriptor,
                    "fee": 1000,
                    "vout": [{"scriptpubkey_type": "op_return"}],
                }
            ),
            "coinjoin",
        ),
    )
    conn.execute(
        """
        INSERT INTO wallet_utxos(
            profile_id, wallet_id, txid, vout, amount, spent_at, address,
            script_pubkey, branch_label, branch_index, address_index, spent_by,
            asset, chain
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        """,
        (
            profile_id,
            "wallet-sensitive",
            "a" * 64,
            0,
            100_000_000,
            None,
            sensitive_address,
            sensitive_script,
            "receive",
            0,
            7,
            None,
            "BTC",
            "bitcoin",
        ),
    )
    conn.execute(
        "INSERT INTO journal_quarantines(profile_id, reason) VALUES (?, ?)",
        (profile_id, "privacy_hop_unresolved"),
    )
    conn.commit()
    return conn


class PrivacyHygieneTests(unittest.TestCase):
    def test_core_payload_is_redacted_and_counts_cli_ai_provider_once(self):
        workspace_id = "workspace-demo"
        profile_id = "profile-demo"
        conn = _minimal_privacy_conn(profile_id)
        try:
            payload = core_reports.report_privacy_hygiene(
                conn,
                None,
                None,
                _privacy_report_hooks(workspace_id, profile_id),
            )
        finally:
            conn.close()

        self.assertTrue(payload["local_only"])
        self.assertTrue(payload["read_only"])
        self.assertEqual(payload["redaction"], "ai_export_safe")
        self.assertEqual(payload["summary"]["remote_backend_count"], 1)
        self.assertEqual(payload["facts"]["ai"]["remote_provider_count"], 1)
        self.assertEqual(payload["facts"]["ai"]["cli_provider_count"], 1)
        self.assertEqual(payload["summary"]["off_device_ai_provider_count"], 2)
        self.assertEqual(payload["summary"]["privacy_quarantine_count"], 1)
        for finding in payload["findings"]:
            self.assertIn(finding["evidence_level"], {"exact", "derived", "unknown"})

        serialized = json.dumps(payload, sort_keys=True)
        for secret in (
            "api.example.com",
            "tok_secret",
            "super-secret-header",
            "wallet-secret-pass",
            "xpub661MySecret",
            "bc1qsecretaddress",
            "0014deadbeefcafebabesecretscript",
            "m/84h/0h/0h",
        ):
            self.assertNotIn(secret, serialized)

    def test_privacy_mirror_payload_is_redacted_and_has_worst_risk(self):
        workspace_id = "workspace-demo"
        profile_id = "profile-demo"
        conn = _minimal_privacy_conn(profile_id)
        try:
            payload = core_reports.report_privacy_mirror(
                conn,
                None,
                None,
                _privacy_report_hooks(workspace_id, profile_id),
            )
        finally:
            conn.close()

        self.assertTrue(payload["local_only"])
        self.assertTrue(payload["read_only"])
        self.assertTrue(payload["advisory_only"])
        self.assertEqual(payload["redaction"], "ai_export_safe")
        self.assertIn(payload["summary"]["evidence_level"], {"exact", "derived", "unknown"})
        self.assertGreaterEqual(payload["summary"]["wallet_count"], 1)
        self.assertGreaterEqual(payload["summary"]["utxo_count"], 1)
        self.assertGreaterEqual(payload["summary"]["adversary_view_count"], 1)
        self.assertTrue(payload["summary"]["worst_risk"]["answer"])
        self.assertTrue(payload["adversary_cards"])
        self.assertTrue(payload["wallet_view"])
        self.assertTrue(payload["utxo_view"])
        self.assertTrue(payload["timeline"])
        self.assertTrue(payload["coverage"])
        self.assertTrue(payload["unknowns"])
        self.assertTrue(payload["evidence_drilldowns"])
        self.assertEqual(
            payload["psbt_what_if_panel"]["status"],
            "available_via_reports_psbt_privacy",
        )
        for section in (
            "adversary_cards",
            "wallet_view",
            "transaction_view",
            "utxo_view",
            "timeline",
            "unknowns",
            "evidence_drilldowns",
        ):
            for row in payload[section]:
                self.assertIn(row["evidence_level"], {"exact", "derived", "unknown"})

        serialized = json.dumps(payload, sort_keys=True)
        for secret in (
            "api.example.com",
            "tok_secret",
            "super-secret-header",
            "wallet-secret-pass",
            "xpub661MySecret",
            "bc1qsecretaddress",
            "0014deadbeefcafebabesecretscript",
            "m/84h/0h/0h",
        ):
            self.assertNotIn(secret, serialized)
        self.assertFalse(
            {
                "address",
                "script_pubkey",
                "raw_json",
                "descriptor",
                "xpub",
                "branch_index",
                "address_index",
                "derivation_path",
            }
            & _json_keys(payload)
        )

    @unittest.skipUnless(
        importlib.util.find_spec("embit") is not None,
        "CLI privacy-hygiene test requires runtime dependencies",
    )
    def test_cli_json_and_table_are_redacted_and_include_evidence_levels(self):
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp)
            workspace_id, profile_id = _bootstrap_book(data_root)
            _seed_sensitive_privacy_material(data_root, workspace_id, profile_id)

            envelope = _run_cli(data_root, "reports", "privacy-hygiene")
            self.assertEqual(envelope["kind"], "reports.privacy-hygiene")
            payload = envelope["data"]
            self.assertTrue(payload["local_only"])
            self.assertTrue(payload["read_only"])
            self.assertEqual(payload["redaction"], "ai_export_safe")
            self.assertEqual(payload["facts"]["database"]["status"], "plaintext")
            self.assertGreaterEqual(payload["summary"]["remote_backend_count"], 1)
            self.assertEqual(payload["summary"]["privacy_quarantine_count"], 1)
            self.assertTrue(payload["findings"])
            for finding in payload["findings"]:
                self.assertIn(finding["evidence_level"], {"exact", "derived", "unknown"})

            serialized = json.dumps(payload, sort_keys=True)
            for secret in (
                "api.example.com",
                "tok_secret",
                "super-secret-header",
                "wallet-secret-pass",
                "xpub661MySecret",
                "bc1qsecretaddress",
                "0014deadbeefcafebabesecretscript",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "m/84h/0h/0h",
            ):
                self.assertNotIn(secret, serialized)

            table = _run_cli(
                data_root,
                "--format",
                "table",
                "reports",
                "privacy-hygiene",
                machine=False,
            )
            self.assertIn("evidence_level", table)
            self.assertIn("remote_backend_endpoints", table)
            self.assertIn("privacy_quarantines_present", table)
            self.assertNotIn("api.example.com", table)
            self.assertNotIn("xpub661MySecret", table)

            mirror_envelope = _run_cli(data_root, "reports", "privacy-mirror")
            self.assertEqual(mirror_envelope["kind"], "reports.privacy-mirror")
            mirror_payload = mirror_envelope["data"]
            self.assertTrue(mirror_payload["local_only"])
            self.assertTrue(mirror_payload["read_only"])
            self.assertTrue(mirror_payload["advisory_only"])
            self.assertEqual(mirror_payload["redaction"], "ai_export_safe")
            self.assertIn(
                mirror_payload["summary"]["evidence_level"],
                {"exact", "derived", "unknown"},
            )
            self.assertTrue(mirror_payload["summary"]["worst_risk"]["answer"])
            self.assertIn("adversary_cards", mirror_payload)
            self.assertIn("wallet_view", mirror_payload)
            self.assertIn("transaction_view", mirror_payload)
            self.assertIn("utxo_view", mirror_payload)
            self.assertIn("timeline", mirror_payload)
            self.assertIn("coverage", mirror_payload)
            self.assertIn("unknowns", mirror_payload)
            self.assertIn("evidence_drilldowns", mirror_payload)
            mirror_serialized = json.dumps(mirror_payload, sort_keys=True)
            for secret in (
                "api.example.com",
                "tok_secret",
                "super-secret-header",
                "wallet-secret-pass",
                "xpub661MySecret",
                "bc1qsecretaddress",
                "0014deadbeefcafebabesecretscript",
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "m/84h/0h/0h",
            ):
                self.assertNotIn(secret, mirror_serialized)
            self.assertFalse(
                {
                    "address",
                    "script_pubkey",
                    "raw_json",
                    "descriptor",
                    "xpub",
                    "branch_index",
                    "address_index",
                    "derivation_path",
                }
                & _json_keys(mirror_payload)
            )

            mirror_table = _run_cli(
                data_root,
                "--format",
                "table",
                "reports",
                "privacy-mirror",
                machine=False,
            )
            self.assertIn("evidence_level", mirror_table)
            self.assertIn("worst_privacy_risk", mirror_table)
            self.assertNotIn("api.example.com", mirror_table)
            self.assertNotIn("xpub661MySecret", mirror_table)

    def test_ai_tool_schema_is_read_only_and_closed(self):
        tool = get_tool("ui.reports.privacy_hygiene")
        self.assertIsNotNone(tool)
        assert tool is not None
        self.assertEqual(tool.kind_class, "read_only")
        self.assertEqual(tool.daemon_kind, PRIVACY_KIND)
        self.assertEqual(tool.parameters["type"], "object")
        self.assertFalse(tool.parameters["additionalProperties"])
        self.assertEqual(tool.parameters["properties"], {})
        self.assertIs(get_tool("ui_reports_privacy_hygiene"), tool)

        definitions = openai_tool_definitions()
        definition = next(
            item
            for item in definitions
            if item["function"]["name"] == "ui_reports_privacy_hygiene"
        )
        self.assertEqual(definition["function"]["parameters"], tool.parameters)

        mirror_tool = get_tool(MIRROR_KIND)
        self.assertIsNotNone(mirror_tool)
        assert mirror_tool is not None
        self.assertEqual(mirror_tool.kind_class, "read_only")
        self.assertEqual(mirror_tool.daemon_kind, MIRROR_KIND)
        self.assertFalse(mirror_tool.parameters["additionalProperties"])
        self.assertEqual(mirror_tool.parameters["properties"], {})
        self.assertIs(get_tool("ui_reports_privacy_mirror"), mirror_tool)
        self.assertIsNone(get_tool(PSBT_KIND))
        self.assertIsNone(get_tool("ui_reports_psbt_privacy"))

        mirror_definition = next(
            item
            for item in definitions
            if item["function"]["name"] == "ui_reports_privacy_mirror"
        )
        self.assertEqual(mirror_definition["function"]["parameters"], mirror_tool.parameters)

    def test_privacy_hygiene_allowlists_are_wired(self):
        daemon = (ROOT / "kassiber" / "daemon.py").read_text(encoding="utf-8")
        supported_match = re.search(
            r"SUPPORTED_KINDS\s*=\s*\((?P<body>.*?)\)\n",
            daemon,
            re.DOTALL,
        )
        self.assertIsNotNone(supported_match)
        supported = set(re.findall(r'"([^"]+)"', supported_match.group("body")))
        self.assertIn(PRIVACY_KIND, supported)
        tauri = (
            ROOT / "ui-tauri" / "src-tauri" / "src" / "lib.rs"
        ).read_text(encoding="utf-8")
        vite = (ROOT / "ui-tauri" / "vite.config.ts").read_text(encoding="utf-8")
        tauri_allowed = set(
            re.findall(
                r'"([^"]+)"',
                re.search(
                    r"ALLOWED_DAEMON_KINDS[^=]*=\s*&\[(?P<body>.*?)\];",
                    tauri,
                    re.DOTALL,
                ).group("body"),
            )
        )
        vite_allowed = set(
            re.findall(
                r'"([^"]+)"',
                re.search(
                    r"ALLOWED_BRIDGE_KINDS\s*=\s*new Set\(\[(?P<body>.*?)\]\);",
                    vite,
                    re.DOTALL,
                ).group("body"),
            )
        )
        for kind in (PRIVACY_KIND, MIRROR_KIND, PSBT_KIND):
            self.assertIn(kind, supported)
        for kind in (PRIVACY_KIND, MIRROR_KIND, PSBT_KIND):
            self.assertIn(kind, tauri_allowed)
            self.assertIn(kind, vite_allowed)

    def test_docs_cover_cli_and_ui_vs_ai_redaction(self):
        daemon_doc = (ROOT / "docs" / "reference" / "daemon.md").read_text(
            encoding="utf-8"
        )
        ai_doc = (ROOT / "docs" / "reference" / "ai.md").read_text(
            encoding="utf-8"
        )
        privacy_doc = (ROOT / "docs" / "reference" / "privacy-mirror.md").read_text(
            encoding="utf-8"
        )
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("kassiber reports privacy-hygiene", daemon_doc)
        self.assertIn("kassiber reports privacy-mirror", daemon_doc)
        self.assertIn("ui_reports_privacy_hygiene", ai_doc)
        self.assertIn("ui_reports_privacy_mirror", ai_doc)
        self.assertIn("privacy-hygiene payload and is not what the AI tool receives", daemon_doc)
        self.assertIn("contents must not be exposed to AI", daemon_doc)
        self.assertIn("The GUI may separately show", ai_doc)
        self.assertIn("evidence_level", privacy_doc)
        self.assertIn("Degraded States", privacy_doc)
        self.assertIn("Redaction", privacy_doc)
        self.assertIn("Non-Goals", privacy_doc)
        self.assertIn("coin selection advice", privacy_doc)
        self.assertIn("kassiber reports privacy-hygiene", readme)
        self.assertIn("kassiber reports privacy-mirror", readme)


if __name__ == "__main__":
    unittest.main()
