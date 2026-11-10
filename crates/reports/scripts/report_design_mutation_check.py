#!/usr/bin/env python3
"""Run the shared offline scratch-tree checker for the report design rules."""
import importlib.util
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
spec = importlib.util.spec_from_file_location("report_design_mutation_check", ROOT / "tax-engine/scripts/mutation_check.py")
checker = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES = (
    "-p", "kassiber-reports", "--offline", "--lib", "--test", "report_design", "--test", "austrian_pdf",
    "--test", "source_funds_pdf", "--test", "artifact_vectors", "--test", "tax_country_vectors",
)
# Tests run one at a time on a shared, loaded build disk.
checker.TEST_TIMEOUT_SECONDS = 7200
checker.MUTATIONS = Path(__file__).with_name("report_design_mutations.json")

if __name__ == "__main__":
    raise SystemExit(checker.main())
