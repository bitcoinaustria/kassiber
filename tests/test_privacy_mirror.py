from __future__ import annotations

import importlib.util
import json
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
from kassiber.db import open_db, set_setting

from .privacy_assertions import assert_tier3_linkage_identifiers_absent


ROOT = Path(__file__).resolve().parent.parent
MIRROR_KIND = "ui.reports.privacy_mirror"
PSBT_KIND = "ui.reports.psbt_privacy"
NOW = "2026-07-01T12:00:00Z"
SENSITIVE_TXID = "a" * 64
SENSITIVE_OUTPOINT = f"{SENSITIVE_TXID}:0"
SENSITIVE_FINGERPRINT = "privacy-fingerprint-1"


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


def _unused_report_hook(*_args, **_kwargs):
    raise AssertionError("privacy mirror should not call tax/report hooks")


def _privacy_report_hooks(workspace_id: str, profile_id: str) -> core_reports.ReportHooks:
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


class PrivacyMirrorTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory(prefix="kassiber-privacy-mirror-")
        self.data_root = Path(self._tmp.name) / "data"
        self.conn = open_db(self.data_root)
        self._bootstrap_book()
        self._seed_sensitive_material()

    def tearDown(self):
        self.conn.close()
        self._tmp.cleanup()

    def _bootstrap_book(self):
        self.conn.execute(
            "INSERT INTO workspaces(id, label, created_at) VALUES(?, ?, ?)",
            ("ws", "Demo", NOW),
        )
        self.conn.execute(
            """
            INSERT INTO profiles(
                id, workspace_id, label, fiat_currency, tax_country,
                tax_long_term_days, gains_algorithm, created_at
            )
            VALUES(?, ?, ?, ?, ?, ?, ?, ?)
            """,
            ("pf", "ws", "Main", "EUR", "generic", 365, "FIFO", NOW),
        )
        set_setting(self.conn, "context_workspace", "ws")
        set_setting(self.conn, "context_profile", "pf")
        self.conn.commit()

    def _seed_sensitive_material(self):
        sensitive_url = "https://user:pass@api.example.com/v1?token=tok_secret"
        sensitive_descriptor = "wpkh([abcd1234/84h/0h/0h]xpub661MySecret/0/*)"
        sensitive_address = "bc1qsecretaddress000000000000000000000000"
        sensitive_script = "0014deadbeefcafebabesecretscript"
        self.conn.execute(
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
                json.dumps({"password": "wallet-secret-pass"}),
                "sensitive backend notes",
                NOW,
                NOW,
            ),
        )
        self.conn.execute(
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
                NOW,
                NOW,
                NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO wallets(
                id, workspace_id, profile_id, account_id, label, kind,
                config_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "wal",
                "ws",
                "pf",
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
                NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO transactions(
                id, workspace_id, profile_id, wallet_id, external_id,
                fingerprint, occurred_at, confirmed_at, direction, asset,
                amount, fee, privacy_boundary, kind, description, raw_json,
                created_at
            )
            VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-sensitive",
                "ws",
                "pf",
                "wal",
                SENSITIVE_TXID,
                SENSITIVE_FINGERPRINT,
                NOW,
                NOW,
                "outbound",
                "BTC",
                100_000_000,
                1_000,
                "coinjoin",
                "withdrawal",
                "Synced",
                json.dumps(
                    {
                        "address": sensitive_address,
                        "script_pubkey": sensitive_script,
                        "descriptor": sensitive_descriptor,
                        "fee": 1000,
                        "vout": [{"scriptpubkey_type": "op_return"}],
                    }
                ),
                NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO wallet_utxos(
                id, workspace_id, profile_id, wallet_id, backend_name,
                backend_kind, chain, network, asset, amount, txid, vout,
                outpoint, confirmation_status, confirmations, block_height,
                block_time, address, script_pubkey, address_label,
                branch_label, branch_index, address_index, anonymity_score,
                spent_by, excluded_from_coinjoin, key_state, anon_history_json,
                first_seen_at, last_seen_at, spent_at, raw_json
            )
            VALUES(
                ?, 'ws', 'pf', 'wal', 'remote-sensitive',
                'esplora', 'bitcoin', 'main', 'BTC', ?, ?, ?,
                ?, 'confirmed', 6, 880000,
                ?, ?, ?, '', 'receive', 0, 7, NULL,
                NULL, NULL, '', '[]', ?, ?, NULL, '{}'
            )
            """,
            (
                "utxo-sensitive",
                100_000_000,
                SENSITIVE_TXID,
                0,
                SENSITIVE_OUTPOINT,
                NOW,
                sensitive_address,
                sensitive_script,
                NOW,
                NOW,
            ),
        )
        self.conn.execute(
            """
            INSERT INTO journal_quarantines(
                transaction_id, workspace_id, profile_id, reason, detail_json, created_at
            )
            VALUES (?, ?, ?, ?, ?, ?)
            """,
            (
                "tx-sensitive",
                "ws",
                "pf",
                "privacy_hop_unresolved",
                json.dumps({"address": sensitive_address}),
                NOW,
            ),
        )
        self.conn.commit()

    def test_core_payload_is_redacted_and_has_worst_risk(self):
        payload = core_reports.report_privacy_mirror(
            self.conn,
            None,
            None,
            _privacy_report_hooks("ws", "pf"),
        )

        self.assertTrue(payload["local_only"])
        self.assertTrue(payload["read_only"])
        self.assertTrue(payload["advisory_only"])
        self.assertEqual(payload["redaction"], "ai_export_safe")
        self.assertIn(payload["summary"]["evidence_level"], {"exact", "derived", "unknown"})
        self.assertGreaterEqual(payload["summary"]["wallet_count"], 1)
        self.assertGreaterEqual(payload["summary"]["utxo_count"], 1)
        self.assertGreaterEqual(payload["summary"]["adversary_view_count"], 1)
        self.assertTrue(payload["summary"]["worst_risk"]["answer"])
        for section in (
            "adversary_cards",
            "wallet_view",
            "transaction_view",
            "utxo_view",
            "timeline",
            "unknowns",
            "evidence_drilldowns",
        ):
            self.assertIn(section, payload)
            for row in payload[section]:
                self.assertIn(row["evidence_level"], {"exact", "derived", "unknown"})
        self.assertEqual(
            payload["psbt_what_if_panel"]["status"],
            "available_via_reports_psbt_privacy",
        )

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
        assert_tier3_linkage_identifiers_absent(
            self,
            payload,
            forbidden_values=(
                SENSITIVE_TXID,
                SENSITIVE_OUTPOINT,
                SENSITIVE_FINGERPRINT,
            ),
        )
        # The reduced payload keeps useful opaque references even though the
        # underlying outpoint and transaction id are gone.
        self.assertTrue(payload["utxo_view"][0]["coin_id"])

    def test_privacy_score_is_grounded_bounded_and_explainable(self):
        payload = core_reports.report_privacy_mirror(
            self.conn,
            None,
            None,
            _privacy_report_hooks("ws", "pf"),
        )
        score = payload["summary"]["privacy_score"]
        self.assertIsInstance(score["value"], int)
        self.assertGreaterEqual(score["value"], 0)
        self.assertLessEqual(score["value"], 100)
        self.assertEqual(score["base"], 100)
        self.assertGreaterEqual(score["coverage_ratio"], 0.0)
        self.assertLessEqual(score["coverage_ratio"], 1.0)
        self.assertEqual(
            {factor["key"] for factor in score["factors"]},
            {"wallet_linkage", "transaction_leaks"},
        )
        # The value is exactly base plus the (negative) factor points, so the UI
        # can render an honest waterfall instead of an opaque number.
        self.assertEqual(
            score["value"],
            max(0, min(100, score["base"] + sum(f["points"] for f in score["factors"]))),
        )

    def test_privacy_score_formula_is_deterministic(self):
        score = core_reports._privacy_mirror_score(
            wallet_rows=[{"linkage_edge_count": 1}, {"linkage_edge_count": 0}],
            transaction_rows=[
                {"tell_count": 2, "tell_kinds": ["sender_common_input", "fee_fingerprint"]},
                {"tell_count": 1, "tell_kinds": ["op_return_output"]},
            ],
            hygiene_summary={"active_transaction_count": 4},
            coverage_known=1,
            coverage_unknown=1,
        )
        # 1 of 2 wallets linked -> linkage 0.5 -> round(100*0.55*0.5) = 28.
        # Leaks weighted by strongest tell: max(1.0, 0.3) + 0.25 = 1.25 over 4 tx
        # -> 0.3125 -> round(100*0.45*0.3125) = 14. 100 - 28 - 14 = 58.
        self.assertEqual(score["value"], 58)
        self.assertEqual(score["coverage_ratio"], 0.5)

        # A strong ownership tell costs more than a weak metadata tell.
        strong = core_reports._privacy_mirror_score(
            wallet_rows=[{"linkage_edge_count": 0}],
            transaction_rows=[{"tell_count": 1, "tell_kinds": ["sender_common_input"]}],
            hygiene_summary={"active_transaction_count": 1},
            coverage_known=1,
            coverage_unknown=0,
        )
        weak = core_reports._privacy_mirror_score(
            wallet_rows=[{"linkage_edge_count": 0}],
            transaction_rows=[{"tell_count": 1, "tell_kinds": ["op_return_output"]}],
            hygiene_summary={"active_transaction_count": 1},
            coverage_known=1,
            coverage_unknown=0,
        )
        self.assertEqual(strong["value"], 55)  # 100 - round(45 * 1.0)
        self.assertEqual(weak["value"], 89)  # 100 - round(45 * 0.25)
        self.assertLess(strong["value"], weak["value"])

        # Unknown coins lower coverage/confidence, never the score itself.
        clean = core_reports._privacy_mirror_score(
            wallet_rows=[{"linkage_edge_count": 0}],
            transaction_rows=[{"tell_count": 0}],
            hygiene_summary={"active_transaction_count": 1},
            coverage_known=0,
            coverage_unknown=5,
        )
        self.assertEqual(clean["value"], 100)
        self.assertEqual(clean["coverage_ratio"], 0.0)

    @unittest.skipUnless(
        importlib.util.find_spec("embit") is not None,
        "CLI privacy-mirror test requires runtime dependencies",
    )
    def test_cli_privacy_mirror_json_and_table_are_redacted(self):
        with tempfile.TemporaryDirectory() as tmp:
            data_root = Path(tmp)
            _run_cli(data_root, "init")
            with sqlite3.connect(data_root / "kassiber.sqlite3") as raw_conn:
                raw_conn.row_factory = sqlite3.Row
                self.conn.backup(raw_conn)

            envelope = _run_cli(data_root, "reports", "privacy-mirror")
            self.assertEqual(envelope["kind"], "reports.privacy-mirror")
            payload = envelope["data"]
            self.assertEqual(payload["redaction"], "ai_export_safe")
            self.assertTrue(payload["summary"]["worst_risk"]["answer"])
            serialized = json.dumps(payload, sort_keys=True)
            self.assertNotIn("api.example.com", serialized)
            self.assertNotIn("xpub661MySecret", serialized)

            table = _run_cli(
                data_root,
                "--format",
                "table",
                "reports",
                "privacy-mirror",
                machine=False,
            )
            self.assertIn("evidence_level", table)
            self.assertIn("worst_risk", table)
            self.assertNotIn("api.example.com", table)

    def test_ai_schema_and_allowlists_are_wired(self):
        tool = get_tool(MIRROR_KIND)
        self.assertIsNotNone(tool)
        assert tool is not None
        self.assertEqual(tool.kind_class, "read_only")
        self.assertEqual(tool.daemon_kind, MIRROR_KIND)
        self.assertFalse(tool.parameters["additionalProperties"])
        self.assertEqual(tool.parameters["properties"], {})
        self.assertIs(get_tool("ui_reports_privacy_mirror"), tool)
        self.assertIsNone(get_tool(PSBT_KIND))
        self.assertIsNone(get_tool("ui_reports_psbt_privacy"))

        definition = next(
            item
            for item in openai_tool_definitions()
            if item["function"]["name"] == "ui_reports_privacy_mirror"
        )
        self.assertEqual(definition["function"]["parameters"], tool.parameters)

        daemon = (ROOT / "kassiber" / "daemon.py").read_text(encoding="utf-8")
        supported_match = re.search(
            r"SUPPORTED_KINDS\s*=\s*\((?P<body>.*?)\)\n",
            daemon,
            re.DOTALL,
        )
        self.assertIsNotNone(supported_match)
        supported = set(re.findall(r'"([^"]+)"', supported_match.group("body")))
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
        for kind in (MIRROR_KIND, PSBT_KIND):
            self.assertIn(kind, supported)
            self.assertIn(kind, tauri_allowed)
            self.assertIn(kind, vite_allowed)

    def test_docs_cover_privacy_mirror_methodology_and_redaction(self):
        daemon_doc = (ROOT / "docs" / "reference" / "daemon.md").read_text(
            encoding="utf-8"
        )
        ai_doc = (ROOT / "docs" / "reference" / "ai.md").read_text(encoding="utf-8")
        privacy_doc = (ROOT / "docs" / "reference" / "privacy-mirror.md").read_text(
            encoding="utf-8"
        )
        readme = (ROOT / "README.md").read_text(encoding="utf-8")
        self.assertIn("kassiber reports privacy-mirror", daemon_doc)
        self.assertIn("ui_reports_privacy_mirror", ai_doc)
        self.assertIn("contents must not be exposed to AI", daemon_doc)
        self.assertIn("evidence_level", privacy_doc)
        self.assertIn("Degraded States", privacy_doc)
        self.assertIn("Redaction", privacy_doc)
        self.assertIn("Non-Goals", privacy_doc)
        self.assertIn("coin selection advice", privacy_doc)
        self.assertIn("kassiber reports privacy-mirror", readme)


if __name__ == "__main__":
    unittest.main()
