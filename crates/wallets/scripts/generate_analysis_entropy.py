from __future__ import annotations

import copy
from contextlib import ExitStack
import json
import math
import random
from pathlib import Path
import sys
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(ROOT))
from kassiber.core.chain_analysis.entropy import analyze_transaction_entropy

DEST = ROOT / 'crates/wallets/tests/data/analysis_entropy.json'


def evaluate(case, snapshots):
    fact = case.get('facts', snapshots[case['snapshot']]['transaction_facts'].get(case.get('transaction')))
    progress = []
    clock = iter(case.get('elapsed_ms', [0]))
    last = 0
    def now():
        nonlocal last
        last = next(clock, last)
        return last / 1000
    calls = 0
    def cancelled():
        nonlocal calls
        calls += 1
        return 'cancel_after' in case and calls >= case['cancel_after']
    with ExitStack() as stack:
        # Budget construction consumes one clock reading; subsequent values are
        # elapsed time at the same forced/64-work checkpoints used by Rust.
        stack.enter_context(patch('kassiber.core.chain_analysis.entropy.time.monotonic', side_effect=now))
        # Explicit initial zero keeps the fixture clock expressed as elapsed ms.
        clock = iter([0, *case.get('elapsed_ms', [0])])
        if 'max_memo_cells' in case:
            stack.enter_context(patch('kassiber.core.chain_analysis.entropy.MAX_MEMO_CELLS', case['max_memo_cells']))
        result = analyze_transaction_entropy(copy.deepcopy(fact), chain=case.get('chain', 'bitcoin'), max_states=case.get('max_states', 200000), max_duration_ms=case.get('max_duration_ms', 1000), scenario=case.get('scenario'), progress=progress.append, cancelled=cancelled)
    return {'result': result, 'progress': progress}


def vectors():
    captured = json.loads((DEST.parent / 'analysis_traversal.json').read_text())['snapshots']
    snapshots = {k: captured[k] for k in ['diamond', 'stale', 'conflicting', 'empty']}
    template = next(f for f in snapshots['diamond']['transaction_facts'].values() if f['inputs'] and f['outputs'])
    def facts(ins, outs, **kw):
        f = copy.deepcopy(template)
        f.update({'complete':True, 'collaboration':None, 'fee_msat':None, **kw})
        f['inputs'] = [dict(template['inputs'][0], output_id=f'captured-input-{i}', amount_msat=str(v*1000)) for i, v in enumerate(ins)]
        f['outputs'] = [dict(template['outputs'][0], output_id=f'captured-output-{i}', amount_msat=str(v*1000)) for i, v in enumerate(outs)]
        return f
    cases = []
    def add(name, **kw):
        c = {'name':name, 'snapshot':'diamond', **kw}
        c['expected'] = evaluate(c, snapshots)
        cases.append(c)
    for name, snapshot in snapshots.items():
        for tx in snapshot['transaction_facts']:
            add(f'captured-{name}-{tx}', **{'snapshot':name, 'transaction':tx})
    # Small exact counts, fees, unequal classes, and exchangeable labeled links.
    for ins, outs in [([1],[1]),([5],[1,1,2]),([1,2,3],[6]),([1,1],[1,1]),([10,10],[9,9]),([7,5],[6,6]),([1,19],[9,9]),([3,2,1],[1,2,3]),([4,3,2],[2,3,3]),([10,10,10],[9,9,9]),([4,4,2,2],[3,3,3,3]),([2,2,2,2],[1,1,1,1])]:
        for received, paid in [(0,0),(1,0),(0,1),(2,2)]:
            scenario = {'kind':'coinjoin_intrafees','protocol':'joinmarket','max_received_fee_msat':str(received*1000),'max_paid_fee_msat':str(paid*1000)}
            add(f'flow-{len(cases)}',facts=facts(ins,outs),scenario=scenario)
        add(f'independent-{len(cases)}',facts=facts(ins,outs))
        add(f'permutation-{len(cases)}',facts=facts(ins[::-1],outs[::-1]))
    for n in [5,8,12,20,32]:
        add(f'big-count-{n}',facts=facts([101]*n,[100]*n),max_states=2000000,max_duration_ms=30000)
    for ins,outs in [([10000],[1]*128),([1]*128,[128]),(list(range(1,18)),list(range(1,18))),([1]*129,[1]),([1],[1]*129)]:
        add(f'shape-{len(cases)}',facts=facts(ins,outs))
    for budget in [0,1,2,5,20,64,100,400,1024,4000,'01',True,None,'999999999999999999999999999999']:
        add(f'state-{len(cases)}',facts=facts([10]*4,[9]*4),max_states=budget,max_duration_ms=30000)
    for cap in [0,10,40,100,300]:
        add(f'memory-{cap}',facts=facts([1,2,3],[1,2,3]),max_memo_cells=cap)
    for checkpoint in [1,2,3,4,5,8]:
        add(f'cancel-{checkpoint}',facts=facts([10]*8,[9]*8),cancel_after=checkpoint)
        add(f'timeout-{checkpoint}',facts=facts([10]*8,[9]*8),elapsed_ms=[0]*(checkpoint-1)+[2000])
    for duration in [0,1,True,'01',None,'999999999999999999999999999999']:
        add(f'duration-{len(cases)}',facts=facts([1],[1]),max_duration_ms=duration)
    for scenario in [False,[],{},'bad',{'kind':'unknown'},{'kind':'independent','extra':1},{'kind':'independent','protocol':None},{'kind':'independent','max_paid_fee_msat':0},*({'kind':'coinjoin_intrafees','max_paid_fee_msat':v,'max_received_fee_msat':0} for v in [None,True,-1,1,'01','1000',2100000000000000001000])]:
        add(f'scenario-{len(cases)}',facts=facts([2],[1]),scenario=scenario)
    for chain in ['liquid','lightning','', 'bitcoin']:
        add(f'chain-{chain}',facts=facts([2],[1]),chain=chain)
    for collaboration in [None,{},False,[],{'kind':'payjoin'},{'kind':'unknown'},'coinjoin',{'kind':'coinjoin'}]:
        add(f'collaboration-{len(cases)}',facts=facts([2],[1],collaboration=collaboration))
    for key,value in [('complete',False),('complete',1),('inputs',None),('outputs',{}),('inputs',[]),('outputs',[None]),('fee_msat','0'),('fee_msat',True),('fee_msat','1000')]:
        f=facts([2],[1]);f[key]=value
        add(f'invalid-{len(cases)}',facts=f)
    for value in [None,True,-1,0,1,1.0,'01','+1',' 1000','1000.0','1e3','1000','999999999999999999999999999999999999999999999999999999999999999999999999000']:
        f=facts([2],[1]);f['inputs'][0]['amount_msat']=value
        add(f'amount-{len(cases)}',facts=f)
    for value in [None,True,1,'']:
        f=facts([2],[1]);f['inputs'][0]['output_id']=value
        add(f'identity-{len(cases)}',facts=f)
    f=facts([2,2],[1,1]);f['inputs'][1]['output_id']=f['inputs'][0]['output_id'];add('duplicate-input',facts=f)
    f=facts([2],[1]);f['outputs'][0]['output_id']=f['inputs'][0]['output_id'];add('intersecting-identities',facts=f)
    add('negative-fee',facts=facts([1],[2]))
    add('cancel-before-timeout', facts=facts([10]*4,[9]*4), cancel_after=1, elapsed_ms=[1000])
    add('timeout-exact-deadline', facts=facts([10]*4,[9]*4), elapsed_ms=[1000])
    add('large-bounded', facts=facts([101]*100,[100]*100), max_states=10000)
    logarithms = [{'count':str(n),'expected':math.log2(n)} for n in [1,2,3,2**53-1,2**53+1,2**1023-1,2**1024-2**971,2**1024+1,2**1100+2**1047,2**1100+3*2**1047,10**1000]]
    randomizer = random.Random(21)
    for bits in [53,54,1023,1024,1025,1100]:
        for _ in range(4):
            n = 2**(bits-1) + randomizer.getrandbits(bits-1)
            logarithms.append({'count':str(n),'expected':math.log2(n)})
    return {'snapshots':snapshots,'cases':cases,'logarithms':logarithms}


def main():
    result=vectors()
    if '--write' in sys.argv:
        DEST.write_text('{\n"snapshots":{\n' + ',\n'.join(json.dumps(k) + ':' + json.dumps(v,ensure_ascii=True,separators=(',',':')) for k,v in result['snapshots'].items()) + '\n},\n"cases":[\n' + ',\n'.join(json.dumps(c,ensure_ascii=True,separators=(',',':')) for c in result['cases']) + '\n],\n"logarithms":' + json.dumps(result['logarithms'],separators=(',',':')) + '\n}\n')
    elif json.loads(DEST.read_text()) != result:
        raise SystemExit('entropy vectors drifted')
    print(f'{len(result["cases"])} entropy vectors')

if __name__ == '__main__':
    main()
