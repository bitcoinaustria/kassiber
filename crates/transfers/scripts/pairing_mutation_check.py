#!/usr/bin/env python3
"""Run only the isolated R4 pairing mutation catalog on disposable scratch trees."""
import importlib.util
from pathlib import Path
import sys

ROOT=Path(__file__).resolve().parents[3]
spec=importlib.util.spec_from_file_location('pairing_mutations',ROOT/'tax-engine/scripts/mutation_check.py')
checker=importlib.util.module_from_spec(spec)
sys.modules[spec.name]=checker
spec.loader.exec_module(checker)
checker.ENGINE_PACKAGES=('--offline','--release','-p','kassiber-transfers','--lib','--test','pairing_matcher','--test','pairing_services')
checker.MUTATIONS=Path(__file__).with_name('pairing_mutations.json')
if __name__=='__main__':
    raise SystemExit(checker.main())
