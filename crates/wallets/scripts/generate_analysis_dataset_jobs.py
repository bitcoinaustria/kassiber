"""Frozen unchanged-Python receipts for the dataset job starters (see
tests/test_rust_analysis_dataset_jobs.py, which also compares them live)."""
from __future__ import annotations

import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
DEST = ROOT / 'crates/wallets/tests/data/analysis_dataset_jobs.json'
SCRATCH = ROOT / 'target/r5-dataset-jobs'


def vectors():
    from tests import test_rust_analysis_dataset_jobs as jobs
    SCRATCH.mkdir(parents=True, exist_ok=True)
    database, source = SCRATCH / 'reference.db', SCRATCH / 'pack.csv'
    for path in (database, source):
        if path.exists():
            path.unlink()
    jobs.book(database)
    source.write_text(jobs.many)
    corpus = []
    for name, (_, steps) in sorted(jobs.SCENARIOS.items()):
        if database.exists():
            database.unlink()
        jobs.book(database)
        corpus.append({'name': name, 'source': jobs.many, 'steps': steps, 'expected': jobs.python_steps(database, source, steps)})
    return corpus


if __name__ == '__main__':
    DEST.write_text(json.dumps(vectors(), ensure_ascii=True, separators=(',', ':')) + '\n')
