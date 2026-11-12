"""Opt-in pytest-only capture of actual persistence boundaries on disposable books.

Load with -p and set KASSIBER_TEST_JOURNAL_PERSIST_DIR to a private target/ path.
The plugin observes build operands and UUIDs; only the oracle clock is injected.
It never changes product registration or touches non-test books.
"""
from contextlib import ExitStack
from contextvars import ContextVar
import copy
import itertools
import json
import os
from pathlib import Path
import uuid
from unittest.mock import patch

from kassiber.core import custody_journal,custody_filed_reports,filed_report_chain
from tests.rust_port.journal_render_oracle import capture_build,_ACTIVE_CAPTURE
from tests.rust_port.journal_persistence_oracle import dump

_LAST=ContextVar('persistence_last_build',default=None)
_SEQUENCE=itertools.count()


def pytest_configure(config):
    directory=os.environ.get('KASSIBER_TEST_JOURNAL_PERSIST_DIR')
    if not directory: raise RuntimeError('capture directory is required')
    directory=Path(directory).resolve()
    target=Path(__file__).resolve().parents[2]/'target'
    if not directory.is_relative_to(target): raise RuntimeError('capture scratch must be under target/')
    directory.mkdir(parents=True,exist_ok=True)
    original_build=custody_journal.CustodyJournalBuilder.build
    original_store=custody_journal.store_ledger_state
    original_uuid=uuid.uuid4

    def build(self):
        if _ACTIVE_CAPTURE.get(): return original_build(self)
        with capture_build(self) as (record,_):
            state=original_build(self)
        record.pop('_custody',None)
        _LAST.set((state,record,copy.deepcopy(state['entries'])))
        return state

    def store(conn,profile,state,**kwargs):
        last=_LAST.get()
        if last is None or last[0] is not state: return original_store(conn,profile,state,**kwargs)
        clock=kwargs.get('created_at') or custody_journal.now_iso()
        before=dump(conn)
        schema=[list(row) for row in conn.execute(
            "SELECT type,name,tbl_name,sql FROM sqlite_master "
            "WHERE sql IS NOT NULL ORDER BY type,name").fetchall()]
        # Storage tests may edit the built entries before passing them to the
        # writer. Retain those exact inputs, independently of render parity.
        fields=('id','occurred_at','description')
        original_entries={entry['id']:entry for entry in last[2]}
        for entry in state['entries']:
            original=original_entries[entry['id']]
            assert {k:v for k,v in original.items() if k not in fields} == {
                k:v for k,v in entry.items() if k not in fields
            }, 'unsupported persistence-only entry edit'
        entry_inputs=[dict(id=entry['id'], **{
            key:entry[key] for key in fields[1:]
            if entry[key] != original_entries[entry['id']][key]
        }) for entry in state['entries']]
        ids=[]
        def identity():
            ident=original_uuid();ids.append(str(ident));return ident
        with ExitStack() as stack:
            stack.enter_context(patch.object(uuid,'uuid4',identity))
            for module in (custody_journal,custody_filed_reports,filed_report_chain):
                stack.enter_context(patch.object(module,'now_iso',lambda:clock))
            result=original_store(conn,profile,state,created_at=clock)
        record=last[1]
        payload=dict(name=os.environ.get('PYTEST_CURRENT_TEST','unknown'),record=record,before=before,
                     schema=schema,entry_inputs=entry_inputs,
                     expected=dump(conn),created_at=clock,ids=ids)
        path=directory/f'{os.getpid()}-{next(_SEQUENCE):06}.json'
        path.write_text(json.dumps(payload,ensure_ascii=True,separators=(',',':'))+'\n')
        return result

    custody_journal.CustodyJournalBuilder.build=build
    custody_journal.store_ledger_state=store
    config.add_cleanup(lambda:setattr(custody_journal.CustodyJournalBuilder,'build',original_build))
    config.add_cleanup(lambda:setattr(custody_journal,'store_ledger_state',original_store))
