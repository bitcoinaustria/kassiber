#!/usr/bin/env python3
"""Run the shared scratch-tree checker for main sync S7 (the Reports screen and
the crypto Kennzahlen from 1 March 2022), offline.

Each entry's named tests live in the reports library, its `vectors` and
`gain_reads` tests, or the journals library, so ``--crate`` builds and runs
only those targets instead of every test binary of both crates:

    reports_screen_mutation_check.py --crate reports   # entries in crates/reports
    reports_screen_mutation_check.py --crate journals  # entries in crates/journals
"""
import importlib.util
import json
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("reports_screen_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
MUTATIONS = Path(__file__).with_name("reports_screen_mutations.json")
TARGETS = {
    "reports": ("-p", "kassiber-reports", "--lib", "--test", "vectors", "--test", "gain_reads", "--offline"),
    "journals": ("-p", "kassiber-journals", "--lib", "--offline"),
}


def main(argv):
    crate = None
    if "--crate" in argv:
        index = argv.index("--crate")
        crate = argv[index + 1]
        argv = argv[:index] + argv[index + 2:]
    if crate is None:
        checker.ENGINE_PACKAGES = ("-p", "kassiber-reports", "-p", "kassiber-journals", "--offline")
        checker.MUTATIONS = MUTATIONS
        return checker.main(argv)
    checker.ENGINE_PACKAGES = TARGETS[crate]
    entries = [entry for entry in json.loads(MUTATIONS.read_text(encoding="utf-8"))
               if entry["file"].startswith(f"crates/{crate}/")]
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as subset:
        json.dump(entries, subset)
    try:
        checker.MUTATIONS = Path(subset.name)
        return checker.main(argv)
    finally:
        Path(subset.name).unlink()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
