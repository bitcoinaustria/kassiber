from __future__ import annotations

import json
import tempfile
import unittest
import unittest.mock as mock
from pathlib import Path

from kassiber.ai import create_db_ai_provider
from kassiber.core import accounts as core_accounts
from kassiber.core import attachments as core_attachments
from kassiber.core import document_import
from kassiber.core import wallets as core_wallets
from kassiber.core import imports as core_imports
from kassiber.core.repo import invalidate_journals, resolve_scope
from kassiber.db import open_db
from kassiber.errors import AppError
from kassiber.time_utils import now_iso


class FakeVisionClient:
    def __init__(self, *, models=None, content=None):
        self._models = models or [{"id": "glm-ocr"}]
        self._content = content or json.dumps(
            {
                "rows": [
                    {
                        "occurred_at": "2026-01-02",
                        "direction": "inbound",
                        "asset": "BTC",
                        "amount_btc": "0.01000000",
                        "fee_btc": "0",
                        "fiat_currency": "EUR",
                        "fiat_value": "500.00",
                        "counterparty": "OTC Desk",
                        "description": "Receipt row",
                        "confidence": 0.94,
                        "cell_confidences": {
                            "occurred_at": 0.96,
                            "direction": 0.91,
                            "amount_btc": 0.95,
                        },
                        "source_region": {
                            "page": 1,
                            "x": 0.1,
                            "y": 0.2,
                            "width": 0.7,
                            "height": 0.1,
                            "unit": "relative",
                        },
                        "evidence_text": "02.01.2026 BTC 0.01000000 EUR 500 OTC Desk",
                    },
                    {
                        "occurred_at": "2026-01-03",
                        "direction": None,
                        "amount_btc": "0.02000000",
                        "confidence": 0.6,
                        "evidence_text": "unclear handwritten row",
                    },
                ]
            }
        )
        self.chat_requests = []

    def list_models(self, *, strict=False):
        self.strict = strict
        return self._models

    def chat(self, **kwargs):
        self.chat_requests.append(kwargs)
        return {"role": "assistant", "content": self._content, "finish_reason": "stop"}


def _book(conn):
    workspace = core_accounts.create_workspace(conn, "Main")
    profile = core_accounts.create_profile(
        conn,
        workspace["id"],
        "Default",
        "EUR",
        "FIFO",
        "generic",
        365,
    )
    wallet = core_wallets.create_wallet(conn, workspace["id"], profile["id"], "Desk", "custom")
    return workspace, profile, wallet


def _hooks():
    def resolve_transaction(conn, profile_id, tx_ref):
        row = conn.execute(
            """
            SELECT * FROM transactions
            WHERE profile_id = ? AND (id = ? OR external_id = ?)
            LIMIT 1
            """,
            (profile_id, tx_ref, tx_ref),
        ).fetchone()
        if row is None:
            raise AppError(f"Transaction '{tx_ref}' not found", code="not_found")
        return row

    return document_import.DocumentImportHooks(
        import_hooks=core_imports.ImportCoordinatorHooks(
            ensure_tag_row=lambda *_args: (_args[3], True),
            invalidate_journals=invalidate_journals,
        ),
        attachment_hooks=core_attachments.AttachmentHooks(
            resolve_scope=resolve_scope,
            resolve_transaction=resolve_transaction,
            now_iso=now_iso,
        ),
    )


class DocumentImportTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.data_root = self.root / "data"
        self.conn = open_db(self.data_root)
        self.source = self.root / "receipt.png"
        self.source.write_bytes(b"not-a-real-png-but-good-enough-for-base64")

    def tearDown(self):
        self.conn.close()
        self.tmp.cleanup()

    def test_preview_requires_local_loopback_provider(self):
        create_db_ai_provider(
            self.conn,
            "lan",
            "http://192.168.1.20:11434/v1",
            kind="remote",
            acknowledged=True,
        )

        with self.assertRaises(AppError) as raised:
            document_import.preview_document_import(
                self.conn,
                source_file=str(self.source),
                provider_name="lan",
                client_factory=lambda _provider: self.fail("client should not be created"),
            )

        self.assertEqual(raised.exception.code, "document_import_local_ai_required")

        with self.assertRaises(AppError) as off_device:
            document_import._validate_local_provider(
                {
                    "name": "mislabelled-lan",
                    "kind": "local",
                    "base_url": "http://192.168.1.20:11434/v1",
                }
            )
        self.assertEqual(off_device.exception.code, "document_import_remote_ai_disabled")

    def test_preview_rejects_google_urls_with_browser_download_hint(self):
        with self.assertRaises(AppError) as raised:
            document_import.preview_document_import(
                self.conn,
                source_file="https://drive.google.com/file/d/example/view",
                client_factory=lambda _provider: self.fail("client should not be created"),
            )

        self.assertEqual(raised.exception.code, "document_import_url_not_supported")
        self.assertIn("logged-in browser", raised.exception.hint)

    def test_preview_requires_installed_vision_model_and_returns_recommendations(self):
        client = FakeVisionClient(models=[{"id": "qwen3:8b"}])

        with self.assertRaises(AppError) as raised:
            document_import.preview_document_import(
                self.conn,
                source_file=str(self.source),
                client_factory=lambda _provider: client,
            )

        self.assertEqual(raised.exception.code, "document_import_model_missing")
        self.assertIn("recommendations", raised.exception.details)
        self.assertTrue(
            any(row["id"] == "glm-ocr" for row in raised.exception.details["recommendations"])
        )

    def test_preview_and_import_ready_rows_attach_source_evidence(self):
        _, profile, wallet = _book(self.conn)
        client = FakeVisionClient()

        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: client,
        )

        self.assertEqual(draft["model"], "glm-ocr")
        self.assertEqual(draft["summary"]["ready"], 1)
        self.assertEqual(draft["summary"]["quarantined"], 1)
        self.assertEqual(draft["rows"][0]["status"], "ready")
        self.assertEqual(draft["rows"][1]["status"], "quarantined")
        draft["rows"][0]["import_record"]["amount"] = "999999"
        user_content = client.chat_requests[0]["messages"][1]["content"]
        self.assertTrue(any(part.get("type") == "image_url" for part in user_content))

        outcome = document_import.import_document_draft(
            self.conn,
            source_file=str(self.source),
            wallet=wallet,
            profile=profile,
            rows=draft["rows"],
            hooks=_hooks(),
        )

        self.assertEqual(outcome["imported"], 1)
        self.assertEqual(outcome["draft_rows_imported"], 1)
        self.assertEqual(outcome["quarantined_skipped"], 1)
        self.assertEqual(len(outcome["attached_evidence"]), 1)

        tx = self.conn.execute("SELECT * FROM transactions").fetchone()
        self.assertIsNotNone(tx)
        self.assertEqual(tx["direction"], "inbound")
        self.assertEqual(tx["amount"], 1_000_000_000)
        raw = json.loads(tx["raw_json"])
        self.assertEqual(raw["source"], "document_import")
        self.assertEqual(raw["model_confidence"], 0.94)

        attachment = self.conn.execute("SELECT * FROM attachments").fetchone()
        self.assertIsNotNone(attachment)
        self.assertEqual(attachment["transaction_id"], tx["id"])
        self.assertEqual(attachment["original_filename"], "receipt.png")
        stored = self.root / "attachments" / attachment["stored_relpath"]
        self.assertTrue(stored.exists())

    def test_import_rejects_source_changed_after_preview(self):
        _, profile, wallet = _book(self.conn)
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(),
        )
        self.source.write_bytes(b"changed-after-preview")

        with self.assertRaises(AppError) as raised:
            document_import.import_document_draft(
                self.conn,
                source_file=str(self.source),
                wallet=wallet,
                profile=profile,
                rows=draft["rows"],
                hooks=_hooks(),
                expected_source_sha256=draft["source"]["sha256"],
            )

        self.assertEqual(raised.exception.code, "document_import_source_changed")
        self.assertEqual(
            self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0],
            0,
        )

    def test_preview_rejects_source_changed_during_model_call(self):
        client = FakeVisionClient()

        def mutate_then_chat(**kwargs):
            self.source.write_bytes(b"changed-during-preview")
            return FakeVisionClient.chat(client, **kwargs)

        client.chat = mutate_then_chat
        with self.assertRaises(AppError) as raised:
            document_import.preview_document_import(
                self.conn,
                source_file=str(self.source),
                client_factory=lambda _provider: client,
            )

        self.assertEqual(raised.exception.code, "document_import_source_changed")

    def test_generic_amount_without_crypto_asset_is_quarantined(self):
        content = json.dumps(
            {
                "rows": [
                    {
                        "occurred_at": "2026-01-02",
                        "direction": "outbound",
                        "amount": "500.00",
                        "fiat_currency": "EUR",
                        "confidence": 0.99,
                    }
                ]
            }
        )
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(content=content),
        )

        self.assertEqual(draft["rows"][0]["status"], "quarantined")
        self.assertIn("missing_amount", draft["rows"][0]["flags"])
        self.assertIsNone(draft["rows"][0]["import_record"])

    def test_unparseable_localized_date_is_quarantined(self):
        content = json.dumps(
            {
                "rows": [
                    {
                        "occurred_at": "02.01.2026",
                        "direction": "inbound",
                        "amount_btc": "0.01",
                        "confidence": 0.99,
                    }
                ]
            }
        )
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(content=content),
        )

        self.assertEqual(draft["rows"][0]["status"], "quarantined")
        self.assertIn("invalid_date", draft["rows"][0]["flags"])

    def test_fenced_nested_json_is_parsed_completely(self):
        content = """```json
        {"rows":[{"occurred_at":"2026-01-02","direction":"inbound","amount_btc":"0.01","confidence":0.99,"source_region":{"page":1}}]}
        ```"""
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(content=content),
        )

        self.assertEqual(draft["summary"]["ready"], 1)

    def test_decimal_comma_values_do_not_shift_magnitude(self):
        content = json.dumps(
            {
                "rows": [
                    {
                        "occurred_at": "2026-01-02",
                        "direction": "inbound",
                        "amount_btc": "0,01000000",
                        "fiat_value": "500,00",
                        "confidence": "0,94",
                    }
                ]
            }
        )
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(content=content),
        )

        record = draft["rows"][0]["import_record"]
        self.assertEqual(record["amount"], "0.01")
        self.assertEqual(record["fiat_value"], "500")
        self.assertEqual(draft["rows"][0]["confidence"], 0.94)

    def test_generated_row_ids_are_unique_to_source_document(self):
        other = self.root / "other.png"
        other.write_bytes(b"different-document")
        first = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(),
        )
        second = document_import.preview_document_import(
            self.conn,
            source_file=str(other),
            client_factory=lambda _provider: FakeVisionClient(),
        )

        self.assertNotEqual(first["rows"][0]["id"], second["rows"][0]["id"])
        self.assertNotEqual(
            first["rows"][0]["import_record"]["id"],
            second["rows"][0]["import_record"]["id"],
        )

    def test_explicit_empty_selection_imports_nothing(self):
        _, profile, wallet = _book(self.conn)
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(),
        )

        with self.assertRaises(AppError) as raised:
            document_import.import_document_draft(
                self.conn,
                source_file=str(self.source),
                wallet=wallet,
                profile=profile,
                rows=draft["rows"],
                selected_row_ids=[],
                hooks=_hooks(),
            )

        self.assertEqual(raised.exception.code, "document_import_no_ready_rows")
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 0)

    def test_attachment_failure_rolls_back_entire_document_import(self):
        _, profile, wallet = _book(self.conn)
        content = json.dumps(
            {
                "rows": [
                    {
                        "occurred_at": "2026-01-02",
                        "direction": "inbound",
                        "amount_btc": "0.01",
                        "confidence": 0.95,
                    },
                    {
                        "occurred_at": "2026-01-03",
                        "direction": "inbound",
                        "amount_btc": "0.02",
                        "confidence": 0.95,
                    },
                ]
            }
        )
        draft = document_import.preview_document_import(
            self.conn,
            source_file=str(self.source),
            client_factory=lambda _provider: FakeVisionClient(content=content),
        )
        real_add = core_attachments.add_attachment
        calls = 0

        def flaky_add(*args, **kwargs):
            nonlocal calls
            calls += 1
            if calls == 2:
                raise AppError("simulated attachment failure", code="test_failure")
            return real_add(*args, **kwargs)

        with (
            mock.patch.object(core_attachments, "add_attachment", side_effect=flaky_add),
            self.assertRaises(AppError),
        ):
            document_import.import_document_draft(
                self.conn,
                source_file=str(self.source),
                wallet=wallet,
                profile=profile,
                rows=draft["rows"],
                hooks=_hooks(),
            )

        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM transactions").fetchone()[0], 0)
        self.assertEqual(self.conn.execute("SELECT COUNT(*) FROM attachments").fetchone()[0], 0)
        attachment_files = [path for path in (self.root / "attachments").rglob("*") if path.is_file()]
        self.assertEqual(attachment_files, [])


if __name__ == "__main__":
    unittest.main()
