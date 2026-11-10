#!/usr/bin/env python3
"""Run every inherited Bitcoin sync and persistence recipe in one scratch tree.

Uses the shared checker: named baselines must pass, mutations must build, and
every named selection must fail. No build error or missing selection is a kill.
Scratch and the shared incremental build live under this checkout's target/.
"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("tax_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = (
    "-p", "kassiber-wallets", "-p", "kassiber-store", "-p", "kassiber-chain", "--lib",
    "--features", "kassiber-wallets/test-harness,kassiber-chain/test-harness", "--offline",
)
CATALOGS = [
    ROOT / "crates/wallets/scripts" / f"{name}_mutations.json"
    for name in ("discovery", "acquisition", "observer", "bdk", "record", "persistence", "fetch", "service")
] + [ROOT / "crates/chain/scripts/mutations.json"]


def main():
    scratch = ROOT / "target/sync-mutation-scratch"
    scratch.mkdir(parents=True, exist_ok=True)
    # The shared checker uses tempfile for its isolated source copy too.
    tempfile.tempdir = str(scratch)
    os.environ.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/sync-mutation-build"))
    os.environ.setdefault("RUST_TEST_THREADS", "1")
    os.environ["KASSIBER_TEST_NO_EGRESS"] = "1"
    entries = [entry for path in CATALOGS for entry in json.loads(path.read_text())]
    with tempfile.NamedTemporaryFile(mode="w", suffix=".json", dir=scratch) as catalog:
        json.dump(entries, catalog)
        catalog.flush()
        checker.MUTATIONS = Path(catalog.name)
        return checker.main()


if __name__ == "__main__":
    raise SystemExit(main())
