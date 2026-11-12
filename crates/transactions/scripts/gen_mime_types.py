#!/usr/bin/env python3
"""Freeze CPython 3.11's built-in table; host overrides are loaded at runtime."""
import argparse
import json
import mimetypes
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("--check", action="store_true")
args = parser.parse_args()
path = Path(__file__).resolve().parents[1] / "src" / "mime-types.json"
expected = json.dumps(mimetypes.MimeTypes().types_map[1], sort_keys=True, indent=2) + "\n"
if args.check:
    if path.read_text() != expected:
        raise SystemExit("CPython MIME table differs; regenerate with the locked Python 3.11 environment")
else:
    path.write_text(expected)
