#!/usr/bin/env python3
"""Reproducible wallet sync benchmark against a local regtest stack.

The benchmark builds one large descriptor wallet on regtest, then times the
steps a user waits for: the first sync, a no-op resync, a small incremental
sync, a forced full replay, and the journal step after each (`journals
process`). Each step runs the Rust `kassiber` binary in a child process
(`KASSIBER_TEST_CLI` or `KASSIBER_RUST_BIN`), so the timings include start-up
exactly as a user sees it.

It needs bitcoind and Fulcrum on loopback; run it through the harness, which
starts both in a disposable per-worktree Compose project:

    ./scripts/integration-harness.sh sync-benchmark
    ./scripts/integration-harness.sh sync-benchmark --txs 5000 --addresses 1000
    ./scripts/integration-harness.sh sync-benchmark --smoke
    ./scripts/integration-harness.sh sync-benchmark --runtimes rust,rust
    ./scripts/integration-harness.sh sync-benchmark --sync-path daemon

`--sync-path daemon` times the desktop's `ui.wallets.sync` freshness job
through `kassiber daemon` instead of the CLI's `wallets sync`.

`--runtimes rust,rust` syncs two books from the same chain, each stage on
every book in turn, and compares the final books table by table
(`tests/integration/runtime_parity.py`): the control for differences between
books alone. The script itself is standard-library Python (`python3 -I`).

Every Kassiber backend in the benchmark book is a regtest backend on a loopback
host, and a test-harness build runs under its test socket guard, so nothing
leaves the machine. Kassiber data lives in a temporary directory that is
removed at the end. Output is one JSON object per line so an interrupted run
still leaves machine-readable results. Timings are observations, not
pass/fail thresholds.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import platform
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import uuid
from collections.abc import Sequence
from decimal import Decimal
from pathlib import Path
from typing import Any
from urllib import parse, request


REPO_ROOT = Path(__file__).resolve().parents[1]
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from tests.integration import cli  # noqa: E402
from tests.integration.book import electrum_call as _electrum_call  # noqa: E402


LOOPBACK_HOSTS = {"127.0.0.1", "localhost", "::1"}
WALLET_LABEL = "Bench"
BACKEND_NAME = "electrum-regtest"
RPC_TIMEOUT = 600
SEND_ROUND = 500
FANOUT_SATS = 5_000_000
# Coinbases mature after 100 blocks; mine in steps until the faucet can fund.
MATURITY_BLOCKS = 101
FUNDING_STEP_BLOCKS = 50
FUNDING_MAX_BLOCKS = 3_000
PROXY_VARIABLES = {"http_proxy", "https_proxy", "all_proxy", "HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY"}
SCHEMA_VERSION = 1

def _emit(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, sort_keys=True), flush=True)


def _wait_for_electrum(url: str, *, min_height: int, txids: list[str]) -> None:
    """Wait until Fulcrum serves `min_height` and every txid in `txids`."""
    deadline = time.monotonic() + 180
    last_error: Exception | None = None
    while time.monotonic() < deadline:
        try:
            header = _electrum_call(url, "blockchain.headers.subscribe")
            if int((header or {}).get("height") or 0) >= min_height:
                for txid in txids:
                    _electrum_call(url, "blockchain.transaction.get", [txid])
                return
        except (OSError, AssertionError, ValueError) as exc:  # Fulcrum may still be indexing.
            last_error = exc
        time.sleep(2)
    raise RuntimeError(f"Fulcrum at {url} did not reach height {min_height}: {last_error}")


def _guard_capable(prefix: Sequence[str]) -> bool:
    """Whether the binary carries the test socket guard (a test-harness build).

    A release build refuses an armed `KASSIBER_TEST_NO_EGRESS` with exit 78.
    """
    env = {key: value for key, value in os.environ.items() if not key.startswith("KASSIBER_")}
    env["KASSIBER_TEST_NO_EGRESS"] = "1"
    result = subprocess.run([*prefix, "--version"], env=env, capture_output=True, timeout=60)
    return result.returncode != 78


def _require_loopback(url: str, what: str) -> None:
    parsed = parse.urlsplit(url if "://" in url else f"tcp://{url}")
    if (parsed.hostname or "") not in LOOPBACK_HOSTS:
        raise SystemExit(f"{what} must be a loopback URL, got {url!r}")


class _RefuseRedirects(request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        raise RuntimeError(f"Bitcoin Core RPC redirected to {newurl!r}; refusing to follow")


class CoreRpc:
    def __init__(self, url: str, username: str, password: str) -> None:
        _require_loopback(url, "Bitcoin Core RPC")
        self.url = url.rstrip("/")
        self.auth = base64.b64encode(f"{username}:{password}".encode()).decode("ascii")
        # Ambient proxy settings must never carry the RPC password off the host.
        self._opener = request.build_opener(request.ProxyHandler({}), _RefuseRedirects())

    def _post(self, payload: Any, wallet: str | None) -> Any:
        endpoint = f"{self.url}/wallet/{wallet}" if wallet else self.url
        req = request.Request(
            endpoint,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json", "Authorization": f"Basic {self.auth}"},
        )
        with self._opener.open(req, timeout=RPC_TIMEOUT) as response:
            return json.loads(response.read().decode("utf-8"))

    def call(self, method: str, params: Sequence[Any] = (), *, wallet: str | None = None) -> Any:
        decoded = self._post({"jsonrpc": "1.0", "id": method, "method": method, "params": list(params)}, wallet)
        if decoded.get("error"):
            raise RuntimeError(f"RPC {method} failed: {decoded['error']}")
        return decoded.get("result")

    def batch(self, calls: Sequence[tuple[str, Sequence[Any]]], *, wallet: str | None = None) -> list[Any]:
        if not calls:
            return []
        payload = [
            {"jsonrpc": "1.0", "id": index, "method": method, "params": list(params)}
            for index, (method, params) in enumerate(calls)
        ]
        responses = sorted(self._post(payload, wallet), key=lambda row: row["id"])
        errors = [row["error"] for row in responses if row.get("error")]
        if errors:
            raise RuntimeError(f"RPC batch failed: {errors[0]} ({len(errors)} errors)")
        return [row["result"] for row in responses]


def _btc(sats: int) -> str:
    return str(Decimal(sats) / Decimal(100_000_000))


class Chain:
    """A faucet and an owner wallet in the regtest node."""

    def __init__(self, rpc: CoreRpc, electrum_url: str, addresses: int) -> None:
        self.rpc = rpc
        self.electrum_url = electrum_url
        run_id = uuid.uuid4().hex[:12]
        self.faucet = f"kassiber-bench-faucet-{run_id}"
        self.owner = f"kassiber-bench-owner-{run_id}"
        for name in (self.faucet, self.owner):
            # load_on_startup=False: a kept node must not reload old runs' wallets.
            rpc.call("createwallet", [name, False, False, "", False, True, False])
        self.mining = rpc.call("getnewaddress", ["mining", "bech32"], wallet=self.faucet)
        self.receive = rpc.batch(
            [("getnewaddress", [f"receive {index}", "bech32"]) for index in range(addresses)],
            wallet=self.owner,
        )
        descriptors = rpc.call("listdescriptors", [False], wallet=self.owner)["descriptors"]
        self.receive_descriptor = next(
            row["desc"] for row in descriptors
            if row.get("active") and not row.get("internal") and row["desc"].startswith("wpkh(")
        )
        self.change_descriptor = next(
            row["desc"] for row in descriptors
            if row.get("active") and row.get("internal") and row["desc"].startswith("wpkh(")
        )
        self.next_receive = 0
        self.last_txid = ""

    def close(self) -> None:
        for name in (self.faucet, self.owner):
            try:
                self.rpc.call("unloadwallet", [name, False])
            except Exception:
                pass

    def mine(self, blocks: int = 1) -> None:
        self.rpc.call("generatetoaddress", [blocks, self.mining])

    def fund_faucet(self, coins: int) -> None:
        # A fan-out keeps enough confirmed coins that each round of sends needs
        # no unconfirmed change. A reused chain pays smaller subsidies after
        # each 150-block halving, so mine until the faucet can afford it.
        needed = coins * FANOUT_SATS + 100_000_000
        self.mine(MATURITY_BLOCKS)
        mined = MATURITY_BLOCKS
        while self._spendable_sats() < needed:
            if mined >= FUNDING_MAX_BLOCKS:
                raise RuntimeError(
                    "regtest subsidy too small to fund the faucet; recreate the stack without "
                    "KASSIBER_REGTEST_REUSE_CORE"
                )
            self.mine(FUNDING_STEP_BLOCKS)
            mined += FUNDING_STEP_BLOCKS
        outputs = {}
        for address in self.rpc.batch(
            [("getnewaddress", ["fanout", "bech32"]) for _ in range(coins)], wallet=self.faucet
        ):
            outputs[address] = _btc(FANOUT_SATS)
        self.rpc.call("sendmany", ["", outputs], wallet=self.faucet)
        self.mine()

    def _spendable_sats(self) -> int:
        trusted = self.rpc.call("getbalances", wallet=self.faucet)["mine"]["trusted"]
        return int(Decimal(str(trusted)) * 100_000_000)

    def _send_rounds(self, wallet: str, sends: list[tuple[str, int]]) -> None:
        for start in range(0, len(sends), SEND_ROUND):
            chunk = sends[start:start + SEND_ROUND]
            txids = self.rpc.batch(
                [("sendtoaddress", [address, _btc(sats)]) for address, sats in chunk],
                wallet=wallet,
            )
            self.last_txid = txids[-1]
            self.mine()

    def receive_payments(self, count: int) -> None:
        sends = []
        for index in range(count):
            address = self.receive[(self.next_receive + index) % len(self.receive)]
            sends.append((address, 100_000 + (index % 97) * 10_000))
        self.next_receive += count
        self._send_rounds(self.faucet, sends)

    def spend_payments(self, count: int) -> None:
        destinations = self.rpc.batch(
            [("getnewaddress", ["payee", "bech32"]) for _ in range(count)], wallet=self.faucet
        )
        self._send_rounds(
            self.owner,
            [(address, 50_000 + (index % 53) * 5_000) for index, address in enumerate(destinations)],
        )

    def wait_indexed(self) -> int:
        height = int(self.rpc.call("getblockcount"))
        _wait_for_electrum(self.electrum_url, min_height=height, txids=[self.last_txid])
        deadline = time.monotonic() + 120
        while time.monotonic() < deadline:
            try:
                proof = _electrum_call(self.electrum_url, "blockchain.transaction.get_merkle", [self.last_txid, height])
                if int((proof or {}).get("block_height") or 0) == height:
                    return height
            except Exception:
                pass
            time.sleep(1)
        raise RuntimeError(f"Fulcrum did not index {self.last_txid} at height {height}")


class Book:
    """One disposable Kassiber data root with the benchmark wallet."""

    def __init__(self, data_root: Path, label: str) -> None:
        self.data_root = data_root
        self.label = label
        self.last_cpu_seconds = 0.0
        self.prefix = list(cli.prefix())
        data_root.parent.mkdir(parents=True, exist_ok=True)
        self.env = self._child_env(data_root.parent)
        # A release build has no test socket guard and refuses it armed;
        # every backend it can reach is a checked loopback URL.
        if not _guard_capable(self.prefix):
            self.env.pop("KASSIBER_TEST_NO_EGRESS", None)

    @staticmethod
    def _child_env(scratch: Path) -> dict[str, str]:
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("KASSIBER_BACKEND_", "SATBOOKS_BACKEND_"))
            and key not in {"KASSIBER_DEFAULT_BACKEND", "SATBOOKS_DEFAULT_BACKEND", "KASSIBER_NO_EGRESS"}
            and key not in PROXY_VARIABLES
        }
        # The socket guard is test-only and allows loopback; the product kill
        # switch would also refuse the loopback Fulcrum that BDK talks to.
        env["KASSIBER_TEST_NO_EGRESS"] = "1"
        env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(scratch / "offline-preference.json")
        return env

    def cli(self, *args: str) -> tuple[dict[str, Any], float, float]:
        command = [*self.prefix, "--data-root", str(self.data_root), "--machine", *args]
        with tempfile.TemporaryFile() as out, tempfile.TemporaryFile() as err:
            started = time.perf_counter()
            process = subprocess.Popen(command, cwd=REPO_ROOT, env=self.env, stdout=out, stderr=err)
            _pid, status, usage = os.wait4(process.pid, 0)
            seconds = time.perf_counter() - started
            process.returncode = os.waitstatus_to_exitcode(status)
            out.seek(0)
            err.seek(0)
            stdout, stderr = out.read().decode("utf-8"), err.read().decode("utf-8")
        if process.returncode != 0:
            raise RuntimeError(f"kassiber {' '.join(args[:4])} failed ({process.returncode})\n{stdout[-2000:]}\n{stderr[-4000:]}")
        # ru_maxrss is KiB on Linux and bytes on macOS.
        self.last_cpu_seconds = usage.ru_utime + usage.ru_stime
        rss_mb = usage.ru_maxrss / (1024 * 1024 if platform.system() == "Darwin" else 1024)
        return json.loads(stdout) if stdout.strip() else {}, seconds, rss_mb

    def daemon_sync(self, *, force_full: bool = False) -> tuple[dict[str, Any], float, float]:
        """`ui.wallets.sync` through `kassiber daemon`: the desktop freshness
        queue, whose per-request source checks differ from the CLI's.

        Closing stdin shuts the daemon down and cancels a running job, so
        stdin stays open until the request's terminal record arrives. The
        timing covers start-up, the whole job and shutdown, as `cli` does.
        """
        request_id = "bench-sync"
        request = {
            "request_id": request_id,
            "kind": "ui.wallets.sync",
            "args": {"wallet": WALLET_LABEL, "force_full": force_full, "process_journals": False},
        }
        command = [*self.prefix, "--data-root", str(self.data_root), "daemon"]
        terminal: dict[str, Any] | None = None
        tail: list[str] = []
        with tempfile.TemporaryFile() as err:
            started = time.perf_counter()
            process = subprocess.Popen(
                command, cwd=REPO_ROOT, env=self.env, stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=err, text=True, encoding="utf-8",
            )
            assert process.stdin is not None and process.stdout is not None
            process.stdin.write(json.dumps(request) + "\n")
            process.stdin.flush()
            for line in process.stdout:
                tail = (tail + [line])[-20:]
                try:
                    record = json.loads(line)
                except json.JSONDecodeError:
                    continue
                if record.get("request_id") != request_id:
                    continue
                if record.get("kind") == "error" or "error" in record or record.get("kind") == "ui.wallets.sync":
                    terminal = record
                    break
            process.stdin.close()
            process.stdout.read()
            _pid, status, usage = os.wait4(process.pid, 0)
            seconds = time.perf_counter() - started
            err.seek(0)
            stderr = err.read().decode("utf-8", "replace")
        if terminal is None:
            raise RuntimeError(f"daemon ui.wallets.sync gave no terminal record\n{''.join(tail)}\n{stderr[-4000:]}")
        if terminal.get("kind") != "ui.wallets.sync":
            raise RuntimeError(f"daemon ui.wallets.sync failed: {json.dumps(terminal.get('error'))[:2000]}")
        if os.waitstatus_to_exitcode(status) != 0:
            raise RuntimeError(f"kassiber daemon exited {os.waitstatus_to_exitcode(status)}\n{stderr[-4000:]}")
        self.last_cpu_seconds = usage.ru_utime + usage.ru_stime
        rss_mb = usage.ru_maxrss / (1024 * 1024 if platform.system() == "Darwin" else 1024)
        return {"data": terminal.get("data") or {}}, seconds, rss_mb

    def journal(self, _stage: str) -> tuple[dict[str, Any], float, float]:
        return self.cli("journals", "process")
    def create(self, electrum_url: str, receive: str, change: str) -> None:
        with tempfile.TemporaryDirectory(prefix="kassiber-bench-descriptors-") as tmp:
            receive_path = Path(tmp) / "receive.txt"
            change_path = Path(tmp) / "change.txt"
            receive_path.write_text(receive + "\n", encoding="utf-8")
            change_path.write_text(change + "\n", encoding="utf-8")
            self.cli("init")
            self.cli("workspaces", "create", "Bench")
            self.cli(
                "profiles", "create", "Default", "--workspace", "Bench", "--fiat-currency", "EUR",
                "--tax-country", "generic", "--gains-algorithm", "FIFO",
            )
            self.cli(
                "backends", "create", BACKEND_NAME, "--kind", "electrum", "--url", electrum_url,
                "--chain", "bitcoin", "--network", "regtest", "--timeout", "60",
            )
            self._pin_regtest_backends()
            # One manual rate before the chain starts prices every row locally.
            self.cli("rates", "set", "BTC-EUR", "2009-01-03T00:00:00Z", "50000")
            self.cli(
                "wallets", "create", "--workspace", "Bench", "--profile", "Default", "--label", WALLET_LABEL,
                "--kind", "descriptor", "--backend", BACKEND_NAME, "--chain", "bitcoin", "--network", "regtest",
                "--descriptor-file", str(receive_path), "--change-descriptor-file", str(change_path),
            )

    def _pin_regtest_backends(self) -> None:
        self.cli("backends", "set-default", BACKEND_NAME)
        for backend in self.cli("backends", "list")[0]["data"]:
            name = str(backend.get("name") or "")
            network = str(backend.get("network") or "").lower()
            if name != BACKEND_NAME and str(backend.get("source") or "").lower() == "database" and network != "regtest":
                self.cli("backends", "delete", name)
        for backend in self.cli("backends", "list")[0]["data"]:
            url = str(backend.get("url") or "")
            if str(backend.get("network") or "").lower() != "regtest" or not url:
                raise SystemExit(f"benchmark book keeps a non-regtest backend: {backend.get('name')!r}")
            _require_loopback(url, f"backend {backend.get('name')!r}")

    def stats(self) -> dict[str, Any]:
        conn = sqlite3.connect(f"file:{self._db_path()}?mode=ro", uri=True)
        try:
            rows, raw_bytes, raw_max = conn.execute(
                "SELECT COUNT(*), COALESCE(SUM(LENGTH(raw_json)), 0), COALESCE(MAX(LENGTH(raw_json)), 0) FROM transactions"
            ).fetchone()
            owned_bytes = 0
            for (raw,) in conn.execute("SELECT raw_json FROM transactions"):
                owned = json.loads(raw or "{}").get("observer_owned_scripts")
                if owned:
                    owned_bytes += len(json.dumps(owned))
            observer_state = conn.execute(
                "SELECT COALESCE(SUM(LENGTH(state_json)), 0) FROM chain_observer_instances"
            ).fetchone()[0]
            utxos = conn.execute("SELECT COUNT(*) FROM wallet_utxos WHERE spent_at IS NULL").fetchone()[0]
        finally:
            conn.close()
        return {
            "transaction_rows": rows,
            "unspent_utxos": utxos,
            "raw_json_bytes": raw_bytes,
            "raw_json_max_bytes": raw_max,
            "observer_owned_scripts_bytes": owned_bytes,
            "observer_state_bytes": observer_state,
            "database_bytes": sum(path.stat().st_size for path in self._db_path().parent.glob(self._db_path().name + "*")),
        }

    def _db_path(self) -> Path:
        matches = sorted(self.data_root.rglob("*.sqlite3")) + sorted(self.data_root.rglob("*.db"))
        if not matches:
            raise RuntimeError(f"no database under {self.data_root}")
        return max(matches, key=lambda path: path.stat().st_size)


def _sync_fields(payload: dict[str, Any]) -> dict[str, Any]:
    data = payload.get("data")
    if isinstance(data, list):
        data = data[0] if data else {}
    return {key: value for key, value in (data or {}).items() if isinstance(value, (int, float, str, bool))}


def _error_codes(value: Any) -> list[str]:
    """Every `error.code` in a result, depth first: a daemon job reports its
    failure inside a successful envelope."""
    codes: list[str] = []
    if isinstance(value, dict):
        error = value.get("error")
        if isinstance(error, dict) and isinstance(error.get("code"), str):
            codes.append(error["code"])
        for item in value.values():
            codes.extend(_error_codes(item))
    elif isinstance(value, list):
        for item in value:
            codes.extend(_error_codes(item))
    return codes


def _run_stage(book: Book, stage: str, action) -> dict[str, Any]:
    payload, seconds, rss_mb = action(stage)
    output = _sync_fields(payload)
    errors = sorted(set(_error_codes(payload.get("data"))))
    result: dict[str, Any] = {
        "event": "benchmark_result",
        "schema_version": SCHEMA_VERSION,
        "stage": stage,
        "book": book.label,
        "ok": output.get("ok") is not False and not errors,
        "errors": errors,
        "seconds": round(seconds, 3),
        "peak_rss_mb": round(rss_mb, 1),
        "cpu_seconds": round(book.last_cpu_seconds, 3),
        "output": output,
    }
    _emit(result)
    return result


def _parity(books: list[Book]) -> dict[str, Any]:
    """Compare every book with the first, identity- and order-blind.

    Each book created its own rows, so random identities and clocks are
    masked and every list is compared as a multiset.
    """
    from datetime import datetime, timezone

    from tests.integration.runtime_parity import Masker, canonical, masked_book

    now = datetime.now(timezone.utc)
    dumps = [
        canonical(masked_book(book.data_root, Masker(set(), now, anonymous=True))) for book in books
    ]
    differing: dict[str, list[str]] = {}
    details: dict[str, dict[str, Any]] = {}
    for book, dump in zip(books[1:], dumps[1:]):
        tables = sorted(table for table in dumps[0] if dumps[0][table] != dump.get(table))
        differing[book.label] = tables
        for table in tables:
            want, got = dumps[0][table][1], dump[table][1]
            first = next(((a, b) for a, b in zip(want, got) if a != b), (None, None))
            details[f"{book.label}:{table}"] = {
                "rows": [len(want), len(got)],
                "columns": dumps[0][table][0],
                "first_difference": [str(first[0])[:1500], str(first[1])[:1500]],
            }
    return {
        "event": "benchmark_parity",
        "schema_version": SCHEMA_VERSION,
        "reference": books[0].label,
        "differing_tables": differing,
        "details": details,
        "ok": not any(differing.values()),
    }


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--txs", type=int, default=2000, help="incoming payments to the wallet")
    parser.add_argument("--addresses", type=int, default=400, help="receive addresses the payments rotate over")
    parser.add_argument("--spend-ratio", type=float, default=0.1, help="outgoing payments per incoming one")
    parser.add_argument("--increment", type=int, default=25, help="payments added before the incremental sync")
    parser.add_argument("--noop-runs", type=int, default=3)
    parser.add_argument("--smoke", action="store_true", help="tiny sizes to check the harness quickly")
    parser.add_argument("--runtimes", default="rust",
                        help="comma-separated runtimes, one book each, synced from the same chain: only rust "
                             "(`rust,rust` compares two books, the control)")
    parser.add_argument("--sync-path", choices=("cli", "daemon"), default="cli",
                        help="`wallets sync` (cli) or the desktop's `ui.wallets.sync` freshness job (daemon)")
    args = parser.parse_args(argv)
    if args.smoke:
        args.txs, args.addresses, args.increment, args.noop_runs = 60, 20, 5, 1

    core_url = os.environ.get("KASSIBER_REGTEST_CORE_URL", "http://127.0.0.1:18443")
    electrum_url = os.environ.get("KASSIBER_REGTEST_ELECTRUM_URL") or (
        f"tcp://127.0.0.1:{os.environ.get('KASSIBER_REGTEST_BITCOIN_ELECTRUM_PORT', '18543')}"
    )
    _require_loopback(electrum_url, "Electrum")
    rpc = CoreRpc(
        core_url,
        os.environ.get("KASSIBER_REGTEST_RPC_USER", "kassiber"),
        os.environ.get("KASSIBER_REGTEST_RPC_PASSWORD", "kassiber"),
    )

    spends = int(args.txs * args.spend_ratio)
    runtimes = [part.strip() for part in args.runtimes.split(",") if part.strip()]
    if not runtimes or set(runtimes) != {"rust"}:
        raise SystemExit("--runtimes takes only rust: the Python runtime is gone (rust,rust is the control)")
    try:
        version = subprocess.run([*cli.prefix(), "--version"], capture_output=True, text=True, timeout=60).stdout.strip()
    except cli.CandidateConfigError as exc:
        raise SystemExit(str(exc)) from exc
    _emit({
        "event": "benchmark_start",
        "schema_version": SCHEMA_VERSION,
        "txs": args.txs,
        "addresses": args.addresses,
        "spends": spends,
        "sync_path": args.sync_path,
        "kassiber": version,
        "platform": platform.platform(),
    })

    chain = Chain(rpc, electrum_url, args.addresses)
    # A daemon job reports failure inside a successful envelope, so a
    # failed stage still times; the run as a whole must not pass.
    failed: list[str] = []
    scratch = Path(tempfile.mkdtemp(prefix="kassiber-sync-bench-"))
    try:
        started = time.perf_counter()
        chain.fund_faucet(min(max(args.txs, 50), 1_000))
        chain.receive_payments(args.txs)
        if spends:
            chain.spend_payments(spends)
        height = chain.wait_indexed()
        _emit({
            "event": "benchmark_chain_ready",
            "schema_version": SCHEMA_VERSION,
            "height": height,
            "seconds": round(time.perf_counter() - started, 3),
        })

        # Two books over one chain show what differs between books alone.
        books = [
            Book(scratch / (f"{index}-rust" if len(runtimes) > 1 else "") / "data", f"{index}-rust")
            for index in range(len(runtimes))
        ]
        for book in books:
            book.create(electrum_url, chain.receive_descriptor, chain.change_descriptor)

        def stage(name: str, *extra: str, journal: bool = False) -> None:
            for book in books:
                if journal:
                    result = _run_stage(book, name, book.journal)
                elif args.sync_path == "daemon":
                    result = _run_stage(book, name, lambda _s, book=book: book.daemon_sync(
                        force_full="--force-full" in extra))
                else:
                    result = _run_stage(book, name, lambda _s, book=book: book.cli(
                        "wallets", "sync", "--wallet", WALLET_LABEL, *extra))
                if not result["ok"]:
                    failed.append(f"{book.label}:{name}")

        def stats() -> None:
            for book in books:
                _emit({"event": "benchmark_book", "schema_version": SCHEMA_VERSION, "book": book.label,
                       **book.stats()})

        stage("sync_cold")
        stage("journal_cold", journal=True)
        stats()
        for run in range(args.noop_runs):
            stage(f"sync_noop_{run + 1}")
            stage(f"journal_noop_{run + 1}", journal=True)
        if args.increment:
            chain.receive_payments(args.increment)
            chain.wait_indexed()
            stage("sync_incremental")
            stage("journal_incremental", journal=True)
        stage("sync_force_full", "--force-full")
        stage("journal_force_full", journal=True)
        stats()
        if len(books) > 1:
            _emit(_parity(books))
    finally:
        chain.close()
        shutil.rmtree(scratch, ignore_errors=True)
    _emit({"event": "benchmark_complete", "schema_version": SCHEMA_VERSION, "ok": not failed,
           "failed_stages": failed})
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(main())
