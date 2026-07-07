import json
import select
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

from kassiber.db import open_db
from kassiber.core.ui_snapshot import build_wallet_utxos_snapshot_for_ai
from kassiber.importers import load_wasabi_bundle


ROOT = Path(__file__).resolve().parent.parent
IN_TXID = "aa" * 32
COINJOIN_TXID = "bb" * 32
SPEND_TXID = "cc" * 32
WASABI_SECRET_MARKERS = (
    "must-drop",
    "EncryptedSecret",
    "ExtPubKey",
    "ChainCode",
    "publicKey",
    "m/84'/0'/0'/0/7",
    "m/84'/0'/0'/1/8",
    "m/84'/0'/0'",
    "m/86'/0'/0'",
)


def _run_cli(data_root: Path, *args: str) -> dict:
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
    if result.returncode != 0:
        raise AssertionError(
            f"CLI failed ({result.returncode}) for {' '.join(args)}\n"
            f"stdout:\n{result.stdout}\nstderr:\n{result.stderr}"
        )
    return json.loads(result.stdout)


def _start_daemon(data_root: Path) -> subprocess.Popen:
    return subprocess.Popen(
        [
            sys.executable,
            "-m",
            "kassiber",
            "--data-root",
            str(data_root),
            "daemon",
        ],
        cwd=ROOT,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )


def _write_daemon(proc: subprocess.Popen, payload: dict) -> None:
    assert proc.stdin is not None
    proc.stdin.write(json.dumps(payload) + "\n")
    proc.stdin.flush()


def _read_daemon(proc: subprocess.Popen, *, timeout: float = 5.0) -> dict:
    assert proc.stdout is not None
    ready, _, _ = select.select([proc.stdout.fileno()], [], [], timeout)
    if not ready:
        raise AssertionError("daemon did not emit a response")
    return json.loads(proc.stdout.readline())


def _read_daemon_until(proc: subprocess.Popen, kind: str, *, timeout: float = 5.0) -> dict:
    deadline = time.monotonic() + timeout
    seen: list[str | None] = []
    while True:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise AssertionError(f"daemon did not emit {kind!r}; saw {seen!r}")
        envelope = _read_daemon(proc, timeout=remaining)
        seen.append(envelope.get("kind"))
        if envelope.get("kind") == kind:
            return envelope


def _stop_daemon(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        proc.terminate()
        try:
            proc.wait(timeout=5)
        except subprocess.TimeoutExpired:
            proc.kill()
            proc.wait(timeout=5)
    for stream in (proc.stdin, proc.stdout, proc.stderr):
        if stream is not None:
            stream.close()


def _wasabi_bundle_payload() -> dict:
    return {
        "gethistory": {
            "result": [
                {
                    "datetime": "2026-01-02T12:00:00Z",
                    "height": 800_000,
                    "amount": "2500000",
                    "label": "exchange withdrawal",
                    "tx": IN_TXID,
                    "islikelycoinjoin": False,
                },
                {
                    "datetime": "2026-01-03T12:00:00Z",
                    "height": 800_050,
                    "amount": "0",
                    "label": "wallet coinjoin",
                    "tx": COINJOIN_TXID,
                    "islikelycoinjoin": True,
                },
                {
                    "datetime": "2026-01-04T12:00:00Z",
                    "height": 800_100,
                    "amount": "-1000000",
                    "label": "post-mix spend",
                    "tx": SPEND_TXID,
                    "islikelycoinjoin": False,
                },
            ]
        },
        "listcoins": {
            "result": [
                {
                    "txid": IN_TXID,
                    "index": 0,
                    "amount": "2500000",
                    "confirmed": True,
                    "confirmations": 42,
                    "height": 800_000,
                    "address": "bc1qsalarycoin",
                    "label": "exchange withdrawal",
                    "keyPath": "m/84'/0'/0'/0/7",
                    "anonymityScore": 3,
                    "spentBy": COINJOIN_TXID,
                    "excludedFromCoinjoin": False,
                    "keyState": "Used",
                    "anonHistory": [
                        {"score": 1, "publicKey": "must-drop-from-history"}
                    ],
                },
                {
                    "txid": COINJOIN_TXID,
                    "index": 2,
                    "amount": "1500000",
                    "confirmed": True,
                    "confirmations": 10,
                    "height": 800_050,
                    "address": "bc1qmixedcoin",
                    "label": "private coin",
                    "keyPath": "m/84'/0'/0'/1/8",
                    "anonymityScore": 50,
                    "excludedFromCoinjoin": True,
                    "keyState": "Used",
                },
            ]
        },
        "getwalletinfo": {
            "result": {
                "walletName": "Wasabi Demo",
                "anonScoreTarget": 50,
                "isWatchOnly": True,
                "isHardwareWallet": False,
                "isAutoCoinjoin": False,
                "accounts": [
                    {
                        "name": "SegWit",
                        "keyPath": "m/84'/0'/0'",
                        "publicKey": "must-drop",
                    }
                ],
            }
        },
        "listkeys": {
            "result": [
                {
                    "keyState": "Used",
                    "fullKeyPath": "m/84'/0'/0'/0/7",
                    "publicKey": "must-drop",
                    "address": "bc1qsalarycoin",
                },
                {"keyState": "Clean", "publicKey": "must-drop-2"},
            ]
        },
        "listpaymentsincoinjoin": {
            "result": [
                {
                    "id": "payment-1",
                    "roundId": "round-1",
                    "amount": "1000",
                    "address": "bc1qexternaldestination",
                    "state": [
                        {
                            "status": "running",
                            "roundId": "round-1",
                            "paymentId": "payment-1",
                            "destination": "must-drop",
                        }
                    ],
                }
            ]
        },
        "wallet_json": {
            "AnonScoreTarget": 60,
            "AutoCoinJoin": True,
            "RedCoinIsolation": True,
            "MinGapLimit": 21,
            "SilentPaymentAccountKeyPath": "m/86'/0'/0'",
            "EncryptedSecret": "must-drop",
            "ChainCode": "must-drop",
            "ExtPubKey": "must-drop",
        },
    }


def _write_bundle(path: Path) -> None:
    path.write_text(json.dumps(_wasabi_bundle_payload()), encoding="utf-8")


def _assert_wasabi_secret_markers_absent(testcase: unittest.TestCase, text: str) -> None:
    for secret in WASABI_SECRET_MARKERS:
        testcase.assertNotIn(secret, text)


class WasabiBundleParserTest(unittest.TestCase):
    def test_parser_normalizes_activity_inventory_and_redacts_wallet_material(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-wasabi-parser-") as tmp:
            bundle_path = Path(tmp) / "wasabi.json"
            _write_bundle(bundle_path)

            bundle = load_wasabi_bundle(str(bundle_path))

        self.assertEqual(len(bundle["records"]), 3)
        coinjoin = bundle["records"][1]
        self.assertEqual(coinjoin["txid"], COINJOIN_TXID)
        self.assertEqual(coinjoin["direction"], "outbound")
        self.assertEqual(coinjoin["kind"], "coinjoin")
        self.assertEqual(coinjoin["amount"], 0)
        self.assertTrue(json.loads(coinjoin["raw_json"])["islikelycoinjoin"])

        self.assertEqual(len(bundle["coins"]), 2)
        spent_coin = bundle["coins"][0]
        self.assertEqual(spent_coin["branch_index"], 0)
        self.assertEqual(spent_coin["address_index"], 7)
        self.assertEqual(spent_coin["spent_by"], COINJOIN_TXID)
        self.assertTrue(spent_coin["spent"])
        self.assertEqual(spent_coin["anonymity_score"], 3)
        mixed_coin = bundle["coins"][1]
        self.assertEqual(mixed_coin["branch_index"], 1)
        self.assertEqual(mixed_coin["address_index"], 8)
        self.assertTrue(mixed_coin["excluded_from_coinjoin"])

        metadata = bundle["metadata"]
        self.assertEqual(metadata["walletName"], "Wasabi Demo")
        self.assertEqual(metadata["anonScoreTarget"], 50)
        self.assertTrue(metadata["isWatchOnly"])
        self.assertTrue(metadata["redCoinIsolation"])
        self.assertEqual(metadata["minGapLimit"], 21)
        self.assertEqual(metadata["silentPaymentAccountPathHint"], "86'/0'/*'")
        self.assertEqual(metadata["keyStateCounts"], {"Used": 1, "Clean": 1})
        self.assertEqual(metadata["paymentsInCoinJoin"][0]["round_id"], "round-1")

        serialized = json.dumps(bundle, sort_keys=True, default=str)
        for secret in (
            "must-drop",
            "EncryptedSecret",
            "ChainCode",
            "ExtPubKey",
            "publicKey",
            "fullKeyPath",
            "m/84'/0'/0'/0/7",
        ):
            self.assertNotIn(secret, serialized)


class WasabiImportFlowTest(unittest.TestCase):
    def test_cli_daemon_import_inventory_redaction_and_readiness_warnings(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-wasabi-flow-") as tmp:
            data_root = Path(tmp) / "state"
            bundle_path = Path(tmp) / "wasabi.json"
            _write_bundle(bundle_path)

            _run_cli(data_root, "init")
            _run_cli(data_root, "workspaces", "create", "Demo")
            _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
            _run_cli(
                data_root,
                "wallets",
                "create",
                "--label",
                "Wasabi",
                "--kind",
                "wasabi",
                "--source-file",
                str(bundle_path),
            )
            import_payload = _run_cli(
                data_root,
                "wallets",
                "import-wasabi",
                "--wallet",
                "Wasabi",
                "--file",
                str(bundle_path),
            )["data"]
            self.assertEqual(import_payload["wasabi_transactions"], 3)
            self.assertEqual(import_payload["wasabi_coins_observed"], 2)
            self.assertEqual(import_payload["wasabi_coins_active"], 1)

            conn = open_db(data_root)
            try:
                wallet = conn.execute("SELECT * FROM wallets WHERE label = 'Wasabi'").fetchone()
                config = json.loads(wallet["config_json"])
                self.assertEqual(config["source_format"], "wasabi_bundle")
                config_text = json.dumps(config, sort_keys=True)
                _assert_wasabi_secret_markers_absent(self, config_text)
                coin_rows = conn.execute(
                    "SELECT * FROM wallet_utxos WHERE wallet_id = ? ORDER BY txid, vout",
                    (wallet["id"],),
                ).fetchall()
                self.assertEqual(len(coin_rows), 2)
                self.assertEqual(coin_rows[0]["spent_by"], COINJOIN_TXID)
                self.assertIsNotNone(coin_rows[0]["spent_at"])
                self.assertEqual(coin_rows[1]["anonymity_score"], 50)
                self.assertEqual(coin_rows[1]["excluded_from_coinjoin"], 1)
                self.assertNotIn("must-drop", coin_rows[0]["raw_json"])

                ai_snapshot = build_wallet_utxos_snapshot_for_ai(
                    conn,
                    None,
                    {"wallet": "Wasabi"},
                )
                self.assertEqual(ai_snapshot["support"]["status"], "supported")
                ai_text = json.dumps(ai_snapshot, sort_keys=True, default=str)
                for redacted in (
                    "bc1qsalarycoin",
                    "bc1qmixedcoin",
                    "branch_index",
                    "address_index",
                    "anon_history",
                    "must-drop",
                ):
                    self.assertNotIn(redacted, ai_text)

                coinjoin_tx = conn.execute(
                    "SELECT * FROM transactions WHERE external_id = ?",
                    (COINJOIN_TXID,),
                ).fetchone()
                self.assertEqual(coinjoin_tx["review_status"], "review")
            finally:
                conn.close()

            tax_payload = _run_cli(data_root, "journals", "process")["data"]
            self.assertGreaterEqual(tax_payload["quarantined"], 1)
            conn = open_db(data_root)
            try:
                quarantine_reasons = {
                    row["reason"]
                    for row in conn.execute("SELECT reason FROM journal_quarantines")
                }
            finally:
                conn.close()
            self.assertIn("privacy_hop_unresolved", quarantine_reasons)

            report_payload = _run_cli(
                data_root,
                "reports",
                "source-funds",
                "--target-transaction",
                COINJOIN_TXID,
                "--target-amount",
                "0.00000001",
            )["data"]
            finding_codes = {finding["code"] for finding in report_payload["findings"]}
            self.assertIn("privacy_hop_unresolved", finding_codes)

            daemon = _start_daemon(data_root)
            try:
                _write_daemon(
                    daemon,
                    {
                        "kind": "ui.connections.sources",
                        "request_id": "sources",
                        "args": {},
                    },
                )
                sources = _read_daemon_until(daemon, "ui.connections.sources")
                self.assertIn("wasabi_bundle", sources["data"]["source_formats"])
                self.assertTrue(
                    any(row["kind"] == "wasabi" for row in sources["data"]["wallet_kinds"])
                )

                _write_daemon(
                    daemon,
                    {
                        "kind": "ui.wallets.import_file",
                        "request_id": "import",
                        "args": {
                            "wallet": "Wasabi",
                            "source_file": str(bundle_path),
                            "source_format": "wasabi_bundle",
                        },
                    },
                )
                daemon_import = _read_daemon_until(daemon, "ui.wallets.import_file")
                self.assertEqual(daemon_import["kind"], "ui.wallets.import_file")
                self.assertEqual(daemon_import["data"]["wasabi_transactions"], 3)
            finally:
                _stop_daemon(daemon)

            daemon = _start_daemon(data_root)
            try:
                _write_daemon(
                    daemon,
                    {
                        "kind": "ui.wallets.import_file",
                        "request_id": "inline-import",
                        "args": {
                            "wallet": "Wasabi",
                            "source_format": "wasabi_bundle",
                            "source_bundle": _wasabi_bundle_payload(),
                        },
                    },
                )
                inline_import = _read_daemon_until(
                    daemon,
                    "ui.wallets.import_file",
                )
                self.assertEqual(inline_import["kind"], "ui.wallets.import_file")
                self.assertEqual(inline_import["data"]["wasabi_transactions"], 3)
                self.assertEqual(inline_import["data"]["wasabi_coins_observed"], 2)
                self.assertEqual(inline_import["data"]["input_format"], "wasabi_bundle")
            finally:
                _stop_daemon(daemon)

            conn = open_db(data_root)
            try:
                wallet = conn.execute("SELECT * FROM wallets WHERE label = 'Wasabi'").fetchone()
                config_text = json.dumps(json.loads(wallet["config_json"]), sort_keys=True)
                transaction_raw_text = json.dumps(
                    [
                        row["raw_json"]
                        for row in conn.execute(
                            "SELECT raw_json FROM transactions WHERE wallet_id = ? ORDER BY id",
                            (wallet["id"],),
                        )
                    ],
                    sort_keys=True,
                )
                utxo_raw_text = json.dumps(
                    [
                        row["raw_json"]
                        for row in conn.execute(
                            "SELECT raw_json FROM wallet_utxos WHERE wallet_id = ? ORDER BY id",
                            (wallet["id"],),
                        )
                    ],
                    sort_keys=True,
                )
                ai_snapshot = build_wallet_utxos_snapshot_for_ai(
                    conn,
                    None,
                    {"wallet": "Wasabi"},
                )
                ai_text = json.dumps(ai_snapshot, sort_keys=True, default=str)
            finally:
                conn.close()
            for text in (config_text, transaction_raw_text, utxo_raw_text, ai_text):
                _assert_wasabi_secret_markers_absent(self, text)

    def test_empty_wasabi_coin_snapshot_marks_previous_inventory_spent(self):
        with tempfile.TemporaryDirectory(prefix="kassiber-wasabi-empty-coins-") as tmp:
            data_root = Path(tmp) / "state"
            bundle_path = Path(tmp) / "wasabi.json"
            _write_bundle(bundle_path)

            _run_cli(data_root, "init")
            _run_cli(data_root, "workspaces", "create", "Demo")
            _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
            _run_cli(
                data_root,
                "wallets",
                "create",
                "--label",
                "Wasabi",
                "--kind",
                "wasabi",
                "--source-file",
                str(bundle_path),
            )
            _run_cli(
                data_root,
                "wallets",
                "import-wasabi",
                "--wallet",
                "Wasabi",
                "--file",
                str(bundle_path),
            )

            empty_coin_payload = _wasabi_bundle_payload()
            empty_coin_payload["listcoins"] = {"result": []}
            empty_coin_payload["listunspentcoins"] = {"result": []}
            bundle_path.write_text(json.dumps(empty_coin_payload), encoding="utf-8")

            import_payload = _run_cli(
                data_root,
                "wallets",
                "import-wasabi",
                "--wallet",
                "Wasabi",
                "--file",
                str(bundle_path),
            )["data"]
            self.assertEqual(import_payload["wasabi_coins_observed"], 0)
            self.assertEqual(import_payload["wasabi_coins_active"], 0)

            conn = open_db(data_root)
            try:
                active_count = conn.execute(
                    "SELECT COUNT(*) AS count FROM wallet_utxos WHERE spent_at IS NULL"
                ).fetchone()["count"]
                self.assertEqual(active_count, 0)
            finally:
                conn.close()

    def test_wasabi_reimport_can_retract_import_authored_coinjoin_evidence(self):
        txid = "dd" * 32
        with tempfile.TemporaryDirectory(prefix="kassiber-wasabi-retract-") as tmp:
            data_root = Path(tmp) / "state"
            bundle_path = Path(tmp) / "wasabi.json"
            payload = {
                "gethistory": {
                    "result": [
                        {
                            "datetime": "2026-02-01T10:00:00Z",
                            "height": 810_000,
                            "amount": "-1000000",
                            "label": "heuristic spend",
                            "tx": txid,
                            "islikelycoinjoin": True,
                        }
                    ]
                }
            }
            bundle_path.write_text(json.dumps(payload), encoding="utf-8")

            _run_cli(data_root, "init")
            _run_cli(data_root, "workspaces", "create", "Demo")
            _run_cli(data_root, "profiles", "create", "Main", "--fiat-currency", "EUR")
            _run_cli(
                data_root,
                "wallets",
                "create",
                "--label",
                "Wasabi",
                "--kind",
                "wasabi",
                "--source-file",
                str(bundle_path),
            )
            _run_cli(
                data_root,
                "wallets",
                "import-wasabi",
                "--wallet",
                "Wasabi",
                "--file",
                str(bundle_path),
            )

            conn = open_db(data_root)
            try:
                first = conn.execute(
                    "SELECT id, kind, privacy_boundary, review_status FROM transactions WHERE external_id = ?",
                    (txid,),
                ).fetchone()
                self.assertEqual(first["kind"], "coinjoin")
                self.assertEqual(first["privacy_boundary"], "coinjoin")
                self.assertEqual(first["review_status"], "review")
            finally:
                conn.close()

            payload["gethistory"]["result"][0]["islikelycoinjoin"] = False
            bundle_path.write_text(json.dumps(payload), encoding="utf-8")
            _run_cli(
                data_root,
                "wallets",
                "import-wasabi",
                "--wallet",
                "Wasabi",
                "--file",
                str(bundle_path),
            )

            conn = open_db(data_root)
            try:
                updated = conn.execute(
                    "SELECT id, kind, privacy_boundary, review_status, raw_json FROM transactions WHERE external_id = ?",
                    (txid,),
                ).fetchone()
                self.assertEqual(updated["kind"], "withdrawal")
                self.assertIsNone(updated["privacy_boundary"])
                self.assertIsNone(updated["review_status"])
                self.assertFalse(json.loads(updated["raw_json"])["islikelycoinjoin"])
                stale_tags = conn.execute(
                    """
                    SELECT COUNT(*) AS count
                    FROM transaction_tags tt
                    JOIN tags t ON t.id = tt.tag_id
                    WHERE tt.transaction_id = ?
                      AND t.code IN ('coinjoin', 'privacy-hop-review')
                    """,
                    (updated["id"],),
                ).fetchone()["count"]
                self.assertEqual(stale_tags, 0)
            finally:
                conn.close()


if __name__ == "__main__":
    unittest.main()
