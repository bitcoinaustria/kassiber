#!/usr/bin/env python3
"""Read oracle over stored component authoring states, including private CLI views."""
import argparse
import json
from pathlib import Path
import sqlite3
import tempfile
from unittest.mock import patch
from export_pairs import ROOT, components, handlers, open_db, AppError


def dump(c):
    tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    return {t:sorted((dict(r) for r in c.execute('SELECT * FROM "'+t+'"')),key=lambda r:json.dumps(r,sort_keys=True)) for t in tables}


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args()
    source=json.loads((ROOT/'crates/transfers/tests/components.json').read_text())
    template=sqlite3.connect(':memory:')
    connect=sqlite3.connect
    clock=connect(':memory:')
    def frozen_connect(*args,**kwargs):
        c=connect(*args,**kwargs)
        def strftime(*values):
            values=tuple('2025-01-01T00:00:00Z' if v=='now' else v for v in values)
            return clock.execute('SELECT strftime('+','.join('?' for _ in values)+')',values).fetchone()[0]
        c.create_function('strftime',-1,strftime)
        return c
    with tempfile.TemporaryDirectory(prefix='component-template-') as root:
        with patch('kassiber.db.sqlite3.connect',side_effect=frozen_connect):
            initial=open_db(root)
        initial.execute("UPDATE settings SET value='00000000000000000000000000000000' WHERE key='database_instance_id'")
        initial.commit();initial.backup(template);initial.close()
    clock.close()
    scenarios=[]
    for scenario in source['scenarios']:
        if scenario['name']=='pair-revision':continue
        chosen={len(scenario['cases'])-1}
        chosen.update(i for i,c in enumerate(scenario['cases']) if c['operation'].endswith('apply') and 'result' in c)
        # Distinct stored states only, retaining drafts, history, conflicts,
        # native support and recovered or stale evidence from the authoring oracle.
        seen=set()
        for i in sorted(chosen):
            case=scenario['cases'][i]
            signature=json.dumps(case['rows'],sort_keys=True)
            if signature in seen:continue
            seen.add(signature)
            with tempfile.TemporaryDirectory(prefix='component-reads-') as root:
                c=sqlite3.connect(':memory:');template.backup(c);c.row_factory=sqlite3.Row;c.execute('PRAGMA foreign_keys=ON')
                for q in scenario['sql']:c.execute(q)
                for prior in scenario['cases'][:i+1]:
                    for q in prior.get('setup',[]):
                        if 'CREATE TEMP TRIGGER' not in q:c.execute(q)
                c.commit();c.execute('PRAGMA foreign_keys=OFF')
                for t in reversed(source['tables']):c.execute('DELETE FROM "'+t+'"')
                restore_order=['chain_index_clock','profiles','wallets','custody_components','custody_component_legs','custody_authored_evidence_snapshots','custody_component_allocations','custody_component_economic_terms','custody_component_evidence_commitments','custody_component_transaction_memberships']
                for t in restore_order:
                    for row in source['states'][case['rows'][t]]:
                        c.execute('INSERT INTO "'+t+'"('+','.join('"'+k+'"' for k in row)+') VALUES('+','.join('?' for _ in row)+')',tuple(row.values()))
                # Insert triggers update observation counters during hydration.
                for t in ('profiles','chain_index_clock'):
                    for row in source['states'][case['rows'][t]]:
                        fields=[k for k in row if k!='id']
                        c.execute('UPDATE "'+t+'" SET '+','.join('"'+k+'"=?' for k in fields)+' WHERE id=?',tuple(row[k] for k in fields)+(row['id'],))
                c.execute('DELETE FROM chain_index_dirty')
                for row in source['states'][case['rows']['chain_index_dirty']]:
                    c.execute('INSERT INTO chain_index_dirty(source_table,source_key,revision) VALUES(?,?,?)',tuple(row[k] for k in ('source_table','source_key','revision')))
                c.commit();c.execute('PRAGMA foreign_keys=ON')
                assert not c.execute('PRAGMA foreign_key_check').fetchall(),scenario['name']
                sql=[q for q in c.iterdump() if q.startswith('INSERT INTO')]
                order=['chain_index_clock','chain_index_dirty','workspaces','profiles','accounts','backends','wallets','transactions','custody_components','custody_component_legs','custody_authored_evidence_snapshots','custody_component_allocations','custody_component_economic_terms','custody_component_evidence_commitments','custody_component_transaction_memberships']
                priority={name:i for i,name in enumerate(order)}
                sql.sort(key=lambda q:(priority.get(q.split('"')[1],len(order)),q if q.startswith('INSERT INTO "schema_migration_audits"') else ''))
                for t in ('chain_index_clock','chain_index_dirty'):
                    inserts=[q for q in sql if q.startswith('INSERT INTO "'+t+'"')]
                    sql += ['DELETE FROM '+t,*inserts]
                ids=[r[0] for r in c.execute("SELECT id FROM custody_components WHERE profile_id='p' ORDER BY id")]
                operations=[('transfers.components.list',kw) for kw in (
                    {},dict(limit=1),dict(effective_only=True,limit=1),dict(state='draft'),dict(state='active'),dict(state='superseded'),dict(component_type='swap'),dict(transaction='d-ext'),dict(transaction='j-ext'),dict(transaction='absent'),dict(limit=0),dict(limit=True),dict(limit=1002),dict(state='bad'),dict(state=' ACTIVE '),dict(component_type='bad'),dict(include_local_evidence=True),
                )]
                operations += [('transfers.components.show',dict(component_id=id,include_local_evidence=local)) for id in [*ids,'absent'] for local in (False,True)]
                operations += [('transfers.components.show',dict(component_id=id)) for id in ('\ud800','x\ud800\ud801',ids[0]+'\ud800' if ids else 'absent\ud800')]
                operations += [('transfers.components.list',dict(transaction='d\ud800-ext'))]
                if ids:operations.append(('transfers.components.show',dict(component_id=ids[0],profile='other')))
                before=dump(c);cases=[]
                for operation,options in operations:
                    profile=options.get('profile','p')
                    try:
                        if operation.endswith('show'):
                            result=components.get_component(c,options['component_id'],profile_id=profile,include_local_evidence=options.get('include_local_evidence',False))
                        else:
                            tx=handlers.resolve_transaction(c,profile,options['transaction'])['id'] if 'transaction' in options else None
                            result=components.list_components(c,profile_id=profile,transaction_id=tx,include_local_evidence=options.get('include_local_evidence',False),**{k:v for k,v in options.items() if k not in ('profile','transaction','include_local_evidence')})
                        expected=dict(result=result)
                    except Exception as e:
                        expected=dict(error=dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None)))
                    assert dump(c)==before,(scenario['name'],i,operation,options)
                    cases.append(dict(operation=operation,args=options,profile=profile,**expected))
                scenarios.append(dict(name=f"{scenario['name']}-{i}",sql=sql,cases=cases));c.close()
    path=ROOT/'crates/transfers/tests/component_reads.json'
    template.close()
    output=json.dumps(dict(scenarios=scenarios),indent=2,ensure_ascii=True)+'\n'
    if args.check:assert path.read_text()==output,'component_reads.json differs from Python'
    else:path.write_text(output)
    print(dict(books=len(scenarios),operations=sum(len(s['cases']) for s in scenarios)))

if __name__=='__main__':main()
