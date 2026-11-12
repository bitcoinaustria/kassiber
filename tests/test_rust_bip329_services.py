"""BIP329 boundary parity, including bytes written outside the response envelope."""
import json
import os
from pathlib import Path
import shutil
import unittest

from kassiber.db import open_db
from tests import cli_runner
from tests import test_rust_transaction_services as services

ROOT = Path(__file__).resolve().parent.parent
RUST_BIN = os.environ.get("KASSIBER_RUST_BIN")


@unittest.skipUnless(RUST_BIN, "set KASSIBER_RUST_BIN to the test-harness binary")
class RustBip329ServicesTest(unittest.TestCase):
    setUp = services.RustTransactionServicesTest.setUp
    cli = services.RustTransactionServicesTest.cli
    ok = services.RustTransactionServicesTest.ok
    same_read = services.RustTransactionServicesTest.same_read

    def seed(self):
        conn = open_db(str(self.root))
        conn.execute("UPDATE transactions SET external_id=id,note='Quote \"€\"\\nline',counterparty=' Merchant ',review_status='completed',fiat_currency='EUR',fiat_rate_exact='123.00000001',fiat_value=1.0,amount=123000,fee=1000")
        conn.commit()
        conn.close()
        path = self.base / "labels.jsonl"
        path.write_text("\n" + "\n".join(json.dumps(row, ensure_ascii=True) for row in (
            {"type": "tx", "ref": "in", "label": "old", "origin": "source", "z": 1e-7, "\ud800": {"n": 1}, "kassiber": {"kept": True}},
            {"type": "tx", "ref": "in", "label": "Final €\n\"quote\"", "origin": None, "a": {"u": "\ud800"}, "kassiber": "opaque"},
            {"type": "addr", "ref": "BC1QSYNTHETIC", "label": "address"},
            {"type": "output", "ref": "out:0", "spendable": False, "label": "change"},
            {"type": "output", "ref": "out:0", "spendable": None, "label": None},
            {"type": "spscan", "ref": "spscan-private-metadata", "extra": [True, None, 2**90, -0.0]},
        )) + "\n", encoding="utf-8")
        return path

    def export_bytes(self, root, path, mode, reference, wallet=None):
        args = ["--machine", "--data-root", str(root), "metadata", "bip329", "export", "--file", str(path), "--mode", mode]
        if wallet is not None:
            args.extend(("--wallet", wallet))
        payload = self.ok(cli_runner.run(args, reference=reference, env=self.env, capture_output=True, cwd=ROOT, timeout=120))
        return payload, path.read_bytes()

    def test_four_leaves_match_python_and_all_export_modes_write_identical_bytes(self):
        path = self.seed()
        self.same_read("metadata", "bip329", "preview", "--file", str(path))
        self.ok(self.cli("metadata", "bip329", "preview", "--file", str(path)))
        reference_root = self.base / "reference" / "data"
        shutil.copytree(self.root, reference_root)
        candidate = self.ok(self.cli("metadata", "bip329", "import", "--file", str(path)))
        reference = self.ok(cli_runner.run(["--machine", "--data-root", str(reference_root), "metadata", "bip329", "import", "--file", str(path)], reference=True, env=self.env, capture_output=True, cwd=ROOT, timeout=120))
        self.assertEqual(candidate, reference)
        conn = open_db(str(self.root))
        conn.execute("UPDATE bip329_labels SET created_at='2025-01-01T00:00:00Z'")
        conn.commit()
        conn.close()
        self.same_read("metadata", "bip329", "list", "--limit", "2")
        page = json.loads(self.cli("metadata", "bip329", "list", "--limit", "2").stdout)
        self.assertTrue(page["has_more"])
        self.same_read("metadata", "bip329", "list", "--limit", "2", "--cursor", page["next_cursor"])
        self.ok(self.cli("metadata", "bip329", "list", "--limit", "2", "--cursor", page["next_cursor"]))
        for mode in ("stored", "synthesized", "all"):
            for wallet in (None, "Cold"):
                output = self.base / "labels-export.jsonl"
                candidate, actual = self.export_bytes(self.root, output, mode, False, wallet)
                reference, expected = self.export_bytes(self.root, output, mode, True, wallet)
                self.assertEqual(candidate, reference)
                self.assertEqual(actual, expected)
        for flags in (("--limit", "0"), ("--limit", "1001"), ("--cursor", "broken")):
            self.same_read("metadata", "bip329", "list", *flags)
        self.same_read("metadata", "bip329", "export", "--file", str(output), "--wallet", "absent")

        def snapshot(root):
            conn = open_db(str(root))
            try:
                return [list(conn.execute(query)) for query in (
                    "SELECT record_type,ref,label,origin,spendable,data_json FROM bip329_labels ORDER BY record_type,ref",
                    "SELECT field,before_value,after_value,diff_json FROM transaction_edit_fields ORDER BY rowid",
                    "SELECT source,reason,journal_input_version,journal_input_version_after FROM transaction_edit_events ORDER BY rowid",
                    "SELECT journal_input_version FROM profiles",
                )]
            finally:
                conn.close()
        self.assertEqual(snapshot(self.root), snapshot(reference_root))

    def test_descriptor_wallets_match_through_the_derived_ownership_index(self):
        """Profiles with descriptor wallets match labels through
        `build_owned_index` with derivation (docs/reference/rust-native-wallet-gaps.md,
        S10): derived receive and change addresses, an address two wallets own,
        stored outputs, and inputs of a stored transaction graph."""
        from kassiber.wallet_descriptors import derive_descriptor_targets, load_descriptor_plan
        from tests.test_rust_wallet_discovery import _config

        config = _config("wpkh-pair", 5)
        targets = list(derive_descriptor_targets(load_descriptor_plan(config), start=0, end=4))
        receive = [t for t in targets if t.branch_index == 0]
        change = [t for t in targets if t.branch_index == 1]
        other = _config("tr", 3)
        conn = open_db(str(self.root))
        workspace, profile = conn.execute("SELECT workspace_id,profile_id FROM wallets WHERE label='Cold'").fetchone()
        for wallet_id, label, wallet_config in (("hot", "Hot", config), ("twin", "Twin", config), ("tap", "Tap", other)):
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) "
                         "VALUES(?,?,?,?,'descriptor',?,'2025-01-01T00:00:00Z')",
                         (wallet_id, workspace, profile, label, json.dumps(wallet_config)))
        conn.execute("UPDATE wallets SET config_json=? WHERE id='twin'",
                     (json.dumps({**config, "descriptor": config["descriptor"], "label": "twin"}),))
        spent = "ab" * 32
        graph = {"txid": "cd" * 32, "vin": [{"txid": spent, "vout": 1, "prevout": {
            "scriptpubkey": change[0].script_pubkey, "value": 5000}}], "vout": [
            {"scriptpubkey": receive[2].script_pubkey, "value": 4000}]}
        conn.execute("""INSERT INTO transactions(id,workspace_id,profile_id,wallet_id,fingerprint,external_id,asset,direction,amount,fee,occurred_at,kind,raw_json,created_at)
            VALUES('graph',?,?,'hot','graph',?,'BTC','outbound',4000000,1000000,'2025-01-02T00:00:00Z','withdrawal',?,'2025-01-02T00:00:00Z')""",
                     (workspace, profile, "cd" * 32, json.dumps(graph)))
        conn.commit()
        conn.close()
        path = self.base / "derived.jsonl"
        path.write_text("\n".join(json.dumps(row) for row in (
            {"type": "addr", "ref": receive[3].address, "label": "deep receive"},
            {"type": "addr", "ref": change[1].address.upper(), "label": "change"},
            {"type": "addr", "ref": "bc1qsynthetic", "label": "listed"},
            {"type": "addr", "ref": "bc1q" + "q" * 38, "label": "foreign"},
            {"type": "output", "ref": f"{'cd' * 32}:0", "label": "graph output"},
            {"type": "input", "ref": f"{'cd' * 32}:0", "label": "graph input"},
            {"type": "input", "ref": f"{'cd' * 32}:7", "label": "missing input"},
            {"type": "input", "ref": f"{'CD' * 32}:0", "label": "case"},
        )) + "\n", encoding="utf-8")
        self.same_read("metadata", "bip329", "preview", "--file", str(path))
        reference_root = self.base / "reference" / "data"
        shutil.copytree(self.root, reference_root)
        candidate = self.ok(self.cli("metadata", "bip329", "import", "--file", str(path), rust=True))
        reference = self.ok(cli_runner.run(["--machine", "--data-root", str(reference_root), "metadata", "bip329", "import",
                                            "--file", str(path)], reference=True, env=self.env, capture_output=True,
                                           cwd=ROOT, timeout=120))
        self.assertEqual(candidate, reference)
        for wallet in ("Hot", "Twin", "Cold", "Tap"):
            output = self.base / "scoped.jsonl"
            rust_payload, rust_bytes = self.export_bytes(self.root, output, "stored", False, wallet)
            python_payload, python_bytes = self.export_bytes(reference_root, output, "stored", True, wallet)
            self.assertEqual(rust_payload, python_payload, wallet)
            self.assertEqual(rust_bytes, python_bytes, wallet)
        preview = json.loads(self.cli("metadata", "bip329", "preview", "--file", str(path), rust=True).stdout)["data"]
        self.assertGreater(preview["counts"]["exact"], 0, preview["counts"])
        self.assertGreater(preview["counts"]["ambiguous"], 0, preview["counts"])

    def test_malformed_lines_and_normalization_errors_match_reference_bytes(self):
        path = self.base / "invalid.jsonl"
        for body in ('\r\n{"type":"tx","ref":"valid"}\r\n{broken\r\n', '\ufeff{}\n', '[]\n', '{"type":"unknown","ref":"value"}\n', '{"type":"tx","ref":" "}\n', '{"type":"output","ref":"a:0","spendable":1}\n', '{"type":"tx","ref":"a","spendable":true}\n', '{"type":"tx","ref":1,"label":false}\nnull\n'):
            path.write_bytes(body.encode("utf-8"))
            for command in ("preview", "import"):
                self.same_read("metadata", "bip329", command, "--file", str(path))
                result = self.cli("metadata", "bip329", command, "--file", str(path))
                self.assertNotEqual(result.returncode, 0)
        path.write_bytes(b"\xff\n")
        self.same_read("metadata", "bip329", "preview", "--file", str(path))
        self.same_read("metadata", "bip329", "preview", "--file", str(self.base / "missing.jsonl"))
        conn = open_db(str(self.root))
        self.assertEqual(conn.execute("SELECT COUNT(*) FROM bip329_labels").fetchone()[0], 0)
        conn.close()

    def test_daemon_preview_import_export_and_errors_use_shared_service(self):
        path = self.seed()
        reference_root = self.base / "daemon-reference" / "data"
        shutil.copytree(self.root, reference_root)
        requests = [
            {"kind": "ui.metadata.bip329.preview", "args": {"file": str(path), "ignored": True}},
            {"kind": "ui.metadata.bip329.import", "args": {"file": str(path)}},
            {"kind": "ui.metadata.bip329.preview", "args": {"file": str(path)}},
            {"kind": "ui.metadata.bip329.preview", "args": {}},
            {"kind": "ui.metadata.bip329.import", "args": {"file": str(path), "apply_ambiguous": "bad"}},
            {"kind": "ui.metadata.bip329.export", "args": {"mode": "bad"}},
            {"kind": "ui.metadata.bip329.export", "args": {"wallets": []}},
            {"kind": "ui.metadata.bip329.export", "args": {"wallets": [3]}},
            {"kind": "ui.metadata.bip329.export", "args": {"mode": "all", "wallets": ["Cold", "Cold"]}},
            {"kind": "daemon.shutdown"},
        ]
        wire = "".join(json.dumps({"request_id": str(i), **request}) + "\n" for i, request in enumerate(requests)).encode()
        def daemon(root, reference):
            result = cli_runner.run(["--machine", "--data-root", str(root), "daemon"], reference=reference, env={**self.env, "KASSIBER_HYBRID_RUST_DAEMON": "1"}, input=wire, capture_output=True, cwd=ROOT, timeout=120)
            self.assertEqual((result.returncode, result.stderr), (0, b""))
            return result.stdout.splitlines()[1:]
        actual, expected = daemon(self.root, False), daemon(reference_root, True)
        for i, (left, right) in enumerate(zip(actual, expected, strict=True)):
            if i == 8:
                left, right = json.loads(left), json.loads(right)
                actual_file, expected_file = Path(left["data"]["file"]), Path(right["data"]["file"])
                self.assertEqual(actual_file.read_bytes(), expected_file.read_bytes())
                for data in (left["data"], right["data"]):
                    data.pop("file")
                    data.pop("filename")
                self.assertEqual(left, right)
            else:
                self.assertEqual(left, right)
        conn = open_db(str(self.root))
        self.assertEqual([tuple(row) for row in conn.execute("SELECT DISTINCT source FROM transaction_edit_events")], [("gui",)])
        conn.close()
