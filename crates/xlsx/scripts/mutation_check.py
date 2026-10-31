#!/usr/bin/env python3
"""Run the shared offline scratch-tree checker for workbook rendering rules."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("xlsx_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = ("-p", "kassiber-xlsx", "--offline")
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
