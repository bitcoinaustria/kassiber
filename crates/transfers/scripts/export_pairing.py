#!/usr/bin/env python3
"""Capture matcher calls made by the existing Python transfer fixtures.

Only synthetic inputs are recorded. Run with TMPDIR under target/.
"""
import dataclasses
import functools
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core import transfer_matching as matching


def main():
    captured = []
    original = matching.suggest_swap_candidates

    @functools.wraps(original)
    def record(rows, **kw):
        # The fixture suite always has non-expiring or explicitly timed vetoes.
        kw.setdefault('now_iso', '2026-10-30T12:00:00Z')
        case = dict(rows=[dict(r) for r in rows],
                    options={k: list(v) if isinstance(v, set) else v for k, v in kw.items()})
        try:
            result = original(rows, **kw)
        except ValueError as error:
            case['error'] = str(error)
            captured.append(case)
            raise
        case['result'] = [dataclasses.asdict(c) for c in result]
        captured.append(case)
        return result

    with patch.object(matching, 'suggest_swap_candidates', record):
        suite = unittest.defaultTestLoader.loadTestsFromNames([
            'tests.test_transfer_matching',
        ])
        result = unittest.TextTestRunner(verbosity=0).run(suite)
    if not result.wasSuccessful():
        raise SystemExit(1)
    from tests.test_transfer_matching import _row
    # Exercise parsed and encoded raw/config payloads at the optimized identity
    # seam, including malformed outer txids and nonfinite metadata.
    for encoded in (False, True):
        for out_raw, in_raw, config, expected in [
            ({'txid':'11'*32}, {'txid':'11'*32}, {}, 1),
            ({'txid':'11'*32}, {'txid':'22'*32}, {}, 0),
            ({'txid':'bad', 'tx':{'txid':'11'*32}}, {'txid':'22'*32}, {}, 1),
            ({'txid':None, 'tx':{'txid':'11'*32}}, {'txid':'22'*32}, {}, 0),
            ({'txid':False, 'tx':{'txid':'11'*32}}, {'txid':'22'*32}, {}, 0),
            ({'txid':0, 'tx':{'txid':'11'*32}}, {'txid':'22'*32}, {}, 0),
            ({'txid':'11'*32}, {'txid':'22'*32}, {'chain':'bitcoin','network':'test'}, 0),
            ({'txid':'11'*32,'network':'main'}, {'txid':'22'*32,'network':'main'}, {'network':'test'}, 1),
            ({'txid':float('nan')}, {'txid':'22'*32}, {}, 1),
            ({'txid':'11'*32}, {'txid':'22'*32}, {'chain':float('nan')}, 1),
        ]:
            rows=[_row(id='o',wallet_id='source',amount=10_000_000,raw_json=out_raw,config_json=config),
                  _row(id='i',wallet_id='target',direction='inbound',amount=9_000_000,raw_json=in_raw,config_json=config)]
            if encoded:
                for row in rows:
                    for key in ('raw_json','config_json'):row[key]=json.dumps(row[key])
            assert len(record(rows)) == expected
    rows=[_row(id='o',wallet_id='source',raw_json=dict(provider='boltz',swap_id='digit-repro',flow='chain',send_amount_msat='²')),
          _row(id='i',wallet_id='target',direction='inbound')]
    # b654fad74: a non-ASCII digit amount is no evidence; the pair still matches.
    assert record(rows), 'a non-ASCII digit amount must not block the heuristic pair'
    assert not record(rows,booked_move_transaction_ids={'o'})
    duplicate_hash=[_row(id='hash-o',wallet_id='source',amount=10_000_000,payment_hash='ab'*32,payment_hash_source='importer'),
                    _row(id='hash-duplicate',wallet_id='third',amount=10_000_000,payment_hash='ab'*32,payment_hash_source='importer'),
                    _row(id='hash-i',wallet_id='target',direction='inbound',amount=9_000_000,payment_hash='ab'*32,payment_hash_source='importer')]
    matches=record(duplicate_hash)
    assert len(matches)==2 and all(c.method=='heuristic' for c in matches)
    raw=dict(provider='boltz',swap_id='amount-conflict',flow='chain',send_txid='11'*32,receive_txid='22'*32,send_amount_msat=10_000_000,receive_amount_msat=9_000_000)
    amount_conflict=[_row(id='amount-o',wallet_id='source',amount=10_000_000,external_id='11'*32,raw_json=dict(raw,txid='11'*32)),
                     _row(id='amount-i',wallet_id='target',direction='inbound',amount=9_000_000,external_id='22'*32,raw_json=dict(raw,txid='22'*32,send_amount_msat=10_000_008))]
    matches=record(amount_conflict)
    assert len(matches)==1 and matches[0].confidence=='strong' and matches[0].evidence_conflicts==('amount',)
    # Avoid unstable iteration order in fixture sets; none affect expected order.
    for case in captured:
        for name in ('booked_move_transaction_ids',):
            if name in case['options']:
                case['options'][name] = sorted(case['options'][name])
    destination = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / 'crates/transfers/tests/pairing_matcher.json'
    destination.write_text(json.dumps(captured, ensure_ascii=True, indent=2) + '\n')
    print(f'{len(captured)} captured matcher cases -> {destination}')


if __name__ == '__main__':
    main()
