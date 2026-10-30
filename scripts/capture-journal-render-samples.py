#!/usr/bin/env python3
"""Capture one scenario cycle from each raw-book generator and optional tax cases."""
import argparse
from collections import Counter
from dataclasses import replace
import importlib
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from tests.rust_port.journal_render_oracle import capture_projection, refusal, write_case

FAMILIES = [
    ('snapshot', 'snapshot', 'SCENARIOS'),
    ('snapshot', 'component_snapshot', 'COMPONENT_SCENARIOS'),
    ('interpretation', 'samourai_snapshot', 'SAMOURAI_SCENARIOS'),
    ('interpretation', 'recorded_snapshot', 'RECORDED_SCENARIOS'),
    ('fanout', 'fanout_snapshot', 'FANOUT_SCENARIOS'),
    ('native', 'native_snapshot', 'NATIVE_SCENARIOS'),
    ('graph', 'graph_snapshot', 'GRAPH_SCENARIOS'),
    ('channel', 'channel_snapshot', 'CHANNEL_SCENARIOS'),
    ('gap', 'snapshot', 'SCENARIOS'),
    ('reconciliation', 'snapshot', 'SCENARIOS'),
    ('safety', 'safety_snapshot', 'SAFETY_SCENARIOS'),
    ('pipeline', 'pipeline_snapshot', 'PIPELINE_SCENARIOS'),
]


def capture_tax_cases(directory):
    from kassiber.core.custody_compute import compute_custody_projection
    from tests.custody_snapshot_generator import snapshot
    from tests.tax_ledger_generator import corpus

    root = directory / 'tax-corpus'
    root.mkdir(exist_ok=True)
    base = compute_custody_projection(snapshot(0))
    counts = Counter()
    for case in corpus():
        custody = replace(
            base,
            finalized_tax_projection=case.inputs.finalized_tax_projection,
            wallet_refs_by_id=case.inputs.wallet_refs_by_id,
            direct_payout_records=case.inputs.direct_payout_records,
        )
        record = capture_projection(case.profile, custody)
        (root / f'{case.name}.json').write_text(json.dumps(record, ensure_ascii=True, allow_nan=True))
        counts['books'] += 1
        counts.update(record['activity'])
        counts['pre_render_refusals'] += int(record['response'] is None and 'error' in record['expected'])
    print('tax-corpus', dict(counts), flush=True)
    return dict(counts=counts)

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory', type=Path)
    parser.add_argument('--tax-corpus', action='store_true')
    args=parser.parse_args()
    args.directory.mkdir(parents=True, exist_ok=True)
    summary={}
    for family, function, scenarios in FAMILIES:
        module=importlib.import_module(f'tests.custody_{family}_generator')
        name=f'{family}-{function}'
        root=args.directory/name
        root.mkdir(exist_ok=True)
        counts=Counter()
        errors=[]
        for seed,scenario in enumerate(getattr(module,scenarios)):
            try:
                record=write_case(getattr(module,function)(seed),root/f'{seed:03}.json')
            except Exception as error:
                counts['upstream_refusals']+=1
                errors.append(dict(scenario=scenario, error=refusal(error)))
                continue
            counts['books']+=1
            counts.update(record['activity'])
            counts['pre_render_refusals']+=int(record['response'] is None and 'error' in record['expected'])
        summary[name]=dict(counts=counts, upstream_refusals=errors)
        print(name, dict(counts), flush=True)
    if args.tax_corpus:
        summary['tax-corpus'] = capture_tax_cases(args.directory)
    (args.directory/'activity.json').write_text(json.dumps(summary,indent=2)+'\n')

if __name__=='__main__':
    main()
