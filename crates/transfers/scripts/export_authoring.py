#!/usr/bin/env python3
"""Pin full stored rows after deterministic rule/dismissal authoring sequences."""
import datetime
import json
from pathlib import Path
import sys
import tempfile
from unittest.mock import patch

ROOT=Path(__file__).resolve().parents[3]
sys.path.insert(0,str(ROOT))
from kassiber.cli import handlers
from kassiber.core import custody_review_terms as terms
from kassiber.db import open_db
from kassiber.errors import AppError

TABLES=('swap_matching_rules','transaction_pair_dismissals','profiles')
STAMP='2025-06-01T00:00:00Z'
REAL_DATETIME=datetime.datetime
class FixedDatetime(REAL_DATETIME):
    @classmethod
    def now(cls,tz=None):
        return cls.current

def main():
    operations=[
        ('transfers.rules.create',dict(name='€ 🚀',predicate={'out_asset':'BTC','in_asset':'LBTC','max_fee_pct':'0.000_001','min_confidence':'exact'})),
        ('transfers.rules.create',dict(predicate={})),
        ('transfers.rules.create',dict(predicate={'out_wallet_id':' \u2000','max_fee_pct':0},enabled=False,policy='taxable',kind='peg-in')),
        ('transfers.rules.enable',dict(rule_id='invalid')),
        ('transfers.rules.disable',dict(rule_id='invalid')),
        ('transfers.rules.disable',dict(rule_id='id-0')),
        ('transfers.rules.enable',dict(rule_id='id-0')),
        ('transfers.rules.delete',dict(rule_id='id-1')),
        ('transfers.rules.delete',dict(rule_id='id-1')),
        ('transfers.rules.create',dict(kind='invalid')),
        ('transfers.rules.create',dict(policy='invalid')),
        ('transfers.rules.create',dict(predicate=[])),
        ('transfers.rules.create',dict(predicate={'aa':1,'zz':2})),
        *[('transfers.rules.create',dict(predicate={field:1})) for field in ('out_asset','in_asset','out_wallet_id','in_wallet_id','out_wallet_kind','in_wallet_kind')],
        *[('transfers.rules.create',dict(predicate={'max_fee_pct':v})) for v in (True,[],{},-1,'NaN','Infinity',10**1000,'bad')],
        *[('transfers.rules.create',dict(predicate={'min_confidence':v})) for v in (False,'weak')],
        ('transfers.dismiss',dict(tx_out='tx0',tx_in='tx1',reason='keep €')),
        ('transfers.dismiss',dict(tx_out='external0',tx_in='external1',reason=None,expires_in_days=0)),
        ('transfers.dismiss',dict(tx_out='tx0',tx_in='tx1',reason='',expires_in_days=-1)),
        ('transfers.dismiss',dict(tx_out='tx1',tx_in='tx0',expires_in_days=1)),
        ('transfers.dismiss',dict(tx_out='absent',tx_in='tx1')),
        *[('transfers.dismiss',dict(tx_out='tx0',tx_in='tx1',expires_in_days=v)) for v in (999999999,1000000000,10000000000,10**30,'bad',float('nan'),float('inf'))],
    ]
    with tempfile.TemporaryDirectory(prefix='kassiber-transfer-write-oracle-') as root:
        c=open_db(root)
        for statement in json.loads((ROOT/'crates/transfers/tests/python.json').read_text())['sql']:
            c.execute(statement)
        c.commit()
        output=[]
        for i,(operation,args) in enumerate(operations):
            current=REAL_DATETIME(2025,6,1,tzinfo=datetime.timezone.utc)+datetime.timedelta(hours=i)
            FixedDatetime.current=current
            stamp=current.isoformat().replace('+00:00','Z')
            with patch.object(handlers.uuid,'uuid4',return_value=f'id-{i}'),patch.object(handlers,'now_iso',return_value=stamp),patch.object(terms,'now_iso',return_value=stamp),patch.object(datetime,'datetime',FixedDatetime):
                try:
                    if operation=='transfers.rules.create':
                        result=handlers.create_transfer_rule(c,'w','p',**args)
                    elif operation=='transfers.rules.delete':
                        result=handlers.delete_transfer_rule(c,'w','p',args['rule_id'])
                    elif operation in ('transfers.rules.enable','transfers.rules.disable'):
                        result=handlers.set_transfer_rule_enabled(c,'w','p',args['rule_id'],operation.endswith('.enable'))
                    else:
                        payload={k:v for k,v in args.items() if k not in ('tx_out','tx_in')}
                        result=handlers.dismiss_transfer_candidate(c,'w','p',args['tx_out'],args['tx_in'],**payload)
                    expected={'result':result}
                except AppError as e:
                    expected={'error':{'code':e.code,'message':str(e),'hint':e.hint,'details':e.details}}
                except Exception as e:
                    expected={'error':{'code':'internal_error','message':str(e),'hint':None,'details':None}}
            output.append({'operation':operation,'args':args,'id':f'id-{i}','now':stamp,**expected,'rows':{t:[dict(r) for r in c.execute(f'SELECT * FROM {t} ORDER BY id')] for t in TABLES}})
        (ROOT/'crates/transfers/tests/authoring.json').write_text(json.dumps(output,indent=2,ensure_ascii=True)+'\n')
        c.close()
if __name__=='__main__':main()
