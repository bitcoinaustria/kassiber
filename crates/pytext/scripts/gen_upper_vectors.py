"""Freeze all CPython 3.11 scalar uppercase changes for authored currency codes."""
import argparse
from pathlib import Path
import sys
import unicodedata

TARGET = Path(__file__).resolve().parents[1] / "tests/data/upper.txt"


def render():
    assert sys.version_info[:2] == (3, 11) and unicodedata.unidata_version == "14.0.0"
    return "".join(
        f"{point:x};" + " ".join(f"{ord(c):x}" for c in chr(point).upper()) + "\n"
        for point in range(0x110000) if chr(point).upper() != chr(point)
    )


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    expected = render()
    if args.check:
        if TARGET.read_text() != expected:
            raise SystemExit("uppercase vectors drifted")
    else:
        TARGET.write_text(expected)
