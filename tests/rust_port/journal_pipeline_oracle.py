"""Full-dump oracle for the ordered journal pre-build and its projections."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from dataclasses import asdict, replace
import json
import sqlite3
import uuid
from unittest.mock import patch

from kassiber.cli import handlers
from kassiber.core import custody_authored_migration as migration
from tests.rust_port.journal_prebuild_oracle import ROOT, CLOCK, refusal, seed, transaction, rate
from tests.rust_port.journal_persistence_oracle import dump
from tests.rust_port.journal_source_overlap_oracle import vectors as source_vectors

PATH = ROOT / "crates/journals/tests/data/prebuild-pipeline.json"


def restore(conn, before):
    seed(conn)
    triggers = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger'").fetchall()
    for name, _ in triggers:
        conn.execute(f'DROP TRIGGER "{name}"')
    for table, data in before.items():
        conn.execute(f'DELETE FROM "{table}"')
        columns = ','.join(f'"{name}"' for name in data['columns'])
        placeholders = ','.join('?' for _ in data['columns'])
        for row in data['rows']:
            conn.execute(f'INSERT INTO "{table}"({columns}) VALUES({placeholders})',
                         tuple(bytes.fromhex(v['$bytes']) if isinstance(v, dict) and '$bytes' in v else v for v in row))
    for _, sql in triggers:
        conn.execute(sql)
    conn.commit()


def capture(conn, name, profile_id):
    conn.commit()
    before = dump(conn)
    profile = dict(conn.execute("SELECT * FROM profiles WHERE id=?", (profile_id,)).fetchone())
    initial_profile = profile
    conn.execute("BEGIN IMMEDIATE")
    conn.execute("SAVEPOINT pipeline_oracle")
    ticks = []
    def clock():
        now = f"2026-10-31T12:00:{len(ticks):02}Z"
        ticks.append(now)
        return now
    ids = iter(uuid.UUID(int=i) for i in range(1, 1000))
    result = error = wire_fields = None
    durable_ids = []
    def read_ids():
        for event in conn.execute("SELECT id FROM transaction_edit_events ORDER BY rowid"):
            if any(row[before['transaction_edit_events']['columns'].index('id')] == event[0]
                   for row in before['transaction_edit_events']['rows']):
                continue
            durable_ids.append(event[0])
            durable_ids.extend(r[0] for r in conn.execute("SELECT id FROM transaction_edit_fields WHERE event_id=? ORDER BY rowid", event))
    with ExitStack() as stack:
        stack.enter_context(patch("uuid.uuid4", side_effect=lambda: next(ids)))
        stack.enter_context(patch.object(handlers, "_metadata_hooks", return_value=replace(handlers._metadata_hooks(), now_iso=clock)))
        stack.enter_context(patch.object(migration, "now_iso", side_effect=clock))
        stack.enter_context(patch("kassiber.core.custody_components._now_iso", side_effect=clock))
        for target in ("socket.getaddrinfo", "socket.socket.connect", "socket.socket.connect_ex", "socket.socket.sendto"):
            stack.enter_context(patch(target, side_effect=AssertionError("pre-build attempted network I/O")))
        try:
            repair = handlers._repair_journal_source_overlaps(conn, profile)
            warning = handlers._journal_source_overlap_warning(conn, profile, repair)
            legacy = migration.refresh_drifted_legacy_pair_components(conn, profile_id=profile_id)
            if legacy.changed:
                profile = dict(conn.execute("SELECT * FROM profiles WHERE id=?", (profile_id,)).fetchone())
            priced = handlers.auto_price_transactions_from_rates_cache(conn, profile)
            result = dict(repair=repair, warning=warning, consolidation=asdict(legacy), profile=profile, auto_priced=priced)
            fields = {}
            if repair is not None:
                fields['source_overlap_repair'] = repair
            if warning is not None:
                fields['source_overlap_warning'] = warning
            if legacy.activated:
                fields['custody_pair_evidence_repairs'] = legacy.activated
            wire_fields = json.dumps(fields)
            read_ids()
        except Exception as exc:
            error = refusal(exc)
            read_ids()
            conn.execute("ROLLBACK TO pipeline_oracle")
    conn.execute("RELEASE pipeline_oracle")
    expected = dump(conn)
    conn.rollback()
    return dict(name=name, before=before, expected=expected, result=result, error=error,
                profile=initial_profile, ids=durable_ids, ticks=ticks, wire_fields=wire_fields)


def vectors():
    source = {case['name']: case for case in source_vectors()}
    cases = []
    for name in ('empty', 'cached-pricing', 'all-stages', 'domain-skip', 'pricing-refusal',
                 'legacy-refusal', 'source-warning', 'repeat'):
        conn = sqlite3.connect(':memory:')
        conn.row_factory = sqlite3.Row
        profile_id = 'p' if name in {'empty', 'cached-pricing'} else 'pf'
        if profile_id == 'p':
            seed(conn)
            if name == 'cached-pricing':
                transaction(conn)
                rate(conn)
        else:
            restore(conn, source['unresolved-amount' if name == 'source-warning' else 'duplicate']['before'])
            conn.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,created_at) VALUES('legacy-wallet','ws','pf','Legacy','custom',?)", (CLOCK,))
            for id, direction, amount in (('legacy-out', 'outbound', 1000), ('legacy-in', 'inbound', 900)):
                transaction(conn, id, workspace_id='ws', profile_id='pf', wallet_id='legacy-wallet', direction=direction, amount=amount, fee=0)
            conn.execute("INSERT INTO transaction_pairs(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,kind,policy,pair_source,out_amount,created_at) VALUES('legacy-pair','ws','pf','legacy-out','legacy-in','manual','carrying-value','manual',900,?)", (CLOCK,))
            with patch.object(migration, 'now_iso', return_value=CLOCK), patch('kassiber.core.custody_components._now_iso', return_value=CLOCK):
                migration.refresh_legacy_authored_components(conn)
            conn.execute("UPDATE transactions SET kind='withdrawal' WHERE id='legacy-out'")
            if name in {'domain-skip', 'legacy-refusal'}:
                triggers = conn.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND tbl_name='transaction_pairs'").fetchall()
                for trigger, _ in triggers:
                    conn.execute(f'DROP TRIGGER "{trigger}"')
                if name == 'domain-skip':
                    conn.execute("UPDATE transaction_pairs SET out_amount=1001")
                else:
                    conn.execute("UPDATE transaction_pairs SET component_id='absent'")
                for _, sql in triggers:
                    conn.execute(sql)
            rate(conn, rate_exact='bad' if name == 'pricing-refusal' else '40000')
        if name == 'repeat':
            conn.commit()
            initial_ids = iter(uuid.UUID(int=i) for i in range(1000, 2000))
            with patch.object(migration, 'now_iso', return_value=CLOCK), patch('kassiber.core.custody_components._now_iso', return_value=CLOCK), patch('uuid.uuid4', side_effect=lambda: next(initial_ids)), patch.object(handlers, '_metadata_hooks', return_value=replace(handlers._metadata_hooks(), now_iso=lambda: CLOCK)):
                handlers._repair_journal_source_overlaps(conn, conn.execute("SELECT * FROM profiles WHERE id='pf'").fetchone())
                migration.refresh_drifted_legacy_pair_components(conn, profile_id='pf')
                handlers.auto_price_transactions_from_rates_cache(conn, conn.execute("SELECT * FROM profiles WHERE id='pf'").fetchone())
        cases.append(capture(conn, name, profile_id))
        conn.close()
    return cases


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    cases = vectors()
    base = cases[0]['before']
    packed = dict(base=base, cases=[dict(case, **{side: {table: item['rows'] for table, item in case[side].items() if item != base[table]} for side in ('before', 'expected')}) for case in cases])
    text = json.dumps(packed, ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f'{len(cases)} complete ordered pre-build dumps match Python')


if __name__ == '__main__':
    main()
