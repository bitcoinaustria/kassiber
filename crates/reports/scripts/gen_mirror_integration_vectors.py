from pathlib import Path
import json
import sys
import uuid
from unittest.mock import patch
ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
sys.path.insert(0,str(ROOT/'crates/wallets/scripts'))
from generate_analysis_durable import database,capture
from generate_analysis_sources import stored,inventory,dataset
from kassiber.core.privacy_mirror import build_privacy_mirror
from kassiber.core.reports import privacy_mirror_table_rows

cases=[]
for caller in [False,True]:
    for name,sql,updates in [('empty','',[]),('stored',stored(),[inventory()]),('owned',stored()+inventory(),["UPDATE wallet_utxos SET amount=9000 WHERE id='utxo';"]),('claims',stored()+inventory()+dataset(),["UPDATE chain_analysis_datasets SET status='revoked';"])]:
        case={'name':f'{name}-{caller}','sql':sql,'caller':caller,'actions':[]}
        conn=database(sql)
        conn.isolation_level=None
        if caller:
            conn.execute('SAVEPOINT caller')
            conn.execute("INSERT INTO settings(key,value) VALUES('caller-pending','keep')")
        try:
            for change in [None,None,*updates]:
                if change:conn.execute(change)
                with patch('kassiber.core.chain_analysis.entropy.time.monotonic',return_value=0), patch('kassiber.core.chain_analysis.projection_store.uuid.uuid4',return_value=uuid.UUID(hex='0123456789abcdef0123456789abcdef')), patch('kassiber.core.chain_analysis.projection_store.now_iso',return_value='2026-01-01T00:00:00Z'):
                    payload=build_privacy_mirror(conn,'p',redacted=False)
                payload['scope']={'workspace':'active','profile':'active','workspace_selected':True,'profile_selected':True}
                case['actions'].append({'sql':change,'report':payload,'table':privacy_mirror_table_rows(payload),'tables':capture(conn)['tables']})
            cases.append(case)
        finally:conn.close()
(ROOT/'crates/reports/tests/data/mirror_integration.json').write_text(json.dumps(cases,indent=2)+'\n')
