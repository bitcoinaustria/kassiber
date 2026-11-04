#!/usr/bin/env python3
"""Freeze Python's update-check decisions for the native release crate.

Covers version parsing and ordering, release selection over GitHub payloads
(stable and prerelease channels) and the human status text. No network.
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber import update_check as uc

DESTINATION = ROOT / "crates/release/tests/data/vectors.json"
VERSIONS = [
    "1.2.3", "v1.2.3", " v1.2.3 ", "vv1.2.3", "01.2.3", "1.2", "1.2.3.4", "1.2.3-rc.1", "1.2.3-rc.01",
    "1.2.3-rc..1", "1.2.3-", "1.2.3+build", "1.2.3+build.01", "1.2.3+", "1.2.3-alpha-1+b-2",
    "1.2.3-α", "99999999999999999999.0.0", "0.0.0", "1.2.3\n", "1.2.3-0", "1.2.3-00",
]
PAIRS = [
    ("1.2.3", "1.2.3-rc.1"), ("1.2.3-rc.10", "1.2.3-rc.9"), ("1.2.3-rc.a", "1.2.3-rc.1"),
    ("1.2.3-rc.1.1", "1.2.3-rc.1"), ("10.0.0", "9.99.99"), ("1.2.3+b", "1.2.3"),
    ("1.2.3-B", "1.2.3-a"), ("1.2.3-a", "1.2.3-B"), ("v2.0.0", "1.9.9"), ("junk", "1.0.0"),
    ("99999999999999999999.0.0", "99999999999999999998.9.9"), ("1.0.0-1", "1.0.0-a"),
]
PAYLOADS = [
    [],
    {},
    [{"tag_name": "v1.0.0"}, {"tag_name": "v2.0.0", "draft": True}, {"tag_name": " v1.1.0-rc.1 "},
     {"tag_name": "junk"}, {"tag_name": "v1.1.0-rc.1", "prerelease": False}],
    [{"tag_name": "v1.0.0", "prerelease": 1}, {"tag_name": 5}, {"tag_name": None}, "x", {"draft": 0, "tag_name": "1.0.1"}],
    [{"tag_name": "v1.0.0/../x"}, {"tag_name": "v0.9.0+meta"}],
    [{"tag_name": "v3.0.0", "draft": ""}, {"tag_name": "v3.0.0", "prerelease": True}],
]
RESULTS = [
    {"current_version": "1.0.0", "latest_version": "1.0.0", "update_available": False,
     "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v1.0.0"},
    {"current_version": "1.0.0", "latest_version": "1.1.0", "update_available": True,
     "release_url": "https://github.com/bitcoinaustria/kassiber/releases/tag/v1.1.0", "update_command": None},
    {"current_version": "1.0.0", "latest_version": "1.1.0", "update_available": True,
     "release_url": "u", "update_command": uc.HOMEBREW_CASK_COMMAND},
    {"current_version": "1.0.0", "latest_version": "1.1.0", "update_available": True,
     "release_url": "u", "update_command": ""},
]


def selection(payload):
    try:
        return uc._release_from_response(payload)
    except ValueError as error:
        return {"error": str(error)}


def generate():
    return {
        "versions": [[v, uc.parse_version(v) is not None] for v in VERSIONS],
        "newer": [[a, b, uc.is_newer_version(a, b), uc.is_newer_version(b, a)] for a, b in PAIRS],
        "selection": [[p, selection(p)] for p in PAYLOADS],
        "render": [[r, uc.render_update_status(r, color=False), uc.render_update_status(r, color=True)] for r in RESULTS],
        "release_url": [[t, uc.release_url_for_tag(t)] for t in ["v1.0.0", "v1.0.0/ä b", "~x_y.z-"]],
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    encoded = json.dumps(generate(), indent=2, ensure_ascii=True) + "\n"
    if args.check:
        if DESTINATION.read_text() != encoded:
            raise SystemExit("release vectors differ; rerun export_vectors.py")
    else:
        DESTINATION.write_text(encoded)
