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
import re
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
                    if re.fullmatch(r"[0-9A-Fa-f]+", credential):
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
            [f"Basic {basic}", basic, "rpcuser:rpc-pass-word", "rpc-pass-word"],
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


# --- BTCPay Greenfield -------------------------------------------------------

BTCPAY_KEY = "0123456789abcdef0123456789abcdef01234567"
BTCPAY_PROXY_PASSWORD = "proxy-pass-Word-42"


def _btcpay_echo_body(key):
    return {
        "code": f"bad-key-{key}",
        "message": f"Key {key} / {quote(key)} / {key.upper()} is invalid " + "x" * 190 + key,
        "missingPermission": f"btcpay.store.{key}",
    }


class BtcpayCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def _client(self, routes):
        from kassiber.btcpay.client import GreenfieldClient
        from tests.test_sync_btcpay_greenfield import _RoutedOpener

        return GreenfieldClient.from_backend(
            {
                "url": "https://pay.example.com",
                "token": BTCPAY_KEY,
                "tor_proxy": f"socks5h://user:{BTCPAY_PROXY_PASSWORD}@127.0.0.1:9050",
            },
            opener=_RoutedOpener(routes),
        )

    def test_every_greenfield_status_withholds_the_key(self):
        for status in (400, 401, 403, 404, 422, 500):
            with self.subTest(status=status):
                body = _btcpay_echo_body(BTCPAY_KEY)
                body["message"] += f" via {BTCPAY_PROXY_PASSWORD}"
                client = self._client({"/api/v1/stores": (status, body)})
                with self.assertRaises(AppError) as raised:
                    client.get_json("/api/v1/stores", context="stores")
                self.assertErrorWithheld(raised.exception, (BTCPAY_KEY, BTCPAY_PROXY_PASSWORD))

    def test_tolerated_failures_withhold_the_key_in_the_body(self):
        client = self._client({"/api/v1/stores": (403, _btcpay_echo_body(BTCPAY_KEY))})
        _payload, failure = client.get_optional_json("/api/v1/stores")
        self.assertWithheld({"body": json.dumps(failure.body)}, (BTCPAY_KEY,))
        self.assertIn("<redacted>", failure.missing_permission)

    def test_a_failed_sync_stores_no_key_in_the_book(self):
        from unittest.mock import patch

        from kassiber import sync_btcpay
        from kassiber.cli import handlers
        from kassiber.core import freshness
        from tests.test_btcpay_ledger_and_fees import _BookTest
        from tests.test_sync_btcpay_greenfield import _RoutedOpener

        class Book(_BookTest):
            def runTest(self):
                pass

        book = Book()
        book.setUp()
        self.addCleanup(book.tearDown)
        book.runtime_config["backends"]["shop"]["token"] = BTCPAY_KEY
        wallet = book._wallet(
            "ln",
            "Shop Lightning",
            {
                "sync_source": "btcpay",
                "source_mode": "payments",
                "backend": "shop",
                "store_id": "S1",
                "payment_method_id": "BTC-LN",
                "chain": "bitcoin",
            },
        )
        opener = _RoutedOpener({"/api/v1/stores/S1/invoices": (500, _btcpay_echo_body(BTCPAY_KEY))})
        job = freshness.enqueue_job(
            book.conn,
            profile_id="prof",
            job_type=freshness.JOB_BTCPAY_WALLET,
            source_key=freshness.source_key("wallet", "ln"),
            source_type="wallet",
            source_label="Shop Lightning",
        )
        book.conn.commit()

        def handler(conn, _job, _progress, _check):
            return handlers._sync_btcpay_wallet(conn, book.runtime_config, book.profile, wallet)

        with patch.object(sync_btcpay, "_backend_http_opener", lambda backend: opener):
            outcome = freshness.run_job(book.conn, job["id"], {freshness.JOB_BTCPAY_WALLET: handler})
        self.assertEqual(outcome["status"], freshness.JOB_ERROR)
        stored = book_text(book.conn)
        self.assertIn("btcpay_store_sync_states", stored)
        self.assertIn("BTCPay returned HTTP 500", stored)
        self.assertWithheld({"book": stored}, (BTCPAY_KEY,))


# --- Loopback echo server ------------------------------------------------------


def echoed(values):
    """One text repeating each value raw, percent- and JSON-escaped, and hex upper-cased."""

    parts = []
    for value in values:
        value = str(value)
        parts.extend((value, quote(value, safe=""), quote_plus(value), json.dumps(value)[1:-1]))
        if re.fullmatch(r"[0-9A-Fa-f]+", value):
            # Hex is the same bytes in either case (an LND macaroon).
            parts.append(value.upper())
    return " | ".join(parts)


def request_secrets(handler):
    """What a server can echo: the request line and every header value, and a
    ``Basic`` header's decoded ``user:password``."""

    values = []
    for name, value in handler.headers.items():
        if name.lower() in {
            "host", "content-length", "content-type", "accept", "accept-encoding", "user-agent", "connection",
        }:
            continue
        values.append(value)
        scheme, _, rest = value.partition(" ")
        if scheme.lower() == "basic":
            # The decoded user:password first, as a server would quote it.
            values.insert(len(values) - 1, base64.b64decode(rest).decode())
    # The request line last: the credentials come first, where a cut at 200
    # characters would keep them.
    return [*values, handler.path]


class EchoServer:
    """A loopback HTTP server whose every answer repeats what the request carried.

    ``respond(handler, echo)`` returns ``(status, content_type, body)``; ``echo``
    is the request's credentials in every spelling (see ``echoed``). Used as an
    upstream server and, for proxy tests, as an HTTP proxy.
    """

    def __init__(self, respond):
        import http.server
        import threading

        requests = self.requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def _answer(self):
                length = int(self.headers.get("Content-Length") or 0)
                body = self.rfile.read(length) if length else b""
                requests.append({"path": self.path, "headers": dict(self.headers), "body": body})
                status, content_type, payload = respond(self, echoed(request_secrets(self)))
                data = payload if isinstance(payload, bytes) else payload.encode()
                self.send_response(status)
                self.send_header("Content-Type", content_type)
                self.send_header("Content-Length", str(len(data)))
                self.send_header("Connection", "close")
                self.end_headers()
                self.wfile.write(data)

            do_GET = do_POST = _answer

            def log_message(self, *_args):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self.thread = threading.Thread(target=self.server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)

    @property
    def url(self):
        return f"http://127.0.0.1:{self.server.server_address[1]}"

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, exc_type, *_exc):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(5)
        if exc_type is None and not self.requests:
            # An error raised before the request (offline mode, a config
            # check) would make a leak test pass for the wrong reason.
            raise AssertionError("the loopback server got no request")
        return False


def json_echo(status, *, key="message"):
    return lambda _handler, echo: (status, "application/json", json.dumps({key: echo, "error": {"message": echo, "code": echo}}))


OFFSETS = (0, 190)


def proxy_echo(status, *, as_json=False):
    """An HTTP proxy's answer repeating only its own credentials."""

    def respond(handler, _echo):
        value = handler.headers.get("Proxy-Authorization") or ""
        decoded = base64.b64decode(value.partition(" ")[2]).decode() if value else ""
        text = echoed([decoded, value])
        if as_json:
            return status, "application/json", json.dumps({"message": text})
        return status, "text/plain", text

    return respond


def text_echo(status, offset=0):
    """A plain-text answer echoing the credentials after ``offset`` characters;
    at 190 the first one straddles the 200-character cut errors apply."""

    return lambda _handler, echo: (status, "text/plain", "x" * offset + echo)


# --- LND REST --------------------------------------------------------------------

MACAROON = "0201036c6e6402" + "ab" * 40 + "0f1e2d3c"


class LndCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def _request(self, respond):
        from kassiber.core.lightning import lnd

        with EchoServer(respond) as server:
            with self.assertRaises(AppError) as raised:
                lnd.LndRestClient({"url": server.url, "token": MACAROON}).get("/v1/getinfo")
        self.assertIn(MACAROON, server.requests[0]["headers"].values())
        return raised.exception

    def test_http_error_bodies_withhold_the_macaroon_in_either_case(self):
        for status in (401, 403, 500, 503):
            for offset in OFFSETS:
                with self.subTest(status=status, offset=offset):
                    error = self._request(text_echo(status, offset))
                    self.assertIn(f"HTTP {status}", str(error))
                    self.assertErrorWithheld(error, (MACAROON,))

    def test_a_non_json_answer_withholds_the_macaroon_in_its_preview(self):
        error = self._request(lambda _h, echo: (200, "text/plain", "<html>" + echo))
        self.assertIn("response_preview", error.details)
        self.assertErrorWithheld(error, (MACAROON,))


# --- Core Lightning ------------------------------------------------------------

RUNE = "tU-RWxo6VjFbHkRmsN5HmwAkZYUcbkWxKZwIG4vSaCw9MyZtZXRob2Q9bGlzdGZ1bmRz"


class ClnCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def test_lightning_cli_output_withholds_the_rune(self):
        import os
        import stat
        import sys

        from kassiber.core.lightning import cln

        with tempfile.TemporaryDirectory() as tmp:
            # A lightning-cli that fails and prints its arguments, rune included.
            script = os.path.join(tmp, "lightning-cli")
            with open(script, "w", encoding="utf-8") as handle:
                handle.write(
                    f"#!{sys.executable}\nimport json, sys, urllib.parse\n"
                    "args = ' '.join(sys.argv[1:])\n"
                    "sys.stderr.write('commando failed: ' + args + ' ' + urllib.parse.quote(args, safe='') + ' ' + json.dumps(args))\n"
                    "sys.exit(1)\n"
                )
            os.chmod(script, stat.S_IRWXU)
            backend = {
                "name": "node",
                "url": "cln://commando",
                "lightning_cli": script,
                "commando_peer_id": "02" + "11" * 32,
                "token": RUNE,
            }
            with patch_online():
                with self.assertRaises(AppError) as raised:
                    cln.call_core_lightning(backend, "getinfo")
        self.assertIn("commando failed", raised.exception.details["stderr"])
        self.assertErrorWithheld(raised.exception, (RUNE,))


def patch_online():
    """Lightning calls refuse to start in offline mode; tests run online."""

    from unittest.mock import patch

    from kassiber import egress_policy

    return patch.object(egress_policy, "require_online", lambda *_a, **_k: None)


# --- Esplora and Bitcoin Core RPC ------------------------------------------------

ESPLORA_TOKEN = "esplora-bearer-T0ken-" + "q" * 16
ESPLORA_QUERY_KEY = "query-api-key-V4lue-77"
RPC_USER = "rpc-user-name-xyz"
RPC_PASSWORD = "rpc/pass+word=" + "Z9" * 8


class ChainBackendCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def test_esplora_errors_withhold_the_token_header_and_query_key(self):
        from kassiber.core import sync_backends

        for status in (400, 401, 403, 404, 500):
            for auth in ({"token": ESPLORA_TOKEN}, {"auth_header": f"Bearer {ESPLORA_TOKEN}"}):
                with self.subTest(status=status, auth=sorted(auth)):
                    with EchoServer(json_echo(status)) as server:
                        backend = {"url": f"{server.url}/api?api_key={ESPLORA_QUERY_KEY}", **auth}
                        headers = sync_backends._esplora_auth_headers(backend)
                        for call in (
                            lambda: sync_backends.http_get_json(f"{server.url}/api/blocks/tip?api_key={ESPLORA_QUERY_KEY}", headers=headers),
                            lambda: sync_backends.http_get_text(f"{server.url}/api/blocks/tip/hash?api_key={ESPLORA_QUERY_KEY}", headers=headers),
                        ):
                            with self.assertRaises(AppError) as raised:
                                call()
                            self.assertErrorWithheld(raised.exception, (ESPLORA_TOKEN, ESPLORA_QUERY_KEY))

    def test_bitcoin_core_errors_withhold_user_password_and_basic_form(self):
        from kassiber.core import sync_backends

        basic = base64.b64encode(f"{RPC_USER}:{RPC_PASSWORD}".encode()).decode()
        for status in (401, 403, 502):
            for offset in OFFSETS:
                with self.subTest(status=status, offset=offset):
                    with EchoServer(text_echo(status, offset)) as server:
                        backend = {"name": "core", "url": server.url, "username": RPC_USER, "password": RPC_PASSWORD}
                        with self.assertRaises(AppError) as raised:
                            sync_backends.bitcoinrpc_call(backend, "getblockchaininfo")
                    self.assertErrorWithheld(raised.exception, (RPC_PASSWORD, basic, f"{RPC_USER}:{RPC_PASSWORD}"))

    def test_bitcoin_core_cookie_errors_withhold_the_cookie(self):
        from kassiber.core import sync_backends

        cookie_password = "c0ffee" * 10
        with tempfile.TemporaryDirectory() as tmp:
            cookie = f"{tmp}/.cookie"
            with open(cookie, "w", encoding="utf-8") as handle:
                handle.write(f"__cookie__:{cookie_password}")
            with EchoServer(text_echo(401)) as server:
                backend = {"name": "core", "url": server.url, "cookiefile": cookie}
                with self.assertRaises(AppError) as raised:
                    sync_backends.bitcoinrpc_call(backend, "getblockchaininfo")
        self.assertErrorWithheld(raised.exception, (cookie_password,))

    def test_bitcoin_core_typed_rpc_errors_keep_no_server_text(self):
        from kassiber.core import sync_backends

        def respond(_handler, echo):
            return 500, "application/json", json.dumps(
                {"result": None, "id": "kassiber-getblockchaininfo", "error": {"code": -1, "message": echo}}
            )

        with EchoServer(respond) as server:
            backend = {"name": "core", "url": server.url, "username": RPC_USER, "password": RPC_PASSWORD}
            with self.assertRaises(AppError) as raised:
                sync_backends.bitcoinrpc_call(backend, "getblockchaininfo")
        self.assertEqual(str(raised.exception), "Bitcoin Core RPC getblockchaininfo failed (-1)")
        self.assertErrorWithheld(raised.exception, (RPC_PASSWORD,))

    def test_core_rescan_failure_text_withholds_the_password(self):
        from kassiber.core import sync_backends

        backend = {"name": "core", "url": "http://127.0.0.1:1", "username": RPC_USER, "password": RPC_PASSWORD}
        with self.assertRaises(AppError) as raised:
            sync_backends._raise_bitcoinrpc_import_failure(backend, None, "importdescriptors", {"error": {"message": f"bad {RPC_PASSWORD}"}})
        self.assertErrorWithheld(raised.exception, (RPC_PASSWORD,))

    def test_electrum_errors_withhold_backend_credentials(self):
        import socketserver
        import threading

        from kassiber.core import sync_backends

        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                for line in self.rfile:
                    request = json.loads(line)
                    reply = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": 1, "message": f"no {ESPLORA_TOKEN}"}}
                    self.wfile.write((json.dumps(reply) + "\n").encode())

        server = socketserver.ThreadingTCPServer(("127.0.0.1", 0), Handler)
        server.daemon_threads = True
        thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
        thread.start()
        try:
            backend = {"name": "fulcrum", "kind": "electrum", "url": f"tcp://127.0.0.1:{server.server_address[1]}", "token": ESPLORA_TOKEN}
            with self.assertRaises(AppError) as raised:
                with sync_backends.ElectrumClient(backend):
                    pass
        finally:
            server.shutdown()
            server.server_close()
        self.assertErrorWithheld(raised.exception, (ESPLORA_TOKEN,))

    def test_a_socket_url_error_withholds_its_userinfo(self):
        from kassiber.core import sync_backends

        # A password with `/` also cuts the URL short, where urllib's own
        # error would quote the part before it as a bad port.
        socket_password = "socket-pass-word-99"
        for url in (f"ssl://user:{RPC_PASSWORD}@", f"ssl://user:{socket_password}@", f"ssl://user:{socket_password}x"):
            with self.subTest(url=url[:9]):
                with self.assertRaises(AppError) as raised:
                    sync_backends.parse_socket_backend_url(url)
                self.assertErrorWithheld(raised.exception, (RPC_PASSWORD, socket_password))


# --- HTTP proxies, exchanges and rate sources -----------------------------------

PROXY_USER = "proxy-user-abc"
PROXY_PASSWORD = "proxy/Pass:word-" + "K" * 10
EXCHANGE_KEY = "exch+Key/" + "A1b2C3d4" * 4
EXCHANGE_SECRET = base64.b64encode(b"synthetic exchange secret bytes!" * 2).decode()


class ProxyCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def _proxy(self, server):
        return f"http://{quote(PROXY_USER, safe='')}:{quote(PROXY_PASSWORD, safe='')}@127.0.0.1:{server.server.server_address[1]}"

    def _proxy_forms(self):
        pair = f"{PROXY_USER}:{PROXY_PASSWORD}"
        return (PROXY_PASSWORD, pair, base64.b64encode(pair.encode()).decode())

    def test_an_http_proxy_error_withholds_proxy_and_exchange_credentials(self):
        from kassiber.core.exchange_imports import fetch_binance_records, fetch_coinbase_records, fetch_kraken_records

        for fetch in (fetch_kraken_records, fetch_coinbase_records, fetch_binance_records):
            for status in (407, 502):
                with self.subTest(fetch=fetch.__name__, status=status):
                    with EchoServer(proxy_echo(status)) as server:
                        backend = {
                            "url": "http://exchange.invalid",
                            "token": EXCHANGE_KEY,
                            "auth_header": EXCHANGE_SECRET,
                            "tor_proxy": self._proxy(server),
                        }
                        with self.assertRaises(AppError) as raised:
                            fetch(backend)
                    # The proxy saw its own credentials (and the exchange key header).
                    self.assertIn("Proxy-Authorization", server.requests[0]["headers"])
                    self.assertErrorWithheld(raised.exception, self._proxy_forms())

    def test_a_rate_source_proxy_error_withholds_proxy_credentials(self):
        from kassiber.core import rates

        with EchoServer(proxy_echo(407)) as server:
            with self.assertRaises(AppError) as raised:
                rates.http_get_json("http://rates.invalid/api/v3/simple/price", proxy_url=self._proxy(server))
        self.assertErrorWithheld(raised.exception, self._proxy_forms())

    def test_a_btcpay_proxy_error_withholds_proxy_credentials(self):
        from kassiber.btcpay.client import GreenfieldClient

        with EchoServer(proxy_echo(502, as_json=True)) as server:
            client = GreenfieldClient.from_backend(
                {"url": "http://pay.invalid", "token": BTCPAY_KEY, "tor_proxy": self._proxy(server)}
            )
            with self.assertRaises(AppError) as raised:
                client.get_json("/api/v1/stores")
        self.assertErrorWithheld(raised.exception, self._proxy_forms())

    def test_malformed_proxy_and_target_urls_are_not_echoed(self):
        from urllib import request as urlrequest

        from kassiber import proxy

        for setting in (
            f"socks5h://{PROXY_USER}:{PROXY_PASSWORD}@:9050",
            f"://{PROXY_USER}:{PROXY_PASSWORD}@127.0.0.1:9050",
            f"ftp://{PROXY_USER}:{PROXY_PASSWORD}@127.0.0.1:21",
            f"socks5h://{PROXY_USER}:{PROXY_USER}x",
        ):
            with self.subTest(setting=setting.split("://")[0]):
                with self.assertRaises(AppError) as raised:
                    # A SOCKS proxy is parsed when the response opens.
                    with proxy.urlopen_with_proxy(urlrequest.Request("http://target.invalid/"), proxy_url=setting):
                        pass
                self.assertErrorWithheld(raised.exception, (PROXY_PASSWORD,))
        with self.assertRaises(AppError) as raised:
            proxy.SocksUrlResponse(f"ftp://user:{PROXY_PASSWORD}@host/", "socks5h://127.0.0.1:9050", 1, {}).__enter__()
        self.assertErrorWithheld(raised.exception, (PROXY_PASSWORD,))


# --- AI providers -----------------------------------------------------------------

# Not an `sk-` key, which pattern redaction would already recognize.
AI_KEY = "ak-live_" + "Rt7q" * 8


class AiProviderCredentialEchoTest(CredentialEchoAssertions, unittest.TestCase):
    def _client(self, server):
        from kassiber.ai.client import OpenAICompatibleClient

        return OpenAICompatibleClient(
            base_url=f"{server.url}/v1", api_key=AI_KEY, api_format="chat_completions", timeout=5
        )

    def test_http_errors_withhold_the_api_key(self):
        for status in (400, 401, 403, 429, 500):
            with self.subTest(status=status):
                with EchoServer(json_echo(status)) as server:
                    with self.assertRaises(AppError) as raised:
                        self._client(server).list_models(strict=True)
                self.assertIn("body", raised.exception.details)
                self.assertErrorWithheld(raised.exception, (AI_KEY,))

    def test_errors_inside_an_answer_withhold_the_api_key(self):
        def respond(_handler, echo):
            return 200, "application/json", json.dumps({"error": {"message": echo, "code": echo, "type": "invalid_request_error"}})

        with EchoServer(respond) as server:
            with self.assertRaises(AppError) as raised:
                self._client(server).chat(messages=[{"role": "user", "content": "hi"}], model="m")
        self.assertErrorWithheld(raised.exception, (AI_KEY,))

    def test_errors_inside_a_stream_withhold_the_api_key(self):
        def respond(_handler, echo):
            event = json.dumps({"error": {"message": echo, "code": echo}})
            return 200, "text/event-stream", f"data: {event}\n\ndata: [DONE]\n\n"

        with EchoServer(respond) as server:
            with self.assertRaises(AppError) as raised:
                list(self._client(server).stream_chat(messages=[{"role": "user", "content": "hi"}], model="m"))
        self.assertErrorWithheld(raised.exception, (AI_KEY,))
