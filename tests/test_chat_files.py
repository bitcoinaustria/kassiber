"""What the chat model sees of attached images and PDFs.

Pinned here: a provider with full agent access is shown an image as an image
and a PDF as its text (or page images of a scan); a restricted provider is
shown none of them and is not told their names; a provider that resumes its
own session is shown only the files a message first sends; the conversation
itself stays text; and the broker passes only inline images.
"""

from __future__ import annotations

import base64
import os
import queue
import shutil
import struct
import tempfile
import unittest
import zlib

from kassiber import daemon
from kassiber.ai.broker_client import _broker_messages_for_chat, _broker_messages_for_context
from kassiber.ai.client import responses_request_context
from kassiber.core import chat_files

POPPLER = all(shutil.which(tool) for tool in ("pdfinfo", "pdftotext", "pdftoppm"))


def png(width: int = 2, height: int = 2) -> bytes:
    """A valid grey PNG without Pillow."""

    def chunk(kind: bytes, data: bytes) -> bytes:
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))

    rows = b"".join(b"\0" + b"\x80" * width for _ in range(height))
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 0, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(rows))
        + chunk(b"IEND", b"")
    )


class ImageViewTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def write(self, name: str, data: bytes) -> str:
        path = os.path.join(self._tmp.name, name)
        with open(path, "wb") as handle:
            handle.write(data)
        return path

    def test_an_image_is_shown_inline(self):
        view = chat_files.chat_file_view(self.write("r.png", png()), "image", file_id="file-1")
        self.assertEqual(view.shown, "shown below as an image")
        (part,) = view.parts
        self.assertEqual(part["type"], "image_url")
        self.assertTrue(part["image_url"]["url"].startswith("data:image/png;base64,"))

    def test_the_bytes_decide_the_type_not_the_name(self):
        view = chat_files.chat_file_view(self.write("r.png", b"not an image"), "image", file_id="file-1")
        self.assertEqual(view.parts, [])
        self.assertIn("not shown", view.shown)

    def test_a_large_image_is_reduced(self):
        try:
            import PIL  # noqa: F401
        except ImportError:
            self.skipTest("Pillow is not installed")
        path = self.write("big.png", png(3000, 40))
        view = chat_files.chat_file_view(path, "image", file_id="file-1")
        self.assertEqual(view.shown, "shown below as a reduced image")
        from PIL import Image
        import io

        data = base64.b64decode(view.parts[0]["image_url"]["url"].split(",", 1)[1])
        with Image.open(io.BytesIO(data)) as image:
            self.assertLessEqual(max(image.size), chat_files.MAX_IMAGE_EDGE)


@unittest.skipUnless(POPPLER, "Poppler is not installed")
class PdfViewTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)

    def pdf(self, *, text: str | None = None, image: bytes | None = None) -> str:
        from reportlab.lib.utils import ImageReader
        from reportlab.pdfgen import canvas
        import io

        path = os.path.join(self._tmp.name, "statement.pdf")
        page = canvas.Canvas(path)
        if text:
            for index, line in enumerate(text.splitlines()):
                page.drawString(72, 720 - 14 * index, line)
        if image:
            page.drawImage(ImageReader(io.BytesIO(image)), 72, 400, width=200, height=200)
        page.save()
        return path

    def test_a_text_pdf_is_shown_as_its_text(self):
        path = self.pdf(text="Kauf 0.5 BTC am 2024-03-01\nGegenwert 21000 EUR")
        view = chat_files.chat_file_view(path, "pdf", file_id="file-2")
        self.assertEqual(view.shown, "shown below as the text of page 1")
        (part,) = view.parts
        self.assertEqual(part["type"], "text")
        self.assertIn("21000 EUR", part["text"])
        # Framed as data, so text inside a statement is not taken as a request.
        self.assertIn("not instructions", part["text"])

    def test_a_scan_is_shown_as_page_images(self):
        view = chat_files.chat_file_view(self.pdf(image=png(64, 64)), "pdf", file_id="file-2")
        self.assertEqual(view.shown, "shown below as images of page 1")
        self.assertEqual([part["type"] for part in view.parts], ["image_url"])


def runtime(files, *, full_access: bool) -> daemon.AiToolRuntime:
    return daemon.AiToolRuntime(
        "/nonexistent",
        {},
        queue.Queue(),
        {"chat_files": files, "provider_full_access": full_access, "provider_on_device": False},
    )


class TurnTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self._tmp.cleanup)
        image = os.path.join(self._tmp.name, "Beleg_Mustermann.png")
        with open(image, "wb") as handle:
            handle.write(png())
        export = os.path.join(self._tmp.name, "kraken.csv")
        with open(export, "w", encoding="utf-8") as handle:
            handle.write("Date,Amount\n")
        self.files = [
            {"id": "file-1", "source_file": export, "filename": "kraken.csv", "kind": "delimited", "new": False},
            {"id": "file-2", "source_file": image, "filename": "Beleg_Mustermann.png", "kind": "image", "new": True},
        ]

    def turn(self, *, full_access=True, resumed=False, files=None):
        prompt = {"role": "user", "content": "What is on the receipt?"}
        messages = [{"role": "system", "content": "sys"}, prompt]
        daemon._attach_chat_files_to_turn(
            messages, runtime(files or self.files, full_access=full_access), resumed=resumed
        )
        return messages, prompt

    def test_full_access_shows_the_image_on_the_users_message(self):
        messages, original = self.turn()
        note, message = messages[-2]["content"], messages[-1]["content"]
        self.assertIn("file-1: CSV/TSV export 'kraken.csv'", note)
        self.assertIn('ui.wallets.analyze_file (file="file-1")', note)
        self.assertIn("file-2: image 'Beleg_Mustermann.png', shown below as an image", note)
        self.assertIn("ui.attachments.add_from_chat", note)
        self.assertEqual(message[0], {"type": "text", "text": "What is on the receipt?"})
        self.assertEqual(message[1], {"type": "text", "text": "file-2:"})
        self.assertEqual(message[2]["type"], "image_url")
        # The stored conversation is untouched: no image bytes in history.
        self.assertEqual(original["content"], "What is on the receipt?")

    def test_a_restricted_provider_sees_no_file_and_no_name(self):
        messages, _ = self.turn(full_access=False)
        note = messages[-2]["content"]
        self.assertEqual(messages[-1]["content"], "What is on the receipt?")
        self.assertNotIn("Mustermann", note)
        self.assertIn("file-2: image (.png), not shown: this provider's agent access is restricted", note)

    def test_a_resumed_session_is_shown_only_new_files(self):
        old = [{**entry, "new": False} for entry in self.files]
        messages, _ = self.turn(resumed=True, files=old)
        self.assertIn("file-2: image 'Beleg_Mustermann.png', shown in an earlier message", messages[-2]["content"])
        self.assertEqual(messages[-1]["content"], "What is on the receipt?")
        # A stateless provider sees the file again on every turn.
        messages, _ = self.turn(resumed=False, files=old)
        self.assertIsInstance(messages[-1]["content"], list)

    def test_no_files_no_note(self):
        messages = [{"role": "user", "content": "hi"}]
        daemon._attach_chat_files_to_turn(messages, runtime([], full_access=True), resumed=False)
        self.assertEqual(messages, [{"role": "user", "content": "hi"}])

    def test_the_turn_image_budget_holds(self):
        many = [
            {**self.files[1], "id": f"file-{index}"}
            for index in range(1, daemon.MAX_CHAT_TURN_IMAGES + 2)
        ]
        messages, _ = self.turn(files=many)
        images = [part for part in messages[-1]["content"] if part["type"] == "image_url"]
        self.assertEqual(len(images), daemon.MAX_CHAT_TURN_IMAGES)
        self.assertIn("as many images as one turn can", messages[-2]["content"])

    def test_the_broker_receives_inline_images_only(self):
        messages, _ = self.turn()
        context = responses_request_context(messages)
        broker = _broker_messages_for_context(context)
        last = broker[-1]
        self.assertEqual(last["role"], "user")
        self.assertIn("What is on the receipt?", last["content"])
        self.assertEqual(last["images"][0]["media_type"], "image/png")
        self.assertEqual(base64.b64decode(last["images"][0]["data"]), png())
        # A remote image URL is never passed on for the provider to fetch.
        remote = responses_request_context(
            [{"role": "user", "content": [
                {"type": "text", "text": "x"},
                {"type": "image_url", "image_url": {"url": "https://example.com/a.png"}},
            ]}]
        )
        self.assertNotIn("images", _broker_messages_for_context(remote)[-1])

    def test_a_turn_without_tools_sends_images_as_images(self):
        # Regression: the no-tools path stringified list content, so the
        # model received the image as base64 text it could not read.
        messages, _ = self.turn()
        broker = _broker_messages_for_chat(messages)
        last = broker[-1]
        self.assertEqual(last["content"], "What is on the receipt?\nfile-2:")
        self.assertNotIn("base64", last["content"])
        self.assertEqual(base64.b64decode(last["images"][0]["data"]), png())
        self.assertEqual(broker[0], {"role": "system", "content": "sys"})


if __name__ == "__main__":
    unittest.main()
