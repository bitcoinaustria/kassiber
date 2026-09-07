from __future__ import annotations

import json
import os
import re
import io
import queue
from unittest.mock import patch
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace

from kassiber.ai.tools import get_tool, responses_tool_definitions
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
    env["PYTHONPATH"] = os.pathsep.join(
        path for path in (str(ROOT), env.get("PYTHONPATH")) if path
    )
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

    def _report(self, *, redacted=False):
        return core_reports.report_privacy_mirror(
            self.conn, None, None, _privacy_report_hooks("ws", "pf"), redacted=redacted,
        )

    def _clear_observations(self):
        self.conn.execute("DELETE FROM journal_quarantines")
        self.conn.execute("DELETE FROM wallet_utxos")
        self.conn.execute("DELETE FROM transactions")

    def _tx(self, number, parents=(), *, outputs=(1000,), network="main", chain="bitcoin", boundary=None,
            record=None, wallet="wal", scripts=None, extra=None):
        txid = f"{number:064x}"
        vin = []
        for parent, vout in parents:
            parent_id = f"{parent:064x}"
            candidates = self.conn.execute("SELECT raw_json FROM transactions WHERE external_id=?", (parent_id,)).fetchall()
            raw_parent = next((json.loads(row[0]) for row in candidates if json.loads(row[0]).get("network") == network), {})
            prevout = raw_parent.get("vout", [])[vout] if raw_parent else {"value": 1000}
            vin.append({"txid": parent_id, "vout": vout, "sequence": 0xFFFFFFFF, "prevout": prevout})
        raw = {
            "txid": txid, "chain": chain, "network": network, "version": 2, "locktime": 0,
            "vin": vin or [{"coinbase": "0101"}], "vsize": 111,
            "vout": [
                {**({"value": value} if value is not None else {"valuecommitment": "08" + "11" * 32}),
                 "scriptpubkey": scripts[index] if scripts else "0014" + f"{number * 10 + index:040x}"}
                for index, value in enumerate(outputs)
            ], **(extra or {}),
        }
        record_id = record or f"{chain}-{network}-{number}"
        self.conn.execute(
            "INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,external_id,fingerprint,occurred_at,confirmed_at,direction,asset,amount,fee,kind,privacy_boundary,raw_json,created_at) VALUES(?, 'ws','pf',?,?,?,?,?,'outbound',?,1000000,0,'withdrawal',?,?,?)",
            (record_id, wallet, txid, record_id, NOW, NOW, "LBTC" if chain == "liquid" else "BTC", boundary, json.dumps(raw), NOW),
        )
        return raw

    def _own(self, number, vout=0, *, network="main", chain="bitcoin", branch="receive", wallet="wal"):
        txid = f"{number:064x}"
        candidates = self.conn.execute("SELECT raw_json FROM transactions WHERE external_id=?", (txid,)).fetchall()
        raw = next((json.loads(row[0]) for row in candidates if json.loads(row[0]).get("network") == network), {})
        output = raw.get("vout", [])[vout] if raw else {}
        canonical_network = "liquidv1" if chain == "liquid" and network == "main" else network
        self.conn.execute(
            "INSERT INTO wallet_utxos(id,workspace_id,profile_id,wallet_id,chain,network,asset,amount,txid,vout,outpoint,confirmation_status,script_pubkey,branch_label,first_seen_at,last_seen_at) VALUES(?,'ws','pf',?,?,?,?,?,?,?,?,'confirmed',?,?,?,?)",
            (f"{wallet}-{chain}-{network}-{number}-{vout}", wallet, chain, canonical_network,
             "LBTC" if chain == "liquid" else "BTC", (output.get("value") or 1000) * 1000,
             txid, vout, f"{txid}:{vout}", output.get("scriptpubkey"), branch, NOW, NOW),
        )

    def _codes(self, payload):
        return {row["code"] for row in payload["findings"]}

    def _checks(self, payload):
        return {row["code"]: row for row in payload["coverage"]["checks"]}

    def _semantic_findings(self, payload):
        return [{key: value for key, value in row.items() if key != "investigation"} for row in payload["findings"]]

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
    def test_v2_report_is_evidence_based_and_redacted_by_default(self):
        payload = core_reports.report_privacy_mirror(self.conn, None, None, _privacy_report_hooks("ws", "pf"))
        self.assertEqual(payload["payload_schema_version"], 2)
        self.assertEqual(payload["observer"], "public")
        self.assertTrue(all(payload[key] for key in ("local_only", "read_only", "advisory_only")))
        self.assertEqual(payload["redaction"], "ai_export_safe")
        self.assertIn(payload["summary"]["status"], {"findings", "no_observed_exposure", "unavailable"})
        self.assertGreaterEqual(payload["summary"]["owned_output_count"], 1)
        self.assertFalse({"privacy_score", "score", "grade", "adversary_cards", "wallet_view", "transaction_view", "psbt_what_if_panel", "capability_catalog"} & _json_keys(payload))
        self.assertTrue(payload["assumptions"])
        serialized = json.dumps(payload, sort_keys=True)
        for secret in ("api.example.com", "tok_secret", "super-secret-header", "wallet-secret-pass", "xpub661MySecret", "bc1qsecretaddress", "0014deadbeefcafebabesecretscript", "m/84h/0h/0h"):
            self.assertNotIn(secret, serialized)
        assert_tier3_linkage_identifiers_absent(self, payload, forbidden_values=(SENSITIVE_TXID, SENSITIVE_OUTPOINT, SENSITIVE_FINGERPRINT))

    def test_empty_or_graphless_book_never_claims_privacy_assurance(self):
        self._clear_observations()
        empty = self._report()
        self.assertEqual(empty["summary"]["status"], "unavailable")
        self.assertEqual(empty["coverage"]["status"], "unavailable")
        self.assertEqual(empty["summary"]["owned_output_count"], 0)
        self._own(1)
        graphless = self._report()
        self.assertEqual(graphless["summary"]["status"], "unavailable")
        self.assertEqual(graphless["summary"]["analyzed_transaction_count"], 0)
        self.assertNotEqual(graphless["coverage"]["status"], "complete")
        self.assertTrue(graphless["coverage"]["missing_nodes"] or graphless["coverage"]["stopped_reasons"])

    def test_mirror_and_workbench_share_the_same_public_snapshot_and_findings(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        self._tx(2, [(1, 0)], outputs=(1700, 200), extra={"vin": [{"txid": f"{1:064x}", "vout": 0, "sequence": 0xFFFFFFFD, "prevout": {"value": 2000}}]})
        with patch("kassiber.core.privacy_mirror.build_index", wraps=build_index) as build:
            mirror = self._report()
        self.assertEqual(build.call_count, 1, "Mirror must freeze one index, not compose independent report reads")
        investigation = mirror["investigation"]
        self.assertEqual(investigation["query"]["observer"], "public")
        self.assertFalse(investigation["query"]["include_relations"])
        workbench = analyze_snapshot(build_index(self.conn, "pf"), investigation["query"])
        self.assertEqual(investigation["snapshot_id"], workbench["snapshot_id"])
        structural = next(row for row in mirror["findings"] if row["code"] == "explicit_rbf_signal")
        original = next(row for row in workbench["findings"] if row["code"] == "explicit_rbf_signal")
        self.assertEqual(structural["id"], original["id"])
        self.assertEqual(structural["authority"], original["authority"])
        self.assertEqual(structural["relevance"], "own_spend")

    def test_collaboration_suppresses_ownership_hypotheses_across_duplicate_wallet_observations(self):
        self._clear_observations()
        self.conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES('other-wallet','ws','pf','Other wallet','descriptor','{}',?)", (NOW,))
        self._tx(1, outputs=(1100,))
        self._tx(2, outputs=(1000,))
        self._own(1)
        for boundary in ("payjoin", "coinjoin"):
            with self.subTest(boundary=boundary):
                self.conn.execute("DELETE FROM transactions WHERE external_id=?", (f"{3:064x}",))
                self._tx(3, [(1, 0), (2, 0)], outputs=(1000, 1000), boundary=boundary)
                before = self._report()
                self.assertIn("collaborative_boundary", self._codes(before))
                physical = next(row for row in before["findings"] if row["code"] == "observed_co_spend")
                self.assertEqual(physical["authority"], "observed")
                self.assertNotIn("common_input_control", self._codes(before))
                self.assertNotIn("change_script_return", self._codes(before))
                self._tx(3, [(1, 0), (2, 0)], outputs=(1000, 1000), record="duplicate-observation", wallet="other-wallet")
                after = self._report()
                self.assertEqual(after["summary"], before["summary"])
                self.assertEqual(self._semantic_findings(after), self._semantic_findings(before))

    def test_co_spend_and_private_change_have_distinct_authority(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._tx(2, outputs=(1000,))
        self._own(1)
        self._own(2)
        self._tx(3, [(1, 0), (2, 0)], outputs=(2600, 300))
        self._own(3, branch="change")
        mirror = self._report()
        public = analyze_snapshot(build_index(self.conn, "pf"), mirror["investigation"]["query"])
        self.assertNotIn("private_wallet_change", {row.get("rule") for row in public["edges"]})
        common = next(row for row in mirror["findings"] if row["code"] == "common_input_control")
        self.assertIn(common["authority"], {"hypothesis", "heuristic"})
        self.assertIn("assumes_no_undetected_collaboration", common["assumptions"])
        self.assertNotIn("private_wallet_change", self._codes(mirror))

    def test_received_context_is_visible_without_becoming_personal_attention(self):
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._tx(2, [(1, 0)], outputs=(1800,), extra={"vin": [{"txid": f"{1:064x}", "vout": 0, "sequence": 0xFFFFFFFD, "prevout": {"value": 2000}}]})
        self._own(2)
        report = self._report()
        rbf = next(row for row in report["findings"] if row["code"] == "explicit_rbf_signal")
        self.assertEqual(rbf["relevance"], "received_context")
        self.assertEqual(rbf["severity"], "info")
        self.assertEqual(report["summary"]["attention_count"], 0)
        self.assertTrue(report["summary"]["finding_count"])
        self.conn.execute("DELETE FROM wallet_utxos")
        unowned = self._report()
        self.assertEqual(unowned["summary"]["status"], "unavailable")
        self.assertEqual(unowned["summary"]["attention_count"], 0)
        self.assertTrue(unowned["findings"], "Context remains inspectable without inventing user ownership")

    def test_heuristic_findings_carry_exact_chain_and_network_investigation(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index
        self._clear_observations()
        for network in ("main", "regtest"):
            self._tx(1, outputs=(2000,), network=network)
            self._tx(2, [(1, 0)], outputs=(1800,), network=network, extra={"vin": [{"txid": f"{1:064x}", "vout": 0, "sequence": 0xFFFFFFFD, "prevout": {"value": 2000}}]})
        self._own(1)
        self._own(2, network="regtest")
        report = self._report()
        rbf = [row for row in report["findings"] if row["code"] == "explicit_rbf_signal"]
        self.assertEqual(len(rbf), 2)
        self.assertEqual({row["relevance"] for row in rbf}, {"own_spend", "received_context"})
        for row in rbf:
            query = row["investigation"]["query"]
            self.assertRegex(query["subject"], r"^bitcoin:(main|regtest):tx:[0-9a-f]{64}$")
            result = analyze_snapshot(build_index(self.conn, "pf"), query)
            self.assertEqual(len({node["network"] for node in result["nodes"]}), 1)
            self.assertEqual(result["snapshot_id"], row["investigation"]["snapshot_id"])

    def test_public_attribution_is_shared_but_private_labels_and_claim_text_do_not_reach_ai(self):
        from kassiber.core import chain_analysis_datasets as datasets
        from kassiber.core.chain_analysis import run_analysis
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        self.conn.commit()
        for visibility in ("public", "private"):
            manifest = {"dataset_key": visibility, "name": "Published attribution", "version": "2026-01", "chain": "bitcoin", "network": "main", "source": "Source", "license": "LicenseRef-Unverified", "attribution_method": "published_claim", "visibility": visibility}
            rows = json.dumps({"subject": f"{1:064x}", "label": f"{visibility}-sensitive-claim"}) + "\n"
            datasets.import_dataset(self.conn, "pf", manifest, io.BytesIO(rows.encode()), format="jsonl")
        desktop = self._report()
        workbench = run_analysis(self.conn, "pf", desktop["investigation"]["query"])
        self.assertEqual(desktop["investigation"]["snapshot_id"], workbench["snapshot_id"])
        self.assertTrue(any(row["category"] == "attribution" for row in desktop["findings"]))
        self.assertNotIn("private-sensitive-claim", json.dumps(desktop))
        exported = self._report(redacted=True)
        self.assertNotIn("public-sensitive-claim", json.dumps(exported))
        self.assertNotIn("private-sensitive-claim", json.dumps(exported))
        self.assertTrue(any(row["category"] == "attribution" for row in exported["findings"]))

    def test_liquid_handoffs_are_valid_canonical_workbench_queries(self):
        from kassiber.core.chain_analysis import run_analysis
        self._clear_observations()
        script = "0014" + "45" * 20
        self._tx(1, outputs=(None,), chain="liquid", scripts=[script])
        self._tx(2, outputs=(None,), chain="liquid", scripts=[script])
        self._own(1, chain="liquid")
        report = self._report()
        finding = next(row for row in report["findings"] if row["code"] == "script_reuse")
        result = run_analysis(self.conn, "pf", finding["investigation"]["query"])
        self.assertEqual({node["network"] for node in result["nodes"]}, {"liquidv1"})
        self.assertEqual(result["snapshot_id"], finding["investigation"]["snapshot_id"])
        self.assertIn(finding["id"], {row["id"] for row in result["findings"]})
        # Public script equality can span disconnected physical histories;
        # the same opaque AI handoff must retain both output observations.
        from kassiber.core.chain_analysis_ai import decode_ai_args
        redacted = self._report(redacted=True)
        projected = next(row for row in redacted["findings"] if row["code"] == "script_reuse")
        args = decode_ai_args(self.conn, "pf", projected["investigation"]["query"])
        ai_result = run_analysis(self.conn, "pf", args)
        self.assertIn(finding["id"], {row["id"] for row in ai_result["findings"]})

    def test_patterns_are_the_workbenchs_findings_including_postmix_reconvergence(self):
        from kassiber.core.chain_analysis import analyze_snapshot, build_index
        self._clear_observations()
        for number in range(1, 6):
            self._tx(number, outputs=(1100,))
        self._tx(6, [(number, 0) for number in range(1, 6)], outputs=(1000,) * 5)
        self._tx(7, [(6, 0)], outputs=(900,))
        self._tx(8, [(6, 1)], outputs=(900,))
        self._tx(9, [(7, 0), (8, 0)], outputs=(1700,))
        self._own(6)
        self._own(7)
        mirror = self._report()
        workbench = analyze_snapshot(build_index(self.conn, "pf"), mirror["investigation"]["query"])
        expected = {row["id"] for row in workbench["patterns"] if row["code"] == "postmix_reconvergence"}
        self.assertTrue(expected)
        actual = {row["id"] for row in mirror["findings"] if row["code"] == "postmix_reconvergence"}
        self.assertEqual(actual, expected)
        self.assertTrue(all(row["category"] == "pattern" for row in mirror["findings"] if row["id"] in expected))

        # Owning an origin input and an unrelated sibling output does not
        # attribute other participants' later reconverging branches to us.
        self.conn.execute("DELETE FROM wallet_utxos")
        self._own(1)
        self._own(6, 4)
        context = self._report()
        for row in context["findings"]:
            if row["code"] == "postmix_reconvergence":
                self.assertEqual(row["severity"], "info")
                self.assertEqual(row["relevance"], "nearby_context")
                self.assertEqual(row["affected_output_count"], 0)

    def test_long_reused_script_keeps_a_valid_complete_investigation(self):
        from kassiber.core.chain_analysis import run_analysis
        self._clear_observations()
        script = "51" * 600
        self._tx(1, scripts=[script])
        self._tx(2, scripts=[script])
        self._own(1)
        report = self._report()
        finding = next(row for row in report["findings"] if row["code"] == "script_reuse")
        handoff = finding["investigation"]
        self.assertEqual(handoff["query"]["mode"], "overview")
        reopened = run_analysis(self.conn, "pf", handoff["query"])
        self.assertEqual(reopened["snapshot_id"], handoff["snapshot_id"])
        self.assertIn(finding["id"], {row["id"] for row in reopened["findings"]})

    def test_snapshot_changes_when_observations_change_but_duplicates_do_not_inflate_findings(self):
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        self._tx(2, [(1, 0)], outputs=(1900,), extra={"version": 3})
        before = self._report()
        self._tx(2, [(1, 0)], outputs=(1900,), extra={"version": 3}, record="duplicate")
        after = self._report()
        self.assertEqual(before["summary"], after["summary"])
        self.assertEqual(self._semantic_findings(before), self._semantic_findings(after))
        self._tx(3, [(2, 0)], outputs=(1800,))
        changed = self._report()
        self.assertNotEqual(changed["investigation"]["snapshot_id"], before["investigation"]["snapshot_id"])

    def test_entropy_runs_the_shared_conditional_model_and_reports_omitted_work(self):
        from kassiber.core.chain_analysis import build_index
        from kassiber.core.chain_analysis.entropy import analyze_transaction_entropy
        from kassiber.core.chain_analysis.index import observer_index
        self._clear_observations()
        for number in range(1, 5):
            self._tx(number, outputs=(2000,))
            self._tx(number + 10, [(number, 0)], outputs=(1900,))
        self._own(1)
        with patch("kassiber.core.privacy_mirror.MAX_ENTROPY_TRANSACTIONS", 2):
            mirror = self._report()
        entropy = mirror["entropy"]
        self.assertEqual(entropy["eligible"], 4)
        self.assertEqual(entropy["evaluated"], 2)
        self.assertEqual(entropy["omitted"], 2)
        self.assertIn("entropy_transaction_budget", mirror["coverage"]["stopped_reasons"])
        self.assertEqual(self._checks(mirror)["conditional_entropy"]["status"], "bounded")
        public = observer_index(build_index(self.conn, "pf"), "public")
        for row in entropy["results"]:
            expected = analyze_transaction_entropy(public.transaction_facts[row["subject"]])
            self.assertEqual(row["status"], "exact")
            self.assertEqual(row["interpretation_count"], expected["interpretation_count"])
            self.assertEqual(row["entropy_bits"], expected["entropy_bits"])
            self.assertTrue(row["conditional_on_model"])
            self.assertIn("no_ownership_constraints", row["assumptions"])

    def test_entropy_keeps_missing_values_and_joint_payment_exclusions_explicit(self):
        self._clear_observations()
        self._tx(1, [(90, 0)], outputs=(900,), extra={"vin": [{"txid": f"{90:064x}", "vout": 0}]})
        self._tx(2, outputs=(2000,))
        self._tx(3, [(2, 0)], outputs=(1900,), boundary="payjoin")
        self._tx(4, outputs=(None,), chain="liquid")
        self._tx(5, [(4, 0)], outputs=(None,), chain="liquid")
        self._own(2)
        mirror = self._report()
        results = {row["subject"]: row for row in mirror["entropy"]["results"]}
        expected = {
            f"bitcoin:main:tx:{1:064x}": "unknown_amount",
            f"bitcoin:main:tx:{3:064x}": "joint_payment_or_unknown_collaboration",
            f"liquid:liquidv1:tx:{5:064x}": "bitcoin_only",
        }
        for subject, reason in expected.items():
            with self.subTest(subject=subject):
                self.assertNotEqual(results[subject]["status"], "exact")
                self.assertEqual(results[subject]["reason"], reason)
                self.assertIsNone(results[subject]["interpretation_count"])
                self.assertIsNone(results[subject]["entropy_bits"])
        self.assertEqual(self._checks(mirror)["conditional_entropy"]["status"], "partial")
        self.assertNotEqual(mirror["coverage"]["status"], "complete")

    def test_computation_and_display_limits_never_hide_incomplete_work(self):
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        for number in range(2, 5):
            self._tx(number, [(number - 1, 0)], outputs=(2100 - 100 * number,), extra={"version": 3})
        with (patch("kassiber.core.privacy_mirror.MAX_FINDINGS", 1),
              patch("kassiber.core.privacy_mirror.ENTROPY_MAX_STATES", 1)):
            limited = self._report()
        self.assertEqual(len(limited["findings"]), 1)
        self.assertGreater(limited["summary"]["finding_count"], len(limited["findings"]))
        self.assertIn("finding_display_limit", limited["coverage"]["stopped_reasons"])
        self.assertTrue(limited["coverage"]["truncated"])
        self.assertEqual(self._checks(limited)["conditional_entropy"]["status"], "partial")
        self.assertTrue(limited["entropy"]["results"])
        for row in limited["entropy"]["results"]:
            self.assertNotEqual(row["status"], "exact")
            self.assertIsNone(row["interpretation_count"])
            self.assertIsNone(row["entropy_bits"])
            self.assertTrue(row["reason"])

    def test_stale_and_conflicting_observations_reduce_coverage_not_claim_current_exposure(self):
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        self._tx(2, [(1, 0)], outputs=(1900,), extra={"version": 3, "confirmations": -1})
        stale = self._report()
        self.assertGreater(stale["coverage"]["stale_nodes"], 0)
        self.assertNotIn("unusual_transaction_version", self._codes(stale))
        self.assertNotEqual(stale["coverage"]["status"], "complete")
        self.conn.execute("DELETE FROM transactions WHERE external_id=?", (f"{2:064x}",))
        self._tx(2, [(1, 0)], outputs=(1900,), extra={"version": 3})
        self._tx(2, [(1, 0)], outputs=(1800, 100), record="contradicting-shape", extra={"version": 3})
        conflict = self._report()
        self.assertGreater(conflict["coverage"]["conflicting_nodes"], 0)
        self.assertNotIn("unusual_transaction_version", self._codes(conflict))
        self.assertNotEqual(conflict["coverage"]["status"], "complete")

    def test_real_cli_and_daemon_ai_dispatch_preserve_analysis_but_scope_identifiers(self):
        from kassiber import daemon
        self._clear_observations()
        self._tx(1, outputs=(2000,))
        self._own(1)
        self._tx(2, [(1, 0)], outputs=(1900,), extra={"version": 3})
        self.conn.commit()
        before = self.conn.total_changes
        runtime = daemon.AiToolRuntime(str(self.data_root), {}, queue.Queue(), {"scope_workspace_id": "ws", "scope_profile_id": "pf", "provider_kind": "remote"})
        with (patch.object(daemon, "_run_on_daemon_main_thread", side_effect=lambda _runtime, callback: callback(self.conn)),
              patch("socket.getaddrinfo", side_effect=AssertionError("unexpected DNS")) as dns,
              patch("socket.socket.connect", side_effect=AssertionError("unexpected socket, including loopback")) as connect):
            desktop_envelope, shutdown = daemon.handle_request(
                SimpleNamespace(conn=self.conn, runtime_config={}, data_root=str(self.data_root)),
                {"kind": MIRROR_KIND, "request_id": "desktop-mirror", "args": {}},
                daemon._OutputChannel(io.StringIO()),
            )
            self.assertFalse(shutdown)
            self.assertEqual(desktop_envelope["request_id"], "desktop-mirror")
            desktop = desktop_envelope["data"]
            call = daemon.ParsedAiToolCall("mirror", "ui_reports_privacy_mirror", {})
            result = daemon._execute_read_only_ai_tool(call, runtime)
            self.assertTrue(result["ok"], result)
            follow_up_query = next(row["investigation"]["query"] for row in result["envelope"]["data"]["findings"] if row["code"] == "unusual_transaction_version")
            follow_up = daemon._execute_read_only_ai_tool(
                daemon.ParsedAiToolCall("investigate", "ui_chain_analysis_query", follow_up_query), runtime,
            )
        self.assertTrue(result["ok"], result)
        self.assertTrue(follow_up["ok"], follow_up)
        self.assertEqual(follow_up["envelope"]["data"]["snapshot_id"], desktop["investigation"]["snapshot_id"])
        self.assertNotIn(f"{2:064x}", json.dumps(follow_up))
        ai = result["envelope"]["data"]
        cli = _run_cli(self.data_root, "reports", "privacy-mirror")["data"]
        self.assertEqual(self.conn.total_changes, before)
        dns.assert_not_called()
        connect.assert_not_called()
        for projected in (ai, cli):
            self.assertEqual(projected["summary"], desktop["summary"])
            self.assertEqual(projected["coverage"], desktop["coverage"])
            for key in ("query", "snapshot_id"):
                self.assertEqual(projected["investigation"][key], desktop["investigation"][key])
            self.assertEqual(projected["investigation"]["ai_reference_scope"], "current_process_and_book")
            self.assertEqual(projected["redaction"], "ai_export_safe")
            self.assertNotIn(f"{2:064x}", json.dumps(projected))
            query = next(row["investigation"]["query"] for row in projected["findings"] if row["code"] == "unusual_transaction_version")
            self.assertTrue(query["subject"].startswith("ca-ref:"))
        local_query = next(row["investigation"]["query"] for row in desktop["findings"] if row["code"] == "unusual_transaction_version")
        self.assertEqual(local_query["subject"], f"bitcoin:main:tx:{2:064x}")
        table = _run_cli(self.data_root, "--format", "table", "reports", "privacy-mirror", machine=False)
        self.assertIn("evidence_level", table)
        self.assertIn("unusual_transaction_version", table)
        self.assertNotIn(f"{2:064x}", table)
        self.assertNotIn("privacy_score", table)

    def test_ai_schema_and_allowlists_are_wired(self):
        tool = get_tool(MIRROR_KIND)
        self.assertIsNotNone(tool)
        self.assertEqual(tool.kind_class, "read_only")
        self.assertFalse(tool.egresses)
        self.assertEqual(tool.daemon_kind, MIRROR_KIND)
        self.assertFalse(tool.parameters["additionalProperties"])
        self.assertEqual(tool.parameters["properties"], {})
        self.assertIs(get_tool("ui_reports_privacy_mirror"), tool)
        self.assertIsNone(get_tool(PSBT_KIND))
        definition = next(item for item in responses_tool_definitions() if item["name"] == "ui_reports_privacy_mirror")
        self.assertEqual(definition["parameters"], tool.parameters)
        for path, expression in (
            ("kassiber/daemon.py", r"SUPPORTED_KINDS\s*=\s*\((?P<body>.*?)\)\n"),
            ("ui-tauri/src-tauri/src/lib.rs", r"ALLOWED_DAEMON_KINDS[^=]*=\s*&\[(?P<body>.*?)\];"),
            ("ui-tauri/vite.config.ts", r"ALLOWED_BRIDGE_KINDS\s*=\s*new Set\(\[(?P<body>.*?)\]\);"),
        ):
            match = re.search(expression, (ROOT / path).read_text(), re.DOTALL)
            self.assertIsNotNone(match, path)
            allowed = set(re.findall(r'"([^"]+)"', match.group("body")))
            self.assertIn(MIRROR_KIND, allowed)
            self.assertIn(PSBT_KIND, allowed)


if __name__ == "__main__":
    unittest.main()
