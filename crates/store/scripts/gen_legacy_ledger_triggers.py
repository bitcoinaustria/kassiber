#!/usr/bin/env python3
"""Write `crates/store/fixtures/legacy-ledger-triggers.json`: every `gl_*`
trigger Kassiber ever installed for the removed general ledger.

For each revision from the ledger's introduction to its removal that touched
`kassiber/core/accounting` or `kassiber/db.py`, the revision's own `kassiber/`
tree is extracted to a scratch directory and its `open_db` builds a fresh
book; the `gl_*` triggers it leaves in `sqlite_master` (name, table and the
SQL exactly as stored) are collected. The union is what a book written by any
of those versions can hold; the store's structural check accepts populated
ledger archives only with triggers from this set.

    uv run --locked python crates/store/scripts/gen_legacy_ledger_triggers.py [--check]
"""
import argparse
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
FIXTURE = ROOT / "crates/store/fixtures/legacy-ledger-triggers.json"
INTRODUCED = "7ce7f9b31"  # Add separate encrypted accounting core and deterministic CLI
REMOVED = "f36af60de"  # Remove the general ledger from the core, CLI, daemon, and AI tools

DUMP = r"""
import json, sys, tempfile
from pathlib import Path
sys.path.insert(0, sys.argv[1])
from kassiber.db import open_db
root = Path(tempfile.mkdtemp(dir=sys.argv[2]))
conn = open_db(root)
rows = conn.execute(
    "SELECT name, tbl_name, sql FROM sqlite_master WHERE type='trigger' AND name LIKE 'gl\\_%' ESCAPE '\\'"
).fetchall()
print(json.dumps([list(row) for row in rows]))
"""


def git(*args):
    return subprocess.run(["git", *args], cwd=ROOT, check=True, capture_output=True, text=True).stdout


def revisions():
    """Oldest first, so `first_seen` names the revision that introduced a shape."""
    commits = git("rev-list", "--reverse", "--topo-order", f"{INTRODUCED}^..{REMOVED}^", "--",
                  "kassiber/core/accounting", "kassiber/db.py").split()
    last = git("rev-parse", f"{REMOVED}^").strip()
    return commits + ([last] if last not in commits else [])


def triggers(commit, scratch):
    tree = scratch / commit
    tree.mkdir()
    archive = subprocess.run(["git", "archive", commit, "kassiber"], cwd=ROOT, check=True, capture_output=True).stdout
    subprocess.run(["tar", "-x", "-C", str(tree)], input=archive, check=True)
    books = scratch / f"{commit}-books"
    books.mkdir()
    # -I: the extracted tree is the only kassiber on the path.
    result = subprocess.run([sys.executable, "-I", "-c", DUMP, str(tree), str(books)],
                            capture_output=True, text=True, cwd=scratch, timeout=300)
    if result.returncode:
        raise SystemExit(f"{commit}: open_db failed\n{result.stderr[-2000:]}")
    return [tuple(row) for row in json.loads(result.stdout)]


def collect():
    found = {}
    target = ROOT / "target"
    target.mkdir(exist_ok=True)
    scratch = Path(tempfile.mkdtemp(prefix="legacy-ledger-triggers-", dir=target.resolve()))
    try:
        for commit in revisions():
            for name, table, sql in triggers(commit, scratch):
                found.setdefault((name, table, sql), commit)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    return [{"name": n, "table": t, "sql": s, "first_seen": c[:9]} for (n, t, s), c in sorted(found.items())]


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--check", action="store_true", help="fail if the fixture is out of date")
    args = parser.parse_args()
    text = json.dumps({"introduced": INTRODUCED, "removed": REMOVED, "triggers": collect()}, indent=1) + "\n"
    if args.check:
        if FIXTURE.read_text() != text:
            raise SystemExit(f"{FIXTURE} is out of date; rerun without --check")
        return
    FIXTURE.write_text(text)


if __name__ == "__main__":
    main()
