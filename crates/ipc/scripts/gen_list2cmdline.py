#!/usr/bin/env python3
"""Regenerate the subprocess.list2cmdline vectors the Windows launcher pins.

    uv run --locked python crates/ipc/scripts/gen_list2cmdline.py
"""
import json
import subprocess
from pathlib import Path

CASES = [
    ["a b c", "d", "e"],
    ['ab"c', "\\", "d"],
    ['ab"c', " \\", "d"],
    ["a\\\\\\b", "de fg", "h"],
    ['a\\"b', "c", "d"],
    ["a\\\\b c", "d", "e"],
    ["a\\\\b\\ c", "d", "e"],
    ["ab", ""],
    ["C:\\Program Files\\Kassiber\\kassiber.exe", "--operator-broker-server"],
    ["tab\there", "trail\\"],
    ['q\\\\"x'],
    ["sp ace\\"],
    ["\u00e4\u20ac\U0001f511", "--data-root", "C:\\B\u00fccher\\"],
]
DEST = Path(__file__).resolve().parents[1] / "tests" / "data" / "list2cmdline.json"


def vectors():
    return [{"argv": case, "command_line": subprocess.list2cmdline(case)} for case in CASES]


if __name__ == "__main__":
    DEST.write_text(json.dumps(vectors(), indent=1, ensure_ascii=True) + "\n", encoding="utf-8")
