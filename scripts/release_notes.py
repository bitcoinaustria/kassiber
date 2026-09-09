#!/usr/bin/env python3
"""Extract a reviewed version section from CHANGELOG.md without rewriting it."""
import argparse
from pathlib import Path
import re


def extract_notes(changelog: str, version: str) -> str:
    version = version.removeprefix("v")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:[.-][A-Za-z0-9.-]+)?", version):
        raise ValueError("Invalid release version")
    headings = list(re.finditer(r"^## ([^\n]+)\n", changelog, re.MULTILINE))
    matches = [i for i, heading in enumerate(headings) if heading.group(1) == version]
    if len(matches) != 1:
        raise ValueError(f"Require exactly one changelog section for {version}")
    index = matches[0]
    end = headings[index + 1].start() if index + 1 < len(headings) else len(changelog)
    notes = changelog[headings[index].end():end].strip("\n")
    if not notes.strip():
        raise ValueError(f"Empty changelog section for {version}")
    return notes + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", required=True)
    parser.add_argument("--changelog", type=Path, default=Path("CHANGELOG.md"))
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    try:
        notes = extract_notes(args.changelog.read_text(encoding="utf-8"), args.version)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Release notes: {error}\n")
    args.output.write_text(notes, encoding="utf-8")
