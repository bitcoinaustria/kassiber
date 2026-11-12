"""Complete process dumps with real ordered pre-build work and lifecycle clocks."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from dataclasses import replace
from copy import deepcopy
import itertools
import json
import sqlite3
import uuid
from unittest.mock import patch

from kassiber.cli import handlers
from kassiber.core import custody_authored_migration, custody_journal
from tests.rust_port.journal_pipeline_oracle import PATH as PREBUILD, restore
from tests.rust_port.journal_prebuild_oracle import CLOCK, ROOT
# Whole-process refusals keep engine error classes and invariants.
from tests.rust_port.journal_render_oracle import refusal
from tests.rust_port.journal_persistence_oracle import dump

PATH = ROOT / "crates/journals/tests/data/process-native.json"


def inputs():
    packed = json.loads(PREBUILD.read_text())
    cases = []
    for case in packed['cases']:
        before = {table: dict(data, rows=case['before'].get(table, data['rows']))
                  for table, data in packed['base'].items()}
        cases.append(dict(name=case['name'], before=before, profile=case['profile']))
    fresh = next(c for c in json.loads((PATH.parent / 'persistence.json').read_text())
                 if c['name'] == 'fresh')
    broken = json.loads(json.dumps(dict(name='build-refusal', before=fresh['before'],
                                       profile=fresh['record']['profile'])))
    data = broken['before']['profiles']
    id_column = data['columns'].index('id')
    for row in data['rows']:
        if row[id_column] == broken['profile']['id']:
            row[data['columns'].index('gains_algorithm')] = 'invalid-method'
    broken['profile']['gains_algorithm'] = 'invalid-method'
    cases.append(broken)
    for payload in (b'{}', b'', b'\xff\0'):
        binary_config = deepcopy(dict(name='blob-config-' + payload.hex(),
                                      before=fresh['before'], profile=fresh['record']['profile']))
        wallets = binary_config['before']['wallets']
        for row in wallets['rows']:
            row[wallets['columns'].index('config_json')] = {'$bytes': payload.hex()}
        cases.append(binary_config)
    for mode in ('conflicted', 'mixed', 'binding-mismatch'):
        network = deepcopy(dict(name='network-' + mode, before=fresh['before'],
                                profile=fresh['record']['profile']))
        wallets = network['before']['wallets']
        for row in wallets['rows']:
            row[wallets['columns'].index('config_json')] = (
                '{"chain":"unsupported"}' if mode == 'conflicted' else '{}')
        transactions = network['before']['transactions']
        for index, row in enumerate(transactions['rows']):
            row[transactions['columns'].index('raw_json')] = json.dumps(dict(
                chain='bitcoin', network='main' if index == 0 or mode != 'mixed' else 'test'))
        if mode == 'binding-mismatch':
            binding = network['before']['book_network_bindings']
            values = dict(_rowid_=1, profile_id=network['profile']['id'],
                          environment_id='binding-test', environment='test', revision=1,
                          chain_instance_id=None, domains_json='[]',
                          acknowledgements_json='[]', inventory_digest='synthetic',
                          created_at=CLOCK)
            binding['rows'] = [[values.get(column) for column in binding['columns']]]
        cases.append(network)
    for filename in ('persistence.json', 'persistence-stages.json'):
        for case in json.loads((PATH.parent / filename).read_text()):
            if case['name'] == 'filed-change' or 'retraction_inbox' in case['name']:
                cases.append(dict(name=case['name'], before=case['before'],
                                  profile=case['record']['profile']))
    return cases


def capture(case, *, connection=None, operation=None, additional_clocks=(), rollback_expected=True):
    conn = connection if connection is not None else sqlite3.connect(':memory:')
    if connection is None:
        conn.row_factory = sqlite3.Row
    try:
        if connection is None:
            restore(conn, case['before'])
        before = dump(conn)
        profile = case['profile']
        ids, ticks, phases = [], [], []
        counter = itertools.count(1)

        class DurableUUID(uuid.UUID):
            def __str__(self):
                value = super().__str__()
                ids.append(value)
                return value

        def identity():
            # Python also generates UUID.hex savepoint names. Those never
            # enter the book or result; retain the exact serialized UUIDs.
            return DurableUUID(int=next(counter), version=4)

        def clock():
            now = f"2026-10-31T12:00:{len(ticks):02}Z"
            ticks.append(now)
            return now

        original_build = custody_journal.build_ledger_state
        hooks = replace(handlers._metadata_hooks(), now_iso=clock)

        def build(*args, **kwargs):
            # Rendering's transient UUIDs do not survive deterministic storage.
            # Keep them separate from the unmasked durable identity tape.
            transient = itertools.count(100_000)
            with patch.object(uuid, 'uuid4', side_effect=lambda: uuid.UUID(int=next(transient))):
                return original_build(*args, **kwargs)

        result = error = None
        with ExitStack() as stack:
            stack.enter_context(patch.object(uuid, 'uuid4', side_effect=identity))
            stack.enter_context(patch.object(handlers, '_metadata_hooks', return_value=hooks))
            stack.enter_context(patch.object(custody_authored_migration, 'now_iso', side_effect=clock))
            stack.enter_context(patch('kassiber.core.custody_components._now_iso', side_effect=clock))
            stack.enter_context(patch.object(custody_journal, 'now_iso', side_effect=clock))
            stack.enter_context(patch('kassiber.core.custody_filed_reports.now_iso', side_effect=clock))
            stack.enter_context(patch('kassiber.core.filed_report_chain.now_iso', side_effect=clock))
            stack.enter_context(patch.object(custody_journal, 'custody_build_clock', return_value=CLOCK))
            stack.enter_context(patch.object(custody_journal, 'build_ledger_state', side_effect=build))
            for target in additional_clocks:
                stack.enter_context(patch(target, side_effect=clock))
            for target in ('socket.getaddrinfo', 'socket.socket.connect',
                           'socket.socket.connect_ex', 'socket.socket.sendto'):
                stack.enter_context(patch(target, side_effect=AssertionError('process attempted network I/O')))
            try:
                result = (operation or handlers.process_journals)(
                    conn, profile['workspace_id'], profile['id'], progress_observer=phases.append)
            except Exception as exc:
                error = refusal(exc)
        expected = dump(conn)
        if error is not None and rollback_expected:
            assert expected == before, case['name']
        if case['name'] == 'build-refusal':
            assert error is not None and phases[-1] == {'phase': 'building'}, (error, phases)
        return dict(case, before=before, expected=expected, result=result, error=error,
                    ids=ids, ticks=ticks, phases=phases, clock=CLOCK)
    finally:
        if connection is None:
            conn.close()


def vectors():
    return [capture(case) for case in inputs()]


def packed_vectors():
    cases = vectors()
    base = cases[0]['before']
    return dict(base=base, cases=[dict(case, **{
        side: {table: item['rows'] for table, item in case[side].items() if item != base[table]}
        for side in ('before', 'expected')}) for case in cases])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    packed = packed_vectors()
    text = json.dumps(packed, ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f"{len(packed['cases'])} real pre-build process dumps match Python")


if __name__ == '__main__':
    main()
