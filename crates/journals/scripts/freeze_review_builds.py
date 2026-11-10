"""Freeze distinct complete outputs from the opt-in synthetic review capture."""
import argparse
import json
from pathlib import Path
import sqlite3

ROOT = Path(__file__).resolve().parents[3]
DESTINATION = ROOT / "crates/journals/tests/data/review-builds.json"


def freeze(directory):
    directory = directory.resolve()
    if not directory.is_relative_to(ROOT / "target"):
        raise ValueError("review captures require target/ scratch")
    cases = {}
    for path in sorted(directory.glob("[0-9]*.json")):
        record = json.loads(path.read_text())
        key = json.dumps(record["expected"], sort_keys=True)
        if key in cases:
            cases[key]["sources"].append(record["name"])
            continue
        with sqlite3.connect(":memory:") as conn:
            conn.executescript(record["sql"])
            live_ids = [row[0] for row in conn.execute(
                "SELECT id FROM transactions WHERE profile_id=? ORDER BY id",
                (record["profile"]["id"],),
            )]
        cases[key] = {
            "name": path.stem, "sources": [record["name"]],
            "snapshot": record["snapshot"], "entry_ids": record["entry_ids"],
            "live_ids": live_ids, "expected": record["expected"],
        }
    if not cases:
        raise ValueError("no review builds were captured")
    return json.dumps(list(cases.values()), ensure_ascii=True, separators=(",", ":")) + "\n"


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--accept", action="store_true")
    args = parser.parse_args()
    text = freeze(args.directory)
    if args.accept:
        DESTINATION.write_text(text)
    elif text != DESTINATION.read_text():
        raise SystemExit("review build vectors differ; inspect before --accept")
    print(f"{len(json.loads(text))} distinct complete review builds")
