#!/usr/bin/env python3
"""Synthetic complete-row payout deletion oracle; creation remains Python setup."""
from contextlib import ExitStack
import hashlib
import json
from pathlib import Path
import tempfile
import uuid
from unittest.mock import patch
from export_pairs import ROOT, STAMP, TABLES, seed, dump, terms, migration, components, handlers, open_db, AppError


def main():
    states = {}; scenarios = []
    for name in ('single', 'independent', 'drift', 'scope', 'superseded', 'gui', 'late-failure'):
        with tempfile.TemporaryDirectory(prefix='kassiber-payout-delete-') as root:
            c = open_db(root); sql = seed(c)
            with ExitStack() as stack:
                for module, field in ((terms, 'now_iso'), (migration, 'now_iso'), (components, '_now_iso')):
                    stack.enter_context(patch.object(module, field, return_value=STAMP))
                stack.enter_context(patch.object(terms.uuid, 'uuid4', return_value=uuid.UUID('00000000-0000-4000-8000-000000000001')))
                writes = []
                c.set_trace_callback(lambda q: writes.append(q) if q.lstrip().split()[0].upper() in ('INSERT', 'UPDATE', 'DELETE') else None)
                payout = handlers.create_direct_swap_payout(c, 'w', 'p', 'd', payout_asset='ETH', payout_amount='0.000000008', notes='€ 🚀', payout_fiat_value='1.2345')
                if name == 'independent':
                    with patch.object(terms.uuid, 'uuid4', return_value=uuid.UUID('00000000-0000-4000-8000-000000000002')):
                        handlers.create_transaction_pair(c, 'w', 'p', 'a', 'b')
                c.set_trace_callback(None)
            # SQLite trace repeats statements when triggers run; replay only the
            # actual top-level writes using consecutive de-duplication.
            for q in writes:
                if not sql or sql[-1] != q: sql.append(q)
            # Triggers cause repeated trace callbacks; restore the exact setup
            # counters after de-duplication (no output values are masked).
            sql.append("UPDATE profiles SET journal_input_version="+str(c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0])+" WHERE id='p'")
            sql.append('UPDATE chain_index_clock SET revision='+str(c.execute('SELECT revision FROM chain_index_clock WHERE id=1').fetchone()[0])+' WHERE id=1')
            sql.append('DELETE FROM chain_index_dirty')
            for row in dump(c, 'chain_index_dirty'):
                sql.append('INSERT INTO chain_index_dirty ('+','.join(row)+') VALUES ('+','.join('NULL' if v is None else str(v) if isinstance(v,int) else "'"+v.replace("'","''")+"'" for v in row.values())+')')
            if name == 'drift':
                sql.append("UPDATE transactions SET amount=900 WHERE id='d'"); c.execute(sql[-1]); c.commit()
            if name == 'superseded':
                sql.append("UPDATE custody_components SET state='superseded',superseded_at='2024-01-01T00:00:00Z' WHERE id='"+payout['component_id']+"'")
                c.execute(sql[-1]); c.commit()
            tables = [r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
            with tempfile.TemporaryDirectory(prefix='kassiber-payout-setup-replay-') as replay_root:
                replay = open_db(replay_root)
                for statement in sql: replay.execute(statement)
                replay.commit()
                for table in TABLES: assert dump(replay, table) == dump(c, table), (name, table)
                replay.close()
            cases=[]
            for i, (id, profile) in enumerate([(payout['id'], 'other' if name == 'scope' else 'p'), (payout['id'],'p'), (payout['id'],'p'), ('missing','p')]):
                setup=[]
                if name == 'late-failure' and i == 0:
                    version=c.execute("SELECT journal_input_version FROM profiles WHERE id='p'").fetchone()[0]
                    setup=[f"CREATE TEMP TRIGGER refuse_payout_invalidation BEFORE UPDATE OF journal_input_version ON profiles WHEN NEW.id='p' AND NEW.journal_input_version={version+2} BEGIN SELECT RAISE(ABORT,'late payout invalidation'); END"]
                elif name == 'late-failure' and i == 1:
                    setup=['DROP TRIGGER refuse_payout_invalidation']
                for statement in setup: c.execute(statement)
                c.commit()
                before={t:dump(c,t) for t in tables}
                now='2025-06-01T00:00:00Z'
                with ExitStack() as stack:
                    for module, field in ((terms,'now_iso'), (migration,'now_iso'), (components,'_now_iso')): stack.enter_context(patch.object(module,field,return_value=now))
                    try: expected={'result':handlers.delete_direct_swap_payout(c,'w',profile,id,authored_source='gui' if name == 'gui' else 'cli')}
                    except Exception as e:
                        c.rollback(); expected={'error':dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None))}
                if name == 'late-failure' and i == 0:
                    assert expected['error']['message'] == 'late payout invalidation'
                    assert {t:dump(c,t) for t in tables} == before
                rows={}
                for table in TABLES:
                    data=dump(c,table); key=hashlib.sha256(json.dumps(data,sort_keys=True,ensure_ascii=True).encode()).hexdigest();states[key]=data;rows[table]=key
                cases.append(dict(operation='transfers.payouts.delete',args={'payout_id':id, 'authored_source':'gui' if name == 'gui' else 'cli'},setup=setup,profile=profile,id='unused',now=now,rows=rows,**expected))
            scenarios.append(dict(name=name,sql=sql,cases=cases));c.close()
    (ROOT/'crates/transfers/tests/payout_delete.json').write_text(json.dumps(dict(states=states,scenarios=scenarios),indent=2,ensure_ascii=True)+'\n')
    print(f'{len(scenarios)} scenarios, {sum(len(s["cases"]) for s in scenarios)} operations')

if __name__=='__main__': main()
