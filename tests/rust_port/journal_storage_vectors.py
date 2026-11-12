"""Python reference operands for R4-12 storage primitives."""
from copy import deepcopy
from decimal import Decimal, localcontext
import json
from pathlib import Path
import sqlite3
import struct
from kassiber.msat import btc_to_msat
from kassiber.core.journal_calculation import encode_calculation
from kassiber.core.journal_storage import journal_entry_id, journal_processed_at
from kassiber.core.tax_events import dedupe_quarantines
from kassiber.core.custody_journal import _quarantine_detail_with_ownership_candidates

PATH = Path(__file__).resolve().parents[2] / 'crates/journals/tests/data/storage.json'


def vectors():
    amounts = []
    decimals = []
    with localcontext() as context:
        context.prec = 32
        for coefficient in ('0', '-0', '1', '-1', '5', '-5',
                            '9223372036854775807', '9223372036854775808',
                            '18446744073709551615', '18446744073709551616',
                            '340282366920938463463374607431768211455',
                            '340282366920938463463374607431768211456',
                            '123456789012345678901234567890123456789012345678901234567890'):
            for exponent in (-1000, -111, -32, -31, -30, -12, -11, -10, 0, 8, 9, 1000):
                for sign in ('', '-') if not coefficient.startswith('-') else ('',):
                    spelling = f'{sign}{coefficient}E{exponent}'
                    decimal = Decimal(spelling)
                    quantity = btc_to_msat(decimal)
                    with sqlite3.connect(':memory:') as conn:
                        try:
                            stored = conn.execute('SELECT ?', (quantity,)).fetchone()[0]
                        except OverflowError as error:
                            amounts.append([spelling, None, type(error).__name__])
                        else:
                            amounts.append([spelling, stored, None])
                    decimals.append([spelling, struct.unpack('>Q', struct.pack('>d', float(decimal)))[0],
                                     format(decimal, 'f')])
    ids = []
    for values in [('p', 'tx', 'wallet', 'disposal', 'BTC', 0),
                   ('p', 'tx', 'wallet', 'disposal', 'BTC', 1),
                   ('p2', 'tx', 'wallet', 'disposal', 'BTC', 0),
                   ('p', 'other', 'wallet', 'disposal', 'BTC', 0),
                   ('p', 'tx', 'other', 'disposal', 'BTC', 0),
                   ('p', 'tx', 'wallet', 'income', 'BTC', 0),
                   ('p', 'tx', 'wallet', 'disposal', 'LBTC', 0),
                   ('p\\"', 'é\n', '🔥', 'disposal', 'BTC', 99999)]:
        ids.append([*values, journal_entry_id(*values)])
    cases = json.loads(PATH.with_name('render.json').read_text())
    calculation = next(e['calculation'] for c in cases if 'state' in c['expected']
                       for e in json.loads(c['expected']['state'])['entries'] if 'calculation' in e)
    calculations = [None, {}, [], False, 0, '', calculation]
    shared = deepcopy(calculation)
    shared['fragments'] *= 2
    calculations.append(shared)
    empty = deepcopy(calculation); empty['fragments'] = []; calculations.append(empty)
    for path, values in [
        ([], [[], 'unknown', {'unknown': 1}]),
        (['schema_version'], [True, 1.0, 2, None]),
        (['engine'], [1, None]), (['method'], [False, None]),
        (['fragments'], [{}, None]), (['fragments', 0], [None, {}]),
        (['fragments', 0, 'quantity_msat'], [True, 1.0]),
        (['fragments', 0, 'quantity_msat_exact'], ['wrong', 0]),
        (['fragments', 0, 'acquisition_quantity_msat'], [True, 1.0]),
        (['fragments', 0, 'acquisition_quantity_msat_exact'], ['wrong', None]),
        (['fragments', 0, 'cost_basis_exact'], [0.0, True]),
        (['fragments', 0, 'event'], [None, {}, {'unknown': 1}]),
        (['fragments', 0, 'event', 'pricing'], [{}, None]),
        (['fragments', 0, 'event', 'pricing', 'fiat_currency'], [1, True]),
        (['fragments', 0, 'event', 'crypto_fee_msat'], [True, 1.0]),
        (['fragments', 0, 'event', 'crypto_fee_msat_exact'], ['wrong']),
        (['fragments', 0, 'event', 'transaction_id'], [1, True]),
    ]:
        for value in values:
            c = deepcopy(calculation)
            if not path: c = value
            else:
                item = c
                for key in path[:-1]: item = item[key]
                item[path[-1]] = value
            calculations.append(c)
    for key in calculation:
        c=deepcopy(calculation);del c[key];calculations.append(c)
    for key in calculation['fragments'][0]:
        c=deepcopy(calculation);del c['fragments'][0][key];calculations.append(c)
    for path in ([], ['fragments', 0], ['fragments', 0, 'event'], ['fragments', 0, 'event', 'pricing']):
        c=deepcopy(calculation);item=c
        for key in path: item=item[key]
        item['future_key']='must retain legacy';calculations.append(c)
    for path in (['fragments', 0, 'event'], ['fragments', 0, 'event', 'pricing']):
        item=calculation
        for key in path: item=item[key]
        for missing in item:
            c=deepcopy(calculation);item=c
            for key in path: item=item[key]
            del item[missing];calculations.append(c)
    nullable=deepcopy(calculation)
    nullable['fragments'][0]['acquisition_quantity_msat']=None
    nullable['fragments'][0]['acquisition_quantity_msat_exact']=None
    calculations.append(nullable)
    bad=deepcopy(nullable);bad['fragments'][0]['acquisition_quantity_msat_exact']='0';calculations.append(bad)
    nonascii=deepcopy(calculation);nonascii['method']='é🔥\x00\n';calculations.append(nonascii)
    markers=[]
    for candidate, previous in [
        ('2026-10-31T12:00:00Z',None),('2026-10-31T12:00:00Z','2026-10-31T12:00:00Z'),
        ('2020-01-01','2026-10-31T23:59:59.999999Z'),('bad','2020-01-01'),
        ('2026-01-01','bad'),('2027-01-01','2026-01-01'),
        ('2026-10-31T13:00:00+01:00','2026-10-31T12:00:00Z'),
    ]: markers.append([candidate,previous,journal_processed_at(candidate,previous)])
    def quarantine(reason, detail, tx='tx'):
        return dict(transaction_id=tx,workspace_id='w',profile_id='p',reason=reason,detail_json=detail)
    quarantines=[]
    details=['{}','broken','null','[1,2]','{"n":Infinity,"s":"\\ud800"}',
             '{"additional_reasons":[{"reason":"carried"}]}',
             '{"additional_reasons":"é🔥"}','{"additional_reasons":{"first":1,"second":2}}',
             '{"additional_reasons":false}','{"additional_reasons":0}']
    for detail in details:
        rows=[quarantine('custody_basis_barrier',detail),quarantine('root','{"root":true}'),
              quarantine('other','{}','other'),quarantine('root','{"root":true}')]
        quarantines.append([rows,dedupe_quarantines(rows)])
    for reason in ('custody_basis_barrier','transfer_pair_dependency_blocked','basis_provenance_incomplete',
                   'derived_transfer_group_blocked','at_swap_basis_carry_unresolved','bitcoin_rail_carry_basis_unresolved'):
        rows=[quarantine(reason,'{}'),quarantine('root','{}')]
        quarantines.append([rows,dedupe_quarantines(rows)])
    rows=[quarantine('root','{}')]*2
    quarantines.append([rows,dedupe_quarantines(rows)])
    enrichment=[]
    for detail in details+['','{"ownership_review_candidates":["old"]}']:
        for candidates in ([],[{'out_id':'tx','note':'é🔥'}]):
            enrichment.append([detail,candidates,_quarantine_detail_with_ownership_candidates(detail,candidates)])
    return dict(amounts=amounts,decimals=decimals,ids=ids,calculations=[[c,list(encode_calculation(c))] for c in calculations],markers=markers,
                quarantines=quarantines,enrichment=enrichment)


if __name__ == '__main__':
    PATH.write_text(json.dumps(vectors(),ensure_ascii=True,indent=2)+'\n')
