#!/usr/bin/env python3
"""Capture sampled generator books for complete native persistence replay.

The raw snapshot is materialized in a disposable database; Python's completed
custody stages and ledger response remain the R4-12 input seam. Refusals are
reported separately and are never counted as successful persistence parity.
"""
import argparse
from collections import Counter
from copy import deepcopy
import importlib
import importlib.util
import json
from pathlib import Path
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[1]
sys.path.insert(0,str(ROOT))
from kassiber.core.custody_compute import compute_custody_projection
from kassiber.db import open_db
from tests.rust_port.journal_persistence_oracle import capture

spec=importlib.util.spec_from_file_location('render_samples',Path(__file__).with_name('capture-journal-render-samples.py'))
families=importlib.util.module_from_spec(spec)
spec.loader.exec_module(families)


def seed(conn,book):
    conn.execute('PRAGMA foreign_keys=OFF')
    def insert(table,row):
        columns={r[1] for r in conn.execute(f'PRAGMA table_info({table})')}
        row={k:v for k,v in row.items() if k in columns}
        conn.execute(f'INSERT INTO {table}({",".join(row)}) VALUES({",".join("?" for _ in row)})',tuple(row.values()))
    profile=dict(book['profile'])
    insert('workspaces',dict(id=profile['workspace_id'],label='Synthetic',created_at='2020'))
    insert('profiles',profile)
    accounts=set()
    for row in book['wallets']:
        account=row.get('wallet_account_id')
        if account and account not in accounts:
            insert('accounts',dict(id=account,workspace_id=profile['workspace_id'],profile_id=profile['id'],
                code=row.get('account_code') or account,label=row.get('account_label') or account,
                account_type='asset',created_at='2020'))
            accounts.add(account)
        insert('wallets',dict(workspace_id=profile['workspace_id'],profile_id=profile['id'],created_at='2020',account_id=account,**row))
    for row in book['transactions']: insert('transactions',row)
    for component in book['components']: insert('custody_components',component['header'])
    conn.commit()
    conn.execute('PRAGMA foreign_keys=ON')
    return conn.execute('SELECT * FROM profiles WHERE id=?',(profile['id'],)).fetchone()


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('directory',type=Path)
    parser.add_argument('--payouts',action='store_true',help='capture the separate real-writer payout corpus')
    args=parser.parse_args()
    args.directory=args.directory.resolve()
    if not args.directory.resolve().is_relative_to((ROOT/'target').resolve()):
        parser.error('capture directory must be under worktree target/')
    args.directory.mkdir(parents=True,exist_ok=True)
    scratch_root=ROOT/'target/r4-persist/tmp'
    scratch_root.mkdir(parents=True,exist_ok=True)
    summary={}
    selected=families.FAMILIES if not args.payouts else []
    for family,function,scenarios in selected:
        module=importlib.import_module(f'tests.custody_{family}_generator')
        name=f'{family}-{function}'
        root=args.directory/name
        root.mkdir(exist_ok=True)
        counts=Counter();errors=[]
        for number,scenario in enumerate(getattr(module,scenarios)):
            try:
                book=deepcopy(getattr(module,function)(number))
                # Raw-stage generators clone transactions without importing
                # them. Give those clones distinct import fingerprints before
                # either runtime computes stages or writes the same book.
                seen=set()
                for transaction in book['transactions']:
                    fingerprint=transaction['fingerprint']
                    if fingerprint in seen:
                        transaction['fingerprint']=f'{fingerprint}:materialized:{transaction["id"]}'
                        counts['materialized_fingerprints']+=1
                    seen.add(transaction['fingerprint'])
                custody=compute_custody_projection(book)
                with tempfile.TemporaryDirectory(prefix='persistence-sample-',dir=scratch_root) as scratch:
                    conn=open_db(Path(scratch)/'book')
                    try:
                        profile=seed(conn,book)
                        record=capture(conn,f'{name}:{scenario}',profile,custody)
                    finally: conn.close()
                (root/f'{number:03}.json').write_text(json.dumps(record,ensure_ascii=True,separators=(',',':'))+'\n')
                counts['books']+=1
                for table,data in record['expected'].items():
                    if data['rows'] and table.startswith('journal_'): counts[table]+=len(data['rows'])
            except Exception as error:
                errors.append(dict(scenario=scenario,type=type(error).__name__,error=str(error)))
                if len(errors)<=2: print(name,scenario,type(error).__name__,str(error),flush=True)
        summary[name]=dict(counts=counts,refusals=errors)
        print(name,dict(counts),f'refusals={len(errors)}',flush=True)
    if args.payouts:
        from tests.custody_projection_payout_books import books
        root=args.directory/'payouts';root.mkdir(exist_ok=True)
        counts=Counter();errors=[]
        with tempfile.TemporaryDirectory(prefix='persistence-payouts-',dir=scratch_root) as scratch:
            for number,(name,book) in enumerate(books(Path(scratch)/'source')):
                try:
                    custody=compute_custody_projection(book)
                    conn=open_db(Path(scratch)/f'copy-{number}')
                    try:
                        profile=seed(conn,book)
                        record=capture(conn,f'payout:{name}',profile,custody)
                    finally: conn.close()
                    (root/f'{number:03}.json').write_text(json.dumps(record,ensure_ascii=True,separators=(',',':'))+'\n')
                    counts['books']+=1
                    counts['economic_relations']+=len(record['expected']['journal_custody_economic_relations']['rows'])
                except Exception as error:
                    errors.append(dict(scenario=name,type=type(error).__name__,error=str(error)))
        summary['payouts']=dict(counts=counts,refusals=errors)
        print('payouts',dict(counts),f'refusals={len(errors)}',flush=True)
    (args.directory/'activity.json').write_text(json.dumps(summary,indent=2)+'\n')


if __name__=='__main__': main()
