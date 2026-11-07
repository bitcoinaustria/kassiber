"""A credential a server or tool echoes back never reaches Kassiber's output.

Every outbound integration (see docs/reference/credential-echo.md) withholds
the credentials it sent from untrusted response text before that text is
truncated or reaches an error. Each test drives one error path with a fake
transport that repeats the credential, then checks every surface the error
reaches: the CLI envelope, the RAM log ring, the public diagnostics report,
the in-product AI's tool-result projection and, where the error is stored,
every table of the book.
"""

import argparse
import base64
import json
import logging
import tempfile
import traceback
import unittest
from urllib.parse import quote, quote_plus

from kassiber.diagnostics import collect_public_diagnostics
from kassiber.envelope import build_error_envelope
from kassiber.errors import AppError
from kassiber.log_ring import LogRing, RingHandler, sanitize_traceback_text
from kassiber.redaction import (
    WITHHELD_CREDENTIAL,
    authorization_credentials,
    backend_credentials,
    credential_forms,
    provider_error_body_preview,
    request_credentials,
    url_credentials,
    withhold_credentials,
    withhold_credentials_value,
    withhold_error_credentials,
)


def error_surfaces(error):
    """The texts an ``AppError`` reaches outside the book."""

    from kassiber.daemon import _tool_result_from_app_error

    debug = sanitize_traceback_text(
        "".join(traceback.format_exception(type(error), error, error.__traceback__))
    )
    envelope = build_error_envelope(
        error.code, str(error), details=error.details, hint=error.hint,
        retryable=error.retryable, debug=debug,
    )
    ring = LogRing()
    logger = logging.getLogger("kassiber.tests.credential_echo")
    handler = RingHandler(ring)
    logger.addHandler(handler)
    try:
        logger.error("operation failed: %s", error, exc_info=error,
                     extra={"kb_fields": {"details": json.dumps(error.details, default=str)}})
    finally:
        logger.removeHandler(handler)
    with tempfile.TemporaryDirectory() as data_root:
        diagnostics = collect_public_diagnostics(
            None, argparse.Namespace(data_root=data_root, env_file=None, runtime_config={}),
            error=error,
        )
    return {
        "envelope": json.dumps(envelope),
        "log ring": json.dumps(ring.snapshot()),
        "diagnostics": json.dumps(diagnostics),
        "AI projection": json.dumps(_tool_result_from_app_error(error), default=str),
        # The raw error itself, as any other caller would read it.
        "error": json.dumps(
            {"message": str(error), "hint": error.hint, "details": error.details}, default=str
        ),
    }


def book_text(conn):
    """Every text cell of every table in a book, for a leak scan."""

    cells = []
    tables = [
        row[0]
        for row in conn.execute("SELECT name FROM sqlite_master WHERE type = 'table'").fetchall()
    ]
    for table in tables:
        for row in conn.execute(f'SELECT * FROM "{table}"').fetchall():
            cells.extend(f"{table}: {value}" for value in tuple(row) if isinstance(value, str))
    return "\n".join(cells)


class CredentialEchoAssertions:
    """``assertWithheld`` checks every 8-character run of each credential."""

    def assertWithheld(self, texts, credentials, *, skip=()):
        for surface, text in texts.items():
            for credential in credentials:
                # Any 8-character run of a credential counts as a leak, which
                # also catches a prefix cut off at a truncation boundary.
                for start in range(max(1, len(credential) - 7)):
                    window = credential[start:start + 8]
                    if window in skip:
                        continue
                    self.assertNotIn(window, text, f"{surface} shows {window!r}")
                    self.assertNotIn(window.lower(), text.lower(), f"{surface} shows {window!r} in another case")

    def assertErrorWithheld(self, error, credentials, **kwargs):
        self.assertWithheld(error_surfaces(error), credentials, **kwargs)


class WithholdCredentialsTest(unittest.TestCase):
    def test_every_spelling_is_withheld_before_the_cut(self):
        key = "kRk+Key/" + "A1b2C3d4" * 4
        text = f"{key} {quote(key, safe='')} {quote_plus(key, safe='')} {json.dumps(key)} {json.dumps(key).replace('/', chr(92) + '/')}"
        self.assertEqual(
            withhold_credentials(text, [key]),
            '<redacted> <redacted> <redacted> "<redacted>" "<redacted>"',
        )

    def test_hex_credentials_are_withheld_in_either_case(self):
        macaroon = "0201036c6e6402" + "ab" * 20
        echoed = f"bad {macaroon.upper()} and {macaroon[:10]}{macaroon[10:].upper()}"
        self.assertEqual(withhold_credentials(echoed, [macaroon]), "bad <redacted> and <redacted>")

    def test_forms_sort_by_length_then_text(self):
        # Equal-length forms sort by text, so the scrub never depends on the
        # interpreter's hash seed.
        self.assertEqual(credential_forms(["bb", "aa", "abc"]), ("abc", "aa", "bb"))
        self.assertEqual(credential_forms(["", None]), ())

    def test_value_and_error_scrub_keys_leaves_and_hint(self):
        secret = "s3cr3t-value-123"
        self.assertEqual(
            withhold_credentials_value({secret: [secret, 1, {"x": f"a {secret}"}]}, [secret]),
            {WITHHELD_CREDENTIAL: [WITHHELD_CREDENTIAL, 1, {"x": f"a {WITHHELD_CREDENTIAL}"}]},
        )
        error = AppError(f"failed {secret}", hint=f"hint {secret}", details={"stderr": secret})
        self.assertIs(withhold_error_credentials(error, [secret]), error)
        self.assertEqual(str(error), f"failed {WITHHELD_CREDENTIAL}")
        self.assertEqual(error.hint, f"hint {WITHHELD_CREDENTIAL}")
        self.assertEqual(error.details, {"stderr": WITHHELD_CREDENTIAL})

    def test_header_url_and_backend_credentials(self):
        basic = base64.b64encode(b"rpcuser:rpc-pass-word").decode()
        self.assertEqual(
            authorization_credentials(f"Basic {basic}"),
            [f"Basic {basic}", basic, "rpcuser:rpc-pass-word", "rpcuser", "rpc-pass-word"],
        )
        self.assertEqual(authorization_credentials("Bearer tok-123"), ["Bearer tok-123", "tok-123"])
        self.assertIn("p/w", url_credentials("https://user:p%2Fw@host/api?api_key=K-1&address=bc1q"))
        self.assertIn("K-1", url_credentials("https://host/api?api_key=K-1&address=bc1q"))
        self.assertNotIn("bc1q", url_credentials("https://host/api?api_key=K-1&address=bc1q"))
        self.assertIn("pp", url_credentials("pu:pp@127.0.0.1:9050", default_scheme="socks5h"))
        credentials = backend_credentials(
            {"token": "tok", "rpcuser": "u", "rpcpassword": "pw", "tor_proxy": "socks5h://pu:pp@127.0.0.1:9050"}
        )
        for item in ("tok", "u:pw", base64.b64encode(b"u:pw").decode(), "pp", base64.b64encode(b"pu:pp").decode()):
            self.assertIn(item, credentials)
        self.assertEqual(
            set(request_credentials(url="https://h", headers={"X-MBX-APIKEY": "mbx", "Accept": "json"})),
            {"mbx"},
        )

    def test_provider_preview_withholds_before_the_cut(self):
        key = "provider-key-" + "z" * 20
        preview, truncated = provider_error_body_preview("x" * 505 + key + "y" * 20, withheld=[key])
        self.assertTrue(truncated)
        self.assertNotIn("provider-key", preview)


if __name__ == "__main__":
    unittest.main()
