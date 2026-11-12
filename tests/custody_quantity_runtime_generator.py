"""Exact-width runtime comparisons with nonvacuous per-scenario activity."""
from collections import Counter
from functools import lru_cache
from copy import deepcopy
import json
from pathlib import Path
from tests.custody_quantity_runtime_vectors import evaluate, PATH

COUNTS=Path(__file__).parent/'fixtures/custody_quantity_runtime_counts.json'

def activity(v):
    if 'expected' not in v: return 0
    x=v['expected']
    if isinstance(x,list): return len(x[0])+len(x[1])+len(x[3])
    if x['$type']=='QuantityTaxEligibility': return len(x['eligible_decisions'])+len(x['pool_barriers'])
    return len(x['projection']['decisions'])+len(x['issues'])

@lru_cache(maxsize=1)
def scenarios(): return json.loads(PATH.read_text())

_evaluated = {}


def evaluate_distinct(value):
    key = json.dumps(value, sort_keys=True, separators=(',', ':'))
    if key not in _evaluated:
        _evaluated[key] = evaluate(value)
    return deepcopy(_evaluated[key])


def scenario(i,variant):
    corpus=scenarios(); base=corpus[i]
    def operands(c): return {k:deepcopy(v) for k,v in c.items() if k not in ('expected','error','message','details','case')}
    v=operands(base)
    multiplier=(1,3,10**50+1,10**90+7)[variant%4]
    def scale(x):
        if isinstance(x,list): return [scale(t) for t in x]
        if not isinstance(x,dict): return x
        return {k:t*multiplier if type(t) is int and (k.endswith('_msat') or k in ('amount','fee')) else scale(t) for k,t in x.items()}
    v=scale(v)
    positive=operands(next(c for c in corpus if c['op']==v['op'] and activity(c)>0))
    results=[evaluate_distinct(v),evaluate_distinct(positive)]
    return base['op']+':'+base['case'],[v,positive],results,sum(map(activity,results))



def variants(index):
    """Yield only distinct primary operands; repetitions are not evidence."""
    seen = set()
    for variant in range(8):
        _, operands, _, _ = scenario(index, variant)
        key = json.dumps(operands[0], sort_keys=True, separators=(',', ':'))
        if key not in seen:
            seen.add(key)
            yield variant


if __name__=='__main__':
    counts=Counter()
    for i in range(len(scenarios())):
        for variant in variants(i):
            name,_,_,active=scenario(i,variant); assert active>0; counts[name]+=active
    COUNTS.write_text(json.dumps(counts,indent=2,sort_keys=True)+'\n')
