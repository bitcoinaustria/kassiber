"""Replay a recorded Bitcoin Core tape through the real CLI over loopback.

`tests/test_regtest_harness.py` replays the same tapes by patching
`bitcoinrpc_call` inside the interpreter, which only a Python runtime can
take. Here a loopback JSON-RPC server answers from the tape instead, so any
runtime `tests/cli_runner.py` can start (`KASSIBER_TEST_CLI`) builds the book
through its own transport. The server is strict: a request the tape lacks is
answered with a Core error and recorded as a miss.

Two transforms keep the tape usable through the CLI, which generates wallet
ids: requests for the Core wallet the book derives (`kassiber-<wallet id>`)
are served from the tape's wallet, and `importdescriptors` labels are
compared with the tape's label. Everything else must match the tape exactly.
"""
from __future__ import annotations

import base64
from contextlib import contextmanager
import json
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
import re
import threading
import time
from typing import Any, Iterator
from urllib.parse import unquote

from tests.integration.tapes import RecordedTape, rpc_key

ROOT = Path(__file__).resolve().parents[2]
TAPES = ROOT / "tests" / "fixtures" / "regtest_tapes"
RPC_USER = "replay-user"
RPC_PASSWORD = "replay-password"


class TapeRpcServer:
    """A loopback Bitcoin Core endpoint that answers only from one tape."""

    def __init__(self, tape: RecordedTape):
        self.tape = tape
        keys = [json.loads(key) for key in tape.interactions]
        wallets = {key["wallet_name"] for key in keys if key["wallet_name"]}
        if len(wallets) != 1:
            raise ValueError(f"{tape.path} must name exactly one Core wallet")
        self.tape_wallet = wallets.pop()
        labels = {
            request["label"]
            for key in keys
            if key["method"] == "importdescriptors"
            for request in key["params"][0]
        }
        self.tape_label = labels.pop() if len(labels) == 1 else None
        # The Core wallet name the book derives; set once the wallet exists.
        self.client_wallet: str | None = None
        self.calls: list[str] = []
        self.misses: list[str] = []
        self.unauthorized = 0
        # Seconds every answer waits, so a sync's reported wall time is testable.
        self.delay = 0.0
        self._lock = threading.Lock()
        self._server: ThreadingHTTPServer | None = None
        self._thread: threading.Thread | None = None

    @property
    def url(self) -> str:
        assert self._server is not None
        host, port = self._server.server_address[:2]
        return f"http://{host}:{port}"

    def key_for(self, method: str, params: list[Any], wallet: str) -> str:
        if wallet:
            wallet = self.tape_wallet if wallet == self.client_wallet else f"<unknown:{wallet}>"
        if method == "importdescriptors" and self.tape_label and params and isinstance(params[0], list):
            params = [[{**request, "label": self.tape_label} for request in params[0]], *params[1:]]
        return rpc_key(method, params, wallet_name=wallet)

    def answer(self, method: str, params: list[Any], wallet: str) -> tuple[bool, Any]:
        key = self.key_for(method, params, wallet)
        with self._lock:
            self.calls.append(key)
            if key not in self.tape.interactions:
                self.misses.append(key)
                return False, None
        result = self.tape.lookup(key)
        if method == "listwallets" and self.client_wallet:
            result = [self.client_wallet if name == self.tape_wallet else name for name in result]
        return True, result

    def unused_interactions(self) -> list[str]:
        with self._lock:
            used = set(self.calls)
        return sorted(key for key in self.tape.interactions if key not in used)

    def start(self) -> None:
        owner = self
        expected = "Basic " + base64.b64encode(f"{RPC_USER}:{RPC_PASSWORD}".encode()).decode()

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):  # keep test output quiet
                pass

            def _reply(self, status: int, payload: Any) -> None:
                body = json.dumps(payload).encode()
                self.send_response(status)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                if owner.delay:
                    time.sleep(owner.delay)
                length = int(self.headers.get("Content-Length") or 0)
                raw = self.rfile.read(length)
                if self.headers.get("Authorization") != expected:
                    with owner._lock:
                        owner.unauthorized += 1
                    self.send_response(401)
                    self.send_header("Content-Length", "0")
                    self.end_headers()
                    return
                request = json.loads(raw)
                match = re.fullmatch(r"/wallet/([^/]+)", self.path.rstrip("/"))
                wallet = unquote(match.group(1)) if match else ""
                found, result = owner.answer(request["method"], list(request.get("params") or []), wallet)
                if found:
                    self._reply(200, {"result": result, "error": None, "id": request.get("id")})
                else:
                    self._reply(500, {"result": None, "error": {"code": -32601, "message": "Method not found"},
                                      "id": request.get("id")})

        self._server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        self._thread = threading.Thread(target=self._server.serve_forever, daemon=True)
        self._thread.start()

    def stop(self) -> None:
        if self._server is not None:
            self._server.shutdown()
            self._server.server_close()
            self._server = None


@contextmanager
def serve(tape_name: str) -> Iterator[TapeRpcServer]:
    server = TapeRpcServer(RecordedTape.load(TAPES / f"{tape_name}.json"))
    server.start()
    try:
        yield server
    finally:
        server.stop()


def setup_steps(server: TapeRpcServer, addresses: list[str]) -> list[tuple[str, ...] | tuple]:
    """CLI argv for an empty book with one Core-backed address wallet."""
    from tests.cli_runner import InputFd

    address_args: list[str] = []
    for address in addresses:
        address_args += ["--address", address]
    return [
        ("init",),
        ("workspaces", "create", "Regtest"),
        ("profiles", "create", "Replay", "--workspace", "Regtest", "--fiat-currency", "EUR",
         "--tax-country", "generic", "--gains-algorithm", "FIFO"),
        ("backends", "create", "core-regtest", "--kind", "bitcoinrpc", "--url", server.url,
         "--chain", "bitcoin", "--network", "regtest",
         "--username-fd", InputFd(RPC_USER.encode()), "--password-fd", InputFd(RPC_PASSWORD.encode()),
         "--timeout", "30"),
        ("wallets", "create", "--label", "Core replay", "--kind", "address", "--backend", "core-regtest",
         "--chain", "bitcoin", "--network", "regtest", *address_args),
    ]


def core_wallet_name(wallet_id: str) -> str:
    from kassiber.core.sync_backends import sanitize_wallet_segment

    return f"kassiber-{sanitize_wallet_segment(wallet_id)}"


BASELINE_RATES = (("2023-11-14T22:13:20Z", "35000"), ("2023-11-14T22:23:20Z", "36000"))
EDGE_RATES = (
    ("2023-11-14T22:13:20Z", "34000"),
    ("2023-11-14T23:13:20Z", "34500"),
    ("2023-11-15T00:13:20Z", "35000"),
    ("2023-11-15T01:00:00Z", "35500"),
    ("2023-11-15T02:00:00Z", "36000"),
)
BASELINE_ADDRESSES = ["bcrt1qs758ursh4q9z627kt3pp5yysm78ddny6txaqgw"]
EDGE_ADDRESSES = [
    "bcrt1qv4jxwefdvdshxefdv9jxgu3ddahx2gfplsfksc",
    "bcrt1qv4jxwefdvdshxefdv9jxgu3dw3mk7gfpl9er9m",
]
TAPE_CASES = {
    "bitcoin_core_address_baseline": (BASELINE_ADDRESSES, BASELINE_RATES),
    "bitcoin_core_edge_cases": (EDGE_ADDRESSES, EDGE_RATES),
}
# Reads and exports run on each runtime's copy of the synced book.
REPORTS = ("summary", "balance-sheet", "portfolio-summary", "capital-gains", "tax-summary",
           "journal-entries", "balance-history")
