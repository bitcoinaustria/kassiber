"""Freeze structural lineage against the unchanged ownership interpreter."""
import argparse
from copy import deepcopy
import json
from pathlib import Path

from kassiber.core import source_funds as sof
from kassiber.core.source_funds_assembly import derive_parent_spend_pairs
from tests.test_source_funds_assembly import ChangePassthroughTests, PassthroughWeightTests, TraversalBoundsTests, FanInBoundsTests, OrderAndScaleTests, _authoritative

DESTINATION=Path(__file__).resolve().parents[1]/'tests/data/source_funds_lineage.json'


def cases():
    result=[]
    def add(name,rows,owned=None):
        owned=owned or []
        index={}
        for row in owned:
            key=(row['chain'],row['network'],row['txid'],int(row['vout']))
            if key in index:
                index[key]['ambiguous']=True
            else:
                index[key]=dict(row,asset_identity=row['asset'])
        derived=derive_parent_spend_pairs(rows,index,skip_row=lambda r: sof._raw_privacy_hop(r) is not None or sof._samourai_metadata_from_wallet_config(r.get('wallet_config_json')) is not None)
        expected=sorted([[p['from_row']['id'],p['to_row']['id'],p['allocation_msat'],p['from_allocation_msat']] for p in derived])
        result.append(dict(name=name,rows=rows,owned=owned,expected=expected))
    for factory in [ChangePassthroughTests,PassthroughWeightTests]:
        fixture=factory();fixture.setUp()
        for reverse in [False,True]:
            rows=deepcopy(fixture.rows)
            add(factory.__name__+str(reverse),list(reversed(rows)) if reverse else rows)
        for change in ['privacy','authority','amount','missing','script','inventory','duplicate-inventory','cycle']:
            rows=deepcopy(fixture.rows);owned=[]
            if change=='privacy': rows[0]['privacy_boundary']='coinjoin'
            elif change=='authority': rows[0]['observation_graph_hash']='0'*64
            elif change=='amount': rows[0]=_authoritative(dict(rows[0],amount=1))
            elif change=='missing': rows=rows[1:]
            elif change=='script':
                raw=json.loads(rows[-1]['raw_json']);raw['observer_owned_scripts']=[];rows[-1]=_authoritative(dict(rows[-1],raw_json=json.dumps(raw)))
            elif change=='cycle':
                raw=json.loads(rows[-2]['raw_json']);raw['vin'][0]['txid']=rows[-1]['external_id'];rows[-2]=_authoritative(dict(rows[-2],raw_json=json.dumps(raw)))
            else:
                raw=json.loads(rows[-1]['raw_json']);txid=raw['vin'][0]['txid'];vout=raw['vin'][0]['vout']
                owned=[dict(chain='bitcoin',network='main',txid=txid,vout=vout,asset='BTC',wallet_id='other' if change=='inventory' else 'w')]
                if change=='duplicate-inventory':owned*=2
            add(factory.__name__+change,rows,owned)
    fixture=ChangePassthroughTests();fixture.setUp()
    other=[_authoritative(dict(row,wallet_id="other",id="other-"+row["id"])) for row in fixture.rows]
    add("wallet-memo-isolation",fixture.rows+other)
    for length in [1,3,12]:
        rows=TraversalBoundsTests()._chain(length)
        add('chain-'+str(length),rows);add('chain-reverse-'+str(length),list(reversed(rows)))
    for width in [2,10]:
        rows,_=FanInBoundsTests()._book(hops=4,width=width)
        add('fanin-'+str(width),rows);add('fanin-reverse-'+str(width),list(reversed(rows)))
    rows=OrderAndScaleTests()._long_chain(1101)
    add('long-chain',list(reversed(rows)))
    return result


def main():
    parser=argparse.ArgumentParser();parser.add_argument('--write',action='store_true');args=parser.parse_args()
    data=cases();generated=json.dumps(data,ensure_ascii=True,separators=(',',':'))+'\n'
    if args.write:DESTINATION.write_text(generated)
    else:assert DESTINATION.read_text()==generated
    print(f'{len(data)} structural lineage cases verified')


if __name__=='__main__':main()
