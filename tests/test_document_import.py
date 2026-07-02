from __future__ import annotations

import json
import tempfile
import unittest
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
            kind="local",
        )

        with self.assertRaises(AppError) as raised:
            document_import.preview_document_import(
                self.conn,
                source_file=str(self.source),
                provider_name="lan",
                client_factory=lambda _provider: self.fail("client should not be created"),
            )

        self.assertEqual(raised.exception.code, "document_import_remote_ai_disabled")

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


if __name__ == "__main__":
    unittest.main()
