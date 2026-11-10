#!/usr/bin/env python3
"""Run the shared scratch-tree mutation checker on the same-time source order catalog.

    uv run --locked python crates/store/scripts/lot_order_mutation_check.py [--only ID ...] [--check]

The catalog spans the crates that read or record the same-time source order. Each entry's id prefix
names the packages whose tests kill it, so a mutation builds only those.
Set TMPDIR to a disposable directory for the scratch trees, and
CARGO_BUILD_BUILD_DIR to one fixed directory reused across runs, so the
third-party crates compile once.
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
checker.MUTATIONS = Path(__file__).with_name("lot_order_mutations.json")

PACKAGES = {
    "store": ("-p", "kassiber-store", "--offline"),
    "journals": ("-p", "kassiber-journals", "--offline"),
    "custody": ("-p", "kassiber-custody", "--offline"),
    "wallets": ("-p", "kassiber-wallets", "--lib", "--offline"),
    "reports": ("-p", "kassiber-reports", "--lib", "--offline"),
    "workspace": ("-p", "kassiber-workspace", "--lib", "--offline"),
}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0])
    parser.add_argument("--only", action="append", default=[], metavar="ID", help="check only this entry (repeatable)")
    parser.add_argument("--check", action="store_true", help="only validate the entries against the worktree")
    args = parser.parse_args(argv)
    sys.stdout.reconfigure(line_buffering=True)
    mutations = checker.load(checker.MUTATIONS)
    unknown = sorted(set(args.only) - {mutation.id for mutation in mutations})
    if unknown:
        print(f"--only names unknown entries: {', '.join(unknown)}", file=sys.stderr)
        return 2
    if args.only:
        mutations = [mutation for mutation in mutations if mutation.id in args.only]
    groups = {}
    for mutation in mutations:
        prefix = mutation.id.split("-", 1)[0]
        if prefix not in PACKAGES:
            print(f"{mutation.id}: no package group {prefix!r}", file=sys.stderr)
            return 2
        groups.setdefault(prefix, []).append(mutation)
    if args.check:
        trouble = checker.problems(mutations)
        print("\n".join(trouble) if trouble else f"{len(mutations)} entries apply")
        return 1 if trouble else 0
    status = 0
    try:
        for prefix, group in groups.items():
            print(f"== {prefix}: {len(group)} mutations")
            checker.ENGINE_PACKAGES = PACKAGES[prefix]
            status = max(status, checker.check(group))
    finally:
        # The scratch trees share CARGO_BUILD_BUILD_DIR with this worktree, and
        # the last mutant's artifacts would look fresh to its older sources.
        for path in {mutation.file for mutation in mutations}:
            os.utime(ROOT / path)
    return status


if __name__ == "__main__":
    # These tests use in-memory, tempfile or loopback-free fake transports only.
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    raise SystemExit(main())
