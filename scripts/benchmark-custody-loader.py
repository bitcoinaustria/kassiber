#!/usr/bin/env python3
"""Compare loaders on disposable journals-bench-books.py books, locally only.

Build custody-load with --release --features test-harness --offline first.
Times exclude open/BEGIN and v1 serialization; the native total includes its
temporary text-free conversion to today's custody snapshot. Every book first
passes the complete v1 byte comparison. Above load 8, alternate five pairs.
"""
from __future__ import annotations

import argparse
import gc
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT))
from kassiber.core.custody_snapshot import load_custody_snapshot
from kassiber.db import open_db


def measure(book: Path, binary: Path, as_of: str):
    metadata = json.loads((book / "bench-book.json").read_text())
    if metadata.get("schema") != "kassiber.journals-bench-book.v1":
        raise ValueError("requires a disposable generated benchmark book")
    root = (book / "data").resolve()
    conn = open_db(root, require_existing_schema=True)
    try:
        profile = dict(conn.execute("SELECT * FROM profiles").fetchone())

        def python():
            conn.execute("BEGIN IMMEDIATE")
            try:
                start = time.perf_counter()
                snapshot = load_custody_snapshot(conn, profile, as_of=as_of)
                seconds = time.perf_counter() - start
                return seconds, snapshot
            finally:
                conn.rollback()

        def rust(timing_only=True):
            request = {"root": str(root), "profile": profile, "as_of": as_of,
                       "timing_only": timing_only}
            result = subprocess.run([str(binary)], input=json.dumps(request).encode() + b"\n",
                                    capture_output=True, check=True, timeout=120)
            if result.stderr:
                raise RuntimeError("native loader probe wrote stderr")
            response = json.loads(result.stdout)
            if "error" in response or "adapter_error" in response:
                raise RuntimeError("native loader probe refused benchmark book")
            return response

        _, snapshot = python()
        expected = json.dumps(snapshot, ensure_ascii=True, allow_nan=True)
        actual = rust(False)
        if actual["v1"] != expected:
            raise AssertionError("native loader benchmark v1 differs")
        digest = hashlib.sha256(expected.encode("ascii")).hexdigest()
        size = len(expected)
        del snapshot, expected, actual
        gc.collect()
        pairs = []
        # Always retain five pairs for reproducibility on a shared machine.
        for index in range(5):
            load = os.getloadavg()
            if index % 2:
                native = rust()
                seconds, snapshot = python()
            else:
                seconds, snapshot = python()
                native = rust()
            del snapshot
            gc.collect()
            pair = {"load": load, "python": seconds, "rust": native["seconds"],
                    "adapter": native["adapter_seconds"],
                    "total": native["seconds"] + native["adapter_seconds"]}
            pairs.append(pair)
            print(json.dumps({"book": book.name, **pair}), flush=True)
        return {"kind": metadata["kind"], "transactions": metadata["transactions"],
                "book": metadata,
                "byte_equal": True, "sha256": digest, "bytes": size, "pairs": pairs}
    finally:
        conn.close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("books", nargs="+", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--as-of", default="2026-10-31T12:00:00Z")
    args = parser.parse_args()
    records = [measure(book, args.binary.resolve(), args.as_of) for book in args.books]
    document = {"schema": "kassiber.custody-loader-bench.v1",
                "as_of": args.as_of,
                "binary_sha256": hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                "records": records}
    args.out.write_text(json.dumps(document, ensure_ascii=True, indent=2) + "\n")


if __name__ == "__main__":
    main()
