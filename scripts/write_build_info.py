#!/usr/bin/env python3
"""Write package identity metadata for frozen builds."""

from __future__ import annotations

import argparse
import json
import os
from datetime import datetime, timezone
from pathlib import Path


def resolve_built_at(explicit: str | None) -> str:
    if explicit:
        return explicit
    source_date_epoch = os.environ.get("SOURCE_DATE_EPOCH")
    if source_date_epoch is not None:
        try:
            epoch = int(source_date_epoch)
        except ValueError as exc:
            raise ValueError("SOURCE_DATE_EPOCH must be an integer Unix timestamp") from exc
        if epoch < 0:
            raise ValueError("SOURCE_DATE_EPOCH must not be negative")
        return datetime.fromtimestamp(epoch, timezone.utc).isoformat().replace("+00:00", "Z")
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--ref", required=True)
    parser.add_argument("--run-id", default="local")
    parser.add_argument("--channel", choices=("dev", "prerelease", "release"), required=True)
    parser.add_argument("--built-at")
    args = parser.parse_args()

    try:
        built_at = resolve_built_at(args.built_at)
    except ValueError as exc:
        parser.error(str(exc))
    payload = {
        "schema_version": 1,
        "version": args.version,
        "channel": args.channel,
        "commit": args.commit,
        "ref": args.ref,
        "run_id": args.run_id,
        "built_at": built_at,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
