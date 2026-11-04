#!/usr/bin/env python3
"""Run the shared offline scratch-tree checker for PDF rendering rules."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("pdf_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
# Only the PDF test targets: filtering every reports test binary is slow.
checker.ENGINE_PACKAGES = (
    "-p", "kassiber-pdf", "-p", "kassiber-reports", "--offline", "--test", "text", "--test", "full_pdf",
)
checker.MUTATIONS = Path(__file__).with_name("mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
