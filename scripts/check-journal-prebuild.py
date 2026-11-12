"""Verify native pricing on real SQLite/SQLCipher copies and record process CPU."""
from __future__ import annotations

import argparse
import ctypes
import json
import os
from pathlib import Path
import sqlite3
import statistics
import subprocess
import sys
import tempfile
import time

if sys.platform == "linux":
    import resource

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from kassiber.cli.handlers import auto_price_transactions_from_rates_cache
from tests.rust_port.journal_prebuild_oracle import capture, rate, seed, transaction, vectors
from tests.rust_port.journal_persistence_oracle import decode_scalar, dump
from tests.rust_port.journal_source_overlap_oracle import vectors as source_vectors
from tests.rust_port.journal_legacy_refresh_oracle import vectors as legacy_vectors
from tests.rust_port.journal_pipeline_oracle import vectors as pipeline_vectors


def deny_network():
    if sys.platform != "linux" or os.uname().machine != "x86_64":
        raise RuntimeError("the local syscall guard requires Linux x86_64")
    resource.setrlimit(resource.RLIMIT_CORE, (0, 0))
    class Filter(ctypes.Structure):
        _fields_ = [("code", ctypes.c_ushort), ("jt", ctypes.c_ubyte), ("jf", ctypes.c_ubyte), ("k", ctypes.c_uint32)]
    class Program(ctypes.Structure):
        _fields_ = [("length", ctypes.c_ushort), ("filter", ctypes.POINTER(Filter))]
    instructions = [Filter(0x20, 0, 0, 0)]
    for number in (41, 42, 44, 46, 53, 307):
        instructions.extend([Filter(0x15, 0, 1, number), Filter(0x06, 0, 0, 0x80000000)])
    instructions.append(Filter(0x06, 0, 0, 0x7FFF0000))
    filters = (Filter * len(instructions))(*instructions)
    program = Program(len(filters), filters)
    libc = ctypes.CDLL(None, use_errno=True)
    if libc.prctl(38, 1, 0, 0, 0) or libc.prctl(22, 2, ctypes.byref(program), 0, 0):
        raise OSError(ctypes.get_errno(), "install network syscall guard")


def native(binary, requests):
    result = subprocess.run([str(binary)], input="".join(json.dumps(v) + "\n" for v in requests),
                            text=True, capture_output=True, timeout=300, check=True, preexec_fn=deny_network)
    assert not result.stderr, result.stderr
    rows = [json.loads(line) for line in result.stdout.splitlines()]
    assert len(rows) == len(requests)
    return rows


def connection(path, cipher):
    if cipher:
        import sqlcipher3
        conn = sqlcipher3.connect(str(path))
        conn.execute("PRAGMA key='synthetic-prebuild-test-only'")
        conn.row_factory = sqlcipher3.Row
    else:
        conn = sqlite3.connect(path)
        conn.row_factory = sqlite3.Row
    return conn


def parity(binary, root, stage=None):
    families = {"pricing": vectors, "sources": source_vectors,
                "legacy": legacy_vectors, "pipeline": pipeline_vectors}
    cases = [dict(v, stage=name) for name, generate in families.items()
             if stage is None or stage == name for v in generate()]
    requests = []
    for cipher in (False, True):
        for i, case in enumerate(cases):
            path = root / f"parity-{cipher}-{i}.sqlite3"
            conn = connection(path, cipher)
            # Fixture construction has no durability requirement. This setting
            # is connection-local; the native replay reopens with FULL sync.
            conn.execute("PRAGMA synchronous=OFF")
            seed(conn)
            conn.execute("PRAGMA foreign_keys=OFF")
            conn.execute("BEGIN")
            manifest = json.loads((ROOT / "crates/store/fixtures/current-schema.json").read_text())
            for obj in manifest["objects"]:
                if obj[0] == "trigger": conn.execute(f'DROP TRIGGER "{obj[1]}"')
            for table, data in case["before"].items():
                conn.execute(f'DELETE FROM "{table}"')
                names = ",".join('"' + name + '"' for name in data["columns"])
                placeholders = ",".join("?" for _ in data["columns"])
                for row in data["rows"]:
                    values = [decode_scalar(v) for v in row]
                    conn.execute(f'INSERT INTO "{table}"({names}) VALUES({placeholders})', values)
            for obj in manifest["objects"]:
                if obj[0] == "trigger": conn.execute(obj[3])
            conn.commit()
            conn.close()
            request = dict(case, database=str(path), before=None, dump=False)
            if cipher:
                request["passphrase"] = "synthetic-prebuild-test-only"
            requests.append(request)
    rows = native(binary, requests)
    for row, request in zip(rows, requests, strict=True):
        assert row["verified"], request["name"]
        if request.get("stage") == "pipeline":
            assert row["result"] == request["result"], request["name"]
            assert row["projection_wire"] == request["wire_fields"], request["name"]
        elif request.get("stage") == "legacy":
            assert row["result"] == request["result"], request["name"]
        elif request.get("stage") == "sources":
            assert row["result"] == request["result"], request["name"]
            assert row["projection_wire"] == request["wire_fields"], request["name"]
        else:
            assert row["count"] == request["count"], request["name"]
        assert row["error"] == request["error"], request["name"]
    for request in requests:
        Path(request["database"]).unlink()
    return len(rows)


def benchmark(binary, root):
    results = []
    for cipher in (False, True):
        for activity in ("priced", "missing-cache", "cache-fill"):
            path = root / f"bench-{cipher}-{activity}.sqlite3"
            conn = connection(path, cipher)
            seed(conn)
            conn.execute("INSERT INTO settings(key,value) VALUES('freshness.policy.profile.p',?)",
                         ('{"source_classes":{"market_rates":true}}',))
            for i in range(20_000):
                transaction(conn, id=f"tx-{i:05}", amount=1000000000+i,
                            fiat_rate=50000 if activity == "priced" else None,
                            fiat_price_source="import" if activity == "priced" else None)
            if activity == "cache-fill": rate(conn)
            conn.commit()
            expected = capture(conn, activity)
            conn.close()
            python_cpu = []
            native_cpu = []
            for _ in range(5):
                conn = connection(path, cipher)
                conn.execute("BEGIN IMMEDIATE")
                start = time.process_time()
                count = auto_price_transactions_from_rates_cache(conn, {"id": "p", "fiat_currency": "EUR"})
                python_cpu.append(time.process_time() - start)
                assert count == expected["count"]
                assert dump(conn) == expected["expected"]
                conn.rollback()
                conn.close()
                request = dict(database=str(path), expected=expected["expected"], name=activity)
                if cipher: request["passphrase"] = "synthetic-prebuild-test-only"
                row = native(binary, [request])[0]
                assert row["verified"] and row["error"] is None and row["count"] == count
                native_cpu.append(row["cpu_seconds"])
            results.append(dict(driver="SQLCipher" if cipher else "SQLite", transactions=20_000,
                                activity=activity, python_cpu_seconds=python_cpu, native_cpu_seconds=native_cpu,
                                python_median=statistics.median(python_cpu), native_median=statistics.median(native_cpu)))
            path.unlink()
    return results


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--benchmark", action="store_true")
    parser.add_argument("--stage", choices=("pricing", "sources", "legacy", "pipeline"))
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    scratch = ROOT / "target/r4-10/scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=scratch, prefix="pricing-check-") as directory:
        root = Path(directory)
        probe = subprocess.run([sys.executable, "-c", "import socket; socket.socket()"], preexec_fn=deny_network)
        assert probe.returncode == -31, probe.returncode
        record = dict(boundary="ordered source repairs, drifted legacy pair refresh/profile reload, and cached pricing", full_dump_comparisons=parity(args.binary, root, args.stage),
                      network="native process killed on socket/connect/send syscalls; guard self-test passed")
        if args.benchmark: record["timings"] = benchmark(args.binary, root)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(record, indent=2) + "\n")
    print(json.dumps(record, indent=2))


if __name__ == "__main__":
    main()
