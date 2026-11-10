"""Private conversion operands; oracle executes the unchanged Python audit AST."""
import ast
from copy import deepcopy
import inspect
from pathlib import Path
from types import SimpleNamespace

from kassiber.core import custody_tax_projection as p

PATH = Path(__file__).parent / 'fixtures/custody_projection_conversions.json'


def evaluate(v):
    root = ast.parse(inspect.getsource(p.compile_finalized_tax_projection)).body[0]
    start = next(i for i, n in enumerate(root.body) if isinstance(n, ast.Assign)
                 and any(isinstance(t, ast.Name) and t.id == 'existing_cross_pair_ids' for t in n.targets))
    env = dict(vars(p), cross_asset_pairs=deepcopy(v['pairs']), projection_rows=v['rows'],
               state=SimpleNamespace(reviewed_conversion_pairs=v['reviewed']))
    try:
        exec(compile(ast.fix_missing_locations(ast.Module(body=root.body[start:-1], type_ignores=[])),
                     '<Python reviewed conversion reference>', 'exec'), env)
        return {'expected': env['cross_asset_pairs']}
    except (TypeError, ValueError, KeyError, OverflowError) as exc:
        return {'error': type(exc).__name__}


def operands():
    base = dict(op='conversions', rows=[
        dict(id='out-row', journal_transaction_id='out', direction='outbound', amount=2**150, asset='BTC'),
        dict(id='in-row', journal_transaction_id='in', direction='inbound', amount=2**151, asset='LBTC')],
        pairs=[], reviewed=[dict(pair_id='review', out_id='out', in_id='in')])
    def case(name, edit):
        v = deepcopy(base); v['case'] = name; edit(v); return v
    yield case('basic', lambda v: None)
    yield case('exact', lambda v: v['reviewed'][0].update(out_amount=str(2**150), in_amount=str(2**151)))
    yield case('overrides', lambda v: v['reviewed'][0].update(component_id='component',out_asset='X',in_asset='Y',policy='carrying-value',kind='peg'))
    yield case('falsey', lambda v: v['reviewed'][0].update(component_id=False,out_asset=[],in_asset=0,policy=False,kind=''))
    yield case('duplicate', lambda v: v['reviewed'].append(dict(pair_id='review',out_id='out',in_id='in',out_amount='bad')))
    yield case('existing', lambda v: v['pairs'].append(dict(pair_id='review',sentinel=True)))
    yield case('wrong_amount', lambda v: v['reviewed'][0].update(out_amount=1))
    yield case('wrong_amount_high', lambda v: v['reviewed'][0].update(out_amount=2**151))
    yield case('wrong_direction', lambda v: v['rows'][0].update(direction='inbound'))
    yield case('ambiguous', lambda v: v['rows'].append(deepcopy(v['rows'][0])))
    yield case('ambiguous_in', lambda v: v['rows'].append(deepcopy(v['rows'][1])))
    yield case('missing_leg', lambda v: v['rows'].pop())
    yield case('empty_pair', lambda v: v['reviewed'][0].update(pair_id=''))
    yield case('lazy_invalid_amount', lambda v: (v['rows'].clear(),v['reviewed'][0].update(out_amount='bad')))
    yield case('bad_review', lambda v: v['reviewed'][0].update(out_amount='bad'))
    yield case('bad_row', lambda v: (v['rows'][0].update(amount={"bad":1}),v['reviewed'][0].update(out_amount=1)))
    yield case('amount_before_direction', lambda v: (v['rows'][0].update(amount='bad',direction='inbound'),v['reviewed'][0].update(out_amount=1)))
    yield case('out_before_in', lambda v: v['reviewed'][0].update(out_amount={},in_amount='bad'))
    yield case('missing_row_id', lambda v: v['rows'][0].pop('id'))
    yield case('empty_amount', lambda v: v['reviewed'][0].update(out_amount='',in_amount=None))
    yield case('bool_amount', lambda v: (v['rows'][0].update(amount=True),v['reviewed'][0].update(out_amount=True)))
    yield case('float_amount', lambda v: (v['rows'][0].update(amount=1.9),v['reviewed'][0].update(out_amount=1.1)))
    yield case('unicode_amount', lambda v: (v['rows'][0].update(amount=' ١_٢ '),v['reviewed'][0].update(out_amount=12)))
    yield case('falsey_row_amount', lambda v: (v['rows'][0].update(amount=[]),v['reviewed'][0].update(out_amount=0)))


def scenario(v, i):
    v = deepcopy(v)
    # Every variation changes the seed's own operands, never a companion.
    v['reviewed'][0]['component_id'] = f'component-{i}'
    return v
