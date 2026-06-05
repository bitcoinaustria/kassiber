from __future__ import annotations

import unittest
from io import BytesIO

from kassiber.core.link_preview import fallback_url_label, preview_url


class _Response:
    def __init__(self, body: bytes, content_type: str = "text/html; charset=utf-8") -> None:
        self._body = BytesIO(body)
        self.headers = {"content-type": content_type}

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False

    def read(self, size: int = -1) -> bytes:
        return self._body.read(size)


class LinkPreviewTest(unittest.TestCase):
    def test_preview_url_uses_open_graph_title_before_document_title(self):
        def opener(request, timeout):
            return _Response(
                b"""
                <html>
                  <head>
                    <title>Fallback title - Google Sheets</title>
                    <meta property="og:title" content="Treasury Review - Google Sheets">
                    <meta property="og:site_name" content="Google Sheets">
                  </head>
                </html>
                """
            )

        preview = preview_url(
            "https://docs.google.com/spreadsheets/d/abc123/edit",
            opener=opener,
        )

        self.assertTrue(preview["available"])
        self.assertEqual(preview["title"], "Treasury Review")
        self.assertEqual(preview["label"], "Treasury Review")
        self.assertEqual(preview["site_name"], "Google Sheets")

    def test_preview_url_falls_back_without_fetching_unsupported_urls(self):
        preview = preview_url("ipfs://bafybeigdyrzt")

        self.assertFalse(preview["available"])
        self.assertEqual(preview["error_code"], "unsupported_url")
        self.assertEqual(preview["label"], "Link attachment")

    def test_fallback_url_label_recognizes_google_workspace_routes(self):
        self.assertEqual(
            fallback_url_label("https://docs.google.com/document/d/abc123/edit"),
            "Google Doc",
        )
        self.assertEqual(
            fallback_url_label("https://docs.google.com/spreadsheets/d/abc123/edit"),
            "Google Sheet",
        )


if __name__ == "__main__":
    unittest.main()
