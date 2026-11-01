#!/usr/bin/env python3
"""Reproduce the open R4-11 lossless-boundary gaps on synthetic operands.

Build the journal-render example first. A nonzero result means at least one
operand still differs from Python; these are not passing parity witnesses.
"""
from __future__ import annotations

import argparse
from copy import deepcopy
from dataclasses import replace
import json
from pathlib import Path
import subprocess
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

from kassiber.core.custody_compute import compute_custody_projection
from tests.custody_snapshot_generator import snapshot
from tests.rust_port.journal_render_oracle import capture_projection


def operands():
    book = snapshot(0)
    custody = compute_custody_projection(book)
    response = capture_projection(book['profile'], custody)['response']
    yield 'surrogate-scalar', capture_projection(
        dict(book['profile'], workspace_id='\ud800'), custody, response_override=response,
    )
    yield 'literal-surrogate-spelling', capture_projection(
        dict(book['profile'], workspace_id='\\ud800'), custody, response_override=response,
    )
    surrogate_response = deepcopy(response)
    surrogate_response['entries'][0]['description'] = '\udfff'
    yield 'surrogate-entry-description', capture_projection(
        book['profile'], custody, response_override=surrogate_response,
    )
    surrogate_rows = tuple(dict(row, pricing_provider='\ud800')
                           for row in custody.finalized_tax_projection.rows)
    yield 'surrogate-source-provenance', capture_projection(
        book['profile'], replace(custody, finalized_tax_projection=replace(
            custody.finalized_tax_projection, rows=surrogate_rows)), response_override=response,
    )
    yield 'surrogate-pass-through-rate', capture_projection(
        book['profile'], replace(custody, rates={'BTC': '\ud800'}), response_override=response,
    )
    shaped_response = deepcopy(response)
    shaped_response['entries'][0]['description'] = ['opaque', {'value': 1}]
    yield 'opaque-entry-description', capture_projection(
        book['profile'], custody, response_override=shaped_response,
    )
    ignored_response = deepcopy(response)
    ignored_response['holdings'].append(dict(
        wallet='unknown', asset='BTC', quantity='00E-1', cost_basis='00E-1',
    ))
    yield 'unused-holding-decimals', capture_projection(
        book['profile'], custody, response_override=ignored_response,
    )
    quarantine_response = dict(
        schema_version=2, ok=True, entries=[], holdings=[], tax_summary=[],
        carried_rows=[], intra_audit=[], quarantines=[dict(
            row=custody.finalized_tax_projection.rows[0]['id'],
            reason='missing_spot_price', detail_kind='plain',
            detail={'large': {'float': '1E999'}},
        )],
    )
    yield 'nonfinite-detail', capture_projection(
        book['profile'], custody, response_override=quarantine_response,
    )
    malformed = replace(custody, direct_payout_records=[dict(
        id='payout', out_transaction_id='source', payout_amount='bad',
    )])
    yield 'malformed-integer', capture_projection(
        book['profile'], malformed, response_override=response,
    )
    malformed_response = deepcopy(response)
    entry = next(e for e in malformed_response['entries'] if e.get('fragments'))
    entry['fragments'][0]['lot']['fiat_fee'] = '00E-1'
    yield 'malformed-canonical-decimal', capture_projection(
        book['profile'], custody, response_override=malformed_response,
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--probe', type=Path, default=Path('target/release/examples/journal-render'))
    parser.add_argument('--output', type=Path, default=Path('target/r4-render/known-gaps'))
    args = parser.parse_args()
    args.output.mkdir(mode=0o700, parents=True, exist_ok=True)
    args.output.chmod(0o700)
    summary = {}
    for name, record in operands():
        path = args.output / f'{name}.json'
        path.touch(mode=0o600, exist_ok=True)
        path.chmod(0o600)
        path.write_text(json.dumps(record, ensure_ascii=True, allow_nan=True))
        probe = subprocess.run([str(args.probe)], input=str(path.resolve()) + '\n',
                               text=True, capture_output=True)
        expected = record['expected']
        if probe.returncode:
            result = dict(matched=False, expected=expected, exit_code=probe.returncode,
                          stderr=probe.stderr)
        else:
            actual = json.loads(probe.stdout)
            matched = (actual.get('state') == expected['state'] if 'state' in expected
                       else actual.get('error') == expected['error'])
            matched = matched and actual.get('entry_ids_consumed') == len(record['entry_ids'])
            result = dict(matched=matched, expected=expected, actual=actual)
        summary[name] = result
        print(f"{name}: {'matched' if result['matched'] else 'DIFFERENT'}", flush=True)
    (args.output / 'summary.json').write_text(json.dumps(summary, ensure_ascii=True, indent=2) + '\n')
    return int(any(not result['matched'] for result in summary.values()))


if __name__ == '__main__':
    raise SystemExit(main())
