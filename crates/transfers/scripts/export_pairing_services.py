#!/usr/bin/env python3
"""Seeded automatic-pairing DB oracle with per-seed distinct-input activity.

No output IDs or DB fields are masked. All UUIDs and clocks are injected before
both implementations execute. Every application table is included in each dump.
"""
from contextlib import ExitStack
from datetime import datetime, timezone
import hashlib
import json
import random
import sqlite3
from pathlib import Path
import sys
import tempfile
import uuid
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from kassiber.cli import handlers
from kassiber.core import custody_review_terms as terms, custody_components as components, custody_authored_migration as migration
from kassiber.daemon_freshness import _auto_pair_before_journals
from kassiber.db import JOURNAL_CALCULATION_SEMANTICS_VERSION, open_db
from kassiber.errors import AppError
from tests.test_transfer_matching import _row

STAMP='2026-10-30T12:00:00Z'


def dump(c):
    tables=[r[0] for r in c.execute("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")]
    return {t:sorted([dict(r) for r in c.execute('SELECT * FROM "'+t.replace('"','""')+'"')],key=lambda r:json.dumps(r,sort_keys=True)) for t in tables}


def sql(c,query,args=()):
    return c.execute(query,args)


def seed(c,rows):
    c.execute("INSERT INTO workspaces(id,label,created_at) VALUES('w','Main',?)",(STAMP,))
    c.execute("INSERT INTO profiles(id,workspace_id,label,tax_country,created_at) VALUES('p','w','Book','generic',?)",(STAMP,))
    wallets={r['wallet_id']:r for r in rows}
    for wid,r in wallets.items():
        config=r.get('config_json',r.get('wallet_config_json',r.get('config',{})))
        if not isinstance(config,str):config=json.dumps(config)
        c.execute("INSERT INTO wallets(id,workspace_id,profile_id,label,kind,config_json,created_at) VALUES(?,'w','p',?,?,?,?)",(wid,wid,r['wallet_kind'],config,STAMP))
    fields={r[1] for r in c.execute('PRAGMA table_info(transactions)')}
    for r in rows:
        vals={k:v for k,v in r.items() if k in fields and k not in ('profile_id',)}
        vals.update(workspace_id='w',profile_id='p',fingerprint='fp-'+r['id'],created_at=STAMP)
        vals['raw_json']=vals['raw_json'] if isinstance(vals['raw_json'],str) else json.dumps(vals['raw_json'])
        c.execute('INSERT INTO transactions('+','.join(vals)+') VALUES('+','.join('?' for _ in vals)+')',tuple(vals.values()))
    c.commit()


def provider_rows(rng,stem='a',fee=0,includes=0):
    amount=rng.randrange(100_000_000,900_000_000)
    inbound=amount-rng.randrange(1000,10000)-fee
    txid=f'{rng.getrandbits(256):064x}'; receive=f'{rng.getrandbits(256):064x}'
    raw=dict(provider='boltz',swap_id=stem,flow='chain',send_txid=txid,receive_txid=receive,send_amount_msat=amount,receive_amount_msat=inbound)
    return [_row(id=stem+'-o',wallet_id='source',wallet_label='Source',amount=amount,external_id=txid,fee=fee,amount_includes_fee=includes,raw_json=dict(raw,txid=txid)),
            _row(id=stem+'-i',wallet_id='target',wallet_label='Target',direction='inbound',amount=inbound,external_id=receive,raw_json=dict(raw,txid=receive))]


def generated(seed_id,scenario):
    rng=random.Random(seed_id)
    rows=provider_rows(rng,fee=seed_id*7,includes=seed_id%2)
    setup=[];pre=[];opts={}
    if scenario=='ambiguous':
        rows.append(dict(rows[1],id='duplicate',wallet_id='third',wallet_label='Third'))
    elif scenario=='tolerance':
        # One positive candidate at the exact floor plus a row one msat beyond.
        amount=rng.randrange(10_000_000,40_000_000)
        rows=[_row(id='o',wallet_id='source',amount=amount,fee=seed_id,amount_includes_fee=seed_id%2),_row(id='i',wallet_id='target',direction='inbound',amount=amount-2_500_000)]
        rows.append(_row(id='beyond',wallet_id='third',direction='inbound',amount=amount-2_500_001))
    elif scenario=='multi-wallet':
        rows+=provider_rows(rng,'b',fee=123,includes=0)
    elif scenario=='rules':
        rows[0]['raw_json']['send_amount_msat']+=1
        rows[1]['raw_json']['send_amount_msat']+=1
        setup += ["INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES('specific','w','p','Specific','{\"out_wallet_id\":\"source\",\"min_confidence\":\"strong\",\"max_fee_pct\":0.02}','chain-swap','carrying-value',1,'2025-01-01','2025-01-01')",
                  "INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES('a-broad','w','p','Broad','{\"min_confidence\":\"strong\"}','manual','carrying-value',1,'2025-01-01','2025-01-01')"]
    elif scenario=='dismissal':
        setup += ["INSERT INTO transaction_pair_dismissals(id,workspace_id,profile_id,out_transaction_id,in_transaction_id,created_at,expires_at) VALUES('veto','w','p','a-o','a-i','2025-01-01','2026-10-30T12:00:00Z')"]
    elif scenario=='unpaired':
        pre=[('pair',{}),('unpair',{})]
    elif scenario=='ownership':
        # Another independent exact graph card competes for the source.
        rows.append(_row(id='owned',wallet_id='third',direction='inbound',amount=rows[1]['amount'],external_id='other-import'))
        card=dict(out_id='a-o',in_id='owned',out_asset='BTC',in_asset='BTC',out_amount_msat=rows[0]['amount'],in_amount_msat=rows[1]['amount'],out_wallet_id='source',in_wallet_id='third',out_wallet_label='Source',in_wallet_label='Third',out_wallet_kind='descriptor',in_wallet_kind='descriptor',out_occurred_at=rows[0]['occurred_at'],in_occurred_at=rows[1]['occurred_at'],confidence='exact',method='ownership_graph',swap_fee_msat=0,swap_fee_kind='combined',default_kind='manual',default_policy='carrying-value')
        detail=json.dumps(dict(ownership_review_candidates=[card]))
        setup += ["UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=3,last_processed_input_version=journal_input_version WHERE id='p'",
                  "INSERT INTO journal_quarantines(workspace_id,profile_id,transaction_id,reason,detail_json,created_at) VALUES('w','p','a-o','Review',"+quote(detail)+",'2025-01-01')"]
    elif scenario in ('booked','guards-after-rule'):
        setup += ["UPDATE profiles SET last_processed_at='2025-01-01',last_processed_tx_count=2,last_processed_input_version=journal_input_version WHERE id='p'",
                  "INSERT INTO journal_custody_decisions(decision_id,workspace_id,profile_id,state,source_transaction_id,target_transaction_id,source_observation_hash,source_start_msat,source_end_msat,target_observation_hash,target_start_msat,target_end_msat,source_network,target_network,source_rail,target_rail,source_asset,target_asset,basis_state,reason,created_at) VALUES('0000000000000000000000000000000000000000000000000000000000000000','w','p','internal_verified','a-o','a-i','1111111111111111111111111111111111111111111111111111111111111111',0,1,'2222222222222222222222222222222222222222222222222222222222222222',0,1,'main','main','bitcoin','bitcoin','BTC','BTC','eligible','move','2025-01-01')"]
        if scenario=='guards-after-rule':
            rows+=provider_rows(rng,'b',fee=13,includes=0)
            setup=[q.replace('last_processed_tx_count=2','last_processed_tx_count=4').replace("'a-o','a-i'","'b-o','b-i'") for q in setup]
            setup.append("INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES('exact-rule','w','p','Exact','{}','chain-swap','carrying-value',1,'2025-01-01','2025-01-01')")
    elif scenario=='fee-variants':
        rows[0]['amount_includes_fee']=1
        rows[0]['fee']=max(1,seed_id*11)
        if seed_id == 1:
            # CPython int/int rounds once; two float casts reject this exact
            # max-fee boundary. Keep its complete write state in the oracle.
            rows[0]['amount']=1_511_910_649_648_900_384
            rows[1]['amount']=rows[0]['amount']-1_886_523_809_418_448
            for row in rows:
                row['raw_json'].update(send_amount_msat=rows[0]['amount'],receive_amount_msat=rows[1]['amount'])
        rows[1]['raw_json']['receive_amount_msat']-=1  # conflicting amounts demote exact
        predicate=json.dumps({'max_fee_pct':0.0012477746683354212} if seed_id == 1 else {})
        setup.append("INSERT INTO swap_matching_rules(id,workspace_id,profile_id,name,predicate_json,kind,policy,enabled,created_at,updated_at) VALUES('default-strong','w','p','Default strong',"+quote(predicate)+",'chain-swap','carrying-value',1,'2025-01-01','2025-01-01')")
    return rows,setup,pre,opts


def quote(v): return "'"+v.replace("'","''")+"'"


def require_activity(counters):
    for seed_id,scenarios in counters.items():
        for name,distinct in scenarios.items():
            if distinct<1:
                raise AssertionError(f'seed {seed_id}, scenario {name}: distinct-input activity {distinct} < 1')


def main():
    scenarios=['exact','ambiguous','tolerance','multi-wallet','rules','dismissal','unpaired','ownership','booked','guards-after-rule','fee-variants']
    states={};output=[];counters={}
    seeds=range(1,13)
    jobs=[]
    for seed_id in seeds:
        counters[seed_id]={}
        for scenario in scenarios:
            jobs.append((seed_id,scenario,*generated(seed_id,scenario)))
    matcher=json.loads((ROOT/'crates/transfers/tests/pairing_matcher.json').read_text())
    for index,case in enumerate(matcher):
        jobs.append((-1,f'fixture-{index}',case['rows'],[],[],{}))
    template=sqlite3.connect(':memory:')
    # Seed the initial SQLite clock before migrations write immutable audits.
    # Pairing outputs are never rewritten or normalized after execution.
    connect=sqlite3.connect
    clock=connect(':memory:')
    def frozen_connect(*args,**kwargs):
        connection=connect(*args,**kwargs)
        def strftime(*values):
            values=tuple(STAMP if v=='now' else v for v in values)
            return clock.execute('SELECT strftime('+','.join('?' for _ in values)+')',values).fetchone()[0]
        connection.create_function('strftime',-1,strftime)
        return connection
    with tempfile.TemporaryDirectory(prefix='pairing-template-') as root:
        # The pair fee review audit (ca630b9a6) is stamped by Python's clock.
        with patch('kassiber.db.sqlite3.connect',side_effect=frozen_connect), \
                patch.object(migration,'now_iso',return_value=STAMP):
            source=open_db(root)
        source.execute("UPDATE settings SET value='00000000000000000000000000000000' WHERE key='database_instance_id'")
        source.commit();source.backup(template);source.close()
    clock.close()
    for seed_id,scenario,rows,setup,pre,opts in jobs:
        c=sqlite3.connect(':memory:');template.backup(c);c.row_factory=sqlite3.Row;c.execute('PRAGMA foreign_keys=ON');seed(c,rows)
        if any("last_processed_at=" in q for q in setup):
            setup.append(f"UPDATE profiles SET journal_semantics_version={JOURNAL_CALCULATION_SEMANTICS_VERSION} WHERE id='p'")
        for q in setup:c.execute(q)
        c.commit()
        cases=[];sequence=0;pair=None
        class Frozen(datetime):
            @classmethod
            def now(cls,tz=None):return cls(2026,10,30,12,tzinfo=timezone.utc)
        def uuid4():
            nonlocal sequence
            # Savepoint names consume random UUIDs in Python but are
            # not stored. Inject only durable review identities.
            if sys._getframe(4).f_globals.get('__name__') != terms.__name__:
                return uuid.UUID(int=0xf000)
            sequence+=1
            return uuid.UUID(int=sequence)
        with ExitStack() as stack:
            stack.enter_context(patch.object(terms.uuid,'uuid4',side_effect=uuid4))
            for module,field in [(terms,'now_iso'),(migration,'now_iso'),(components,'_now_iso')]:stack.enter_context(patch.object(module,field,return_value=STAMP))
            stack.enter_context(patch.object(__import__('kassiber.core.transfer_matching',fromlist=['datetime']),'datetime',Frozen))
            for action,_ in pre:
                if action=='pair':pair=handlers.create_transaction_pair(c,'w','p','a-o','a-i',kind='chain-swap')
                else:handlers.delete_transaction_pair(c,'w','p',pair['id'])
            # Complete pre-state SQL includes generated historical review rows.
            sql_lines=[q for q in c.iterdump() if q.startswith('INSERT INTO')]
            # Restore authored dependencies before their guarded rows.
            order=['chain_index_clock','chain_index_dirty','workspaces','profiles','accounts','backends','wallets','transactions','custody_components','custody_component_legs','custody_authored_evidence_snapshots','custody_component_allocations','custody_component_economic_terms','custody_component_evidence_commitments','custody_component_transaction_memberships']
            priority={name:i for i,name in enumerate(order)}
            sql_lines.sort(key=lambda q:(priority.get(q.split('"')[1],len(order)),q if q.startswith('INSERT INTO "schema_migration_audits"') else ''))
            for table in ('chain_index_clock','chain_index_dirty'):
                inserts=[q for q in sql_lines if q.startswith('INSERT INTO "'+table+'"')]
                sql_lines += ['DELETE FROM '+table, *inserts]
            initial_sequence=sequence
            operations=[(action,{}) for action in ['suggest','auto','auto','suggest','rules','bulk','suggest']]
            if scenario=='exact':
                operations += [('suggest',{'candidate_type':'bad'}),('bulk',{'confidence':'bad'}),('suggest',{'asset_pair':'BTC'}),('suggest',{'route_pair':'BTC'}),('rules',{'candidate_type':'bad'})]
            if scenario=='tolerance' or seed_id < 0:
                operations += [('bulk',{'confidence':'strong'}),('suggest',{})]
            for action,opts in operations:
                before=dump(c)
                try:
                    if action=='suggest':result=handlers.suggest_transfer_candidates(c,'w','p',**opts)
                    elif action=='auto':result=_auto_pair_before_journals(c,dict(workspace_id='w',profile_id='p'))
                    elif action=='rules':result=handlers.apply_transfer_rules(c,'w','p',commit=False,**opts)
                    else:result=handlers.bulk_pair_transfers(c,'w','p',commit=False,**opts)
                    expected={'result':result}
                    c.commit()
                except Exception as e:
                    c.rollback()
                    expected={'error':dict(code=e.code if isinstance(e,AppError) else 'internal_error',message=str(e),hint=getattr(e,'hint',None),details=getattr(e,'details',None))}
                current=dump(c);encoded=json.dumps(current,sort_keys=True);key=hashlib.sha256(encoded.encode()).hexdigest();states[key]=current
                cases.append(dict(operation=action,options=opts,**expected,state=key))
            if seed_id > 0:
                # Count distinct inputs only when the intended branch is
                # observed, never because a book merely has rows.
                first=cases[0]['result']['counts'];auto=cases[1]['result']
                expected={
                    'exact':(1,1,0,0,1), 'ambiguous':(2,0,2,1,0),
                    'tolerance':(1,0,1,0,0), 'multi-wallet':(2,2,0,0,2),
                    'rules':(1,0,1,0,1), 'dismissal':(1,1,0,0,1),
                    'unpaired':(1,1,0,0,0), 'ownership':(2,2,0,1,0),
                    'booked':(0,0,0,0,0), 'guards-after-rule':(1,1,0,0,1), 'fee-variants':(1,0,1,0,1),
                }[scenario]
                observed=tuple(first[k] for k in ('total','exact','strong','conflicts'))+(auto['applied'],)
                active=observed==expected
                if scenario=='booked':
                    # A no-candidate result alone is vacuous: prove the same
                    # inputs have an exact edge without the stored MOVE veto.
                    from kassiber.core.transfer_matching import suggest_swap_candidates
                    active &= len(suggest_swap_candidates(rows,now_iso=STAMP))==1
                input_hash=hashlib.sha256(json.dumps(rows,sort_keys=True).encode()).hexdigest()
                counters[seed_id][scenario]=len({input_hash} if active else set())
            output.append(dict(name=f'{seed_id}-{scenario}',sql=sql_lines,uuid_start=initial_sequence+1,cases=cases))
            c.close()
    template.close()
    require_activity(counters)
    try:require_activity({1:{'deliberately-vacuous':0}})
    except AssertionError as exc:vacuous=str(exc)
    else:raise AssertionError('vacuous activity check passed')
    dest=Path(sys.argv[1]) if len(sys.argv)>1 else ROOT/'crates/transfers/tests/pairing_services.json'
    dest.write_text(json.dumps(dict(states=states,scenarios=output,activity=counters,vacuous_failure=vacuous),indent=2)+'\n')
    print(json.dumps(dict(books=len(output),operations=sum(len(s['cases']) for s in output),activity=counters,vacuous_failure=vacuous)))


if __name__=='__main__':main()
