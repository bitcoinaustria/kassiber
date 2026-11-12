"""R4-11 oracle: execute production build(), capture its exact render operands.

Only test code injects the already finalized projection and UUID source. Every
state field is encoded with the custody oracle's type tags, without redaction or
masking. UUIDs are declared individually and used identically on each side;
created_at is a persistence concern and does not exist in build() state.
"""
from __future__ import annotations
from contextlib import contextmanager
import dataclasses
from contextlib import nullcontext
from contextvars import ContextVar
import json
from pathlib import Path
import time
import uuid
from unittest.mock import patch

from kassiber.core import custody_journal
from kassiber.core.custody_compute import compute_custody_projection
from kassiber.core.custody_snapshot import encode_owned_index
from kassiber.core.engines import ledger, ledger_response
from tests.custody_snapshot import to_jsonable

CLOCK = '2026-10-30T12:00:00Z'
_ACTIVE_CAPTURE = ContextVar('journal_render_capture', default=False)


def canonical(value):
    return json.dumps(value, sort_keys=True, ensure_ascii=True,
                      separators=(',', ':'), allow_nan=False)


def refusal(exc):
    return {'family': type(exc).__name__, 'message': str(exc),
            **{key: getattr(exc, key, None) for key in
               ('code', 'hint', 'details', 'retryable', 'error_class', 'invariant')},
            'cause': refusal(exc.__cause__) if exc.__cause__ is not None else None}


@contextmanager
def capture_build(builder):
    """Observe a real builder; no product hook or behavior is modified."""
    original_projection = builder.build_custody_projection
    original_reader = ledger.read_ledger
    record = {'profile': dict(builder.profile), 'response': None, 'entry_ids': [],
              'clock': builder.as_of, 'uuid_mapping': []}
    elapsed = [0.0]

    def projection():
        c = original_projection()
        record['_custody'] = c
        facts = dataclasses.replace(c, owned_index=encode_owned_index(c.owned_index, c.rows))
        record['custody'] = to_jsonable(facts, redact_config=False)
        record['wallets'] = to_jsonable(list(c.wallet_refs_by_id.values()), redact_config=False)
        return c

    def reader(profile, inputs, response):
        record['response'] = response
        start = time.perf_counter()
        try:
            with patch.object(uuid, 'uuid4', identity), \
                 patch.object(ledger_response, 'journal_entry_id', entry_identity):
                return original_reader(profile, inputs, response)
        finally:
            elapsed[0] += time.perf_counter() - start

    original_uuid4 = uuid.uuid4
    original_entry_id = ledger_response.journal_entry_id

    def entry_identity(*args):
        # Observe each id at generation, including builds that later refuse.
        text = original_entry_id(*args)
        record['entry_ids'].append(text)
        record['uuid_mapping'].append({'python': text, 'rust': text})
        return text

    def identity():
        generated = original_uuid4()
        text = str(generated)
        record['entry_ids'].append(text)
        # Declared bijection, with no wildcard replacement anywhere in state.
        record['uuid_mapping'].append({'python': text, 'rust': text})
        return generated

    token = _ACTIVE_CAPTURE.set(True)
    try:
        with patch.object(builder, 'build_custody_projection', projection), \
             patch.object(ledger, 'read_ledger', reader):
            yield record, elapsed
    finally:
        _ACTIVE_CAPTURE.reset(token)


def capture(snapshot):
    c = compute_custody_projection(snapshot)
    return capture_projection(snapshot['profile'], c)


def capture_projection(profile, c, *, response_override=None):
    builder = custody_journal.CustodyJournalBuilder(None, profile, as_of=c.as_of)
    builder.build_custody_projection = lambda: c
    override = (patch.object(ledger.KassiberTaxEngine, 'build_ledger_state',
                            lambda self, inputs: ledger.read_ledger(self.profile, inputs, response_override))
                if response_override is not None else nullcontext())
    with override, capture_build(builder) as (record, elapsed):
        start = time.perf_counter()
        try:
            state = builder.build()
        except Exception as exc:
            record['expected'] = {'error': refusal(exc)}
        else:
            record['expected'] = {'state': canonical(to_jsonable(state, redact_config=False))}
        record['python_build_seconds'] = time.perf_counter() - start
    record.pop('_custody', None)
    record['python_render_seconds'] = elapsed[0]
    record['activity'] = {
        'entries': len(state['entries']) if 'state' in locals() else 0,
        'quarantines': len(state['quarantines']) if 'state' in locals() else 0,
        'ownership_candidates': len(state['ownership_review_candidates']) if 'state' in locals() else 0,
        'differences': len(state['quantity_differences']) if 'state' in locals() else 0,
        'refusals': int('error' in record['expected']),
    }
    return record


def write_case(snapshot, path):
    record = capture(snapshot)
    Path(path).write_text(json.dumps(record, ensure_ascii=True, allow_nan=True))
    return record


_ORIGINAL_BUILD = None


def install_from_environment():
    """Opt-in pytest/child hook. Artifacts may contain private book facts."""
    import functools
    import itertools
    import os
    global _ORIGINAL_BUILD
    destination = os.environ.get('KASSIBER_TEST_JOURNAL_RENDER_DIR')
    if not destination or _ORIGINAL_BUILD is not None:
        return
    root = Path(destination)
    root.mkdir(parents=True, exist_ok=True)
    sequence = itertools.count()
    _ORIGINAL_BUILD = custody_journal.CustodyJournalBuilder.build

    @functools.wraps(_ORIGINAL_BUILD)
    def observed(builder):
        if _ACTIVE_CAPTURE.get():
            return _ORIGINAL_BUILD(builder)
        with capture_build(builder) as (record, elapsed):
            try:
                state = _ORIGINAL_BUILD(builder)
            except Exception as exc:
                record['expected'] = {'error': refusal(exc)}
                raise
            else:
                record['expected'] = {'state': canonical(to_jsonable(state, redact_config=False))}
                return state
            finally:
                record.pop('_custody', None)
                record['python_render_seconds'] = elapsed[0]
                if 'custody' in record:
                    (root / f'{os.getpid()}-{next(sequence):06}.json').write_text(
                        json.dumps(record, ensure_ascii=True, allow_nan=True))
    custody_journal.CustodyJournalBuilder.build = observed


def pytest_configure(config):
    install_from_environment()


def pytest_unconfigure(config):
    global _ORIGINAL_BUILD
    if _ORIGINAL_BUILD is not None:
        custody_journal.CustodyJournalBuilder.build = _ORIGINAL_BUILD
        _ORIGINAL_BUILD = None
