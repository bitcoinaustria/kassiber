#!/usr/bin/env python3
"""Verify the release sidecar's real daemon/bundled-data smoke transcript."""

from __future__ import annotations

import argparse
import json
from pathlib import Path


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--jsonl", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--manifest", required=True, type=Path)
    return parser.parse_args()


def main() -> None:
    args = parse_args()
    records = [
        json.loads(line)
        for line in args.jsonl.read_text(encoding="utf-8").splitlines()
        if line.strip()
    ]
    ready = next((record for record in records if record.get("kind") == "daemon.ready"), None)
    if ready is None:
        raise SystemExit("sidecar smoke emitted no daemon.ready record")
    ready_data = ready.get("data") or {}
    if ready_data.get("version") != args.version:
        raise SystemExit(
            f"sidecar version mismatch: {ready_data.get('version')!r} != {args.version!r}"
        )
    expected_kinds = json.loads(args.manifest.read_text(encoding="utf-8"))
    if ready_data.get("supported_kinds") != expected_kinds:
        raise SystemExit("sidecar supported_kinds differ from the generated daemon contract")

    rates = next(
        (
            record
            for record in records
            if record.get("kind") == "ui.rates.kraken_csv.import"
            and record.get("request_id") == "kraken-bundled-1"
        ),
        None,
    )
    if rates is None or rates.get("error") is not None:
        raise SystemExit("sidecar bundled Kraken smoke did not complete successfully")
    rates_data = rates.get("data") or {}
    totals = rates_data.get("totals") or {}
    expected = {"bundled": True, "pairs": 2, "rows": 255_181}
    actual = {
        "bundled": rates_data.get("bundled"),
        "pairs": totals.get("pairs"),
        "rows": totals.get("rows"),
    }
    if actual != expected:
        raise SystemExit(f"sidecar bundled Kraken smoke mismatch: {actual!r}")

    shutdown = next(
        (
            record
            for record in records
            if record.get("kind") == "daemon.shutdown"
            and record.get("request_id") == "shutdown-1"
        ),
        None,
    )
    if shutdown is None or shutdown.get("error") is not None:
        raise SystemExit("sidecar smoke emitted no successful daemon.shutdown terminal")
    print(
        f"sidecar smoke verified version {args.version}, "
        f"{len(expected_kinds)} kinds, and bundled Kraken data"
    )


if __name__ == "__main__":
    main()
