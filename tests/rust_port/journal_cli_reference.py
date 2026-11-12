"""Disposable CLI reference with clocks bound for registered-route byte tests."""
from contextlib import ExitStack
from datetime import datetime
import os
from pathlib import Path
import sys
import itertools
import uuid
from unittest.mock import patch


def main():
    from kassiber.operator import project
    from kassiber.cli.main import main as cli
    from kassiber.core import custody_journal
    owner = Path(os.environ['KASSIBER_RUST_TEST_OWNER_ROOT'])
    with ExitStack() as stack:
        stack.enter_context(patch.object(project, '_owner_lock_root', lambda: owner))
        if os.environ.get('KASSIBER_RUST_TEST_JOURNAL_FAIL_PAIRED_STORE') == '1':
            original_process = custody_journal.process_journals
            def process(conn, *args, **kwargs):
                conn.execute("CREATE TEMP TRIGGER IF NOT EXISTS refresh_failure BEFORE UPDATE OF last_processed_at ON profiles "
                             "WHEN NEW.last_processed_at IS NOT NULL AND EXISTS(SELECT 1 FROM custody_component_economic_terms "
                             "WHERE term_kind='transaction_pair') BEGIN SELECT RAISE(ABORT,'second rebuild failed'); END")
                return original_process(conn, *args, **kwargs)
            stack.enter_context(patch.object(custody_journal, 'process_journals', process))
        if os.environ.get('KASSIBER_RUST_TEST_JOURNAL_IDS') == 'sequential':
            counter = itertools.count(1)
            class DurableUUID(uuid.UUID):
                def __str__(self):
                    return str(uuid.UUID(int=next(counter), version=4))
            original_build = custody_journal.build_ledger_state
            original_uuid = uuid.uuid4
            def build(*args, **kwargs):
                with patch.object(uuid, 'uuid4', original_uuid):
                    return original_build(*args, **kwargs)
            stack.enter_context(patch.object(uuid, 'uuid4', lambda: DurableUUID(int=0, version=4)))
            stack.enter_context(patch.object(custody_journal, 'build_ledger_state', build))
        if clock := os.environ.get('KASSIBER_RUST_TEST_JOURNAL_TIME'):
            for target in (
                'kassiber.core.custody_journal.now_iso',
                'kassiber.core.custody_journal.custody_build_clock',
                'kassiber.core.custody_filed_reports.now_iso',
                'kassiber.core.filed_report_chain.now_iso',
                'kassiber.core.custody_components._now_iso',
                'kassiber.core.custody_review_terms.now_iso',
                'kassiber.core.custody_authored_migration.now_iso',
                'kassiber.core.review_workflow.now_iso',
                'kassiber.cli.handlers.now_iso',
            ):
                stack.enter_context(patch(target, return_value=clock))
            class Frozen(datetime):
                @classmethod
                def now(cls, tz=None):
                    return datetime.fromisoformat(clock.replace('Z', '+00:00'))
            stack.enter_context(patch('kassiber.core.transfer_matching.datetime', Frozen))
        return cli(sys.argv[1:])


if __name__ == '__main__':
    raise SystemExit(main())
