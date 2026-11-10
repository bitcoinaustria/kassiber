"""Stdlib-only helpers the regtest harnesses share instead of importing the
product (docs/plan/22-python-removal.md, decision D2).

The harnesses run with `python3 -I` and drive the Rust `kassiber` binary
through `tests/cli_runner.py`. What they used to borrow from the Python
package lives here: plain `sqlite3` reads and fixture writes on the
plaintext demo books, one daemon request, and copies of a few pure helpers.
Each copy names the product code it mirrors; none of them decides anything
the product stores.
"""

from __future__ import annotations

import hashlib
import json
import re
import socket
import sqlite3
import time
from decimal import ROUND_HALF_UP, Decimal
from pathlib import Path
from typing import Any
from urllib import parse

from tests import cli_runner

DB_FILENAME = "kassiber.sqlite3"
LEGACY_DB_FILENAME = "satbooks.sqlite3"
MSAT_PER_BTC = Decimal("100000000000")
APP_NAME = "kassiber"
SILENT_PAYMENTS_WALLET_KIND = "silent-payment"
# The generic ledger template's columns, in order (crates/imports, the
# `wallets import-ledger` template).
GENERIC_LEDGER_COLUMNS = (
    "Type",
    "Date",
    "Received Amount",
    "Received Asset",
    "Sent Amount",
    "Sent Asset",
    "Fee Amount",
    "Fee Asset",
    "Fiat Value",
    "Counterparty",
    "Note",
    "Tx-ID",
    "Payment Hash",
    "Payment Hash Source",
    "Swap Refund Funding Tx-ID",
    "Swap Refund Funding Vout",
)
# Market-rate providers the product refreshes on its own, and the setting
# that names the selected one.
RATE_SOURCE_MEMPOOL = "mempool"
LIVE_MARKET_RATE_SOURCES = ("coinbase-exchange", "coingecko", RATE_SOURCE_MEMPOOL)
MARKET_RATE_PROVIDER_SETTING = "market_rate_provider"
FRESHNESS_POLICY_PREFIX = "freshness.policy.profile."


def database_path(data_root: str | Path) -> Path:
    """`kassiber.sqlite3` under the data root, or the legacy name if only it exists."""
    root = Path(data_root).expanduser()
    current = root / DB_FILENAME
    legacy = root / LEGACY_DB_FILENAME
    if current.exists() or not legacy.exists():
        return current
    return legacy


def open_book(data_root: str | Path, *, readonly: bool = False) -> sqlite3.Connection:
    """A plain sqlite3 connection to a plaintext demo book.

    Harness books are never encrypted. Reads see what the product stored;
    the few fixture writes (rate-cache cleanup, Lightning backdating) are
    test-data edits, made before the product processes journals again.
    """
    path = database_path(data_root)
    if not path.is_file():
        raise FileNotFoundError(f"No Kassiber book database at {path}")
    uri = f"file:{path}?mode=ro" if readonly else f"file:{path}"
    conn = sqlite3.connect(uri, uri=True, timeout=30)
    conn.row_factory = sqlite3.Row
    conn.execute("PRAGMA foreign_keys=ON")
    return conn


def sanitize_wallet_segment(value: Any) -> str:
    """The product's wallet-name segment (crates/chain, Core watch-only names)."""
    text = str(value).strip().lower()
    cleaned = [char if char.isalnum() or char in {"-", "_", "."} else "-" for char in text]
    return "".join(cleaned).strip("-") or APP_NAME


def bitcoinrpc_wallet_name(backend: dict[str, Any], wallet: dict[str, Any]) -> str:
    """The Core watch-only wallet the product creates for `wallet`."""
    explicit = backend.get("wallet")
    if explicit:
        return str(explicit)
    prefix = backend.get("walletprefix") or backend.get("wallet_prefix") or APP_NAME
    return f"{sanitize_wallet_segment(prefix)}-{sanitize_wallet_segment(wallet['id'])}"


def dec(value: Any, default: str = "0") -> Decimal:
    """Mixed input to Decimal; empty means `default`."""
    if value is None or (isinstance(value, str) and value == ""):
        return Decimal(default)
    return Decimal(str(value))


def btc_to_msat(value: Any) -> int | None:
    """BTC to integer msat, half up, as the product's import arithmetic."""
    if value is None:
        return None
    return int((dec(value) * MSAT_PER_BTC).to_integral_value(rounding=ROUND_HALF_UP))


def descriptor_fingerprint(material: Any) -> str:
    """SHA-256 of a Silent Payments descriptor without whitespace or checksum."""
    text = re.sub(r"\s+", "", str(material or ""))
    if "#" in text:
        text = text.split("#", 1)[0]
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def looks_like_sp_descriptor(material: Any) -> bool:
    """A structural check only; the product validates the key material when
    the wallet is created."""
    text = re.sub(r"\s+", "", str(material or "")).split("#", 1)[0]
    return text.lower().startswith("sp(") and text.endswith(")") and len(text) > 4 and "spspend" not in text.lower()


def stable_hash(parts: list[Any] | tuple[Any, ...]) -> str:
    """The CLN adapter's `external_id` hash: SHA-256 of sorted compact JSON."""
    payload = json.dumps(list(parts), sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(payload).hexdigest()


def daemon_call(data_root: str | Path, kind: str, args: dict[str, Any] | None = None) -> dict[str, Any]:
    """One request to `kassiber daemon`; returns the terminal record's `data`."""
    request: dict[str, Any] = {"request_id": "harness-1", "kind": kind}
    if args is not None:
        request["args"] = args
    result = cli_runner.run(
        ["--data-root", str(data_root), "daemon"],
        input=json.dumps(request) + "\n",
        capture_output=True,
        text=True,
        check=False,
        timeout=600,
    )
    for line in result.stdout.splitlines():
        try:
            record = json.loads(line)
        except json.JSONDecodeError:
            continue
        if record.get("request_id") != "harness-1":
            continue
        if record.get("kind") == "error" or "error" in record:
            raise RuntimeError(f"daemon {kind} failed: {record.get('error')}")
        if record.get("kind") == kind:
            return record.get("data") or {}
    raise RuntimeError(f"daemon {kind} gave no answer\nstdout={result.stdout}\nstderr={result.stderr}")


def latest_rate(data_root: str | Path, pair: str, source: str) -> dict[str, Any]:
    """The latest quote for `pair` from `source`, cached by the product.

    `ui.rates.latest` (Python's `sync_latest_rates`) refuses unless the active
    profile's freshness policy allows market-rate lookups. The demo opted in
    by its scenario, so allow them for this one call and put the policy row
    back exactly as it was.
    """
    conn = open_book(data_root)
    try:
        profile = conn.execute(
            "SELECT value FROM settings WHERE key = 'context_profile'"
        ).fetchone()
        if profile is None:
            raise RuntimeError("The demo book has no active profile for the live rate refresh")
        key = FRESHNESS_POLICY_PREFIX + str(profile["value"])
        prior = conn.execute("SELECT value FROM settings WHERE key = ?", (key,)).fetchone()
        policy = json.loads(prior["value"]) if prior else {}
        if not isinstance(policy, dict):
            policy = {}
        classes = dict(policy.get("source_classes") or {})
        classes["market_rates"] = True
        conn.execute(
            "INSERT INTO settings(key, value) VALUES(?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            (key, json.dumps({**policy, "source_classes": classes})),
        )
        conn.commit()
    finally:
        conn.close()
    try:
        return daemon_call(data_root, "ui.rates.latest", {"pair": pair, "source": source})
    finally:
        conn = open_book(data_root)
        try:
            if prior is None:
                conn.execute("DELETE FROM settings WHERE key = ?", (key,))
            else:
                conn.execute("UPDATE settings SET value = ? WHERE key = ?", (prior["value"], key))
            conn.commit()
        finally:
            conn.close()


def electrum_call(url: str, method: str, params: list[Any] | None = None) -> Any:
    """One plaintext Electrum JSON-RPC call to a loopback regtest server."""
    parsed = parse.urlsplit(url if "://" in url else f"tcp://{url}")
    if parsed.scheme != "tcp" or not parsed.hostname or not parsed.port:
        raise AssertionError(f"Expected a tcp:// Electrum URL, got {url!r}")
    payload = {"jsonrpc": "2.0", "id": f"kassiber-{method}", "method": method, "params": list(params or [])}
    with socket.create_connection((parsed.hostname, parsed.port), timeout=5) as sock:
        sock.sendall(json.dumps(payload).encode("utf-8") + b"\n")
        raw = b""
        while not raw.endswith(b"\n"):
            chunk = sock.recv(65536)
            if not chunk:
                raise AssertionError("Electrum server closed the connection")
            raw += chunk
    response = json.loads(raw.decode("utf-8"))
    if response.get("error"):
        raise AssertionError(f"Electrum {method} failed: {response['error']}")
    return response.get("result")


def wait_for_electrum_height(url: str, expected: int) -> None:
    """Wait until the Electrum server indexes exactly `expected` blocks."""
    deadline = time.monotonic() + 60
    last: Any = None
    while time.monotonic() < deadline:
        try:
            last = electrum_call(url, "blockchain.headers.subscribe")
            if int((last or {}).get("height") or 0) == expected:
                return
        except Exception as exc:  # noqa: BLE001 - keep polling until the deadline
            last = exc
        time.sleep(1)
    raise AssertionError(f"Electrum did not reach height {expected}: {last}")
