#!/usr/bin/env python3
"""Freeze Python's ID-less import identity state from the 8f repro scenarios.

Runs tests/test_import_csv_upgrade_compatibility.py and, at every point a
scenario inspects its held duplicates (`_held`), records the book's import
rows and what `core/import_identity.py` derives from them: occurrence slots,
held groups and the quarantine records custody receives. The Rust test
replays each state on an empty schema and compares byte for byte. The
scenarios draw fresh identities and clocks, so a rerun records new (equally
valid) states rather than the same bytes; the recorded outputs always belong
to the recorded inputs.

Run with `uv run --locked python crates/wallets/scripts/generate_import_identity_vectors.py`.
"""
import argparse
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
DEST = ROOT / "crates/wallets/tests/vectors/python_import_identity.json"
TABLES = ("workspaces", "profiles", "wallets", "transactions", "import_batches",
          "import_batch_transactions", "synthetic_import_duplicate_reviews")


def capture(conn, profile_id):
    from kassiber.core import import_identity

    def rows(table):
        result = []
        for row in conn.execute(f"SELECT * FROM {table} ORDER BY rowid"):
            result.append({key: row[key] for key in row.keys()})
        return result

    groups = []
    for (wallet, economic), members in import_identity.stored_synthetic_occurrences(conn, profile_id=profile_id).items():
        groups.append({
            "wallet_id": wallet, "economic_hash": economic,
            "members": [row["id"] for row in members],
            "slots": {str(number): row["id"] for number, row in sorted(import_identity.occurrence_slots(members).items())},
        })
    held = [{
        "wallet_id": group["wallet_id"], "economic_hash": group["economic_hash"],
        "group_fingerprint": group["group_fingerprint"], "active": [row["id"] for row in group["active"]],
        "largest": group["largest"], "import_count": group["import_count"],
        "hold_reasons": group["hold_reasons"], "refilled_excluded_ids": group["refilled_excluded_ids"],
    } for group in import_identity.held_duplicate_groups(conn, profile_id)]
    return {
        "profile_id": profile_id,
        "tables": {table: rows(table) for table in TABLES},
        "groups": groups, "held": held,
        "quarantines": [dict(item) for item in import_identity.duplicate_review_quarantines(conn, profile_id)],
    }


class Recorder:
    def __init__(self):
        self.cases = []
        self.test = None

    def pytest_runtest_setup(self, item):
        self.test = item.nodeid.split("::", 1)[1]
        self.index = 0

    def pytest_configure(self, config):
        import tests.test_import_csv_upgrade_compatibility as module
        original = module._held

        def held(conn, profile):
            self.index += 1
            self.cases.append({"name": f"{self.test}#{self.index}", **capture(conn, str(profile["id"]))})
            return original(conn, profile)

        module._held = held


def vectors():
    import pytest
    recorder = Recorder()
    with tempfile.TemporaryDirectory(prefix="kassiber-import-identity-") as root:
        code = pytest.main([str(ROOT / "tests/test_import_csv_upgrade_compatibility.py"), "-q", "-p", "no:cacheprovider",
                            "--basetemp", str(Path(root) / "pytest"), "-p", "no:randomly"], plugins=[recorder])
    if code != 0:
        raise SystemExit("the Python repro scenarios must pass before freezing their state")
    return recorder.cases


def main():
    argparse.ArgumentParser(description=__doc__).parse_args()
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, indent=1, sort_keys=True) + "\n")


if __name__ == "__main__":
    main()
