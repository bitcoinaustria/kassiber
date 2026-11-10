"""Generated exact-width compiler comparisons with independently counted activity.

Inputs are private typed compiler operands, separate from raw snapshot v1.
Each refusal/control has an independently compiled active companion, so no
scenario can succeed by emitting only diagnostics or no claims.
"""
from copy import deepcopy
from dataclasses import fields
from functools import lru_cache
import json
from pathlib import Path

from kassiber.core import custody_evidence as evidence
from kassiber.core.custody_component_claims import compile_component_quantity_claims
from kassiber.core.custody_native_audit import compile_verified_native_claims
from tests.custody_quantity_composition_vectors import PATH
from tests.custody_quantity_composition_vectors import outcome

COUNTS = Path(__file__).parent / 'fixtures/custody_quantity_composition_counts.json'


def restore(value):
    if isinstance(value, list):
        return tuple(restore(v) for v in value)
    if not isinstance(value, dict):
        return value
    if '$type' in value:
        cls = getattr(evidence, value['$type'])
        return cls(**{f.name: restore(value[f.name]) for f in fields(cls)})
    return {k: restore(v) for k, v in value.items()}


def evaluate(v):
    if v['op'] == 'reviewed':
        observations = v['observations']
        if isinstance(observations, list):
            observations = dict(observations)
        return outcome(lambda: compile_component_quantity_claims(v['component'], restore(observations)))
    return outcome(lambda: compile_verified_native_claims(restore(v['canonical']), v['audit'], component_transaction_ids=v['excluded'], reserved_source_msat=v['reserved']))


def activity(result):
    if 'expected' not in result:
        return 0
    return len(result['expected']['claims'])


@lru_cache(maxsize=1)
def scenarios():
    return json.loads(PATH.read_text())


_evaluated = {}


def evaluate_distinct(value):
    key = json.dumps(value, sort_keys=True, separators=(',', ':'))
    if key not in _evaluated:
        _evaluated[key] = evaluate(value)
    return deepcopy(_evaluated[key])


def scenario(index, variant):
    corpus = scenarios()
    template = corpus[index]
    value = {k: deepcopy(v) for k, v in template.items() if k not in ('expected', 'error', 'case', 'message', 'details')}
    # Scale exact quantities, including beyond machine width and one-msat
    # offsets; booleans and source metadata are deliberately left alone.
    multiplier = (1, 3, 10**50 + 1, 10**90 + 7)[variant % 4]
    amount_fields = {'amount_msat', 'fee_msat', 'source_amount_msat', 'sink_amount_msat', 'swap_fee_msat', 'payout_amount_msat', 'reviewed_source_amount_msat', 'crypto_sent_msat', 'crypto_received_msat', 'crypto_fee_msat'}
    def scale(v):
        if isinstance(v, list):
            return [scale(item) for item in v]
        if not isinstance(v, dict):
            return v
        return {k: item * multiplier if k in amount_fields and type(item) is int else scale(item) for k, item in v.items()}
    value = scale(value)
    if value['op'] == 'native':
        value['reserved'] = {k: v * multiplier if type(v) is int else v for k, v in value['reserved'].items()}
        # Lexical timestamps remain valid even when they aren't chronological.
        for event in value['canonical']['events']:
            for o in event['legs']:
                if variant % 2:
                    o['occurred_at'] = '2025-01-01T00:00:00+01:00'
        if variant % 2:
            for audit in value['audit']:
                if audit.get('occurred_at'):
                    audit['occurred_at'] = '2025-01-01T00:00:00+01:00'
    else:
        if isinstance(value['observations'], dict):
            value['observations'] = list(value['observations'].items())
    expected = evaluate_distinct(value)
    # Same operation's separately compiled positive witness. It cannot mask
    # any mismatch/refusal in the scenario's first result.
    companion = next(c for c in corpus if c['op'] == value['op'] and activity(c) > 0)
    companion = {k: deepcopy(v) for k, v in companion.items() if k not in ('expected', 'error', 'case', 'message', 'details')}
    if companion['op'] == 'reviewed':
        if isinstance(companion['observations'], dict):
            companion['observations'] = list(companion['observations'].items())
    positive = evaluate_distinct(companion)
    active = activity(expected) + activity(positive)
    assert active > 0
    return template['op'] + ':' + template['case'], [value, companion], [expected, positive], active




def variants(index):
    """Yield only distinct primary operands; repetitions are not evidence."""
    seen = set()
    for variant in range(8):
        _, operands, _, _ = scenario(index, variant)
        key = json.dumps(operands[0], sort_keys=True, separators=(',', ':'))
        if key not in seen:
            seen.add(key)
            yield variant


if __name__ == '__main__':
    from collections import Counter
    counts = Counter()
    for i in range(len(scenarios())):
        for variant in variants(i):
            name, _, _, active = scenario(i, variant)
            counts[name] += active
    COUNTS.write_text(json.dumps(counts, sort_keys=True, indent=2) + '\n')
