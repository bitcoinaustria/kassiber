#!/usr/bin/env python3
"""Freeze synthetic SQL/byte vectors from the live loader oracle tests.

Use TMPDIR under the worktree's target directory. No owner books are read and
no production data recording hook is installed.
"""
import argparse
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.custody_snapshot import load_custody_snapshot
from kassiber.db import open_db
from tests import test_rust_custody_loader as cases
from tests.test_rp2_ownership_transfers import OwnershipDeriverHandlerTest

DEST = ROOT / "crates/journals/tests/data/loader.json"


def vectors():
    result = []
    current_name = None

    def record(conn):
        profile = dict(cases._profile(conn))
        snapshot = load_custody_snapshot(conn, profile, as_of=cases.AS_OF)
        assert snapshot["transactions"], f"{current_name}: fixture has no admitted SQL observations"
        if current_name.startswith(("owned-", "retired-", "receipts-")):
            assert snapshot["owned_index"] is not None, f"{current_name}: ownership branch inactive"
        if current_name.startswith("receipts-"):
            assert snapshot["receipt_sources"], f"{current_name}: receipt branch inactive"
        result.append({"name": current_name, "sql": "\n".join(conn.iterdump()),
                       "profile": profile, "as_of": cases.AS_OF,
                       "expected": json.dumps(snapshot, ensure_ascii=True, allow_nan=True)})

    original = cases.check
    cases.check = record
    try:
        with tempfile.TemporaryDirectory(prefix="custody-loader-vectors-") as directory, patch(
            "kassiber.core.chain_observer.provenance.now_iso", return_value=cases.AS_OF
        ):
            source = open_db(Path(directory))
            helper = OwnershipDeriverHandlerTest()
            helper._seed(source)
            selected = [
                ("structural-columns", cases.test_loader_covers_every_structural_table_column_and_filter, ()),
                ("legacy-rates", cases.test_loader_supports_missing_network_table_and_legacy_rate_columns, ()),
                ("retired-policies", cases.test_loader_preserves_retired_policies_warnings_and_core_outpoints, ()),
                *[("owned-" + str(descriptor), cases.test_loader_owns_inventory_addresses_and_descriptors, (descriptor,)) for descriptor in (False, True)],
                *[("receipts-" + str(descriptor), cases.test_loader_reads_receipt_sources, (descriptor,)) for descriptor in (False, True)],
                *[("config-" + str(i), cases.test_loader_normalizes_config_and_preserves_storage, (config,))
                  for i, config in enumerate(cases.CONFIGS)],
            ]
            for current_name, function, arguments in selected:
                book = cases.loose_book.__wrapped__((source, helper))
                try:
                    function(next(book), *arguments)
                finally:
                    book.close()
            source.close()
    finally:
        cases.check = original
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    text = json.dumps(vectors(), ensure_ascii=True, indent=2) + "\n"
    if args.check:
        if DEST.read_text() != text:
            raise SystemExit("native custody loader vectors differ from Python")
    else:
        DEST.write_text(text)
    print(f"{len(json.loads(text))} native loader vectors match Python")


if __name__ == "__main__":
    main()
