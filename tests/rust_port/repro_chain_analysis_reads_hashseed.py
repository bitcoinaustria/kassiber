"""Record the unchanged Python investigation envelope's hash-seed variants."""
from __future__ import annotations

import hashlib
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[2]
SPEC=importlib.util.spec_from_file_location('read_vectors',ROOT/'crates/wallets/scripts/generate_chain_analysis_reads.py')
REFERENCE=importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REFERENCE)


def reproduce():
    r=REFERENCE
    sql=''
    for number in range(1,5):
        txid=f'{number:064x}'
        sql+=r.assertion(f'{r.BLOCK}:{number}',txid=txid)
        sql+=r.assertion(f'{"d"*64}:{number}',txid=txid)
        sql+=r.insert('transactions',{'id':f'ambiguous-{number}','workspace_id':'ws','profile_id':'p','wallet_id':'cold','fingerprint':f'ambiguous-{number}','external_id':txid,'asset':'BTC','direction':'inbound','amount':1000,'fee':0,'occurred_at':'fixed','created_at':'fixed','raw_json':json.dumps({'txid':txid,'chain':'bitcoin','network':'main'})})
    from kassiber.core.chain_analysis.index import build_index
    conn=r.database(sql)
    try: full_build=[list(f['node_ids']) for f in build_index(conn,'p').findings if f['code']=='ambiguous_transaction_occurrence']
    finally: conn.close()
    state=r.evaluate({'sql':sql,'actions':[{'operation':'query'}]})[0]
    result=state['result']['data']
    return {'full_build_finding_node_ids':full_build,'snapshot_id':result['snapshot_id'],
            'finding_node_ids':[f['node_ids'] for f in result['findings'] if f['code']=='ambiguous_transaction_occurrence'],
            'wire_sha256':hashlib.sha256(state['wire'].encode()).hexdigest(),
            'tables':state['state']['tables']}


def variants():
    values={}
    for seed in range(8):
        env={**os.environ,'PYTHONHASHSEED':str(seed)}
        p=subprocess.run([sys.executable,str(Path(__file__).resolve())],env=env,capture_output=True,check=True,timeout=30)
        values[str(seed)]=json.loads(p.stdout)
    return values


if __name__=='__main__':
    print(json.dumps(variants() if '--seeds' in sys.argv else reproduce(),sort_keys=True,indent=2))
