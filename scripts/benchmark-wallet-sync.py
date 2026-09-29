#!/usr/bin/env python3
"""Reproducible wallet sync benchmark against a local regtest stack.

The benchmark builds one large descriptor wallet on regtest, then times the
steps a user waits for: the first sync, a no-op resync, a small incremental
sync, a forced full replay, and the journal step the desktop runs after every
user-triggered sync (exact automatic pairing plus the journal rebuild). Each
step runs the real CLI or core entry point in a child process, so the timings
include interpreter start-up exactly as a user sees it.

It needs bitcoind and Fulcrum on loopback; run it through the harness, which
starts both in a disposable per-worktree Compose project:

    ./scripts/integration-harness.sh sync-benchmark
    ./scripts/integration-harness.sh sync-benchmark --txs 5000 --addresses 1000
    ./scripts/integration-harness.sh sync-benchmark --smoke --profile-dir /tmp/kb-prof

Every Kassiber backend in the benchmark book is a regtest backend on a loopback
host, and child processes run under the test socket guard, so nothing leaves
the machine. Kassiber data lives in a temporary directory that is removed at
the end. Output is one JSON object per line so an interrupted run still leaves
machine-readable results. Timings are observations, not pass/fail thresholds.
"""

from __future__ import annotations

import argparse
import base64
import json
import os
import platform
import pstats
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

from tests.integration.test_live_bitcoin_electrum_parity import (  # noqa: E402
    _electrum_call,
    _wait_for_electrum,
)


LOOPBACK_HOSTS = {"127.0.0.1", "localhost", "::1"}
WALLET_LABEL = "Bench"
BACKEND_NAME = "electrum-regtest"
RPC_TIMEOUT = 600
SEND_ROUND = 500
FANOUT_SATS = 5_000_000
PROFILE_TOP = 25
SCHEMA_VERSION = 1

JOURNAL_STEP = """\
import json
import sys

from kassiber import daemon_freshness
from kassiber.db import open_db

conn = open_db(sys.argv[1])
try:
    profile_id = conn.execute("SELECT id FROM profiles ORDER BY rowid LIMIT 1").fetchone()[0]
    # The desktop's post-sync step: see daemon_freshness._wallets_sync_payload.
    result = daemon_freshness.refresh_journals_step(
        conn,
        str(profile_id),
        auto_pair=True,
        skip_rebuild_when_current=True,
    )
    conn.commit()
finally:
    conn.close()
print(json.dumps({"data": {key: value for key, value in result.items() if isinstance(value, (int, str, bool))}}))
"""

# `python -m cProfile` catches SystemExit to write its stats and then exits 0,
# which would report a failed step as a success. This keeps the exit code.
PROFILE_SHIM = """\
import cProfile
import runpy
import sys

out, kind, target, *argv = sys.argv[1:]
sys.argv = [target, *argv]
profiler = cProfile.Profile()
code = 0
profiler.enable()
try:
    if kind == "module":
        runpy.run_module(target, run_name="__main__", alter_sys=True)
    else:
        runpy.run_path(target, run_name="__main__")
except SystemExit as exc:
    code = exc.code if isinstance(exc.code, int) else (0 if exc.code is None else 1)
finally:
    profiler.disable()
    profiler.dump_stats(out)
sys.exit(code)
"""


def _emit(payload: dict[str, Any]) -> None:
    print(json.dumps(payload, sort_keys=True), flush=True)


def _require_loopback(url: str, what: str) -> None:
    parsed = parse.urlsplit(url if "://" in url else f"tcp://{url}")
    if (parsed.hostname or "") not in LOOPBACK_HOSTS:
        raise SystemExit(f"{what} must be a loopback URL, got {url!r}")


class CoreRpc:
    def __init__(self, url: str, username: str, password: str) -> None:
        _require_loopback(url, "Bitcoin Core RPC")
        self.url = url.rstrip("/")
        self.auth = base64.b64encode(f"{username}:{password}".encode()).decode("ascii")

    def _post(self, payload: Any, wallet: str | None) -> Any:
        endpoint = f"{self.url}/wallet/{wallet}" if wallet else self.url
        req = request.Request(
            endpoint,
            data=json.dumps(payload).encode("utf-8"),
            headers={"Content-Type": "application/json", "Authorization": f"Basic {self.auth}"},
        )
        with request.urlopen(req, timeout=RPC_TIMEOUT) as response:
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
            rpc.call("createwallet", [name, False, False, "", False, True, True])
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
                self.rpc.call("unloadwallet", [name])
            except Exception:
                pass

    def mine(self, blocks: int = 1) -> None:
        self.rpc.call("generatetoaddress", [blocks, self.mining])

    def fund_faucet(self, coins: int) -> None:
        # 110 blocks leave ten 50 BTC coinbases spendable; a fan-out keeps enough
        # confirmed coins that each round of sends needs no unconfirmed change.
        self.mine(110)
        outputs = {}
        for address in self.rpc.batch(
            [("getnewaddress", ["fanout", "bech32"]) for _ in range(coins)], wallet=self.faucet
        ):
            outputs[address] = _btc(FANOUT_SATS)
        self.rpc.call("sendmany", ["", outputs], wallet=self.faucet)
        self.mine()

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

    def __init__(self, data_root: Path, profile_dir: Path | None) -> None:
        self.data_root = data_root
        self.profile_dir = profile_dir
        self.journal_script = data_root.parent / "journal_step.py"
        self.journal_script.write_text(JOURNAL_STEP, encoding="utf-8")
        self.profile_shim = data_root.parent / "profile_shim.py"
        self.profile_shim.write_text(PROFILE_SHIM, encoding="utf-8")
        self.env = self._child_env(data_root.parent)

    @staticmethod
    def _child_env(scratch: Path) -> dict[str, str]:
        env = {
            key: value
            for key, value in os.environ.items()
            if not key.startswith(("KASSIBER_BACKEND_", "SATBOOKS_BACKEND_"))
            and key not in {"KASSIBER_DEFAULT_BACKEND", "SATBOOKS_DEFAULT_BACKEND", "KASSIBER_NO_EGRESS"}
        }
        # The socket guard is test-only and allows loopback; the product kill
        # switch would also refuse the loopback Fulcrum that BDK talks to.
        env["KASSIBER_TEST_NO_EGRESS"] = "1"
        env["KASSIBER_OFFLINE_PREFERENCE_FILE"] = str(scratch / "offline-preference.json")
        guard = REPO_ROOT / "tests" / "_egress_guard"
        env["PYTHONPATH"] = os.pathsep.join(
            part for part in (str(guard), str(REPO_ROOT), env.get("PYTHONPATH", "")) if part
        )
        return env

    def _spawn(self, argv: list[str], stage: str | None) -> tuple[dict[str, Any], float, float]:
        command = [sys.executable, *argv]
        if stage and self.profile_dir is not None:
            kind, target = ("module", argv[1]) if argv[0] == "-m" else ("path", argv[0])
            rest = argv[2:] if kind == "module" else argv[1:]
            command = [
                sys.executable, str(self.profile_shim), str(self.profile_dir / f"{stage}.prof"), kind, target, *rest,
            ]
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
            raise RuntimeError(f"{' '.join(argv[:6])} failed ({process.returncode})\n{stdout[-2000:]}\n{stderr[-4000:]}")
        # ru_maxrss is KiB on Linux and bytes on macOS.
        rss_mb = usage.ru_maxrss / (1024 * 1024 if platform.system() == "Darwin" else 1024)
        return json.loads(stdout) if stdout.strip() else {}, seconds, rss_mb

    def cli(self, *args: str, stage: str | None = None) -> tuple[dict[str, Any], float, float]:
        return self._spawn(["-m", "kassiber", "--data-root", str(self.data_root), "--machine", *args], stage)

    def journal(self, stage: str) -> tuple[dict[str, Any], float, float]:
        return self._spawn([str(self.journal_script), str(self.data_root)], stage)

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


def _profile_top(path: Path) -> list[dict[str, Any]]:
    stats = pstats.Stats(str(path))
    rows = []
    for (filename, line, name), (_cc, calls, tottime, cumtime, _callers) in stats.stats.items():  # type: ignore[attr-defined]
        rows.append({
            "function": f"{os.path.relpath(filename, REPO_ROOT) if filename.startswith(str(REPO_ROOT)) else filename}:{line}:{name}",
            "calls": calls,
            "tottime": round(tottime, 4),
            "cumtime": round(cumtime, 4),
        })
    rows.sort(key=lambda row: row["tottime"], reverse=True)
    return rows[:PROFILE_TOP]


def _sync_fields(payload: dict[str, Any]) -> dict[str, Any]:
    data = payload.get("data")
    if isinstance(data, list):
        data = data[0] if data else {}
    return {key: value for key, value in (data or {}).items() if isinstance(value, (int, float, str, bool))}


def _run_stage(book: Book, stage: str, action) -> dict[str, Any]:
    payload, seconds, rss_mb = action(stage)
    result: dict[str, Any] = {
        "event": "benchmark_result",
        "schema_version": SCHEMA_VERSION,
        "stage": stage,
        "ok": True,
        "seconds": round(seconds, 3),
        "peak_rss_mb": round(rss_mb, 1),
        "output": _sync_fields(payload),
    }
    if book.profile_dir is not None:
        result["profile_top_tottime"] = _profile_top(book.profile_dir / f"{stage}.prof")
    _emit(result)
    return result


def main(argv: Sequence[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--txs", type=int, default=2000, help="incoming payments to the wallet")
    parser.add_argument("--addresses", type=int, default=400, help="receive addresses the payments rotate over")
    parser.add_argument("--spend-ratio", type=float, default=0.1, help="outgoing payments per incoming one")
    parser.add_argument("--increment", type=int, default=25, help="payments added before the incremental sync")
    parser.add_argument("--noop-runs", type=int, default=3)
    parser.add_argument("--profile-dir", type=Path, help="write a cProfile file per stage and report the hottest functions")
    parser.add_argument("--smoke", action="store_true", help="tiny sizes to check the harness quickly")
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
    if args.profile_dir is not None:
        args.profile_dir.mkdir(parents=True, exist_ok=True)

    spends = int(args.txs * args.spend_ratio)
    _emit({
        "event": "benchmark_start",
        "schema_version": SCHEMA_VERSION,
        "txs": args.txs,
        "addresses": args.addresses,
        "spends": spends,
        "python": platform.python_version(),
        "platform": platform.platform(),
    })

    chain = Chain(rpc, electrum_url, args.addresses)
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

        book = Book(scratch / "data", args.profile_dir)
        book.create(electrum_url, chain.receive_descriptor, chain.change_descriptor)
        sync = lambda *extra: (lambda stage: book.cli("wallets", "sync", "--wallet", WALLET_LABEL, *extra, stage=stage))

        _run_stage(book, "sync_cold", sync())
        _run_stage(book, "journal_cold", book.journal)
        _emit({"event": "benchmark_book", "schema_version": SCHEMA_VERSION, **book.stats()})
        for run in range(args.noop_runs):
            _run_stage(book, f"sync_noop_{run + 1}", sync())
            _run_stage(book, f"journal_noop_{run + 1}", book.journal)
        if args.increment:
            chain.receive_payments(args.increment)
            chain.wait_indexed()
            _run_stage(book, "sync_incremental", sync())
            _run_stage(book, "journal_incremental", book.journal)
        _run_stage(book, "sync_force_full", sync("--force-full"))
        _run_stage(book, "journal_force_full", book.journal)
        _emit({"event": "benchmark_book", "schema_version": SCHEMA_VERSION, **book.stats()})
    finally:
        chain.close()
        shutil.rmtree(scratch, ignore_errors=True)
    _emit({"event": "benchmark_complete", "schema_version": SCHEMA_VERSION, "ok": True})
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
