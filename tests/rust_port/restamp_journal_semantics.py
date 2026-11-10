"""Restamp recorded journal stores with the current calculation semantics.

Frozen recordings (``persistence-stages.json``) hold Python's
``store_ledger_state`` output from before a semantics bump. That writer sets
``journal_semantics_version`` to ``JOURNAL_CALCULATION_SEMANTICS_VERSION`` and
``required_journal_semantics_version`` to ``MAX(required, it)``, and nothing
else about the stamp, so a recorded store under an older version ``OLD`` reads
exactly as the current writer's once each expected profile row stamped
``(MAX(before, OLD), OLD)`` becomes ``(MAX(before, NEW), NEW)``. Every other
stamp refuses; regenerate such a vector instead.

    python -m tests.rust_port.restamp_journal_semantics --from 2 [--check] FILE...
"""
from __future__ import annotations

import argparse
import json
from pathlib import Path

from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION as NEW

COLUMNS = ("required_journal_semantics_version", "journal_semantics_version")


def stamps(dump):
    columns = dump["columns"]
    return [columns.index(c) for c in COLUMNS], {row[columns.index("id")]: row for row in dump["rows"]}


def restamp(case, old, path):
    before_index, before = stamps(case["before"]["profiles"])
    expected = case["expected"]["profiles"]
    (required, built), _ = stamps(expected)
    changed = False
    for row in expected["rows"]:
        prior = before[row[expected["columns"].index("id")]][before_index[0]]
        if (row[required], row[built]) == (max(prior, NEW), NEW):
            continue
        if (row[required], row[built]) != (max(prior, old), old):
            raise SystemExit(f"{path}: {case['name']}: stamp {row[required], row[built]} "
                             f"is not the version-{old} store's")
        row[required], row[built] = max(prior, NEW), NEW
        changed = True
    return changed


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--from", dest="old", type=int, required=True)
    parser.add_argument("--check", action="store_true")
    parser.add_argument("files", nargs="+", type=Path)
    args = parser.parse_args()
    stale = []
    for path in args.files:
        raw = path.read_text()
        cases = json.loads(raw)
        if any([restamp(case, args.old, path) for case in cases]):
            stale.append(path)
            if not args.check:
                path.write_text(json.dumps(cases, ensure_ascii=True, separators=(",", ":")) + "\n")
    if args.check and stale:
        raise SystemExit(f"stale semantics stamps: {', '.join(map(str, stale))}")
    print(f"{len(stale)} restamped")


if __name__ == "__main__":
    main()
