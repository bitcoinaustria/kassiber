#!/usr/bin/env python3
"""Run the rounds 5 and 8 group A entries of the crate mutation catalogs.

    uv run --locked python crates/store/scripts/rounds_5_8_a_mutation_check.py [--only ID ...] [--check]

The entries live in each crate's ``scripts/mutations.json`` with ``-r58a-`` in
their id. Each catalog runs with only the packages whose tests kill its
entries, so a mutation does not rebuild the whole crate family. Set TMPDIR to
a disposable directory for the scratch trees, and CARGO_BUILD_BUILD_DIR to one
fixed directory reused across runs, so the third-party crates compile once.
"""
import argparse
import importlib.util
import os
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)

CATALOGS = {
    "crates/wallets/scripts/mutations.json": ("-p", "kassiber-wallets", "--lib", "--offline"),
    "crates/store/scripts/mutations.json": ("-p", "kassiber-store", "--lib", "--offline"),
    "crates/reports/scripts/mutations.json": ("-p", "kassiber-reports", "--lib", "--offline"),
    "crates/journals/scripts/mutations.json": ("-p", "kassiber-journals", "--test", "prebuild_pricing", "--offline"),
    "crates/transactions/scripts/mutations.json": ("-p", "kassiber-transactions", "--test", "metadata", "--offline"),
    "crates/rates/scripts/mutations.json": ("-p", "kassiber-rates", "--lib", "--offline"),
}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--only", action="append", default=[], metavar="ID", help="check only this entry (repeatable)")
    parser.add_argument("--check", action="store_true", help="only validate the entries against the worktree")
    args = parser.parse_args(argv)
    sys.stdout.reconfigure(line_buffering=True)
    groups = {
        catalog: [m for m in checker.load(ROOT / catalog) if "-r58a-" in m.id]
        for catalog in CATALOGS
    }
    known = {m.id for group in groups.values() for m in group}
    unknown = sorted(set(args.only) - known)
    if unknown:
        print(f"--only names unknown entries: {', '.join(unknown)}", file=sys.stderr)
        return 2
    if args.only:
        groups = {catalog: [m for m in group if m.id in args.only] for catalog, group in groups.items()}
    mutations = [m for group in groups.values() for m in group]
    if args.check:
        trouble = checker.problems(mutations)
        print("\n".join(trouble) if trouble else f"{len(mutations)} entries apply")
        return 1 if trouble else 0
    status = 0
    try:
        for catalog, group in groups.items():
            if not group:
                continue
            print(f"== {catalog}: {len(group)} mutations")
            checker.ENGINE_PACKAGES = CATALOGS[catalog]
            status = max(status, checker.check(group))
    finally:
        # The scratch trees share CARGO_BUILD_BUILD_DIR with this worktree, and
        # the last mutant's artifacts would look fresh to its older sources.
        for path in {m.file for m in mutations}:
            os.utime(ROOT / path)
    return status


if __name__ == "__main__":
    # These tests use in-memory and tempfile books only.
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(main())
