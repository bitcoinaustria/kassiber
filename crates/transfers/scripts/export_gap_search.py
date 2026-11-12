#!/usr/bin/env python3
"""Replay R2's synthetic gap inputs against the current Python reference.

Only the search needed by gap authoring is integrated here. Inputs originate
from t3code/r2d-gaps; no journal composition or automatic pairing is enabled.
"""
import json
from pathlib import Path
import sys

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import custody_gaps as gaps, custody_gap_reviews as reviews
from tests.custody_snapshot import to_jsonable
from tests.custody_gap_vectors import worklist_range_vectors

PATH = ROOT / 'tests/fixtures/custody_gap_cases.json'


def candidate(value):
    return gaps.CustodyGapCandidate(**{
        k: tuple(v) if isinstance(v, list) else v
        for k, v in value.items() if k != '$type'
    })


def outcome(call):
    try:
        return {'expected': to_jsonable(call(), redact_config=False)}
    except Exception as exc:
        return {'error': getattr(exc, 'code', type(exc).__name__)}


def main():
    data = json.loads(PATH.read_text())
    for case in data['search']:
        case.pop('expected', None)
        case.pop('error', None)
        case.update(outcome(lambda: gaps.search_custody_gap_candidates(
            case['rows'], ignored_ids=case['ignored'], **case['limits'])))
    for case in data['compute']:
        case.pop('expected', None)
        case.pop('error', None)
        case.update(outcome(lambda: gaps.compute_gap_search_result(
            case['rows'], **{k: case[k] for k in (
                'large_book', 'ignored_transaction_ids',
                'accounting_ignored_transaction_ids')})[0]))
    for case in data['fingerprints']:
        c = candidate(case['candidate'])
        case['candidate_fingerprint'] = reviews.candidate_fingerprint(c)
        case['authored_claim_fingerprint'] = reviews.authored_claim_fingerprint(c)
    for case in data['reviews']:
        case['expected'] = reviews.dismissed_fingerprints_from_rows(case['rows'])
    for case in data['display']:
        legs = [leg for row in case['rows']
                if (leg := gaps._normalize_leg(row, set())) is not None]
        case['expected'] = gaps._snapshot_gap(candidate(case['candidate']), legs)
    data['worklist_ranges'] = worklist_range_vectors(data['search'])
    text = json.dumps(data, ensure_ascii=True, separators=(',', ':')) + '\n'
    if '--check' in sys.argv:
        assert PATH.read_text() == text, PATH
    else:
        PATH.write_text(text)


if __name__ == '__main__':
    main()
