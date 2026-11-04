"""Full-book automatic-pairing refresh oracle, including both commit boundaries."""
from __future__ import annotations

import argparse
from contextlib import ExitStack
from datetime import datetime, timezone
import itertools
import json
import sqlite3
import uuid
from unittest.mock import patch

from crates.transfers.scripts.export_pairing_services import generated, seed as seed_pairing
from kassiber import daemon_freshness
from kassiber.cli import handlers
from tests.rust_port.journal_native_process_oracle import capture
from tests.rust_port.journal_prebuild_oracle import ROOT, CLOCK, seed

PATH = ROOT / 'crates/journals/tests/data/process-refresh.json'
SCENARIOS = ('exact', 'ambiguous', 'tolerance', 'multi-wallet', 'rules', 'dismissal',
             'unpaired', 'ownership', 'booked', 'guards-after-rule', 'fee-variants')


def book(scenario, *, connection=None):
    conn = connection
    if conn is None:
        conn = sqlite3.connect(':memory:')
        conn.row_factory = sqlite3.Row
        seed(conn)
    conn.execute('DELETE FROM wallets')
    conn.execute('DELETE FROM profiles')
    conn.execute('DELETE FROM workspaces')
    rows, setup, actions, _ = generated(1, scenario)
    seed_pairing(conn, rows)
    for statement in setup:
        conn.execute(statement)
    if actions:
        counter = itertools.count(100_000)
        with ExitStack() as stack:
            stack.enter_context(patch('uuid.uuid4', side_effect=lambda: uuid.UUID(int=next(counter))))
            for target in ('kassiber.core.custody_review_terms.now_iso',
                           'kassiber.core.custody_components._now_iso',
                           'kassiber.core.custody_authored_migration.now_iso'):
                stack.enter_context(patch(target, return_value=CLOCK))
            pair = None
            for action, _ in actions:
                if action == 'pair':
                    pair = handlers.create_transaction_pair(conn, 'w', 'p', 'a-o', 'a-i', kind='chain-swap')
                else:
                    handlers.delete_transaction_pair(conn, 'w', 'p', pair['id'])
    conn.commit()
    return conn


def run(conn, name, *, auto_pair=True, skip=False, schema_sql=(), current_before_rebuild=False,
        fail_pairing_clock=None):
    profile = dict(conn.execute("SELECT * FROM profiles WHERE id='p'").fetchone())
    precise_reads = []

    class Frozen(datetime):
        @classmethod
        def now(cls, tz=None):
            precise_reads.append(CLOCK)
            if fail_pairing_clock == len(precise_reads):
                raise RuntimeError('injected pairing clock failure')
            return cls(2026, 10, 31, 12, tzinfo=timezone.utc)

    def refresh(conn, _workspace, profile_id, *, progress_observer):
        def observe(value):
            if current_before_rebuild and value['phase'] == 'preparing':
                assert daemon_freshness.core_custody_journal.projection_freshness(conn, profile_id)['is_current']
            progress_observer(value)

        return daemon_freshness.refresh_journals_step(
            conn, profile_id, auto_pair=auto_pair, skip_rebuild_when_current=skip,
            phase=observe, journal_progress_observer=observe,
        )

    with patch('kassiber.core.transfer_matching.datetime', Frozen):
        case = capture(
            dict(name=name, profile=profile), connection=conn, operation=refresh,
            additional_clocks=('kassiber.core.custody_review_terms.now_iso',),
            rollback_expected=False,
        )
    case.update(auto_pair=auto_pair, skip_rebuild_when_current=skip,
                schema_sql=list(schema_sql),
                precise_reads=len(precise_reads) + sum(p['phase'] == 'building' for p in case['phases']),
                payload_json=json.dumps(case['result']) if case['error'] is None else None)
    case['profile'] = {key: {'$kassiber_bytes': value.hex()} if isinstance(value, bytes) else value
                       for key, value in case['profile'].items()}
    return case


def vectors():
    cases = []
    for scenario in SCENARIOS:
        for current, skip in ((False, False), (True, False), (True, True), (False, True)):
            conn = book(scenario)
            try:
                if current:
                    conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count="
                                 "(SELECT COUNT(*) FROM transactions WHERE profile_id='p' AND excluded=0),"
                                 "last_processed_input_version=journal_input_version WHERE id='p'", (CLOCK,))
                    # Python resets the semantics stamp whenever markers move.
                    conn.execute("UPDATE profiles SET journal_semantics_version=2 WHERE id='p'")
                else:
                    conn.execute("UPDATE profiles SET last_processed_at=NULL WHERE id='p'")
                conn.commit()
                cases.append(run(conn, f'{scenario}-current{int(current)}-skip{int(skip)}', skip=skip))
            finally:
                conn.close()
    for auto_pair in (False, True):
        for skip in (False, True):
            conn = book('exact')
            try:
                conn.execute('DELETE FROM transactions')
                conn.commit()
                cases.append(run(conn, f'empty-pair{int(auto_pair)}-skip{int(skip)}', auto_pair=auto_pair, skip=skip))
            finally:
                conn.close()
    for failure, trigger in (
        ('pairing-cache-failure', "CREATE TRIGGER refresh_failure BEFORE UPDATE OF swap_candidate_count ON profiles BEGIN SELECT RAISE(ABORT,'pairing cache failed'); END"),
        ('second-rebuild-failure', "CREATE TRIGGER refresh_failure BEFORE UPDATE OF last_processed_at ON profiles WHEN NEW.last_processed_at IS NOT NULL AND EXISTS(SELECT 1 FROM custody_component_economic_terms WHERE term_kind='transaction_pair') BEGIN SELECT RAISE(ABORT,'second rebuild failed'); END"),
    ):
        for current in (False, True):
            conn = book('exact')
            try:
                conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=2,last_processed_input_version=journal_input_version WHERE id='p'", (CLOCK if current else None,))
                if current:
                    # Seed exactly like the exact case, whose book recovery reuses.
                    conn.execute("UPDATE profiles SET journal_semantics_version=2 WHERE id='p'")
                conn.execute(trigger)
                conn.commit()
                cases.append(run(conn, f'{failure}-current{int(current)}', schema_sql=(trigger,)))
            finally:
                conn.close()
    for name, count, version in (
        ('bytes-markers', b' +2 ', b'0'), ('float-markers', 2.9, 0.9),
        ('invalid-text-marker', '²', 0), ('invalid-bytes-marker', b'\xff', 0),
    ):
        conn = book('exact')
        try:
            conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=?,"
                         "journal_input_version=?,last_processed_input_version=? WHERE id='p'",
                         (CLOCK, count, version, version))
            conn.execute("UPDATE profiles SET journal_semantics_version=2 WHERE id='p'")
            conn.commit()
            cases.append(run(conn, name, auto_pair=False, skip=True))
        finally:
            conn.close()
    conn = book('exact')
    try:
        conn.execute("UPDATE profiles SET last_processed_at=?,last_processed_tx_count=2,"
                     "last_processed_input_version=journal_input_version WHERE id='p'", (CLOCK,))
        conn.execute("UPDATE profiles SET journal_semantics_version=2 WHERE id='p'")
        trigger = (
            "CREATE TRIGGER keep_refresh_current AFTER UPDATE OF last_processed_at,journal_input_version ON profiles "
            "BEGIN UPDATE profiles SET last_processed_at='" + CLOCK + "',last_processed_tx_count="
            "(SELECT COUNT(*) FROM transactions WHERE profile_id=NEW.id AND excluded=0),"
            "last_processed_input_version=journal_input_version WHERE id=NEW.id; END"
        )
        # The schema's unstamping trigger fires after this one; restamp after it.
        stamp = (
            "CREATE TRIGGER keep_refresh_semantics AFTER UPDATE OF journal_semantics_version ON profiles "
            "WHEN NEW.journal_semantics_version=0 "
            "BEGIN UPDATE profiles SET journal_semantics_version=2 WHERE id=NEW.id; END"
        )
        conn.execute(trigger)
        conn.execute(stamp)
        conn.commit()
        cases.append(run(conn, 'applied-pair-keeps-current-markers', skip=True,
                         schema_sql=(trigger, stamp), current_before_rebuild=True))
    finally:
        conn.close()
    return cases


def packed_vectors():
    cases = vectors()
    base = cases[0]['before']
    return dict(base=base, cases=[dict(case, **{
        side: {table: item['rows'] for table, item in case[side].items() if item != base[table]}
        for side in ('before', 'expected')}) for case in cases])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--write', action='store_true')
    args = parser.parse_args()
    packed = packed_vectors()
    text = json.dumps(packed, ensure_ascii=True, separators=(',', ':'), allow_nan=False) + '\n'
    if args.write:
        PATH.write_text(text)
    else:
        assert PATH.read_text() == text
    print(f"{len(packed['cases'])} complete refresh books match Python")


if __name__ == '__main__':
    main()
